use agentic_gpt_protocol::{AgentMessage, HubCommand, HubMessage, SessionInfo};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout, Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Response as WsResponse;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    client_async_tls_with_config, connect_async, MaybeTlsStream, WebSocketStream,
};

use crate::{
    command_preview, confirmation, exec, log_info, log_warn, mcp, notebook, notify, sessions,
    AppState, RunMode, CONNECT_TIMEOUT_SECS, HEARTBEAT_ACK_TIMEOUT_SECS, HEARTBEAT_INTERVAL_SECS,
    RECONNECT_DELAY_SECS,
};

pub(crate) async fn connect_loop(state: AppState) -> Result<()> {
    loop {
        let config = state.config.read().await.clone();
        let url = format!(
            "{}/v1/agents/{}/connect",
            config.hub_url.trim_end_matches('/'),
            config.agent_id
        )
        .replace("http://", "ws://")
        .replace("https://", "wss://");
        let mut request = url.into_client_request()?;
        request
            .headers_mut()
            .insert("x-agent-secret", config.agent_secret.parse()?);

        let proxy = proxy_url(&config.hub_url);
        log_info(format!(
            "connecting to hub; agentId={}; proxy={}",
            config.agent_id,
            proxy.as_deref().unwrap_or("none")
        ));
        match timeout(
            Duration::from_secs(CONNECT_TIMEOUT_SECS),
            connect_hub(request, proxy),
        )
        .await
        {
            Err(_) => {
                log_warn(format!("connect timed out after {CONNECT_TIMEOUT_SECS}s"));
            }
            Ok(Err(error)) => {
                log_warn(format!("connect failed: {error}"));
            }
            Ok(Ok((stream, _))) => {
                log_info("connected to hub".to_string());
                let (mut write, mut read) = stream.split();
                let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
                *state.hub_sender.lock().await = Some(tx.clone());
                let writer = tokio::spawn(async move {
                    while let Some(message) = rx.recv().await {
                        if write.send(message).await.is_err() {
                            break;
                        }
                    }
                });
                let hello = AgentMessage::Hello {
                    role: state.run_mode.role(),
                    config_summary: config.safe_summary(),
                    notification_channels: notify::freedesktop_notification_channel(&config)
                        .into_iter()
                        .collect(),
                };
                tx.send(Message::Text(serde_json::to_string(&hello)?.into()))?;
                let mut heartbeat =
                    tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut last_heartbeat_ack = Instant::now();
                loop {
                    tokio::select! {
                        maybe_message = read.next() => {
                            let Some(message) = maybe_message else {
                                log_warn("hub connection closed".to_string());
                                break;
                            };
                            let message = match message {
                                Ok(Message::Text(text)) => text.to_string(),
                                Ok(Message::Close(frame)) => {
                                    log_warn(format!("hub closed websocket; frame={frame:?}"));
                                    break;
                                }
                                Ok(Message::Pong(_)) => {
                                    last_heartbeat_ack = Instant::now();
                                    continue;
                                }
                                Ok(_) => continue,
                                Err(error) => {
                                    log_warn(format!("hub websocket error: {error}"));
                                    break;
                                }
                            };
                            let value: serde_json::Value = match serde_json::from_str(&message) {
                                Ok(value) => value,
                                Err(error) => {
                                    log_warn(format!("ignored invalid hub message: {error}"));
                                    continue;
                                }
                            };
                            if let Ok(message) = serde_json::from_value::<HubMessage>(value.clone()) {
                                match message {
                                    HubMessage::HeartbeatAck { .. } => {
                                        last_heartbeat_ack = Instant::now();
                                    }
                                    HubMessage::ConfirmationResponse { request_id, decision, reason } => {
                                        let value = confirmation::confirmation_decision_value(decision);
                                        log_info(format!(
                                            "confirmation response received; requestId={request_id}; decision={value}; reason={reason}"
                                        ));
                                        if let Some(sender) = state.pending_confirmations.lock().await.remove(&request_id) {
                                            let _ = sender.send(value);
                                        }
                                    }
                                }
                                continue;
                            }
                            let command: HubCommand = match serde_json::from_value(value) {
                                Ok(command) => command,
                                Err(error) => {
                                    log_warn(format!("ignored unknown hub command: {error}"));
                                    continue;
                                }
                            };
                            let command_state = state.clone();
                            tokio::spawn(async move {
                                if let Err(error) = handle_hub_command(command_state, command).await {
                                    log_warn(format!("hub command failed: {error}"));
                                }
                            });
                        }
                        _ = heartbeat.tick() => {
                            if last_heartbeat_ack.elapsed() > Duration::from_secs(HEARTBEAT_ACK_TIMEOUT_SECS) {
                                log_warn("heartbeat ack timeout; reconnecting".to_string());
                                break;
                            }
                            let heartbeat = AgentMessage::Heartbeat { sent_at: Utc::now() };
                            if let Err(error) = tx.send(Message::Text(serde_json::to_string(&heartbeat)?.into())) {
                                log_warn(format!("heartbeat send failed: {error}"));
                                break;
                            }
                        }
                    }
                }
                *state.hub_sender.lock().await = None;
                confirmation::fail_pending_confirmations(&state, "provider_unavailable").await;
                writer.abort();
            }
        }
        log_info(format!("reconnecting in {RECONNECT_DELAY_SECS}s"));
        sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
    }
}

async fn connect_hub(
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
    proxy: Option<String>,
) -> Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, WsResponse)> {
    let Some(proxy) = proxy else {
        return connect_async(request)
            .await
            .map_err(|error| anyhow!("{error}"));
    };
    let host = request
        .uri()
        .host()
        .ok_or_else(|| anyhow!("hub URL is missing host"))?
        .to_string();
    let port = request.uri().port_u16().unwrap_or_else(|| {
        if request.uri().scheme_str() == Some("ws") {
            80
        } else {
            443
        }
    });
    let proxy_addr = parse_http_proxy_addr(&proxy)?;
    let mut stream = TcpStream::connect(proxy_addr).await?;
    let connect_request = format!(
        "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Connection: Keep-Alive\r\n\r\n"
    );
    stream.write_all(connect_request.as_bytes()).await?;

    let mut response = Vec::with_capacity(1024);
    let mut buffer = [0_u8; 512];
    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Err(anyhow!("proxy closed before CONNECT response completed"));
        }
        response.extend_from_slice(&buffer[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if response.len() > 8192 {
            return Err(anyhow!("proxy CONNECT response too large"));
        }
    }
    let response_text = String::from_utf8_lossy(&response);
    let status_ok = response_text
        .lines()
        .next()
        .map(|line| line.contains(" 200 "))
        .unwrap_or(false);
    if !status_ok {
        return Err(anyhow!(
            "proxy CONNECT failed: {}",
            response_text.lines().next().unwrap_or("<empty response>")
        ));
    }

    client_async_tls_with_config(request, stream, None, None)
        .await
        .map_err(|error| anyhow!("{error}"))
}

fn proxy_url(target_url: &str) -> Option<String> {
    if should_bypass_proxy(target_url) {
        return None;
    }
    ["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.trim().is_empty())
}

fn should_bypass_proxy(target_url: &str) -> bool {
    let host = target_url
        .split("://")
        .nth(1)
        .unwrap_or(target_url)
        .split('/')
        .next()
        .unwrap_or(target_url)
        .split(':')
        .next()
        .unwrap_or(target_url);
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    let no_proxy = std::env::var("no_proxy")
        .or_else(|_| std::env::var("NO_PROXY"))
        .unwrap_or_default();
    no_proxy.split(',').any(|entry| {
        let entry = entry.trim();
        !entry.is_empty()
            && (entry == "*" || host == entry || host.ends_with(entry.trim_start_matches('.')))
    })
}

fn parse_http_proxy_addr(proxy: &str) -> Result<String> {
    let trimmed = proxy.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    if without_scheme.contains('@') {
        return Err(anyhow!("proxy authentication is not supported"));
    }
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    if authority.contains(':') {
        Ok(authority.to_string())
    } else {
        Ok(format!("{authority}:8080"))
    }
}

pub(crate) async fn handle_hub_command(state: AppState, command: HubCommand) -> Result<()> {
    match command {
        HubCommand::Exec {
            request_id,
            task_id,
            payload,
        } => {
            log_info(format!(
                "exec received; taskId={task_id}; command={}",
                command_preview(&payload.program, &payload.args)
            ));
            let result = exec::run_exec_task(state.clone(), task_id, payload).await;
            log_info(format!(
                "exec finished; taskId={}; status={}; exitCode={:?}; rejectReason={:?}",
                result.task_id, result.status, result.exit_code, result.reject_reason
            ));
            send_response(&state, &request_id, serde_json::to_value(&result)?).await?;
        }
        HubCommand::BatchExec {
            request_id,
            task_id,
            payload,
        } => {
            log_info(format!(
                "batchExec received; batchId={task_id}; elements={}",
                payload.elements.len()
            ));
            let result = exec::run_batch_task(state.clone(), task_id, payload).await;
            log_info(format!(
                "batchExec finished; batchId={}; status={}; results={}",
                result.batch_id,
                result.status,
                result.results.len()
            ));
            send_response(&state, &request_id, serde_json::to_value(&result)?).await?;
        }
        HubCommand::StartSession {
            request_id,
            session_id,
            payload,
        } => {
            log_info(format!(
                "startSession received; sessionId={session_id}; command={}",
                command_preview(&payload.program, &payload.args)
            ));
            let info = sessions::start_session(state.clone(), session_id, payload).await;
            log_info(format!(
                "startSession result; sessionId={}; state={}; rejectReason={:?}",
                info.session_id, info.state, info.reject_reason
            ));
            send_session(&state, &info).await?;
            send_response(&state, &request_id, serde_json::to_value(&info)?).await?;
        }
        HubCommand::ListSessions { request_id } => {
            let sessions = sessions::current_sessions(&state).await;
            send_response(&state, &request_id, serde_json::to_value(sessions)?).await?;
        }
        HubCommand::InspectSession {
            request_id,
            session_id,
        } => {
            let session = sessions::inspect_session(&state, &session_id).await;
            send_response(&state, &request_id, serde_json::to_value(session)?).await?;
        }
        HubCommand::WaitSession {
            request_id,
            session_id,
            seconds,
        } => {
            sleep(Duration::from_secs(seconds.min(30))).await;
            let session = sessions::inspect_session(&state, &session_id).await;
            send_response(&state, &request_id, serde_json::to_value(session)?).await?;
        }
        HubCommand::KillSession {
            request_id,
            session_id,
        } => {
            log_info(format!("killSession received; sessionId={session_id}"));
            let session = sessions::kill_session(&state, &session_id).await;
            send_response(&state, &request_id, serde_json::to_value(session)?).await?;
        }
        HubCommand::McpListServers { request_id } => {
            let result = mcp::list_servers(&state).await;
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::McpListTools {
            request_id,
            payload,
        } => {
            let result = match mcp::list_tools(&state, payload).await {
                Ok(result) => result,
                Err(error) => serde_json::json!({
                    "error": {
                        "code": "mcp_list_tools_failed",
                        "message": error.to_string()
                    }
                }),
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::McpCallTool {
            request_id,
            payload,
        } => {
            let result = match mcp::call_tool(&state, payload).await {
                Ok(result) => result,
                Err(error) => serde_json::json!({
                    "error": {
                        "code": "mcp_call_tool_failed",
                        "message": error.to_string()
                    }
                }),
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::UserNotifyDeliver {
            request_id,
            payload,
        } => {
            let result = notify::deliver_freedesktop_notification(payload).await;
            send_response(&state, &request_id, serde_json::to_value(result)?).await?;
        }
        HubCommand::RoomNotebookAppend {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match notebook::append(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_notebook_append_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::RoomNotebookRecent {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match notebook::recent(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_notebook_recent_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::RoomNotebookSelectExact {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match notebook::select_exact(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_notebook_select_exact_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::RoomNotebookSearch {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match notebook::search(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_notebook_search_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::RoomNotebookCurrent {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match notebook::current(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_notebook_current_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::RoomNotebookUpdate {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match notebook::update(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => notebook_command_error("room_notebook_update_failed", error),
                }
            };
            send_response(&state, &request_id, result).await?;
        }
        HubCommand::RoomNotebookRemove {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match notebook::remove(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => notebook_command_error("room_notebook_remove_failed", error),
                }
            };
            send_response(&state, &request_id, result).await?;
        }
    }
    Ok(())
}

fn notebook_command_error(default_code: &str, error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = if message == "not_found" {
        "not_found"
    } else if message.starts_with("validation_error")
        || message.ends_with("_required")
        || message.ends_with("_too_long")
    {
        "validation_error"
    } else {
        default_code
    };
    serde_json::json!({
        "error": {
            "code": code,
            "message": if code == "not_found" { "passage not found" } else { &message }
        }
    })
}

pub(crate) fn room_agent_required_error() -> serde_json::Value {
    serde_json::json!({
        "error": {
            "code": "room_agent_required",
            "message": "room notebook commands require run-as-room"
        }
    })
}

async fn send_session(state: &AppState, session: &SessionInfo) -> Result<()> {
    send_agent_message(
        state,
        AgentMessage::SessionUpdate {
            session: session.clone(),
        },
    )
    .await
}

async fn send_response(state: &AppState, request_id: &str, data: serde_json::Value) -> Result<()> {
    send_agent_message(
        state,
        AgentMessage::Response {
            request_id: request_id.to_string(),
            data,
        },
    )
    .await
}

pub(crate) async fn send_agent_message(state: &AppState, message: AgentMessage) -> Result<()> {
    let sender = state
        .hub_sender
        .lock()
        .await
        .clone()
        .ok_or_else(|| anyhow!("hub_sender_unavailable"))?;
    sender
        .send(Message::Text(serde_json::to_string(&message)?.into()))
        .map_err(|_| anyhow!("hub_send_failed"))?;
    Ok(())
}
