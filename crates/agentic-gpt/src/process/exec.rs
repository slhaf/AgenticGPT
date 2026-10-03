use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

use crate::{config::Config, policy::PolicyDecision};

#[derive(Clone)]
pub(crate) enum ExecutionSpec {
    Shell { command: String },
    Argv { program: String, args: Vec<String> },
}

#[derive(Clone)]
pub(crate) struct ExecutionRequest {
    pub(crate) agent_id: String,
    pub(crate) group: Option<String>,
    pub(crate) execution: ExecutionSpec,
    pub(crate) need_confirm: bool,
    pub(crate) confirm_method: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) wait_seconds: Option<u64>,
}

impl From<agentic_gpt_protocol::ProcessExecRequest> for ExecutionRequest {
    fn from(request: agentic_gpt_protocol::ProcessExecRequest) -> Self {
        Self {
            agent_id: request.agent_id,
            group: request.group,
            execution: ExecutionSpec::Shell {
                command: request.command,
            },
            need_confirm: request.need_confirm,
            confirm_method: request.confirm_method,
            cwd: request.cwd,
            wait_seconds: request.wait_seconds,
        }
    }
}

impl ExecutionRequest {
    pub(crate) fn argv(
        agent_id: String,
        group: Option<String>,
        program: String,
        args: Vec<String>,
        cwd: Option<String>,
        wait_seconds: Option<u64>,
    ) -> Self {
        Self {
            agent_id,
            group,
            execution: ExecutionSpec::Argv { program, args },
            need_confirm: false,
            confirm_method: None,
            cwd,
            wait_seconds,
        }
    }
}

#[derive(Clone)]
pub(crate) struct PreparedBatchElement {
    pub(crate) index: usize,
    pub(crate) command: String,
    pub(crate) cwd: Option<String>,
    pub(crate) resolved_working_directory: PathBuf,
    pub(crate) decision: PolicyDecision,
}

pub(crate) fn preflight(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> std::result::Result<(), String> {
    if program == "sudo" {
        return Err("interactive_credential_required".to_string());
    }
    if matches!(program, "passwd" | "su" | "login") {
        return Err("interactive_credential_required".to_string());
    }
    if matches!(
        program,
        "vim" | "vi" | "nano" | "less" | "more" | "top" | "htop"
    ) {
        return Err("requires_tty_not_supported".to_string());
    }
    check_path_policy(config, working_directory, program, args)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PathAccessKind {
    Read,
    Write,
}

fn classify_program_access(program: &str) -> PathAccessKind {
    if matches!(
        program,
        "cat"
            | "head"
            | "tail"
            | "stat"
            | "file"
            | "wc"
            | "ls"
            | "find"
            | "du"
            | "df"
            | "upower"
            | "free"
            | "uptime"
            | "fastfetch"
            | "journalctl"
            | "btrfs"
            | "pacman"
    ) {
        PathAccessKind::Read
    } else {
        PathAccessKind::Write
    }
}

fn looks_like_path(arg: &str) -> bool {
    arg == "~"
        || arg.starts_with("~/")
        || arg.starts_with('/')
        || arg.starts_with("./")
        || arg.starts_with("../")
}

fn check_path_policy(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> std::result::Result<(), String> {
    let access = classify_program_access(program);
    let policy = expanded_path_policy(config).map_err(|_| "path_policy_error".to_string())?;
    for arg in args {
        if !looks_like_path(arg) {
            continue;
        }
        let path = resolve_argument_path(working_directory, arg, access)?;
        if path_in_roots(&path, &policy.deny_roots) {
            return Err("path_denied".to_string());
        }
        if program == "df" && arg == "/" {
            continue;
        }
        if path_in_roots(&path, &policy.write_roots) {
            if access == PathAccessKind::Read && !path.exists() {
                return Err("path_not_found".to_string());
            }
            continue;
        }
        if path_in_roots(&path, &policy.read_only_roots) {
            if access == PathAccessKind::Read {
                if !path.exists() {
                    return Err("path_not_found".to_string());
                }
                continue;
            }
            return Err("path_readonly".to_string());
        }
        return Err("path_outside_allowed_roots".to_string());
    }
    Ok(())
}

fn resolve_argument_path(
    workspace_root: &Path,
    arg: &str,
    _access: PathAccessKind,
) -> std::result::Result<PathBuf, String> {
    let expanded = expand_path(arg).map_err(|_| "path_policy_error".to_string())?;
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        workspace_root.join(expanded)
    };
    if candidate.exists() {
        return candidate
            .canonicalize()
            .map_err(|_| "path_policy_error".to_string());
    }
    let parent = candidate
        .parent()
        .ok_or_else(|| "path_not_found".to_string())?;
    let parent = parent
        .canonicalize()
        .map_err(|_| "path_not_found".to_string())?;
    Ok(candidate
        .file_name()
        .map(|name| parent.join(name))
        .unwrap_or(parent))
}

fn path_in_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

#[derive(Debug)]
struct ExpandedPathPolicy {
    write_roots: Vec<PathBuf>,
    read_only_roots: Vec<PathBuf>,
    deny_roots: Vec<PathBuf>,
}
#[derive(Debug)]
pub(crate) enum PathRootNormalizationError {
    Expansion(anyhow::Error),
    Resolution(anyhow::Error),
}

impl std::fmt::Display for PathRootNormalizationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expansion(error) | Self::Resolution(error) => {
                std::fmt::Display::fmt(error, formatter)
            }
        }
    }
}

impl std::error::Error for PathRootNormalizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expansion(error) | Self::Resolution(error) => Some(error.as_ref()),
        }
    }
}
fn expanded_path_policy(config: &Config) -> Result<ExpandedPathPolicy> {
    Ok(ExpandedPathPolicy {
        write_roots: normalize_roots(
            config
                .path_policy
                .write_roots
                .iter()
                .map(PathBuf::as_path)
                .chain(std::iter::once(config.workspace_root.as_path())),
        )?,
        read_only_roots: normalize_roots(
            config
                .path_policy
                .read_only_roots
                .iter()
                .map(PathBuf::as_path),
        )?,
        deny_roots: normalize_roots(config.path_policy.deny_roots.iter().map(PathBuf::as_path))?,
    })
}
pub(crate) fn normalize_roots<'a>(
    roots: impl Iterator<Item = &'a Path>,
) -> std::result::Result<Vec<PathBuf>, PathRootNormalizationError> {
    let mut normalized = Vec::new();
    for root in roots {
        let expanded = expand_pathbuf(root).map_err(PathRootNormalizationError::Expansion)?;
        let normalized_root = canonicalize_existing_or_parent(&expanded)
            .map_err(PathRootNormalizationError::Resolution)?;
        if !normalized
            .iter()
            .any(|existing| existing == &normalized_root)
        {
            normalized.push(normalized_root);
        }
    }
    Ok(normalized)
}

pub(crate) fn canonicalize_existing_or_parent(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut resolved = PathBuf::new();
    let mut missing = Vec::<OsString>::new();
    let mut resolved_is_dir = true;

    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if missing.pop().is_none() {
                    if !resolved_is_dir {
                        return Err(std::io::Error::from(std::io::ErrorKind::NotADirectory).into());
                    }
                    if resolved.pop() {
                        resolved_is_dir = std::fs::metadata(&resolved)?.is_dir();
                    }
                }
            }
            Component::Normal(name) => {
                if !resolved_is_dir {
                    return Err(std::io::Error::from(std::io::ErrorKind::NotADirectory).into());
                }
                if missing.is_empty() {
                    let next = resolved.join(name);
                    match std::fs::symlink_metadata(&next) {
                        Ok(_) => {
                            let metadata = std::fs::metadata(&next)?;
                            resolved = std::fs::canonicalize(next)?;
                            resolved_is_dir = metadata.is_dir();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            missing.push(name.to_os_string());
                        }
                        Err(error) => return Err(error.into()),
                    }
                } else {
                    missing.push(name.to_os_string());
                }
            }
        }
    }

    for component in missing {
        resolved.push(component);
    }
    Ok(resolved)
}

fn expand_path(value: &str) -> Result<PathBuf> {
    if value == "~" {
        return dirs::home_dir().context("home directory not found");
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(dirs::home_dir()
            .context("home directory not found")?
            .join(rest));
    }
    Ok(PathBuf::from(value))
}

pub(crate) fn expand_pathbuf(value: &Path) -> Result<PathBuf> {
    value
        .to_str()
        .map(expand_path)
        .unwrap_or_else(|| Ok(value.to_path_buf()))
}

pub(crate) fn resolve_working_directory(
    config: &Config,
    working_directory: Option<&str>,
) -> std::result::Result<PathBuf, String> {
    let candidate = match working_directory {
        Some(value) if value.trim().is_empty() => {
            return Err("working_directory_empty".to_string());
        }
        Some(value) => {
            let expanded =
                expand_path(value).map_err(|_| "working_directory_invalid".to_string())?;
            if expanded.is_absolute() {
                expanded
            } else {
                config.workspace_root.join(expanded)
            }
        }
        None => config.workspace_root.clone(),
    };
    let directory = candidate
        .canonicalize()
        .map_err(|_| "working_directory_not_found".to_string())?;
    if !directory.is_dir() {
        return Err("working_directory_not_directory".to_string());
    }
    let policy = expanded_path_policy(config).map_err(|_| "path_policy_error".to_string())?;
    if path_in_roots(&directory, &policy.deny_roots) {
        return Err("working_directory_denied".to_string());
    }
    if !path_in_roots(&directory, &policy.write_roots) {
        return Err("working_directory_outside_allowed_roots".to_string());
    }
    Ok(directory)
}

pub(crate) const SHELL_STARTUP_FD: i32 = 3;

pub(crate) fn build_command(
    config: &Config,
    working_directory: &Path,
    execution: &ExecutionSpec,
) -> Result<Command> {
    let (program, args) = match execution {
        ExecutionSpec::Shell { command } => (
            "/usr/bin/bash",
            vec![
                "--noprofile".to_string(),
                "--norc".to_string(),
                "-o".to_string(),
                "pipefail".to_string(),
                "-c".to_string(),
                shell_bootstrap(config, working_directory, command)?,
            ],
        ),
        ExecutionSpec::Argv { program, args } => (program.as_str(), args.clone()),
    };
    if config.sandbox.enabled {
        let policy = expanded_path_policy(config)?;
        let mut command = Command::new(&config.sandbox.bubblewrap_path);
        command
            .arg("--die-with-parent")
            .arg("--unshare-all")
            .arg("--dev")
            .arg("/dev")
            .arg("--chdir")
            .arg(working_directory);
        let mut created_dirs = HashSet::new();
        for path in &policy.write_roots {
            if path.exists() {
                add_bwrap_bind(&mut command, &mut created_dirs, "--bind", path);
            }
        }
        for path in &policy.read_only_roots {
            if path.exists() {
                add_bwrap_bind(&mut command, &mut created_dirs, "--ro-bind", path);
            }
        }
        for path in &config.sandbox.required_runtime_paths {
            if path.exists() {
                add_bwrap_bind(&mut command, &mut created_dirs, "--ro-bind", path);
            }
        }
        command.arg("--").arg(program).args(args);
        if matches!(execution, ExecutionSpec::Shell { .. }) {
            sanitize_shell_environment(&mut command);
        }
        Ok(command)
    } else {
        let mut command = Command::new(program);
        command.current_dir(working_directory).args(args);
        if matches!(execution, ExecutionSpec::Shell { .. }) {
            sanitize_shell_environment(&mut command);
        }
        Ok(command)
    }
}

fn sanitize_shell_environment(command: &mut Command) {
    let environment = std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            !matches!(
                key.as_ref(),
                "BASH_ENV" | "ENV" | "SHELLOPTS" | "BASHOPTS" | "BASH_XTRACEFD" | "POSIXLY_CORRECT"
            ) && !key.starts_with("BASH_FUNC_")
        })
        .collect::<Vec<_>>();
    command.env_clear().envs(environment);
}

fn shell_bootstrap(config: &Config, working_directory: &Path, script: &str) -> Result<String> {
    let init_file = match &config.shell.init_file {
        crate::config::ShellInitFile::Default => Some((
            dirs::home_dir()
                .context("home directory not found")?
                .join(".agentic_gpt")
                .join(".bashrc"),
            true,
        )),
        crate::config::ShellInitFile::Disabled => None,
        crate::config::ShellInitFile::Path(path) => Some((expand_path(path)?, false)),
    };
    let mut bootstrap = String::from("set +e\nset -o pipefail\nbuiltin printf 'B\\n' >&3\n");
    if let Some((path, is_default)) = init_file {
        let path = shell_quote(&path.to_string_lossy());
        if is_default {
            bootstrap.push_str(&format!(
                "__agentic_load_init() {{\n  local __agentic_diag __agentic_open_status __agentic_probe_fd __agentic_init_status\n  __agentic_diag=$(export LC_ALL=C; exec 2>&1 {{__agentic_probe_fd}}< {path})\n  __agentic_open_status=$?\n  if (( __agentic_open_status != 0 )); then\n    if [[ $__agentic_diag == *': No such file or directory' ]]; then return 0; fi\n    builtin printf '%s\\n' \"$__agentic_diag\" >&2\n    return 1\n  fi\n  if builtin source {path}; then return 0; else __agentic_init_status=$?; fi\n  return \"$__agentic_init_status\"\n}}\n"
            ));
        } else {
            bootstrap.push_str(&format!(
                "__agentic_load_init() {{\n  local __agentic_init_status\n  if builtin source {path}; then return 0; else __agentic_init_status=$?; fi\n  return \"$__agentic_init_status\"\n}}\n"
            ));
        }
        bootstrap.push_str(
            "__agentic_load_init\n__agentic_init_status=$?\nbuiltin unset -f __agentic_load_init\nbuiltin unset BASH_ENV ENV POSIXLY_CORRECT\nset +e\nset -o pipefail\nif (( __agentic_init_status != 0 )); then builtin printf 'I:%s\\n' \"$__agentic_init_status\" >&3; exec 3>&-; exit \"$__agentic_init_status\"; fi\n",
        );
    }
    bootstrap.push_str(&format!(
        "builtin cd -- {} || {{ builtin printf 'C:1\\n' >&3; exec 3>&-; exit 1; }}\nbuiltin printf 'R\\n' >&3\nexec 3>&-\nbuiltin eval {}\n",
        shell_quote(&working_directory.to_string_lossy()),
        shell_quote(script),
    ));
    Ok(bootstrap)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn add_bwrap_bind(
    command: &mut Command,
    created_dirs: &mut HashSet<PathBuf>,
    bind_arg: &str,
    path: &Path,
) {
    add_bwrap_parent_dirs(command, created_dirs, path);
    command.arg(bind_arg).arg(path).arg(path);
}

fn add_bwrap_parent_dirs(command: &mut Command, created_dirs: &mut HashSet<PathBuf>, path: &Path) {
    let mut parents = path.ancestors().skip(1).collect::<Vec<_>>();
    parents.reverse();
    for parent in parents {
        if parent == Path::new("/") || parent.as_os_str().is_empty() {
            continue;
        }
        let parent = parent.to_path_buf();
        if created_dirs.insert(parent.clone()) {
            command.arg("--dir").arg(parent);
        }
    }
}
