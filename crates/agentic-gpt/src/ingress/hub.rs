use agentic_gpt_protocol::{
    AgentConnectionMode, AgentMessage, AgentRunReport, BoundedJsonValue, EventOrigin,
    EventResponseDisposition, EventSource, HubCommand, HubCommandEnvelope, HubMessage, ProcessInfo,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
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
    config::Config,
    confirmation, notify,
    operation::{hub_command_name, RequestContext, RequestIngress},
    process,
    state::AppState,
    transport_ledger::{self, StoredCommand},
    utils::{
        log_info, log_warn, CONNECT_TIMEOUT_SECS, HEARTBEAT_ACK_TIMEOUT_SECS,
        HEARTBEAT_INTERVAL_SECS, RECONNECT_DELAY_SECS,
    },
};

pub(crate) async fn connect_loop(state: AppState) -> Result<()> {
    if state.runtime.hub_mode == crate::state::HubMode::ReportingOnly {
        return connect_reporting_loop(state).await;
    }
    loop {
        let config = state.config.read().await.clone();
        if config.hub.transport == "sse" {
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
            config.hub.url.trim_end_matches('/'),
            config.agent_id
        )
        .replace("http://", "ws://")
        .replace("https://", "wss://");
        let mut request = url.into_client_request()?;
        request
            .headers_mut()
            .insert("x-agent-secret", config.hub.agent_secret.parse()?);

        let proxy = proxy_url(&config.hub.url);
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
                    boot_generation: state.boot_generation.clone(),
                    role: state.runtime.profile.role(),
                    connection_mode: AgentConnectionMode::CommandCapable,
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

async fn connect_reporting_loop(state: AppState) -> Result<()> {
    loop {
        let config = state.config.read().await.clone();
        let enabled = config
            .tunnel
            .as_ref()
            .map(|tunnel| tunnel.hub_reporting.enabled)
            .unwrap_or(false);
        if !enabled {
            return Ok(());
        }
        let result = if config.hub.transport == "sse" {
            connect_reporting_sse(state.clone(), config).await
        } else {
            connect_reporting_websocket(state.clone(), config).await
        };
        if let Err(error) = result {
            log_warn(format!("hub reporting connection failed: {error}"));
        }
        confirmation::fail_pending_confirmations(&state, "provider_unavailable").await;
        log_info(format!(
            "reconnecting reporting connection in {RECONNECT_DELAY_SECS}s"
        ));
        sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
    }
}

async fn connect_reporting_websocket(state: AppState, config: Config) -> Result<()> {
    let url = format!(
        "{}/v1/agents/{}/connect",
        config.hub.url.trim_end_matches('/'),
        config.agent_id
    )
    .replace("http://", "ws://")
    .replace("https://", "wss://");
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("x-agent-secret", config.hub.agent_secret.parse()?);
    let proxy = proxy_url(&config.hub.url);
    let (stream, _) = timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        connect_hub(request, proxy),
    )
    .await
    .map_err(|_| anyhow!("hub reporting connect timeout"))??;
    let (mut write, mut read) = stream.split();
    let (control_tx, mut control_rx) = mpsc::unbounded_channel::<AgentMessage>();
    let (event_tx, mut event_rx) = mpsc::channel::<AgentMessage>(64);
    *state.hub_sender.lock().await = Some(control_tx.clone());
    *state.reporting_sender.lock().await = Some(event_tx.clone());
    let writer = tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                biased;
                Some(message) = control_rx.recv() => message,
                Some(message) = event_rx.recv() => message,
                else => break,
            };
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if write.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    control_tx.send(AgentMessage::Hello {
        boot_generation: state.boot_generation.clone(),
        role: state.runtime.profile.role(),
        connection_mode: AgentConnectionMode::ReportingOnly,
        config_summary: config.safe_summary(),
        notification_channels: notify::freedesktop_notification_channel(&config)
            .into_iter()
            .collect(),
    })?;
    log_info(format!(
        "hub reporting connected; transport=websocket; agentId={}",
        config.agent_id
    ));
    send_current_process_snapshots(&state, &event_tx).await;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_heartbeat_ack = Instant::now();
    loop {
        tokio::select! {
            maybe_message = read.next() => {
                let Some(message) = maybe_message else { break; };
                let text = match message {
                    Ok(Message::Text(text)) => text.to_string(),
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Pong(_)) => { last_heartbeat_ack = Instant::now(); continue; }
                    Ok(_) => continue,
                    Err(error) => return Err(anyhow!("hub reporting websocket error: {error}")),
                };
                handle_reporting_inbound(&state, &text).await;
            }
            _ = heartbeat.tick() => {
                if last_heartbeat_ack.elapsed() > Duration::from_secs(HEARTBEAT_ACK_TIMEOUT_SECS) {
                    return Err(anyhow!("hub reporting heartbeat timeout"));
                }
                if control_tx.send(AgentMessage::Heartbeat { sent_at: Utc::now() }).is_err() {
                    break;
                }
            }
        }
    }
    writer.abort();
    clear_reporting_senders(&state, &control_tx, &event_tx).await;
    log_info("hub reporting disconnected; transport=websocket".to_string());
    Ok(())
}

async fn connect_reporting_sse(state: AppState, config: Config) -> Result<()> {
    let connection_id = format!("conn_{}", Uuid::new_v4().simple());
    let base = config.hub.url.trim_end_matches('/');
    let events_url = format!(
        "{}/v1/agents/{}/events?connectionId={}",
        base, config.agent_id, connection_id
    );
    let messages_url = format!(
        "{}/v1/agents/{}/messages?connectionId={}",
        base, config.agent_id, connection_id
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(|error| anyhow!("{error}"))?;
    let response = client
        .get(events_url)
        .header("x-agent-secret", &config.hub.agent_secret)
        .send()
        .await
        .map_err(|error| anyhow!("{error}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "sse reporting connect failed: {}",
            response.status()
        ));
    }
    let (control_tx, mut control_rx) = mpsc::unbounded_channel::<AgentMessage>();
    let (event_tx, mut event_rx) = mpsc::channel::<AgentMessage>(64);
    *state.hub_sender.lock().await = Some(control_tx.clone());
    *state.reporting_sender.lock().await = Some(event_tx.clone());
    let post_client = client.clone();
    let post_url = messages_url.clone();
    let post_secret = config.hub.agent_secret.clone();
    let writer = tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                biased;
                Some(message) = control_rx.recv() => message,
                Some(message) = event_rx.recv() => message,
                else => break,
            };
            let result = post_client
                .post(&post_url)
                .header("x-agent-secret", &post_secret)
                .json(&message)
                .send()
                .await;
            if let Err(error) = result {
                log_warn(format!("sse reporting event dropped: {error}"));
            }
        }
    });
    control_tx.send(AgentMessage::Hello {
        boot_generation: state.boot_generation.clone(),
        role: state.runtime.profile.role(),
        connection_mode: AgentConnectionMode::ReportingOnly,
        config_summary: config.safe_summary(),
        notification_channels: notify::freedesktop_notification_channel(&config)
            .into_iter()
            .collect(),
    })?;
    log_info(format!(
        "hub reporting connected; transport=sse; agentId={}",
        config.agent_id
    ));
    send_current_process_snapshots(&state, &event_tx).await;
    let heartbeat_tx = control_tx.clone();
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
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
                    handle_reporting_inbound(&state, &data).await;
                    data.clear();
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.trim_start());
            }
        }
    }
    writer.abort();
    heartbeat.abort();
    clear_reporting_senders(&state, &control_tx, &event_tx).await;
    log_info("hub reporting disconnected; transport=sse".to_string());
    Ok(())
}

async fn handle_reporting_inbound(state: &AppState, text: &str) {
    if let Ok(message) = serde_json::from_str::<HubMessage>(text) {
        match message {
            HubMessage::HeartbeatAck { .. } => {}
            HubMessage::ConfirmationResponse {
                request_id,
                decision,
                reason,
            } => {
                let value = confirmation::confirmation_decision_value(decision);
                log_info(format!(
                    "reporting confirmation response received; requestId={request_id}; decision={value}; reason={reason}"
                ));
                if let Some(sender) = state.pending_confirmations.lock().await.remove(&request_id) {
                    let _ = sender.send(value);
                }
            }
        }
    } else if serde_json::from_str::<HubCommandEnvelope>(text).is_ok() {
        log_warn("ignored execution command on reporting-only connection".to_string());
    }
}

async fn clear_reporting_senders(
    state: &AppState,
    control_tx: &mpsc::UnboundedSender<AgentMessage>,
    event_tx: &mpsc::Sender<AgentMessage>,
) {
    let mut control = state.hub_sender.lock().await;
    if control
        .as_ref()
        .map(|sender| sender.same_channel(control_tx))
        .unwrap_or(false)
    {
        *control = None;
    }
    let mut reporting = state.reporting_sender.lock().await;
    if reporting
        .as_ref()
        .map(|sender| sender.same_channel(event_tx))
        .unwrap_or(false)
    {
        *reporting = None;
    }
}

async fn send_current_process_snapshots(state: &AppState, sender: &mpsc::Sender<AgentMessage>) {
    for process in process::current_processes(state).await {
        let _ = sender.try_send(AgentMessage::ProcessUpdate {
            process: process_for_reporting(state, process),
        });
    }
}

const REPORT_MAX_JSON_BYTES: usize = 16 * 1024;

#[allow(clippy::too_many_arguments)]
pub(crate) fn report_run_event(
    state: &AppState,
    run_id: &str,
    request_id: &str,
    tool_name: &str,
    status: &str,
    started_at: DateTime<Utc>,
    result: Option<serde_json::Value>,
    reason: Option<String>,
    process_id: Option<String>,
    exit_code: Option<i32>,
    process: Option<ProcessInfo>,
) {
    let detail = reporting_detail(state);
    let updated_at = Utc::now();
    let full = detail == "full";
    let arguments = None;
    let result = if full {
        result.map(bounded_json_value)
    } else {
        None
    };
    let process_id = process_id.or_else(|| process.as_ref().map(|value| value.process_id.clone()));
    let exit_code = exit_code.or_else(|| process.as_ref().and_then(|value| value.exit_code));
    try_send_reporting(
        state,
        AgentMessage::RunReport {
            report: Box::new(AgentRunReport {
                run_id: run_id.to_string(),
                request_id: request_id.to_string(),
                tool_name: tool_name.to_string(),
                source: "tunnel".to_string(),
                profile: state.runtime.profile.label().to_string(),
                detail,
                status: status.to_string(),
                started_at,
                updated_at,
                duration_ms: if status == "started" {
                    None
                } else {
                    Some((updated_at - started_at).num_milliseconds().max(0) as u64)
                },
                process_id,
                exit_code,
                reason: reason.map(|value| bounded_reason(&value)),
                arguments,
                result,
                process: if full { process } else { None },
            }),
        },
    );
}

pub(crate) fn report_tool_arguments(
    state: &AppState,
    run_id: &str,
    request_id: &str,
    tool_name: &str,
    arguments: serde_json::Value,
    started_at: DateTime<Utc>,
) {
    let detail = reporting_detail(state);
    let arguments = if detail == "full" {
        Some(bounded_json_value(arguments))
    } else {
        None
    };
    try_send_reporting(
        state,
        AgentMessage::RunReport {
            report: Box::new(AgentRunReport {
                run_id: run_id.to_string(),
                request_id: request_id.to_string(),
                tool_name: tool_name.to_string(),
                source: "tunnel".to_string(),
                profile: state.runtime.profile.label().to_string(),
                detail,
                status: "started".to_string(),
                started_at,
                updated_at: started_at,
                duration_ms: None,
                process_id: None,
                exit_code: None,
                reason: None,
                arguments,
                result: None,
                process: None,
            }),
        },
    );
}

pub(crate) fn report_process(state: &AppState, process: ProcessInfo) {
    try_send_reporting(
        state,
        AgentMessage::ProcessUpdate {
            process: process_for_reporting(state, process),
        },
    );
}

fn reporting_detail(state: &AppState) -> String {
    state
        .config
        .try_read()
        .ok()
        .and_then(|config| {
            config
                .tunnel
                .as_ref()
                .map(|tunnel| tunnel.hub_reporting.detail.to_string())
        })
        .unwrap_or_else(|| "metadata".to_string())
}

fn process_for_reporting(state: &AppState, mut process: ProcessInfo) -> ProcessInfo {
    if reporting_detail(state) == "metadata" {
        process.program = Some("<redacted>".to_string());
        process.args.clear();
        process.working_directory = None;
        process.command_preview = Some("<redacted>".to_string());
    }
    process
}

fn try_send_reporting(state: &AppState, message: AgentMessage) {
    let Ok(sender) = state.reporting_sender.try_lock() else {
        log_warn("hub reporting event dropped: queue lock unavailable".to_string());
        return;
    };
    let Some(sender) = sender.as_ref() else {
        return;
    };
    if sender.try_send(message).is_err() {
        log_warn("hub reporting event dropped: queue full or disconnected".to_string());
    }
}

fn bounded_json_value(value: serde_json::Value) -> BoundedJsonValue {
    let json = serde_json::to_vec(&value).unwrap_or_default();
    let byte_count = json.len();
    let digest = Sha256::digest(&json);
    let sha256 = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if byte_count <= REPORT_MAX_JSON_BYTES {
        return BoundedJsonValue {
            value,
            byte_count,
            sha256,
            truncated: false,
        };
    }
    BoundedJsonValue {
        value: serde_json::json!({
            "truncated": true,
            "byteCount": byte_count,
            "sha256": sha256,
        }),
        byte_count,
        sha256,
        truncated: true,
    }
}

fn bounded_reason(value: &str) -> String {
    const MAX_REASON_CHARS: usize = 2048;
    let mut output = value.chars().take(MAX_REASON_CHARS).collect::<String>();
    if value.chars().count() > MAX_REASON_CHARS {
        output.push('…');
    }
    output
}

async fn connect_sse(state: AppState, config: Config) -> Result<()> {
    let connection_id = format!("conn_{}", Uuid::new_v4().simple());
    let base = config.hub.url.trim_end_matches('/');
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
        .header("x-agent-secret", &config.hub.agent_secret)
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
    let agent_secret = config.hub.agent_secret.clone();
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
            boot_generation: state.boot_generation.clone(),
            role: state.runtime.profile.role(),
            connection_mode: AgentConnectionMode::CommandCapable,
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
    let agent_id = state.config.read().await.agent_id.clone();
    let outcome = match transport_ledger::accept(&envelope, &agent_id) {
        Ok(outcome) => outcome,
        Err(error) => {
            log_warn(format!("transport ledger accept failed: {error}"));
            return;
        }
    };
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
        transport_ledger::AcceptOutcome::OwnerMismatch => {
            let _ = tx.send(AgentMessage::TransportRunStatus {
                run_id: envelope.run_id,
                request_id: envelope.request_id,
                status: "unknown".to_string(),
                reason: Some("transport_owner_mismatch".to_string()),
            });
            return;
        }
        transport_ledger::AcceptOutcome::LegacyUnowned => {
            let _ = tx.send(AgentMessage::TransportRunStatus {
                run_id: envelope.run_id,
                request_id: envelope.request_id,
                status: "unknown".to_string(),
                reason: Some("legacy_transport_record_unowned".to_string()),
            });
            return;
        }
        transport_ledger::AcceptOutcome::Completed(result) => {
            let event_sources =
                match event_response_sources(state, hub_command_name(&envelope.command), &result) {
                    Ok(sources) => sources,
                    Err(error) => {
                        log_warn(format!("transport replay event metadata failed: {error}"));
                        return;
                    }
                };
            let _ = tx.send(transport_ledger::ack_message(&envelope));
            let _ = tx.send(AgentMessage::Response {
                run_id: Some(envelope.run_id),
                request_id: envelope.request_id,
                event_sources,
                data: result,
            });
            return;
        }
        transport_ledger::AcceptOutcome::FirstAccepted
        | transport_ledger::AcceptOutcome::DuplicateAccepted
        | transport_ledger::AcceptOutcome::DuplicateStarted => {}
    }
    let claim = match transport_ledger::claim_started(
        &envelope.run_id,
        &envelope.request_id,
        &envelope.command_hash,
        &agent_id,
    ) {
        Ok(claim) => claim,
        Err(error) => {
            log_warn(format!("transport ledger claim failed: {error}"));
            let _ = tx.send(AgentMessage::TransportRunStatus {
                run_id: envelope.run_id,
                request_id: envelope.request_id,
                status: "unknown".to_string(),
                reason: Some("transport_claim_failed".to_string()),
            });
            return;
        }
    };
    match claim {
        transport_ledger::ClaimOutcome::Claimed => {
            let _ = tx.send(transport_ledger::ack_message(&envelope));
            let identity = RunIdentity {
                run_id: envelope.run_id.clone(),
                request_id: envelope.request_id.clone(),
                command_hash: envelope.command_hash.clone(),
                agent_id,
            };
            let command_state = state.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    handle_hub_command(command_state, envelope.command, Some(identity)).await
                {
                    log_warn(format!("hub command failed: {error}"));
                }
            });
        }
        transport_ledger::ClaimOutcome::AlreadyStarted => {
            let _ = tx.send(transport_ledger::ack_message(&envelope));
        }
        transport_ledger::ClaimOutcome::Completed(result) => {
            let event_sources =
                match event_response_sources(state, hub_command_name(&envelope.command), &result) {
                    Ok(sources) => sources,
                    Err(error) => {
                        log_warn(format!("transport replay event metadata failed: {error}"));
                        return;
                    }
                };
            let _ = tx.send(transport_ledger::ack_message(&envelope));
            let _ = tx.send(AgentMessage::Response {
                run_id: Some(envelope.run_id),
                request_id: envelope.request_id,
                event_sources,
                data: result,
            });
        }
        transport_ledger::ClaimOutcome::Missing
        | transport_ledger::ClaimOutcome::Unowned
        | transport_ledger::ClaimOutcome::OwnerMismatch => {
            let _ = tx.send(AgentMessage::TransportRunStatus {
                run_id: envelope.run_id,
                request_id: envelope.request_id,
                status: "unknown".to_string(),
                reason: Some("transport_claim_not_owned".to_string()),
            });
        }
    }
}

fn pending_remote_event_sources(state: &AppState) -> Result<Vec<(EventOrigin, Vec<EventSource>)>> {
    let mut groups: Vec<(EventOrigin, Vec<EventSource>)> = Vec::new();
    for source in state.event_store.pending_internal_sources()? {
        let Some(origin) = state.event_store.remote_origin(&source)? else {
            continue;
        };
        if let Some((_, sources)) = groups.iter_mut().find(|(existing, _)| existing == &origin) {
            sources.push(source);
        } else {
            groups.push((origin, vec![source]));
        }
    }
    Ok(groups)
}

async fn reconcile_transport_runs(state: &AppState, tx: &mpsc::UnboundedSender<AgentMessage>) {
    match pending_remote_event_sources(state) {
        Ok(groups) => {
            for (origin, sources) in groups {
                if let Err(error) = tx.send(AgentMessage::EventSources { origin, sources }) {
                    log_warn(format!(
                        "pending event source recovery send failed: {error}"
                    ));
                    break;
                }
            }
        }
        Err(error) => log_warn(format!(
            "pending event source recovery scan failed: {error}"
        )),
    }
    let agent_id = state.config.read().await.agent_id.clone();
    let records = match transport_ledger::latest_records() {
        Ok(records) => records,
        Err(error) => {
            log_warn(format!("transport ledger scan failed: {error}"));
            return;
        }
    };
    for record in records.into_values() {
        if record.agent_id.as_deref() != Some(agent_id.as_str()) {
            // Legacy records and records owned by another Agent remain durable
            // evidence, but cannot be replayed or disclosed from this process.
            continue;
        }
        match record.status.as_str() {
            "completed" => {
                match transport_ledger::completed_response(&record, &state.event_store) {
                    Ok(Some(message)) => {
                        let _ = tx.send(message);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        log_warn(format!("transport replay event metadata failed: {error}"));
                    }
                }
            }
            "accepted" => {
                let command = match record.command.clone() {
                    Some(StoredCommand::Current(command)) => command,
                    Some(StoredCommand::RetiredProcessCommand { .. }) => {
                        let _ = tx.send(AgentMessage::TransportRunStatus {
                            run_id: record.run_id.clone(),
                            request_id: record.request_id.clone(),
                            status: "unknown".to_string(),
                            reason: Some("transport_command_retired".to_string()),
                        });
                        continue;
                    }
                    None => {
                        let _ = tx.send(AgentMessage::TransportRunStatus {
                            run_id: record.run_id.clone(),
                            request_id: record.request_id.clone(),
                            status: "unknown".to_string(),
                            reason: Some("transport_command_missing".to_string()),
                        });
                        continue;
                    }
                };
                let claim = match transport_ledger::claim_started(
                    &record.run_id,
                    &record.request_id,
                    &record.command_hash,
                    &agent_id,
                ) {
                    Ok(claim) => claim,
                    Err(error) => {
                        log_warn(format!("transport ledger claim failed: {error}"));
                        continue;
                    }
                };
                if !matches!(claim, transport_ledger::ClaimOutcome::Claimed) {
                    continue;
                }
                let identity = RunIdentity {
                    run_id: record.run_id.clone(),
                    request_id: record.request_id.clone(),
                    command_hash: record.command_hash.clone(),
                    agent_id: agent_id.clone(),
                };
                let command_state = state.clone();
                tokio::spawn(async move {
                    if let Err(error) =
                        handle_hub_command(command_state, command, Some(identity)).await
                    {
                        log_warn(format!("hub command failed during reconciliation: {error}"));
                    }
                });
            }
            "started" | "running" => {
                let reason = if matches!(
                    record.command.as_ref(),
                    Some(StoredCommand::RetiredProcessCommand { .. })
                ) {
                    "transport_command_retired"
                } else {
                    "agent_restarted_before_completion"
                };
                let _ = tx.send(AgentMessage::TransportRunStatus {
                    run_id: record.run_id,
                    request_id: record.request_id,
                    status: "unknown".to_string(),
                    reason: Some(reason.to_string()),
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

#[derive(Clone, Debug)]
pub(crate) struct RunIdentity {
    pub(crate) run_id: String,
    pub(crate) request_id: String,
    pub(crate) command_hash: String,
    pub(crate) agent_id: String,
}

fn event_response_sources(
    state: &AppState,
    operation: &str,
    data: &serde_json::Value,
) -> Result<Vec<EventResponseDisposition>> {
    crate::event_notifications::registered_response_dispositions(
        &state.event_store,
        operation,
        data,
    )
}

async fn report_unreturned_event_sources(
    state: &AppState,
    origin: Option<&EventOrigin>,
    dispositions: &[EventResponseDisposition],
) {
    let Some(origin) = origin else {
        return;
    };
    let sources = dispositions
        .iter()
        .map(|disposition| disposition.source.clone())
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return;
    }
    if let Err(error) = send_agent_message(
        state,
        AgentMessage::EventSources {
            origin: origin.clone(),
            sources,
        },
    )
    .await
    {
        log_warn(format!(
            "unreturned event source recovery send failed: {error}"
        ));
    }
}

pub(crate) async fn handle_hub_command(
    state: AppState,
    command: HubCommand,
    identity: Option<RunIdentity>,
) -> Result<()> {
    let suppress_event_panel = matches!(
        &command,
        HubCommand::McpListServers {
            suppress_event_panel: true,
            ..
        }
    );
    let request_id = command.request_id().to_string();
    let operation = hub_command_name(&command);
    let event_origin = identity.as_ref().map(|identity| EventOrigin {
        run_id: identity.run_id.clone(),
        request_id: identity.request_id.clone(),
        command_hash: identity.command_hash.clone(),
    });
    let context =
        RequestContext::with_event_origin(RequestIngress::Hub, operation, event_origin.as_ref());
    if operation != "event.settle" {
        if let Err(error) = crate::event_notifications::drain_completion_notifications(&state).await
        {
            log_warn(format!("event completion drain failed: {error}"));
        }
    }
    let mut snapshots = Vec::new();
    let mut data =
        match crate::local_service::dispatch(state.clone(), command, context, Some(&mut snapshots))
            .await
        {
            Ok(data) => data,
            Err(error) => {
                snapshots.clear();
                serde_json::json!({
                    "error": {
                        "code": "local_dispatch_failed",
                        "message": error.to_string()
                    }
                })
            }
        };
    let event_sources = event_response_sources(&state, operation, &data)?;
    if !suppress_event_panel && !matches!(operation, "event.panel" | "event.settle") {
        let panel_result = state
            .event_store
            .panel()
            .and_then(|panel| crate::operation_result::attach_event_panel(&mut data, &panel));
        if let Err(error) = panel_result {
            report_unreturned_event_sources(&state, event_origin.as_ref(), &event_sources).await;
            return Err(error);
        }
    }
    if let Some(identity) = identity.as_ref() {
        if identity.request_id != request_id {
            return Err(anyhow!("transport_request_id_mismatch"));
        }
        if let Err(error) = transport_ledger::mark_completed(
            &identity.run_id,
            &identity.request_id,
            &identity.command_hash,
            &identity.agent_id,
            &data,
        ) {
            report_unreturned_event_sources(&state, event_origin.as_ref(), &event_sources).await;
            return Err(error);
        }
    }
    let response = AgentMessage::Response {
        run_id: identity.as_ref().map(|value| value.run_id.clone()),
        request_id,
        event_sources,
        data: data.clone(),
    };
    let mut delivery_error = None;
    for process in snapshots {
        let process = process_for_reporting(&state, process);
        if let Err(error) =
            send_agent_message(&state, AgentMessage::ProcessUpdate { process }).await
        {
            if delivery_error.is_none() {
                delivery_error = Some(error);
            }
        }
    }
    if let Err(error) = send_agent_message(&state, response).await {
        if delivery_error.is_none() {
            delivery_error = Some(error);
        }
    }
    delivery_error.map_or(Ok(()), Err)
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

#[cfg(test)]
mod reporting_tests {
    use super::*;

    use std::fmt::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    struct RecoveryTestHome {
        root: PathBuf,
        ledger: PathBuf,
    }

    impl Drop for RecoveryTestHome {
        fn drop(&mut self) {
            transport_ledger::set_test_ledger_path(None);
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn recovery_test_home() -> RecoveryTestHome {
        let root = std::env::temp_dir().join(format!("agentic-hub-recovery-{}", Uuid::new_v4()));
        let ledger_dir = root.join(".agentic_gpt");
        std::fs::create_dir_all(&ledger_dir).unwrap();
        let ledger = ledger_dir.join("transport-runs.jsonl");
        transport_ledger::set_test_ledger_path(Some(ledger.clone()));
        RecoveryTestHome { root, ledger }
    }

    fn recovery_test_state(root: &Path) -> AppState {
        let mut config = Config::default_config().expect("test config");
        config.agent_id = "recovery-agent".to_string();
        config.workspace_root = root.join("workspace");
        config.ensure_workspace().expect("test workspace");
        let private_state = crate::private_state::PrivateStatePaths::for_test_agent(
            root.join("private-state"),
            config.agent_id.clone(),
        );
        let process_history = crate::process_history::ProcessHistoryStore::open(&private_state);
        let event_store =
            crate::event_store::EventStore::open(&private_state).expect("event store");
        AppState {
            config_path: root.join("agent.json"),
            config: Arc::new(tokio::sync::RwLock::new(config)),
            private_state,
            event_store,
            process_history,
            browser_runtime: None,
            runtime: crate::state::RuntimeModel::hub(crate::state::CapabilityProfile::Normal),
            started_at: Utc::now(),
            boot_generation: "recovery-test".to_string(),
            supervised: true,
            file_locks: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            processes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            hub_sender: Arc::new(tokio::sync::Mutex::new(None)),
            reporting_sender: Arc::new(tokio::sync::Mutex::new(None)),
            pending_confirmations: Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            temporary_mcp_allows: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::process::McpConcurrency::new()),
            room_repository_writes: Arc::new(tokio::sync::Mutex::new(())),
            skills_writes: Arc::new(tokio::sync::Mutex::new(())),
            skill_leases: Arc::new(crate::skills::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        }
    }

    #[tokio::test]
    async fn recovery_replays_completed_results_but_never_executes_retired_argv_commands() {
        let home = recovery_test_home();
        let state = recovery_test_state(&home.root);
        let marker = home.root.join("retired-command-ran");
        let old_exec = serde_json::json!({
            "type": "process.exec",
            "requestId": "request-old-exec",
            "payload": {
                "agentId": "recovery-agent",
                "program": "touch",
                "args": [marker.to_string_lossy()],
                "needConfirm": false
            }
        });
        let old_batch = serde_json::json!({
            "type": "process.batch",
            "requestId": "request-old-batch",
            "payload": {
                "agentId": "recovery-agent",
                "elements": [{
                    "program": "touch",
                    "args": [marker.to_string_lossy()]
                }],
                "needConfirm": false
            }
        });
        let old_completed = serde_json::json!({
            "type": "process.exec",
            "requestId": "request-old-completed",
            "payload": {
                "agentId": "recovery-agent",
                "program": "touch",
                "args": [marker.to_string_lossy()],
                "needConfirm": false
            }
        });
        let old_result = serde_json::json!({"historical": "already completed"});
        let current = serde_json::to_value(HubCommand::EventPanel {
            request_id: "request-current".to_string(),
        })
        .unwrap();
        let records = [
            serde_json::json!({
                "runId": "run-old-exec",
                "requestId": "request-old-exec",
                "commandHash": "hash-old-exec",
                "status": "accepted",
                "agentId": "recovery-agent",
                "command": old_exec,
            }),
            serde_json::json!({
                "runId": "run-old-batch",
                "requestId": "request-old-batch",
                "commandHash": "hash-old-batch",
                "status": "started",
                "agentId": "recovery-agent",
                "command": old_batch,
            }),
            serde_json::json!({
                "runId": "run-old-completed",
                "requestId": "request-old-completed",
                "commandHash": "hash-old-completed",
                "status": "completed",
                "agentId": "recovery-agent",
                "command": old_completed,
                "result": old_result,
            }),
            serde_json::json!({
                "runId": "run-current",
                "requestId": "request-current",
                "commandHash": "hash-current",
                "status": "accepted",
                "agentId": "recovery-agent",
                "command": current,
            }),
        ];
        let mut ledger = String::new();
        for record in &records {
            writeln!(&mut ledger, "{record}").unwrap();
        }
        std::fs::write(&home.ledger, ledger).unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = state;
        state.hub_sender = Arc::new(tokio::sync::Mutex::new(Some(tx.clone())));
        reconcile_transport_runs(&state, &tx).await;

        let mut saw_completed = false;
        let mut saw_accepted_retired = false;
        let mut saw_started_retired = false;
        let mut saw_current = false;
        for _ in 0..4 {
            match timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("recovery response timed out")
                .expect("recovery channel closed")
            {
                AgentMessage::TransportRunStatus {
                    run_id,
                    request_id,
                    status,
                    reason,
                } => {
                    assert_eq!(status, "unknown");
                    assert_eq!(reason.as_deref(), Some("transport_command_retired"));
                    if run_id == "run-old-exec" && request_id == "request-old-exec" {
                        saw_accepted_retired = true;
                    } else {
                        assert_eq!(run_id, "run-old-batch");
                        assert_eq!(request_id, "request-old-batch");
                        saw_started_retired = true;
                    }
                }
                AgentMessage::Response {
                    run_id,
                    request_id,
                    data,
                    ..
                } if request_id == "request-old-completed" => {
                    assert_eq!(run_id.as_deref(), Some("run-old-completed"));
                    assert_eq!(data, old_result);
                    saw_completed = true;
                }
                AgentMessage::Response { request_id, .. } => {
                    assert_eq!(request_id, "request-current");
                    saw_current = true;
                }
                message => panic!("unexpected recovery message: {message:?}"),
            }
        }
        assert!(saw_completed);
        assert!(saw_accepted_retired);
        assert!(saw_started_retired);
        assert!(saw_current);
        assert!(!marker.exists());

        let recovered = transport_ledger::latest_records().unwrap();
        assert_eq!(recovered["run-old-exec"].status, "accepted");
        assert_eq!(recovered["run-old-exec"].command_hash, "hash-old-exec");
        assert_eq!(recovered["run-old-batch"].status, "started");
        assert_eq!(recovered["run-old-batch"].command_hash, "hash-old-batch");
        assert_eq!(recovered["run-old-completed"].result, Some(old_result));
        assert_eq!(recovered["run-current"].status, "completed");
        assert_eq!(recovered["run-current"].command_hash, "hash-current");
    }
    #[test]
    fn oversized_report_json_becomes_a_hash_record() {
        let bounded = bounded_json_value(serde_json::json!({
            "payload": "x".repeat(REPORT_MAX_JSON_BYTES)
        }));
        assert!(bounded.truncated);
        assert!(bounded.byte_count > REPORT_MAX_JSON_BYTES);
        assert_eq!(bounded.value["truncated"], true);
        assert_eq!(bounded.value["byteCount"], bounded.byte_count);
        assert_eq!(bounded.value["sha256"], bounded.sha256);
    }
}
