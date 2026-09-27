use agentic_gpt_protocol::{AgentRole, HubCommand};
use serde_json::Value;

use crate::agents::dispatch::request_room;
use crate::state::HubState;

#[derive(Clone, Debug)]
pub(crate) struct ActiveRoomConnection {
    pub(crate) agent_id: String,
    pub(crate) connection_id: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RoomRouteError {
    NotActive,
    StateConflict,
    Timeout(String),
}

pub(crate) async fn request_active_room(
    state: &HubState,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, RoomRouteError> {
    request_room(state, command, timeout_secs).await
}

pub(crate) async fn register_connection_role(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    role: AgentRole,
) -> std::result::Result<(), &'static str> {
    match role {
        AgentRole::Normal => {
            release_active_room_for_agent(state, agent_id).await;
            Ok(())
        }
        AgentRole::Room => {
            let mut active = state.active_room.lock().await;
            match active.as_ref() {
                None => {
                    *active = Some(ActiveRoomConnection {
                        agent_id: agent_id.to_string(),
                        connection_id: connection_id.to_string(),
                    });
                    Ok(())
                }
                Some(current) if current.agent_id == agent_id => {
                    *active = Some(ActiveRoomConnection {
                        agent_id: agent_id.to_string(),
                        connection_id: connection_id.to_string(),
                    });
                    Ok(())
                }
                Some(_) => Err("room_already_active"),
            }
        }
    }
}

pub(crate) async fn release_active_room_if_current(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
) {
    let mut active = state.active_room.lock().await;
    let should_release = active
        .as_ref()
        .map(|current| current.agent_id == agent_id && current.connection_id == connection_id)
        .unwrap_or(false);
    if should_release {
        *active = None;
    }
}

pub(crate) async fn release_active_room_for_agent(state: &HubState, agent_id: &str) {
    let mut active = state.active_room.lock().await;
    if active
        .as_ref()
        .map(|current| current.agent_id == agent_id)
        .unwrap_or(false)
    {
        *active = None;
    }
}
