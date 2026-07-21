use agentic_gpt_protocol::{
    AgentMessage, ExecRequest, HubCommand, HubCommandEnvelope, HubMessage, SessionInfo,
    SkillRunRequest, SkillRunResponse,
};
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
use uuid::Uuid;

use crate::{
    bootstrap,
    config::Config,
    confirmation, diary, exec, mcp, notebook, notify, sessions, skills, tmux, transport_ledger,
    utils::{
        command_preview, log_info, log_warn, CONNECT_TIMEOUT_SECS, HEARTBEAT_ACK_TIMEOUT_SECS,
        HEARTBEAT_INTERVAL_SECS, RECONNECT_DELAY_SECS,
    },
    AppState, RunMode,
};

pub(crate) async fn connect_loop(state: AppState) -> Result<()> {
    loop {
        let config = state.config.read().await.clone();
        if config.hub_transport == "sse" {
            match connect_sse(state.clone(), config).await {
                Err(error) => log_warn(format!("sse connection failed: {error}")),
                Ok(()) => log_warn("sse connection closed".to_string()),
            }
            confirmation::fail_pending_confirmations(&state, "provider_unavailable").await;
            log_info(format!("reconnecting in {RECONNECT_DELAY_SECS}s"));
            sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
            continue;
        }
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
                let (tx, mut rx) = mpsc::unbounded_channel::<AgentMessage>();
                *state.hub_sender.lock().await = Some(tx.clone());
                let writer = tokio::spawn(async move {
                    while let Some(message) = rx.recv().await {
                        let Ok(text) = serde_json::to_string(&message) else {
                            break;
                        };
                        if write.send(Message::Text(text.into())).await.is_err() {
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
                tx.send(hello)?;
                reconcile_transport_runs(&state, &tx).await;
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
                            let envelope: HubCommandEnvelope = match serde_json::from_value(value) {
                                Ok(envelope) => envelope,
                                Err(error) => {
                                    log_warn(format!("ignored unknown reliable hub envelope: {error}"));
                                    continue;
                                }
                            };
                            handle_reliable_envelope(&state, &tx, envelope).await;
                        }
                        _ = heartbeat.tick() => {
                            if last_heartbeat_ack.elapsed() > Duration::from_secs(HEARTBEAT_ACK_TIMEOUT_SECS) {
                                log_warn("heartbeat ack timeout; reconnecting".to_string());
                                break;
                            }
                            let heartbeat = AgentMessage::Heartbeat { sent_at: Utc::now() };
                            if let Err(error) = tx.send(heartbeat) {
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

async fn connect_sse(state: AppState, config: Config) -> Result<()> {
    let connection_id = format!("conn_{}", Uuid::new_v4().simple());
    let base = config.hub_url.trim_end_matches('/');
    let events_url = format!(
        "{}/v1/agents/{}/events?connectionId={}",
        base, config.agent_id, connection_id
    );
    let messages_url = format!(
        "{}/v1/agents/{}/messages?connectionId={}",
        base, config.agent_id, connection_id
    );
    log_info(format!(
        "connecting to hub via sse; agentId={}; connectionId={}",
        config.agent_id, connection_id
    ));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(|error| anyhow!("{error}"))?;
    let response = client
        .get(events_url)
        .header("x-agent-secret", &config.agent_secret)
        .send()
        .await
        .map_err(|error| anyhow!("{error}"))?;
    if !response.status().is_success() {
        return Err(anyhow!("sse connect failed: {}", response.status()));
    }
    log_info("connected to hub via sse".to_string());

    let (tx, mut rx) = mpsc::unbounded_channel::<AgentMessage>();
    *state.hub_sender.lock().await = Some(tx.clone());
    let post_client = client.clone();
    let agent_secret = config.agent_secret.clone();
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            let mut delay = Duration::from_millis(250);
            loop {
                let response = post_client
                    .post(&messages_url)
                    .header("x-agent-secret", &agent_secret)
                    .json(&message)
                    .send()
                    .await;
                match response {
                    Ok(response) if response.status().is_success() => break,
                    Ok(response) => {
                        if classify_sse_post_status(response.status()) == SsePostStatus::Stale {
                            log_warn(
                                "sse post rejected as stale connection; stopping writer"
                                    .to_string(),
                            );
                            return;
                        }
                        log_warn(format!("sse post failed; status={}", response.status()));
                    }
                    Err(error) => log_warn(format!("sse post failed: {error}")),
                }
                sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
            }
        }
    });

    let heartbeat_tx = tx.clone();
    let heartbeat = tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            heartbeat.tick().await;
            if heartbeat_tx
                .send(AgentMessage::Heartbeat {
                    sent_at: Utc::now(),
                })
                .is_err()
            {
                break;
            }
        }
    });

    let result = async {
        tx.send(AgentMessage::Hello {
            role: state.run_mode.role(),
            config_summary: config.safe_summary(),
            notification_channels: notify::freedesktop_notification_channel(&config)
                .into_iter()
                .collect(),
        })?;
        reconcile_transport_runs(&state, &tx).await;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut data = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| anyhow!("{error}"))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(index) = buffer.find('\n') {
                let mut line = buffer[..index].to_string();
                buffer = buffer[index + 1..].to_string();
                if line.ends_with('\r') {
                    line.pop();
                }
                if line.is_empty() {
                    if !data.is_empty() {
                        handle_sse_data(&state, &tx, std::mem::take(&mut data)).await;
                    }
                    continue;
                }
                if let Some(value) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value.trim_start());
                }
            }
        }
        Ok(())
    }
    .await;
    writer.abort();
    heartbeat.abort();
    clear_hub_sender_if_current(&state, &tx).await;
    result
}

async fn clear_hub_sender_if_current(state: &AppState, tx: &mpsc::UnboundedSender<AgentMessage>) {
    let mut current = state.hub_sender.lock().await;
    if current
        .as_ref()
        .map(|sender| sender.same_channel(tx))
        .unwrap_or(false)
    {
        *current = None;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SsePostStatus {
    Delivered,
    Stale,
    Retry,
}

pub(crate) fn classify_sse_post_status(status: reqwest::StatusCode) -> SsePostStatus {
    if status.is_success() {
        SsePostStatus::Delivered
    } else if status == reqwest::StatusCode::CONFLICT {
        SsePostStatus::Stale
    } else {
        SsePostStatus::Retry
    }
}

async fn handle_sse_data(state: &AppState, tx: &mpsc::UnboundedSender<AgentMessage>, data: String) {
    if let Ok(message) = serde_json::from_str::<HubMessage>(&data) {
        match message {
            HubMessage::HeartbeatAck { .. } => {}
            HubMessage::ConfirmationResponse {
                request_id,
                decision,
                reason,
            } => {
                let value = confirmation::confirmation_decision_value(decision);
                log_info(format!(
                    "confirmation response received; requestId={request_id}; decision={value}; reason={reason}"
                ));
                if let Some(sender) = state.pending_confirmations.lock().await.remove(&request_id) {
                    let _ = sender.send(value);
                }
            }
        }
        return;
    }
    let envelope = match serde_json::from_str::<HubCommandEnvelope>(&data) {
        Ok(envelope) => envelope,
        Err(error) => {
            log_warn(format!("ignored invalid sse envelope: {error}"));
            return;
        }
    };
    handle_reliable_envelope(state, tx, envelope).await;
}

async fn handle_reliable_envelope(
    state: &AppState,
    tx: &mpsc::UnboundedSender<AgentMessage>,
    envelope: HubCommandEnvelope,
) {
    let outcome = match transport_ledger::accept(&envelope) {
        Ok(outcome) => outcome,
        Err(error) => {
            log_warn(format!("transport ledger accept failed: {error}"));
            return;
        }
    };
    let _ = tx.send(transport_ledger::ack_message(&envelope));
    match outcome {
        transport_ledger::AcceptOutcome::HashMismatch => {
            let _ = tx.send(AgentMessage::TransportRunStatus {
                run_id: envelope.run_id,
                request_id: envelope.request_id,
                status: "failed".to_string(),
                reason: Some("command_hash_mismatch".to_string()),
            });
            return;
        }
        transport_ledger::AcceptOutcome::Completed(result) => {
            let _ = tx.send(AgentMessage::Response {
                run_id: Some(envelope.run_id),
                request_id: envelope.request_id,
                data: result,
            });
            return;
        }
        transport_ledger::AcceptOutcome::DuplicateStarted => return,
        transport_ledger::AcceptOutcome::FirstAccepted
        | transport_ledger::AcceptOutcome::DuplicateAccepted => {}
    }
    if let Err(error) = transport_ledger::mark_started(&envelope.run_id) {
        log_warn(format!("transport ledger mark started failed: {error}"));
        return;
    }
    let command_state = state.clone();
    tokio::spawn(async move {
        if let Err(error) =
            handle_hub_command(command_state, envelope.command, Some(envelope.run_id)).await
        {
            log_warn(format!("hub command failed: {error}"));
        }
    });
}

async fn reconcile_transport_runs(state: &AppState, tx: &mpsc::UnboundedSender<AgentMessage>) {
    let records = match transport_ledger::latest_records() {
        Ok(records) => records,
        Err(error) => {
            log_warn(format!("transport ledger scan failed: {error}"));
            return;
        }
    };
    for record in records.into_values() {
        match record.status.as_str() {
            "completed" => {
                if let Some(message) = transport_ledger::completed_response(&record) {
                    let _ = tx.send(message);
                }
            }
            "accepted" => {
                let Some(command) = record.command.clone() else {
                    continue;
                };
                if let Err(error) = transport_ledger::mark_started(&record.run_id) {
                    log_warn(format!("transport ledger mark started failed: {error}"));
                    continue;
                }
                let run_id = record.run_id.clone();
                let command_state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) =
                        handle_hub_command(command_state, command, Some(run_id)).await
                    {
                        log_warn(format!("hub command failed during reconciliation: {error}"));
                    }
                });
            }
            "started" | "running" => {
                let _ = tx.send(AgentMessage::TransportRunStatus {
                    run_id: record.run_id,
                    request_id: record.request_id,
                    status: "unknown".to_string(),
                    reason: Some("agent_restarted_before_completion".to_string()),
                });
            }
            _ => {}
        }
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

pub(crate) async fn handle_hub_command(
    state: AppState,
    command: HubCommand,
    run_id: Option<String>,
) -> Result<()> {
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
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(&result)?,
            )
            .await?;
        }
        HubCommand::BatchExec {
            request_id,
            task_id,
            payload,
        } => {
            log_info(format!(
                "process.batchExec received; batchId={task_id}; elements={}",
                payload.elements.len()
            ));
            let result = exec::run_batch_task(state.clone(), task_id, payload).await;
            log_info(format!(
                "process.batchExec finished; batchId={}; status={}; results={}",
                result.batch_id,
                result.status,
                result.results.len()
            ));
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(&result)?,
            )
            .await?;
        }
        HubCommand::StartSession {
            request_id,
            session_id,
            payload,
        } => {
            log_info(format!(
                "session.start received; sessionId={session_id}; command={}",
                command_preview(&payload.program, &payload.args)
            ));
            let info = sessions::start_session_async(state.clone(), session_id, payload).await;
            log_info(format!(
                "session.start result; sessionId={}; state={}; rejectReason={:?}",
                info.session_id, info.state, info.reject_reason
            ));
            send_session(&state, &info).await?;
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(&info)?,
            )
            .await?;
        }
        HubCommand::ListSessions { request_id } => {
            let sessions = sessions::current_sessions(&state).await;
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(sessions)?,
            )
            .await?;
        }
        HubCommand::InspectSession {
            request_id,
            session_id,
        } => {
            let session = sessions::inspect_session(&state, &session_id).await;
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(session)?,
            )
            .await?;
        }
        HubCommand::WaitSession {
            request_id,
            session_id,
            seconds,
        } => {
            let deadline = Instant::now() + Duration::from_secs(seconds.min(30));
            let mut session = sessions::inspect_session(&state, &session_id).await;
            while let Some(info) = session.as_ref() {
                if !matches!(
                    info.state.as_str(),
                    "starting" | "running" | "waiting_confirmation"
                ) || Instant::now() >= deadline
                {
                    break;
                }
                sleep(Duration::from_millis(20)).await;
                session = sessions::inspect_session(&state, &session_id).await;
            }
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(session)?,
            )
            .await?;
        }
        HubCommand::KillSession {
            request_id,
            session_id,
        } => {
            log_info(format!("session.kill received; sessionId={session_id}"));
            let session = sessions::kill_session(&state, &session_id).await;
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(session)?,
            )
            .await?;
        }
        HubCommand::TmuxListSessions { request_id } => {
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                tmux::list_sessions().await,
            )
            .await?;
        }
        HubCommand::TmuxListPanes {
            request_id,
            payload,
        } => {
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                tmux::list_panes(payload).await,
            )
            .await?;
        }
        HubCommand::TmuxCapturePane {
            request_id,
            payload,
        } => {
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                tmux::capture_pane(payload).await,
            )
            .await?;
        }
        HubCommand::TmuxPasteText {
            request_id,
            payload,
        } => {
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                tmux::paste_text(&state, payload).await,
            )
            .await?;
        }
        HubCommand::TmuxExec {
            request_id,
            payload,
        } => {
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                tmux::exec(&state, payload).await,
            )
            .await?;
        }
        HubCommand::TmuxCreateSession {
            request_id,
            payload,
        } => {
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                tmux::create_session(&state, payload).await,
            )
            .await?;
        }
        HubCommand::TmuxCloseSession {
            request_id,
            payload,
        } => {
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                tmux::close_session(&state, payload).await,
            )
            .await?;
        }
        HubCommand::McpListServers { request_id } => {
            let result = mcp::list_servers(&state).await;
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::UserNotifyDeliver {
            request_id,
            payload,
        } => {
            let result = notify::deliver_freedesktop_notification(payload).await;
            send_response(
                &state,
                run_id.as_deref(),
                &request_id,
                serde_json::to_value(result)?,
            )
            .await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
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
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::RoomDiaryAppend {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match diary::append(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_diary_append_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::RoomDiaryRecent {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match diary::recent(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_diary_recent_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::RoomDiarySelectExact {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match diary::select_exact(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => serde_json::json!({
                        "error": { "code": "room_diary_select_exact_failed", "message": error.to_string() }
                    }),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::RoomBootstrap { request_id } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match bootstrap::load(&state).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => bootstrap_command_error("bootstrap_read_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::RoomBootstrapRead {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match bootstrap::read(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => bootstrap_command_error("bootstrap_read_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsList { request_id } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match skills::list(&state).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => skills_command_error("skills_list_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsRead {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match skills::read(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => skills_command_error("skills_read_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsSearch {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match skills::search(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => skills_command_error("skills_search_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsActive { request_id } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match skills::active(&state).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => skills_command_error("skills_active_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsActivate {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match skills::activate(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => skills_command_error("skills_activate_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsDeactivate {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match skills::deactivate(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => skills_command_error("skills_deactivate_failed", error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsInstall {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match state.skill_installs.start(state.clone(), payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => install_command_error(error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsInstallGet {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match state.skill_installs.get(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => install_command_error(error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsInstallCancel {
            request_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                match state.skill_installs.cancel(&state, payload).await {
                    Ok(result) => serde_json::to_value(result)?,
                    Err(error) => install_command_error(error),
                }
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
        HubCommand::SkillsRun {
            request_id,
            session_id,
            payload,
        } => {
            let result = if state.run_mode != RunMode::Room {
                room_agent_required_error()
            } else {
                run_skill(&state, session_id, payload).await
            };
            send_response(&state, run_id.as_deref(), &request_id, result).await?;
        }
    }
    Ok(())
}

fn skills_command_error(default_code: &str, error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = match message.as_str() {
        "invalid_id" | "query_required" => "validation_error",
        "not_found" => "not_found",
        _ => default_code,
    };
    serde_json::json!({
        "error": {
            "code": code,
            "message": if code == "not_found" { "skill not found" } else { &message }
        }
    })
}

fn bootstrap_command_error(default_code: &str, error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = match message.as_str() {
        "bootstrap_not_found"
        | "guide_not_found"
        | "bootstrap_invalid"
        | "bootstrap_read_failed" => message.as_str(),
        _ => default_code,
    };
    serde_json::json!({ "error": { "code": code, "message": message } })
}

fn install_command_error(error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = match message.as_str() {
        "install_not_found" => "install_not_found",
        "target_exists" => "target_exists",
        "idempotency_conflict" => "idempotency_conflict",
        "reserved_id" => "reserved_id",
        "invalid_id"
        | "invalid_files"
        | "invalid_file_source"
        | "invalid_path"
        | "duplicate_path"
        | "invalid_base64"
        | "package_limit_exceeded"
        | "download_blocked"
        | "invalid_github_source"
        | "invalid_github_repository"
        | "invalid_github_url"
        | "unsupported_github_host"
        | "ambiguous_github_url"
        | "invalid_idempotency_key" => "validation_error",
        _ => "skills_install_failed",
    };
    serde_json::json!({ "error": { "code": code, "message": message } })
}

async fn run_skill(
    state: &AppState,
    session_id: String,
    request: SkillRunRequest,
) -> serde_json::Value {
    let program = match skills::resolve_run_program(state, &request).await {
        Ok(program) => program,
        Err(error) => return skill_run_command_error(error),
    };
    let config = state.config.read().await.clone();
    let wait_seconds = request.effective_wait_seconds();
    if let Some(working_directory) = request.working_directory.as_deref() {
        if let Err(reason) = exec::resolve_working_directory(&config, Some(working_directory)) {
            return serde_json::json!({
                "error": { "code": "invalid_working_directory", "message": reason }
            });
        }
    }
    let info = sessions::start_skill_session_async(
        state.clone(),
        session_id.clone(),
        ExecRequest {
            agent_id: config.agent_id.clone(),
            program: program.to_string_lossy().to_string(),
            args: request.args.unwrap_or_default(),
            need_confirm: false,
            confirm_method: None,
            working_directory: request.working_directory,
        },
        &request.id,
        &request.path,
    )
    .await;
    let info = wait_for_skill_session(state, info, wait_seconds).await;
    let completed_inline = !matches!(
        info.state.as_str(),
        "starting" | "running" | "waiting_confirmation"
    );
    let response = SkillRunResponse {
        agent_id: config.agent_id,
        session_id,
        completed_inline,
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        session: info,
    };
    serde_json::to_value(response).unwrap_or_else(|_| {
        serde_json::json!({
            "error": { "code": "skills_run_failed", "message": "failed to encode skill run response" }
        })
    })
}

async fn wait_for_skill_session(
    state: &AppState,
    mut info: SessionInfo,
    wait_seconds: u64,
) -> SessionInfo {
    if wait_seconds == 0 {
        return info;
    }
    let deadline = Instant::now() + Duration::from_secs(wait_seconds.min(30));
    while matches!(
        info.state.as_str(),
        "starting" | "running" | "waiting_confirmation"
    ) && Instant::now() < deadline
    {
        sleep(Duration::from_millis(20)).await;
        if let Some(latest) = sessions::inspect_session(state, &info.session_id).await {
            info = latest;
        } else {
            break;
        }
    }
    info
}

fn skill_run_command_error(error: anyhow::Error) -> serde_json::Value {
    let message = error.to_string();
    let code = match message.as_str() {
        "invalid_id"
        | "skill_inactive"
        | "skill_not_runnable"
        | "invalid_script_path"
        | "script_path_forbidden"
        | "script_not_found"
        | "script_not_executable"
        | "script_symlink"
        | "invalid_working_directory" => message.as_str(),
        _ => "skills_run_failed",
    };
    serde_json::json!({ "error": { "code": code, "message": message } })
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
            "message": "room commands require run-as-room"
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

async fn send_response(
    state: &AppState,
    run_id: Option<&str>,
    request_id: &str,
    data: serde_json::Value,
) -> Result<()> {
    if let Some(run_id) = run_id {
        transport_ledger::mark_completed(run_id, request_id, &data)?;
    }
    send_agent_message(
        state,
        AgentMessage::Response {
            run_id: run_id.map(|value| value.to_string()),
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
        .send(message)
        .map_err(|_| anyhow!("hub_send_failed"))?;
    Ok(())
}
