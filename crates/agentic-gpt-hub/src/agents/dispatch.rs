use agentic_gpt_protocol::{AgentMessage, HubCommand, HubCommandEnvelope};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use tracing::warn;

use crate::agents::lifecycle;
use crate::registry::registry_entries;
use crate::runs;
use crate::state::{HubState, OutboundAgentMessage};
use crate::utils::random_id;
use crate::REQUEST_TIMEOUT_SECS;

struct PendingResponse {
    agent_id: String,
    request_id: String,
    command_hash: String,
    sender: oneshot::Sender<Value>,
}

pub(crate) struct Dispatch {
    pending: Mutex<HashMap<String, PendingResponse>>,
}

impl Dispatch {
    pub(crate) fn new() -> Self {
        Self {
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
        } => {
            let Some(run_id) = run_id else {
                return Err("response_run_id_required".to_string());
            };
            let outcome = match runs::store_result(state, agent_id, &run_id, &request_id, &data) {
                Ok(outcome) => outcome,
                Err(error) => {
                    warn!(%agent_id, %request_id, %error, "failed to store agent result");
                    return Err("response_result_store_failed".to_string());
                }
            };
            match outcome {
                runs::StoreResultOutcome::Unmatched => Err("response_run_mismatch".to_string()),
                runs::StoreResultOutcome::Conflict => Err("response_result_conflict".to_string()),
                runs::StoreResultOutcome::Stored { command_hash }
                | runs::StoreResultOutcome::Duplicate { command_hash } => {
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
                        let _ = sender.send(data);
                    }
                    Ok(())
                }
            }
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
        | AgentMessage::JobUpdate { .. }
        | AgentMessage::RunReport { .. }
        | AgentMessage::ConfirmationRequest { .. } => {
            unreachable!("reliable message classification drifted")
        }
    }
}

pub(crate) async fn request_agent(
    state: &HubState,
    agent_id: &str,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, String> {
    let target = lifecycle::resolve_command_target(state, agent_id).await?;
    request_target(state, target, command, timeout_secs).await
}

pub(crate) async fn request_room(
    state: &HubState,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, crate::room::RoomRouteError> {
    let target = lifecycle::resolve_room_target(state).await?;
    request_target(state, target, command, timeout_secs)
        .await
        .map_err(crate::room::RoomRouteError::Timeout)
}

async fn request_target(
    state: &HubState,
    target: lifecycle::DispatchTarget,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, String> {
    let request_id = command.request_id();
    let run = runs::prepare_run(state, &target.agent_id, request_id, &command)
        .map_err(|error| error.to_string())?;
    let text = envelope_text(&run.run_id, &run.request_id, &run.command_hash, command)
        .map_err(|error| error.to_string())?;
    let run_id = run.run_id;
    let run_request_id = run.request_id;
    let command_hash = run.command_hash;
    let (tx, rx) = oneshot::channel();
    state.dispatch.pending.lock().await.insert(
        run_id.clone(),
        PendingResponse {
            agent_id: target.agent_id.clone(),
            request_id: run_request_id,
            command_hash,
            sender: tx,
        },
    );
    if target
        .sender
        .send(OutboundAgentMessage::Text(text))
        .is_err()
    {
        state.dispatch.pending.lock().await.remove(&run_id);
        if let Err(error) = runs::mark_not_sent(state, &run_id, "agent_offline") {
            warn!(runId = %run_id, %error, "failed to mark run not sent");
        }
        let _ =
            lifecycle::disconnect_agent(state, &target.agent_id, &target.connection_id, None).await;
        return Err("agent_offline".to_string());
    }
    if let Err(error) = runs::mark_dispatched(state, &run_id) {
        warn!(runId = %run_id, %error, "failed to mark run dispatched");
    }
    match timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(value)) => Ok(value),
        _ => {
            state.dispatch.pending.lock().await.remove(&run_id);
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
            for run in pending {
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
pub(crate) async fn mcp_list_servers_all_agents(
    state: &HubState,
) -> std::result::Result<Value, String> {
    let entries = registry_entries(state).map_err(|error| error.to_string())?;
    let online_agent_ids = state.agents.online_agents(&entries).await;

    let mut agents = Vec::new();
    for (agent_id, display_name) in online_agent_ids {
        let command = HubCommand::McpListServers {
            request_id: random_id("req"),
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

pub(crate) async fn cached_job(
    state: &HubState,
    agent_id: &str,
    job_id: &str,
) -> Option<crate::state::JobCacheSnapshot> {
    state.job_cache.snapshot(agent_id, job_id).await
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
