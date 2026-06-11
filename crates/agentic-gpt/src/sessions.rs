use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use agentic_gpt_protocol::{ExecRequest, SessionInfo};
use anyhow::Result;
use chrono::Utc;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::{
    command_preview, confirmation, exec, policy_decision_for_mode, AppState, Config,
    PolicyDecision, SESSION_TAIL_MAX,
};

pub(crate) struct ManagedSession {
    info: SessionInfo,
    child: Child,
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
    last_activity: Instant,
}

#[derive(Debug, Default)]
pub(crate) struct TailBuffer {
    data: std::collections::VecDeque<u8>,
    pub(crate) max: usize,
    pub(crate) truncated: bool,
}

impl TailBuffer {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            data: std::collections::VecDeque::with_capacity(max),
            max,
            truncated: false,
        }
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.data.len() == self.max {
                self.data.pop_front();
                self.truncated = true;
            }
            self.data.push_back(*byte);
        }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.data.iter().copied().collect::<Vec<_>>()).to_string()
    }
}

pub(crate) async fn start_session(
    state: AppState,
    session_id: String,
    request: ExecRequest,
) -> SessionInfo {
    let config = state.config.read().await.clone();
    let started_at = Utc::now();
    let mut info = SessionInfo {
        agent_id: request.agent_id.clone(),
        session_id: session_id.clone(),
        state: "running".to_string(),
        program: request.program.clone(),
        args: request.args.clone(),
        working_directory: request.working_directory.clone(),
        command_preview: command_preview(&request.program, &request.args),
        started_at,
        updated_at: started_at,
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
    };
    let decision = policy_decision_for_mode(
        &config,
        state.run_mode,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    let working_directory =
        match exec::resolve_working_directory(&config, request.working_directory.as_deref()) {
            Ok(working_directory) => working_directory,
            Err(reason) => {
                info.state = "failed".to_string();
                info.reject_reason = Some(reason);
                return info;
            }
        };
    if let Err(reason) =
        exec::preflight(&config, &working_directory, &request.program, &request.args)
    {
        info.state = "failed".to_string();
        info.reject_reason = Some(reason);
        return info;
    }
    if decision == PolicyDecision::Deny {
        info.state = "failed".to_string();
        info.reject_reason = Some("policy_denied".to_string());
        return info;
    }
    if decision == PolicyDecision::Confirm {
        let confirmation = confirmation::request_confirmation(
            &state,
            &config,
            request.confirm_method.as_deref(),
            &request.program,
            &request.args,
        )
        .await;
        if confirmation != "allow_once" {
            info.state = "failed".to_string();
            info.reject_reason = Some(confirmation);
            return info;
        }
    }
    if state.sessions.lock().await.len() >= config.limits.max_active_sessions {
        info.state = "failed".to_string();
        info.reject_reason = Some("max_active_sessions_reached".to_string());
        return info;
    }
    match spawn_session(&config, &working_directory, &request.program, &request.args).await {
        Ok((child, stdout, stderr)) => {
            state.sessions.lock().await.insert(
                session_id,
                ManagedSession {
                    info: info.clone(),
                    child,
                    stdout,
                    stderr,
                    last_activity: Instant::now(),
                },
            );
        }
        Err(error) => {
            info.state = "failed".to_string();
            info.reject_reason = Some(format!("spawn_failed: {error}"));
        }
    }
    info
}

async fn spawn_session(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> Result<(Child, Arc<Mutex<TailBuffer>>, Arc<Mutex<TailBuffer>>)> {
    let mut command = exec::build_command(config, working_directory, program)?;
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    let stderr = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    if let Some(out) = child.stdout.take() {
        tokio::spawn(read_tail(out, stdout.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(read_tail(err, stderr.clone()));
    }
    Ok((child, stdout, stderr))
}

async fn read_tail<R: AsyncRead + Unpin>(
    mut reader: R,
    tail: Arc<Mutex<TailBuffer>>,
) -> Result<()> {
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        tail.lock().await.push(&buffer[..read]);
    }
    Ok(())
}

pub(crate) async fn current_sessions(state: &AppState) -> Vec<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let mut result = Vec::new();
    for session in sessions.values_mut() {
        refresh_session(session).await;
        if matches!(
            session.info.state.as_str(),
            "running" | "waiting_confirmation"
        ) {
            result.push(session.info.clone());
        }
    }
    result
}

pub(crate) async fn inspect_session(state: &AppState, session_id: &str) -> Option<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    refresh_session(session).await;
    Some(session.info.clone())
}

pub(crate) async fn kill_session(state: &AppState, session_id: &str) -> Option<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    let _ = session.child.kill().await;
    refresh_session(session).await;
    session.info.state = "killed".to_string();
    session.info.updated_at = Utc::now();
    Some(session.info.clone())
}

async fn refresh_session(session: &mut ManagedSession) {
    if let Ok(Some(status)) = session.child.try_wait() {
        session.info.exit_code = status.code();
        session.info.state = if status.success() { "exited" } else { "failed" }.to_string();
    }
    let stdout = session.stdout.lock().await;
    let stderr = session.stderr.lock().await;
    session.info.stdout_tail = stdout.text();
    session.info.stderr_tail = stderr.text();
    session.info.truncated = stdout.truncated || stderr.truncated;
    session.info.updated_at = Utc::now();
    session.last_activity = Instant::now();
}
