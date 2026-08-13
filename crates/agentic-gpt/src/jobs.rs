use std::path::Path;
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Weak,
};

use agentic_gpt_protocol::{
    normalize_job_group, BatchExecRequest, ExecRequest, JobBatchResponse, JobDetail, JobError,
    JobInfo, JobKind, JobListRequest, JobResponse, JobState,
};
use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use rmcp::{model::RequestId, service::Peer, RoleClient};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::sync::{Mutex, OwnedRwLockReadGuard, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::time::{sleep, Instant};

use crate::{
    audit::{write_audit, AuditRecord},
    config::Config,
    confirmation, exec,
    policy::{policy_decision_for_profile, PolicyDecision},
    utils::{command_preview, JOB_TAIL_MAX},
    AppState,
};

const TERMINAL_JOB_HOT_CACHE_MINUTES: i64 = 5;
const MAX_TERMINAL_JOBS: usize = 100;
const MAX_LIST_JOBS: usize = 100;
pub(crate) const MAX_MCP_ARGUMENT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_MCP_RESULT_BYTES: usize = 512 * 1024;
const MAX_MCP_RESULT_PREVIEW_BYTES: usize = 8 * 1024;
const MAX_JOB_ERROR_BYTES: usize = 8 * 1024;
pub(crate) const MCP_GLOBAL_CONCURRENCY: usize = 8;
pub(crate) const MCP_PER_SERVER_CONCURRENCY: usize = 2;

pub(crate) fn capacity_rejection(active: usize, requested: usize, limit: usize) -> String {
    format!("max_active_jobs_reached; active={active}; requested={requested}; limit={limit}")
}

fn resolved_job_limit(config: &Config) -> usize {
    config.limits.max_active_jobs.resolve().resolved
}

fn validated_group(group: Option<&str>) -> std::result::Result<Option<String>, String> {
    normalize_job_group(group).map_err(|error| format!("{}: {}", error.code(), error.message()))
}

pub(crate) type TerminalEventHook = Arc<dyn Fn(&JobInfo) + Send + Sync>;

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

pub(crate) struct ManagedJob {
    info: JobInfo,
    detail: ManagedJobDetail,
    runtime: ManagedJobRuntime,
    cancel_requested: Arc<std::sync::atomic::AtomicBool>,
    audit: Option<ManagedAuditContext>,
    history_terminal_snapshot_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Default)]
struct ManagedJobDetail {
    result: Option<serde_json::Value>,
    error: Option<JobError>,
    result_truncated: bool,
    result_bytes: Option<usize>,
    result_sha256: Option<String>,
    result_preview: Option<String>,
}

pub(crate) enum ManagedJobRuntime {
    Process(ManagedProcessRuntime),
    Mcp(ManagedMcpRuntime),
}

pub(crate) struct ManagedMcpRuntime {
    peer: Option<Peer<RoleClient>>,
    request_id: Option<RequestId>,
}

pub(crate) struct ManagedProcessRuntime {
    child: Option<Child>,
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
    skill_lease: Option<SkillLease>,
}

pub(crate) struct ManagedJobOptions {
    pub(crate) request_source: String,
    pub(crate) skill_id: Option<String>,
    pub(crate) skill_path: Option<String>,
    pub(crate) installed_digest: Option<String>,
    pub(crate) terminal_event_hook: Option<TerminalEventHook>,
}

impl ManagedJobOptions {
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

pub(crate) struct ManagedProcessSpec {
    pub(crate) request: ExecRequest,
    pub(crate) working_directory: std::path::PathBuf,
    pub(crate) decision: PolicyDecision,
    pub(crate) confirmation_result: Option<String>,
    pub(crate) request_source: String,
    pub(crate) terminal_event_hook: Option<TerminalEventHook>,
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
}

pub(crate) struct ManagedMcpRegistration {
    pub(crate) info: JobInfo,
    pub(crate) cancel_requested: Arc<std::sync::atomic::AtomicBool>,
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

pub(crate) async fn start_managed_process_job(
    state: AppState,
    request: ExecRequest,
    options: ManagedJobOptions,
) -> JobInfo {
    start_process_job_inner(state, request, None, options, None, None).await
}

#[cfg(test)]
pub(crate) async fn start_process_job(state: AppState, request: ExecRequest) -> JobInfo {
    start_managed_process_job(
        state,
        request,
        ManagedJobOptions::for_source("hub:process.exec"),
    )
    .await
}

pub(crate) async fn start_skill_job_with_hook_and_source(
    state: AppState,
    request: ExecRequest,
    skill_id: &str,
    skill_path: &str,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> JobInfo {
    let config = state.config.read().await.clone();
    let lease = state.skill_leases.try_shared(skill_id).await;
    let lease_available = lease.is_some();
    start_process_job_inner(
        state,
        request,
        lease,
        ManagedJobOptions {
            request_source: request_source.to_string(),
            skill_id: Some(skill_id.to_string()),
            skill_path: Some(skill_path.to_string()),
            installed_digest: crate::skill_installs::package_sha256(&config, skill_id).ok(),
            terminal_event_hook,
        },
        None,
        (!lease_available).then(|| (JobState::Rejected, "skill_update_pending".to_string())),
    )
    .await
}

pub(crate) async fn register_mcp_job(
    state: &AppState,
    spec: ManagedMcpSpec,
) -> Result<ManagedMcpRegistration, String> {
    let group = validated_group(spec.group.as_deref())?;
    let config = state.config.read().await.clone();
    let job_id = state.new_job_id();
    let now = Utc::now();
    let info = JobInfo {
        agent_id: spec.agent_id,
        job_id: job_id.clone(),
        group,
        batch_id: spec.batch_id.clone(),
        batch_call_id: spec.batch_call_id.clone(),
        batch_index: spec.batch_index,
        kind: JobKind::Mcp,
        state: JobState::WaitingConfirmation,
        created_at: now,
        started_at: None,
        updated_at: now,
        finished_at: None,
        program: None,
        args: Vec::new(),
        working_directory: None,
        command_preview: None,
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
        skill_id: None,
        skill_path: None,
        installed_digest: None,
        mcp_server_id: Some(spec.server_id.clone()),
        mcp_tool_name: Some(spec.tool_name.clone()),
        cancel_requested: false,
        cancel_outcome: None,
        termination_evidence: None,
    };
    let cancel_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut jobs = state.jobs.lock().await;
    refresh_jobs(state, &mut jobs).await;
    let active = jobs
        .values()
        .filter(|job| job.info.state.is_active())
        .count();
    let limit = resolved_job_limit(&config);
    if active >= limit {
        return Err(capacity_rejection(active, 1, limit));
    }
    jobs.insert(
        job_id,
        ManagedJob {
            info: info.clone(),
            detail: ManagedJobDetail::default(),
            runtime: ManagedJobRuntime::Mcp(ManagedMcpRuntime {
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
        },
    );
    let _ = state.job_history.insert_admission(&info);
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
    let config = state.config.read().await.clone();
    let requested = specs.len();
    let limit = resolved_job_limit(&config);
    let now = Utc::now();
    let mut jobs = state.jobs.lock().await;
    refresh_jobs(state, &mut jobs).await;
    let active = jobs
        .values()
        .filter(|job| job.info.state.is_active())
        .count();
    if active.saturating_add(requested) > limit {
        return Err(capacity_rejection(active, requested, limit));
    }
    let mut registrations = Vec::with_capacity(requested);
    for spec in specs {
        let group = validated_group(spec.group.as_deref())?;
        let job_id = state.new_job_id();
        let info = JobInfo {
            agent_id: spec.agent_id,
            job_id: job_id.clone(),
            group,
            batch_id: spec.batch_id.clone(),
            batch_call_id: spec.batch_call_id.clone(),
            batch_index: spec.batch_index,
            kind: JobKind::Mcp,
            state: JobState::WaitingConfirmation,
            created_at: now,
            started_at: None,
            updated_at: now,
            finished_at: None,
            program: None,
            args: Vec::new(),
            working_directory: None,
            command_preview: None,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: Some(spec.server_id.clone()),
            mcp_tool_name: Some(spec.tool_name.clone()),
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
        };
        let cancel_requested = Arc::new(AtomicBool::new(false));
        jobs.insert(
            job_id,
            ManagedJob {
                info: info.clone(),
                detail: ManagedJobDetail::default(),
                runtime: ManagedJobRuntime::Mcp(ManagedMcpRuntime {
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
            },
        );
        let _ = state.job_history.insert_admission(&info);
        registrations.push(ManagedMcpRegistration {
            info,
            cancel_requested,
        });
    }
    Ok(registrations)
}

pub(crate) async fn set_mcp_preflight_rejection(
    state: &AppState,
    job_id: &str,
) -> Result<(), String> {
    let mut jobs = state.jobs.lock().await;
    let job = jobs
        .get_mut(job_id)
        .ok_or_else(|| missing_job_reason(state, job_id))?;
    if let Some(audit) = job.audit.as_mut() {
        audit.policy_decision = "Rejected".to_string();
        audit.confirmation_result = None;
    }
    Ok(())
}

pub(crate) async fn set_mcp_authorization(
    state: &AppState,
    job_id: &str,
    decision: &str,
) -> Result<(), String> {
    let mut jobs = state.jobs.lock().await;
    let job = jobs
        .get_mut(job_id)
        .ok_or_else(|| missing_job_reason(state, job_id))?;
    let Some(audit) = job.audit.as_mut() else {
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

pub(crate) async fn set_mcp_job_state(
    state: &AppState,
    job_id: &str,
    state_name: JobState,
) -> Result<(), String> {
    let mut jobs = state.jobs.lock().await;
    let job = jobs
        .get_mut(job_id)
        .ok_or_else(|| missing_job_reason(state, job_id))?;
    if job.info.state.is_active() {
        job.info.state = state_name;
        job.info.updated_at = Utc::now();
    }
    Ok(())
}

pub(crate) async fn attach_mcp_request(
    state: &AppState,
    job_id: &str,
    peer: Peer<RoleClient>,
    request_id: RequestId,
) -> Result<(), String> {
    let mut jobs = state.jobs.lock().await;
    let job = jobs
        .get_mut(job_id)
        .ok_or_else(|| missing_job_reason(state, job_id))?;
    let ManagedJobRuntime::Mcp(runtime) = &mut job.runtime else {
        return Err("job_kind_mismatch".to_string());
    };
    if !job.info.state.is_active() {
        return Err("job_not_active".to_string());
    }
    runtime.peer = Some(peer);
    runtime.request_id = Some(request_id);
    let now = Utc::now();
    job.info.state = JobState::Running;
    job.info.started_at = Some(now);
    job.info.updated_at = now;
    let _ = state.job_history.mark_started(&job.info);
    Ok(())
}

pub(crate) async fn complete_mcp_result(
    state: &AppState,
    job_id: &str,
    value: serde_json::Value,
    downstream_error: bool,
    cancel_outcome: Option<(&str, &str)>,
) -> Result<JobDetail, String> {
    let bytes = serde_json::to_vec(&value).map_err(|_| "mcp_result_encode_failed".to_string())?;
    let byte_count = bytes.len();
    let sha256 = format!("sha256:{:x}", Sha256::digest(&bytes));
    let truncated = byte_count > MAX_MCP_RESULT_BYTES;
    let preview = truncated.then(|| {
        let text = String::from_utf8_lossy(&bytes);
        utf8_prefix(&text, MAX_MCP_RESULT_PREVIEW_BYTES).to_string()
    });
    let mut jobs = state.jobs.lock().await;
    let job = jobs
        .get_mut(job_id)
        .ok_or_else(|| missing_job_reason(state, job_id))?;
    if job.info.state.is_terminal() {
        return Ok(job_detail(job));
    }
    job.detail.result = (!truncated).then_some(value);
    job.detail.result_truncated = truncated;
    job.info.truncated |= truncated;
    job.detail.result_bytes = Some(byte_count);
    job.detail.result_sha256 = Some(sha256);
    job.detail.result_preview = preview;
    if downstream_error {
        job.detail.error = Some(JobError {
            code: "mcp_tool_error".to_string(),
            message: "Downstream MCP tool returned isError=true".to_string(),
        });
    }
    let now = Utc::now();
    job.info.state = if downstream_error {
        JobState::Failed
    } else {
        JobState::Completed
    };
    job.info.updated_at = now;
    job.info.finished_at = Some(now);
    if let Some((outcome, evidence)) = cancel_outcome {
        job.info.cancel_requested = true;
        job.info.cancel_outcome = Some(outcome.to_string());
        job.info.termination_evidence = Some(evidence.to_string());
    } else {
        job.info.termination_evidence = Some("remote_response".to_string());
    }
    finalize_job(state, job).await;
    let detail = job_detail(job);
    prune_terminal_jobs(state, &mut jobs);
    Ok(detail)
}

pub(crate) async fn finish_mcp_error(
    state: &AppState,
    job_id: &str,
    terminal: JobState,
    code: impl Into<String>,
    message: impl Into<String>,
    cancel_outcome: Option<&str>,
    evidence: Option<&str>,
) -> Result<JobDetail, String> {
    let code = code.into();
    let message = bounded_error_message(message.into());
    let mut jobs = state.jobs.lock().await;
    let job = jobs
        .get_mut(job_id)
        .ok_or_else(|| missing_job_reason(state, job_id))?;
    if job.info.state.is_terminal() {
        return Ok(job_detail(job));
    }
    let now = Utc::now();
    job.info.state = terminal;
    job.info.updated_at = now;
    job.info.finished_at = Some(now);
    job.info.reject_reason = Some(code.clone());
    job.detail.error = Some(JobError { code, message });
    if let Some(outcome) = cancel_outcome {
        job.info.cancel_requested = true;
        job.info.cancel_outcome = Some(outcome.to_string());
    }
    if let Some(evidence) = evidence {
        job.info.termination_evidence = Some(evidence.to_string());
    }
    finalize_job(state, job).await;
    let detail = job_detail(job);
    prune_terminal_jobs(state, &mut jobs);
    Ok(detail)
}

pub(crate) async fn mcp_job_response(
    state: &AppState,
    job_id: &str,
    wait_seconds: u64,
) -> Result<JobResponse, String> {
    let detail = get_job_detail(state, job_id, wait_seconds).await?;
    let completed_inline = detail.job.state.is_terminal();
    Ok(JobResponse {
        status: detail.job.state,
        completed_inline,
        job_id: detail.job.job_id.clone(),
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        detail,
    })
}

fn bounded_error_message(value: String) -> String {
    if value.len() <= MAX_JOB_ERROR_BYTES {
        return value;
    }
    const SUFFIX: &str = "...[truncated]";
    let prefix_limit = MAX_JOB_ERROR_BYTES.saturating_sub(SUFFIX.len());
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

pub(crate) fn response(job: JobInfo, completed_inline: bool) -> JobResponse {
    JobResponse {
        status: job.state,
        completed_inline,
        job_id: job.job_id.clone(),
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        detail: JobDetail {
            job,
            detail_available: true,
            result: None,
            error: None,
            result_truncated: false,
            result_bytes: None,
            result_sha256: None,
            result_preview: None,
        },
    }
}

pub(crate) async fn start_and_wait_process(
    state: AppState,
    request: ExecRequest,
    options: ManagedJobOptions,
) -> JobResponse {
    let wait_seconds = request.effective_wait_seconds();
    let job = start_managed_process_job(state.clone(), request, options).await;
    let job = wait_for_job(&state, job, wait_seconds).await;
    response(job.clone(), job.state.is_terminal())
}

pub(crate) async fn start_process_batch(
    state: AppState,
    request: BatchExecRequest,
    request_source: String,
    terminal_event_hook: Option<TerminalEventHook>,
) -> Result<JobBatchResponse, String> {
    let wait_seconds = request.effective_wait_seconds();
    let batch_id = format!("batch_{}", uuid::Uuid::new_v4().simple());
    let group = validated_group(request.group.as_deref())?;
    if request.elements.is_empty() {
        return Ok(JobBatchResponse {
            batch_id,
            status: "completed".to_string(),
            completed_inline: true,
            poll_after_ms: 0,
            jobs: Vec::new(),
        });
    }
    let config = state.config.read().await.clone();
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
            request: ExecRequest {
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
        })
        .collect::<Vec<_>>();
    let mut jobs = start_prepared_managed_batch(state.clone(), specs).await?;
    let deadline = Instant::now() + std::time::Duration::from_secs(wait_seconds);
    loop {
        let mut all_terminal = true;
        for job in &mut jobs {
            if let Ok(latest) = get_job(&state, &job.job_id, 0).await {
                *job = latest;
            }
            all_terminal &= job.state.is_terminal();
        }
        if all_terminal || Instant::now() >= deadline {
            break;
        }
        sleep(std::time::Duration::from_millis(20)).await;
    }
    let completed_inline = jobs.iter().all(|job| job.state.is_terminal());
    let status = if !completed_inline {
        "running"
    } else if jobs.iter().any(|job| job.state != JobState::Completed) {
        "completed_with_errors"
    } else {
        "completed"
    };
    Ok(JobBatchResponse {
        batch_id,
        status: status.to_string(),
        completed_inline,
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        jobs,
    })
}

pub(crate) async fn start_prepared_managed_batch(
    state: AppState,
    specs: Vec<ManagedProcessSpec>,
) -> Result<Vec<JobInfo>, String> {
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    for spec in &specs {
        validated_group(spec.request.group.as_deref())?;
    }
    let config = state.config.read().await.clone();
    let requested = specs.len();
    let limit = resolved_job_limit(&config);
    let batch_concurrency = config.limits.max_concurrent_tasks.max(1).min(requested);
    let batch_slots = Arc::new(Semaphore::new(batch_concurrency));
    let mut registered = Vec::with_capacity(requested);
    {
        let mut jobs = state.jobs.lock().await;
        refresh_jobs(&state, &mut jobs).await;
        let active = jobs
            .values()
            .filter(|job| job.info.state.is_active())
            .count();
        if active.saturating_add(requested) > limit {
            return Err(capacity_rejection(active, requested, limit));
        }
        for spec in specs {
            let job_id = state.new_job_id();
            let now = Utc::now();
            let info = process_job_info(
                &spec.request,
                job_id,
                JobKind::Process,
                JobState::Queued,
                now,
                None,
            );
            let runtime = process_runtime(None);
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
            let stdout = runtime.stdout.clone();
            let stderr = runtime.stderr.clone();
            jobs.insert(
                info.job_id.clone(),
                ManagedJob {
                    info: info.clone(),
                    detail: ManagedJobDetail::default(),
                    runtime: ManagedJobRuntime::Process(runtime),
                    cancel_requested: cancel_requested.clone(),
                    audit: Some(audit),
                    history_terminal_snapshot_at: None,
                },
            );
            let _ = state.job_history.insert_admission(&info);
            registered.push((spec, info, stdout, stderr, cancel_requested));
        }
    }
    let mut infos = Vec::with_capacity(registered.len());
    for (spec, info, stdout, stderr, cancel_requested) in registered {
        let runner_state = state.clone();
        let runner_job_id = info.job_id.clone();
        let runner_slots = batch_slots.clone();
        tokio::spawn(async move {
            let permit = runner_slots
                .acquire_owned()
                .await
                .expect("batch semaphore remains open");
            set_job_state(&runner_state, &runner_job_id, JobState::Starting).await;
            run_async_job(
                runner_state.clone(),
                runner_job_id.clone(),
                spec.request,
                stdout,
                stderr,
                cancel_requested,
                Some((spec.working_directory, spec.decision)),
                spec.confirmation_result,
            )
            .await;
            monitor_job(runner_state, runner_job_id, Some(permit)).await;
        });
        infos.push(info);
    }
    Ok(infos)
}

async fn start_process_job_inner(
    state: AppState,
    request: ExecRequest,
    skill_lease: Option<SkillLease>,
    options: ManagedJobOptions,
    prepared: Option<(std::path::PathBuf, PolicyDecision)>,
    initial_terminal: Option<(JobState, String)>,
) -> JobInfo {
    let config = state.config.read().await.clone();
    let job_id = state.new_job_id();
    let now = Utc::now();
    let group_error = validated_group(request.group.as_deref()).err();
    let kind = if options.skill_id.is_some() {
        JobKind::Skill
    } else {
        JobKind::Process
    };
    let state_name = if matches!(prepared.as_ref(), Some((_, PolicyDecision::Confirm))) {
        JobState::WaitingConfirmation
    } else {
        JobState::Starting
    };
    let info = process_job_info(
        &request,
        job_id.clone(),
        kind,
        state_name,
        now,
        Some(&options),
    );
    let runtime = process_runtime(skill_lease);
    let stdout = runtime.stdout.clone();
    let stderr = runtime.stderr.clone();
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
    let capacity_error = {
        let mut jobs = state.jobs.lock().await;
        refresh_jobs(&state, &mut jobs).await;
        let active = jobs
            .values()
            .filter(|job| job.info.state.is_active())
            .count();
        let limit = resolved_job_limit(&config);
        let error = (active >= limit).then(|| capacity_rejection(active, 1, limit));
        jobs.insert(
            job_id.clone(),
            ManagedJob {
                info: info.clone(),
                detail: ManagedJobDetail::default(),
                runtime: ManagedJobRuntime::Process(runtime),
                cancel_requested: cancel_requested.clone(),
                audit: Some(audit),
                history_terminal_snapshot_at: None,
            },
        );
        let _ = state.job_history.insert_admission(&info);
        error
    };
    if let Some(reason) = capacity_error {
        finish_job(&state, &job_id, JobState::Rejected, &reason).await;
        return get_job_now(&state, &job_id).await.unwrap_or(info);
    }
    if let Some(reason) = group_error {
        finish_job(&state, &job_id, JobState::Rejected, &reason).await;
        return get_job_now(&state, &job_id).await.unwrap_or(info);
    }
    if let Some((terminal, reason)) = initial_terminal {
        finish_job(&state, &job_id, terminal, &reason).await;
        return get_job_now(&state, &job_id).await.unwrap_or(info);
    }
    tokio::spawn(run_async_job(
        state.clone(),
        job_id.clone(),
        request,
        stdout,
        stderr,
        cancel_requested,
        prepared,
        None,
    ));
    tokio::spawn(monitor_job(state, job_id, None));
    info
}

fn process_job_info(
    request: &ExecRequest,
    job_id: String,
    kind: JobKind,
    state: JobState,
    now: chrono::DateTime<Utc>,
    options: Option<&ManagedJobOptions>,
) -> JobInfo {
    JobInfo {
        agent_id: request.agent_id.clone(),
        job_id,
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
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
        skill_id: options.and_then(|options| options.skill_id.clone()),
        skill_path: options.and_then(|options| options.skill_path.clone()),
        installed_digest: options.and_then(|options| options.installed_digest.clone()),
        mcp_server_id: None,
        mcp_tool_name: None,
        cancel_requested: false,
        cancel_outcome: None,
        termination_evidence: None,
    }
}

fn process_runtime(skill_lease: Option<SkillLease>) -> ManagedProcessRuntime {
    ManagedProcessRuntime {
        child: None,
        stdout: Arc::new(Mutex::new(TailBuffer::new(JOB_TAIL_MAX))),
        stderr: Arc::new(Mutex::new(TailBuffer::new(JOB_TAIL_MAX))),
        skill_lease,
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_async_job(
    state: AppState,
    job_id: String,
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
        set_policy_decision(&state, &job_id, format!("{decision:?}")).await;
        let working_directory =
            match exec::resolve_working_directory(&config, request.working_directory.as_deref()) {
                Ok(directory) => directory,
                Err(reason) => {
                    finish_job(&state, &job_id, JobState::Rejected, &reason).await;
                    return;
                }
            };
        if let Err(reason) =
            exec::preflight(&config, &working_directory, &request.program, &request.args)
        {
            finish_job(&state, &job_id, JobState::Rejected, &reason).await;
            return;
        }
        (working_directory, decision)
    };
    if decision == PolicyDecision::Deny {
        finish_job(&state, &job_id, JobState::Rejected, "policy_denied").await;
        return;
    }
    if decision == PolicyDecision::Confirm {
        set_job_state(&state, &job_id, JobState::WaitingConfirmation).await;
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
        set_confirmation_result(&state, &job_id, confirmation.clone()).await;
        if confirmation != "allow_once" {
            let terminal = if cancel_requested.load(std::sync::atomic::Ordering::Acquire) {
                JobState::Cancelled
            } else {
                JobState::Rejected
            };
            finish_job(&state, &job_id, terminal, &confirmation).await;
            return;
        }
    }
    if cancel_requested.load(std::sync::atomic::Ordering::Acquire) {
        finish_job(&state, &job_id, JobState::Cancelled, "cancelled").await;
        return;
    }
    let spawned = spawn_process_with_buffers(
        &config,
        &working_directory,
        &request.program,
        &request.args,
        stdout,
        stderr,
    )
    .await;
    let child = match spawned {
        Ok(child) => child,
        Err(error) => {
            finish_job(
                &state,
                &job_id,
                JobState::Failed,
                &format!("spawn_failed: {error}"),
            )
            .await;
            return;
        }
    };
    let mut jobs = state.jobs.lock().await;
    let Some(job) = jobs.get_mut(&job_id) else {
        let mut child = child;
        let _ = child.kill().await;
        return;
    };
    if cancel_requested.load(std::sync::atomic::Ordering::Acquire) || !job.info.state.is_active() {
        drop(jobs);
        let mut child = child;
        let _ = child.kill().await;
        finish_job(&state, &job_id, JobState::Cancelled, "cancelled").await;
        return;
    }
    let now = Utc::now();
    job.info.state = JobState::Running;
    job.info.started_at = Some(now);
    job.info.updated_at = now;
    let _ = state.job_history.mark_started(&job.info);
    let ManagedJobRuntime::Process(runtime) = &mut job.runtime else {
        drop(jobs);
        let mut child = child;
        let _ = child.kill().await;
        return;
    };
    runtime.child = Some(child);
}

async fn spawn_process_with_buffers(
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

async fn monitor_job(state: AppState, job_id: String, _batch_permit: Option<OwnedSemaphorePermit>) {
    loop {
        sleep(std::time::Duration::from_millis(50)).await;
        let Some(info) = get_job_now(&state, &job_id).await else {
            return;
        };
        if info.state.is_terminal() {
            return;
        }
    }
}

async fn set_policy_decision(state: &AppState, job_id: &str, decision: String) {
    let mut jobs = state.jobs.lock().await;
    if let Some(job) = jobs.get_mut(job_id) {
        if let Some(audit) = job.audit.as_mut() {
            audit.policy_decision = decision;
        }
    }
}

async fn set_confirmation_result(state: &AppState, job_id: &str, result: String) {
    let mut jobs = state.jobs.lock().await;
    if let Some(job) = jobs.get_mut(job_id) {
        if let Some(audit) = job.audit.as_mut() {
            audit.confirmation_result = Some(result);
        }
    }
}

async fn set_job_state(state: &AppState, job_id: &str, state_name: JobState) {
    let mut jobs = state.jobs.lock().await;
    if let Some(job) = jobs.get_mut(job_id) {
        if job.info.state.is_active() {
            job.info.state = state_name;
            job.info.updated_at = Utc::now();
        }
    }
}

async fn finish_job(state: &AppState, job_id: &str, terminal: JobState, reason: &str) {
    let mut jobs = state.jobs.lock().await;
    if let Some(job) = jobs.get_mut(job_id) {
        if job.info.state.is_active() {
            let now = Utc::now();
            job.info.state = terminal;
            job.info.reject_reason = (!reason.is_empty()).then(|| reason.to_string());
            job.info.updated_at = now;
            job.info.finished_at = Some(now);
            job.info.cancel_requested = terminal == JobState::Cancelled;
            if terminal == JobState::Cancelled {
                job.info.cancel_outcome = Some("cancelled".to_string());
                job.info.termination_evidence = Some("local_process".to_string());
            }
            if let ManagedJobRuntime::Process(runtime) = &mut job.runtime {
                runtime.skill_lease = None;
            }
        }
        finalize_job(state, job).await;
    }
    prune_terminal_jobs(state, &mut jobs);
}

async fn finalize_job(state: &AppState, job: &mut ManagedJob) {
    if job.info.state.is_terminal() && job.history_terminal_snapshot_at != Some(job.info.updated_at)
    {
        let detail = job_detail(job);
        let _ = state.job_history.upsert_terminal(&detail);
        job.history_terminal_snapshot_at = Some(job.info.updated_at);
    }
    let Some(context) = job.audit.take() else {
        return;
    };
    let info = job.info.clone();
    let _ = write_audit(
        &context.config,
        AuditRecord {
            task_id: None,
            job_id: Some(info.job_id.clone()),
            batch_id: context.batch_id,
            batch_call_id: context.batch_call_id,
            batch_index: context.batch_index,
            time: info.updated_at,
            program: if info.kind == JobKind::Mcp {
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
            truncated: info.truncated || job.detail.result_truncated,
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
            result_bytes: job.detail.result_bytes,
            result_sha256: job.detail.result_sha256.clone(),
            terminal_state: Some(info.state.label().to_string()),
            termination_evidence: info.termination_evidence.clone(),
        },
    );
    crate::hub::report_job(state, info.clone());
    if let Some(hook) = context.terminal_event_hook {
        hook(&info);
    }
}

pub(crate) async fn wait_for_job(
    state: &AppState,
    mut info: JobInfo,
    wait_seconds: u64,
) -> JobInfo {
    if wait_seconds == 0 {
        return info;
    }
    let deadline = Instant::now() + std::time::Duration::from_secs(wait_seconds.min(30));
    while info.state.is_active() && Instant::now() < deadline {
        sleep(std::time::Duration::from_millis(20)).await;
        if let Some(latest) = get_job_now(state, &info.job_id).await {
            info = latest;
        } else {
            break;
        }
    }
    info
}

pub(crate) async fn list_jobs_page(
    state: &AppState,
    request: JobListRequest,
) -> std::result::Result<crate::job_history::JobHistoryPage, String> {
    let cursor = request
        .cursor
        .as_deref()
        .map(crate::job_history::decode_list_cursor)
        .transpose()
        .map_err(|error| error.to_string())?;
    let limit = request.effective_limit();
    let live = {
        let mut jobs = state.jobs.lock().await;
        refresh_jobs(state, &mut jobs).await;
        prune_terminal_jobs(state, &mut jobs);
        jobs.values()
            .map(|job| job.info.clone())
            .collect::<Vec<_>>()
    };

    let mut persisted = Vec::new();
    let mut history_request = request.clone();
    history_request.limit = Some(JobListRequest::MAX_LIMIT);
    let mut history_cursor = request.cursor.clone();
    let mut history_failed = false;
    loop {
        history_request.cursor = history_cursor.clone();
        match state.job_history.list(&history_request) {
            Ok(page) => {
                let next_cursor = page.next_cursor.clone();
                persisted.extend(page.jobs);
                let merged = merge_job_infos(&live, &persisted, &request, cursor.as_ref());
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

    let mut jobs = merge_job_infos(&live, &persisted, &request, cursor.as_ref());
    let next_cursor = if jobs.len() > limit {
        jobs.truncate(limit);
        jobs.last().map(crate::job_history::encode_list_cursor)
    } else {
        None
    };
    Ok(crate::job_history::JobHistoryPage { jobs, next_cursor })
}

pub(crate) async fn list_jobs(state: &AppState, request: JobListRequest) -> Vec<JobInfo> {
    list_jobs_page(state, request)
        .await
        .map(|page| page.jobs)
        .unwrap_or_default()
}

fn merge_job_infos(
    live: &[JobInfo],
    persisted: &[JobInfo],
    request: &JobListRequest,
    cursor: Option<&crate::job_history::JobHistoryCursor>,
) -> Vec<JobInfo> {
    let mut by_id = std::collections::HashMap::new();
    for job in persisted {
        by_id
            .entry(job.job_id.clone())
            .or_insert_with(|| job.clone());
    }
    for job in live {
        by_id.insert(job.job_id.clone(), job.clone());
    }
    let mut jobs = by_id
        .into_values()
        .filter(|job| {
            request
                .group
                .as_ref()
                .is_none_or(|group| job.group.as_deref() == Some(group.as_str()))
                && request.kind.is_none_or(|kind| job.kind == kind)
                && request.state.is_none_or(|state| job.state == state)
                && cursor.is_none_or(|cursor| {
                    job.created_at < cursor.created_at
                        || (job.created_at == cursor.created_at && job.job_id < cursor.job_id)
                })
        })
        .collect::<Vec<_>>();
    jobs.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.job_id.cmp(&left.job_id))
    });
    jobs
}

pub(crate) async fn current_jobs(state: &AppState) -> Vec<JobInfo> {
    list_jobs(
        state,
        JobListRequest {
            group: None,
            kind: None,
            state: None,
            limit: Some(MAX_LIST_JOBS),
            cursor: None,
        },
    )
    .await
    .into_iter()
    .filter(|job| job.state.is_active())
    .collect()
}

pub(crate) async fn get_job(
    state: &AppState,
    job_id: &str,
    wait_seconds: u64,
) -> Result<JobInfo, String> {
    if let Some(info) = get_job_now(state, job_id).await {
        return Ok(wait_for_job(state, info, wait_seconds).await);
    }
    match state.job_history.get(job_id) {
        Ok(Some(record)) if record.info.state.is_terminal() => Ok(record.info),
        _ => Err(missing_job_reason(state, job_id)),
    }
}

pub(crate) async fn get_job_detail(
    state: &AppState,
    job_id: &str,
    wait_seconds: u64,
) -> Result<JobDetail, String> {
    let info = get_job(state, job_id, wait_seconds).await?;
    {
        let mut jobs = state.jobs.lock().await;
        if let Some(job) = jobs.get_mut(job_id) {
            refresh_job(state, job).await;
            return Ok(job_detail(job));
        }
    }
    match state.job_history.get(job_id) {
        Ok(Some(record)) if record.info.state.is_terminal() => {
            Ok(record.detail.unwrap_or(JobDetail {
                job: info,
                detail_available: false,
                result: None,
                error: None,
                result_truncated: false,
                result_bytes: None,
                result_sha256: None,
                result_preview: None,
            }))
        }
        _ => Err(missing_job_reason(state, job_id)),
    }
}

fn job_detail(job: &ManagedJob) -> JobDetail {
    JobDetail {
        job: job.info.clone(),
        detail_available: true,
        result: job.detail.result.clone(),
        error: job.detail.error.clone(),
        result_truncated: job.detail.result_truncated,
        result_bytes: job.detail.result_bytes,
        result_sha256: job.detail.result_sha256.clone(),
        result_preview: job.detail.result_preview.clone(),
    }
}

async fn get_job_now(state: &AppState, job_id: &str) -> Option<JobInfo> {
    let mut jobs = state.jobs.lock().await;
    let job = jobs.get_mut(job_id)?;
    refresh_job(state, job).await;
    let info = job.info.clone();
    prune_terminal_jobs(state, &mut jobs);
    Some(info)
}

pub(crate) async fn cancel_job(state: &AppState, job_id: &str) -> Result<JobDetail, String> {
    let kind = {
        let mut jobs = state.jobs.lock().await;
        let Some(job) = jobs.get_mut(job_id) else {
            return Err(missing_job_reason(state, job_id));
        };
        refresh_job(state, job).await;
        job.info.kind
    };
    match kind {
        JobKind::Process | JobKind::Skill => cancel_process_job(state, job_id).await,
        JobKind::Mcp => cancel_mcp_job(state, job_id).await,
    }
}

async fn cancel_process_job(state: &AppState, job_id: &str) -> Result<JobDetail, String> {
    let child = {
        let mut jobs = state.jobs.lock().await;
        let Some(job) = jobs.get_mut(job_id) else {
            return Err(missing_job_reason(state, job_id));
        };
        refresh_job(state, job).await;
        job.cancel_requested
            .store(true, std::sync::atomic::Ordering::Release);
        job.info.cancel_requested = true;
        if job.info.state.is_terminal() {
            job.info.updated_at = Utc::now();
            job.info.cancel_outcome = Some("already_terminal".to_string());
            job.info.termination_evidence = Some("job_state".to_string());
            finalize_job(state, job).await;
            let detail = job_detail(job);
            prune_terminal_jobs(state, &mut jobs);
            return Ok(detail);
        }
        job.info.state = JobState::CancelRequested;
        job.info.updated_at = Utc::now();
        let ManagedJobRuntime::Process(runtime) = &mut job.runtime else {
            return Err("job_kind_mismatch".to_string());
        };
        match runtime.child.take() {
            Some(child) => Some(child),
            None => {
                mark_cancelled(
                    &mut job.info,
                    "cancelled_before_start",
                    "cancel_flag_before_process_start",
                );
                runtime.skill_lease = None;
                finalize_job(state, job).await;
                let detail = job_detail(job);
                prune_terminal_jobs(state, &mut jobs);
                return Ok(detail);
            }
        }
    };

    let Some(mut child) = child else {
        return Err("job_cancel_internal".to_string());
    };
    let kill_result = child.kill().await;
    let mut jobs = state.jobs.lock().await;
    let Some(job) = jobs.get_mut(job_id) else {
        return Err("job_not_found".to_string());
    };
    let ManagedJobRuntime::Process(runtime) = &mut job.runtime else {
        return Err("job_kind_changed".to_string());
    };
    match kill_result {
        Ok(()) => {
            mark_cancelled(&mut job.info, "cancelled", "local_process_kill_completed");
            runtime.skill_lease = None;
        }
        Err(_) => match child.try_wait() {
            Ok(Some(status)) => {
                let now = Utc::now();
                job.info.exit_code = status.code();
                job.info.state = if status.success() {
                    JobState::Completed
                } else {
                    JobState::Failed
                };
                job.info.updated_at = now;
                job.info.finished_at = Some(now);
                job.info.cancel_outcome = Some("already_terminal".to_string());
                job.info.termination_evidence = Some("process_exit_status".to_string());
                runtime.skill_lease = None;
            }
            _ => {
                job.info.state = JobState::CancelRequested;
                job.info.updated_at = Utc::now();
                job.info.cancel_outcome = Some("cancel_failed".to_string());
                job.info.termination_evidence = Some("process_kill_error".to_string());
                runtime.child = Some(child);
            }
        },
    }
    if job.info.state.is_terminal() {
        finalize_job(state, job).await;
    }
    let detail = job_detail(job);
    prune_terminal_jobs(state, &mut jobs);
    Ok(detail)
}

async fn cancel_mcp_job(state: &AppState, job_id: &str) -> Result<JobDetail, String> {
    let request = {
        let mut jobs = state.jobs.lock().await;
        let Some(job) = jobs.get_mut(job_id) else {
            return Err(missing_job_reason(state, job_id));
        };
        job.cancel_requested
            .store(true, std::sync::atomic::Ordering::Release);
        job.info.cancel_requested = true;
        if job.info.state.is_terminal() {
            job.info.updated_at = Utc::now();
            job.info.cancel_outcome = Some("already_terminal".to_string());
            job.info.termination_evidence = Some("job_state".to_string());
            finalize_job(state, job).await;
            let detail = job_detail(job);
            prune_terminal_jobs(state, &mut jobs);
            return Ok(detail);
        }
        let ManagedJobRuntime::Mcp(runtime) = &job.runtime else {
            return Err("job_kind_mismatch".to_string());
        };
        let request = runtime.peer.clone().zip(runtime.request_id.clone());
        if request.is_none() {
            let now = Utc::now();
            job.info.state = JobState::Cancelled;
            job.info.updated_at = now;
            job.info.finished_at = Some(now);
            job.info.reject_reason = Some("mcp_cancelled".to_string());
            job.info.cancel_outcome = Some("cancelled_before_request".to_string());
            job.info.termination_evidence =
                Some("local_cancel_before_downstream_request".to_string());
            job.detail.error = Some(JobError {
                code: "mcp_cancelled".to_string(),
                message: "MCP Job was cancelled before the downstream request started".to_string(),
            });
            finalize_job(state, job).await;
            let detail = job_detail(job);
            prune_terminal_jobs(state, &mut jobs);
            return Ok(detail);
        }
        job.info.state = JobState::CancelRequested;
        job.info.updated_at = Utc::now();
        request
    };
    let Some((peer, request_id)) = request else {
        return Err("job_cancel_internal".to_string());
    };
    let notification = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        peer.notify_cancelled(rmcp::model::CancelledNotificationParam {
            request_id,
            reason: Some("Cancelled by Agentic job.cancel".to_string()),
        }),
    )
    .await;
    let mut jobs = state.jobs.lock().await;
    let Some(job) = jobs.get_mut(job_id) else {
        return Err("job_not_found".to_string());
    };
    if job.info.state.is_terminal() {
        return Ok(job_detail(job));
    }
    match notification {
        Ok(Ok(())) => {
            job.info.state = JobState::CancelRequested;
            job.info.cancel_outcome = Some("notification_sent".to_string());
            job.info.termination_evidence = Some("mcp_cancel_notification_sent".to_string());
        }
        Ok(Err(error)) => {
            let now = Utc::now();
            job.info.state = JobState::Detached;
            job.info.updated_at = now;
            job.info.finished_at = Some(now);
            job.info.reject_reason = Some("mcp_cancel_notification_failed".to_string());
            job.info.cancel_outcome = Some("notification_failed".to_string());
            job.info.termination_evidence =
                Some("mcp_cancel_notification_delivery_failed".to_string());
            job.detail.error = Some(JobError {
                code: "mcp_cancel_notification_failed".to_string(),
                message: bounded_error_message(error.to_string()),
            });
            finalize_job(state, job).await;
        }
        Err(_) => {
            let now = Utc::now();
            job.info.state = JobState::Detached;
            job.info.updated_at = now;
            job.info.finished_at = Some(now);
            job.info.reject_reason = Some("mcp_cancel_notification_timeout".to_string());
            job.info.cancel_outcome = Some("notification_timeout".to_string());
            job.info.termination_evidence =
                Some("mcp_cancel_notification_delivery_timeout".to_string());
            job.detail.error = Some(JobError {
                code: "mcp_cancel_notification_timeout".to_string(),
                message: "MCP cancellation notification delivery exceeded 2 seconds".to_string(),
            });
            finalize_job(state, job).await;
        }
    }
    let detail = job_detail(job);
    prune_terminal_jobs(state, &mut jobs);
    Ok(detail)
}

fn mark_cancelled(info: &mut JobInfo, outcome: &str, evidence: &str) {
    let now = Utc::now();
    info.state = JobState::Cancelled;
    info.updated_at = now;
    info.finished_at = Some(now);
    info.reject_reason = Some("cancelled".to_string());
    info.cancel_requested = true;
    info.cancel_outcome = Some(outcome.to_string());
    info.termination_evidence = Some(evidence.to_string());
}

fn missing_job_reason(state: &AppState, job_id: &str) -> String {
    match state.job_id_generation(job_id) {
        Some(generation) if generation != state.boot_generation => {
            "job_lost_after_restart".to_string()
        }
        _ => "job_not_found".to_string(),
    }
}

async fn refresh_jobs(state: &AppState, jobs: &mut std::collections::HashMap<String, ManagedJob>) {
    for job in jobs.values_mut() {
        refresh_job(state, job).await;
    }
}

async fn refresh_job(state: &AppState, job: &mut ManagedJob) {
    let ManagedJobRuntime::Process(runtime) = &mut job.runtime else {
        if job.info.state.is_terminal() {
            finalize_job(state, job).await;
        }
        return;
    };
    if let Some(child) = runtime.child.as_mut() {
        if let Ok(Some(status)) = child.try_wait() {
            let now = Utc::now();
            job.info.exit_code = status.code();
            if job
                .cancel_requested
                .load(std::sync::atomic::Ordering::Acquire)
            {
                job.info.state = JobState::Cancelled;
                job.info.reject_reason = Some("cancelled".to_string());
                job.info.cancel_requested = true;
                job.info
                    .cancel_outcome
                    .get_or_insert_with(|| "cancelled".to_string());
                job.info
                    .termination_evidence
                    .get_or_insert_with(|| "process_exit_after_cancel".to_string());
            } else {
                job.info.state = if status.success() {
                    JobState::Completed
                } else {
                    JobState::Failed
                };
            }
            job.info.updated_at = now;
            job.info.finished_at = Some(now);
            runtime.skill_lease = None;
        }
    }
    let (stdout_tail, stderr_tail, truncated) = {
        let stdout = runtime.stdout.lock().await;
        let stderr = runtime.stderr.lock().await;
        (
            stdout.text(),
            stderr.text(),
            stdout.truncated || stderr.truncated,
        )
    };
    job.info.stdout_tail = stdout_tail;
    job.info.stderr_tail = stderr_tail;
    job.info.truncated = truncated;
    if job.info.state.is_terminal() {
        finalize_job(state, job).await;
    }
}

fn prune_terminal_jobs(state: &AppState, jobs: &mut std::collections::HashMap<String, ManagedJob>) {
    let _ = state.job_history.retry_pending();
    let cutoff = Utc::now() - ChronoDuration::minutes(TERMINAL_JOB_HOT_CACHE_MINUTES);
    let mut terminal = jobs
        .iter()
        .filter(|(_, job)| {
            job.info.state.is_terminal()
                && job.history_terminal_snapshot_at.is_some()
                && !state.job_history.terminal_pending(&job.info.job_id)
        })
        .map(|(id, job)| {
            (
                id.clone(),
                job.info.finished_at.unwrap_or(job.info.updated_at),
            )
        })
        .collect::<Vec<_>>();
    terminal.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    for (index, (id, updated_at)) in terminal.into_iter().enumerate() {
        if index >= MAX_TERMINAL_JOBS || updated_at < cutoff {
            jobs.remove(&id);
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

    fn exec_request(program: &str, working_directory: &Path) -> ExecRequest {
        ExecRequest {
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

    async fn test_state(max_active_jobs: usize) -> (AppState, PathBuf) {
        let root = unique_temp_dir("jobs-max-active");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.limits.max_active_jobs = crate::config::MaxActiveJobs::Explicit(max_active_jobs);
        config.confirmation_provider =
            crate::config::ConfirmationProviderConfig::from_legacy("none").unwrap();
        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(RwLock::new(config)),
            private_state: crate::private_state::PrivateStatePaths::for_test(
                std::env::temp_dir().join(format!(
                    "agentic-test-private-{}",
                    uuid::Uuid::new_v4().simple()
                )),
            ),
            job_history: crate::job_history::JobHistoryStore::disabled(
                root.join("disabled-history-parent").join("jobs.sqlite3"),
            ),
            runtime: crate::state::RuntimeModel::hub(crate::state::CapabilityProfile::Normal),
            started_at: Utc::now(),
            boot_generation: "testboot0001".to_string(),
            supervised: false,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::jobs::McpConcurrency::new()),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        };
        (state, workspace)
    }

    async fn test_state_with_history(max_active_jobs: usize) -> (AppState, PathBuf) {
        let (mut state, workspace) = test_state(max_active_jobs).await;
        state.job_history = crate::job_history::JobHistoryStore::open(&state.private_state);
        (state, workspace)
    }

    fn synthetic_job(
        job_id: &str,
        group: Option<&str>,
        kind: JobKind,
        state: JobState,
        created_at: chrono::DateTime<Utc>,
    ) -> JobInfo {
        let started_at = Some(created_at + chrono::Duration::milliseconds(1));
        let updated_at = created_at + chrono::Duration::milliseconds(2);
        JobInfo {
            agent_id: "test-agent".to_string(),
            job_id: job_id.to_string(),
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
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
        }
    }

    fn synthetic_detail(info: JobInfo) -> JobDetail {
        JobDetail {
            job: info,
            detail_available: true,
            result: None,
            error: None,
            result_truncated: false,
            result_bytes: None,
            result_sha256: None,
            result_preview: None,
        }
    }

    async fn wait_terminal(state: &AppState, job: JobInfo) -> JobInfo {
        wait_for_job(state, job, 3).await
    }

    #[test]
    fn job_error_messages_are_utf8_safe_and_bounded() {
        let value = "错".repeat(MAX_JOB_ERROR_BYTES);
        let bounded = bounded_error_message(value);
        assert!(bounded.len() <= MAX_JOB_ERROR_BYTES);
        assert!(bounded.ends_with("...[truncated]"));
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[tokio::test]
    async fn completed_jobs_release_capacity_and_keep_output() {
        let (state, workspace) = test_state(1).await;
        let first = start_process_job(state.clone(), exec_request("true", &workspace)).await;
        assert!(first.job_id.starts_with("job_testboot0001_"));
        assert_eq!(first.kind, JobKind::Process);
        let first = wait_terminal(&state, first).await;
        assert_eq!(first.state, JobState::Completed);

        let mut second_request = exec_request("printf", &workspace);
        second_request.args = vec!["done".to_string()];
        let second = start_process_job(state.clone(), second_request).await;
        let second = wait_terminal(&state, second).await;
        assert_eq!(second.state, JobState::Completed);
        assert_eq!(second.stdout_tail, "done");
    }

    #[tokio::test]
    async fn process_batch_respects_max_concurrent_tasks_without_blocking_batch_return() {
        let (state, workspace) = test_state(4).await;
        state.config.write().await.limits.max_concurrent_tasks = 1;
        let request = BatchExecRequest {
            agent_id: "test-agent".to_string(),
            group: None,
            elements: vec![
                agentic_gpt_protocol::ExecElement {
                    program: "sleep".to_string(),
                    args: vec!["2".to_string()],
                    working_directory: None,
                },
                agentic_gpt_protocol::ExecElement {
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
        )
        .await
        .unwrap();
        assert_eq!(batch.status, "running");
        assert!(!batch.completed_inline);
        assert_eq!(batch.jobs.len(), 2);

        let mut states = Vec::new();
        for _ in 0..100 {
            states = Vec::with_capacity(batch.jobs.len());
            for job in &batch.jobs {
                states.push(get_job(&state, &job.job_id, 0).await.unwrap().state);
            }
            if states.contains(&JobState::Running) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            states
                .iter()
                .filter(|state| **state == JobState::Running)
                .count(),
            1
        );
        assert_eq!(
            states
                .iter()
                .filter(|state| **state == JobState::Queued)
                .count(),
            1
        );

        for job in &batch.jobs {
            let _ = cancel_job(&state, &job.job_id).await;
        }
    }

    #[tokio::test]
    async fn active_job_capacity_and_cancel_are_truthful() {
        let (state, workspace) = test_state(1).await;
        let mut request = exec_request("sleep", &workspace);
        request.args = vec!["2".to_string()];
        let running = start_process_job(state.clone(), request).await;
        let mut running_state = running.clone();
        for _ in 0..100 {
            running_state = get_job(&state, &running.job_id, 0).await.unwrap();
            if running_state.state == JobState::Running {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(running_state.state, JobState::Running);
        let rejected = start_process_job(state.clone(), exec_request("true", &workspace)).await;
        let rejected = wait_terminal(&state, rejected).await;
        assert_eq!(rejected.state, JobState::Rejected);
        assert!(rejected
            .reject_reason
            .unwrap()
            .starts_with("max_active_jobs_reached"));

        let cancelled = cancel_job(&state, &running.job_id).await.unwrap();
        assert_eq!(cancelled.job.state, JobState::Cancelled);
        assert_eq!(cancelled.job.cancel_outcome.as_deref(), Some("cancelled"));
        assert_eq!(
            cancelled.job.termination_evidence.as_deref(),
            Some("local_process_kill_completed")
        );
    }

    #[tokio::test]
    async fn cancelling_a_terminal_job_reports_already_terminal_without_rewriting_state() {
        let (state, workspace) = test_state(1).await;
        let completed = wait_terminal(
            &state,
            start_process_job(state.clone(), exec_request("true", &workspace)).await,
        )
        .await;
        let cancelled = cancel_job(&state, &completed.job_id).await.unwrap();
        assert_eq!(cancelled.job.state, JobState::Completed);
        assert!(cancelled.job.cancel_requested);
        assert_eq!(
            cancelled.job.cancel_outcome.as_deref(),
            Some("already_terminal")
        );
        assert_eq!(
            cancelled.job.termination_evidence.as_deref(),
            Some("job_state")
        );
    }

    #[tokio::test]
    async fn job_filters_and_restart_loss_are_explicit() {
        let (state, workspace) = test_state(2).await;
        let completed = wait_terminal(
            &state,
            start_process_job(state.clone(), exec_request("true", &workspace)).await,
        )
        .await;
        let listed = list_jobs(
            &state,
            JobListRequest {
                group: None,
                kind: Some(JobKind::Process),
                state: Some(JobState::Completed),
                limit: Some(1),
                cursor: None,
            },
        )
        .await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].job_id, completed.job_id);
        assert_eq!(
            get_job(&state, "job_oldboot_abc", 0).await.unwrap_err(),
            "job_lost_after_restart"
        );
        assert_eq!(
            get_job(&state, "not-a-job", 0).await.unwrap_err(),
            "job_not_found"
        );
    }

    #[tokio::test]
    async fn runtime_history_tracks_group_timestamps_and_hot_cache_fallback() {
        let (state, workspace) = test_state_with_history(2).await;
        let mut request = exec_request("printf", &workspace);
        request.group = Some("  runtime-group  ".to_string());
        request.args = vec!["history-output".to_string()];
        let admitted = start_process_job(state.clone(), request).await;
        assert_eq!(admitted.group.as_deref(), Some("runtime-group"));
        assert!(admitted.started_at.is_none());

        let terminal = wait_terminal(&state, admitted).await;
        let started_at = terminal.started_at.expect("process should start");
        let finished_at = terminal.finished_at.expect("process should finish");
        assert!(terminal.created_at <= started_at);
        assert!(started_at <= finished_at);
        assert_eq!(terminal.stdout_tail, "history-output");

        let record = state
            .job_history
            .get(&terminal.job_id)
            .unwrap()
            .expect("terminal admission should be persisted");
        assert_eq!(record.info.group.as_deref(), Some("runtime-group"));
        assert_eq!(record.info.started_at, terminal.started_at);
        assert_eq!(record.info.finished_at, terminal.finished_at);
        assert_eq!(record.detail.unwrap().job.stdout_tail, "history-output");

        {
            let mut jobs = state.jobs.lock().await;
            let job = jobs.get_mut(&terminal.job_id).unwrap();
            job.info.finished_at = Some(Utc::now() - chrono::Duration::minutes(6));
            prune_terminal_jobs(&state, &mut jobs);
            assert!(!jobs.contains_key(&terminal.job_id));
        }
        let recovered = get_job_detail(&state, &terminal.job_id, 0).await.unwrap();
        assert_eq!(recovered.job.group.as_deref(), Some("runtime-group"));
        assert_eq!(recovered.job.state, JobState::Completed);
        assert_eq!(recovered.job.stdout_tail, "history-output");
    }

    #[tokio::test]
    async fn job_list_merges_live_wins_and_uses_global_cursor_order() {
        let (state, _workspace) = test_state_with_history(4).await;
        let now = Utc::now();
        let persisted_old = synthetic_job(
            "job_persisted_old",
            Some("alpha"),
            JobKind::Process,
            JobState::Completed,
            now - chrono::Duration::seconds(2),
        );
        let persisted_new = synthetic_job(
            "job_persisted_new",
            Some("alpha"),
            JobKind::Mcp,
            JobState::Completed,
            now - chrono::Duration::seconds(1),
        );
        for info in [&persisted_old, &persisted_new] {
            let _ = state.job_history.insert_admission(info);
            let _ = state
                .job_history
                .upsert_terminal(&synthetic_detail(info.clone()));
        }
        let live_duplicate = synthetic_job(
            "job_persisted_old",
            Some("alpha"),
            JobKind::Process,
            JobState::Running,
            persisted_old.created_at,
        );
        state.jobs.lock().await.insert(
            live_duplicate.job_id.clone(),
            ManagedJob {
                info: live_duplicate,
                detail: ManagedJobDetail::default(),
                runtime: ManagedJobRuntime::Process(process_runtime(None)),
                cancel_requested: Arc::new(AtomicBool::new(false)),
                audit: None,
                history_terminal_snapshot_at: None,
            },
        );

        let first = list_jobs_page(
            &state,
            JobListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: None,
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(first.jobs.len(), 1);
        assert_eq!(first.jobs[0].job_id, "job_persisted_new");
        let cursor = first.next_cursor.clone().expect("second page cursor");

        let running = list_jobs_page(
            &state,
            JobListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: Some(JobState::Running),
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(running.jobs.len(), 1);
        assert_eq!(running.jobs[0].job_id, "job_persisted_old");
        assert_eq!(running.jobs[0].state, JobState::Running);

        let completed = list_jobs_page(
            &state,
            JobListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: Some(JobState::Completed),
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(completed.jobs.len(), 1);
        assert_eq!(completed.jobs[0].job_id, "job_persisted_new");

        let second = list_jobs_page(
            &state,
            JobListRequest {
                group: Some("alpha".to_string()),
                kind: None,
                state: None,
                limit: Some(10),
                cursor: Some(cursor),
            },
        )
        .await
        .unwrap();
        assert_eq!(second.jobs.len(), 1);
        assert_eq!(second.jobs[0].job_id, "job_persisted_old");
        assert_eq!(second.jobs[0].state, JobState::Running);
        assert!(second.next_cursor.is_none());
    }

    #[tokio::test]
    async fn degraded_history_does_not_change_live_result_or_cancel_truth() {
        let (state, workspace) = test_state(1).await;
        let mut state = state;
        state.job_history = crate::job_history::JobHistoryStore::disabled(
            workspace
                .join("missing-history-parent")
                .join("jobs.sqlite3"),
        );
        let mut request = exec_request("printf", &workspace);
        request.args = vec!["live-only".to_string()];
        let terminal = wait_terminal(&state, start_process_job(state.clone(), request).await).await;
        assert_eq!(terminal.state, JobState::Completed);
        assert_eq!(terminal.stdout_tail, "live-only");
        let detail = get_job_detail(&state, &terminal.job_id, 0).await.unwrap();
        assert_eq!(detail.job.stdout_tail, "live-only");
        assert_eq!(
            state.job_history.health().status,
            crate::job_history::HistoryHealthStatus::Degraded
        );
        assert!(state.job_history.health().pending_terminal_count > 0);
    }

    #[tokio::test]
    async fn skill_leases_still_block_updates() {
        let manager = SkillLeaseManager::new();
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
