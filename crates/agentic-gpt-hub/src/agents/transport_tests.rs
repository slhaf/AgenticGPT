use super::*;
use crate::agents::test_support::*;
use agentic_gpt_protocol::{AgentConnectionMode, AgentMessage, AgentRole, HubMessage};
use axum::body::to_bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use futures_util::StreamExt;
use rusqlite::params;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

use crate::agents::lifecycle::replace_agent_connection;
use crate::agents::lifecycle::tests::{generation_handle, generation_hello, generation_snapshot};
use crate::agents::transport::{connect_agent_sse, post_agent_message, SseConnectQuery};

#[tokio::test]
async fn generation_sse_rejects_duplicate_and_empty_ids() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let (current_tx, mut current_rx) = mpsc::unbounded_channel();
    replace_agent_connection(&state, "agent", "same", AgentTransport::Sse, current_tx)
        .await
        .unwrap();
    generation_handle(
        &state,
        "agent",
        "same",
        generation_hello(
            AgentRole::Room,
            AgentConnectionMode::CommandCapable,
            "boot-same",
            "same",
        ),
    )
    .await
    .unwrap();

    let before_duplicate = generation_snapshot(&state).await;
    let duplicate = connect_agent_sse(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("same".to_string()))),
        agent_headers("secret"),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let duplicate_body = to_bytes(duplicate.into_body(), usize::MAX).await.unwrap();
    let duplicate_value: Value = serde_json::from_slice(&duplicate_body).unwrap();
    assert_eq!(duplicate_value["error"]["code"], "connection_id_in_use");
    assert_eq!(generation_snapshot(&state).await, before_duplicate);

    generation_handle(
        &state,
        "agent",
        "same",
        AgentMessage::Heartbeat {
            sent_at: chrono::Utc::now(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(5), current_rx.recv())
            .await
            .expect("current heartbeat ack timed out"),
        Some(OutboundAgentMessage::Text(_))
    ));
    let before_empty = generation_snapshot(&state).await;

    let empty = connect_agent_sse(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some(String::new()))),
        agent_headers("secret"),
    )
    .await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    let empty_body = to_bytes(empty.into_body(), usize::MAX).await.unwrap();
    let empty_value: Value = serde_json::from_slice(&empty_body).unwrap();
    assert_eq!(empty_value["error"]["code"], "invalid_connection_id");
    assert_eq!(generation_snapshot(&state).await, before_empty);

    let fresh = connect_agent_sse(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("fresh".to_string()))),
        agent_headers("secret"),
    )
    .await;
    assert_eq!(fresh.status(), StatusCode::OK);
    assert!(matches!(
        timeout(Duration::from_secs(5), current_rx.recv())
            .await
            .expect("old SSE close timed out"),
        Some(OutboundAgentMessage::Close)
    ));
    let _fresh_response = fresh;
    assert_eq!(
        state
            .agents
            .lock()
            .await
            .get("agent")
            .map(|connection| connection.connection_id.as_str()),
        Some("fresh")
    );
}
#[tokio::test]
async fn current_sse_heartbeat_route_sends_ack_and_touches_registry() {
    let state = test_state();
    register_agent(&state, "agent", "secret");
    let response = connect_agent_sse(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("sse-route".to_string()))),
        agent_headers("secret"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "update agents set last_seen_at = ?2 where agent_id = ?1",
            params!["agent", "2000-01-01T00:00:00Z"],
        )
        .unwrap();
    }
    let before = registry_entry(&state, "agent")
        .unwrap()
        .unwrap()
        .last_seen_at
        .unwrap();
    let mut body = response.into_body().into_data_stream();
    let sent_at = chrono::Utc::now();
    let post_response = post_agent_message(
        State(state.clone()),
        Path("agent".to_string()),
        Query(SseConnectQuery::for_test(Some("sse-route".to_string()))),
        agent_headers("secret"),
        axum::Json(AgentMessage::Heartbeat { sent_at }),
    )
    .await;
    assert_eq!(post_response.status(), StatusCode::OK);

    let chunk = timeout(Duration::from_secs(5), body.next())
        .await
        .expect("SSE heartbeat body timed out")
        .expect("SSE heartbeat body ended")
        .expect("SSE heartbeat body failed");
    let text = String::from_utf8(chunk.to_vec()).unwrap();
    let data = text
        .lines()
        .find_map(|line| line.strip_prefix("data:").map(str::trim))
        .expect("SSE heartbeat response missing data");
    assert!(matches!(
        serde_json::from_str::<HubMessage>(data).unwrap(),
        HubMessage::HeartbeatAck {
            sent_at: ack_sent,
            ..
        } if ack_sent == sent_at
    ));

    let after = registry_entry(&state, "agent")
        .unwrap()
        .unwrap()
        .last_seen_at
        .unwrap();
    assert!(after > before);
}
