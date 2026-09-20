use agentic_gpt_protocol::AgentMessage;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, Stream, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tracing::{info, warn};

use crate::agents::lifecycle;
use crate::api_error;
use crate::registry::registry_entry;
use crate::state::{AgentTransport, HubState, OutboundAgentMessage};
use crate::utils::{constant_time_equal, random_id, sha256_hex};

pub(crate) async fn connect_agent(
    State(state): State<HubState>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(response) = require_agent_secret(&state, &agent_id, &headers) {
        return response;
    }
    ws.on_upgrade(move |socket| handle_socket(state, agent_id, socket))
        .into_response()
}

#[derive(Deserialize)]
pub(crate) struct SseConnectQuery {
    #[serde(rename = "connectionId")]
    connection_id: Option<String>,
}

#[cfg(test)]
impl SseConnectQuery {
    pub(crate) fn for_test(connection_id: Option<String>) -> Self {
        Self { connection_id }
    }
}

pub(crate) async fn connect_agent_sse(
    State(state): State<HubState>,
    Path(agent_id): Path<String>,
    Query(query): Query<SseConnectQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = require_agent_secret(&state, &agent_id, &headers) {
        return response;
    }
    let connection_id = query.connection_id.unwrap_or_else(|| random_id("conn"));
    info!(%agent_id, %connection_id, "agent sse connected");
    let (tx, rx) = mpsc::unbounded_channel::<OutboundAgentMessage>();
    if let Err(reason) = lifecycle::replace_agent_connection(
        &state,
        &agent_id,
        &connection_id,
        AgentTransport::Sse,
        tx.clone(),
    )
    .await
    {
        return match reason {
            "invalid_connection_id" => {
                api_error(StatusCode::BAD_REQUEST, "invalid_connection_id", reason)
            }
            "connection_id_in_use" => {
                api_error(StatusCode::CONFLICT, "connection_id_in_use", reason)
            }
            _ => api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "connection_rejected",
                reason,
            ),
        };
    }
    let stream = sse_stream(rx);
    let mut response = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

pub(crate) async fn post_agent_message(
    State(state): State<HubState>,
    Path(agent_id): Path<String>,
    Query(query): Query<SseConnectQuery>,
    headers: HeaderMap,
    axum::Json(message): axum::Json<AgentMessage>,
) -> Response {
    if let Err(response) = require_agent_secret(&state, &agent_id, &headers) {
        return response;
    }
    let connection_id = query.connection_id.unwrap_or_default();
    match lifecycle::handle_agent_message(&state, &agent_id, &connection_id, message).await {
        Ok(()) => axum::Json(json!({ "ok": true })).into_response(),
        Err(reason) if reason == "stale_connection" => {
            api_error(StatusCode::CONFLICT, "stale_connection", reason)
        }
        Err(reason) => api_error(StatusCode::BAD_REQUEST, "agent_message_rejected", reason),
    }
}

fn sse_stream(
    rx: mpsc::UnboundedReceiver<OutboundAgentMessage>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    futures_util::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(OutboundAgentMessage::Text(text)) => {
                Some((Ok(Event::default().event("message").data(text)), rx))
            }
            Some(OutboundAgentMessage::Close) | None => None,
        }
    })
}

#[allow(clippy::result_large_err)]
fn require_agent_secret(
    state: &HubState,
    agent_id: &str,
    headers: &HeaderMap,
) -> std::result::Result<(), Response> {
    let secret = headers
        .get("x-agent-secret")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    match registry_entry(state, agent_id) {
        Ok(Some(entry))
            if entry.enabled && constant_time_equal(&sha256_hex(secret), &entry.secret_hash) =>
        {
            Ok(())
        }
        Ok(Some(_)) => Err(api_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized_agent",
            "Invalid agent secret",
        )),
        Ok(None) => Err(api_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "Agent is not registered or enabled",
        )),
        Err(error) => Err(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_error",
            error,
        )),
    }
}

async fn handle_socket(state: HubState, agent_id: String, socket: WebSocket) {
    let connection_id = random_id("conn");
    info!(%agent_id, %connection_id, "agent connected");
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<OutboundAgentMessage>();
    if let Err(reason) = lifecycle::replace_agent_connection(
        &state,
        &agent_id,
        &connection_id,
        AgentTransport::WebSocket,
        tx.clone(),
    )
    .await
    {
        warn!(%agent_id, %connection_id, %reason, "agent connection rejected");
        let _ = sink.send(Message::Close(None)).await;
        return;
    }

    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                OutboundAgentMessage::Text(text) => {
                    if sink.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                OutboundAgentMessage::Close => {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
        }
    });

    while let Some(message) = stream.next().await {
        let Ok(Message::Text(text)) = message else {
            continue;
        };
        let parsed = match serde_json::from_str::<AgentMessage>(&text) {
            Ok(parsed) => parsed,
            Err(error) => {
                warn!(%agent_id, %error, "ignored invalid agent message");
                continue;
            }
        };
        if let Err(reason) =
            lifecycle::handle_agent_message(&state, &agent_id, &connection_id, parsed).await
        {
            warn!(%agent_id, %connection_id, %reason, "agent message rejected");
        }
    }

    writer.abort();
    let _ = lifecycle::disconnect_agent(&state, &agent_id, &connection_id, None).await;
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
