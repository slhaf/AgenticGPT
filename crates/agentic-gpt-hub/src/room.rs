use agentic_gpt_protocol::{
    AgentRole, BootstrapReadRequest, HubCommand, NotebookAppendRequest, NotebookCurrentRequest,
    NotebookRecentRequest, NotebookRemoveRequest, NotebookSearchRequest,
    NotebookSelectExactRequest, NotebookUpdateRequest, SkillActivationRequest,
    SkillInstallCancelRequest, SkillInstallGetRequest, SkillInstallRequest, SkillReadRequest,
    SkillRunRequest, SkillSearchRequest,
};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use crate::agents::dispatch::request_room;
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

pub(crate) async fn room_bootstrap(State(state): State<HubState>, headers: HeaderMap) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomBootstrap {
            request_id: random_id("req"),
        },
        "room_bootstrap_timeout",
    )
    .await
}

pub(crate) async fn room_bootstrap_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<BootstrapReadRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomBootstrapRead {
            request_id: random_id("req"),
            payload,
        },
        "room_bootstrap_read_timeout",
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

pub(crate) async fn skills_install(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillInstallRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsInstall {
            request_id: random_id("req"),
            payload,
        },
        "skills_install_timeout",
    )
    .await
}

pub(crate) async fn skills_install_get(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillInstallGetRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsInstallGet {
            request_id: random_id("req"),
            payload,
        },
        "skills_install_get_timeout",
    )
    .await
}

pub(crate) async fn skills_install_cancel(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillInstallCancelRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsInstallCancel {
            request_id: random_id("req"),
            payload,
        },
        "skills_install_cancel_timeout",
    )
    .await
}

pub(crate) async fn skills_run(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<SkillRunRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::SkillsRun {
            request_id: random_id("req"),
            payload,
        },
        "skills_run_timeout",
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
        Ok(value) => room_value_response(value),
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

fn room_value_response(value: Value) -> Response {
    let Some(code) = value
        .get("error")
        .and_then(Value::as_object)
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
    else {
        return Json(value).into_response();
    };
    let status = match code {
        "target_exists" | "idempotency_conflict" | "room_state_conflict" => StatusCode::CONFLICT,
        "not_found"
        | "skill_not_found"
        | "install_not_found"
        | "bootstrap_not_found"
        | "guide_not_found"
        | "room_not_active" => StatusCode::NOT_FOUND,
        "bootstrap_read_failed" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(value)).into_response()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::lifecycle::replace_agent_connection;
    use crate::agents::transport::{post_agent_message, SseConnectQuery};
    use crate::db::init_db;
    use crate::registry::{handle_agent_command, AgentCommand};
    use crate::state::{AgentConnection, AgentTransport, OutboundAgentMessage};
    use crate::{HubConfig, McpProfile, RemoteConfirmationConfig};
    use agentic_gpt_protocol::{AgentConnectionMode, AgentMessage, HubCommand, HubCommandEnvelope};
    use axum::body::to_bytes;
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
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
            mcp_profile: McpProfile::Full,
            agents: Arc::new(crate::agents::lifecycle::Connections::new()),
            dispatch: Arc::new(crate::agents::dispatch::Dispatch::new()),
            confirmations: Arc::new(crate::confirmation::Confirmations::new()),
            job_cache: Arc::new(crate::state::JobCache::new()),
            boot_generations: Arc::new(Mutex::new(HashMap::new())),
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
        state
            .agents
            .insert_for_test(
                agent_id,
                AgentConnection {
                    connection_id: connection_id.to_string(),
                    sender: tx,
                    last_seen_at: Utc::now(),
                    role,
                    connection_mode: AgentConnectionMode::CommandCapable,
                    hello_received: true,
                    boot_generation: Some("testboot".to_string()),
                    transport: AgentTransport::WebSocket,
                    config_summary: None,
                    notification_channels: Vec::new(),
                },
            )
            .await;
        rx
    }

    #[test]
    fn bootstrap_error_codes_map_to_frozen_http_statuses() {
        assert_eq!(
            room_value_response(json!({
                "error": { "code": "bootstrap_not_found", "message": "missing" }
            }))
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            room_value_response(json!({
                "error": { "code": "guide_not_found", "message": "missing" }
            }))
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            room_value_response(json!({
                "error": { "code": "bootstrap_invalid", "message": "invalid" }
            }))
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            room_value_response(json!({
                "error": { "code": "bootstrap_read_failed", "message": "failed" }
            }))
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
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
        .await
        .unwrap();
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
        {
            let conn = state.db.lock().unwrap();
            handle_agent_command(
                &conn,
                AgentCommand::Add {
                    agent_id: "room".to_string(),
                    alias: None,
                    display_name: "Room".to_string(),
                    secret: "test-secret".to_string(),
                },
            )
            .unwrap();
        }
        let mut room_rx = insert_connection(&state, "room", "conn1", AgentRole::Room).await;
        let mut normal_rx = insert_connection(&state, "normal", "normal1", AgentRole::Normal).await;
        register_connection_role(&state, "room", "conn1", AgentRole::Room)
            .await
            .unwrap();

        let request_state = state.clone();
        let mut action_headers = HeaderMap::new();
        action_headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer test-api-key"),
        );
        let task = tokio::spawn(async move {
            room_notebook_current(
                State(request_state),
                action_headers,
                Json(NotebookCurrentRequest {
                    scope: "agentic".to_string(),
                }),
            )
            .await
        });
        let OutboundAgentMessage::Text(text) = room_rx.recv().await.unwrap() else {
            panic!("expected text command");
        };
        let envelope = serde_json::from_str::<HubCommandEnvelope>(&text).unwrap();
        assert!(matches!(
            &envelope.command,
            HubCommand::RoomNotebookCurrent { .. }
        ));

        let response_data = json!({ "current": null, "warnings": [] });
        let mut agent_headers = HeaderMap::new();
        agent_headers.insert("x-agent-secret", HeaderValue::from_static("test-secret"));
        let query: SseConnectQuery =
            serde_json::from_value(json!({ "connectionId": "conn1" })).unwrap();
        let response = post_agent_message(
            State(state.clone()),
            Path("room".to_string()),
            Query(query),
            agent_headers,
            axum::Json(AgentMessage::Response {
                run_id: Some(envelope.run_id.clone()),
                request_id: envelope.request_id.clone(),
                data: response_data.clone(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let route_response = task.await.unwrap();
        assert_eq!(route_response.status(), StatusCode::OK);
        let body = to_bytes(route_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value, response_data);

        assert!(matches!(
            normal_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        let run = crate::runs::get_run(&state, &envelope.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(run.agent_id, "room");
        assert_eq!(run.status, "completed");
        assert_eq!(run.result, Some(response_data));
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
