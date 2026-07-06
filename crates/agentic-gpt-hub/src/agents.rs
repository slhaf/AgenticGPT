use agentic_gpt_protocol::{
    AgentMessage, AgentRole, BatchExecRequest, BatchExecResult, HubCommand, HubCommandEnvelope,
    HubMessage, SessionInfo, TaskResult,
};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{sleep, timeout, Duration};
use tracing::{info, warn};

use crate::registry::{registry_entries, registry_entry, update_last_seen};
use crate::runs;
use crate::state::{AgentConnection, AgentTransport, HubState, OutboundAgentMessage};
use crate::utils::{constant_time_equal, random_id, sha256_hex};
use crate::{
    api_error, discard_agent_confirmations, handle_confirmation_request, room,
    send_confirmation_response, REQUEST_TIMEOUT_SECS,
};

const AGENT_CONNECTION_SWEEP_SECS: u64 = 15;
const AGENT_CONNECTION_TTL_SECS: i64 = 60;

pub(crate) async fn connect_agent(
    State(state): State<HubState>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let secret = headers
        .get("x-agent-secret")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    match registry_entry(&state, &agent_id) {
        Ok(Some(entry))
            if entry.enabled && constant_time_equal(&sha256_hex(secret), &entry.secret_hash) =>
        {
            update_last_seen(&state, &agent_id).ok();
            ws.on_upgrade(move |socket| handle_socket(state, agent_id, socket))
                .into_response()
        }
        Ok(Some(_)) => api_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized_agent",
            "Invalid agent secret",
        ),
        Ok(None) => api_error(
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "Agent is not registered or enabled",
        ),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

#[derive(Deserialize)]
pub(crate) struct SseConnectQuery {
    #[serde(rename = "connectionId")]
    connection_id: Option<String>,
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
    update_last_seen(&state, &agent_id).ok();
    let connection_id = query.connection_id.unwrap_or_else(|| random_id("conn"));
    info!(%agent_id, %connection_id, "agent sse connected");
    let (tx, rx) = mpsc::unbounded_channel::<OutboundAgentMessage>();
    replace_agent_connection(
        &state,
        &agent_id,
        &connection_id,
        AgentTransport::Sse,
        tx.clone(),
    )
    .await;
    send_pending_replays(&state, &agent_id, &tx).await;
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
    let is_current = is_current_connection(&state, &agent_id, &connection_id).await;
    if !is_current && !is_reliable_agent_message(&message) {
        return api_error(StatusCode::CONFLICT, "stale_connection", "stale_connection");
    }
    if is_current {
        update_last_seen(&state, &agent_id).ok();
    }
    match handle_agent_message(&state, &agent_id, &connection_id, message, is_current).await {
        Ok(()) => axum::Json(json!({ "ok": true })).into_response(),
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
    replace_agent_connection(
        &state,
        &agent_id,
        &connection_id,
        AgentTransport::WebSocket,
        tx.clone(),
    )
    .await;
    send_pending_replays(&state, &agent_id, &tx).await;

    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            match message {
                OutboundAgentMessage::Text(text) => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
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
            handle_agent_message(&state, &agent_id, &connection_id, parsed, true).await
        {
            warn!(%agent_id, %connection_id, %reason, "agent message rejected");
        }
    }

    writer.abort();
    disconnect_agent(&state, &agent_id, &connection_id).await;
}

async fn handle_agent_message(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    parsed: AgentMessage,
    touch_current: bool,
) -> std::result::Result<(), String> {
    if touch_current {
        touch_agent(state, agent_id).await;
    }
    match parsed {
        AgentMessage::Hello {
            role,
            config_summary,
            notification_channels,
        } => match room::register_connection_role(state, agent_id, connection_id, role).await {
            Ok(()) => {
                let mut agents = state.agents.lock().await;
                if let Some(connection) = agents.get_mut(agent_id) {
                    connection.role = role;
                    connection.config_summary = Some(config_summary);
                    connection.notification_channels = notification_channels;
                }
            }
            Err(reason) => {
                warn!(%agent_id, %connection_id, %reason, "room role rejected");
                send_to_connection(
                    state,
                    agent_id,
                    connection_id,
                    serde_json::to_string(&json!({
                        "error": { "code": reason, "message": reason }
                    }))
                    .map_err(|error| error.to_string())?,
                )
                .await;
                close_connection(state, agent_id, connection_id).await;
                return Err(reason.to_string());
            }
        },
        AgentMessage::Heartbeat { sent_at } => {
            let ack = HubMessage::HeartbeatAck {
                sent_at,
                received_at: chrono::Utc::now(),
            };
            send_to_connection(
                state,
                agent_id,
                connection_id,
                serde_json::to_string(&ack).map_err(|error| error.to_string())?,
            )
            .await;
        }
        AgentMessage::SessionUpdate { session } => {
            state
                .sessions
                .lock()
                .await
                .entry(agent_id.to_string())
                .or_default()
                .insert(session.session_id.clone(), session);
        }
        AgentMessage::Response {
            run_id,
            request_id,
            data,
        } => {
            if let Err(error) =
                runs::store_result(state, agent_id, run_id.as_deref(), &request_id, &data)
            {
                warn!(%agent_id, %request_id, %error, "failed to store agent result");
            }
            if let Some(sender) = state.pending.lock().await.remove(&request_id) {
                let _ = sender.send(data);
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
                send_confirmation_response(
                    state,
                    agent_id,
                    &request_id,
                    agentic_gpt_protocol::ConfirmationDecision::ProviderUnavailable,
                    "agent_id_mismatch",
                )
                .await;
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
        }
    }
    Ok(())
}

async fn is_current_connection(state: &HubState, agent_id: &str, connection_id: &str) -> bool {
    state
        .agents
        .lock()
        .await
        .get(agent_id)
        .map(|connection| connection.connection_id == connection_id)
        .unwrap_or(false)
}

fn is_reliable_agent_message(message: &AgentMessage) -> bool {
    matches!(
        message,
        AgentMessage::Response { .. }
            | AgentMessage::TransportAck { .. }
            | AgentMessage::TransportRunStatus { .. }
    )
}

async fn send_to_connection(state: &HubState, agent_id: &str, connection_id: &str, text: String) {
    let sender = {
        let agents = state.agents.lock().await;
        agents
            .get(agent_id)
            .filter(|connection| connection.connection_id == connection_id)
            .map(|connection| connection.sender.clone())
    };
    if let Some(sender) = sender {
        let _ = sender.send(OutboundAgentMessage::Text(text));
    }
}

async fn close_connection(state: &HubState, agent_id: &str, connection_id: &str) {
    let sender = {
        let agents = state.agents.lock().await;
        agents
            .get(agent_id)
            .filter(|connection| connection.connection_id == connection_id)
            .map(|connection| connection.sender.clone())
    };
    if let Some(sender) = sender {
        let _ = sender.send(OutboundAgentMessage::Close);
    }
}

async fn disconnect_agent(state: &HubState, agent_id: &str, connection_id: &str) {
    let removed_current_connection = {
        let mut agents = state.agents.lock().await;
        let should_remove = agents
            .get(agent_id)
            .map(|connection| connection.connection_id == connection_id)
            .unwrap_or(false);
        if should_remove {
            agents.remove(agent_id);
            true
        } else {
            false
        }
    };
    if removed_current_connection {
        room::release_active_room_if_current(state, agent_id, connection_id).await;
        discard_agent_confirmations(state, agent_id).await;
    }
    info!(%agent_id, %connection_id, removedCurrentConnection = removed_current_connection, "agent disconnected");
}

pub(crate) async fn request_agent(
    state: &HubState,
    agent_id: &str,
    mut command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, String> {
    let request_id = command_request_id(&command).to_string();
    set_command_request_id(&mut command, request_id.clone());
    let sender = {
        let agents = state.agents.lock().await;
        agents
            .get(agent_id)
            .map(|connection| (connection.connection_id.clone(), connection.sender.clone()))
            .ok_or_else(|| "agent_offline".to_string())?
    };
    let run = runs::prepare_run(state, agent_id, &request_id, &command)
        .map_err(|error| error.to_string())?;
    let (tx, rx) = oneshot::channel();
    state.pending.lock().await.insert(request_id.clone(), tx);
    let text = envelope_text(
        &run.run_id,
        &run.request_id,
        &run.command_hash,
        command.clone(),
    )
    .map_err(|error| error.to_string())?;
    if sender.1.send(OutboundAgentMessage::Text(text)).is_err() {
        state.pending.lock().await.remove(&request_id);
        disconnect_agent(state, agent_id, &sender.0).await;
        return Err("agent_offline".to_string());
    }
    if let Err(error) = runs::mark_dispatched(state, &run.run_id) {
        warn!(runId = %run.run_id, %error, "failed to mark run dispatched");
    }
    match timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(value)) => Ok(value),
        _ => {
            state.pending.lock().await.remove(&request_id);
            if let Err(error) = runs::mark_timeout(state, &run.run_id, "exec_timeout_use_session") {
                warn!(runId = %run.run_id, %error, "failed to mark run timeout");
            }
            Err(format!("exec_timeout_use_session; runId={}", run.run_id))
        }
    }
}

async fn send_pending_replays(
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

pub(crate) async fn replace_agent_connection(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    transport: AgentTransport,
    sender: mpsc::UnboundedSender<OutboundAgentMessage>,
) {
    let old = {
        let mut agents = state.agents.lock().await;
        agents.insert(
            agent_id.to_string(),
            AgentConnection {
                connection_id: connection_id.to_string(),
                sender,
                last_seen_at: chrono::Utc::now(),
                role: AgentRole::Normal,
                transport,
                config_summary: None,
                notification_channels: Vec::new(),
            },
        )
    };
    if let Some(old) = old {
        room::release_active_room_if_current(state, agent_id, &old.connection_id).await;
        let _ = old.sender.send(OutboundAgentMessage::Close);
    }
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
        warn!(%agent_id, %connection_id, %last_seen_at, "agent connection expired");
        disconnect_agent(state, &agent_id, &connection_id).await;
    }
}

pub(crate) async fn mcp_list_servers_all_agents(
    state: &HubState,
) -> std::result::Result<Value, String> {
    let entries = registry_entries(state).map_err(|error| error.to_string())?;
    let online_agent_ids = {
        let online = state.agents.lock().await;
        entries
            .into_iter()
            .filter(|entry| entry.enabled && online.contains_key(&entry.agent_id))
            .map(|entry| (entry.agent_id, entry.display_name))
            .collect::<Vec<_>>()
    };

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

pub(crate) fn command_request_id(command: &HubCommand) -> &str {
    match command {
        HubCommand::Exec { request_id, .. }
        | HubCommand::BatchExec { request_id, .. }
        | HubCommand::StartSession { request_id, .. }
        | HubCommand::ListSessions { request_id }
        | HubCommand::InspectSession { request_id, .. }
        | HubCommand::WaitSession { request_id, .. }
        | HubCommand::KillSession { request_id, .. }
        | HubCommand::TmuxListSessions { request_id }
        | HubCommand::TmuxListPanes { request_id, .. }
        | HubCommand::TmuxCapturePane { request_id, .. }
        | HubCommand::TmuxPasteText { request_id, .. }
        | HubCommand::TmuxExec { request_id, .. }
        | HubCommand::TmuxCreateSession { request_id, .. }
        | HubCommand::TmuxCloseSession { request_id, .. }
        | HubCommand::McpListServers { request_id }
        | HubCommand::McpListTools { request_id, .. }
        | HubCommand::McpCallTool { request_id, .. }
        | HubCommand::UserNotifyDeliver { request_id, .. }
        | HubCommand::RoomNotebookAppend { request_id, .. }
        | HubCommand::RoomNotebookRecent { request_id, .. }
        | HubCommand::RoomNotebookSelectExact { request_id, .. }
        | HubCommand::RoomNotebookSearch { request_id, .. }
        | HubCommand::RoomNotebookCurrent { request_id, .. }
        | HubCommand::RoomNotebookUpdate { request_id, .. }
        | HubCommand::RoomNotebookRemove { request_id, .. }
        | HubCommand::RoomDiaryAppend { request_id, .. }
        | HubCommand::RoomDiaryRecent { request_id, .. }
        | HubCommand::RoomDiarySelectExact { request_id, .. }
        | HubCommand::SkillsList { request_id }
        | HubCommand::SkillsRead { request_id, .. }
        | HubCommand::SkillsSearch { request_id, .. }
        | HubCommand::SkillsActive { request_id }
        | HubCommand::SkillsActivate { request_id, .. }
        | HubCommand::SkillsDeactivate { request_id, .. } => request_id,
    }
}

pub(crate) fn set_command_request_id(command: &mut HubCommand, value: String) {
    match command {
        HubCommand::Exec { request_id, .. }
        | HubCommand::BatchExec { request_id, .. }
        | HubCommand::StartSession { request_id, .. }
        | HubCommand::ListSessions { request_id }
        | HubCommand::InspectSession { request_id, .. }
        | HubCommand::WaitSession { request_id, .. }
        | HubCommand::KillSession { request_id, .. }
        | HubCommand::TmuxListSessions { request_id }
        | HubCommand::TmuxListPanes { request_id, .. }
        | HubCommand::TmuxCapturePane { request_id, .. }
        | HubCommand::TmuxPasteText { request_id, .. }
        | HubCommand::TmuxExec { request_id, .. }
        | HubCommand::TmuxCreateSession { request_id, .. }
        | HubCommand::TmuxCloseSession { request_id, .. }
        | HubCommand::McpListServers { request_id }
        | HubCommand::McpListTools { request_id, .. }
        | HubCommand::McpCallTool { request_id, .. }
        | HubCommand::UserNotifyDeliver { request_id, .. }
        | HubCommand::RoomNotebookAppend { request_id, .. }
        | HubCommand::RoomNotebookRecent { request_id, .. }
        | HubCommand::RoomNotebookSelectExact { request_id, .. }
        | HubCommand::RoomNotebookSearch { request_id, .. }
        | HubCommand::RoomNotebookCurrent { request_id, .. }
        | HubCommand::RoomNotebookUpdate { request_id, .. }
        | HubCommand::RoomNotebookRemove { request_id, .. }
        | HubCommand::RoomDiaryAppend { request_id, .. }
        | HubCommand::RoomDiaryRecent { request_id, .. }
        | HubCommand::RoomDiarySelectExact { request_id, .. }
        | HubCommand::SkillsList { request_id }
        | HubCommand::SkillsRead { request_id, .. }
        | HubCommand::SkillsSearch { request_id, .. }
        | HubCommand::SkillsActive { request_id }
        | HubCommand::SkillsActivate { request_id, .. }
        | HubCommand::SkillsDeactivate { request_id, .. } => *request_id = value,
    }
}

pub(crate) async fn cached_session(
    state: &HubState,
    agent_id: &str,
    session_id: &str,
) -> Option<SessionInfo> {
    state
        .sessions
        .lock()
        .await
        .get(agent_id)
        .and_then(|sessions| sessions.get(session_id).cloned())
}

async fn touch_agent(state: &HubState, agent_id: &str) {
    if let Some(connection) = state.agents.lock().await.get_mut(agent_id) {
        connection.last_seen_at = chrono::Utc::now();
    }
}

pub(crate) fn timeout_task_result(agent_id: &str, task_id: &str, reason: String) -> TaskResult {
    let at = chrono::Utc::now();
    TaskResult {
        agent_id: agent_id.to_string(),
        task_id: task_id.to_string(),
        status: if reason == "agent_offline" {
            "failed"
        } else {
            "timeout"
        }
        .to_string(),
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: Some(reason),
        started_at: at,
        updated_at: at,
    }
}

pub(crate) fn timeout_batch_result(
    payload: &BatchExecRequest,
    batch_id: &str,
    reason: String,
) -> BatchExecResult {
    let at = chrono::Utc::now();
    let status = if reason == "agent_offline" {
        "partial_failed"
    } else {
        "timeout"
    };
    BatchExecResult {
        agent_id: payload.agent_id.clone(),
        batch_id: batch_id.to_string(),
        status: status.to_string(),
        results: payload
            .elements
            .iter()
            .enumerate()
            .map(
                |(index, element)| agentic_gpt_protocol::BatchElementResult {
                    index,
                    program: element.program.clone(),
                    args: element.args.clone(),
                    working_directory: element
                        .working_directory
                        .clone()
                        .or_else(|| payload.working_directory.clone()),
                    result: timeout_task_result(
                        &payload.agent_id,
                        &format!("{batch_id}:element:{index}"),
                        reason.clone(),
                    ),
                },
            )
            .collect(),
        started_at: at,
        updated_at: at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::{HubConfig, NtfyConfig, RemoteConfirmationConfig};
    use agentic_gpt_protocol::{Capabilities, ExecRequest};
    use axum::http::HeaderValue;
    use rusqlite::{params, Connection};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(HubConfig {
                remote_confirmation: RemoteConfirmationConfig {
                    enabled: false,
                    provider: "none".to_string(),
                    timeout_seconds: 45,
                    ntfy: NtfyConfig {
                        server_url: String::new(),
                        topic: String::new(),
                        callback_base_url: String::new(),
                    },
                },
            }),
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: None,
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(None)),
        }
    }

    fn register_agent(state: &HubState, agent_id: &str, secret: &str) {
        let conn = state.db.lock().unwrap();
        let capabilities = Capabilities {
            sessions: true,
            confirmation: true,
            notification_actions: true,
        };
        conn.execute(
            "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
             values (?1, null, ?1, 1, ?2, null, ?3)",
            params![
                agent_id,
                sha256_hex(secret),
                serde_json::to_string(&capabilities).unwrap()
            ],
        )
        .unwrap();
    }

    fn agent_headers(secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-agent-secret", HeaderValue::from_str(secret).unwrap());
        headers
    }

    async fn insert_connection(
        state: &HubState,
        agent_id: &str,
        connection_id: &str,
        last_seen_at: chrono::DateTime<chrono::Utc>,
    ) -> mpsc::UnboundedReceiver<OutboundAgentMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        state.agents.lock().await.insert(
            agent_id.to_string(),
            AgentConnection {
                connection_id: connection_id.to_string(),
                sender: tx,
                last_seen_at,
                role: AgentRole::Normal,
                transport: AgentTransport::Sse,
                config_summary: None,
                notification_channels: Vec::new(),
            },
        );
        rx
    }

    #[tokio::test]
    async fn pending_replay_sends_reliable_envelope() {
        let state = test_state();
        let command = HubCommand::Exec {
            request_id: "req_replay".to_string(),
            task_id: "task_replay".to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
            },
        };
        let run = runs::prepare_run(&state, "agent", "req_replay", &command).unwrap();
        runs::mark_dispatched(&state, &run.run_id).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();

        send_pending_replays(&state, "agent", &tx).await;

        let OutboundAgentMessage::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected replay envelope");
        };
        let envelope = serde_json::from_str::<HubCommandEnvelope>(&text).unwrap();
        assert_eq!(envelope.run_id, run.run_id);
        assert_eq!(envelope.request_id, "req_replay");
        assert_eq!(envelope.command_hash, run.command_hash);
        assert!(matches!(envelope.command, HubCommand::Exec { .. }));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn stale_heartbeat_is_rejected_without_touching_current_connection() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let previous_seen = chrono::Utc::now() - chrono::Duration::seconds(10);
        let _rx = insert_connection(&state, "agent", "current", previous_seen).await;

        let response = post_agent_message(
            State(state.clone()),
            Path("agent".to_string()),
            Query(SseConnectQuery {
                connection_id: Some("old".to_string()),
            }),
            agent_headers("secret"),
            axum::Json(AgentMessage::Heartbeat {
                sent_at: chrono::Utc::now(),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let agents = state.agents.lock().await;
        let connection = agents.get("agent").unwrap();
        assert_eq!(connection.connection_id, "current");
        assert_eq!(connection.last_seen_at, previous_seen);
    }

    #[tokio::test]
    async fn stale_session_update_is_rejected_without_writing_session_cache() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let _rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;

        let response = post_agent_message(
            State(state.clone()),
            Path("agent".to_string()),
            Query(SseConnectQuery {
                connection_id: Some("old".to_string()),
            }),
            agent_headers("secret"),
            axum::Json(AgentMessage::SessionUpdate {
                session: SessionInfo {
                    agent_id: "agent".to_string(),
                    session_id: "session-old".to_string(),
                    program: "sleep".to_string(),
                    args: vec!["10".to_string()],
                    working_directory: None,
                    command_preview: "sleep 10".to_string(),
                    state: "running".to_string(),
                    started_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    exit_code: None,
                    stdout_tail: String::new(),
                    stderr_tail: String::new(),
                    truncated: false,
                    reject_reason: None,
                },
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(state.sessions.lock().await.is_empty());
    }

    #[tokio::test]
    async fn stale_response_with_matching_run_is_accepted() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let _rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        let command = HubCommand::Exec {
            request_id: "req_late".to_string(),
            task_id: "task_late".to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
            },
        };
        let run = runs::prepare_run(&state, "agent", "req_late", &command).unwrap();

        let response = post_agent_message(
            State(state.clone()),
            Path("agent".to_string()),
            Query(SseConnectQuery {
                connection_id: Some("old".to_string()),
            }),
            agent_headers("secret"),
            axum::Json(AgentMessage::Response {
                run_id: Some(run.run_id.clone()),
                request_id: "req_late".to_string(),
                data: json!({ "ok": true }),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let stored = runs::get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.result, Some(json!({ "ok": true })));
    }

    #[tokio::test]
    async fn failed_request_send_removes_current_connection() {
        let state = test_state();
        let rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        drop(rx);
        let command = HubCommand::Exec {
            request_id: "req_send_failed".to_string(),
            task_id: "task_send_failed".to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
            },
        };

        let result = request_agent(&state, "agent", command, 1).await;

        assert_eq!(result.unwrap_err(), "agent_offline");
        assert!(!state.agents.lock().await.contains_key("agent"));
    }

    #[tokio::test]
    async fn expired_connection_cleanup_removes_only_stale_current_entries() {
        let state = test_state();
        let old_seen = chrono::Utc::now() - chrono::Duration::seconds(120);
        let fresh_seen = chrono::Utc::now();
        let _old_rx = insert_connection(&state, "old-agent", "old", old_seen).await;
        let _fresh_rx = insert_connection(&state, "fresh-agent", "fresh", fresh_seen).await;

        cleanup_expired_agent_connections_once(&state, chrono::Utc::now()).await;

        let agents = state.agents.lock().await;
        assert!(!agents.contains_key("old-agent"));
        assert!(agents.contains_key("fresh-agent"));
    }
}
