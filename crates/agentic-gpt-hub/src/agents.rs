use agentic_gpt_protocol::{
    AgentConnectionMode, AgentMessage, AgentRole, HubCommand, HubCommandEnvelope, HubMessage,
    JobInfo, JobState,
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
use crate::state::{
    AgentConnection, AgentTransport, HubState, OutboundAgentMessage, PendingResponse,
};
use crate::utils::{constant_time_equal, random_id, sha256_hex};
use crate::{
    api_error, discard_agent_confirmations, handle_confirmation_request, room, REQUEST_TIMEOUT_SECS,
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
    let connection_id = query.connection_id.unwrap_or_else(|| random_id("conn"));
    info!(%agent_id, %connection_id, "agent sse connected");
    let (tx, rx) = mpsc::unbounded_channel::<OutboundAgentMessage>();
    if let Err(reason) = replace_agent_connection(
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
    match handle_agent_message(&state, &agent_id, &connection_id, message).await {
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
    if let Err(reason) = replace_agent_connection(
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
        if let Err(reason) = handle_agent_message(&state, &agent_id, &connection_id, parsed).await {
            warn!(%agent_id, %connection_id, %reason, "agent message rejected");
        }
    }

    writer.abort();
    let _ = disconnect_agent(&state, &agent_id, &connection_id, None).await;
}

// `agents` is the admission and side-effect linearization guard. While it is
// held, acquire only short state locks in agents -> room/boot/jobs/
// confirmations order; do not await network, receivers, or spawned tasks, and
// do not call helpers that reacquire `state.agents`.
async fn handle_agent_message(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    parsed: AgentMessage,
) -> std::result::Result<(), String> {
    let reliable = is_reliable_agent_message(&parsed);
    let mut agents = state.agents.lock().await;
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
        return match parsed {
            AgentMessage::Response {
                run_id,
                request_id,
                data,
            } => {
                let Some(run_id) = run_id else {
                    return Err("response_run_id_required".to_string());
                };
                let outcome = match runs::store_result(state, agent_id, &run_id, &request_id, &data)
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        warn!(%agent_id, %request_id, %error, "failed to store agent result");
                        return Err("response_result_store_failed".to_string());
                    }
                };
                match outcome {
                    runs::StoreResultOutcome::Unmatched => Err("response_run_mismatch".to_string()),
                    runs::StoreResultOutcome::Conflict => {
                        Err("response_result_conflict".to_string())
                    }
                    runs::StoreResultOutcome::Stored { command_hash }
                    | runs::StoreResultOutcome::Duplicate { command_hash } => {
                        let sender = {
                            let mut pending = state.pending.lock().await;
                            let owner_matches = pending.get(&run_id).map(|owner| {
                                owner.agent_id == agent_id
                                    && owner.request_id == request_id
                                    && owner.command_hash == command_hash
                            });
                            match owner_matches {
                                None => None,
                                Some(true) => pending.remove(&run_id).map(|pending| pending.sender),
                                Some(false) => {
                                    return Err("response_waiter_owner_mismatch".to_string());
                                }
                            }
                        };
                        if let Some(sender) = sender {
                            let _ = sender.send(data);
                        }
                        Ok(())
                    }
                }
            }
            AgentMessage::TransportAck {
                event_id: _,
                run_id,
                request_id,
                command_hash,
            } => {
                let matched =
                    runs::mark_acked(state, agent_id, &run_id, &request_id, &command_hash)
                        .map_err(|error| error.to_string())?;
                if !matched {
                    return Err("transport_ack_run_mismatch".to_string());
                }
                Ok(())
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
                Ok(())
            }
            _ => unreachable!("reliable message classification drifted"),
        };
    }

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
            state
                .jobs
                .lock()
                .await
                .entry(agent_id.to_string())
                .or_default()
                .insert(job.job_id.clone(), job);
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
            None
        }
        AgentMessage::Response { .. }
        | AgentMessage::TransportAck { .. }
        | AgentMessage::TransportRunStatus { .. } => {
            unreachable!("reliable message classification drifted")
        }
    };
    drop(agents);
    if let Some(sender) = replay_sender {
        send_pending_replays(state, agent_id, &sender).await;
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

async fn disconnect_agent(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    expiry_check_at: Option<chrono::DateTime<chrono::Utc>>,
) -> bool {
    let removed_current_connection = {
        let mut agents = state.agents.lock().await;
        let should_remove = agents.get(agent_id).is_some_and(|connection| {
            connection.connection_id == connection_id
                && expiry_check_at.is_none_or(|now| {
                    now.signed_duration_since(connection.last_seen_at)
                        .num_seconds()
                        > AGENT_CONNECTION_TTL_SECS
                })
        });
        if should_remove {
            agents.remove(agent_id);
            room::release_active_room_if_current(state, agent_id, connection_id).await;
            discard_agent_confirmations(state, agent_id).await;
            true
        } else {
            false
        }
    };
    info!(%agent_id, %connection_id, removedCurrentConnection = removed_current_connection, "agent disconnected");
    removed_current_connection
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
        match agents.get(agent_id) {
            Some(connection)
                if connection.connection_mode == AgentConnectionMode::CommandCapable =>
            {
                if connection.hello_received {
                    Ok((connection.connection_id.clone(), connection.sender.clone()))
                } else {
                    Err("agent_not_ready".to_string())
                }
            }
            Some(_) => Err("agent_reporting_only".to_string()),
            None => Err("agent_offline".to_string()),
        }
    }?;
    let run = runs::prepare_run(state, agent_id, &request_id, &command)
        .map_err(|error| error.to_string())?;
    let text = envelope_text(&run.run_id, &run.request_id, &run.command_hash, command)
        .map_err(|error| error.to_string())?;
    let run_id = run.run_id;
    let run_request_id = run.request_id;
    let command_hash = run.command_hash;
    let (tx, rx) = oneshot::channel();
    state.pending.lock().await.insert(
        run_id.clone(),
        PendingResponse {
            agent_id: agent_id.to_string(),
            request_id: run_request_id,
            command_hash,
            sender: tx,
        },
    );
    if sender.1.send(OutboundAgentMessage::Text(text)).is_err() {
        state.pending.lock().await.remove(&run_id);
        let _ = disconnect_agent(state, agent_id, &sender.0, None).await;
        return Err("agent_offline".to_string());
    }
    if let Err(error) = runs::mark_dispatched(state, &run_id) {
        warn!(runId = %run_id, %error, "failed to mark run dispatched");
    }
    match timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(value)) => Ok(value),
        _ => {
            state.pending.lock().await.remove(&run_id);
            if let Err(error) = runs::mark_timeout(state, &run_id, "process_exec_timeout") {
                warn!(runId = %run_id, %error, "failed to mark run timeout");
            }
            Err(format!("process_exec_timeout; runId={}", run_id))
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
) -> std::result::Result<(), &'static str> {
    if connection_id.is_empty() {
        return Err("invalid_connection_id");
    }
    let mut agents = state.agents.lock().await;
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
        room::release_active_room_if_current(state, agent_id, &old.connection_id).await;
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
        if disconnect_agent(state, &agent_id, &connection_id, Some(now)).await {
            warn!(%agent_id, %connection_id, %last_seen_at, "agent connection expired");
        }
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
        | HubCommand::ProcessBatch { request_id, .. }
        | HubCommand::JobList { request_id, .. }
        | HubCommand::JobGet { request_id, .. }
        | HubCommand::JobCancel { request_id, .. }
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
        | HubCommand::McpBatch { request_id, .. }
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
        | HubCommand::RoomBootstrap { request_id }
        | HubCommand::RoomBootstrapRead { request_id, .. }
        | HubCommand::Bootstrap { request_id }
        | HubCommand::BootstrapRead { request_id, .. }
        | HubCommand::SkillsList { request_id }
        | HubCommand::SkillsRead { request_id, .. }
        | HubCommand::SkillsSearch { request_id, .. }
        | HubCommand::SkillsActive { request_id }
        | HubCommand::SkillsActivate { request_id, .. }
        | HubCommand::SkillsDeactivate { request_id, .. }
        | HubCommand::SkillsInstall { request_id, .. }
        | HubCommand::SkillsInstallGet { request_id, .. }
        | HubCommand::SkillsInstallCancel { request_id, .. }
        | HubCommand::SkillsRun { request_id, .. } => request_id,
    }
}

pub(crate) fn set_command_request_id(command: &mut HubCommand, value: String) {
    match command {
        HubCommand::Exec { request_id, .. }
        | HubCommand::ProcessBatch { request_id, .. }
        | HubCommand::JobList { request_id, .. }
        | HubCommand::JobGet { request_id, .. }
        | HubCommand::JobCancel { request_id, .. }
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
        | HubCommand::McpBatch { request_id, .. }
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
        | HubCommand::RoomBootstrap { request_id }
        | HubCommand::RoomBootstrapRead { request_id, .. }
        | HubCommand::Bootstrap { request_id }
        | HubCommand::BootstrapRead { request_id, .. }
        | HubCommand::SkillsList { request_id }
        | HubCommand::SkillsRead { request_id, .. }
        | HubCommand::SkillsSearch { request_id, .. }
        | HubCommand::SkillsActive { request_id }
        | HubCommand::SkillsActivate { request_id, .. }
        | HubCommand::SkillsDeactivate { request_id, .. }
        | HubCommand::SkillsInstall { request_id, .. }
        | HubCommand::SkillsInstallGet { request_id, .. }
        | HubCommand::SkillsInstallCancel { request_id, .. }
        | HubCommand::SkillsRun { request_id, .. } => *request_id = value,
    }
}

pub(crate) async fn cached_job(state: &HubState, agent_id: &str, job_id: &str) -> Option<JobInfo> {
    state
        .jobs
        .lock()
        .await
        .get(agent_id)
        .and_then(|jobs| jobs.get(job_id).cloned())
}

async fn mark_cached_jobs_unknown_after_restart(state: &HubState, agent_id: &str) {
    let mut jobs = state.jobs.lock().await;
    let Some(agent_jobs) = jobs.get_mut(agent_id) else {
        return;
    };
    let now = chrono::Utc::now();
    for job in agent_jobs.values_mut() {
        if job.state.is_active() {
            job.state = JobState::UnknownAfterRestart;
            job.updated_at = now;
            job.finished_at = Some(now);
            job.reject_reason = Some("unknown_after_restart".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::state::PendingConfirmation;
    use crate::{HubConfig, McpProfile, NtfyConfig, RemoteConfirmationConfig};
    use agentic_gpt_protocol::{
        AgentRunReport, Capabilities, ConfirmationPayload, ExecRequest, JobKind, SafeConfigSummary,
    };
    use axum::body::to_bytes;
    use axum::http::HeaderValue;
    use futures_util::StreamExt;
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
            mcp_profile: McpProfile::Full,
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            boot_generations: Arc::new(Mutex::new(HashMap::new())),
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
            jobs: true,
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

    fn test_running_job(job_id: &str) -> JobInfo {
        let now = chrono::Utc::now();
        JobInfo {
            agent_id: "agent".to_string(),
            job_id: job_id.to_string(),
            group: None,
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind: JobKind::Process,
            state: JobState::Running,
            created_at: now,
            started_at: Some(now),
            updated_at: now,
            finished_at: None,
            program: Some("sleep".to_string()),
            args: vec!["10".to_string()],
            working_directory: None,
            command_preview: Some("sleep 10".to_string()),
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
        }
    }

    fn test_config_summary() -> SafeConfigSummary {
        serde_json::from_value(json!({
            "workspaceRoot": "configured",
            "sandbox": {"enabled": false, "mode": "disabled"},
            "pathPolicy": {
                "writeRootCount": 1,
                "readOnlyRootCount": 0,
                "denyRootCount": 0,
                "writeRoots": [{"path": "workspace", "source": "workspace"}],
                "readOnlyRoots": [],
                "denyRoots": []
            },
            "policyRuleCounts": {"allow": 0, "confirm": 0, "deny": 0},
            "policyRules": {
                "allow": [],
                "confirm": [],
                "deny": [],
                "builtins": {"confirm": [], "deny": []}
            },
            "confirmationProvider": "none"
        }))
        .unwrap()
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
                connection_mode: AgentConnectionMode::CommandCapable,
                hello_received: true,
                boot_generation: Some("testboot".to_string()),
                transport: AgentTransport::Sse,
                config_summary: None,
                notification_channels: Vec::new(),
            },
        );
        rx
    }

    fn generation_config_summary(workspace_root: &str) -> SafeConfigSummary {
        let mut summary = test_config_summary();
        summary.workspace_root = workspace_root.to_string();
        summary
    }

    fn generation_hello(
        role: AgentRole,
        connection_mode: AgentConnectionMode,
        boot_generation: &str,
        workspace_root: &str,
    ) -> AgentMessage {
        AgentMessage::Hello {
            role,
            boot_generation: boot_generation.to_string(),
            connection_mode,
            config_summary: generation_config_summary(workspace_root),
            notification_channels: Vec::new(),
        }
    }

    fn generation_report(run_id: &str, request_id: &str) -> AgentMessage {
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        AgentMessage::RunReport {
            report: Box::new(AgentRunReport {
                run_id: run_id.to_string(),
                request_id: request_id.to_string(),
                tool_name: "generation.probe".to_string(),
                source: "tunnel".to_string(),
                profile: "normal".to_string(),
                detail: "metadata".to_string(),
                status: "started".to_string(),
                started_at: timestamp,
                updated_at: timestamp,
                duration_ms: None,
                job_id: None,
                exit_code: None,
                reason: None,
                arguments: None,
                result: None,
                job: None,
            }),
        }
    }

    fn generation_confirmation(request_id: &str, request_agent_id: &str) -> AgentMessage {
        AgentMessage::ConfirmationRequest {
            request_id: request_id.to_string(),
            agent_id: request_agent_id.to_string(),
            timeout_seconds: 5,
            payload: ConfirmationPayload {
                program: "generation-probe".to_string(),
                args: Vec::new(),
                command_preview: "generation probe".to_string(),
                risk_level: "LOW".to_string(),
                reason: "generation probe".to_string(),
                kind: None,
                server_id: None,
                tool_name: None,
            },
        }
    }

    fn generation_pending_confirmation(
        confirmation_id: &str,
        request_id: &str,
    ) -> PendingConfirmation {
        let now = chrono::Utc::now();
        PendingConfirmation {
            confirmation_id: confirmation_id.to_string(),
            request_id: request_id.to_string(),
            agent_id: "agent".to_string(),
            token_hash: "generation-token-hash".to_string(),
            command_preview: "generation probe".to_string(),
            risk_level: "LOW".to_string(),
            reason: "generation probe".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::seconds(60),
            resolved: false,
            decision: None,
        }
    }

    async fn generation_handle(
        state: &HubState,
        agent_id: &str,
        connection_id: &str,
        message: AgentMessage,
    ) -> std::result::Result<(), String> {
        timeout(
            Duration::from_secs(5),
            handle_agent_message(state, agent_id, connection_id, message),
        )
        .await
        .expect("generation handler timed out")
    }

    async fn generation_fixture() -> (HubState, mpsc::UnboundedReceiver<OutboundAgentMessage>) {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let mut old_rx = insert_connection(
            &state,
            "agent",
            "old",
            chrono::Utc::now() - chrono::Duration::seconds(10),
        )
        .await;
        let (new_tx, new_rx) = mpsc::unbounded_channel();
        replace_agent_connection(&state, "agent", "new", AgentTransport::Sse, new_tx)
            .await
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(5), old_rx.recv())
                .await
                .expect("replacement close timed out"),
            Some(OutboundAgentMessage::Close)
        ));
        generation_handle(
            &state,
            "agent",
            "new",
            generation_hello(
                AgentRole::Room,
                AgentConnectionMode::CommandCapable,
                "boot-new",
                "new",
            ),
        )
        .await
        .unwrap();
        (state, new_rx)
    }

    async fn generation_snapshot(state: &HubState) -> Value {
        let connection = {
            let agents = state.agents.lock().await;
            let connection = agents.get("agent").unwrap();
            json!({
                "connectionId": connection.connection_id,
                "role": connection.role,
                "connectionMode": connection.connection_mode,
                "helloReceived": connection.hello_received,
                "bootGeneration": connection.boot_generation,
                "transport": match connection.transport {
                    AgentTransport::WebSocket => "websocket",
                    AgentTransport::Sse => "sse",
                },
                "lastSeenAt": connection.last_seen_at,
                "configSummary": connection.config_summary,
                "notificationChannels": connection.notification_channels,
            })
        };
        let registry_last_seen = registry_entry(&state, "agent")
            .unwrap()
            .and_then(|entry| entry.last_seen_at);
        let boot_generation = state.boot_generations.lock().await.get("agent").cloned();
        let jobs = state
            .jobs
            .lock()
            .await
            .get("agent")
            .cloned()
            .unwrap_or_default();
        let active_room = state.active_room.lock().await.as_ref().map(|active| {
            json!({
                "agentId": active.agent_id,
                "connectionId": active.connection_id,
            })
        });
        let pending_confirmations = state.pending_confirmations.lock().await.len();
        json!({
            "connection": connection,
            "registryLastSeenAt": registry_last_seen,
            "bootGeneration": boot_generation,
            "jobs": jobs,
            "activeRoom": active_room,
            "pendingConfirmations": pending_confirmations,
        })
    }

    async fn start_response_owner_request(
        request_id: &str,
    ) -> (
        HubState,
        mpsc::UnboundedReceiver<OutboundAgentMessage>,
        tokio::task::JoinHandle<std::result::Result<Value, String>>,
        HubCommandEnvelope,
    ) {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        register_agent(&state, "foreign", "foreign-secret");
        let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        let command = HubCommand::Exec {
            request_id: request_id.to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                group: None,
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
            },
        };
        let request_state = state.clone();
        let caller =
            tokio::spawn(async move { request_agent(&request_state, "agent", command, 5).await });

        let OutboundAgentMessage::Text(text) = outbound.recv().await.unwrap() else {
            panic!("expected command envelope");
        };
        let envelope = serde_json::from_str::<HubCommandEnvelope>(&text).unwrap();
        while runs::get_run(&state, &envelope.run_id)
            .unwrap()
            .unwrap()
            .status
            != "dispatched"
        {
            tokio::task::yield_now().await;
        }
        (state, outbound, caller, envelope)
    }

    async fn post_response(
        state: &HubState,
        routed_agent_id: &str,
        secret: &str,
        run_id: Option<String>,
        request_id: String,
        data: Value,
    ) -> Response {
        post_agent_message(
            State(state.clone()),
            Path(routed_agent_id.to_string()),
            Query(SseConnectQuery {
                connection_id: Some("current".to_string()),
            }),
            agent_headers(secret),
            axum::Json(AgentMessage::Response {
                run_id,
                request_id,
                data,
            }),
        )
        .await
    }

    async fn rejected_reason(response: Response) -> String {
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], "agent_message_rejected");
        value["error"]["message"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn changed_boot_generation_marks_only_active_jobs_unknown_after_restart() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let _rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        state
            .boot_generations
            .lock()
            .await
            .insert("agent".to_string(), "boot-a".to_string());
        let running = test_running_job("job_boot-a_running");
        let mut completed = test_running_job("job_boot-a_completed");
        completed.state = JobState::Completed;
        completed.finished_at = Some(completed.updated_at);
        state.jobs.lock().await.insert(
            "agent".to_string(),
            HashMap::from([
                (running.job_id.clone(), running),
                (completed.job_id.clone(), completed),
            ]),
        );

        handle_agent_message(
            &state,
            "agent",
            "current",
            AgentMessage::Hello {
                role: AgentRole::Normal,
                boot_generation: "boot-b".to_string(),
                connection_mode: AgentConnectionMode::CommandCapable,
                config_summary: test_config_summary(),
                notification_channels: Vec::new(),
            },
        )
        .await
        .unwrap();

        let jobs = state.jobs.lock().await;
        let agent_jobs = jobs.get("agent").unwrap();
        let running = agent_jobs.get("job_boot-a_running").unwrap();
        assert_eq!(running.state, JobState::UnknownAfterRestart);
        assert_eq!(
            running.reject_reason.as_deref(),
            Some("unknown_after_restart")
        );
        assert!(running.finished_at.is_some());
        assert_eq!(
            agent_jobs.get("job_boot-a_completed").unwrap().state,
            JobState::Completed
        );
        drop(jobs);
        assert_eq!(
            state
                .boot_generations
                .lock()
                .await
                .get("agent")
                .map(String::as_str),
            Some("boot-b")
        );
        assert_eq!(
            state
                .agents
                .lock()
                .await
                .get("agent")
                .and_then(|connection| connection.boot_generation.as_deref()),
            Some("boot-b")
        );
    }

    #[tokio::test]
    async fn pending_replay_sends_reliable_envelope() {
        let state = test_state();
        let command = HubCommand::Exec {
            request_id: "req_replay".to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                group: None,
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
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
    async fn stale_job_update_is_rejected_without_writing_job_cache() {
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
            axum::Json(AgentMessage::JobUpdate {
                job: test_running_job("job_oldboot_123"),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(state.jobs.lock().await.is_empty());
    }

    #[tokio::test]
    async fn stale_response_with_matching_run_is_accepted() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let _rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        let command = HubCommand::Exec {
            request_id: "req_late".to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                group: None,
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
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
    async fn response_owner_rejects_unmatched_without_consuming_waiter() {
        for case in ["nonexistent_run", "wrong_request", "missing_run_id"] {
            let request_id = format!("req_response_owner_unmatched_{case}");
            let (state, _outbound, caller, envelope) =
                start_response_owner_request(&request_id).await;
            let (routed_agent_id, secret, invalid_run_id, invalid_request_id, expected_reason) =
                match case {
                    "nonexistent_run" => (
                        "foreign",
                        "foreign-secret",
                        Some(format!("{}-missing", envelope.run_id)),
                        envelope.request_id.clone(),
                        "response_run_mismatch",
                    ),
                    "wrong_request" => (
                        "agent",
                        "secret",
                        Some(envelope.run_id.clone()),
                        format!("{}-wrong", envelope.request_id),
                        "response_run_mismatch",
                    ),
                    "missing_run_id" => (
                        "agent",
                        "secret",
                        None,
                        envelope.request_id.clone(),
                        "response_run_id_required",
                    ),
                    _ => unreachable!(),
                };

            let response = post_response(
                &state,
                routed_agent_id,
                secret,
                invalid_run_id,
                invalid_request_id,
                json!({ "servers": ["mismatched"] }),
            )
            .await;
            assert_eq!(rejected_reason(response).await, expected_reason);
            assert!(!caller.is_finished());
            assert!(runs::get_run(&state, &envelope.run_id)
                .unwrap()
                .unwrap()
                .result
                .is_none());

            let valid_data = json!({ "servers": [] });
            let response = post_response(
                &state,
                "agent",
                "secret",
                Some(envelope.run_id.clone()),
                envelope.request_id.clone(),
                valid_data.clone(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(caller.await.unwrap().unwrap(), valid_data);

            let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
            assert_eq!(stored.status, "completed");
            assert_eq!(stored.result, Some(valid_data));
        }
    }
    #[tokio::test]
    async fn response_owner_isolates_runs_sharing_request_id() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        let request_id = "req_response_owner_shared_request".to_string();
        let make_command = || HubCommand::Exec {
            request_id: request_id.clone(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                group: None,
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
            },
        };

        let first_command = make_command();
        let second_command = make_command();
        let first_state = state.clone();
        let first_caller =
            tokio::spawn(
                async move { request_agent(&first_state, "agent", first_command, 5).await },
            );
        let OutboundAgentMessage::Text(first_text) = outbound.recv().await.unwrap() else {
            panic!("expected first command envelope");
        };
        let first_envelope = serde_json::from_str::<HubCommandEnvelope>(&first_text).unwrap();

        let second_state = state.clone();
        let second_caller =
            tokio::spawn(
                async move { request_agent(&second_state, "agent", second_command, 5).await },
            );
        let OutboundAgentMessage::Text(second_text) = outbound.recv().await.unwrap() else {
            panic!("expected second command envelope");
        };
        let second_envelope = serde_json::from_str::<HubCommandEnvelope>(&second_text).unwrap();

        assert_eq!(first_envelope.request_id, request_id);
        assert_eq!(second_envelope.request_id, request_id);
        assert_ne!(first_envelope.run_id, second_envelope.run_id);
        assert!(!first_envelope.event_id.is_empty());
        assert!(!second_envelope.event_id.is_empty());
        assert!(!first_envelope.command_hash.is_empty());
        assert!(!second_envelope.command_hash.is_empty());

        let first_data = json!({ "runId": first_envelope.run_id.clone() });
        let second_data = json!({ "runId": second_envelope.run_id.clone() });

        let second_response = post_response(
            &state,
            "agent",
            "secret",
            Some(second_envelope.run_id.clone()),
            second_envelope.request_id.clone(),
            second_data.clone(),
        )
        .await;
        assert_eq!(second_response.status(), StatusCode::OK);

        let first_response = post_response(
            &state,
            "agent",
            "secret",
            Some(first_envelope.run_id.clone()),
            first_envelope.request_id.clone(),
            first_data.clone(),
        )
        .await;
        assert_eq!(first_response.status(), StatusCode::OK);

        let (first_joined, second_joined) = tokio::join!(first_caller, second_caller);
        let first_result = first_joined.unwrap().unwrap();
        let second_result = second_joined.unwrap().unwrap();
        assert_eq!(first_result, first_data);
        assert_eq!(second_result, second_data);

        let first_run = runs::get_run(&state, &first_envelope.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(first_run.status, "completed");
        assert_eq!(first_run.result, Some(first_data));

        let second_run = runs::get_run(&state, &second_envelope.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(second_run.status, "completed");
        assert_eq!(second_run.result, Some(second_data));
    }
    #[tokio::test]
    async fn response_owner_timeout_preserves_other_run_and_accepts_late_result() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        let request_id = "req_response_owner_timeout_shared_request".to_string();
        let make_command = || HubCommand::Exec {
            request_id: request_id.clone(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                group: None,
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
            },
        };
        let long_command = make_command();
        let short_command = make_command();
        let long_state = state.clone();
        let long_caller =
            tokio::spawn(async move { request_agent(&long_state, "agent", long_command, 5).await });
        let OutboundAgentMessage::Text(long_text) = outbound.recv().await.unwrap() else {
            panic!("expected long-running command envelope");
        };
        let long_envelope = serde_json::from_str::<HubCommandEnvelope>(&long_text).unwrap();

        let short_state = state.clone();
        let short_caller =
            tokio::spawn(
                async move { request_agent(&short_state, "agent", short_command, 0).await },
            );
        let OutboundAgentMessage::Text(short_text) = outbound.recv().await.unwrap() else {
            panic!("expected short-running command envelope");
        };
        let short_envelope = serde_json::from_str::<HubCommandEnvelope>(&short_text).unwrap();
        assert_eq!(long_envelope.request_id, request_id);
        assert_eq!(short_envelope.request_id, request_id);
        assert_ne!(long_envelope.run_id, short_envelope.run_id);

        let short_error = short_caller.await.unwrap().unwrap_err();
        assert_eq!(
            short_error,
            format!("process_exec_timeout; runId={}", short_envelope.run_id)
        );
        assert!(!long_caller.is_finished());

        let short_data = json!({ "runId": short_envelope.run_id.clone() });
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(short_envelope.run_id.clone()),
            short_envelope.request_id.clone(),
            short_data.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!long_caller.is_finished());
        let short_run = runs::get_run(&state, &short_envelope.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(short_run.status, "completed");
        assert_eq!(short_run.result, Some(short_data));

        let long_data = json!({ "runId": long_envelope.run_id.clone() });
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(long_envelope.run_id.clone()),
            long_envelope.request_id.clone(),
            long_data.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(long_caller.await.unwrap().unwrap(), long_data.clone());
        let long_run = runs::get_run(&state, &long_envelope.run_id)
            .unwrap()
            .unwrap();
        assert_eq!(long_run.status, "completed");
        assert_eq!(long_run.result, Some(long_data));
    }

    #[tokio::test]
    async fn response_owner_checks_pending_hash_against_durable_run() {
        let (state, _outbound, caller, envelope) =
            start_response_owner_request("req_response_owner_pending_hash").await;
        state
            .pending
            .lock()
            .await
            .get_mut(&envelope.run_id)
            .expect("request waiter should be present")
            .command_hash = "tampered-command-hash".to_string();

        let data = json!({ "servers": ["canonical"] });
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(envelope.run_id.clone()),
            envelope.request_id.clone(),
            data.clone(),
        )
        .await;
        assert_eq!(
            rejected_reason(response).await,
            "response_waiter_owner_mismatch"
        );
        assert!(!caller.is_finished());
        let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.result, Some(data.clone()));

        state
            .pending
            .lock()
            .await
            .get_mut(&envelope.run_id)
            .expect("mismatched waiter should be retained")
            .command_hash = envelope.command_hash.clone();
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(envelope.run_id.clone()),
            envelope.request_id.clone(),
            data.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(caller.await.unwrap().unwrap(), data);
    }

    #[tokio::test]
    async fn response_owner_rejects_conflict_and_accepts_duplicate() {
        let (state, _outbound, caller, envelope) =
            start_response_owner_request("req_response_owner_conflict").await;
        let canonical = json!({ "servers": ["canonical"] });
        assert!(matches!(
            runs::store_result(
                &state,
                "agent",
                &envelope.run_id,
                &envelope.request_id,
                &canonical,
            )
            .unwrap(),
            runs::StoreResultOutcome::Stored { .. }
        ));

        let conflict = json!({ "servers": ["conflict"] });
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(envelope.run_id.clone()),
            envelope.request_id.clone(),
            conflict.clone(),
        )
        .await;
        assert_eq!(rejected_reason(response).await, "response_result_conflict");
        assert!(!caller.is_finished());

        let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
        assert_eq!(stored.result, Some(canonical.clone()));
        let conflict_json: String = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "select conflict_json from agent_runs where run_id = ?1",
                params![envelope.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&conflict_json).unwrap(),
            conflict
        );

        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(envelope.run_id.clone()),
            envelope.request_id.clone(),
            canonical.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(caller.await.unwrap().unwrap(), canonical);
    }

    #[tokio::test]
    async fn response_owner_keeps_waiter_on_store_failure() {
        let (state, _outbound, caller, envelope) =
            start_response_owner_request("req_response_owner_store_failure").await;
        state
            .db
            .lock()
            .unwrap()
            .execute_batch(
                "create trigger force_result_write_failure
                 before update of result_json on agent_runs
                 begin
                     select raise(abort, 'forced_result_write_failure');
                 end;",
            )
            .unwrap();

        let data = json!({ "servers": ["retry"] });
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(envelope.run_id.clone()),
            envelope.request_id.clone(),
            data.clone(),
        )
        .await;
        assert_eq!(
            rejected_reason(response).await,
            "response_result_store_failed"
        );
        assert!(!caller.is_finished());
        assert!(runs::get_run(&state, &envelope.run_id)
            .unwrap()
            .unwrap()
            .result
            .is_none());

        state
            .db
            .lock()
            .unwrap()
            .execute_batch("drop trigger force_result_write_failure;")
            .unwrap();
        let response = post_response(
            &state,
            "agent",
            "secret",
            Some(envelope.run_id.clone()),
            envelope.request_id.clone(),
            data.clone(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(caller.await.unwrap().unwrap(), data.clone());
        let stored = runs::get_run(&state, &envelope.run_id).unwrap().unwrap();
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.result, Some(data));
    }

    #[tokio::test]
    async fn failed_request_send_removes_current_connection() {
        let state = test_state();
        let rx = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;
        drop(rx);
        let command = HubCommand::Exec {
            request_id: "req_send_failed".to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                group: None,
                program: "printf".to_string(),
                args: vec!["ok".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
            },
        };

        let result = request_agent(&state, "agent", command, 1).await;

        assert_eq!(result.unwrap_err(), "agent_offline");
        assert!(!state.agents.lock().await.contains_key("agent"));
        let run_id: String = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "select run_id from agent_runs where request_id = ?1",
                params!["req_send_failed"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!state.pending.lock().await.contains_key(&run_id));
    }

    #[tokio::test]
    async fn reporting_only_connection_is_not_a_command_target() {
        let state = test_state();
        let _rx = insert_connection(&state, "agent", "reporting", chrono::Utc::now()).await;
        state
            .agents
            .lock()
            .await
            .get_mut("agent")
            .unwrap()
            .connection_mode = AgentConnectionMode::ReportingOnly;
        let command = HubCommand::Exec {
            request_id: "req_reporting_only".to_string(),
            payload: ExecRequest {
                agent_id: "agent".to_string(),
                group: None,
                program: "printf".to_string(),
                args: vec!["blocked".to_string()],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
            },
        };

        let result = request_agent(&state, "agent", command, 1).await;

        assert_eq!(result.unwrap_err(), "agent_reporting_only");
        let run_count: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(run_count, 0);
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
    #[tokio::test]
    async fn generation_stale_messages_preserve_current_state() {
        for case in [
            "hello_normal",
            "hello_reporting_only",
            "hello_room_changed_boot",
            "heartbeat",
            "job_update",
            "run_report",
            "confirmation_request",
        ] {
            let (state, mut new_rx) = generation_fixture().await;
            if case == "hello_room_changed_boot" {
                let running = test_running_job("generation_active");
                let mut completed = test_running_job("generation_terminal");
                completed.state = JobState::Completed;
                completed.finished_at = Some(completed.updated_at);
                state.jobs.lock().await.insert(
                    "agent".to_string(),
                    HashMap::from([
                        (running.job_id.clone(), running),
                        (completed.job_id.clone(), completed),
                    ]),
                );
            }
            let before = generation_snapshot(&state).await;
            let result = match case {
                "hello_normal" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        generation_hello(
                            AgentRole::Normal,
                            AgentConnectionMode::CommandCapable,
                            "boot-old",
                            "old",
                        ),
                    )
                    .await
                }
                "hello_reporting_only" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        generation_hello(
                            AgentRole::Normal,
                            AgentConnectionMode::ReportingOnly,
                            "boot-old",
                            "old",
                        ),
                    )
                    .await
                }
                "hello_room_changed_boot" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        generation_hello(
                            AgentRole::Room,
                            AgentConnectionMode::CommandCapable,
                            "boot-old",
                            "old",
                        ),
                    )
                    .await
                }
                "heartbeat" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        AgentMessage::Heartbeat {
                            sent_at: chrono::Utc::now(),
                        },
                    )
                    .await
                }
                "job_update" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        AgentMessage::JobUpdate {
                            job: test_running_job("generation_stale_job"),
                        },
                    )
                    .await
                }
                "run_report" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        generation_report("stale-report", "stale-report-request"),
                    )
                    .await
                }
                "confirmation_request" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        generation_confirmation("stale-confirmation", "agent"),
                    )
                    .await
                }
                _ => unreachable!(),
            };
            assert_eq!(
                result,
                Err("stale_connection".to_string()),
                "stale case {case}"
            );
            assert_eq!(
                generation_snapshot(&state).await,
                before,
                "stale case {case} changed current state"
            );
            if case == "run_report" {
                assert!(runs::get_run(&state, "stale-report").unwrap().is_none());
            }
            assert!(matches!(
                new_rx.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
    }

    #[tokio::test]
    async fn generation_current_messages_keep_existing_effects() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let mut outbound = insert_connection(&state, "agent", "current", chrono::Utc::now()).await;

        generation_handle(
            &state,
            "agent",
            "current",
            generation_hello(
                AgentRole::Room,
                AgentConnectionMode::CommandCapable,
                "boot-a",
                "current",
            ),
        )
        .await
        .unwrap();
        assert_eq!(
            state
                .active_room
                .lock()
                .await
                .as_ref()
                .map(|active| active.connection_id.as_str()),
            Some("current")
        );

        let running = test_running_job("generation_running");
        let mut completed = test_running_job("generation_completed");
        completed.state = JobState::Completed;
        completed.finished_at = Some(completed.updated_at);
        state.jobs.lock().await.insert(
            "agent".to_string(),
            HashMap::from([
                (running.job_id.clone(), running),
                (completed.job_id.clone(), completed),
            ]),
        );
        generation_handle(
            &state,
            "agent",
            "current",
            generation_hello(
                AgentRole::Room,
                AgentConnectionMode::CommandCapable,
                "boot-a",
                "current",
            ),
        )
        .await
        .unwrap();
        {
            let jobs = state.jobs.lock().await;
            assert_eq!(jobs["agent"]["generation_running"].state, JobState::Running);
            assert_eq!(
                jobs["agent"]["generation_completed"].state,
                JobState::Completed
            );
        }

        generation_handle(
            &state,
            "agent",
            "current",
            generation_hello(
                AgentRole::Room,
                AgentConnectionMode::CommandCapable,
                "boot-b",
                "current-new",
            ),
        )
        .await
        .unwrap();
        {
            let jobs = state.jobs.lock().await;
            assert_eq!(
                jobs["agent"]["generation_running"].state,
                JobState::UnknownAfterRestart
            );
            assert_eq!(
                jobs["agent"]["generation_completed"].state,
                JobState::Completed
            );
        }

        let sent_at = chrono::DateTime::parse_from_rfc3339("2026-09-16T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        generation_handle(
            &state,
            "agent",
            "current",
            AgentMessage::Heartbeat { sent_at },
        )
        .await
        .unwrap();
        let OutboundAgentMessage::Text(text) = timeout(Duration::from_secs(5), outbound.recv())
            .await
            .expect("current heartbeat ack timed out")
            .expect("current heartbeat sender closed")
        else {
            panic!("expected heartbeat ack");
        };
        assert!(matches!(
            serde_json::from_str::<HubMessage>(&text).unwrap(),
            HubMessage::HeartbeatAck {
                sent_at: ack_sent,
                ..
            } if ack_sent == sent_at
        ));

        generation_handle(
            &state,
            "agent",
            "current",
            AgentMessage::JobUpdate {
                job: test_running_job("generation_current_job"),
            },
        )
        .await
        .unwrap();
        assert!(state
            .jobs
            .lock()
            .await
            .get("agent")
            .is_some_and(|jobs| jobs.contains_key("generation_current_job")));

        generation_handle(
            &state,
            "agent",
            "current",
            generation_report("generation-current-report", "generation-current-request"),
        )
        .await
        .unwrap();
        let report = runs::get_run(&state, "generation-current-report")
            .unwrap()
            .expect("current report should be stored");
        assert_eq!(report.status, "started");
        assert_eq!(report.agent_id, "agent");
    }

    #[tokio::test]
    async fn generation_stale_reliable_messages_do_not_touch_current() {
        for case in ["response", "transport_ack", "transport_status"] {
            let (state, mut new_rx) = generation_fixture().await;
            let command = HubCommand::McpListServers {
                request_id: format!("generation-{case}-request"),
            };
            let run = runs::prepare_run(
                &state,
                "agent",
                &format!("generation-{case}-request"),
                &command,
            )
            .unwrap();
            let before = generation_snapshot(&state).await;
            let result = match case {
                "response" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        AgentMessage::Response {
                            run_id: Some(run.run_id.clone()),
                            request_id: run.request_id.clone(),
                            data: json!({ "servers": [] }),
                        },
                    )
                    .await
                }
                "transport_ack" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        AgentMessage::TransportAck {
                            event_id: "generation-event".to_string(),
                            run_id: run.run_id.clone(),
                            request_id: run.request_id.clone(),
                            command_hash: run.command_hash.clone(),
                        },
                    )
                    .await
                }
                "transport_status" => {
                    generation_handle(
                        &state,
                        "agent",
                        "old",
                        AgentMessage::TransportRunStatus {
                            run_id: run.run_id.clone(),
                            request_id: run.request_id.clone(),
                            status: "started".to_string(),
                            reason: None,
                        },
                    )
                    .await
                }
                _ => unreachable!(),
            };
            assert_eq!(result, Ok(()), "reliable case {case}");
            assert_eq!(
                generation_snapshot(&state).await,
                before,
                "reliable case {case} touched current state"
            );
            let stored = runs::get_run(&state, &run.run_id).unwrap().unwrap();
            match case {
                "response" => {
                    assert_eq!(stored.status, "completed");
                    assert_eq!(stored.result, Some(json!({ "servers": [] })));
                }
                "transport_ack" => assert_eq!(stored.status, "acked"),
                "transport_status" => assert_eq!(stored.status, "started"),
                _ => unreachable!(),
            }
            assert!(matches!(
                new_rx.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
        }
    }

    #[tokio::test]
    async fn generation_job_update_and_replace_are_linearized() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let _old_rx = insert_connection(&state, "agent", "old", chrono::Utc::now()).await;
        let jobs = state.jobs.lock().await;
        let mut update = Box::pin(handle_agent_message(
            &state,
            "agent",
            "old",
            AgentMessage::JobUpdate {
                job: test_running_job("generation_linearized_job"),
            },
        ));
        assert!(matches!(
            futures_util::poll!(update.as_mut()),
            std::task::Poll::Pending
        ));

        let (new_tx, _new_rx) = mpsc::unbounded_channel();
        let mut replacement = Box::pin(replace_agent_connection(
            &state,
            "agent",
            "new",
            AgentTransport::Sse,
            new_tx,
        ));
        let replacement_poll = futures_util::poll!(replacement.as_mut());
        let mut replacement_result = match replacement_poll {
            std::task::Poll::Ready(result) => Some(result),
            std::task::Poll::Pending => None,
        };
        let replacement_finished = replacement_result.is_some();
        drop(jobs);

        let update_result = timeout(Duration::from_secs(5), update.as_mut())
            .await
            .expect("job update timed out");
        if replacement_result.is_none() {
            replacement_result = Some(
                timeout(Duration::from_secs(5), replacement.as_mut())
                    .await
                    .expect("replacement timed out"),
            );
        }
        assert_eq!(replacement_result.unwrap(), Ok(()));
        if replacement_finished {
            assert_eq!(update_result, Err("stale_connection".to_string()));
            assert!(!state
                .jobs
                .lock()
                .await
                .get("agent")
                .is_some_and(|jobs| jobs.contains_key("generation_linearized_job")));
        } else {
            assert_eq!(update_result, Ok(()));
            assert!(state
                .jobs
                .lock()
                .await
                .get("agent")
                .is_some_and(|jobs| jobs.contains_key("generation_linearized_job")));
        }
    }

    #[tokio::test]
    async fn generation_replacement_preserves_new_room_on_old_disconnect() {
        let (state, _new_rx) = generation_fixture().await;
        state.pending_confirmations.lock().await.insert(
            "generation-confirmation".to_string(),
            generation_pending_confirmation(
                "generation-confirmation",
                "generation-confirmation-request",
            ),
        );
        let before = generation_snapshot(&state).await;

        assert!(!disconnect_agent(&state, "agent", "old", None).await);
        assert_eq!(generation_snapshot(&state).await, before);
        assert!(
            !state
                .pending_confirmations
                .lock()
                .await
                .get("generation-confirmation")
                .unwrap()
                .resolved
        );

        assert!(disconnect_agent(&state, "agent", "new", None).await);
        assert!(state.active_room.lock().await.is_none());
        let pending = state
            .pending_confirmations
            .lock()
            .await
            .get("generation-confirmation")
            .unwrap()
            .clone();
        assert!(pending.resolved);
        assert!(matches!(
            pending.decision,
            Some(agentic_gpt_protocol::ConfirmationDecision::ProviderUnavailable)
        ));
    }

    #[tokio::test]
    async fn generation_expiry_rechecks_current_liveness() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let mut outbound = insert_connection(
            &state,
            "agent",
            "current",
            chrono::Utc::now() - chrono::Duration::seconds(120),
        )
        .await;
        generation_handle(
            &state,
            "agent",
            "current",
            AgentMessage::Heartbeat {
                sent_at: chrono::Utc::now(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(5), outbound.recv())
                .await
                .expect("heartbeat ack timed out"),
            Some(OutboundAgentMessage::Text(_))
        ));
        let fresh_now = chrono::Utc::now();
        assert!(!disconnect_agent(&state, "agent", "current", Some(fresh_now),).await);
        assert!(state.agents.lock().await.contains_key("agent"));

        state
            .agents
            .lock()
            .await
            .get_mut("agent")
            .unwrap()
            .last_seen_at = fresh_now - chrono::Duration::seconds(120);
        assert!(disconnect_agent(&state, "agent", "current", Some(fresh_now),).await);
        assert!(!state.agents.lock().await.contains_key("agent"));

        let (replaced_state, _new_rx) = generation_fixture().await;
        assert!(
            !disconnect_agent(
                &replaced_state,
                "agent",
                "old",
                Some(chrono::Utc::now() + chrono::Duration::seconds(120)),
            )
            .await
        );
        assert_eq!(
            replaced_state
                .agents
                .lock()
                .await
                .get("agent")
                .map(|connection| connection.connection_id.as_str()),
            Some("new")
        );
    }

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
            Query(SseConnectQuery {
                connection_id: Some("same".to_string()),
            }),
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
            Query(SseConnectQuery {
                connection_id: Some(String::new()),
            }),
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
            Query(SseConnectQuery {
                connection_id: Some("fresh".to_string()),
            }),
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
    async fn generation_disconnect_blocks_replacement_until_cleanup_finishes() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let _old_rx = insert_connection(&state, "agent", "old", chrono::Utc::now()).await;
        state.pending_confirmations.lock().await.insert(
            "generation-old-confirmation".to_string(),
            generation_pending_confirmation(
                "generation-old-confirmation",
                "generation-old-request",
            ),
        );
        let pending_guard = state.pending_confirmations.lock().await;
        let mut disconnect = Box::pin(disconnect_agent(&state, "agent", "old", None));
        assert!(matches!(
            futures_util::poll!(disconnect.as_mut()),
            std::task::Poll::Pending
        ));

        let (new_tx, _new_rx) = mpsc::unbounded_channel();
        let mut replacement = Box::pin(replace_agent_connection(
            &state,
            "agent",
            "new",
            AgentTransport::Sse,
            new_tx,
        ));
        assert!(matches!(
            futures_util::poll!(replacement.as_mut()),
            std::task::Poll::Pending
        ));
        drop(pending_guard);

        assert!(timeout(Duration::from_secs(5), disconnect.as_mut())
            .await
            .expect("disconnect cleanup timed out"));
        timeout(Duration::from_secs(5), replacement.as_mut())
            .await
            .expect("replacement registration timed out")
            .expect("replacement registration rejected");
        state.pending_confirmations.lock().await.insert(
            "generation-new-confirmation".to_string(),
            generation_pending_confirmation(
                "generation-new-confirmation",
                "generation-new-request",
            ),
        );
        assert!(
            !state
                .pending_confirmations
                .lock()
                .await
                .get("generation-new-confirmation")
                .unwrap()
                .resolved
        );
    }

    #[tokio::test]
    async fn generation_stale_heartbeat_direct_handler() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let mut old_rx = insert_connection(
            &state,
            "agent",
            "old",
            chrono::Utc::now() - chrono::Duration::seconds(10),
        )
        .await;
        let (new_tx, new_rx) = mpsc::unbounded_channel();
        replace_agent_connection(&state, "agent", "new", AgentTransport::WebSocket, new_tx)
            .await
            .unwrap();
        assert!(matches!(
            old_rx.recv().await,
            Some(OutboundAgentMessage::Close)
        ));
        let new_seen = state
            .agents
            .lock()
            .await
            .get("agent")
            .map(|connection| connection.last_seen_at)
            .unwrap();

        let result = handle_agent_message(
            &state,
            "agent",
            "old",
            AgentMessage::Heartbeat {
                sent_at: chrono::Utc::now(),
            },
        )
        .await;

        assert_eq!(result, Err("stale_connection".to_string()));
        assert_eq!(
            state
                .agents
                .lock()
                .await
                .get("agent")
                .map(|connection| connection.last_seen_at),
            Some(new_seen)
        );
        drop(new_rx);
    }
    #[tokio::test]
    async fn current_sse_heartbeat_route_sends_ack_and_touches_registry() {
        let state = test_state();
        register_agent(&state, "agent", "secret");
        let response = connect_agent_sse(
            State(state.clone()),
            Path("agent".to_string()),
            Query(SseConnectQuery {
                connection_id: Some("sse-route".to_string()),
            }),
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
            Query(SseConnectQuery {
                connection_id: Some("sse-route".to_string()),
            }),
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
}
