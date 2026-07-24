use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use agentic_gpt_protocol::{
    BatchElementResult, BatchExecRequest, BatchExecResult, ExecElement, ExecRequest, TaskResult,
};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::task::JoinSet;
use tokio::time::{timeout, Duration, Instant};

use crate::{
    audit::{write_audit, AuditRecord},
    config::Config,
    confirmation,
    policy::{policy_decision, policy_decision_for_profile, PolicyDecision},
    sessions,
    utils::{log_warn, EXEC_TIMEOUT_SECS, STDERR_MAX, STDOUT_MAX},
    AppState,
};

pub(crate) async fn run_exec_task(
    state: AppState,
    task_id: String,
    request: ExecRequest,
) -> TaskResult {
    let started_at = Utc::now();
    let mut result = TaskResult {
        agent_id: request.agent_id.clone(),
        task_id: task_id.clone(),
        status: "running".to_string(),
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
        started_at,
        updated_at: started_at,
    };
    let config = state.config.read().await.clone();
    let started = Instant::now();
    let decision = policy_decision_for_profile(
        &config,
        state.runtime.profile,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    let mut confirmation_result = None;
    let working_directory =
        match resolve_working_directory(&config, request.working_directory.as_deref()) {
            Ok(working_directory) => Some(working_directory),
            Err(reason) => {
                result.status = "rejected".to_string();
                result.reject_reason = Some(reason);
                None
            }
        };

    if let Some(working_directory) = working_directory.as_deref() {
        if let Err(reason) = preflight(&config, working_directory, &request.program, &request.args)
        {
            result.status = "rejected".to_string();
            result.reject_reason = Some(reason);
        }
    }
    if result.reject_reason.is_none() && decision == PolicyDecision::Deny {
        result.status = "rejected".to_string();
        result.reject_reason = Some("policy_denied".to_string());
    } else if result.reject_reason.is_none() && decision == PolicyDecision::Confirm {
        let confirmation = confirmation::request_confirmation(
            &state,
            &config,
            request.confirm_method.as_deref(),
            &request.program,
            &request.args,
        )
        .await;
        confirmation_result = Some(confirmation.clone());
        if confirmation != "allow_once" {
            result.status = "rejected".to_string();
            result.reject_reason = Some(confirmation);
        }
    }

    if result.reject_reason.is_none() {
        let execution = execute_command(
            &config,
            working_directory
                .as_deref()
                .unwrap_or(&config.workspace_root),
            &request.program,
            &request.args,
        )
        .await;
        match execution {
            Ok(output) => {
                result.status = if output.exit_code == Some(0) {
                    "completed"
                } else {
                    "failed"
                }
                .to_string();
                result.exit_code = output.exit_code;
                result.stdout_tail = output.stdout;
                result.stderr_tail = output.stderr;
                result.truncated = output.truncated;
            }
            Err(reason) => {
                let reason = reason.to_string();
                if reason == "timeout" {
                    result.status = "timeout".to_string();
                    result.reject_reason = Some("exec_timeout_use_session".to_string());
                } else {
                    result.status = "failed".to_string();
                    result.reject_reason = Some(reason);
                }
            }
        }
    }
    result.updated_at = Utc::now();
    let _ = write_audit(
        &config,
        AuditRecord {
            task_id: Some(task_id),
            session_id: None,
            time: result.updated_at,
            program: request.program,
            args: request.args,
            working_directory: request.working_directory,
            need_confirm: request.need_confirm,
            policy_decision: format!("{decision:?}"),
            confirmation_result,
            exit_code: result.exit_code,
            duration_ms: started.elapsed().as_millis(),
            truncated: result.truncated,
            request_source: "hub".to_string(),
            reject_reason: result.reject_reason.clone(),
            skill_id: None,
            skill_path: None,
            installed_digest: None,
        },
    );
    result
}

#[derive(Clone)]
pub(crate) struct PreparedBatchElement {
    pub(crate) index: usize,
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) working_directory: Option<String>,
    pub(crate) resolved_working_directory: PathBuf,
    pub(crate) decision: PolicyDecision,
    pub(crate) reject_reason: Option<String>,
}

pub(crate) fn prepare_batch_element(
    config: &Config,
    index: usize,
    element: ExecElement,
    batch_working_directory: Option<String>,
    need_confirm: bool,
) -> PreparedBatchElement {
    let program = element.program;
    let args = element.args;
    let working_directory = element.working_directory.or(batch_working_directory);
    let decision = policy_decision(config, &program, &args, need_confirm);
    let mut reject_reason = None;
    let resolved_working_directory =
        match resolve_working_directory(config, working_directory.as_deref()) {
            Ok(directory) => directory,
            Err(reason) => {
                reject_reason = Some(reason);
                config.workspace_root.clone()
            }
        };
    if reject_reason.is_none() {
        if let Err(reason) = preflight(config, &resolved_working_directory, &program, &args) {
            reject_reason = Some(reason);
        }
    }
    if reject_reason.is_none() && decision == PolicyDecision::Deny {
        reject_reason = Some("policy_denied".to_string());
    }
    PreparedBatchElement {
        index,
        program,
        args,
        working_directory,
        resolved_working_directory,
        decision,
        reject_reason,
    }
}

fn batch_element_result(
    agent_id: &str,
    batch_id: &str,
    element: &PreparedBatchElement,
    status: &str,
    reject_reason: Option<String>,
    started_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> BatchElementResult {
    BatchElementResult {
        index: element.index,
        program: element.program.clone(),
        args: element.args.clone(),
        working_directory: element.working_directory.clone(),
        result: TaskResult {
            agent_id: agent_id.to_string(),
            task_id: format!("{batch_id}:element:{}", element.index),
            status: status.to_string(),
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason,
            started_at,
            updated_at,
        },
    }
}

async fn run_prepared_batch_element(
    config: Config,
    agent_id: String,
    batch_id: String,
    element: PreparedBatchElement,
    need_confirm: bool,
    confirmation_result: Option<String>,
) -> BatchElementResult {
    let started_at = Utc::now();
    let started = Instant::now();
    let task_id = format!("{batch_id}:element:{}", element.index);
    let mut result = TaskResult {
        agent_id: agent_id.clone(),
        task_id: task_id.clone(),
        status: "running".to_string(),
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
        started_at,
        updated_at: started_at,
    };

    let execution = execute_command(
        &config,
        &element.resolved_working_directory,
        &element.program,
        &element.args,
    )
    .await;
    match execution {
        Ok(output) => {
            result.status = if output.exit_code == Some(0) {
                "completed"
            } else {
                "failed"
            }
            .to_string();
            result.exit_code = output.exit_code;
            result.stdout_tail = output.stdout;
            result.stderr_tail = output.stderr;
            result.truncated = output.truncated;
        }
        Err(reason) => {
            let reason = reason.to_string();
            if reason == "timeout" {
                result.status = "timeout".to_string();
                result.reject_reason = Some("exec_timeout_use_session".to_string());
            } else {
                result.status = "failed".to_string();
                result.reject_reason = Some(reason);
            }
        }
    }
    result.updated_at = Utc::now();
    let _ = write_audit(
        &config,
        AuditRecord {
            task_id: Some(task_id),
            session_id: None,
            time: result.updated_at,
            program: element.program.clone(),
            args: element.args.clone(),
            working_directory: element.working_directory.clone(),
            need_confirm,
            policy_decision: format!("{:?}", element.decision),
            confirmation_result,
            exit_code: result.exit_code,
            duration_ms: started.elapsed().as_millis(),
            truncated: result.truncated,
            request_source: "hub:batch".to_string(),
            reject_reason: result.reject_reason.clone(),
            skill_id: None,
            skill_path: None,
            installed_digest: None,
        },
    );
    BatchElementResult {
        index: element.index,
        program: element.program,
        args: element.args,
        working_directory: element.working_directory,
        result,
    }
}

pub(crate) async fn run_batch_task(
    state: AppState,
    batch_id: String,
    request: BatchExecRequest,
) -> BatchExecResult {
    let started_at = Utc::now();
    let agent_id = request.agent_id.clone();
    let need_confirm = request.need_confirm;
    let confirm_method = request.confirm_method.clone();
    let config = state.config.read().await.clone();
    let prepared = request
        .elements
        .into_iter()
        .enumerate()
        .map(|(index, element)| {
            prepare_batch_element(
                &config,
                index,
                element,
                request.working_directory.clone(),
                need_confirm,
            )
        })
        .collect::<Vec<_>>();
    let total = prepared.len();
    let max_concurrent = config.limits.max_concurrent_tasks.max(1).min(total.max(1));

    if prepared
        .iter()
        .any(|element| element.reject_reason.is_some())
    {
        let updated_at = Utc::now();
        let results = prepared
            .iter()
            .map(|element| {
                if let Some(reason) = &element.reject_reason {
                    batch_element_result(
                        &agent_id,
                        &batch_id,
                        element,
                        "rejected",
                        Some(reason.clone()),
                        started_at,
                        updated_at,
                    )
                } else {
                    batch_element_result(
                        &agent_id,
                        &batch_id,
                        element,
                        "skipped",
                        Some("batch_rejected".to_string()),
                        started_at,
                        updated_at,
                    )
                }
            })
            .collect::<Vec<_>>();
        return BatchExecResult {
            agent_id,
            batch_id,
            status: "rejected".to_string(),
            results,
            started_at,
            updated_at,
        };
    }

    let needs_confirmation = prepared
        .iter()
        .filter(|element| element.decision == PolicyDecision::Confirm)
        .cloned()
        .collect::<Vec<_>>();
    let mut batch_confirmation_result = None;
    if !needs_confirmation.is_empty() {
        let confirmation = confirmation::request_batch_confirmation(
            &state,
            &config,
            confirm_method.as_deref(),
            &needs_confirmation,
            &prepared,
        )
        .await;
        batch_confirmation_result = Some(confirmation.clone());
        if confirmation != "allow_once" {
            let updated_at = Utc::now();
            let reason = if confirmation == "timeout" {
                "batch_confirmation_timeout".to_string()
            } else {
                format!("batch_confirmation_{confirmation}")
            };
            let results = prepared
                .iter()
                .map(|element| {
                    if element.decision == PolicyDecision::Confirm {
                        batch_element_result(
                            &agent_id,
                            &batch_id,
                            element,
                            "rejected",
                            Some(reason.clone()),
                            started_at,
                            updated_at,
                        )
                    } else {
                        batch_element_result(
                            &agent_id,
                            &batch_id,
                            element,
                            "skipped",
                            Some("batch_rejected".to_string()),
                            started_at,
                            updated_at,
                        )
                    }
                })
                .collect::<Vec<_>>();
            return BatchExecResult {
                agent_id,
                batch_id,
                status: "rejected".to_string(),
                results,
                started_at,
                updated_at,
            };
        }
    }

    let mut pending = prepared.into_iter().collect::<VecDeque<_>>();
    let mut running = JoinSet::new();
    let mut results: Vec<Option<BatchElementResult>> = vec![None; total];
    let deadline = Instant::now() + Duration::from_secs(EXEC_TIMEOUT_SECS);

    loop {
        while running.len() < max_concurrent {
            let Some(element) = pending.pop_front() else {
                break;
            };
            let element_config = config.clone();
            let element_agent_id = agent_id.clone();
            let element_batch_id = batch_id.clone();
            let confirmation_result = if element.decision == PolicyDecision::Confirm {
                batch_confirmation_result.clone()
            } else {
                None
            };
            running.spawn(async move {
                run_prepared_batch_element(
                    element_config,
                    element_agent_id,
                    element_batch_id,
                    element,
                    need_confirm,
                    confirmation_result,
                )
                .await
            });
        }

        if running.is_empty() {
            break;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match timeout(remaining, running.join_next()).await {
            Ok(Some(Ok(element_result))) => {
                let index = element_result.index;
                if index < results.len() {
                    results[index] = Some(element_result);
                }
            }
            Ok(Some(Err(error))) => {
                log_warn(format!("batch element task join failed: {error}"));
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    running.abort_all();

    let updated_at = Utc::now();
    let results = results
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            result.unwrap_or_else(|| {
                let fallback = PreparedBatchElement {
                    index,
                    program: "<unknown>".to_string(),
                    args: Vec::new(),
                    working_directory: None,
                    resolved_working_directory: config.workspace_root.clone(),
                    decision: PolicyDecision::Allow,
                    reject_reason: None,
                };
                batch_element_result(
                    &agent_id,
                    &batch_id,
                    &fallback,
                    "timeout",
                    Some("exec_timeout_use_session".to_string()),
                    started_at,
                    updated_at,
                )
            })
        })
        .collect::<Vec<_>>();

    let status = if results
        .iter()
        .any(|element| element.result.status == "timeout")
    {
        "timeout"
    } else if results
        .iter()
        .any(|element| element.result.status != "completed")
    {
        "partial_failed"
    } else {
        "completed"
    }
    .to_string();

    BatchExecResult {
        agent_id,
        batch_id,
        status,
        results,
        started_at,
        updated_at,
    }
}

#[derive(Debug)]
struct CommandOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    truncated: bool,
}

async fn execute_command(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> Result<CommandOutput> {
    let mut command = build_command(config, working_directory, program)?;
    command.args(args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| anyhow!("spawn_failed: {error}"))?;
    let stdout = child.stdout.take().context("stdout pipe missing")?;
    let stderr = child.stderr.take().context("stderr pipe missing")?;
    let stdout_task = tokio::spawn(read_limited(stdout, STDOUT_MAX));
    let stderr_task = tokio::spawn(read_limited(stderr, STDERR_MAX));
    let status = match timeout(Duration::from_secs(EXEC_TIMEOUT_SECS), child.wait()).await {
        Ok(status) => status.map_err(|error| anyhow!("wait_failed: {error}"))?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(anyhow!("timeout"));
        }
    };
    let stdout = stdout_task.await??;
    let stderr = stderr_task.await??;
    Ok(CommandOutput {
        exit_code: status.code(),
        truncated: stdout.1 || stderr.1,
        stdout: stdout.0,
        stderr: stderr.0,
    })
}

async fn read_limited<R: AsyncRead + Unpin>(mut reader: R, max: usize) -> Result<(String, bool)> {
    let mut tail = sessions::TailBuffer::new(max);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        tail.push(&buffer[..read]);
    }
    Ok((tail.text(), tail.truncated))
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

fn expanded_path_policy(config: &Config) -> Result<ExpandedPathPolicy> {
    Ok(ExpandedPathPolicy {
        write_roots: normalize_roots(
            config
                .path_policy
                .write_roots
                .iter()
                .chain(std::iter::once(&config.workspace_root)),
        )?,
        read_only_roots: normalize_roots(config.path_policy.read_only_roots.iter())?,
        deny_roots: normalize_roots(config.path_policy.deny_roots.iter())?,
    })
}

fn normalize_roots<'a>(roots: impl Iterator<Item = &'a PathBuf>) -> Result<Vec<PathBuf>> {
    let mut normalized = Vec::new();
    for root in roots {
        let expanded = expand_pathbuf(root)?;
        let normalized_root = canonicalize_existing_or_parent(&expanded)?;
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
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    if let Some(parent) = path.parent() {
        if parent.exists() {
            let parent = parent.canonicalize()?;
            return Ok(path
                .file_name()
                .map(|name| parent.join(name))
                .unwrap_or(parent));
        }
    }
    Ok(path.to_path_buf())
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

pub(crate) fn build_command(
    config: &Config,
    working_directory: &Path,
    program: &str,
) -> Result<Command> {
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
        command.arg("--").arg(program);
        Ok(command)
    } else {
        let mut command = Command::new(program);
        command.current_dir(working_directory);
        Ok(command)
    }
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
