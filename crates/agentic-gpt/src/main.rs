use agentic_gpt_protocol::{
    AgentMessage, BatchElementResult, BatchExecRequest, BatchExecResult, ConfirmationDecision,
    ConfirmationPayload, ExecElement, ExecRequest, HubCommand, HubMessage, PolicyCounts,
    SafeConfigSummary, SafePathPolicySummary, SafeSandboxSummary, SessionInfo, TaskResult,
};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
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
}

#[derive(Subcommand)]
enum RuleCommand {
    Add {
        program: String,
        args_prefix: Vec<String>,
    },
    Remove {
        rule_id: String,
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
    sandbox: SandboxConfig,
    #[serde(default)]
    path_policy: PathPolicyConfig,
    policy: PolicyConfig,
    limits: LimitsConfig,
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
    id: String,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditRecord {
    task_id: Option<String>,
    session_id: Option<String>,
    time: DateTime<Utc>,
    program: String,
    args: Vec<String>,
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
    sessions: Arc<Mutex<HashMap<String, ManagedSession>>>,
    hub_sender: Arc<Mutex<Option<mpsc::UnboundedSender<Message>>>>,
    pending_confirmations: Arc<Mutex<HashMap<String, oneshot::Sender<String>>>>,
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
        Commands::Run { config } => run(config_path(config)).await,
        Commands::Config { config, command } => handle_config(config_path(config), command).await,
    }
}

async fn run(config_path: PathBuf) -> Result<()> {
    log_info(format!(
        "agentic-gpt starting; config={}",
        config_path.display()
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
        sessions: Arc::new(Mutex::new(HashMap::new())),
        hub_sender: Arc::new(Mutex::new(None)),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
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
    }
    Ok(())
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
    let decision = policy_decision(
        &config,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    let mut confirmation_result = None;

    if let Err(reason) = preflight(&config, &request.program, &request.args) {
        result.status = "rejected".to_string();
        result.reject_reason = Some(reason);
    } else if decision == PolicyDecision::Deny {
        result.status = "rejected".to_string();
        result.reject_reason = Some("policy_denied".to_string());
    } else if decision == PolicyDecision::Confirm {
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
        let execution = execute_command(&config, &request.program, &request.args).await;
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

async fn run_batch_task(
    state: AppState,
    batch_id: String,
    request: BatchExecRequest,
) -> BatchExecResult {
    let started_at = Utc::now();
    let agent_id = request.agent_id.clone();
    let need_confirm = request.need_confirm;
    let confirm_method = request.confirm_method.clone();
    let previews = request
        .elements
        .iter()
        .map(|element| (element.program.clone(), element.args.clone()))
        .collect::<Vec<_>>();
    let total = previews.len();
    let max_concurrent = {
        let config = state.config.read().await;
        config.limits.max_concurrent_tasks.max(1).min(total.max(1))
    };

    let mut pending = request
        .elements
        .into_iter()
        .enumerate()
        .collect::<VecDeque<(usize, ExecElement)>>();
    let mut running = JoinSet::new();
    let mut results: Vec<Option<BatchElementResult>> = vec![None; total];
    let deadline = Instant::now() + Duration::from_secs(EXEC_TIMEOUT_SECS);

    loop {
        while running.len() < max_concurrent {
            let Some((index, element)) = pending.pop_front() else {
                break;
            };
            let task_id = format!("{batch_id}:element:{index}");
            let element_agent_id = agent_id.clone();
            let element_state = state.clone();
            let element_confirm_method = confirm_method.clone();
            running.spawn(async move {
                let program = element.program;
                let args = element.args;
                let request = ExecRequest {
                    agent_id: element_agent_id,
                    program: program.clone(),
                    args: args.clone(),
                    need_confirm,
                    confirm_method: element_confirm_method,
                };
                let result = run_exec_task(element_state, task_id, request).await;
                BatchElementResult {
                    index,
                    program,
                    args,
                    result,
                }
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
                let (program, args) = previews
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| ("<unknown>".to_string(), Vec::new()));
                BatchElementResult {
                    index,
                    program,
                    args,
                    result: TaskResult {
                        agent_id: agent_id.clone(),
                        task_id: format!("{batch_id}:element:{index}"),
                        status: "timeout".to_string(),
                        exit_code: None,
                        stdout_tail: String::new(),
                        stderr_tail: String::new(),
                        truncated: false,
                        reject_reason: Some("exec_timeout_use_session".to_string()),
                        started_at,
                        updated_at,
                    },
                }
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

async fn execute_command(config: &Config, program: &str, args: &[String]) -> Result<CommandOutput> {
    let mut command = build_command(config, program)?;
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

fn preflight(config: &Config, program: &str, args: &[String]) -> std::result::Result<(), String> {
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
    check_path_policy(config, program, args)
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
    program: &str,
    args: &[String],
) -> std::result::Result<(), String> {
    let access = classify_program_access(program);
    let policy = expanded_path_policy(config).map_err(|_| "path_policy_error".to_string())?;
    for arg in args {
        if !looks_like_path(arg) {
            continue;
        }
        let path = resolve_argument_path(&config.workspace_root, arg, access)?;
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

fn build_command(config: &Config, program: &str) -> Result<Command> {
    if config.sandbox.enabled {
        let policy = expanded_path_policy(config)?;
        let mut command = Command::new(&config.sandbox.bubblewrap_path);
        command
            .arg("--die-with-parent")
            .arg("--unshare-all")
            .arg("--dev")
            .arg("/dev")
            .arg("--chdir")
            .arg(&config.workspace_root);
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
        command.current_dir(&config.workspace_root);
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
    let warning = if !config.sandbox.enabled && risky_file_mutation(program) {
        "\nWARNING: bubblewrap is disabled; this file mutation command has broader host visibility."
    } else {
        ""
    };
    let body = format!(
        "{}{}{}",
        command_preview(program, args),
        warning,
        "\nAllow once?"
    );
    let provider = notify_rust::Notification::new()
        .summary("Agentic GPT confirmation")
        .body(&body)
        .action("allow_once", "Allow once")
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
    let request_id = format!("confirm_req_{}", Uuid::new_v4().simple());
    let (tx, rx) = oneshot::channel();
    state
        .pending_confirmations
        .lock()
        .await
        .insert(request_id.clone(), tx);
    let payload = ConfirmationPayload {
        program: program.to_string(),
        args: args.to_vec(),
        command_preview: truncate_chars(&command_preview(program, args), 1000),
        risk_level: risk_level(program),
        reason: format!("Command matched confirm policy: {program}"),
    };
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

fn confirmation_decision_value(decision: ConfirmationDecision) -> String {
    match decision {
        ConfirmationDecision::AllowOnce => "allow_once",
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
    let mut decision = if need_confirm {
        PolicyDecision::Confirm
    } else {
        PolicyDecision::Allow
    };
    for rule in builtin_rules(PolicyDecision::Confirm) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Confirm);
        }
    }
    for rule in builtin_rules(PolicyDecision::Deny) {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Deny);
        }
    }
    for rule in &config.policy.allow {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Allow);
        }
    }
    for rule in &config.policy.confirm {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Confirm);
        }
    }
    for rule in &config.policy.deny {
        if rule.matches(program, args) {
            decision = decision.max(PolicyDecision::Deny);
        }
    }
    decision
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

fn builtin_rules(decision: PolicyDecision) -> Vec<Rule> {
    let programs = match decision {
        PolicyDecision::Deny => vec!["su", "mkfs", "dd", "ssh"],
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
            id: format!("builtin:{program}"),
            program: program.to_string(),
            args_prefix: vec![],
        })
        .collect::<Vec<_>>();
    if decision == PolicyDecision::Confirm {
        rules.push(Rule {
            id: "builtin:python-c".to_string(),
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        });
        rules.push(Rule {
            id: "builtin:node-e".to_string(),
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
        command_preview: command_preview(&request.program, &request.args),
        started_at,
        updated_at: started_at,
        exit_code: None,
        stdout_tail: String::new(),
        stderr_tail: String::new(),
        truncated: false,
        reject_reason: None,
    };
    let decision = policy_decision(
        &config,
        &request.program,
        &request.args,
        request.need_confirm,
    );
    if let Err(reason) = preflight(&config, &request.program, &request.args) {
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
    match spawn_session(&config, &request.program, &request.args).await {
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
    program: &str,
    args: &[String],
) -> Result<(Child, Arc<Mutex<TailBuffer>>, Arc<Mutex<TailBuffer>>)> {
    let mut command = build_command(config, program)?;
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
                "sandbox.enabled" => config.sandbox.enabled = value.parse::<bool>()?,
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
                id: Uuid::new_v4().to_string(),
                program,
                args_prefix,
            };
            println!("{}", rule.id);
            rules.push(rule);
        }
        RuleCommand::Remove { rule_id } => {
            rules.retain(|rule| rule.id != rule_id);
        }
    }
    write_config_with_backup(&config_path, &config)
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
        SafeConfigSummary {
            workspace_root: if self.workspace_root
                == agentic_home().unwrap_or_default().join("workspace")
            {
                "default".to_string()
            } else {
                self.workspace_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| format!("workspace:{name}"))
                    .unwrap_or_else(|| "configured".to_string())
            },
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
                write_root_count: effective_write_root_count(self),
                read_only_root_count: self.path_policy.read_only_roots.len(),
                deny_root_count: self.path_policy.deny_roots.len(),
            },
            policy_rule_counts: PolicyCounts {
                allow: self.policy.allow.len(),
                confirm: self.policy.confirm.len(),
                deny: self.policy.deny.len(),
            },
            confirmation_provider: self.confirmation_provider.provider.clone(),
        }
    }
}

fn effective_write_root_count(config: &Config) -> usize {
    config.path_policy.write_roots.len()
        + usize::from(
            !config
                .path_policy
                .write_roots
                .iter()
                .any(|root| paths_match(root, &config.workspace_root)),
        )
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

    #[test]
    fn rule_matches_program_and_args_prefix_structurally() {
        let rule = Rule {
            id: "r".to_string(),
            program: "python".to_string(),
            args_prefix: vec!["-c".to_string()],
        };
        assert!(rule.matches("python", &["-c".to_string(), "print(1)".to_string()]));
        assert!(!rule.matches("python3", &["-c".to_string()]));
        assert!(!rule.matches("python", &["script.py".to_string()]));
    }

    #[test]
    fn need_confirm_upgrades_allow() {
        let mut config = Config::default_config().unwrap();
        config.policy.allow.push(Rule {
            id: "r".to_string(),
            program: "git".to_string(),
            args_prefix: vec!["status".to_string()],
        });
        assert_eq!(
            policy_decision(&config, "git", &["status".to_string()], true),
            PolicyDecision::Confirm
        );
    }

    #[test]
    fn sudo_requires_credentials() {
        let config = Config::default_config().unwrap();
        assert_eq!(
            preflight(&config, "sudo", &["true".to_string()]).unwrap_err(),
            "interactive_credential_required"
        );
    }

    #[test]
    fn read_only_system_file_is_allowed() {
        let config = Config::default_config().unwrap();
        assert!(preflight(&config, "cat", &["/proc/meminfo".to_string()]).is_ok());
        assert!(preflight(&config, "df", &["/".to_string()]).is_ok());
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
            "touch",
            &[downloads.join("test-file").to_string_lossy().to_string()]
        )
        .is_ok());
        assert_eq!(
            preflight(
                &config,
                "touch",
                &[cache.join("test-file").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_readonly"
        );
        assert!(preflight(&config, "du", &[cache.to_string_lossy().to_string()]).is_ok());
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
                "cat",
                &[secret.join("token").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_denied"
        );
        assert_eq!(
            preflight(
                &config,
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
                "custom-tool",
                &[cache.join("file").to_string_lossy().to_string()]
            )
            .unwrap_err(),
            "path_readonly"
        );
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
            preflight(&config, "cat", &[link.to_string_lossy().to_string()]).unwrap_err(),
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
