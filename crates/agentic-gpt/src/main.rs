mod mcp;
mod notebook;

use agentic_gpt_protocol::{
    AgentMessage, AgentRole, BatchElementResult, BatchExecRequest, BatchExecResult,
    ConfirmationDecision, ConfirmationPayload, ExecElement, ExecRequest, HubCommand, HubMessage,
    PolicyCounts, SafeBuiltinPolicyRules, SafeConfigSummary, SafePathPolicySummary, SafePathRoot,
    SafePolicyRules, SafeRule, SafeSandboxSummary, SessionInfo, TaskResult,
};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use mcp::{McpConfigCommand, McpServerConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout, Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Response as WsResponse;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    client_async_tls_with_config, connect_async, MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;

const DEFAULT_BACKUP_LIMIT: usize = 5;
const EXEC_TIMEOUT_SECS: u64 = 30;
const CONFIRM_TIMEOUT_SECS: u64 = 45;
const STDOUT_MAX: usize = 64 * 1024;
const STDERR_MAX: usize = 64 * 1024;
const SESSION_TAIL_MAX: usize = 32 * 1024;
const RECONNECT_DELAY_SECS: u64 = 3;
const CONNECT_TIMEOUT_SECS: u64 = 20;
const HEARTBEAT_INTERVAL_SECS: u64 = 15;
const HEARTBEAT_ACK_TIMEOUT_SECS: u64 = 45;

#[derive(Parser)]
#[command(name = "agentic-gpt")]
#[command(about = "Linux local agent for Agentic GPT")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    RunAsRoom {
        #[arg(long)]
        config: Option<PathBuf>,
    },
    Config {
        #[arg(long)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    Init,
    Show,
    Set {
        key: String,
        value: String,
    },
    Allow {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Confirm {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Deny {
        #[command(subcommand)]
        command: RuleCommand,
    },
    Path {
        #[command(subcommand)]
        command: PathCommand,
    },
    Mcp {
        #[command(subcommand)]
        command: McpConfigCommand,
    },
}

#[derive(Subcommand)]
enum RuleCommand {
    Add {
        program: String,
        args_prefix: Vec<String>,
    },
    Remove {
        program: String,
        args_prefix: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PathCommand {
    List,
    Write {
        #[command(subcommand)]
        command: PathRootCommand,
    },
    Readonly {
        #[command(subcommand)]
        command: PathRootCommand,
    },
    Deny {
        #[command(subcommand)]
        command: PathRootCommand,
    },
}

#[derive(Subcommand)]
enum PathRootCommand {
    Add { path: PathBuf },
    Remove { path: PathBuf },
}

#[derive(Clone, Copy, Debug)]
enum PathRootKind {
    Write,
    Readonly,
    Deny,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum PolicyDecision {
    Allow,
    Confirm,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunMode {
    Normal,
    Room,
}

impl RunMode {
    fn role(self) -> AgentRole {
        match self {
            RunMode::Normal => AgentRole::Normal,
            RunMode::Room => AgentRole::Room,
        }
    }

    fn label(self) -> &'static str {
        match self {
            RunMode::Normal => "normal",
            RunMode::Room => "room",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    agent_id: String,
    display_name: String,
    #[serde(alias = "workerUrl")]
    hub_url: String,
    agent_secret: String,
    workspace_root: PathBuf,
    backup_limit: usize,
    confirmation_provider: ConfirmationProviderConfig,
    #[serde(default = "default_confirmation_language")]
    confirmation_language: String,
    sandbox: SandboxConfig,
    #[serde(default)]
    mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    path_policy: PathPolicyConfig,
    policy: PolicyConfig,
    limits: LimitsConfig,
    #[serde(default = "default_room_config")]
    room: RoomConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmationProviderConfig {
    provider: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SandboxConfig {
    enabled: bool,
    bubblewrap_path: String,
    required_runtime_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PathPolicyConfig {
    #[serde(default)]
    write_roots: Vec<PathBuf>,
    #[serde(default)]
    read_only_roots: Vec<PathBuf>,
    #[serde(default)]
    deny_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyConfig {
    allow: Vec<Rule>,
    confirm: Vec<Rule>,
    deny: Vec<Rule>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Rule {
    program: String,
    args_prefix: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LimitsConfig {
    max_concurrent_tasks: usize,
    max_active_sessions: usize,
    session_idle_timeout_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notebook_root: Option<PathBuf>,
    timezone: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditRecord {
    task_id: Option<String>,
    session_id: Option<String>,
    time: DateTime<Utc>,
    program: String,
    args: Vec<String>,
    working_directory: Option<String>,
    need_confirm: bool,
    policy_decision: String,
    confirmation_result: Option<String>,
    exit_code: Option<i32>,
    duration_ms: u128,
    truncated: bool,
    request_source: String,
    reject_reason: Option<String>,
}

#[derive(Clone)]
struct AppState {
    config_path: PathBuf,
    config: Arc<RwLock<Config>>,
    run_mode: RunMode,
    sessions: Arc<Mutex<HashMap<String, ManagedSession>>>,
    hub_sender: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
    pending_confirmations: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
    temporary_mcp_allows: Arc<Mutex<Vec<TemporaryMcpAllow>>>,
    notebook_writes: Arc<Mutex<()>>,
}

#[derive(Clone, Debug)]
struct TemporaryMcpAllow {
    agent_id: String,
    server_id: String,
    expires_at: DateTime<Utc>,
}

struct ManagedSession {
    info: SessionInfo,
    child: Child,
    stdout: Arc<Mutex<TailBuffer>>,
    stderr: Arc<Mutex<TailBuffer>>,
    last_activity: Instant,
}

#[derive(Debug, Default)]
struct TailBuffer {
    data: VecDeque<u8>,
    max: usize,
    truncated: bool,
}

impl TailBuffer {
    fn new(max: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(max),
            max,
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.data.len() == self.max {
                self.data.pop_front();
                self.truncated = true;
            }
            self.data.push_back(*byte);
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.data.iter().copied().collect::<Vec<_>>()).to_string()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    match cli.command {
        Commands::Run { config } => run(config_path(config), RunMode::Normal).await,
        Commands::RunAsRoom { config } => run(config_path(config), RunMode::Room).await,
        Commands::Config { config, command } => handle_config(config_path(config), command).await,
    }
}

async fn run(config_path: PathBuf, run_mode: RunMode) -> Result<()> {
    log_info(format!(
        "agentic-gpt starting; mode={}; config={}",
        run_mode.label(),
        config_path.display(),
    ));
    ensure_parent(&config_path)?;
    if !config_path.exists() {
        write_config_with_backup(&config_path, &Config::default_config()?)?;
        log_info("default config created".to_string());
    }
    let initial = Config::load(&config_path)?;
    initial.ensure_workspace()?;
    log_info(format!(
        "config loaded; agentId={}; hubUrl={}; workspaceRoot={}; sandbox={}",
        initial.agent_id,
        initial.hub_url,
        initial.workspace_root.display(),
        if initial.sandbox.enabled {
            "enabled"
        } else {
            "disabled"
        }
    ));
    let state = AppState {
        config_path: config_path.clone(),
        config: Arc::new(RwLock::new(initial)),
        run_mode,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        hub_sender: Arc::new(Mutex::new(None)),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
        notebook_writes: Arc::new(Mutex::new(())),
    };
    tokio::spawn(watch_config(state.clone()));
    connect_loop(state).await
}

async fn connect_loop(state: AppState) -> Result<()> {
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
                                        let value = confirmation_decision_value(decision);
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
                fail_pending_confirmations(&state, "provider_unavailable").await;
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

async fn handle_hub_command(state: AppState, command: HubCommand) -> Result<()> {
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
            let result = run_exec_task(state.clone(), task_id, payload).await;
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
            let result = run_batch_task(state.clone(), task_id, payload).await;
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
            let info = start_session(state.clone(), session_id, payload).await;
            log_info(format!(
                "startSession result; sessionId={}; state={}; rejectReason={:?}",
                info.session_id, info.state, info.reject_reason
            ));
            send_session(&state, &info).await?;
            send_response(&state, &request_id, serde_json::to_value(&info)?).await?;
        }
        HubCommand::ListSessions { request_id } => {
            let sessions = current_sessions(&state).await;
            send_response(&state, &request_id, serde_json::to_value(sessions)?).await?;
        }
        HubCommand::InspectSession {
            request_id,
            session_id,
        } => {
            let session = inspect_session(&state, &session_id).await;
            send_response(&state, &request_id, serde_json::to_value(session)?).await?;
        }
        HubCommand::WaitSession {
            request_id,
            session_id,
            seconds,
        } => {
            sleep(Duration::from_secs(seconds.min(30))).await;
            let session = inspect_session(&state, &session_id).await;
            send_response(&state, &request_id, serde_json::to_value(session)?).await?;
        }
        HubCommand::KillSession {
            request_id,
            session_id,
        } => {
            log_info(format!("killSession received; sessionId={session_id}"));
            let session = kill_session(&state, &session_id).await;
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

fn room_agent_required_error() -> serde_json::Value {
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

async fn send_agent_message(state: &AppState, message: AgentMessage) -> Result<()> {
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

async fn run_exec_task(state: AppState, task_id: String, request: ExecRequest) -> TaskResult {
    let started_at = Utc::now();
    let mut result = TaskResult {
        agent_id: request.agent_id.clone(),
        task_id: task_id.clone(),
        status: "running".to_string(),
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
        started_at,
        updated_at: started_at,
    };
    let config = state.config.read().await.clone();
    let started = Instant::now();
    let decision = policy_decision_for_mode(
        &config,
        state.run_mode,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    let mut confirmation_result = None;
    let working_directory =
        match resolve_working_directory(&config, request.working_directory.as_deref()) {
            Ok(working_directory) => Some(working_directory),
            Err(reason) => {
                result.status = "rejected".to_string();
                result.reject_reason = Some(reason);
                None
            }
        };

    if let Some(working_directory) = working_directory.as_deref() {
        if let Err(reason) = preflight(&config, working_directory, &request.program, &request.args)
        {
            result.status = "rejected".to_string();
            result.reject_reason = Some(reason);
        }
    }
    if result.reject_reason.is_none() && decision == PolicyDecision::Deny {
        result.status = "rejected".to_string();
        result.reject_reason = Some("policy_denied".to_string());
    } else if result.reject_reason.is_none() && decision == PolicyDecision::Confirm {
        let confirmation = request_confirmation(
            &state,
            &config,
            request.confirm_method.as_deref(),
            &request.program,
            &request.args,
        )
        .await;
        confirmation_result = Some(confirmation.clone());
        if confirmation != "allow_once" {
            result.status = "rejected".to_string();
            result.reject_reason = Some(confirmation);
        }
    }

    if result.reject_reason.is_none() {
        let execution = execute_command(
            &config,
            working_directory
                .as_deref()
                .unwrap_or(&config.workspace_root),
            &request.program,
            &request.args,
        )
        .await;
        match execution {
            Ok(output) => {
                result.status = if output.exit_code == Some(0) {
                    "completed"
                } else {
                    "failed"
                }
                .to_string();
                result.exit_code = output.exit_code;
                result.stdout_tail = output.stdout;
                result.stderr_tail = output.stderr;
                result.truncated = output.truncated;
            }
            Err(reason) => {
                let reason = reason.to_string();
                if reason == "timeout" {
                    result.status = "timeout".to_string();
                    result.reject_reason = Some("exec_timeout_use_session".to_string());
                } else {
                    result.status = "failed".to_string();
                    result.reject_reason = Some(reason);
                }
            }
        }
    }
    result.updated_at = Utc::now();
    let _ = write_audit(
        &config,
        AuditRecord {
            task_id: Some(task_id),
            session_id: None,
            time: result.updated_at,
            program: request.program,
            args: request.args,
            working_directory: request.working_directory,
            need_confirm: request.need_confirm,
            policy_decision: format!("{decision:?}"),
            confirmation_result,
            exit_code: result.exit_code,
            duration_ms: started.elapsed().as_millis(),
            truncated: result.truncated,
            request_source: "hub".to_string(),
            reject_reason: result.reject_reason.clone(),
        },
    );
    result
}

#[derive(Clone)]
struct PreparedBatchElement {
    index: usize,
    program: String,
    args: Vec<String>,
    working_directory: Option<String>,
    resolved_working_directory: PathBuf,
    decision: PolicyDecision,
    reject_reason: Option<String>,
}

fn prepare_batch_element(
    config: &Config,
    index: usize,
    element: ExecElement,
    batch_working_directory: Option<String>,
    need_confirm: bool,
) -> PreparedBatchElement {
    let program = element.program;
    let args = element.args;
    let working_directory = element.working_directory.or(batch_working_directory);
    let decision = policy_decision(config, &program, &args, need_confirm);
    let mut reject_reason = None;
    let resolved_working_directory =
        match resolve_working_directory(config, working_directory.as_deref()) {
            Ok(directory) => directory,
            Err(reason) => {
                reject_reason = Some(reason);
                config.workspace_root.clone()
            }
        };
    if reject_reason.is_none() {
        if let Err(reason) = preflight(config, &resolved_working_directory, &program, &args) {
            reject_reason = Some(reason);
        }
    }
    if reject_reason.is_none() && decision == PolicyDecision::Deny {
        reject_reason = Some("policy_denied".to_string());
    }
    PreparedBatchElement {
        index,
        program,
        args,
        working_directory,
        resolved_working_directory,
        decision,
        reject_reason,
    }
}

fn batch_element_result(
    agent_id: &str,
    batch_id: &str,
    element: &PreparedBatchElement,
    status: &str,
    reject_reason: Option<String>,
    started_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> BatchElementResult {
    BatchElementResult {
        index: element.index,
        program: element.program.clone(),
        args: element.args.clone(),
        working_directory: element.working_directory.clone(),
        result: TaskResult {
            agent_id: agent_id.to_string(),
            task_id: format!("{batch_id}:element:{}", element.index),
            status: status.to_string(),
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason,
            started_at,
            updated_at,
        },
    }
}

async fn run_prepared_batch_element(
    config: Config,
    agent_id: String,
    batch_id: String,
    element: PreparedBatchElement,
    need_confirm: bool,
    confirmation_result: Option<String>,
) -> BatchElementResult {
    let started_at = Utc::now();
    let started = Instant::now();
    let task_id = format!("{batch_id}:element:{}", element.index);
    let mut result = TaskResult {
        agent_id: agent_id.clone(),
        task_id: task_id.clone(),
        status: "running".to_string(),
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
        started_at,
        updated_at: started_at,
    };

    let execution = execute_command(
        &config,
        &element.resolved_working_directory,
        &element.program,
        &element.args,
    )
    .await;
    match execution {
        Ok(output) => {
            result.status = if output.exit_code == Some(0) {
                "completed"
            } else {
                "failed"
            }
            .to_string();
            result.exit_code = output.exit_code;
            result.stdout_tail = output.stdout;
            result.stderr_tail = output.stderr;
            result.truncated = output.truncated;
        }
        Err(reason) => {
            let reason = reason.to_string();
            if reason == "timeout" {
                result.status = "timeout".to_string();
                result.reject_reason = Some("exec_timeout_use_session".to_string());
            } else {
                result.status = "failed".to_string();
                result.reject_reason = Some(reason);
            }
        }
    }
    result.updated_at = Utc::now();
    let _ = write_audit(
        &config,
        AuditRecord {
            task_id: Some(task_id),
            session_id: None,
            time: result.updated_at,
            program: element.program.clone(),
            args: element.args.clone(),
            working_directory: element.working_directory.clone(),
            need_confirm,
            policy_decision: format!("{:?}", element.decision),
            confirmation_result,
            exit_code: result.exit_code,
            duration_ms: started.elapsed().as_millis(),
            truncated: result.truncated,
            request_source: "hub:batch".to_string(),
            reject_reason: result.reject_reason.clone(),
        },
    );
    BatchElementResult {
        index: element.index,
        program: element.program,
        args: element.args,
        working_directory: element.working_directory,
        result,
    }
}

async fn run_batch_task(
    state: AppState,
    batch_id: String,
    request: BatchExecRequest,
) -> BatchExecResult {
    let started_at = Utc::now();
    let agent_id = request.agent_id.clone();
    let need_confirm = request.need_confirm;
    let confirm_method = request.confirm_method.clone();
    let config = state.config.read().await.clone();
    let prepared = request
        .elements
        .into_iter()
        .enumerate()
        .map(|(index, element)| {
            prepare_batch_element(
                &config,
                index,
                element,
                request.working_directory.clone(),
                need_confirm,
            )
        })
        .collect::<Vec<_>>();
    let total = prepared.len();
    let max_concurrent = config.limits.max_concurrent_tasks.max(1).min(total.max(1));

    if prepared
        .iter()
        .any(|element| element.reject_reason.is_some())
    {
        let updated_at = Utc::now();
        let results = prepared
            .iter()
            .map(|element| {
                if let Some(reason) = &element.reject_reason {
                    batch_element_result(
                        &agent_id,
                        &batch_id,
                        element,
                        "rejected",
                        Some(reason.clone()),
                        started_at,
                        updated_at,
                    )
                } else {
                    batch_element_result(
                        &agent_id,
                        &batch_id,
                        element,
                        "skipped",
                        Some("batch_rejected".to_string()),
                        started_at,
                        updated_at,
                    )
                }
            })
            .collect::<Vec<_>>();
        return BatchExecResult {
            agent_id,
            batch_id,
            status: "rejected".to_string(),
            results,
            started_at,
            updated_at,
        };
    }

    let needs_confirmation = prepared
        .iter()
        .filter(|element| element.decision == PolicyDecision::Confirm)
        .cloned()
        .collect::<Vec<_>>();
    let mut batch_confirmation_result = None;
    if !needs_confirmation.is_empty() {
        let confirmation = request_batch_confirmation(
            &state,
            &config,
            confirm_method.as_deref(),
            &needs_confirmation,
            &prepared,
        )
        .await;
        batch_confirmation_result = Some(confirmation.clone());
        if confirmation != "allow_once" {
            let updated_at = Utc::now();
            let reason = if confirmation == "timeout" {
                "batch_confirmation_timeout".to_string()
            } else {
                format!("batch_confirmation_{confirmation}")
            };
            let results = prepared
                .iter()
                .map(|element| {
                    if element.decision == PolicyDecision::Confirm {
                        batch_element_result(
                            &agent_id,
                            &batch_id,
                            element,
                            "rejected",
                            Some(reason.clone()),
                            started_at,
                            updated_at,
                        )
                    } else {
                        batch_element_result(
                            &agent_id,
                            &batch_id,
                            element,
                            "skipped",
                            Some("batch_rejected".to_string()),
                            started_at,
                            updated_at,
                        )
                    }
                })
                .collect::<Vec<_>>();
            return BatchExecResult {
                agent_id,
                batch_id,
                status: "rejected".to_string(),
                results,
                started_at,
                updated_at,
            };
        }
    }

    let mut pending = prepared.into_iter().collect::<VecDeque<_>>();
    let mut running = JoinSet::new();
    let mut results: Vec<Option<BatchElementResult>> = vec![None; total];
    let deadline = Instant::now() + Duration::from_secs(EXEC_TIMEOUT_SECS);

    loop {
        while running.len() < max_concurrent {
            let Some(element) = pending.pop_front() else {
                break;
            };
            let element_config = config.clone();
            let element_agent_id = agent_id.clone();
            let element_batch_id = batch_id.clone();
            let confirmation_result = if element.decision == PolicyDecision::Confirm {
                batch_confirmation_result.clone()
            } else {
                None
            };
            running.spawn(async move {
                run_prepared_batch_element(
                    element_config,
                    element_agent_id,
                    element_batch_id,
                    element,
                    need_confirm,
                    confirmation_result,
                )
                .await
            });
        }

        if running.is_empty() {
            break;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match timeout(remaining, running.join_next()).await {
            Ok(Some(Ok(element_result))) => {
                let index = element_result.index;
                if index < results.len() {
                    results[index] = Some(element_result);
                }
            }
            Ok(Some(Err(error))) => {
                log_warn(format!("batch element task join failed: {error}"));
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    running.abort_all();

    let updated_at = Utc::now();
    let results = results
        .into_iter()
        .enumerate()
        .map(|(index, result)| {
            result.unwrap_or_else(|| {
                let fallback = PreparedBatchElement {
                    index,
                    program: "<unknown>".to_string(),
                    args: Vec::new(),
                    working_directory: None,
                    resolved_working_directory: config.workspace_root.clone(),
                    decision: PolicyDecision::Allow,
                    reject_reason: None,
                };
                batch_element_result(
                    &agent_id,
                    &batch_id,
                    &fallback,
                    "timeout",
                    Some("exec_timeout_use_session".to_string()),
                    started_at,
                    updated_at,
                )
            })
        })
        .collect::<Vec<_>>();

    let status = if results
        .iter()
        .any(|element| element.result.status == "timeout")
    {
        "timeout"
    } else if results
        .iter()
        .any(|element| element.result.status != "completed")
    {
        "partial_failed"
    } else {
        "completed"
    }
    .to_string();

    BatchExecResult {
        agent_id,
        batch_id,
        status,
        results,
        started_at,
        updated_at,
    }
}

#[derive(Debug)]
struct CommandOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    truncated: bool,
}

async fn execute_command(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> Result<CommandOutput> {
    let mut command = build_command(config, working_directory, program)?;
    command.args(args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| anyhow!("spawn_failed: {error}"))?;
    let stdout = child.stdout.take().context("stdout pipe missing")?;
    let stderr = child.stderr.take().context("stderr pipe missing")?;
    let stdout_task = tokio::spawn(read_limited(stdout, STDOUT_MAX));
    let stderr_task = tokio::spawn(read_limited(stderr, STDERR_MAX));
    let status = match timeout(Duration::from_secs(EXEC_TIMEOUT_SECS), child.wait()).await {
        Ok(status) => status.map_err(|error| anyhow!("wait_failed: {error}"))?,
        Err(_) => {
            let _ = child.kill().await;
            return Err(anyhow!("timeout"));
        }
    };
    let stdout = stdout_task.await??;
    let stderr = stderr_task.await??;
    Ok(CommandOutput {
        exit_code: status.code(),
        truncated: stdout.1 || stderr.1,
        stdout: stdout.0,
        stderr: stderr.0,
    })
}

async fn read_limited<R: AsyncRead + Unpin>(mut reader: R, max: usize) -> Result<(String, bool)> {
    let mut tail = TailBuffer::new(max);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        tail.push(&buffer[..read]);
    }
    Ok((tail.text(), tail.truncated))
}

fn preflight(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> std::result::Result<(), String> {
    if program == "sudo" {
        return Err("interactive_credential_required".to_string());
    }
    if matches!(program, "passwd" | "su" | "login") {
        return Err("interactive_credential_required".to_string());
    }
    if matches!(
        program,
        "vim" | "vi" | "nano" | "less" | "more" | "top" | "htop"
    ) {
        return Err("requires_tty_not_supported".to_string());
    }
    check_path_policy(config, working_directory, program, args)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PathAccessKind {
    Read,
    Write,
}

fn classify_program_access(program: &str) -> PathAccessKind {
    if matches!(
        program,
        "cat"
            | "head"
            | "tail"
            | "stat"
            | "file"
            | "wc"
            | "ls"
            | "find"
            | "du"
            | "df"
            | "upower"
            | "free"
            | "uptime"
            | "fastfetch"
            | "journalctl"
            | "btrfs"
            | "pacman"
    ) {
        PathAccessKind::Read
    } else {
        PathAccessKind::Write
    }
}

fn looks_like_path(arg: &str) -> bool {
    arg == "~"
        || arg.starts_with("~/")
        || arg.starts_with('/')
        || arg.starts_with("./")
        || arg.starts_with("../")
}

fn check_path_policy(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> std::result::Result<(), String> {
    let access = classify_program_access(program);
    let policy = expanded_path_policy(config).map_err(|_| "path_policy_error".to_string())?;
    for arg in args {
        if !looks_like_path(arg) {
            continue;
        }
        let path = resolve_argument_path(working_directory, arg, access)?;
        if path_in_roots(&path, &policy.deny_roots) {
            return Err("path_denied".to_string());
        }
        if program == "df" && arg == "/" {
            continue;
        }
        if path_in_roots(&path, &policy.write_roots) {
            if access == PathAccessKind::Read && !path.exists() {
                return Err("path_not_found".to_string());
            }
            continue;
        }
        if path_in_roots(&path, &policy.read_only_roots) {
            if access == PathAccessKind::Read {
                if !path.exists() {
                    return Err("path_not_found".to_string());
                }
                continue;
            }
            return Err("path_readonly".to_string());
        }
        return Err("path_outside_allowed_roots".to_string());
    }
    Ok(())
}

fn resolve_argument_path(
    workspace_root: &Path,
    arg: &str,
    _access: PathAccessKind,
) -> std::result::Result<PathBuf, String> {
    let expanded = expand_path(arg).map_err(|_| "path_policy_error".to_string())?;
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        workspace_root.join(expanded)
    };
    if candidate.exists() {
        return candidate
            .canonicalize()
            .map_err(|_| "path_policy_error".to_string());
    }
    let parent = candidate
        .parent()
        .ok_or_else(|| "path_not_found".to_string())?;
    let parent = parent
        .canonicalize()
        .map_err(|_| "path_not_found".to_string())?;
    Ok(candidate
        .file_name()
        .map(|name| parent.join(name))
        .unwrap_or(parent))
}

fn path_in_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

#[derive(Debug)]
struct ExpandedPathPolicy {
    write_roots: Vec<PathBuf>,
    read_only_roots: Vec<PathBuf>,
    deny_roots: Vec<PathBuf>,
}

fn expanded_path_policy(config: &Config) -> Result<ExpandedPathPolicy> {
    Ok(ExpandedPathPolicy {
        write_roots: normalize_roots(
            config
                .path_policy
                .write_roots
                .iter()
                .chain(std::iter::once(&config.workspace_root)),
        )?,
        read_only_roots: normalize_roots(config.path_policy.read_only_roots.iter())?,
        deny_roots: normalize_roots(config.path_policy.deny_roots.iter())?,
    })
}

fn normalize_roots<'a>(roots: impl Iterator<Item = &'a PathBuf>) -> Result<Vec<PathBuf>> {
    let mut normalized = Vec::new();
    for root in roots {
        let expanded = expand_pathbuf(root)?;
        let normalized_root = canonicalize_existing_or_parent(&expanded)?;
        if !normalized
            .iter()
            .any(|existing| existing == &normalized_root)
        {
            normalized.push(normalized_root);
        }
    }
    Ok(normalized)
}

fn canonicalize_existing_or_parent(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    if let Some(parent) = path.parent() {
        if parent.exists() {
            let parent = parent.canonicalize()?;
            return Ok(path
                .file_name()
                .map(|name| parent.join(name))
                .unwrap_or(parent));
        }
    }
    Ok(path.to_path_buf())
}

fn expand_path(value: &str) -> Result<PathBuf> {
    if value == "~" {
        return dirs::home_dir().context("home directory not found");
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(dirs::home_dir()
            .context("home directory not found")?
            .join(rest));
    }
    Ok(PathBuf::from(value))
}

fn expand_pathbuf(value: &Path) -> Result<PathBuf> {
    value
        .to_str()
        .map(expand_path)
        .unwrap_or_else(|| Ok(value.to_path_buf()))
}

fn resolve_working_directory(
    config: &Config,
    working_directory: Option<&str>,
) -> std::result::Result<PathBuf, String> {
    let candidate = match working_directory {
        Some(value) if value.trim().is_empty() => {
            return Err("working_directory_empty".to_string());
        }
        Some(value) => {
            let expanded =
                expand_path(value).map_err(|_| "working_directory_invalid".to_string())?;
            if expanded.is_absolute() {
                expanded
            } else {
                config.workspace_root.join(expanded)
            }
        }
        None => config.workspace_root.clone(),
    };
    let directory = candidate
        .canonicalize()
        .map_err(|_| "working_directory_not_found".to_string())?;
    if !directory.is_dir() {
        return Err("working_directory_not_directory".to_string());
    }
    let policy = expanded_path_policy(config).map_err(|_| "path_policy_error".to_string())?;
    if path_in_roots(&directory, &policy.deny_roots) {
        return Err("working_directory_denied".to_string());
    }
    if !path_in_roots(&directory, &policy.write_roots) {
        return Err("working_directory_outside_allowed_roots".to_string());
    }
    Ok(directory)
}

fn build_command(config: &Config, working_directory: &Path, program: &str) -> Result<Command> {
    if config.sandbox.enabled {
        let policy = expanded_path_policy(config)?;
        let mut command = Command::new(&config.sandbox.bubblewrap_path);
        command
            .arg("--die-with-parent")
            .arg("--unshare-all")
            .arg("--dev")
            .arg("/dev")
            .arg("--chdir")
            .arg(working_directory);
        let mut created_dirs = HashSet::new();
        for path in &policy.write_roots {
            if path.exists() {
                add_bwrap_bind(&mut command, &mut created_dirs, "--bind", path);
            }
        }
        for path in &policy.read_only_roots {
            if path.exists() {
                add_bwrap_bind(&mut command, &mut created_dirs, "--ro-bind", path);
            }
        }
        for path in &config.sandbox.required_runtime_paths {
            if path.exists() {
                add_bwrap_bind(&mut command, &mut created_dirs, "--ro-bind", path);
            }
        }
        command.arg("--").arg(program);
        Ok(command)
    } else {
        let mut command = Command::new(program);
        command.current_dir(working_directory);
        Ok(command)
    }
}

fn add_bwrap_bind(
    command: &mut Command,
    created_dirs: &mut HashSet<PathBuf>,
    bind_arg: &str,
    path: &Path,
) {
    add_bwrap_parent_dirs(command, created_dirs, path);
    command.arg(bind_arg).arg(path).arg(path);
}

fn add_bwrap_parent_dirs(command: &mut Command, created_dirs: &mut HashSet<PathBuf>, path: &Path) {
    let mut parents = path.ancestors().skip(1).collect::<Vec<_>>();
    parents.reverse();
    for parent in parents {
        if parent == Path::new("/") || parent.as_os_str().is_empty() {
            continue;
        }
        let parent = parent.to_path_buf();
        if created_dirs.insert(parent.clone()) {
            command.arg("--dir").arg(parent);
        }
    }
}

fn batch_confirmation_preview(
    config: &Config,
    needs_confirmation: &[PreparedBatchElement],
    all_elements: &[PreparedBatchElement],
) -> String {
    let zh = confirmation_language_is_zh(config);
    let mut lines = vec![if zh {
        format!(
            "该批次共有 {} 条命令，其中 {} 条需要确认：",
            all_elements.len(),
            needs_confirmation.len()
        )
    } else {
        format!(
            "Batch requires confirmation for {} of {} commands:",
            needs_confirmation.len(),
            all_elements.len()
        )
    }];
    for element in needs_confirmation.iter().take(8) {
        let cwd = element
            .working_directory
            .as_ref()
            .map(|directory| {
                if zh {
                    format!("（工作目录：{directory}）")
                } else {
                    format!(" (cwd: {directory})")
                }
            })
            .unwrap_or_default();
        lines.push(format!(
            "[{}] {}{}",
            element.index,
            command_preview(&element.program, &element.args),
            cwd
        ));
    }
    if needs_confirmation.len() > 8 {
        lines.push(if zh {
            format!(
                "……另外还有 {} 条需要确认的命令",
                needs_confirmation.len() - 8
            )
        } else {
            format!(
                "... and {} more commands requiring confirmation",
                needs_confirmation.len() - 8
            )
        });
    }
    let other_count = all_elements.len().saturating_sub(needs_confirmation.len());
    if other_count > 0 {
        lines.push(if zh {
            format!("另外包含 {other_count} 条不需要确认的命令。")
        } else {
            format!("Also included: {other_count} command(s) that do not require confirmation.")
        });
    }
    lines.push(if zh {
        "是否允许整个批次执行一次？".to_string()
    } else {
        "Allow the entire batch once?".to_string()
    });
    lines.join("\n")
}

async fn request_batch_confirmation(
    state: &AppState,
    config: &Config,
    confirm_method: Option<&str>,
    needs_confirmation: &[PreparedBatchElement],
    all_elements: &[PreparedBatchElement],
) -> String {
    let configured_provider = config.confirmation_provider.provider.as_str();
    let provider = confirm_method
        .filter(|method| !method.trim().is_empty())
        .unwrap_or(configured_provider);
    let provider = if provider == "default" {
        configured_provider
    } else if provider == "freedesktopThenHub" {
        "freedesktop-then-hub"
    } else {
        provider
    };
    let preview = batch_confirmation_preview(config, needs_confirmation, all_elements);
    if provider == "freedesktop" || provider == "freedesktop-then-hub" {
        let local =
            request_freedesktop_batch_confirmation(config, &preview, needs_confirmation).await;
        if local == "confirmation_provider_unavailable" && provider == "freedesktop-then-hub" {
            return request_hub_batch_confirmation(state, config, &preview, needs_confirmation)
                .await;
        }
        return local;
    }
    if provider == "hub" {
        return request_hub_batch_confirmation(state, config, &preview, needs_confirmation).await;
    }
    "confirmation_provider_unavailable".to_string()
}

async fn request_freedesktop_batch_confirmation(
    config: &Config,
    preview: &str,
    needs_confirmation: &[PreparedBatchElement],
) -> String {
    let supports_actions = tokio::task::spawn_blocking(|| {
        notify_rust::get_capabilities()
            .map(|capabilities| {
                capabilities
                    .iter()
                    .any(|capability| capability == "actions")
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !supports_actions {
        return "confirmation_provider_unavailable".to_string();
    }
    let has_risky_file_mutation = needs_confirmation
        .iter()
        .any(|element| risky_file_mutation(&element.program));
    let zh = confirmation_language_is_zh(config);
    let warning = if !config.sandbox.enabled && has_risky_file_mutation {
        if zh {
            "\n警告：bubblewrap 未启用；该批次包含文件变更命令，对宿主机的可见范围更大。"
        } else {
            "\nWARNING: bubblewrap is disabled; this batch includes file mutation commands with broader host visibility."
        }
    } else {
        ""
    };
    let body = format!("{preview}{warning}");
    let provider = notify_rust::Notification::new()
        .summary(if zh {
            "Agentic GPT 批量确认"
        } else {
            "Agentic GPT batch confirmation"
        })
        .body(&body)
        .action(
            "allow_once",
            if zh {
                "允许本批次"
            } else {
                "Allow batch once"
            },
        )
        .action("deny", if zh { "拒绝本批次" } else { "Deny batch" })
        .timeout((CONFIRM_TIMEOUT_SECS * 1000) as i32)
        .show();
    match provider {
        Ok(handle) => {
            let action = tokio::task::spawn_blocking(move || {
                let mut selected = "timeout".to_string();
                handle.wait_for_action(|action| selected = action.to_string());
                selected
            })
            .await
            .unwrap_or_else(|_| "timeout".to_string());
            if action == "allow_once" {
                action
            } else {
                "deny".to_string()
            }
        }
        Err(_) => "confirmation_provider_unavailable".to_string(),
    }
}

async fn request_hub_batch_confirmation(
    state: &AppState,
    config: &Config,
    preview: &str,
    needs_confirmation: &[PreparedBatchElement],
) -> String {
    let risk = if needs_confirmation
        .iter()
        .any(|element| risk_level(&element.program) == "HIGH")
    {
        "HIGH"
    } else {
        "MEDIUM"
    };
    let payload = ConfirmationPayload {
        program: "batchExec".to_string(),
        args: Vec::new(),
        command_preview: truncate_chars(preview, 1000),
        risk_level: risk.to_string(),
        reason: if confirmation_language_is_zh(config) {
            "批量命令中包含匹配确认策略的命令".to_string()
        } else {
            "Batch contains command(s) matching confirm policy".to_string()
        },
        kind: Some("batchExec".to_string()),
        server_id: None,
        tool_name: None,
    };
    request_hub_confirmation_payload(state, payload).await
}

async fn request_confirmation(
    state: &AppState,
    config: &Config,
    confirm_method: Option<&str>,
    program: &str,
    args: &[String],
) -> String {
    let configured_provider = config.confirmation_provider.provider.as_str();
    let provider = confirm_method
        .filter(|method| !method.trim().is_empty())
        .unwrap_or(configured_provider);
    let provider = if provider == "default" {
        configured_provider
    } else if provider == "freedesktopThenHub" {
        "freedesktop-then-hub"
    } else {
        provider
    };
    if provider == "freedesktop" || provider == "freedesktop-then-hub" {
        let local = request_freedesktop_confirmation(config, program, args).await;
        if local == "confirmation_provider_unavailable" && provider == "freedesktop-then-hub" {
            return request_hub_confirmation(state, config, program, args).await;
        }
        return local;
    }
    if provider == "hub" {
        return request_hub_confirmation(state, config, program, args).await;
    }
    "confirmation_provider_unavailable".to_string()
}

async fn request_freedesktop_confirmation(
    config: &Config,
    program: &str,
    args: &[String],
) -> String {
    let supports_actions = tokio::task::spawn_blocking(|| {
        notify_rust::get_capabilities()
            .map(|capabilities| {
                capabilities
                    .iter()
                    .any(|capability| capability == "actions")
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !supports_actions {
        return "confirmation_provider_unavailable".to_string();
    }
    let zh = confirmation_language_is_zh(config);
    let warning = if !config.sandbox.enabled && risky_file_mutation(program) {
        if zh {
            "\n警告：bubblewrap 未启用；该文件变更命令对宿主机的可见范围更大。"
        } else {
            "\nWARNING: bubblewrap is disabled; this file mutation command has broader host visibility."
        }
    } else {
        ""
    };
    let body = format!(
        "{}{}{}",
        command_preview(program, args),
        warning,
        if zh {
            "\n是否允许本次执行？"
        } else {
            "\nAllow once?"
        }
    );
    let provider = notify_rust::Notification::new()
        .summary(if zh {
            "Agentic GPT 确认"
        } else {
            "Agentic GPT confirmation"
        })
        .body(&body)
        .action("allow_once", if zh { "允许本次" } else { "Allow once" })
        .action("deny", if zh { "拒绝" } else { "Deny" })
        .timeout((CONFIRM_TIMEOUT_SECS * 1000) as i32)
        .show();
    match provider {
        Ok(handle) => {
            let action = tokio::task::spawn_blocking(move || {
                let mut selected = "timeout".to_string();
                handle.wait_for_action(|action| selected = action.to_string());
                selected
            })
            .await
            .unwrap_or_else(|_| "timeout".to_string());
            if action == "allow_once" {
                action
            } else {
                "deny".to_string()
            }
        }
        Err(_) => "confirmation_provider_unavailable".to_string(),
    }
}

async fn request_hub_confirmation(
    state: &AppState,
    _config: &Config,
    program: &str,
    args: &[String],
) -> String {
    let payload = ConfirmationPayload {
        program: program.to_string(),
        args: args.to_vec(),
        command_preview: truncate_chars(&command_preview(program, args), 1000),
        risk_level: risk_level(program),
        reason: if confirmation_language_is_zh(_config) {
            format!("命令匹配确认策略：{program}")
        } else {
            format!("Command matched confirm policy: {program}")
        },
        kind: None,
        server_id: None,
        tool_name: None,
    };
    request_hub_confirmation_payload(state, payload).await
}

async fn request_hub_confirmation_payload(
    state: &AppState,
    payload: ConfirmationPayload,
) -> String {
    let request_id = format!("confirm_req_{}", Uuid::new_v4().simple());
    let (tx, rx) = oneshot::channel();
    state
        .pending_confirmations
        .lock()
        .await
        .insert(request_id.clone(), tx);
    let config = state.config.read().await.clone();
    let message = AgentMessage::ConfirmationRequest {
        request_id: request_id.clone(),
        agent_id: config.agent_id,
        timeout_seconds: CONFIRM_TIMEOUT_SECS,
        payload,
    };
    if let Err(error) = send_agent_message(state, message).await {
        state.pending_confirmations.lock().await.remove(&request_id);
        log_warn(format!("hub confirmation unavailable: {error}"));
        return "provider_unavailable".to_string();
    }
    log_info(format!(
        "hub confirmation requested; requestId={request_id}"
    ));
    match timeout(Duration::from_secs(CONFIRM_TIMEOUT_SECS + 5), rx).await {
        Ok(Ok(decision)) => decision,
        _ => {
            state.pending_confirmations.lock().await.remove(&request_id);
            "timeout".to_string()
        }
    }
}

async fn fail_pending_confirmations(state: &AppState, reason: &str) {
    let pending = state
        .pending_confirmations
        .lock()
        .await
        .drain()
        .map(|(_, sender)| sender)
        .collect::<Vec<_>>();
    for sender in pending {
        let _ = sender.send(reason.to_string());
    }
}

pub(crate) async fn authorize_mcp_tool_call(
    state: &AppState,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    if temporary_mcp_allowed(state, server_id).await {
        return "temporary_mcp_allow".to_string();
    }

    let config = state.config.read().await.clone();
    let decision =
        request_mcp_tool_confirmation(&state, &config, server_id, tool_name, arguments).await;
    match decision.as_str() {
        "allow_mcp_server_15m" => add_temporary_mcp_allow(state, server_id, 15).await,
        "allow_mcp_server_30m" => add_temporary_mcp_allow(state, server_id, 30).await,
        _ => {}
    }
    decision
}

async fn temporary_mcp_allowed(state: &AppState, server_id: &str) -> bool {
    let agent_id = state.config.read().await.agent_id.clone();
    let now = Utc::now();
    let mut allows = state.temporary_mcp_allows.lock().await;
    allows.retain(|allow| allow.expires_at > now);
    allows
        .iter()
        .any(|allow| allow.agent_id == agent_id && allow.server_id == server_id)
}

async fn add_temporary_mcp_allow(state: &AppState, server_id: &str, minutes: i64) {
    let agent_id = state.config.read().await.agent_id.clone();
    let expires_at = Utc::now() + chrono::Duration::minutes(minutes);
    let mut allows = state.temporary_mcp_allows.lock().await;
    allows.retain(|allow| allow.expires_at > Utc::now());
    allows.retain(|allow| !(allow.agent_id == agent_id && allow.server_id == server_id));
    allows.push(TemporaryMcpAllow {
        agent_id,
        server_id: server_id.to_string(),
        expires_at,
    });
}

async fn request_mcp_tool_confirmation(
    state: &AppState,
    config: &Config,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let provider = match config.confirmation_provider.provider.as_str() {
        "default" => "freedesktop-then-hub",
        "freedesktopThenHub" => "freedesktop-then-hub",
        other => other,
    };
    if provider == "freedesktop" || provider == "freedesktop-then-hub" {
        let local =
            request_freedesktop_mcp_confirmation(config, server_id, tool_name, arguments).await;
        if local == "confirmation_provider_unavailable" && provider == "freedesktop-then-hub" {
            return request_hub_mcp_confirmation(state, config, server_id, tool_name, arguments)
                .await;
        }
        return local;
    }
    if provider == "hub" {
        return request_hub_mcp_confirmation(state, config, server_id, tool_name, arguments).await;
    }
    "confirmation_provider_unavailable".to_string()
}

async fn request_freedesktop_mcp_confirmation(
    _config: &Config,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let supports_actions = tokio::task::spawn_blocking(|| {
        notify_rust::get_capabilities()
            .map(|capabilities| {
                capabilities
                    .iter()
                    .any(|capability| capability == "actions")
            })
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false);
    if !supports_actions {
        return "confirmation_provider_unavailable".to_string();
    }
    let body = format!(
        "{}\n\nAllow once, or temporarily allow this MCP server?",
        mcp_tool_command_preview(server_id, tool_name, arguments)
    );
    let provider = notify_rust::Notification::new()
        .summary("Agentic GPT MCP confirmation")
        .body(&body)
        .action("allow_once", "Allow once")
        .action("allow_mcp_server_15m", "Allow this MCP 15m")
        .action("allow_mcp_server_30m", "Allow this MCP 30m")
        .action("deny", "Deny")
        .timeout((CONFIRM_TIMEOUT_SECS * 1000) as i32)
        .show();
    match provider {
        Ok(handle) => {
            let action = tokio::task::spawn_blocking(move || {
                let mut selected = "timeout".to_string();
                handle.wait_for_action(|action| selected = action.to_string());
                selected
            })
            .await
            .unwrap_or_else(|_| "timeout".to_string());
            match action.as_str() {
                "allow_once" | "allow_mcp_server_15m" | "allow_mcp_server_30m" => action,
                _ => "deny".to_string(),
            }
        }
        Err(_) => "confirmation_provider_unavailable".to_string(),
    }
}

async fn request_hub_mcp_confirmation(
    state: &AppState,
    config: &Config,
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let request_id = format!("confirm_req_{}", Uuid::new_v4().simple());
    let (tx, rx) = oneshot::channel();
    state
        .pending_confirmations
        .lock()
        .await
        .insert(request_id.clone(), tx);
    let payload = ConfirmationPayload {
        program: "mcpCallTool".to_string(),
        args: vec![server_id.to_string(), tool_name.to_string()],
        command_preview: mcp_tool_command_preview(server_id, tool_name, arguments),
        risk_level: "MEDIUM".to_string(),
        reason: "MCP tool call requires confirmation".to_string(),
        kind: Some("mcpTool".to_string()),
        server_id: Some(server_id.to_string()),
        tool_name: Some(tool_name.to_string()),
    };
    let message = AgentMessage::ConfirmationRequest {
        request_id: request_id.clone(),
        agent_id: config.agent_id.clone(),
        timeout_seconds: CONFIRM_TIMEOUT_SECS,
        payload,
    };
    if let Err(error) = send_agent_message(state, message).await {
        state.pending_confirmations.lock().await.remove(&request_id);
        log_warn(format!("hub MCP confirmation unavailable: {error}"));
        return "provider_unavailable".to_string();
    }
    log_info(format!(
        "hub MCP confirmation requested; requestId={request_id}; serverId={server_id}; toolName={tool_name}"
    ));
    match timeout(Duration::from_secs(CONFIRM_TIMEOUT_SECS + 5), rx).await {
        Ok(Ok(decision)) => decision,
        _ => {
            state.pending_confirmations.lock().await.remove(&request_id);
            "timeout".to_string()
        }
    }
}

fn mcp_tool_command_preview(
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let arguments = serde_json::to_string_pretty(arguments)
        .unwrap_or_else(|_| serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string()));
    format!(
        "MCP Tool Call\nServer: {server_id}\nTool: {tool_name}\nArguments:\n{}",
        truncate_chars(&arguments, 2000)
    )
}

fn confirmation_decision_value(decision: ConfirmationDecision) -> String {
    match decision {
        ConfirmationDecision::AllowOnce => "allow_once",
        ConfirmationDecision::AllowMcpServer15m => "allow_mcp_server_15m",
        ConfirmationDecision::AllowMcpServer30m => "allow_mcp_server_30m",
        ConfirmationDecision::Deny => "deny",
        ConfirmationDecision::Timeout => "timeout",
        ConfirmationDecision::ProviderUnavailable => "provider_unavailable",
        ConfirmationDecision::CallbackTokenInvalid => "callback_token_invalid",
        ConfirmationDecision::Expired => "expired",
    }
    .to_string()
}

fn risk_level(program: &str) -> String {
    if risky_file_mutation(program) {
        "HIGH"
    } else if matches!(
        program,
        "curl" | "wget" | "docker" | "systemctl" | "service"
    ) {
        "MEDIUM"
    } else {
        "LOW"
    }
    .to_string()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn risky_file_mutation(program: &str) -> bool {
    matches!(
        program,
        "rm" | "mv" | "chmod" | "chown" | "git" | "python" | "node"
    )
}

fn policy_decision(
    config: &Config,
    program: &str,
    args: &[String],
    need_confirm: bool,
) -> PolicyDecision {
    policy_decision_for_mode(config, RunMode::Normal, program, args, need_confirm)
}

fn policy_decision_for_mode(
    config: &Config,
    run_mode: RunMode,
    program: &str,
    args: &[String],
    need_confirm: bool,
) -> PolicyDecision {
    let mut decision = if need_confirm {
        PolicyDecision::Confirm
    } else {
        PolicyDecision::Allow
    };
    for rule in builtin_rules(run_mode, PolicyDecision::Confirm) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Confirm);
        }
    }
    for rule in builtin_rules(run_mode, PolicyDecision::Deny) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Deny);
        }
    }

    let mut configured_decision = None;
    for rule in &config.policy.allow {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Allow);
        }
    }
    for rule in &config.policy.confirm {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Confirm);
        }
    }
    for rule in &config.policy.deny {
        if rule.matches(program, args) {
            configured_decision = Some(PolicyDecision::Deny);
        }
    }

    configured_decision.unwrap_or(decision)
}

impl Rule {
    fn matches(&self, program: &str, args: &[String]) -> bool {
        self.program == program
            && args.len() >= self.args_prefix.len()
            && self
                .args_prefix
                .iter()
                .zip(args.iter())
                .all(|(expected, actual)| expected == actual)
    }
}

fn builtin_rules(run_mode: RunMode, decision: PolicyDecision) -> Vec<Rule> {
    let programs = match decision {
        PolicyDecision::Deny => vec!["su", "mkfs", "dd", "ssh"],
        PolicyDecision::Confirm if run_mode == RunMode::Room => {
            vec!["sudo", "mount", "systemctl", "service", "scp"]
        }
        PolicyDecision::Confirm => vec![
            "sudo",
            "rm",
            "mv",
            "chmod",
            "chown",
            "mount",
            "systemctl",
            "service",
            "docker",
            "scp",
            "curl",
            "wget",
            "bash",
            "sh",
            "zsh",
            "fish",
            "perl",
            "ruby",
        ],
        PolicyDecision::Allow => vec![],
    };
    let mut rules = programs
        .into_iter()
        .map(|program| Rule {
            program: program.to_string(),
            args_prefix: vec![],
        })
        .collect::<Vec<_>>();
    if decision == PolicyDecision::Confirm && run_mode == RunMode::Normal {
        rules.push(Rule {
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        });
        rules.push(Rule {
            program: "node".to_string(),
            args_prefix: vec!["-e".to_string()],
        });
    }
    rules
}

async fn start_session(state: AppState, session_id: String, request: ExecRequest) -> SessionInfo {
    let config = state.config.read().await.clone();
    let started_at = Utc::now();
    let mut info = SessionInfo {
        agent_id: request.agent_id.clone(),
        session_id: session_id.clone(),
        state: "running".to_string(),
        program: request.program.clone(),
        args: request.args.clone(),
        working_directory: request.working_directory.clone(),
        command_preview: command_preview(&request.program, &request.args),
        started_at,
        updated_at: started_at,
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
    };
    let decision = policy_decision_for_mode(
        &config,
        state.run_mode,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    let working_directory =
        match resolve_working_directory(&config, request.working_directory.as_deref()) {
            Ok(working_directory) => working_directory,
            Err(reason) => {
                info.state = "failed".to_string();
                info.reject_reason = Some(reason);
                return info;
            }
        };
    if let Err(reason) = preflight(&config, &working_directory, &request.program, &request.args) {
        info.state = "failed".to_string();
        info.reject_reason = Some(reason);
        return info;
    }
    if decision == PolicyDecision::Deny {
        info.state = "failed".to_string();
        info.reject_reason = Some("policy_denied".to_string());
        return info;
    }
    if decision == PolicyDecision::Confirm {
        let confirmation = request_confirmation(
            &state,
            &config,
            request.confirm_method.as_deref(),
            &request.program,
            &request.args,
        )
        .await;
        if confirmation != "allow_once" {
            info.state = "failed".to_string();
            info.reject_reason = Some(confirmation);
            return info;
        }
    }
    if state.sessions.lock().await.len() >= config.limits.max_active_sessions {
        info.state = "failed".to_string();
        info.reject_reason = Some("max_active_sessions_reached".to_string());
        return info;
    }
    match spawn_session(&config, &working_directory, &request.program, &request.args).await {
        Ok((child, stdout, stderr)) => {
            state.sessions.lock().await.insert(
                session_id,
                ManagedSession {
                    info: info.clone(),
                    child,
                    stdout,
                    stderr,
                    last_activity: Instant::now(),
                },
            );
        }
        Err(error) => {
            info.state = "failed".to_string();
            info.reject_reason = Some(format!("spawn_failed: {error}"));
        }
    }
    info
}

async fn spawn_session(
    config: &Config,
    working_directory: &Path,
    program: &str,
    args: &[String],
) -> Result<(Child, Arc<Mutex<TailBuffer>>, Arc<Mutex<TailBuffer>>)> {
    let mut command = build_command(config, working_directory, program)?;
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    let stderr = Arc::new(Mutex::new(TailBuffer::new(SESSION_TAIL_MAX)));
    if let Some(out) = child.stdout.take() {
        tokio::spawn(read_tail(out, stdout.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(read_tail(err, stderr.clone()));
    }
    Ok((child, stdout, stderr))
}

async fn read_tail<R: AsyncRead + Unpin>(
    mut reader: R,
    tail: Arc<Mutex<TailBuffer>>,
) -> Result<()> {
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        tail.lock().await.push(&buffer[..read]);
    }
    Ok(())
}

async fn current_sessions(state: &AppState) -> Vec<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let mut result = Vec::new();
    for session in sessions.values_mut() {
        refresh_session(session).await;
        if matches!(
            session.info.state.as_str(),
            "running" | "waiting_confirmation"
        ) {
            result.push(session.info.clone());
        }
    }
    result
}

async fn inspect_session(state: &AppState, session_id: &str) -> Option<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    refresh_session(session).await;
    Some(session.info.clone())
}

async fn kill_session(state: &AppState, session_id: &str) -> Option<SessionInfo> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions.get_mut(session_id)?;
    let _ = session.child.kill().await;
    refresh_session(session).await;
    session.info.state = "killed".to_string();
    session.info.updated_at = Utc::now();
    Some(session.info.clone())
}

async fn refresh_session(session: &mut ManagedSession) {
    if let Ok(Some(status)) = session.child.try_wait() {
        session.info.exit_code = status.code();
        session.info.state = if status.success() { "exited" } else { "failed" }.to_string();
    }
    let stdout = session.stdout.lock().await;
    let stderr = session.stderr.lock().await;
    session.info.stdout_tail = stdout.text();
    session.info.stderr_tail = stderr.text();
    session.info.truncated = stdout.truncated || stderr.truncated;
    session.info.updated_at = Utc::now();
    session.last_activity = Instant::now();
}

async fn handle_config(config_path: PathBuf, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Init => {
            let config = Config::default_config()?;
            write_config_with_backup(&config_path, &config)?;
            println!("initialized {}", config_path.display());
        }
        ConfigCommand::Show => {
            let config = Config::load(&config_path)?;
            println!("{}", serde_json::to_string_pretty(&config)?);
        }
        ConfigCommand::Set { key, value } => {
            let mut config = Config::load_or_default(&config_path)?;
            match key.as_str() {
                "workspaceRoot" => {
                    let old_workspace = config.workspace_root.clone();
                    let new_workspace = PathBuf::from(value);
                    for root in &mut config.path_policy.write_roots {
                        if paths_match(root, &old_workspace) {
                            *root = new_workspace.clone();
                        }
                    }
                    config.workspace_root = new_workspace;
                }
                "confirmationProvider" => config.confirmation_provider.provider = value,
                "confirmationLanguage" => {
                    config.confirmation_language = normalize_confirmation_language(&value)
                }
                "sandbox.enabled" => config.sandbox.enabled = value.parse::<bool>()?,
                "room.notebookRoot" => config.room.notebook_root = Some(PathBuf::from(value)),
                "room.timezone" => config.room.timezone = value,
                "hubUrl" | "workerUrl" => config.hub_url = value,
                "agentId" => config.agent_id = value,
                "agentSecret" => config.agent_secret = value,
                _ => return Err(anyhow!("unsupported config key: {key}")),
            }
            write_config_with_backup(&config_path, &config)?;
        }
        ConfigCommand::Allow { command } => {
            mutate_rule(config_path, PolicyDecision::Allow, command)?
        }
        ConfigCommand::Confirm { command } => {
            mutate_rule(config_path, PolicyDecision::Confirm, command)?
        }
        ConfigCommand::Deny { command } => mutate_rule(config_path, PolicyDecision::Deny, command)?,
        ConfigCommand::Path { command } => mutate_path_policy(config_path, command)?,
        ConfigCommand::Mcp { command } => mcp::mutate_servers(config_path, command)?,
    }
    Ok(())
}

fn mutate_rule(config_path: PathBuf, decision: PolicyDecision, command: RuleCommand) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
    let rules = match decision {
        PolicyDecision::Allow => &mut config.policy.allow,
        PolicyDecision::Confirm => &mut config.policy.confirm,
        PolicyDecision::Deny => &mut config.policy.deny,
    };
    match command {
        RuleCommand::Add {
            program,
            args_prefix,
        } => {
            let rule = Rule {
                program,
                args_prefix,
            };
            println!("added {}", rule_display(&rule));
            rules.push(rule);
        }
        RuleCommand::Remove {
            program,
            args_prefix,
        } => {
            remove_rule(rules, &program, &args_prefix)?;
        }
    }
    write_config_with_backup(&config_path, &config)
}

fn remove_rule(rules: &mut Vec<Rule>, program: &str, args_prefix: &[String]) -> Result<()> {
    let matches = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule.program == program && rule.args_prefix == args_prefix)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(anyhow!(
            "rule not found: {}",
            command_preview(program, args_prefix)
        )),
        1 => {
            let removed = rules.remove(matches[0]);
            println!("removed {}", rule_display(&removed));
            Ok(())
        }
        _ if io::stdin().is_terminal() => {
            let selected = choose_rule_interactively(rules, &matches)?;
            let removed = rules.remove(selected);
            println!("removed {}", rule_display(&removed));
            Ok(())
        }
        _ => {
            eprintln!(
                "multiple rules match {}; rerun in an interactive terminal or provide a more specific args prefix:",
                command_preview(program, args_prefix)
            );
            for index in matches {
                eprintln!("  {}", rule_display(&rules[index]));
            }
            Err(anyhow!("multiple_matching_rules"))
        }
    }
}

fn choose_rule_interactively(rules: &[Rule], matches: &[usize]) -> Result<usize> {
    println!("multiple matching rules:");
    for (ordinal, index) in matches.iter().enumerate() {
        println!("  {}) {}", ordinal + 1, rule_display(&rules[*index]));
    }
    print!("select rule to remove [1-{}]: ", matches.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let selected = input
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow!("invalid selection"))?;
    if selected == 0 || selected > matches.len() {
        return Err(anyhow!("selection out of range"));
    }
    Ok(matches[selected - 1])
}

fn rule_display(rule: &Rule) -> String {
    command_preview(&rule.program, &rule.args_prefix)
}

fn mutate_path_policy(config_path: PathBuf, command: PathCommand) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
    match command {
        PathCommand::List => {
            println!("{}", serde_json::to_string_pretty(&config.path_policy)?);
            return Ok(());
        }
        PathCommand::Write { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Write, command)
        }
        PathCommand::Readonly { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Readonly, command)
        }
        PathCommand::Deny { command } => {
            mutate_path_roots(&mut config.path_policy, PathRootKind::Deny, command)
        }
    }
    write_config_with_backup(&config_path, &config)
}

fn mutate_path_roots(policy: &mut PathPolicyConfig, kind: PathRootKind, command: PathRootCommand) {
    match command {
        PathRootCommand::Add { path } => {
            let roots = roots_for_kind(policy, kind);
            if !roots.iter().any(|existing| paths_match(existing, &path)) {
                roots.push(path);
            }
        }
        PathRootCommand::Remove { path } => {
            let roots = roots_for_kind(policy, kind);
            roots.retain(|existing| !paths_match(existing, &path));
        }
    }
}

fn roots_for_kind(policy: &mut PathPolicyConfig, kind: PathRootKind) -> &mut Vec<PathBuf> {
    match kind {
        PathRootKind::Write => &mut policy.write_roots,
        PathRootKind::Readonly => &mut policy.read_only_roots,
        PathRootKind::Deny => &mut policy.deny_roots,
    }
}

fn paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (
        expand_pathbuf(left).and_then(|path| canonicalize_existing_or_parent(&path)),
        expand_pathbuf(right).and_then(|path| canonicalize_existing_or_parent(&path)),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

async fn watch_config(state: AppState) {
    let mut last_modified = fs::metadata(&state.config_path)
        .and_then(|meta| meta.modified())
        .ok();
    loop {
        sleep(Duration::from_secs(2)).await;
        let modified = fs::metadata(&state.config_path)
            .and_then(|meta| meta.modified())
            .ok();
        if modified.is_some() && modified != last_modified {
            if let Ok(config) = Config::load(&state.config_path) {
                let _ = config.ensure_workspace();
                log_info(format!(
                    "config reloaded; agentId={}; workspaceRoot={}; sandbox={}",
                    config.agent_id,
                    config.workspace_root.display(),
                    if config.sandbox.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
                *state.config.write().await = config;
                last_modified = modified;
            } else {
                log_warn("config reload failed; keeping previous config".to_string());
            }
        }
    }
}

fn default_confirmation_language() -> String {
    "en".to_string()
}

fn normalize_confirmation_language(language: &str) -> String {
    let language = language.trim();
    if language.eq_ignore_ascii_case("zh")
        || language.eq_ignore_ascii_case("zh-cn")
        || language.eq_ignore_ascii_case("zh_cn")
        || language.eq_ignore_ascii_case("cn")
    {
        "zh-CN".to_string()
    } else {
        "en".to_string()
    }
}

fn confirmation_language_is_zh(config: &Config) -> bool {
    normalize_confirmation_language(&config.confirmation_language) == "zh-CN"
}

impl Config {
    fn default_config() -> Result<Self> {
        let base = agentic_home()?;
        Ok(Self {
            agent_id: "laptop".to_string(),
            display_name: hostname_fallback(),
            hub_url: "http://localhost:8787".to_string(),
            agent_secret: "change-me".to_string(),
            workspace_root: base.join("workspace"),
            backup_limit: DEFAULT_BACKUP_LIMIT,
            confirmation_provider: ConfirmationProviderConfig {
                provider: "freedesktop-then-hub".to_string(),
            },
            confirmation_language: default_confirmation_language(),
            mcp_servers: BTreeMap::new(),
            sandbox: SandboxConfig {
                enabled: false,
                bubblewrap_path: "bwrap".to_string(),
                required_runtime_paths: vec![
                    PathBuf::from("/usr"),
                    PathBuf::from("/bin"),
                    PathBuf::from("/lib"),
                    PathBuf::from("/lib64"),
                    PathBuf::from("/etc/ssl"),
                ],
            },
            path_policy: default_path_policy(&base.join("workspace")),
            policy: PolicyConfig::default(),
            limits: LimitsConfig {
                max_concurrent_tasks: 2,
                max_active_sessions: 4,
                session_idle_timeout_secs: 3600,
            },
            room: default_room_config(),
        })
    }

    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let has_path_policy = value.get("pathPolicy").is_some();
        let mut config: Self = serde_json::from_value(value)?;
        if !has_path_policy {
            config.path_policy = default_path_policy(&config.workspace_root);
        }
        Ok(config)
    }

    fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Self::default_config()
        }
    }

    fn ensure_workspace(&self) -> Result<()> {
        fs::create_dir_all(&self.workspace_root)?;
        Ok(())
    }

    fn safe_summary(&self) -> SafeConfigSummary {
        let write_roots = safe_write_roots(self);
        SafeConfigSummary {
            workspace_root: workspace_root_summary(&self.workspace_root),
            sandbox: SafeSandboxSummary {
                enabled: self.sandbox.enabled,
                mode: if self.sandbox.enabled {
                    "bubblewrap"
                } else {
                    "disabled"
                }
                .to_string(),
            },
            path_policy: SafePathPolicySummary {
                write_root_count: write_roots.len(),
                read_only_root_count: self.path_policy.read_only_roots.len(),
                deny_root_count: self.path_policy.deny_roots.len(),
                write_roots,
                read_only_roots: safe_path_roots(
                    &self.path_policy.read_only_roots,
                    &self.workspace_root,
                    "configured",
                ),
                deny_roots: safe_path_roots(
                    &self.path_policy.deny_roots,
                    &self.workspace_root,
                    "configured",
                ),
            },
            policy_rule_counts: PolicyCounts {
                allow: self.policy.allow.len(),
                confirm: self.policy.confirm.len(),
                deny: self.policy.deny.len(),
            },
            policy_rules: SafePolicyRules {
                allow: safe_rules(&self.policy.allow),
                confirm: safe_rules(&self.policy.confirm),
                deny: safe_rules(&self.policy.deny),
                builtins: SafeBuiltinPolicyRules {
                    confirm: safe_rules(&builtin_rules(RunMode::Normal, PolicyDecision::Confirm)),
                    deny: safe_rules(&builtin_rules(RunMode::Normal, PolicyDecision::Deny)),
                },
            },
            confirmation_provider: self.confirmation_provider.provider.clone(),
        }
    }
}

fn workspace_root_summary(workspace_root: &Path) -> String {
    if *workspace_root == agentic_home().unwrap_or_default().join("workspace") {
        "default".to_string()
    } else {
        workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("workspace:{name}"))
            .unwrap_or_else(|| "configured".to_string())
    }
}

fn safe_write_roots(config: &Config) -> Vec<SafePathRoot> {
    let mut has_workspace_root = false;
    let mut roots = config
        .path_policy
        .write_roots
        .iter()
        .map(|root| {
            let is_workspace_root = paths_match(root, &config.workspace_root);
            has_workspace_root |= is_workspace_root;
            SafePathRoot {
                path: safe_path_display(root, &config.workspace_root),
                source: if is_workspace_root {
                    "workspaceRoot"
                } else {
                    "configured"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    if !has_workspace_root {
        roots.insert(
            0,
            SafePathRoot {
                path: "workspace".to_string(),
                source: "workspaceRoot".to_string(),
            },
        );
    }
    roots
}

fn safe_path_roots(roots: &[PathBuf], workspace_root: &Path, source: &str) -> Vec<SafePathRoot> {
    roots
        .iter()
        .map(|root| SafePathRoot {
            path: safe_path_display(root, workspace_root),
            source: source.to_string(),
        })
        .collect()
}

fn safe_path_display(path: &Path, workspace_root: &Path) -> String {
    if paths_match(path, workspace_root) {
        return "workspace".to_string();
    }
    let raw = path.to_string_lossy().to_string();
    if raw == "~" || raw.starts_with("~/") {
        return raw;
    }
    if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = path.strip_prefix(&home) {
            if relative.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", relative.to_string_lossy());
        }
    }
    raw
}

fn safe_rules(rules: &[Rule]) -> Vec<SafeRule> {
    rules
        .iter()
        .map(|rule| SafeRule {
            program: rule.program.clone(),
            args_prefix: rule.args_prefix.clone(),
        })
        .collect()
}

fn default_room_config() -> RoomConfig {
    RoomConfig {
        notebook_root: None,
        timezone: "Asia/Shanghai".to_string(),
    }
}

fn default_path_policy(workspace_root: &Path) -> PathPolicyConfig {
    PathPolicyConfig {
        write_roots: vec![
            workspace_root.to_path_buf(),
            PathBuf::from("~/Documents"),
            PathBuf::from("~/Downloads"),
            PathBuf::from("/tmp"),
        ],
        read_only_roots: vec![
            PathBuf::from("~/.cache"),
            PathBuf::from("~/.local/share"),
            PathBuf::from("/etc/os-release"),
            PathBuf::from("/proc/meminfo"),
            PathBuf::from("/proc/cpuinfo"),
            PathBuf::from("/proc/loadavg"),
            PathBuf::from("/proc/uptime"),
            PathBuf::from("/sys/class/power_supply"),
            PathBuf::from("/sys/class/thermal"),
        ],
        deny_roots: vec![
            PathBuf::from("~/.ssh"),
            PathBuf::from("~/.gnupg"),
            PathBuf::from("~/.local/share/keyrings"),
            PathBuf::from("~/.password-store"),
            PathBuf::from("~/.mozilla"),
            PathBuf::from("~/.config/google-chrome"),
            PathBuf::from("~/.config/chromium"),
            PathBuf::from("~/.config/Code/User/globalStorage"),
            PathBuf::from("~/.npmrc"),
            PathBuf::from("~/.pypirc"),
            PathBuf::from("~/.cargo/credentials"),
            PathBuf::from("~/.docker/config.json"),
            PathBuf::from("~/.kube"),
            PathBuf::from("~/.aws"),
            PathBuf::from("~/.config/gcloud"),
            PathBuf::from("~/.config/gh"),
            PathBuf::from("~/.config/hub"),
            PathBuf::from("~/.config/clash"),
            PathBuf::from("~/.config/clash-verge"),
        ],
    }
}

fn write_config_with_backup(path: &Path, config: &Config) -> Result<()> {
    ensure_parent(path)?;
    if path.exists() {
        let backup_dir = path.parent().unwrap().join("backups");
        fs::create_dir_all(&backup_dir)?;
        let backup = backup_dir.join(format!("config.{}.json", Utc::now().timestamp_millis()));
        fs::copy(path, backup)?;
        prune_backups(&backup_dir, config.backup_limit)?;
    }
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

fn prune_backups(dir: &Path, limit: usize) -> Result<()> {
    let mut entries = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("config."))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok());
    while entries.len() > limit {
        if let Some(entry) = entries.first() {
            fs::remove_file(entry.path())?;
        }
        entries.remove(0);
    }
    Ok(())
}

fn write_audit(config: &Config, record: AuditRecord) -> Result<()> {
    let path = agentic_home()?.join("audit.log");
    ensure_parent(&path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    let _ = config;
    Ok(())
}

fn config_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        agentic_home()
            .unwrap_or_else(|_| PathBuf::from(".agentic_gpt"))
            .join("config.json")
    })
}

fn agentic_home() -> Result<PathBuf> {
    let home = dirs::home_dir().context("home directory not found")?;
    Ok(home.join(".agentic_gpt"))
}

fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn hostname_fallback() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "agentic-gpt-linux".to_string())
}

fn command_preview(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .map(|part| {
            if part.contains(char::is_whitespace) {
                format!("{part:?}")
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn log_info(message: String) {
    log_line("INFO", message);
}

fn log_warn(message: String) {
    log_line("WARN", message);
}

fn log_line(level: &str, message: String) {
    eprintln!("{} {level} {message}", Utc::now().to_rfc3339());
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_gpt_protocol::{
        NotebookAppendRequest, NotebookRemoveRequest, NotebookUpdateRequest, PassageSignificance,
    };

    #[test]
    fn run_modes_declare_expected_roles() {
        assert_eq!(RunMode::Normal.role(), AgentRole::Normal);
        assert_eq!(RunMode::Room.role(), AgentRole::Room);
    }

    #[test]
    fn run_as_room_reuses_same_base_config_identity_and_workspace() {
        let config = Config::default_config().unwrap();
        assert_eq!(config.agent_id, "laptop");
        assert_eq!(
            notebook::notebook_root(&config),
            config.workspace_root.join("notebook")
        );
    }

    #[test]
    fn configured_room_notebook_root_overrides_default() {
        let mut config = Config::default_config().unwrap();
        let root = unique_temp_dir("configured-notebook-root");
        config.room.notebook_root = Some(root.clone());
        assert_eq!(notebook::notebook_root(&config), root);
    }

    #[test]
    fn room_timezone_defaults_and_can_be_overridden() {
        let mut config = Config::default_config().unwrap();
        assert_eq!(config.room.timezone, "Asia/Shanghai");
        config.room.timezone = "UTC".to_string();
        assert_eq!(config.room.timezone, "UTC");
    }

    #[test]
    fn normal_mode_room_command_error_is_structured() {
        let value = room_agent_required_error();
        assert_eq!(value["error"]["code"], "room_agent_required");
        assert_eq!(
            value["error"]["message"],
            "room notebook commands require run-as-room"
        );
    }

    fn command_test_state(
        run_mode: RunMode,
        workspace_root: PathBuf,
    ) -> (AppState, mpsc::UnboundedReceiver<Message>) {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace_root;
        let (tx, rx) = mpsc::unbounded_channel();
        (
            AppState {
                config_path: PathBuf::from("test-config.json"),
                config: Arc::new(RwLock::new(config)),
                run_mode,
                sessions: Arc::new(Mutex::new(HashMap::new())),
                hub_sender: Arc::new(Mutex::new(Some(tx))),
                pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
                temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
                notebook_writes: Arc::new(Mutex::new(())),
            },
            rx,
        )
    }

    async fn recv_response(rx: &mut mpsc::UnboundedReceiver<Message>) -> serde_json::Value {
        let Message::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text response");
        };
        let message = serde_json::from_str::<AgentMessage>(&text).unwrap();
        let AgentMessage::Response { data, .. } = message else {
            panic!("expected agent response");
        };
        data
    }

    #[tokio::test]
    async fn normal_mode_rejects_update_and_remove_room_commands() {
        let workspace = unique_temp_dir("normal-room-update-remove").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(RunMode::Normal, workspace);
        handle_hub_command(
            state.clone(),
            HubCommand::RoomNotebookUpdate {
                request_id: "req-update".to_string(),
                payload: NotebookUpdateRequest {
                    id: "psg_1".to_string(),
                    significance: None,
                    abstract_text: Some("updated".to_string()),
                    content: None,
                    tags: None,
                },
            },
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["error"]["code"], "room_agent_required");

        handle_hub_command(
            state,
            HubCommand::RoomNotebookRemove {
                request_id: "req-remove".to_string(),
                payload: NotebookRemoveRequest {
                    id: "psg_1".to_string(),
                },
            },
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["error"]["code"], "room_agent_required");
    }

    #[tokio::test]
    async fn room_mode_executes_update_and_remove_room_commands() {
        let workspace = unique_temp_dir("room-update-remove").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (state, mut rx) = command_test_state(RunMode::Room, workspace);
        let appended = notebook::append(
            &state,
            NotebookAppendRequest {
                datetime: None,
                scope: "agentic".to_string(),
                significance: PassageSignificance::Anchor,
                abstract_text: "original".to_string(),
                content: "details".to_string(),
                tags: vec![],
            },
        )
        .await
        .unwrap();
        handle_hub_command(
            state.clone(),
            HubCommand::RoomNotebookUpdate {
                request_id: "req-update".to_string(),
                payload: NotebookUpdateRequest {
                    id: appended.id.clone(),
                    significance: None,
                    abstract_text: Some("updated".to_string()),
                    content: Some("updated details".to_string()),
                    tags: Some(vec!["tag".to_string()]),
                },
            },
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["updated"], true);
        assert_eq!(response["id"], appended.id);

        handle_hub_command(
            state,
            HubCommand::RoomNotebookRemove {
                request_id: "req-remove".to_string(),
                payload: NotebookRemoveRequest { id: appended.id },
            },
        )
        .await
        .unwrap();
        let response = recv_response(&mut rx).await;
        assert_eq!(response["removed"], true);
    }

    #[test]
    fn room_policy_overlay_differs_from_normal_policy() {
        let config = Config::default_config().unwrap();
        assert_eq!(
            policy_decision_for_mode(&config, RunMode::Normal, "rm", &[], false),
            PolicyDecision::Confirm
        );
        assert_eq!(
            policy_decision_for_mode(&config, RunMode::Room, "rm", &[], false),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn room_policy_keeps_high_risk_commands_restricted() {
        let config = Config::default_config().unwrap();
        for program in ["sudo", "scp", "mount", "systemctl", "service"] {
            assert_eq!(
                policy_decision_for_mode(&config, RunMode::Room, program, &[], false),
                PolicyDecision::Confirm
            );
        }
        assert_eq!(
            policy_decision_for_mode(&config, RunMode::Room, "ssh", &[], false),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn rule_matches_program_and_args_prefix_structurally() {
        let rule = Rule {
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        };
        assert!(rule.matches("python", &["-c".to_string(), "print(1)".to_string()]));
        assert!(!rule.matches("python3", &["-c".to_string()]));
        assert!(!rule.matches("python", &["script.py".to_string()]));
    }

    #[test]
    fn safe_summary_includes_path_roots_and_policy_rules() {
        let root = unique_temp_dir("safe-summary");
        let workspace = root.join("workspace");
        let write_root = root.join("write");
        let read_only_root = root.join("readonly");
        let deny_root = root.join("deny");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&write_root).unwrap();
        fs::create_dir_all(&read_only_root).unwrap();
        fs::create_dir_all(&deny_root).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: vec![write_root.clone()],
            read_only_roots: vec![read_only_root.clone()],
            deny_roots: vec![deny_root.clone()],
        };
        config.policy.allow.push(Rule {
            program: "git".to_string(),
            args_prefix: vec!["status".to_string()],
        });
        config.policy.confirm.push(Rule {
            program: "bash".to_string(),
            args_prefix: vec!["-lc".to_string()],
        });
        config.policy.deny.push(Rule {
            program: "rm".to_string(),
            args_prefix: vec!["-rf".to_string()],
        });

        let summary = config.safe_summary();
        assert_eq!(summary.path_policy.write_root_count, 2);
        assert_eq!(summary.path_policy.read_only_root_count, 1);
        assert_eq!(summary.path_policy.deny_root_count, 1);
        assert!(summary
            .path_policy
            .write_roots
            .iter()
            .any(|root| root.path == "workspace" && root.source == "workspaceRoot"));
        assert!(summary
            .path_policy
            .write_roots
            .iter()
            .any(|root| root.path.ends_with("/write") && root.source == "configured"));
        assert!(summary
            .path_policy
            .read_only_roots
            .iter()
            .any(|root| root.path.ends_with("/readonly") && root.source == "configured"));
        assert!(summary
            .path_policy
            .deny_roots
            .iter()
            .any(|root| root.path.ends_with("/deny") && root.source == "configured"));

        assert_eq!(summary.policy_rule_counts.allow, 1);
        assert_eq!(summary.policy_rule_counts.confirm, 1);
        assert_eq!(summary.policy_rule_counts.deny, 1);
        assert!(summary.policy_rules.allow.iter().any(|rule| {
            rule.program == "git" && rule.args_prefix == vec!["status".to_string()]
        }));
        assert!(summary
            .policy_rules
            .confirm
            .iter()
            .any(|rule| { rule.program == "bash" && rule.args_prefix == vec!["-lc".to_string()] }));
        assert!(summary
            .policy_rules
            .deny
            .iter()
            .any(|rule| { rule.program == "rm" && rule.args_prefix == vec!["-rf".to_string()] }));
        assert!(summary
            .policy_rules
            .builtins
            .confirm
            .iter()
            .any(|rule| rule.program == "bash" && rule.args_prefix.is_empty()));
        assert!(summary
            .policy_rules
            .builtins
            .confirm
            .iter()
            .any(|rule| rule.program == "python" && rule.args_prefix == vec!["-c".to_string()]));
        assert!(summary
            .policy_rules
            .builtins
            .deny
            .iter()
            .any(|rule| rule.program == "ssh" && rule.args_prefix.is_empty()));
    }

    #[test]
    fn configured_allow_overrides_need_confirm() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "git".to_string(),
            args_prefix: vec!["status".to_string()],
        });
        assert_eq!(
            policy_decision(&config, "git", &["status".to_string()], true),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn configured_allow_overrides_builtin_confirm() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "curl".to_string(),
            args_prefix: vec!["--version".to_string()],
        });
        assert_eq!(
            policy_decision(&config, "curl", &["--version".to_string()], false),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn configured_allow_overrides_builtin_deny() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "ssh".to_string(),
            args_prefix: vec!["-V".to_string()],
        });
        assert_eq!(
            policy_decision(&config, "ssh", &["-V".to_string()], false),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn configured_deny_wins_when_multiple_config_rules_match() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            program: "git".to_string(),
            args_prefix: vec![],
        });
        config.policy.deny.push(Rule {
            program: "git".to_string(),
            args_prefix: vec!["push".to_string()],
        });
        assert_eq!(
            policy_decision(&config, "git", &["push".to_string()], false),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn sudo_requires_credentials() {
        let config = Config::default_config().unwrap();
        assert_eq!(
            preflight(
                &config,
                &config.workspace_root,
                "sudo",
                &["true".to_string()]
            )
            .unwrap_err(),
            "interactive_credential_required"
        );
    }

    #[test]
    fn read_only_system_file_is_allowed() {
        let config = Config::default_config().unwrap();
        assert!(preflight(
            &config,
            &config.workspace_root,
            "cat",
            &["/proc/meminfo".to_string()]
        )
        .is_ok());
        assert!(preflight(&config, &config.workspace_root, "df", &["/".to_string()]).is_ok());
    }

    #[test]
    fn path_policy_allows_write_root_and_blocks_readonly_write() {
        let root = unique_temp_dir("path-policy");
        let workspace = root.join("workspace");
        let downloads = root.join("Downloads");
        let cache = root.join(".cache");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&downloads).unwrap();
        fs::create_dir_all(&cache).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: vec![downloads.clone()],
            read_only_roots: vec![cache.clone()],
            deny_roots: Vec::new(),
        };

        assert!(preflight(
            &config,
            &config.workspace_root,
            "touch",
            &[downloads.join("test-file").to_string_lossy().to_string()]
        )
        .is_ok());
        assert_eq!(
            preflight(
                &config,
                &config.workspace_root,
                "touch",
                &[cache.join("test-file").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_readonly"
        );
        assert!(preflight(
            &config,
            &config.workspace_root,
            "du",
            &[cache.to_string_lossy().to_string()]
        )
        .is_ok());
    }

    #[test]
    fn deny_roots_override_read_and_write() {
        let root = unique_temp_dir("path-deny");
        let workspace = root.join("workspace");
        let downloads = root.join("Downloads");
        let secret = downloads.join("secret");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&secret).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: vec![downloads.clone()],
            read_only_roots: Vec::new(),
            deny_roots: vec![secret.clone()],
        };

        assert_eq!(
            preflight(
                &config,
                &config.workspace_root,
                "cat",
                &[secret.join("token").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_denied"
        );
        assert_eq!(
            preflight(
                &config,
                &config.workspace_root,
                "rm",
                &[secret.join("token").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_denied"
        );
    }

    #[test]
    fn unknown_program_defaults_to_write_access() {
        let root = unique_temp_dir("path-unknown");
        let workspace = root.join("workspace");
        let cache = root.join(".cache");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&cache).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: vec![cache.clone()],
            deny_roots: Vec::new(),
        };

        assert_eq!(
            preflight(
                &config,
                &config.workspace_root,
                "custom-tool",
                &[cache.join("file").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_readonly"
        );
    }

    #[tokio::test]
    async fn batch_rejects_entire_group_when_confirmation_is_unavailable() {
        let root = unique_temp_dir("batch-confirm-unavailable");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("marker.txt"), "untouched").unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.confirmation_provider.provider = "none".to_string();
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: Vec::new(),
        };

        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(RwLock::new(config)),
            run_mode: RunMode::Normal,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
        };

        let result = run_batch_task(
            state,
            "batch_test".to_string(),
            BatchExecRequest {
                agent_id: "test-agent".to_string(),
                elements: vec![
                    ExecElement {
                        program: "pwd".to_string(),
                        args: Vec::new(),
                        working_directory: None,
                    },
                    ExecElement {
                        program: "bash".to_string(),
                        args: vec!["-lc".to_string(), "echo changed > marker.txt".to_string()],
                        working_directory: None,
                    },
                ],
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
            },
        )
        .await;

        assert_eq!(result.status, "rejected");
        assert_eq!(result.results.len(), 2);
        assert_eq!(result.results[0].result.status, "skipped");
        assert_eq!(
            result.results[0].result.reject_reason.as_deref(),
            Some("batch_rejected")
        );
        assert_eq!(result.results[1].result.status, "rejected");
        assert_eq!(
            result.results[1].result.reject_reason.as_deref(),
            Some("batch_confirmation_confirmation_provider_unavailable")
        );
        assert_eq!(
            fs::read_to_string(workspace.join("marker.txt")).unwrap(),
            "untouched"
        );
    }

    #[test]
    fn batch_confirmation_preview_supports_chinese() {
        let mut config = Config::default_config().unwrap();
        config.confirmation_language = "zh-CN".to_string();
        let element = PreparedBatchElement {
            index: 1,
            program: "python".to_string(),
            args: vec!["-c".to_string(), "print(1)".to_string()],
            working_directory: Some("/tmp".to_string()),
            resolved_working_directory: PathBuf::from("/tmp"),
            decision: PolicyDecision::Confirm,
            reject_reason: None,
        };
        let preview = batch_confirmation_preview(&config, &[element.clone()], &[element]);

        assert!(preview.contains("该批次共有 1 条命令，其中 1 条需要确认"));
        assert!(preview.contains("工作目录：/tmp"));
        assert!(preview.contains("是否允许整个批次执行一次？"));
        assert!(!preview.contains("\\n"));
    }

    #[test]
    fn batch_prepare_detects_confirm_and_reject_before_execution() {
        let root = unique_temp_dir("batch-prepare");
        let workspace = root.join("workspace");
        let secret = workspace.join("secret");
        fs::create_dir_all(&secret).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: vec![secret],
        };

        let confirm = prepare_batch_element(
            &config,
            0,
            ExecElement {
                program: "bash".to_string(),
                args: vec!["-lc".to_string(), "echo hi".to_string()],
                working_directory: None,
            },
            None,
            false,
        );
        assert_eq!(confirm.decision, PolicyDecision::Confirm);
        assert!(confirm.reject_reason.is_none());

        let rejected = prepare_batch_element(
            &config,
            1,
            ExecElement {
                program: "cat".to_string(),
                args: vec!["./secret/token".to_string()],
                working_directory: None,
            },
            None,
            false,
        );
        assert_eq!(rejected.reject_reason.as_deref(), Some("path_denied"));
    }

    #[test]
    fn working_directory_must_be_existing_writable_directory() {
        let root = unique_temp_dir("working-dir-policy");
        let workspace = root.join("workspace");
        let subdir = workspace.join("subdir");
        let cache = root.join("cache");
        let secret = workspace.join("secret");
        fs::create_dir_all(&subdir).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&secret).unwrap();
        fs::write(workspace.join("file"), "not a directory").unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace.clone();
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: vec![cache],
            deny_roots: vec![secret.clone()],
        };

        assert_eq!(
            resolve_working_directory(&config, None).unwrap(),
            workspace.canonicalize().unwrap()
        );
        assert_eq!(
            resolve_working_directory(&config, Some("subdir")).unwrap(),
            subdir.canonicalize().unwrap()
        );
        assert_eq!(
            resolve_working_directory(&config, Some("file")).unwrap_err(),
            "working_directory_not_directory"
        );
        assert_eq!(
            resolve_working_directory(&config, Some("missing")).unwrap_err(),
            "working_directory_not_found"
        );
        assert_eq!(
            resolve_working_directory(&config, Some("secret")).unwrap_err(),
            "working_directory_denied"
        );
        assert_eq!(
            resolve_working_directory(&config, Some(root.join("cache").to_string_lossy().as_ref()))
                .unwrap_err(),
            "working_directory_outside_allowed_roots"
        );
    }

    #[test]
    fn relative_path_arguments_are_resolved_from_working_directory() {
        let root = unique_temp_dir("working-dir-relative");
        let workspace = root.join("workspace");
        let subdir = workspace.join("subdir");
        fs::create_dir_all(&subdir).unwrap();
        fs::write(subdir.join("target.txt"), "ok").unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: Vec::new(),
        };

        assert_eq!(
            preflight(
                &config,
                &config.workspace_root,
                "cat",
                &["./target.txt".to_string()]
            )
            .unwrap_err(),
            "path_not_found"
        );
        assert!(preflight(&config, &subdir, "cat", &["./target.txt".to_string()]).is_ok());
    }

    #[test]
    fn symlink_to_denied_path_is_rejected() {
        let root = unique_temp_dir("path-symlink");
        let workspace = root.join("workspace");
        let secret = root.join("secret");
        let link = workspace.join("secret-link");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&secret).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.path_policy = PathPolicyConfig {
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: vec![secret],
        };

        #[cfg(unix)]
        assert_eq!(
            preflight(
                &config,
                &config.workspace_root,
                "cat",
                &[link.to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_denied"
        );
    }

    #[test]
    fn load_old_config_without_path_policy_adds_defaults() {
        let root = unique_temp_dir("old-config");
        let config_path = root.join("config.json");
        let config = Config::default_config().unwrap();
        let mut value = serde_json::to_value(config).unwrap();
        value.as_object_mut().unwrap().remove("pathPolicy");
        fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&config_path).unwrap();
        assert!(!loaded.path_policy.write_roots.is_empty());
        assert!(!loaded.path_policy.read_only_roots.is_empty());
        assert!(!loaded.path_policy.deny_roots.is_empty());
    }

    #[test]
    fn load_partial_path_policy_defaults_missing_lists_to_empty() {
        let root = unique_temp_dir("partial-config");
        let config_path = root.join("config.json");
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["pathPolicy"] = serde_json::json!({
            "writeRoots": [root.join("write")]
        });
        fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.path_policy.write_roots.len(), 1);
        assert!(loaded.path_policy.read_only_roots.is_empty());
        assert!(loaded.path_policy.deny_roots.is_empty());
    }

    #[test]
    fn old_rule_ids_are_ignored_when_loading_config() {
        let root = unique_temp_dir("old-rule-id");
        let config_path = root.join("config.json");
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["policy"]["allow"] = serde_json::json!([
            {
                "id": "legacy-id",
                "program": "bash",
                "argsPrefix": []
            }
        ]);
        fs::write(&config_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.policy.allow.len(), 1);
        assert_eq!(loaded.policy.allow[0].program, "bash");
        assert!(serde_json::to_value(&loaded.policy.allow[0])
            .unwrap()
            .get("id")
            .is_none());
    }

    #[test]
    fn remove_rule_matches_command_without_uuid() {
        let mut rules = vec![Rule {
            program: "bash".to_string(),
            args_prefix: Vec::new(),
        }];

        remove_rule(&mut rules, "bash", &[]).unwrap();
        assert!(rules.is_empty());
    }

    #[test]
    fn remove_rule_matches_command_and_args_prefix() {
        let mut rules = vec![
            Rule {
                program: "python".to_string(),
                args_prefix: vec!["-c".to_string()],
            },
            Rule {
                program: "python".to_string(),
                args_prefix: vec!["script.py".to_string()],
            },
        ];

        remove_rule(&mut rules, "python", &["-c".to_string()]).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].args_prefix, vec!["script.py".to_string()]);
    }

    #[test]
    fn remove_rule_refuses_ambiguous_non_interactive_match() {
        let mut rules = vec![
            Rule {
                program: "bash".to_string(),
                args_prefix: Vec::new(),
            },
            Rule {
                program: "bash".to_string(),
                args_prefix: Vec::new(),
            },
        ];

        let error = remove_rule(&mut rules, "bash", &[]).unwrap_err();
        assert!(error.to_string().contains("multiple_matching_rules"));
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn path_root_remove_matches_expanded_equivalent_path() {
        let root = unique_temp_dir("path-cli");
        let target = root.join("target");
        fs::create_dir_all(&target).unwrap();
        let mut policy = PathPolicyConfig::default();

        mutate_path_roots(
            &mut policy,
            PathRootKind::Write,
            PathRootCommand::Add {
                path: target.clone(),
            },
        );
        assert_eq!(policy.write_roots.len(), 1);
        mutate_path_roots(
            &mut policy,
            PathRootKind::Write,
            PathRootCommand::Remove {
                path: target.join("..").join("target"),
            },
        );
        assert!(policy.write_roots.is_empty());
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
