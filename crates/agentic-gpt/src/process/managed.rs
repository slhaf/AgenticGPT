use std::path::Path;
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Weak,
};

use agentic_gpt_protocol::{
    normalize_process_group, EventOrigin, EventSource, ProcessBatchExecRequest,
    ProcessBatchResponse, ProcessCancelResponse, ProcessCaptureStatus, ProcessCursor,
    ProcessDetail, ProcessError, ProcessExecRequest, ProcessInfo, ProcessInlineOutput,
    ProcessInlineStream, ProcessKind, ProcessListItem, ProcessListRequest, ProcessListResponse,
    ProcessOutputEncoding, ProcessOutputGap, ProcessOutputPreview, ProcessOutputRequest,
    ProcessOutputResponse, ProcessOutputSegment, ProcessResponse, ProcessResultRequest,
    ProcessResultResponse, ProcessResultStatus, ProcessState, ProcessStatusRequest,
    ProcessStatusResponse,
};
use anyhow::Result;
use base64::{
    engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{Duration as ChronoDuration, Utc};
use rmcp::{model::RequestId, service::Peer, RoleClient};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
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
const PROCESS_INLINE_RESPONSE_BYTES: usize = 8 * 1024;
const PROCESS_INLINE_PREVIEW_BYTES: usize = 2 * 1024;

const TERMINAL_PROCESS_HOT_CACHE_MINUTES: i64 = 5;
const MAX_TERMINAL_PROCESSES: usize = 100;
const MAX_LIST_PROCESSES: usize = 100;
pub(crate) const MAX_MCP_ARGUMENT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_MCP_RESULT_BYTES: usize = 512 * 1024;
const MAX_MCP_RESULT_PREVIEW_BYTES: usize = 8 * 1024;
const MAX_PROCESS_ERROR_BYTES: usize = 8 * 1024;
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
}

pub(crate) struct ManagedProcessRuntime {
    child: Option<Child>,
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
    stdout_reader: Option<JoinHandle<ReaderOutcome>>,
    stderr_reader: Option<JoinHandle<ReaderOutcome>>,
    skill_lease: Option<SkillLease>,
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

pub(crate) struct ManagedProcessSpec {
    pub(crate) request: ProcessExecRequest,
    pub(crate) working_directory: std::path::PathBuf,
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

#[derive(Debug, Default)]
pub(crate) struct OutputRing {
    data: std::collections::VecDeque<u8>,
    start_offset: u64,
    end_offset: u64,
    capture: RingCapture,
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
    pub(crate) fn new() -> Self {
        Self {
            data: std::collections::VecDeque::with_capacity(PROCESS_OUTPUT_RING_CAPACITY),
            start_offset: 0,
            end_offset: 0,
            capture: RingCapture::NotStarted,
        }
    }

    fn mark_started(&mut self) {
        self.capture = RingCapture::Capturing;
    }

    fn push(&mut self, bytes: &[u8]) -> bool {
        let Some(end_offset) = self.end_offset.checked_add(bytes.len() as u64) else {
            self.capture = RingCapture::Failed("output_offset_overflow".to_string());
            return false;
        };
        self.end_offset = end_offset;
        self.data.extend(bytes.iter().copied());
        while self.data.len() > PROCESS_OUTPUT_RING_CAPACITY {
            self.data.pop_front();
        }
        self.start_offset = self.end_offset - self.data.len() as u64;
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
    }

    fn abort(&mut self, reason: &str) {
        self.capture = RingCapture::Failed(reason.to_string());
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
    start_managed_process_inner(state, request, config, None, options, None, None).await
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
    request: ProcessExecRequest,
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
        finalize_process(state, process).await;
        return Err(reason);
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
    let detail = process_detail(process);
    prune_terminal_processes(state, &mut processes);
    Ok(detail)
}

pub(crate) async fn mcp_process_response(
    state: &AppState,
    process_id: &str,
    wait_seconds: u64,
) -> Result<ProcessResponse, String> {
    let detail = get_process_detail(state, process_id, wait_seconds).await?;
    let completed_inline = detail.process.state.is_terminal();
    Ok(creation_response(state, detail.process, completed_inline).await)
}

fn bounded_error_message(value: String) -> String {
    if value.len() <= MAX_PROCESS_ERROR_BYTES {
        return value;
    }
    const SUFFIX: &str = "...[truncated]";
    let prefix_limit = MAX_PROCESS_ERROR_BYTES.saturating_sub(SUFFIX.len());
    format!("{}{}", utf8_prefix(&value, prefix_limit), SUFFIX)
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

pub(crate) async fn response(
    state: &AppState,
    process: ProcessInfo,
    completed_inline: bool,
) -> ProcessResponse {
    creation_response(state, process, completed_inline).await
}

async fn creation_response(
    state: &AppState,
    process: ProcessInfo,
    completed_inline: bool,
) -> ProcessResponse {
    let mut response = ProcessResponse {
        status: process.state,
        completed_inline,
        process: process.clone(),
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        inline_output: None,
        output_preview: None,
        result: None,
        result_status: None,
        result_available: false,
        result_bytes: None,
        result_sha256: None,
        result_preview: None,
        error: None,
    };
    let mut output_snapshot = None;
    if process.kind == ProcessKind::Mcp {
        if let Ok(detail) = get_process_detail(state, &process.process_id, 0).await {
            response.result_available = detail.result_available;
            response.result_bytes = detail.result_bytes;
            response.result_sha256 = detail.result_sha256.clone();
            response.result_preview = detail
                .result_preview
                .as_deref()
                .map(|preview| utf8_prefix(preview, PROCESS_INLINE_PREVIEW_BYTES).to_string());
            response.error = detail.error;
            if detail.result_available {
                response.result_status = Some(ProcessResultStatus::Complete);
                response.result = detail.result;
            } else if completed_inline {
                response.result_status = Some(
                    if detail
                        .result_bytes
                        .is_some_and(|bytes| bytes > MAX_MCP_RESULT_BYTES)
                    {
                        ProcessResultStatus::TooLarge
                    } else {
                        ProcessResultStatus::Unavailable
                    },
                );
            }
        } else {
            response.result_status = Some(ProcessResultStatus::Unavailable);
        }
    } else if let Ok(read) = process_output_for(state, &process.process_id).await {
        let output = read.snapshot;
        let capture_complete = read.capture_status == ProcessCaptureStatus::Complete;
        let retained_from_start =
            output.stdout_start_offset == 0 && output.stderr_start_offset == 0;
        if completed_inline && capture_complete && retained_from_start {
            response.inline_output = Some(ProcessInlineOutput {
                stdout: inline_stream(&output.stdout),
                stderr: inline_stream(&output.stderr),
            });
        } else if !output.stdout.is_empty()
            || !output.stderr.is_empty()
            || read.capture_status != ProcessCaptureStatus::Complete
            || !retained_from_start
        {
            response.output_preview =
                Some(output_preview(&output, PROCESS_INLINE_PREVIEW_BYTES, true));
        }
        output_snapshot = Some(output);
    }
    if process.kind != ProcessKind::Mcp && completed_inline && response.inline_output.is_none() {
        response.completed_inline = false;
        response.poll_after_ms = 1_000;
    }
    if serialized_size(&response) > PROCESS_INLINE_RESPONSE_BYTES {
        response.completed_inline = false;
        response.poll_after_ms = 1_000;
        if response.inline_output.take().is_some() {
            if let Some(output) = output_snapshot.as_ref() {
                response.output_preview =
                    Some(output_preview(output, PROCESS_INLINE_PREVIEW_BYTES, true));
            }
        }
        if let Some(result) = response.result.take() {
            response.result_status = Some(ProcessResultStatus::TooLarge);
            let serialized = serde_json::to_string(&result).unwrap_or_default();
            response.result_preview =
                Some(utf8_prefix(&serialized, PROCESS_INLINE_PREVIEW_BYTES).to_string());
        }
    }
    fit_process_response(&mut response);
    response
}

fn inline_stream(bytes: &[u8]) -> ProcessInlineStream {
    match std::str::from_utf8(bytes) {
        Ok(text) => ProcessInlineStream {
            data: text.to_string(),
            encoding: ProcessOutputEncoding::Utf8,
        },
        Err(_) => ProcessInlineStream {
            data: BASE64.encode(bytes),
            encoding: ProcessOutputEncoding::Base64,
        },
    }
}

fn output_preview(
    output: &crate::process_history::ProcessOutputSnapshot,
    max_bytes: usize,
    truncated: bool,
) -> ProcessOutputPreview {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let first = max_bytes / 2;
    let mut stdout_budget = stdout.len().min(first);
    let mut stderr_budget = stderr.len().min(max_bytes - first);
    let mut spare = max_bytes.saturating_sub(stdout_budget + stderr_budget);
    let more_stdout = stdout.len().saturating_sub(stdout_budget).min(spare);
    stdout_budget += more_stdout;
    spare -= more_stdout;
    stderr_budget += stderr.len().saturating_sub(stderr_budget).min(spare);
    let stdout_value = utf8_prefix(&stdout, stdout_budget).to_string();
    let stderr_value = utf8_prefix(&stderr, stderr_budget).to_string();
    ProcessOutputPreview {
        truncated: truncated
            || stdout_value.len() < stdout.len()
            || stderr_value.len() < stderr.len()
            || output.stdout_start_offset > 0
            || output.stderr_start_offset > 0,
        stdout: stdout_value,
        stderr: stderr_value,
    }
}

fn serialized_size<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

fn fit_process_response(response: &mut ProcessResponse) {
    if serialized_size(response) <= PROCESS_INLINE_RESPONSE_BYTES {
        return;
    }
    response.completed_inline = false;
    response.poll_after_ms = 1_000;
    compact_process_response(response);
    if serialized_size(response) > PROCESS_INLINE_RESPONSE_BYTES {
        response.output_preview = None;
        response.result_preview = None;
        response.error = None;
        response.process.reject_reason = None;
        response.process.command_preview = None;
        response.process.working_directory = None;
        response.process.skill_path = None;
        response.process.capture_error = None;
    }
}

fn compact_process_response(response: &mut ProcessResponse) {
    response.process.args.clear();
    response.process.agent_id = utf8_prefix(&response.process.agent_id, 256).to_string();
    response.process.program = response
        .process
        .program
        .as_deref()
        .map(|value| utf8_prefix(value, 256).to_string());
    response.process.command_preview = response
        .process
        .command_preview
        .as_deref()
        .map(|value| utf8_prefix(value, 256).to_string());
    response.process.working_directory = response
        .process
        .working_directory
        .as_deref()
        .map(|value| utf8_prefix(value, 256).to_string());
    response.process.reject_reason = response
        .process
        .reject_reason
        .as_deref()
        .map(|value| utf8_prefix(value, 512).to_string());
    response.process.skill_path = response
        .process
        .skill_path
        .as_deref()
        .map(|value| utf8_prefix(value, 256).to_string());
    response.process.mcp_server_id = response
        .process
        .mcp_server_id
        .as_deref()
        .map(|value| utf8_prefix(value, 256).to_string());
    response.process.mcp_tool_name = response
        .process
        .mcp_tool_name
        .as_deref()
        .map(|value| utf8_prefix(value, 256).to_string());
    response.process.capture_error = response
        .process
        .capture_error
        .as_deref()
        .map(|value| utf8_prefix(value, 512).to_string());
    if let Some(error) = response.error.as_mut() {
        error.code = utf8_prefix(&error.code, 128).to_string();
        error.message = utf8_prefix(&error.message, 512).to_string();
    }
    response.result_preview = response
        .result_preview
        .as_deref()
        .map(|value| utf8_prefix(value, 512).to_string());
}

pub(crate) async fn start_and_wait_process(
    state: AppState,
    request: ProcessExecRequest,
    options: ProcessOptions,
) -> ProcessResponse {
    let wait_seconds = request.effective_wait_seconds();
    let process = start_managed_process(state.clone(), request, options).await;
    let process = wait_for_process(&state, process, wait_seconds).await;
    response(&state, process.clone(), process.state.is_terminal()).await
}

pub(crate) async fn start_and_wait_skill_process(
    state: AppState,
    request: ProcessExecRequest,
    skill_id: &str,
    skill_path: &str,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    event_origin: Option<EventOrigin>,
) -> ProcessResponse {
    let wait_seconds = request.effective_wait_seconds();
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
    let process = wait_for_process(&state, process, wait_seconds).await;
    response(&state, process.clone(), process.state.is_terminal()).await
}

pub(crate) async fn start_process_batch(
    state: AppState,
    request: ProcessBatchExecRequest,
    request_source: String,
    terminal_event_hook: Option<TerminalEventHook>,
    event_origin: Option<EventOrigin>,
) -> Result<ProcessBatchResponse, String> {
    let wait_seconds = request.effective_wait_seconds();
    let batch_id = format!("batch_{}", uuid::Uuid::new_v4().simple());
    let group = validated_group(request.group.as_deref())?;
    if request.elements.is_empty() {
        return Ok(ProcessBatchResponse {
            batch_id,
            status: "completed".to_string(),
            completed_inline: true,
            poll_after_ms: 0,
            processes: Vec::new(),
        });
    }
    ensure_process_batch_response_fits(
        &batch_id,
        &request.agent_id,
        &state.boot_generation,
        request.elements.len(),
    )?;
    let config = Arc::new(state.config.read().await.clone());
    let mut prepared = Vec::with_capacity(request.elements.len());
    for (index, element) in request.elements.into_iter().enumerate() {
        let working_directory = element
            .working_directory
            .clone()
            .or_else(|| request.working_directory.clone());
        let decision = policy_decision_for_profile(
            &config,
            state.runtime.profile,
            &element.program,
            &element.args,
            request.need_confirm,
        );
        let resolved_working_directory =
            exec::resolve_working_directory(&config, working_directory.as_deref())?;
        exec::preflight(
            &config,
            &resolved_working_directory,
            &element.program,
            &element.args,
        )?;
        if decision == PolicyDecision::Deny {
            return Err(format!(
                "batch_element_rejected; index={index}; reason=policy_denied"
            ));
        }
        prepared.push(exec::PreparedBatchElement {
            index,
            program: element.program,
            args: element.args,
            working_directory,
            resolved_working_directory,
            decision,
        });
    }
    let needs_confirmation = prepared
        .iter()
        .filter(|element| element.decision == PolicyDecision::Confirm)
        .cloned()
        .collect::<Vec<_>>();
    let confirmation_result = if needs_confirmation.is_empty() {
        None
    } else {
        let result = confirmation::request_batch_confirmation(
            &state,
            &config,
            request.confirm_method.as_deref(),
            &needs_confirmation,
            &prepared,
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
            request: ProcessExecRequest {
                agent_id: request.agent_id.clone(),
                group: group.clone(),
                program: element.program,
                args: element.args,
                need_confirm: request.need_confirm,
                confirm_method: request.confirm_method.clone(),
                working_directory: element.working_directory,
                wait_seconds: request.wait_seconds,
            },
            working_directory: element.resolved_working_directory,
            decision: element.decision,
            confirmation_result: confirmation_result.clone(),
            request_source: request_source.clone(),
            terminal_event_hook: terminal_event_hook.clone(),
            event_origin: event_origin.clone(),
        })
        .collect::<Vec<_>>();
    let mut processes = start_prepared_managed_batch(state.clone(), config, specs).await?;
    let deadline = Instant::now() + std::time::Duration::from_secs(wait_seconds);
    loop {
        let mut all_terminal = true;
        for process in &mut processes {
            if let Ok(latest) = get_process(&state, &process.process_id, 0).await {
                *process = latest;
            }
            all_terminal &= process.state.is_terminal();
        }
        if all_terminal || Instant::now() >= deadline {
            break;
        }
        sleep(std::time::Duration::from_millis(20)).await;
    }
    let all_terminal = processes.iter().all(|process| process.state.is_terminal());
    let status = if !all_terminal {
        "running"
    } else if processes
        .iter()
        .any(|process| process.state != ProcessState::Completed)
    {
        "completed_with_errors"
    } else {
        "completed"
    };
    let mut items = Vec::with_capacity(processes.len());
    for process in processes {
        let child_completed = process.state.is_terminal();
        items.push(creation_response(&state, process, child_completed).await);
    }
    let completed_inline = all_terminal && items.iter().all(|process| process.completed_inline);
    Ok(fit_process_batch_response(ProcessBatchResponse {
        batch_id,
        status: status.to_string(),
        completed_inline,
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        processes: items,
    }))
}

fn fit_process_batch_response(mut response: ProcessBatchResponse) -> ProcessBatchResponse {
    let preview_bytes = response
        .processes
        .iter()
        .map(|process| {
            process
                .output_preview
                .as_ref()
                .map(|preview| preview.stdout.len() + preview.stderr.len())
                .unwrap_or(0)
                + process
                    .result_preview
                    .as_ref()
                    .map(String::len)
                    .unwrap_or(0)
        })
        .sum::<usize>();
    if serialized_size(&response) <= PROCESS_INLINE_RESPONSE_BYTES
        && preview_bytes <= PROCESS_INLINE_PREVIEW_BYTES
    {
        return response;
    }
    response.completed_inline = false;
    response.poll_after_ms = 1_000;
    let mut preview_budget = PROCESS_INLINE_PREVIEW_BYTES;
    for process in &mut response.processes {
        let mut output_preview_generated = false;
        let mut result_preview_generated = false;
        if let Some(inline) = process.inline_output.take() {
            process.completed_inline = false;
            process.poll_after_ms = 1_000;
            let stdout = decode_inline_stream(&inline.stdout).unwrap_or_default();
            let stderr = decode_inline_stream(&inline.stderr).unwrap_or_default();
            let output = crate::process_history::ProcessOutputSnapshot {
                stdout_end_offset: stdout.len() as u64,
                stderr_end_offset: stderr.len() as u64,
                stdout,
                stderr,
                ..crate::process_history::ProcessOutputSnapshot::default()
            };
            if preview_budget > 0 {
                let preview = output_preview(&output, preview_budget, true);
                preview_budget =
                    preview_budget.saturating_sub(preview.stdout.len() + preview.stderr.len());
                process.output_preview = Some(preview);
                output_preview_generated = true;
            }
        }
        if let Some(result) = process.result.take() {
            process.completed_inline = false;
            process.poll_after_ms = 1_000;
            process.result_status = Some(ProcessResultStatus::TooLarge);
            let text = serde_json::to_string(&result).unwrap_or_default();
            if preview_budget > 0 {
                let preview = utf8_prefix(&text, preview_budget).to_string();
                preview_budget = preview_budget.saturating_sub(preview.len());
                process.result_preview = Some(preview);
                result_preview_generated = true;
            }
        }
        if !output_preview_generated {
            if let Some(preview) = process.output_preview.as_mut() {
                let limited = limit_output_preview(preview, preview_budget);
                preview_budget =
                    preview_budget.saturating_sub(limited.stdout.len() + limited.stderr.len());
                *preview = limited;
            }
        }
        if !result_preview_generated {
            if let Some(preview) = process.result_preview.as_mut() {
                let limited = utf8_prefix(preview, preview_budget).to_string();
                preview_budget = preview_budget.saturating_sub(limited.len());
                *preview = limited;
            }
        }
    }

    if serialized_size(&response) > PROCESS_INLINE_RESPONSE_BYTES {
        for process in &mut response.processes {
            compact_batch_process_response(process);
        }
    }
    debug_assert!(serialized_size(&response) <= PROCESS_INLINE_RESPONSE_BYTES);
    response
}

fn compact_batch_process_response(response: &mut ProcessResponse) {
    compact_process_response(response);
    response.completed_inline = false;
    response.poll_after_ms = 1_000;
    response.inline_output = None;
    response.output_preview = None;
    response.result = None;
    response.result_status = None;
    response.result_available = false;
    response.result_bytes = None;
    response.result_sha256 = None;
    response.result_preview = None;
    response.error = None;

    let process = &mut response.process;
    process.agent_id = utf8_prefix(&process.agent_id, 64).to_string();
    process.group = None;
    process.batch_id = None;
    process.batch_call_id = None;
    process.batch_index = None;
    process.started_at = None;
    process.finished_at = None;
    process.program = None;
    process.args.clear();
    process.working_directory = None;
    process.command_preview = None;
    process.exit_code = None;
    process.reject_reason = None;
    process.skill_id = None;
    process.skill_path = None;
    process.installed_digest = None;
    process.mcp_server_id = None;
    process.mcp_tool_name = None;
    process.cancel_requested = false;
    process.cancel_outcome = None;
    process.termination_evidence = None;
    process.capture_error = None;
}

fn ensure_process_batch_response_fits(
    batch_id: &str,
    agent_id: &str,
    boot_generation: &str,
    process_count: usize,
) -> Result<(), String> {
    let now = Utc::now();
    let process = ProcessInfo {
        agent_id: utf8_prefix(agent_id, 64).to_string(),
        process_id: format!("process_{}_{}", boot_generation, "x".repeat(32)),
        group: None,
        batch_id: None,
        batch_call_id: None,
        batch_index: None,
        kind: ProcessKind::Command,
        state: ProcessState::UnknownAfterRestart,
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
        mcp_server_id: None,
        mcp_tool_name: None,
        cancel_requested: false,
        cancel_outcome: None,
        termination_evidence: None,
        capture_status: ProcessCaptureStatus::NotApplicable,
        capture_error: None,
    };
    let mut item = ProcessResponse {
        status: process.state,
        completed_inline: false,
        process,
        poll_after_ms: 1_000,
        inline_output: None,
        output_preview: None,
        result: None,
        result_status: None,
        result_available: false,
        result_bytes: None,
        result_sha256: None,
        result_preview: None,
        error: None,
    };
    compact_batch_process_response(&mut item);
    let item_bytes = serialized_size(&item);
    let response = ProcessBatchResponse {
        batch_id: batch_id.to_string(),
        status: "completed_with_errors".to_string(),
        completed_inline: false,
        poll_after_ms: 1_000,
        processes: vec![item],
    };
    let additional_items = process_count.saturating_sub(1);
    let size = serialized_size(&response)
        .saturating_add(additional_items.saturating_mul(item_bytes.saturating_add(1)))
        .saturating_add(process_count.saturating_mul(64));
    if size > PROCESS_INLINE_RESPONSE_BYTES {
        return Err("process_batch_response_too_large".to_string());
    }
    Ok(())
}

fn decode_inline_stream(stream: &ProcessInlineStream) -> Option<Vec<u8>> {
    match stream.encoding {
        ProcessOutputEncoding::Utf8 => Some(stream.data.as_bytes().to_vec()),
        ProcessOutputEncoding::Base64 => BASE64.decode(&stream.data).ok(),
    }
}

fn limit_output_preview(preview: &ProcessOutputPreview, max_bytes: usize) -> ProcessOutputPreview {
    let output = crate::process_history::ProcessOutputSnapshot {
        stdout: preview.stdout.as_bytes().to_vec(),
        stderr: preview.stderr.as_bytes().to_vec(),
        stdout_end_offset: preview.stdout.len() as u64,
        stderr_end_offset: preview.stderr.len() as u64,
        ..crate::process_history::ProcessOutputSnapshot::default()
    };
    let mut limited = output_preview(&output, max_bytes, preview.truncated);
    limited.truncated |=
        limited.stdout.len() < preview.stdout.len() || limited.stderr.len() < preview.stderr.len();
    limited
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
            let info = process_info(
                &spec.request,
                process_id,
                ProcessKind::Command,
                ProcessState::Queued,
                now,
                None,
            );
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
                child: None,
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
    request: ProcessExecRequest,
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
    request: &ProcessExecRequest,
    process_id: String,
    kind: ProcessKind,
    state: ProcessState,
    now: chrono::DateTime<Utc>,
    options: Option<&ProcessOptions>,
) -> ProcessInfo {
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
        program: Some(request.program.clone()),
        args: request.args.clone(),
        working_directory: request.working_directory.clone(),
        command_preview: Some(command_preview(&request.program, &request.args)),
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
    ManagedProcessRuntime {
        child: None,
        stdout: Arc::new(Mutex::new(OutputRing::new())),
        stderr: Arc::new(Mutex::new(OutputRing::new())),
        stdout_reader: None,
        stderr_reader: None,
        skill_lease,
    }
}

/// `config` is the admission snapshot; workers must not reload live state here.
#[allow(clippy::too_many_arguments)]
async fn run_async_process(
    state: AppState,
    process_id: String,
    config: Arc<Config>,
    request: ProcessExecRequest,
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
    prepared: Option<(std::path::PathBuf, PolicyDecision)>,
    prepared_confirmation_result: Option<String>,
) {
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
        set_policy_decision(&state, &process_id, format!("{decision:?}")).await;
        let working_directory =
            match exec::resolve_working_directory(&config, request.working_directory.as_deref()) {
                Ok(directory) => directory,
                Err(reason) => {
                    finish_process(&state, &process_id, ProcessState::Rejected, &reason).await;
                    return;
                }
            };
        if let Err(reason) =
            exec::preflight(&config, &working_directory, &request.program, &request.args)
        {
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
    drop(processes);

    let spawned = spawn_process_with_readers(
        &config,
        &working_directory,
        &request.program,
        &request.args,
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
                let _ = spawned.child.kill().await;
                settle_reader(spawned.stdout_reader, spawned.stdout).await;
                settle_reader(spawned.stderr_reader, spawned.stderr).await;
            }
            return;
        };
        let ProcessRuntime::Process(runtime) = &mut process.runtime else {
            drop(processes);
            if let Some(mut spawned) = spawned.take() {
                let _ = spawned.child.kill().await;
                settle_reader(spawned.stdout_reader, spawned.stdout).await;
                settle_reader(spawned.stderr_reader, spawned.stderr).await;
            }
            return;
        };
        let spawned = spawned.take().expect("spawn result is present");
        runtime.child = Some(spawned.child);
        runtime.stdout_reader = spawned.stdout_reader;
        runtime.stderr_reader = spawned.stderr_reader;
        process.info.capture_status = ProcessCaptureStatus::Capturing;
        cancel_requested.load(std::sync::atomic::Ordering::Acquire)
            || !process.info.state.is_active()
    };
    if cancel_now {
        let _ = cancel_command_process(&state, &process_id).await;
    }
}

struct SpawnedProcess {
    child: Child,
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
    stdout_reader: Option<JoinHandle<ReaderOutcome>>,
    stderr_reader: Option<JoinHandle<ReaderOutcome>>,
}

async fn spawn_process_with_readers(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
    stdout: Arc<Mutex<OutputRing>>,
    stderr: Arc<Mutex<OutputRing>>,
) -> Result<SpawnedProcess> {
    let mut command = exec::build_command(config, working_directory, program)?;
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
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
        stdout,
        stderr,
        stdout_reader,
        stderr_reader,
    })
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
    let Some(reader) = reader else {
        let mut ring = ring.lock().await;
        if matches!(ring.capture, RingCapture::Capturing) {
            ring.abort("reader_handle_missing");
        }
        return;
    };
    let outcome = match reader.await {
        Ok(outcome) => outcome,
        Err(error) => ReaderOutcome::Failed(format!("reader_task_failed: {error}")),
    };
    ring.lock().await.finish(outcome);
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

fn capture_summary(
    stdout: &OutputRing,
    stderr: &OutputRing,
) -> (ProcessCaptureStatus, Option<String>) {
    let settled = stdout.is_settled() && stderr.is_settled();
    let status = if settled && (stdout.is_failed() || stderr.is_failed()) {
        ProcessCaptureStatus::Incomplete
    } else if settled {
        ProcessCaptureStatus::Complete
    } else if stdout.capture_not_started() && stderr.capture_not_started() {
        ProcessCaptureStatus::NotStarted
    } else {
        ProcessCaptureStatus::Capturing
    };
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
    let stdout = runtime.stdout.lock().await.snapshot();
    let stderr = runtime.stderr.lock().await.snapshot();
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
    mut info: ProcessInfo,
    wait_seconds: u64,
) -> ProcessInfo {
    if wait_seconds == 0 {
        return info;
    }
    let deadline = Instant::now() + std::time::Duration::from_secs(wait_seconds.min(30));
    while info.state.is_active() && Instant::now() < deadline {
        sleep(std::time::Duration::from_millis(20)).await;
        if let Some(latest) = get_process_now(state, &info.process_id).await {
            info = latest;
        } else {
            break;
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
pub(crate) async fn get_process_status(
    state: &AppState,
    request: ProcessStatusRequest,
) -> Result<ProcessStatusResponse, String> {
    let started = Instant::now();
    let process = get_process(state, &request.process_id, request.effective_wait_seconds()).await?;
    Ok(ProcessStatusResponse {
        process,
        wait_elapsed_ms: Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64),
    })
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

pub(crate) async fn get_process_output(
    state: &AppState,
    request: ProcessOutputRequest,
) -> Result<ProcessOutputResponse, String> {
    let info = get_process(state, &request.process_id, 0).await?;
    let ProcessOutputRead {
        snapshot: output,
        capture_status: output_capture_status,
    } = process_output_for(state, &request.process_id).await?;
    let max_bytes = request.effective_max_bytes();
    let cursor = match request.cursor.as_deref() {
        Some(encoded) => decode_output_cursor(encoded, &request.process_id)?,
        None => ProcessCursor {
            version: 1,
            process_id: request.process_id.clone(),
            stdout_offset: 0,
            stderr_offset: 0,
        },
    };
    if cursor.stdout_offset > output.stdout_end_offset
        || cursor.stderr_offset > output.stderr_end_offset
    {
        return Err("process_output_cursor_ahead_of_output".to_string());
    }
    let stdout_budget = max_bytes / 2;
    let stderr_budget = max_bytes - stdout_budget;
    let (mut stdout, mut stdout_next, stdout_used) = output_segment(
        &output.stdout,
        output.stdout_start_offset,
        output.stdout_end_offset,
        cursor.stdout_offset,
        stdout_budget,
    )?;
    let (mut stderr, mut stderr_next, stderr_used) = output_segment(
        &output.stderr,
        output.stderr_start_offset,
        output.stderr_end_offset,
        cursor.stderr_offset,
        stderr_budget,
    )?;
    let spare = max_bytes.saturating_sub(stdout_used + stderr_used);
    if stdout_next == output.stdout_end_offset
        && stderr_next < output.stderr_end_offset
        && spare > 0
    {
        let (segment, next_offset, _) = output_segment(
            &output.stderr,
            output.stderr_start_offset,
            output.stderr_end_offset,
            cursor.stderr_offset,
            max_bytes.saturating_sub(stdout_used),
        )?;
        stderr = segment;
        stderr_next = next_offset;
    } else if stderr_next == output.stderr_end_offset
        && stdout_next < output.stdout_end_offset
        && spare > 0
    {
        let (segment, next_offset, _) = output_segment(
            &output.stdout,
            output.stdout_start_offset,
            output.stdout_end_offset,
            cursor.stdout_offset,
            max_bytes.saturating_sub(stderr_used),
        )?;
        stdout = segment;
        stdout_next = next_offset;
    }
    if stdout_next == cursor.stdout_offset && stderr_next == cursor.stderr_offset {
        if cursor.stdout_offset < output.stdout_end_offset {
            let (segment, next_offset, _) = output_segment(
                &output.stdout,
                output.stdout_start_offset,
                output.stdout_end_offset,
                cursor.stdout_offset,
                max_bytes,
            )?;
            stdout = segment;
            stdout_next = next_offset;
        } else if cursor.stderr_offset < output.stderr_end_offset {
            let (segment, next_offset, _) = output_segment(
                &output.stderr,
                output.stderr_start_offset,
                output.stderr_end_offset,
                cursor.stderr_offset,
                max_bytes,
            )?;
            stderr = segment;
            stderr_next = next_offset;
        }
    }
    if stdout_next == cursor.stdout_offset
        && stderr_next == cursor.stderr_offset
        && (cursor.stdout_offset < output.stdout_end_offset
            || cursor.stderr_offset < output.stderr_end_offset)
    {
        return Err("process_output_max_bytes_too_small_for_next_unit".to_string());
    }
    let has_more = stdout_next < output.stdout_end_offset || stderr_next < output.stderr_end_offset;
    let capture_status = if info.kind == ProcessKind::Mcp {
        ProcessCaptureStatus::NotApplicable
    } else {
        output_capture_status
    };
    let eof = !has_more
        && matches!(
            capture_status,
            ProcessCaptureStatus::Complete | ProcessCaptureStatus::NotApplicable
        );
    let next_cursor = encode_output_cursor(ProcessCursor {
        version: 1,
        process_id: request.process_id.clone(),
        stdout_offset: stdout_next,
        stderr_offset: stderr_next,
    })?;
    Ok(ProcessOutputResponse {
        process_id: request.process_id,
        stdout,
        stderr,
        next_cursor,
        has_more,
        eof,
        capture_status,
    })
}

pub(crate) async fn get_process_result(
    state: &AppState,
    request: ProcessResultRequest,
) -> Result<ProcessResultResponse, String> {
    let detail = get_process_detail(state, &request.process_id, 0).await?;
    let unavailable = |code: &str, message: &str| ProcessResultResponse {
        process_id: request.process_id.clone(),
        status: ProcessResultStatus::Unavailable,
        result_available: false,
        result: None,
        error: Some(ProcessError {
            code: code.to_string(),
            message: message.to_string(),
        }),
        result_bytes: detail.result_bytes,
        result_sha256: detail.result_sha256.clone(),
        result_preview: detail.result_preview.clone(),
    };
    let result_error = detail.error.clone();
    let result_sha256 = detail.result_sha256.clone();
    let result_preview = detail.result_preview.clone();
    let process_id = request.process_id.clone();
    let too_large = |result_bytes| ProcessResultResponse {
        process_id: process_id.clone(),
        status: ProcessResultStatus::TooLarge,
        result_available: false,
        result: None,
        error: result_error.clone().or_else(|| {
            Some(ProcessError {
                code: "process_result_not_retained".to_string(),
                message: "Result exceeded the retained result limit".to_string(),
            })
        }),
        result_bytes: Some(result_bytes),
        result_sha256: result_sha256.clone(),
        result_preview: result_preview.clone(),
    };
    if detail.process.kind != ProcessKind::Mcp {
        return Ok(unavailable(
            "process_result_not_applicable",
            "Process does not produce a structured MCP result",
        ));
    }
    if detail.process.state.is_active() {
        return Ok(unavailable(
            "process_result_not_ready",
            "Process has not completed",
        ));
    }
    if let Some(result_bytes) = detail
        .result_bytes
        .filter(|bytes| *bytes > MAX_MCP_RESULT_BYTES)
    {
        return Ok(too_large(result_bytes));
    }
    if !detail.detail_available {
        return Ok(unavailable(
            "process_result_unavailable",
            "Retained process result is unavailable",
        ));
    }
    if !detail.result_available {
        let error = detail.error.or_else(|| {
            Some(ProcessError {
                code: "process_result_unavailable".to_string(),
                message: "No structured result was retained for this process".to_string(),
            })
        });
        return Ok(ProcessResultResponse {
            process_id: request.process_id,
            status: ProcessResultStatus::Unavailable,
            result_available: false,
            result: None,
            error,
            result_bytes: detail.result_bytes,
            result_sha256: detail.result_sha256,
            result_preview: detail.result_preview,
        });
    }
    let Some(result) = detail.result else {
        return Ok(unavailable(
            "process_result_unavailable",
            "Retained process result is unavailable",
        ));
    };
    let encoded =
        serde_json::to_vec(&result).map_err(|_| "process_result_encode_failed".to_string())?;
    let result_bytes = detail.result_bytes.unwrap_or(encoded.len());
    if result_bytes > MAX_MCP_RESULT_BYTES {
        return Ok(too_large(result_bytes));
    }
    let max_bytes = request.effective_max_bytes();
    let status = if result_bytes > max_bytes {
        ProcessResultStatus::TooLarge
    } else {
        ProcessResultStatus::Complete
    };
    Ok(ProcessResultResponse {
        process_id: request.process_id,
        status,
        result_available: true,
        result: (result_bytes <= max_bytes).then_some(result),
        error: detail.error,
        result_bytes: Some(result_bytes),
        result_sha256: detail.result_sha256,
        result_preview: if result_bytes <= max_bytes {
            None
        } else {
            detail.result_preview
        },
    })
}

async fn process_output_for(
    state: &AppState,
    process_id: &str,
) -> Result<ProcessOutputRead, String> {
    let live = {
        let processes = state.processes.lock().await;
        processes
            .get(process_id)
            .map(|process| match &process.runtime {
                ProcessRuntime::Process(runtime) => {
                    Some((runtime.stdout.clone(), runtime.stderr.clone()))
                }
                ProcessRuntime::Mcp(_) => None,
            })
    };
    if let Some(live) = live {
        if let Some((stdout_ring, stderr_ring)) = live {
            let (stdout_guard, stderr_guard) = tokio::join!(stdout_ring.lock(), stderr_ring.lock());
            let (stdout, stdout_start_offset, stdout_end_offset) = stdout_guard.snapshot();
            let (stderr, stderr_start_offset, stderr_end_offset) = stderr_guard.snapshot();
            let (capture_status, capture_error) = capture_summary(&stdout_guard, &stderr_guard);
            let _ = capture_error;
            return Ok(ProcessOutputRead {
                snapshot: crate::process_history::ProcessOutputSnapshot {
                    stdout,
                    stdout_start_offset,
                    stdout_end_offset,
                    stderr,
                    stderr_start_offset,
                    stderr_end_offset,
                },
                capture_status,
            });
        }
        return Ok(ProcessOutputRead {
            snapshot: crate::process_history::ProcessOutputSnapshot::default(),
            capture_status: ProcessCaptureStatus::NotApplicable,
        });
    }
    match state.process_history.get(process_id) {
        Ok(Some(record)) if record.info.state.is_terminal() => Ok(ProcessOutputRead {
            snapshot: record.output,
            capture_status: record.info.capture_status,
        }),
        _ => Err(missing_process_reason(state, process_id)),
    }
}

struct ProcessOutputRead {
    snapshot: crate::process_history::ProcessOutputSnapshot,
    capture_status: ProcessCaptureStatus,
}

fn encode_output_cursor(cursor: ProcessCursor) -> Result<String, String> {
    serde_json::to_vec(&cursor)
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
        .map_err(|_| "process_output_cursor_encode_failed".to_string())
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

fn output_segment(
    data: &[u8],
    retained_start: u64,
    end_offset: u64,
    requested_offset: u64,
    max_encoded_bytes: usize,
) -> Result<(ProcessOutputSegment, u64, usize), String> {
    if retained_start > end_offset || end_offset - retained_start != data.len() as u64 {
        return Err("process_output_snapshot_invalid".to_string());
    }
    let data_start = requested_offset.max(retained_start);
    let gap = (requested_offset < retained_start).then(|| ProcessOutputGap {
        start_offset: requested_offset.to_string(),
        end_offset: retained_start.to_string(),
    });
    let start_index = usize::try_from(data_start - retained_start)
        .map_err(|_| "process_output_snapshot_invalid".to_string())?;
    let available = &data[start_index..];
    let (payload, encoding, raw_bytes) = encode_output_payload(available, max_encoded_bytes);
    let next_offset = data_start + raw_bytes as u64;
    let encoded_bytes = payload.len();
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

fn encode_output_payload(
    data: &[u8],
    max_encoded_bytes: usize,
) -> (String, ProcessOutputEncoding, usize) {
    if data.is_empty() || max_encoded_bytes == 0 {
        return (String::new(), ProcessOutputEncoding::Utf8, 0);
    }
    let candidate = &data[..data.len().min(max_encoded_bytes)];
    match std::str::from_utf8(candidate) {
        Ok(text) => (
            text.to_string(),
            ProcessOutputEncoding::Utf8,
            candidate.len(),
        ),
        Err(error) if error.error_len().is_none() && error.valid_up_to() > 0 => {
            let valid = &candidate[..error.valid_up_to()];
            (
                std::str::from_utf8(valid)
                    .expect("valid prefix")
                    .to_string(),
                ProcessOutputEncoding::Utf8,
                valid.len(),
            )
        }
        Err(_) => {
            let raw_limit = (max_encoded_bytes / 4).saturating_mul(3);
            if raw_limit == 0 {
                return (String::new(), ProcessOutputEncoding::Base64, 0);
            }
            let raw = &data[..data.len().min(raw_limit)];
            (BASE64.encode(raw), ProcessOutputEncoding::Base64, raw.len())
        }
    }
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
    let child = {
        let mut processes = state.processes.lock().await;
        let Some(process) = processes.get_mut(process_id) else {
            return Err(missing_process_reason(state, process_id));
        };
        refresh_process(state, process).await;
        if process.info.state.is_terminal() {
            let detail = process_detail(process);
            prune_terminal_processes(state, &mut processes);
            return Ok(detail);
        }
        process
            .cancel_requested
            .store(true, std::sync::atomic::Ordering::Release);
        process.info.cancel_requested = true;
        process.info.state = ProcessState::CancelRequested;
        process.info.updated_at = Utc::now();
        let ProcessRuntime::Process(runtime) = &mut process.runtime else {
            return Err("process_kind_mismatch".to_string());
        };
        match runtime.child.take() {
            Some(child) => Some(child),
            None => {
                process.info.cancel_outcome = Some("cancel_requested".to_string());
                process.info.termination_evidence =
                    Some("cancel_flag_before_process_start".to_string());
                runtime.skill_lease = None;
                let detail = process_detail(process);
                prune_terminal_processes(state, &mut processes);
                return Ok(detail);
            }
        }
    };

    let Some(mut child) = child else {
        return Err("process_cancel_internal".to_string());
    };
    let kill_result = child.kill().await;
    let mut processes = state.processes.lock().await;
    let Some(process) = processes.get_mut(process_id) else {
        return Err("process_not_found".to_string());
    };
    let ProcessRuntime::Process(runtime) = &mut process.runtime else {
        return Err("process_kind_changed".to_string());
    };
    match kill_result {
        Ok(()) => {
            mark_cancelled(
                &mut process.info,
                "cancelled",
                "local_process_kill_completed",
            );
            runtime.skill_lease = None;
        }
        Err(_) => match child.try_wait() {
            Ok(Some(status)) => {
                let now = Utc::now();
                process.info.exit_code = status.code();
                process.info.state = if status.success() {
                    ProcessState::Completed
                } else {
                    ProcessState::Failed
                };
                process.info.updated_at = now;
                process.info.finished_at = Some(now);
                process.info.cancel_outcome = Some("already_terminal".to_string());
                process.info.termination_evidence = Some("process_exit_status".to_string());
                runtime.skill_lease = None;
            }
            _ => {
                process.info.state = ProcessState::CancelRequested;
                process.info.updated_at = Utc::now();
                process.info.cancel_outcome = Some("cancel_failed".to_string());
                process.info.termination_evidence = Some("process_kill_error".to_string());
                runtime.child = Some(child);
            }
        },
    }
    if process.info.state.is_terminal() {
        finalize_process(state, process).await;
    }
    let detail = process_detail(process);
    prune_terminal_processes(state, &mut processes);
    Ok(detail)
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
            finalize_process(state, process).await;
            let detail = process_detail(process);
            prune_terminal_processes(state, &mut processes);
            return Ok(detail);
        }
        process.info.state = ProcessState::CancelRequested;
        process.info.updated_at = Utc::now();
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
        let child_status = runtime
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten());
        if let Some(status) = child_status {
            runtime.child = None;
            if !process.info.state.is_terminal() {
                let now = Utc::now();
                process.info.exit_code = status.code();
                if process
                    .cancel_requested
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    process.info.state = ProcessState::Cancelled;
                    process.info.reject_reason = Some("cancelled".to_string());
                    process.info.cancel_requested = true;
                    process
                        .info
                        .cancel_outcome
                        .get_or_insert_with(|| "cancelled".to_string());
                    process
                        .info
                        .termination_evidence
                        .get_or_insert_with(|| "process_exit_after_cancel".to_string());
                } else {
                    process.info.state = if status.success() {
                        ProcessState::Completed
                    } else {
                        ProcessState::Failed
                    };
                }
                process.info.updated_at = now;
                process.info.finished_at = Some(now);
                runtime.skill_lease = None;
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

    fn exec_request(program: &str, working_directory: &Path) -> ProcessExecRequest {
        ProcessExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            program: program.to_string(),
            args: Vec::new(),
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(working_directory.to_string_lossy().to_string()),
            wait_seconds: Some(2),
        }
    }

    async fn test_state(max_active_processes: usize) -> (AppState, PathBuf) {
        let root = unique_temp_dir("processes-max-active");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.limits.max_active_processes =
            crate::config::MaxActiveProcesses::Explicit(max_active_processes);
        config.confirmation_provider =
            crate::config::ConfirmationProviderConfig::from_legacy("none").unwrap();
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
                 WHEN json_extract(NEW.info_json, '$.program') = '__history_trigger_failure__'
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

    async fn wait_output_capture(state: &AppState, process_id: &str) -> ProcessOutputResponse {
        for _ in 0..100 {
            let output = get_process_output(
                state,
                ProcessOutputRequest {
                    process_id: process_id.to_string(),
                    cursor: None,
                    max_bytes: None,
                },
            )
            .await
            .unwrap();
            if matches!(
                output.capture_status,
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

    #[tokio::test]
    async fn completed_processes_release_capacity_and_keep_output() {
        let (state, workspace) = test_state(1).await;
        let first = start_process_for_test(state.clone(), exec_request("true", &workspace)).await;
        assert!(first.process_id.starts_with("process_testboot0001_"));
        assert_eq!(first.kind, ProcessKind::Command);
        let first = wait_terminal(&state, first).await;
        assert_eq!(first.state, ProcessState::Completed);

        let mut second_request = exec_request("printf", &workspace);
        second_request.args = vec!["done".to_string()];
        let second = start_process_for_test(state.clone(), second_request).await;
        let second = wait_terminal(&state, second).await;
        assert_eq!(second.state, ProcessState::Completed);
        let output = wait_output_capture(&state, &second.process_id).await;
        assert_eq!(output.stdout.data, "done");
        assert!(output.eof);
    }

    #[tokio::test]
    async fn reader_eof_is_visible_while_the_child_keeps_running() {
        let (state, workspace) = test_state(1).await;
        let mut request = exec_request("sh", &workspace);
        request.args = vec![
            "-c".to_string(),
            "exec >/dev/null 2>&1; sleep 0.8".to_string(),
        ];
        state
            .config
            .write()
            .await
            .policy
            .allow
            .push(crate::config::Rule {
                program: "sh".to_string(),
                args_prefix: Vec::new(),
            });
        let process = start_process_for_test(state.clone(), request).await;
        let mut eof_while_running = None;
        for _ in 0..100 {
            let info = get_process(&state, &process.process_id, 0).await.unwrap();
            if info.state.is_active() {
                let output = get_process_output(
                    &state,
                    ProcessOutputRequest {
                        process_id: process.process_id.clone(),
                        cursor: None,
                        max_bytes: None,
                    },
                )
                .await
                .unwrap();
                if output.eof {
                    eof_while_running = Some((info, output));
                    break;
                }
            }
            sleep(Duration::from_millis(10)).await;
        }
        let (info, output) =
            eof_while_running.expect("closed output pipes must report EOF before child exit");
        assert!(info.state.is_active());
        assert_eq!(output.capture_status, ProcessCaptureStatus::Complete);
        assert!(output.eof);
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
        let mut request = exec_request("sh", &workspace);
        request.args = vec![
            "-c".to_string(),
            "printf before; (sleep 0.6; printf after) &".to_string(),
        ];
        state
            .config
            .write()
            .await
            .policy
            .allow
            .push(crate::config::Rule {
                program: "sh".to_string(),
                args_prefix: Vec::new(),
            });
        let started = start_managed_process(state.clone(), request, options).await;
        let terminal = wait_terminal(&state, started).await;
        assert_eq!(terminal.state, ProcessState::Completed);
        assert_eq!(hook_count.load(Ordering::Acquire), 1);

        let before_eof = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: terminal.process_id.clone(),
                cursor: None,
                max_bytes: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(before_eof.capture_status, ProcessCaptureStatus::Capturing);
        assert!(!before_eof.eof);
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

        let output = wait_output_capture(&state, &terminal.process_id).await;
        assert!(output.eof);
        assert_eq!(decode_segment(&output.stdout), b"beforeafter");
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
    async fn output_cursors_are_replayable_and_page_raw_offsets() {
        let (state, workspace) = test_state(2).await;
        let mut request = exec_request("printf", &workspace);
        request.args = vec!["%s".to_string(), "abcdefghij".to_string()];
        let process = start_process_for_test(state.clone(), request).await;
        let process = wait_terminal(&state, process).await;
        let _ = wait_output_capture(&state, &process.process_id).await;

        let first = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id.clone(),
                cursor: None,
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap();
        assert_eq!(first.stdout.data, "abcd");
        assert_eq!(first.stdout.start_offset, "0");
        assert_eq!(first.stdout.end_offset, "4");
        assert!(first.has_more);
        assert!(!first.eof);

        let replay = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id.clone(),
                cursor: None,
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap();
        assert_eq!(replay.stdout.data, first.stdout.data);
        assert_eq!(replay.next_cursor, first.next_cursor);

        let second = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id.clone(),
                cursor: Some(first.next_cursor.clone()),
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap();
        let third = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id.clone(),
                cursor: Some(second.next_cursor),
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap();
        assert_eq!(second.stdout.data, "efgh");
        assert_eq!(third.stdout.data, "ij");
        assert!(!third.has_more);
        assert!(third.eof);
        let malformed = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id.clone(),
                cursor: Some("invalid".to_string()),
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(malformed, "invalid_process_output_cursor");
        let ahead_cursor = encode_output_cursor(agentic_gpt_protocol::ProcessCursor {
            version: 1,
            process_id: process.process_id.clone(),
            stdout_offset: u64::MAX,
            stderr_offset: 0,
        })
        .unwrap();
        let ahead = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id.clone(),
                cursor: Some(ahead_cursor),
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(ahead, "process_output_cursor_ahead_of_output");

        let other = start_process_for_test(state.clone(), exec_request("true", &workspace)).await;
        let other = wait_terminal(&state, other).await;
        let _ = wait_output_capture(&state, &other.process_id).await;
        let bound_cursor = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: other.process_id,
                cursor: Some(first.next_cursor),
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(bound_cursor, "invalid_process_output_cursor");
    }

    #[tokio::test]
    async fn output_pages_base64_invalid_utf8_without_loss() {
        let (state, workspace) = test_state(1).await;
        let mut request = exec_request("printf", &workspace);
        request.args = vec!["\\377\\000\\200".to_string()];
        let process = start_process_for_test(state.clone(), request).await;
        let process = wait_terminal(&state, process).await;
        let output = wait_output_capture(&state, &process.process_id).await;
        assert_eq!(output.stdout.encoding, ProcessOutputEncoding::Base64);
        assert_eq!(decode_segment(&output.stdout), [0xff, 0x00, 0x80]);
        assert!(output.eof);
        let bounded = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id.clone(),
                cursor: None,
                max_bytes: Some(4),
            },
        )
        .await
        .unwrap();
        assert_eq!(bounded.stdout.encoding, ProcessOutputEncoding::Base64);
        assert_eq!(bounded.stdout.data, BASE64.encode([0xff, 0x00, 0x80]));
        assert!(bounded.eof);
        let too_small = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: process.process_id,
                cursor: None,
                max_bytes: Some(3),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            too_small,
            "process_output_max_bytes_too_small_for_next_unit"
        );
    }

    #[tokio::test]
    async fn output_overflow_reports_exact_retained_gap() {
        let mut ring = OutputRing::new();
        ring.mark_started();
        ring.push(&vec![b'x'; PROCESS_OUTPUT_RING_CAPACITY + 5]);
        ring.finish(ReaderOutcome::Eof);
        let (data, start, end) = ring.snapshot();
        let (segment, next, used) = output_segment(&data, start, end, 0, 32).unwrap();
        let gap = segment
            .gap
            .as_ref()
            .expect("retained output must report a gap");
        assert_eq!(gap.start_offset, "0");
        assert_eq!(gap.end_offset, "5");
        assert_eq!(segment.start_offset, "5");
        assert_eq!(segment.end_offset, next.to_string());
        assert_eq!(used, 32);
        assert_eq!(decode_segment(&segment), vec![b'x'; 32]);
    }

    #[tokio::test]
    async fn creation_responses_stay_within_inline_and_preview_budgets() {
        let (state, workspace) = test_state(2).await;
        let mut request = exec_request("printf", &workspace);
        request.args = vec!["%s".to_string(), "x".repeat(12 * 1024)];
        let response =
            start_and_wait_process(state, request, ProcessOptions::for_source("test")).await;
        assert_eq!(response.status, ProcessState::Completed);
        assert!(!response.completed_inline);
        assert!(serde_json::to_vec(&response).unwrap().len() <= PROCESS_INLINE_RESPONSE_BYTES);
        assert!(response.inline_output.is_none());
        let preview = response
            .output_preview
            .expect("oversized output has a preview");
        assert!(preview.stdout.len() + preview.stderr.len() <= PROCESS_INLINE_PREVIEW_BYTES);
        assert!(preview.truncated);
    }

    #[tokio::test]
    async fn mcp_result_retrieval_distinguishes_complete_oversized_and_unavailable() {
        let (state, _workspace) = test_state(4).await;
        let registration = register_mcp_process(&state, mcp_spec("retained-result"))
            .await
            .unwrap();
        let process_id = registration.info.process_id;
        let value = serde_json::json!({"ok": true, "value": "retained"});
        complete_mcp_result(&state, &process_id, value.clone(), false, None)
            .await
            .unwrap();

        let too_small = get_process_result(
            &state,
            ProcessResultRequest {
                process_id: process_id.clone(),
                max_bytes: Some(1),
            },
        )
        .await
        .unwrap();
        assert!(matches!(too_small.status, ProcessResultStatus::TooLarge));
        assert!(too_small.result_available);
        assert!(too_small.result.is_none());

        let complete = get_process_result(
            &state,
            ProcessResultRequest {
                process_id,
                max_bytes: Some(ProcessResultRequest::MAX_MAX_BYTES),
            },
        )
        .await
        .unwrap();
        assert!(matches!(complete.status, ProcessResultStatus::Complete));
        assert!(complete.result_available);
        assert_eq!(complete.result, Some(value));

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
        let too_large = get_process_result(
            &state,
            ProcessResultRequest {
                process_id,
                max_bytes: Some(ProcessResultRequest::MAX_MAX_BYTES),
            },
        )
        .await
        .unwrap();
        assert!(matches!(too_large.status, ProcessResultStatus::TooLarge));
        assert!(!too_large.result_available);
        assert!(too_large.result.is_none());
        assert!(too_large.result_bytes.unwrap() > MAX_MCP_RESULT_BYTES);
    }

    #[tokio::test]
    async fn process_batch_applies_shared_response_preview_budget() {
        let (state, workspace) = test_state(4).await;
        let payload = "b".repeat(5 * 1024);
        let request = ProcessBatchExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            elements: vec![
                agentic_gpt_protocol::ProcessExecElement {
                    program: "printf".to_string(),
                    args: vec!["%s".to_string(), payload.clone()],
                    working_directory: None,
                },
                agentic_gpt_protocol::ProcessExecElement {
                    program: "printf".to_string(),
                    args: vec!["%s".to_string(), payload],
                    working_directory: None,
                },
            ],
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(workspace.to_string_lossy().to_string()),
            wait_seconds: Some(2),
        };
        let response =
            start_process_batch(state, request, "test:process.batch".to_string(), None, None)
                .await
                .unwrap();
        assert!(serde_json::to_vec(&response).unwrap().len() <= PROCESS_INLINE_RESPONSE_BYTES);
        assert_eq!(response.processes.len(), 2);
        assert!(response
            .processes
            .iter()
            .all(|process| !process.process.process_id.is_empty()));
        let preview_bytes = response
            .processes
            .iter()
            .map(|process| {
                process
                    .output_preview
                    .as_ref()
                    .map(|preview| preview.stdout.len() + preview.stderr.len())
                    .unwrap_or(0)
                    + process
                        .result_preview
                        .as_ref()
                        .map(String::len)
                        .unwrap_or(0)
            })
            .sum::<usize>();
        assert!(preview_bytes <= PROCESS_INLINE_PREVIEW_BYTES);
        assert!(!response.completed_inline);
        assert!(response
            .processes
            .iter()
            .all(|process| !process.completed_inline));
    }

    #[tokio::test]
    async fn oversized_process_admission_fails_before_command_effect() {
        let (state, workspace) = test_state(1).await;
        let marker = workspace.join("oversized-admission-must-not-run");
        let mut request = exec_request("touch", &workspace);
        request.args = vec![marker.to_string_lossy().to_string()];
        request.args.extend((0..70).map(|_| "x".repeat(4 * 1024)));

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
                    program: "touch".to_string(),
                    args: vec![marker.to_string_lossy().to_string()],
                    working_directory: None,
                })
                .collect(),
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(workspace.to_string_lossy().to_string()),
            wait_seconds: Some(0),
        };

        let error = start_process_batch(
            state.clone(),
            request,
            "test:process.batch".to_string(),
            None,
            None,
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
        let mut request = exec_request("printf", &workspace);
        request.args = vec!["admitted".to_string()];
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
        assert_eq!(output.stdout.data, "admitted");
    }

    #[tokio::test]
    async fn process_batch_admission_failure_is_atomic_before_spawn() {
        let (state, workspace) = test_state(2).await;
        let marker = workspace.join("process-batch-must-not-run");
        let mut first_request = exec_request("touch", &workspace);
        first_request.args = vec![marker.to_string_lossy().to_string()];
        let second_request = exec_request("__history_trigger_failure__", &workspace);
        let specs = vec![
            ManagedProcessSpec {
                request: first_request,
                working_directory: workspace.clone(),
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
                    request: exec_request("true", &workspace),
                    working_directory: workspace.clone(),
                    decision: PolicyDecision::Allow,
                    confirmation_result: None,
                    request_source: "test:process.batch".to_string(),
                    terminal_event_hook: None,
                    event_origin: None,
                },
                ManagedProcessSpec {
                    request: exec_request("true", &workspace),
                    working_directory: workspace.clone(),
                    decision: PolicyDecision::Allow,
                    confirmation_result: None,
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
                    program: "sleep".to_string(),
                    args: vec!["2".to_string()],
                    working_directory: None,
                },
                agentic_gpt_protocol::ProcessExecElement {
                    program: "sleep".to_string(),
                    args: vec!["2".to_string()],
                    working_directory: None,
                },
            ],
            need_confirm: false,
            confirm_method: None,
            working_directory: Some(workspace.to_string_lossy().to_string()),
            wait_seconds: Some(0),
        };

        let batch = start_process_batch(
            state.clone(),
            request,
            "test:process.batch".to_string(),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(batch.status, "running");
        assert!(!batch.completed_inline);
        assert_eq!(batch.processes.len(), 2);

        let mut states = Vec::new();
        for _ in 0..100 {
            states = Vec::with_capacity(batch.processes.len());
            for process in &batch.processes {
                states.push(
                    get_process(&state, &process.process.process_id, 0)
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
            let _ = cancel_process(&state, &process.process.process_id).await;
        }
    }

    #[tokio::test]
    async fn active_process_capacity_and_cancel_are_truthful() {
        let (state, workspace) = test_state(1).await;
        let mut request = exec_request("sleep", &workspace);
        request.args = vec!["2".to_string()];
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
        assert_eq!(
            cancelled.termination_evidence,
            "local_process_kill_completed"
        );
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
        let mut request = exec_request("printf", &workspace);
        request.group = Some("  runtime-group  ".to_string());
        request.args = vec!["history-output".to_string()];
        let admitted = start_process_for_test(state.clone(), request).await;
        assert_eq!(admitted.group.as_deref(), Some("runtime-group"));
        assert!(admitted.started_at.is_none());

        let terminal = wait_terminal(&state, admitted).await;
        let output = wait_output_capture(&state, &terminal.process_id).await;
        assert_eq!(decode_segment(&output.stdout), b"history-output");
        assert!(output.eof);
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
        let recovered_output = get_process_output(
            &state,
            ProcessOutputRequest {
                process_id: terminal.process_id.clone(),
                cursor: None,
                max_bytes: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(decode_segment(&recovered_output.stdout), b"history-output");
        assert!(recovered_output.eof);
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
        let mut request = exec_request("printf", &workspace);
        request.args = vec!["must-not-run".to_string()];
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
