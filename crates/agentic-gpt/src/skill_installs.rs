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
use reqwest::{Client, StatusCode, Url};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::net::IpAddr;
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
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_concurrency(2)
    }

    pub(crate) fn with_concurrency(max_concurrent_installs: usize) -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
            target_locks: Arc::new(Mutex::new(HashMap::new())),
            changed: Arc::new(Notify::new()),
            worker_slots: Arc::new(Semaphore::new(max_concurrent_installs.max(1))),
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
            if validate_install_id(&record.install_id).is_err() {
                continue;
            }
            if is_terminal(record.status.status) {
                // A terminal record should have no live commit journal. If a
                // process stopped after persisting the terminal state but
                // before journal cleanup, the commit has already reached its
                // terminal outcome, so only discard the stale marker here.
                remove_commit_journal(&config, &record.install_id);
                self.records
                    .lock()
                    .await
                    .insert(record.install_id.clone(), record);
                continue;
            }
            if let Err(error) = reconcile_commit_journal(&config, &record.install_id) {
                record.status.status = SkillInstallStatus::Failed;
                record.status.phase = None;
                record.status.error = Some(SkillInstallError {
                    code: "recovery_failed".to_string(),
                    message: format!("commit journal reconciliation failed: {error}"),
                    phase: Some(SkillInstallPhase::Committing),
                    retryable: true,
                });
                record.status.finished_at = Some(Utc::now());
                record.status.poll_after_ms = 0;
                touch_status(&mut record.status);
                self.save_cache(&config, &record).await?;
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
            max_attempts: config.skills.max_attempts.max(1),
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
        let deadline = started + ChronoDuration::seconds(config.skills.total_deadline_secs as i64);
        let max_attempts = config.skills.max_attempts.max(1);
        let mut materialized = None;
        for attempt in 1..=max_attempts {
            record = self.load_record(&config, install_id).await?;
            record.status.attempt = attempt;
            record.status.phase = Some(if attempt == 1 {
                SkillInstallPhase::Resolving
            } else {
                SkillInstallPhase::Downloading
            });
            touch_status(&mut record.status);
            self.save_cache(&config, &record).await?;
            let remaining = remaining_deadline(deadline);
            if remaining.is_zero() {
                let _ = fs::remove_dir_all(&staging);
                return self
                    .fail(
                        state,
                        install_id,
                        "install_deadline_exceeded",
                        Some(SkillInstallPhase::Downloading),
                        true,
                    )
                    .await;
            }
            let result = tokio::select! {
                result = timeout(
                    remaining,
                    materialize_source(
                        &config,
                        &record.request,
                        &staging,
                        &self.cancellations,
                        install_id,
                    ),
                ) => Some(result),
                _ = self.wait_until_cancelled(install_id) => None,
            };
            match result {
                None => {
                    let _ = fs::remove_dir_all(&staging);
                    return self.cancelled(state, install_id).await;
                }
                Some(Ok(Ok(value))) => {
                    materialized = Some(value);
                    break;
                }
                Some(Ok(Err(error))) => {
                    let message = error.to_string();
                    if message == "cancelled" || self.is_cancelled(install_id).await {
                        let _ = fs::remove_dir_all(&staging);
                        return self.cancelled(state, install_id).await;
                    }
                    if !is_retryable_materialization_error(&message) || attempt == max_attempts {
                        let _ = fs::remove_dir_all(&staging);
                        return self
                            .fail(
                                state,
                                install_id,
                                &message,
                                Some(SkillInstallPhase::Downloading),
                                is_retryable_materialization_error(&message),
                            )
                            .await;
                    }
                    let _ = fs::remove_dir_all(&staging);
                    let backoff = Duration::from_secs(1_u64 << (attempt - 1).min(5));
                    tokio::time::sleep(backoff.min(remaining)).await;
                    fs::create_dir_all(&staging)?;
                }
                Some(Err(_)) => {
                    let _ = fs::remove_dir_all(&staging);
                    return self
                        .fail(
                            state,
                            install_id,
                            "install_deadline_exceeded",
                            Some(SkillInstallPhase::Downloading),
                            true,
                        )
                        .await;
                }
            }
        }
        let materialized = materialized.ok_or_else(|| anyhow!("materialization_failed"))?;
        if self.is_cancelled(install_id).await {
            let _ = fs::remove_dir_all(&staging);
            return self.cancelled(state, install_id).await;
        }
        record = self.load_record(&config, install_id).await?;
        record.status.phase = Some(SkillInstallPhase::Validating);
        record.status.progress.files_completed = materialized.summaries.len() as u64;
        record.status.progress.files_total = materialized.summaries.len() as u64;
        record.status.progress.bytes_downloaded = materialized
            .summaries
            .iter()
            .map(|summary| summary.size_bytes)
            .sum();
        record.status.progress.bytes_total = Some(record.status.progress.bytes_downloaded);
        record.status.source = materialized.source;
        touch_status(&mut record.status);
        self.save_cache(&config, &record).await?;
        normalize_directory_modes(&staging)?;
        validate_staging(&staging, &record.request.id, &config.skills)?;

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
        let target_result = tokio::select! {
            result = timeout(remaining, target_lock.lock()) => Some(result),
            _ = self.wait_until_cancelled(install_id) => None,
        };
        let guard = match target_result {
            Some(Ok(guard)) => guard,
            None if self.is_cancelled(install_id).await => {
                let _ = fs::remove_dir_all(&staging);
                return self.cancelled(state, install_id).await;
            }
            Some(Err(_)) | None => {
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
        let lease_result = tokio::select! {
            result = state.skill_leases.acquire_exclusive(
                &record.request.id,
                remaining_deadline(deadline),
            ) => Some(result),
            _ = self.wait_until_cancelled(install_id) => None,
        };
        let lease = match lease_result {
            Some(Ok(lease)) => lease,
            None if self.is_cancelled(install_id).await => {
                drop(guard);
                let _ = fs::remove_dir_all(&staging);
                return self.cancelled(state, install_id).await;
            }
            Some(Err(_)) | None => {
                drop(guard);
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
            drop(lease);
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
            code: install_error_code(message),
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
        validate_install_id(&record.install_id)?;
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
        validate_install_id(install_id)?;
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
        validate_install_id(&record.install_id)?;
        if record.install_id != install_id {
            return Err(anyhow!("install_record_mismatch"));
        }
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
                if validate_install_id(&record.install_id).is_err() {
                    continue;
                }
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

    async fn wait_until_cancelled(&self, install_id: &str) {
        while !self.is_cancelled(install_id).await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
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
        if files.is_empty() || files.len() > config.skills.max_files {
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
            if let Some(url) = file.url.as_deref() {
                let parsed = Url::parse(url).map_err(|_| anyhow!("download_blocked"))?;
                if parsed.scheme() != "https"
                    || parsed.username() != ""
                    || parsed.password().is_some()
                    || parsed.host_str().is_none()
                {
                    return Err(anyhow!("download_blocked"));
                }
            }
        }
    }
    if let SkillInstallSource::Github {
        repository, url, ..
    } = &request.source
    {
        let _ = parse_github_source(repository.as_deref(), url.as_deref())?;
    }
    if let SkillInstallSource::Files { files } = &request.source {
        let mut inline_total = 0_u64;
        for file in files {
            inline_total = inline_total.saturating_add(
                file.content
                    .as_deref()
                    .map_or(0, |content| content.len() as u64),
            );
            if let Some(content) = file.content_base64.as_deref() {
                let decoded = BASE64
                    .decode(content)
                    .map_err(|_| anyhow!("invalid_base64"))?;
                inline_total = inline_total.saturating_add(decoded.len() as u64);
            }
        }
        if inline_total > config.skills.max_inline_bytes {
            return Err(anyhow!("package_limit_exceeded"));
        }
    }
    Ok(())
}

struct MaterializedSource {
    summaries: Vec<SkillInstallFileSummary>,
    source: SkillInstallSourceSummary,
}

async fn materialize_source(
    config: &crate::config::Config,
    request: &SkillInstallRequest,
    staging: &Path,
    cancellations: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    install_id: &str,
) -> Result<MaterializedSource> {
    if let SkillInstallSource::Github { .. } = &request.source {
        return materialize_github(config, request, staging, cancellations, install_id).await;
    }
    let SkillInstallSource::Files { files } = &request.source else {
        return Err(anyhow!("source_not_supported"));
    };
    let cancel = cancellations.lock().await.get(install_id).cloned();
    let mut summaries = Vec::with_capacity(files.len());
    let mut planned_paths = std::collections::HashSet::new();
    let mut planned_casefolded_paths = std::collections::HashSet::new();
    let mut total_bytes = 0_u64;
    let mut source = source_summary(&request.source);
    for file in files {
        if cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(anyhow!("cancelled"));
        }
        let relative = normalize_package_path(&file.path)?;
        register_package_path(&mut planned_paths, &mut planned_casefolded_paths, &relative)?;
        let (bytes, source_type) = if let Some(url) = file.url.as_deref() {
            (download_url(config, url).await?, "url")
        } else {
            inline_file_bytes(config, file)?
        };
        let size = bytes.len() as u64;
        if size > config.skills.max_file_bytes
            || total_bytes.saturating_add(size) > config.skills.max_package_bytes
        {
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
    source.files = summaries.clone();
    Ok(MaterializedSource { summaries, source })
}

fn inline_file_bytes(
    config: &crate::config::Config,
    file: &SkillInstallFile,
) -> Result<(Vec<u8>, &'static str)> {
    if let Some(content) = file.content.as_deref() {
        if content.len() as u64 > config.skills.max_inline_bytes {
            return Err(anyhow!("package_limit_exceeded"));
        }
        return Ok((content.as_bytes().to_vec(), "inline_utf8"));
    }
    if let Some(content) = file.content_base64.as_deref() {
        let bytes = BASE64
            .decode(content)
            .map_err(|_| anyhow!("invalid_base64"))?;
        if bytes.len() as u64 > config.skills.max_inline_bytes {
            return Err(anyhow!("package_limit_exceeded"));
        }
        return Ok((bytes, "inline_base64"));
    }
    Err(anyhow!("source_not_supported"))
}

fn is_retryable_materialization_error(message: &str) -> bool {
    message == "download_failed"
        || message == "download_timeout"
        || message
            .strip_prefix("download_http:")
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(|status| matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504))
}

fn register_package_path(
    paths: &mut std::collections::HashSet<PathBuf>,
    casefolded_paths: &mut std::collections::HashSet<String>,
    path: &Path,
) -> Result<()> {
    let key = path.to_string_lossy().replace('\\', "/").to_lowercase();
    if casefolded_paths.iter().any(|existing| {
        existing == &key
            || existing.starts_with(&format!("{key}/"))
            || key.starts_with(&format!("{existing}/"))
    }) {
        return Err(anyhow!("duplicate_path"));
    }
    paths.insert(path.to_path_buf());
    casefolded_paths.insert(key);
    Ok(())
}

async fn materialize_github(
    config: &crate::config::Config,
    request: &SkillInstallRequest,
    staging: &Path,
    cancellations: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    install_id: &str,
) -> Result<MaterializedSource> {
    let SkillInstallSource::Github {
        repository,
        url,
        ref_name,
        path,
    } = &request.source
    else {
        return Err(anyhow!("source_not_supported"));
    };
    let (repo, parsed_ref, parsed_path) =
        parse_github_source(repository.as_deref(), url.as_deref())?;
    let requested_ref = ref_name.clone().or(parsed_ref);
    let subtree = path.clone().or(parsed_path);
    let client = github_client(config)?;
    let repo_info =
        github_get_json(&client, &format!("https://api.github.com/repos/{repo}")).await?;
    let resolved_ref = requested_ref
        .clone()
        .or_else(|| {
            repo_info
                .get("default_branch")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .ok_or_else(|| anyhow!("ref_not_found"))?;
    let commit_info = github_get_json(
        &client,
        &format!(
            "https://api.github.com/repos/{repo}/commits/{}",
            urlencoding::encode(&resolved_ref)
        ),
    )
    .await?;
    let commit = commit_info
        .get("sha")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("ref_not_found"))?
        .to_string();
    let tree = github_get_json(
        &client,
        &format!("https://api.github.com/repos/{repo}/git/trees/{commit}?recursive=1"),
    )
    .await?;
    let prefix = subtree
        .as_deref()
        .map(|value| {
            normalize_package_path(value).map(|path| path.to_string_lossy().replace('\\', "/"))
        })
        .transpose()?;
    let entries = tree
        .get("tree")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("archive_invalid"))?;
    let cancel = cancellations.lock().await.get(install_id).cloned();
    let mut summaries = Vec::new();
    let mut planned_paths = std::collections::HashSet::new();
    let mut planned_casefolded_paths = std::collections::HashSet::new();
    let mut total = 0_u64;
    for entry in entries {
        if cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
        {
            return Err(anyhow!("cancelled"));
        }
        if entry.get("type").and_then(Value::as_str) != Some("blob") {
            continue;
        }
        let mode = entry
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("100644");
        if !matches!(mode, "100644" | "100755") {
            return Err(anyhow!("archive_invalid"));
        }
        let repo_path = entry
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let relative = match prefix.as_deref() {
            Some(prefix) if repo_path == prefix => PathBuf::from(
                Path::new(repo_path)
                    .file_name()
                    .ok_or_else(|| anyhow!("invalid_path"))?,
            ),
            Some(prefix) => {
                let Some(rest) = repo_path.strip_prefix(&format!("{prefix}/")) else {
                    continue;
                };
                normalize_package_path(rest)?
            }
            None => normalize_package_path(repo_path)?,
        };
        if relative.as_os_str().is_empty() {
            continue;
        }
        register_package_path(&mut planned_paths, &mut planned_casefolded_paths, &relative)?;
        let raw_url = format!(
            "https://raw.githubusercontent.com/{repo}/{commit}/{}",
            repo_path
                .split('/')
                .map(urlencoding::encode)
                .collect::<Vec<_>>()
                .join("/")
        );
        let bytes = download_url_with_client(config, &client, &raw_url).await?;
        let size = bytes.len() as u64;
        if size > config.skills.max_file_bytes
            || total.saturating_add(size) > config.skills.max_package_bytes
        {
            return Err(anyhow!("package_limit_exceeded"));
        }
        total += size;
        let destination = staging.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, &bytes)?;
        let executable = mode == "100755";
        set_mode(&destination, executable)?;
        summaries.push(SkillInstallFileSummary {
            path: relative.to_string_lossy().replace('\\', "/"),
            size_bytes: size,
            sha256: hex_sha256(&bytes),
            source_type: "github".to_string(),
        });
        if summaries.len() > config.skills.max_files {
            return Err(anyhow!("package_limit_exceeded"));
        }
    }
    if summaries.is_empty() {
        return Err(anyhow!("source_not_found"));
    }
    Ok(MaterializedSource {
        summaries: summaries.clone(),
        source: SkillInstallSourceSummary {
            source_type: "github".to_string(),
            repository: Some(repo),
            requested_ref: Some(resolved_ref),
            resolved_commit: Some(commit),
            path: subtree,
            files: summaries,
        },
    })
}

fn parse_github_source(
    repository: Option<&str>,
    url: Option<&str>,
) -> Result<(String, Option<String>, Option<String>)> {
    if repository.is_some() == url.is_some() {
        return Err(anyhow!("invalid_github_source"));
    }
    if let Some(repository) = repository {
        let parts = repository.trim().split('/').collect::<Vec<_>>();
        if parts.len() != 2
            || parts.iter().any(|part| {
                part.is_empty()
                    || *part == "."
                    || *part == ".."
                    || !part.chars().all(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
                    })
            })
        {
            return Err(anyhow!("invalid_github_repository"));
        }
        return Ok((repository.trim().to_string(), None, None));
    }
    let url = Url::parse(url.unwrap()).map_err(|_| anyhow!("invalid_github_url"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.username() != ""
        || url.password().is_some()
    {
        return Err(anyhow!("unsupported_github_host"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(anyhow!("ambiguous_github_url"));
    }
    let parts = url
        .path_segments()
        .ok_or_else(|| anyhow!("invalid_github_url"))?
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 || parts[0].contains('.') || parts[1].contains('.') {
        return Err(anyhow!("invalid_github_url"));
    }
    if parts[..2].iter().any(|part| {
        part.is_empty()
            || !part.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    }) {
        return Err(anyhow!("invalid_github_url"));
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    if parts.len() == 2 {
        return Ok((repo, None, None));
    }
    if parts.len() < 4 || !matches!(parts[2], "tree" | "blob") {
        return Err(anyhow!("ambiguous_github_url"));
    }
    let ref_name = parts[3].to_string();
    let path = if parts.len() > 4 {
        Some(parts[4..].join("/"))
    } else {
        None
    };
    Ok((repo, Some(ref_name), path))
}

fn github_client(config: &crate::config::Config) -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(config.skills.connect_timeout_secs))
        .timeout(Duration::from_secs(config.skills.request_timeout_secs))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("agentic-gpt-skill-installer")
        .build()
        .map_err(|error| anyhow!("download_client_failed: {error}"))
}

async fn github_get_json(client: &Client, url: &str) -> Result<Value> {
    let response = client
        .get(url)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| anyhow!("download_failed"))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(anyhow!("source_not_found"));
    }
    if !response.status().is_success() {
        return Err(anyhow!("download_http:{}", response.status().as_u16()));
    }
    response
        .json()
        .await
        .map_err(|_| anyhow!("archive_invalid"))
}

async fn download_url(config: &crate::config::Config, url: &str) -> Result<Vec<u8>> {
    let client = github_client(config)?;
    download_url_with_client(config, &client, url).await
}

async fn download_url_with_client(
    config: &crate::config::Config,
    client: &Client,
    url: &str,
) -> Result<Vec<u8>> {
    let mut current = Url::parse(url).map_err(|_| anyhow!("download_blocked"))?;
    for redirect in 0..=config.skills.max_redirects {
        validate_public_url(config, &current).await?;
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|_| anyhow!("download_failed"))?;
        if response.status().is_redirection() {
            if redirect == config.skills.max_redirects {
                return Err(anyhow!("download_failed"));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| anyhow!("download_failed"))?;
            current = current
                .join(location)
                .map_err(|_| anyhow!("download_blocked"))?;
            continue;
        }
        if !response.status().is_success() {
            if response.status() == StatusCode::NOT_FOUND {
                return Err(anyhow!("source_not_found"));
            }
            return Err(anyhow!("download_http:{}", response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|size| size > config.skills.max_file_bytes)
        {
            return Err(anyhow!("package_limit_exceeded"));
        }
        let mut bytes = Vec::new();
        let mut response = response;
        while let Some(chunk) = tokio::time::timeout(
            Duration::from_secs(config.skills.idle_timeout_secs),
            response.chunk(),
        )
        .await
        .map_err(|_| anyhow!("download_timeout"))?
        .map_err(|_| anyhow!("download_failed"))?
        {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > config.skills.max_file_bytes {
                return Err(anyhow!("package_limit_exceeded"));
            }
        }
        return Ok(bytes);
    }
    Err(anyhow!("download_failed"))
}

async fn validate_public_url(config: &crate::config::Config, url: &Url) -> Result<()> {
    if url.scheme() != "https" || url.username() != "" || url.password().is_some() {
        return Err(anyhow!("download_blocked"));
    }
    let host = url.host_str().ok_or_else(|| anyhow!("download_blocked"))?;
    if !config.skills.allowed_hosts.is_empty()
        && !config
            .room
            .skills
            .allowed_hosts
            .iter()
            .any(|allowed| allowed == host)
    {
        return Err(anyhow!("download_blocked"));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("download_blocked"))?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| anyhow!("download_blocked"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(anyhow!("download_blocked"));
    }
    Ok(())
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !ip.is_loopback()
                && !ip.is_private()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && octets[0] != 0
                && !(octets[0] == 100 && (64..=127).contains(&octets[1]))
                && !(octets[0] == 192 && octets[1] == 0)
                && !(octets[0] == 198 && (18..=19).contains(&octets[1]))
                && !(octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
                && !(octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && (segments[0] & 0xfe00) != 0xfc00
                && (segments[0] & 0xffc0) != 0xfe80
        }
    }
}

fn validate_staging(
    staging: &Path,
    id: &str,
    limits: &crate::config::RoomSkillsConfig,
) -> Result<()> {
    let skill_md = staging.join("SKILL.md");
    if !skill_md.is_file() {
        return Err(anyhow!("skill_md_missing"));
    }
    if fs::metadata(&skill_md)?.len() > limits.max_skill_md_bytes {
        return Err(anyhow!("skill_md_too_large"));
    }
    let mut count = 0_usize;
    let mut total = 0_u64;
    validate_tree(staging, staging, &mut count, &mut total)?;
    if count > limits.max_files || total > limits.max_package_bytes {
        return Err(anyhow!("package_limit_exceeded"));
    }
    let _ = id;
    Ok(())
}

fn normalize_directory_modes(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        normalize_directory_modes(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
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
    if validate_install_id(install_id).is_err() {
        return;
    }
    let _ = fs::remove_file(commit_journal_path(config, install_id));
}

fn reconcile_commit_journal(config: &crate::config::Config, install_id: &str) -> Result<()> {
    validate_install_id(install_id)?;
    let journal_path = commit_journal_path(config, install_id);
    if !journal_path.exists() {
        return Ok(());
    }
    let journal: CommitJournal = serde_json::from_slice(&fs::read(&journal_path)?)?;
    if journal.install_id != install_id {
        return Err(anyhow!("commit_journal_mismatch"));
    }
    skills::validate_skill_id(&journal.id)?;
    let skills_root = skills::skills_root(config);
    let target = PathBuf::from(&journal.target);
    let expected_target = skills_root.join(&journal.id);
    if target != expected_target {
        return Err(anyhow!("commit_journal_target_invalid"));
    }
    let archive = journal.archive.as_ref().map(PathBuf::from);
    if let Some(archive) = archive.as_ref() {
        let expected_archive = skills_root
            .join(".archive")
            .join(&journal.id)
            .join(install_id);
        if archive != &expected_archive {
            return Err(anyhow!("commit_journal_archive_invalid"));
        }
    }

    let target_present = fs::symlink_metadata(&target).is_ok();
    let archive_present = archive
        .as_ref()
        .is_some_and(|path| fs::symlink_metadata(path).is_ok());
    if journal.candidate_committed {
        // The candidate rename completed. Roll it back, restoring the
        // archived package when one exists. A malformed or externally changed
        // archive is left in place and reported instead of being overwritten.
        if target_present {
            remove_path(&target)?;
        }
        if let Some(archive) = archive.as_ref() {
            if archive_present {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(archive, &target)?;
            }
        }
    } else if archive_present {
        // The old target was moved but the candidate rename did not complete.
        // Preserve any unexpected target rather than deleting user data.
        if target_present {
            return Err(anyhow!("commit_journal_target_changed"));
        }
        if let Some(archive) = archive.as_ref() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(archive, &target)?;
        }
    }
    fs::remove_file(journal_path)?;
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn validate_install_id(install_id: &str) -> Result<()> {
    if install_id.is_empty()
        || install_id.len() > 128
        || install_id.starts_with('.')
        || !install_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(anyhow!("invalid_install_id"));
    }
    Ok(())
}

pub(crate) fn package_sha256(config: &crate::config::Config, id: &str) -> Result<String> {
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
        } => {
            let parsed_repository = url
                .as_deref()
                .and_then(|value| parse_github_source(None, Some(value)).ok())
                .map(|(repository, _, _)| repository);
            SkillInstallSourceSummary {
                source_type: "github".to_string(),
                repository: repository.clone().or(parsed_repository),
                requested_ref: ref_name.clone(),
                resolved_commit: None,
                path: path.clone(),
                files: Vec::new(),
            }
        }
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

fn install_error_code(message: &str) -> String {
    let code = message.split(':').next().unwrap_or(message);
    match code {
        "source_not_found"
        | "ref_not_found"
        | "download_blocked"
        | "download_failed"
        | "download_timeout"
        | "digest_mismatch"
        | "archive_invalid"
        | "package_limit_exceeded"
        | "skill_md_missing"
        | "skill_invalid"
        | "target_changed"
        | "target_busy"
        | "install_deadline_exceeded"
        | "activation_failed"
        | "recovery_failed" => code.to_string(),
        "download_http" => "download_failed".to_string(),
        _ => "internal_error".to_string(),
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
    use crate::config::Config;
    use agentic_gpt_protocol::{SkillInstallFile, SkillInstallGetRequest};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn test_state() -> AppState {
        let root =
            std::env::temp_dir().join(format!("agentic-install-{}", Uuid::new_v4().simple()));
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root;
        config.confirmation_provider.set_legacy("none").unwrap();
        AppState {
            config_path: PathBuf::from("test-config.json"),
            config: Arc::new(RwLock::new(config)),
            runtime: crate::state::RuntimeModel::hub(crate::state::CapabilityProfile::Room),
            started_at: chrono::Utc::now(),
            supervised: false,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(crate::sessions::SkillLeaseManager::new()),
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
    async fn idempotency_conflict_is_rejected_before_creating_a_second_job() {
        let state = test_state();
        let manager = state.skill_installs.clone();
        let mut first_request = inline_request("demo");
        first_request.idempotency_key = Some("same-key".to_string());
        manager.start(state.clone(), first_request).await.unwrap();
        let mut conflicting = inline_request("demo");
        conflicting.idempotency_key = Some("same-key".to_string());
        if let SkillInstallSource::Files { files } = &mut conflicting.source {
            files[0].content = Some("# Different\n".to_string());
        }
        assert_eq!(
            manager
                .start(state, conflicting)
                .await
                .unwrap_err()
                .to_string(),
            "idempotency_conflict"
        );
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

    #[test]
    fn github_sources_resolve_structured_and_convenience_forms() {
        assert_eq!(
            parse_github_source(Some("octo/demo"), None).unwrap(),
            ("octo/demo".to_string(), None, None)
        );
        assert_eq!(
            parse_github_source(None, Some("https://github.com/octo/demo/tree/main/scripts"))
                .unwrap(),
            (
                "octo/demo".to_string(),
                Some("main".to_string()),
                Some("scripts".to_string())
            )
        );
        assert!(
            parse_github_source(None, Some("https://github.com/octo/demo?token=secret")).is_err()
        );
        assert!(parse_github_source(None, Some("https://github.com:443/octo/demo")).is_ok());
        assert!(parse_github_source(None, Some("https://user:pass@github.com/octo/demo")).is_err());
    }

    #[test]
    fn package_paths_reject_case_conflicts_and_file_directory_collisions() {
        let mut exact = std::collections::HashSet::new();
        let mut folded = std::collections::HashSet::new();
        register_package_path(&mut exact, &mut folded, Path::new("scripts/run"))
            .expect("first path is valid");
        assert!(register_package_path(&mut exact, &mut folded, Path::new("SCRIPTS/RUN")).is_err());
        assert!(register_package_path(&mut exact, &mut folded, Path::new("scripts")).is_err());
        assert!(
            register_package_path(&mut exact, &mut folded, Path::new("scripts/run/out")).is_err()
        );
    }

    #[test]
    fn remote_policy_rejects_private_and_reserved_addresses() {
        assert!(!is_public_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("10.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("169.254.1.1".parse().unwrap()));
        assert!(!is_public_ip("::1".parse().unwrap()));
        assert!(!is_public_ip("fd00::1".parse().unwrap()));
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn transient_download_errors_are_the_only_retryable_materialization_failures() {
        assert!(is_retryable_materialization_error("download_failed"));
        assert!(is_retryable_materialization_error("download_timeout"));
        assert!(is_retryable_materialization_error("download_http:503"));
        assert!(!is_retryable_materialization_error("download_http:403"));
        assert!(!is_retryable_materialization_error("source_not_found"));
        assert_eq!(install_error_code("download_http:503"), "download_failed");
    }

    #[test]
    fn commit_journal_recovery_restores_archive_without_destroying_precommit_target() {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = std::env::temp_dir().join(format!(
            "agentic-install-journal-{}",
            Uuid::new_v4().simple()
        ));
        let root = skills::skills_root(&config);
        let target = root.join("demo");
        let archive = root.join(".archive/demo/install-test");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("SKILL.md"), "new").unwrap();
        fs::create_dir_all(&archive).unwrap();
        fs::write(archive.join("SKILL.md"), "old").unwrap();
        let journal = CommitJournal {
            install_id: "install-test".to_string(),
            id: "demo".to_string(),
            target: target.to_string_lossy().to_string(),
            archive: Some(archive.to_string_lossy().to_string()),
            candidate_committed: true,
        };
        fs::create_dir_all(install_records_root(&config)).unwrap();
        fs::write(
            commit_journal_path(&config, "install-test"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();

        reconcile_commit_journal(&config, "install-test").unwrap();
        assert_eq!(fs::read_to_string(target.join("SKILL.md")).unwrap(), "old");
        assert!(!archive.exists());
        assert!(!commit_journal_path(&config, "install-test").exists());

        // A journal written before the old target is moved must not delete it.
        fs::write(target.join("SKILL.md"), "still-old").unwrap();
        let precommit = CommitJournal {
            install_id: "install-test-2".to_string(),
            id: "demo".to_string(),
            target: target.to_string_lossy().to_string(),
            archive: None,
            candidate_committed: false,
        };
        fs::write(
            commit_journal_path(&config, "install-test-2"),
            serde_json::to_vec(&precommit).unwrap(),
        )
        .unwrap();
        reconcile_commit_journal(&config, "install-test-2").unwrap();
        assert_eq!(
            fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "still-old"
        );
    }

    #[test]
    fn commit_journal_recovery_rejects_paths_outside_skills_root() {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = std::env::temp_dir().join(format!(
            "agentic-install-journal-invalid-{}",
            Uuid::new_v4().simple()
        ));
        let outside = config.workspace_root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        let journal = CommitJournal {
            install_id: "install-test".to_string(),
            id: "demo".to_string(),
            target: outside.to_string_lossy().to_string(),
            archive: None,
            candidate_committed: true,
        };
        fs::create_dir_all(install_records_root(&config)).unwrap();
        fs::write(
            commit_journal_path(&config, "install-test"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();

        assert!(reconcile_commit_journal(&config, "install-test").is_err());
        assert!(outside.exists());
    }
}
