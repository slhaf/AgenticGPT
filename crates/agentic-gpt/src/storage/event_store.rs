#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use agentic_gpt_protocol::{
    EventInjectRequest, EventListItem, EventListRequest, EventListResponse, EventMarkResponse,
    EventOrigin, EventPanel, EventRecord, EventSeverity, EventSource, EventSourceKind, EventStatus,
};
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Datelike, Duration, SecondsFormat, Utc};
use rusqlite::{
    params, types::Value, Connection, OptionalExtension, ToSql, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::private_state::PrivateStatePaths;

const EVENT_STORE_SCHEMA_VERSION: i64 = 3;
const SQLITE_BUSY_TIMEOUT_MS: u64 = 1_000;
const EVENT_HISTORY_RETENTION: Duration = Duration::days(7);
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_REFERENCE_BYTES: usize = 512;
const MAX_EVENT_TYPE_BYTES: usize = 128;
const MAX_AGENT_SCOPE_BYTES: usize = 512;
const MAX_ORIGIN_COMPONENT_BYTES: usize = 512;
const MAX_CURSOR_BYTES: usize = 2 * 1024;
const MAX_EVENT_IDS_PER_MARK: usize = 512;
const MAX_PENDING_EVENTS: i64 = 8_192;
pub(crate) const MAX_LOW_TTL_SECONDS: u64 = (i64::MAX / 1000) as u64;
const DEFAULT_PAGE_SIZE: usize = 20;
const MAX_PAGE_SIZE: usize = 100;
const INTERNAL_POLICY_MAX_OVERRIDES: usize = 128;
const COMPACT_POLICY_JSON: &str = r#"{"low_ttl_seconds":86400,"overrides":{}}"#;

struct InternalSourceOriginRow {
    response_state: String,
    run_id: Option<String>,
    request_id: Option<String>,
    command_hash: Option<String>,
}

struct InternalSourceSettlementRow {
    response_state: String,
    policy_json: String,
    completion_seen: i64,
    event_type: Option<String>,
    message: Option<String>,
    completed_at: Option<String>,
    run_id: Option<String>,
    request_id: Option<String>,
    command_hash: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS events (
    event_id TEXT PRIMARY KEY NOT NULL,
    message TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('low','medium','high')),
    created_at TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending','handled','expired')),
    source_kind TEXT NOT NULL CHECK (source_kind IN ('process','skill_install','external')),
    source_ref TEXT NOT NULL,
    shown_count INTEGER NOT NULL DEFAULT 0 CHECK (shown_count >= 0),
    expires_at TEXT,
    terminal_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_list
    ON events(status, severity, created_at DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_events_pending
    ON events(status, severity, created_at, event_id);
CREATE TABLE IF NOT EXISTS internal_sources (
    source_kind TEXT NOT NULL CHECK (source_kind IN ('process','skill_install')),
    source_ref TEXT NOT NULL,
    response_state TEXT NOT NULL CHECK (response_state IN ('awaiting_response','async_eligible','suppressed')),
    settled_at TEXT,
    policy_json TEXT NOT NULL,
    completion_seen INTEGER NOT NULL DEFAULT 0 CHECK (completion_seen IN (0,1)),
    completion_event_type TEXT,
    completion_message TEXT,
    completed_at TEXT,
    notified INTEGER NOT NULL DEFAULT 0 CHECK (notified IN (0,1)),
    origin_run_id TEXT,
    origin_request_id TEXT,
    origin_command_hash TEXT,
    PRIMARY KEY (source_kind, source_ref)
);
CREATE INDEX IF NOT EXISTS idx_internal_sources_recovery
    ON internal_sources(response_state, completion_seen, source_kind, source_ref);
CREATE TABLE IF NOT EXISTS event_store_metadata (
    id INTEGER PRIMARY KEY CHECK (id=1),
    owner_fingerprint TEXT NOT NULL
);

"#;
const MIGRATE_V1_TO_V2: &str = r#"
ALTER TABLE internal_sources ADD COLUMN origin_run_id TEXT;
ALTER TABLE internal_sources ADD COLUMN origin_request_id TEXT;
ALTER TABLE internal_sources ADD COLUMN origin_command_hash TEXT;
"#;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct InternalEventPolicy {
    pub(crate) low_ttl_seconds: u64,
    pub(crate) overrides: BTreeMap<String, Option<EventSeverity>>,
}

impl Default for InternalEventPolicy {
    fn default() -> Self {
        Self {
            low_ttl_seconds: 24 * 60 * 60,
            overrides: BTreeMap::new(),
        }
    }
}

impl InternalEventPolicy {
    fn validate(&self) -> Result<()> {
        checked_expiry_at(self.low_ttl_seconds, Utc::now())?;
        if self.overrides.len() > INTERNAL_POLICY_MAX_OVERRIDES {
            return Err(anyhow!("event_policy_too_many_overrides"));
        }
        for event_type in self.overrides.keys() {
            validate_event_type(event_type)?;
        }
        Ok(())
    }

    fn severity_for(&self, event_type: &str) -> Option<EventSeverity> {
        self.overrides
            .get(event_type)
            .copied()
            .unwrap_or(Some(EventSeverity::Low))
    }
}

pub(crate) struct EventStore {
    connection: Mutex<Connection>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListCursor {
    version: u8,
    agent_id: String,
    status: EventStatus,
    severity: Option<EventSeverity>,
    created_at: String,
    event_id: String,
}

#[derive(Debug)]
struct RawEvent {
    event_id: String,
    message: String,
    severity: String,
    created_at: String,
    status: String,
    source_kind: String,
    source_ref: String,
    shown_count: i64,
    expires_at: Option<String>,
}

impl RawEvent {
    fn into_record(self) -> Result<EventRecord> {
        Ok(EventRecord {
            event_id: self.event_id,
            message: self.message,
            severity: parse_severity(&self.severity)?,
            created_at: parse_time(&self.created_at)?,
            status: parse_status(&self.status)?,
            source: EventSource {
                kind: parse_source_kind(&self.source_kind)?,
                reference: self.source_ref,
            },
            shown_count: u32::try_from(self.shown_count)
                .map_err(|_| anyhow!("event_store_invalid_shown_count"))?,
            expires_at: self.expires_at.as_deref().map(parse_time).transpose()?,
        })
    }
}

impl EventStore {
    pub(crate) fn open(paths: &PrivateStatePaths) -> Result<Arc<Self>> {
        let owner_fingerprint = event_store_owner_fingerprint(&paths.agent_id)?;
        let path = paths.root.join("events.sqlite3");
        ensure_private_database_file(&paths.root, &path)?;
        let mut connection = Connection::open(&path)
            .with_context(|| format!("event_store_open_failed: {}", path.display()))?;
        connection
            .busy_timeout(StdDuration::from_millis(SQLITE_BUSY_TIMEOUT_MS))
            .context("event_store_busy_timeout_failed")?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .context("event_store_synchronous_failed")?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("event_store_schema_transaction_failed")?;
        let version: i64 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .context("event_store_schema_version_read_failed")?;
        if version > EVENT_STORE_SCHEMA_VERSION {
            return Err(anyhow!(
                "unsupported event store schema version {version}; maximum supported {EVENT_STORE_SCHEMA_VERSION}"
            ));
        }
        transaction
            .execute_batch(SCHEMA)
            .context("event_store_schema_create_failed")?;
        if version == 1 {
            transaction
                .execute_batch(MIGRATE_V1_TO_V2)
                .context("event_store_schema_v1_migration_failed")?;
        }
        ensure_store_owner(&transaction, &owner_fingerprint)?;
        transaction
            .pragma_update(None, "user_version", EVENT_STORE_SCHEMA_VERSION)
            .context("event_store_schema_version_write_failed")?;
        transaction
            .commit()
            .context("event_store_schema_commit_failed")?;

        Ok(Arc::new(Self {
            connection: Mutex::new(connection),
        }))
    }

    pub(crate) fn list(&self, request: &EventListRequest) -> Result<EventListResponse> {
        self.list_at(request, Utc::now())
    }

    fn list_at(&self, request: &EventListRequest, now: DateTime<Utc>) -> Result<EventListResponse> {
        validate_agent_scope(&request.agent_id)?;
        let limit = request
            .limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
        let cursor = request
            .cursor
            .as_deref()
            .map(|value| decode_cursor(value, request))
            .transpose()?;

        let mut sql = String::from(
            "SELECT event_id,message,severity,created_at,status,source_kind,source_ref,shown_count,expires_at FROM events WHERE 1=1",
        );
        let mut values = Vec::<Value>::new();
        let status = request.status.unwrap_or(EventStatus::Pending);
        sql.push_str(" AND status=?");
        values.push(Value::Text(status.as_str().to_string()));
        if let Some(severity) = request.severity {
            sql.push_str(" AND severity=?");
            values.push(Value::Text(severity.as_str().to_string()));
        }
        if let Some(cursor) = &cursor {
            sql.push_str(" AND (created_at<? OR (created_at=? AND event_id<?))");
            values.push(Value::Text(cursor.created_at.clone()));
            values.push(Value::Text(cursor.created_at.clone()));
            values.push(Value::Text(cursor.event_id.clone()));
        }
        sql.push_str(" ORDER BY created_at DESC,event_id DESC LIMIT ?");
        values.push(Value::Integer((limit + 1) as i64));

        let records = self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            let parameters = values
                .iter()
                .map(|value| value as &dyn ToSql)
                .collect::<Vec<_>>();
            let mut statement = transaction
                .prepare(&sql)
                .context("event_store_list_prepare_failed")?;
            let rows = statement
                .query_map(parameters.as_slice(), raw_event_from_row)
                .context("event_store_list_query_failed")?;
            let mut raw_events = Vec::new();
            for row in rows {
                raw_events.push(row.context("event_store_list_row_failed")?);
            }
            raw_events
                .into_iter()
                .map(RawEvent::into_record)
                .collect::<Result<Vec<_>>>()
        })?;

        let mut records = records;
        let has_more = records.len() > limit;
        if has_more {
            records.truncate(limit);
        }
        let next_cursor = if has_more {
            records
                .last()
                .map(|record| encode_cursor(request, record))
                .transpose()?
        } else {
            None
        };
        let items = records
            .into_iter()
            .map(|record| EventListItem {
                event_id: record.event_id,
                summary: summarize(&record.message),
                severity: record.severity,
                created_at: record.created_at,
                status: record.status,
            })
            .collect();
        Ok(EventListResponse { items, next_cursor })
    }

    pub(crate) fn get(&self, event_id: &str) -> Result<EventRecord> {
        self.get_at(event_id, Utc::now())
    }

    fn get_at(&self, event_id: &str, now: DateTime<Utc>) -> Result<EventRecord> {
        validate_reference(event_id, "event_id")?;
        let record = self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            let raw = transaction
                .query_row(
                    "SELECT event_id,message,severity,created_at,status,source_kind,source_ref,shown_count,expires_at FROM events WHERE event_id=?1",
                    params![event_id],
                    raw_event_from_row,
                )
                .optional()
                .context("event_store_get_query_failed")?;
            raw.map(RawEvent::into_record).transpose()
        })?;
        record.ok_or_else(|| anyhow!("event_not_found"))
    }

    pub(crate) fn mark(&self, event_ids: &[String]) -> Result<EventMarkResponse> {
        self.mark_at(event_ids, Utc::now())
    }

    fn mark_at(&self, event_ids: &[String], now: DateTime<Utc>) -> Result<EventMarkResponse> {
        if event_ids.len() > MAX_EVENT_IDS_PER_MARK {
            return Err(anyhow!("event_mark_too_many_ids"));
        }
        for event_id in event_ids {
            validate_reference(event_id, "event_id")?;
        }
        self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            let now_text = format_time(now);
            let mut seen = BTreeSet::new();
            let mut response = EventMarkResponse::default();
            for event_id in event_ids {
                if !seen.insert(event_id.as_str()) {
                    continue;
                }
                let changed = transaction
                    .execute(
                        "UPDATE events SET status='handled',terminal_at=?1 WHERE event_id=?2 AND status='pending'",
                        params![now_text, event_id],
                    )
                    .context("event_store_mark_update_failed")?;
                if changed == 1 {
                    response.handled_ids.push(event_id.clone());
                    continue;
                }
                let status: Option<String> = transaction
                    .query_row(
                        "SELECT status FROM events WHERE event_id=?1",
                        params![event_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .context("event_store_mark_status_query_failed")?;
                if status.as_deref() == Some(EventStatus::Handled.as_str()) {
                    response.handled_ids.push(event_id.clone());
                } else {
                    response.not_found_ids.push(event_id.clone());
                }
            }
            Ok(response)
        })
    }

    pub(crate) fn panel(&self) -> Result<EventPanel> {
        self.panel_at(Utc::now())
    }

    fn panel_at(&self, now: DateTime<Utc>) -> Result<EventPanel> {
        self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            let counts = pending_counts(transaction)?;
            let mut statement = transaction
                .prepare(
                    "SELECT event_id,message,severity,created_at,shown_count FROM events
                     WHERE status='pending' AND (
                         severity='high' OR
                         (severity='medium' AND shown_count<3) OR
                         (severity='low' AND shown_count<1)
                     )
                     ORDER BY CASE severity WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                         created_at ASC,event_id ASC LIMIT 5",
                )
                .context("event_store_panel_prepare_failed")?;
            let rows = statement
                .query_map([], |row| {
                    Ok(PanelRow {
                        event_id: row.get(0)?,
                        message: row.get(1)?,
                        severity: row.get(2)?,
                        created_at: row.get(3)?,
                        shown_count: row.get(4)?,
                    })
                })
                .context("event_store_panel_query_failed")?;
            let mut candidates = Vec::new();
            for row in rows {
                candidates.push(row.context("event_store_panel_row_failed")?);
            }
            drop(statement);

            let mut new = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let changed = transaction
                    .execute(
                        "UPDATE events SET shown_count=shown_count+1 WHERE event_id=?1 AND status='pending' AND shown_count=?2",
                        params![candidate.event_id, candidate.shown_count],
                    )
                    .context("event_store_panel_exposure_update_failed")?;
                if changed != 1 {
                    return Err(anyhow!("event_panel_exposure_conflict"));
                }
                let severity = parse_severity(&candidate.severity)?;
                let mut item = BTreeMap::new();
                item.insert(
                    format!("{} | {}", candidate.event_id, summarize(&candidate.message)),
                    format!("{} | {}", severity.as_str(), candidate.created_at),
                );
                new.push(item);
            }
            Ok(EventPanel {
                current: format!(
                    "low: {} | medium: {} | high: {}",
                    counts.low, counts.medium, counts.high
                ),
                new,
            })
        })
    }

    pub(crate) fn inject(
        &self,
        request: &EventInjectRequest,
        low_ttl_seconds: u64,
    ) -> Result<EventRecord> {
        self.inject_at(request, low_ttl_seconds, Utc::now())
    }

    fn inject_at(
        &self,
        request: &EventInjectRequest,
        low_ttl_seconds: u64,
        now: DateTime<Utc>,
    ) -> Result<EventRecord> {
        validate_message(&request.message)?;
        validate_reference(&request.reference, "event_ref")?;
        let severity = request.severity.unwrap_or(EventSeverity::Low);
        let expires_at = expiry_for(severity, low_ttl_seconds, now)?;
        self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            ensure_pending_capacity(transaction)?;
            let source = EventSource {
                kind: EventSourceKind::External,
                reference: request.reference.clone(),
            };
            let event_id = insert_event(
                transaction,
                &request.message,
                severity,
                now,
                &source,
                expires_at.as_ref(),
            )?;
            let raw = transaction
                .query_row(
                    "SELECT event_id,message,severity,created_at,status,source_kind,source_ref,shown_count,expires_at FROM events WHERE event_id=?1",
                    params![event_id],
                    raw_event_from_row,
                )
                .context("event_store_injected_record_query_failed")?;
            raw.into_record()
        })
    }

    pub(crate) fn register_internal(
        &self,
        source: &EventSource,
        policy: &InternalEventPolicy,
    ) -> Result<()> {
        validate_internal_source(source)?;
        self.transact(|transaction| {
            let exists: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM internal_sources WHERE source_kind=?1 AND source_ref=?2",
                    params![source.kind.as_str(), source.reference],
                    |row| row.get(0),
                )
                .optional()
                .context("event_store_internal_registration_lookup_failed")?;
            if exists.is_some() {
                return Ok(());
            }
            policy.validate()?;
            let policy_json = serde_json::to_string(policy)
                .context("event_store_internal_policy_serialize_failed")?;
            transaction
                .execute(
                    "INSERT INTO internal_sources(source_kind,source_ref,response_state,policy_json)
                     VALUES (?1,?2,'awaiting_response',?3)",
                    params![source.kind.as_str(), source.reference, policy_json],
                )
                .context("event_store_internal_registration_failed")?;
            Ok(())
        })
    }

    pub(crate) fn bind_origin(&self, source: &EventSource, origin: &EventOrigin) -> Result<()> {
        validate_internal_source(source)?;
        validate_origin(origin)?;
        self.transact(|transaction| {
            let row: Option<InternalSourceOriginRow> = transaction
                .query_row(
                    "SELECT response_state,origin_run_id,origin_request_id,origin_command_hash
                     FROM internal_sources WHERE source_kind=?1 AND source_ref=?2",
                    params![source.kind.as_str(), source.reference],
                    |row| {
                        Ok(InternalSourceOriginRow {
                            response_state: row.get(0)?,
                            run_id: row.get(1)?,
                            request_id: row.get(2)?,
                            command_hash: row.get(3)?,
                        })
                    },
                )
                .optional()
                .context("event_store_origin_binding_lookup_failed")?;
            let Some(InternalSourceOriginRow {
                response_state,
                run_id,
                request_id,
                command_hash,
            }) = row
            else {
                return Err(anyhow!("event_internal_source_not_registered"));
            };
            if let Some(existing) = origin_from_fields(run_id, request_id, command_hash)? {
                return if existing == *origin {
                    Ok(())
                } else {
                    Err(anyhow!("event_origin_binding_conflict"))
                };
            }
            if response_state != "awaiting_response" {
                return Err(anyhow!("event_origin_bind_after_settlement"));
            }
            let changed = transaction
                .execute(
                    "UPDATE internal_sources
                     SET origin_run_id=?1,origin_request_id=?2,origin_command_hash=?3
                     WHERE source_kind=?4 AND source_ref=?5
                        AND origin_run_id IS NULL AND origin_request_id IS NULL AND origin_command_hash IS NULL
                        AND response_state='awaiting_response'",
                    params![
                        origin.run_id,
                        origin.request_id,
                        origin.command_hash,
                        source.kind.as_str(),
                        source.reference
                    ],
                )
                .context("event_store_origin_binding_write_failed")?;
            if changed != 1 {
                return Err(anyhow!("event_origin_binding_conflict"));
            }
            Ok(())
        })
    }

    pub(crate) fn remote_origin(&self, source: &EventSource) -> Result<Option<EventOrigin>> {
        validate_internal_source(source)?;
        self.transact(|transaction| {
            let row: Option<(Option<String>, Option<String>, Option<String>)> = transaction
                .query_row(
                    "SELECT origin_run_id,origin_request_id,origin_command_hash
                     FROM internal_sources WHERE source_kind=?1 AND source_ref=?2",
                    params![source.kind.as_str(), source.reference],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .context("event_store_origin_read_failed")?;
            row.map(|(run_id, request_id, command_hash)| {
                origin_from_fields(run_id, request_id, command_hash)
            })
            .transpose()
            .map(Option::flatten)
        })
    }

    pub(crate) fn record_internal_completion(
        &self,
        source: &EventSource,
        event_type: &str,
        message: &str,
        at: DateTime<Utc>,
    ) -> Result<()> {
        validate_internal_source(source)?;
        let now = Utc::now();
        self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            let row: Option<(String, String, i64)> = transaction
                .query_row(
                    "SELECT response_state,policy_json,completion_seen FROM internal_sources WHERE source_kind=?1 AND source_ref=?2",
                    params![source.kind.as_str(), source.reference],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .context("event_store_internal_completion_lookup_failed")?;
            let Some((response_state, policy_json, completion_seen)) = row else {
                return Err(anyhow!("event_internal_source_not_registered"));
            };
            if completion_seen != 0 {
                return Ok(());
            }
            validate_event_type(event_type)?;
            validate_message(message)?;
            transaction
                .execute(
                    "UPDATE internal_sources SET completion_seen=1,completion_event_type=?1,completion_message=?2,completed_at=?3
                     WHERE source_kind=?4 AND source_ref=?5 AND completion_seen=0",
                    params![event_type, message, format_time(at), source.kind.as_str(), source.reference],
                )
                .context("event_store_internal_completion_write_failed")?;
            match response_state.as_str() {
                "awaiting_response" => Ok(()),
                "suppressed" => {
                    clear_internal_completion(transaction, source)?;
                    Ok(())
                }
                "async_eligible" => {
                    emit_internal_completion(
                        transaction,
                        source,
                        &policy_json,
                        event_type,
                        message,
                        at,
                    )?;
                    clear_internal_completion(transaction, source)?;
                    Ok(())
                }
                _ => Err(anyhow!("event_store_invalid_internal_response_state")),
            }
        })
    }

    pub(crate) fn settle_response(
        &self,
        source: &EventSource,
        includes_terminal: bool,
    ) -> Result<()> {
        validate_internal_source(source)?;
        self.settle_response_inner(source, None, includes_terminal)
    }

    pub(crate) fn settle_remote_response(
        &self,
        source: &EventSource,
        origin: &EventOrigin,
        includes_terminal: bool,
    ) -> Result<()> {
        validate_internal_source(source)?;
        validate_origin(origin)?;
        self.settle_response_inner(source, Some(origin), includes_terminal)
    }

    pub(crate) fn contains_internal_sources<'a>(
        &self,
        sources: impl Iterator<Item = &'a EventSource>,
    ) -> Result<Vec<bool>> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut statement = connection
            .prepare("SELECT 1 FROM internal_sources WHERE source_kind=?1 AND source_ref=?2")
            .context("event_store_internal_source_lookup_prepare_failed")?;
        let mut found = Vec::new();
        for source in sources {
            validate_internal_source(source)?;
            let exists: Option<i64> = statement
                .query_row(params![source.kind.as_str(), source.reference], |row| {
                    row.get(0)
                })
                .optional()
                .context("event_store_internal_source_lookup_failed")?;
            found.push(exists.is_some());
        }
        Ok(found)
    }

    fn settle_response_inner(
        &self,
        source: &EventSource,
        expected_origin: Option<&EventOrigin>,
        includes_terminal: bool,
    ) -> Result<()> {
        let now = Utc::now();
        self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            let row: Option<InternalSourceSettlementRow> = transaction
                .query_row(
                    "SELECT response_state,policy_json,completion_seen,completion_event_type,
                            completion_message,completed_at,origin_run_id,origin_request_id,origin_command_hash
                     FROM internal_sources WHERE source_kind=?1 AND source_ref=?2",
                    params![source.kind.as_str(), source.reference],
                    |row| {
                        Ok(InternalSourceSettlementRow {
                            response_state: row.get(0)?,
                            policy_json: row.get(1)?,
                            completion_seen: row.get(2)?,
                            event_type: row.get(3)?,
                            message: row.get(4)?,
                            completed_at: row.get(5)?,
                            run_id: row.get(6)?,
                            request_id: row.get(7)?,
                            command_hash: row.get(8)?,
                        })
                    },
                )
                .optional()
                .context("event_store_response_settlement_lookup_failed")?;
            let Some(InternalSourceSettlementRow {
                response_state,
                policy_json,
                completion_seen,
                event_type,
                message,
                completed_at,
                run_id,
                request_id,
                command_hash,
            }) = row
            else {
                return Err(anyhow!("event_internal_source_not_registered"));
            };
            let stored_origin = origin_from_fields(run_id, request_id, command_hash)?;
            match (expected_origin, stored_origin.as_ref()) {
                (None, None) => {}
                (None, Some(_)) => {
                    return Err(anyhow!("event_remote_response_requires_hub_settlement"));
                }
                (Some(expected), Some(stored)) if expected == stored => {}
                (Some(_), _) => return Err(anyhow!("event_origin_mismatch")),
            }

            if response_state != "awaiting_response" {
                if !matches!(response_state.as_str(), "async_eligible" | "suppressed") {
                    return Err(anyhow!("event_store_invalid_internal_response_state"));
                }
                if expected_origin.is_some() {
                    let same_disposition = (response_state == "suppressed") == includes_terminal;
                    return if same_disposition {
                        Ok(())
                    } else {
                        Err(anyhow!("event_response_disposition_conflict"))
                    };
                }
                return Ok(());
            }

            if includes_terminal {
                let changed = transaction
                    .execute(
                        "UPDATE internal_sources SET response_state='suppressed',settled_at=?1,
                            completion_event_type=NULL,completion_message=NULL,policy_json=?2,
                            notified=CASE WHEN completion_seen=1 THEN 1 ELSE notified END
                         WHERE source_kind=?3 AND source_ref=?4 AND response_state='awaiting_response'",
                        params![
                            format_time(now),
                            COMPACT_POLICY_JSON,
                            source.kind.as_str(),
                            source.reference
                        ],
                    )
                    .context("event_store_response_suppression_failed")?;
                if changed != 1 {
                    return Err(anyhow!("event_response_settlement_conflict"));
                }
                return Ok(());
            }

            let changed = transaction
                .execute(
                    "UPDATE internal_sources SET response_state='async_eligible',settled_at=?1
                     WHERE source_kind=?2 AND source_ref=?3 AND response_state='awaiting_response'",
                    params![format_time(now), source.kind.as_str(), source.reference],
                )
                .context("event_store_response_eligibility_failed")?;
            if changed != 1 {
                return Err(anyhow!("event_response_settlement_conflict"));
            }
            if completion_seen != 0 {
                let event_type = event_type
                    .as_deref()
                    .ok_or_else(|| anyhow!("event_store_internal_completion_missing_type"))?;
                let message = message
                    .as_deref()
                    .ok_or_else(|| anyhow!("event_store_internal_completion_missing_message"))?;
                let at = completed_at
                    .as_deref()
                    .map(parse_time)
                    .transpose()?
                    .ok_or_else(|| anyhow!("event_store_internal_completion_missing_time"))?;
                emit_internal_completion(
                    transaction,
                    source,
                    &policy_json,
                    event_type,
                    message,
                    at,
                )?;
                clear_internal_completion(transaction, source)?;
            }
            Ok(())
        })
    }

    pub(crate) fn pending_internal_sources(&self) -> Result<Vec<EventSource>> {
        let now = Utc::now();
        self.transact(|transaction| {
            expire_and_cleanup_at(transaction, now)?;
            let mut statement = transaction
                .prepare(
                    "SELECT source_kind,source_ref FROM internal_sources
                     WHERE response_state='awaiting_response'
                        OR (response_state='async_eligible' AND completion_seen=0)
                     ORDER BY source_kind,source_ref",
                )
                .context("event_store_internal_recovery_prepare_failed")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .context("event_store_internal_recovery_query_failed")?;
            let mut sources = Vec::new();
            for row in rows {
                let (kind, reference) = row.context("event_store_internal_recovery_row_failed")?;
                sources.push(EventSource {
                    kind: parse_source_kind(&kind)?,
                    reference,
                });
            }
            Ok(sources)
        })
    }

    fn transact<R>(&self, operation: impl FnOnce(&Transaction<'_>) -> Result<R>) -> Result<R> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("event_store_transaction_begin_failed")?;
        let result = operation(&transaction)?;
        transaction
            .commit()
            .context("event_store_transaction_commit_failed")?;
        Ok(result)
    }
}

#[derive(Debug)]
struct PanelRow {
    event_id: String,
    message: String,
    severity: String,
    created_at: String,
    shown_count: i64,
}

#[derive(Default)]
struct PendingCounts {
    low: i64,
    medium: i64,
    high: i64,
}

fn event_store_owner_fingerprint(agent_id: &str) -> Result<String> {
    if agent_id.trim().is_empty() {
        return Err(anyhow!("event_store_agent_id_invalid"));
    }
    let digest = Sha256::digest(agent_id.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn ensure_store_owner(transaction: &Transaction<'_>, owner_fingerprint: &str) -> Result<()> {
    let stored: Option<String> = transaction
        .query_row(
            "SELECT owner_fingerprint FROM event_store_metadata WHERE id=1",
            [],
            |row| row.get(0),
        )
        .optional()
        .context("event_store_owner_read_failed")?;
    match stored {
        Some(stored) if stored == owner_fingerprint => Ok(()),
        Some(_) => Err(anyhow!("event_store_owner_mismatch")),
        None => {
            transaction
                .execute(
                    "INSERT INTO event_store_metadata(id,owner_fingerprint) VALUES (1,?1)",
                    params![owner_fingerprint],
                )
                .context("event_store_owner_write_failed")?;
            Ok(())
        }
    }
}

fn ensure_private_database_file(root: &Path, path: &Path) -> Result<()> {
    let root_metadata = fs::symlink_metadata(root)
        .with_context(|| format!("event_store_private_root_unavailable: {}", root.display()))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(anyhow!("event_store_private_root_invalid"));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(anyhow!("event_store_database_path_invalid"));
            }
            set_private_file_permissions(path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut options = OpenOptions::new();
            options.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(path) {
                Ok(file) => drop(file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(path)
                        .context("event_store_database_path_check_failed")?;
                    if metadata.file_type().is_symlink() || !metadata.is_file() {
                        return Err(anyhow!("event_store_database_path_invalid"));
                    }
                    set_private_file_permissions(path)?;
                }
                Err(error) => return Err(error).context("event_store_database_create_failed"),
            }
        }
        Err(error) => return Err(error).context("event_store_database_path_check_failed"),
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).with_context(|| {
            format!(
                "event_store_database_permissions_failed: {}",
                path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn raw_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawEvent> {
    Ok(RawEvent {
        event_id: row.get(0)?,
        message: row.get(1)?,
        severity: row.get(2)?,
        created_at: row.get(3)?,
        status: row.get(4)?,
        source_kind: row.get(5)?,
        source_ref: row.get(6)?,
        shown_count: row.get(7)?,
        expires_at: row.get(8)?,
    })
}

fn expire_and_cleanup_at(transaction: &Transaction<'_>, now: DateTime<Utc>) -> Result<()> {
    let now_text = format_time(now);
    transaction
        .execute(
            "UPDATE events SET status='expired',terminal_at=expires_at
             WHERE status='pending' AND expires_at IS NOT NULL AND expires_at<=?1",
            params![now_text],
        )
        .context("event_store_expiration_update_failed")?;
    let retention_boundary = format_time(now - EVENT_HISTORY_RETENTION);
    transaction
        .execute(
            "DELETE FROM events WHERE status IN ('handled','expired') AND terminal_at IS NOT NULL AND terminal_at<=?1",
            params![retention_boundary],
        )
        .context("event_store_history_cleanup_failed")?;
    Ok(())
}

fn pending_counts(transaction: &Transaction<'_>) -> Result<PendingCounts> {
    let mut counts = PendingCounts::default();
    let mut statement = transaction
        .prepare("SELECT severity,COUNT(*) FROM events WHERE status='pending' GROUP BY severity")
        .context("event_store_panel_count_prepare_failed")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .context("event_store_panel_count_query_failed")?;
    for row in rows {
        let (severity, count) = row.context("event_store_panel_count_row_failed")?;
        match parse_severity(&severity)? {
            EventSeverity::Low => counts.low = count,
            EventSeverity::Medium => counts.medium = count,
            EventSeverity::High => counts.high = count,
        }
    }
    Ok(counts)
}

fn ensure_pending_capacity(transaction: &Transaction<'_>) -> Result<()> {
    let count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM events WHERE status='pending'",
            [],
            |row| row.get(0),
        )
        .context("event_store_pending_capacity_query_failed")?;
    if count >= MAX_PENDING_EVENTS {
        return Err(anyhow!("event_pending_capacity_reached"));
    }
    Ok(())
}

fn insert_event(
    transaction: &Transaction<'_>,
    message: &str,
    severity: EventSeverity,
    created_at: DateTime<Utc>,
    source: &EventSource,
    expires_at: Option<&DateTime<Utc>>,
) -> Result<String> {
    let event_id = Uuid::new_v4().simple().to_string();
    transaction
        .execute(
            "INSERT INTO events(event_id,message,severity,created_at,status,source_kind,source_ref,shown_count,expires_at)
             VALUES (?1,?2,?3,?4,'pending',?5,?6,0,?7)",
            params![
                event_id,
                message,
                severity.as_str(),
                format_time(created_at),
                source.kind.as_str(),
                source.reference,
                expires_at.map(|value| format_time(*value)),
            ],
        )
        .context("event_store_insert_failed")?;
    Ok(event_id)
}

fn emit_internal_completion(
    transaction: &Transaction<'_>,
    source: &EventSource,
    policy_json: &str,
    event_type: &str,
    message: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    let policy: InternalEventPolicy = serde_json::from_str(policy_json)
        .context("event_store_internal_policy_deserialize_failed")?;
    let Some(severity) = policy.severity_for(event_type) else {
        return Ok(());
    };
    ensure_pending_capacity(transaction)?;
    let expires_at = expiry_for(severity, policy.low_ttl_seconds, at)?;
    insert_event(
        transaction,
        message,
        severity,
        at,
        source,
        expires_at.as_ref(),
    )?;
    Ok(())
}

fn clear_internal_completion(transaction: &Transaction<'_>, source: &EventSource) -> Result<()> {
    transaction
        .execute(
            "UPDATE internal_sources SET completion_event_type=NULL,completion_message=NULL,
                policy_json=?1,notified=1 WHERE source_kind=?2 AND source_ref=?3",
            params![COMPACT_POLICY_JSON, source.kind.as_str(), source.reference],
        )
        .context("event_store_internal_completion_cleanup_failed")?;
    Ok(())
}

fn origin_from_fields(
    run_id: Option<String>,
    request_id: Option<String>,
    command_hash: Option<String>,
) -> Result<Option<EventOrigin>> {
    match (run_id, request_id, command_hash) {
        (None, None, None) => Ok(None),
        (Some(run_id), Some(request_id), Some(command_hash)) => Ok(Some(EventOrigin {
            run_id,
            request_id,
            command_hash,
        })),
        _ => Err(anyhow!("event_store_incomplete_origin")),
    }
}

fn validate_origin(origin: &EventOrigin) -> Result<()> {
    for component in [&origin.run_id, &origin.request_id, &origin.command_hash] {
        if component.is_empty()
            || component.len() > MAX_ORIGIN_COMPONENT_BYTES
            || component.chars().any(char::is_control)
        {
            return Err(anyhow!("event_origin_invalid"));
        }
    }
    Ok(())
}

fn validate_internal_source(source: &EventSource) -> Result<()> {
    validate_reference(&source.reference, "event_ref")?;
    if source.kind == EventSourceKind::External {
        return Err(anyhow!("event_internal_source_kind_invalid"));
    }
    Ok(())
}

fn validate_reference(reference: &str, label: &str) -> Result<()> {
    if reference.is_empty()
        || reference.len() > MAX_REFERENCE_BYTES
        || reference.chars().any(char::is_control)
    {
        return Err(anyhow!("{label}_invalid"));
    }
    Ok(())
}

fn validate_message(message: &str) -> Result<()> {
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(anyhow!("event_message_too_large"));
    }
    Ok(())
}

fn validate_event_type(event_type: &str) -> Result<()> {
    if event_type.is_empty()
        || event_type.len() > MAX_EVENT_TYPE_BYTES
        || event_type.chars().any(char::is_control)
    {
        return Err(anyhow!("event_type_invalid"));
    }
    Ok(())
}

fn validate_agent_scope(agent_id: &str) -> Result<()> {
    if agent_id.len() > MAX_AGENT_SCOPE_BYTES || agent_id.chars().any(char::is_control) {
        return Err(anyhow!("event_agent_scope_invalid"));
    }
    Ok(())
}

fn checked_ttl_duration(low_ttl_seconds: u64) -> Result<Duration> {
    let seconds =
        i64::try_from(low_ttl_seconds).map_err(|_| anyhow!("event_low_ttl_out_of_range"))?;
    if low_ttl_seconds > MAX_LOW_TTL_SECONDS {
        return Err(anyhow!("event_low_ttl_out_of_range"));
    }
    Duration::try_seconds(seconds).ok_or_else(|| anyhow!("event_low_ttl_out_of_range"))
}

fn checked_expiry_at(low_ttl_seconds: u64, created_at: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let duration = checked_ttl_duration(low_ttl_seconds)?;
    let expires_at = created_at
        .checked_add_signed(duration)
        .ok_or_else(|| anyhow!("event_low_ttl_out_of_range"))?;
    if !(0..=9_999).contains(&expires_at.year()) {
        return Err(anyhow!("event_low_ttl_out_of_range"));
    }
    Ok(expires_at)
}

fn expiry_for(
    severity: EventSeverity,
    low_ttl_seconds: u64,
    created_at: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    if severity != EventSeverity::Low {
        return Ok(None);
    }
    checked_expiry_at(low_ttl_seconds, created_at).map(Some)
}

fn summarize(message: &str) -> String {
    let mut chars = message.chars();
    let summary = chars.by_ref().take(32).collect::<String>();
    if chars.next().is_some() {
        summary.chars().take(31).collect::<String>() + "…"
    } else {
        summary
    }
}

fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| anyhow!("event_store_invalid_timestamp"))
}

fn parse_severity(value: &str) -> Result<EventSeverity> {
    match value {
        "low" => Ok(EventSeverity::Low),
        "medium" => Ok(EventSeverity::Medium),
        "high" => Ok(EventSeverity::High),
        _ => Err(anyhow!("event_store_invalid_severity")),
    }
}

fn parse_status(value: &str) -> Result<EventStatus> {
    match value {
        "pending" => Ok(EventStatus::Pending),
        "handled" => Ok(EventStatus::Handled),
        "expired" => Ok(EventStatus::Expired),
        _ => Err(anyhow!("event_store_invalid_status")),
    }
}

fn parse_source_kind(value: &str) -> Result<EventSourceKind> {
    match value {
        "process" => Ok(EventSourceKind::Process),
        "skill_install" => Ok(EventSourceKind::SkillInstall),
        "external" => Ok(EventSourceKind::External),
        _ => Err(anyhow!("event_store_invalid_source_kind")),
    }
}

fn decode_cursor(value: &str, request: &EventListRequest) -> Result<ListCursor> {
    if value.len() > MAX_CURSOR_BYTES {
        return Err(anyhow!("event_cursor_invalid"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| anyhow!("event_cursor_invalid"))?;
    let cursor: ListCursor =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("event_cursor_invalid"))?;
    if cursor.version != 1 {
        return Err(anyhow!("event_cursor_invalid"));
    }
    if cursor.agent_id != request.agent_id
        || cursor.status != request.status.unwrap_or(EventStatus::Pending)
        || cursor.severity != request.severity
    {
        return Err(anyhow!("event_cursor_scope_mismatch"));
    }
    if cursor.event_id.is_empty() || cursor.event_id.len() > MAX_REFERENCE_BYTES {
        return Err(anyhow!("event_cursor_invalid"));
    }
    parse_time(&cursor.created_at).map_err(|_| anyhow!("event_cursor_invalid"))?;
    Ok(cursor)
}

fn encode_cursor(request: &EventListRequest, record: &EventRecord) -> Result<String> {
    let cursor = ListCursor {
        version: 1,
        agent_id: request.agent_id.clone(),
        status: request.status.unwrap_or(EventStatus::Pending),
        severity: request.severity,
        created_at: format_time(record.created_at),
        event_id: record.event_id.clone(),
    };
    let bytes = serde_json::to_vec(&cursor).context("event_cursor_serialize_failed")?;
    let value = URL_SAFE_NO_PAD.encode(bytes);
    if value.len() > MAX_CURSOR_BYTES {
        return Err(anyhow!("event_cursor_too_large"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    use crate::private_state::PrivateStatePaths;

    fn store(name: &str) -> (Arc<EventStore>, PrivateStatePaths, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("agentic-events-{name}-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let paths = PrivateStatePaths::for_test(root.clone());
        (EventStore::open(&paths).unwrap(), paths, root)
    }

    fn request(
        reference: &str,
        message: &str,
        severity: Option<EventSeverity>,
    ) -> EventInjectRequest {
        EventInjectRequest {
            message: message.to_string(),
            severity,
            reference: reference.to_string(),
        }
    }

    fn policy(overrides: &[(&str, Option<EventSeverity>)]) -> InternalEventPolicy {
        InternalEventPolicy {
            low_ttl_seconds: 24 * 60 * 60,
            overrides: overrides
                .iter()
                .map(|(kind, severity)| ((*kind).to_string(), *severity))
                .collect(),
        }
    }

    fn source(kind: EventSourceKind, reference: &str) -> EventSource {
        EventSource {
            kind,
            reference: reference.to_string(),
        }
    }

    #[test]
    fn list_summaries_count_unicode_scalars_and_preserve_full_message_on_get() {
        let (store, _, root) = store("unicode-summary");
        let exactly_32 = "界".repeat(32);
        let longer = "界".repeat(33);
        let first = store
            .inject(
                &request("unicode-32", &exactly_32, Some(EventSeverity::High)),
                0,
            )
            .unwrap();
        let second = store
            .inject(
                &request("unicode-33", &longer, Some(EventSeverity::High)),
                0,
            )
            .unwrap();
        let list = store
            .list(&EventListRequest {
                severity: Some(EventSeverity::High),
                ..EventListRequest::default()
            })
            .unwrap();
        let exact = list
            .items
            .iter()
            .find(|item| item.event_id == first.event_id)
            .unwrap();
        let truncated = list
            .items
            .iter()
            .find(|item| item.event_id == second.event_id)
            .unwrap();
        assert_eq!(exact.summary, exactly_32);
        assert_eq!(truncated.summary, format!("{}…", "界".repeat(31)));
        assert_eq!(truncated.summary.chars().count(), 32);
        assert_eq!(store.get(&second.event_id).unwrap().message, longer);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn panel_orders_severity_caps_five_and_counts_hidden_pending_events() {
        let (store, _, root) = store("panel-order-cap");
        let now = Utc::now();
        for (reference, severity, seconds) in [
            ("low-old", EventSeverity::Low, 0),
            ("medium-new", EventSeverity::Medium, 3),
            ("high-new", EventSeverity::High, 5),
            ("medium-old", EventSeverity::Medium, 1),
            ("high-old", EventSeverity::High, 2),
            ("low-new", EventSeverity::Low, 4),
        ] {
            store
                .inject_at(
                    &request(reference, reference, Some(severity)),
                    24 * 60 * 60,
                    now + Duration::seconds(seconds),
                )
                .unwrap();
        }
        let panel = store.panel_at(now + Duration::seconds(10)).unwrap();
        assert_eq!(panel.new.len(), 5);
        let displayed = panel
            .new
            .iter()
            .map(|item| item.keys().next().unwrap().clone())
            .collect::<Vec<_>>();
        assert!(displayed[0].contains("high-old"));
        assert!(displayed[1].contains("high-new"));
        assert!(displayed[2].contains("medium-old"));
        assert!(displayed[3].contains("medium-new"));
        assert!(displayed[4].contains("low-old"));
        assert_eq!(panel.current, "low: 2 | medium: 2 | high: 2");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn low_one_medium_three_high_unlimited_exposure_and_hidden_counting() {
        let (store, _, root) = store("exposure-caps");
        let low = store
            .inject(&request("low", "low", None), 24 * 60 * 60)
            .unwrap();
        let medium = store
            .inject(&request("medium", "medium", Some(EventSeverity::Medium)), 0)
            .unwrap();
        let high = store
            .inject(&request("high", "high", Some(EventSeverity::High)), 0)
            .unwrap();

        let first = store.panel().unwrap();
        assert_eq!(first.new.len(), 3);
        let second = store.panel().unwrap();
        assert_eq!(second.new.len(), 2);
        let third = store.panel().unwrap();
        assert_eq!(third.new.len(), 2);
        let fourth = store.panel().unwrap();
        assert_eq!(fourth.new.len(), 1);
        assert_eq!(fourth.current, "low: 1 | medium: 1 | high: 1");
        assert_eq!(store.get(&low.event_id).unwrap().shown_count, 1);
        assert_eq!(store.get(&medium.event_id).unwrap().shown_count, 3);
        assert_eq!(store.get(&high.event_id).unwrap().shown_count, 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn expiry_boundary_and_seven_day_history_retention_are_applied_before_reads() {
        let (store, _, root) = store("retention-boundary");
        let now = Utc::now();
        let event = store
            .inject_at(&request("boundary", "boundary", None), 60, now)
            .unwrap();
        let boundary = now + Duration::seconds(60);
        let list = store
            .list_at(
                &EventListRequest {
                    status: Some(EventStatus::Expired),
                    ..EventListRequest::default()
                },
                boundary,
            )
            .unwrap();
        assert_eq!(
            list.items
                .iter()
                .find(|item| item.event_id == event.event_id)
                .unwrap()
                .status,
            EventStatus::Expired
        );
        assert_eq!(
            store.get_at(&event.event_id, boundary).unwrap().status,
            EventStatus::Expired
        );

        let old = store
            .inject_at(
                &request("old", "old", Some(EventSeverity::High)),
                0,
                now - Duration::days(8),
            )
            .unwrap();
        store
            .mark_at(std::slice::from_ref(&old.event_id), now - Duration::days(8))
            .unwrap();
        let retained = store
            .list_at(&EventListRequest::default(), now - Duration::days(1))
            .unwrap();
        assert!(!retained
            .items
            .iter()
            .any(|item| item.event_id == old.event_id));
        assert_eq!(
            store
                .get_at(&old.event_id, now - Duration::days(1))
                .unwrap_err()
                .to_string(),
            "event_not_found"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn marking_is_idempotent_and_unknown_ids_remain_not_found() {
        let (store, _, root) = store("mark-idempotent");
        let event = store
            .inject(&request("mark", "mark", Some(EventSeverity::High)), 0)
            .unwrap();
        let first = store.mark(std::slice::from_ref(&event.event_id)).unwrap();
        assert_eq!(first.handled_ids, vec![event.event_id.clone()]);
        let repeated = store.mark(std::slice::from_ref(&event.event_id)).unwrap();
        assert_eq!(repeated.handled_ids, vec![event.event_id]);
        assert!(repeated.not_found_ids.is_empty());
        let unknown = vec!["does-not-exist".to_string()];
        assert_eq!(store.mark(&unknown).unwrap().not_found_ids, unknown);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn list_cursor_is_stable_and_bound_to_agent_scope_and_filters() {
        let (store, _, root) = store("cursor-scope");
        for reference in ["first", "second", "third"] {
            store
                .inject(&request(reference, reference, Some(EventSeverity::High)), 0)
                .unwrap();
        }
        let request = EventListRequest {
            agent_id: "agent-a".to_string(),
            severity: Some(EventSeverity::High),
            limit: Some(1),
            ..EventListRequest::default()
        };
        let page1 = store.list(&request).unwrap();
        let cursor = page1.next_cursor.clone().unwrap();
        let page2 = store
            .list(&EventListRequest {
                cursor: Some(cursor.clone()),
                ..request.clone()
            })
            .unwrap();
        assert_ne!(page1.items[0].event_id, page2.items[0].event_id);
        assert!(store
            .list(&EventListRequest {
                agent_id: "agent-b".to_string(),
                cursor: Some(cursor.clone()),
                ..request.clone()
            })
            .is_err());
        assert!(store
            .list(&EventListRequest {
                severity: Some(EventSeverity::Medium),
                cursor: Some(cursor),
                ..request
            })
            .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn list_defaults_to_twenty_pending_and_can_filter_handled_history() {
        let (store, _, root) = store("list-default");
        let mut first_id = None;
        for index in 0..22 {
            let reference = format!("default-{index}");
            let record = store
                .inject(
                    &request(&reference, &reference, Some(EventSeverity::High)),
                    0,
                )
                .unwrap();
            if index == 0 {
                first_id = Some(record.event_id);
            }
        }
        store
            .mark(&[first_id.expect("first event was inserted")])
            .unwrap();

        let first_page = store.list(&EventListRequest::default()).unwrap();
        assert_eq!(first_page.items.len(), 20);
        assert!(first_page
            .items
            .iter()
            .all(|item| item.status == EventStatus::Pending));
        let next_cursor = first_page.next_cursor.clone().unwrap();
        let second_page = store
            .list(&EventListRequest {
                cursor: Some(next_cursor),
                ..EventListRequest::default()
            })
            .unwrap();
        assert_eq!(second_page.items.len(), 1);
        assert!(second_page.next_cursor.is_none());

        let handled = store
            .list(&EventListRequest {
                status: Some(EventStatus::Handled),
                ..EventListRequest::default()
            })
            .unwrap();
        assert_eq!(handled.items.len(), 1);
        assert_eq!(handled.items[0].status, EventStatus::Handled);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_panels_expose_a_low_event_exactly_once() {
        let (store, paths, root) = store("concurrent-panel");
        let event = store
            .inject(&request("once", "once", None), 24 * 60 * 60)
            .unwrap();
        let competitor = EventStore::open(&paths).unwrap();
        let mut workers = Vec::new();
        for index in 0..16 {
            let store = if index % 2 == 0 {
                Arc::clone(&store)
            } else {
                Arc::clone(&competitor)
            };
            workers.push(thread::spawn(move || store.panel().unwrap().new.len()));
        }
        let shown = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .sum::<usize>();
        assert_eq!(shown, 1);
        assert_eq!(store.get(&event.event_id).unwrap().shown_count, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reopen_restores_events_and_internal_response_arbitration() {
        let (store, paths, root) = store("reopen");
        let event = store
            .inject(
                &request("reopen-event", "persisted", Some(EventSeverity::High)),
                0,
            )
            .unwrap();
        let source = source(EventSourceKind::Process, "process-1");
        store.register_internal(&source, &policy(&[])).unwrap();
        store
            .record_internal_completion(&source, "process.completed", "finished", Utc::now())
            .unwrap();
        assert_eq!(
            store.pending_internal_sources().unwrap(),
            vec![source.clone()]
        );
        drop(store);
        let reopened = EventStore::open(&paths).unwrap();
        reopened.settle_response(&source, false).unwrap();
        assert_eq!(reopened.get(&event.event_id).unwrap().message, "persisted");
        let events = reopened
            .list(&EventListRequest {
                severity: Some(EventSeverity::Low),
                ..EventListRequest::default()
            })
            .unwrap();
        assert!(events.items.iter().any(|item| item.summary == "finished"));
        assert!(reopened.pending_internal_sources().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn completion_order_off_suppression_and_later_reads_preserve_original_settlement() {
        let (store, _, root) = store("completion-arbitration");
        let early = source(EventSourceKind::Process, "early");
        store.register_internal(&early, &policy(&[])).unwrap();
        store
            .record_internal_completion(&early, "process.completed", "early done", Utc::now())
            .unwrap();
        assert_eq!(
            store.pending_internal_sources().unwrap(),
            vec![early.clone()]
        );
        store.settle_response(&early, false).unwrap();
        let early_page = store.list(&EventListRequest::default()).unwrap();
        assert!(early_page
            .items
            .iter()
            .any(|item| item.summary == "early done"));
        let early_event = early_page
            .items
            .iter()
            .find(|item| item.summary == "early done")
            .unwrap()
            .event_id
            .clone();
        store.get(&early_event).unwrap();
        store.list(&EventListRequest::default()).unwrap();
        store.settle_response(&early, true).unwrap();
        assert_eq!(
            store.get(&early_event).unwrap().status,
            EventStatus::Pending
        );

        let late = source(EventSourceKind::SkillInstall, "late");
        store.register_internal(&late, &policy(&[])).unwrap();
        store.settle_response(&late, false).unwrap();
        store
            .record_internal_completion(&late, "skill_install.completed", "late done", Utc::now())
            .unwrap();
        let list = store.list(&EventListRequest::default()).unwrap();
        assert!(list.items.iter().any(|item| item.summary == "late done"));

        let suppressed = source(EventSourceKind::Process, "suppressed");
        store.register_internal(&suppressed, &policy(&[])).unwrap();
        store.settle_response(&suppressed, true).unwrap();
        store
            .record_internal_completion(&suppressed, "process.completed", "not emitted", Utc::now())
            .unwrap();
        let off = source(EventSourceKind::Process, "off");
        store
            .register_internal(&off, &policy(&[("process.completed", None)]))
            .unwrap();
        store.settle_response(&off, false).unwrap();
        store
            .record_internal_completion(&off, "process.completed", "off", Utc::now())
            .unwrap();
        let list = store.list(&EventListRequest::default()).unwrap();
        assert!(!list.items.iter().any(|item| item.summary == "not emitted"));
        assert!(!list.items.iter().any(|item| item.summary == "off"));
        assert_eq!(
            store.pending_internal_sources().unwrap(),
            Vec::<EventSource>::new()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn awaiting_completion_survives_recovery_and_source_cannot_be_forged() {
        let (store, _, root) = store("source-provenance");
        let internal = source(EventSourceKind::Process, "recover-me");
        store.register_internal(&internal, &policy(&[])).unwrap();
        store
            .record_internal_completion(
                &internal,
                "process.completed",
                "already finished",
                Utc::now(),
            )
            .unwrap();
        assert_eq!(
            store.pending_internal_sources().unwrap(),
            vec![internal.clone()]
        );
        store.settle_response(&internal, false).unwrap();
        let list = store.list(&EventListRequest::default()).unwrap();
        assert!(list
            .items
            .iter()
            .any(|item| item.summary == "already finished"));
        assert!(store
            .register_internal(&source(EventSourceKind::External, "spoof"), &policy(&[]))
            .is_err());

        let spoofed: serde_json::Result<EventInjectRequest> = serde_json::from_str(
            r#"{"message":"forged","ref":"x","source":{"kind":"process","ref":"p"}}"#,
        );
        assert!(spoofed.is_err());
        let injected = store
            .inject(&request("trusted-ref", "external", None), 60)
            .unwrap();
        assert_eq!(injected.source.kind, EventSourceKind::External);
        assert_eq!(injected.source.reference, "trusted-ref");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn low_ttl_out_of_range_returns_an_error_without_panicking() {
        let too_large = MAX_LOW_TTL_SECONDS + 1;
        assert_eq!(
            checked_ttl_duration(too_large).unwrap_err().to_string(),
            "event_low_ttl_out_of_range"
        );
        assert_eq!(
            expiry_for(EventSeverity::Low, too_large, Utc::now())
                .unwrap_err()
                .to_string(),
            "event_low_ttl_out_of_range"
        );
        let policy = InternalEventPolicy {
            low_ttl_seconds: too_large,
            ..InternalEventPolicy::default()
        };
        assert_eq!(
            policy.validate().unwrap_err().to_string(),
            "event_low_ttl_out_of_range"
        );
    }

    #[test]
    fn unrepresentable_rfc3339_ttl_is_rejected_before_event_insertion() {
        let (store, _, root) = store("rfc3339-ttl-bound");
        let invalid_ttl = 1_000_000_000_000;
        assert!(checked_ttl_duration(invalid_ttl).is_ok());
        assert_eq!(
            checked_expiry_at(invalid_ttl, Utc::now())
                .unwrap_err()
                .to_string(),
            "event_low_ttl_out_of_range"
        );
        let policy = InternalEventPolicy {
            low_ttl_seconds: invalid_ttl,
            ..InternalEventPolicy::default()
        };
        assert_eq!(
            policy.validate().unwrap_err().to_string(),
            "event_low_ttl_out_of_range"
        );
        assert_eq!(
            store
                .inject(&request("rfc3339-invalid", "too far", None), invalid_ttl)
                .unwrap_err()
                .to_string(),
            "event_low_ttl_out_of_range"
        );
        assert!(store
            .list(&EventListRequest::default())
            .unwrap()
            .items
            .is_empty());

        let valid = store
            .inject(
                &request("rfc3339-valid", "representable", None),
                100_000_000_000,
            )
            .unwrap();
        let expires_at = valid.expires_at.unwrap();
        assert!((0..=9_999).contains(&expires_at.year()));
        assert_eq!(
            store.get(&valid.event_id).unwrap().expires_at,
            Some(expires_at)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_v1_openers_migrate_the_schema_once() {
        let root = std::env::temp_dir().join(format!(
            "agentic-events-migration-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        let paths = PrivateStatePaths::for_test(root.clone());
        let legacy = Connection::open(root.join("events.sqlite3")).unwrap();
        legacy
            .execute_batch(
                "CREATE TABLE internal_sources (
                    source_kind TEXT NOT NULL,
                    source_ref TEXT NOT NULL,
                    response_state TEXT NOT NULL,
                    settled_at TEXT,
                    policy_json TEXT NOT NULL,
                    completion_seen INTEGER NOT NULL DEFAULT 0,
                    completion_event_type TEXT,
                    completion_message TEXT,
                    completed_at TEXT,
                    notified INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (source_kind, source_ref)
                );
                PRAGMA user_version=1;",
            )
            .unwrap();
        drop(legacy);

        let start = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let paths = paths.clone();
            let start = Arc::clone(&start);
            workers.push(thread::spawn(move || {
                start.wait();
                EventStore::open(&paths)
            }));
        }
        start.wait();
        let stores = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(stores.len(), 2);
        drop(stores);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remote_settlement_requires_matching_origin_and_stays_sticky() {
        let (store, paths, root) = store("remote-origin-settlement");
        let remote = source(EventSourceKind::Process, "remote-process");
        let remote_policy = policy(&[("process.completed", Some(EventSeverity::High))]);
        store.register_internal(&remote, &remote_policy).unwrap();
        let origin = EventOrigin {
            run_id: "run-1".to_string(),
            request_id: "request-1".to_string(),
            command_hash: "hash-1".to_string(),
        };
        store.bind_origin(&remote, &origin).unwrap();
        store.bind_origin(&remote, &origin).unwrap();
        assert_eq!(store.remote_origin(&remote).unwrap(), Some(origin.clone()));
        let conflicting_origin = EventOrigin {
            request_id: "other-request".to_string(),
            ..origin.clone()
        };
        assert!(store.bind_origin(&remote, &conflicting_origin).is_err());

        store
            .record_internal_completion(
                &remote,
                "process.completed",
                "terminal after Hub timeout",
                Utc::now(),
            )
            .unwrap();
        assert!(store.settle_response(&remote, false).is_err());
        assert!(store
            .settle_remote_response(&remote, &conflicting_origin, false)
            .is_err());
        let other_source = source(EventSourceKind::Process, "other-process");
        assert!(store
            .settle_remote_response(&other_source, &origin, false)
            .is_err());
        assert!(store
            .list(&EventListRequest::default())
            .unwrap()
            .items
            .is_empty());
        assert!(store.pending_internal_sources().unwrap().contains(&remote));

        store
            .settle_remote_response(&remote, &origin, false)
            .unwrap();
        store
            .settle_remote_response(&remote, &origin, false)
            .unwrap();
        let event = store
            .list(&EventListRequest::default())
            .unwrap()
            .items
            .remove(0);
        store.get(&event.event_id).unwrap();
        assert!(store
            .settle_remote_response(&remote, &origin, true)
            .is_err());
        assert_eq!(
            store.get(&event.event_id).unwrap().status,
            EventStatus::Pending
        );

        let returned_terminal = source(EventSourceKind::SkillInstall, "returned-terminal");
        store
            .register_internal(&returned_terminal, &policy(&[]))
            .unwrap();
        let terminal_origin = EventOrigin {
            run_id: "run-2".to_string(),
            request_id: "request-2".to_string(),
            command_hash: "hash-2".to_string(),
        };
        store
            .bind_origin(&returned_terminal, &terminal_origin)
            .unwrap();
        store
            .record_internal_completion(
                &returned_terminal,
                "skill_install.completed",
                "terminal returned by Hub",
                Utc::now(),
            )
            .unwrap();
        store
            .settle_remote_response(&returned_terminal, &terminal_origin, true)
            .unwrap();
        store
            .settle_remote_response(&returned_terminal, &terminal_origin, true)
            .unwrap();
        assert!(store
            .settle_remote_response(&returned_terminal, &terminal_origin, false)
            .is_err());
        assert_eq!(
            store
                .list(&EventListRequest::default())
                .unwrap()
                .items
                .len(),
            1
        );

        let awaiting = source(EventSourceKind::Process, "remote-awaiting");
        store.register_internal(&awaiting, &policy(&[])).unwrap();
        let awaiting_origin = EventOrigin {
            run_id: "run-3".to_string(),
            request_id: "request-3".to_string(),
            command_hash: "hash-3".to_string(),
        };
        store.bind_origin(&awaiting, &awaiting_origin).unwrap();
        assert!(store
            .pending_internal_sources()
            .unwrap()
            .contains(&awaiting));
        assert!(store.settle_response(&awaiting, false).is_err());
        assert_eq!(
            store.remote_origin(&awaiting).unwrap(),
            Some(awaiting_origin)
        );
        drop(store);
        let reopened = EventStore::open(&paths).unwrap();
        assert_eq!(
            reopened.remote_origin(&remote).unwrap(),
            Some(origin.clone())
        );
        assert_eq!(
            reopened
                .list(&EventListRequest::default())
                .unwrap()
                .items
                .len(),
            1
        );
        assert!(reopened
            .settle_remote_response(&remote, &origin, true)
            .is_err());
        assert!(reopened
            .pending_internal_sources()
            .unwrap()
            .contains(&awaiting));
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn event_database_rejects_a_different_configured_agent_identity() {
        let root =
            std::env::temp_dir().join(format!("agentic-events-owner-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let paths = PrivateStatePaths::for_test(root.clone());
        let store = EventStore::open(&paths).unwrap();
        let event = store
            .inject(
                &request("owner", "owned event", Some(EventSeverity::High)),
                0,
            )
            .unwrap();
        drop(store);

        let mut other_paths = paths.clone();
        other_paths.agent_id = "different-agent".to_string();
        let error = EventStore::open(&other_paths).err().unwrap();
        assert!(error.to_string().contains("event_store_owner_mismatch"));

        let reopened = EventStore::open(&paths).unwrap();
        assert_eq!(
            reopened.get(&event.event_id).unwrap().message,
            "owned event"
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn reopen_does_not_replace_existing_policy_snapshot_or_completion() {
        let (store, _, root) = store("immutable-internal-snapshot");
        let source = source(EventSourceKind::Process, "snapshot");
        store.register_internal(&source, &policy(&[])).unwrap();
        store
            .register_internal(
                &source,
                &InternalEventPolicy {
                    low_ttl_seconds: 0,
                    overrides: BTreeMap::from([(
                        "process.completed".to_string(),
                        Some(EventSeverity::High),
                    )]),
                },
            )
            .unwrap();
        store.settle_response(&source, false).unwrap();
        store
            .record_internal_completion(&source, "process.completed", "snapshot", Utc::now())
            .unwrap();
        let events = store.list(&EventListRequest::default()).unwrap();
        assert_eq!(events.items[0].severity, EventSeverity::Low);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn duplicate_completion_cannot_reset_or_resurrect_a_terminal_event() {
        let (store, _, root) = store("completion-tombstone");
        let source = source(EventSourceKind::SkillInstall, "install-1");
        let original_policy = policy(&[
            ("skill_install.completed", Some(EventSeverity::High)),
            ("skill_install.failed", Some(EventSeverity::Low)),
        ]);
        store.register_internal(&source, &original_policy).unwrap();
        store.settle_response(&source, false).unwrap();
        let completed_at = Utc::now();
        store
            .record_internal_completion(
                &source,
                "skill_install.completed",
                "original completion",
                completed_at,
            )
            .unwrap();
        let event = store
            .list(&EventListRequest::default())
            .unwrap()
            .items
            .remove(0);
        store.panel().unwrap();
        store.mark(std::slice::from_ref(&event.event_id)).unwrap();

        store
            .record_internal_completion(
                &source,
                "skill_install.failed",
                "replacement completion",
                completed_at + Duration::days(1),
            )
            .unwrap();
        let persisted = store.get(&event.event_id).unwrap();
        assert_eq!(persisted.message, "original completion");
        assert_eq!(persisted.status, EventStatus::Handled);
        assert_eq!(persisted.severity, EventSeverity::High);
        assert_eq!(persisted.shown_count, 1);

        let after_retention = Utc::now() + Duration::days(8);
        store
            .list_at(&EventListRequest::default(), after_retention)
            .unwrap();
        store
            .register_internal(
                &source,
                &policy(&[("skill_install.completed", Some(EventSeverity::Low))]),
            )
            .unwrap();
        store
            .record_internal_completion(
                &source,
                "skill_install.completed",
                "replayed after retention",
                after_retention,
            )
            .unwrap();
        assert!(store
            .list_at(&EventListRequest::default(), after_retention)
            .unwrap()
            .items
            .is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
