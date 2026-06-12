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
    config::Config,
    confirmation, exec,
    policy::{policy_decision_for_mode, PolicyDecision},
    utils::{command_preview, SESSION_TAIL_MAX},
    AppState,
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
    let active_session_count = {
        let mut sessions = state.sessions.lock().await;
        refresh_sessions(&mut sessions).await;
        sessions
            .values()
            .filter(|session| is_active_session_state(&session.info.state))
            .count()
    };
    if active_session_count >= config.limits.max_active_sessions {
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
    refresh_sessions(&mut sessions).await;
    sessions
        .values()
        .filter(|session| is_active_session_state(&session.info.state))
        .map(|session| session.info.clone())
        .collect()
}

async fn refresh_sessions(sessions: &mut std::collections::HashMap<String, ManagedSession>) {
    for session in sessions.values_mut() {
        refresh_session(session).await;
    }
}

fn is_active_session_state(state: &str) -> bool {
    matches!(state, "running" | "waiting_confirmation")
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::PathBuf, sync::Arc, time::Duration};

    use tokio::sync::{Mutex, RwLock};
    use uuid::Uuid;

    use super::*;
    use crate::state::RunMode;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn exec_request(program: &str, working_directory: &std::path::Path) -> ExecRequest {
        ExecRequest {
            agent_id: "test-agent".to_string(),
            program: program.to_string(),
            args: Vec::new(),
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(working_directory.to_string_lossy().to_string()),
        }
    }

    async fn test_state(max_active_sessions: usize) -> (AppState, PathBuf) {
        let root = unique_temp_dir("sessions-max-active");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.limits.max_active_sessions = max_active_sessions;
        config.confirmation_provider.provider = "none".to_string();

        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(RwLock::new(config)),
            run_mode: RunMode::Normal,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
        };
        (state, workspace)
    }

    async fn wait_until_terminal(state: &AppState, session_id: &str) -> SessionInfo {
        for _ in 0..50 {
            let session = inspect_session(state, session_id)
                .await
                .expect("session should be inspectable");
            if !is_active_session_state(&session.state) {
                return session;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        inspect_session(state, session_id)
            .await
            .expect("session should be inspectable")
    }

    #[tokio::test]
    async fn completed_sessions_do_not_consume_max_active_session_slots() {
        let (state, workspace) = test_state(1).await;

        let first = start_session(
            state.clone(),
            "sess-first".to_string(),
            exec_request("true", &workspace),
        )
        .await;
        assert_eq!(first.state, "running");

        let first = wait_until_terminal(&state, "sess-first").await;
        assert_eq!(first.state, "exited");

        assert!(current_sessions(&state).await.is_empty());

        let second = start_session(
            state.clone(),
            "sess-second".to_string(),
            exec_request("true", &workspace),
        )
        .await;
        assert_ne!(
            second.reject_reason.as_deref(),
            Some("max_active_sessions_reached")
        );
        assert_eq!(second.state, "running");
    }
}
