use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use agentic_gpt_protocol::{ExecRequest, SessionInfo};
use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::sync::{Mutex, OwnedRwLockReadGuard, RwLock};
use tokio::time::{sleep, Instant};

use crate::{
    audit::{write_audit, AuditRecord},
    config::Config,
    confirmation, exec,
    policy::{policy_decision_for_mode, PolicyDecision},
    utils::{command_preview, SESSION_TAIL_MAX},
    AppState,
};

pub(crate) struct ManagedSession {
    info: SessionInfo,
    child: Option<Child>,
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
    last_activity: Instant,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
    skill_lease: Option<SkillLease>,
    skill_audit: Option<SkillAuditContext>,
}

struct SkillAuditContext {
    config: Config,
    skill_id: String,
    skill_path: String,
    installed_digest: Option<String>,
}

#[derive(Clone, Default)]
pub(crate) struct SkillLeaseManager {
    locks: Arc<Mutex<std::collections::HashMap<String, Arc<RwLock<()>>>>>,
}

pub(crate) struct SkillLease {
    _guard: OwnedRwLockReadGuard<()>,
}

impl SkillLeaseManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    async fn lock_for(&self, id: &str) -> Arc<RwLock<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }

    pub(crate) async fn try_shared(&self, id: &str) -> Option<SkillLease> {
        self.lock_for(id)
            .await
            .try_read_owned()
            .ok()
            .map(|guard| SkillLease { _guard: guard })
    }

    pub(crate) async fn acquire_exclusive(
        &self,
        id: &str,
        deadline: tokio::time::Duration,
    ) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, tokio::time::error::Elapsed> {
        tokio::time::timeout(deadline, self.lock_for(id).await.write_owned()).await
    }
}

const TERMINAL_RETENTION_HOURS: i64 = 24;
const MAX_TERMINAL_SESSIONS: usize = 100;

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

#[cfg(test)]
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
                    child: Some(child),
                    stdout,
                    stderr,
                    last_activity: Instant::now(),
                    cancel_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    skill_lease: None,
                    skill_audit: None,
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

pub(crate) async fn start_session_async(
    state: AppState,
    session_id: String,
    request: ExecRequest,
) -> SessionInfo {
    start_session_async_inner(state, session_id, request, None, None).await
}

async fn start_session_async_inner(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    skill_lease: Option<SkillLease>,
    skill_audit: Option<SkillAuditContext>,
) -> SessionInfo {
    let config = state.config.read().await.clone();
    let started_at = Utc::now();
    let mut info = SessionInfo {
        agent_id: request.agent_id.clone(),
        session_id: session_id.clone(),
        state: "starting".to_string(),
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
            Ok(directory) => directory,
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
    {
        let mut sessions = state.sessions.lock().await;
        prune_terminal_sessions(&mut sessions);
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
    if decision == PolicyDecision::Confirm {
        info.state = "waiting_confirmation".to_string();
    }
    let stdout = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    let stderr = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    let cancel_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    state.sessions.lock().await.insert(
        session_id.clone(),
        ManagedSession {
            info: info.clone(),
            child: None,
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            last_activity: Instant::now(),
            cancel_requested: cancel_requested.clone(),
            skill_lease,
            skill_audit,
        },
    );
    let monitor_state = state.clone();
    tokio::spawn(async move {
        run_async_session(
            state,
            session_id,
            request,
            working_directory,
            decision,
            stdout,
            stderr,
            cancel_requested,
        )
        .await;
    });
    let monitor_session_id = info.session_id.clone();
    tokio::spawn(async move {
        monitor_session(monitor_state, monitor_session_id).await;
    });
    info
}

pub(crate) async fn start_skill_session_async(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    skill_id: &str,
    skill_path: &str,
) -> SessionInfo {
    let config = state.config.read().await.clone();
    let lease = state.skill_leases.try_shared(skill_id).await;
    if lease.is_none() {
        let now = Utc::now();
        return SessionInfo {
            agent_id: request.agent_id.clone(),
            session_id,
            state: "failed".to_string(),
            program: request.program.clone(),
            args: request.args.clone(),
            working_directory: request.working_directory.clone(),
            command_preview: command_preview(&request.program, &request.args),
            started_at: now,
            updated_at: now,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: Some("skill_update_pending".to_string()),
        };
    } else {
        start_session_async_inner(
            state,
            session_id,
            request,
            lease,
            Some(SkillAuditContext {
                config: config.clone(),
                skill_id: skill_id.to_string(),
                skill_path: skill_path.to_string(),
                installed_digest: crate::skill_installs::package_sha256(&config, skill_id).ok(),
            }),
        )
        .await
    }
}

async fn run_async_session(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    working_directory: std::path::PathBuf,
    decision: PolicyDecision,
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
) {
    if decision == PolicyDecision::Confirm {
        let config = state.config.read().await.clone();
        let confirmation = confirmation::request_confirmation_cancellable(
            &state,
            &config,
            request.confirm_method.as_deref(),
            &request.program,
            &request.args,
            cancel_requested.clone(),
        )
        .await;
        if confirmation != "allow_once" {
            finish_pending_session(&state, &session_id, &confirmation).await;
            return;
        }
    }
    if cancel_requested.load(std::sync::atomic::Ordering::Acquire) {
        finish_pending_session(&state, &session_id, "cancelled").await;
        return;
    }
    let config = state.config.read().await.clone();
    let spawned = spawn_session_with_buffers(
        &config,
        &working_directory,
        &request.program,
        &request.args,
        stdout,
        stderr,
    )
    .await;
    let Ok(child) = spawned else {
        let reason = spawned
            .err()
            .map(|error| format!("spawn_failed: {error}"))
            .unwrap_or_else(|| "spawn_failed".to_string());
        finish_pending_session(&state, &session_id, &reason).await;
        return;
    };
    let mut sessions = state.sessions.lock().await;
    let cancelled_before_attach = cancel_requested.load(std::sync::atomic::Ordering::Acquire);
    if let Some(session) = sessions.get_mut(&session_id) {
        if cancelled_before_attach {
            session.info.state = "killed".to_string();
            session.info.reject_reason = Some("cancelled".to_string());
        } else {
            session.info.state = "running".to_string();
            session.info.updated_at = Utc::now();
            session.child = Some(child);
            return;
        }
    }
    drop(sessions);
    let mut child = child;
    let _ = child.kill().await;
    if cancelled_before_attach {
        finish_pending_session(&state, &session_id, "cancelled").await;
    }
}

async fn spawn_session_with_buffers(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
) -> Result<Child> {
    let mut command = exec::build_command(config, working_directory, program)?;
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(out) = child.stdout.take() {
        tokio::spawn(read_tail(out, stdout));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(read_tail(err, stderr));
    }
    Ok(child)
}

async fn monitor_session(state: AppState, session_id: String) {
    loop {
        sleep(std::time::Duration::from_millis(50)).await;
        let Some(info) = inspect_session(&state, &session_id).await else {
            return;
        };
        if !is_active_session_state(&info.state) {
            return;
        }
    }
}

async fn finish_pending_session(state: &AppState, session_id: &str, reason: &str) {
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(session_id) {
        session.info.state = if reason == "cancelled" {
            "killed".to_string()
        } else {
            "failed".to_string()
        };
        session.info.reject_reason = Some(reason.to_string());
        session.info.updated_at = Utc::now();
        session.skill_lease = None;
        if let Some(context) = session.skill_audit.take() {
            let _ = write_audit(
                &context.config,
                AuditRecord {
                    task_id: None,
                    session_id: Some(session.info.session_id.clone()),
                    time: session.info.updated_at,
                    program: session.info.program.clone(),
                    args: session.info.args.clone(),
                    working_directory: session.info.working_directory.clone(),
                    need_confirm: false,
                    policy_decision: "inherited".to_string(),
                    confirmation_result: Some(reason.to_string()),
                    exit_code: session.info.exit_code,
                    duration_ms: (session.info.updated_at - session.info.started_at)
                        .num_milliseconds()
                        .max(0) as u128,
                    truncated: session.info.truncated,
                    request_source: "skills.run".to_string(),
                    reject_reason: session.info.reject_reason.clone(),
                    skill_id: Some(context.skill_id),
                    skill_path: Some(context.skill_path),
                    installed_digest: context.installed_digest,
                },
            );
        }
    }
    prune_terminal_sessions(&mut sessions);
}

#[cfg(test)]
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
    prune_terminal_sessions(&mut sessions);
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
    matches!(state, "starting" | "running" | "waiting_confirmation")
}

pub(crate) async fn inspect_session(state: &AppState, session_id: &str) -> Option<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    refresh_session(session).await;
    let info = session.info.clone();
    prune_terminal_sessions(&mut sessions);
    Some(info)
}

pub(crate) async fn kill_session(state: &AppState, session_id: &str) -> Option<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    session
        .cancel_requested
        .store(true, std::sync::atomic::Ordering::Release);
    if let Some(child) = session.child.as_mut() {
        let _ = child.kill().await;
    }
    refresh_session(session).await;
    if is_active_session_state(&session.info.state) {
        session.info.state = "killed".to_string();
        session.info.reject_reason = Some("killed".to_string());
        session.info.updated_at = Utc::now();
        session.skill_lease = None;
    }
    let info = session.info.clone();
    prune_terminal_sessions(&mut sessions);
    Some(info)
}

async fn refresh_session(session: &mut ManagedSession) {
    let mut terminal_audit = None;
    if let Some(child) = session.child.as_mut() {
        if let Ok(Some(status)) = child.try_wait() {
            session.info.exit_code = status.code();
            session.info.state = if status.success() { "exited" } else { "failed" }.to_string();
            session.skill_lease = None;
            terminal_audit = session.skill_audit.take();
        }
    }
    let stdout = session.stdout.lock().await;
    let stderr = session.stderr.lock().await;
    session.info.stdout_tail = stdout.text();
    session.info.stderr_tail = stderr.text();
    session.info.truncated = stdout.truncated || stderr.truncated;
    session.info.updated_at = Utc::now();
    session.last_activity = Instant::now();
    if let Some(context) = terminal_audit {
        let _ = write_audit(
            &context.config,
            AuditRecord {
                task_id: None,
                session_id: Some(session.info.session_id.clone()),
                time: session.info.updated_at,
                program: session.info.program.clone(),
                args: session.info.args.clone(),
                working_directory: session.info.working_directory.clone(),
                need_confirm: false,
                policy_decision: "inherited".to_string(),
                confirmation_result: None,
                exit_code: session.info.exit_code,
                duration_ms: (session.info.updated_at - session.info.started_at)
                    .num_milliseconds()
                    .max(0) as u128,
                truncated: session.info.truncated,
                request_source: "skills.run".to_string(),
                reject_reason: session.info.reject_reason.clone(),
                skill_id: Some(context.skill_id),
                skill_path: Some(context.skill_path),
                installed_digest: context.installed_digest,
            },
        );
    }
}

fn prune_terminal_sessions(sessions: &mut std::collections::HashMap<String, ManagedSession>) {
    let cutoff = Utc::now() - ChronoDuration::hours(TERMINAL_RETENTION_HOURS);
    let mut terminal = sessions
        .iter()
        .filter(|(_, session)| !is_active_session_state(&session.info.state))
        .map(|(id, session)| (id.clone(), session.info.updated_at))
        .collect::<Vec<_>>();
    terminal.sort_by(|left, right| right.1.cmp(&left.1));
    for (index, (id, updated_at)) in terminal.into_iter().enumerate() {
        if index >= MAX_TERMINAL_SESSIONS || updated_at < cutoff {
            sessions.remove(&id);
        }
    }
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
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
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

    #[tokio::test]
    async fn async_session_is_inspectable_before_process_spawn_finishes() {
        let (state, workspace) = test_state(2).await;
        let info = start_session_async(
            state.clone(),
            "sess-async".to_string(),
            exec_request("true", &workspace),
        )
        .await;
        assert!(matches!(
            info.state.as_str(),
            "starting" | "running" | "waiting_confirmation"
        ));
        let terminal = wait_until_terminal(&state, "sess-async").await;
        assert_eq!(terminal.state, "exited");
    }

    #[tokio::test]
    async fn skill_session_holds_lease_and_writes_provenance_audit() {
        let (state, workspace) = test_state(2).await;
        let package = workspace.join("skills/demo");
        let scripts = package.join("scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(package.join("SKILL.md"), "# Demo").unwrap();
        let script = scripts.join("check.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf done\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        crate::skills::activate(
            &state,
            agentic_gpt_protocol::SkillActivationRequest {
                id: "demo".to_string(),
            },
        )
        .await
        .unwrap();
        let info = start_skill_session_async(
            state.clone(),
            "sess-skill".to_string(),
            exec_request(script.to_string_lossy().as_ref(), &workspace),
            "demo",
            "scripts/check.sh",
        )
        .await;
        assert!(matches!(info.state.as_str(), "starting" | "running"));
        let terminal = wait_until_terminal(&state, "sess-skill").await;
        assert_eq!(terminal.state, "exited");
        let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl")).unwrap();
        assert!(audit.contains("\"requestSource\":\"skills.run\""));
        assert!(audit.contains("\"skillId\":\"demo\""));
        assert!(audit.contains("\"skillPath\":\"scripts/check.sh\""));
    }

    #[tokio::test]
    async fn skill_lease_blocks_new_readers_once_writer_is_waiting() {
        let leases = SkillLeaseManager::new();
        let reader = leases.try_shared("demo").await.unwrap();
        let waiting_writer = tokio::spawn({
            let leases = leases.clone();
            async move {
                leases
                    .acquire_exclusive("demo", tokio::time::Duration::from_millis(100))
                    .await
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(leases.try_shared("demo").await.is_none());
        drop(reader);
        assert!(waiting_writer.await.unwrap().is_ok());
    }
}
