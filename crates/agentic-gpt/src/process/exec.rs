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
/// Reject a single argument that the Linux kernel cannot pass to `execve`.
///
/// For shell execution, count the exact `bash -c` bootstrap using the same
/// emitter as command construction, without allocating the argument string.
pub(crate) fn kernel_argument_limit_error(
    config: &Config,
    working_directory: Option<&Path>,
    execution: &ExecutionSpec,
) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let page_size = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
        let limit = page_size.checked_mul(32)?;
        let oversized_argument = |argument: &str| {
            let bytes = argument.len().saturating_add(1);
            (bytes > limit).then_some(bytes)
        };
        match execution {
            ExecutionSpec::Shell { command } => {
                let working_directory = working_directory?;
                let bytes = shell_bootstrap_argument_len(config, working_directory, command)
                    .ok()?
                    .saturating_add(1);
                (bytes > limit).then(|| {
                    format!(
                        "process_kernel_argument_too_large: argument=bash_-c; bytes={bytes}; max={limit}"
                    )
                })
            }
            ExecutionSpec::Argv { program, args } => {
                if let Some(bytes) = oversized_argument(program) {
                    return Some(format!(
                        "process_kernel_argument_too_large: argument=program; bytes={bytes}; max={limit}"
                    ));
                }
                args.iter()
                    .enumerate()
                    .find_map(|(index, argument)| {
                        oversized_argument(argument).map(|bytes| {
                            format!(
                                "process_kernel_argument_too_large: argument=argv[{index}]; bytes={bytes}; max={limit}"
                            )
                        })
                    })
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (config, working_directory, execution);
        None
    }
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
    let mut bootstrap = String::new();
    append_shell_bootstrap(&mut bootstrap, config, working_directory, script)?;
    Ok(bootstrap)
}

#[cfg(target_os = "linux")]
fn shell_bootstrap_argument_len(
    config: &Config,
    working_directory: &Path,
    script: &str,
) -> Result<usize> {
    let mut length = BootstrapLength(0);
    append_shell_bootstrap(&mut length, config, working_directory, script)?;
    Ok(length.0)
}

#[cfg(target_os = "linux")]
struct BootstrapLength(usize);

#[cfg(target_os = "linux")]
impl std::fmt::Write for BootstrapLength {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0 = self.0.saturating_add(text.len());
        Ok(())
    }
}

fn append_shell_bootstrap(
    output: &mut impl std::fmt::Write,
    config: &Config,
    working_directory: &Path,
    script: &str,
) -> Result<()> {
    let init_file = match &config.shell.init_file {
        crate::config::ShellInitFile::Default => Some((
            dirs::home_dir()
                .context("home directory not found")?
                .join(".agentic_gpt")
                .join(".bashrc"),
            true,
        )),
        crate::config::ShellInitFile::Disabled => None,
        crate::config::ShellInitFile::Path(path) => {
            let path = expand_path(path)?;
            let path = if path.is_absolute() {
                path
            } else {
                working_directory.join(path)
            };
            Some((path, false))
        }
    };
    write_bootstrap_text(
        output,
        "set +e\nset -o pipefail\nif ! exec {__agentic_shell_control_fd}>&3; then exit 1; fi\nexec 3>&-\nif ! builtin printf 'B\\n' >&\"$__agentic_shell_control_fd\"; then exit 1; fi\n",
    );
    if let Some((path, is_default)) = init_file {
        if is_default {
            write_bootstrap_text(
                output,
                "__agentic_diag=$(export LC_ALL=C; exec 2>&1 {__agentic_probe_fd}< ",
            );
            write_shell_quote_path(output, &path);
            write_bootstrap_text(
                output,
                ")\n__agentic_open_status=$?\nif (( __agentic_open_status == 0 )); then\n  if builtin source ",
            );
            write_shell_quote_path(output, &path);
            write_bootstrap_text(
                output,
                "; then __agentic_init_status=0; else __agentic_init_status=$?; fi\nelif [[ $__agentic_diag == *': No such file or directory' ]]; then\n  __agentic_init_status=0\nelse\n  builtin printf '%s\\n' \"$__agentic_diag\" >&2\n  __agentic_init_status=1\nfi\nbuiltin unset __agentic_diag __agentic_open_status\n",
            );
        } else {
            write_bootstrap_text(output, "if builtin source ");
            write_shell_quote_path(output, &path);
            write_bootstrap_text(
                output,
                "; then __agentic_init_status=0; else __agentic_init_status=$?; fi\n",
            );
        }
        write_bootstrap_text(
            output,
            "builtin unset BASH_ENV ENV POSIXLY_CORRECT\nset +e\nset -o pipefail\n",
        );
        write_bootstrap_text(
            output,
            "if (( __agentic_init_status != 0 )); then builtin printf 'I:%s\\n' \"$__agentic_init_status\" >&\"$__agentic_shell_control_fd\"; exec {__agentic_shell_control_fd}>&-; exit \"$__agentic_init_status\"; fi\n",
        );
        write_bootstrap_text(output, "builtin unset __agentic_init_status\n");
    }
    write_bootstrap_text(output, "builtin cd -- ");
    write_shell_quote_path(output, working_directory);
    write_bootstrap_text(
        output,
        " || { builtin printf 'C:1\\n' >&\"$__agentic_shell_control_fd\"; exec {__agentic_shell_control_fd}>&-; exit 1; }\n",
    );
    write_bootstrap_text(
        output,
        "if ! builtin printf 'R\\n' >&\"$__agentic_shell_control_fd\"; then exit 1; fi\n",
    );
    write_bootstrap_text(
        output,
        "exec {__agentic_shell_control_fd}>&-\nbuiltin unset __agentic_shell_control_fd\nbuiltin eval -- ",
    );
    write_shell_quote(output, script);
    output
        .write_char('\n')
        .expect("bootstrap writer is infallible");
    Ok(())
}

fn write_shell_quote_path(output: &mut impl std::fmt::Write, path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let mut bytes = path.as_os_str().as_bytes();
        write_bootstrap_text(output, "'");
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    write_shell_quote_contents(output, text);
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    let valid = std::str::from_utf8(&bytes[..valid_up_to])
                        .expect("UTF-8 error prefix is valid");
                    write_shell_quote_contents(output, valid);
                    output
                        .write_char('\u{FFFD}')
                        .expect("bootstrap writer is infallible");
                    let invalid_len = error
                        .error_len()
                        .unwrap_or_else(|| bytes.len() - valid_up_to);
                    bytes = &bytes[valid_up_to + invalid_len..];
                }
            }
        }
        write_bootstrap_text(output, "'");
    }
    #[cfg(not(unix))]
    {
        write_shell_quote(output, &path.to_string_lossy());
    }
}

fn write_shell_quote(output: &mut impl std::fmt::Write, value: &str) {
    write_bootstrap_text(output, "'");
    write_shell_quote_contents(output, value);
    write_bootstrap_text(output, "'");
}

fn write_shell_quote_contents(output: &mut impl std::fmt::Write, value: &str) {
    let mut pieces = value.split('\'').peekable();
    while let Some(piece) = pieces.next() {
        write_bootstrap_text(output, piece);
        if pieces.peek().is_some() {
            write_bootstrap_text(output, "'\\''");
        }
    }
}

fn write_bootstrap_text(output: &mut impl std::fmt::Write, text: &str) {
    output
        .write_str(text)
        .expect("bootstrap writer is infallible");
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
#[cfg(all(test, unix))]
mod tests {
    use std::{
        fs,
        io::Read,
        os::unix::{io::AsRawFd, process::CommandExt},
        process::Stdio,
        time::Duration,
    };

    use super::{build_command, ExecutionSpec, SHELL_STARTUP_FD};
    use crate::config::{Config, ShellInitFile};
    use uuid::Uuid;

    #[tokio::test]
    async fn relative_shell_init_file_is_not_resolved_through_path() {
        let root = std::env::temp_dir().join(format!("shell-init-path-{}", Uuid::new_v4()));
        let home = root.join("home");
        let tmp = root.join("tmp");
        let working_directory = root.join("cwd");
        let path_directory = root.join("path");
        for directory in [&home, &tmp, &working_directory, &path_directory] {
            fs::create_dir_all(directory).unwrap();
        }
        let init_name = format!("init-{}.bash", Uuid::new_v4());
        fs::write(
            working_directory.join(&init_name),
            "CUTOVER_INIT_SOURCE=from_working_directory\n",
        )
        .unwrap();
        fs::write(
            path_directory.join(&init_name),
            "CUTOVER_INIT_SOURCE=from_PATH\n",
        )
        .unwrap();

        let mut config = Config::default_config().unwrap();
        config.sandbox.enabled = false;
        config.shell.init_file = ShellInitFile::Path(init_name);
        let mut command = build_command(
            &config,
            &working_directory,
            &ExecutionSpec::Shell {
                command: "printf '%s' \"$CUTOVER_INIT_SOURCE\"".to_string(),
            },
        )
        .unwrap();
        command
            .env("HOME", &home)
            .env("TMPDIR", &tmp)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("PATH", &path_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let (startup_reader, startup_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let startup_fd = startup_writer.as_raw_fd();
        unsafe {
            command.as_std_mut().pre_exec(move || {
                if startup_fd != SHELL_STARTUP_FD {
                    if libc::dup2(startup_fd, SHELL_STARTUP_FD) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                } else {
                    let flags = libc::fcntl(startup_fd, libc::F_GETFD);
                    if flags == -1
                        || libc::fcntl(startup_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }

        let child = command.spawn().unwrap();
        drop(startup_writer);
        let output = child.wait_with_output().await.unwrap();
        let mut startup_markers = Vec::new();
        startup_reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut startup_reader = startup_reader;
        startup_reader.read_to_end(&mut startup_markers).unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout.as_slice(), b"from_working_directory");
        assert_eq!(startup_markers.as_slice(), b"B\nR\n");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn losing_startup_channel_before_ready_prevents_command_execution() {
        let root = std::env::temp_dir().join(format!("shell-startup-channel-{}", Uuid::new_v4()));
        let working_directory = root.join("cwd");
        fs::create_dir_all(&working_directory).unwrap();
        let release_init = root.join("release-init");
        let init_file = root.join("init.bash");
        let marker = root.join("must-not-run");
        fs::write(
            &init_file,
            format!(
                "while [[ ! -e '{}' ]]; do :; done\n",
                release_init.display()
            ),
        )
        .unwrap();

        let mut config = Config::default_config().unwrap();
        config.sandbox.enabled = false;
        config.shell.init_file = ShellInitFile::Path(init_file.to_string_lossy().to_string());
        let mut command = build_command(
            &config,
            &working_directory,
            &ExecutionSpec::Shell {
                command: format!("printf ran > '{}'", marker.display()),
            },
        )
        .unwrap();
        command
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let (startup_reader, startup_writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let startup_fd = startup_writer.as_raw_fd();
        unsafe {
            command.as_std_mut().pre_exec(move || {
                if startup_fd != SHELL_STARTUP_FD {
                    if libc::dup2(startup_fd, SHELL_STARTUP_FD) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                } else {
                    let flags = libc::fcntl(startup_fd, libc::F_GETFD);
                    if flags == -1
                        || libc::fcntl(startup_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }

        let child = command.spawn().unwrap();
        drop(startup_writer);
        let mut startup_reader = startup_reader;
        startup_reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut boot_marker = [0; 2];
        startup_reader.read_exact(&mut boot_marker).unwrap();
        assert_eq!(boot_marker, *b"B\n");
        startup_reader.shutdown(std::net::Shutdown::Both).unwrap();
        drop(startup_reader);
        fs::write(&release_init, "").unwrap();

        let output = tokio::time::timeout(Duration::from_secs(3), child.wait_with_output())
            .await
            .expect("shell should stop after startup control delivery fails")
            .unwrap();
        assert!(!output.status.success());
        assert!(!marker.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
