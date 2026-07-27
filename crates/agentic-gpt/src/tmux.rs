use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use agentic_gpt_protocol::{
    TmuxCapturePaneRequest, TmuxCloseSessionRequest, TmuxCreateSessionRequest, TmuxExecRequest,
    TmuxListPanesRequest, TmuxPasteTextRequest,
};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::sleep;
use uuid::Uuid;

use crate::config::Config;
use crate::policy::{policy_decision_for_profile, PolicyDecision};
use crate::{confirmation, exec, AppState};

const SESSION_FORMAT: &str = "#{session_name}\t#{session_windows}\t#{session_attached}\t#{session_created}\t#{session_activity}";
const PANE_FORMAT: &str = "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_id}\t#{pane_current_path}\t#{pane_current_command}\t#{pane_width}\t#{pane_height}\t#{pane_pid}\t#{pane_in_mode}\t#{pane_dead}";
const MAX_TMUX_EXEC_WAIT_MS: u64 = 5_000;
const MAX_TMUX_CAPTURE_LINES: u32 = 5_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TmuxSession {
    session: String,
    windows: Option<u64>,
    attached: bool,
    created_epoch: Option<u64>,
    activity_epoch: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TmuxPane {
    session: String,
    window_index: Option<u64>,
    pane_index: Option<u64>,
    pane_id: String,
    current_path: String,
    current_command: String,
    width: Option<u64>,
    height: Option<u64>,
    pane_pid: Option<u64>,
    pane_in_mode: bool,
    pane_dead: bool,
    is_likely_shell: bool,
}

pub(crate) async fn list_sessions() -> Value {
    match tmux_output(["list-sessions", "-F", SESSION_FORMAT]).await {
        Ok(stdout) => json!({ "sessions": parse_sessions(&stdout) }),
        Err(error) if error.code == "tmux_server_not_running" => json!({ "sessions": [] }),
        Err(error) => error.value(),
    }
}

pub(crate) async fn list_panes(request: TmuxListPanesRequest) -> Value {
    if let Some(session) = request.session.as_deref() {
        if let Err(error) = validate_identifier("session", session) {
            return error.value();
        }
    }
    let args = list_panes_arguments(request.session.as_deref());
    match tmux_output(args).await {
        Ok(stdout) => json!({ "session": request.session, "panes": parse_panes(&stdout) }),
        Err(error) if error.code == "tmux_server_not_running" && request.session.is_none() => {
            json!({ "session": null, "panes": [] })
        }
        Err(error) => error.value(),
    }
}

fn list_panes_arguments(session: Option<&str>) -> Vec<String> {
    let mut args = vec!["list-panes".to_string()];
    if let Some(session) = session {
        args.extend(["-t".to_string(), session.to_string()]);
    } else {
        args.push("-a".to_string());
    }
    args.extend(["-F".to_string(), PANE_FORMAT.to_string()]);
    args
}

pub(crate) async fn capture_pane(request: TmuxCapturePaneRequest) -> Value {
    if let Err(error) = validate_identifier("target", &request.target) {
        return error.value();
    }
    let lines = request.lines.clamp(1, 5000);
    match tmux_output([
        "capture-pane".to_string(),
        "-t".to_string(),
        request.target.clone(),
        "-p".to_string(),
        "-S".to_string(),
        format!("-{lines}"),
    ])
    .await
    {
        Ok(capture) => json!({ "target": request.target, "lines": lines, "capture": capture }),
        Err(error) => error.value(),
    }
}

pub(crate) async fn paste_text(state: &AppState, request: TmuxPasteTextRequest) -> Value {
    let started = Instant::now();
    let target = request.target.clone();
    let need_confirm = request.need_confirm;
    let submit = request.submit;
    let result = paste_text_inner(state, request).await;
    audit_tmux(
        state,
        "tmux.pasteText",
        vec![target.clone(), format!("submit={submit}")],
        Some(target),
        None,
        need_confirm,
        if need_confirm { "Confirm" } else { "Allow" },
        "hub:tmux.pasteText",
        &result,
        started,
    )
    .await;
    result
}

async fn paste_text_inner(state: &AppState, request: TmuxPasteTextRequest) -> Value {
    if let Err(error) = validate_identifier("target", &request.target) {
        return error.value();
    }
    let pane = match inspect_pane(&request.target).await {
        Ok(pane) => pane,
        Err(error) => return error.value(),
    };
    if pane.is_likely_shell {
        return TmuxError::new(
            "tmux_shell_paste_forbidden",
            "pasteText cannot target a shell pane; use tmux.exec",
        )
        .value();
    }
    if request.need_confirm
        && !confirmed(
            state,
            "paste-buffer",
            &["-t".to_string(), request.target.clone()],
        )
        .await
    {
        return TmuxError::new("confirmation_denied", "tmux paste was not approved").value();
    }

    let mut bytes = request.text.into_bytes();
    if request.submit {
        bytes.push(b'\r');
    }
    match paste_bytes(&request.target, bytes).await {
        Ok(()) => {
            json!({ "target": request.target, "submitted": request.submit, "accepted": true })
        }
        Err(error) => error.value(),
    }
}

pub(crate) async fn exec(state: &AppState, request: TmuxExecRequest) -> Value {
    let started = Instant::now();
    let program = request.program.clone();
    let args = request.args.clone();
    let target = request.target.clone();
    let need_confirm = request.need_confirm;
    let config = state.config.read().await.clone();
    let decision = policy_decision_for_profile(
        &config,
        state.runtime.profile,
        &program,
        &args,
        need_confirm,
    );
    let result = exec_inner(state, request).await;
    let cwd = result
        .get("currentPath")
        .and_then(Value::as_str)
        .map(str::to_string);
    audit_tmux(
        state,
        &program,
        args,
        Some(target),
        cwd,
        need_confirm,
        &format!("{decision:?}"),
        "hub:tmux.exec",
        &result,
        started,
    )
    .await;
    result
}

async fn exec_inner(state: &AppState, request: TmuxExecRequest) -> Value {
    if let Err(error) = validate_identifier("target", &request.target) {
        return error.value();
    }
    if let Err(error) = validate_identifier("program", &request.program) {
        return error.value();
    }
    let pane = match inspect_pane(&request.target).await {
        Ok(pane) => pane,
        Err(error) => return error.value(),
    };
    if pane.pane_dead || pane.pane_in_mode || !pane.is_likely_shell {
        return TmuxError::new(
            "tmux_shell_not_ready",
            "tmux.exec requires a live shell pane outside tmux copy mode",
        )
        .value_with_current_path(&pane.current_path);
    }

    let config = state.config.read().await.clone();
    let cwd = match exec::resolve_working_directory(&config, Some(&pane.current_path)) {
        Ok(cwd) => cwd,
        Err(reason) => {
            return TmuxError::new(&reason, reason.clone())
                .value_with_current_path(&pane.current_path);
        }
    };
    if request.program == "cd" {
        if let Err(reason) = validate_cd_target(&config, &cwd, &request.args) {
            return TmuxError::new(&reason, reason.clone())
                .value_with_current_path(&pane.current_path);
        }
    } else if let Err(reason) = exec::preflight(&config, &cwd, &request.program, &request.args) {
        return TmuxError::new(&reason, reason.clone()).value_with_current_path(&pane.current_path);
    }
    let decision = policy_decision_for_profile(
        &config,
        state.runtime.profile,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    if decision == PolicyDecision::Deny {
        return TmuxError::new("policy_denied", "command was denied by local policy")
            .value_with_current_path(&pane.current_path);
    }
    if decision == PolicyDecision::Confirm
        && confirmation::request_confirmation(state, &config, None, &request.program, &request.args)
            .await
            != "allow_once"
    {
        return TmuxError::new("confirmation_denied", "tmux command was not approved")
            .value_with_current_path(&pane.current_path);
    }

    let mut command = shell_quote(&request.program);
    for arg in &request.args {
        command.push(' ');
        command.push_str(&shell_quote(arg));
    }
    let mut bytes = command.as_bytes().to_vec();
    bytes.push(b'\r');
    match paste_bytes(&request.target, bytes).await {
        Ok(()) => {
            let wait_ms = request.wait_ms.min(MAX_TMUX_EXEC_WAIT_MS);
            let capture_lines = request.capture_lines.min(MAX_TMUX_CAPTURE_LINES);
            let mut result = json!({
                "target": request.target,
                "program": request.program,
                "args": request.args,
                "currentPath": pane.current_path,
                "policyDecision": format!("{decision:?}").to_lowercase(),
                "accepted": true,
                "waitMs": wait_ms,
                "captureLines": capture_lines
            });
            if capture_lines > 0 {
                result["snapshot"] =
                    post_submit_snapshot(&result["target"], wait_ms, capture_lines).await;
            }
            result
        }
        Err(error) => error.value_with_current_path(&pane.current_path),
    }
}

async fn post_submit_snapshot(target: &Value, wait_ms: u64, lines: u32) -> Value {
    let target = target.as_str().unwrap_or_default();
    if wait_ms > 0 {
        sleep(Duration::from_millis(wait_ms)).await;
    }
    match tmux_output([
        "capture-pane".to_string(),
        "-t".to_string(),
        target.to_string(),
        "-p".to_string(),
        "-S".to_string(),
        format!("-{lines}"),
    ])
    .await
    {
        Ok(capture) => {
            json!({ "target": target, "waitMs": wait_ms, "lines": lines, "capture": capture })
        }
        Err(error) => {
            json!({ "target": target, "waitMs": wait_ms, "lines": lines, "error": error.error_detail() })
        }
    }
}

async fn paste_bytes(target: &str, bytes: Vec<u8>) -> Result<(), TmuxError> {
    let buffer = format!("agentic-{}", Uuid::new_v4());
    let mut child = Command::new("tmux")
        .args(["load-buffer", "-b", &buffer, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_error)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| TmuxError::new("tmux_buffer_write_failed", error.to_string()))?;
    }
    match child.wait_with_output().await {
        Ok(output) if output.status.success() => {}
        Ok(output) => return Err(output_error(output)),
        Err(error) => return Err(TmuxError::new("tmux_wait_failed", error.to_string())),
    }
    let pasted = tmux_output(["paste-buffer", "-d", "-t", target, "-b", &buffer]).await;
    if pasted.is_err() {
        let _ = tmux_output(["delete-buffer", "-b", &buffer]).await;
    }
    pasted.map(|_| ())
}

pub(crate) async fn create_session(state: &AppState, request: TmuxCreateSessionRequest) -> Value {
    let started = Instant::now();
    let name = request.name.clone();
    let cwd = request.cwd.clone();
    let result = create_session_inner(state, request).await;
    audit_tmux(
        state,
        "tmux.createSession",
        vec![name.clone()],
        Some(name),
        Some(cwd),
        false,
        "Allow",
        "hub:tmux.createSession",
        &result,
        started,
    )
    .await;
    result
}

async fn create_session_inner(state: &AppState, request: TmuxCreateSessionRequest) -> Value {
    if let Err(error) = validate_identifier("session", &request.name) {
        return error.value();
    }
    let config = state.config.read().await.clone();
    let cwd = match exec::resolve_working_directory(&config, Some(&request.cwd)) {
        Ok(cwd) => cwd,
        Err(reason) => return TmuxError::new(&reason, reason.clone()).value(),
    };
    match create_session_at(&request.name, &cwd).await {
        Ok(created) => json!({ "session": request.name, "cwd": cwd, "created": created }),
        Err(error) => error.value(),
    }
}

pub(crate) async fn create_session_for_config(config: &Config, name: &str, cwd: &str) -> Value {
    if let Err(error) = validate_identifier("session", name) {
        return error.value();
    }
    let cwd = match exec::resolve_working_directory(config, Some(cwd)) {
        Ok(cwd) => cwd,
        Err(reason) => return TmuxError::new(&reason, reason.clone()).value(),
    };
    match create_session_at(name, &cwd).await {
        Ok(created) => json!({ "session": name, "cwd": cwd, "created": created }),
        Err(error) => error.value(),
    }
}

pub(crate) async fn close_session_local(name: &str) -> Value {
    if let Err(error) = validate_identifier("session", name) {
        return error.value();
    }
    match tmux_output(["kill-session", "-t", name]).await {
        Ok(_) => json!({ "session": name, "closed": true }),
        Err(error) => error.value(),
    }
}

pub(crate) async fn close_session(state: &AppState, request: TmuxCloseSessionRequest) -> Value {
    let started = Instant::now();
    let name = request.name.clone();
    let need_confirm = request.need_confirm;
    let result = close_session_inner(state, request).await;
    audit_tmux(
        state,
        "tmux.closeSession",
        vec![name.clone()],
        Some(name),
        None,
        need_confirm,
        if need_confirm { "Confirm" } else { "Allow" },
        "hub:tmux.closeSession",
        &result,
        started,
    )
    .await;
    result
}

async fn close_session_inner(state: &AppState, request: TmuxCloseSessionRequest) -> Value {
    if let Err(error) = validate_identifier("session", &request.name) {
        return error.value();
    }
    if request.need_confirm
        && !confirmed(
            state,
            "kill-session",
            &["-t".to_string(), request.name.clone()],
        )
        .await
    {
        return TmuxError::new("confirmation_denied", "tmux close was not approved").value();
    }
    match tmux_output(["kill-session", "-t", &request.name]).await {
        Ok(_) => json!({ "session": request.name, "closed": true }),
        Err(error) => error.value(),
    }
}

pub(crate) async fn ensure_default_session(workspace_root: &Path) -> Result<bool, TmuxError> {
    create_session_at("agentic", workspace_root).await
}

pub(crate) async fn attach(session: &str) -> Result<(), TmuxError> {
    validate_identifier("session", session)?;
    let status = Command::new("tmux")
        .args(["attach-session", "-t", session])
        .status()
        .await
        .map_err(spawn_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(TmuxError::new(
            "tmux_attach_failed",
            format!("tmux exited with {status}"),
        ))
    }
}

async fn create_session_at(name: &str, cwd: &Path) -> Result<bool, TmuxError> {
    validate_identifier("session", name)?;
    let exists = Command::new("tmux")
        .args(["has-session", "-t", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(spawn_error)?
        .success();
    if exists {
        return Ok(false);
    }
    let output = Command::new("tmux")
        .args(["new-session", "-d", "-s", name, "-c"])
        .arg(cwd)
        .output()
        .await
        .map_err(spawn_error)?;
    if output.status.success() {
        Ok(true)
    } else if String::from_utf8_lossy(&output.stderr).contains("duplicate session") {
        Ok(false)
    } else {
        Err(output_error(output))
    }
}

#[allow(clippy::too_many_arguments)]
async fn audit_tmux(
    state: &AppState,
    program: &str,
    args: Vec<String>,
    job_id: Option<String>,
    working_directory: Option<String>,
    need_confirm: bool,
    policy_decision: &str,
    request_source: &str,
    result: &Value,
    started: Instant,
) {
    let reject_reason = result
        .pointer("/error/code")
        .and_then(Value::as_str)
        .map(str::to_string);
    let confirmation_result = if reject_reason.as_deref() == Some("confirmation_denied") {
        Some("denied".to_string())
    } else if (need_confirm || policy_decision == "Confirm") && result.get("error").is_none() {
        Some("allow_once".to_string())
    } else {
        None
    };
    let config = state.config.read().await.clone();
    let _ = write_audit(
        &config,
        AuditRecord {
            task_id: Some(format!("tmux-{}", Uuid::new_v4())),
            job_id,
            time: Utc::now(),
            program: program.to_string(),
            args,
            working_directory,
            need_confirm,
            policy_decision: policy_decision.to_string(),
            confirmation_result,
            exit_code: None,
            duration_ms: started.elapsed().as_millis(),
            truncated: false,
            request_source: request_source.to_string(),
            reject_reason,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            argument_keys: Vec::new(),
            argument_key_count: None,
            argument_keys_truncated: None,
            argument_bytes: None,
            argument_sha256: None,
            config_revision: None,
            result_bytes: None,
            result_sha256: None,
            terminal_state: None,
            termination_evidence: None,
        },
    );
}

async fn confirmed(state: &AppState, operation: &str, args: &[String]) -> bool {
    let config = state.config.read().await.clone();
    confirmation::request_confirmation(
        state,
        &config,
        None,
        "tmux",
        &[operation.to_string(), args.join(" ")],
    )
    .await
        == "allow_once"
}

async fn tmux_output<I, S>(args: I) -> Result<String, TmuxError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new("tmux")
        .args(args)
        .output()
        .await
        .map_err(spawn_error)?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    Err(output_error(output))
}

async fn inspect_pane(target: &str) -> Result<TmuxPane, TmuxError> {
    let stdout = tmux_output(["display-message", "-p", "-t", target, PANE_FORMAT]).await?;
    parse_panes(&stdout)
        .into_iter()
        .next()
        .ok_or_else(|| TmuxError::new("tmux_target_not_found", "tmux pane was not found"))
}

fn is_likely_shell(command: &str) -> bool {
    matches!(
        command.rsplit('/').next().unwrap_or(command),
        "sh" | "bash" | "dash" | "ash" | "zsh" | "ksh" | "fish"
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn validate_cd_target(
    config: &Config,
    cwd: &Path,
    args: &[String],
) -> std::result::Result<(), String> {
    let target = cd_target_arg(args)?;
    let expanded = exec::expand_pathbuf(Path::new(target))
        .map_err(|_| "working_directory_invalid".to_string())?;
    let candidate: PathBuf = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    let candidate = candidate
        .to_str()
        .ok_or_else(|| "working_directory_invalid".to_string())?;
    exec::resolve_working_directory(config, Some(candidate)).map(|_| ())
}

fn cd_target_arg(args: &[String]) -> std::result::Result<&str, String> {
    if args.len() != 1 {
        return Err("tmux_cd_requires_single_path".to_string());
    }
    let target = args[0].trim();
    if target.is_empty() || target == "-" || target == "--" {
        return Err("tmux_cd_requires_explicit_path".to_string());
    }
    Ok(target)
}

fn parse_sessions(stdout: &str) -> Vec<TmuxSession> {
    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            TmuxSession {
                session: field(&fields, 0).to_string(),
                windows: number(&fields, 1),
                attached: number(&fields, 2).unwrap_or(0) > 0,
                created_epoch: number(&fields, 3),
                activity_epoch: number(&fields, 4),
            }
        })
        .collect()
}

fn parse_panes(stdout: &str) -> Vec<TmuxPane> {
    stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            TmuxPane {
                session: field(&fields, 0).to_string(),
                window_index: number(&fields, 1),
                pane_index: number(&fields, 2),
                pane_id: field(&fields, 3).to_string(),
                current_path: field(&fields, 4).to_string(),
                current_command: field(&fields, 5).to_string(),
                width: number(&fields, 6),
                height: number(&fields, 7),
                pane_pid: number(&fields, 8),
                pane_in_mode: number(&fields, 9).unwrap_or(0) != 0,
                pane_dead: number(&fields, 10).unwrap_or(0) != 0,
                is_likely_shell: is_likely_shell(field(&fields, 5)),
            }
        })
        .collect()
}

fn field<'a>(fields: &'a [&str], index: usize) -> &'a str {
    fields.get(index).copied().unwrap_or_default()
}

fn number(fields: &[&str], index: usize) -> Option<u64> {
    field(fields, index).parse().ok()
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), TmuxError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(TmuxError::new(
            format!("invalid_{kind}"),
            format!("{kind} must be non-empty and contain no control characters"),
        ))
    } else {
        Ok(())
    }
}

fn spawn_error(error: std::io::Error) -> TmuxError {
    if error.kind() == std::io::ErrorKind::NotFound {
        TmuxError::new("tmux_not_installed", "tmux executable was not found")
    } else {
        TmuxError::new("tmux_spawn_failed", error.to_string())
    }
}

fn output_error(output: std::process::Output) -> TmuxError {
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let code = if message.contains("no server running")
        || message.contains("failed to connect to server")
    {
        "tmux_server_not_running"
    } else if message.contains("can't find session")
        || message.contains("can't find pane")
        || message.contains("no such")
    {
        "tmux_target_not_found"
    } else {
        "tmux_command_failed"
    };
    TmuxError::new(
        code,
        if message.is_empty() {
            format!("tmux exited with {}", output.status)
        } else {
            message
        },
    )
}

#[derive(Debug)]
pub(crate) struct TmuxError {
    code: String,
    message: String,
}

impl TmuxError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn error_detail(&self) -> Value {
        json!({ "code": self.code, "message": self.message })
    }

    pub(crate) fn value(&self) -> Value {
        json!({ "error": self.error_detail() })
    }

    fn value_with_current_path(&self, current_path: &str) -> Value {
        json!({
            "error": self.error_detail(),
            "currentPath": current_path
        })
    }
}

impl std::fmt::Display for TmuxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_and_pane_metadata() {
        let sessions = parse_sessions("agentic\t2\t1\t10\t20\n");
        assert_eq!(sessions[0].session, "agentic");
        assert!(sessions[0].attached);
        let panes = parse_panes("agentic\t0\t1\t%3\t/tmp/work\tbash\t120\t40\n");
        assert_eq!(panes[0].pane_id, "%3");
        assert_eq!(panes[0].width, Some(120));
        assert!(panes[0].is_likely_shell);
    }

    #[test]
    fn shell_detection_and_argument_quoting_are_structural() {
        assert!(is_likely_shell("bash"));
        assert!(is_likely_shell("/bin/zsh"));
        assert!(!is_likely_shell("python"));
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn cd_requires_one_explicit_path() {
        assert_eq!(
            cd_target_arg(&[]),
            Err("tmux_cd_requires_single_path".to_string())
        );
        assert_eq!(
            cd_target_arg(&["-".to_string()]),
            Err("tmux_cd_requires_explicit_path".to_string())
        );
        assert_eq!(cd_target_arg(&["/tmp".to_string()]), Ok("/tmp"));
    }

    #[test]
    fn session_scoped_pane_listing_does_not_request_all_panes() {
        let scoped = list_panes_arguments(Some("agentic"));
        assert_eq!(scoped, ["list-panes", "-t", "agentic", "-F", PANE_FORMAT]);
        let all = list_panes_arguments(None);
        assert!(all.iter().any(|argument| argument == "-a"));
    }
}
use crate::audit::{write_audit, AuditRecord};
