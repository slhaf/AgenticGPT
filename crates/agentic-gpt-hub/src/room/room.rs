#[path = "control.rs"]
pub(crate) mod control;
#[path = "http.rs"]
pub(crate) mod http;

#[cfg(test)]
mod tests {
    use super::control::{
        register_connection_role, release_active_room_if_current, request_active_room,
        RoomRouteError,
    };
    use super::http::{room_notebook_read, room_value_response};
    use crate::agents::lifecycle::replace_agent_connection;
    use crate::agents::transport::{post_agent_message, SseConnectQuery};
    use crate::config::RemoteConfirmationConfig;
    use crate::db::init_db;
    use crate::registry::{handle_agent_command, AgentCommand};
    use crate::state::{
        AgentConnection, AgentTransport, HubState, McpProfile, OutboundAgentMessage,
    };
    use crate::HubConfig;
    use agentic_gpt_protocol::{
        AgentConnectionMode, AgentMessage, AgentRole, HubCommand, HubCommandEnvelope,
        RoomMaintenanceStatusRequest, RoomNotebookReadRequest,
    };
    use axum::body::to_bytes;
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::Json;
    use chrono::Utc;
    use rusqlite::Connection;
    use serde_json::{json, Value};
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
            process_cache: Arc::new(crate::state::ProcessCache::new()),
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
            HubCommand::RoomNotebookRead {
                request_id: "req".to_string(),
                payload: RoomNotebookReadRequest {
                    path: "Notebook/topic.md".to_string(),
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
            HubCommand::RoomNotebookRead {
                request_id: "req".to_string(),
                payload: RoomNotebookReadRequest {
                    path: "Notebook/topic.md".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }

    #[tokio::test]
    async fn read_and_maintenance_room_api_without_active_room_returns_not_active() {
        let state = test_state();
        let read = request_active_room(
            &state,
            HubCommand::RoomNotebookRead {
                request_id: "req-read".to_string(),
                payload: RoomNotebookReadRequest {
                    path: "Notebook/missing.md".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(read.unwrap_err(), RoomRouteError::NotActive);
        let maintenance = request_active_room(
            &state,
            HubCommand::RoomMaintenanceStatus {
                request_id: "req-status".to_string(),
                payload: RoomMaintenanceStatusRequest {},
            },
            1,
        )
        .await;
        assert_eq!(maintenance.unwrap_err(), RoomRouteError::NotActive);
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
            room_notebook_read(
                State(request_state),
                action_headers,
                Json(RoomNotebookReadRequest {
                    path: "Notebook/topic.md".to_string(),
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
            HubCommand::RoomNotebookRead { .. }
        ));

        let response_data = json!({ "path": "Notebook/topic.md", "content": "# Topic" });
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
                event_sources: vec![],
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
            HubCommand::RoomNotebookRead {
                request_id: "req".to_string(),
                payload: RoomNotebookReadRequest {
                    path: "Notebook/topic.md".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }
}
