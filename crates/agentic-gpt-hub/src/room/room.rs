#[path = "control.rs"]
pub(crate) mod control;
#[path = "http.rs"]
pub(crate) mod http;

#[cfg(test)]
mod tests {
    use super::control::register_connection_role;
    use super::http::room_notebook_read;
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
        RoomNotebookReadRequest,
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
}
