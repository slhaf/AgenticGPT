use agentic_gpt_protocol::{
    SkillInstallCancelOutcome, SkillInstallCancelRequest, SkillInstallCancelResponse,
    SkillInstallError, SkillInstallFile, SkillInstallFileSummary, SkillInstallGetRequest,
    SkillInstallJobRecord, SkillInstallPhase, SkillInstallProgress, SkillInstallRequest,
    SkillInstallResult, SkillInstallSource, SkillInstallSourceSummary, SkillInstallStartResponse,
    SkillInstallStatus, SkillInstallStatusResponse, SkillReadRequest, SkillSummary,
    SKILL_INSTALL_JOB_SCHEMA_VERSION,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::{skills, AppState};

const DEFAULT_POLL_AFTER_MS: u64 = 1_000;
const MAX_ATTEMPTS: u32 = 3;
const INSTALL_DEADLINE_SECS: u64 = 600;
const MAX_FILES: usize = 256;
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_SKILL_MD_BYTES: u64 = 256 * 1024;
const MAX_PATH_BYTES: usize = 240;
const MAX_PATH_DEPTH: usize = 16;
const TERMINAL_RETENTION_DAYS: i64 = 7;
const MAX_TERMINAL_RECORDS: usize = 100;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitJournal {
    install_id: String,
    id: String,
    target: String,
    archive: Option<String>,
    candidate_committed: bool,
}

#[derive(Clone)]
pub(crate) struct InstallManager {
    records: Arc<Mutex<HashMap<String, SkillInstallJobRecord>>>,
    cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    target_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    changed: Arc<Notify>,
    worker_slots: Arc<Semaphore>,
}

impl InstallManager {
    pub(crate) fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            target_locks: Arc::new(Mutex::new(HashMap::new())),
            changed: Arc::new(Notify::new()),
            worker_slots: Arc::new(Semaphore::new(2)),
        }
    }

    pub(crate) async fn recover(&self, state: AppState) -> Result<()> {
        let config = state.config.read().await.clone();
        let root = install_records_root(&config);
        let Ok(entries) = fs::read_dir(&root) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(mut record) = serde_json::from_str::<SkillInstallJobRecord>(&text) else {
                continue;
            };
            if is_terminal(record.status.status) {
                self.records
                    .lock()
                    .await
                    .insert(record.install_id.clone(), record);
                continue;
            }
            if matches!(
                record.status.phase,
                Some(SkillInstallPhase::Committing | SkillInstallPhase::Activating)
            ) {
                record.status.status = SkillInstallStatus::Failed;
                record.status.phase = None;
                record.status.error = Some(SkillInstallError {
                    code: "recovery_failed".to_string(),
                    message: "installation stopped during commit and requires retry".to_string(),
                    phase: Some(SkillInstallPhase::Committing),
                    retryable: true,
                });
                record.status.finished_at = Some(Utc::now());
                record.status.poll_after_ms = 0;
                touch_status(&mut record.status);
                self.save_cache(&config, &record).await?;
                continue;
            }
            let _ = fs::remove_dir_all(staging_path(&config, &record.install_id));
            record.status.status = SkillInstallStatus::Queued;
            record.status.phase = None;
            record.status.cancel_requested_at = None;
            record.status.poll_after_ms = DEFAULT_POLL_AFTER_MS;
            touch_status(&mut record.status);
            self.insert_record(&config, record.clone()).await?;
            self.cancellations
                .lock()
                .await
                .insert(record.install_id.clone(), Arc::new(AtomicBool::new(false)));
            let manager = self.clone();
            let worker_state = state.clone();
            tokio::spawn(async move { manager.run_worker(worker_state, record.install_id).await });
        }
        Ok(())
    }

    pub(crate) async fn start(
        &self,
        state: AppState,
        request: SkillInstallRequest,
    ) -> Result<SkillInstallStartResponse> {
        let config = state.config.read().await.clone();
        validate_install_request(&config, &request)?;
        let canonical_request_sha256 = sha256_json(&request)?;
        if let Some(existing) = self
            .find_idempotent(&config, request.idempotency_key.as_deref())
            .await?
        {
            if existing.canonical_request_sha256 != canonical_request_sha256 {
                return Err(anyhow!("idempotency_conflict"));
            }
            return Ok(start_response(&existing.status, true));
        }
        let target = skills::skills_root(&config).join(&request.id);
        if target.exists() && !request.replace_existing {
            return Err(anyhow!("target_exists"));
        }

        let now = Utc::now();
        let install_id = format!("install-{}", Uuid::new_v4().simple());
        let status = SkillInstallStatusResponse {
            install_id: install_id.clone(),
            id: request.id.clone(),
            revision: 1,
            status: SkillInstallStatus::Queued,
            phase: None,
            attempt: 0,
            max_attempts: MAX_ATTEMPTS,
            progress: initial_progress(&request.source),
            source: source_summary(&request.source),
            created_at: now,
            started_at: None,
            updated_at: now,
            finished_at: None,
            elapsed_ms: 0,
            cancel_requested_at: None,
            result: None,
            error: None,
            poll_after_ms: DEFAULT_POLL_AFTER_MS,
        };
        let record = SkillInstallJobRecord {
            schema_version: SKILL_INSTALL_JOB_SCHEMA_VERSION,
            install_id: install_id.clone(),
            request,
            canonical_request_sha256,
            status: status.clone(),
        };
        self.insert_record(&config, record).await?;
        self.cancellations
            .lock()
            .await
            .insert(install_id.clone(), Arc::new(AtomicBool::new(false)));
        let manager = self.clone();
        tokio::spawn(async move { manager.run_worker(state, install_id).await });
        Ok(start_response(&status, false))
    }

    pub(crate) async fn get(
        &self,
        state: &AppState,
        request: SkillInstallGetRequest,
    ) -> Result<SkillInstallStatusResponse> {
        let config = state.config.read().await.clone();
        let initial = self.load_record(&config, &request.install_id).await?;
        let wait_seconds = request.effective_wait_seconds();
        if wait_seconds == 0 || is_terminal(initial.status.status) {
            return Ok(initial.status);
        }
        let revision = initial.status.revision;
        let notified = self.changed.notified();
        let _ = timeout(Duration::from_secs(wait_seconds.min(30)), notified).await;
        let latest = self.load_record(&config, &request.install_id).await?;
        if latest.status.revision > revision || is_terminal(latest.status.status) {
            return Ok(latest.status);
        }
        Ok(latest.status)
    }

    pub(crate) async fn cancel(
        &self,
        state: &AppState,
        request: SkillInstallCancelRequest,
    ) -> Result<SkillInstallCancelResponse> {
        let config = state.config.read().await.clone();
        let mut record = self.load_record(&config, &request.install_id).await?;
        let status = record.status.status;
        if is_terminal(status) && status != SkillInstallStatus::Cancelled {
            return Ok(cancel_response(
                &record.status,
                SkillInstallCancelOutcome::AlreadyTerminal,
                false,
            ));
        }
        if status == SkillInstallStatus::Cancelled {
            return Ok(cancel_response(
                &record.status,
                SkillInstallCancelOutcome::AlreadyCancelled,
                false,
            ));
        }
        if matches!(
            record.status.phase,
            Some(SkillInstallPhase::Committing | SkillInstallPhase::Activating)
        ) {
            return Ok(cancel_response(
                &record.status,
                SkillInstallCancelOutcome::TooLate,
                false,
            ));
        }

        let cancel = self.cancellation(&request.install_id).await;
        cancel.store(true, Ordering::Release);
        let now = Utc::now();
        record.status.cancel_requested_at = Some(now);
        if status == SkillInstallStatus::Queued {
            record.status.status = SkillInstallStatus::Cancelled;
            record.status.phase = None;
            record.status.finished_at = Some(now);
            record.status.poll_after_ms = 0;
        }
        touch_status(&mut record.status);
        self.save_record(&config, &record).await?;
        self.records
            .lock()
            .await
            .insert(record.install_id.clone(), record.clone());
        self.changed.notify_waiters();
        let outcome = if status == SkillInstallStatus::Queued {
            SkillInstallCancelOutcome::Cancelled
        } else {
            SkillInstallCancelOutcome::CancelRequested
        };
        Ok(cancel_response(&record.status, outcome, true))
    }

    async fn run_worker(&self, state: AppState, install_id: String) {
        let permit = match self.worker_slots.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let result = self.execute_worker(&state, &install_id, permit).await;
        if let Err(error) = result {
            let _ = self
                .fail(&state, &install_id, &error.to_string(), None, false)
                .await;
        }
        let _ = self.cancellations.lock().await.remove(&install_id);
    }

    async fn execute_worker(
        &self,
        state: &AppState,
        install_id: &str,
        _permit: OwnedSemaphorePermit,
    ) -> Result<()> {
        let config = state.config.read().await.clone();
        let mut record = self.load_record(&config, install_id).await?;
        if record.status.status == SkillInstallStatus::Cancelled {
            return Ok(());
        }
        set_running(&mut record.status);
        record.status.phase = Some(SkillInstallPhase::Resolving);
        record.status.attempt = 1;
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;

        let staging = staging_path(&config, install_id);
        if let Err(error) = fs::remove_dir_all(&staging) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return self
                    .fail(
                        state,
                        install_id,
                        &format!("staging_cleanup_failed: {error}"),
                        Some(SkillInstallPhase::Validating),
                        false,
                    )
                    .await;
            }
        }
        fs::create_dir_all(&staging)?;
        let started = record.status.started_at.unwrap_or_else(Utc::now);
        let deadline = started + ChronoDuration::seconds(INSTALL_DEADLINE_SECS as i64);
        let summaries =
            match materialize_source(&record.request, &staging, &self.cancellations, install_id)
                .await
            {
                Ok(summaries) => summaries,
                Err(error) => {
                    let _ = fs::remove_dir_all(&staging);
                    return self
                        .fail(
                            state,
                            install_id,
                            &error.to_string(),
                            Some(SkillInstallPhase::Downloading),
                            false,
                        )
                        .await;
                }
            };
        if self.is_cancelled(install_id).await {
            let _ = fs::remove_dir_all(&staging);
            return self.cancelled(state, install_id).await;
        }
        record = self.load_record(&config, install_id).await?;
        record.status.phase = Some(SkillInstallPhase::Validating);
        record.status.progress.files_completed = summaries.len() as u64;
        record.status.progress.bytes_downloaded =
            summaries.iter().map(|summary| summary.size_bytes).sum();
        record.status.progress.bytes_total = Some(record.status.progress.bytes_downloaded);
        record.status.source.files = summaries;
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;
        validate_staging(&staging, &record.request.id)?;

        if self.is_cancelled(install_id).await {
            let _ = fs::remove_dir_all(&staging);
            return self.cancelled(state, install_id).await;
        }
        record = self.load_record(&config, install_id).await?;
        record.status.phase = Some(SkillInstallPhase::WaitingForTarget);
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;
        let target_lock = self.target_lock(&record.request.id).await;
        let remaining = remaining_deadline(deadline);
        let guard = match timeout(remaining, target_lock.lock()).await {
            Ok(guard) => guard,
            Err(_) => {
                let _ = fs::remove_dir_all(&staging);
                return self
                    .fail(
                        state,
                        install_id,
                        "target_busy",
                        Some(SkillInstallPhase::WaitingForTarget),
                        true,
                    )
                    .await;
            }
        };
        if self.is_cancelled(install_id).await {
            drop(guard);
            let _ = fs::remove_dir_all(&staging);
            return self.cancelled(state, install_id).await;
        }
        record = self.load_record(&config, install_id).await?;
        record.status.phase = Some(SkillInstallPhase::Committing);
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;
        let was_active = skills::is_active(state, &record.request.id).await?;
        let archive = commit_staging(&config, &record.request, &staging, install_id)?;
        drop(guard);

        record = self.load_record(&config, install_id).await?;
        record.status.phase = Some(SkillInstallPhase::Activating);
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;
        let should_activate = record.request.activate_after_install == Some(true)
            || (!archive.is_some() && !was_active)
            || was_active;
        if should_activate {
            if let Err(error) = skills::activate(
                state,
                agentic_gpt_protocol::SkillActivationRequest {
                    id: record.request.id.clone(),
                },
            )
            .await
            {
                rollback_commit(&config, &record.request.id, archive.as_deref())?;
                remove_commit_journal(&config, install_id);
                let _ = fs::remove_dir_all(&staging);
                return self
                    .fail(
                        state,
                        install_id,
                        &format!("activation_failed: {error}"),
                        Some(SkillInstallPhase::Activating),
                        false,
                    )
                    .await;
            }
        }
        let installed = skills::read(
            state,
            SkillReadRequest {
                id: record.request.id.clone(),
                path: None,
            },
        )
        .await?;
        let detail = installed.skill;
        let result = SkillInstallResult {
            skill: SkillSummary {
                id: detail.id,
                name: detail.name,
                description: detail.description,
                version: detail.version,
                tags: detail.tags,
                active: detail.active,
                origin: detail.origin,
                read_only: detail.read_only,
                package_summary: detail.package_summary,
                warnings: detail.warnings,
            },
            source: record.status.source.clone(),
            package_sha256: package_sha256(&config, &record.request.id)?,
        };
        let _ = fs::remove_dir_all(&staging);
        remove_commit_journal(&config, install_id);
        record = self.load_record(&config, install_id).await?;
        record.status.phase = None;
        record.status.status = SkillInstallStatus::Completed;
        record.status.result = Some(result);
        record.status.finished_at = Some(Utc::now());
        record.status.poll_after_ms = 0;
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;
        Ok(())
    }

    async fn fail(
        &self,
        state: &AppState,
        install_id: &str,
        message: &str,
        phase: Option<SkillInstallPhase>,
        retryable: bool,
    ) -> Result<()> {
        let config = state.config.read().await.clone();
        let mut record = self.load_record(&config, install_id).await?;
        record.status.status = SkillInstallStatus::Failed;
        record.status.phase = None;
        record.status.error = Some(SkillInstallError {
            code: if message == "target_busy" {
                "target_busy".to_string()
            } else {
                "internal_error".to_string()
            },
            message: safe_error_message(message),
            phase,
            retryable,
        });
        record.status.finished_at = Some(Utc::now());
        record.status.poll_after_ms = 0;
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;
        let _ = fs::remove_dir_all(staging_path(&config, install_id));
        remove_commit_journal(&config, install_id);
        Ok(())
    }

    async fn cancelled(&self, state: &AppState, install_id: &str) -> Result<()> {
        let config = state.config.read().await.clone();
        let mut record = self.load_record(&config, install_id).await?;
        record.status.status = SkillInstallStatus::Cancelled;
        record.status.phase = None;
        record.status.finished_at = Some(Utc::now());
        record.status.poll_after_ms = 0;
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await
    }

    async fn insert_record(
        &self,
        config: &crate::config::Config,
        record: SkillInstallJobRecord,
    ) -> Result<()> {
        self.save_record(config, &record).await?;
        self.records
            .lock()
            .await
            .insert(record.install_id.clone(), record);
        self.changed.notify_waiters();
        Ok(())
    }

    async fn save_cache(
        &self,
        config: &crate::config::Config,
        record: &SkillInstallJobRecord,
    ) -> Result<()> {
        self.save_record(config, record).await?;
        self.records
            .lock()
            .await
            .insert(record.install_id.clone(), record.clone());
        self.changed.notify_waiters();
        Ok(())
    }

    async fn save_record(
        &self,
        config: &crate::config::Config,
        record: &SkillInstallJobRecord,
    ) -> Result<()> {
        let root = install_records_root(config);
        fs::create_dir_all(&root)?;
        let path = root.join(format!("{}.json", record.install_id));
        let tmp = root.join(format!(".{}.tmp", record.install_id));
        fs::write(&tmp, serde_json::to_vec_pretty(record)?)?;
        fs::rename(tmp, path)?;
        self.prune_records(config).await?;
        Ok(())
    }

    async fn load_record(
        &self,
        config: &crate::config::Config,
        install_id: &str,
    ) -> Result<SkillInstallJobRecord> {
        if let Some(record) = self.records.lock().await.get(install_id).cloned() {
            return Ok(record);
        }
        let path = install_records_root(config).join(format!("{install_id}.json"));
        let text = fs::read_to_string(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                anyhow!("install_not_found")
            } else {
                anyhow!("install_record_unreadable")
            }
        })?;
        let record: SkillInstallJobRecord = serde_json::from_str(&text)?;
        self.records
            .lock()
            .await
            .insert(install_id.to_string(), record.clone());
        Ok(record)
    }

    async fn find_idempotent(
        &self,
        config: &crate::config::Config,
        key: Option<&str>,
    ) -> Result<Option<SkillInstallJobRecord>> {
        let Some(key) = key else { return Ok(None) };
        let root = install_records_root(config);
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let Ok(text) = fs::read_to_string(path) else {
                    continue;
                };
                let Ok(record) = serde_json::from_str::<SkillInstallJobRecord>(&text) else {
                    continue;
                };
                if record.request.idempotency_key.as_deref() == Some(key) {
                    return Ok(Some(record));
                }
            }
        }
        Ok(None)
    }

    async fn cancellation(&self, install_id: &str) -> Arc<AtomicBool> {
        let mut cancellations = self.cancellations.lock().await;
        cancellations
            .entry(install_id.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    async fn is_cancelled(&self, install_id: &str) -> bool {
        self.cancellation(install_id).await.load(Ordering::Acquire)
    }

    async fn target_lock(&self, id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.target_locks.lock().await;
        locks
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn prune_records(&self, config: &crate::config::Config) -> Result<()> {
        let root = install_records_root(config);
        let now = Utc::now();
        let mut terminal = Vec::new();
        for entry in fs::read_dir(&root).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<SkillInstallJobRecord>(&text) else {
                continue;
            };
            if is_terminal(record.status.status) {
                terminal.push((
                    record
                        .status
                        .finished_at
                        .unwrap_or(record.status.updated_at),
                    record.install_id,
                ));
            }
        }
        terminal.sort_by(|left, right| right.0.cmp(&left.0));
        for (index, (finished_at, install_id)) in terminal.into_iter().enumerate() {
            if index >= MAX_TERMINAL_RECORDS
                || now - finished_at > ChronoDuration::days(TERMINAL_RETENTION_DAYS)
            {
                let _ = fs::remove_file(root.join(format!("{install_id}.json")));
                self.records.lock().await.remove(&install_id);
            }
        }
        Ok(())
    }
}

fn validate_install_request(
    config: &crate::config::Config,
    request: &SkillInstallRequest,
) -> Result<()> {
    skills::validate_skill_id(&request.id)?;
    if request.id == "skill-installer" || request.id.starts_with('.') {
        return Err(anyhow!("reserved_id"));
    }
    if let Some(key) = request.idempotency_key.as_deref() {
        if key.is_empty() || key.len() > 128 {
            return Err(anyhow!("invalid_idempotency_key"));
        }
    }
    if let SkillInstallSource::Files { files } = &request.source {
        if files.is_empty() || files.len() > MAX_FILES {
            return Err(anyhow!("invalid_files"));
        }
        let mut paths = std::collections::HashSet::new();
        for file in files {
            let path = normalize_package_path(&file.path)?;
            if !paths.insert(path) {
                return Err(anyhow!("duplicate_path"));
            }
            let values = usize::from(file.url.is_some())
                + usize::from(file.content.is_some())
                + usize::from(file.content_base64.is_some());
            if values != 1 {
                return Err(anyhow!("invalid_file_source"));
            }
        }
    }
    let _ = config;
    Ok(())
}

async fn materialize_source(
    request: &SkillInstallRequest,
    staging: &Path,
    cancellations: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    install_id: &str,
) -> Result<Vec<SkillInstallFileSummary>> {
    let SkillInstallSource::Files { files } = &request.source else {
        return Err(anyhow!("source_not_supported"));
    };
    let cancel = cancellations.lock().await.get(install_id).cloned();
    let mut summaries = Vec::with_capacity(files.len());
    let mut total_bytes = 0_u64;
    for file in files {
        if cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(anyhow!("cancelled"));
        }
        let relative = normalize_package_path(&file.path)?;
        let (bytes, source_type) = inline_file_bytes(file)?;
        let size = bytes.len() as u64;
        if size > MAX_FILE_BYTES || total_bytes.saturating_add(size) > MAX_PACKAGE_BYTES {
            return Err(anyhow!("package_limit_exceeded"));
        }
        if let Some(expected) = file.sha256.as_deref() {
            if hex_sha256(&bytes) != expected.to_ascii_lowercase() {
                return Err(anyhow!("digest_mismatch"));
            }
        }
        total_bytes += size;
        let destination = staging.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &bytes)?;
        set_mode(&destination, file.executable.unwrap_or(false))?;
        summaries.push(SkillInstallFileSummary {
            path: relative.to_string_lossy().replace('\\', "/"),
            size_bytes: size,
            sha256: hex_sha256(&bytes),
            source_type: source_type.to_string(),
        });
    }
    Ok(summaries)
}

fn inline_file_bytes(file: &SkillInstallFile) -> Result<(Vec<u8>, &'static str)> {
    if let Some(content) = file.content.as_deref() {
        return Ok((content.as_bytes().to_vec(), "inline_utf8"));
    }
    if let Some(content) = file.content_base64.as_deref() {
        return BASE64
            .decode(content)
            .map(|bytes| (bytes, "inline_base64"))
            .map_err(|_| anyhow!("invalid_base64"));
    }
    Err(anyhow!("source_not_supported"))
}

fn validate_staging(staging: &Path, id: &str) -> Result<()> {
    let skill_md = staging.join("SKILL.md");
    if !skill_md.is_file() {
        return Err(anyhow!("skill_md_missing"));
    }
    if fs::metadata(&skill_md)?.len() > MAX_SKILL_MD_BYTES {
        return Err(anyhow!("skill_md_too_large"));
    }
    let mut count = 0_usize;
    let mut total = 0_u64;
    validate_tree(staging, staging, &mut count, &mut total)?;
    if count > MAX_FILES || total > MAX_PACKAGE_BYTES {
        return Err(anyhow!("package_limit_exceeded"));
    }
    let _ = id;
    Ok(())
}

fn validate_tree(root: &Path, path: &Path, count: &mut usize, total: &mut u64) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink()
            || metadata.file_type().is_dir() && entry.file_name() == ".archive"
        {
            return Err(anyhow!("archive_invalid"));
        }
        if metadata.is_dir() {
            validate_tree(root, &entry.path(), count, total)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(anyhow!("archive_invalid"));
        }
        *count += 1;
        *total = total.saturating_add(metadata.len());
        let entry_path = entry.path();
        let relative = entry_path
            .strip_prefix(root)
            .map_err(|_| anyhow!("archive_invalid"))?;
        normalize_package_path(&relative.to_string_lossy())?;
    }
    Ok(())
}

fn commit_staging(
    config: &crate::config::Config,
    request: &SkillInstallRequest,
    staging: &Path,
    install_id: &str,
) -> Result<Option<PathBuf>> {
    let root = skills::skills_root(config);
    fs::create_dir_all(&root)?;
    let target = root.join(&request.id);
    let archive = root.join(".archive").join(&request.id).join(install_id);
    let had_target = target.exists();
    let mut archived = None;
    let journal_path = commit_journal_path(config, install_id);
    let journal = CommitJournal {
        install_id: install_id.to_string(),
        id: request.id.clone(),
        target: target.to_string_lossy().to_string(),
        archive: if had_target {
            Some(archive.to_string_lossy().to_string())
        } else {
            None
        },
        candidate_committed: false,
    };
    if let Some(parent) = journal_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&journal_path, serde_json::to_vec_pretty(&journal)?)?;
    if had_target {
        if !request.replace_existing {
            return Err(anyhow!("target_changed"));
        }
        if let Some(parent) = archive.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&target, &archive)?;
        archived = Some(archive.clone());
    }
    if let Err(error) = fs::rename(staging, &target) {
        if let Some(archive) = archived.as_ref() {
            let _ = fs::rename(archive, &target);
        }
        remove_commit_journal(config, install_id);
        return Err(anyhow!("commit_failed: {error}"));
    }
    let mut journal = journal;
    journal.candidate_committed = true;
    if let Err(error) = fs::write(&journal_path, serde_json::to_vec_pretty(&journal)?) {
        let _ = fs::remove_dir_all(&target);
        if let Some(archive) = archived.as_ref() {
            let _ = fs::rename(archive, &target);
        }
        remove_commit_journal(config, install_id);
        return Err(anyhow!("commit_journal_failed: {error}"));
    }
    Ok(archived)
}

fn rollback_commit(config: &crate::config::Config, id: &str, archive: Option<&Path>) -> Result<()> {
    let target = skills::skills_root(config).join(id);
    let _ = fs::remove_dir_all(&target);
    if let Some(archive) = archive {
        fs::rename(archive, target)?;
    }
    Ok(())
}

fn commit_journal_path(config: &crate::config::Config, install_id: &str) -> PathBuf {
    install_records_root(config).join(format!("{install_id}.commit.json"))
}

fn remove_commit_journal(config: &crate::config::Config, install_id: &str) {
    let _ = fs::remove_file(commit_journal_path(config, install_id));
}

fn package_sha256(config: &crate::config::Config, id: &str) -> Result<String> {
    let root = skills::skills_root(config).join(id);
    let mut files = Vec::new();
    collect_files(&root, &root, &mut files)?;
    files.sort();
    let mut digest = Sha256::new();
    for file in files {
        digest.update(file.as_bytes());
        digest.update(fs::read(root.join(&file))?);
    }
    Ok(format_digest(digest.finalize()))
}

fn collect_files(base: &Path, current: &Path, files: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect_files(base, &entry.path(), files)?;
        } else if entry.file_type()?.is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(base)
                    .map_err(|_| anyhow!("package_invalid"))?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn normalize_package_path(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains('\0') || path.contains('\\')
    {
        return Err(anyhow!("invalid_path"));
    }
    let mut output = PathBuf::new();
    let mut depth = 0;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                output.push(part);
                depth += 1;
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return Err(anyhow!("invalid_path")),
        }
    }
    if depth == 0 || depth > MAX_PATH_DEPTH {
        return Err(anyhow!("invalid_path"));
    }
    Ok(output)
}

fn set_mode(path: &Path, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
        )?;
    }
    let _ = (path, executable);
    Ok(())
}

fn initial_progress(source: &SkillInstallSource) -> SkillInstallProgress {
    let total = match source {
        SkillInstallSource::Files { files } => files.len() as u64,
        SkillInstallSource::Github { .. } => 0,
    };
    SkillInstallProgress {
        files_completed: 0,
        files_total: total,
        bytes_downloaded: 0,
        bytes_total: None,
    }
}

fn source_summary(source: &SkillInstallSource) -> SkillInstallSourceSummary {
    match source {
        SkillInstallSource::Github {
            repository,
            url,
            ref_name,
            path,
        } => SkillInstallSourceSummary {
            source_type: "github".to_string(),
            repository: repository.clone().or_else(|| url.clone()),
            requested_ref: ref_name.clone(),
            resolved_commit: None,
            path: path.clone(),
            files: Vec::new(),
        },
        SkillInstallSource::Files { .. } => SkillInstallSourceSummary {
            source_type: "files".to_string(),
            ..Default::default()
        },
    }
}

fn start_response(
    status: &SkillInstallStatusResponse,
    deduplicated: bool,
) -> SkillInstallStartResponse {
    SkillInstallStartResponse {
        install_id: status.install_id.clone(),
        id: status.id.clone(),
        status: status.status,
        queued: status.status == SkillInstallStatus::Queued,
        deduplicated,
        created_at: status.created_at,
        updated_at: status.updated_at,
        poll_after_ms: status.poll_after_ms,
    }
}

fn cancel_response(
    status: &SkillInstallStatusResponse,
    outcome: SkillInstallCancelOutcome,
    changed: bool,
) -> SkillInstallCancelResponse {
    SkillInstallCancelResponse {
        install_id: status.install_id.clone(),
        outcome,
        changed,
        status: status.status,
        phase: status.phase,
        cancel_requested_at: status.cancel_requested_at,
    }
}

fn set_running(status: &mut SkillInstallStatusResponse) {
    status.status = SkillInstallStatus::Running;
    status.started_at = Some(Utc::now());
    status.poll_after_ms = DEFAULT_POLL_AFTER_MS;
}

fn touch_status(status: &mut SkillInstallStatusResponse) {
    status.revision += 1;
    status.updated_at = Utc::now();
    status.elapsed_ms = status
        .started_at
        .map(|started| {
            (status.finished_at.unwrap_or(status.updated_at) - started)
                .num_milliseconds()
                .max(0) as u64
        })
        .unwrap_or(0);
    if is_terminal(status.status) {
        status.poll_after_ms = 0;
    }
}

fn is_terminal(status: SkillInstallStatus) -> bool {
    matches!(
        status,
        SkillInstallStatus::Completed | SkillInstallStatus::Failed | SkillInstallStatus::Cancelled
    )
}

fn safe_error_message(message: &str) -> String {
    if message == "cancelled" {
        "installation cancelled".to_string()
    } else {
        message.chars().take(256).collect()
    }
}

fn remaining_deadline(deadline: DateTime<Utc>) -> Duration {
    let seconds = (deadline - Utc::now()).num_seconds().max(0) as u64;
    Duration::from_secs(seconds)
}

fn install_records_root(config: &crate::config::Config) -> PathBuf {
    config.workspace_root.join("state").join("skill-installs")
}

fn staging_path(config: &crate::config::Config, install_id: &str) -> PathBuf {
    install_records_root(config)
        .join("staging")
        .join(install_id)
}

fn sha256_json<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(hex_sha256(&serde_json::to_vec(value)?))
}

fn hex_sha256(bytes: &[u8]) -> String {
    format_digest(Sha256::digest(bytes))
}

fn format_digest<D: AsRef<[u8]>>(digest: D) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, state::RunMode};
    use agentic_gpt_protocol::{SkillInstallFile, SkillInstallGetRequest};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn test_state() -> AppState {
        let root =
            std::env::temp_dir().join(format!("agentic-install-{}", Uuid::new_v4().simple()));
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root;
        config.confirmation_provider.provider = "none".to_string();
        AppState {
            config_path: PathBuf::from("test-config.json"),
            config: Arc::new(RwLock::new(config)),
            run_mode: RunMode::Room,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_installs: Arc::new(InstallManager::new()),
        }
    }

    fn inline_request(id: &str) -> SkillInstallRequest {
        SkillInstallRequest {
            id: id.to_string(),
            source: SkillInstallSource::Files {
                files: vec![
                    SkillInstallFile {
                        path: "SKILL.md".to_string(),
                        url: None,
                        content: Some("# Installed\n".to_string()),
                        content_base64: None,
                        sha256: None,
                        executable: Some(false),
                    },
                    SkillInstallFile {
                        path: "references/info.txt".to_string(),
                        url: None,
                        content: Some("info".to_string()),
                        content_base64: None,
                        sha256: None,
                        executable: None,
                    },
                ],
            },
            replace_existing: false,
            activate_after_install: None,
            idempotency_key: None,
        }
    }

    #[tokio::test]
    async fn inline_install_is_persisted_and_completes_atomically() {
        let state = test_state();
        let manager = state.skill_installs.clone();
        let response = manager
            .start(state.clone(), inline_request("demo"))
            .await
            .unwrap();
        assert!(response.queued);
        assert!(!response.install_id.is_empty());

        let status = manager
            .get(
                &state,
                SkillInstallGetRequest {
                    install_id: response.install_id.clone(),
                    wait_seconds: Some(5),
                },
            )
            .await
            .unwrap();
        assert_eq!(status.status, SkillInstallStatus::Completed);
        assert_eq!(status.poll_after_ms, 0);
        assert!(status.result.is_some());
        let config = state.config.read().await.clone();
        assert!(config.workspace_root.join("skills/demo/SKILL.md").is_file());
        assert!(config
            .workspace_root
            .join("skills/demo/references/info.txt")
            .is_file());
        assert!(config
            .workspace_root
            .join("state/skill-installs")
            .join(format!("{}.json", response.install_id))
            .is_file());
    }

    #[tokio::test]
    async fn idempotency_retries_return_the_original_install() {
        let state = test_state();
        let manager = state.skill_installs.clone();
        let mut request = inline_request("demo");
        request.idempotency_key = Some("retry-key".to_string());
        let first = manager.start(state.clone(), request.clone()).await.unwrap();
        let second = manager.start(state.clone(), request).await.unwrap();
        assert_eq!(first.install_id, second.install_id);
        assert!(second.deduplicated);
    }

    #[tokio::test]
    async fn queued_cancel_is_idempotent_and_status_is_terminal() {
        let state = test_state();
        let manager = state.skill_installs.clone();
        let response = manager
            .start(state.clone(), inline_request("demo"))
            .await
            .unwrap();
        let cancelled = manager
            .cancel(
                &state,
                SkillInstallCancelRequest {
                    install_id: response.install_id.clone(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            cancelled.outcome,
            SkillInstallCancelOutcome::Cancelled | SkillInstallCancelOutcome::CancelRequested
        ));
        let repeated = manager
            .cancel(
                &state,
                SkillInstallCancelRequest {
                    install_id: response.install_id,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            repeated.outcome,
            SkillInstallCancelOutcome::AlreadyCancelled
                | SkillInstallCancelOutcome::AlreadyTerminal
                | SkillInstallCancelOutcome::TooLate
        ));
    }
}
