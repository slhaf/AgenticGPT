use agentic_gpt_protocol::{AgentConnectionMode, AgentMessage, AgentRole, HubMessage, JobState};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

use crate::agents::dispatch;
use crate::registry::update_last_seen;
use crate::runs;
use crate::state::{AgentConnection, AgentTransport, HubState, OutboundAgentMessage};
use crate::{discard_agent_confirmations, handle_confirmation_request, room};

const AGENT_CONNECTION_SWEEP_SECS: u64 = 15;
const AGENT_CONNECTION_TTL_SECS: i64 = 60;

// `agents` is the admission and side-effect linearization guard. While it is
// held, acquire only short state locks in agents -> room/boot/jobs/
// confirmations order; do not await network, receivers, or spawned tasks, and
// do not call helpers that reacquire `state.agents`.
pub(super) async fn handle_agent_message(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    parsed: AgentMessage,
) -> std::result::Result<(), String> {
    let reliable = is_reliable_agent_message(&parsed);
    let mut agents = state.agents.lock().await;
    let is_current = agents
        .get(agent_id)
        .map(|connection| connection.connection_id == connection_id)
        .unwrap_or(false);
    if !reliable && !is_current {
        return Err("stale_connection".to_string());
    }
    if is_current {
        let transport = {
            let connection = agents
                .get_mut(agent_id)
                .expect("current connection disappeared under agents guard");
            connection.last_seen_at = chrono::Utc::now();
            connection.transport
        };
        if transport == AgentTransport::Sse {
            update_last_seen(state, agent_id).ok();
        }
    }

    if reliable {
        drop(agents);
        return dispatch::handle_reliable_message(state, agent_id, parsed).await;
    }

    let replay_sender = match parsed {
        AgentMessage::Hello {
            boot_generation,
            role,
            connection_mode,
            config_summary,
            notification_channels,
        } => match register_connection_mode(state, agent_id, connection_id, role, connection_mode)
            .await
        {
            Ok(()) => {
                let generation_changed = {
                    let mut generations = state.boot_generations.lock().await;
                    generations
                        .insert(agent_id.to_string(), boot_generation.clone())
                        .is_some_and(|previous| previous != boot_generation)
                };
                if generation_changed {
                    mark_cached_jobs_unknown_after_restart(state, agent_id).await;
                }
                let connection = agents
                    .get_mut(agent_id)
                    .expect("current connection disappeared under agents guard");
                connection.role = role;
                connection.connection_mode = connection_mode;
                connection.hello_received = true;
                connection.boot_generation = Some(boot_generation);
                connection.config_summary = Some(config_summary);
                connection.notification_channels = notification_channels;
                if connection_mode == AgentConnectionMode::CommandCapable {
                    Some(connection.sender.clone())
                } else {
                    None
                }
            }
            Err(reason) => {
                warn!(%agent_id, %connection_id, %reason, "room role rejected");
                let text = serde_json::to_string(&json!({
                    "error": { "code": reason, "message": reason }
                }))
                .map_err(|error| error.to_string())?;
                let sender = agents
                    .get(agent_id)
                    .expect("current connection disappeared under agents guard")
                    .sender
                    .clone();
                let _ = sender.send(OutboundAgentMessage::Text(text));
                let _ = sender.send(OutboundAgentMessage::Close);
                return Err(reason.to_string());
            }
        },
        AgentMessage::Heartbeat { sent_at } => {
            let ack = HubMessage::HeartbeatAck {
                sent_at,
                received_at: chrono::Utc::now(),
            };
            let text = serde_json::to_string(&ack).map_err(|error| error.to_string())?;
            let sender = agents
                .get(agent_id)
                .expect("current connection disappeared under agents guard")
                .sender
                .clone();
            let _ = sender.send(OutboundAgentMessage::Text(text));
            None
        }
        AgentMessage::JobUpdate { job } => {
            state
                .jobs
                .lock()
                .await
                .entry(agent_id.to_string())
                .or_default()
                .insert(job.job_id.clone(), job);
            None
        }
        AgentMessage::RunReport { report } => {
            if let Err(error) = runs::upsert_agent_report(state, agent_id, *report) {
                warn!(%agent_id, %error, "failed to store agent run report");
            }
            None
        }
        AgentMessage::ConfirmationRequest {
            request_id,
            agent_id: request_agent_id,
            timeout_seconds,
            payload,
        } => {
            if request_agent_id != agent_id {
                warn!(
                    %agent_id,
                    requestAgentId = %request_agent_id,
                    "rejected confirmation request with mismatched agentId"
                );
                let message = HubMessage::ConfirmationResponse {
                    request_id: request_id.clone(),
                    decision: agentic_gpt_protocol::ConfirmationDecision::ProviderUnavailable,
                    reason: "agent_id_mismatch".to_string(),
                };
                let text = serde_json::to_string(&message).map_err(|error| error.to_string())?;
                let sender = agents
                    .get(agent_id)
                    .expect("current connection disappeared under agents guard")
                    .sender
                    .clone();
                let _ = sender.send(OutboundAgentMessage::Text(text));
                return Err("agent_id_mismatch".to_string());
            }
            let state = state.clone();
            let agent_id = agent_id.to_string();
            tokio::spawn(async move {
                if let Err(error) = handle_confirmation_request(
                    state,
                    agent_id,
                    request_id,
                    timeout_seconds,
                    payload,
                )
                .await
                {
                    warn!(%error, "confirmation request failed");
                }
            });
            None
        }
        AgentMessage::Response { .. }
        | AgentMessage::TransportAck { .. }
        | AgentMessage::TransportRunStatus { .. } => {
            unreachable!("reliable message classification drifted")
        }
    };
    drop(agents);
    if let Some(sender) = replay_sender {
        dispatch::send_pending_replays(state, agent_id, &sender).await;
    }
    Ok(())
}

fn is_reliable_agent_message(message: &AgentMessage) -> bool {
    matches!(
        message,
        AgentMessage::Response { .. }
            | AgentMessage::TransportAck { .. }
            | AgentMessage::TransportRunStatus { .. }
    )
}

async fn register_connection_mode(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    role: AgentRole,
    connection_mode: AgentConnectionMode,
) -> std::result::Result<(), &'static str> {
    if connection_mode == AgentConnectionMode::ReportingOnly {
        room::release_active_room_for_agent(state, agent_id).await;
        return Ok(());
    }
    room::register_connection_role(state, agent_id, connection_id, role).await
}

pub(super) async fn disconnect_agent(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    expiry_check_at: Option<chrono::DateTime<chrono::Utc>>,
) -> bool {
    let removed_current_connection = {
        let mut agents = state.agents.lock().await;
        let should_remove = agents.get(agent_id).is_some_and(|connection| {
            connection.connection_id == connection_id
                && expiry_check_at.is_none_or(|now| {
                    now.signed_duration_since(connection.last_seen_at)
                        .num_seconds()
                        > AGENT_CONNECTION_TTL_SECS
                })
        });
        if should_remove {
            agents.remove(agent_id);
            room::release_active_room_if_current(state, agent_id, connection_id).await;
            discard_agent_confirmations(state, agent_id).await;
            true
        } else {
            false
        }
    };
    info!(%agent_id, %connection_id, removedCurrentConnection = removed_current_connection, "agent disconnected");
    removed_current_connection
}
pub(crate) async fn replace_agent_connection(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    transport: AgentTransport,
    sender: mpsc::UnboundedSender<OutboundAgentMessage>,
) -> std::result::Result<(), &'static str> {
    if connection_id.is_empty() {
        return Err("invalid_connection_id");
    }
    let mut agents = state.agents.lock().await;
    if agents
        .get(agent_id)
        .is_some_and(|connection| connection.connection_id == connection_id)
    {
        return Err("connection_id_in_use");
    }
    let old = agents.insert(
        agent_id.to_string(),
        AgentConnection {
            connection_id: connection_id.to_string(),
            sender,
            last_seen_at: chrono::Utc::now(),
            role: AgentRole::Normal,
            connection_mode: AgentConnectionMode::CommandCapable,
            hello_received: false,
            boot_generation: None,
            transport,
            config_summary: None,
            notification_channels: Vec::new(),
        },
    );
    if let Some(old) = old {
        room::release_active_room_if_current(state, agent_id, &old.connection_id).await;
        let _ = old.sender.send(OutboundAgentMessage::Close);
    }
    update_last_seen(state, agent_id).ok();
    Ok(())
}

pub(crate) async fn cleanup_agent_connections(state: HubState) {
    loop {
        sleep(Duration::from_secs(AGENT_CONNECTION_SWEEP_SECS)).await;
        cleanup_expired_agent_connections_once(&state, chrono::Utc::now()).await;
    }
}

pub(crate) async fn cleanup_expired_agent_connections_once(
    state: &HubState,
    now: chrono::DateTime<chrono::Utc>,
) {
    let expired = {
        let agents = state.agents.lock().await;
        agents
            .iter()
            .filter(|(_, connection)| {
                now.signed_duration_since(connection.last_seen_at)
                    .num_seconds()
                    > AGENT_CONNECTION_TTL_SECS
            })
            .map(|(agent_id, connection)| {
                (
                    agent_id.clone(),
                    connection.connection_id.clone(),
                    connection.last_seen_at,
                )
            })
            .collect::<Vec<_>>()
    };
    for (agent_id, connection_id, last_seen_at) in expired {
        if disconnect_agent(state, &agent_id, &connection_id, Some(now)).await {
            warn!(%agent_id, %connection_id, %last_seen_at, "agent connection expired");
        }
    }
}
async fn mark_cached_jobs_unknown_after_restart(state: &HubState, agent_id: &str) {
    let mut jobs = state.jobs.lock().await;
    let Some(agent_jobs) = jobs.get_mut(agent_id) else {
        return;
    };
    let now = chrono::Utc::now();
    for job in agent_jobs.values_mut() {
        if job.state.is_active() {
            job.state = JobState::UnknownAfterRestart;
            job.updated_at = now;
            job.finished_at = Some(now);
            job.reject_reason = Some("unknown_after_restart".to_string());
        }
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
pub(crate) mod tests;
