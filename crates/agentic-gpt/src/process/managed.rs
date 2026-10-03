use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Weak,
};

#[cfg(test)]
use agentic_gpt_protocol::DEFAULT_PROCESS_RESPONSE_BYTES;
use agentic_gpt_protocol::{
    normalize_process_group, EventOrigin, EventSource, ProcessBatchExecRequest,
    ProcessBatchResponse, ProcessCancelResponse, ProcessCaptureStatus, ProcessCursor,
    ProcessDetail, ProcessError, ProcessExecRequest, ProcessInfo, ProcessKind, ProcessListItem,
    ProcessListRequest, ProcessListResponse, ProcessMcpResult, ProcessMcpResultStatus,
    ProcessOutputEncoding, ProcessOutputGap, ProcessOutputPage, ProcessOutputSegment,
    ProcessReadRequest, ProcessReadView, ProcessResponse, ProcessState, MAX_PROCESS_RESPONSE_BYTES,
    MIN_PROCESS_RESPONSE_BYTES,
};
use anyhow::Result;
use base64::{
    engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::future::join_all;
use rmcp::{model::RequestId, service::Peer, RoleClient};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant};

use crate::{
    audit::{write_audit, AuditRecord},
    config::Config,
    confirmation, exec,
    policy::{policy_decision_for_profile, PolicyDecision},
    skills::{package_sha256, SkillLease},
    state::AppState,
    utils::command_preview,
};

const PROCESS_OUTPUT_RING_CAPACITY: usize = 64 * 1024;

const TERMINAL_PROCESS_HOT_CACHE_MINUTES: i64 = 5;
const MAX_TERMINAL_PROCESSES: usize = 100;
const MAX_LIST_PROCESSES: usize = 100;
pub(crate) const MAX_MCP_ARGUMENT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_MCP_RESULT_BYTES: usize = 512 * 1024;
const MAX_MCP_RESULT_PREVIEW_BYTES: usize = 8 * 1024;
pub(crate) const MAX_PROCESS_ERROR_BYTES: usize = 8 * 1024;
pub(crate) const MAX_PROCESS_ERROR_CODE_BYTES: usize = 64;
pub(crate) const MAX_PROCESS_RESPONSE_TEXT_BYTES: usize = 512;
pub(crate) const PROCESS_RESPONSE_TRUNCATION_MARKER: &str = "[truncated]";
pub(crate) const MCP_GLOBAL_CONCURRENCY: usize = 8;
pub(crate) const MCP_PER_SERVER_CONCURRENCY: usize = 2;

pub(crate) fn capacity_rejection(active: usize, requested: usize, limit: usize) -> String {
    format!("max_active_processes_reached; active={active}; requested={requested}; limit={limit}")
}

fn resolved_process_limit(config: &Config) -> usize {
    config.limits.max_active_processes.resolve().resolved
}

fn validated_group(group: Option<&str>) -> std::result::Result<Option<String>, String> {
    normalize_process_group(group).map_err(|error| format!("{}: {}", error.code(), error.message()))
}

pub(crate) type TerminalEventHook = Arc<dyn Fn(&ProcessInfo) + Send + Sync>;

pub(crate) struct McpConcurrency {
    global: Arc<Semaphore>,
    per_server: Mutex<std::collections::HashMap<String, Weak<Semaphore>>>,
    queued: AtomicUsize,
    active: Arc<AtomicUsize>,
}

pub(crate) struct McpConcurrencyPermit {
    _global: OwnedSemaphorePermit,
    _server: OwnedSemaphorePermit,
    active: Arc<AtomicUsize>,
}

impl Drop for McpConcurrencyPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Default for McpConcurrency {
    fn default() -> Self {
        Self::new()
    }
}

impl McpConcurrency {
    pub(crate) fn new() -> Self {
        Self {
            global: Arc::new(Semaphore::new(MCP_GLOBAL_CONCURRENCY)),
            per_server: Mutex::new(std::collections::HashMap::new()),
            queued: AtomicUsize::new(0),
            active: Arc::new(AtomicUsize::new(0)),
        }
    }

    async fn server_semaphore(&self, server_id: &str) -> Arc<Semaphore> {
        let mut semaphores = self.per_server.lock().await;
        semaphores.retain(|_, semaphore| semaphore.strong_count() > 0);
        if let Some(semaphore) = semaphores.get(server_id).and_then(Weak::upgrade) {
            return semaphore;
        }
        let semaphore = Arc::new(Semaphore::new(MCP_PER_SERVER_CONCURRENCY));
        semaphores.insert(server_id.to_string(), Arc::downgrade(&semaphore));
        semaphore
    }

    pub(crate) async fn acquire(
        &self,
        server_id: &str,
        cancel_requested: Arc<AtomicBool>,
    ) -> Result<McpConcurrencyPermit, String> {
        let server = self.server_semaphore(server_id).await;
        self.queued.fetch_add(1, Ordering::AcqRel);
        let permits = loop {
            if cancel_requested.load(Ordering::Acquire) {
                break Err("cancelled".to_string());
            }
            match server.clone().try_acquire_owned() {
                Ok(server_permit) => match self.global.clone().try_acquire_owned() {
                    Ok(global_permit) => break Ok((global_permit, server_permit)),
                    Err(tokio::sync::TryAcquireError::NoPermits) => {
                        drop(server_permit);
                    }
                    Err(tokio::sync::TryAcquireError::Closed) => {
                        drop(server_permit);
                        break Err("mcp_concurrency_closed".to_string());
                    }
                },
                Err(tokio::sync::TryAcquireError::NoPermits) => {}
                Err(tokio::sync::TryAcquireError::Closed) => {
                    break Err("mcp_concurrency_closed".to_string());
                }
            }
            tokio::select! {
                _ = sleep(std::time::Duration::from_millis(10)) => {}
                _ = wait_for_atomic_cancel(cancel_requested.clone()) => {
                    break Err("cancelled".to_string());
                }
            }
        };
        self.queued.fetch_sub(1, Ordering::AcqRel);
        let (global, server) = permits?;
        self.active.fetch_add(1, Ordering::AcqRel);
        Ok(McpConcurrencyPermit {
            _global: global,
            _server: server,
            active: self.active.clone(),
        })
    }

    pub(crate) fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    pub(crate) fn queued(&self) -> usize {
        self.queued.load(Ordering::Acquire)
    }
}

async fn wait_for_atomic_cancel(cancel_requested: Arc<AtomicBool>) {
    while !cancel_requested.load(Ordering::Acquire) {
        sleep(std::time::Duration::from_millis(25)).await;
    }
}

pub(crate) struct ManagedProcess {
    info: ProcessInfo,
    detail: ManagedProcessDetail,
    runtime: ProcessRuntime,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
    audit: Option<ManagedAuditContext>,
    history_terminal_snapshot_at: Option<chrono::DateTime<Utc>>,
    event_completion_recorded: bool,
}

#[derive(Default)]
struct ManagedProcessDetail {
    result: Option<serde_json::Value>,
    error: Option<ProcessError>,
    result_available: bool,
    result_bytes: Option<usize>,
    result_sha256: Option<String>,
    result_preview: Option<String>,
}

pub(crate) enum ProcessRuntime {
    Process(ManagedProcessRuntime),
    Mcp(ManagedMcpRuntime),
}

pub(crate) struct ManagedMcpRuntime {
    peer: Option<Peer<RoleClient>>,
    request_id: Option<RequestId>,
    changed: Arc<Notify>,
}

pub(crate) struct ManagedProcessRuntime {
    changed: Arc<Notify>,
    child: Option<Child>,
    process_group_id: Option<i32>,
    exit_status: Option<ProcessExitStatus>,
    cancel_evidence: Option<String>,
    startup_reader: Option<JoinHandle<ShellStartupStatus>>,
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
    stdout_reader: Option<JoinHandle<ReaderOutcome>>,
    stderr_reader: Option<JoinHandle<ReaderOutcome>>,
    skill_lease: Option<SkillLease>,
}

#[derive(Clone, Copy)]
struct ProcessExitStatus {
    code: Option<i32>,
    success: bool,
}

impl From<ExitStatus> for ProcessExitStatus {
    fn from(status: ExitStatus) -> Self {
        Self {
            code: status.code(),
            success: status.success(),
        }
    }
}

enum ShellStartupStatus {
    Ready,
    InitFailed(Option<i32>),
    StartupFailed,
}

pub(crate) struct ProcessOptions {
    pub(crate) request_source: String,
    pub(crate) skill_id: Option<String>,
    pub(crate) skill_path: Option<String>,
    pub(crate) installed_digest: Option<String>,
    pub(crate) terminal_event_hook: Option<TerminalEventHook>,
    pub(crate) event_origin: Option<EventOrigin>,
}

impl ProcessOptions {
    pub(crate) fn for_source(request_source: impl Into<String>) -> Self {
        Self {
            request_source: request_source.into(),
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            terminal_event_hook: None,
            event_origin: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ManagedProcessResponse {
    pub(crate) response: ProcessResponse,
    pub(crate) process: ProcessInfo,
}

#[derive(Debug)]
pub(crate) struct ManagedProcessBatchResponse {
    pub(crate) response: ProcessBatchResponse,
    pub(crate) processes: Vec<ProcessInfo>,
}

pub(crate) struct ManagedProcessSpec {
    pub(crate) request: exec::ExecutionRequest,
    pub(crate) working_directory: PathBuf,
    pub(crate) batch_id: Option<String>,
    pub(crate) batch_index: Option<usize>,
    pub(crate) decision: PolicyDecision,
    pub(crate) confirmation_result: Option<String>,
    pub(crate) request_source: String,
    pub(crate) terminal_event_hook: Option<TerminalEventHook>,
    pub(crate) event_origin: Option<EventOrigin>,
}

pub(crate) struct ManagedMcpSpec {
    pub(crate) agent_id: String,
    pub(crate) group: Option<String>,
    pub(crate) batch_id: Option<String>,
    pub(crate) batch_call_id: Option<String>,
    pub(crate) batch_index: Option<usize>,
    pub(crate) server_id: String,
    pub(crate) tool_name: String,
    pub(crate) request_source: String,
    pub(crate) argument_keys: Vec<String>,
    pub(crate) argument_key_count: usize,
    pub(crate) argument_keys_truncated: bool,
    pub(crate) argument_bytes: usize,
    pub(crate) argument_sha256: String,
    pub(crate) config_revision: String,
    pub(crate) terminal_event_hook: Option<TerminalEventHook>,
    pub(crate) event_origin: Option<EventOrigin>,
}

pub(crate) struct ManagedMcpRegistration {
    pub(crate) info: ProcessInfo,
    pub(crate) cancel_requested: Arc<std::sync::atomic::AtomicBool>,
}

struct ManagedAuditContext {
    config: Arc<Config>,
    request_source: String,
    need_confirm: bool,
    policy_decision: String,
    confirmation_result: Option<String>,
    skill_id: Option<String>,
    skill_path: Option<String>,
    installed_digest: Option<String>,
    batch_id: Option<String>,
    batch_call_id: Option<String>,
    batch_index: Option<usize>,
    mcp_server_id: Option<String>,
    mcp_tool_name: Option<String>,
    argument_keys: Vec<String>,
    argument_key_count: Option<usize>,
    argument_keys_truncated: Option<bool>,
    argument_bytes: Option<usize>,
    argument_sha256: Option<String>,
    config_revision: Option<String>,
    terminal_event_hook: Option<TerminalEventHook>,
}
#[derive(Debug, Clone)]
enum ReaderOutcome {
    Eof,
    Failed(String),
}

#[derive(Debug)]
pub(crate) struct OutputRing {
    data: std::collections::VecDeque<u8>,
    start_offset: u64,
    end_offset: u64,
    capture: RingCapture,
    changed: Arc<Notify>,
}

#[derive(Debug, Default, Clone)]
enum RingCapture {
    #[default]
    NotStarted,
    Capturing,
    Eof,
    Failed(String),
}

impl OutputRing {
    pub(crate) fn new(changed: Arc<Notify>) -> Self {
        Self {
            data: std::collections::VecDeque::with_capacity(PROCESS_OUTPUT_RING_CAPACITY),
            start_offset: 0,
            end_offset: 0,
            capture: RingCapture::NotStarted,
            changed,
        }
    }

    fn mark_started(&mut self) {
        self.capture = RingCapture::Capturing;
        self.changed.notify_waiters();
    }

    fn push(&mut self, bytes: &[u8]) -> bool {
        let Some(end_offset) = self.end_offset.checked_add(bytes.len() as u64) else {
            self.capture = RingCapture::Failed("output_offset_overflow".to_string());
            self.changed.notify_waiters();
            return false;
        };
        self.end_offset = end_offset;
        self.data.extend(bytes.iter().copied());
        while self.data.len() > PROCESS_OUTPUT_RING_CAPACITY {
            self.data.pop_front();
        }
        self.start_offset = self.end_offset - self.data.len() as u64;
        self.changed.notify_waiters();
        true
    }

    fn finish(&mut self, outcome: ReaderOutcome) {
        if self.is_settled() {
            return;
        }
        self.capture = match outcome {
            ReaderOutcome::Eof => RingCapture::Eof,
            ReaderOutcome::Failed(error) => RingCapture::Failed(error),
        };
        self.changed.notify_waiters();
    }

    fn abort(&mut self, reason: &str) {
        self.capture = RingCapture::Failed(reason.to_string());
        self.changed.notify_waiters();
    }
    fn is_eof(&self) -> bool {
        matches!(self.capture, RingCapture::Eof)
    }

    fn is_failed(&self) -> bool {
        matches!(self.capture, RingCapture::Failed(_))
    }

    fn failure(&self) -> Option<&str> {
        match &self.capture {
            RingCapture::Failed(error) => Some(error),
            _ => None,
        }
    }

    fn capture_not_started(&self) -> bool {
        matches!(self.capture, RingCapture::NotStarted)
    }
    fn is_settled(&self) -> bool {
        self.is_eof() || self.is_failed()
    }

    fn snapshot(&self) -> (Vec<u8>, u64, u64) {
        (
            self.data.iter().copied().collect(),
            self.start_offset,
            self.end_offset,
        )
    }
}

pub(crate) async fn start_managed_process(
    state: AppState,
    request: ProcessExecRequest,
    options: ProcessOptions,
) -> ProcessInfo {
    let config = Arc::new(state.config.read().await.clone());
    start_managed_process_inner(state, request.into(), config, None, options, None, None).await
}

#[cfg(test)]
pub(crate) async fn start_process_for_test(
    state: AppState,
    request: ProcessExecRequest,
) -> ProcessInfo {
    start_managed_process(
        state,
        request,
        ProcessOptions::for_source("hub:process.exec"),
    )
    .await
}

pub(crate) async fn start_skill_process_with_hook_and_source(
    state: AppState,
    request: exec::ExecutionRequest,
    skill_id: &str,
    skill_path: &str,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    event_origin: Option<EventOrigin>,
) -> ProcessInfo {
    let config = Arc::new(state.config.read().await.clone());
    let lease = state.skill_leases.try_shared(skill_id).await;
    let lease_available = lease.is_some();
    start_managed_process_inner(
        state,
        request,
        config.clone(),
        lease,
        ProcessOptions {
            request_source: request_source.to_string(),
            skill_id: Some(skill_id.to_string()),
            skill_path: Some(skill_path.to_string()),
            installed_digest: package_sha256(&config, skill_id).ok(),
            terminal_event_hook,
            event_origin,
        },
        None,
        (!lease_available).then(|| (ProcessState::Rejected, "skill_update_pending".to_string())),
    )
    .await
}

pub(crate) async fn register_mcp_process(
    state: &AppState,
    spec: ManagedMcpSpec,
) -> Result<ManagedMcpRegistration, String> {
    let group = validated_group(spec.group.as_deref())?;
    let config = Arc::new(state.config.read().await.clone());
    let process_id = state.new_process_id();
    let now = Utc::now();
    let info = ProcessInfo {
        agent_id: spec.agent_id,
        process_id: process_id.clone(),
        group,
        batch_id: spec.batch_id.clone(),
        batch_call_id: spec.batch_call_id.clone(),
        batch_index: spec.batch_index,
        kind: ProcessKind::Mcp,
        state: ProcessState::WaitingConfirmation,
        created_at: now,
        started_at: None,
        updated_at: now,
        finished_at: None,
        program: None,
        args: Vec::new(),
        working_directory: None,
        command_preview: None,
        exit_code: None,
        reject_reason: None,
        skill_id: None,
        skill_path: None,
        installed_digest: None,
        mcp_server_id: Some(spec.server_id.clone()),
        mcp_tool_name: Some(spec.tool_name.clone()),
        cancel_requested: false,
        cancel_outcome: None,
        termination_evidence: None,
        capture_status: ProcessCaptureStatus::NotApplicable,
        capture_error: None,
    };
    let cancel_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut processes = state.processes.lock().await;
    refresh_processes(state, &mut processes).await;
    let active = processes
        .values()
        .filter(|process| process.info.state.is_active())
        .count();
    let limit = resolved_process_limit(&config);
    if active >= limit {
        return Err(capacity_rejection(active, 1, limit));
    }
    let source = crate::event_notifications::process_source(&process_id);
    crate::event_notifications::register_internal_source(
        state,
        &source,
        &config.events.internal_policy(),
        spec.event_origin.as_ref(),
    )
    .map_err(|error| format!("internal_event_registration_failed: {error}"))?;
    let admission = state
        .process_history
        .insert_admissions_with_event_tracking([&info]);
    if !admission.is_persisted() {
        crate::event_notifications::abort_unadmitted_source(
            state,
            &source,
            spec.event_origin.as_ref(),
        );
        let error = admission
            .error()
            .unwrap_or("history admission persistence failed");
        return Err(format!("history_admission_failed: {error}"));
    }
    processes.insert(
        process_id,
        ManagedProcess {
            info: info.clone(),
            detail: ManagedProcessDetail::default(),
            runtime: ProcessRuntime::Mcp(ManagedMcpRuntime {
                peer: None,
                request_id: None,
                changed: Arc::new(Notify::new()),
            }),
            cancel_requested: cancel_requested.clone(),
            audit: Some(ManagedAuditContext {
                config,
                request_source: spec.request_source,
                need_confirm: true,
                policy_decision: "pending".to_string(),
                confirmation_result: None,
                skill_id: None,
                skill_path: None,
                installed_digest: None,
                batch_id: spec.batch_id,
                batch_call_id: spec.batch_call_id,
                batch_index: spec.batch_index,
                mcp_server_id: Some(spec.server_id),
                mcp_tool_name: Some(spec.tool_name),
                argument_keys: spec.argument_keys,
                argument_key_count: Some(spec.argument_key_count),
                argument_keys_truncated: Some(spec.argument_keys_truncated),
                argument_bytes: Some(spec.argument_bytes),
                argument_sha256: Some(spec.argument_sha256),
                config_revision: Some(spec.config_revision),
                terminal_event_hook: spec.terminal_event_hook,
            }),
            history_terminal_snapshot_at: None,
            event_completion_recorded: false,
        },
    );
    Ok(ManagedMcpRegistration {
        info,
        cancel_requested,
    })
}

pub(crate) async fn register_mcp_batch(
    state: &AppState,
    specs: Vec<ManagedMcpSpec>,
) -> Result<Vec<ManagedMcpRegistration>, String> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    for spec in &specs {
        validated_group(spec.group.as_deref())?;
    }
    let config = Arc::new(state.config.read().await.clone());
    let requested = specs.len();
    let limit = resolved_process_limit(&config);
    let now = Utc::now();
    let mut processes = state.processes.lock().await;
    refresh_processes(state, &mut processes).await;
    let active = processes
        .values()
        .filter(|process| process.info.state.is_active())
        .count();
    if active.saturating_add(requested) > limit {
        return Err(capacity_rejection(active, requested, limit));
    }
    let mut staged: Vec<(ManagedMcpSpec, ProcessInfo, Arc<AtomicBool>)> =
        Vec::with_capacity(requested);
    for spec in specs {
        let group = validated_group(spec.group.as_deref())?;
        let info = ProcessInfo {
            agent_id: spec.agent_id.clone(),
            process_id: state.new_process_id(),
            group,
            batch_id: spec.batch_id.clone(),
            batch_call_id: spec.batch_call_id.clone(),
            batch_index: spec.batch_index,
            kind: ProcessKind::Mcp,
            state: ProcessState::WaitingConfirmation,
            created_at: now,
            started_at: None,
            updated_at: now,
            finished_at: None,
            program: None,
            args: Vec::new(),
            working_directory: None,
            command_preview: None,
            exit_code: None,
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: Some(spec.server_id.clone()),
            mcp_tool_name: Some(spec.tool_name.clone()),
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
            capture_status: ProcessCaptureStatus::NotApplicable,
            capture_error: None,
        };
        let cancel_requested = Arc::new(AtomicBool::new(false));
        staged.push((spec, info, cancel_requested));
    }
    let mut sources: Vec<(EventSource, Option<EventOrigin>)> = Vec::with_capacity(staged.len());
    for (spec, info, _) in &staged {
        let source = crate::event_notifications::process_source(&info.process_id);
        if let Err(error) = crate::event_notifications::register_internal_source(
            state,
            &source,
            &config.events.internal_policy(),
            spec.event_origin.as_ref(),
        ) {
            for (registered_source, origin) in &sources {
                crate::event_notifications::abort_unadmitted_source(
                    state,
                    registered_source,
                    origin.as_ref(),
                );
            }
            return Err(format!("internal_event_registration_failed: {error}"));
        }
        sources.push((source, spec.event_origin.clone()));
    }
    let admission = state
        .process_history
        .insert_admissions_with_event_tracking(staged.iter().map(|(_, info, _)| info));
    if !admission.is_persisted() {
        for (source, origin) in &sources {
            crate::event_notifications::abort_unadmitted_source(state, source, origin.as_ref());
        }
        let error = admission
            .error()
            .unwrap_or("history admission persistence failed");
        return Err(format!("history_admission_failed: {error}"));
    }
    let mut registrations: Vec<ManagedMcpRegistration> = Vec::with_capacity(requested);
    for (spec, info, cancel_requested) in staged {
        let process_id = info.process_id.clone();
        processes.insert(
            process_id,
            ManagedProcess {
                info: info.clone(),
                detail: ManagedProcessDetail::default(),
                runtime: ProcessRuntime::Mcp(ManagedMcpRuntime {
                    peer: None,
                    request_id: None,
                    changed: Arc::new(Notify::new()),
                }),
                cancel_requested: cancel_requested.clone(),
                audit: Some(ManagedAuditContext {
                    config: config.clone(),
                    request_source: spec.request_source,
                    need_confirm: true,
                    policy_decision: "pending".to_string(),
                    confirmation_result: None,
                    skill_id: None,
                    skill_path: None,
                    installed_digest: None,
                    batch_id: spec.batch_id,
                    batch_call_id: spec.batch_call_id,
                    batch_index: spec.batch_index,
                    mcp_server_id: Some(spec.server_id),
                    mcp_tool_name: Some(spec.tool_name),
                    argument_keys: spec.argument_keys,
                    argument_key_count: Some(spec.argument_key_count),
                    argument_keys_truncated: Some(spec.argument_keys_truncated),
                    argument_bytes: Some(spec.argument_bytes),
                    argument_sha256: Some(spec.argument_sha256),
                    config_revision: Some(spec.config_revision),
                    terminal_event_hook: spec.terminal_event_hook,
                }),
                history_terminal_snapshot_at: None,
                event_completion_recorded: false,
            },
        );
        registrations.push(ManagedMcpRegistration {
            info,
            cancel_requested,
        });
    }
    Ok(registrations)
}

pub(crate) async fn set_mcp_preflight_rejection(
    state: &AppState,
    process_id: &str,
) -> Result<(), String> {
    let mut processes = state.processes.lock().await;
    let process = processes
        .get_mut(process_id)
        .ok_or_else(|| missing_process_reason(state, process_id))?;
    if let Some(audit) = process.audit.as_mut() {
        audit.policy_decision = "Rejected".to_string();
        audit.confirmation_result = None;
    }
    Ok(())
}

pub(crate) async fn set_mcp_authorization(
    state: &AppState,
    process_id: &str,
    decision: &str,
) -> Result<(), String> {
    let mut processes = state.processes.lock().await;
    let process = processes
        .get_mut(process_id)
        .ok_or_else(|| missing_process_reason(state, process_id))?;
    let Some(audit) = process.audit.as_mut() else {
        return Ok(());
    };
    audit.policy_decision = if matches!(
        decision,
        "allow_once" | "allow_mcp_server_15m" | "allow_mcp_server_30m" | "temporary_mcp_allow"
    ) {
        "Allow".to_string()
    } else {
        "Confirm".to_string()
    };
    audit.confirmation_result = Some(decision.to_string());
    Ok(())
}

pub(crate) async fn set_mcp_process_state(
    state: &AppState,
    process_id: &str,
    state_name: ProcessState,
) -> Result<(), String> {
    let mut processes = state.processes.lock().await;
    let process = processes
        .get_mut(process_id)
        .ok_or_else(|| missing_process_reason(state, process_id))?;
    if process.info.state.is_active() {
        process.info.state = state_name;
        process.info.updated_at = Utc::now();
        if let ProcessRuntime::Mcp(runtime) = &process.runtime {
            runtime.changed.notify_waiters();
        }
    }
    Ok(())
}

pub(crate) async fn attach_mcp_request(
    state: &AppState,
    process_id: &str,
    peer: Peer<RoleClient>,
    request_id: RequestId,
) -> Result<(), String> {
    let mut processes = state.processes.lock().await;
    let process = processes
        .get_mut(process_id)
        .ok_or_else(|| missing_process_reason(state, process_id))?;
    let ProcessRuntime::Mcp(runtime) = &mut process.runtime else {
        return Err("process_kind_mismatch".to_string());
    };
    if !process.info.state.is_active() {
        return Err("process_not_active".to_string());
    }
    runtime.peer = Some(peer);
    runtime.request_id = Some(request_id);
    let now = Utc::now();
    process.info.state = ProcessState::Running;
    process.info.started_at = Some(now);
    process.info.updated_at = now;
    let outcome = state.process_history.mark_started(&process.info);
    if !outcome.is_persisted() {
        let reason = format!(
            "history_start_failed: {}",
            outcome
                .error()
                .unwrap_or("history start persistence failed")
        );
        process.info.state = ProcessState::Failed;
        process.info.updated_at = Utc::now();
        process.info.finished_at = Some(process.info.updated_at);
        process.info.reject_reason = Some(reason.clone());
        process.info.termination_evidence = Some("history_start_failed".to_string());
        process_change_notifier(process).notify_waiters();
        finalize_process(state, process).await;
        return Err(reason);
    }
    if let ProcessRuntime::Mcp(runtime) = &process.runtime {
        runtime.changed.notify_waiters();
    }
    Ok(())
}

pub(crate) async fn complete_mcp_result(
    state: &AppState,
    process_id: &str,
    value: serde_json::Value,
    downstream_error: bool,
    cancel_outcome: Option<(&str, &str)>,
) -> Result<ProcessDetail, String> {
    let bytes = serde_json::to_vec(&value).map_err(|_| "mcp_result_encode_failed".to_string())?;
    let byte_count = bytes.len();
    let sha256 = format!("sha256:{:x}", Sha256::digest(&bytes));
    let unavailable = byte_count > MAX_MCP_RESULT_BYTES;
    let preview = unavailable.then(|| {
        let text = String::from_utf8_lossy(&bytes);
        utf8_prefix(&text, MAX_MCP_RESULT_PREVIEW_BYTES).to_string()
    });
    let mut processes = state.processes.lock().await;
    let process = processes
        .get_mut(process_id)
        .ok_or_else(|| missing_process_reason(state, process_id))?;
    if process.info.state.is_terminal() {
        return Ok(process_detail(process));
    }
    process.detail.result = (!unavailable).then_some(value);
    process.detail.result_available = !unavailable;
    process.detail.result_bytes = Some(byte_count);
    process.detail.result_sha256 = Some(sha256);
    process.detail.result_preview = preview;
    if downstream_error {
        process.detail.error = Some(ProcessError {
            code: "mcp_tool_error".to_string(),
            message: "Downstream MCP tool returned isError=true".to_string(),
        });
    }
    process.info.capture_status = ProcessCaptureStatus::NotApplicable;
    process.info.capture_error = None;
    let now = Utc::now();
    process.info.state = if downstream_error {
        ProcessState::Failed
    } else {
        ProcessState::Completed
    };
    process.info.updated_at = now;
    process.info.finished_at = Some(now);
    if let Some((outcome, evidence)) = cancel_outcome {
        process.info.cancel_requested = true;
        process.info.cancel_outcome = Some(outcome.to_string());
        process.info.termination_evidence = Some(evidence.to_string());
    } else {
        process.info.termination_evidence = Some("remote_response".to_string());
    }
    process.info.capture_status = ProcessCaptureStatus::NotApplicable;
    process.info.capture_error = None;
    finalize_process(state, process).await;
    if let ProcessRuntime::Mcp(runtime) = &process.runtime {
        runtime.changed.notify_waiters();
    }
    let detail = process_detail(process);
    prune_terminal_processes(state, &mut processes);
    Ok(detail)
}

pub(crate) async fn finish_mcp_error(
    state: &AppState,
    process_id: &str,
    terminal: ProcessState,
    code: impl Into<String>,
    message: impl Into<String>,
    cancel_outcome: Option<&str>,
    evidence: Option<&str>,
) -> Result<ProcessDetail, String> {
    let code = code.into();
    let message = bounded_error_message(message.into());
    let mut processes = state.processes.lock().await;
    let process = processes
        .get_mut(process_id)
        .ok_or_else(|| missing_process_reason(state, process_id))?;
    if process.info.state.is_terminal() {
        return Ok(process_detail(process));
    }
    let now = Utc::now();
    process.info.state = terminal;
    process.info.updated_at = now;
    process.info.finished_at = Some(now);
    process.info.reject_reason = Some(code.clone());
    process.info.capture_status = ProcessCaptureStatus::NotApplicable;
    process.info.capture_error = None;
    process.detail.error = Some(ProcessError { code, message });
    if let Some(outcome) = cancel_outcome {
        process.info.cancel_requested = true;
        process.info.cancel_outcome = Some(outcome.to_string());
    }
    if let Some(evidence) = evidence {
        process.info.termination_evidence = Some(evidence.to_string());
    }
    finalize_process(state, process).await;
    if let ProcessRuntime::Mcp(runtime) = &process.runtime {
        runtime.changed.notify_waiters();
    }
    let detail = process_detail(process);
    prune_terminal_processes(state, &mut processes);
    Ok(detail)
}

pub(crate) async fn mcp_process_response(
    state: &AppState,
    process_id: &str,
    wait_seconds: u64,
    response_budget: usize,
) -> Result<ManagedProcessResponse, String> {
    get_process_read_with_budget(
        state,
        ProcessReadRequest {
            process_id: process_id.to_string(),
            wait_seconds: Some(wait_seconds),
            view: ProcessReadView::Auto,
            cursor: None,
            max_bytes: Some(response_budget),
        },
        response_budget,
    )
    .await
}

fn bounded_error_message(value: String) -> String {
    if value.len() <= MAX_PROCESS_ERROR_BYTES {
        return value;
    }
    const SUFFIX: &str = "...[truncated]";
    let prefix_limit = MAX_PROCESS_ERROR_BYTES.saturating_sub(SUFFIX.len());
    format!("{}{}", utf8_prefix(&value, prefix_limit), SUFFIX)
}

pub(crate) fn bounded_response_text(value: &str) -> String {
    if value.len() <= MAX_PROCESS_RESPONSE_TEXT_BYTES {
        return value.to_string();
    }
    const SUFFIX: &str = "...[truncated]";
    format!(
        "{}{}",
        utf8_prefix(
            value,
            MAX_PROCESS_RESPONSE_TEXT_BYTES.saturating_sub(SUFFIX.len())
        ),
        SUFFIX
    )
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

struct ProcessObservation {
    process: ProcessInfo,
    detail: ProcessDetail,
    output: Option<crate::process_history::ProcessOutputSnapshot>,
}

fn process_recorded_failure_error(process: &ProcessInfo) -> Option<ProcessError> {
    let reason = match process.state {
        ProcessState::Rejected => process
            .reject_reason
            .as_deref()
            .unwrap_or("process_rejected"),
        ProcessState::Failed => process.reject_reason.as_deref()?,
        _ => return None,
    };
    let code = reason
        .split([':', ';'])
        .next()
        .map(str::trim)
        .filter(|code| {
            !code.is_empty()
                && code
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .unwrap_or("process_rejected")
        .chars()
        .take(MAX_PROCESS_ERROR_CODE_BYTES)
        .collect();
    Some(ProcessError {
        code,
        message: bounded_response_text(reason),
    })
}

fn process_mcp_result(detail: ProcessDetail) -> ProcessMcpResult {
    let ProcessDetail {
        process,
        detail_available,
        result,
        result_available,
        result_bytes,
        result_sha256,
        result_preview,
        ..
    } = detail;
    let status = if process.state.is_active() {
        ProcessMcpResultStatus::Pending
    } else if result_bytes.is_some_and(|bytes| bytes > MAX_MCP_RESULT_BYTES) {
        ProcessMcpResultStatus::NotRetained
    } else if !detail_available || !result_available || result.is_none() {
        ProcessMcpResultStatus::Unavailable
    } else {
        ProcessMcpResultStatus::Included
    };
    ProcessMcpResult {
        status,
        bytes: result_bytes,
        sha256: result_sha256,
        value: (status == ProcessMcpResultStatus::Included)
            .then_some(result)
            .flatten(),
        preview: (status != ProcessMcpResultStatus::Included)
            .then_some(result_preview)
            .flatten()
            .map(|value| bounded_response_text(&value)),
    }
}

struct CountingWriter {
    bytes: usize,
}

impl std::io::Write for CountingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialized_size<T: serde::Serialize>(value: &T) -> usize {
    let mut writer = CountingWriter { bytes: 0 };
    if serde_json::to_writer(&mut writer, value).is_ok() {
        writer.bytes
    } else {
        usize::MAX
    }
}

pub(crate) fn serialized_json_size<T: serde::Serialize>(value: &T) -> usize {
    serialized_size(value)
}

fn json_string_content_size(value: &str) -> usize {
    value.chars().fold(0usize, |bytes, character| {
        bytes.saturating_add(match character {
            '"' | '\\' | '\u{0008}' | '\u{0009}' | '\u{000a}' | '\u{000c}' | '\u{000d}' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            _ => character.len_utf8(),
        })
    })
}

fn utf8_prefix_for_json_budget(value: &str, budget: usize) -> &str {
    let mut bytes = 0usize;
    let mut end = 0usize;
    for character in value.chars() {
        let cost = match character {
            '"' | '\\' | '\u{0008}' | '\u{0009}' | '\u{000a}' | '\u{000c}' | '\u{000d}' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            _ => character.len_utf8(),
        };
        if bytes.saturating_add(cost) > budget {
            break;
        }
        bytes += cost;
        end += character.len_utf8();
    }
    &value[..end]
}

fn output_payload_plan(data: &[u8], json_budget: usize) -> (ProcessOutputEncoding, usize, usize) {
    if data.is_empty() || json_budget == 0 {
        return (ProcessOutputEncoding::Utf8, 0, 0);
    }
    match std::str::from_utf8(data) {
        Ok(text) => {
            let prefix = utf8_prefix_for_json_budget(text, json_budget);
            (
                ProcessOutputEncoding::Utf8,
                prefix.len(),
                json_string_content_size(prefix),
            )
        }
        Err(error) if error.valid_up_to() > 0 => {
            let text =
                std::str::from_utf8(&data[..error.valid_up_to()]).expect("valid UTF-8 prefix");
            let prefix = utf8_prefix_for_json_budget(text, json_budget);
            (
                ProcessOutputEncoding::Utf8,
                prefix.len(),
                json_string_content_size(prefix),
            )
        }
        Err(_) => {
            let mut raw_bytes = (json_budget / 4).saturating_mul(3).min(data.len());
            while raw_bytes > 0 && raw_bytes.saturating_add(2) / 3 * 4 > json_budget {
                raw_bytes -= 1;
            }
            let encoded_bytes = raw_bytes.saturating_add(2) / 3 * 4;
            (ProcessOutputEncoding::Base64, raw_bytes, encoded_bytes)
        }
    }
}

fn encode_output_prefix(data: &[u8], encoding: ProcessOutputEncoding, raw_bytes: usize) -> String {
    match encoding {
        ProcessOutputEncoding::Utf8 => std::str::from_utf8(&data[..raw_bytes])
            .expect("planned output prefix is UTF-8")
            .to_string(),
        ProcessOutputEncoding::Base64 => BASE64.encode(&data[..raw_bytes]),
    }
}

fn output_segment_for_json_budget(
    data: &[u8],
    retained_start: u64,
    end_offset: u64,
    requested_offset: u64,
    json_budget: usize,
) -> Result<(ProcessOutputSegment, u64, usize), String> {
    if retained_start > end_offset || end_offset - retained_start != data.len() as u64 {
        return Err("process_output_snapshot_invalid".to_string());
    }
    if requested_offset > end_offset {
        return Err("process_output_cursor_ahead_of_output".to_string());
    }
    let data_start = requested_offset.max(retained_start);
    let gap = (requested_offset < retained_start).then(|| ProcessOutputGap {
        start_offset: requested_offset.to_string(),
        end_offset: retained_start.to_string(),
    });
    let start_index = usize::try_from(data_start - retained_start)
        .map_err(|_| "process_output_snapshot_invalid".to_string())?;
    let available = &data[start_index..];
    let (encoding, raw_bytes, encoded_bytes) = output_payload_plan(available, json_budget);
    let payload = encode_output_prefix(available, encoding, raw_bytes);
    let next_offset = data_start + raw_bytes as u64;
    Ok((
        ProcessOutputSegment {
            data: payload,
            start_offset: data_start.to_string(),
            end_offset: next_offset.to_string(),
            encoding,
            gap,
        },
        next_offset,
        encoded_bytes,
    ))
}

fn encode_output_cursor(cursor: ProcessCursor) -> Result<String, String> {
    serde_json::to_vec(&cursor)
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
        .map_err(|_| "process_output_cursor_encode_failed".to_string())
}

fn output_page_reservation(
    snapshot: &crate::process_history::ProcessOutputSnapshot,
    process_id: &str,
    cursor: &ProcessCursor,
) -> Result<ProcessOutputPage, String> {
    let (mut stdout, _, _) = output_segment_for_json_budget(
        &snapshot.stdout,
        snapshot.stdout_start_offset,
        snapshot.stdout_end_offset,
        cursor.stdout_offset,
        0,
    )?;
    let (mut stderr, _, _) = output_segment_for_json_budget(
        &snapshot.stderr,
        snapshot.stderr_start_offset,
        snapshot.stderr_end_offset,
        cursor.stderr_offset,
        0,
    )?;
    stdout.encoding = ProcessOutputEncoding::Base64;
    stdout.end_offset = u64::MAX.to_string();
    stderr.encoding = ProcessOutputEncoding::Base64;
    stderr.end_offset = u64::MAX.to_string();
    Ok(ProcessOutputPage {
        stdout,
        stderr,
        next_cursor: encode_output_cursor(ProcessCursor {
            version: 1,
            process_id: process_id.to_string(),
            stdout_offset: u64::MAX,
            stderr_offset: u64::MAX,
        })?,
        has_more: false,
        eof: false,
    })
}

fn output_page_for_json_budget(
    snapshot: &crate::process_history::ProcessOutputSnapshot,
    process_id: &str,
    cursor: &ProcessCursor,
    capture_status: ProcessCaptureStatus,
    json_budget: usize,
) -> Result<(ProcessOutputPage, usize), String> {
    let stdout_available =
        snapshot.stdout_end_offset > cursor.stdout_offset.max(snapshot.stdout_start_offset);
    let stderr_available =
        snapshot.stderr_end_offset > cursor.stderr_offset.max(snapshot.stderr_start_offset);
    let (stdout_budget, stderr_budget) = match (stdout_available, stderr_available) {
        (true, true) => (json_budget / 2, json_budget - json_budget / 2),
        (true, false) => (json_budget, 0),
        (false, true) => (0, json_budget),
        (false, false) => (0, 0),
    };
    let (stdout, stdout_next, stdout_used) = output_segment_for_json_budget(
        &snapshot.stdout,
        snapshot.stdout_start_offset,
        snapshot.stdout_end_offset,
        cursor.stdout_offset,
        stdout_budget,
    )?;
    let (stderr, stderr_next, stderr_used) = output_segment_for_json_budget(
        &snapshot.stderr,
        snapshot.stderr_start_offset,
        snapshot.stderr_end_offset,
        cursor.stderr_offset,
        stderr_budget,
    )?;
    let used = stdout_used.saturating_add(stderr_used);
    let has_more =
        stdout_next < snapshot.stdout_end_offset || stderr_next < snapshot.stderr_end_offset;
    let eof = !has_more
        && matches!(
            capture_status,
            ProcessCaptureStatus::Complete | ProcessCaptureStatus::NotApplicable
        );
    Ok((
        ProcessOutputPage {
            stdout,
            stderr,
            next_cursor: encode_output_cursor(ProcessCursor {
                version: 1,
                process_id: process_id.to_string(),
                stdout_offset: stdout_next,
                stderr_offset: stderr_next,
            })?,
            has_more,
            eof,
        },
        used,
    ))
}

fn fit_process_response(
    response: &mut ProcessResponse,
    output: Option<&crate::process_history::ProcessOutputSnapshot>,
    cursor: Option<&ProcessCursor>,
    fallback_result_preview: Option<String>,
    response_budget: usize,
) -> Result<(), String> {
    if response
        .mcp_result
        .as_ref()
        .is_some_and(|result| result.status == ProcessMcpResultStatus::Included)
        && serialized_size(response) <= response_budget
    {
        return Ok(());
    }
    if let Some(result) = response.mcp_result.as_mut() {
        if result.status == ProcessMcpResultStatus::Included {
            result.status = ProcessMcpResultStatus::Deferred;
            result.value = None;
            result.preview = fallback_result_preview.map(|value| bounded_response_text(&value));
        }
    }
    if let Some(snapshot) = output {
        let empty_cursor = ProcessCursor {
            version: 1,
            process_id: response.process_id.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
        };
        response.output = Some(output_page_reservation(
            snapshot,
            &response.process_id,
            cursor.unwrap_or(&empty_cursor),
        )?);
    }
    if let Some(error) = response.error.as_mut() {
        error.message = bounded_response_text(&error.message);
    }
    if let Some(capture_error) = response.capture_error.as_mut() {
        *capture_error = bounded_response_text(capture_error);
    }
    let mut size = serialized_size(response);
    if size > response_budget {
        if let Some(error) = response.error.as_mut() {
            if !error.message.is_empty() {
                error.message = PROCESS_RESPONSE_TRUNCATION_MARKER.to_string();
            }
        }
        response.capture_error = None;
        if let Some(result) = response.mcp_result.as_mut() {
            result.preview = None;
        }
        size = serialized_size(response);
    }
    if size > response_budget {
        return Err("process_response_budget_too_small".to_string());
    }
    if let Some(snapshot) = output {
        let cursor = cursor.cloned().unwrap_or_else(|| ProcessCursor {
            version: 1,
            process_id: response.process_id.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
        });
        let available_data_budget = response_budget.saturating_sub(size);
        let (actual_page, used) = output_page_for_json_budget(
            snapshot,
            &response.process_id,
            &cursor,
            response.capture_status,
            available_data_budget,
        )?;
        let stdout_progress = actual_page
            .stdout
            .end_offset
            .parse::<u64>()
            .ok()
            .is_some_and(|offset| offset > cursor.stdout_offset);
        let stderr_progress = actual_page
            .stderr
            .end_offset
            .parse::<u64>()
            .ok()
            .is_some_and(|offset| offset > cursor.stderr_offset);
        if used > available_data_budget
            || (actual_page.has_more && !stdout_progress && !stderr_progress)
        {
            return Err("process_response_budget_too_small".to_string());
        }
        response.output = Some(actual_page);
    }
    if serialized_size(response) > response_budget {
        return Err("process_response_budget_too_small".to_string());
    }
    Ok(())
}

fn response_error(process: &ProcessInfo, detail: &ProcessDetail) -> Option<ProcessError> {
    detail
        .error
        .clone()
        .or_else(|| process_recorded_failure_error(process))
        .map(|mut error| {
            error.message = bounded_response_text(&error.message);
            error
        })
}

fn process_response_base(
    observation: ProcessObservation,
    view: ProcessReadView,
    wait_elapsed_ms: u64,
) -> (
    ProcessResponse,
    ProcessInfo,
    Option<crate::process_history::ProcessOutputSnapshot>,
    Option<String>,
) {
    let ProcessObservation {
        process,
        detail,
        output,
    } = observation;
    let include_artifacts = view == ProcessReadView::Auto;
    let fallback_result_preview = detail.result_preview.clone();
    let error = response_error(&process, &detail);
    let mcp_result =
        (include_artifacts && process.kind == ProcessKind::Mcp).then(|| process_mcp_result(detail));
    let response = ProcessResponse {
        agent_id: process.agent_id.clone(),
        process_id: process.process_id.clone(),
        kind: process.kind,
        state: process.state,
        capture_status: process.capture_status,
        group: process.group.clone(),
        batch_id: process.batch_id.clone(),
        batch_index: process.batch_index,
        exit_code: process.exit_code,
        wait_elapsed_ms: Some(wait_elapsed_ms),
        error,
        cancel_outcome: process.cancel_outcome.clone(),
        termination_evidence: process.termination_evidence.clone(),
        capture_error: process
            .capture_error
            .clone()
            .map(|value| bounded_response_text(&value)),
        output: None,
        mcp_result,
    };
    let output = (include_artifacts && process.kind != ProcessKind::Mcp)
        .then_some(output)
        .flatten();
    (response, process, output, fallback_result_preview)
}

fn managed_response_from_observation(
    observation: ProcessObservation,
    view: ProcessReadView,
    cursor: Option<ProcessCursor>,
    wait_elapsed_ms: u64,
    response_budget: usize,
) -> Result<ManagedProcessResponse, String> {
    let (mut response, process, output, fallback_result_preview) =
        process_response_base(observation, view, wait_elapsed_ms);
    fit_process_response(
        &mut response,
        output.as_ref(),
        cursor.as_ref(),
        fallback_result_preview,
        response_budget,
    )?;
    Ok(ManagedProcessResponse { response, process })
}

async fn response_after_process_wait(
    state: &AppState,
    process: ProcessInfo,
    wait_seconds: u64,
    started: Instant,
    response_budget: usize,
) -> Result<ManagedProcessResponse, String> {
    let process = wait_for_process(state, process, wait_seconds).await;
    let observation = if process.state.is_terminal()
        && process.termination_evidence.as_deref() == Some("not_started")
    {
        ProcessObservation {
            detail: status_process_detail(&process, None),
            process,
            output: None,
        }
    } else {
        get_process_observation(state, &process.process_id, true).await?
    };
    managed_response_from_observation(
        observation,
        ProcessReadView::Auto,
        None,
        started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        response_budget,
    )
}
pub(crate) async fn start_and_wait_process(
    state: AppState,
    request: ProcessExecRequest,
    options: ProcessOptions,
    response_budget: usize,
) -> Result<ManagedProcessResponse, String> {
    let group = validated_group(request.group.as_deref())?;
    ensure_process_response_fits(
        &request.agent_id,
        group.as_deref(),
        ProcessKind::Command,
        &state.boot_generation,
        response_budget,
    )?;
    let wait_seconds = request.effective_wait_seconds();
    let started = Instant::now();
    let process = start_managed_process(state.clone(), request, options).await;
    response_after_process_wait(&state, process, wait_seconds, started, response_budget).await
}

pub(crate) async fn start_and_wait_skill_process(
    state: AppState,
    request: exec::ExecutionRequest,
    (skill_id, skill_path): (&str, &str),
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    event_origin: Option<EventOrigin>,
    response_budget: usize,
) -> Result<ManagedProcessResponse, String> {
    let group = validated_group(request.group.as_deref())?;
    ensure_process_response_fits(
        &request.agent_id,
        group.as_deref(),
        ProcessKind::Skill,
        &state.boot_generation,
        response_budget,
    )?;
    let wait_seconds = request
        .wait_seconds
        .unwrap_or(ProcessExecRequest::DEFAULT_WAIT_SECONDS)
        .min(ProcessExecRequest::MAX_WAIT_SECONDS);
    let started = Instant::now();
    let process = start_skill_process_with_hook_and_source(
        state.clone(),
        request,
        skill_id,
        skill_path,
        request_source,
        terminal_event_hook,
        event_origin,
    )
    .await;
    response_after_process_wait(&state, process, wait_seconds, started, response_budget).await
}

pub(crate) async fn start_process_batch(
    state: AppState,
    request: ProcessBatchExecRequest,
    request_source: String,
    terminal_event_hook: Option<TerminalEventHook>,
    event_origin: Option<EventOrigin>,
    response_budget: usize,
) -> Result<ManagedProcessBatchResponse, String> {
    ensure_process_response_budget(response_budget)?;
    let wait_seconds = request.effective_wait_seconds();
    let batch_id = format!("batch_{}", uuid::Uuid::new_v4().simple());
    let group = validated_group(request.group.as_deref())?;
    if request.elements.is_empty() {
        let response = ProcessBatchResponse {
            batch_id,
            status: "completed".to_string(),
            processes: Vec::new(),
        };
        if serialized_size(&response) > response_budget {
            return Err("process_batch_response_too_large".to_string());
        }
        return Ok(ManagedProcessBatchResponse {
            response,
            processes: Vec::new(),
        });
    }
    ensure_process_batch_response_fits(
        &batch_id,
        &request.agent_id,
        group.as_deref(),
        &state.boot_generation,
        request.elements.len(),
        response_budget,
    )?;
    let config = Arc::new(state.config.read().await.clone());
    let mut prepared = Vec::with_capacity(request.elements.len());
    for (index, element) in request.elements.into_iter().enumerate() {
        let cwd = element.cwd.clone().or_else(|| request.cwd.clone());
        let decision = crate::policy::shell_policy_decision_for_profile(
            &config,
            state.runtime.profile,
            &element.command,
            request.need_confirm,
        );
        let resolved_working_directory = exec::resolve_working_directory(&config, cwd.as_deref())?;
        if decision == PolicyDecision::Deny {
            return Err(format!(
                "batch_element_rejected; index={index}; reason=policy_denied"
            ));
        }
        preflight_shell_paths(&config, &resolved_working_directory, &element.command)?;
        prepared.push(exec::PreparedBatchElement {
            index,
            command: element.command,
            cwd,
            resolved_working_directory,
            decision,
        });
    }
    let all_confirmation_elements = prepared
        .iter()
        .map(|element| confirmation::BatchConfirmationElement {
            index: element.index,
            command: element.command.clone(),
            cwd: element.cwd.clone(),
        })
        .collect::<Vec<_>>();
    let needs_confirmation = prepared
        .iter()
        .filter(|element| element.decision == PolicyDecision::Confirm)
        .map(|element| confirmation::BatchConfirmationElement {
            index: element.index,
            command: element.command.clone(),
            cwd: element.cwd.clone(),
        })
        .collect::<Vec<_>>();
    let confirmation_result = if needs_confirmation.is_empty() {
        None
    } else {
        let result = confirmation::request_batch_confirmation(
            &state,
            &config,
            request.confirm_method.as_deref(),
            &needs_confirmation,
            &all_confirmation_elements,
        )
        .await;
        if result != "allow_once" {
            return Err(result);
        }
        Some(result)
    };
    let specs = prepared
        .into_iter()
        .map(|element| ManagedProcessSpec {
            request: exec::ExecutionRequest {
                agent_id: request.agent_id.clone(),
                group: group.clone(),
                execution: exec::ExecutionSpec::Shell {
                    command: element.command,
                },
                need_confirm: request.need_confirm,
                confirm_method: request.confirm_method.clone(),
                cwd: element.cwd,
                wait_seconds: request.wait_seconds,
            },
            working_directory: element.resolved_working_directory,
            batch_id: Some(batch_id.clone()),
            batch_index: Some(element.index),
            decision: element.decision,
            confirmation_result: confirmation_result.clone(),
            request_source: request_source.clone(),
            terminal_event_hook: terminal_event_hook.clone(),
            event_origin: event_origin.clone(),
        })
        .collect::<Vec<_>>();
    let processes = start_prepared_managed_batch(state.clone(), config, specs).await?;
    let started = Instant::now();
    let deadline = started + std::time::Duration::from_secs(wait_seconds);
    let processes = join_all(
        processes
            .into_iter()
            .map(|process| wait_for_process_until(&state, process, deadline)),
    )
    .await;
    let mut response_processes = Vec::with_capacity(processes.len());
    let mut snapshots = Vec::with_capacity(processes.len());
    let mut outputs = Vec::with_capacity(processes.len());
    let wait_elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let mut all_terminal = true;
    let mut all_completed = true;
    for process in processes {
        let observation = get_process_observation(&state, &process.process_id, true).await?;
        all_terminal &= observation.process.state.is_terminal();
        all_completed &= observation.process.state == ProcessState::Completed;
        let (response, snapshot, output, _) =
            process_response_base(observation, ProcessReadView::Auto, wait_elapsed_ms);
        response_processes.push(response);
        snapshots.push(snapshot);
        outputs.push(output);
    }
    let status = if !all_terminal {
        "running"
    } else if !all_completed {
        "completed_with_errors"
    } else {
        "completed"
    };
    let response = fit_process_batch_response(
        ProcessBatchResponse {
            batch_id,
            status: status.to_string(),
            processes: response_processes,
        },
        &outputs,
        response_budget,
    )?;
    Ok(ManagedProcessBatchResponse {
        response,
        processes: snapshots,
    })
}

fn fit_process_batch_response(
    mut response: ProcessBatchResponse,
    outputs: &[Option<crate::process_history::ProcessOutputSnapshot>],
    response_budget: usize,
) -> Result<ProcessBatchResponse, String> {
    for (process, output) in response.processes.iter_mut().zip(outputs) {
        if let Some(output) = output {
            let cursor = ProcessCursor {
                version: 1,
                process_id: process.process_id.clone(),
                stdout_offset: 0,
                stderr_offset: 0,
            };
            process.output = Some(output_page_reservation(
                output,
                &process.process_id,
                &cursor,
            )?);
        }
    }
    let mut size = serialized_size(&response);
    if size > response_budget {
        for process in &mut response.processes {
            let previous_size = serialized_size(process);
            if let Some(error) = process.error.as_mut() {
                if !error.message.is_empty() {
                    error.message = PROCESS_RESPONSE_TRUNCATION_MARKER.to_string();
                }
            }
            process.capture_error = None;
            if let Some(result) = process.mcp_result.as_mut() {
                result.preview = None;
            }
            size = size
                .saturating_sub(previous_size)
                .saturating_add(serialized_size(process));
            if size <= response_budget {
                break;
            }
        }
    }
    if size > response_budget {
        return Err("process_batch_response_budget_too_small".to_string());
    }
    let output_count = outputs.iter().filter(|output| output.is_some()).count();
    let mut remaining = response_budget - size;
    let mut remaining_outputs = output_count;
    for (process, output) in response.processes.iter_mut().zip(outputs) {
        let Some(output) = output else {
            continue;
        };
        let share = remaining.checked_div(remaining_outputs).unwrap_or(0);
        let cursor = ProcessCursor {
            version: 1,
            process_id: process.process_id.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
        };
        let (page, used) = output_page_for_json_budget(
            output,
            &process.process_id,
            &cursor,
            process.capture_status,
            share,
        )?;
        process.output = Some(page);
        remaining = remaining.saturating_sub(used);
        remaining_outputs = remaining_outputs.saturating_sub(1);
    }
    if serialized_size(&response) > response_budget {
        return Err("process_batch_response_budget_too_small".to_string());
    }
    Ok(response)
}

fn worst_case_batch_process_response(
    batch_id: &str,
    agent_id: &str,
    group: Option<&str>,
    process_id: &str,
    batch_index: usize,
) -> ProcessResponse {
    let segment = || ProcessOutputSegment {
        data: String::new(),
        start_offset: u64::MAX.to_string(),
        end_offset: u64::MAX.to_string(),
        encoding: ProcessOutputEncoding::Base64,
        gap: Some(ProcessOutputGap {
            start_offset: u64::MAX.to_string(),
            end_offset: u64::MAX.to_string(),
        }),
    };
    ProcessResponse {
        agent_id: agent_id.to_string(),
        process_id: process_id.to_string(),
        kind: ProcessKind::Command,
        state: ProcessState::UnknownAfterRestart,
        capture_status: ProcessCaptureStatus::NotApplicable,
        group: group.map(str::to_string),
        batch_id: Some(batch_id.to_string()),
        batch_index: Some(batch_index),
        exit_code: Some(i32::MIN),
        wait_elapsed_ms: Some(u64::MAX),
        error: Some(ProcessError {
            code: "x".repeat(64),
            message: PROCESS_RESPONSE_TRUNCATION_MARKER.to_string(),
        }),
        cancel_outcome: Some("x".repeat(64)),
        termination_evidence: Some("x".repeat(128)),
        capture_error: None,
        output: Some(ProcessOutputPage {
            stdout: segment(),
            stderr: segment(),
            next_cursor: encode_output_cursor(ProcessCursor {
                version: 1,
                process_id: process_id.to_string(),
                stdout_offset: u64::MAX,
                stderr_offset: u64::MAX,
            })
            .expect("ProcessCursor serialization is infallible"),
            has_more: false,
            eof: false,
        }),
        mcp_result: None,
    }
}

fn ensure_process_batch_response_fits(
    batch_id: &str,
    agent_id: &str,
    group: Option<&str>,
    boot_generation: &str,
    process_count: usize,
    response_budget: usize,
) -> Result<(), String> {
    let process_id = format!("process_{}_{}", boot_generation, "x".repeat(32));
    let worst_case = worst_case_batch_process_response(
        batch_id,
        agent_id,
        group,
        &process_id,
        process_count.saturating_sub(1),
    );
    let empty = ProcessBatchResponse {
        batch_id: batch_id.to_string(),
        status: "completed_with_errors".to_string(),
        processes: Vec::new(),
    };
    let total = serialized_size(&empty)
        .saturating_add(process_count.saturating_mul(serialized_size(&worst_case)))
        .saturating_add(process_count.saturating_sub(1));
    if total > response_budget {
        return Err("process_batch_response_too_large".to_string());
    }
    Ok(())
}

fn ensure_process_response_budget(response_budget: usize) -> Result<(), String> {
    if (MIN_PROCESS_RESPONSE_BYTES..=MAX_PROCESS_RESPONSE_BYTES).contains(&response_budget) {
        Ok(())
    } else {
        Err("process_response_config_invalid".to_string())
    }
}

pub(crate) fn ensure_process_response_fits(
    agent_id: &str,
    group: Option<&str>,
    kind: ProcessKind,
    boot_generation: &str,
    response_budget: usize,
) -> Result<(), String> {
    ensure_process_response_budget(response_budget)?;
    let process_id = format!("process_{}_{}", boot_generation, "x".repeat(32));
    let mut worst_case = worst_case_batch_process_response("", agent_id, group, &process_id, 0);
    worst_case.batch_id = None;
    worst_case.batch_index = None;
    worst_case.kind = kind;
    if kind == ProcessKind::Mcp {
        worst_case.output = None;
        worst_case.mcp_result = Some(ProcessMcpResult {
            status: ProcessMcpResultStatus::NotRetained,
            bytes: Some(usize::MAX),
            sha256: Some(format!("sha256:{}", "x".repeat(64))),
            value: None,
            preview: None,
        });
    }
    if serialized_size(&worst_case) > response_budget {
        return Err("process_response_budget_too_small".to_string());
    }
    Ok(())
}

type RegisteredProcess = (
    ManagedProcessSpec,
    ProcessInfo,
    Arc<Mutex<OutputRing>>,
    Arc<Mutex<OutputRing>>,
    Arc<AtomicBool>,
);

/// `config` is captured by the caller during batch preflight and remains the
/// effective configuration for admission and every queued worker.
pub(crate) async fn start_prepared_managed_batch(
    state: AppState,
    config: Arc<Config>,
    specs: Vec<ManagedProcessSpec>,
) -> Result<Vec<ProcessInfo>, String> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    for spec in &specs {
        validated_group(spec.request.group.as_deref())?;
    }
    let requested = specs.len();
    let limit = resolved_process_limit(&config);
    let batch_concurrency = config.limits.max_concurrent_tasks.max(1).min(requested);
    let batch_slots = Arc::new(Semaphore::new(batch_concurrency));
    let mut registered: Vec<RegisteredProcess> = Vec::with_capacity(requested);
    {
        let mut processes = state.processes.lock().await;
        refresh_processes(&state, &mut processes).await;
        let active = processes
            .values()
            .filter(|process| process.info.state.is_active())
            .count();
        if active.saturating_add(requested) > limit {
            return Err(capacity_rejection(active, requested, limit));
        }
        for spec in specs {
            let process_id = state.new_process_id();
            let now = Utc::now();
            let mut info = process_info(
                &spec.request,
                process_id,
                ProcessKind::Command,
                ProcessState::Queued,
                now,
                None,
            );
            info.batch_id = spec.batch_id.clone();
            info.batch_index = spec.batch_index;
            let runtime = process_runtime(None);
            let cancel_requested = Arc::new(AtomicBool::new(false));
            let stdout = runtime.stdout.clone();
            let stderr = runtime.stderr.clone();
            registered.push((spec, info, stdout, stderr, cancel_requested));
        }
        let mut sources: Vec<(EventSource, Option<EventOrigin>)> =
            Vec::with_capacity(registered.len());
        for (spec, info, _, _, _) in &registered {
            let source = crate::event_notifications::process_source(&info.process_id);
            if let Err(error) = crate::event_notifications::register_internal_source(
                &state,
                &source,
                &config.events.internal_policy(),
                spec.event_origin.as_ref(),
            ) {
                for (registered_source, origin) in &sources {
                    crate::event_notifications::abort_unadmitted_source(
                        &state,
                        registered_source,
                        origin.as_ref(),
                    );
                }
                return Err(format!("internal_event_registration_failed: {error}"));
            }
            sources.push((source, spec.event_origin.clone()));
        }
        let admission = state.process_history.insert_admissions_with_event_tracking(
            registered.iter().map(|(_, info, _, _, _)| info),
        );
        if !admission.is_persisted() {
            for (source, origin) in &sources {
                crate::event_notifications::abort_unadmitted_source(
                    &state,
                    source,
                    origin.as_ref(),
                );
            }
            let error = admission
                .error()
                .unwrap_or("history admission persistence failed");
            return Err(format!("history_admission_failed: {error}"));
        }
        for (spec, info, stdout, stderr, cancel_requested) in &registered {
            let runtime = ManagedProcessRuntime {
                changed: stdout.lock().await.changed.clone(),
                child: None,
                process_group_id: None,
                exit_status: None,
                cancel_evidence: None,
                startup_reader: None,
                stdout: stdout.clone(),
                stderr: stderr.clone(),
                stdout_reader: None,
                stderr_reader: None,
                skill_lease: None,
            };
            let audit = ManagedAuditContext {
                config: config.clone(),
                request_source: spec.request_source.clone(),
                need_confirm: spec.request.need_confirm,
                policy_decision: format!("{:?}", spec.decision),
                confirmation_result: spec.confirmation_result.clone(),
                skill_id: None,
                skill_path: None,
                installed_digest: None,
                batch_id: spec.batch_id.clone(),
                batch_call_id: None,
                batch_index: spec.batch_index,
                mcp_server_id: None,
                mcp_tool_name: None,
                argument_keys: Vec::new(),
                argument_key_count: None,
                argument_keys_truncated: None,
                argument_bytes: None,
                argument_sha256: None,
                config_revision: None,
                terminal_event_hook: spec.terminal_event_hook.clone(),
            };
            processes.insert(
                info.process_id.clone(),
                ManagedProcess {
                    info: info.clone(),
                    detail: ManagedProcessDetail::default(),
                    runtime: ProcessRuntime::Process(runtime),
                    cancel_requested: cancel_requested.clone(),
                    audit: Some(audit),
                    history_terminal_snapshot_at: None,
                    event_completion_recorded: false,
                },
            );
        }
    }
    let mut infos = Vec::with_capacity(registered.len());
    for (spec, info, stdout, stderr, cancel_requested) in registered {
        let runner_state = state.clone();
        let runner_process_id = info.process_id.clone();
        let runner_slots = batch_slots.clone();
        let runner_config = config.clone();
        tokio::spawn(async move {
            let permit = runner_slots
                .acquire_owned()
                .await
                .expect("batch semaphore remains open");
            set_process_state(&runner_state, &runner_process_id, ProcessState::Starting).await;
            run_async_process(
                runner_state.clone(),
                runner_process_id.clone(),
                runner_config,
                spec.request,
                stdout,
                stderr,
                cancel_requested,
                Some((spec.working_directory, spec.decision)),
                spec.confirmation_result,
            )
            .await;
            monitor_process(runner_state, runner_process_id, Some(permit)).await;
        });
        infos.push(info);
    }
    Ok(infos)
}

async fn start_managed_process_inner(
    state: AppState,
    request: exec::ExecutionRequest,
    config: Arc<Config>,
    skill_lease: Option<SkillLease>,
    options: ProcessOptions,
    prepared: Option<(std::path::PathBuf, PolicyDecision)>,
    initial_terminal: Option<(ProcessState, String)>,
) -> ProcessInfo {
    let process_id = state.new_process_id();
    let now = Utc::now();
    let group_error = validated_group(request.group.as_deref()).err();
    let kind = if options.skill_id.is_some() {
        ProcessKind::Skill
    } else {
        ProcessKind::Command
    };
    let state_name = if matches!(prepared.as_ref(), Some((_, PolicyDecision::Confirm))) {
        ProcessState::WaitingConfirmation
    } else {
        ProcessState::Starting
    };
    let info = process_info(
        &request,
        process_id.clone(),
        kind,
        state_name,
        now,
        Some(&options),
    );
    let runtime = process_runtime(skill_lease);
    let stdout = runtime.stdout.clone();
    let stderr = runtime.stderr.clone();
    let cancel_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let event_origin = options.event_origin.clone();
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
        batch_id: None,
        batch_call_id: None,
        batch_index: None,
        mcp_server_id: None,
        mcp_tool_name: None,
        argument_keys: Vec::new(),
        argument_key_count: None,
        argument_keys_truncated: None,
        argument_bytes: None,
        argument_sha256: None,
        config_revision: None,
        terminal_event_hook: options.terminal_event_hook,
    };
    let (capacity_error, history_error) = {
        let mut processes = state.processes.lock().await;
        refresh_processes(&state, &mut processes).await;
        let active = processes
            .values()
            .filter(|process| process.info.state.is_active())
            .count();
        let limit = resolved_process_limit(&config);
        let capacity_error = (active >= limit).then(|| capacity_rejection(active, 1, limit));
        if capacity_error.is_some() {
            (capacity_error, None)
        } else {
            let source = crate::event_notifications::process_source(&process_id);
            let registration = crate::event_notifications::register_internal_source(
                &state,
                &source,
                &config.events.internal_policy(),
                event_origin.as_ref(),
            );
            if let Err(error) = registration {
                (
                    None,
                    Some(format!("internal_event_registration_failed: {error}")),
                )
            } else {
                let admission = state
                    .process_history
                    .insert_admissions_with_event_tracking([&info]);
                if !admission.is_persisted() {
                    crate::event_notifications::abort_unadmitted_source(
                        &state,
                        &source,
                        event_origin.as_ref(),
                    );
                    let error = admission
                        .error()
                        .unwrap_or("history admission persistence failed");
                    (None, Some(format!("history_admission_failed: {error}")))
                } else {
                    processes.insert(
                        process_id.clone(),
                        ManagedProcess {
                            info: info.clone(),
                            detail: ManagedProcessDetail::default(),
                            runtime: ProcessRuntime::Process(runtime),
                            cancel_requested: cancel_requested.clone(),
                            audit: Some(audit),
                            history_terminal_snapshot_at: None,
                            event_completion_recorded: false,
                        },
                    );
                    (None, None)
                }
            }
        }
    };
    if let Some(reason) = capacity_error {
        return terminal_without_admission(info, ProcessState::Rejected, reason);
    }
    if let Some(reason) = history_error {
        return terminal_without_admission(info, ProcessState::Failed, reason);
    }
    if let Some(reason) = group_error {
        finish_process(&state, &process_id, ProcessState::Rejected, &reason).await;
        return get_process_now(&state, &process_id).await.unwrap_or(info);
    }
    if let Some((terminal, reason)) = initial_terminal {
        finish_process(&state, &process_id, terminal, &reason).await;
        return get_process_now(&state, &process_id).await.unwrap_or(info);
    }
    tokio::spawn(run_async_process(
        state.clone(),
        process_id.clone(),
        config,
        request,
        stdout,
        stderr,
        cancel_requested,
        prepared,
        None,
    ));
    tokio::spawn(monitor_process(state, process_id, None));
    info
}

fn terminal_without_admission(
    mut info: ProcessInfo,
    state: ProcessState,
    reason: String,
) -> ProcessInfo {
    let now = Utc::now();
    info.state = state;
    info.updated_at = now;
    info.finished_at = Some(now);
    info.reject_reason = Some(reason);
    info.termination_evidence = Some("not_started".to_string());
    info
}

fn process_info(
    request: &exec::ExecutionRequest,
    process_id: String,
    kind: ProcessKind,
    state: ProcessState,
    now: chrono::DateTime<Utc>,
    options: Option<&ProcessOptions>,
) -> ProcessInfo {
    let (program, args, command_preview) = match &request.execution {
        exec::ExecutionSpec::Shell { command } => (
            Some("/usr/bin/bash".to_string()),
            vec!["-c".to_string(), command.clone()],
            command.clone(),
        ),
        exec::ExecutionSpec::Argv { program, args } => (
            Some(program.clone()),
            args.clone(),
            command_preview(program, args),
        ),
    };
    ProcessInfo {
        agent_id: request.agent_id.clone(),
        process_id,
        group: validated_group(request.group.as_deref()).ok().flatten(),
        batch_id: None,
        batch_call_id: None,
        batch_index: None,
        kind,
        state,
        created_at: now,
        started_at: None,
        updated_at: now,
        finished_at: None,
        program,
        args,
        working_directory: request.cwd.clone(),
        command_preview: Some(command_preview),
        exit_code: None,
        reject_reason: None,
        skill_id: options.and_then(|options| options.skill_id.clone()),
        skill_path: options.and_then(|options| options.skill_path.clone()),
        installed_digest: options.and_then(|options| options.installed_digest.clone()),
        mcp_server_id: None,
        mcp_tool_name: None,
        cancel_requested: false,
        cancel_outcome: None,
        termination_evidence: None,
        capture_status: if kind == ProcessKind::Mcp {
            ProcessCaptureStatus::NotApplicable
        } else {
            ProcessCaptureStatus::NotStarted
        },
        capture_error: None,
    }
}

fn process_runtime(skill_lease: Option<SkillLease>) -> ManagedProcessRuntime {
    let changed = Arc::new(Notify::new());
    ManagedProcessRuntime {
        child: None,
        process_group_id: None,
        exit_status: None,
        cancel_evidence: None,
        startup_reader: None,
        stdout: Arc::new(Mutex::new(OutputRing::new(changed.clone()))),
        stderr: Arc::new(Mutex::new(OutputRing::new(changed.clone()))),
        stdout_reader: None,
        stderr_reader: None,
        skill_lease,
        changed,
    }
}

fn execution_policy_decision(
    config: &Config,
    profile: crate::state::CapabilityProfile,
    execution: &exec::ExecutionSpec,
    need_confirm: bool,
) -> PolicyDecision {
    match execution {
        exec::ExecutionSpec::Shell { command } => {
            crate::policy::shell_policy_decision_for_profile(config, profile, command, need_confirm)
        }
        exec::ExecutionSpec::Argv { program, args } => {
            policy_decision_for_profile(config, profile, program, args, need_confirm)
        }
    }
}

fn preflight_execution(
    config: &Config,
    working_directory: &Path,
    execution: &exec::ExecutionSpec,
) -> std::result::Result<(), String> {
    match execution {
        exec::ExecutionSpec::Shell { command } => {
            preflight_shell_paths(config, working_directory, command)
        }
        exec::ExecutionSpec::Argv { program, args } => {
            exec::preflight(config, working_directory, program, args)
        }
    }
}

fn preflight_shell_paths(
    config: &Config,
    working_directory: &Path,
    command: &str,
) -> std::result::Result<(), String> {
    let extraction = crate::policy::shell_parser::extract_literal_commands(command);
    for invocation in extraction.commands {
        if invocation.complete {
            exec::preflight(
                config,
                working_directory,
                &invocation.program,
                &invocation.args,
            )?;
        }
    }
    Ok(())
}

/// `config` is the admission snapshot; workers must not reload live state here.
#[allow(clippy::too_many_arguments)]
async fn run_async_process(
    state: AppState,
    process_id: String,
    config: Arc<Config>,
    request: exec::ExecutionRequest,
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
    prepared: Option<(PathBuf, PolicyDecision)>,
    prepared_confirmation_result: Option<String>,
) {
    let (working_directory, decision) = if let Some(prepared) = prepared {
        prepared
    } else {
        let decision = execution_policy_decision(
            &config,
            state.runtime.profile,
            &request.execution,
            request.need_confirm,
        );
        set_policy_decision(&state, &process_id, format!("{decision:?}")).await;
        let working_directory =
            match exec::resolve_working_directory(&config, request.cwd.as_deref()) {
                Ok(directory) => directory,
                Err(reason) => {
                    finish_process(&state, &process_id, ProcessState::Rejected, &reason).await;
                    return;
                }
            };
        if let Err(reason) = preflight_execution(&config, &working_directory, &request.execution) {
            finish_process(&state, &process_id, ProcessState::Rejected, &reason).await;
            return;
        }
        (working_directory, decision)
    };
    if decision == PolicyDecision::Deny {
        finish_process(&state, &process_id, ProcessState::Rejected, "policy_denied").await;
        return;
    }
    if decision == PolicyDecision::Confirm {
        set_process_state(&state, &process_id, ProcessState::WaitingConfirmation).await;
        let confirmation = if let Some(confirmation) = prepared_confirmation_result {
            confirmation
        } else {
            match &request.execution {
                exec::ExecutionSpec::Shell { command } => {
                    confirmation::request_shell_confirmation_cancellable(
                        &state,
                        &config,
                        request.confirm_method.as_deref(),
                        command,
                        request.cwd.as_deref(),
                        cancel_requested.clone(),
                    )
                    .await
                }
                exec::ExecutionSpec::Argv { program, args } => {
                    confirmation::request_confirmation_cancellable(
                        &state,
                        &config,
                        request.confirm_method.as_deref(),
                        program,
                        args,
                        cancel_requested.clone(),
                    )
                    .await
                }
            }
        };
        set_confirmation_result(&state, &process_id, confirmation.clone()).await;
        if confirmation != "allow_once" {
            let terminal = if cancel_requested.load(std::sync::atomic::Ordering::Acquire) {
                ProcessState::Cancelled
            } else {
                ProcessState::Rejected
            };
            finish_process(&state, &process_id, terminal, &confirmation).await;
            return;
        }
    }
    if cancel_requested.load(std::sync::atomic::Ordering::Acquire) {
        finish_process(&state, &process_id, ProcessState::Cancelled, "cancelled").await;
        return;
    }
    let mut processes = state.processes.lock().await;
    let Some(process) = processes.get_mut(&process_id) else {
        return;
    };
    if cancel_requested.load(std::sync::atomic::Ordering::Acquire)
        || !process.info.state.is_active()
    {
        drop(processes);
        finish_process(&state, &process_id, ProcessState::Cancelled, "cancelled").await;
        return;
    }
    let now = Utc::now();
    process.info.state = ProcessState::Running;
    process.info.started_at = Some(now);
    process.info.updated_at = now;
    let outcome = state.process_history.mark_started(&process.info);
    if !outcome.is_persisted() {
        let reason = format!(
            "history_start_failed: {}",
            outcome
                .error()
                .unwrap_or("history start persistence failed")
        );
        drop(processes);
        finish_process(&state, &process_id, ProcessState::Failed, &reason).await;
        return;
    }
    process_change_notifier(process).notify_waiters();
    drop(processes);

    let spawned = spawn_process_with_readers(
        &config,
        &working_directory,
        &request.execution,
        stdout,
        stderr,
    )
    .await;
    let spawned = match spawned {
        Ok(spawned) => spawned,
        Err(error) => {
            mark_capture_failure(&state, &process_id, format!("spawn_failed: {error}")).await;
            finish_process(
                &state,
                &process_id,
                ProcessState::Failed,
                &format!("spawn_failed: {error}"),
            )
            .await;
            return;
        }
    };
    let mut spawned = Some(spawned);
    let cancel_now = {
        let mut processes = state.processes.lock().await;
        let Some(process) = processes.get_mut(&process_id) else {
            drop(processes);
            if let Some(mut spawned) = spawned.take() {
                terminate_process_group(&mut spawned.child, spawned.process_group_id).await;
                settle_reader(spawned.stdout_reader, spawned.stdout).await;
                settle_reader(spawned.stderr_reader, spawned.stderr).await;
            }
            return;
        };
        let ProcessRuntime::Process(runtime) = &mut process.runtime else {
            drop(processes);
            if let Some(mut spawned) = spawned.take() {
                terminate_process_group(&mut spawned.child, spawned.process_group_id).await;
                settle_reader(spawned.stdout_reader, spawned.stdout).await;
                settle_reader(spawned.stderr_reader, spawned.stderr).await;
            }
            return;
        };
        let spawned = spawned.take().expect("spawn result is present");
        runtime.child = Some(spawned.child);
        runtime.process_group_id = spawned.process_group_id;
        runtime.startup_reader = spawned.startup_reader;
        runtime.stdout_reader = spawned.stdout_reader;
        runtime.stderr_reader = spawned.stderr_reader;
        process.info.capture_status = ProcessCaptureStatus::Capturing;
        runtime.changed.notify_waiters();
        cancel_requested.load(std::sync::atomic::Ordering::Acquire)
            || !process.info.state.is_active()
    };
    if cancel_now {
        let _ = cancel_command_process(&state, &process_id).await;
    }
}

struct SpawnedProcess {
    child: Child,
    process_group_id: Option<i32>,
    startup_reader: Option<JoinHandle<ShellStartupStatus>>,
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
    stdout_reader: Option<JoinHandle<ReaderOutcome>>,
    stderr_reader: Option<JoinHandle<ReaderOutcome>>,
}

async fn spawn_process_with_readers(
    config: &Config,
    working_directory: &Path,
    execution: &exec::ExecutionSpec,
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
) -> Result<SpawnedProcess> {
    #[cfg(unix)]
    let (startup_stream, startup_writer, startup_fd) =
        if matches!(execution, exec::ExecutionSpec::Shell { .. }) {
            use std::os::unix::io::AsRawFd;
            let (reader, writer) = std::os::unix::net::UnixStream::pair()?;
            reader.set_nonblocking(true)?;
            (
                Some(tokio::net::UnixStream::from_std(reader)?),
                Some(writer),
                Some(writer.as_raw_fd()),
            )
        } else {
            (None, None, None)
        };

    #[cfg(not(unix))]
    let startup_writer: Option<()> = None;
    let mut command = exec::build_command(config, working_directory, execution)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
        if let Some(startup_fd) = startup_fd {
            unsafe {
                command.as_std_mut().pre_exec(move || {
                    if startup_fd != exec::SHELL_STARTUP_FD {
                        if libc::dup2(startup_fd, exec::SHELL_STARTUP_FD) == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                    } else {
                        let flags = libc::fcntl(startup_fd, libc::F_GETFD);
                        if flags == -1
                            || libc::fcntl(startup_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC)
                                == -1
                        {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                    Ok(())
                });
            }
        }
    }

    let mut child = command.spawn()?;
    drop(startup_writer);
    let process_group_id = child
        .id()
        .and_then(|process_id| i32::try_from(process_id).ok());
    #[cfg(unix)]
    let startup_reader = startup_stream.map(|stream| tokio::spawn(read_shell_startup(stream)));
    #[cfg(not(unix))]
    let startup_reader: Option<JoinHandle<ShellStartupStatus>> = None;
    let stdout_reader = if let Some(reader) = child.stdout.take() {
        stdout.lock().await.mark_started();
        Some(tokio::spawn(read_output(reader, stdout.clone())))
    } else {
        stdout
            .lock()
            .await
            .finish(ReaderOutcome::Failed("stdout_pipe_unavailable".to_string()));
        None
    };
    let stderr_reader = if let Some(reader) = child.stderr.take() {
        stderr.lock().await.mark_started();
        Some(tokio::spawn(read_output(reader, stderr.clone())))
    } else {
        stderr
            .lock()
            .await
            .finish(ReaderOutcome::Failed("stderr_pipe_unavailable".to_string()));
        None
    };
    Ok(SpawnedProcess {
        child,
        process_group_id,
        startup_reader,
        stdout,
        stderr,
        stdout_reader,
        stderr_reader,
    })
}

#[cfg(unix)]
async fn read_shell_startup(mut reader: tokio::net::UnixStream) -> ShellStartupStatus {
    let mut line = Vec::with_capacity(16);
    let mut booted = false;
    let mut byte = [0_u8; 1];
    loop {
        match reader.read(&mut byte).await {
            Ok(0) => {
                return if booted {
                    ShellStartupStatus::InitFailed(None)
                } else {
                    ShellStartupStatus::StartupFailed
                };
            }
            Ok(_) if byte[0] == b'\n' => {
                if line == b"B" {
                    booted = true;
                } else if line == b"R" {
                    return ShellStartupStatus::Ready;
                } else if let Some(status) = line.strip_prefix(b"I:") {
                    return ShellStartupStatus::InitFailed(
                        std::str::from_utf8(status)
                            .ok()
                            .and_then(|status| status.parse().ok())
                            .filter(|status| *status > 0)
                            .or(Some(1)),
                    );
                } else if line.starts_with(b"C:") {
                    return ShellStartupStatus::StartupFailed;
                }
                line.clear();
            }
            Ok(_) if line.len() < 64 => line.push(byte[0]),
            Ok(_) => return ShellStartupStatus::StartupFailed,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return ShellStartupStatus::StartupFailed,
        }
    }
}

async fn read_output<R: AsyncRead + Unpin>(
    mut reader: R,
    ring: Arc<Mutex<OutputRing>>,
) -> ReaderOutcome {
    let mut buffer = [0_u8; 4096];
    let outcome = loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break ReaderOutcome::Eof,
            Ok(read) => {
                if !ring.lock().await.push(&buffer[..read]) {
                    break ReaderOutcome::Failed("output_offset_overflow".to_string());
                }
            }
            Err(error) => break ReaderOutcome::Failed(format!("pipe_read_failed: {error}")),
        }
    };
    ring.lock().await.finish(outcome.clone());
    outcome
}

async fn settle_reader(reader: Option<JoinHandle<ReaderOutcome>>, ring: Arc<Mutex<OutputRing>>) {
    let Some(mut reader) = reader else {
        let mut ring = ring.lock().await;
        if matches!(ring.capture, RingCapture::Capturing) {
            ring.abort("reader_handle_missing");
        }
        return;
    };
    let outcome = match tokio::time::timeout(std::time::Duration::from_secs(2), &mut reader).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => ReaderOutcome::Failed(format!("reader_task_failed: {error}")),
        Err(_) => {
            reader.abort();
            let _ = reader.await;
            ReaderOutcome::Failed("output_reader_did_not_converge".to_string())
        }
    };
    ring.lock().await.finish(outcome);
}

struct GroupTermination {
    group_stopped: bool,
    evidence: &'static str,
    exit_status: Option<ProcessExitStatus>,
}

async fn terminate_process_group(
    mut child: Option<&mut Child>,
    process_group_id: Option<i32>,
    known_exit_status: Option<ProcessExitStatus>,
) -> GroupTermination {
    #[cfg(unix)]
    if let Some(process_group_id) = process_group_id {
        if process_group_id > 0 && !process_group_exists(process_group_id) {
            return GroupTermination {
                group_stopped: true,
                evidence: "process_group_already_gone",
                exit_status: known_exit_status,
            };
        }
        let term_sent = send_process_group_signal(process_group_id, libc::SIGTERM).is_ok();
        let mut exit_status = known_exit_status;
        if wait_for_process_group_exit(
            &mut child,
            process_group_id,
            std::time::Duration::from_millis(500),
            &mut exit_status,
        )
        .await
        {
            return GroupTermination {
                group_stopped: true,
                evidence: if term_sent {
                    "process_group_sigterm_observed"
                } else {
                    "process_group_already_gone"
                },
                exit_status,
            };
        }
        let _ = send_process_group_signal(process_group_id, libc::SIGKILL);
        if wait_for_process_group_exit(
            &mut child,
            process_group_id,
            std::time::Duration::from_secs(2),
            &mut exit_status,
        )
        .await
        {
            return GroupTermination {
                group_stopped: true,
                evidence: "process_group_sigkill_observed",
                exit_status,
            };
        }
        return GroupTermination {
            group_stopped: false,
            evidence: "process_group_termination_unverified",
            exit_status,
        };
    }

    #[cfg(unix)]
    let evidence = "process_group_termination_unverified";
    #[cfg(not(unix))]
    let evidence = "local_child_termination_only";
    let mut exit_status = known_exit_status;
    if let Some(child) = child.as_deref_mut() {
        if child.kill().await.is_ok() {
            if let Ok(Some(status)) = child.try_wait() {
                exit_status = Some(status.into());
            }
            return GroupTermination {
                group_stopped: true,
                evidence,
                exit_status,
            };
        }
        if let Ok(Some(status)) = child.try_wait() {
            exit_status = Some(status.into());
            return GroupTermination {
                group_stopped: true,
                evidence,
                exit_status,
            };
        }
    }
    GroupTermination {
        group_stopped: false,
        evidence,
        exit_status,
    }
}

#[cfg(unix)]
async fn wait_for_process_group_exit(
    child: &mut Option<&mut Child>,
    process_group_id: i32,
    timeout: std::time::Duration,
    exit_status: &mut Option<ProcessExitStatus>,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if exit_status.is_none() {
            if let Some(child) = child.as_deref_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    *exit_status = Some(status.into());
                }
            }
        }
        if !process_group_exists(process_group_id) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[cfg(unix)]
fn process_group_exists(process_group_id: i32) -> bool {
    if process_group_id <= 0 {
        return false;
    }
    if unsafe { libc::kill(-process_group_id, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
fn send_process_group_signal(process_group_id: i32, signal: i32) -> std::io::Result<()> {
    if process_group_id <= 0 {
        return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
    }
    if unsafe { libc::kill(-process_group_id, signal) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn process_group_exists(_process_group_id: i32) -> bool {
    false
}

async fn monitor_process(
    state: AppState,
    process_id: String,
    batch_permit: Option<OwnedSemaphorePermit>,
) {
    loop {
        sleep(std::time::Duration::from_millis(50)).await;
        let Some(info) = get_process_now(&state, &process_id).await else {
            return;
        };
        if info.state.is_terminal() {
            break;
        }
    }
    // Batch slots model executing children, not the separate time required to
    // drain inherited stdout/stderr pipes.
    drop(batch_permit);

    let readers = {
        let mut processes = state.processes.lock().await;
        let Some(process) = processes.get_mut(&process_id) else {
            return;
        };
        let ProcessRuntime::Process(runtime) = &mut process.runtime else {
            return;
        };
        (
            runtime.stdout.clone(),
            runtime.stderr.clone(),
            runtime.stdout_reader.take(),
            runtime.stderr_reader.take(),
        )
    };
    settle_reader(readers.2, readers.0).await;
    settle_reader(readers.3, readers.1).await;

    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(&process_id) {
        refresh_process(&state, process).await;
        finalize_process(&state, process).await;
    }
    prune_terminal_processes(&state, &mut processes);
}

async fn set_policy_decision(state: &AppState, process_id: &str, decision: String) {
    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(process_id) {
        if let Some(audit) = process.audit.as_mut() {
            audit.policy_decision = decision;
        }
    }
}

async fn set_confirmation_result(state: &AppState, process_id: &str, result: String) {
    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(process_id) {
        if let Some(audit) = process.audit.as_mut() {
            audit.confirmation_result = Some(result);
        }
    }
}

async fn set_process_state(state: &AppState, process_id: &str, state_name: ProcessState) {
    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(process_id) {
        if process.info.state.is_active() {
            process.info.state = state_name;
            process.info.updated_at = Utc::now();
            process_change_notifier(process).notify_waiters();
        }
    }
}

async fn mark_capture_failure(state: &AppState, process_id: &str, error: String) {
    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(process_id) {
        if let ProcessRuntime::Process(runtime) = &mut process.runtime {
            runtime.stdout.lock().await.abort(&error);
            runtime.stderr.lock().await.abort(&error);
            process.info.capture_status = ProcessCaptureStatus::Incomplete;
            process.info.capture_error = Some(bounded_error_message(error));
        }
    }
}

async fn finish_process(state: &AppState, process_id: &str, terminal: ProcessState, reason: &str) {
    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(process_id) {
        if process.info.state.is_active() {
            let now = Utc::now();
            process.info.state = terminal;
            process.info.reject_reason = (!reason.is_empty()).then(|| reason.to_string());
            process.info.updated_at = now;
            process.info.finished_at = Some(now);
            process.info.cancel_requested = terminal == ProcessState::Cancelled;
            if terminal == ProcessState::Cancelled {
                process.info.cancel_outcome = Some("cancelled".to_string());
                process.info.termination_evidence = Some("local_process".to_string());
            }
            if let ProcessRuntime::Process(runtime) = &mut process.runtime {
                runtime.skill_lease = None;
                if runtime.child.is_none()
                    && runtime.stdout_reader.is_none()
                    && runtime.stderr_reader.is_none()
                {
                    let mut stdout = runtime.stdout.lock().await;
                    let mut stderr = runtime.stderr.lock().await;
                    if stdout.capture_not_started() && stderr.capture_not_started() {
                        stdout.finish(ReaderOutcome::Eof);
                        stderr.finish(ReaderOutcome::Eof);
                    }
                }
            }
            process_change_notifier(process).notify_waiters();
        }
        refresh_capture_status(process).await;
        finalize_process(state, process).await;
    }
    prune_terminal_processes(state, &mut processes);
}

async fn refresh_capture_status(process: &mut ManagedProcess) {
    let ProcessRuntime::Process(runtime) = &process.runtime else {
        process.info.capture_status = ProcessCaptureStatus::NotApplicable;
        process.info.capture_error = None;
        return;
    };
    let stdout = runtime.stdout.lock().await;
    let stderr = runtime.stderr.lock().await;
    let (status, error) = capture_summary(&stdout, &stderr);
    process.info.capture_status = status;
    process.info.capture_error = error.map(bounded_error_message);
}

fn capture_status_summary(stdout: &OutputRing, stderr: &OutputRing) -> ProcessCaptureStatus {
    let settled = stdout.is_settled() && stderr.is_settled();
    if settled && (stdout.is_failed() || stderr.is_failed()) {
        ProcessCaptureStatus::Incomplete
    } else if settled {
        ProcessCaptureStatus::Complete
    } else if stdout.capture_not_started() && stderr.capture_not_started() {
        ProcessCaptureStatus::NotStarted
    } else {
        ProcessCaptureStatus::Capturing
    }
}

fn capture_summary(
    stdout: &OutputRing,
    stderr: &OutputRing,
) -> (ProcessCaptureStatus, Option<String>) {
    let status = capture_status_summary(stdout, stderr);
    let mut errors = Vec::new();
    if let Some(error) = stdout.failure() {
        errors.push(format!("stdout: {error}"));
    }
    if let Some(error) = stderr.failure() {
        errors.push(format!("stderr: {error}"));
    }
    let error = (!errors.is_empty()).then(|| errors.join("; "));
    (status, error)
}

async fn process_output_snapshot(
    process: &ManagedProcess,
) -> crate::process_history::ProcessOutputSnapshot {
    let ProcessRuntime::Process(runtime) = &process.runtime else {
        return crate::process_history::ProcessOutputSnapshot::default();
    };
    let stdout_guard = runtime.stdout.lock().await;
    let stderr_guard = runtime.stderr.lock().await;
    let stdout = stdout_guard.snapshot();
    let stderr = stderr_guard.snapshot();
    crate::process_history::ProcessOutputSnapshot {
        stdout: stdout.0,
        stdout_start_offset: stdout.1,
        stdout_end_offset: stdout.2,
        stderr: stderr.0,
        stderr_start_offset: stderr.1,
        stderr_end_offset: stderr.2,
    }
}

fn persist_and_deliver_process_completion(state: &AppState, process: &ManagedProcess) -> bool {
    let outcome = state.process_history.record_terminal_event(&process.info);
    if !outcome.is_persisted() {
        crate::utils::log_warn(format!(
            "process event owner evidence remains pending; processId={}; error={}",
            process.info.process_id,
            outcome
                .error()
                .unwrap_or("process_event_evidence_persistence_failed")
        ));
        return false;
    }
    if let Err(error) = crate::event_notifications::record_process_completion(state, &process.info)
    {
        crate::utils::log_warn(format!(
            "process event delivery remains pending; processId={}; error={error}",
            process.info.process_id
        ));
        return false;
    }
    true
}

async fn finalize_process(state: &AppState, process: &mut ManagedProcess) {
    if !process.info.state.is_terminal() {
        return;
    }
    if !process.event_completion_recorded {
        process.event_completion_recorded = persist_and_deliver_process_completion(state, process);
    }
    refresh_capture_status(process).await;
    let capture_settled = !matches!(
        process.info.capture_status,
        ProcessCaptureStatus::NotStarted | ProcessCaptureStatus::Capturing
    );
    let persist_history =
        capture_settled && process.history_terminal_snapshot_at != Some(process.info.updated_at);
    if !persist_history && process.audit.is_none() {
        return;
    }

    let output = process_output_snapshot(process).await;
    if persist_history {
        let detail = process_detail(process);
        let outcome = state.process_history.upsert_terminal(&detail, &output);
        if outcome.is_persisted() {
            process.history_terminal_snapshot_at = Some(process.info.updated_at);
        } else {
            // Keep the hot copy until this exact terminal outcome is durable.
            process.history_terminal_snapshot_at = None;
        }
    }

    let Some(context) = process.audit.take() else {
        return;
    };
    let info = process.info.clone();
    let _ = write_audit(
        &context.config,
        AuditRecord {
            task_id: None,
            target_id: Some(info.process_id.clone()),
            batch_id: context.batch_id,
            batch_call_id: context.batch_call_id,
            batch_index: context.batch_index,
            time: info.updated_at,
            program: if info.kind == ProcessKind::Mcp {
                "mcp.callTool".to_string()
            } else {
                info.program.clone().unwrap_or_default()
            },
            args: info.args.clone(),
            working_directory: info.working_directory.clone(),
            need_confirm: context.need_confirm,
            policy_decision: context.policy_decision,
            confirmation_result: context.confirmation_result,
            exit_code: info.exit_code,
            duration_ms: info
                .started_at
                .map(|started_at| (info.updated_at - started_at).num_milliseconds().max(0) as u128)
                .unwrap_or(0),
            truncated: output.stdout_start_offset > 0
                || output.stderr_start_offset > 0
                || (info.kind == ProcessKind::Mcp && !process.detail.result_available),
            request_source: context.request_source,
            reject_reason: info.reject_reason.clone(),
            skill_id: context.skill_id,
            skill_path: context.skill_path,
            installed_digest: context.installed_digest,
            mcp_server_id: context.mcp_server_id,
            mcp_tool_name: context.mcp_tool_name,
            argument_keys: context.argument_keys,
            argument_key_count: context.argument_key_count,
            argument_keys_truncated: context.argument_keys_truncated,
            argument_bytes: context.argument_bytes,
            argument_sha256: context.argument_sha256,
            config_revision: context.config_revision,
            result_bytes: process.detail.result_bytes,
            result_sha256: process.detail.result_sha256.clone(),
            terminal_state: Some(info.state.label().to_string()),
            termination_evidence: info.termination_evidence.clone(),
        },
    );
    crate::hub::report_process(state, info.clone());
    if let Some(hook) = context.terminal_event_hook {
        hook(&info);
    }
}

pub(crate) async fn wait_for_process(
    state: &AppState,
    info: ProcessInfo,
    wait_seconds: u64,
) -> ProcessInfo {
    if wait_seconds == 0 {
        return info;
    }
    let deadline = Instant::now() + std::time::Duration::from_secs(wait_seconds.min(30));
    wait_for_process_until(state, info, deadline).await
}

pub(crate) async fn wait_for_process_until(
    state: &AppState,
    mut info: ProcessInfo,
    deadline: Instant,
) -> ProcessInfo {
    let change_notify = process_change_notify(state, &info.process_id).await;
    loop {
        // Subscribe before refreshing: an exit between the refresh and await must wake us.
        let changed = change_notify.as_ref().map(|notify| notify.notified());
        let Some(latest) = get_process_now(state, &info.process_id).await else {
            if let Ok(Some(record)) = state.process_history.get(&info.process_id) {
                if record.info.state.is_terminal() {
                    info = record.info;
                }
            }
            break;
        };
        info = latest;
        if !info.state.is_active() || Instant::now() >= deadline {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Some(changed) = changed {
            let _ = tokio::time::timeout(remaining, changed).await;
        } else {
            sleep(remaining).await;
        }
    }
    info
}

pub(crate) async fn list_processes_page(
    state: &AppState,
    request: ProcessListRequest,
) -> std::result::Result<crate::process_history::ProcessHistoryPage, String> {
    let cursor = request
        .cursor
        .as_deref()
        .map(crate::process_history::decode_list_cursor)
        .transpose()
        .map_err(|error| error.to_string())?;
    let limit = request.effective_limit();
    let live = {
        let mut processes = state.processes.lock().await;
        refresh_processes(state, &mut processes).await;
        prune_terminal_processes(state, &mut processes);
        processes
            .values()
            .map(|process| process.info.clone())
            .collect::<Vec<_>>()
    };

    let mut persisted = Vec::new();
    let mut history_request = request.clone();
    history_request.limit = Some(ProcessListRequest::MAX_LIMIT);
    let mut history_cursor = request.cursor.clone();
    let mut history_failed = false;
    loop {
        history_request.cursor = history_cursor.clone();
        match state.process_history.list(&history_request) {
            Ok(page) => {
                let next_cursor = page.next_cursor.clone();
                persisted.extend(page.processes);
                let merged = merge_process_infos(&live, &persisted, &request, cursor.as_ref());
                if merged.len() > limit || next_cursor.is_none() {
                    break;
                }
                history_cursor = next_cursor;
            }
            Err(_) => {
                history_failed = true;
                break;
            }
        }
    }
    if history_failed {
        persisted.clear();
    }

    let mut processes = merge_process_infos(&live, &persisted, &request, cursor.as_ref());
    let next_cursor = if processes.len() > limit {
        processes.truncate(limit);
        processes
            .last()
            .map(crate::process_history::encode_list_cursor)
    } else {
        None
    };
    Ok(crate::process_history::ProcessHistoryPage {
        processes,
        next_cursor,
    })
}

pub(crate) async fn get_process_list(
    state: &AppState,
    request: ProcessListRequest,
) -> Result<ProcessListResponse, String> {
    let page = list_processes_page(state, request).await?;
    Ok(ProcessListResponse {
        processes: page
            .processes
            .into_iter()
            .map(|process| ProcessListItem {
                process_id: process.process_id,
                group: process.group,
                kind: process.kind,
                state: process.state,
                created_at: process.created_at,
                started_at: process.started_at,
                finished_at: process.finished_at,
                capture_status: process.capture_status,
            })
            .collect(),
        next_cursor: page.next_cursor,
    })
}
pub(crate) async fn list_processes(
    state: &AppState,
    request: ProcessListRequest,
) -> Vec<ProcessInfo> {
    list_processes_page(state, request)
        .await
        .map(|page| page.processes)
        .unwrap_or_default()
}

fn merge_process_infos(
    live: &[ProcessInfo],
    persisted: &[ProcessInfo],
    request: &ProcessListRequest,
    cursor: Option<&crate::process_history::ProcessHistoryCursor>,
) -> Vec<ProcessInfo> {
    let mut by_id = std::collections::HashMap::new();
    for process in persisted {
        by_id
            .entry(process.process_id.clone())
            .or_insert_with(|| process.clone());
    }
    for process in live {
        by_id.insert(process.process_id.clone(), process.clone());
    }
    let mut processes = by_id
        .into_values()
        .filter(|process| {
            request
                .group
                .as_ref()
                .is_none_or(|group| process.group.as_deref() == Some(group.as_str()))
                && request.kind.is_none_or(|kind| process.kind == kind)
                && request.state.is_none_or(|state| process.state == state)
                && cursor.is_none_or(|cursor| {
                    process.created_at < cursor.created_at
                        || (process.created_at == cursor.created_at
                            && process.process_id < cursor.process_id)
                })
        })
        .collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.process_id.cmp(&left.process_id))
    });
    processes
}

pub(crate) async fn current_processes(state: &AppState) -> Vec<ProcessInfo> {
    list_processes(
        state,
        ProcessListRequest {
            group: None,
            kind: None,
            state: None,
            limit: Some(MAX_LIST_PROCESSES),
            cursor: None,
        },
    )
    .await
    .into_iter()
    .filter(|process| process.state.is_active())
    .collect()
}

pub(crate) async fn get_process(
    state: &AppState,
    process_id: &str,
    wait_seconds: u64,
) -> Result<ProcessInfo, String> {
    if let Some(info) = get_process_now(state, process_id).await {
        return Ok(wait_for_process(state, info, wait_seconds).await);
    }
    match state.process_history.get(process_id) {
        Ok(Some(record)) if record.info.state.is_terminal() => Ok(record.info),
        _ => Err(missing_process_reason(state, process_id)),
    }
}
pub(crate) async fn get_process_detail(
    state: &AppState,
    process_id: &str,
    wait_seconds: u64,
) -> Result<ProcessDetail, String> {
    let info = get_process(state, process_id, wait_seconds).await?;
    {
        let mut processes = state.processes.lock().await;
        if let Some(process) = processes.get_mut(process_id) {
            refresh_process(state, process).await;
            return Ok(process_detail(process));
        }
    }
    match state.process_history.get(process_id) {
        Ok(Some(record)) if record.info.state.is_terminal() => {
            Ok(record.detail.unwrap_or(ProcessDetail {
                process: info,
                detail_available: false,
                result: None,
                error: None,
                result_available: false,
                result_bytes: None,
                result_sha256: None,
                result_preview: None,
            }))
        }
        _ => Err(missing_process_reason(state, process_id)),
    }
}

fn process_detail(process: &ManagedProcess) -> ProcessDetail {
    ProcessDetail {
        process: process.info.clone(),
        detail_available: true,
        result: process.detail.result.clone(),
        error: process.detail.error.clone(),
        result_available: process.detail.result_available,
        result_bytes: process.detail.result_bytes,
        result_sha256: process.detail.result_sha256.clone(),
        result_preview: process.detail.result_preview.clone(),
    }
}

fn status_process_detail(info: &ProcessInfo, error: Option<ProcessError>) -> ProcessDetail {
    ProcessDetail {
        process: info.clone(),
        detail_available: true,
        result: None,
        error,
        result_available: false,
        result_bytes: None,
        result_sha256: None,
        result_preview: None,
    }
}

pub(crate) async fn get_process_read(
    state: &AppState,
    request: ProcessReadRequest,
) -> Result<ManagedProcessResponse, String> {
    let configured_budget = state.config.read().await.limits.process_response_bytes;
    get_process_read_with_budget(state, request, configured_budget).await
}

async fn get_process_read_with_budget(
    state: &AppState,
    request: ProcessReadRequest,
    configured_budget: usize,
) -> Result<ManagedProcessResponse, String> {
    if request.view == ProcessReadView::Status && request.cursor.is_some() {
        return Err("process_read_cursor_with_status_view".to_string());
    }
    if !(MIN_PROCESS_RESPONSE_BYTES..=MAX_PROCESS_RESPONSE_BYTES).contains(&configured_budget) {
        return Err("process_read_config_invalid".to_string());
    }
    let response_budget = request.effective_max_bytes(configured_budget)?;
    let cursor = request
        .cursor
        .as_deref()
        .map(|encoded| decode_output_cursor(encoded, &request.process_id))
        .transpose()?;
    let started = Instant::now();
    let deadline = started + std::time::Duration::from_secs(request.effective_wait_seconds());
    let change_notify = process_change_notify(state, &request.process_id).await;
    loop {
        let changed = change_notify.as_ref().map(|notify| notify.notified());
        let summary = get_process_read_summary(state, &request.process_id).await?;
        validate_read_cursor_summary(&summary, cursor.as_ref())?;
        let ready = match request.view {
            ProcessReadView::Status => summary.state.is_terminal(),
            ProcessReadView::Auto => process_read_auto_ready(&summary, cursor.as_ref()),
        };
        if ready || Instant::now() >= deadline {
            break;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Some(changed) = changed {
            let _ = tokio::time::timeout(remaining, changed).await;
        } else {
            sleep(remaining).await;
        }
    }
    let observation = get_process_observation(
        state,
        &request.process_id,
        request.view == ProcessReadView::Auto,
    )
    .await?;
    validate_read_cursor(&observation, cursor.as_ref())?;
    managed_response_from_observation(
        observation,
        request.view,
        cursor,
        started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        response_budget,
    )
}

fn validate_read_cursor(
    observation: &ProcessObservation,
    cursor: Option<&ProcessCursor>,
) -> Result<(), String> {
    let Some(cursor) = cursor else {
        return Ok(());
    };
    if observation.process.kind == ProcessKind::Mcp {
        return Err("process_read_cursor_not_supported_for_mcp".to_string());
    }
    if let Some(output) = observation.output.as_ref() {
        if cursor.stdout_offset > output.stdout_end_offset
            || cursor.stderr_offset > output.stderr_end_offset
        {
            return Err("process_output_cursor_ahead_of_output".to_string());
        }
    }
    Ok(())
}

fn validate_read_cursor_summary(
    summary: &crate::process_history::ProcessHistoryReadSummary,
    cursor: Option<&ProcessCursor>,
) -> Result<(), String> {
    let Some(cursor) = cursor else {
        return Ok(());
    };
    if summary.kind == ProcessKind::Mcp {
        return Err("process_read_cursor_not_supported_for_mcp".to_string());
    }
    if cursor.stdout_offset > summary.stdout_end_offset
        || cursor.stderr_offset > summary.stderr_end_offset
    {
        return Err("process_output_cursor_ahead_of_output".to_string());
    }
    Ok(())
}

fn process_change_notifier(process: &ManagedProcess) -> &Arc<Notify> {
    match &process.runtime {
        ProcessRuntime::Process(runtime) => &runtime.changed,
        ProcessRuntime::Mcp(runtime) => &runtime.changed,
    }
}

async fn process_change_notify(state: &AppState, process_id: &str) -> Option<Arc<Notify>> {
    let processes = state.processes.lock().await;
    processes
        .get(process_id)
        .map(|process| process_change_notifier(process).clone())
}

fn process_read_auto_ready(
    summary: &crate::process_history::ProcessHistoryReadSummary,
    cursor: Option<&ProcessCursor>,
) -> bool {
    if summary.kind == ProcessKind::Mcp {
        return summary.state.is_terminal();
    }
    let cursor_stdout = cursor.map_or(0, |cursor| cursor.stdout_offset);
    let cursor_stderr = cursor.map_or(0, |cursor| cursor.stderr_offset);
    if cursor_stdout < summary.stdout_end_offset || cursor_stderr < summary.stderr_end_offset {
        return true;
    }
    if matches!(
        summary.capture_status,
        ProcessCaptureStatus::Complete
            | ProcessCaptureStatus::Incomplete
            | ProcessCaptureStatus::NotApplicable
    ) {
        return true;
    }
    summary.state.is_terminal() && summary.capture_status != ProcessCaptureStatus::Capturing
}

async fn get_process_read_summary(
    state: &AppState,
    process_id: &str,
) -> Result<crate::process_history::ProcessHistoryReadSummary, String> {
    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(process_id) {
        refresh_process(state, process).await;
        let summary = match &process.runtime {
            ProcessRuntime::Process(runtime) => {
                let stdout = runtime.stdout.lock().await;
                let stderr = runtime.stderr.lock().await;
                crate::process_history::ProcessHistoryReadSummary {
                    kind: process.info.kind,
                    state: process.info.state,
                    capture_status: capture_status_summary(&stdout, &stderr),
                    stdout_start_offset: stdout.start_offset,
                    stdout_end_offset: stdout.end_offset,
                    stderr_start_offset: stderr.start_offset,
                    stderr_end_offset: stderr.end_offset,
                }
            }
            ProcessRuntime::Mcp(_) => crate::process_history::ProcessHistoryReadSummary {
                kind: process.info.kind,
                state: process.info.state,
                capture_status: process.info.capture_status,
                stdout_start_offset: 0,
                stdout_end_offset: 0,
                stderr_start_offset: 0,
                stderr_end_offset: 0,
            },
        };
        prune_terminal_processes(state, &mut processes);
        return Ok(summary);
    }
    drop(processes);
    match state.process_history.read_summary(process_id) {
        Ok(Some(summary)) if summary.state.is_terminal() => Ok(summary),
        _ => Err(missing_process_reason(state, process_id)),
    }
}

async fn get_process_observation(
    state: &AppState,
    process_id: &str,
    include_artifacts: bool,
) -> Result<ProcessObservation, String> {
    let mut processes = state.processes.lock().await;
    if let Some(process) = processes.get_mut(process_id) {
        refresh_process(state, process).await;
        let mut info = process.info.clone();
        let mut detail = if include_artifacts {
            process_detail(process)
        } else {
            status_process_detail(&process.info, process.detail.error.clone())
        };
        let output = match &process.runtime {
            ProcessRuntime::Process(runtime) => {
                let stdout = runtime.stdout.lock().await;
                let stderr = runtime.stderr.lock().await;
                let (capture_status, capture_error) = capture_summary(&stdout, &stderr);
                info.capture_status = capture_status;
                info.capture_error = capture_error.map(bounded_error_message);
                if include_artifacts {
                    let (stdout, stdout_start_offset, stdout_end_offset) = stdout.snapshot();
                    let (stderr, stderr_start_offset, stderr_end_offset) = stderr.snapshot();
                    Some(crate::process_history::ProcessOutputSnapshot {
                        stdout,
                        stdout_start_offset,
                        stdout_end_offset,
                        stderr,
                        stderr_start_offset,
                        stderr_end_offset,
                    })
                } else {
                    None
                }
            }
            ProcessRuntime::Mcp(_) => None,
        };
        detail.process = info.clone();
        let observation = ProcessObservation {
            process: info,
            detail,
            output,
        };
        prune_terminal_processes(state, &mut processes);
        return Ok(observation);
    }
    drop(processes);
    if include_artifacts {
        return match state.process_history.get(process_id) {
            Ok(Some(record)) if record.info.state.is_terminal() => {
                let info = record.info;
                let mut detail = record.detail.unwrap_or(ProcessDetail {
                    process: info.clone(),
                    detail_available: false,
                    result: None,
                    error: None,
                    result_available: false,
                    result_bytes: None,
                    result_sha256: None,
                    result_preview: None,
                });
                detail.process = info.clone();
                let output = (info.kind != ProcessKind::Mcp).then_some(record.output);
                Ok(ProcessObservation {
                    process: info,
                    detail,
                    output,
                })
            }
            _ => Err(missing_process_reason(state, process_id)),
        };
    }
    match state.process_history.status_record(process_id) {
        Ok(Some(record)) if record.summary.state.is_terminal() => {
            let mut info = record.info;
            info.state = record.summary.state;
            info.capture_status = record.summary.capture_status;
            let detail = status_process_detail(&info, record.error);
            Ok(ProcessObservation {
                process: info,
                detail,
                output: None,
            })
        }
        _ => Err(missing_process_reason(state, process_id)),
    }
}
fn decode_output_cursor(encoded: &str, process_id: &str) -> Result<ProcessCursor, String> {
    if encoded.len() > 2048 {
        return Err("invalid_process_output_cursor".to_string());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "invalid_process_output_cursor".to_string())?;
    let cursor: ProcessCursor =
        serde_json::from_slice(&bytes).map_err(|_| "invalid_process_output_cursor".to_string())?;
    if cursor.version != 1 || cursor.process_id != process_id {
        return Err("invalid_process_output_cursor".to_string());
    }
    Ok(cursor)
}

async fn get_process_now(state: &AppState, process_id: &str) -> Option<ProcessInfo> {
    let mut processes = state.processes.lock().await;
    let process = processes.get_mut(process_id)?;
    refresh_process(state, process).await;
    let info = process.info.clone();
    prune_terminal_processes(state, &mut processes);
    Some(info)
}

pub(crate) async fn cancel_process(
    state: &AppState,
    process_id: &str,
) -> Result<ProcessCancelResponse, String> {
    let detail = cancel_process_detail(state, process_id).await?;
    let process = detail.process;
    Ok(ProcessCancelResponse {
        process_id: process.process_id,
        state: process.state,
        cancel_outcome: process.cancel_outcome.unwrap_or_else(|| {
            if process.state.is_terminal() {
                "already_terminal".to_string()
            } else {
                "cancel_requested".to_string()
            }
        }),
        termination_evidence: process
            .termination_evidence
            .unwrap_or_else(|| "process_state".to_string()),
        error: detail.error,
    })
}

async fn cancel_process_detail(
    state: &AppState,
    process_id: &str,
) -> Result<ProcessDetail, String> {
    let kind = {
        let mut processes = state.processes.lock().await;
        let Some(process) = processes.get_mut(process_id) else {
            return Err(missing_process_reason(state, process_id));
        };
        refresh_process(state, process).await;
        process.info.kind
    };
    match kind {
        ProcessKind::Command | ProcessKind::Skill => {
            cancel_command_process(state, process_id).await
        }
        ProcessKind::Mcp => cancel_mcp_process(state, process_id).await,
    }
}

async fn cancel_command_process(
    state: &AppState,
    process_id: &str,
) -> Result<ProcessDetail, String> {
    let (mut child, process_group_id, exit_status) = {
        let mut processes = state.processes.lock().await;
        let Some(process) = processes.get_mut(process_id) else {
            return Err(missing_process_reason(state, process_id));
        };
        refresh_process(state, process).await;
        let ProcessRuntime::Process(runtime) = &mut process.runtime else {
            return Err("process_kind_mismatch".to_string());
        };
        let group_alive = runtime.process_group_id.is_some_and(process_group_exists);
        if process.info.state.is_terminal() && !group_alive {
            let detail = process_detail(process);
            prune_terminal_processes(state, &mut processes);
            return Ok(detail);
        }
        process
            .cancel_requested
            .store(true, std::sync::atomic::Ordering::Release);
        process.info.cancel_requested = true;
        if !process.info.state.is_terminal() {
            process.info.state = ProcessState::CancelRequested;
            process.info.updated_at = Utc::now();
        }
        runtime.changed.notify_waiters();
        let child = runtime.child.take();
        if child.is_none() && runtime.process_group_id.is_none() {
            process.info.cancel_outcome = Some("cancel_requested".to_string());
            process.info.termination_evidence =
                Some("cancel_flag_before_process_start".to_string());
            runtime.skill_lease = None;
            let detail = process_detail(process);
            prune_terminal_processes(state, &mut processes);
            return Ok(detail);
        }
        (child, runtime.process_group_id, runtime.exit_status)
    };

    let termination = terminate_process_group(child.as_mut(), process_group_id, exit_status).await;
    let group_stopped = termination.group_stopped;
    let mut processes = state.processes.lock().await;
    let Some(process) = processes.get_mut(process_id) else {
        return Err("process_not_found".to_string());
    };
    let ProcessRuntime::Process(runtime) = &mut process.runtime else {
        return Err("process_kind_changed".to_string());
    };
    runtime.exit_status = termination.exit_status.or(runtime.exit_status);
    if group_stopped {
        runtime.child = None;
        runtime.process_group_id = None;
        runtime.cancel_evidence = Some(termination.evidence.to_string());
        process.info.exit_code = runtime.exit_status.and_then(|status| status.code);
        if process.info.state.is_terminal() {
            process.info.cancel_requested = true;
            process.info.cancel_outcome = Some("cancelled".to_string());
            process.info.termination_evidence = Some(termination.evidence.to_string());
            process.info.updated_at = Utc::now();
        } else {
            mark_cancelled(&mut process.info, "cancelled", termination.evidence);
        }
        runtime.skill_lease = None;
        runtime.changed.notify_waiters();
        finalize_process(state, process).await;
    } else {
        runtime.child = child;
        if !process.info.state.is_terminal() {
            process.info.state = ProcessState::CancelRequested;
        }
        process.info.updated_at = Utc::now();
        process.info.cancel_outcome = Some("cancel_failed".to_string());
        process.info.termination_evidence = Some(termination.evidence.to_string());
        process.detail.error = Some(ProcessError {
            code: "process_group_termination_unverified".to_string(),
            message: "Process-group termination could not be verified after SIGTERM and SIGKILL"
                .to_string(),
        });
        runtime.changed.notify_waiters();
        if process.info.state.is_terminal() {
            finalize_process(state, process).await;
        }
    }
    let detail = process_detail(process);
    prune_terminal_processes(state, &mut processes);
    drop(processes);
    if group_stopped {
        return wait_for_process_capture(state, process_id).await;
    }
    Ok(detail)
}

async fn wait_for_process_capture(
    state: &AppState,
    process_id: &str,
) -> Result<ProcessDetail, String> {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let (detail, settled) = {
            let mut processes = state.processes.lock().await;
            let Some(process) = processes.get_mut(process_id) else {
                return Err(missing_process_reason(state, process_id));
            };
            refresh_process(state, process).await;
            (
                process_detail(process),
                process.info.capture_status != ProcessCaptureStatus::Capturing,
            )
        };
        if settled || Instant::now() >= deadline {
            return Ok(detail);
        }
        sleep(std::time::Duration::from_millis(10)).await;
    }
}

async fn cancel_mcp_process(state: &AppState, process_id: &str) -> Result<ProcessDetail, String> {
    let request = {
        let mut processes = state.processes.lock().await;
        let Some(process) = processes.get_mut(process_id) else {
            return Err(missing_process_reason(state, process_id));
        };
        if process.info.state.is_terminal() {
            let detail = process_detail(process);
            prune_terminal_processes(state, &mut processes);
            return Ok(detail);
        }
        process
            .cancel_requested
            .store(true, std::sync::atomic::Ordering::Release);
        process.info.cancel_requested = true;
        let ProcessRuntime::Mcp(runtime) = &process.runtime else {
            return Err("process_kind_mismatch".to_string());
        };
        let request = runtime.peer.clone().zip(runtime.request_id.clone());
        if request.is_none() {
            let now = Utc::now();
            process.info.state = ProcessState::Cancelled;
            process.info.updated_at = now;
            process.info.finished_at = Some(now);
            process.info.reject_reason = Some("mcp_cancelled".to_string());
            process.info.cancel_outcome = Some("cancelled_before_request".to_string());
            process.info.termination_evidence =
                Some("local_cancel_before_downstream_request".to_string());
            process.detail.error = Some(ProcessError {
                code: "mcp_cancelled".to_string(),
                message: "MCP Process was cancelled before the downstream request started"
                    .to_string(),
            });
            process_change_notifier(process).notify_waiters();
            finalize_process(state, process).await;
            let detail = process_detail(process);
            prune_terminal_processes(state, &mut processes);
            return Ok(detail);
        }
        process.info.state = ProcessState::CancelRequested;
        process.info.updated_at = Utc::now();
        process_change_notifier(process).notify_waiters();
        request
    };
    let Some((peer, request_id)) = request else {
        return Err("process_cancel_internal".to_string());
    };
    let notification = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        peer.notify_cancelled(rmcp::model::CancelledNotificationParam {
            request_id,
            reason: Some("Cancelled by Agentic process.cancel".to_string()),
        }),
    )
    .await;
    let mut processes = state.processes.lock().await;
    let Some(process) = processes.get_mut(process_id) else {
        return Err("process_not_found".to_string());
    };
    if process.info.state.is_terminal() {
        return Ok(process_detail(process));
    }
    match notification {
        Ok(Ok(())) => {
            process.info.state = ProcessState::CancelRequested;
            process.info.cancel_outcome = Some("notification_sent".to_string());
            process.info.termination_evidence = Some("mcp_cancel_notification_sent".to_string());
        }
        Ok(Err(error)) => {
            let now = Utc::now();
            process.info.state = ProcessState::Detached;
            process.info.updated_at = now;
            process.info.finished_at = Some(now);
            process.info.reject_reason = Some("mcp_cancel_notification_failed".to_string());
            process.info.cancel_outcome = Some("notification_failed".to_string());
            process.info.termination_evidence =
                Some("mcp_cancel_notification_delivery_failed".to_string());
            process.detail.error = Some(ProcessError {
                code: "mcp_cancel_notification_failed".to_string(),
                message: bounded_error_message(error.to_string()),
            });
            finalize_process(state, process).await;
        }
        Err(_) => {
            let now = Utc::now();
            process.info.state = ProcessState::Detached;
            process.info.updated_at = now;
            process.info.finished_at = Some(now);
            process.info.reject_reason = Some("mcp_cancel_notification_timeout".to_string());
            process.info.cancel_outcome = Some("notification_timeout".to_string());
            process.info.termination_evidence =
                Some("mcp_cancel_notification_delivery_timeout".to_string());
            process.detail.error = Some(ProcessError {
                code: "mcp_cancel_notification_timeout".to_string(),
                message: "MCP cancellation notification delivery exceeded 2 seconds".to_string(),
            });
            finalize_process(state, process).await;
        }
    }
    process_change_notifier(process).notify_waiters();
    let detail = process_detail(process);
    prune_terminal_processes(state, &mut processes);
    Ok(detail)
}

fn mark_cancelled(info: &mut ProcessInfo, outcome: &str, evidence: &str) {
    let now = Utc::now();
    info.state = ProcessState::Cancelled;
    info.updated_at = now;
    info.finished_at = Some(now);
    info.reject_reason = Some("cancelled".to_string());
    info.cancel_requested = true;
    info.cancel_outcome = Some(outcome.to_string());
    info.termination_evidence = Some(evidence.to_string());
}

fn missing_process_reason(state: &AppState, process_id: &str) -> String {
    match state.process_id_generation(process_id) {
        Some(generation) if generation != state.boot_generation => {
            "process_lost_after_restart".to_string()
        }
        _ => "process_not_found".to_string(),
    }
}

async fn refresh_processes(
    state: &AppState,
    processes: &mut std::collections::HashMap<String, ManagedProcess>,
) {
    for process in processes.values_mut() {
        refresh_process(state, process).await;
    }
}

async fn refresh_process(state: &AppState, process: &mut ManagedProcess) {
    if let ProcessRuntime::Process(runtime) = &mut process.runtime {
        if runtime.exit_status.is_none() {
            if let Some(child) = runtime.child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    runtime.exit_status = Some(status.into());
                    runtime.child = None;
                }
            }
        }
        if let Some(exit_status) = runtime.exit_status {
            let group_stopped = runtime
                .process_group_id
                .is_none_or(|process_group_id| !process_group_exists(process_group_id));
            if group_stopped {
                runtime.process_group_id = None;
            }
            if !process.info.state.is_terminal() {
                let now = Utc::now();
                process.info.exit_code = exit_status.code;
                let cancel_requested = process
                    .cancel_requested
                    .load(std::sync::atomic::Ordering::Acquire);
                if cancel_requested && !group_stopped {
                    process.info.state = ProcessState::CancelRequested;
                    process.info.updated_at = now;
                } else {
                    let startup_status = if let Some(mut reader) = runtime.startup_reader.take() {
                        match tokio::time::timeout(
                            std::time::Duration::from_millis(200),
                            &mut reader,
                        )
                        .await
                        {
                            Ok(Ok(status)) => Some(status),
                            Ok(Err(_)) | Err(_) => {
                                reader.abort();
                                let _ = reader.await;
                                Some(ShellStartupStatus::StartupFailed)
                            }
                        }
                    } else {
                        None
                    };
                    if cancel_requested {
                        process.info.state = ProcessState::Cancelled;
                        process.info.reject_reason = Some("cancelled".to_string());
                        process.info.cancel_requested = true;
                        process
                            .info
                            .cancel_outcome
                            .get_or_insert_with(|| "cancelled".to_string());
                        process.info.termination_evidence = Some(
                            runtime
                                .cancel_evidence
                                .unwrap_or("process_group_exit_after_cancel")
                                .to_string(),
                        );
                    } else {
                        match startup_status {
                            Some(ShellStartupStatus::InitFailed(status)) => {
                                let status = status.or(exit_status.code).unwrap_or(1);
                                process.info.state = ProcessState::Failed;
                                let message =
                                    format!("Shell init file failed with exit status {status}");
                                process.info.reject_reason =
                                    Some(format!("shell_init_file_failed: {message}"));
                                process.detail.error = Some(ProcessError {
                                    code: "shell_init_file_failed".to_string(),
                                    message,
                                });
                            }
                            Some(ShellStartupStatus::StartupFailed) => {
                                process.info.state = ProcessState::Failed;
                                let message =
                                    "Shell failed before the requested command started".to_string();
                                process.info.reject_reason =
                                    Some(format!("shell_startup_failed: {message}"));
                                process.detail.error = Some(ProcessError {
                                    code: "shell_startup_failed".to_string(),
                                    message,
                                });
                            }
                            Some(ShellStartupStatus::Ready) => {
                                process.info.state = ProcessState::Completed;
                            }
                            None if process.info.kind == ProcessKind::Command => {
                                process.info.state = ProcessState::Completed;
                            }
                            None => {
                                process.info.state = if exit_status.success {
                                    ProcessState::Completed
                                } else {
                                    ProcessState::Failed
                                };
                            }
                        }
                    }
                    process.info.updated_at = now;
                    process.info.finished_at = Some(now);
                    runtime.skill_lease = None;
                    runtime.changed.notify_waiters();
                }
            }
        }
        if let (Ok(stdout), Ok(stderr)) = (runtime.stdout.try_lock(), runtime.stderr.try_lock()) {
            let (status, error) = capture_summary(&stdout, &stderr);
            process.info.capture_status = status;
            process.info.capture_error = error.map(bounded_error_message);
        }
    }
    if process.info.state.is_terminal() {
        finalize_process(state, process).await;
    }
}

fn prune_terminal_processes(
    state: &AppState,
    processes: &mut std::collections::HashMap<String, ManagedProcess>,
) {
    let _ = state.process_history.retry_pending();
    for process in processes.values_mut() {
        if process.info.state.is_terminal()
            && process.history_terminal_snapshot_at.is_none()
            && !state
                .process_history
                .terminal_pending(&process.info.process_id)
            && state
                .process_history
                .terminal_snapshot_matches(&process.info.process_id, process.info.updated_at)
        {
            process.history_terminal_snapshot_at = Some(process.info.updated_at);
        }
    }
    let cutoff = Utc::now() - ChronoDuration::minutes(TERMINAL_PROCESS_HOT_CACHE_MINUTES);
    let mut terminal = processes
        .iter()
        .filter(|(_, process)| {
            process.info.state.is_terminal()
                && process.info.state != ProcessState::UnknownAfterRestart
                && process.history_terminal_snapshot_at.is_some()
                && process.info.capture_status != ProcessCaptureStatus::Capturing
                && !matches!(
                    &process.runtime,
                    ProcessRuntime::Process(runtime)
                        if runtime
                            .process_group_id
                            .is_some_and(process_group_exists)
                )
                && !state
                    .process_history
                    .terminal_pending(&process.info.process_id)
        })
        .map(|(id, process)| {
            (
                id.clone(),
                process.info.finished_at.unwrap_or(process.info.updated_at),
            )
        })
        .collect::<Vec<_>>();
    terminal.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (index, (id, updated_at)) in terminal.into_iter().enumerate() {
        if index >= MAX_TERMINAL_PROCESSES || updated_at < cutoff {
            processes.remove(&id);
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

    fn exec_request(command: &str, working_directory: &Path) -> ProcessExecRequest {
        ProcessExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            command: command.to_string(),
            need_confirm: false,
            confirm_method: None,
            cwd: Some(working_directory.to_string_lossy().to_string()),
            wait_seconds: Some(2),
        }
    }

    async fn start_shell_process_for_test_with_decision(
        state: AppState,
        request: ProcessExecRequest,
        options: ProcessOptions,
    ) -> ProcessInfo {
        let config = Arc::new(state.config.read().await.clone());
        let working_directory = PathBuf::from(request.cwd.as_deref().unwrap());
        start_managed_process_inner(
            state,
            request.into(),
            config,
            None,
            options,
            Some((working_directory, PolicyDecision::Allow)),
            None,
        )
        .await
    }

    async fn test_state(max_active_processes: usize) -> (AppState, PathBuf) {
        let root = unique_temp_dir("processes-max-active");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.shell.init_file = crate::config::ShellInitFile::Disabled;
        config.limits.max_active_processes =
            crate::config::MaxActiveProcesses::Explicit(max_active_processes);
        config.confirmation_provider =
            crate::config::ConfirmationProviderConfig::from_legacy("none").unwrap();
        config.policy.allow = [
            "false",
            "pwd",
            "printf",
            "sleep",
            "true",
            "touch",
            "__history_trigger_failure__",
        ]
        .into_iter()
        .map(|program| crate::config::Rule {
            program: program.to_string(),
            args_prefix: Vec::new(),
        })
        .collect();
        let private_state =
            crate::private_state::PrivateStatePaths::for_test(root.join("private-state"));
        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(RwLock::new(config)),
            event_store: crate::event_store::EventStore::open(&private_state).unwrap(),
            private_state: private_state.clone(),
            process_history: crate::process_history::ProcessHistoryStore::open(&private_state),
            browser_runtime: None,
            runtime: crate::state::RuntimeModel::hub(crate::state::CapabilityProfile::Normal),
            started_at: Utc::now(),
            boot_generation: "testboot0001".to_string(),
            supervised: false,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::process::McpConcurrency::new()),
            room_repository_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(crate::skills::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        };
        (state, workspace)
    }

    async fn test_state_with_history(max_active_processes: usize) -> (AppState, PathBuf) {
        test_state(max_active_processes).await
    }
    fn install_process_batch_admission_failure_trigger(state: &AppState) {
        let connection = rusqlite::Connection::open(state.process_history.path()).unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER IF EXISTS test_process_batch_admission_failure;
                 CREATE TRIGGER test_process_batch_admission_failure
                 BEFORE INSERT ON processes
                 WHEN json_extract(NEW.info_json, '$.commandPreview') = '__history_trigger_failure__'
                 BEGIN
                     SELECT RAISE(ABORT, 'test process batch admission failure');
                 END;",
            )
            .unwrap();
    }

    fn install_mcp_batch_admission_failure_trigger(state: &AppState) {
        let connection = rusqlite::Connection::open(state.process_history.path()).unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER IF EXISTS test_mcp_batch_admission_failure;
                 CREATE TRIGGER test_mcp_batch_admission_failure
                 BEFORE INSERT ON processes
                 WHEN json_extract(NEW.info_json, '$.mcpToolName') = '__history_trigger_failure__'
                 BEGIN
                     SELECT RAISE(ABORT, 'test MCP batch admission failure');
                 END;",
            )
            .unwrap();
    }

    fn remove_batch_admission_failure_triggers(state: &AppState) {
        let connection = rusqlite::Connection::open(state.process_history.path()).unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER IF EXISTS test_process_batch_admission_failure;
                 DROP TRIGGER IF EXISTS test_mcp_batch_admission_failure;",
            )
            .unwrap();
    }

    fn history_rows_after_reopen(state: &AppState) -> Vec<ProcessInfo> {
        crate::process_history::ProcessHistoryStore::open(&state.private_state)
            .list(&ProcessListRequest {
                group: None,
                kind: None,
                state: None,
                limit: Some(100),
                cursor: None,
            })
            .unwrap()
            .processes
    }

    fn mcp_spec(tool_name: &str) -> ManagedMcpSpec {
        ManagedMcpSpec {
            agent_id: "test-agent".to_string(),
            group: Some("batch-group".to_string()),
            batch_id: Some("batch-test".to_string()),
            batch_call_id: Some(format!("call-{tool_name}")),
            batch_index: Some(0),
            server_id: "test-server".to_string(),
            tool_name: tool_name.to_string(),
            request_source: "test:mcp.batch".to_string(),
            argument_keys: Vec::new(),
            argument_key_count: 0,
            argument_keys_truncated: false,
            argument_bytes: 2,
            argument_sha256: "sha256:test".to_string(),
            config_revision: "test-revision".to_string(),
            terminal_event_hook: None,
            event_origin: None,
        }
    }

    fn synthetic_process(
        process_id: &str,
        group: Option<&str>,
        kind: ProcessKind,
        state: ProcessState,
        created_at: chrono::DateTime<Utc>,
    ) -> ProcessInfo {
        let started_at = Some(created_at + chrono::Duration::milliseconds(1));
        let updated_at = created_at + chrono::Duration::milliseconds(2);
        ProcessInfo {
            agent_id: "test-agent".to_string(),
            process_id: process_id.to_string(),
            group: group.map(str::to_string),
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind,
            state,
            created_at,
            started_at,
            updated_at,
            finished_at: state.is_terminal().then_some(updated_at),
            program: Some("true".to_string()),
            args: Vec::new(),
            working_directory: None,
            command_preview: Some("true".to_string()),
            exit_code: state.is_terminal().then_some(0),
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
            capture_status: ProcessCaptureStatus::Complete,
            capture_error: None,
        }
    }

    fn synthetic_detail(info: ProcessInfo) -> ProcessDetail {
        ProcessDetail {
            process: info,
            detail_available: true,
            result: None,
            error: None,
            result_available: false,
            result_bytes: None,
            result_sha256: None,
            result_preview: None,
        }
    }

    async fn wait_terminal(state: &AppState, process: ProcessInfo) -> ProcessInfo {
        wait_for_process(state, process, 3).await
    }

    async fn read_process(
        state: &AppState,
        process_id: &str,
        wait_seconds: u64,
        view: ProcessReadView,
        cursor: Option<String>,
        max_bytes: Option<usize>,
    ) -> Result<ManagedProcessResponse, String> {
        get_process_read(
            state,
            ProcessReadRequest {
                process_id: process_id.to_string(),
                wait_seconds: Some(wait_seconds),
                view,
                cursor,
                max_bytes,
            },
        )
        .await
    }

    async fn wait_output_capture(state: &AppState, process_id: &str) -> ManagedProcessResponse {
        for _ in 0..100 {
            let output = read_process(state, process_id, 0, ProcessReadView::Auto, None, None)
                .await
                .unwrap();
            if matches!(
                output.response.capture_status,
                ProcessCaptureStatus::Complete | ProcessCaptureStatus::Incomplete
            ) {
                return output;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("process output capture did not settle");
    }

    fn decode_segment(segment: &ProcessOutputSegment) -> Vec<u8> {
        match segment.encoding {
            ProcessOutputEncoding::Utf8 => segment.data.as_bytes().to_vec(),
            ProcessOutputEncoding::Base64 => BASE64.decode(&segment.data).unwrap(),
        }
    }

    #[test]
    fn process_error_messages_are_utf8_safe_and_bounded() {
        let value = "错".repeat(MAX_PROCESS_ERROR_BYTES);
        let bounded = bounded_error_message(value);
        assert!(bounded.len() <= MAX_PROCESS_ERROR_BYTES);
        assert!(bounded.ends_with("...[truncated]"));
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[test]
    fn failed_process_reasons_are_projected_without_fabricating_exit_errors() {
        let now = Utc::now();
        let mut failed = synthetic_process(
            "process_spawn_failure",
            None,
            ProcessKind::Command,
            ProcessState::Failed,
            now,
        );
        failed.reject_reason = Some("spawn_failed: executable missing".to_string());
        let detail = synthetic_detail(failed.clone());
        let response = managed_response_from_observation(
            ProcessObservation {
                process: failed,
                detail,
                output: None,
            },
            ProcessReadView::Status,
            None,
            0,
            DEFAULT_PROCESS_RESPONSE_BYTES,
        )
        .unwrap();
        let error = response.response.error.unwrap();
        assert_eq!(error.code, "spawn_failed");
        assert!(error.message.starts_with("spawn_failed:"));

        let mut nonzero_exit = synthetic_process(
            "process_nonzero_exit",
            None,
            ProcessKind::Command,
            ProcessState::Failed,
            now + chrono::Duration::seconds(1),
        );
        nonzero_exit.exit_code = Some(127);
        let detail = synthetic_detail(nonzero_exit.clone());
        let response = managed_response_from_observation(
            ProcessObservation {
                process: nonzero_exit,
                detail,
                output: None,
            },
            ProcessReadView::Status,
            None,
            0,
            DEFAULT_PROCESS_RESPONSE_BYTES,
        )
        .unwrap();
        assert!(response.response.error.is_none());
    }

    #[test]
    fn process_read_rejects_output_pages_that_cannot_advance() {
        let mut info = synthetic_process(
            "process_tiny_output_budget",
            None,
            ProcessKind::Command,
            ProcessState::Running,
            Utc::now(),
        );
        info.agent_id.clear();
        info.capture_status = ProcessCaptureStatus::Capturing;
        let detail = synthetic_detail(info.clone());
        let output = crate::process_history::ProcessOutputSnapshot {
            stdout: vec![0xff],
            stdout_start_offset: 0,
            stdout_end_offset: 1,
            ..Default::default()
        };
        let (mut response, _, _, fallback_preview) = process_response_base(
            ProcessObservation {
                process: info,
                detail,
                output: Some(output.clone()),
            },
            ProcessReadView::Auto,
            0,
        );
        let cursor = ProcessCursor {
            version: 1,
            process_id: response.process_id.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
        };
        response.output =
            Some(output_page_reservation(&output, &response.process_id, &cursor).unwrap());
        let fixed_size = serialized_size(&response);
        assert!(fixed_size < MIN_PROCESS_RESPONSE_BYTES);
        response.agent_id = "a".repeat(MIN_PROCESS_RESPONSE_BYTES - fixed_size - 1);
        response.output = None;

        let error = fit_process_response(
            &mut response,
            Some(&output),
            Some(&cursor),
            fallback_preview,
            MIN_PROCESS_RESPONSE_BYTES,
        )
        .unwrap_err();
        assert_eq!(error, "process_response_budget_too_small");
    }

    #[tokio::test]
    async fn exec_waits_for_terminal_state_after_early_output() {
        let (state, workspace) = test_state(1).await;
        let mut request = exec_request("printf progress; sleep 0.15", &workspace);
        request.wait_seconds = Some(5);
        let budget = state.config.read().await.limits.process_response_bytes;
        let response =
            start_and_wait_process(state, request, ProcessOptions::for_source("test"), budget)
                .await
                .unwrap();
        assert_eq!(response.response.state, ProcessState::Completed);
        assert!(response.response.wait_elapsed_ms.unwrap() >= 100);
    }

    #[tokio::test]
    async fn exec_preserves_rejection_when_process_was_not_admitted() {
        let (state, workspace) = test_state(1).await;
        let long_running = exec_request("sleep 2", &workspace);
        let running = start_process_for_test(state.clone(), long_running).await;

        let mut rejected_request = exec_request("true", &workspace);
        rejected_request.wait_seconds = Some(0);
        let response = start_and_wait_process(
            state.clone(),
            rejected_request,
            ProcessOptions::for_source("test"),
            DEFAULT_PROCESS_RESPONSE_BYTES,
        )
        .await
        .unwrap();
        assert_eq!(response.response.state, ProcessState::Rejected);
        assert_eq!(
            response.response.error.as_ref().unwrap().code,
            "max_active_processes_reached"
        );
        assert_eq!(
            response.response.termination_evidence.as_deref(),
            Some("not_started")
        );
        assert!(response
            .process
            .reject_reason
            .as_deref()
            .unwrap()
            .contains("max_active"));

        let _ = cancel_process(&state, &running.process_id).await;
    }
    #[tokio::test]
    async fn completed_processes_release_capacity_and_keep_output() {
        let (state, workspace) = test_state(1).await;
        let first = start_process_for_test(state.clone(), exec_request("true", &workspace)).await;
        let first_starting = first.clone();
        assert!(first.process_id.starts_with("process_testboot0001_"));
        assert_eq!(first.kind, ProcessKind::Command);
        let first = wait_terminal(&state, first).await;
        assert_eq!(first.state, ProcessState::Completed);
        let stale_wait = tokio::time::timeout(
            Duration::from_millis(500),
            wait_for_process(&state, first_starting, 30),
        )
        .await
        .expect("a stale starting snapshot must refresh before waiting");
        assert_eq!(stale_wait.state, ProcessState::Completed);

        let second_request = exec_request("printf done", &workspace);
        let second = start_process_for_test(state.clone(), second_request).await;
        let second = wait_terminal(&state, second).await;
        assert_eq!(second.state, ProcessState::Completed);
        let output = wait_output_capture(&state, &second.process_id).await;
        let output_page = output.response.output.as_ref().unwrap();
        assert_eq!(output_page.stdout.data, "done");
        assert!(output_page.eof);
    }

    #[tokio::test]
    async fn shell_init_runs_before_working_directory_is_restored() {
        let (state, workspace) = test_state(1).await;
        let marker = workspace.join("shell-init-ran");
        let init_file = workspace.join("init.bash");
        fs::write(&init_file, format!(": > '{}'\ncd /\n", marker.display())).unwrap();
        state.config.write().await.shell.init_file =
            crate::config::ShellInitFile::Path(init_file.to_string_lossy().to_string());

        let process = start_process_for_test(state.clone(), exec_request("pwd", &workspace)).await;
        let process = wait_terminal(&state, process).await;
        assert_eq!(process.state, ProcessState::Completed);
        let output = wait_output_capture(&state, &process.process_id).await;
        assert_eq!(
            output.response.output.as_ref().unwrap().stdout.data,
            format!("{}\n", workspace.display())
        );
        assert!(marker.exists());
    }

    #[tokio::test]
    async fn shell_init_failure_blocks_command_and_reports_init_error() {
        let (state, workspace) = test_state(1).await;
        let marker = workspace.join("must-not-run");
        let init_file = workspace.join("init.bash");
        fs::write(&init_file, "return 23\n").unwrap();
        state.config.write().await.shell.init_file =
            crate::config::ShellInitFile::Path(init_file.to_string_lossy().to_string());

        let process = start_process_for_test(
            state.clone(),
            exec_request(&format!("touch {}", marker.display()), &workspace),
        )
        .await;
        let process = wait_terminal(&state, process).await;
        assert_eq!(process.state, ProcessState::Failed);
        assert_eq!(process.exit_code, Some(23));
        assert!(!marker.exists());
        let detail = get_process_detail(&state, &process.process_id, 0)
            .await
            .unwrap();
        assert_eq!(
            detail.error.as_ref().unwrap().code,
            "shell_init_file_failed"
        );
    }

    #[tokio::test]
    async fn pipefail_nonzero_exit_completes_with_shell_status() {
        let (state, workspace) = test_state(1).await;
        let process = start_process_for_test(
            state.clone(),
            exec_request("printf before | false", &workspace),
        )
        .await;
        let process = wait_terminal(&state, process).await;
        assert_eq!(process.state, ProcessState::Completed);
        assert_eq!(process.exit_code, Some(1));
        assert!(process.reject_reason.is_none());
        let detail = get_process_detail(&state, &process.process_id, 0)
            .await
            .unwrap();
        assert!(detail.error.is_none());
    }
    #[tokio::test]
    async fn reader_eof_is_visible_while_the_child_keeps_running() {
        let (state, workspace) = test_state(1).await;
        let request = exec_request("exec >/dev/null 2>&1; sleep 0.8", &workspace);
        let process = start_shell_process_for_test_with_decision(
            state.clone(),
            request,
            ProcessOptions::for_source("test"),
        )
        .await;
        let mut eof_while_running = None;
        for _ in 0..100 {
            let info = get_process(&state, &process.process_id, 0).await.unwrap();
            if info.state.is_active() {
                let output = read_process(
                    &state,
                    &process.process_id,
                    0,
                    ProcessReadView::Auto,
                    None,
                    None,
                )
                .await
                .unwrap();
                if output.response.output.as_ref().unwrap().eof {
                    eof_while_running = Some((info, output));
                    break;
                }
            }
            sleep(Duration::from_millis(10)).await;
        }
        let (info, output) =
            eof_while_running.expect("closed output pipes must report EOF before child exit");
        assert!(info.state.is_active());
        assert_eq!(
            output.response.capture_status,
            ProcessCaptureStatus::Complete
        );
        assert!(output.response.output.as_ref().unwrap().eof);
        assert_eq!(
            wait_terminal(&state, process).await.state,
            ProcessState::Completed
        );
    }

    #[tokio::test]
    async fn terminal_events_precede_inherited_pipe_eof_and_history_keeps_final_output() {
        let (state, workspace) = test_state(1).await;
        let hook_count = Arc::new(AtomicUsize::new(0));
        let hook_count_for_event = hook_count.clone();
        let mut options = ProcessOptions::for_source("test");
        options.terminal_event_hook = Some(Arc::new(move |_| {
            hook_count_for_event.fetch_add(1, Ordering::AcqRel);
        }));
        let request = exec_request(
            "printf before; (sleep 2; printf after) & sleep 0.15",
            &workspace,
        );
        let started =
            start_shell_process_for_test_with_decision(state.clone(), request, options).await;
        let terminal = tokio::time::timeout(Duration::from_secs(1), wait_terminal(&state, started))
            .await
            .expect("terminal notification must arrive before inherited pipe EOF");
        assert_eq!(terminal.state, ProcessState::Completed);
        assert_eq!(hook_count.load(Ordering::Acquire), 1);

        let before_eof = read_process(
            &state,
            &terminal.process_id,
            0,
            ProcessReadView::Auto,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            before_eof.response.capture_status,
            ProcessCaptureStatus::Capturing
        );
        assert!(!before_eof.response.output.as_ref().unwrap().eof);
        let admission = state
            .process_history
            .get(&terminal.process_id)
            .unwrap()
            .expect("the active admission row remains until reader EOF");
        assert!(admission.info.state.is_active());

        let source = crate::event_notifications::process_source(&terminal.process_id);
        state.event_store.settle_response(&source, false).unwrap();
        let notifications = state
            .event_store
            .list(&agentic_gpt_protocol::EventListRequest::default())
            .unwrap();
        let notification = notifications
            .items
            .first()
            .expect("completed process event is visible before inherited pipe EOF");
        let persisted_event = state.event_store.get(&notification.event_id).unwrap();
        assert_eq!(persisted_event.source, source);
        assert!(persisted_event.message.contains("completed"));

        let output = tokio::time::timeout(
            Duration::from_secs(4),
            read_process(
                &state,
                &terminal.process_id,
                30,
                ProcessReadView::Auto,
                before_eof
                    .response
                    .output
                    .as_ref()
                    .map(|page| page.next_cursor.clone()),
                None,
            ),
        )
        .await
        .expect("auto read should wake for inherited output and EOF")
        .unwrap();
        let output_page = output.response.output.as_ref().unwrap();
        assert_eq!(decode_segment(&output_page.stdout), b"after");
        let settled = tokio::time::timeout(
            Duration::from_secs(2),
            read_process(
                &state,
                &terminal.process_id,
                30,
                ProcessReadView::Auto,
                Some(output_page.next_cursor.clone()),
                None,
            ),
        )
        .await
        .expect("capture EOF should settle after the final inherited output")
        .unwrap();
        let settled_page = settled.response.output.unwrap();
        assert!(settled_page.eof);
        assert!(decode_segment(&settled_page.stdout).is_empty());
        assert_eq!(
            get_process(&state, &terminal.process_id, 0)
                .await
                .unwrap()
                .state,
            ProcessState::Completed
        );
        let history = state
            .process_history
            .get(&terminal.process_id)
            .unwrap()
            .expect("the final terminal snapshot must be durable after EOF");
        assert_eq!(history.info.state, ProcessState::Completed);
        assert_eq!(history.output.stdout, b"beforeafter");
        assert_eq!(hook_count.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn process_read_cursors_are_replayable_and_page_raw_offsets() {
        let (state, workspace) = test_state(2).await;
        let payload = "x".repeat(12 * 1024);
        let request = exec_request(&format!("printf '%s' '{payload}'"), &workspace);
        let process = start_process_for_test(state.clone(), request).await;
        let process = wait_terminal(&state, process).await;
        let _ = wait_output_capture(&state, &process.process_id).await;

        let first = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MIN_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap();
        assert!(serialized_size(&first.response) <= MIN_PROCESS_RESPONSE_BYTES);
        let first_page = first.response.output.unwrap();
        assert_eq!(first_page.stdout.start_offset, "0");
        assert!(!first_page.stdout.data.is_empty());
        assert!(first_page.has_more);
        assert!(!first_page.eof);
        let replay = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MIN_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap();
        let replay_page = replay.response.output.unwrap();
        assert_eq!(replay_page.stdout.data, first_page.stdout.data);
        assert_eq!(replay_page.next_cursor, first_page.next_cursor);

        let cursor_for_other = first_page.next_cursor.clone();
        let mut current_page = first_page;
        let mut collected = decode_segment(&current_page.stdout);
        while current_page.has_more {
            let previous_end = current_page.stdout.end_offset.clone();
            let next = read_process(
                &state,
                &process.process_id,
                0,
                ProcessReadView::Auto,
                Some(current_page.next_cursor.clone()),
                Some(MIN_PROCESS_RESPONSE_BYTES),
            )
            .await
            .unwrap();
            current_page = next.response.output.unwrap();
            assert_eq!(current_page.stdout.start_offset, previous_end);
            collected.extend(decode_segment(&current_page.stdout));
        }
        assert!(current_page.eof);
        assert_eq!(collected.as_slice(), payload.as_bytes());

        let malformed = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Auto,
            Some("invalid".to_string()),
            Some(MIN_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap_err();
        assert_eq!(malformed, "invalid_process_output_cursor");
        let invalid_budget = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MIN_PROCESS_RESPONSE_BYTES - 1),
        )
        .await
        .unwrap_err();
        assert_eq!(invalid_budget, "process_read_max_bytes_out_of_range");
        let status_cursor = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Status,
            Some(cursor_for_other.clone()),
            Some(MIN_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap_err();
        assert_eq!(status_cursor, "process_read_cursor_with_status_view");
        let ahead_cursor = encode_output_cursor(agentic_gpt_protocol::ProcessCursor {
            version: 1,
            process_id: process.process_id.clone(),
            stdout_offset: u64::MAX,
            stderr_offset: 0,
        })
        .unwrap();
        let ahead = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Auto,
            Some(ahead_cursor),
            Some(MIN_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap_err();
        assert_eq!(ahead, "process_output_cursor_ahead_of_output");

        let other = start_process_for_test(state.clone(), exec_request("true", &workspace)).await;
        let other = wait_terminal(&state, other).await;
        let _ = wait_output_capture(&state, &other.process_id).await;
        let bound_cursor = read_process(
            &state,
            &other.process_id,
            0,
            ProcessReadView::Auto,
            Some(cursor_for_other),
            Some(MIN_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap_err();
        assert_eq!(bound_cursor, "invalid_process_output_cursor");
    }

    #[tokio::test]
    async fn output_pages_base64_invalid_utf8_without_loss() {
        let (state, workspace) = test_state(1).await;
        let request = exec_request(r"printf '\377\000\200'", &workspace);
        let process = start_process_for_test(state.clone(), request).await;
        let process = wait_terminal(&state, process).await;
        let output = wait_output_capture(&state, &process.process_id).await;
        let output_page = output.response.output.as_ref().unwrap();
        assert_eq!(output_page.stdout.encoding, ProcessOutputEncoding::Base64);
        assert_eq!(decode_segment(&output_page.stdout), [0xff, 0x00, 0x80]);
        assert!(output_page.eof);

        let bounded = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MIN_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap();
        let bounded_page = bounded.response.output.unwrap();
        assert_eq!(bounded_page.stdout.encoding, ProcessOutputEncoding::Base64);
        assert_eq!(bounded_page.stdout.data, BASE64.encode([0xff, 0x00, 0x80]));
        assert!(bounded_page.eof);
        let too_small = read_process(
            &state,
            &process.process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MIN_PROCESS_RESPONSE_BYTES - 1),
        )
        .await
        .unwrap_err();
        assert_eq!(too_small, "process_read_max_bytes_out_of_range");
    }

    #[tokio::test]
    async fn output_overflow_reports_exact_retained_gap() {
        let mut ring = OutputRing::new(Arc::new(Notify::new()));
        ring.mark_started();
        ring.push(&vec![b'x'; PROCESS_OUTPUT_RING_CAPACITY + 5]);
        ring.finish(ReaderOutcome::Eof);
        let (data, start, end) = ring.snapshot();
        let snapshot = crate::process_history::ProcessOutputSnapshot {
            stdout: data,
            stdout_start_offset: start,
            stdout_end_offset: end,
            ..Default::default()
        };
        let cursor = ProcessCursor {
            version: 1,
            process_id: "process_gap".to_string(),
            stdout_offset: 0,
            stderr_offset: 0,
        };
        let (page, used) = output_page_for_json_budget(
            &snapshot,
            "process_gap",
            &cursor,
            ProcessCaptureStatus::Complete,
            64,
        )
        .unwrap();
        let gap = page
            .stdout
            .gap
            .as_ref()
            .expect("retained output must report a gap");
        assert_eq!(gap.start_offset, "0");
        assert_eq!(gap.end_offset, "5");
        assert_eq!(page.stdout.start_offset, "5");
        assert_eq!(page.stdout.end_offset, "69");
        assert_eq!(used, 64);
        assert_eq!(decode_segment(&page.stdout), vec![b'x'; 64]);
        assert!(page.has_more);
        assert!(!page.eof);
    }

    #[tokio::test]
    async fn process_read_compacts_output_to_configured_response_budget() {
        let (state, workspace) = test_state(2).await;
        let budget = state.config.read().await.limits.process_response_bytes;
        let request = exec_request(
            &format!("printf '%s' '{}'", "x".repeat(12 * 1024)),
            &workspace,
        );
        let response =
            start_and_wait_process(state, request, ProcessOptions::for_source("test"), budget)
                .await
                .unwrap();
        assert_eq!(response.response.state, ProcessState::Completed);
        assert_eq!(response.response.agent_id, "test-agent");
        assert!(serialized_size(&response.response) <= budget);
        let output = response.response.output.unwrap();
        assert!(!output.stdout.data.is_empty());
        assert!(output.has_more);
        assert!(!output.eof);
    }

    #[tokio::test]
    async fn process_read_mcp_result_exposes_retained_and_not_retained_values() {
        let (state, _workspace) = test_state(4).await;
        let registration = register_mcp_process(&state, mcp_spec("retained-result"))
            .await
            .unwrap();
        let process_id = registration.info.process_id;
        let value = serde_json::json!({"ok": true, "value": "retained"});
        complete_mcp_result(&state, &process_id, value.clone(), false, None)
            .await
            .unwrap();

        let too_small = read_process(
            &state,
            &process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MIN_PROCESS_RESPONSE_BYTES - 1),
        )
        .await
        .unwrap_err();
        assert_eq!(too_small, "process_read_max_bytes_out_of_range");

        let complete = read_process(
            &state,
            &process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MAX_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap();
        let result = complete.response.mcp_result.unwrap();
        assert_eq!(result.status, ProcessMcpResultStatus::Included);
        assert_eq!(result.value, Some(value));
        let cursor = encode_output_cursor(ProcessCursor {
            version: 1,
            process_id: process_id.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
        })
        .unwrap();
        let cursor_error = read_process(
            &state,
            &process_id,
            0,
            ProcessReadView::Auto,
            Some(cursor),
            Some(MAX_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap_err();
        assert_eq!(cursor_error, "process_read_cursor_not_supported_for_mcp");

        let registration = register_mcp_process(&state, mcp_spec("oversized-result"))
            .await
            .unwrap();
        let process_id = registration.info.process_id;
        let value = serde_json::json!({
            "payload": "x".repeat(MAX_MCP_RESULT_BYTES)
        });
        complete_mcp_result(&state, &process_id, value, false, None)
            .await
            .unwrap();
        let too_large = read_process(
            &state,
            &process_id,
            0,
            ProcessReadView::Auto,
            None,
            Some(MAX_PROCESS_RESPONSE_BYTES),
        )
        .await
        .unwrap();
        let result = too_large.response.mcp_result.unwrap();
        assert_eq!(result.status, ProcessMcpResultStatus::NotRetained);
        assert!(result.value.is_none());
        assert!(result.bytes.unwrap() > MAX_MCP_RESULT_BYTES);
    }

    #[tokio::test]
    async fn process_batch_response_obeys_whole_body_budget_and_keeps_identities() {
        let (state, workspace) = test_state(4).await;
        let budget = state.config.read().await.limits.process_response_bytes;
        let payload = "\\\"quoted\\n".repeat(700);
        let request = ProcessBatchExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            elements: vec![
                agentic_gpt_protocol::ProcessExecElement {
                    command: format!("printf '%s' '{payload}'"),
                    cwd: None,
                },
                agentic_gpt_protocol::ProcessExecElement {
                    command: format!("printf '%s' '{payload}'"),
                    cwd: None,
                },
            ],
            need_confirm: false,
            confirm_method: None,
            cwd: Some(workspace.to_string_lossy().to_string()),
            wait_seconds: Some(2),
        };
        let response = start_process_batch(
            state.clone(),
            request,
            "test:process.batch".to_string(),
            None,
            None,
            budget,
        )
        .await
        .unwrap();
        assert!(serialized_size(&response.response) <= budget);
        assert_eq!(response.response.processes.len(), 2);
        assert!(response
            .response
            .processes
            .iter()
            .enumerate()
            .all(|(index, process)| {
                process.agent_id == "test-agent"
                    && !process.process_id.is_empty()
                    && process.batch_id.as_deref() == Some(response.response.batch_id.as_str())
                    && process.batch_index == Some(index)
            }));
        assert_eq!(response.processes.len(), 2);
        // Execution can finish before readers settle. Fit populated snapshots to
        // exercise escaped JSON budgeting without assuming exit implies capture EOF.
        let mut response = response.response;
        let mut outputs = Vec::new();
        for child in &mut response.processes {
            let captured = wait_output_capture(&state, &child.process_id).await;
            assert_eq!(
                captured.response.capture_status,
                ProcessCaptureStatus::Complete
            );
            let observation = get_process_observation(&state, &child.process_id, true)
                .await
                .unwrap();
            let (base, _, output, _) = process_response_base(observation, ProcessReadView::Auto, 0);
            *child = base;
            outputs.push(output);
        }
        let fitted = fit_process_batch_response(response, &outputs, budget).unwrap();
        assert!(serialized_size(&fitted) <= budget);
        let pages = fitted
            .processes
            .iter()
            .map(|process| process.output.as_ref().expect("batch output page"))
            .collect::<Vec<_>>();
        assert!(pages.iter().all(|page| {
            page.stdout
                .end_offset
                .parse::<u64>()
                .expect("numeric output offset")
                > page.stdout.start_offset.parse::<u64>().unwrap()
        }));
    }

    #[tokio::test]
    async fn oversized_process_admission_fails_before_command_effect() {
        let (state, workspace) = test_state(1).await;
        let marker = workspace.join("oversized-admission-must-not-run");
        let oversized_args = (0..70)
            .map(|_| "x".repeat(4 * 1024))
            .collect::<Vec<_>>()
            .join(" ");
        let request = exec_request(
            &format!("touch {} {oversized_args}", marker.display()),
            &workspace,
        );

        let rejected = start_process_for_test(state.clone(), request).await;
        assert_eq!(rejected.state, ProcessState::Failed);
        assert!(rejected
            .reject_reason
            .as_deref()
            .unwrap()
            .contains("process_history_admission_metadata_too_large"));
        assert!(!marker.exists());
        assert!(state.processes.lock().await.is_empty());
        assert!(history_rows_after_reopen(&state).is_empty());
    }

    #[tokio::test]
    async fn oversized_process_batch_is_rejected_before_any_admission() {
        let (state, workspace) = test_state(100).await;
        let markers = (0..100)
            .map(|index| workspace.join(format!("must-not-run-{index}")))
            .collect::<Vec<_>>();
        let request = ProcessBatchExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            elements: markers
                .iter()
                .map(|marker| agentic_gpt_protocol::ProcessExecElement {
                    command: format!("touch {}", marker.display()),
                    cwd: None,
                })
                .collect(),
            need_confirm: false,
            confirm_method: None,
            cwd: Some(workspace.to_string_lossy().to_string()),
            wait_seconds: Some(0),
        };

        let error = start_process_batch(
            state.clone(),
            request,
            "test:process.batch".to_string(),
            None,
            None,
            DEFAULT_PROCESS_RESPONSE_BYTES,
        )
        .await
        .unwrap_err();
        assert_eq!(error, "process_batch_response_too_large");
        assert!(state.processes.lock().await.is_empty());
        assert!(history_rows_after_reopen(&state).is_empty());
        assert!(markers.iter().all(|marker| !marker.exists()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn admitted_process_keeps_policy_snapshot_across_reload() {
        let (state, workspace) = test_state(1).await;
        let request = exec_request("printf admitted", &workspace);
        let admitted = start_process_for_test(state.clone(), request).await;
        assert!(admitted.started_at.is_none());

        state
            .config
            .write()
            .await
            .policy
            .deny
            .push(crate::config::Rule {
                program: "printf".to_string(),
                args_prefix: Vec::new(),
            });
        let finished = wait_terminal(&state, admitted).await;
        assert_eq!(finished.state, ProcessState::Completed);
        let output = wait_output_capture(&state, &finished.process_id).await;
        assert_eq!(
            output.response.output.as_ref().unwrap().stdout.data,
            "admitted"
        );
    }

    #[tokio::test]
    async fn process_batch_admission_failure_is_atomic_before_spawn() {
        let (state, workspace) = test_state(2).await;
        let marker = workspace.join("process-batch-must-not-run");
        let first_request: exec::ExecutionRequest =
            exec_request(&format!("touch {}", marker.display()), &workspace).into();
        let second_request: exec::ExecutionRequest =
            exec_request("__history_trigger_failure__", &workspace).into();
        let specs = vec![
            ManagedProcessSpec {
                request: first_request,
                working_directory: workspace.clone(),
                batch_id: None,
                batch_index: None,
                decision: PolicyDecision::Allow,
                confirmation_result: None,
                request_source: "test:process.batch".to_string(),
                terminal_event_hook: None,
                event_origin: None,
            },
            ManagedProcessSpec {
                request: second_request,
                working_directory: workspace.clone(),
                decision: PolicyDecision::Allow,
                confirmation_result: None,
                batch_id: None,
                batch_index: None,
                request_source: "test:process.batch".to_string(),
                terminal_event_hook: None,
                event_origin: None,
            },
        ];
        let config = Arc::new(state.config.read().await.clone());
        install_process_batch_admission_failure_trigger(&state);

        let error = start_prepared_managed_batch(state.clone(), config.clone(), specs)
            .await
            .unwrap_err();
        assert!(error.starts_with("history_admission_failed:"));
        assert!(state.processes.lock().await.is_empty());
        assert!(!marker.exists());
        assert!(history_rows_after_reopen(&state).is_empty());

        remove_batch_admission_failure_triggers(&state);
        let retry = start_prepared_managed_batch(
            state.clone(),
            config,
            vec![
                ManagedProcessSpec {
                    request: exec_request("true", &workspace).into(),
                    working_directory: workspace.clone(),
                    decision: PolicyDecision::Allow,
                    confirmation_result: None,
                    batch_id: None,
                    batch_index: None,
                    request_source: "test:process.batch".to_string(),
                    terminal_event_hook: None,
                    event_origin: None,
                },
                ManagedProcessSpec {
                    request: exec_request("true", &workspace).into(),
                    working_directory: workspace.clone(),
                    decision: PolicyDecision::Allow,
                    confirmation_result: None,
                    batch_id: None,
                    batch_index: None,
                    request_source: "test:process.batch".to_string(),
                    terminal_event_hook: None,
                    event_origin: None,
                },
            ],
        )
        .await
        .unwrap();
        assert_eq!(retry.len(), 2);
        for info in retry {
            assert_eq!(
                wait_terminal(&state, info).await.state,
                ProcessState::Completed
            );
        }
    }

    #[tokio::test]
    async fn mcp_batch_admission_failure_is_atomic_before_registration() {
        let (state, _workspace) = test_state(2).await;
        let mut second = mcp_spec("__history_trigger_failure__");
        second.batch_index = Some(1);
        install_mcp_batch_admission_failure_trigger(&state);

        let error = match register_mcp_batch(&state, vec![mcp_spec("first-tool"), second]).await {
            Ok(_) => panic!("MCP batch admission unexpectedly succeeded"),
            Err(error) => error,
        };
        assert!(error.starts_with("history_admission_failed:"));
        assert!(state.processes.lock().await.is_empty());
        assert!(history_rows_after_reopen(&state).is_empty());

        remove_batch_admission_failure_triggers(&state);
        let mut retry_second = mcp_spec("second-tool");
        retry_second.batch_index = Some(1);
        let registrations = register_mcp_batch(&state, vec![mcp_spec("first-tool"), retry_second])
            .await
            .unwrap();
        assert_eq!(registrations.len(), 2);
        assert_eq!(state.processes.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn process_batch_respects_max_concurrent_tasks_without_blocking_batch_return() {
        let (state, workspace) = test_state(4).await;
        state.config.write().await.limits.max_concurrent_tasks = 1;
        let request = ProcessBatchExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            elements: vec![
                agentic_gpt_protocol::ProcessExecElement {
                    command: "sleep 2".to_string(),
                    cwd: None,
                },
                agentic_gpt_protocol::ProcessExecElement {
                    command: "sleep 2".to_string(),
                    cwd: None,
                },
            ],
            need_confirm: false,
            confirm_method: None,
            cwd: Some(workspace.to_string_lossy().to_string()),
            wait_seconds: Some(0),
        };

        let batch = start_process_batch(
            state.clone(),
            request,
            "test:process.batch".to_string(),
            None,
            None,
            DEFAULT_PROCESS_RESPONSE_BYTES,
        )
        .await
        .unwrap();
        assert_eq!(batch.response.status, "running");
        assert_eq!(batch.processes.len(), 2);

        let mut states = Vec::new();
        for _ in 0..100 {
            states = Vec::with_capacity(batch.processes.len());
            for process in &batch.processes {
                states.push(
                    get_process(&state, &process.process_id, 0)
                        .await
                        .unwrap()
                        .state,
                );
            }
            if states.contains(&ProcessState::Running) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            states
                .iter()
                .filter(|state| **state == ProcessState::Running)
                .count(),
            1
        );
        assert_eq!(
            states
                .iter()
                .filter(|state| **state == ProcessState::Queued)
                .count(),
            1
        );

        for process in &batch.processes {
            let _ = cancel_process(&state, &process.process_id).await;
        }
    }

    #[tokio::test]
    async fn process_batch_wait_wakes_for_later_child() {
        let (state, workspace) = test_state(4).await;
        state.config.write().await.limits.max_concurrent_tasks = 1;
        let request = ProcessBatchExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            elements: vec![
                agentic_gpt_protocol::ProcessExecElement {
                    command: "true".to_string(),
                    cwd: None,
                },
                agentic_gpt_protocol::ProcessExecElement {
                    command: "sleep 0.15".to_string(),
                    cwd: None,
                },
            ],
            need_confirm: false,
            confirm_method: None,
            cwd: Some(workspace.to_string_lossy().to_string()),
            wait_seconds: Some(30),
        };

        let batch = tokio::time::timeout(
            Duration::from_secs(3),
            start_process_batch(
                state,
                request,
                "test:process.batch".to_string(),
                None,
                None,
                DEFAULT_PROCESS_RESPONSE_BYTES,
            ),
        )
        .await
        .expect("batch wait should return when the queued child finishes")
        .unwrap();
        assert_eq!(batch.response.status, "completed");
        assert_eq!(batch.processes.len(), 2);
        assert!(batch
            .processes
            .iter()
            .all(|process| process.state == ProcessState::Completed));
    }

    #[tokio::test]
    async fn process_read_wait_survives_process_lock_contention() {
        let (state, _workspace) = test_state(2).await;
        let registration = register_mcp_process(&state, mcp_spec("read-lock-contention"))
            .await
            .unwrap();
        let process_id = registration.info.process_id;
        let auto_read = read_process(&state, &process_id, 30, ProcessReadView::Auto, None, None);
        let status_read =
            read_process(&state, &process_id, 30, ProcessReadView::Status, None, None);
        tokio::pin!(auto_read);
        tokio::pin!(status_read);

        let processes = state.processes.lock().await;
        assert!(futures_util::poll!(auto_read.as_mut()).is_pending());
        assert!(futures_util::poll!(status_read.as_mut()).is_pending());
        drop(processes);
        assert!(futures_util::poll!(auto_read.as_mut()).is_pending());
        assert!(futures_util::poll!(status_read.as_mut()).is_pending());

        let completion = complete_mcp_result(
            &state,
            &process_id,
            serde_json::json!({"ok": true, "value": "complete"}),
            false,
            None,
        );
        // Readers may hold queued FIFO mutex reservations after manual polling.
        // Drive them alongside the producer rather than awaiting the producer alone.
        let (completion, auto, status) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(completion, auto_read, status_read)
        })
        .await
        .expect("both waiting readers should wake after MCP completion");
        completion.unwrap();
        let auto = auto.unwrap();
        let status = status.unwrap();
        assert_eq!(auto.response.state, ProcessState::Completed);
        assert_eq!(
            auto.response.mcp_result.unwrap().status,
            ProcessMcpResultStatus::Included
        );
        assert_eq!(status.response.state, ProcessState::Completed);
        assert!(status.response.output.is_none());
    }

    #[tokio::test]
    async fn active_process_capacity_and_cancel_are_truthful() {
        let (state, workspace) = test_state(1).await;
        let request = exec_request("sleep 2", &workspace);
        let running = start_process_for_test(state.clone(), request).await;
        let mut running_state = running.clone();
        for _ in 0..100 {
            running_state = get_process(&state, &running.process_id, 0).await.unwrap();
            if running_state.state == ProcessState::Running {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(running_state.state, ProcessState::Running);
        let rejected =
            start_process_for_test(state.clone(), exec_request("true", &workspace)).await;
        let rejected = wait_terminal(&state, rejected).await;
        assert_eq!(rejected.state, ProcessState::Rejected);
        assert!(rejected
            .reject_reason
            .unwrap()
            .starts_with("max_active_processes_reached"));

        let cancelled = cancel_process(&state, &running.process_id).await.unwrap();
        assert_eq!(cancelled.state, ProcessState::Cancelled);
        assert_eq!(cancelled.cancel_outcome, "cancelled");
        assert!(matches!(
            cancelled.termination_evidence.as_str(),
            "process_group_sigterm_observed" | "process_group_sigkill_observed"
        ));
    }

    #[tokio::test]
    async fn cancellation_reaches_background_descendant_after_leader_exit() {
        let (state, workspace) = test_state(1).await;
        let request = exec_request("/usr/bin/sleep 100 &", &workspace);
        let started = start_shell_process_for_test_with_decision(
            state.clone(),
            request,
            ProcessOptions::for_source("test"),
        )
        .await;
        let completed = wait_terminal(&state, started).await;
        assert_eq!(completed.state, ProcessState::Completed);
        assert_eq!(completed.exit_code, Some(0));
        assert_ne!(completed.capture_status, ProcessCaptureStatus::Complete);
        let process_group_id = {
            let processes = state.processes.lock().await;
            let process = processes.get(&completed.process_id).unwrap();
            match &process.runtime {
                ProcessRuntime::Process(runtime) => runtime.process_group_id.unwrap(),
                ProcessRuntime::Mcp(_) => unreachable!(),
            }
        };
        assert!(process_group_exists(process_group_id));

        let cancelled = cancel_process(&state, &completed.process_id).await.unwrap();
        assert_eq!(cancelled.state, ProcessState::Completed);
        assert_eq!(cancelled.cancel_outcome, "cancelled");
        assert!(matches!(
            cancelled.termination_evidence.as_str(),
            "process_group_sigterm_observed" | "process_group_sigkill_observed"
        ));
        assert!(!process_group_exists(process_group_id));
        let settled = get_process(&state, &completed.process_id, 0).await.unwrap();
        assert_ne!(settled.capture_status, ProcessCaptureStatus::Capturing);
    }

    #[tokio::test]
    async fn cancelling_a_terminal_process_reports_already_terminal_without_rewriting_state() {
        let (state, workspace) = test_state(1).await;
        let completed = wait_terminal(
            &state,
            start_process_for_test(state.clone(), exec_request("true", &workspace)).await,
        )
        .await;
        let _ = wait_output_capture(&state, &completed.process_id).await;
        let before = get_process(&state, &completed.process_id, 0).await.unwrap();
        let mut terminal_history_durable = false;
        for _ in 0..100 {
            terminal_history_durable = state
                .process_history
                .get(&completed.process_id)
                .unwrap()
                .is_some_and(|record| record.info.state.is_terminal());
            if terminal_history_durable {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(terminal_history_durable);
        let history_before = state
            .process_history
            .get(&completed.process_id)
            .unwrap()
            .expect("completed process must be durable before cancellation");
        let cancelled = cancel_process(&state, &completed.process_id).await.unwrap();
        assert_eq!(cancelled.state, ProcessState::Completed);
        assert_eq!(cancelled.cancel_outcome, "already_terminal");
        assert_eq!(cancelled.termination_evidence, "process_state");

        let after = get_process(&state, &completed.process_id, 0).await.unwrap();
        let history_after = state
            .process_history
            .get(&completed.process_id)
            .unwrap()
            .expect("already-terminal cancellation must retain history");
        assert_eq!(after.updated_at, before.updated_at);
        assert_eq!(after.updated_at, history_after.info.updated_at);
        assert_eq!(after.cancel_requested, before.cancel_requested);
        assert_eq!(after.cancel_requested, history_after.info.cancel_requested);
        assert_eq!(after.cancel_outcome, before.cancel_outcome);
        assert_eq!(after.cancel_outcome, history_after.info.cancel_outcome);
        assert_eq!(after.termination_evidence, before.termination_evidence);
        assert_eq!(
            after.termination_evidence,
            history_after.info.termination_evidence
        );
        assert_eq!(
            history_before.info.updated_at,
            history_after.info.updated_at
        );
    }

    #[tokio::test]
    async fn process_filters_and_restart_loss_are_explicit() {
        let (state, workspace) = test_state(2).await;
        let completed = wait_terminal(
            &state,
            start_process_for_test(state.clone(), exec_request("true", &workspace)).await,
        )
        .await;
        let listed = list_processes(
            &state,
            ProcessListRequest {
                group: None,
                kind: Some(ProcessKind::Command),
                state: Some(ProcessState::Completed),
                limit: Some(1),
                cursor: None,
            },
        )
        .await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].process_id, completed.process_id);
        assert_eq!(
            get_process(&state, "process_oldboot_abc", 0)
                .await
                .unwrap_err(),
            "process_lost_after_restart"
        );
        assert_eq!(
            get_process(&state, "not-a-process", 0).await.unwrap_err(),
            "process_not_found"
        );
    }

    #[tokio::test]
    async fn runtime_history_tracks_group_timestamps_and_hot_cache_fallback() {
        let (state, workspace) = test_state_with_history(2).await;
        let mut request = exec_request("printf history-output", &workspace);
        request.group = Some("  runtime-group  ".to_string());
        let admitted = start_process_for_test(state.clone(), request).await;
        assert_eq!(admitted.group.as_deref(), Some("runtime-group"));
        assert!(admitted.started_at.is_none());

        let terminal = wait_terminal(&state, admitted).await;
        let output = wait_output_capture(&state, &terminal.process_id).await;
        let output_page = output.response.output.as_ref().unwrap();
        assert_eq!(decode_segment(&output_page.stdout), b"history-output");
        assert!(output_page.eof);
        let started_at = terminal.started_at.expect("process should start");
        let finished_at = terminal.finished_at.expect("process should finish");
        assert!(terminal.created_at <= started_at);
        assert!(started_at <= finished_at);

        for _ in 0..100 {
            if state
                .process_history
                .get(&terminal.process_id)
                .unwrap()
                .is_some_and(|record| record.info.state.is_terminal())
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        let record = state
            .process_history
            .get(&terminal.process_id)
            .unwrap()
            .expect("terminal admission should be persisted");
        assert_eq!(record.info.group.as_deref(), Some("runtime-group"));
        assert_eq!(record.info.started_at, terminal.started_at);
        assert_eq!(record.info.finished_at, terminal.finished_at);
        assert_eq!(record.output.stdout, b"history-output");
        assert_eq!(record.output.stdout_start_offset, 0);
        assert_eq!(record.output.stdout_end_offset, 14);

        {
            let mut processes = state.processes.lock().await;
            let process = processes.get_mut(&terminal.process_id).unwrap();
            process.info.finished_at = Some(Utc::now() - chrono::Duration::minutes(6));
            prune_terminal_processes(&state, &mut processes);
            assert!(!processes.contains_key(&terminal.process_id));
        }
        let recovered = get_process_detail(&state, &terminal.process_id, 0)
            .await
            .unwrap();
        assert_eq!(recovered.process.group.as_deref(), Some("runtime-group"));
        assert_eq!(recovered.process.state, ProcessState::Completed);
        let recovered_output = read_process(
            &state,
            &terminal.process_id,
            0,
            ProcessReadView::Auto,
            None,
            None,
        )
        .await
        .unwrap();
        let output = recovered_output.response.output.unwrap();
        assert_eq!(decode_segment(&output.stdout), b"history-output");
        assert!(output.eof);
    }

    #[tokio::test]
    async fn process_list_merges_live_wins_and_uses_global_cursor_order() {
        let (state, _workspace) = test_state_with_history(4).await;
        let now = Utc::now();
        let persisted_old = synthetic_process(
            "process_persisted_old",
            Some("alpha"),
            ProcessKind::Command,
            ProcessState::Completed,
            now - chrono::Duration::seconds(2),
        );
        let persisted_new = synthetic_process(
            "process_persisted_new",
            Some("alpha"),
            ProcessKind::Mcp,
            ProcessState::Completed,
            now - chrono::Duration::seconds(1),
        );
        for info in [&persisted_old, &persisted_new] {
            let _ = state.process_history.insert_admissions([info]);
            let _ = state.process_history.upsert_terminal(
                &synthetic_detail(info.clone()),
                &crate::process_history::ProcessOutputSnapshot::default(),
            );
        }
        let live_duplicate = synthetic_process(
            "process_persisted_old",
            Some("alpha"),
            ProcessKind::Command,
            ProcessState::Running,
            persisted_old.created_at,
        );
        state.processes.lock().await.insert(
            live_duplicate.process_id.clone(),
            ManagedProcess {
                info: live_duplicate,
                detail: ManagedProcessDetail::default(),
                runtime: ProcessRuntime::Process(process_runtime(None)),
                cancel_requested: Arc::new(AtomicBool::new(false)),
                audit: None,
                history_terminal_snapshot_at: None,
                event_completion_recorded: false,
            },
        );

        let first = list_processes_page(
            &state,
            ProcessListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: None,
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(first.processes.len(), 1);
        assert_eq!(first.processes[0].process_id, "process_persisted_new");
        let cursor = first.next_cursor.clone().expect("second page cursor");

        let running = list_processes_page(
            &state,
            ProcessListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: Some(ProcessState::Running),
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(running.processes.len(), 1);
        assert_eq!(running.processes[0].process_id, "process_persisted_old");
        assert_eq!(running.processes[0].state, ProcessState::Running);

        let completed = list_processes_page(
            &state,
            ProcessListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: Some(ProcessState::Completed),
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.processes.len(), 1);
        assert_eq!(completed.processes[0].process_id, "process_persisted_new");

        let second = list_processes_page(
            &state,
            ProcessListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: None,
                limit: Some(10),
                cursor: Some(cursor),
            },
        )
        .await
        .unwrap();
        assert_eq!(second.processes.len(), 1);
        assert_eq!(second.processes[0].process_id, "process_persisted_old");
        assert_eq!(second.processes[0].state, ProcessState::Running);
        assert!(second.next_cursor.is_none());
    }

    #[tokio::test]
    async fn degraded_history_fails_closed_before_process_effect() {
        let (state, workspace) = test_state(1).await;
        let mut state = state;
        state.process_history = crate::process_history::ProcessHistoryStore::disabled(
            workspace
                .join("missing-history-parent")
                .join("process.sqlite3"),
        );
        let request = exec_request("printf must-not-run", &workspace);
        let terminal = start_process_for_test(state.clone(), request).await;
        assert_eq!(terminal.state, ProcessState::Failed);
        assert!(terminal
            .reject_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("history_admission_failed:")));
        assert_eq!(
            state.process_history.health().status,
            crate::process_history::HistoryHealthStatus::Degraded
        );
        assert_eq!(state.process_history.health().pending_terminal_count, 0);
    }

    #[tokio::test]
    async fn skill_leases_still_block_updates() {
        let manager = crate::skills::SkillLeaseManager::new();
        let shared = manager.try_shared("demo").await.unwrap();
        assert!(manager
            .acquire_exclusive("demo", Duration::from_millis(20))
            .await
            .is_err());
        drop(shared);
        assert!(manager
            .acquire_exclusive("demo", Duration::from_millis(100))
            .await
            .is_ok());
    }
}
