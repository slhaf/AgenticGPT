use agentic_gpt_protocol::{AgentConnectionMode, AgentMessage, AgentRole, HubMessage};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

use crate::agents::dispatch;
use crate::registry::update_last_seen;
use crate::runs;
use crate::state::{AgentConnection, AgentTransport, HubState, OutboundAgentMessage};
use crate::{confirmation, room};

pub(crate) struct Connections {
    current: tokio::sync::Mutex<std::collections::HashMap<String, AgentConnection>>,
}

pub(crate) struct AgentListEntry {
    pub(crate) agent_id: String,
    pub(crate) alias: Option<String>,
    pub(crate) display_name: String,
    pub(crate) capabilities: agentic_gpt_protocol::Capabilities,
    pub(crate) online: bool,
    pub(crate) transport: Option<AgentTransport>,
    pub(crate) connection_mode: Option<AgentConnectionMode>,
    pub(crate) last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) config_summary: agentic_gpt_protocol::SafeConfigSummary,
}

impl Connections {
    pub(crate) fn new() -> Self {
        Self {
            current: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub(crate) async fn online_count(&self) -> usize {
        self.current.lock().await.len()
    }

    pub(crate) async fn list_agents(
        &self,
        entries: &[agentic_gpt_protocol::AgentRegistryEntry],
    ) -> Vec<AgentListEntry> {
        let current = self.current.lock().await;
        entries
            .iter()
            .filter(|entry| entry.enabled)
            .map(|entry| {
                let connection = current.get(&entry.agent_id);
                AgentListEntry {
                    agent_id: entry.agent_id.clone(),
                    alias: entry.alias.clone(),
                    display_name: entry.display_name.clone(),
                    capabilities: entry.capabilities.clone(),
                    online: connection.is_some(),
                    transport: connection.map(|connection| connection.transport),
                    connection_mode: connection.map(|connection| connection.connection_mode),
                    last_seen_at: connection
                        .map(|connection| connection.last_seen_at)
                        .or(entry.last_seen_at),
                    config_summary: connection
                        .and_then(|connection| connection.config_summary.clone())
                        .unwrap_or_else(crate::config::default_config_summary),
                }
            })
            .collect()
    }

    pub(crate) async fn online_agents(
        &self,
        entries: &[agentic_gpt_protocol::AgentRegistryEntry],
    ) -> Vec<(String, String)> {
        let current = self.current.lock().await;
        entries
            .iter()
            .filter(|entry| entry.enabled && current.contains_key(&entry.agent_id))
            .map(|entry| (entry.agent_id.clone(), entry.display_name.clone()))
            .collect()
    }

    pub(crate) async fn notification_channels(
        &self,
        entries: &[agentic_gpt_protocol::AgentRegistryEntry],
    ) -> Vec<agentic_gpt_protocol::NotificationChannel> {
        let by_id = entries
            .iter()
            .map(|entry| (entry.agent_id.as_str(), entry))
            .collect::<std::collections::HashMap<_, _>>();
        let current = self.current.lock().await;
        let mut channels = Vec::new();
        for (agent_id, connection) in current.iter() {
            let Some(entry) = by_id.get(agent_id.as_str()) else {
                continue;
            };
            if !entry.enabled {
                continue;
            }
            let alias = entry.alias.as_deref().unwrap_or(&entry.agent_id);
            for channel in &connection.notification_channels {
                if channel.kind == "freedesktop" {
                    channels.push(agentic_gpt_protocol::NotificationChannel {
                        key: format!("agent::{alias}::freedesktop"),
                        display_name: format!("{} desktop notification", entry.display_name),
                        available: true,
                        kind: "freedesktop".to_string(),
                        supports_actions: channel.supports_actions,
                        reason: None,
                        agent_id: Some(entry.agent_id.clone()),
                    });
                }
            }
        }
        channels
    }

    #[cfg(test)]
    pub(crate) async fn insert_for_test(&self, agent_id: &str, connection: AgentConnection) {
        self.current
            .lock()
            .await
            .insert(agent_id.to_string(), connection);
    }

    #[cfg(test)]
    pub(crate) async fn snapshot_for_test(
        &self,
    ) -> std::collections::HashMap<String, AgentConnection> {
        self.current.lock().await.clone()
    }
}

#[derive(Clone, Debug)]
pub(super) struct DispatchTarget {
    pub(super) agent_id: String,
    pub(super) connection_id: String,
    pub(super) sender: mpsc::UnboundedSender<OutboundAgentMessage>,
}

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
    let mut agents = state.agents.current.lock().await;
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

    let mut confirmation_publication = None;
    let current_boot_generation = agents
        .get(agent_id)
        .and_then(|connection| connection.boot_generation.clone());
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
            if job.agent_id != agent_id {
                warn!(%agent_id, jobAgentId = %job.agent_id, "rejected JobUpdate for another agent");
                return Err("job_agent_id_mismatch".to_string());
            }
            state
                .job_cache
                .record(
                    agent_id,
                    connection_id,
                    current_boot_generation.as_deref(),
                    job,
                )
                .await;
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
            let connection = agents
                .get(agent_id)
                .expect("current connection disappeared under agents guard");
            let publication = state
                .confirmations
                .admit(
                    agent_id,
                    &connection.connection_id,
                    request_id,
                    connection.sender.clone(),
                    timeout_seconds,
                    state.config.remote_confirmation.timeout_seconds,
                    payload,
                )
                .await;
            confirmation_publication = Some(publication);
            None
        }
        AgentMessage::Response { .. }
        | AgentMessage::TransportAck { .. }
        | AgentMessage::TransportRunStatus { .. } => {
            unreachable!("reliable message classification drifted")
        }
    };
    drop(agents);
    if let Some(publication) = confirmation_publication {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = confirmation::handle_confirmation_request(state, publication).await
            {
                warn!(%error, "confirmation request failed");
            }
        });
    }
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
    let removed_connection = {
        let mut agents = state.agents.current.lock().await;
        let should_remove = agents.get(agent_id).is_some_and(|connection| {
            connection.connection_id == connection_id
                && expiry_check_at.is_none_or(|now| {
                    now.signed_duration_since(connection.last_seen_at)
                        .num_seconds()
                        > AGENT_CONNECTION_TTL_SECS
                })
        });
        if should_remove {
            let removed = agents
                .remove(agent_id)
                .expect("current connection disappeared under agents guard");
            state
                .job_cache
                .mark_connection_stale(agent_id, connection_id)
                .await;
            room::release_active_room_if_current(state, agent_id, connection_id).await;
            confirmation::retire_generation(state, agent_id, connection_id).await;
            Some(removed)
        } else {
            None
        }
    };
    let removed_current_connection = removed_connection.is_some();
    if let Some(connection) = removed_connection {
        let _ = connection.sender.send(OutboundAgentMessage::Close);
    }
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
    let old = {
        let mut agents = state.agents.current.lock().await;
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
            state
                .job_cache
                .mark_connection_stale(agent_id, &old.connection_id)
                .await;
            room::release_active_room_if_current(state, agent_id, &old.connection_id).await;
            confirmation::retire_generation(state, agent_id, &old.connection_id).await;
            Some(old)
        } else {
            None
        }
    };
    if let Some(old) = old {
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
        let agents = state.agents.current.lock().await;
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
pub(super) async fn resolve_command_target(
    state: &HubState,
    agent_id: &str,
) -> std::result::Result<DispatchTarget, String> {
    let current = state.agents.current.lock().await;
    command_target(&current, agent_id)
}

fn command_target(
    current: &std::collections::HashMap<String, AgentConnection>,
    agent_id: &str,
) -> std::result::Result<DispatchTarget, String> {
    let Some(connection) = current.get(agent_id) else {
        return Err("agent_offline".to_string());
    };
    if connection.connection_mode == AgentConnectionMode::ReportingOnly {
        return Err("agent_reporting_only".to_string());
    }
    if !connection.hello_received {
        return Err("agent_not_ready".to_string());
    }
    Ok(DispatchTarget {
        agent_id: agent_id.to_string(),
        connection_id: connection.connection_id.clone(),
        sender: connection.sender.clone(),
    })
}

pub(super) async fn resolve_room_target(
    state: &HubState,
) -> std::result::Result<DispatchTarget, room::RoomRouteError> {
    let current = state.agents.current.lock().await;
    let active = state
        .active_room
        .lock()
        .await
        .clone()
        .ok_or(room::RoomRouteError::NotActive)?;
    let Some(connection) = current.get(&active.agent_id) else {
        return Err(room::RoomRouteError::StateConflict);
    };
    if connection.connection_id != active.connection_id
        || connection.role != AgentRole::Room
        || connection.connection_mode != AgentConnectionMode::CommandCapable
    {
        return Err(room::RoomRouteError::StateConflict);
    }
    if !connection.hello_received {
        return Err(room::RoomRouteError::Timeout("agent_not_ready".to_string()));
    }
    Ok(DispatchTarget {
        agent_id: active.agent_id,
        connection_id: connection.connection_id.clone(),
        sender: connection.sender.clone(),
    })
}
async fn mark_cached_jobs_unknown_after_restart(state: &HubState, agent_id: &str) {
    state.job_cache.mark_unknown_after_restart(agent_id).await;
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
pub(crate) mod tests;
