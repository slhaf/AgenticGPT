use agentic_gpt_protocol::{
    AgentMessage, AgentRole, BatchExecRequest, BatchExecResult, HubCommand, HubMessage,
    SessionInfo, TaskResult,
};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

use crate::registry::{registry_entries, registry_entry, update_last_seen};
use crate::state::{AgentConnection, HubState};
use crate::utils::{constant_time_equal, random_id, sha256_hex};
use crate::{
    api_error, discard_agent_confirmations, handle_confirmation_request, room,
    send_confirmation_response, REQUEST_TIMEOUT_SECS,
};

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

async fn handle_socket(state: HubState, agent_id: String, socket: WebSocket) {
    let connection_id = random_id("conn");
    info!(%agent_id, %connection_id, "agent connected");
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    replace_agent_connection(&state, &agent_id, &connection_id, tx.clone()).await;

    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if sink.send(message).await.is_err() {
                break;
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
        touch_agent(&state, &agent_id).await;
        match parsed {
            AgentMessage::Hello {
                role,
                config_summary,
                notification_channels,
            } => {
                match room::register_connection_role(&state, &agent_id, &connection_id, role).await
                {
                    Ok(()) => {
                        let mut agents = state.agents.lock().await;
                        if let Some(connection) = agents.get_mut(&agent_id) {
                            connection.role = role;
                            connection.config_summary = Some(config_summary);
                            connection.notification_channels = notification_channels;
                        }
                    }
                    Err(reason) => {
                        warn!(%agent_id, %connection_id, %reason, "room role rejected");
                        let _ = tx.send(Message::Text(
                            serde_json::to_string(&json!({
                                "error": {
                                    "code": reason,
                                    "message": reason
                                }
                            }))
                            .unwrap(),
                        ));
                        let _ = tx.send(Message::Close(None));
                        break;
                    }
                }
            }
            AgentMessage::Heartbeat { sent_at } => {
                let ack = HubMessage::HeartbeatAck {
                    sent_at,
                    received_at: chrono::Utc::now(),
                };
                let _ = tx.send(Message::Text(serde_json::to_string(&ack).unwrap()));
            }
            AgentMessage::SessionUpdate { session } => {
                state
                    .sessions
                    .lock()
                    .await
                    .entry(agent_id.clone())
                    .or_default()
                    .insert(session.session_id.clone(), session);
            }
            AgentMessage::Response { request_id, data } => {
                if let Some(sender) = state.pending.lock().await.remove(&request_id) {
                    let _ = sender.send(data);
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
                        &state,
                        &agent_id,
                        &request_id,
                        agentic_gpt_protocol::ConfirmationDecision::ProviderUnavailable,
                        "agent_id_mismatch",
                    )
                    .await;
                    continue;
                }
                let state = state.clone();
                let agent_id = agent_id.clone();
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
    }

    writer.abort();
    let removed_current_connection = {
        let mut agents = state.agents.lock().await;
        let should_remove = agents
            .get(&agent_id)
            .map(|connection| connection.connection_id == connection_id)
            .unwrap_or(false);
        if should_remove {
            agents.remove(&agent_id);
            true
        } else {
            false
        }
    };
    if removed_current_connection {
        room::release_active_room_if_current(&state, &agent_id, &connection_id).await;
        discard_agent_confirmations(&state, &agent_id).await;
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
    let sender = {
        let agents = state.agents.lock().await;
        agents
            .get(agent_id)
            .map(|connection| connection.sender.clone())
            .ok_or_else(|| "agent_offline".to_string())?
    };
    let (tx, rx) = oneshot::channel();
    state.pending.lock().await.insert(request_id.clone(), tx);
    set_command_request_id(&mut command, request_id.clone());
    if sender
        .send(Message::Text(
            serde_json::to_string(&command).map_err(|error| error.to_string())?,
        ))
        .is_err()
    {
        state.pending.lock().await.remove(&request_id);
        return Err("agent_offline".to_string());
    }
    match timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(value)) => Ok(value),
        _ => {
            state.pending.lock().await.remove(&request_id);
            Err("exec_timeout_use_session".to_string())
        }
    }
}

pub(crate) async fn replace_agent_connection(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    sender: mpsc::UnboundedSender<Message>,
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
                config_summary: None,
                notification_channels: Vec::new(),
            },
        )
    };
    if let Some(old) = old {
        room::release_active_room_if_current(state, agent_id, &old.connection_id).await;
        let _ = old.sender.send(Message::Close(None));
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
        | HubCommand::RoomDiarySelectExact { request_id, .. } => request_id,
    }
}

fn set_command_request_id(command: &mut HubCommand, value: String) {
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
        | HubCommand::RoomDiarySelectExact { request_id, .. } => *request_id = value,
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
