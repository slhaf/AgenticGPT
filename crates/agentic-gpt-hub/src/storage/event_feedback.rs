use agentic_gpt_protocol::{
    EventOrigin, EventResponseDisposition, EventSettleRequest, EventSource, EventSourceKind,
    HubCommand,
};
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use tracing::warn;

use crate::state::HubState;
use crate::utils::random_id;
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FeedbackKey {
    agent_id: String,
    run_id: String,
    request_id: String,
    command_hash: String,
}

impl FeedbackKey {
    fn new(agent_id: &str, origin: &EventOrigin) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            run_id: origin.run_id.clone(),
            request_id: origin.request_id.clone(),
            command_hash: origin.command_hash.clone(),
        }
    }

    fn origin(&self) -> EventOrigin {
        EventOrigin {
            run_id: self.run_id.clone(),
            request_id: self.request_id.clone(),
            command_hash: self.command_hash.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PendingRepair {
    no_terminal: bool,
    dispositions: Option<Vec<EventResponseDisposition>>,
    recovery_sources: Option<Vec<EventSource>>,
    conflict: bool,
}

impl PendingRepair {
    fn is_empty(&self) -> bool {
        !self.no_terminal
            && self.dispositions.is_none()
            && self.recovery_sources.is_none()
            && !self.conflict
    }
}

#[derive(Default)]
struct FeedbackCoordinatorState {
    pending: std::collections::HashMap<FeedbackKey, PendingRepair>,
    barriers: std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>,
}

/// Per-Hub live repair queue and per-Agent completion barriers.
///
/// This is owned by Hub Dispatch, so unrelated Hub instances and tests never
/// share response-owner state.
#[derive(Default)]
pub(crate) struct FeedbackCoordinator {
    state: std::sync::Mutex<FeedbackCoordinatorState>,
}

impl FeedbackCoordinator {
    fn with_state<T>(&self, f: impl FnOnce(&mut FeedbackCoordinatorState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut state)
    }

    fn agent_barrier(&self, agent_id: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        self.with_state(|state| {
            if let Some(barrier) = state.barriers.get(agent_id).and_then(|weak| weak.upgrade()) {
                return barrier;
            }
            let barrier = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            state
                .barriers
                .insert(agent_id.to_string(), std::sync::Arc::downgrade(&barrier));
            barrier
        })
    }

    fn queue_dispositions(
        &self,
        key: &FeedbackKey,
        dispositions: &[EventResponseDisposition],
    ) -> bool {
        self.with_state(|state| {
            let pending = state.pending.entry(key.clone()).or_default();
            match &pending.dispositions {
                Some(existing) if existing.as_slice() != dispositions => {
                    pending.conflict = true;
                    false
                }
                Some(_) => true,
                None => {
                    pending.dispositions = Some(dispositions.to_vec());
                    true
                }
            }
        })
    }

    fn queue_recovery_sources(&self, key: &FeedbackKey, sources: Vec<EventSource>) {
        self.with_state(|state| {
            let pending = state.pending.entry(key.clone()).or_default();
            let mut merged = pending.recovery_sources.take().unwrap_or_default();
            for source in sources {
                if !merged.contains(&source) {
                    merged.push(source);
                }
            }
            merged.sort_by(|left, right| {
                source_kind_order(&left.kind)
                    .cmp(&source_kind_order(&right.kind))
                    .then_with(|| left.reference.cmp(&right.reference))
            });
            pending.recovery_sources = Some(merged);
        });
    }

    fn queue_no_terminal(&self, key: &FeedbackKey) {
        self.with_state(|state| {
            state.pending.entry(key.clone()).or_default().no_terminal = true;
        });
    }

    fn pending_for_agent(&self, agent_id: &str) -> Vec<(FeedbackKey, PendingRepair)> {
        self.with_state(|state| {
            state
                .pending
                .iter()
                .filter(|(key, _)| key.agent_id == agent_id)
                .map(|(key, pending)| (key.clone(), pending.clone()))
                .collect()
        })
    }

    fn clear_persisted(&self, key: &FeedbackKey, completed: &PendingRepair) {
        self.with_state(|state| {
            let Some(pending) = state.pending.get_mut(key) else {
                return;
            };
            if completed.no_terminal {
                pending.no_terminal = false;
            }
            if completed.dispositions.is_some() && pending.dispositions == completed.dispositions {
                pending.dispositions = None;
            }
            if completed.recovery_sources.is_some()
                && pending.recovery_sources == completed.recovery_sources
            {
                pending.recovery_sources = None;
            }
            if pending.is_empty() {
                state.pending.remove(key);
            }
        });
    }

    fn clear_no_terminal(&self, key: &FeedbackKey) {
        self.with_state(|state| {
            let Some(pending) = state.pending.get_mut(key) else {
                return;
            };
            pending.no_terminal = false;
            if pending.is_empty() {
                state.pending.remove(key);
            }
        });
    }

    fn clear_all(&self, key: &FeedbackKey) {
        self.with_state(|state| {
            state.pending.remove(key);
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FeedbackDecision {
    Returned,
    NoTerminal,
}

/// Records an explicit final outcome if this response owner exits unresolved.
///
/// The guard is created only after the durable Awaiting intent exists. It does
/// no I/O or async work in Drop; the next explicit per-Agent drain persists the
/// repair. A committed decision must be confirmed before the caller disarms it.
pub(crate) struct ResponseOwnerGuard<'a> {
    coordinator: &'a FeedbackCoordinator,
    key: FeedbackKey,
    command_type: &'static str,
    armed: bool,
}

impl<'a> ResponseOwnerGuard<'a> {
    pub(crate) fn new(
        coordinator: &'a FeedbackCoordinator,
        agent_id: &str,
        origin: &EventOrigin,
        command_type: &'static str,
    ) -> Self {
        Self {
            coordinator,
            key: FeedbackKey::new(agent_id, origin),
            command_type,
            armed: true,
        }
    }

    pub(crate) fn set_sources(&mut self, sources: &[EventResponseDisposition]) -> Result<()> {
        validate_sources(self.command_type, sources)?;
        if !self.coordinator.queue_dispositions(&self.key, sources) {
            bail!("event_feedback_owner_metadata_conflict");
        }
        Ok(())
    }

    pub(crate) fn complete_returned(&mut self, decision: FeedbackDecision) -> Result<()> {
        if decision != FeedbackDecision::Returned {
            bail!("event_feedback_returned_decision_not_committed");
        }
        self.coordinator.clear_all(&self.key);
        self.armed = false;
        Ok(())
    }

    pub(crate) fn complete_no_terminal(&mut self, decision: FeedbackDecision) -> Result<()> {
        if decision != FeedbackDecision::NoTerminal {
            bail!("event_feedback_no_terminal_decision_not_committed");
        }
        self.coordinator.clear_no_terminal(&self.key);
        self.armed = false;
        Ok(())
    }
}

impl Drop for ResponseOwnerGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.coordinator.queue_no_terminal(&self.key);
        }
    }
}

const AWAITING: &str = "awaiting";
const RETURNED: &str = "returned";
const NO_TERMINAL: &str = "no_terminal";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingFeedback {
    pub(crate) origin: EventOrigin,
    pub(crate) request_id: String,
    pub(crate) payload: EventSettleRequest,
}

#[derive(Debug)]
struct StoredFeedback {
    agent_id: String,
    request_id: String,
    command_hash: String,
    command_type: String,
    decision: String,
    event_sources_json: Option<String>,
    sources_json: Option<String>,
    feedback_request_id: Option<String>,
    feedback_payload_json: Option<String>,
    acked_at: Option<String>,
}

/// Creates the response-decision/outbox table in the Hub database.
const EVENT_FEEDBACK_SCHEMA: &str = "
    create table if not exists event_response_feedback (
        run_id text primary key not null,
        request_id text not null,
        agent_id text not null,
        command_hash text not null,
        command_type text not null,
        decision text not null check (decision in ('awaiting', 'returned', 'no_terminal')),
        event_sources_json text,
        sources_json text,
        feedback_request_id text,
        feedback_payload_json text,
        acked_at text,
        created_at text not null,
        updated_at text not null
    );
    create index if not exists event_response_feedback_pending
        on event_response_feedback(agent_id, acked_at, created_at);
    create table if not exists event_response_feedback_delta (
        run_id text not null,
        source_kind text not null,
        source_ref text not null,
        request_id text not null unique,
        payload_json text not null,
        acked_at text,
        created_at text not null,
        primary key(run_id, source_kind, source_ref)
    );";

pub(crate) fn init_transaction(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute_batch(EVENT_FEEDBACK_SCHEMA)?;
    ensure_sources_column(tx)
}

/// Also initializes the feedback table when opening a Hub DB outside migration.
pub(crate) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(EVENT_FEEDBACK_SCHEMA)?;
    ensure_sources_column(conn)
}

fn ensure_sources_column(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare("pragma table_info(event_response_feedback)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    if !columns.iter().any(|column| column == "sources_json") {
        conn.execute(
            "alter table event_response_feedback add column sources_json text",
            [],
        )?;
    }
    Ok(())
}

/// Converts intents left unresolved by a previous Hub process into NoTerminal.
///
/// This is called once when serving starts, not from general schema init. Any
/// persisted response dispositions or recovery identities are materialized
/// into a durable EventSettle outbox payload before the transaction commits.
pub(crate) fn recover_after_restart(conn: &Connection) -> Result<()> {
    let transaction = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_string();
    transaction.execute(
        "update event_response_feedback
         set decision = ?1, updated_at = ?2
         where decision = ?3",
        params![NO_TERMINAL, now, AWAITING],
    )?;

    let run_ids = {
        let mut statement = transaction.prepare(
            "select run_id from event_response_feedback
             where decision != ?1
               and (event_sources_json is not null or sources_json is not null)
             order by created_at asc",
        )?;
        let rows = statement.query_map(params![AWAITING], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for run_id in run_ids {
        materialize_pending(&transaction, &run_id)?;
    }
    transaction.commit()?;
    Ok(())
}

const REPAIR_ORPHAN_FEEDBACK_SQL: &str = "
    insert into event_response_feedback(
        run_id, request_id, agent_id, command_hash, command_type,
        decision, created_at, updated_at
    )
    select run.run_id, run.request_id, run.agent_id, run.command_hash,
           run.command_type, ?1, run.created_at, ?2
    from agent_runs as run
    where run.agent_id = ?3
      and run.result_json is null
      and run.acked_at is null
      and run.status in ('created', 'dispatched', 'timeout_waiting_result')
      and run.command_type in (
           'process.exec', 'process.batch', 'mcp.callTool', 'mcp.batch',
           'skills.run', 'skills.install'
      )
      and (?4 is null or run.run_id = ?4)
      and (?5 is null or run.request_id = ?5)
      and (?6 is null or run.command_hash = ?6)
      and not exists (
           select 1 from event_response_feedback as feedback
           where feedback.run_id = run.run_id
      )";

fn insert_orphan_feedback(
    transaction: &rusqlite::Transaction<'_>,
    agent_id: &str,
    origin: Option<&EventOrigin>,
    now: &str,
) -> Result<()> {
    let run_id = origin.map(|origin| origin.run_id.as_str());
    let request_id = origin.map(|origin| origin.request_id.as_str());
    let command_hash = origin.map(|origin| origin.command_hash.as_str());
    transaction.execute(
        REPAIR_ORPHAN_FEEDBACK_SQL,
        params![NO_TERMINAL, now, agent_id, run_id, request_id, command_hash],
    )?;
    Ok(())
}

fn repair_orphan_for_origin(state: &HubState, agent_id: &str, origin: &EventOrigin) -> Result<()> {
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    insert_orphan_feedback(&transaction, agent_id, Some(origin), &now_string())?;
    transaction.commit()?;
    Ok(())
}

/// Repairs replayable creation runs whose Hub crash window preceded intent
/// persistence. Existing decisions are never changed by this recovery scan.
pub(crate) fn repair_orphan_runs(state: &HubState, agent_id: &str) -> Result<()> {
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now_string();
    insert_orphan_feedback(&transaction, agent_id, None, &now)?;
    let run_ids = {
        let mut statement = transaction.prepare(
            "select run_id from event_response_feedback
             where agent_id = ?1 and decision = ?2
               and (event_sources_json is not null or sources_json is not null)
             order by created_at asc",
        )?;
        let rows = statement.query_map(params![agent_id, NO_TERMINAL], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for run_id in run_ids {
        materialize_pending(&transaction, &run_id)?;
    }
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
/// Persists an intent before the Hub sends a creation command to an Agent.
pub(crate) fn prepare(state: &HubState, agent_id: &str, origin: &EventOrigin) -> Result<()> {
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    prepare_in_transaction(&transaction, agent_id, origin)?;
    transaction.commit()?;
    Ok(())
}

/// Inserts and validates a creation-run intent inside the caller's transaction.
pub(crate) fn prepare_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    agent_id: &str,
    origin: &EventOrigin,
) -> Result<()> {
    let command_type = validate_creation_run(transaction, agent_id, origin)?;
    let now = now_string();
    transaction.execute(
        "insert into event_response_feedback(
            run_id, request_id, agent_id, command_hash, command_type,
            decision, created_at, updated_at
         ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         on conflict(run_id) do nothing",
        params![
            origin.run_id,
            origin.request_id,
            agent_id,
            origin.command_hash,
            command_type,
            AWAITING,
            now
        ],
    )?;
    let stored = load_feedback(transaction, &origin.run_id)?
        .ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    ensure_identity(&stored, agent_id, origin)?;
    if stored.command_type != command_type {
        bail!("event_feedback_command_mismatch");
    }
    Ok(())
}

/// Stores only metadata from a validated Agent reply; it never decides whether
/// that reply was actually returned to the original Hub caller.
pub(crate) fn record_reply_metadata(
    state: &HubState,
    agent_id: &str,
    origin: &EventOrigin,
    sources: &[EventResponseDisposition],
) -> Result<()> {
    let result = record_reply_metadata_inner(state, agent_id, origin, sources);
    if let Err(error) = &result {
        if error.downcast_ref::<rusqlite::Error>().is_some() {
            state
                .dispatch
                .response_feedback
                .queue_dispositions(&FeedbackKey::new(agent_id, origin), sources);
        }
    }
    result
}

fn record_reply_metadata_inner(
    state: &HubState,
    agent_id: &str,
    origin: &EventOrigin,
    sources: &[EventResponseDisposition],
) -> Result<()> {
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let command_type = validate_creation_run(&transaction, agent_id, origin)?;
    validate_sources(command_type.as_str(), sources)?;

    let stored = load_feedback(&transaction, &origin.run_id)?
        .ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    ensure_identity(&stored, agent_id, origin)?;
    if stored.command_type != command_type {
        bail!("event_feedback_command_mismatch");
    }

    let identities = canonical_identities(command_type.as_str(), sources)?;
    let metadata_json = serde_json::to_string(sources)?;
    match stored.event_sources_json.as_deref() {
        Some(existing) if existing != metadata_json => {
            bail!("event_feedback_metadata_conflict");
        }
        Some(_) => {}
        None => {
            transaction.execute(
                "update event_response_feedback
                 set event_sources_json = ?1, updated_at = ?2
                 where run_id = ?3 and event_sources_json is null",
                params![metadata_json, now_string(), origin.run_id],
            )?;
        }
    }
    persist_complete_identity_sources(&transaction, &origin.run_id, &stored, &identities)?;
    materialize_pending(&transaction, &origin.run_id)?;
    transaction.commit()?;
    Ok(())
}

/// Persists only source identities learned from Agent recovery. It deliberately
/// does not infer or store any `includes_terminal` value.
pub(crate) fn record_recovery_sources(
    state: &HubState,
    agent_id: &str,
    origin: &EventOrigin,
    sources: &[EventSource],
) -> Result<()> {
    let result = (|| {
        repair_orphan_for_origin(state, agent_id, origin)?;
        record_recovery_sources_inner(state, agent_id, origin, sources)
    })();
    if let Err(error) = &result {
        if error.downcast_ref::<rusqlite::Error>().is_some() {
            if let Ok(sources) = canonical_recovery_sources(sources) {
                state
                    .dispatch
                    .response_feedback
                    .queue_recovery_sources(&FeedbackKey::new(agent_id, origin), sources);
            }
        }
    }
    result
}

fn record_recovery_sources_inner(
    state: &HubState,
    agent_id: &str,
    origin: &EventOrigin,
    sources: &[EventSource],
) -> Result<()> {
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let command_type = validate_creation_run(&transaction, agent_id, origin)?;
    let sources = canonical_sources(command_type.as_str(), sources)?;
    let stored = load_feedback(&transaction, &origin.run_id)?
        .ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    ensure_identity(&stored, agent_id, origin)?;
    if stored.command_type != command_type {
        bail!("event_feedback_command_mismatch");
    }
    if let Some(metadata_json) = stored.event_sources_json.as_deref() {
        let metadata: Vec<EventResponseDisposition> = serde_json::from_str(metadata_json)?;
        let identities = canonical_identities(command_type.as_str(), &metadata)?;
        if !identity_sources_are_subset(&sources, &identities) {
            bail!("event_feedback_recovery_source_conflict");
        }
        persist_complete_identity_sources(&transaction, &origin.run_id, &stored, &identities)?;
    } else if let Some(existing_json) = stored.sources_json.as_deref() {
        let existing: Vec<EventSource> = serde_json::from_str(existing_json)?;
        let mut merged = existing;
        for source in sources {
            if !merged.contains(&source) {
                merged.push(source);
            }
        }
        let merged = canonical_sources(command_type.as_str(), &merged)?;
        persist_complete_identity_sources(&transaction, &origin.run_id, &stored, &merged)?;
    } else {
        persist_complete_identity_sources(&transaction, &origin.run_id, &stored, &sources)?;
    }
    materialize_pending(&transaction, &origin.run_id)?;
    transaction.commit()?;
    Ok(())
}

/// Makes the original caller's final response the single sticky decision.
///
/// `Some(sources)` means the initial Hub waiter returned the value; those
/// dispositions must exactly match the previously validated reply metadata.
/// `None` means no terminal value was returned and forces every saved source
/// to `includes_terminal: false`. The return value is the durable decision,
/// including an already-final decision that this call did not change.
pub(crate) fn finalize_original(
    state: &HubState,
    agent_id: &str,
    origin: &EventOrigin,
    returned_sources: Option<&[EventResponseDisposition]>,
) -> Result<FeedbackDecision> {
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    validate_creation_run(&transaction, agent_id, origin)?;
    let stored = load_feedback(&transaction, &origin.run_id)?
        .ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    ensure_identity(&stored, agent_id, origin)?;

    let decision = match stored.decision.as_str() {
        AWAITING => {
            let decision = if let Some(returned_sources) = returned_sources {
                validate_returned_sources(&stored, returned_sources)?;
                RETURNED
            } else {
                NO_TERMINAL
            };
            transaction.execute(
                "update event_response_feedback
                 set decision = ?1, updated_at = ?2
                 where run_id = ?3 and decision = ?4",
                params![decision, now_string(), origin.run_id, AWAITING],
            )?;
            decision
        }
        RETURNED => {
            if let Some(returned_sources) = returned_sources {
                validate_returned_sources(&stored, returned_sources)?;
            }
            RETURNED
        }
        NO_TERMINAL => NO_TERMINAL,
        _ => bail!("event_feedback_invalid_decision"),
    };

    // A late waiter outcome must not rewrite the already-final decision or its
    // previously materialized payload.
    materialize_pending(&transaction, &origin.run_id)?;
    let confirmed = load_feedback(&transaction, &origin.run_id)?
        .ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    if confirmed.decision.as_str() != decision {
        bail!("event_feedback_decision_confirmation_failed");
    }
    transaction.commit()?;
    match decision {
        RETURNED => Ok(FeedbackDecision::Returned),
        NO_TERMINAL => Ok(FeedbackDecision::NoTerminal),
        _ => bail!("event_feedback_invalid_decision"),
    }
}

fn validate_returned_sources(
    stored: &StoredFeedback,
    returned_sources: &[EventResponseDisposition],
) -> Result<()> {
    validate_sources(&stored.command_type, returned_sources)?;
    let metadata_json = stored
        .event_sources_json
        .as_deref()
        .ok_or_else(|| anyhow!("event_feedback_metadata_missing"))?;
    let metadata: Vec<EventResponseDisposition> = serde_json::from_str(metadata_json)?;
    if metadata.as_slice() != returned_sources {
        bail!("event_feedback_returned_metadata_mismatch");
    }
    if let Some(sources_json) = stored.sources_json.as_deref() {
        let identities: Vec<EventSource> = serde_json::from_str(sources_json)?;
        if canonical_identities(&stored.command_type, returned_sources)? != identities {
            bail!("event_feedback_returned_source_identity_mismatch");
        }
    }
    Ok(())
}

/// Returns durable, unacknowledged feedback for one Agent only.
pub(crate) fn pending_for_agent(state: &HubState, agent_id: &str) -> Result<Vec<PendingFeedback>> {
    let conn = state.db.lock().unwrap();
    let mut pending = Vec::new();
    {
        let mut statement = conn.prepare(
            "select run_id, request_id, command_hash, feedback_request_id,
                    feedback_payload_json
             from event_response_feedback
             where agent_id = ?1 and decision != ?2 and acked_at is null
                   and feedback_request_id is not null
                   and feedback_payload_json is not null
             order by created_at asc",
        )?;
        let rows = statement.query_map(params![agent_id, AWAITING], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (run_id, original_request_id, command_hash, request_id, payload_json) = row?;
            let origin = EventOrigin {
                run_id,
                request_id: original_request_id,
                command_hash,
            };
            let payload: EventSettleRequest = serde_json::from_str(&payload_json)?;
            if payload.origin != origin {
                bail!("event_feedback_payload_identity_mismatch");
            }
            pending.push(PendingFeedback {
                origin,
                request_id,
                payload,
            });
        }
    }
    {
        let mut statement = conn.prepare(
            "select feedback.run_id, feedback.request_id, feedback.command_hash,
                    delta.request_id, delta.payload_json
             from event_response_feedback_delta as delta
             join event_response_feedback as feedback using (run_id)
             where feedback.agent_id = ?1 and feedback.decision != ?2
                   and delta.acked_at is null
             order by delta.created_at asc",
        )?;
        let rows = statement.query_map(params![agent_id, AWAITING], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (run_id, original_request_id, command_hash, request_id, payload_json) = row?;
            let origin = EventOrigin {
                run_id,
                request_id: original_request_id,
                command_hash,
            };
            let payload: EventSettleRequest = serde_json::from_str(&payload_json)?;
            if payload.origin != origin {
                bail!("event_feedback_payload_identity_mismatch");
            }
            pending.push(PendingFeedback {
                origin,
                request_id,
                payload,
            });
        }
    }
    Ok(pending)
}

/// Acknowledges only the exact stable request and payload that were delivered.
pub(crate) fn ack(
    state: &HubState,
    agent_id: &str,
    origin: &EventOrigin,
    request_id: &str,
    payload: &EventSettleRequest,
) -> Result<()> {
    let mut conn = state.db.lock().unwrap();
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let stored = load_feedback(&transaction, &origin.run_id)?
        .ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    ensure_identity(&stored, agent_id, origin)?;
    let payload_json = serde_json::to_string(payload)?;
    let matches_primary = stored.feedback_request_id.as_deref() == Some(request_id)
        && stored.feedback_payload_json.as_deref() == Some(payload_json.as_str());
    if stored.decision == AWAITING || payload.origin != *origin {
        bail!("event_feedback_ack_mismatch");
    }
    if matches_primary {
        if stored.acked_at.is_none() {
            transaction.execute(
                "update event_response_feedback
                 set acked_at = ?1, updated_at = ?1
                 where run_id = ?2 and agent_id = ?3 and acked_at is null",
                params![now_string(), origin.run_id, agent_id],
            )?;
        }
    } else {
        let changed = transaction.execute(
            "update event_response_feedback_delta
             set acked_at = coalesce(acked_at, ?1)
             where run_id = ?2 and request_id = ?3 and payload_json = ?4
                   and exists (
                       select 1 from event_response_feedback
                       where run_id = ?2 and agent_id = ?5
                         and request_id = ?6 and command_hash = ?7
                   )",
            params![
                now_string(),
                origin.run_id,
                request_id,
                payload_json,
                agent_id,
                origin.request_id,
                origin.command_hash
            ],
        )?;
        if changed != 1 {
            bail!("event_feedback_ack_mismatch");
        }
    }
    transaction.commit()?;
    Ok(())
}

/// Persists the in-memory owner/recovery queue before delivery.
fn persist_queued_repairs(state: &HubState, agent_id: &str) -> Result<()> {
    let coordinator = &state.dispatch.response_feedback;
    for (key, pending) in coordinator.pending_for_agent(agent_id) {
        if pending.conflict {
            bail!("event_feedback_live_repair_conflict");
        }
        let origin = key.origin();
        if let Some(sources) = pending.recovery_sources.as_deref() {
            record_recovery_sources(state, agent_id, &origin, sources)?;
        }
        if let Some(dispositions) = pending.dispositions.as_deref() {
            record_reply_metadata(state, agent_id, &origin, dispositions)?;
        }
        if pending.no_terminal {
            match finalize_original(state, agent_id, &origin, None)? {
                FeedbackDecision::NoTerminal => {}
                FeedbackDecision::Returned => {
                    bail!("event_feedback_no_terminal_conflicts_with_returned");
                }
            }
        }
        coordinator.clear_persisted(&key, &pending);
    }
    Ok(())
}

/// Attempts the synchronous public-command readiness path without waiting.
///
/// The caller keeps this guard through its admission and command enqueue. A
/// returned guard proves that no active drain, durable delivery, or queued
/// owner repair remains for this Agent. `None` means the caller must release
/// its admission locks, await `flush_for_agent`, then revalidate its target.
pub(crate) fn try_public_preflight(
    state: &HubState,
    agent_id: &str,
) -> Result<Option<tokio::sync::OwnedMutexGuard<()>>> {
    let coordinator = &state.dispatch.response_feedback;
    let barrier = coordinator.agent_barrier(agent_id);
    let Ok(guard) = barrier.try_lock_owned() else {
        return Ok(None);
    };
    persist_queued_repairs(state, agent_id)?;
    if pending_for_agent(state, agent_id)?.is_empty()
        && coordinator.pending_for_agent(agent_id).is_empty()
    {
        Ok(Some(guard))
    } else {
        Ok(None)
    }
}

/// Waits for any active per-Agent flush, retries live repairs, and sends
/// durable EventSettle entries before allowing a public request to continue.
pub(crate) fn flush_for_agent<'a>(
    state: &'a HubState,
    agent_id: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let coordinator = &state.dispatch.response_feedback;
        let barrier = coordinator.agent_barrier(agent_id);
        let _flush = barrier.lock().await;
        loop {
            persist_queued_repairs(state, agent_id)?;
            let pending = pending_for_agent(state, agent_id)?;
            if pending.is_empty() {
                if coordinator.pending_for_agent(agent_id).is_empty() {
                    return Ok(());
                }
                continue;
            }
            for feedback in pending {
                let command = HubCommand::EventSettle {
                    request_id: feedback.request_id.clone(),
                    payload: feedback.payload.clone(),
                };
                match crate::agents::dispatch::request_agent(
                    state,
                    agent_id,
                    command,
                    crate::REQUEST_TIMEOUT_SECS,
                )
                .await
                {
                    Ok(response)
                        if response.get("status").and_then(serde_json::Value::as_str)
                            == Some("settled") =>
                    {
                        ack(
                            state,
                            agent_id,
                            &feedback.origin,
                            &feedback.request_id,
                            &feedback.payload,
                        )?;
                    }
                    Ok(response) => {
                        warn!(%agent_id, runId = %feedback.origin.run_id, ?response, "Agent did not confirm event feedback");
                        bail!("event_feedback_delivery_not_confirmed");
                    }
                    Err(error) => {
                        warn!(%agent_id, runId = %feedback.origin.run_id, %error, "event feedback delivery failed");
                        return Err(anyhow!(error));
                    }
                }
            }
        }
    })
}

/// Returns whether a Hub command can create event sources in its initial reply.
pub(crate) fn is_creation_command(command: &HubCommand) -> bool {
    is_creation_command_type(crate::runs::command_type(command))
}

fn validate_creation_run(
    conn: &Connection,
    agent_id: &str,
    origin: &EventOrigin,
) -> Result<String> {
    let command_type = validate_run_identity(conn, agent_id, origin)?;
    if !is_creation_command_type(&command_type) {
        bail!("event_feedback_non_creation_run");
    }
    Ok(command_type)
}

fn validate_run_identity(
    conn: &Connection,
    agent_id: &str,
    origin: &EventOrigin,
) -> Result<String> {
    let run = conn
        .query_row(
            "select agent_id, request_id, command_hash, command_type
             from agent_runs where run_id = ?1",
            params![origin.run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("event_feedback_run_missing"))?;
    if run.0 != agent_id || run.1 != origin.request_id || run.2 != origin.command_hash {
        bail!("event_feedback_origin_mismatch");
    }
    Ok(run.3)
}

pub(crate) fn is_creation_command_type(command_type: &str) -> bool {
    matches!(
        command_type,
        "process.exec"
            | "process.batch"
            | "mcp.callTool"
            | "mcp.batch"
            | "skills.run"
            | "skills.install"
    )
}

fn validate_sources(command_type: &str, sources: &[EventResponseDisposition]) -> Result<()> {
    canonical_identities(command_type, sources).map(|_| ())
}

fn canonical_identities(
    command_type: &str,
    dispositions: &[EventResponseDisposition],
) -> Result<Vec<EventSource>> {
    let sources = dispositions
        .iter()
        .map(|disposition| disposition.source.clone())
        .collect::<Vec<_>>();
    canonical_sources(command_type, &sources)
}

fn canonical_sources(command_type: &str, sources: &[EventSource]) -> Result<Vec<EventSource>> {
    validate_source_identities(command_type, sources)?;
    let mut sources = sources.to_vec();
    sources.sort_by(|left, right| {
        source_kind_order(&left.kind)
            .cmp(&source_kind_order(&right.kind))
            .then_with(|| left.reference.cmp(&right.reference))
    });
    Ok(sources)
}

fn canonical_recovery_sources(sources: &[EventSource]) -> Result<Vec<EventSource>> {
    for (index, source) in sources.iter().enumerate() {
        if source.reference.is_empty() || source.kind == EventSourceKind::External {
            bail!("event_feedback_invalid_source");
        }
        if sources[..index].iter().any(|previous| previous == source) {
            bail!("event_feedback_duplicate_source");
        }
    }
    let mut sources = sources.to_vec();
    sources.sort_by(|left, right| {
        source_kind_order(&left.kind)
            .cmp(&source_kind_order(&right.kind))
            .then_with(|| left.reference.cmp(&right.reference))
    });
    Ok(sources)
}

fn source_kind_order(kind: &EventSourceKind) -> u8 {
    match kind {
        EventSourceKind::Process => 0,
        EventSourceKind::SkillInstall => 1,
        EventSourceKind::External => 2,
    }
}

fn validate_source_identities(command_type: &str, sources: &[EventSource]) -> Result<()> {
    for (index, source) in sources.iter().enumerate() {
        if source.reference.is_empty() || source.kind == EventSourceKind::External {
            bail!("event_feedback_invalid_source");
        }
        let kind_matches = match command_type {
            "skills.install" => source.kind == EventSourceKind::SkillInstall,
            "process.exec" | "process.batch" | "skills.run" => {
                source.kind == EventSourceKind::Process
            }
            // MCP tools may start a process or create a skill-install job.
            "mcp.callTool" | "mcp.batch" => matches!(
                source.kind,
                EventSourceKind::Process | EventSourceKind::SkillInstall
            ),
            _ => false,
        };
        if !kind_matches {
            bail!("event_feedback_source_command_mismatch");
        }
        if sources[..index].iter().any(|previous| previous == source) {
            bail!("event_feedback_duplicate_source");
        }
    }
    Ok(())
}

fn identity_sources_are_subset(subset: &[EventSource], superset: &[EventSource]) -> bool {
    subset
        .iter()
        .all(|source| superset.iter().any(|known| known == source))
}

fn persist_complete_identity_sources(
    conn: &Connection,
    run_id: &str,
    stored: &StoredFeedback,
    identities: &[EventSource],
) -> Result<()> {
    let update = match stored.sources_json.as_deref() {
        Some(existing_json) => {
            let existing: Vec<EventSource> = serde_json::from_str(existing_json)?;
            let existing = canonical_sources(&stored.command_type, &existing)?;
            if existing == identities {
                false
            } else if identity_sources_are_subset(&existing, identities) {
                true
            } else {
                bail!("event_feedback_recovery_source_conflict");
            }
        }
        None => true,
    };
    if update {
        let sources_json = serde_json::to_string(identities)?;
        conn.execute(
            "update event_response_feedback
             set sources_json = ?1, updated_at = ?2
             where run_id = ?3",
            params![sources_json, now_string(), run_id],
        )?;
    }
    Ok(())
}

fn load_feedback(conn: &Connection, run_id: &str) -> Result<Option<StoredFeedback>> {
    Ok(conn
        .query_row(
            "select agent_id, request_id, command_hash, command_type, decision,
                    event_sources_json, sources_json, feedback_request_id,
                    feedback_payload_json, acked_at
             from event_response_feedback where run_id = ?1",
            params![run_id],
            |row| {
                Ok(StoredFeedback {
                    agent_id: row.get(0)?,
                    request_id: row.get(1)?,
                    command_hash: row.get(2)?,
                    command_type: row.get(3)?,
                    decision: row.get(4)?,
                    event_sources_json: row.get(5)?,
                    sources_json: row.get(6)?,
                    feedback_request_id: row.get(7)?,
                    feedback_payload_json: row.get(8)?,
                    acked_at: row.get(9)?,
                })
            },
        )
        .optional()?)
}

fn ensure_identity(stored: &StoredFeedback, agent_id: &str, origin: &EventOrigin) -> Result<()> {
    if stored.agent_id != agent_id
        || stored.request_id != origin.request_id
        || stored.command_hash != origin.command_hash
    {
        bail!("event_feedback_origin_mismatch");
    }
    Ok(())
}

fn materialize_pending(conn: &Connection, run_id: &str) -> Result<()> {
    let stored =
        load_feedback(conn, run_id)?.ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    if stored.decision == AWAITING {
        return Ok(());
    }
    if stored.decision == NO_TERMINAL && stored.feedback_payload_json.is_some() {
        validate_materialized_no_terminal(&stored, run_id)?;
        return materialize_delta_coverage(conn, &stored, run_id);
    }

    let dispositions = match stored.decision.as_str() {
        RETURNED => {
            let Some(metadata_json) = stored.event_sources_json.as_deref() else {
                return Ok(());
            };
            serde_json::from_str::<Vec<EventResponseDisposition>>(metadata_json)?
        }
        NO_TERMINAL => {
            let identities = if let Some(sources_json) = stored.sources_json.as_deref() {
                let sources: Vec<EventSource> = serde_json::from_str(sources_json)?;
                canonical_sources(&stored.command_type, &sources)?
            } else if let Some(metadata_json) = stored.event_sources_json.as_deref() {
                let metadata: Vec<EventResponseDisposition> = serde_json::from_str(metadata_json)?;
                canonical_identities(&stored.command_type, &metadata)?
            } else {
                return Ok(());
            };
            if let Some(metadata_json) = stored.event_sources_json.as_deref() {
                let metadata: Vec<EventResponseDisposition> = serde_json::from_str(metadata_json)?;
                if canonical_identities(&stored.command_type, &metadata)? != identities {
                    bail!("event_feedback_recovery_source_conflict");
                }
            }
            identities
                .into_iter()
                .map(|source| EventResponseDisposition {
                    source,
                    includes_terminal: false,
                })
                .collect()
        }
        _ => bail!("event_feedback_invalid_decision"),
    };
    if dispositions.is_empty() {
        return Ok(());
    }
    let origin = EventOrigin {
        run_id: run_id.to_string(),
        request_id: stored.request_id.clone(),
        command_hash: stored.command_hash.clone(),
    };
    let payload = EventSettleRequest {
        origin,
        dispositions,
    };
    let payload_json = serde_json::to_string(&payload)?;

    match (
        stored.feedback_request_id.as_deref(),
        stored.feedback_payload_json.as_deref(),
    ) {
        (None, None) => {
            let request_id = random_id("req");
            conn.execute(
                "update event_response_feedback
                 set feedback_request_id = ?1, feedback_payload_json = ?2, updated_at = ?3
                 where run_id = ?4 and feedback_request_id is null
                       and feedback_payload_json is null",
                params![request_id, payload_json, now_string(), run_id],
            )?;
            let updated = load_feedback(conn, run_id)?
                .ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
            if updated.feedback_request_id.as_deref() != Some(request_id.as_str())
                || updated.feedback_payload_json.as_deref() != Some(payload_json.as_str())
            {
                bail!("event_feedback_payload_conflict");
            }
        }
        (Some(_), Some(existing_payload)) if existing_payload == payload_json => {}
        _ => bail!("event_feedback_payload_conflict"),
    }
    let updated =
        load_feedback(conn, run_id)?.ok_or_else(|| anyhow!("event_feedback_intent_missing"))?;
    if updated.decision == NO_TERMINAL {
        materialize_delta_coverage(conn, &updated, run_id)?;
    }
    Ok(())
}

fn materialize_delta_coverage(
    conn: &Connection,
    stored: &StoredFeedback,
    run_id: &str,
) -> Result<()> {
    let known = if let Some(sources_json) = stored.sources_json.as_deref() {
        let sources: Vec<EventSource> = serde_json::from_str(sources_json)?;
        canonical_sources(&stored.command_type, &sources)?
    } else if let Some(metadata_json) = stored.event_sources_json.as_deref() {
        let metadata: Vec<EventResponseDisposition> = serde_json::from_str(metadata_json)?;
        canonical_identities(&stored.command_type, &metadata)?
    } else {
        return Ok(());
    };
    let primary_json = stored
        .feedback_payload_json
        .as_deref()
        .ok_or_else(|| anyhow!("event_feedback_payload_missing"))?;
    let primary: EventSettleRequest = serde_json::from_str(primary_json)?;
    let primary_sources = canonical_identities(&stored.command_type, &primary.dispositions)?;
    if !identity_sources_are_subset(&primary_sources, &known)
        || primary
            .dispositions
            .iter()
            .any(|disposition| disposition.includes_terminal)
    {
        bail!("event_feedback_payload_conflict");
    }
    let mut covered = primary_sources;
    let mut statement = conn.prepare(
        "select source_kind, source_ref, payload_json
         from event_response_feedback_delta where run_id = ?1",
    )?;
    let rows = statement.query_map(params![run_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let payloads = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for (source_kind, source_ref, payload_json) in payloads {
        let payload: EventSettleRequest = serde_json::from_str(&payload_json)?;
        if payload.origin != primary.origin
            || payload.dispositions.len() != 1
            || payload.dispositions[0].includes_terminal
        {
            bail!("event_feedback_delta_payload_conflict");
        }
        let source = &payload.dispositions[0].source;
        if serde_json::to_string(&source.kind)? != source_kind
            || source.reference != source_ref
            || !identity_sources_are_subset(std::slice::from_ref(source), &known)
        {
            bail!("event_feedback_delta_source_conflict");
        }
        if !covered.contains(source) {
            covered.push(source.clone());
        }
    }
    for source in known {
        if covered.contains(&source) {
            continue;
        }
        let payload = EventSettleRequest {
            origin: primary.origin.clone(),
            dispositions: vec![EventResponseDisposition {
                source: source.clone(),
                includes_terminal: false,
            }],
        };
        let payload_json = serde_json::to_string(&payload)?;
        let request_id = random_id("req");
        let source_kind = serde_json::to_string(&source.kind)?;
        conn.execute(
            "insert or ignore into event_response_feedback_delta(
                 run_id, source_kind, source_ref, request_id, payload_json, created_at
             ) values (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                run_id,
                source_kind,
                source.reference,
                request_id,
                payload_json,
                now_string()
            ],
        )?;
        let saved_payload: String = conn.query_row(
            "select payload_json from event_response_feedback_delta
             where run_id = ?1 and source_kind = ?2 and source_ref = ?3",
            params![run_id, source_kind, source.reference],
            |row| row.get(0),
        )?;
        if saved_payload != payload_json {
            bail!("event_feedback_delta_payload_conflict");
        }
        covered.push(source);
    }
    Ok(())
}

fn validate_materialized_no_terminal(stored: &StoredFeedback, run_id: &str) -> Result<()> {
    let request_id = stored
        .feedback_request_id
        .as_deref()
        .ok_or_else(|| anyhow!("event_feedback_payload_missing_request_id"))?;
    let payload_json = stored
        .feedback_payload_json
        .as_deref()
        .ok_or_else(|| anyhow!("event_feedback_payload_missing"))?;
    let payload: EventSettleRequest = serde_json::from_str(payload_json)?;
    let origin = EventOrigin {
        run_id: run_id.to_string(),
        request_id: stored.request_id.clone(),
        command_hash: stored.command_hash.clone(),
    };
    if request_id.is_empty()
        || payload.origin != origin
        || payload
            .dispositions
            .iter()
            .any(|disposition| disposition.includes_terminal)
    {
        bail!("event_feedback_payload_conflict");
    }
    let payload_identities = canonical_identities(&stored.command_type, &payload.dispositions)?;
    let stored_identities = stored
        .sources_json
        .as_deref()
        .map(|sources_json| {
            let sources: Vec<EventSource> = serde_json::from_str(sources_json)?;
            canonical_sources(&stored.command_type, &sources)
        })
        .transpose()?;
    let response_identities = stored
        .event_sources_json
        .as_deref()
        .map(|metadata_json| {
            let metadata: Vec<EventResponseDisposition> = serde_json::from_str(metadata_json)?;
            canonical_identities(&stored.command_type, &metadata)
        })
        .transpose()?;
    if stored_identities
        .as_ref()
        .zip(response_identities.as_ref())
        .is_some_and(|(stored, response)| stored != response)
    {
        bail!("event_feedback_recovery_source_conflict");
    }
    let known_identities = stored_identities
        .or(response_identities)
        .unwrap_or_else(|| payload_identities.clone());
    if !identity_sources_are_subset(&payload_identities, &known_identities) {
        bail!("event_feedback_payload_conflict");
    }
    Ok(())
}

fn now_string() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteConfirmationConfig;
    use crate::state::McpProfile;
    use crate::state::{AgentTransport, OutboundAgentMessage};
    use crate::{HubConfig, NtfyConfig};
    use agentic_gpt_protocol::{
        AgentConnectionMode, AgentMessage, AgentRole, HubCommand, HubCommandEnvelope,
        ProcessExecRequest,
    };
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use rusqlite::Connection;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Barrier, Mutex as StdMutex};
    use tokio::sync::mpsc;
    use tokio::sync::Mutex;
    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        init(&conn).unwrap();
        state_with_connection(conn)
    }

    fn state_with_connection(conn: Connection) -> HubState {
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(HubConfig {
                remote_confirmation: RemoteConfirmationConfig {
                    enabled: false,
                    provider: "none".to_string(),
                    timeout_seconds: 45,
                    ntfy: NtfyConfig {
                        server_url: String::new(),
                        topic: String::new(),
                        callback_base_url: String::new(),
                    },
                },
            }),
            mcp_profile: McpProfile::Full,
            agents: Arc::new(crate::agents::lifecycle::Connections::new()),
            dispatch: Arc::new(crate::agents::dispatch::Dispatch::new()),
            confirmations: Arc::new(crate::confirmation::Confirmations::new()),
            process_cache: Arc::new(crate::state::ProcessCache::new()),
            boot_generations: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: None,
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(None)),
        }
    }

    fn fixture(command_type: &str, label: &str) -> (HubState, EventOrigin) {
        let state = test_state();
        let origin = EventOrigin {
            run_id: format!("run_{label}"),
            request_id: format!("request_{label}"),
            command_hash: format!("hash_{label}"),
        };
        {
            let conn = state.db.lock().unwrap();
            let now = now_string();
            conn.execute(
                "insert into agent_runs(
                    run_id, request_id, agent_id, command_type, command_json,
                    command_hash, status, created_at, updated_at
                 ) values (?1, ?2, 'agent', ?3, '{}', ?4, 'dispatched', ?5, ?5)",
                params![
                    origin.run_id,
                    origin.request_id,
                    command_type,
                    origin.command_hash,
                    now
                ],
            )
            .unwrap();
        }
        prepare(&state, "agent", &origin).unwrap();
        (state, origin)
    }

    fn disposition(reference: &str, includes_terminal: bool) -> EventResponseDisposition {
        EventResponseDisposition {
            source: agentic_gpt_protocol::EventSource {
                kind: EventSourceKind::Process,
                reference: reference.to_string(),
            },
            includes_terminal,
        }
    }

    fn source(reference: &str) -> EventSource {
        EventSource {
            kind: EventSourceKind::Process,
            reference: reference.to_string(),
        }
    }

    async fn answer_agent_request(
        state: &HubState,
        envelope: HubCommandEnvelope,
        result: serde_json::Value,
    ) {
        let mut headers = HeaderMap::new();
        headers.insert("x-agent-secret", HeaderValue::from_static("secret"));
        let response = crate::agents::transport::post_agent_message(
            State(state.clone()),
            Path("agent".to_string()),
            Query(crate::agents::transport::SseConnectQuery::for_test(Some(
                "feedback-test-connection".to_string(),
            ))),
            headers,
            axum::Json(AgentMessage::Response {
                run_id: Some(envelope.run_id),
                request_id: envelope.request_id,
                data: result,
                event_sources: Vec::new(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    fn stored_decision(state: &HubState, origin: &EventOrigin) -> String {
        state
            .db
            .lock()
            .unwrap()
            .query_row(
                "select decision from event_response_feedback where run_id = ?1",
                params![origin.run_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn returned_terminal_result_forwards_suppression_disposition() {
        let (state, origin) = fixture("process.exec", "returned_terminal");
        let sources = vec![disposition("process-1", true)];
        record_reply_metadata(&state, "agent", &origin, &sources).unwrap();
        finalize_original(&state, "agent", &origin, Some(&sources)).unwrap();

        assert_eq!(stored_decision(&state, &origin), RETURNED);
        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].payload.dispositions, sources);
        assert!(pending[0].payload.dispositions[0].includes_terminal);
    }

    #[test]
    fn returned_active_result_has_pending_feedback() {
        let (state, origin) = fixture("process.exec", "returned_active");
        let sources = vec![disposition("process-2", false)];
        record_reply_metadata(&state, "agent", &origin, &sources).unwrap();
        finalize_original(&state, "agent", &origin, Some(&sources)).unwrap();

        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].payload.origin, origin);
        assert_eq!(pending[0].payload.dispositions, sources);
    }

    #[test]
    fn timeout_before_late_metadata_forces_no_terminal() {
        let (state, origin) = fixture("process.exec", "late_metadata");
        finalize_original(&state, "agent", &origin, None).unwrap();
        record_reply_metadata(
            &state,
            "agent",
            &origin,
            &[disposition("process-late", true)],
        )
        .unwrap();

        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(stored_decision(&state, &origin), NO_TERMINAL);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].payload.dispositions.len(), 1);
        assert!(!pending[0].payload.dispositions[0].includes_terminal);
    }

    #[test]
    fn concurrent_waiter_outcomes_make_one_sticky_decision() {
        let (state, origin) = fixture("process.exec", "concurrent_decision");
        let sources = vec![disposition("process-race", true)];
        record_reply_metadata(&state, "agent", &origin, &sources).unwrap();
        let state = Arc::new(state);
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for returned_sources in [Some(sources.clone()), None] {
            let state = Arc::clone(&state);
            let origin = origin.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                finalize_original(&state, "agent", &origin, returned_sources.as_deref()).unwrap();
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }

        let before_decision = stored_decision(&state, &origin);
        assert!(before_decision == RETURNED || before_decision == NO_TERMINAL);
        let before_pending = pending_for_agent(&state, "agent").unwrap();
        let opposite = if before_decision == NO_TERMINAL {
            Some(sources.as_slice())
        } else {
            None
        };
        finalize_original(&state, "agent", &origin, opposite).unwrap();
        assert_eq!(stored_decision(&state, &origin), before_decision);
        assert_eq!(pending_for_agent(&state, "agent").unwrap(), before_pending);
    }

    #[test]
    fn migration_adds_identity_column_to_existing_feedback_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table event_response_feedback (
                run_id text primary key not null,
                request_id text not null,
                agent_id text not null,
                command_hash text not null,
                command_type text not null,
                decision text not null check (decision in ('awaiting', 'returned', 'no_terminal')),
                event_sources_json text,
                feedback_request_id text,
                feedback_payload_json text,
                acked_at text,
                created_at text not null,
                updated_at text not null
            );",
        )
        .unwrap();
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        init_transaction(&transaction).unwrap();
        transaction.commit().unwrap();

        let mut statement = conn
            .prepare("pragma table_info(event_response_feedback)")
            .unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "sources_json"));
    }

    #[test]
    fn restart_converts_awaiting_intent_to_no_terminal() {
        let path = std::env::temp_dir().join(format!(
            "agentic-gpt-hub-feedback-{}.sqlite",
            random_id("test")
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "create table agent_runs (
                run_id text primary key,
                request_id text not null,
                agent_id text not null,
                command_type text not null,
                command_json text not null,
                command_hash text not null,
                status text not null,
                created_at text not null,
                updated_at text not null
            );",
        )
        .unwrap();
        init(&conn).unwrap();
        let state = state_with_connection(conn);
        let origin = EventOrigin {
            run_id: "run_restart_awaiting".to_string(),
            request_id: "request_restart_awaiting".to_string(),
            command_hash: "hash_restart_awaiting".to_string(),
        };
        {
            let conn = state.db.lock().unwrap();
            let now = now_string();
            conn.execute(
                "insert into agent_runs(
                    run_id, request_id, agent_id, command_type, command_json,
                    command_hash, status, created_at, updated_at
                 ) values (?1, ?2, 'agent', 'process.exec', '{}', ?3, 'dispatched', ?4, ?4)",
                params![origin.run_id, origin.request_id, origin.command_hash, now],
            )
            .unwrap();
        }
        prepare(&state, "agent", &origin).unwrap();
        record_reply_metadata(
            &state,
            "agent",
            &origin,
            &[disposition("process-before-restart", true)],
        )
        .unwrap();
        drop(state);

        let conn = Connection::open(&path).unwrap();
        init(&conn).unwrap();
        recover_after_restart(&conn).unwrap();
        let reopened_state = state_with_connection(conn);
        assert_eq!(stored_decision(&reopened_state, &origin), NO_TERMINAL);
        let pending = pending_for_agent(&reopened_state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        assert!(!pending[0].payload.dispositions[0].includes_terminal);
        drop(reopened_state);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn metadata_identity_conflicts_are_rejected_without_changing_intent() {
        let (state, origin) = fixture("process.exec", "metadata_identity");
        let mut foreign_origin = origin.clone();
        foreign_origin.request_id.push_str("_foreign");
        assert!(record_reply_metadata(
            &state,
            "agent",
            &foreign_origin,
            &[disposition("process-identity", true)],
        )
        .is_err());

        let mut foreign_hash = origin.clone();
        foreign_hash.command_hash.push_str("_foreign");
        assert!(record_reply_metadata(
            &state,
            "agent",
            &foreign_hash,
            &[disposition("process-identity", true)],
        )
        .is_err());
        assert!(record_reply_metadata(
            &state,
            "different-agent",
            &origin,
            &[disposition("process-identity", true)],
        )
        .is_err());
        let original = vec![disposition("process-identity", true)];
        record_reply_metadata(&state, "agent", &origin, &original).unwrap();
        assert!(record_reply_metadata(
            &state,
            "agent",
            &origin,
            &[disposition("process-identity", false)],
        )
        .is_err());
        assert_eq!(stored_decision(&state, &origin), AWAITING);
        assert!(pending_for_agent(&state, "agent").unwrap().is_empty());
    }
    #[test]
    fn returned_dispositions_must_match_persisted_reply_metadata() {
        let (state, origin) = fixture("process.exec", "final_metadata_match");
        let sources = vec![disposition("process-final", true)];
        record_reply_metadata(&state, "agent", &origin, &sources).unwrap();
        let mismatched_sources = vec![disposition("process-final", false)];
        assert!(finalize_original(&state, "agent", &origin, Some(&mismatched_sources),).is_err());
        assert_eq!(stored_decision(&state, &origin), AWAITING);
        assert!(pending_for_agent(&state, "agent").unwrap().is_empty());

        finalize_original(&state, "agent", &origin, Some(&sources)).unwrap();
        assert_eq!(stored_decision(&state, &origin), RETURNED);
    }

    #[test]
    fn late_returned_outcome_cannot_rewrite_final_no_terminal_payload() {
        let sources = vec![disposition("process-sticky", true)];
        let (state, origin) = fixture("process.exec", "sticky_payload");
        assert_eq!(
            finalize_original(&state, "agent", &origin, None).unwrap(),
            FeedbackDecision::NoTerminal
        );
        record_reply_metadata(&state, "agent", &origin, &sources).unwrap();
        let before = pending_for_agent(&state, "agent").unwrap();
        assert!(!before[0].payload.dispositions[0].includes_terminal);

        assert_eq!(
            finalize_original(&state, "agent", &origin, Some(&sources)).unwrap(),
            FeedbackDecision::NoTerminal
        );
        assert_eq!(stored_decision(&state, &origin), NO_TERMINAL);
        assert_eq!(pending_for_agent(&state, "agent").unwrap(), before);
    }

    #[test]
    fn ack_and_reflush_keep_a_stable_idempotent_tombstone() {
        let (state, origin) = fixture("process.exec", "ack_reflush");
        finalize_original(&state, "agent", &origin, None).unwrap();
        record_reply_metadata(
            &state,
            "agent",
            &origin,
            &[disposition("process-ack", true)],
        )
        .unwrap();
        let first = pending_for_agent(&state, "agent").unwrap();
        let second = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 1);
        let feedback = &first[0];
        let mut altered_payload = feedback.payload.clone();
        altered_payload.dispositions[0].includes_terminal = true;
        assert!(ack(
            &state,
            "agent",
            &feedback.origin,
            &feedback.request_id,
            &altered_payload,
        )
        .is_err());
        assert!(ack(
            &state,
            "agent",
            &feedback.origin,
            "different-feedback-request",
            &feedback.payload,
        )
        .is_err());
        assert_eq!(pending_for_agent(&state, "agent").unwrap(), first);

        ack(
            &state,
            "agent",
            &feedback.origin,
            &feedback.request_id,
            &feedback.payload,
        )
        .unwrap();
        ack(
            &state,
            "agent",
            &feedback.origin,
            &feedback.request_id,
            &feedback.payload,
        )
        .unwrap();
        assert!(pending_for_agent(&state, "agent").unwrap().is_empty());

        finalize_original(&state, "agent", &origin, None).unwrap();
        record_reply_metadata(
            &state,
            "agent",
            &origin,
            &[disposition("process-ack", true)],
        )
        .unwrap();
        assert!(pending_for_agent(&state, "agent").unwrap().is_empty());
    }

    #[test]
    fn identity_recovery_during_wait_does_not_override_returned_flags() {
        let (state, origin) = fixture("process.exec", "identity_while_waiting");
        let identity = source("process-recovery");
        record_recovery_sources(&state, "agent", &origin, std::slice::from_ref(&identity)).unwrap();
        {
            let conn = state.db.lock().unwrap();
            let (metadata, identities): (Option<String>, String) = conn
                .query_row(
                    "select event_sources_json, sources_json
                     from event_response_feedback where run_id = ?1",
                    params![origin.run_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert!(metadata.is_none());
            assert_eq!(
                serde_json::from_str::<Vec<EventSource>>(&identities).unwrap(),
                vec![identity]
            );
        }

        let dispositions = vec![disposition("process-recovery", true)];
        record_reply_metadata(&state, "agent", &origin, &dispositions).unwrap();
        assert_eq!(
            finalize_original(&state, "agent", &origin, Some(&dispositions)).unwrap(),
            FeedbackDecision::Returned
        );
        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].payload.dispositions[0].includes_terminal);
    }

    #[test]
    fn recovery_subset_preserves_complete_identity_set_and_outbox() {
        let (state, origin) = fixture("process.exec", "recovery_known_subset");
        let dispositions = vec![
            disposition("process-a", true),
            disposition("process-b", false),
        ];
        record_reply_metadata(&state, "agent", &origin, &dispositions).unwrap();
        finalize_original(&state, "agent", &origin, None).unwrap();
        let before = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(before.len(), 1);

        record_recovery_sources(&state, "agent", &origin, &[source("process-a")]).unwrap();
        assert_eq!(pending_for_agent(&state, "agent").unwrap(), before);
        let identities_json: String = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "select sources_json from event_response_feedback where run_id = ?1",
                params![origin.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<EventSource>>(&identities_json).unwrap(),
            vec![source("process-a"), source("process-b")]
        );

        assert!(
            record_recovery_sources(&state, "agent", &origin, &[source("process-unknown")])
                .is_err()
        );
        assert_eq!(pending_for_agent(&state, "agent").unwrap(), before);
    }

    #[test]
    fn late_complete_reply_metadata_adds_an_immutable_delta_for_batch_child() {
        let (state, origin) = fixture("process.batch", "recovery_subset_then_reply");
        finalize_original(&state, "agent", &origin, None).unwrap();
        record_recovery_sources(&state, "agent", &origin, &[source("process-a")]).unwrap();
        let before = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(before.len(), 1);
        let primary = before[0].clone();
        assert_eq!(
            primary.payload.dispositions,
            vec![disposition("process-a", false)]
        );

        record_reply_metadata(
            &state,
            "agent",
            &origin,
            &[
                disposition("process-a", true),
                disposition("process-b", true),
            ],
        )
        .unwrap();
        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0], primary);
        assert_eq!(pending[1].origin, origin);
        assert_eq!(
            pending[1].payload.dispositions,
            vec![disposition("process-b", false)]
        );
        assert_ne!(pending[0].request_id, pending[1].request_id);

        let primary_json: String = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "select feedback_payload_json from event_response_feedback where run_id = ?1",
                params![origin.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            primary_json,
            serde_json::to_string(&primary.payload).unwrap()
        );
        let identities_json: String = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "select sources_json from event_response_feedback where run_id = ?1",
                params![origin.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<EventSource>>(&identities_json).unwrap(),
            vec![source("process-a"), source("process-b")]
        );
    }
    #[test]
    fn recovery_delta_after_acked_primary_is_stable_and_not_reissued_after_ack() {
        let (state, origin) = fixture("process.batch", "acked_primary_delta");
        finalize_original(&state, "agent", &origin, None).unwrap();
        record_recovery_sources(&state, "agent", &origin, &[source("process-a")]).unwrap();
        let primary = pending_for_agent(&state, "agent").unwrap().remove(0);
        ack(
            &state,
            "agent",
            &origin,
            &primary.request_id,
            &primary.payload,
        )
        .unwrap();

        record_recovery_sources(&state, "agent", &origin, &[source("process-b")]).unwrap();
        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        let delta = pending[0].clone();
        assert_eq!(
            delta.payload.dispositions,
            vec![disposition("process-b", false)]
        );
        assert!(ack(
            &state,
            "agent",
            &origin,
            "wrong-delta-request",
            &delta.payload,
        )
        .is_err());
        let mut altered_delta = delta.payload.clone();
        altered_delta.dispositions[0].includes_terminal = true;
        assert!(ack(&state, "agent", &origin, &delta.request_id, &altered_delta,).is_err());
        assert!(ack(
            &state,
            "other-agent",
            &origin,
            &delta.request_id,
            &delta.payload,
        )
        .is_err());
        ack(&state, "agent", &origin, &delta.request_id, &delta.payload).unwrap();

        record_recovery_sources(&state, "agent", &origin, &[source("process-b")]).unwrap();
        assert!(pending_for_agent(&state, "agent").unwrap().is_empty());
        let saved_request: String = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "select request_id from event_response_feedback_delta
                 where run_id = ?1 and source_ref = 'process-b'",
                params![origin.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved_request, delta.request_id);
    }

    #[test]
    fn restart_repairs_missing_delta_coverage_without_rewriting_primary() {
        let path = std::env::temp_dir().join(format!(
            "agentic-gpt-hub-feedback-delta-{}.sqlite",
            random_id("test")
        ));
        let conn = Connection::open(&path).unwrap();
        crate::db::init_db(&conn).unwrap();
        init(&conn).unwrap();
        let state = state_with_connection(conn);
        let origin = EventOrigin {
            run_id: "run_restart_delta_repair".to_string(),
            request_id: "request_restart_delta_repair".to_string(),
            command_hash: "hash_restart_delta_repair".to_string(),
        };
        {
            let conn = state.db.lock().unwrap();
            let now = now_string();
            conn.execute(
                "insert into agent_runs(
                    run_id, request_id, agent_id, command_type, command_json,
                    command_hash, status, created_at, updated_at
                 ) values (?1, ?2, 'agent', 'process.batch', '{}', ?3, 'dispatched', ?4, ?4)",
                params![origin.run_id, origin.request_id, origin.command_hash, now],
            )
            .unwrap();
        }
        prepare(&state, "agent", &origin).unwrap();
        finalize_original(&state, "agent", &origin, None).unwrap();
        record_recovery_sources(&state, "agent", &origin, &[source("process-a")]).unwrap();
        let primary = pending_for_agent(&state, "agent").unwrap().remove(0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "update event_response_feedback set sources_json = ?1 where run_id = ?2",
                params![
                    serde_json::to_string(&vec![source("process-a"), source("process-b")]).unwrap(),
                    origin.run_id
                ],
            )
            .unwrap();
        }
        drop(state);

        let conn = Connection::open(&path).unwrap();
        init(&conn).unwrap();
        recover_after_restart(&conn).unwrap();
        let reopened = state_with_connection(conn);
        let pending = pending_for_agent(&reopened, "agent").unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0], primary);
        assert_eq!(
            pending[1].payload.dispositions,
            vec![disposition("process-b", false)]
        );
        let primary_json: String = reopened
            .db
            .lock()
            .unwrap()
            .query_row(
                "select feedback_payload_json from event_response_feedback where run_id = ?1",
                params![origin.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            primary_json,
            serde_json::to_string(&primary.payload).unwrap()
        );
        {
            let conn = reopened.db.lock().unwrap();
            recover_after_restart(&conn).unwrap();
        }
        assert_eq!(pending_for_agent(&reopened, "agent").unwrap(), pending);
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn no_terminal_recovery_sources_materialize_false_dispositions() {
        let (state, origin) = fixture("process.exec", "identity_after_no_terminal");
        assert_eq!(
            finalize_original(&state, "agent", &origin, None).unwrap(),
            FeedbackDecision::NoTerminal
        );
        record_recovery_sources(&state, "agent", &origin, &[source("process-reconnected")])
            .unwrap();

        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].payload.dispositions,
            vec![disposition("process-reconnected", false)]
        );
    }

    #[test]
    fn late_metadata_preserves_identity_only_outbox_order_and_request_id() {
        let (state, origin) = fixture("process.exec", "late_metadata_order");
        finalize_original(&state, "agent", &origin, None).unwrap();
        record_recovery_sources(
            &state,
            "agent",
            &origin,
            &[source("process-z"), source("process-a")],
        )
        .unwrap();
        let before = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(
            before[0]
                .payload
                .dispositions
                .iter()
                .map(|disposition| disposition.source.reference.as_str())
                .collect::<Vec<_>>(),
            vec!["process-a", "process-z"]
        );

        record_reply_metadata(
            &state,
            "agent",
            &origin,
            &[
                disposition("process-z", true),
                disposition("process-a", true),
            ],
        )
        .unwrap();
        assert_eq!(pending_for_agent(&state, "agent").unwrap(), before);
    }

    #[test]
    fn orphan_repair_only_decides_missing_replayable_creation_runs() {
        let state = test_state();
        let now = now_string();
        let orphan = EventOrigin {
            run_id: "orphan_replayable".to_string(),
            request_id: "orphan_request".to_string(),
            command_hash: "orphan_hash".to_string(),
        };
        let reported_orphan = EventOrigin {
            run_id: "reported_orphan".to_string(),
            request_id: "reported_request".to_string(),
            command_hash: "reported_hash".to_string(),
        };
        let existing = EventOrigin {
            run_id: "existing_intent".to_string(),
            request_id: "existing_request".to_string(),
            command_hash: "existing_hash".to_string(),
        };
        let completed = EventOrigin {
            run_id: "completed_no_intent".to_string(),
            request_id: "completed_request".to_string(),
            command_hash: "completed_hash".to_string(),
        };
        let non_creation = EventOrigin {
            run_id: "status_no_intent".to_string(),
            request_id: "status_request".to_string(),
            command_hash: "status_hash".to_string(),
        };
        {
            let conn = state.db.lock().unwrap();
            for (origin, command_type, status, result_json) in [
                (&orphan, "process.exec", "dispatched", None),
                (&reported_orphan, "process.exec", "dispatched", None),
                (&existing, "process.exec", "dispatched", None),
                (&completed, "process.exec", "completed", Some("{}")),
                (&non_creation, "process.read", "dispatched", None),
            ] {
                conn.execute(
                    "insert into agent_runs(
                        run_id, request_id, agent_id, command_type, command_json,
                        command_hash, status, result_json, created_at, updated_at
                     ) values (?1, ?2, 'agent', ?3, '{}', ?4, ?5, ?6, ?7, ?7)",
                    params![
                        origin.run_id,
                        origin.request_id,
                        command_type,
                        origin.command_hash,
                        status,
                        result_json,
                        now
                    ],
                )
                .unwrap();
            }
        }
        record_recovery_sources(
            &state,
            "agent",
            &reported_orphan,
            &[source("reported-process")],
        )
        .unwrap();
        prepare(&state, "agent", &existing).unwrap();
        repair_orphan_runs(&state, "agent").unwrap();

        assert_eq!(stored_decision(&state, &orphan), NO_TERMINAL);
        assert_eq!(stored_decision(&state, &reported_orphan), NO_TERMINAL);
        let recovered = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(
            recovered[0].payload.dispositions,
            vec![disposition("reported-process", false)]
        );
        assert_eq!(stored_decision(&state, &existing), AWAITING);
        let conn = state.db.lock().unwrap();
        for absent in [&completed, &non_creation] {
            let feedback: Option<String> = conn
                .query_row(
                    "select decision from event_response_feedback where run_id = ?1",
                    params![absent.run_id],
                    |row| row.get(0),
                )
                .optional()
                .unwrap();
            assert!(feedback.is_none());
        }
    }

    #[test]
    fn dropped_owner_queues_and_persists_no_terminal_with_raw_metadata() {
        let (state, origin) = fixture("process.exec", "owner_drop_repair");
        let dispositions = vec![disposition("process-owner-drop", true)];
        {
            let mut owner = ResponseOwnerGuard::new(
                &state.dispatch.response_feedback,
                "agent",
                &origin,
                "process.exec",
            );
            owner.set_sources(&dispositions).unwrap();
        }
        let queued = state.dispatch.response_feedback.pending_for_agent("agent");
        assert_eq!(queued.len(), 1);
        assert!(queued[0].1.no_terminal);
        assert_eq!(queued[0].1.dispositions, Some(dispositions.clone()));

        persist_queued_repairs(&state, "agent").unwrap();
        assert_eq!(stored_decision(&state, &origin), NO_TERMINAL);
        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].payload.dispositions,
            vec![disposition("process-owner-drop", false)]
        );
        assert!(state
            .dispatch
            .response_feedback
            .pending_for_agent("agent")
            .is_empty());
    }

    #[test]
    fn failed_metadata_write_is_retained_and_retried_without_losing_flags() {
        let (state, origin) = fixture("process.exec", "metadata_write_retry");
        let dispositions = vec![disposition("process-retry-metadata", true)];
        state
            .db
            .lock()
            .unwrap()
            .execute_batch(
                "create trigger fail_feedback_source_write
                 before update of sources_json on event_response_feedback
                 begin select raise(abort, 'forced_feedback_source_write_failure'); end;",
            )
            .unwrap();
        assert!(record_reply_metadata(&state, "agent", &origin, &dispositions).is_err());
        let queued = state.dispatch.response_feedback.pending_for_agent("agent");
        assert_eq!(queued.len(), 1);
        assert!(!queued[0].1.no_terminal);
        assert_eq!(queued[0].1.dispositions, Some(dispositions.clone()));

        state
            .db
            .lock()
            .unwrap()
            .execute_batch("drop trigger fail_feedback_source_write;")
            .unwrap();
        persist_queued_repairs(&state, "agent").unwrap();
        assert_eq!(stored_decision(&state, &origin), AWAITING);
        assert_eq!(
            finalize_original(&state, "agent", &origin, Some(&dispositions)).unwrap(),
            FeedbackDecision::Returned
        );
        assert_eq!(
            pending_for_agent(&state, "agent").unwrap()[0]
                .payload
                .dispositions,
            dispositions
        );
    }

    #[test]
    fn returned_commit_failure_is_repaired_as_no_terminal() {
        let (state, origin) = fixture("process.exec", "returned_commit_failure");
        let dispositions = vec![disposition("process-failed-returned", true)];
        record_reply_metadata(&state, "agent", &origin, &dispositions).unwrap();
        let mut owner = ResponseOwnerGuard::new(
            &state.dispatch.response_feedback,
            "agent",
            &origin,
            "process.exec",
        );
        owner.set_sources(&dispositions).unwrap();
        state
            .db
            .lock()
            .unwrap()
            .execute_batch(
                "create trigger fail_feedback_returned
                 before update of decision on event_response_feedback
                 when new.decision = 'returned'
                 begin select raise(abort, 'forced_feedback_returned_failure'); end;",
            )
            .unwrap();
        assert!(finalize_original(&state, "agent", &origin, Some(&dispositions)).is_err());
        state
            .db
            .lock()
            .unwrap()
            .execute_batch("drop trigger fail_feedback_returned;")
            .unwrap();
        drop(owner);

        persist_queued_repairs(&state, "agent").unwrap();
        assert_eq!(stored_decision(&state, &origin), NO_TERMINAL);
        assert_eq!(
            pending_for_agent(&state, "agent").unwrap()[0]
                .payload
                .dispositions,
            vec![disposition("process-failed-returned", false)]
        );
    }

    #[tokio::test]
    async fn delta_arriving_during_ack_is_settled_before_public_request_dispatch() {
        let (state, origin) = fixture("process.batch", "flush_delta_barrier");
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "insert into agents(
                    agent_id, alias, display_name, enabled, secret_hash,
                    last_seen_at, capabilities_json
                 ) values ('agent', null, 'agent', 1, ?1, null, ?2)",
                params![
                    crate::utils::sha256_hex("secret"),
                    r#"{"processes":true,"confirmation":true,"notification_actions":true}"#
                ],
            )
            .unwrap();
        }
        let (sender, mut outbound) = mpsc::unbounded_channel();
        state
            .agents
            .insert_for_test(
                "agent",
                crate::state::AgentConnection {
                    connection_id: "feedback-test-connection".to_string(),
                    sender,
                    last_seen_at: chrono::Utc::now(),
                    role: AgentRole::Normal,
                    connection_mode: AgentConnectionMode::CommandCapable,
                    hello_received: true,
                    boot_generation: Some("testboot".to_string()),
                    transport: AgentTransport::Sse,
                    config_summary: None,
                    notification_channels: Vec::new(),
                },
            )
            .await;

        finalize_original(&state, "agent", &origin, None).unwrap();
        record_recovery_sources(&state, "agent", &origin, &[source("process-a")]).unwrap();
        let state = Arc::new(state);
        let flush_state = Arc::clone(&state);
        let flush = tokio::spawn(async move { flush_for_agent(&flush_state, "agent").await });
        let OutboundAgentMessage::Text(text) = outbound.recv().await.unwrap() else {
            panic!("expected primary EventSettle request");
        };
        let primary_envelope: HubCommandEnvelope = serde_json::from_str(&text).unwrap();
        assert!(matches!(
            primary_envelope.command,
            HubCommand::EventSettle { .. }
        ));

        let public_state = Arc::clone(&state);
        let public_request = tokio::spawn(async move {
            crate::agents::dispatch::request_agent(
                &public_state,
                "agent",
                HubCommand::Exec {
                    request_id: "public-after-feedback".to_string(),
                    payload: ProcessExecRequest {
                        agent_id: "agent".to_string(),
                        group: None,
                        command: "true".to_string(),
                        need_confirm: false,
                        confirm_method: None,
                        cwd: None,
                        wait_seconds: None,
                    },
                },
                5,
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(outbound.try_recv().is_err());
        record_recovery_sources(&state, "agent", &origin, &[source("process-b")]).unwrap();

        answer_agent_request(&state, primary_envelope, json!({"status":"settled"})).await;
        let OutboundAgentMessage::Text(text) = outbound.recv().await.unwrap() else {
            panic!("expected delta EventSettle request");
        };
        let delta_envelope: HubCommandEnvelope = serde_json::from_str(&text).unwrap();
        assert!(matches!(
            &delta_envelope.command,
            HubCommand::EventSettle { payload, .. }
                if payload.dispositions == vec![disposition("process-b", false)]
        ));
        answer_agent_request(&state, delta_envelope, json!({"status":"settled"})).await;

        let OutboundAgentMessage::Text(text) = outbound.recv().await.unwrap() else {
            panic!("expected public request after all feedback acknowledgments");
        };
        let public_envelope: HubCommandEnvelope = serde_json::from_str(&text).unwrap();
        assert!(matches!(public_envelope.command, HubCommand::Exec { .. }));
        answer_agent_request(&state, public_envelope, json!({"ok":true})).await;

        flush.await.unwrap().unwrap();
        assert!(public_request.await.unwrap().is_ok());
        assert!(pending_for_agent(&state, "agent").unwrap().is_empty());
    }

    #[test]
    fn public_preflight_requires_idle_barrier_and_drained_queued_repairs() {
        let (state, origin) = fixture("process.exec", "public_preflight");
        let ready = try_public_preflight(&state, "agent").unwrap().unwrap();
        assert!(try_public_preflight(&state, "agent").unwrap().is_none());
        drop(ready);

        let dispositions = vec![disposition("process-preflight", true)];
        let mut owner = ResponseOwnerGuard::new(
            &state.dispatch.response_feedback,
            "agent",
            &origin,
            "process.exec",
        );
        owner.set_sources(&dispositions).unwrap();
        drop(owner);

        assert!(try_public_preflight(&state, "agent").unwrap().is_none());
        let pending = pending_for_agent(&state, "agent").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].payload.dispositions,
            vec![disposition("process-preflight", false)]
        );
    }

    #[test]
    fn non_creation_and_source_less_runs_do_not_create_feedback_commands() {
        let state = test_state();
        let read_origin = EventOrigin {
            run_id: "run_process_read_only".to_string(),
            request_id: "request_process_read_only".to_string(),
            command_hash: "hash_process_read_only".to_string(),
        };
        {
            let conn = state.db.lock().unwrap();
            let now = now_string();
            conn.execute(
                "insert into agent_runs(
                    run_id, request_id, agent_id, command_type, command_json,
                    command_hash, status, created_at, updated_at
                 ) values (?1, ?2, 'agent', 'process.read', '{}', ?3, 'dispatched', ?4, ?4)",
                params![
                    read_origin.run_id,
                    read_origin.request_id,
                    read_origin.command_hash,
                    now
                ],
            )
            .unwrap();
        }
        assert!(prepare(&state, "agent", &read_origin).is_err());

        let (state, origin) = fixture("process.exec", "source_less");
        record_reply_metadata(&state, "agent", &origin, &[]).unwrap();
        finalize_original(&state, "agent", &origin, Some(&[])).unwrap();
        assert!(pending_for_agent(&state, "agent").unwrap().is_empty());
    }
}
