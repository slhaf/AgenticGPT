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
use uuid::Uuid;

use crate::{
    audit::{write_audit, AuditRecord},
    config::Config,
    confirmation, exec,
    policy::{policy_decision_for_profile, PolicyDecision},
    utils::{command_preview, SESSION_TAIL_MAX},
    AppState,
};

pub(crate) fn capacity_rejection(active: usize, requested: usize, limit: usize) -> String {
    format!("max_active_sessions_reached; active={active}; requested={requested}; limit={limit}")
}

fn resolved_session_limit(config: &Config) -> usize {
    config.limits.max_active_sessions.resolve().resolved
}

pub(crate) struct ManagedSession {
    info: SessionInfo,
    child: Option<Child>,
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
    last_activity: Instant,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
    skill_lease: Option<SkillLease>,
    audit: Option<ManagedAuditContext>,
}

pub(crate) type TerminalEventHook = Arc<dyn Fn(&SessionInfo) + Send + Sync>;

pub(crate) struct ManagedSessionOptions {
    pub(crate) request_source: String,
    pub(crate) skill_id: Option<String>,
    pub(crate) skill_path: Option<String>,
    pub(crate) installed_digest: Option<String>,
    pub(crate) terminal_event_hook: Option<TerminalEventHook>,
}

pub(crate) struct ManagedProcessSpec {
    pub(crate) request: ExecRequest,
    pub(crate) working_directory: std::path::PathBuf,
    pub(crate) decision: PolicyDecision,
    pub(crate) confirmation_result: Option<String>,
    pub(crate) request_source: String,
    pub(crate) terminal_event_hook: Option<TerminalEventHook>,
}

impl ManagedSessionOptions {
    pub(crate) fn for_source(request_source: impl Into<String>) -> Self {
        Self {
            request_source: request_source.into(),
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            terminal_event_hook: None,
        }
    }
}

struct ManagedAuditContext {
    config: Config,
    request_source: String,
    need_confirm: bool,
    policy_decision: String,
    confirmation_result: Option<String>,
    skill_id: Option<String>,
    skill_path: Option<String>,
    installed_digest: Option<String>,
    terminal_event_hook: Option<TerminalEventHook>,
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
    let decision = policy_decision_for_profile(
        &config,
        state.runtime.profile,
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
        refresh_sessions(&state, &mut sessions).await;
        sessions
            .values()
            .filter(|session| is_active_session_state(&session.info.state))
            .count()
    };
    let limit = resolved_session_limit(&config);
    if active_session_count >= limit {
        info.state = "failed".to_string();
        info.reject_reason = Some(capacity_rejection(active_session_count, 1, limit));
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
                    audit: None,
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
    let config = state.config.read().await.clone();
    let decision = policy_decision_for_profile(
        &config,
        state.runtime.profile,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    let working_directory =
        match exec::resolve_working_directory(&config, request.working_directory.as_deref()) {
            Ok(directory) => directory,
            Err(reason) => return rejected_session(&request, session_id, reason),
        };
    if let Err(reason) =
        exec::preflight(&config, &working_directory, &request.program, &request.args)
    {
        return rejected_session(&request, session_id, reason);
    }
    if decision == PolicyDecision::Deny {
        return rejected_session(&request, session_id, "policy_denied".to_string());
    }
    start_prepared_session_async(
        state,
        session_id,
        request,
        ManagedSessionOptions::for_source("hub:session.start"),
        Some((working_directory, decision)),
    )
    .await
}

pub(crate) async fn start_managed_session_async(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    options: ManagedSessionOptions,
) -> SessionInfo {
    start_session_async_inner(state, session_id, request, None, options, None, None).await
}

async fn start_prepared_session_async(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    options: ManagedSessionOptions,
    prepared: Option<(std::path::PathBuf, PolicyDecision)>,
) -> SessionInfo {
    start_session_async_inner(state, session_id, request, None, options, prepared, None).await
}

pub(crate) async fn start_prepared_managed_batch(
    state: AppState,
    specs: Vec<ManagedProcessSpec>,
) -> Result<Vec<SessionInfo>, String> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let config = state.config.read().await.clone();
    let limit = resolved_session_limit(&config);
    let requested = specs.len();
    let mut registered = Vec::with_capacity(requested);
    {
        let mut sessions = state.sessions.lock().await;
        refresh_sessions(&state, &mut sessions).await;
        let active = sessions
            .values()
            .filter(|session| is_active_session_state(&session.info.state))
            .count();
        if active.saturating_add(requested) > limit {
            return Err(capacity_rejection(active, requested, limit));
        }

        for spec in specs {
            let started_at = Utc::now();
            let info = SessionInfo {
                agent_id: spec.request.agent_id.clone(),
                session_id: format!("sess_{}", Uuid::new_v4().simple()),
                state: "starting".to_string(),
                program: spec.request.program.clone(),
                args: spec.request.args.clone(),
                working_directory: spec.request.working_directory.clone(),
                command_preview: command_preview(&spec.request.program, &spec.request.args),
                started_at,
                updated_at: started_at,
                exit_code: None,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                truncated: false,
                reject_reason: None,
            };
            let stdout = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
            let stderr = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
            let cancel_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let audit = ManagedAuditContext {
                config: config.clone(),
                request_source: spec.request_source.clone(),
                need_confirm: spec.request.need_confirm,
                policy_decision: format!("{:?}", spec.decision),
                confirmation_result: spec.confirmation_result.clone(),
                skill_id: None,
                skill_path: None,
                installed_digest: None,
                terminal_event_hook: spec.terminal_event_hook.clone(),
            };
            sessions.insert(
                info.session_id.clone(),
                ManagedSession {
                    info: info.clone(),
                    child: None,
                    stdout: stdout.clone(),
                    stderr: stderr.clone(),
                    last_activity: Instant::now(),
                    cancel_requested: cancel_requested.clone(),
                    skill_lease: None,
                    audit: Some(audit),
                },
            );
            registered.push((spec, info, stdout, stderr, cancel_requested));
        }
    }
    let mut infos = Vec::with_capacity(registered.len());
    for (spec, info, stdout, stderr, cancel_requested) in registered {
        let confirmation_result = spec.confirmation_result.clone();
        let monitor_state = state.clone();
        let monitor_session_id = info.session_id.clone();
        tokio::spawn(run_async_session(
            state.clone(),
            info.session_id.clone(),
            spec.request,
            stdout,
            stderr,
            cancel_requested,
            Some((spec.working_directory, spec.decision)),
            confirmation_result,
        ));
        tokio::spawn(async move {
            monitor_session(monitor_state, monitor_session_id).await;
        });
        infos.push(info);
    }
    Ok(infos)
}

async fn start_session_async_inner(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    skill_lease: Option<SkillLease>,
    options: ManagedSessionOptions,
    prepared: Option<(std::path::PathBuf, PolicyDecision)>,
    initial_failure: Option<String>,
) -> SessionInfo {
    let config = state.config.read().await.clone();
    let started_at = Utc::now();
    let info = SessionInfo {
        agent_id: request.agent_id.clone(),
        session_id: session_id.clone(),
        state: if matches!(prepared.as_ref(), Some((_, PolicyDecision::Confirm))) {
            "waiting_confirmation".to_string()
        } else {
            "starting".to_string()
        },
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
    let stdout = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    let stderr = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    let cancel_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let audit = ManagedAuditContext {
        config: config.clone(),
        request_source: options.request_source,
        need_confirm: request.need_confirm,
        policy_decision: prepared
            .as_ref()
            .map(|(_, decision)| format!("{decision:?}"))
            .unwrap_or_else(|| "undetermined".to_string()),
        confirmation_result: None,
        skill_id: options.skill_id,
        skill_path: options.skill_path,
        installed_digest: options.installed_digest,
        terminal_event_hook: options.terminal_event_hook,
    };
    let registered_info = info.clone();
    let capacity_error = {
        let mut sessions = state.sessions.lock().await;
        refresh_sessions(&state, &mut sessions).await;
        let active = sessions
            .values()
            .filter(|session| is_active_session_state(&session.info.state))
            .count();
        sessions.insert(
            session_id.clone(),
            ManagedSession {
                info: info.clone(),
                child: None,
                stdout: stdout.clone(),
                stderr: stderr.clone(),
                last_activity: Instant::now(),
                cancel_requested: cancel_requested.clone(),
                skill_lease,
                audit: Some(audit),
            },
        );
        let limit = resolved_session_limit(&config);
        (active >= limit).then(|| capacity_rejection(active, 1, limit))
    };
    if let Some(reason) = capacity_error {
        finish_pending_session(&state, &session_id, &reason).await;
        return inspect_session(&state, &session_id)
            .await
            .unwrap_or(registered_info);
    }
    if let Some(reason) = initial_failure {
        finish_pending_session(&state, &session_id, &reason).await;
        return inspect_session(&state, &session_id)
            .await
            .unwrap_or(registered_info);
    }
    let monitor_state = state.clone();
    tokio::spawn(async move {
        run_async_session(
            state,
            session_id,
            request,
            stdout,
            stderr,
            cancel_requested,
            prepared,
            None,
        )
        .await;
    });
    let monitor_session_id = info.session_id.clone();
    tokio::spawn(async move {
        monitor_session(monitor_state, monitor_session_id).await;
    });
    registered_info
}

pub(crate) async fn start_skill_session_async(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    skill_id: &str,
    skill_path: &str,
) -> SessionInfo {
    start_skill_session_async_with_hook(state, session_id, request, skill_id, skill_path, None)
        .await
}

pub(crate) async fn start_skill_session_async_with_hook(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    skill_id: &str,
    skill_path: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> SessionInfo {
    start_skill_session_async_with_hook_and_source(
        state,
        session_id,
        request,
        skill_id,
        skill_path,
        "skills.run",
        terminal_event_hook,
    )
    .await
}

pub(crate) async fn start_skill_session_async_with_hook_and_source(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    skill_id: &str,
    skill_path: &str,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> SessionInfo {
    let config = state.config.read().await.clone();
    let lease = state.skill_leases.try_shared(skill_id).await;
    let lease_available = lease.is_some();
    start_session_async_inner(
        state,
        session_id,
        request,
        lease,
        ManagedSessionOptions {
            request_source: request_source.to_string(),
            skill_id: Some(skill_id.to_string()),
            skill_path: Some(skill_path.to_string()),
            installed_digest: crate::skill_installs::package_sha256(&config, skill_id).ok(),
            terminal_event_hook,
        },
        None,
        if !lease_available {
            Some("skill_update_pending".to_string())
        } else {
            None
        },
    )
    .await
}

pub(crate) async fn wait_for_session(
    state: &AppState,
    mut info: SessionInfo,
    wait_seconds: u64,
) -> SessionInfo {
    if wait_seconds == 0 {
        return info;
    }
    let deadline = Instant::now() + std::time::Duration::from_secs(wait_seconds.min(30));
    while is_active_session_state(&info.state) && Instant::now() < deadline {
        sleep(std::time::Duration::from_millis(20)).await;
        if let Some(latest) = inspect_session(state, &info.session_id).await {
            info = latest;
        } else {
            break;
        }
    }
    info
}

fn rejected_session(request: &ExecRequest, session_id: String, reason: String) -> SessionInfo {
    let now = Utc::now();
    SessionInfo {
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
        reject_reason: Some(reason),
    }
}

async fn run_async_session(
    state: AppState,
    session_id: String,
    request: ExecRequest,
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
    prepared: Option<(std::path::PathBuf, PolicyDecision)>,
    prepared_confirmation_result: Option<String>,
) {
    let config = state.config.read().await.clone();
    let (working_directory, decision) = if let Some(prepared) = prepared {
        prepared
    } else {
        let decision = policy_decision_for_profile(
            &config,
            state.runtime.profile,
            &request.program,
            &request.args,
            request.need_confirm,
        );
        set_policy_decision(&state, &session_id, format!("{decision:?}")).await;
        let working_directory =
            match exec::resolve_working_directory(&config, request.working_directory.as_deref()) {
                Ok(directory) => directory,
                Err(reason) => {
                    finish_pending_session(&state, &session_id, &reason).await;
                    return;
                }
            };
        if let Err(reason) =
            exec::preflight(&config, &working_directory, &request.program, &request.args)
        {
            finish_pending_session(&state, &session_id, &reason).await;
            return;
        }
        (working_directory, decision)
    };
    if decision == PolicyDecision::Deny {
        finish_pending_session(&state, &session_id, "policy_denied").await;
        return;
    }
    if decision == PolicyDecision::Confirm {
        set_session_state(&state, &session_id, "waiting_confirmation").await;
        let confirmation = if let Some(confirmation) = prepared_confirmation_result {
            confirmation
        } else {
            confirmation::request_confirmation_cancellable(
                &state,
                &config,
                request.confirm_method.as_deref(),
                &request.program,
                &request.args,
                cancel_requested.clone(),
            )
            .await
        };
        set_confirmation_result(&state, &session_id, confirmation.clone()).await;
        if confirmation != "allow_once" {
            finish_pending_session(&state, &session_id, &confirmation).await;
            return;
        }
    }
    if cancel_requested.load(std::sync::atomic::Ordering::Acquire) {
        finish_pending_session(&state, &session_id, "cancelled").await;
        return;
    }
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
        if cancelled_before_attach || !is_active_session_state(&session.info.state) {
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
    finish_pending_session(&state, &session_id, "cancelled").await;
}

/*
 * The old skill-specific context was intentionally replaced with a generic
 * managed audit context. Keep the construction above close to the skill
 * entrypoint so the skill provenance fields remain explicit.
 */

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

async fn set_policy_decision(state: &AppState, session_id: &str, decision: String) {
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(session_id) {
        if let Some(audit) = session.audit.as_mut() {
            audit.policy_decision = decision;
        }
    }
}

async fn set_confirmation_result(state: &AppState, session_id: &str, result: String) {
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(session_id) {
        if let Some(audit) = session.audit.as_mut() {
            audit.confirmation_result = Some(result);
        }
    }
}

async fn set_session_state(state: &AppState, session_id: &str, state_name: &str) {
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(session_id) {
        if is_active_session_state(&session.info.state) {
            session.info.state = state_name.to_string();
            session.info.updated_at = Utc::now();
        }
    }
}

async fn finish_pending_session(state: &AppState, session_id: &str, reason: &str) {
    let mut sessions = state.sessions.lock().await;
    if let Some(session) = sessions.get_mut(session_id) {
        if is_active_session_state(&session.info.state) {
            session.info.state = if reason == "cancelled" {
                "killed".to_string()
            } else {
                "failed".to_string()
            };
            session.info.reject_reason = Some(reason.to_string());
            session.info.updated_at = Utc::now();
            session.skill_lease = None;
        }
        finalize_session(state, session).await;
    }
    prune_terminal_sessions(&mut sessions);
}

async fn finalize_session(state: &AppState, session: &mut ManagedSession) {
    let Some(context) = session.audit.take() else {
        return;
    };
    let info = session.info.clone();
    let _ = write_audit(
        &context.config,
        AuditRecord {
            task_id: None,
            session_id: Some(info.session_id.clone()),
            time: info.updated_at,
            program: info.program.clone(),
            args: info.args.clone(),
            working_directory: info.working_directory.clone(),
            need_confirm: context.need_confirm,
            policy_decision: context.policy_decision,
            confirmation_result: context.confirmation_result,
            exit_code: info.exit_code,
            duration_ms: (info.updated_at - info.started_at)
                .num_milliseconds()
                .max(0) as u128,
            truncated: info.truncated,
            request_source: context.request_source,
            reject_reason: info.reject_reason.clone(),
            skill_id: context.skill_id,
            skill_path: context.skill_path,
            installed_digest: context.installed_digest,
        },
    );
    crate::hub::report_session(state, info.clone());
    if let Some(hook) = context.terminal_event_hook {
        hook(&info);
    }
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
    refresh_sessions(state, &mut sessions).await;
    prune_terminal_sessions(&mut sessions);
    sessions
        .values()
        .filter(|session| is_active_session_state(&session.info.state))
        .map(|session| session.info.clone())
        .collect()
}

async fn refresh_sessions(
    state: &AppState,
    sessions: &mut std::collections::HashMap<String, ManagedSession>,
) {
    for session in sessions.values_mut() {
        refresh_session(state, session).await;
    }
}

fn is_active_session_state(state: &str) -> bool {
    matches!(state, "starting" | "running" | "waiting_confirmation")
}

pub(crate) async fn inspect_session(state: &AppState, session_id: &str) -> Option<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    refresh_session(state, session).await;
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
    refresh_session(state, session).await;
    if is_active_session_state(&session.info.state) {
        session.info.state = "killed".to_string();
        session.info.reject_reason = Some("killed".to_string());
        session.info.updated_at = Utc::now();
        session.skill_lease = None;
    }
    finalize_session(state, session).await;
    let info = session.info.clone();
    prune_terminal_sessions(&mut sessions);
    Some(info)
}

async fn refresh_session(state: &AppState, session: &mut ManagedSession) {
    if let Some(child) = session.child.as_mut() {
        if let Ok(Some(status)) = child.try_wait() {
            session.info.exit_code = status.code();
            session.info.state = if status.success() { "exited" } else { "failed" }.to_string();
            session.skill_lease = None;
        }
    }
    let (stdout_tail, stderr_tail, truncated) = {
        let stdout = session.stdout.lock().await;
        let stderr = session.stderr.lock().await;
        (
            stdout.text(),
            stderr.text(),
            stdout.truncated || stderr.truncated,
        )
    };
    session.info.stdout_tail = stdout_tail;
    session.info.stderr_tail = stderr_tail;
    session.info.truncated = truncated;
    session.info.updated_at = Utc::now();
    session.last_activity = Instant::now();
    if !is_active_session_state(&session.info.state) {
        finalize_session(state, session).await;
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
        config.limits.max_active_sessions =
            crate::config::MaxActiveSessions::Explicit(max_active_sessions);
        config.confirmation_provider.provider = "none".to_string();

        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(RwLock::new(config)),
            runtime: crate::state::RuntimeModel::hub(crate::state::CapabilityProfile::Normal),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
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
    async fn ordinary_managed_session_writes_one_terminal_audit() {
        let (state, workspace) = test_state(2).await;
        let info = start_session_async(
            state.clone(),
            "sess-audited".to_string(),
            exec_request("true", &workspace),
        )
        .await;
        let terminal = wait_until_terminal(&state, &info.session_id).await;
        assert_eq!(terminal.state, "exited");
        let _ = inspect_session(&state, &info.session_id).await;
        let _ = inspect_session(&state, &info.session_id).await;
        let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl")).unwrap();
        assert_eq!(audit.lines().count(), 1);
        assert!(audit.contains("\"requestSource\":\"hub:session.start\""));
        assert!(audit.contains("\"sessionId\":\"sess-audited\""));
    }

    #[tokio::test]
    async fn managed_preflight_failure_is_retained_and_terminal_hook_is_once() {
        let (state, workspace) = test_state(2).await;
        let events = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hook_events = events.clone();
        let options = ManagedSessionOptions {
            request_source: "tunnel:process.exec".to_string(),
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            terminal_event_hook: Some(Arc::new(move |_| {
                hook_events.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })),
        };
        let request = ExecRequest {
            agent_id: "test-agent".to_string(),
            program: "true".to_string(),
            args: Vec::new(),
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(workspace.join("missing").to_string_lossy().to_string()),
        };
        let info = start_managed_session_async(
            state.clone(),
            "sess-preflight".to_string(),
            request,
            options,
        )
        .await;
        assert_eq!(info.state, "starting");
        let terminal = wait_until_terminal(&state, &info.session_id).await;
        assert_eq!(terminal.state, "failed");
        let _ = inspect_session(&state, &info.session_id).await;
        assert_eq!(events.load(std::sync::atomic::Ordering::SeqCst), 1);
        let audit = std::fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl")).unwrap();
        assert_eq!(audit.lines().count(), 1);
        assert!(audit.contains("\"requestSource\":\"tunnel:process.exec\""));
    }

    #[tokio::test]
    async fn skill_lease_admission_failure_is_retained_and_finalized_once() {
        let (state, workspace) = test_state(2).await;
        let _writer = state
            .skill_leases
            .acquire_exclusive("demo", Duration::from_secs(1))
            .await
            .unwrap();
        let hook_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hook_count_clone = hook_count.clone();
        let info = start_skill_session_async_with_hook(
            state.clone(),
            "sess-skill-pending".to_string(),
            exec_request("true", &workspace),
            "demo",
            "scripts/check.sh",
            Some(Arc::new(move |_| {
                hook_count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })),
        )
        .await;
        assert_eq!(info.state, "failed");
        assert_eq!(info.reject_reason.as_deref(), Some("skill_update_pending"));
        let retained = inspect_session(&state, &info.session_id)
            .await
            .expect("lease failure must remain queryable");
        assert_eq!(retained.state, "failed");
        let _ = inspect_session(&state, &info.session_id).await;
        assert_eq!(
            hook_count.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "terminal hook must run once for retained lease failures"
        );
        let audit = fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl")).unwrap();
        assert_eq!(audit.lines().count(), 1);
        assert!(audit.contains("\"rejectReason\":\"skill_update_pending\""));
    }

    #[tokio::test]
    async fn managed_process_preserves_sixty_four_kib_per_stream_bound() {
        let (state, workspace) = test_state(2).await;
        {
            let mut config = state.config.write().await;
            config.policy.allow.push(crate::config::Rule {
                program: "sh".to_string(),
                args_prefix: Vec::new(),
            });
        }
        let request = ExecRequest {
            agent_id: "test-agent".to_string(),
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "dd if=/dev/zero bs=4096 count=10 2>/dev/null; dd if=/dev/zero bs=4096 count=10 >&2 2>/dev/null".to_string(),
            ],
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(workspace.to_string_lossy().to_string()),
        };
        let info = start_managed_session_async(
            state.clone(),
            "sess-large-output".to_string(),
            request,
            ManagedSessionOptions::for_source("tunnel:process.exec"),
        )
        .await;
        let terminal = wait_until_terminal(&state, &info.session_id).await;
        assert_eq!(terminal.state, "exited");
        assert_eq!(terminal.stdout_tail.len(), 40 * 1024);
        assert_eq!(terminal.stderr_tail.len(), 40 * 1024);
        assert!(!terminal.truncated);

        let oversize = ExecRequest {
            agent_id: "test-agent".to_string(),
            program: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                "dd if=/dev/zero bs=4096 count=18 2>/dev/null; dd if=/dev/zero bs=4096 count=18 >&2 2>/dev/null".to_string(),
            ],
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(workspace.to_string_lossy().to_string()),
        };
        let info = start_managed_session_async(
            state.clone(),
            "sess-oversize-output".to_string(),
            oversize,
            ManagedSessionOptions::for_source("tunnel:process.exec"),
        )
        .await;
        let terminal = wait_until_terminal(&state, &info.session_id).await;
        assert_eq!(terminal.stdout_tail.len(), 64 * 1024);
        assert_eq!(terminal.stderr_tail.len(), 64 * 1024);
        assert!(terminal.truncated);
    }

    #[tokio::test]
    async fn concurrent_async_starts_reserve_active_capacity_atomically() {
        let (state, workspace) = test_state(1).await;
        let request = || ExecRequest {
            agent_id: "test-agent".to_string(),
            program: "sleep".to_string(),
            args: vec!["1".to_string()],
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(workspace.to_string_lossy().to_string()),
        };
        let (first, second) = tokio::join!(
            start_session_async(state.clone(), "sess-cap-1".to_string(), request()),
            start_session_async(state.clone(), "sess-cap-2".to_string(), request()),
        );
        let states = [first.state.as_str(), second.state.as_str()];
        assert_eq!(
            states.iter().filter(|state| **state == "starting").count(),
            1
        );
        assert_eq!(states.iter().filter(|state| **state == "failed").count(), 1);
        let failed = if first.state == "failed" {
            first
        } else {
            second
        };
        let reason = failed.reject_reason.as_deref().unwrap();
        assert!(reason.starts_with("max_active_sessions_reached; "));
        assert!(reason.contains("active=1; requested=1; limit=1"));
    }

    #[tokio::test]
    async fn oversized_batch_rejects_before_session_registration() {
        let (state, workspace) = test_state(1).await;
        let specs = (0..2)
            .map(|_| ManagedProcessSpec {
                request: exec_request("true", &workspace),
                working_directory: workspace.clone(),
                decision: PolicyDecision::Allow,
                confirmation_result: None,
                request_source: "test:batch".to_string(),
                terminal_event_hook: None,
            })
            .collect();

        let error = start_prepared_managed_batch(state.clone(), specs)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "max_active_sessions_reached; active=0; requested=2; limit=1"
        );
        assert!(state.sessions.lock().await.is_empty());
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
