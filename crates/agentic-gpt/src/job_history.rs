#![allow(dead_code)]

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agentic_gpt_protocol::{JobDetail, JobInfo, JobKind, JobListRequest, JobState};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, types::Value, Connection, ErrorCode, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::private_state::PrivateStatePaths;
use crate::utils::log_warn;

const HISTORY_RETENTION: Duration = Duration::days(30);
const HISTORY_CAP_BYTES: u64 = 512 * 1024 * 1024;
const CLEANUP_INTERVAL: Duration = Duration::hours(1);
const MAX_PENDING_TERMINALS: usize = 100;
const MAX_PENDING_RETRY_ATTEMPTS: u8 = 5;
const PENDING_RETRY_BASE_SECONDS: i64 = 2;
const MAX_RICH_DETAIL_BYTES: usize = 1024 * 1024;
const MAX_RESULT_BYTES: usize = 512 * 1024;
const MAX_RESULT_PREVIEW_BYTES: usize = 8 * 1024;
const MAX_STRING_BYTES: usize = 64 * 1024;
const MAX_ERROR_BYTES: usize = 8 * 1024;
const MAX_ARGS: usize = 128;
const MAX_ARG_BYTES: usize = 4 * 1024;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS jobs (
    job_id TEXT PRIMARY KEY NOT NULL,
    agent_id TEXT NOT NULL,
    group_name TEXT,
    batch_id TEXT,
    batch_call_id TEXT,
    batch_index INTEGER,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    updated_at TEXT NOT NULL,
    finished_at TEXT,
    program TEXT,
    args_json TEXT NOT NULL,
    working_directory TEXT,
    command_preview TEXT,
    exit_code INTEGER,
    stdout_tail TEXT NOT NULL,
    stderr_tail TEXT NOT NULL,
    truncated INTEGER NOT NULL DEFAULT 0,
    reject_reason TEXT,
    skill_id TEXT,
    skill_path TEXT,
    installed_digest TEXT,
    mcp_server_id TEXT,
    mcp_tool_name TEXT,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    cancel_outcome TEXT,
    termination_evidence TEXT,
    detail_json TEXT,
    detail_bytes INTEGER
);
CREATE INDEX IF NOT EXISTS idx_jobs_created_job ON jobs(created_at DESC, job_id DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_group_created ON jobs(group_name, created_at DESC, job_id DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_kind_created ON jobs(kind, created_at DESC, job_id DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_state_created ON jobs(state, created_at DESC, job_id DESC);
CREATE INDEX IF NOT EXISTS idx_jobs_finished ON jobs(finished_at);
"#;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HistoryHealthStatus {
    Healthy,
    Degraded,
}

#[derive(Clone, Debug)]
pub(crate) struct HistoryHealth {
    pub(crate) status: HistoryHealthStatus,
    pub(crate) path: PathBuf,
    pub(crate) pending_terminal_count: usize,
    pub(crate) dropped_terminal_count: usize,
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HistoryWriteOutcome {
    Persisted,
    Deferred,
}

#[derive(Clone, Debug)]
pub(crate) struct JobHistoryRecord {
    pub(crate) info: JobInfo,
    pub(crate) detail: Option<JobDetail>,
}

#[derive(Clone, Debug)]
pub(crate) struct JobHistoryPage {
    pub(crate) jobs: Vec<JobInfo>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cursor {
    created_at: String,
    job_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JobHistoryCursor {
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) job_id: String,
}

#[derive(Debug)]
struct HealthState {
    status: HistoryHealthStatus,
    last_error: Option<String>,
    dropped_terminal_count: usize,
}

#[derive(Debug)]
struct PendingTerminal {
    detail: JobDetail,
    attempts: u8,
    next_retry_at: DateTime<Utc>,
}

pub(crate) struct JobHistoryStore {
    path: PathBuf,
    disabled: bool,
    connection: Mutex<Option<Connection>>,
    health: Mutex<HealthState>,
    pending_terminals: Mutex<VecDeque<PendingTerminal>>,
    last_cleanup: Mutex<Option<DateTime<Utc>>>,
}

impl JobHistoryStore {
    pub(crate) fn open(paths: &PrivateStatePaths) -> std::sync::Arc<Self> {
        let store = std::sync::Arc::new(Self {
            path: paths.root.join("jobs.sqlite3"),
            disabled: false,
            connection: Mutex::new(None),
            health: Mutex::new(HealthState {
                status: HistoryHealthStatus::Degraded,
                last_error: None,
                dropped_terminal_count: 0,
            }),
            pending_terminals: Mutex::new(VecDeque::new()),
            last_cleanup: Mutex::new(None),
        });
        store.initialize();
        store
    }

    pub(crate) fn disabled(path: PathBuf) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            path,
            disabled: true,
            connection: Mutex::new(None),
            health: Mutex::new(HealthState {
                status: HistoryHealthStatus::Degraded,
                last_error: Some("history_disabled".to_string()),
                dropped_terminal_count: 0,
            }),
            pending_terminals: Mutex::new(VecDeque::new()),
            last_cleanup: Mutex::new(None),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn health(&self) -> HistoryHealth {
        let pending = self
            .pending_terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        HistoryHealth {
            status: health.status.clone(),
            path: self.path.clone(),
            pending_terminal_count: pending,
            dropped_terminal_count: health.dropped_terminal_count,
            last_error: health.last_error.clone(),
        }
    }

    pub(crate) fn insert_admission(&self, info: &JobInfo) -> HistoryWriteOutcome {
        if let Err(error) = self.ensure_ready() {
            self.degrade(error);
            return HistoryWriteOutcome::Deferred;
        }
        let bounded = bounded_info(info);
        let result = self.with_connection(|connection| {
            let args_json = serde_json::to_string(&bounded.args)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            connection.execute(
                "INSERT OR IGNORE INTO jobs (
                    job_id, agent_id, group_name, batch_id, batch_call_id, batch_index,
                    kind, state, created_at, started_at, updated_at, finished_at,
                    program, args_json, working_directory, command_preview, exit_code,
                    stdout_tail, stderr_tail, truncated, reject_reason, skill_id, skill_path,
                    installed_digest, mcp_server_id, mcp_tool_name, cancel_requested,
                    cancel_outcome, termination_evidence, detail_json, detail_bytes
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,
                          ?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,NULL,NULL)",
                params![
                    bounded.job_id,
                    bounded.agent_id,
                    bounded.group,
                    bounded.batch_id,
                    bounded.batch_call_id,
                    bounded.batch_index.map(|value| value as i64),
                    kind_label(bounded.kind),
                    bounded.state.label(),
                    format_time(bounded.created_at),
                    bounded.started_at.map(format_time),
                    format_time(bounded.updated_at),
                    bounded.finished_at.map(format_time),
                    bounded.program,
                    args_json,
                    bounded.working_directory,
                    bounded.command_preview,
                    bounded.exit_code,
                    bounded.stdout_tail,
                    bounded.stderr_tail,
                    bounded.truncated as i64,
                    bounded.reject_reason,
                    bounded.skill_id,
                    bounded.skill_path,
                    bounded.installed_digest,
                    bounded.mcp_server_id,
                    bounded.mcp_tool_name,
                    bounded.cancel_requested as i64,
                    bounded.cancel_outcome,
                    bounded.termination_evidence,
                ],
            )?;
            Ok(())
        });
        match result {
            Ok(()) => {
                self.healthy();
                HistoryWriteOutcome::Persisted
            }
            Err(error) => {
                self.degrade(error);
                HistoryWriteOutcome::Deferred
            }
        }
    }

    pub(crate) fn mark_started(&self, info: &JobInfo) -> HistoryWriteOutcome {
        let Some(started_at) = info.started_at else {
            return HistoryWriteOutcome::Persisted;
        };
        if let Err(error) = self.ensure_ready() {
            self.degrade(error);
            return HistoryWriteOutcome::Deferred;
        }
        let result = self.with_connection(|connection| {
            connection.execute(
                "UPDATE jobs
                    SET state = ?1, started_at = ?2, updated_at = ?3
                  WHERE job_id = ?4 AND finished_at IS NULL",
                params![
                    info.state.label(),
                    format_time(started_at),
                    format_time(info.updated_at),
                    &info.job_id,
                ],
            )?;
            Ok(())
        });
        match result {
            Ok(()) => {
                self.healthy();
                HistoryWriteOutcome::Persisted
            }
            Err(error) => {
                self.degrade(error);
                HistoryWriteOutcome::Deferred
            }
        }
    }

    pub(crate) fn upsert_terminal(&self, detail: &JobDetail) -> HistoryWriteOutcome {
        let bounded = bounded_detail(detail.clone());
        let _ = self.retry_pending();
        if let Err(error) = self.ensure_ready() {
            self.defer_terminal(bounded, Utc::now());
            self.degrade(error);
            return HistoryWriteOutcome::Deferred;
        }
        let result = self.write_terminal(&bounded);
        match result {
            Ok(()) => {
                self.healthy();
                self.maybe_cleanup(Utc::now());
                HistoryWriteOutcome::Persisted
            }
            Err(error) => {
                self.defer_terminal(bounded, Utc::now());
                self.degrade(error);
                HistoryWriteOutcome::Deferred
            }
        }
    }

    pub(crate) fn retry_pending(&self) -> usize {
        self.retry_pending_at(Utc::now())
    }

    pub(crate) fn terminal_pending(&self, job_id: &str) -> bool {
        self.pending_terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|pending| pending.detail.job.job_id == job_id)
    }

    fn retry_pending_at(&self, now: DateTime<Utc>) -> usize {
        if let Err(error) = self.ensure_ready() {
            self.degrade(error);
            return 0;
        }
        let pending_count = self
            .pending_terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let mut persisted = 0;
        for _ in 0..pending_count {
            let pending = self
                .pending_terminals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front();
            let Some(mut pending) = pending else { break };
            if pending.next_retry_at > now {
                self.pending_terminals
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push_back(pending);
                continue;
            }
            match self.write_terminal(&pending.detail) {
                Ok(()) => {
                    persisted += 1;
                    self.healthy();
                }
                Err(error) => {
                    pending.attempts = pending.attempts.saturating_add(1);
                    if pending.attempts >= MAX_PENDING_RETRY_ATTEMPTS {
                        self.record_terminal_drop(
                            &pending.detail.job.job_id,
                            "persistence retry budget exhausted",
                        );
                    } else {
                        pending.next_retry_at = now + pending_retry_delay(pending.attempts);
                        self.pending_terminals
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push_back(pending);
                    }
                    self.degrade(error);
                }
            }
        }
        if persisted > 0 {
            self.maybe_cleanup(now);
        }
        persisted
    }

    pub(crate) fn get(&self, job_id: &str) -> Result<Option<JobHistoryRecord>> {
        self.ensure_ready().map_err(|error| anyhow!(error))?;
        self.with_connection(|connection| {
            let row = connection
                .query_row(
                    "SELECT job_id, agent_id, group_name, batch_id, batch_call_id, batch_index,
                            kind, state, created_at, started_at, updated_at, finished_at,
                            program, args_json, working_directory, command_preview, exit_code,
                            stdout_tail, stderr_tail, truncated, reject_reason, skill_id,
                            skill_path, installed_digest, mcp_server_id, mcp_tool_name,
                            cancel_requested, cancel_outcome, termination_evidence,
                            detail_json
                       FROM jobs WHERE job_id = ?1",
                    params![job_id],
                    row_to_record,
                )
                .optional()?;
            Ok(row)
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn list(&self, request: &JobListRequest) -> Result<JobHistoryPage> {
        self.ensure_ready().map_err(|error| anyhow!(error))?;
        let limit = request.effective_limit();
        let mut sql = String::from(
            "SELECT job_id, agent_id, group_name, batch_id, batch_call_id, batch_index,
                    kind, state, created_at, started_at, updated_at, finished_at,
                    program, args_json, working_directory, command_preview, exit_code,
                    stdout_tail, stderr_tail, truncated, reject_reason, skill_id,
                    skill_path, installed_digest, mcp_server_id, mcp_tool_name,
                    cancel_requested, cancel_outcome, termination_evidence, detail_json
               FROM jobs WHERE 1=1",
        );
        let mut values = Vec::<Value>::new();
        if let Some(group) = &request.group {
            sql.push_str(" AND group_name = ?");
            values.push(Value::Text(group.clone()));
        }
        if let Some(kind) = request.kind {
            sql.push_str(" AND kind = ?");
            values.push(Value::Text(kind_label(kind).to_string()));
        }
        if let Some(state) = request.state {
            sql.push_str(" AND state = ?");
            values.push(Value::Text(state.label().to_string()));
        }
        if let Some(cursor) = request.cursor.as_deref() {
            let cursor = decode_list_cursor(cursor)?;
            sql.push_str(" AND (created_at < ? OR (created_at = ? AND job_id < ?))");
            let created_at = format_time(cursor.created_at);
            values.push(Value::Text(created_at.clone()));
            values.push(Value::Text(created_at));
            values.push(Value::Text(cursor.job_id));
        }
        sql.push_str(" ORDER BY created_at DESC, job_id DESC LIMIT ?");
        values.push(Value::Integer((limit + 1) as i64));
        self.with_connection(|connection| {
            let params = values
                .iter()
                .map(|value| value as &dyn ToSql)
                .collect::<Vec<_>>();
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(params.as_slice(), row_to_info)?;
            let mut jobs = Vec::new();
            for row in rows {
                jobs.push(row?);
            }
            let next_cursor = if jobs.len() > limit {
                jobs.truncate(limit);
                let last = jobs.last().expect("limit row exists");
                Some(encode_list_cursor(last))
            } else {
                None
            };
            Ok(JobHistoryPage { jobs, next_cursor })
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn recover_active(&self, now: DateTime<Utc>) -> Result<usize> {
        self.ensure_ready().map_err(|error| anyhow!(error))?;
        self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE jobs SET state = ?1, finished_at = ?2, updated_at = ?2
                   WHERE state IN ('queued','waiting_confirmation','starting','running','cancel_requested')",
                params![JobState::UnknownAfterRestart.label(), format_time(now)],
            )?;
            Ok(changed)
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn logical_size_bytes(&self) -> Result<u64> {
        self.ensure_ready().map_err(|error| anyhow!(error))?;
        self.with_connection(|connection| {
            let page_count: u64 =
                connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
            let freelist: u64 =
                connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
            let page_size: u64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
            Ok(page_count
                .saturating_sub(freelist)
                .saturating_mul(page_size))
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn cleanup(&self, now: DateTime<Utc>) -> Result<usize> {
        self.cleanup_with_limits(now, HISTORY_RETENTION, HISTORY_CAP_BYTES)
    }

    fn cleanup_with_limits(
        &self,
        now: DateTime<Utc>,
        retention: Duration,
        cap_bytes: u64,
    ) -> Result<usize> {
        self.ensure_ready().map_err(|error| anyhow!(error))?;
        let deleted = self.with_connection(|connection| {
            let cutoff = format_time(now - retention);
            let mut deleted = connection.execute(
                "DELETE FROM jobs WHERE finished_at IS NOT NULL
                   AND state NOT IN ('queued','waiting_confirmation','starting','running','cancel_requested')
                   AND finished_at < ?1",
                params![cutoff],
            )?;
            loop {
                let page_count: u64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
                let freelist: u64 = connection.query_row("PRAGMA freelist_count", [], |row| row.get(0))?;
                let page_size: u64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
                let logical = page_count.saturating_sub(freelist).saturating_mul(page_size);
                if logical <= cap_bytes {
                    break;
                }
                let removed = connection.execute(
                    "DELETE FROM jobs WHERE job_id = (
                        SELECT job_id FROM jobs
                         WHERE finished_at IS NOT NULL
                           AND state NOT IN ('queued','waiting_confirmation','starting','running','cancel_requested')
                         ORDER BY finished_at ASC, created_at ASC, job_id ASC LIMIT 1
                    )",
                    [],
                )?;
                if removed == 0 {
                    break;
                }
                deleted += removed;
            }
            Ok(deleted)
        })
        .map_err(|error| anyhow!(error))?;
        *self
            .last_cleanup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(now);
        Ok(deleted)
    }

    fn initialize(&self) {
        let result = (|| -> Result<()> {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut connection = match Connection::open(&self.path) {
                Ok(connection) => connection,
                Err(error) if is_corruption(&error) => {
                    self.isolate_corrupt_database()?;
                    Connection::open(&self.path)?
                }
                Err(error) => return Err(error.into()),
            };
            connection.busy_timeout(std::time::Duration::from_millis(750))?;
            if let Err(error) = connection.execute_batch(SCHEMA) {
                if !is_corruption(&error) {
                    return Err(error.into());
                }
                drop(connection);
                self.isolate_corrupt_database()?;
                connection = Connection::open(&self.path)?;
                connection.busy_timeout(std::time::Duration::from_millis(750))?;
                connection.execute_batch(SCHEMA)?;
            }
            *self
                .connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection);
            self.recover_active(Utc::now())?;
            self.cleanup(Utc::now())?;
            Ok(())
        })();
        match result {
            Ok(()) => self.healthy(),
            Err(error) => self.degrade(error),
        }
    }

    fn ensure_ready(&self) -> Result<()> {
        if self.disabled {
            return Err(anyhow!("history_disabled"));
        }
        if self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
        {
            return Ok(());
        }
        let mut connection = Connection::open(&self.path)
            .map_err(|error| anyhow!(error))
            .and_then(|connection| {
                connection
                    .busy_timeout(std::time::Duration::from_millis(750))
                    .map_err(|error| anyhow!(error))?;
                Ok(connection)
            })?;
        if let Err(error) = connection.execute_batch(SCHEMA) {
            if !is_corruption(&error) {
                return Err(anyhow!(error));
            }
            drop(connection);
            self.isolate_corrupt_database()?;
            connection = Connection::open(&self.path).map_err(|error| anyhow!(error))?;
            connection
                .busy_timeout(std::time::Duration::from_millis(750))
                .map_err(|error| anyhow!(error))?;
            connection
                .execute_batch(SCHEMA)
                .map_err(|error| anyhow!(error))?;
        }
        *self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection);
        Ok(())
    }

    fn with_connection<R>(
        &self,
        operation: impl FnOnce(&Connection) -> rusqlite::Result<R>,
    ) -> rusqlite::Result<R> {
        let guard = self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let connection = guard.as_ref().ok_or_else(|| {
            rusqlite::Error::InvalidParameterName("history_database_unavailable".to_string())
        })?;
        operation(connection)
    }

    fn write_terminal(&self, detail: &JobDetail) -> Result<()> {
        let info = bounded_info(&detail.job);
        let bounded_detail = bounded_detail(detail.clone());
        let detail_json = serde_json::to_string(&bounded_detail)?;
        let detail_bytes = detail_json.len();
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO jobs (
                    job_id, agent_id, group_name, batch_id, batch_call_id, batch_index,
                    kind, state, created_at, started_at, updated_at, finished_at,
                    program, args_json, working_directory, command_preview, exit_code,
                    stdout_tail, stderr_tail, truncated, reject_reason, skill_id, skill_path,
                    installed_digest, mcp_server_id, mcp_tool_name, cancel_requested,
                    cancel_outcome, termination_evidence, detail_json, detail_bytes
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,
                          ?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31)
                ON CONFLICT(job_id) DO UPDATE SET
                    agent_id=excluded.agent_id, group_name=excluded.group_name,
                    batch_id=excluded.batch_id, batch_call_id=excluded.batch_call_id,
                    batch_index=excluded.batch_index, kind=excluded.kind, state=excluded.state,
                    created_at=excluded.created_at, started_at=excluded.started_at,
                    updated_at=excluded.updated_at, finished_at=excluded.finished_at,
                    program=excluded.program, args_json=excluded.args_json,
                    working_directory=excluded.working_directory, command_preview=excluded.command_preview,
                    exit_code=excluded.exit_code, stdout_tail=excluded.stdout_tail,
                    stderr_tail=excluded.stderr_tail, truncated=excluded.truncated,
                    reject_reason=excluded.reject_reason, skill_id=excluded.skill_id,
                    skill_path=excluded.skill_path, installed_digest=excluded.installed_digest,
                    mcp_server_id=excluded.mcp_server_id, mcp_tool_name=excluded.mcp_tool_name,
                    cancel_requested=excluded.cancel_requested, cancel_outcome=excluded.cancel_outcome,
                    termination_evidence=excluded.termination_evidence, detail_json=excluded.detail_json,
                    detail_bytes=excluded.detail_bytes",
                params![
                    info.job_id,
                    info.agent_id,
                    info.group,
                    info.batch_id,
                    info.batch_call_id,
                    info.batch_index.map(|value| value as i64),
                    kind_label(info.kind),
                    info.state.label(),
                    format_time(info.created_at),
                    info.started_at.map(format_time),
                    format_time(info.updated_at),
                    info.finished_at.map(format_time),
                    info.program,
                    serde_json::to_string(&info.args).unwrap_or_else(|_| "[]".to_string()),
                    info.working_directory,
                    info.command_preview,
                    info.exit_code,
                    info.stdout_tail,
                    info.stderr_tail,
                    info.truncated as i64,
                    info.reject_reason,
                    info.skill_id,
                    info.skill_path,
                    info.installed_digest,
                    info.mcp_server_id,
                    info.mcp_tool_name,
                    info.cancel_requested as i64,
                    info.cancel_outcome,
                    info.termination_evidence,
                    detail_json,
                    detail_bytes as i64,
                ],
            )?;
            Ok(())
        })
        .map_err(|error| anyhow!(error))
    }

    fn maybe_cleanup(&self, now: DateTime<Utc>) {
        let over_cap = self
            .logical_size_bytes()
            .map(|size| size > HISTORY_CAP_BYTES)
            .unwrap_or(false);
        let due = self
            .last_cleanup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .map(|last| now - last >= CLEANUP_INTERVAL)
            .unwrap_or(true);
        if due || over_cap {
            if let Err(error) = self.cleanup(now) {
                self.degrade(error);
            }
        }
    }

    fn defer_terminal(&self, detail: JobDetail, now: DateTime<Utc>) {
        let dropped_job_id = {
            let mut pending = self
                .pending_terminals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(existing) = pending
                .iter_mut()
                .find(|pending| pending.detail.job.job_id == detail.job.job_id)
            {
                existing.detail = detail;
                None
            } else {
                let dropped = if pending.len() >= MAX_PENDING_TERMINALS {
                    pending.pop_front().map(|pending| pending.detail.job.job_id)
                } else {
                    None
                };
                pending.push_back(PendingTerminal {
                    detail,
                    attempts: 1,
                    next_retry_at: now + pending_retry_delay(1),
                });
                dropped
            }
        };
        if let Some(job_id) = dropped_job_id {
            self.record_terminal_drop(&job_id, "pending terminal queue capacity exceeded");
        }
    }

    fn record_terminal_drop(&self, job_id: &str, reason: &str) {
        self.health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .dropped_terminal_count += 1;
        log_warn(format!(
            "job history terminal snapshot dropped; jobId={job_id}; reason={reason}"
        ));
    }

    fn healthy(&self) {
        let pending = self
            .pending_terminals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pending == 0 {
            health.status = HistoryHealthStatus::Healthy;
            health.last_error = None;
        } else {
            health.status = HistoryHealthStatus::Degraded;
            if health.last_error.is_none() {
                health.last_error = Some("terminal_history_pending_persistence".to_string());
            }
        }
    }

    fn degrade(&self, error: impl std::fmt::Display) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.status = HistoryHealthStatus::Degraded;
        health.last_error = Some(truncate_text(&error.to_string(), MAX_ERROR_BYTES));
    }

    fn isolate_corrupt_database(&self) -> Result<()> {
        let isolated = self
            .path
            .with_extension(format!("sqlite3.corrupt-{}", Uuid::new_v4().simple()));
        fs::rename(&self.path, isolated)?;
        Ok(())
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobHistoryRecord> {
    let info = row_to_info(row)?;
    let detail_json: Option<String> = row.get(29)?;
    let detail = detail_json
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(JobHistoryRecord { info, detail })
}

fn row_to_info(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobInfo> {
    let args_json: String = row.get(13)?;
    let kind: String = row.get(6)?;
    let state: String = row.get(7)?;
    Ok(JobInfo {
        job_id: row.get(0)?,
        agent_id: row.get(1)?,
        group: row.get(2)?,
        batch_id: row.get(3)?,
        batch_call_id: row.get(4)?,
        batch_index: row.get::<_, Option<i64>>(5)?.map(|value| value as usize),
        kind: parse_kind(&kind)?,
        state: parse_state(&state)?,
        created_at: parse_time(row.get::<_, String>(8)?)?,
        started_at: row
            .get::<_, Option<String>>(9)?
            .map(parse_time)
            .transpose()?,
        updated_at: parse_time(row.get::<_, String>(10)?)?,
        finished_at: row
            .get::<_, Option<String>>(11)?
            .map(parse_time)
            .transpose()?,
        program: row.get(12)?,
        args: serde_json::from_str(&args_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                13,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        working_directory: row.get(14)?,
        command_preview: row.get(15)?,
        exit_code: row.get(16)?,
        stdout_tail: row.get(17)?,
        stderr_tail: row.get(18)?,
        truncated: row.get::<_, i64>(19)? != 0,
        reject_reason: row.get(20)?,
        skill_id: row.get(21)?,
        skill_path: row.get(22)?,
        installed_digest: row.get(23)?,
        mcp_server_id: row.get(24)?,
        mcp_tool_name: row.get(25)?,
        cancel_requested: row.get::<_, i64>(26)? != 0,
        cancel_outcome: row.get(27)?,
        termination_evidence: row.get(28)?,
    })
}

fn parse_kind(value: &str) -> rusqlite::Result<JobKind> {
    match value {
        "process" => Ok(JobKind::Process),
        "skill" => Ok(JobKind::Skill),
        "mcp" => Ok(JobKind::Mcp),
        _ => Err(rusqlite::Error::InvalidColumnType(
            6,
            "kind".to_string(),
            rusqlite::types::Type::Text,
        )),
    }
}

fn kind_label(value: JobKind) -> &'static str {
    match value {
        JobKind::Process => "process",
        JobKind::Skill => "skill",
        JobKind::Mcp => "mcp",
    }
}

fn parse_state(value: &str) -> rusqlite::Result<JobState> {
    let state = match value {
        "queued" => JobState::Queued,
        "waiting_confirmation" => JobState::WaitingConfirmation,
        "starting" => JobState::Starting,
        "running" => JobState::Running,
        "completed" => JobState::Completed,
        "failed" => JobState::Failed,
        "rejected" => JobState::Rejected,
        "cancel_requested" => JobState::CancelRequested,
        "cancelled" => JobState::Cancelled,
        "timed_out" => JobState::TimedOut,
        "detached" => JobState::Detached,
        "unknown_after_restart" => JobState::UnknownAfterRestart,
        "skipped" => JobState::Skipped,
        _ => {
            return Err(rusqlite::Error::InvalidColumnType(
                7,
                "state".to_string(),
                rusqlite::types::Type::Text,
            ))
        }
    };
    Ok(state)
}

fn format_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn parse_time(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

fn encode_cursor(cursor: &Cursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).expect("cursor serialization cannot fail"))
}

pub(crate) fn encode_list_cursor(info: &JobInfo) -> String {
    encode_cursor(&Cursor {
        created_at: format_time(info.created_at),
        job_id: info.job_id.clone(),
    })
}

fn decode_cursor(value: &str) -> Result<Cursor> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| anyhow!("invalid_job_history_cursor"))?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid_job_history_cursor"))
}

pub(crate) fn decode_list_cursor(value: &str) -> Result<JobHistoryCursor> {
    let cursor = decode_cursor(value)?;
    let created_at = DateTime::parse_from_rfc3339(&cursor.created_at)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| anyhow!("invalid_job_history_cursor"))?;
    if cursor.job_id.is_empty() {
        return Err(anyhow!("invalid_job_history_cursor"));
    }
    Ok(JobHistoryCursor {
        created_at,
        job_id: cursor.job_id,
    })
}

fn is_corruption(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(code.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
    )
}

fn pending_retry_delay(attempts: u8) -> Duration {
    let exponent = attempts.saturating_sub(1).min(5) as u32;
    Duration::seconds(PENDING_RETRY_BASE_SECONDS.saturating_mul(1_i64 << exponent))
}

fn truncate_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn bounded_info(info: &JobInfo) -> JobInfo {
    let mut bounded = info.clone();
    bounded.agent_id = truncate_text(&bounded.agent_id, MAX_STRING_BYTES);
    bounded.job_id = truncate_text(&bounded.job_id, MAX_STRING_BYTES);
    bounded.group = bounded.group.map(|value| truncate_text(&value, 256));
    bounded.batch_id = bounded
        .batch_id
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.batch_call_id = bounded
        .batch_call_id
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.program = bounded
        .program
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.args = bounded
        .args
        .into_iter()
        .take(MAX_ARGS)
        .map(|value| truncate_text(&value, MAX_ARG_BYTES))
        .collect();
    bounded.working_directory = bounded
        .working_directory
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.command_preview = bounded
        .command_preview
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.stdout_tail = truncate_text(&bounded.stdout_tail, MAX_STRING_BYTES);
    bounded.stderr_tail = truncate_text(&bounded.stderr_tail, MAX_STRING_BYTES);
    bounded.reject_reason = bounded
        .reject_reason
        .map(|value| truncate_text(&value, MAX_ERROR_BYTES));
    bounded.skill_id = bounded
        .skill_id
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.skill_path = bounded
        .skill_path
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.installed_digest = bounded
        .installed_digest
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.mcp_server_id = bounded
        .mcp_server_id
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.mcp_tool_name = bounded
        .mcp_tool_name
        .map(|value| truncate_text(&value, MAX_STRING_BYTES));
    bounded.cancel_outcome = bounded
        .cancel_outcome
        .map(|value| truncate_text(&value, MAX_ERROR_BYTES));
    bounded.termination_evidence = bounded
        .termination_evidence
        .map(|value| truncate_text(&value, MAX_ERROR_BYTES));
    bounded
}

fn bounded_detail(mut detail: JobDetail) -> JobDetail {
    detail.job = bounded_info(&detail.job);
    if let Some(error) = &mut detail.error {
        error.code = truncate_text(&error.code, 256);
        error.message = truncate_text(&error.message, MAX_ERROR_BYTES);
    }
    if let Some(result) = &detail.result {
        let serialized = serde_json::to_vec(result).unwrap_or_default();
        if serialized.len() > MAX_RESULT_BYTES {
            let hash = Sha256::digest(&serialized);
            detail.result_bytes = Some(serialized.len());
            detail.result_sha256 = Some(format!("{hash:x}"));
            detail.result_preview = Some(truncate_text(
                &String::from_utf8_lossy(&serialized),
                MAX_RESULT_PREVIEW_BYTES,
            ));
            detail.result = None;
            detail.result_truncated = true;
        }
    }
    if let Ok(serialized) = serde_json::to_vec(&detail) {
        if serialized.len() > MAX_RICH_DETAIL_BYTES {
            detail.result = None;
            detail.result_truncated = true;
            detail.job.args.truncate(16);
            detail.job.stdout_tail = truncate_text(&detail.job.stdout_tail, 8 * 1024);
            detail.job.stderr_tail = truncate_text(&detail.job.stderr_tail, 8 * 1024);
            detail.job.command_preview = detail
                .job
                .command_preview
                .map(|value| truncate_text(&value, 8 * 1024));
            if let Some(error) = &mut detail.error {
                error.message = truncate_text(&error.message, MAX_ERROR_BYTES);
            }
        }
    }
    if serde_json::to_vec(&detail)
        .map(|serialized| serialized.len() > MAX_RICH_DETAIL_BYTES)
        .unwrap_or(true)
    {
        detail.result = None;
        detail.error = None;
        detail.job.args.clear();
        detail.job.stdout_tail.clear();
        detail.job.stderr_tail.clear();
        detail.job.command_preview = None;
        detail.detail_available = false;
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::private_state::PrivateStatePaths;
    use serde_json::json;

    fn store(name: &str) -> (std::sync::Arc<JobHistoryStore>, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agentic-job-history-{name}-{}",
            Uuid::new_v4().simple()
        ));
        let store = JobHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        (store, root)
    }

    fn info(id: &str, state: JobState, created_at: DateTime<Utc>) -> JobInfo {
        JobInfo {
            agent_id: "agent-test".to_string(),
            job_id: id.to_string(),
            group: Some("group-a".to_string()),
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind: JobKind::Process,
            state,
            created_at,
            started_at: None,
            updated_at: created_at,
            finished_at: (state.is_terminal()).then_some(created_at),
            program: Some("echo".to_string()),
            args: vec!["hello".to_string()],
            working_directory: Some("/tmp".to_string()),
            command_preview: Some("echo hello".to_string()),
            exit_code: None,
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

    fn detail(info: JobInfo) -> JobDetail {
        JobDetail {
            job: info,
            detail_available: true,
            result: Some(json!({"ok": true})),
            error: None,
            result_truncated: false,
            result_bytes: None,
            result_sha256: None,
            result_preview: None,
        }
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn schema_is_idempotent_and_indexes_exist() {
        let (store, root) = store("schema");
        assert_eq!(store.health().status, HistoryHealthStatus::Healthy);
        let second = JobHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        let indexes: Vec<String> = second
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_jobs_%'",
                )?;
                let rows = statement.query_map([], |row| row.get(0))?;
                rows.collect()
            })
            .unwrap();
        assert_eq!(indexes.len(), 5);
        cleanup(&root);
    }

    #[test]
    fn admission_and_terminal_upsert_are_idempotent() {
        let (store, root) = store("upsert");
        let now = Utc::now();
        let admitted = info("job-1", JobState::Running, now);
        assert_eq!(
            store.insert_admission(&admitted),
            HistoryWriteOutcome::Persisted
        );
        assert_eq!(
            store.insert_admission(&admitted),
            HistoryWriteOutcome::Persisted
        );
        let mut terminal = info("job-1", JobState::Completed, now);
        terminal.finished_at = Some(now + Duration::seconds(1));
        terminal.updated_at = now + Duration::seconds(1);
        let terminal = detail(terminal);
        assert_eq!(
            store.upsert_terminal(&terminal),
            HistoryWriteOutcome::Persisted
        );
        let found = store.get("job-1").unwrap().unwrap();
        assert_eq!(found.info.state, JobState::Completed);
        assert_eq!(found.detail.unwrap().result, Some(json!({"ok": true})));
        cleanup(&root);
    }

    #[test]
    fn list_orders_and_filters_with_cursor() {
        let (store, root) = store("list");
        let base = Utc::now();
        for (id, offset, group, kind) in [
            ("a", 1, Some("one"), JobKind::Process),
            ("b", 2, Some("two"), JobKind::Skill),
            ("c", 3, Some("one"), JobKind::Process),
        ] {
            let mut value = info(id, JobState::Completed, base + Duration::seconds(offset));
            value.group = group.map(str::to_string);
            value.kind = kind;
            assert_eq!(
                store.upsert_terminal(&detail(value)),
                HistoryWriteOutcome::Persisted
            );
        }
        let page = store
            .list(&JobListRequest {
                group: Some("one".to_string()),
                kind: Some(JobKind::Process),
                state: Some(JobState::Completed),
                limit: Some(1),
                cursor: None,
            })
            .unwrap();
        assert_eq!(page.jobs.len(), 1);
        assert_eq!(page.jobs[0].job_id, "c");
        let next = page.next_cursor.unwrap();
        let page = store
            .list(&JobListRequest {
                group: Some("one".to_string()),
                kind: Some(JobKind::Process),
                state: Some(JobState::Completed),
                limit: Some(1),
                cursor: Some(next),
            })
            .unwrap();
        assert_eq!(page.jobs[0].job_id, "a");
        cleanup(&root);
    }

    #[test]
    fn recovery_only_changes_persisted_active_rows() {
        let (store, root) = store("recovery");
        let now = Utc::now();
        let mut active = info("active", JobState::Running, now);
        active.started_at = Some(now - Duration::seconds(10));
        assert_eq!(
            store.insert_admission(&active),
            HistoryWriteOutcome::Persisted
        );
        let terminal = info("terminal", JobState::Completed, now);
        assert_eq!(
            store.upsert_terminal(&detail(terminal)),
            HistoryWriteOutcome::Persisted
        );
        drop(store);
        let store = JobHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        let recovered = store.get("active").unwrap().unwrap().info;
        assert_eq!(recovered.state, JobState::UnknownAfterRestart);
        assert_eq!(recovered.group, active.group);
        assert_eq!(recovered.started_at, active.started_at);
        assert!(recovered.finished_at.is_some());
        assert_eq!(
            store.get("terminal").unwrap().unwrap().info.state,
            JobState::Completed
        );
        cleanup(&root);
    }

    #[test]
    fn cleanup_prunes_old_terminals_but_preserves_active_rows() {
        let (store, root) = store("retention");
        let now = Utc::now();
        let old = info("old", JobState::Completed, now - Duration::days(31));
        let active = info("active", JobState::Running, now - Duration::days(31));
        assert_eq!(
            store.upsert_terminal(&detail(old)),
            HistoryWriteOutcome::Persisted
        );
        assert_eq!(
            store.insert_admission(&active),
            HistoryWriteOutcome::Persisted
        );
        store.cleanup(now).unwrap();
        assert!(store.get("old").unwrap().is_none());
        assert!(store.get("active").unwrap().is_some());
        cleanup(&root);
    }

    #[test]
    fn oversized_result_is_bounded() {
        let (store, root) = store("bounds");
        let value = info("large", JobState::Completed, Utc::now());
        let mut snapshot = detail(value);
        snapshot.result = Some(json!("x".repeat(MAX_RESULT_BYTES + 10)));
        assert_eq!(
            store.upsert_terminal(&snapshot),
            HistoryWriteOutcome::Persisted
        );
        let found = store.get("large").unwrap().unwrap().detail.unwrap();
        assert!(found.result.is_none());
        assert!(found.result_truncated);
        assert!(store.path().exists());
        cleanup(&root);
    }

    #[test]
    fn degraded_storage_is_fail_open_and_terminal_queue_is_bounded() {
        let root = std::env::temp_dir().join(format!(
            "agentic-job-history-failure-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        let store = JobHistoryStore::disabled(root.clone());
        for index in 0..=MAX_PENDING_TERMINALS {
            let snapshot = detail(info(
                &format!("degraded-{index}"),
                JobState::Completed,
                Utc::now(),
            ));
            assert_eq!(
                store.upsert_terminal(&snapshot),
                HistoryWriteOutcome::Deferred
            );
        }
        let health = store.health();
        assert_eq!(health.status, HistoryHealthStatus::Degraded);
        assert_eq!(health.pending_terminal_count, MAX_PENDING_TERMINALS);
        assert_eq!(health.dropped_terminal_count, 1);
        assert!(health.last_error.is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn pending_terminal_retries_use_backoff_and_have_a_finite_budget() {
        let (store, root) = store("retry-budget");
        store
            .with_connection(|connection| connection.execute_batch("PRAGMA query_only = ON"))
            .unwrap();
        let snapshot = detail(info("retry-me", JobState::Completed, Utc::now()));
        assert_eq!(
            store.upsert_terminal(&snapshot),
            HistoryWriteOutcome::Deferred
        );
        let first_retry_at = {
            let pending = store.pending_terminals.lock().unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].attempts, 1);
            pending[0].next_retry_at
        };
        assert_eq!(
            store.retry_pending_at(first_retry_at - Duration::milliseconds(1)),
            0
        );
        assert_eq!(store.pending_terminals.lock().unwrap()[0].attempts, 1);

        for attempt in 2..=MAX_PENDING_RETRY_ATTEMPTS {
            assert_eq!(
                store.retry_pending_at(first_retry_at + Duration::hours(attempt as i64)),
                0
            );
            let pending = store.pending_terminals.lock().unwrap();
            if attempt < MAX_PENDING_RETRY_ATTEMPTS {
                assert_eq!(pending.len(), 1);
                assert_eq!(pending[0].attempts, attempt);
            } else {
                assert!(pending.is_empty());
            }
        }
        let health = store.health();
        assert_eq!(health.status, HistoryHealthStatus::Degraded);
        assert_eq!(health.dropped_terminal_count, 1);
        cleanup(&root);
    }

    #[test]
    fn successful_writes_do_not_hide_pending_terminal_degradation() {
        let (store, root) = store("pending-health");
        store
            .with_connection(|connection| connection.execute_batch("PRAGMA query_only = ON"))
            .unwrap();
        let snapshot = detail(info("pending", JobState::Completed, Utc::now()));
        assert_eq!(
            store.upsert_terminal(&snapshot),
            HistoryWriteOutcome::Deferred
        );
        let retry_at = store.pending_terminals.lock().unwrap()[0].next_retry_at;
        store
            .with_connection(|connection| connection.execute_batch("PRAGMA query_only = OFF"))
            .unwrap();

        let admission = info("new-admission", JobState::Running, Utc::now());
        assert_eq!(
            store.insert_admission(&admission),
            HistoryWriteOutcome::Persisted
        );
        assert_eq!(store.health().status, HistoryHealthStatus::Degraded);
        assert_eq!(store.health().pending_terminal_count, 1);

        assert_eq!(store.retry_pending_at(retry_at + Duration::seconds(1)), 1);
        assert_eq!(store.health().status, HistoryHealthStatus::Healthy);
        assert_eq!(store.health().pending_terminal_count, 0);
        cleanup(&root);
    }

    #[test]
    fn logical_cap_prunes_oldest_terminals_only() {
        let (store, root) = store("cap");
        let now = Utc::now();
        let first = info("first", JobState::Completed, now - Duration::seconds(3));
        let second = info("second", JobState::Completed, now - Duration::seconds(2));
        let active = info("active", JobState::Running, now - Duration::seconds(1));
        assert_eq!(
            store.upsert_terminal(&detail(first)),
            HistoryWriteOutcome::Persisted
        );
        assert_eq!(
            store.upsert_terminal(&detail(second)),
            HistoryWriteOutcome::Persisted
        );
        assert_eq!(
            store.insert_admission(&active),
            HistoryWriteOutcome::Persisted
        );
        store
            .cleanup_with_limits(now, Duration::days(30), 0)
            .unwrap();
        assert!(store.get("first").unwrap().is_none());
        assert!(store.get("second").unwrap().is_none());
        assert!(store.get("active").unwrap().is_some());
        cleanup(&root);
    }

    #[test]
    fn diagnosed_corruption_isolated_without_touching_other_state() {
        let root = std::env::temp_dir().join(format!(
            "agentic-job-history-corrupt-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("jobs.sqlite3"), b"not sqlite").unwrap();
        let store = JobHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        assert_eq!(store.health().status, HistoryHealthStatus::Healthy);
        assert!(store.path().exists());
        let isolated = fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("jobs.sqlite3.corrupt-")
            });
        assert!(isolated);
        cleanup(&root);
    }
}
