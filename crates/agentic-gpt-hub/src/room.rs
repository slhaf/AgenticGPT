use agentic_gpt_protocol::{
    AgentRole, HubCommand, NotebookAppendRequest, NotebookCurrentRequest, NotebookRecentRequest,
    NotebookRemoveRequest, NotebookSearchRequest, NotebookSelectExactRequest,
    NotebookUpdateRequest, SkillActivationRequest, SkillReadRequest, SkillSearchRequest,
};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use crate::agents::request_agent;
use crate::routes::{api_error, require_action_auth};
use crate::state::HubState;
use crate::utils::random_id;
use crate::REQUEST_TIMEOUT_SECS;

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

pub(crate) async fn room_notebook_append(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookAppendRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookAppend {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_append_timeout",
    )
    .await
}

pub(crate) async fn room_notebook_recent(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookRecentRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookRecent {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_recent_timeout",
    )
    .await
}

pub(crate) async fn room_notebook_select_exact(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookSelectExactRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookSelectExact {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_select_exact_timeout",
    )
    .await
}

pub(crate) async fn room_notebook_search(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookSearchRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookSearch {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_search_timeout",
    )
    .await
}

pub(crate) async fn room_notebook_current(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookCurrentRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookCurrent {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_current_timeout",
    )
    .await
}

pub(crate) async fn room_notebook_update(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookUpdateRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookUpdate {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_update_timeout",
    )
    .await
}

pub(crate) async fn room_notebook_remove(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookRemoveRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookRemove {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_remove_timeout",
    )
    .await
}

pub(crate) async fn skills_list(State(state): State<HubState>, headers: HeaderMap) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsList {
            request_id: random_id("req"),
        },
        "skills_list_timeout",
    )
    .await
}

pub(crate) async fn skills_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsRead {
            request_id: random_id("req"),
            payload,
        },
        "skills_read_timeout",
    )
    .await
}

pub(crate) async fn skills_search(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillSearchRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsSearch {
            request_id: random_id("req"),
            payload,
        },
        "skills_search_timeout",
    )
    .await
}

pub(crate) async fn skills_active(State(state): State<HubState>, headers: HeaderMap) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsActive {
            request_id: random_id("req"),
        },
        "skills_active_timeout",
    )
    .await
}

pub(crate) async fn skills_activate(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillActivationRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsActivate {
            request_id: random_id("req"),
            payload,
        },
        "skills_activate_timeout",
    )
    .await
}

pub(crate) async fn skills_deactivate(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillActivationRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsDeactivate {
            request_id: random_id("req"),
            payload,
        },
        "skills_deactivate_timeout",
    )
    .await
}

async fn forward_room_command(
    state: HubState,
    headers: HeaderMap,
    command: HubCommand,
    timeout_code: &'static str,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match request_active_room(&state, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(RoomRouteError::NotActive) => api_error(
            StatusCode::NOT_FOUND,
            "room_not_active",
            "no active room agent",
        ),
        Err(RoomRouteError::StateConflict) => api_error(
            StatusCode::CONFLICT,
            "room_state_conflict",
            "active room state is inconsistent",
        ),
        Err(RoomRouteError::Timeout(reason)) => {
            api_error(StatusCode::GATEWAY_TIMEOUT, timeout_code, reason)
        }
    }
}

pub(crate) async fn request_active_room(
    state: &HubState,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, RoomRouteError> {
    let active = state
        .active_room
        .lock()
        .await
        .clone()
        .ok_or(RoomRouteError::NotActive)?;
    let valid = {
        let agents = state.agents.lock().await;
        agents
            .get(&active.agent_id)
            .map(|connection| {
                connection.connection_id == active.connection_id
                    && connection.role == AgentRole::Room
            })
            .unwrap_or(false)
    };
    if !valid {
        return Err(RoomRouteError::StateConflict);
    }
    request_agent(state, &active.agent_id, command, timeout_secs)
        .await
        .map_err(RoomRouteError::Timeout)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::{command_request_id, replace_agent_connection};
    use crate::db::init_db;
    use crate::state::{AgentConnection, AgentTransport, OutboundAgentMessage};
    use crate::{HubConfig, RemoteConfirmationConfig};
    use agentic_gpt_protocol::{HubCommand, HubCommandEnvelope};
    use chrono::Utc;
    use rusqlite::Connection;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::{mpsc, Mutex};

    fn test_hub_config() -> HubConfig {
        HubConfig {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: true,
                provider: "ntfy".to_string(),
                timeout_seconds: 45,
                ntfy: crate::NtfyConfig {
                    server_url: "https://ntfy.example.invalid".to_string(),
                    topic: "secret-topic-for-test".to_string(),
                    callback_base_url: "https://callback.example.invalid".to_string(),
                },
            },
        }
    }

    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(test_hub_config()),
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: Some("https://hub.example.invalid".to_string()),
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(Some(crate::notify::NtfyHealthCache {
                server_url: "https://ntfy.example.invalid".to_string(),
                checked_at: Utc::now(),
                result: crate::notify::NtfyHealthStatus::Healthy,
            }))),
        }
    }

    async fn insert_connection(
        state: &HubState,
        agent_id: &str,
        connection_id: &str,
        role: AgentRole,
    ) -> mpsc::UnboundedReceiver<OutboundAgentMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        state.agents.lock().await.insert(
            agent_id.to_string(),
            AgentConnection {
                connection_id: connection_id.to_string(),
                sender: tx,
                last_seen_at: Utc::now(),
                role,
                transport: AgentTransport::WebSocket,
                config_summary: None,
                notification_channels: Vec::new(),
            },
        );
        rx
    }

    fn command_from_envelope(text: &str) -> HubCommand {
        serde_json::from_str::<HubCommandEnvelope>(text)
            .unwrap()
            .command
    }

    async fn replace_connection(
        state: &HubState,
        agent_id: &str,
        connection_id: &str,
    ) -> mpsc::UnboundedReceiver<OutboundAgentMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        replace_agent_connection(
            state,
            agent_id,
            connection_id,
            AgentTransport::WebSocket,
            tx,
        )
        .await;
        rx
    }

    #[tokio::test]
    async fn first_room_agent_becomes_active_room() {
        let state = test_state();
        let _rx = insert_connection(&state, "room", "conn1", AgentRole::Normal).await;
        register_connection_role(&state, "room", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let active = state.active_room.lock().await.clone().unwrap();
        assert_eq!(active.agent_id, "room");
        assert_eq!(active.connection_id, "conn1");
    }

    #[tokio::test]
    async fn second_different_room_agent_is_rejected() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room-a", "conn1", AgentRole::Room).await;
        register_connection_role(&state, "room-a", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = insert_connection(&state, "room-b", "conn2", AgentRole::Normal).await;
        assert_eq!(
            register_connection_role(&state, "room-b", "conn2", AgentRole::Room).await,
            Err("room_already_active")
        );
    }

    #[tokio::test]
    async fn same_room_agent_reconnect_replaces_old_room_connection() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "new").await;
        assert!(state.active_room.lock().await.is_none());
        register_connection_role(&state, "room", "new", AgentRole::Room)
            .await
            .unwrap();
        let active = state.active_room.lock().await.clone().unwrap();
        assert_eq!(active.connection_id, "new");
    }

    #[tokio::test]
    async fn same_agent_normal_hello_releases_old_active_room() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "normal").await;
        register_connection_role(&state, "room", "normal", AgentRole::Normal)
            .await
            .unwrap();
        assert!(state.active_room.lock().await.is_none());
    }

    #[tokio::test]
    async fn same_agent_replacement_without_hello_does_not_leave_stale_active_room() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "new-no-hello").await;
        assert!(state.active_room.lock().await.is_none());
        release_active_room_if_current(&state, "room", "new-no-hello").await;
        assert!(state.active_room.lock().await.is_none());
    }

    #[tokio::test]
    async fn room_api_after_replacement_without_hello_returns_not_active() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "new-no-hello").await;
        let result = request_active_room(
            &state,
            HubCommand::RoomNotebookCurrent {
                request_id: "req".to_string(),
                payload: NotebookCurrentRequest {
                    scope: "agentic".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }

    #[tokio::test]
    async fn stale_room_disconnect_does_not_release_new_room_connection() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = insert_connection(&state, "room", "new", AgentRole::Room).await;
        register_connection_role(&state, "room", "new", AgentRole::Room)
            .await
            .unwrap();
        release_active_room_if_current(&state, "room", "old").await;
        let active = state.active_room.lock().await.clone().unwrap();
        assert_eq!(active.connection_id, "new");
    }

    #[tokio::test]
    async fn room_api_without_active_room_returns_not_active() {
        let state = test_state();
        let result = request_active_room(
            &state,
            HubCommand::RoomNotebookCurrent {
                request_id: "req".to_string(),
                payload: NotebookCurrentRequest {
                    scope: "agentic".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }

    #[tokio::test]
    async fn update_remove_room_api_without_active_room_returns_not_active() {
        let state = test_state();
        let update = request_active_room(
            &state,
            HubCommand::RoomNotebookUpdate {
                request_id: "req-update".to_string(),
                payload: NotebookUpdateRequest {
                    id: "psg_missing".to_string(),
                    significance: None,
                    abstract_text: Some("updated".to_string()),
                    content: None,
                    tags: None,
                },
            },
            1,
        )
        .await;
        assert_eq!(update.unwrap_err(), RoomRouteError::NotActive);
        let remove = request_active_room(
            &state,
            HubCommand::RoomNotebookRemove {
                request_id: "req-remove".to_string(),
                payload: NotebookRemoveRequest {
                    id: "psg_missing".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(remove.unwrap_err(), RoomRouteError::NotActive);
    }

    #[tokio::test]
    async fn room_api_routes_to_active_room_connection() {
        let state = test_state();
        let mut rx = insert_connection(&state, "room", "conn1", AgentRole::Room).await;
        register_connection_role(&state, "room", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            request_active_room(
                &request_state,
                HubCommand::RoomNotebookCurrent {
                    request_id: "req".to_string(),
                    payload: NotebookCurrentRequest {
                        scope: "agentic".to_string(),
                    },
                },
                5,
            )
            .await
            .unwrap()
        });
        let OutboundAgentMessage::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text command");
        };
        let command = command_from_envelope(&text);
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::RoomNotebookCurrent { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({ "current": null, "warnings": [] }))
            .unwrap();
        let value = task.await.unwrap();
        assert_eq!(value["current"], Value::Null);
    }

    #[tokio::test]
    async fn update_remove_room_api_routes_to_active_room_connection() {
        let state = test_state();
        let mut rx = insert_connection(&state, "room", "conn1", AgentRole::Room).await;
        register_connection_role(&state, "room", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            request_active_room(
                &request_state,
                HubCommand::RoomNotebookUpdate {
                    request_id: "req-update".to_string(),
                    payload: NotebookUpdateRequest {
                        id: "psg_1".to_string(),
                        significance: None,
                        abstract_text: Some("updated".to_string()),
                        content: None,
                        tags: None,
                    },
                },
                5,
            )
            .await
            .unwrap()
        });
        let OutboundAgentMessage::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text command");
        };
        let command = command_from_envelope(&text);
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::RoomNotebookUpdate { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({ "updated": true, "id": "psg_1", "warnings": [] }))
            .unwrap();
        let value = task.await.unwrap();
        assert_eq!(value["updated"], true);

        let request_state = state.clone();
        let task = tokio::spawn(async move {
            request_active_room(
                &request_state,
                HubCommand::RoomNotebookRemove {
                    request_id: "req-remove".to_string(),
                    payload: NotebookRemoveRequest {
                        id: "psg_1".to_string(),
                    },
                },
                5,
            )
            .await
            .unwrap()
        });
        let OutboundAgentMessage::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text command");
        };
        let command = command_from_envelope(&text);
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::RoomNotebookRemove { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({ "removed": true, "id": "psg_1", "warnings": [] }))
            .unwrap();
        let value = task.await.unwrap();
        assert_eq!(value["removed"], true);
    }

    #[tokio::test]
    async fn normal_agent_is_not_room_api_fallback() {
        let state = test_state();
        let _rx = insert_connection(&state, "normal", "conn1", AgentRole::Normal).await;
        let result = request_active_room(
            &state,
            HubCommand::RoomNotebookCurrent {
                request_id: "req".to_string(),
                payload: NotebookCurrentRequest {
                    scope: "agentic".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }
}
