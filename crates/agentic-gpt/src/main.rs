use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};
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
    worker_url: String,
    agent_secret: String,
    workspace_root: PathBuf,
    backup_limit: usize,
    confirmation_provider: ConfirmationProviderConfig,
    sandbox: SandboxConfig,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SafeConfigSummary {
    workspace_root: String,
    sandbox: SafeSandboxSummary,
    policy_rule_counts: PolicyCounts,
    confirmation_provider: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SafeSandboxSummary {
    enabled: bool,
    mode: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PolicyCounts {
    allow: usize,
    confirm: usize,
    deny: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum WorkerCommand {
    #[serde(rename = "exec")]
    Exec {
        request_id: String,
        task_id: String,
        payload: ExecRequest,
    },
    #[serde(rename = "batchExec")]
    BatchExec {
        request_id: String,
        task_id: String,
        payload: BatchExecRequest,
    },
    #[serde(rename = "startSession")]
    StartSession {
        request_id: String,
        session_id: String,
        payload: ExecRequest,
    },
    #[serde(rename = "listSessions")]
    ListSessions { request_id: String },
    #[serde(rename = "inspectSession")]
    InspectSession {
        request_id: String,
        session_id: String,
    },
    #[serde(rename = "waitSession")]
    WaitSession {
        request_id: String,
        session_id: String,
        seconds: u64,
    },
    #[serde(rename = "killSession")]
    KillSession {
        request_id: String,
        session_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExecRequest {
    agent_id: String,
    program: String,
    args: Vec<String>,
    need_confirm: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchExecRequest {
    agent_id: String,
    elements: Vec<ExecElement>,
    need_confirm: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct ExecElement {
    program: String,
    args: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskResult {
    agent_id: String,
    task_id: String,
    status: String,
    exit_code: Option<i32>,
    stdout_tail: String,
    stderr_tail: String,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reject_reason: Option<String>,
    started_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionInfo {
    agent_id: String,
    session_id: String,
    state: String,
    program: String,
    args: Vec<String>,
    command_preview: String,
    started_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    exit_code: Option<i32>,
    stdout_tail: String,
    stderr_tail: String,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reject_reason: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentMessage<'a> {
    Hello {
        #[serde(rename = "configSummary")]
        config_summary: SafeConfigSummary,
    },
    Heartbeat {
        #[serde(rename = "sentAt")]
        sent_at: DateTime<Utc>,
    },
    TaskUpdate {
        task: &'a TaskResult,
    },
    SessionUpdate {
        session: &'a SessionInfo,
    },
    Response {
        #[serde(rename = "requestId")]
        request_id: &'a str,
        data: serde_json::Value,
    },
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
        "config loaded; agentId={}; workerUrl={}; workspaceRoot={}; sandbox={}",
        initial.agent_id,
        initial.worker_url,
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
    };
    tokio::spawn(watch_config(state.clone()));
    connect_loop(state).await
}

async fn connect_loop(state: AppState) -> Result<()> {
    loop {
        let config = state.config.read().await.clone();
        let url = format!(
            "{}/v1/agents/{}/connect",
            config.worker_url.trim_end_matches('/'),
            config.agent_id
        )
        .replace("http://", "ws://")
        .replace("https://", "wss://");
        let mut request = url.into_client_request()?;
        request
            .headers_mut()
            .insert("x-agent-secret", config.agent_secret.parse()?);

        let proxy = proxy_url();
        log_info(format!(
            "connecting to worker; agentId={}; proxy={}",
            config.agent_id,
            proxy.as_deref().unwrap_or("none")
        ));
        match timeout(
            Duration::from_secs(CONNECT_TIMEOUT_SECS),
            connect_worker(request, proxy),
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
                log_info("connected to worker".to_string());
                let (mut write, mut read) = stream.split();
                let hello = AgentMessage::Hello {
                    config_summary: config.safe_summary(),
                };
                write
                    .send(Message::Text(serde_json::to_string(&hello)?.into()))
                    .await?;
                let mut heartbeat =
                    tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut last_heartbeat_ack = Instant::now();
                loop {
                    tokio::select! {
                        maybe_message = read.next() => {
                            let Some(message) = maybe_message else {
                                log_warn("worker connection closed".to_string());
                                break;
                            };
                            let message = match message {
                                Ok(Message::Text(text)) => text.to_string(),
                                Ok(Message::Close(frame)) => {
                                    log_warn(format!("worker closed websocket; frame={frame:?}"));
                                    break;
                                }
                                Ok(Message::Pong(_)) => {
                                    last_heartbeat_ack = Instant::now();
                                    continue;
                                }
                                Ok(_) => continue,
                                Err(error) => {
                                    log_warn(format!("worker websocket error: {error}"));
                                    break;
                                }
                            };
                            let value: serde_json::Value = match serde_json::from_str(&message) {
                                Ok(value) => value,
                                Err(error) => {
                                    log_warn(format!("ignored invalid worker message: {error}"));
                                    continue;
                                }
                            };
                            if value.get("type").and_then(|value| value.as_str()) == Some("heartbeat_ack") {
                                last_heartbeat_ack = Instant::now();
                                continue;
                            }
                            let command: WorkerCommand = match serde_json::from_value(value) {
                                Ok(command) => command,
                                Err(error) => {
                                    log_warn(format!("ignored unknown worker command: {error}"));
                                    continue;
                                }
                            };
                            handle_worker_command(state.clone(), command, &mut write).await?;
                        }
                        _ = heartbeat.tick() => {
                            if last_heartbeat_ack.elapsed() > Duration::from_secs(HEARTBEAT_ACK_TIMEOUT_SECS) {
                                log_warn("heartbeat ack timeout; reconnecting".to_string());
                                let _ = write.close().await;
                                break;
                            }
                            let heartbeat = AgentMessage::Heartbeat { sent_at: Utc::now() };
                            if let Err(error) = write.send(Message::Text(serde_json::to_string(&heartbeat)?.into())).await {
                                log_warn(format!("heartbeat send failed: {error}"));
                                break;
                            }
                        }
                    }
                }
            }
        }
        log_info(format!("reconnecting in {RECONNECT_DELAY_SECS}s"));
        sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
    }
}

async fn connect_worker(
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
        .ok_or_else(|| anyhow!("worker URL is missing host"))?
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

fn proxy_url() -> Option<String> {
    ["https_proxy", "HTTPS_PROXY", "http_proxy", "HTTP_PROXY"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.trim().is_empty())
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

async fn handle_worker_command<W>(
    state: AppState,
    command: WorkerCommand,
    write: &mut W,
) -> Result<()>
where
    W: SinkExt<Message> + Unpin,
    <W as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    match command {
        WorkerCommand::Exec {
            request_id: _request_id,
            task_id,
            payload,
        } => {
            log_info(format!(
                "exec received; taskId={task_id}; command={}",
                command_preview(&payload.program, &payload.args)
            ));
            let result = run_exec_task(state, task_id, payload).await;
            log_info(format!(
                "exec finished; taskId={}; status={}; exitCode={:?}; rejectReason={:?}",
                result.task_id, result.status, result.exit_code, result.reject_reason
            ));
            send_task(write, &result).await?;
        }
        WorkerCommand::BatchExec {
            request_id: _request_id,
            task_id,
            payload,
        } => {
            log_info(format!(
                "batchExec received; taskId={task_id}; elements={}",
                payload.elements.len()
            ));
            let result = run_batch_task(state, task_id, payload).await;
            log_info(format!(
                "batchExec finished; taskId={}; status={}; exitCode={:?}; rejectReason={:?}",
                result.task_id, result.status, result.exit_code, result.reject_reason
            ));
            send_task(write, &result).await?;
        }
        WorkerCommand::StartSession {
            request_id,
            session_id,
            payload,
        } => {
            log_info(format!(
                "startSession received; sessionId={session_id}; command={}",
                command_preview(&payload.program, &payload.args)
            ));
            let info = start_session(state, session_id, payload).await;
            log_info(format!(
                "startSession result; sessionId={}; state={}; rejectReason={:?}",
                info.session_id, info.state, info.reject_reason
            ));
            send_session(write, &info).await?;
            send_response(write, &request_id, serde_json::to_value(&info)?).await?;
        }
        WorkerCommand::ListSessions { request_id } => {
            let sessions = current_sessions(&state).await;
            send_response(write, &request_id, serde_json::to_value(sessions)?).await?;
        }
        WorkerCommand::InspectSession {
            request_id,
            session_id,
        } => {
            let session = inspect_session(&state, &session_id).await;
            send_response(write, &request_id, serde_json::to_value(session)?).await?;
        }
        WorkerCommand::WaitSession {
            request_id,
            session_id,
            seconds,
        } => {
            sleep(Duration::from_secs(seconds.min(30))).await;
            let session = inspect_session(&state, &session_id).await;
            send_response(write, &request_id, serde_json::to_value(session)?).await?;
        }
        WorkerCommand::KillSession {
            request_id,
            session_id,
        } => {
            log_info(format!("killSession received; sessionId={session_id}"));
            let session = kill_session(&state, &session_id).await;
            send_response(write, &request_id, serde_json::to_value(session)?).await?;
        }
    }
    Ok(())
}

async fn send_task<W>(write: &mut W, result: &TaskResult) -> Result<()>
where
    W: SinkExt<Message> + Unpin,
    <W as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    write
        .send(Message::Text(
            serde_json::to_string(&AgentMessage::TaskUpdate { task: result })?.into(),
        ))
        .await?;
    Ok(())
}

async fn send_session<W>(write: &mut W, session: &SessionInfo) -> Result<()>
where
    W: SinkExt<Message> + Unpin,
    <W as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    write
        .send(Message::Text(
            serde_json::to_string(&AgentMessage::SessionUpdate { session })?.into(),
        ))
        .await?;
    Ok(())
}

async fn send_response<W>(write: &mut W, request_id: &str, data: serde_json::Value) -> Result<()>
where
    W: SinkExt<Message> + Unpin,
    <W as futures_util::Sink<Message>>::Error: std::error::Error + Send + Sync + 'static,
{
    write
        .send(Message::Text(
            serde_json::to_string(&AgentMessage::Response { request_id, data })?.into(),
        ))
        .await?;
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
        let confirmation = request_confirmation(&config, &request.program, &request.args).await;
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
                result.status = "failed".to_string();
                result.reject_reason = Some(reason.to_string());
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
            request_source: "worker".to_string(),
            reject_reason: result.reject_reason.clone(),
        },
    );
    result
}

async fn run_batch_task(state: AppState, task_id: String, request: BatchExecRequest) -> TaskResult {
    let started_at = Utc::now();
    let mut combined_stdout = String::new();
    let mut combined_stderr = String::new();
    let mut exit_code = Some(0);
    let mut status = "completed".to_string();
    let mut reject_reason = None;
    let mut truncated = false;
    for element in request.elements {
        let single = ExecRequest {
            agent_id: request.agent_id.clone(),
            program: element.program,
            args: element.args,
            need_confirm: request.need_confirm,
        };
        let result = run_exec_task(state.clone(), format!("{task_id}:element"), single).await;
        combined_stdout.push_str(&result.stdout_tail);
        combined_stderr.push_str(&result.stderr_tail);
        truncated |= result.truncated;
        if result.status != "completed" {
            status = result.status;
            exit_code = result.exit_code;
            reject_reason = result.reject_reason;
            break;
        }
    }
    TaskResult {
        agent_id: request.agent_id,
        task_id,
        status,
        exit_code,
        stdout_tail: tail_string(&combined_stdout, STDOUT_MAX).0,
        stderr_tail: tail_string(&combined_stderr, STDERR_MAX).0,
        truncated,
        reject_reason,
        started_at,
        updated_at: Utc::now(),
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
    let mut command = if config.sandbox.enabled {
        let mut command = Command::new(&config.sandbox.bubblewrap_path);
        command
            .arg("--die-with-parent")
            .arg("--unshare-all")
            .arg("--proc")
            .arg("/proc")
            .arg("--dev")
            .arg("/dev")
            .arg("--tmpfs")
            .arg("/tmp")
            .arg("--chdir")
            .arg(&config.workspace_root)
            .arg("--bind")
            .arg(&config.workspace_root)
            .arg(&config.workspace_root);
        for path in &config.sandbox.required_runtime_paths {
            if path.exists() {
                command.arg("--ro-bind").arg(path).arg(path);
            }
        }
        command.arg("--").arg(program);
        command
    } else {
        let mut command = Command::new(program);
        command.current_dir(&config.workspace_root);
        command
    };
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
    for arg in args {
        if looks_like_path(arg) && escapes_workspace(&config.workspace_root, arg) {
            return Err("path_outside_workspace".to_string());
        }
    }
    Ok(())
}

fn looks_like_path(arg: &str) -> bool {
    arg.starts_with('/') || arg.starts_with("./") || arg.starts_with("../")
}

fn escapes_workspace(workspace: &Path, arg: &str) -> bool {
    let path = PathBuf::from(arg);
    let candidate = if path.is_absolute() {
        path
    } else {
        workspace.join(path)
    };
    match candidate
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
    {
        Some(parent) => !parent.starts_with(
            workspace
                .canonicalize()
                .unwrap_or_else(|_| workspace.to_path_buf()),
        ),
        None => true,
    }
}

async fn request_confirmation(config: &Config, program: &str, args: &[String]) -> String {
    if config.confirmation_provider.provider != "freedesktop" {
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
        let confirmation = request_confirmation(&config, &request.program, &request.args).await;
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
    let mut command = Command::new(program);
    command.current_dir(&config.workspace_root);
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
                "workspaceRoot" => config.workspace_root = PathBuf::from(value),
                "confirmationProvider" => config.confirmation_provider.provider = value,
                "sandbox.enabled" => config.sandbox.enabled = value.parse::<bool>()?,
                "workerUrl" => config.worker_url = value,
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
            worker_url: "http://localhost:8787".to_string(),
            agent_secret: "change-me".to_string(),
            workspace_root: base.join("workspace"),
            backup_limit: DEFAULT_BACKUP_LIMIT,
            confirmation_provider: ConfirmationProviderConfig {
                provider: "freedesktop".to_string(),
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
        Ok(serde_json::from_str(&text)?)
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
            policy_rule_counts: PolicyCounts {
                allow: self.policy.allow.len(),
                confirm: self.policy.confirm.len(),
                deny: self.policy.deny.len(),
            },
            confirmation_provider: self.confirmation_provider.provider.clone(),
        }
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

fn tail_string(value: &str, max: usize) -> (String, bool) {
    if value.len() <= max {
        (value.to_string(), false)
    } else {
        (value[value.len() - max..].to_string(), true)
    }
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
}
