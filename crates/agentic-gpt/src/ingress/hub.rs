use std::collections::HashMap;
use std::sync::{atomic::Ordering, Arc, Mutex as StdMutex};

use crate::process_model::ProcessInfo;
use agentic_gpt_protocol::{
    AgentConnectionMode, AgentMessage, AgentRunReport, BoundedJsonValue, ConfigApplyStatus,
    EventOrigin, EventResponseDisposition, EventSource, HubCommand, HubCommandEnvelope, HubMessage,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio::time::{sleep, timeout, Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Response as WsResponse;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    client_async_tls_with_config, connect_async, MaybeTlsStream, WebSocketStream,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    config::Config,
    confirmation, notify,
    operation::{hub_command_name, RequestContext, RequestIngress},
    process,
    state::AppState,
    transport_ledger::{self, StoredCommand},
    utils::{
        log_warn, CONNECT_TIMEOUT_SECS, HEARTBEAT_ACK_TIMEOUT_SECS, HEARTBEAT_INTERVAL_SECS,
        RECONNECT_DELAY_SECS,
    },
};

/// Current connection attempt, independent of the durable desired configuration.
#[derive(Clone, Debug)]
pub(crate) struct HubConnectionState {
    pub(crate) target: Option<String>,
    pub(crate) status: ConfigApplyStatus,
    pub(crate) error: Option<String>,
}

impl Default for HubConnectionState {
    fn default() -> Self {
        Self {
            target: None,
            status: ConfigApplyStatus::Applied,
            error: None,
        }
    }
}

#[derive(Clone)]
struct ConnectionRoute {
    target: String,
    sender: mpsc::UnboundedSender<AgentMessage>,
    config_requests: Arc<StdMutex<HashMap<String, String>>>,
    progress: Arc<Notify>,
}

impl ConnectionRoute {
    fn new(target: String, sender: mpsc::UnboundedSender<AgentMessage>) -> Self {
        Self {
            target,
            sender,
            config_requests: Arc::new(StdMutex::new(HashMap::new())),
            progress: Arc::new(Notify::new()),
        }
    }

    fn track_config(&self, envelope: &HubCommandEnvelope) {
        self.track_config_command(&envelope.run_id, &envelope.request_id, &envelope.command);
    }

    fn track_config_command(&self, run_id: &str, request_id: &str, command: &HubCommand) {
        if matches!(command, HubCommand::ConfigUpdate { .. }) {
            self.config_requests
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(run_id.to_string(), request_id.to_string());
        }
    }

    fn acknowledge(&self, run_id: &str, request_id: &str) {
        let mut pending = self
            .config_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if pending
            .get(run_id)
            .is_some_and(|expected| expected == request_id)
        {
            pending.remove(run_id);
            self.progress.notify_one();
        }
    }

    fn has_pending_config(&self) -> bool {
        !self
            .config_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

struct ConnectionTask(tokio::task::JoinHandle<()>);

impl ConnectionTask {
    async fn stop(&mut self) {
        self.0.abort();
        let _ = (&mut self.0).await;
    }
}

impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn hub_target(config: &Config) -> String {
    if let Ok(mut url) = reqwest::Url::parse(&config.hub.url) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_fragment(None);
        return url.as_str().trim_end_matches('/').to_owned();
    }
    // Invalid endpoints cannot connect; avoid retaining credential-like input in diagnostics.
    "invalid_hub_target".to_owned()
}

async fn connection_status(
    state: &AppState,
    target: Option<String>,
    status: ConfigApplyStatus,
    error: Option<&str>,
) {
    *state.config_runtime.hub_connection.write().await = HubConnectionState {
        target,
        status,
        error: error.map(str::to_owned),
    };
}

pub(crate) async fn connect_loop(state: AppState) -> Result<()> {
    connect_loop_with_shutdown(state, CancellationToken::new()).await
}

pub(crate) async fn connect_loop_with_shutdown(
    state: AppState,
    shutdown: CancellationToken,
) -> Result<()> {
    // Non-Hub runtimes retain this owner even when the auxiliary connection starts disabled.
    let reporting = state.runtime.transport != crate::state::Transport::Hub;
    loop {
        let config = state.config.read().await.clone();
        let enabled = !reporting || config.auxiliary_hub_enabled();
        if !enabled {
            // No connection is the successfully applied disabled configuration.
            *state.config_runtime.hub_applied.write().await = Some(config);
            connection_status(&state, None, ConfigApplyStatus::Applied, None).await;
            tokio::select! {
                _ = state.config_runtime.hub_changed.notified() => {},
                _ = shutdown.cancelled() => return Ok(()),
            }
            continue;
        }
        *state.config_runtime.hub_applied.write().await = None;
        let target = hub_target(&config);
        connection_status(
            &state,
            Some(target.clone()),
            ConfigApplyStatus::Applying,
            None,
        )
        .await;
        let result = if config.hub.transport == "sse" {
            connect_sse_session(state.clone(), config, reporting, shutdown.clone()).await
        } else {
            connect_websocket_session(state.clone(), config, reporting, shutdown.clone()).await
        };
        // Also cover failures/cancellation before a transport reached its ready point.
        *state.config_runtime.hub_applied.write().await = None;
        if shutdown.is_cancelled() {
            return result;
        }
        let reconfigured = result.is_ok()
            && state.config_runtime.hub_connection.read().await.status
                == ConfigApplyStatus::Applying;
        confirmation::fail_pending_confirmations(&state, "provider_unavailable").await;
        if let Err(error) = result {
            log_warn(format!("hub connection ended: {error}"));
            connection_status(
                &state,
                Some(target),
                ConfigApplyStatus::Failed,
                Some("hub_connection_failed"),
            )
            .await;
        } else if !reconfigured {
            // A normal peer close still leaves no applied connection during backoff.
            connection_status(&state, Some(target), ConfigApplyStatus::Applying, None).await;
        }
        if reconfigured {
            continue;
        }
        let desired = state.config.read().await.clone();
        if hub_target(&desired) != hub_target_from_state_config(&state).await
            || (reporting && !desired.auxiliary_hub_enabled())
        {
            continue;
        }
        // Notifications interrupt backoff. No spawned transport tasks remain at this point.
        tokio::select! {
            _ = state.config_runtime.hub_changed.notified() => {},
            _ = sleep(Duration::from_secs(RECONNECT_DELAY_SECS)) => {},
            _ = shutdown.cancelled() => return Ok(()),
        }
    }
}

async fn hub_target_from_state_config(state: &AppState) -> String {
    state
        .config_runtime
        .hub_connection
        .read()
        .await
        .target
        .clone()
        .unwrap_or_default()
}

async fn connect_websocket_session(
    state: AppState,
    config: Config,
    reporting: bool,
    shutdown: CancellationToken,
) -> Result<()> {
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
    let connecting = timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        connect_hub(request, proxy_url(&config.hub.url)),
    );
    let (stream, _) = tokio::select! {
        result = connecting => result.map_err(|_| anyhow!("hub_connect_timeout"))??,
        _ = state.config_runtime.hub_changed.notified() => return Ok(()),
        _ = shutdown.cancelled() => return Ok(()),
    };
    let (mut write, mut read) = stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentMessage>();
    let (event_tx, mut event_rx) = mpsc::channel::<AgentMessage>(64);
    let route = ConnectionRoute::new(hub_target(&config), tx.clone());
    *state.hub_sender.lock().await = Some(tx.clone());
    let reporting_enabled = state.config_runtime.hub_reporting_enabled.clone();
    let mut writer = ConnectionTask(tokio::spawn(async move {
        loop {
            let (message, is_report) = tokio::select! {
                biased;
                Some(message) = rx.recv() => (message, false),
                Some(message) = event_rx.recv() => (message, true),
                else => break,
            };
            if is_report && !reporting_enabled.load(Ordering::Acquire) {
                continue;
            }
            let text = match serde_json::to_string(&message) {
                Ok(text) => text,
                Err(_) => continue,
            };
            if write.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    }));
    let result = async {
        tx.send(AgentMessage::Hello {
            boot_generation: state.boot_generation.clone(), role: state.runtime.profile.role(),
            connection_mode: if reporting { AgentConnectionMode::ReportingOnly } else { AgentConnectionMode::CommandCapable },
            config_summary: config.safe_summary(),
            notification_channels: notify::freedesktop_notification_channel(&config).into_iter().collect(),
        })?;
        *state.config_runtime.hub_applied.write().await = Some(config.clone());
        let reporting_enabled = if reporting {
            apply_reporting(&state, &config, &event_tx).await
        } else {
            connection_status(&state, Some(route.target.clone()), ConfigApplyStatus::Applied, None).await;
            Some(false)
        };
        if reporting_enabled == Some(true) {
            send_current_process_snapshots(&state, &event_tx).await;
        } else if !reporting {
            reconcile_transport_runs(&state, &route).await;
        }
        let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_heartbeat_ack = Instant::now();
        let mut reconfigure = false;
        let mut reconfigure_started = None;
        loop {
            if reconfigure && !route.has_pending_config() { break; }
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = state.config_runtime.hub_changed.notified() => {
                    reconfigure = true;
                    reconfigure_started.get_or_insert_with(Instant::now);
                    connection_status(&state, Some(route.target.clone()), ConfigApplyStatus::Applying, None).await;
                }
                _ = state.config_runtime.hub_reporting_changed.notified(), if reporting => {
                    if apply_reporting(&state, &config, &event_tx).await == Some(true) {
                        send_current_process_snapshots(&state, &event_tx).await;
                    }
                }
                _ = route.progress.notified(), if reconfigure => {},
                message = read.next() => {
                    let Some(message) = message else { break; };
                    match message? {
                        Message::Text(text) => {
                            if handle_connection_inbound(&state, &route, &text, reporting, !reconfigure).await {
                                last_heartbeat_ack = Instant::now();
                            }
                        }
                        Message::Pong(_) => last_heartbeat_ack = Instant::now(),
                        Message::Close(_) => break,
                        _ => {},
                    }
                }
                _ = heartbeat.tick() => {
                    if reconfigure_started.is_some_and(|started: Instant| started.elapsed() > Duration::from_secs(180)) {
                        return Err(anyhow!("hub_reconfigure_reply_timeout"));
                    }
                    if last_heartbeat_ack.elapsed() > Duration::from_secs(HEARTBEAT_ACK_TIMEOUT_SECS) {
                        return Err(anyhow!("hub_heartbeat_timeout"));
                    }
                    tx.send(AgentMessage::Heartbeat { sent_at: Utc::now() })?;
                }
            }
        }
        Ok(())
    }.await;
    writer.stop().await;
    clear_reporting_senders(&state, &tx, &event_tx).await;
    *state.config_runtime.hub_applied.write().await = None;
    result
}

async fn handle_connection_inbound(
    state: &AppState,
    route: &ConnectionRoute,
    text: &str,
    reporting: bool,
    admit_commands: bool,
) -> bool {
    if let Ok(message) = serde_json::from_str::<HubMessage>(text) {
        match message {
            HubMessage::ResponseAck { run_id, request_id } => {
                route.acknowledge(&run_id, &request_id)
            }
            HubMessage::HeartbeatAck { .. } => return true,
            HubMessage::SecretCommand {
                dispatch_id,
                command,
            } => {
                if !reporting && admit_commands {
                    handle_secret_command(state, &route.sender, dispatch_id, *command);
                }
            }
            HubMessage::ConfirmationResponse {
                request_id,
                decision,
                ..
            } => {
                if let Some(sender) = state.pending_confirmations.lock().await.remove(&request_id) {
                    let _ = sender.send(confirmation::confirmation_decision_value(decision));
                }
            }
        }
        return false;
    }
    if reporting {
        log_warn("ignored execution command on reporting-only connection".to_owned());
    } else if admit_commands {
        if let Ok(envelope) = serde_json::from_str::<HubCommandEnvelope>(text) {
            handle_reliable_envelope(state, route, envelope).await;
        }
    }
    false
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

async fn apply_reporting(
    state: &AppState,
    connection: &Config,
    sender: &mpsc::Sender<AgentMessage>,
) -> Option<bool> {
    let _serial = state.config_runtime.apply_lock.lock().await;
    let config = state.config.read().await;
    if config.hub.enabled != connection.hub.enabled
        || config.hub.url != connection.hub.url
        || config.hub.transport != connection.hub.transport
        || config.hub.agent_secret != connection.hub.agent_secret
        || config.hub.extra != connection.hub.extra
    {
        return None;
    }
    let enabled = config.auxiliary_hub_enabled() && config.hub_reporting_enabled();
    let reporting = config
        .tunnel
        .as_ref()
        .map(|tunnel| tunnel.hub_reporting.clone());
    drop(config);
    state
        .config_runtime
        .hub_reporting_enabled
        .store(enabled, Ordering::Release);
    *state.reporting_sender.lock().await = enabled.then(|| sender.clone());
    if let Some(applied) = state.config_runtime.hub_applied.write().await.as_mut() {
        if let (Some(tunnel), Some(reporting)) = (&mut applied.tunnel, reporting) {
            tunnel.hub_reporting = reporting;
        }
    }
    connection_status(
        state,
        Some(hub_target(connection)),
        ConfigApplyStatus::Applied,
        None,
    )
    .await;
    Some(enabled)
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
                process: if full {
                    process.map(|info| info.into_wire(&state.transport_agent_id))
                } else {
                    None
                },
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

fn process_for_reporting(
    state: &AppState,
    mut process: ProcessInfo,
) -> agentic_gpt_protocol::ProcessInfo {
    if reporting_detail(state) == "metadata" {
        process.program = Some("<redacted>".to_string());
        process.args.clear();
        process.working_directory = None;
        process.command_preview = Some("<redacted>".to_string());
    }
    process.into_wire(&state.transport_agent_id)
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

async fn connect_sse_session(
    state: AppState,
    config: Config,
    reporting: bool,
    shutdown: CancellationToken,
) -> Result<()> {
    let connection_id = format!("conn_{}", Uuid::new_v4().simple());
    let base = config.hub.url.trim_end_matches('/');
    let events_url = format!(
        "{base}/v1/agents/{}/events?connectionId={connection_id}",
        config.agent_id
    );
    let messages_url = format!(
        "{base}/v1/agents/{}/messages?connectionId={connection_id}",
        config.agent_id
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .build()?;
    let connecting = client
        .get(events_url)
        .header("x-agent-secret", &config.hub.agent_secret)
        .send();
    let response = tokio::select! {
        response = timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS), connecting) => response.map_err(|_| anyhow!("hub_connect_timeout"))??,
        _ = state.config_runtime.hub_changed.notified() => return Ok(()),
        _ = shutdown.cancelled() => return Ok(()),
    };
    if !response.status().is_success() {
        return Err(anyhow!("sse_connect_failed"));
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentMessage>();
    let (event_tx, mut event_rx) = mpsc::channel::<AgentMessage>(64);
    let route = ConnectionRoute::new(hub_target(&config), tx.clone());
    *state.hub_sender.lock().await = Some(tx.clone());
    let reporting_enabled = state.config_runtime.hub_reporting_enabled.clone();
    let post_client = client.clone();
    let secret = config.hub.agent_secret.clone();
    let writer_route = route.clone();
    let mut writer = ConnectionTask(tokio::spawn(async move {
        loop {
            let (message, is_report) = tokio::select! {
                biased;
                Some(message) = rx.recv() => (message, false),
                Some(message) = event_rx.recv() => (message, true),
                else => break,
            };
            if is_report && !reporting_enabled.load(Ordering::Acquire) {
                continue;
            }
            let secret_response = matches!(&message, AgentMessage::SecretResponse { .. });
            let mut delay = Duration::from_millis(250);
            loop {
                if is_report && !reporting_enabled.load(Ordering::Acquire) {
                    break;
                }
                let response = post_client
                    .post(&messages_url)
                    .header("x-agent-secret", &secret)
                    .json(&message)
                    .timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                    .send()
                    .await;
                match response {
                    Ok(response) if response.status().is_success() => {
                        if let AgentMessage::Response {
                            run_id: Some(run_id),
                            request_id,
                            ..
                        } = &message
                        {
                            // POST success means the Hub has processed/store_result'd the reply.
                            writer_route.acknowledge(run_id, request_id);
                        }
                        break;
                    }
                    Ok(response)
                        if classify_sse_post_status(response.status()) == SsePostStatus::Stale =>
                    {
                        return
                    }
                    _ if secret_response || reporting => break,
                    _ => {}
                }
                sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
            }
        }
    }));
    let result = async {
        tx.send(AgentMessage::Hello {
            boot_generation: state.boot_generation.clone(), role: state.runtime.profile.role(),
            connection_mode: if reporting { AgentConnectionMode::ReportingOnly } else { AgentConnectionMode::CommandCapable },
            config_summary: config.safe_summary(),
            notification_channels: notify::freedesktop_notification_channel(&config).into_iter().collect(),
        })?;
        *state.config_runtime.hub_applied.write().await = Some(config.clone());
        let reporting_enabled = if reporting {
            apply_reporting(&state, &config, &event_tx).await
        } else {
            connection_status(&state, Some(route.target.clone()), ConfigApplyStatus::Applied, None).await;
            Some(false)
        };
        if reporting_enabled == Some(true) {
            send_current_process_snapshots(&state, &event_tx).await;
        } else if !reporting {
            reconcile_transport_runs(&state, &route).await;
        }
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut data = String::new();
        let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_heartbeat_ack = Instant::now();
        let mut reconfigure = false;
        let mut reconfigure_started = None;
        loop {
            if reconfigure && !route.has_pending_config() { break; }
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = state.config_runtime.hub_changed.notified() => {
                    reconfigure = true;
                    reconfigure_started.get_or_insert_with(Instant::now);
                    connection_status(&state, Some(route.target.clone()), ConfigApplyStatus::Applying, None).await;
                }
                _ = state.config_runtime.hub_reporting_changed.notified(), if reporting => {
                    if apply_reporting(&state, &config, &event_tx).await == Some(true) {
                        send_current_process_snapshots(&state, &event_tx).await;
                    }
                }
                _ = route.progress.notified(), if reconfigure => {},
                chunk = stream.next() => {
                    let Some(chunk) = chunk else { break; };
                    buffer.push_str(&String::from_utf8_lossy(&chunk?));
                    while let Some(index) = buffer.find('\n') {
                        let line = buffer[..index].trim_end_matches('\r').to_owned();
                        buffer = buffer[index + 1..].to_owned();
                        if line.is_empty() {
                            if !data.is_empty() {
                                if handle_connection_inbound(&state, &route, &data, reporting, !reconfigure).await {
                                    last_heartbeat_ack = Instant::now();
                                }
                                data.clear();
                            }
                        } else if let Some(value) = line.strip_prefix("data:") {
                            if !data.is_empty() { data.push('\n'); }
                            data.push_str(value.trim_start());
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    if reconfigure_started.is_some_and(|started: Instant| started.elapsed() > Duration::from_secs(180)) {
                        return Err(anyhow!("hub_reconfigure_reply_timeout"));
                    }
                    if last_heartbeat_ack.elapsed() > Duration::from_secs(HEARTBEAT_ACK_TIMEOUT_SECS) {
                        return Err(anyhow!("hub_heartbeat_timeout"));
                    }
                    tx.send(AgentMessage::Heartbeat { sent_at: Utc::now() })?;
                }
            }
        }
        Ok(())
    }.await;
    writer.stop().await;
    clear_reporting_senders(&state, &tx, &event_tx).await;
    *state.config_runtime.hub_applied.write().await = None;
    result
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

fn handle_secret_command(
    state: &AppState,
    tx: &mpsc::UnboundedSender<AgentMessage>,
    dispatch_id: String,
    command: HubCommand,
) {
    let state = state.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let request_id = command.request_id().to_owned();
        let mut data = if command.is_room_secret() {
            let operation = hub_command_name(&command);
            let context = crate::operation::RequestContext::new(
                crate::operation::RequestIngress::Hub,
                operation,
            );
            crate::local_service::dispatch(state.clone(), command, context, None).await
                .unwrap_or_else(|_| serde_json::json!({"error":{"code":"secret_operation_failed","message":"Secret operation failed"}}))
        } else {
            serde_json::json!({"error":{"code":"secret_command_required","message":"Secret command required"}})
        };
        if let Ok(panel) = state.event_store.panel() {
            let _ = crate::operation_result::attach_event_panel(&mut data, &panel);
        }
        // Use the originating connection. A reconnect must not deliver an old secret response.
        let _ = tx.send(AgentMessage::SecretResponse {
            dispatch_id,
            request_id,
            data,
        });
    });
}

async fn handle_reliable_envelope(
    state: &AppState,
    route: &ConnectionRoute,
    envelope: HubCommandEnvelope,
) {
    let tx = &route.sender;
    if envelope.command.is_room_secret() {
        log_warn("rejected persistent secret command".to_string());
        return;
    }
    let agent_id = state.config.read().await.agent_id.clone();
    let outcome = match transport_ledger::accept_for_target(&envelope, &agent_id, &route.target) {
        Ok(outcome) => outcome,
        Err(error) => {
            log_warn(format!("transport ledger accept failed: {error}"));
            return;
        }
    };
    if matches!(
        &outcome,
        transport_ledger::AcceptOutcome::FirstAccepted
            | transport_ledger::AcceptOutcome::DuplicateAccepted
            | transport_ledger::AcceptOutcome::DuplicateStarted
            | transport_ledger::AcceptOutcome::Completed(_)
    ) {
        route.track_config(&envelope);
    }
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
            let command_route = route.clone();
            tokio::spawn(async move {
                if let Err(error) = handle_hub_command_on_connection(
                    command_state,
                    envelope.command,
                    Some(identity),
                    Some(command_route),
                )
                .await
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

async fn reconcile_transport_runs(state: &AppState, route: &ConnectionRoute) {
    let tx = &route.sender;
    let records = match transport_ledger::latest_records() {
        Ok(records) => records,
        Err(error) => {
            log_warn(format!("transport ledger scan failed: {error}"));
            return;
        }
    };
    match pending_remote_event_sources(state) {
        Ok(groups) => {
            for (origin, sources) in groups {
                if !records.get(&origin.run_id).is_some_and(|record| {
                    record.hub_target.as_deref() == Some(route.target.as_str())
                        && record.agent_id.as_deref() == Some(state.transport_agent_id.as_str())
                }) {
                    continue;
                }
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
    for record in records.into_values() {
        if record.agent_id.as_deref() != Some(agent_id.as_str())
            || record.hub_target.as_deref() != Some(route.target.as_str())
        {
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
                route.track_config_command(&record.run_id, &record.request_id, &command);
                let identity = RunIdentity {
                    run_id: record.run_id.clone(),
                    request_id: record.request_id.clone(),
                    command_hash: record.command_hash.clone(),
                    agent_id: agent_id.clone(),
                };
                let command_state = state.clone();
                let command_route = route.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_hub_command_on_connection(
                        command_state,
                        command,
                        Some(identity),
                        Some(command_route),
                    )
                    .await
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
    route: Option<&ConnectionRoute>,
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
    if let Err(error) = send_bound_message(
        state,
        route,
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

#[cfg(test)]
pub(crate) async fn handle_hub_command(
    state: AppState,
    command: HubCommand,
    identity: Option<RunIdentity>,
) -> Result<()> {
    handle_hub_command_on_connection(state, command, identity, None).await
}

async fn handle_hub_command_on_connection(
    state: AppState,
    command: HubCommand,
    identity: Option<RunIdentity>,
    route: Option<ConnectionRoute>,
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
            report_unreturned_event_sources(
                &state,
                route.as_ref(),
                event_origin.as_ref(),
                &event_sources,
            )
            .await;
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
            report_unreturned_event_sources(
                &state,
                route.as_ref(),
                event_origin.as_ref(),
                &event_sources,
            )
            .await;
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
        if let Err(error) = send_bound_message(
            &state,
            route.as_ref(),
            AgentMessage::ProcessUpdate { process },
        )
        .await
        {
            if delivery_error.is_none() {
                delivery_error = Some(error);
            }
        }
    }
    if let Err(error) = send_bound_message(&state, route.as_ref(), response).await {
        if delivery_error.is_none() {
            delivery_error = Some(error);
        }
    }
    delivery_error.map_or(Ok(()), Err)
}

async fn send_bound_message(
    state: &AppState,
    route: Option<&ConnectionRoute>,
    message: AgentMessage,
) -> Result<()> {
    let Some(route) = route else {
        return send_agent_message(state, message).await;
    };
    let message = match route.sender.send(message) {
        Ok(()) => return Ok(()),
        Err(error) => error.0,
    };
    // Same-authority reconnect may recover a running request. Holding the target read lock
    // through sender selection prevents an endpoint replacement from racing this fallback.
    let current = state.config_runtime.hub_connection.read().await;
    if current.target.as_deref() != Some(route.target.as_str()) {
        return Err(anyhow!("hub_response_target_changed"));
    }
    let sender = state
        .hub_sender
        .lock()
        .await
        .clone()
        .ok_or_else(|| anyhow!("hub_sender_unavailable"))?;
    sender.send(message).map_err(|_| anyhow!("hub_send_failed"))
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

    use agentic_gpt_protocol::{ConfigChange, ConfigGetRequest, ConfigUpdateRequest};
    use std::fmt::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    async fn set_config(state: &AppState, path: &[&str], value: serde_json::Value) -> Result<()> {
        let response = crate::config_api::update(
            state,
            &crate::operation::RequestContext::new(
                crate::operation::RequestIngress::LocalUnix,
                "config.update",
            ),
            ConfigUpdateRequest {
                changes: vec![ConfigChange::Set {
                    path: path.iter().map(|part| (*part).to_owned()).collect(),
                    value,
                }],
                dry_run: false,
            },
        )
        .await?;
        anyhow::ensure!(
            response["saved"] == true,
            "config update rejected: {response}"
        );
        Ok(())
    }

    async fn get_reporting(state: &AppState) -> Result<serde_json::Value> {
        crate::config_api::get(
            state,
            ConfigGetRequest {
                paths: vec![vec![
                    "tunnel".into(),
                    "hubReporting".into(),
                    "enabled".into(),
                ]],
            },
        )
        .await
    }

    async fn wait_reporting_applied(state: &AppState, expected: bool) -> Result<serde_json::Value> {
        timeout(Duration::from_secs(3), async {
            loop {
                let response = get_reporting(state).await?;
                if response["items"][0]["value"] == expected
                    && response["items"][0]["applyStatus"] == "applied"
                    && response["items"][0].get("appliedValue").is_none()
                {
                    return Ok::<_, anyhow::Error>(response);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?
    }

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
        let private_state =
            crate::private_state::PrivateStatePaths::for_test(root.join("private-state"));
        let process_history = crate::process_history::ProcessHistoryStore::open(&private_state);
        let event_store =
            crate::event_store::EventStore::open(&private_state).expect("event store");
        AppState {
            email: crate::email::EmailService::open(&private_state).expect("email store"),
            secret: crate::secret::SecretService::open(&private_state).expect("secret service"),
            transport_agent_id: config.agent_id.clone(),
            config_path: root.join("agent.json"),
            config: Arc::new(tokio::sync::RwLock::new(config)),
            private_state,
            event_store,
            process_history,
            browser_runtime: std::sync::Arc::new(crate::state::BrowserRuntimeSlot::default()),
            config_runtime: std::sync::Arc::new(crate::config_runtime::ConfigRuntime::default()),
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
    async fn auxiliary_hub_confirms_without_reporting_or_executing_commands() -> Result<()> {
        for mode in [
            crate::config::RuntimeMode::Local,
            crate::config::RuntimeMode::Standalone,
        ] {
            let home = recovery_test_home();
            let mut state = recovery_test_state(&home.root);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let mut config = state.config.read().await.clone();
            config.mode = mode;
            config.hub.enabled = true;
            config.hub.url = format!("http://{}", listener.local_addr()?);
            config.hub.agent_secret = "auxiliary-test-secret".to_string();
            config.confirmation_provider.channels = vec![crate::config::ConfirmationChannel::Ntfy];
            config.policy.enabled = false;
            config.tunnel = None;
            config.validate_local()?;
            state.runtime = match mode {
                crate::config::RuntimeMode::Local => crate::state::RuntimeModel::local_with_hub(
                    crate::state::CapabilityProfile::Normal,
                    true,
                ),
                _ => crate::state::RuntimeModel::tunnel(
                    crate::state::CapabilityProfile::Normal,
                    true,
                ),
            };
            *state.config.write().await = config.clone();
            let marker = home.root.join("remote-command-ran");
            let peer = async {
                let (stream, _) = listener.accept().await?;
                let mut socket = tokio_tungstenite::accept_async(stream).await?;
                let hello = socket
                    .next()
                    .await
                    .ok_or_else(|| anyhow!("missing hello"))??;
                anyhow::ensure!(matches!(
                    serde_json::from_str::<AgentMessage>(hello.to_text()?)?,
                    AgentMessage::Hello {
                        connection_mode: AgentConnectionMode::ReportingOnly,
                        ..
                    }
                ));
                anyhow::ensure!(state.hub_sender.lock().await.is_some());
                anyhow::ensure!(state.reporting_sender.lock().await.is_none());
                let command = HubCommandEnvelope {
                    event_id: "aux-command-event".to_string(),
                    run_id: "aux-command-run".to_string(),
                    request_id: "aux-command-request".to_string(),
                    command_hash: "aux-command-hash".to_string(),
                    command: HubCommand::Exec {
                        request_id: "aux-command-request".to_string(),
                        payload: agentic_gpt_protocol::ProcessExecRequest {
                            agent_id: config.agent_id.clone(),
                            group: None,
                            command: format!("touch {}", marker.display()),
                            acquire_confirmation: None,
                            cwd: None,
                            wait_seconds: Some(0),
                        },
                    },
                };
                socket
                    .send(Message::Text(serde_json::to_string(&command)?.into()))
                    .await?;
                let receive_confirmation = async {
                    loop {
                        let message = socket
                            .next()
                            .await
                            .ok_or_else(|| anyhow!("confirmation connection closed"))??;
                        let message: AgentMessage = serde_json::from_str(message.to_text()?)?;
                        match message {
                            AgentMessage::Heartbeat { .. } => {}
                            AgentMessage::ConfirmationRequest { request_id, .. } => {
                                socket.send(Message::Text(serde_json::to_string(&HubMessage::ConfirmationResponse {
                                    request_id,
                                    decision: agentic_gpt_protocol::ConfirmationDecision::AllowOnce,
                                    reason: "local test approval".to_string(),
                                })?.into())).await?;
                                break;
                            }
                            _ => {
                                return Err(anyhow!(
                                    "auxiliary connection sent reporting or execution output"
                                ));
                            }
                        }
                    }
                    Ok::<_, anyhow::Error>(())
                };
                let confirmation = confirmation::authorize_operation(
                    &state,
                    &config,
                    None,
                    crate::policy::PolicyDecision::Confirm,
                    "true",
                    &[],
                    "请求运行 true",
                );
                let (approval, response) = tokio::join!(confirmation, receive_confirmation);
                response?;
                anyhow::ensure!(approval.map_err(|error| anyhow!(error))?.approved);
                socket.close(None).await?;
                Ok::<_, anyhow::Error>(())
            };
            let (transport, peer) = timeout(Duration::from_secs(5), async {
                tokio::join!(
                    connect_websocket_session(
                        state.clone(),
                        config.clone(),
                        true,
                        CancellationToken::new()
                    ),
                    peer
                )
            })
            .await
            .map_err(|_| anyhow!("auxiliary Hub test timed out"))?;
            peer?;
            transport?;
            anyhow::ensure!(!marker.exists());
            anyhow::ensure!(state.hub_sender.lock().await.is_none());
        }
        Ok(())
    }

    // ⑤ Real WebSocket ingress, durable config/receipt, acknowledgement and endpoint
    // replacement. Protects against losing a self-update reply or disclosing it to a new Hub.
    #[tokio::test]
    async fn hub_reconfiguration_waits_for_reply_ack_and_does_not_replay_to_new_target(
    ) -> Result<()> {
        async fn next_agent_message(
            socket: &mut WebSocketStream<TcpStream>,
        ) -> Result<AgentMessage> {
            loop {
                let message = socket
                    .next()
                    .await
                    .ok_or_else(|| anyhow!("socket closed"))??;
                let Message::Text(text) = message else {
                    continue;
                };
                let message: AgentMessage = serde_json::from_str(&text)?;
                if let AgentMessage::Heartbeat { sent_at } = message {
                    socket
                        .send(Message::Text(
                            serde_json::to_string(&HubMessage::HeartbeatAck {
                                sent_at,
                                received_at: Utc::now(),
                            })?
                            .into(),
                        ))
                        .await?;
                } else {
                    return Ok(message);
                }
            }
        }

        let home = recovery_test_home();
        let state = recovery_test_state(&home.root);
        let original = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let replacement = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let replacement_url = format!("http://{}", replacement.local_addr()?);
        let mut config = state.config.read().await.clone();
        config.mode = crate::config::RuntimeMode::Hub;
        config.hub.enabled = true;
        config.hub.url = format!("http://{}", original.local_addr()?);
        let original_url = config.hub.url.clone();
        config.hub.agent_secret = "test-agent-secret".to_owned();
        config.toolsets.enable(crate::config::ToolNamespace::Config);
        config.config_api.update_policy = crate::config::ConfigUpdatePolicy::Allow;
        crate::config::write_config_with_backup(&state.config_path, &config)?;
        *state.config.write().await = config;
        let shutdown = CancellationToken::new();
        let owner_state = state.clone();
        let owner_shutdown = shutdown.clone();
        let mut owner = ConnectionTask(tokio::spawn(async move {
            let _ = connect_loop_with_shutdown(owner_state, owner_shutdown).await;
        }));
        let scenario = timeout(Duration::from_secs(15), async {
            let (stream, _) = original.accept().await?;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            anyhow::ensure!(matches!(
                next_agent_message(&mut socket).await?,
                AgentMessage::Hello { .. }
            ));
            let envelope = HubCommandEnvelope {
                event_id: "config-change-event".to_owned(),
                run_id: "config-change-run".to_owned(),
                request_id: "config-change-request".to_owned(),
                command_hash: "config-change-hash".to_owned(),
                command: HubCommand::ConfigUpdate {
                    request_id: "config-change-request".to_owned(),
                    payload: agentic_gpt_protocol::ConfigUpdateRequest {
                        changes: vec![agentic_gpt_protocol::ConfigChange::Set {
                            path: vec!["hub".into(), "url".into()],
                            value: serde_json::json!(replacement_url),
                        }],
                        dry_run: false,
                    },
                },
            };
            socket
                .send(Message::Text(serde_json::to_string(&envelope)?.into()))
                .await?;
            loop {
                match next_agent_message(&mut socket).await? {
                    AgentMessage::TransportAck { .. } => {}
                    AgentMessage::Response { run_id, data, .. } => {
                        anyhow::ensure!(run_id.as_deref() == Some(envelope.run_id.as_str()));
                        anyhow::ensure!(data["saved"] == true, "update failed: {data}");
                        anyhow::ensure!(data.get("events").is_some());
                        anyhow::ensure!(data["changes"][0]["appliedValue"] == original_url);
                        anyhow::ensure!(data["changes"][0]["applyStatus"] == "applying");
                        break;
                    }
                    _ => return Err(anyhow!("unexpected message before config reply")),
                }
            }
            anyhow::ensure!(
                timeout(Duration::from_millis(250), replacement.accept())
                    .await
                    .is_err(),
                "replacement connected before the original Hub acknowledged its reply"
            );
            socket
                .send(Message::Text(
                    serde_json::to_string(&HubMessage::ResponseAck {
                        run_id: envelope.run_id.clone(),
                        request_id: envelope.request_id.clone(),
                    })?
                    .into(),
                ))
                .await?;
            let (stream, _) = timeout(Duration::from_secs(5), replacement.accept()).await??;
            let mut next = tokio_tungstenite::accept_async(stream).await?;
            anyhow::ensure!(matches!(
                next_agent_message(&mut next).await?,
                AgentMessage::Hello { .. }
            ));
            anyhow::ensure!(
                timeout(Duration::from_millis(250), next_agent_message(&mut next))
                    .await
                    .is_err(),
                "replacement Hub received a response/receipt belonging to the original target"
            );
            next.close(None).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(|_| anyhow!("Hub configuration switch test timed out"))
        .and_then(|result| result);
        // Expected failures also take this path: stop the owner cooperatively and await its
        // writer cleanup before RecoveryTestHome can remove the private fixture.
        shutdown.cancel();
        let cleaned = timeout(Duration::from_secs(5), &mut owner.0).await;
        if cleaned.is_err() {
            owner.stop().await;
        }
        scenario?;
        anyhow::ensure!(matches!(cleaned, Ok(Ok(()))), "Hub owner cleanup failed");
        Ok(())
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
                "hubTarget": "recovery-hub",
                "command": old_exec,
            }),
            serde_json::json!({
                "runId": "run-old-batch",
                "requestId": "request-old-batch",
                "commandHash": "hash-old-batch",
                "status": "started",
                "agentId": "recovery-agent",
                "hubTarget": "recovery-hub",
                "command": old_batch,
            }),
            serde_json::json!({
                "runId": "run-old-completed",
                "requestId": "request-old-completed",
                "commandHash": "hash-old-completed",
                "status": "completed",
                "agentId": "recovery-agent",
                "hubTarget": "recovery-hub",
                "command": old_completed,
                "result": old_result,
            }),
            serde_json::json!({
                "runId": "run-current",
                "requestId": "request-current",
                "commandHash": "hash-current",
                "status": "accepted",
                "agentId": "recovery-agent",
                "hubTarget": "recovery-hub",
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
        reconcile_transport_runs(
            &state,
            &ConnectionRoute::new("recovery-hub".to_owned(), tx.clone()),
        )
        .await;

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

    // ⑤ Keeps reporting hot changes, pending confirmation delivery, config.get projection,
    // and the auxiliary connection's master switch on one real transport lifecycle.
    #[tokio::test]
    async fn reporting_toggle_keeps_connection_and_master_disable_pauses_saved_intent() -> Result<()>
    {
        async fn next_non_heartbeat(
            socket: &mut WebSocketStream<TcpStream>,
        ) -> Result<AgentMessage> {
            loop {
                let message = socket
                    .next()
                    .await
                    .ok_or_else(|| anyhow!("socket closed"))??;
                let Message::Text(text) = message else {
                    continue;
                };
                let message: AgentMessage = serde_json::from_str(&text)?;
                if let AgentMessage::Heartbeat { sent_at } = message {
                    socket
                        .send(Message::Text(
                            serde_json::to_string(&HubMessage::HeartbeatAck {
                                sent_at,
                                received_at: Utc::now(),
                            })?
                            .into(),
                        ))
                        .await?;
                } else {
                    return Ok(message);
                }
            }
        }

        let home = recovery_test_home();
        let mut state = recovery_test_state(&home.root);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let replacement_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let replacement_url = format!("http://{}", replacement_listener.local_addr()?);
        let mut config = state.config.read().await.clone();
        config.mode = crate::config::RuntimeMode::Standalone;
        config.hub.enabled = true;
        config.hub.url = format!("http://{}", listener.local_addr()?);
        config.hub.agent_secret = "report-toggle-secret".to_owned();
        config.tunnel = Some(crate::config::TunnelConfig {
            tunnel_id: "report-toggle-test".to_owned(),
            api_key: "env:AGENTIC_TEST_TUNNEL_API_KEY".to_owned(),
            hub_reporting: crate::config::HubReportingConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        });
        config.toolsets.enable(crate::config::ToolNamespace::Config);
        config.config_api.update_policy = crate::config::ConfigUpdatePolicy::Allow;
        config.confirmation_provider.channels = vec![crate::config::ConfirmationChannel::Ntfy];
        crate::config::write_config_with_backup(&state.config_path, &config)?;
        *state.config.write().await = config.clone();
        state.runtime =
            crate::state::RuntimeModel::tunnel(crate::state::CapabilityProfile::Normal, true);

        let shutdown = CancellationToken::new();
        let owner_state = state.clone();
        let owner_shutdown = shutdown.clone();
        let mut owner = ConnectionTask(tokio::spawn(async move {
            let _ = connect_loop_with_shutdown(owner_state, owner_shutdown).await;
        }));

        let scenario = timeout(Duration::from_secs(30), async {
            let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
                .await
                .map_err(|_| anyhow!("Hub did not accept auxiliary connection"))??;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            anyhow::ensure!(matches!(
                next_non_heartbeat(&mut socket).await?,
                AgentMessage::Hello {
                    connection_mode: AgentConnectionMode::ReportingOnly,
                    ..
                }
            ));

            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(true)).await?;
            wait_reporting_applied(&state, true).await?;
            report_tool_arguments(
                &state,
                "report-open-run",
                "report-open-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            anyhow::ensure!(matches!(
                timeout(Duration::from_secs(3), next_non_heartbeat(&mut socket)).await??,
                AgentMessage::RunReport { .. }
            ));

            // A report queued while the active connection changes target must never reach the
            // old Hub. Then toggle reporting off and on to prove it resumes on the new target.
            set_config(&state, &["hub", "url"], serde_json::json!(replacement_url)).await?;
            report_tool_arguments(
                &state,
                "report-during-hub-change-run",
                "report-during-hub-change-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            match timeout(Duration::from_millis(200), next_non_heartbeat(&mut socket)).await {
                Ok(Ok(AgentMessage::RunReport { report }))
                    if report.request_id == "report-during-hub-change-request" =>
                {
                    return Err(anyhow!("new-target report reached the old Hub"));
                }
                Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {}
            }
            let (stream, _) = timeout(Duration::from_secs(5), replacement_listener.accept())
                .await
                .map_err(|_| anyhow!("Hub did not connect to the replacement address"))??;
            let mut socket = tokio_tungstenite::accept_async(stream).await?;
            anyhow::ensure!(matches!(
                next_non_heartbeat(&mut socket).await?,
                AgentMessage::Hello {
                    connection_mode: AgentConnectionMode::ReportingOnly,
                    ..
                }
            ));
            set_config(
                &state,
                &["tunnel", "hubReporting", "enabled"],
                serde_json::json!(false),
            )
            .await?;
            wait_reporting_applied(&state, false).await?;
            set_config(
                &state,
                &["tunnel", "hubReporting", "enabled"],
                serde_json::json!(true),
            )
            .await?;
            wait_reporting_applied(&state, true).await?;
            report_tool_arguments(
                &state,
                "report-after-hub-change-run",
                "report-after-hub-change-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            anyhow::ensure!(matches!(
                timeout(Duration::from_secs(3), next_non_heartbeat(&mut socket)).await??,
                AgentMessage::RunReport { report }
                    if report.request_id == "report-after-hub-change-request"
            ));
            anyhow::ensure!(
                timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err(),
                "the old Hub accepted a replacement connection"
            );
            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(false)).await?;
            let disabled = wait_reporting_applied(&state, false).await?;
            report_tool_arguments(
                &state,
                "report-closed-run",
                "report-closed-request",
                "process.exec",
                serde_json::json!({"command":"false"}),
                Utc::now(),
            );
            anyhow::ensure!(
                timeout(Duration::from_millis(150), next_non_heartbeat(&mut socket))
                    .await
                    .is_err()
            );
            anyhow::ensure!(disabled["items"][0]["applyStatus"] == "applied");

            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(true)).await?;
            wait_reporting_applied(&state, true).await?;
            report_tool_arguments(
                &state,
                "report-reopened-run",
                "report-reopened-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            anyhow::ensure!(matches!(
                timeout(Duration::from_secs(3), next_non_heartbeat(&mut socket)).await??,
                AgentMessage::RunReport { .. }
            ));

            let confirm_state = state.clone();
            let confirm_config = state.config.read().await.clone();
            let confirmation = confirmation::authorize_operation(
                &confirm_state,
                &confirm_config,
                None,
                crate::policy::PolicyDecision::Confirm,
                "true",
                &[],
                "请求运行 true",
            );
            tokio::pin!(confirmation);
            let request_id = tokio::select! {
                _result = &mut confirmation => return Err(anyhow!("confirmation ended before Hub request")),
                message = timeout(Duration::from_secs(3), next_non_heartbeat(&mut socket)) => {
                    match message?? {
                        AgentMessage::ConfirmationRequest { request_id, .. } => request_id,
                        other => return Err(anyhow!("unexpected confirmation message: {other:?}")),
                    }
                }
            };
            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(false)).await?;
            wait_reporting_applied(&state, false).await?;
            socket
                .send(Message::Text(serde_json::to_string(&HubMessage::ConfirmationResponse {
                    request_id,
                    decision: agentic_gpt_protocol::ConfirmationDecision::AllowOnce,
                    reason: "test".to_owned(),
                })?.into()))
                .await?;
            anyhow::ensure!(confirmation.await.map_err(|error| anyhow!(error))?.approved);
            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(true)).await?;
            wait_reporting_applied(&state, true).await?;

            set_config(&state, &["hub", "enabled"], serde_json::json!(false)).await?;
            timeout(Duration::from_secs(5), async {
                loop {
                    if state.hub_sender.lock().await.is_none() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await?;
            anyhow::ensure!(state.config.read().await.hub_reporting_enabled());
            let paused = wait_reporting_applied(&state, true).await?;
            anyhow::ensure!(paused["items"][0]["value"] == true);
            anyhow::ensure!(paused["items"][0]["applyStatus"] == "applied");
            anyhow::ensure!(paused["items"][0].get("appliedValue").is_none());
            anyhow::ensure!(
                crate::agent_info::collect(&state).await["connections"]["hubReporting"]["status"]
                    == "paused"
            );
            let disabled_config = crate::config_api::get(
                &state,
                ConfigGetRequest {
                    paths: vec![
                        vec!["hub".into(), "enabled".into()],
                        vec!["tunnel".into(), "hubReporting".into(), "enabled".into()],
                        vec!["confirmationProvider".into(), "channels".into()],
                    ],
                },
            )
            .await?;
            anyhow::ensure!(disabled_config["items"][0]["value"] == false);
            anyhow::ensure!(disabled_config["items"][0]["applyStatus"] == "applied");
            anyhow::ensure!(disabled_config["items"][1]["value"] == true);
            anyhow::ensure!(disabled_config["items"][1]["applyStatus"] == "applied");
            anyhow::ensure!(disabled_config["items"][2]["value"][0] == "ntfy");
            let confirmation_config = state.config.read().await.clone();
            let unavailable = confirmation::authorize_operation(
                &state,
                &confirmation_config,
                None,
                crate::policy::PolicyDecision::Confirm,
                "true",
                &[],
                "请求运行 true",
            )
            .await
            .expect_err("ntfy confirmation must fail while the Hub connection is disabled");
            anyhow::ensure!(unavailable == "provider_unavailable");
            anyhow::ensure!(
                timeout(Duration::from_millis(100), replacement_listener.accept())
                    .await
                    .is_err()
            );

            set_config(&state, &["hub", "enabled"], serde_json::json!(true)).await?;
            let (stream, _) = timeout(Duration::from_secs(5), replacement_listener.accept())
                .await
                .map_err(|_| anyhow!("Hub did not reconnect after enabling"))??;
            let mut reconnected = tokio_tungstenite::accept_async(stream).await?;
            anyhow::ensure!(matches!(
                next_non_heartbeat(&mut reconnected).await?,
                AgentMessage::Hello {
                    connection_mode: AgentConnectionMode::ReportingOnly,
                    ..
                }
            ));
            wait_reporting_applied(&state, true).await?;
            report_tool_arguments(
                &state,
                "report-resumed-run",
                "report-resumed-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            anyhow::ensure!(matches!(
                timeout(Duration::from_secs(3), next_non_heartbeat(&mut reconnected)).await??,
                AgentMessage::RunReport { .. }
            ));
            let resumed_config = crate::config_api::get(
                &state,
                ConfigGetRequest {
                    paths: vec![vec!["hub".into(), "enabled".into()]],
                },
            )
            .await?;
            anyhow::ensure!(resumed_config["items"][0]["value"] == true);
            anyhow::ensure!(resumed_config["items"][0]["applyStatus"] == "applied");
            anyhow::ensure!(resumed_config["items"][0].get("appliedValue").is_none());
            reconnected.close(None).await?;
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(|_| anyhow!("WebSocket reporting test timed out"))
        .and_then(|result| result);
        shutdown.cancel();
        owner.stop().await;
        scenario?;
        Ok(())
    }

    // ⑤ Covers the separate SSE event stream and POST writer lifecycle for the same behavior.
    #[tokio::test]
    async fn sse_reporting_toggle_keeps_connection_confirmation_and_paused_intent() -> Result<()> {
        use axum::{
            extract::{Path, State as AxumState},
            response::sse::{Event, Sse},
            routing::{get, post},
            Json, Router,
        };
        use std::{
            convert::Infallible,
            sync::atomic::{AtomicUsize, Ordering},
        };

        #[derive(Clone)]
        struct SseTestState {
            messages: mpsc::UnboundedSender<AgentMessage>,
            events: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>>,
            connections: Arc<AtomicUsize>,
        }

        async fn events(
            Path(_agent_id): Path<String>,
            AxumState(state): AxumState<SseTestState>,
        ) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
            state.connections.fetch_add(1, Ordering::Relaxed);
            let stream =
                futures_util::stream::unfold((state.events, true), |(events, first)| async move {
                    if first {
                        return Some((
                            Ok::<_, Infallible>(Event::default().comment("connected")),
                            (events, false),
                        ));
                    }
                    let data = events.lock().await.recv().await?;
                    Some((
                        Ok::<_, Infallible>(Event::default().data(data)),
                        (events, false),
                    ))
                });
            Sse::new(stream)
        }

        async fn messages(
            AxumState(state): AxumState<SseTestState>,
            Json(message): Json<AgentMessage>,
        ) -> axum::http::StatusCode {
            let _ = state.messages.send(message);
            axum::http::StatusCode::OK
        }

        async fn next_message(
            messages: &mut mpsc::UnboundedReceiver<AgentMessage>,
            events: &mpsc::UnboundedSender<String>,
        ) -> Result<AgentMessage> {
            loop {
                let message = timeout(Duration::from_secs(3), messages.recv())
                    .await
                    .map_err(|_| anyhow!("timed out waiting for Agent SSE POST"))?
                    .ok_or_else(|| anyhow!("SSE test server stopped"))?;
                if let AgentMessage::Heartbeat { sent_at } = message {
                    events.send(serde_json::to_string(&HubMessage::HeartbeatAck {
                        sent_at,
                        received_at: Utc::now(),
                    })?)?;
                } else {
                    return Ok(message);
                }
            }
        }

        let home = recovery_test_home();
        let mut state = recovery_test_state(&home.root);
        let (agent_messages, mut received_messages) = mpsc::unbounded_channel();
        let (hub_events, event_stream) = mpsc::unbounded_channel();
        let connections = Arc::new(AtomicUsize::new(0));
        let server_state = SseTestState {
            messages: agent_messages,
            events: Arc::new(tokio::sync::Mutex::new(event_stream)),
            connections: connections.clone(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let hub_addr = listener.local_addr()?;

        let mut config = state.config.read().await.clone();
        config.mode = crate::config::RuntimeMode::Standalone;
        config.hub.enabled = true;
        config.hub.transport = "sse".to_owned();
        config.hub.url = format!("http://{hub_addr}");
        config.hub.agent_secret = "sse-report-toggle-secret".to_owned();
        config.tunnel = Some(crate::config::TunnelConfig {
            tunnel_id: "sse-report-toggle-test".to_owned(),
            api_key: "env:AGENTIC_TEST_SSE_TUNNEL_API_KEY".to_owned(),
            hub_reporting: crate::config::HubReportingConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        });
        config.toolsets.enable(crate::config::ToolNamespace::Config);
        config.config_api.update_policy = crate::config::ConfigUpdatePolicy::Allow;
        config.confirmation_provider.channels = vec![crate::config::ConfirmationChannel::Ntfy];
        crate::config::write_config_with_backup(&state.config_path, &config)?;
        *state.config.write().await = config;
        state.runtime =
            crate::state::RuntimeModel::tunnel(crate::state::CapabilityProfile::Normal, true);

        let app = Router::new()
            .route("/v1/agents/:agent_id/events", get(events))
            .route("/v1/agents/:agent_id/messages", post(messages))
            .with_state(server_state);
        let mut server =
            tokio::spawn(async move { axum::serve(listener, app).await.expect("SSE test server") });

        let shutdown = CancellationToken::new();
        let owner_state = state.clone();
        let owner_shutdown = shutdown.clone();
        let mut owner = ConnectionTask(tokio::spawn(async move {
            let _ = connect_loop_with_shutdown(owner_state, owner_shutdown).await;
        }));

        let scenario = timeout(Duration::from_secs(30), async {
            anyhow::ensure!(matches!(
                next_message(&mut received_messages, &hub_events).await?,
                AgentMessage::Hello {
                    connection_mode: AgentConnectionMode::ReportingOnly,
                    ..
                }
            ));
            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(true)).await?;
            wait_reporting_applied(&state, true).await?;
            report_tool_arguments(
                &state,
                "sse-report-open-run",
                "sse-report-open-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            anyhow::ensure!(matches!(
                next_message(&mut received_messages, &hub_events).await?,
                AgentMessage::RunReport { .. }
            ));

            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(false)).await?;
            wait_reporting_applied(&state, false).await?;
            report_tool_arguments(
                &state,
                "sse-report-closed-run",
                "sse-report-closed-request",
                "process.exec",
                serde_json::json!({"command":"false"}),
                Utc::now(),
            );
            anyhow::ensure!(
                timeout(Duration::from_millis(150), received_messages.recv())
                    .await
                    .is_err()
            );

            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(true)).await?;
            wait_reporting_applied(&state, true).await?;
            report_tool_arguments(
                &state,
                "sse-report-reopen-run",
                "sse-report-reopen-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            anyhow::ensure!(matches!(
                next_message(&mut received_messages, &hub_events).await?,
                AgentMessage::RunReport { .. }
            ));

            let confirm_config = state.config.read().await.clone();
            let confirmation = confirmation::authorize_operation(
                &state,
                &confirm_config,
                None,
                crate::policy::PolicyDecision::Confirm,
                "true",
                &[],
                "请求运行 true",
            );
            tokio::pin!(confirmation);
            let request_id = tokio::select! {
                _result = &mut confirmation => return Err(anyhow!("confirmation ended before SSE request")),
                message = next_message(&mut received_messages, &hub_events) => {
                    match message? {
                        AgentMessage::ConfirmationRequest { request_id, .. } => request_id,
                        other => return Err(anyhow!("unexpected SSE confirmation message: {other:?}")),
                    }
                }
            };
            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(false)).await?;
            wait_reporting_applied(&state, false).await?;
            hub_events.send(serde_json::to_string(&HubMessage::ConfirmationResponse {
                request_id,
                decision: agentic_gpt_protocol::ConfirmationDecision::AllowOnce,
                reason: "test".to_owned(),
            })?)?;
            anyhow::ensure!(confirmation.await.map_err(|error| anyhow!(error))?.approved);
            set_config(&state, &["tunnel", "hubReporting", "enabled"], serde_json::json!(true)).await?;
            wait_reporting_applied(&state, true).await?;
            anyhow::ensure!(connections.load(Ordering::Relaxed) == 1);

            set_config(&state, &["hub", "enabled"], serde_json::json!(false)).await?;
            timeout(Duration::from_secs(5), async {
                loop {
                    if state.hub_sender.lock().await.is_none() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await?;
            wait_reporting_applied(&state, true).await?;
            anyhow::ensure!(connections.load(Ordering::Relaxed) == 1);
            anyhow::ensure!(state.config.read().await.hub_reporting_enabled());
            anyhow::ensure!(
                crate::agent_info::collect(&state).await["connections"]["hubReporting"]["status"]
                    == "paused"
            );
            let disabled_config = crate::config_api::get(
                &state,
                ConfigGetRequest {
                    paths: vec![
                        vec!["hub".into(), "enabled".into()],
                        vec!["tunnel".into(), "hubReporting".into(), "enabled".into()],
                        vec!["confirmationProvider".into(), "channels".into()],
                    ],
                },
            )
            .await?;
            anyhow::ensure!(disabled_config["items"][0]["value"] == false);
            anyhow::ensure!(disabled_config["items"][0]["applyStatus"] == "applied");
            anyhow::ensure!(disabled_config["items"][1]["value"] == true);
            anyhow::ensure!(disabled_config["items"][1]["applyStatus"] == "applied");
            anyhow::ensure!(disabled_config["items"][2]["value"][0] == "ntfy");
            tokio::time::sleep(Duration::from_millis(100)).await;
            anyhow::ensure!(connections.load(Ordering::Relaxed) == 1);

            set_config(&state, &["hub", "enabled"], serde_json::json!(true)).await?;
            anyhow::ensure!(matches!(
                next_message(&mut received_messages, &hub_events).await?,
                AgentMessage::Hello {
                    connection_mode: AgentConnectionMode::ReportingOnly,
                    ..
                }
            ));
            wait_reporting_applied(&state, true).await?;
            report_tool_arguments(
                &state,
                "sse-report-resumed-run",
                "sse-report-resumed-request",
                "process.exec",
                serde_json::json!({"command":"true"}),
                Utc::now(),
            );
            anyhow::ensure!(matches!(
                next_message(&mut received_messages, &hub_events).await?,
                AgentMessage::RunReport { .. }
            ));
            anyhow::ensure!(connections.load(Ordering::Relaxed) == 2);
            let resumed_config = crate::config_api::get(
                &state,
                ConfigGetRequest {
                    paths: vec![vec!["hub".into(), "enabled".into()]],
                },
            )
            .await?;
            anyhow::ensure!(resumed_config["items"][0]["value"] == true);
            anyhow::ensure!(resumed_config["items"][0]["applyStatus"] == "applied");
            anyhow::ensure!(resumed_config["items"][0].get("appliedValue").is_none());
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(|_| anyhow!("SSE reporting test timed out"))
        .and_then(|result| result);

        shutdown.cancel();
        owner.stop().await;
        server.abort();
        let _ = (&mut server).await;
        scenario?;
        Ok(())
    }
}
