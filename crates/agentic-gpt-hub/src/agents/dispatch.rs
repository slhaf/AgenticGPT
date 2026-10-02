use agentic_gpt_protocol::{
    AgentMessage, EventOrigin, EventResponseDisposition, HubCommand, HubCommandEnvelope,
};
use rusqlite::TransactionBehavior;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use tracing::warn;

use crate::agents::lifecycle;
use crate::event_feedback;
use crate::registry::registry_entries;
use crate::runs;
use crate::state::{HubState, OutboundAgentMessage};
use crate::utils::random_id;
use crate::REQUEST_TIMEOUT_SECS;

struct PendingResponse {
    agent_id: String,
    request_id: String,
    command_hash: String,
    sender: oneshot::Sender<(Value, Option<Vec<EventResponseDisposition>>)>,
}

pub(crate) struct Dispatch {
    pub(crate) response_feedback: event_feedback::FeedbackCoordinator,
    pending: Mutex<HashMap<String, PendingResponse>>,
}

impl Dispatch {
    pub(crate) fn new() -> Self {
        Self {
            response_feedback: event_feedback::FeedbackCoordinator::default(),
            pending: Mutex::new(HashMap::new()),
        }
    }
    pub(crate) async fn pending_count(&self) -> usize {
        self.pending.lock().await.len()
    }
}
pub(super) async fn handle_reliable_message(
    state: &HubState,
    agent_id: &str,
    message: AgentMessage,
) -> Result<(), String> {
    match message {
        AgentMessage::Response {
            run_id,
            request_id,
            data,
            event_sources,
        } => {
            let Some(run_id) = run_id else {
                return Err("response_run_id_required".to_string());
            };
            let outcome = match runs::store_result(state, agent_id, &run_id, &request_id, &data) {
                Ok(outcome) => outcome,
                Err(error) => {
                    if let Ok(Some(run)) = runs::get_run(state, &run_id) {
                        if run.agent_id == agent_id
                            && run.request_id == request_id
                            && event_feedback::is_creation_command_type(&run.command_type)
                        {
                            let origin = EventOrigin {
                                run_id: run.run_id,
                                request_id: run.request_id,
                                command_hash: run.command_hash,
                            };
                            if let Err(metadata_error) = event_feedback::record_reply_metadata(
                                state,
                                agent_id,
                                &origin,
                                &event_sources,
                            ) {
                                warn!(%agent_id, %request_id, %metadata_error, "failed to preserve event response metadata after result-store failure");
                            }
                        }
                    }
                    warn!(%agent_id, %request_id, %error, "failed to store agent result");
                    return Err("response_result_store_failed".to_string());
                }
            };
            match outcome {
                runs::StoreResultOutcome::Unmatched => Err("response_run_mismatch".to_string()),
                runs::StoreResultOutcome::Conflict => Err("response_result_conflict".to_string()),
                runs::StoreResultOutcome::Stored {
                    command_hash,
                    command_type,
                }
                | runs::StoreResultOutcome::Duplicate {
                    command_hash,
                    command_type,
                } => {
                    let origin = EventOrigin {
                        run_id: run_id.clone(),
                        request_id: request_id.clone(),
                        command_hash: command_hash.clone(),
                    };
                    let feedback_eligible = event_feedback::is_creation_command_type(&command_type);
                    let response_sources = if feedback_eligible {
                        match event_feedback::record_reply_metadata(
                            state,
                            agent_id,
                            &origin,
                            &event_sources,
                        ) {
                            Ok(()) => Some(event_sources),
                            Err(error) => {
                                let retryable = error.downcast_ref::<rusqlite::Error>().is_some();
                                warn!(%agent_id, %request_id, %error, "failed to record event response metadata");
                                retryable.then_some(event_sources)
                            }
                        }
                    } else {
                        None
                    };
                    if feedback_eligible {
                        let flush_state = state.clone();
                        let flush_agent = agent_id.to_string();
                        tokio::spawn(async move {
                            if let Err(error) =
                                event_feedback::flush_for_agent(&flush_state, &flush_agent).await
                            {
                                warn!(%flush_agent, %error, "response event feedback retry failed");
                            }
                        });
                    }
                    let sender = {
                        let mut pending = state.dispatch.pending.lock().await;
                        let owner_matches = pending.get(&run_id).map(|owner| {
                            owner.agent_id == agent_id
                                && owner.request_id == request_id
                                && owner.command_hash == command_hash
                        });
                        match owner_matches {
                            None => None,
                            Some(true) => pending.remove(&run_id).map(|pending| pending.sender),
                            Some(false) => {
                                return Err("response_waiter_owner_mismatch".to_string());
                            }
                        }
                    };
                    if let Some(sender) = sender {
                        let _ = sender.send((data, response_sources));
                    }
                    Ok(())
                }
            }
        }
        AgentMessage::EventSources { origin, sources } => {
            let result =
                event_feedback::record_recovery_sources(state, agent_id, &origin, &sources);
            result.map_err(|error| {
                let reason = error.to_string();
                if is_event_sources_validation_error(&reason) {
                    format!("event_sources_validation:{reason}")
                } else {
                    reason
                }
            })?;
            let flush_state = state.clone();
            let flush_agent = agent_id.to_string();
            tokio::spawn(async move {
                if let Err(error) =
                    event_feedback::flush_for_agent(&flush_state, &flush_agent).await
                {
                    warn!(%flush_agent, %error, "recovery event feedback delivery failed");
                }
            });
            Ok(())
        }
        AgentMessage::TransportAck {
            event_id: _,
            run_id,
            request_id,
            command_hash,
        } => {
            let matched = runs::mark_acked(state, agent_id, &run_id, &request_id, &command_hash)
                .map_err(|error| error.to_string())?;
            if !matched {
                return Err("transport_ack_run_mismatch".to_string());
            }
            Ok(())
        }
        AgentMessage::TransportRunStatus {
            run_id,
            request_id,
            status,
            reason,
        } => {
            let matched = runs::mark_status(
                state,
                agent_id,
                &run_id,
                &request_id,
                &status,
                reason.as_deref(),
            )
            .map_err(|error| error.to_string())?;
            if !matched {
                return Err("transport_status_run_mismatch".to_string());
            }
            Ok(())
        }
        AgentMessage::Hello { .. }
        | AgentMessage::Heartbeat { .. }
        | AgentMessage::ProcessUpdate { .. }
        | AgentMessage::RunReport { .. }
        | AgentMessage::ConfirmationRequest { .. } => {
            unreachable!("reliable message classification drifted")
        }
    }
}
fn is_event_sources_validation_error(reason: &str) -> bool {
    matches!(
        reason,
        "event_feedback_run_missing"
            | "event_feedback_origin_mismatch"
            | "event_feedback_non_creation_run"
            | "event_feedback_intent_missing"
            | "event_feedback_command_mismatch"
            | "event_feedback_invalid_source"
            | "event_feedback_source_command_mismatch"
            | "event_feedback_duplicate_source"
            | "event_feedback_recovery_source_conflict"
    )
}

pub(crate) async fn request_agent(
    state: &HubState,
    agent_id: &str,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, String> {
    let (target, admission) = lifecycle::resolve_command_target(state, agent_id).await?;
    let pending = state.dispatch.pending.lock().await;
    request_target(state, target, admission, pending, command, timeout_secs).await
}

pub(crate) async fn request_room(
    state: &HubState,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, crate::room::control::RoomRouteError> {
    let (target, admission) = lifecycle::resolve_room_target(state).await?;
    let pending = state.dispatch.pending.lock().await;
    request_target(state, target, admission, pending, command, timeout_secs)
        .await
        .map_err(|reason| match reason.as_str() {
            "agent_offline"
            | "agent_connection_changed"
            | "agent_reporting_only"
            | "agent_role_changed" => crate::room::control::RoomRouteError::StateConflict,
            _ => crate::room::control::RoomRouteError::Timeout(reason),
        })
}

async fn request_target<'a>(
    state: &'a HubState,
    target: lifecycle::DispatchTarget,
    admission: lifecycle::DispatchAdmission<'a>,
    pending: tokio::sync::MutexGuard<'a, HashMap<String, PendingResponse>>,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, String> {
    let request_id = command.request_id().to_string();
    let feedback_eligible = event_feedback::is_creation_command(&command);
    let needs_public_preflight = !matches!(&command, HubCommand::EventSettle { .. });
    let mut command = Some(command);
    let mut pending_slot = Some(pending);
    let mut admission_slot = Some(admission);
    let (run, origin, mut owner_guard, rx, send_result) = loop {
        let mut pending = pending_slot.take().expect("dispatch pending guard");
        let admission = admission_slot.take().expect("dispatch admission guard");
        let feedback_guard = if needs_public_preflight {
            match event_feedback::try_public_preflight(state, &target.agent_id) {
                Ok(Some(guard)) => Some(guard),
                Ok(None) => {
                    drop(admission);
                    drop(pending);
                    event_feedback::flush_for_agent(state, &target.agent_id)
                        .await
                        .map_err(|error| error.to_string())?;
                    admission_slot = Some(lifecycle::admit_dispatch_target(state, &target).await?);
                    pending_slot = Some(state.dispatch.pending.lock().await);
                    continue;
                }
                Err(error) => return Err(error.to_string()),
            }
        } else {
            None
        };

        let mut conn = state.db.lock().unwrap();
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let run = runs::prepare_run_in_transaction(
            &transaction,
            &target.agent_id,
            &request_id,
            command
                .as_ref()
                .expect("dispatch command remains available"),
        )
        .map_err(|error| error.to_string())?;
        let origin = EventOrigin {
            run_id: run.run_id.clone(),
            request_id: run.request_id.clone(),
            command_hash: run.command_hash.clone(),
        };
        if feedback_eligible {
            event_feedback::prepare_in_transaction(&transaction, &target.agent_id, &origin)
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        drop(conn);

        let owner_guard = if feedback_eligible {
            Some(event_feedback::ResponseOwnerGuard::new(
                &state.dispatch.response_feedback,
                &target.agent_id,
                &origin,
                runs::command_type(
                    command
                        .as_ref()
                        .expect("dispatch command remains available"),
                ),
            ))
        } else {
            None
        };
        let text = envelope_text(
            &run.run_id,
            &run.request_id,
            &run.command_hash,
            command.take().expect("dispatch command is sent once"),
        )
        .map_err(|error| error.to_string())?;
        let (tx, rx) = oneshot::channel();
        pending.insert(
            run.run_id.clone(),
            PendingResponse {
                agent_id: target.agent_id.clone(),
                request_id: run.request_id.clone(),
                command_hash: run.command_hash.clone(),
                sender: tx,
            },
        );
        let send_result = admission.send(OutboundAgentMessage::Text(text));
        if send_result.is_err() {
            pending.remove(&run.run_id);
        }
        drop(feedback_guard);
        drop(admission);
        drop(pending);
        break (run, origin, owner_guard, rx, send_result);
    };
    let run_id = run.run_id;
    if let Err(send_reason) = send_result {
        if let Err(error) = runs::mark_not_sent(state, &run_id, &send_reason) {
            warn!(runId = %run_id, %error, "failed to mark run not sent");
        }
        if feedback_eligible {
            if let Some(mut guard) = owner_guard.take() {
                match event_feedback::finalize_original(state, &target.agent_id, &origin, None) {
                    Ok(decision) => {
                        if let Err(error) = guard.complete_no_terminal(decision) {
                            warn!(runId = %origin.run_id, %error, "failed to confirm offline event decision");
                        }
                    }
                    Err(error) => {
                        warn!(runId = %origin.run_id, %error, "failed to persist offline event decision");
                    }
                }
            }
            let flush_state = state.clone();
            let flush_agent = target.agent_id.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    event_feedback::flush_for_agent(&flush_state, &flush_agent).await
                {
                    warn!(%flush_agent, %error, "offline event feedback retry failed");
                }
            });
        }
        if send_reason == "agent_offline" {
            let _ =
                lifecycle::disconnect_agent(state, &target.agent_id, &target.connection_id, None)
                    .await;
        }
        return Err(send_reason);
    }
    if let Err(error) = runs::mark_dispatched(state, &run_id) {
        warn!(runId = %run_id, %error, "failed to mark run dispatched");
    }
    match timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok((value, sources))) => {
            if feedback_eligible {
                if let Some(sources) = sources {
                    let mut guard = owner_guard.take().expect("feedback owner guard");
                    if let Err(error) = guard.set_sources(&sources) {
                        warn!(runId = %origin.run_id, %error, "event response metadata was rejected");
                        return Err("event_response_metadata_rejected".to_string());
                    }
                    let decision = event_feedback::finalize_original(
                        state,
                        &target.agent_id,
                        &origin,
                        Some(&sources),
                    )
                    .map_err(|error| error.to_string())?;
                    guard
                        .complete_returned(decision)
                        .map_err(|error| error.to_string())?;
                    let flush_state = state.clone();
                    let flush_agent = target.agent_id.clone();
                    tokio::spawn(async move {
                        if let Err(error) =
                            event_feedback::flush_for_agent(&flush_state, &flush_agent).await
                        {
                            warn!(%flush_agent, %error, "returned event feedback delivery failed");
                        }
                    });
                } else {
                    warn!(runId = %origin.run_id, "event response metadata was not persisted");
                    return Err("event_response_metadata_unavailable".to_string());
                }
            }
            Ok(value)
        }
        _ => {
            state.dispatch.pending.lock().await.remove(&run_id);
            if feedback_eligible {
                let mut guard = owner_guard.take().expect("feedback owner guard");
                let decision =
                    event_feedback::finalize_original(state, &target.agent_id, &origin, None)
                        .map_err(|error| error.to_string())?;
                guard
                    .complete_no_terminal(decision)
                    .map_err(|error| error.to_string())?;
                let flush_state = state.clone();
                let flush_agent = target.agent_id.clone();
                tokio::spawn(async move {
                    if let Err(error) =
                        event_feedback::flush_for_agent(&flush_state, &flush_agent).await
                    {
                        warn!(%flush_agent, %error, "timeout event feedback delivery failed");
                    }
                });
            }
            if let Err(error) = runs::mark_timeout(state, &run_id, "process_exec_timeout") {
                warn!(runId = %run_id, %error, "failed to mark run timeout");
            }
            Err(format!("process_exec_timeout; runId={}", run_id))
        }
    }
}

pub(super) async fn send_pending_replays(
    state: &HubState,
    agent_id: &str,
    tx: &mpsc::UnboundedSender<OutboundAgentMessage>,
) {
    match runs::pending_unacked(state, agent_id) {
        Ok(pending) => {
            let has_creation_runs = pending
                .iter()
                .any(|run| event_feedback::is_creation_command(&run.command));
            let creation_repair_succeeded = if has_creation_runs {
                match event_feedback::repair_orphan_runs(state, agent_id) {
                    Ok(()) => true,
                    Err(error) => {
                        warn!(%agent_id, %error, "failed to repair replay feedback intents");
                        false
                    }
                }
            } else {
                true
            };
            for run in pending {
                if event_feedback::is_creation_command(&run.command) && !creation_repair_succeeded {
                    continue;
                }
                match envelope_text(&run.run_id, &run.request_id, &run.command_hash, run.command) {
                    Ok(text) => {
                        let _ = tx.send(OutboundAgentMessage::Text(text));
                    }
                    Err(error) => warn!(%agent_id, %error, "failed to encode pending replay"),
                }
            }
        }
        Err(error) => warn!(%agent_id, %error, "failed to load pending replay"),
    }
}

fn envelope_text(
    run_id: &str,
    request_id: &str,
    command_hash: &str,
    command: HubCommand,
) -> serde_json::Result<String> {
    serde_json::to_string(&HubCommandEnvelope {
        event_id: random_id("evt"),
        run_id: run_id.to_string(),
        request_id: request_id.to_string(),
        command_hash: command_hash.to_string(),
        command,
    })
}
pub(crate) async fn cache_value_with_event_panel(
    state: &HubState,
    agent_id: &str,
    mut value: Value,
) -> Value {
    let panel = request_agent(
        state,
        agent_id,
        HubCommand::EventPanel {
            request_id: random_id("req"),
        },
        2,
    )
    .await
    .ok()
    .and_then(|panel| {
        serde_json::from_value::<agentic_gpt_protocol::EventPanel>(panel.clone())
            .ok()
            .map(|_| panel)
    });
    if let Some(panel) = panel {
        if let Some(object) = value.as_object_mut() {
            object.entry("events").or_insert(panel);
        }
    }
    value
}
pub(crate) async fn mcp_list_servers_all_agents(
    state: &HubState,
) -> std::result::Result<Value, String> {
    let entries = registry_entries(state).map_err(|error| error.to_string())?;
    let online_agent_ids = state.agents.online_agents(&entries).await;

    let mut agents = Vec::new();
    for (agent_id, display_name) in online_agent_ids {
        let command = HubCommand::McpListServers {
            request_id: random_id("req"),
            suppress_event_panel: true,
        };
        let value = request_agent(state, &agent_id, command, REQUEST_TIMEOUT_SECS).await;
        match value {
            Ok(value) => {
                let servers = value
                    .get("servers")
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                agents.push(json!({
                    "agentId": agent_id,
                    "displayName": display_name,
                    "online": true,
                    "servers": servers,
                }));
            }
            Err(reason) => {
                agents.push(json!({
                    "agentId": agent_id,
                    "displayName": display_name,
                    "online": true,
                    "servers": [],
                    "error": {
                        "code": "mcp_list_servers_timeout",
                        "message": reason,
                    },
                }));
            }
        }
    }

    Ok(json!({ "agents": agents }))
}

pub(crate) async fn cached_process(
    state: &HubState,
    agent_id: &str,
    process_id: &str,
) -> Option<crate::state::ProcessCacheSnapshot> {
    state.process_cache.snapshot(agent_id, process_id).await
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
