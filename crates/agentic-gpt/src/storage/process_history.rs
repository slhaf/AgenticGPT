#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agentic_gpt_protocol::{
    ProcessCaptureStatus, ProcessDetail, ProcessError, ProcessInfo, ProcessKind,
    ProcessListRequest, ProcessState,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, types::Value, Connection, OptionalExtension, ToSql, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::private_state::PrivateStatePaths;
use crate::utils::log_warn;

const HISTORY_RETENTION: Duration = Duration::days(30);
const HISTORY_CAP_BYTES: u64 = 512 * 1024 * 1024;
const CLEANUP_INTERVAL: Duration = Duration::hours(1);
const HISTORY_SCHEMA_VERSION: i64 = 1;
const SQLITE_BUSY_TIMEOUT_MS: u64 = 750;
const MAX_INFO_JSON_BYTES: usize = 256 * 1024;
const MAX_DETAIL_JSON_BYTES: usize = 1024 * 1024;
const MAX_ERROR_BYTES: usize = 8 * 1024;
// Two bounded terminal error strings may each expand sixfold when JSON-escaped.
const TERMINAL_INFO_RESERVE_BYTES: usize = MAX_ERROR_BYTES * 2 * 6 + 1024;
const MAX_ADMISSION_INFO_JSON_BYTES: usize = MAX_INFO_JSON_BYTES - TERMINAL_INFO_RESERVE_BYTES;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS processes (
    process_id TEXT PRIMARY KEY NOT NULL,
    agent_id TEXT NOT NULL,
    group_name TEXT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    started_at TEXT,
    updated_at TEXT NOT NULL,
    finished_at TEXT,
    info_json TEXT NOT NULL,
    detail_json TEXT,
    stdout_bytes BLOB NOT NULL DEFAULT X'',
    stdout_start_offset TEXT NOT NULL DEFAULT '0',
    stdout_end_offset TEXT NOT NULL DEFAULT '0',
    stderr_bytes BLOB NOT NULL DEFAULT X'',
    stderr_start_offset TEXT NOT NULL DEFAULT '0',
    stderr_end_offset TEXT NOT NULL DEFAULT '0',
    capture_status TEXT NOT NULL,
    capture_error TEXT
);
CREATE INDEX IF NOT EXISTS idx_processes_created_id ON processes(created_at DESC, process_id DESC);
CREATE INDEX IF NOT EXISTS idx_processes_group_created ON processes(group_name, created_at DESC, process_id DESC);
CREATE INDEX IF NOT EXISTS idx_processes_kind_created ON processes(kind, created_at DESC, process_id DESC);
CREATE INDEX IF NOT EXISTS idx_processes_state_created ON processes(state, created_at DESC, process_id DESC);
CREATE INDEX IF NOT EXISTS idx_processes_finished ON processes(finished_at);
CREATE TABLE IF NOT EXISTS process_event_completions (
    process_id TEXT PRIMARY KEY NOT NULL,
    event_type TEXT,
    message TEXT,
    completed_at TEXT
);
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
    Deferred(String),
    Failed(String),
}

impl HistoryWriteOutcome {
    pub(crate) fn is_persisted(&self) -> bool {
        matches!(self, Self::Persisted)
    }

    pub(crate) fn error(&self) -> Option<&str> {
        match self {
            Self::Persisted => None,
            Self::Deferred(error) | Self::Failed(error) => Some(error),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProcessOutputSnapshot {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stdout_start_offset: u64,
    pub(crate) stdout_end_offset: u64,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stderr_start_offset: u64,
    pub(crate) stderr_end_offset: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProcessHistoryReadSummary {
    pub(crate) kind: ProcessKind,
    pub(crate) state: ProcessState,
    pub(crate) capture_status: ProcessCaptureStatus,
    pub(crate) stdout_start_offset: u64,
    pub(crate) stdout_end_offset: u64,
    pub(crate) stderr_start_offset: u64,
    pub(crate) stderr_end_offset: u64,
}

pub(crate) struct ProcessHistoryStatusRecord {
    pub(crate) info: ProcessInfo,
    pub(crate) error: Option<ProcessError>,
    pub(crate) summary: ProcessHistoryReadSummary,
}

#[derive(Clone, Debug)]
pub(crate) struct ProcessHistoryRecord {
    pub(crate) info: ProcessInfo,
    pub(crate) detail: Option<ProcessDetail>,
    pub(crate) output: ProcessOutputSnapshot,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingProcessEventCompletion {
    pub(crate) process_id: String,
    pub(crate) event_type: String,
    pub(crate) message: String,
    pub(crate) completed_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProcessHistoryPage {
    pub(crate) processes: Vec<ProcessInfo>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListCursor {
    version: u8,
    created_at: String,
    process_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessHistoryCursor {
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) process_id: String,
}

#[derive(Debug)]
struct HealthState {
    status: HistoryHealthStatus,
    last_error: Option<String>,
    dropped_terminal_count: usize,
}

pub(crate) struct ProcessHistoryStore {
    path: PathBuf,
    disabled: bool,
    connection: Mutex<Option<Connection>>,
    health: Mutex<HealthState>,
    last_cleanup: Mutex<Option<DateTime<Utc>>>,
}

impl ProcessHistoryStore {
    pub(crate) fn open(paths: &PrivateStatePaths) -> std::sync::Arc<Self> {
        let store = std::sync::Arc::new(Self {
            path: paths.root.join("process.sqlite3"),
            disabled: false,
            connection: Mutex::new(None),
            health: Mutex::new(HealthState {
                status: HistoryHealthStatus::Degraded,
                last_error: None,
                dropped_terminal_count: 0,
            }),
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
            last_cleanup: Mutex::new(None),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn health(&self) -> HistoryHealth {
        let health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        HistoryHealth {
            status: health.status.clone(),
            path: self.path.clone(),
            pending_terminal_count: 0,
            dropped_terminal_count: health.dropped_terminal_count,
            last_error: health.last_error.clone(),
        }
    }

    pub(crate) fn insert_admissions<'a>(
        &self,
        infos: impl IntoIterator<Item = &'a ProcessInfo>,
    ) -> HistoryWriteOutcome {
        self.insert_admissions_inner(infos, false)
    }

    pub(crate) fn insert_admissions_with_event_tracking<'a>(
        &self,
        infos: impl IntoIterator<Item = &'a ProcessInfo>,
    ) -> HistoryWriteOutcome {
        self.insert_admissions_inner(infos, true)
    }

    fn insert_admissions_inner<'a>(
        &self,
        infos: impl IntoIterator<Item = &'a ProcessInfo>,
        track_event_completions: bool,
    ) -> HistoryWriteOutcome {
        let mut admissions = Vec::new();
        for info in infos {
            let info = bounded_info(info);
            let info_json = match serde_json::to_string(&info) {
                Ok(value) => value,
                Err(error) => return self.failed(anyhow!(error)),
            };
            if info_json.len() > MAX_ADMISSION_INFO_JSON_BYTES {
                return HistoryWriteOutcome::Failed(
                    "process_history_admission_metadata_too_large".to_string(),
                );
            }
            admissions.push((info, info_json));
        }
        if let Err(error) = self.ensure_ready() {
            return self.failed(error);
        }
        let result = (|| -> rusqlite::Result<()> {
            let mut guard = self
                .connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let connection = guard.as_mut().ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("process_history_unavailable".to_string())
            })?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            {
                let mut statement = transaction.prepare_cached(
                    "INSERT OR IGNORE INTO processes (
                        process_id, agent_id, group_name, kind, state, created_at,
                        started_at, updated_at, finished_at, info_json, detail_json,
                        stdout_bytes, stdout_start_offset, stdout_end_offset,
                        stderr_bytes, stderr_start_offset, stderr_end_offset,
                        capture_status, capture_error
                    ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,X'', '0','0',X'', '0','0',?11,?12)",
                )?;
                for (info, info_json) in &admissions {
                    statement.execute(params![
                        info.process_id,
                        info.agent_id,
                        info.group,
                        kind_label(info.kind),
                        info.state.label(),
                        format_time(info.created_at),
                        info.started_at.map(format_time),
                        format_time(info.updated_at),
                        info.finished_at.map(format_time),
                        info_json,
                        capture_label(info.capture_status),
                        info.capture_error,
                    ])?;
                }
                if track_event_completions {
                    for (info, _) in &admissions {
                        transaction.execute(
                            "INSERT OR IGNORE INTO process_event_completions(process_id) VALUES (?1)",
                            params![info.process_id],
                        )?;
                    }
                }
            }
            transaction.commit()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.healthy();
                HistoryWriteOutcome::Persisted
            }
            Err(error) => self.failed(error),
        }
    }

    pub(crate) fn mark_started(&self, info: &ProcessInfo) -> HistoryWriteOutcome {
        let info = bounded_info(info);
        let info_json = match serde_json::to_string(&info) {
            Ok(value) => value,
            Err(error) => return self.failed(anyhow!(error)),
        };
        if info_json.len() > MAX_ADMISSION_INFO_JSON_BYTES {
            return HistoryWriteOutcome::Failed(
                "process_history_admission_metadata_too_large".to_string(),
            );
        }
        if let Err(error) = self.ensure_ready() {
            return self.failed(error);
        }
        let result = self.with_connection(|connection| {
            let changed = connection.execute(
                "UPDATE processes SET state=?1, started_at=?2, updated_at=?3, info_json=?4, capture_status=?5, capture_error=?6 WHERE process_id=?7 AND state IN ('queued','waiting_confirmation','starting','running','cancel_requested')",
                params![
                    info.state.label(),
                    info.started_at.map(format_time),
                    format_time(info.updated_at),
                    info_json,
                    capture_label(info.capture_status),
                    info.capture_error,
                    info.process_id,
                ],
            )?;
            if changed != 1 {
                return Err(rusqlite::Error::InvalidParameterName("history_admission_missing".to_string()));
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                self.healthy();
                HistoryWriteOutcome::Persisted
            }
            Err(error) => self.failed(error),
        }
    }

    pub(crate) fn upsert_terminal(
        &self,
        detail: &ProcessDetail,
        output: &ProcessOutputSnapshot,
    ) -> HistoryWriteOutcome {
        let detail = bounded_detail(detail.clone());
        let info = &detail.process;
        let info_json = match serde_json::to_string(info) {
            Ok(value) => value,
            Err(error) => return self.failed(anyhow!(error)),
        };
        let detail_json = match serde_json::to_string(&detail) {
            Ok(value) => value,
            Err(error) => return self.failed(anyhow!(error)),
        };
        if info_json.len() > MAX_INFO_JSON_BYTES || detail_json.len() > MAX_DETAIL_JSON_BYTES {
            return self.failed(anyhow!("process_history_snapshot_too_large"));
        }
        if let Err(error) = self.ensure_ready() {
            return self.failed(error);
        }
        let result = (|| -> rusqlite::Result<()> {
            let mut guard = self
                .connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let connection = guard.as_mut().ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("process_history_unavailable".to_string())
            })?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let changed = transaction.execute(
                "INSERT INTO processes (
                    process_id, agent_id, group_name, kind, state, created_at,
                    started_at, updated_at, finished_at, info_json, detail_json,
                    stdout_bytes, stdout_start_offset, stdout_end_offset,
                    stderr_bytes, stderr_start_offset, stderr_end_offset,
                    capture_status, capture_error
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
                ON CONFLICT(process_id) DO UPDATE SET
                    agent_id=excluded.agent_id, group_name=excluded.group_name,
                    kind=excluded.kind, state=excluded.state, created_at=excluded.created_at,
                    started_at=excluded.started_at, updated_at=excluded.updated_at,
                    finished_at=excluded.finished_at, info_json=excluded.info_json,
                    detail_json=excluded.detail_json, stdout_bytes=excluded.stdout_bytes,
                    stdout_start_offset=excluded.stdout_start_offset, stdout_end_offset=excluded.stdout_end_offset,
                    stderr_bytes=excluded.stderr_bytes, stderr_start_offset=excluded.stderr_start_offset,
                    stderr_end_offset=excluded.stderr_end_offset, capture_status=excluded.capture_status,
                    capture_error=excluded.capture_error
                WHERE processes.state IN ('queued','waiting_confirmation','starting','running','cancel_requested')",
                params![
                    info.process_id,
                    info.agent_id,
                    info.group,
                    kind_label(info.kind),
                    info.state.label(),
                    format_time(info.created_at),
                    info.started_at.map(format_time),
                    format_time(info.updated_at),
                    info.finished_at.map(format_time),
                    info_json,
                    detail_json,
                    output.stdout,
                    output.stdout_start_offset.to_string(),
                    output.stdout_end_offset.to_string(),
                    output.stderr,
                    output.stderr_start_offset.to_string(),
                    output.stderr_end_offset.to_string(),
                    capture_label(info.capture_status),
                    info.capture_error,
                ],
            )?;
            if changed == 0 {
                let existing_terminal = transaction
                    .query_row(
                        "SELECT state NOT IN ('queued','waiting_confirmation','starting','running','cancel_requested') FROM processes WHERE process_id=?1",
                        params![info.process_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .optional()?
                    .unwrap_or(false);
                if !existing_terminal {
                    return Err(rusqlite::Error::InvalidParameterName(
                        "terminal_snapshot_not_written".to_string(),
                    ));
                }
            }
            persist_process_event_completion(&transaction, info)?;
            transaction.commit()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.healthy();
                self.maybe_cleanup(Utc::now());
                HistoryWriteOutcome::Persisted
            }
            Err(error) => self.failed(error),
        }
    }

    pub(crate) fn record_terminal_event(&self, process: &ProcessInfo) -> HistoryWriteOutcome {
        if process.state.is_active() {
            return HistoryWriteOutcome::Failed(
                "process_event_requires_terminal_state".to_string(),
            );
        }
        if let Err(error) = self.ensure_ready() {
            return self.failed(error);
        }
        let result = (|| -> rusqlite::Result<()> {
            let mut guard = self
                .connection
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let connection = guard.as_mut().ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("process_history_unavailable".to_string())
            })?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            persist_process_event_completion(&transaction, process)?;
            transaction.commit()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.healthy();
                HistoryWriteOutcome::Persisted
            }
            Err(error) => self.failed(error),
        }
    }

    pub(crate) fn retry_pending(&self) -> usize {
        0
    }

    pub(crate) fn terminal_pending(&self, _process_id: &str) -> bool {
        false
    }

    pub(crate) fn terminal_snapshot_matches(
        &self,
        process_id: &str,
        updated_at: DateTime<Utc>,
    ) -> bool {
        if self.ensure_ready().is_err() {
            return false;
        }
        matches!(
            self.with_connection(|connection| {
                connection.query_row(
                    "SELECT 1 FROM processes WHERE process_id=?1 AND updated_at=?2 AND finished_at IS NOT NULL",
                    params![process_id, format_time(updated_at)],
                    |_| Ok(()),
                )
            }),
            Ok(())
        )
    }

    pub(crate) fn pending_event_completions(&self) -> Result<Vec<PendingProcessEventCompletion>> {
        self.ensure_ready()?;
        let rows = self
            .with_connection(|connection| {
                let mut statement = connection.prepare(
                    "SELECT process_id,event_type,message,completed_at
                     FROM process_event_completions
                     WHERE event_type IS NOT NULL AND message IS NOT NULL AND completed_at IS NOT NULL
                     ORDER BY completed_at,process_id",
                )?;
                let mapped = statement.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?;
                mapped.collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|error| anyhow!(error))?;
        rows.into_iter()
            .map(|(process_id, event_type, message, completed_at)| {
                Ok(PendingProcessEventCompletion {
                    process_id,
                    event_type,
                    message,
                    completed_at: DateTime::parse_from_rfc3339(&completed_at)
                        .map_err(|_| anyhow!("invalid_process_event_completion_timestamp"))?
                        .with_timezone(&Utc),
                })
            })
            .collect()
    }

    pub(crate) fn acknowledge_event_completion(&self, process_id: &str) -> Result<()> {
        self.ensure_ready()?;
        self.with_connection(|connection| {
            connection.execute(
                "DELETE FROM process_event_completions WHERE process_id=?1",
                params![process_id],
            )?;
            Ok(())
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn get(&self, process_id: &str) -> Result<Option<ProcessHistoryRecord>> {
        self.ensure_ready()?;
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT info_json, detail_json, stdout_bytes, stdout_start_offset,
                        stdout_end_offset, stderr_bytes, stderr_start_offset,
                        stderr_end_offset FROM processes WHERE process_id=?1",
                    params![process_id],
                    row_to_record,
                )
                .optional()
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn read_summary(
        &self,
        process_id: &str,
    ) -> Result<Option<ProcessHistoryReadSummary>> {
        self.ensure_ready()?;
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT kind, state, capture_status, stdout_start_offset,
                        stdout_end_offset, stderr_start_offset, stderr_end_offset
                        FROM processes WHERE process_id=?1",
                    params![process_id],
                    row_to_read_summary,
                )
                .optional()
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn status_record(
        &self,
        process_id: &str,
    ) -> Result<Option<ProcessHistoryStatusRecord>> {
        self.ensure_ready()?;
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT info_json, detail_json, kind, state, capture_status,
                        stdout_start_offset, stdout_end_offset, stderr_start_offset,
                        stderr_end_offset FROM processes WHERE process_id=?1",
                    params![process_id],
                    row_to_status_record,
                )
                .optional()
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn list(&self, request: &ProcessListRequest) -> Result<ProcessHistoryPage> {
        self.ensure_ready()?;
        let limit = request.effective_limit();
        let mut sql = String::from("SELECT info_json FROM processes WHERE 1=1");
        let mut values = Vec::<Value>::new();
        if let Some(group) = &request.group {
            sql.push_str(" AND group_name=?");
            values.push(Value::Text(group.clone()));
        }
        if let Some(kind) = request.kind {
            sql.push_str(" AND kind=?");
            values.push(Value::Text(kind_label(kind).to_string()));
        }
        if let Some(state) = request.state {
            sql.push_str(" AND state=?");
            values.push(Value::Text(state.label().to_string()));
        }
        if let Some(value) = request.cursor.as_deref() {
            let cursor = decode_list_cursor(value)?;
            let created_at = format_time(cursor.created_at);
            sql.push_str(" AND (created_at<? OR (created_at=? AND process_id<?))");
            values.push(Value::Text(created_at.clone()));
            values.push(Value::Text(created_at));
            values.push(Value::Text(cursor.process_id));
        }
        sql.push_str(" ORDER BY created_at DESC, process_id DESC LIMIT ?");
        values.push(Value::Integer((limit.saturating_add(1)) as i64));
        self.with_connection(|connection| {
            let params = values
                .iter()
                .map(|value| value as &dyn ToSql)
                .collect::<Vec<_>>();
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(params.as_slice(), |row| {
                let info_json: String = row.get(0)?;
                serde_json::from_str(&info_json)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
            })?;
            let mut processes = Vec::new();
            for row in rows {
                processes.push(row?);
            }
            let next_cursor = if processes.len() > limit {
                processes.truncate(limit);
                processes.last().map(encode_list_cursor)
            } else {
                None
            };
            Ok(ProcessHistoryPage {
                processes,
                next_cursor,
            })
        })
        .map_err(|error| anyhow!(error))
    }

    pub(crate) fn recover_active(&self, now: DateTime<Utc>) -> Result<usize> {
        self.ensure_ready()?;
        self.with_connection(|connection| {
            let transaction = connection.unchecked_transaction()?;
            let mut statement = transaction.prepare(
                "SELECT info_json FROM processes WHERE state IN ('queued','waiting_confirmation','starting','running','cancel_requested')",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            let mut recovered = Vec::new();
            for row in rows {
                let json = row?;
                let mut info: ProcessInfo = serde_json::from_str(&json).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                info.state = ProcessState::UnknownAfterRestart;
                info.updated_at = now;
                info.finished_at = Some(now);
                let capture_lost = info.kind != ProcessKind::Mcp;
                info.capture_status = if capture_lost {
                    ProcessCaptureStatus::Incomplete
                } else {
                    ProcessCaptureStatus::NotApplicable
                };
                info.capture_error = capture_lost
                    .then(|| "agent_restarted_before_output_capture_completed".to_string());
                info.termination_evidence = Some("agent_restart".to_string());
                let info_json = serde_json::to_string(&info).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                let detail = ProcessDetail {
                    process: info.clone(),
                    detail_available: false,
                    result: None,
                    error: None,
                    result_available: false,
                    result_bytes: None,
                    result_sha256: None,
                    result_preview: None,
                };
                let detail_json = serde_json::to_string(&detail).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
                recovered.push((info, info_json, detail_json));
            }
            drop(statement);
            for (info, info_json, detail_json) in &recovered {
                transaction.execute(
                    "UPDATE processes SET state=?1, updated_at=?2, finished_at=?2, info_json=?3, detail_json=?4, capture_status='incomplete', capture_error=?5 WHERE process_id=?6",
                    params![info.state.label(), format_time(now), info_json, detail_json, info.capture_error, info.process_id],
                )?;
                persist_process_event_completion(&transaction, info)?;
            }
            transaction.commit()?;
            Ok(recovered.len())
        }).map_err(|error| anyhow!(error))
    }

    fn initialize(&self) {
        if self.disabled {
            return;
        }
        match self.open_initialized_connection() {
            Ok(connection) => {
                *self
                    .connection
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection);
                self.healthy();
            }
            Err(error) => {
                self.degrade(&error.to_string());
                log_warn(format!("process history store unavailable at {}; process creation will fail closed: {error}", self.path.display()));
            }
        }
    }

    fn ensure_ready(&self) -> Result<()> {
        if self.disabled {
            return Err(anyhow!("history_disabled"));
        }
        let ready = self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some();
        if ready {
            return Ok(());
        }
        let connection = self.open_initialized_connection()?;
        *self
            .connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(connection);
        self.healthy();
        Ok(())
    }

    fn open_initialized_connection(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !self.path.exists() {
            let mut options = OpenOptions::new();
            options.create_new(true).read(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&self.path) {
                Ok(file) => drop(file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut connection = Connection::open(&self.path)?;
        connection.busy_timeout(std::time::Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > HISTORY_SCHEMA_VERSION {
            return Err(anyhow!("unsupported process history schema version {version}; maximum supported {HISTORY_SCHEMA_VERSION}"));
        }
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(SCHEMA)?;
        transaction.pragma_update(None, "user_version", HISTORY_SCHEMA_VERSION)?;
        transaction.commit()?;
        Ok(connection)
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
            rusqlite::Error::InvalidParameterName("process_history_unavailable".to_string())
        })?;
        operation(connection)
    }

    fn maybe_cleanup(&self, now: DateTime<Utc>) {
        let mut last_cleanup = self
            .last_cleanup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last_cleanup.is_some_and(|last| now - last < CLEANUP_INTERVAL) {
            return;
        }
        *last_cleanup = Some(now);
        let result = self.with_connection(|connection| {
            connection.execute("DELETE FROM processes WHERE finished_at IS NOT NULL AND finished_at<?", params![format_time(now - HISTORY_RETENTION)])?;
            let page = connection.query_row("SELECT COALESCE(SUM(length(info_json)+length(COALESCE(detail_json,''))+length(stdout_bytes)+length(stderr_bytes)),0) FROM processes", [], |row| row.get::<_, i64>(0))?;
            if page > HISTORY_CAP_BYTES as i64 {
                connection.execute("DELETE FROM processes WHERE process_id IN (SELECT process_id FROM processes WHERE finished_at IS NOT NULL ORDER BY finished_at ASC LIMIT (SELECT max(count(*)/10,1) FROM processes))", [])?;
            }
            Ok(())
        });
        if let Err(error) = result {
            self.degrade(&error.to_string());
        }
    }

    fn healthy(&self) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.status = HistoryHealthStatus::Healthy;
        health.last_error = None;
    }

    fn degrade(&self, error: &str) {
        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        health.status = HistoryHealthStatus::Degraded;
        health.last_error = Some(truncate_text(error, MAX_ERROR_BYTES));
    }

    fn failed(&self, error: impl std::fmt::Display) -> HistoryWriteOutcome {
        let message = truncate_text(&error.to_string(), MAX_ERROR_BYTES);
        self.degrade(&message);
        HistoryWriteOutcome::Failed(message)
    }
}

fn persist_process_event_completion(
    transaction: &rusqlite::Transaction<'_>,
    process: &ProcessInfo,
) -> rusqlite::Result<()> {
    let Some((event_type, message, completed_at)) =
        crate::event_notifications::process_completion_details(process)
    else {
        return Ok(());
    };
    transaction.execute(
        "UPDATE process_event_completions
         SET event_type=?1,message=?2,completed_at=?3
         WHERE process_id=?4 AND event_type IS NULL",
        params![
            event_type,
            message,
            format_time(completed_at),
            process.process_id
        ],
    )?;
    Ok(())
}

#[derive(Deserialize)]
struct ProcessDetailErrorProjection {
    error: Option<ProcessError>,
}

fn row_to_read_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessHistoryReadSummary> {
    row_to_read_summary_at(row, 0)
}

fn row_to_read_summary_at(
    row: &rusqlite::Row<'_>,
    start_column: usize,
) -> rusqlite::Result<ProcessHistoryReadSummary> {
    let kind_label: String = row.get(start_column)?;
    let state_label: String = row.get(start_column + 1)?;
    let capture_label: String = row.get(start_column + 2)?;
    let stdout_start: String = row.get(start_column + 3)?;
    let stdout_end: String = row.get(start_column + 4)?;
    let stderr_start: String = row.get(start_column + 5)?;
    let stderr_end: String = row.get(start_column + 6)?;
    let kind = match kind_label.as_str() {
        "command" => ProcessKind::Command,
        "skill" => ProcessKind::Skill,
        "mcp" => ProcessKind::Mcp,
        _ => return Err(invalid_summary_value(start_column, "kind")),
    };
    let state = match state_label.as_str() {
        "queued" => ProcessState::Queued,
        "waiting_confirmation" => ProcessState::WaitingConfirmation,
        "starting" => ProcessState::Starting,
        "running" => ProcessState::Running,
        "completed" => ProcessState::Completed,
        "failed" => ProcessState::Failed,
        "rejected" => ProcessState::Rejected,
        "cancel_requested" => ProcessState::CancelRequested,
        "cancelled" => ProcessState::Cancelled,
        "timed_out" => ProcessState::TimedOut,
        "detached" => ProcessState::Detached,
        "unknown_after_restart" => ProcessState::UnknownAfterRestart,
        "skipped" => ProcessState::Skipped,
        _ => return Err(invalid_summary_value(start_column + 1, "state")),
    };
    let capture_status = match capture_label.as_str() {
        "not_started" => ProcessCaptureStatus::NotStarted,
        "capturing" => ProcessCaptureStatus::Capturing,
        "complete" => ProcessCaptureStatus::Complete,
        "incomplete" => ProcessCaptureStatus::Incomplete,
        "not_applicable" => ProcessCaptureStatus::NotApplicable,
        _ => return Err(invalid_summary_value(start_column + 2, "capture_status")),
    };
    Ok(ProcessHistoryReadSummary {
        kind,
        state,
        capture_status,
        stdout_start_offset: parse_offset(&stdout_start)?,
        stdout_end_offset: parse_offset(&stdout_end)?,
        stderr_start_offset: parse_offset(&stderr_start)?,
        stderr_end_offset: parse_offset(&stderr_end)?,
    })
}

fn row_to_status_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessHistoryStatusRecord> {
    let info_json: String = row.get(0)?;
    let info: ProcessInfo = serde_json::from_str(&info_json)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let error = match row.get_ref(1)? {
        rusqlite::types::ValueRef::Null => None,
        rusqlite::types::ValueRef::Text(bytes) => {
            serde_json::from_slice::<ProcessDetailErrorProjection>(bytes)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .error
        }
        _ => return Err(invalid_summary_value(1, "detail")),
    };
    let summary = row_to_read_summary_at(row, 2)?;
    Ok(ProcessHistoryStatusRecord {
        info,
        error,
        summary,
    })
}

fn invalid_summary_value(column: usize, name: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid process history {name}"),
        )),
    )
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessHistoryRecord> {
    let info_json: String = row.get(0)?;
    let detail_json: Option<String> = row.get(1)?;
    let stdout: Vec<u8> = row.get(2)?;
    let stdout_start_offset: String = row.get(3)?;
    let stdout_end_offset: String = row.get(4)?;
    let stderr: Vec<u8> = row.get(5)?;
    let stderr_start_offset: String = row.get(6)?;
    let stderr_end_offset: String = row.get(7)?;
    let info: ProcessInfo = serde_json::from_str(&info_json)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let detail = detail_json
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let output = ProcessOutputSnapshot {
        stdout,
        stdout_start_offset: parse_offset(&stdout_start_offset)?,
        stdout_end_offset: parse_offset(&stdout_end_offset)?,
        stderr,
        stderr_start_offset: parse_offset(&stderr_start_offset)?,
        stderr_end_offset: parse_offset(&stderr_end_offset)?,
    };
    Ok(ProcessHistoryRecord {
        info,
        detail,
        output,
    })
}

fn parse_offset(value: &str) -> rusqlite::Result<u64> {
    value
        .parse::<u64>()
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

fn kind_label(kind: ProcessKind) -> &'static str {
    match kind {
        ProcessKind::Command => "command",
        ProcessKind::Skill => "skill",
        ProcessKind::Mcp => "mcp",
    }
}

fn capture_label(status: ProcessCaptureStatus) -> &'static str {
    match status {
        ProcessCaptureStatus::NotStarted => "not_started",
        ProcessCaptureStatus::Capturing => "capturing",
        ProcessCaptureStatus::Complete => "complete",
        ProcessCaptureStatus::Incomplete => "incomplete",
        ProcessCaptureStatus::NotApplicable => "not_applicable",
    }
}

fn format_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339()
}

fn bounded_info(info: &ProcessInfo) -> ProcessInfo {
    let mut info = info.clone();
    info.agent_id = truncate_text(&info.agent_id, 64 * 1024);
    info.process_id = truncate_text(&info.process_id, 64 * 1024);
    info.group = info.group.map(|value| truncate_text(&value, 64 * 1024));
    info.batch_id = info.batch_id.map(|value| truncate_text(&value, 64 * 1024));
    info.batch_call_id = info
        .batch_call_id
        .map(|value| truncate_text(&value, 64 * 1024));
    info.program = info.program.map(|value| truncate_text(&value, 64 * 1024));
    info.args.truncate(128);
    for arg in &mut info.args {
        *arg = truncate_text(arg, 4 * 1024);
    }
    info.working_directory = info
        .working_directory
        .map(|value| truncate_text(&value, 64 * 1024));
    info.command_preview = info
        .command_preview
        .map(|value| truncate_text(&value, 8 * 1024));
    info.reject_reason = info
        .reject_reason
        .map(|value| truncate_text(&value, MAX_ERROR_BYTES));
    info.skill_id = info.skill_id.map(|value| truncate_text(&value, 64 * 1024));
    info.skill_path = info
        .skill_path
        .map(|value| truncate_text(&value, 64 * 1024));
    info.installed_digest = info
        .installed_digest
        .map(|value| truncate_text(&value, 64 * 1024));
    info.mcp_server_id = info
        .mcp_server_id
        .map(|value| truncate_text(&value, 64 * 1024));
    info.mcp_tool_name = info
        .mcp_tool_name
        .map(|value| truncate_text(&value, 64 * 1024));
    info.cancel_outcome = info
        .cancel_outcome
        .map(|value| truncate_text(&value, MAX_ERROR_BYTES));
    info.termination_evidence = info
        .termination_evidence
        .map(|value| truncate_text(&value, MAX_ERROR_BYTES));
    info.capture_error = info
        .capture_error
        .map(|value| truncate_text(&value, MAX_ERROR_BYTES));
    info
}

fn bounded_detail(mut detail: ProcessDetail) -> ProcessDetail {
    detail.process = bounded_info(&detail.process);
    if let Some(error) = &mut detail.error {
        error.code = truncate_text(&error.code, 256);
        error.message = truncate_text(&error.message, MAX_ERROR_BYTES);
    }
    if let Some(preview) = &mut detail.result_preview {
        *preview = truncate_text(preview, 8 * 1024);
    }
    if let Some(result) = &detail.result {
        let size = serde_json::to_vec(result)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX);
        if size > 512 * 1024 {
            detail.result = None;
            detail.result_available = false;
        }
    }
    detail
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

pub(crate) fn encode_list_cursor(info: &ProcessInfo) -> String {
    let cursor = ListCursor {
        version: 1,
        created_at: format_time(info.created_at),
        process_id: info.process_id.clone(),
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).expect("list cursor serializes"))
}

pub(crate) fn decode_list_cursor(value: &str) -> Result<ProcessHistoryCursor> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| anyhow!("invalid_process_list_cursor"))?;
    let cursor: ListCursor =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("invalid_process_list_cursor"))?;
    if cursor.version != 1 || cursor.process_id.is_empty() {
        return Err(anyhow!("invalid_process_list_cursor"));
    }
    let created_at = DateTime::parse_from_rfc3339(&cursor.created_at)
        .map_err(|_| anyhow!("invalid_process_list_cursor"))?
        .with_timezone(&Utc);
    Ok(ProcessHistoryCursor {
        created_at,
        process_id: cursor.process_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentic-process-history-{name}-{}",
            Uuid::new_v4().simple()
        ))
    }

    fn info(process_id: &str, state: ProcessState) -> ProcessInfo {
        let now = Utc::now();
        ProcessInfo {
            agent_id: "agent".to_string(),
            process_id: process_id.to_string(),
            group: Some("group-a".to_string()),
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind: ProcessKind::Command,
            state,
            created_at: now,
            started_at: Some(now),
            updated_at: now,
            finished_at: state.is_terminal().then_some(now),
            program: Some("printf".to_string()),
            args: vec![],
            working_directory: None,
            command_preview: None,
            exit_code: Some(0),
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: Some("process_exit".to_string()),
            capture_status: ProcessCaptureStatus::Complete,
            capture_error: None,
        }
    }

    fn detail(info: ProcessInfo) -> ProcessDetail {
        ProcessDetail {
            process: info,
            detail_available: true,
            result: Some(json!({"ok": true})),
            error: None,
            result_available: true,
            result_bytes: Some(11),
            result_sha256: Some("sha256:test".to_string()),
            result_preview: None,
        }
    }

    #[test]
    fn opens_fresh_process_database_without_touching_legacy_jobs_database() {
        let root = root("fresh");
        fs::create_dir_all(&root).unwrap();
        let legacy = root.join("jobs.sqlite3");
        fs::write(&legacy, b"user data must remain untouched").unwrap();
        let store = ProcessHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        assert_eq!(store.path(), root.join("process.sqlite3"));
        assert_eq!(
            fs::read(&legacy).unwrap(),
            b"user data must remain untouched"
        );
        assert!(store.path().exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_output_and_offsets_survive_reopen_atomically() {
        let root = root("output");
        let paths = PrivateStatePaths::for_test(root.clone());
        let store = ProcessHistoryStore::open(&paths);
        let admission = info("process_boot_1", ProcessState::Running);
        assert!(store.insert_admissions([&admission]).is_persisted());
        let snapshot = detail(info("process_boot_1", ProcessState::Completed));
        let output = ProcessOutputSnapshot {
            stdout: vec![0xff, b'a', b'b'],
            stdout_start_offset: 7,
            stdout_end_offset: 10,
            stderr: b"err".to_vec(),
            stderr_start_offset: 2,
            stderr_end_offset: 5,
        };
        assert!(store.upsert_terminal(&snapshot, &output).is_persisted());
        let replacement = detail(info("process_boot_1", ProcessState::Failed));
        let replacement_output = ProcessOutputSnapshot {
            stdout: b"replacement".to_vec(),
            stdout_start_offset: 0,
            stdout_end_offset: 11,
            ..ProcessOutputSnapshot::default()
        };
        assert!(store
            .upsert_terminal(&replacement, &replacement_output)
            .is_persisted());
        drop(store);
        let reopened = ProcessHistoryStore::open(&paths);
        let record = reopened.get("process_boot_1").unwrap().unwrap();
        assert_eq!(record.info.state, ProcessState::Completed);
        assert_eq!(record.output.stdout, [0xff, b'a', b'b']);
        assert_eq!(record.output.stdout_start_offset, 7);
        assert_eq!(record.output.stdout_end_offset, 10);
        assert_eq!(record.output.stderr, b"err");
        assert_eq!(record.output.stderr_start_offset, 2);
        assert_eq!(record.output.stderr_end_offset, 5);
        assert_eq!(record.detail.unwrap().result, Some(json!({"ok": true})));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_event_outbox_survives_reopen_until_acknowledged() {
        let root = root("event-outbox");
        let paths = PrivateStatePaths::for_test(root.clone());
        let process_id = "process_event_outbox";
        let store = ProcessHistoryStore::open(&paths);
        let admission = info(process_id, ProcessState::Running);
        assert!(store
            .insert_admissions_with_event_tracking([&admission])
            .is_persisted());
        let terminal = info(process_id, ProcessState::Completed);
        assert!(store
            .upsert_terminal(&detail(terminal.clone()), &ProcessOutputSnapshot::default(),)
            .is_persisted());
        let pending = store.pending_event_completions().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].process_id, process_id);
        assert_eq!(pending[0].event_type, "process.completed");
        assert_eq!(pending[0].completed_at, terminal.updated_at);
        drop(store);

        let reopened = ProcessHistoryStore::open(&paths);
        assert_eq!(reopened.pending_event_completions().unwrap(), pending);
        assert!(reopened
            .get(process_id)
            .unwrap()
            .is_some_and(|record| record.info.state == ProcessState::Completed));
        reopened.acknowledge_event_completion(process_id).unwrap();
        assert!(reopened.pending_event_completions().unwrap().is_empty());
        drop(reopened);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn early_terminal_evidence_survives_full_snapshot_and_restart_recovery() {
        let root = root("early-event");
        let store = ProcessHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        let full_snapshot_id = "process_early_event";
        let restart_id = "process_restart_after_terminal";
        let full_admission = info(full_snapshot_id, ProcessState::Running);
        let restart_admission = info(restart_id, ProcessState::Running);
        assert!(store
            .insert_admissions_with_event_tracking([&full_admission, &restart_admission])
            .is_persisted());

        let completed = info(full_snapshot_id, ProcessState::Completed);
        assert!(store.record_terminal_event(&completed).is_persisted());
        assert_eq!(
            store.get(full_snapshot_id).unwrap().unwrap().info.state,
            ProcessState::Running
        );
        let early_completion = store
            .pending_event_completions()
            .unwrap()
            .into_iter()
            .find(|completion| completion.process_id == full_snapshot_id)
            .unwrap();
        let final_output = ProcessOutputSnapshot {
            stdout: b"final output".to_vec(),
            stdout_end_offset: 12,
            ..ProcessOutputSnapshot::default()
        };
        assert!(store
            .upsert_terminal(&detail(completed), &final_output)
            .is_persisted());
        assert_eq!(
            store
                .pending_event_completions()
                .unwrap()
                .into_iter()
                .find(|completion| completion.process_id == full_snapshot_id)
                .unwrap(),
            early_completion
        );
        assert_eq!(
            store.get(full_snapshot_id).unwrap().unwrap().output.stdout,
            b"final output"
        );

        let completed_before_restart = info(restart_id, ProcessState::Completed);
        assert!(store
            .record_terminal_event(&completed_before_restart)
            .is_persisted());
        assert_eq!(store.recover_active(Utc::now()).unwrap(), 1);
        let recovered_history = store.get(restart_id).unwrap().unwrap();
        assert_eq!(
            recovered_history.info.state,
            ProcessState::UnknownAfterRestart
        );
        let recovered_event = store
            .pending_event_completions()
            .unwrap()
            .into_iter()
            .find(|completion| completion.process_id == restart_id)
            .unwrap();
        assert_eq!(recovered_event.event_type, "process.completed");
        assert_eq!(
            recovered_event.completed_at,
            completed_before_restart.updated_at
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_admission_metadata_rejects_the_entire_batch() {
        let root = root("admission-size");
        let store = ProcessHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        let accepted = info("process_admission_first", ProcessState::Running);
        let mut oversized = info("process_admission_oversized", ProcessState::Running);
        oversized.args = vec!["x".repeat(4 * 1024); 70];

        let outcome = store.insert_admissions([&accepted, &oversized]);
        assert_eq!(
            outcome.error(),
            Some("process_history_admission_metadata_too_large")
        );
        assert!(store.get(&accepted.process_id).unwrap().is_none());
        assert!(store.get(&oversized.process_id).unwrap().is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn admitted_metadata_reserves_terminal_error_space() {
        let root = root("admission-reserve");
        let store = ProcessHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        let mut admission = info("process_terminal_reserve", ProcessState::Queued);
        admission.args = vec!["x".repeat(4 * 1024); 35];
        assert!(store.insert_admissions([&admission]).is_persisted());

        let mut running = admission.clone();
        running.state = ProcessState::Running;
        running.capture_status = ProcessCaptureStatus::Capturing;
        running.started_at = Some(Utc::now());
        running.updated_at = Utc::now();
        assert!(store.mark_started(&running).is_persisted());

        let mut terminal = running;
        terminal.state = ProcessState::Failed;
        terminal.capture_status = ProcessCaptureStatus::Incomplete;
        terminal.updated_at = Utc::now();
        terminal.finished_at = Some(terminal.updated_at);
        let error = "\0".repeat(MAX_ERROR_BYTES);
        terminal.reject_reason = Some(error.clone());
        terminal.capture_error = Some(error.clone());
        assert!(store
            .upsert_terminal(&detail(terminal.clone()), &ProcessOutputSnapshot::default())
            .is_persisted());

        let persisted = store
            .get(&terminal.process_id)
            .unwrap()
            .expect("bounded terminal snapshot must persist");
        assert_eq!(persisted.info.args.len(), 35);
        assert_eq!(
            persisted.info.reject_reason.as_deref(),
            Some(error.as_str())
        );
        assert_eq!(
            persisted.info.capture_error.as_deref(),
            Some(error.as_str())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn process_list_filters_and_cursor_rejects_malformed_values() {
        let root = root("list");
        let store = ProcessHistoryStore::open(&PrivateStatePaths::for_test(root.clone()));
        let older = info("process_a", ProcessState::Completed);
        let newer = info("process_b", ProcessState::Completed);
        store.insert_admissions([&older, &newer]);
        let request = ProcessListRequest {
            group: Some("group-a".to_string()),
            kind: Some(ProcessKind::Command),
            state: Some(ProcessState::Completed),
            limit: Some(1),
            cursor: None,
        };
        let page = store.list(&request).unwrap();
        assert_eq!(page.processes.len(), 1);
        assert!(page.next_cursor.is_some());
        let error = store
            .list(&ProcessListRequest {
                cursor: Some("not-a-cursor".to_string()),
                ..ProcessListRequest::default()
            })
            .unwrap_err();
        assert!(error.to_string().contains("invalid_process_list_cursor"));
        let _ = fs::remove_dir_all(root);
    }
}
