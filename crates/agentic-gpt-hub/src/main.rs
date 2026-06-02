mod mcp_server;
mod oauth;

use agentic_gpt_protocol::{
    AgentMessage, AgentRegistryEntry, BatchExecRequest, BatchExecResult, Capabilities,
    ConfirmationDecision, ConfirmationPayload, ExecRequest, HubCommand, HubMessage,
    McpCallToolRequest, McpListServersRequest, McpListToolsRequest, SafeConfigSummary,
    SafePathPolicySummary, SafeSandboxSummary, SessionInfo, TaskResult,
};
use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::{sleep, timeout, Duration};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

const REQUEST_TIMEOUT_SECS: u64 = 35;
const MAX_WAIT_SECONDS: u64 = 30;
const DEFAULT_REMOTE_CONFIRM_TIMEOUT_SECS: u64 = 45;
const MAX_COMMAND_PREVIEW_CHARS: usize = 1000;

#[derive(Parser)]
#[command(name = "agentic-gpt-hub")]
#[command(about = "VPS Hub for Agentic GPT")]
struct Cli {
    #[arg(long, env = "AGENTIC_GPT_HUB_DB")]
    db: Option<PathBuf>,
    #[arg(long, env = "AGENTIC_GPT_HUB_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: HubCommandCli,
}

#[derive(Subcommand)]
enum HubCommandCli {
    Init,
    Serve {
        #[arg(long, env = "AGENTIC_GPT_HUB_BIND", default_value = "127.0.0.1:8787")]
        bind: SocketAddr,
        #[arg(long, env = "AGENTIC_GPT_API_KEY")]
        api_key: String,
        #[arg(long, env = "AGENTIC_GPT_PUBLIC_BASE_URL")]
        public_base_url: Option<String>,
    },
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    Add {
        #[arg(long)]
        agent_id: String,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        secret: String,
    },
    Remove {
        #[arg(long)]
        agent_id: String,
    },
    Disable {
        #[arg(long)]
        agent_id: String,
    },
    Enable {
        #[arg(long)]
        agent_id: String,
    },
    List,
}

#[derive(Clone)]
struct HubState {
    api_key: String,
    db: Arc<StdMutex<Connection>>,
    config: Arc<HubConfig>,
    agents: Arc<Mutex<HashMap<String, AgentConnection>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>,
    pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    sessions: Arc<Mutex<HashMap<String, HashMap<String, SessionInfo>>>>,
    http: reqwest::Client,
    public_base_url: Option<String>,
    oauth_codes: Arc<Mutex<HashMap<String, oauth::OAuthAuthorizationCode>>>,
    oauth_tokens: Arc<Mutex<HashMap<String, oauth::OAuthAccessToken>>>,
}

#[derive(Clone)]
struct AgentConnection {
    connection_id: String,
    sender: mpsc::UnboundedSender<Message>,
    last_seen_at: DateTime<Utc>,
    config_summary: Option<SafeConfigSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct HubConfig {
    remote_confirmation: RemoteConfirmationConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteConfirmationConfig {
    enabled: bool,
    provider: String,
    timeout_seconds: u64,
    ntfy: NtfyConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct NtfyConfig {
    server_url: String,
    topic: String,
    callback_base_url: String,
}

#[derive(Clone, Debug)]
struct PendingConfirmation {
    confirmation_id: String,
    request_id: String,
    agent_id: String,
    token_hash: String,
    command_preview: String,
    risk_level: String,
    reason: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    resolved: bool,
    decision: Option<ConfirmationDecision>,
}

#[derive(Deserialize)]
struct AgentIdQuery {
    #[serde(rename = "agentId")]
    agent_id: String,
}

#[derive(Deserialize)]
struct WaitRequest {
    seconds: Option<u64>,
}

#[derive(Deserialize)]
struct ConfirmationCallbackQuery {
    token: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agentic_gpt_hub=info,tower_http=info,axum=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let db_path = cli.db.unwrap_or_else(default_db_path);
    let config_path = cli.config.unwrap_or_else(default_config_path);
    let config = HubConfig::load_or_default(&config_path)?;
    let conn = open_db(&db_path)?;
    match cli.command {
        HubCommandCli::Init => {
            init_db(&conn)?;
            config.write_if_missing(&config_path)?;
            println!("initialized {}", db_path.display());
            println!("config {}", config_path.display());
        }
        HubCommandCli::Serve {
            bind,
            api_key,
            public_base_url,
        } => {
            init_db(&conn)?;
            config.write_if_missing(&config_path)?;
            serve(bind, api_key, public_base_url, conn, config).await?;
        }
        HubCommandCli::Agent { command } => {
            init_db(&conn)?;
            handle_agent_command(&conn, command)?;
        }
    }
    Ok(())
}

async fn serve(
    bind: SocketAddr,
    api_key: String,
    public_base_url: Option<String>,
    conn: Connection,
    config: HubConfig,
) -> Result<()> {
    let state = HubState {
        api_key,
        db: Arc::new(StdMutex::new(conn)),
        config: Arc::new(config),
        agents: Arc::new(Mutex::new(HashMap::new())),
        pending: Arc::new(Mutex::new(HashMap::new())),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        sessions: Arc::new(Mutex::new(HashMap::new())),
        http: reqwest::Client::new(),
        public_base_url: public_base_url.map(|value| value.trim_end_matches('/').to_string()),
        oauth_codes: Arc::new(Mutex::new(HashMap::new())),
        oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
    };
    tokio::spawn(cleanup_confirmations(state.clone()));
    tokio::spawn(oauth::cleanup_oauth(state.clone()));
    let mcp_service = mcp_server::service(state.clone());
    let app = Router::new()
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/:agent_id/connect", get(connect_agent))
        .route(
            "/v1/confirmations/:confirmation_id/:decision",
            post(confirmation_callback),
        )
        .route("/v1/exec", post(exec))
        .route("/v1/batchExec", post(batch_exec))
        .route("/v1/sessions/start", post(start_session))
        .route("/v1/sessions", get(list_sessions))
        .route("/v1/sessions/:session_id", get(inspect_session))
        .route("/v1/sessions/:session_id/wait", post(wait_session))
        .route("/v1/sessions/:session_id/kill", post(kill_session))
        .route("/v1/mcp/servers", post(mcp_list_servers))
        .route("/v1/mcp/tools", post(mcp_list_tools))
        .route("/v1/mcp/callTool", post(mcp_call_tool))
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth::protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth::authorization_server_metadata),
        )
        .route(
            "/.well-known/openid-configuration",
            get(oauth::authorization_server_metadata),
        )
        .route(
            "/oauth/authorize",
            get(oauth::authorize).post(oauth::authorize_submit),
        )
        .route("/oauth/token", post(oauth::token))
        .nest_service("/mcp", mcp_service)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            mcp_server::require_auth_on_mcp_path,
        ))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path()
                )
            }),
        )
        .with_state(state);

    let listener = TcpListener::bind(bind).await?;
    info!("agentic-gpt hub listening on {bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn list_agents(State(state): State<HubState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let entries = match registry_entries(&state) {
        Ok(entries) => entries,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    };
    let online = state.agents.lock().await;
    let agents = entries
        .into_iter()
        .filter(|entry| entry.enabled)
        .map(|entry| {
            let status = online.get(&entry.agent_id);
            json!({
                "agentId": entry.agent_id,
                "displayName": entry.display_name,
                "online": status.is_some(),
                "lastSeenAt": status.map(|s| s.last_seen_at).or(entry.last_seen_at),
                "capabilities": entry.capabilities,
                "configSummary": status.and_then(|s| s.config_summary.clone()).unwrap_or_else(default_config_summary)
            })
        })
        .collect::<Vec<_>>();
    Json(json!({ "agents": agents })).into_response()
}

async fn exec(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<ExecRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let task_id = random_id("task");
    let command = HubCommand::Exec {
        request_id: random_id("req"),
        task_id: task_id.clone(),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => {
            Json(timeout_task_result(&payload.agent_id, &task_id, reason)).into_response()
        }
    }
}

async fn batch_exec(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<BatchExecRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let batch_id = random_id("batch");
    let command = HubCommand::BatchExec {
        request_id: random_id("req"),
        task_id: batch_id.clone(),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => Json(timeout_batch_result(&payload, &batch_id, reason)).into_response(),
    }
}

async fn start_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<ExecRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let session_id = random_id("sess");
    let command = HubCommand::StartSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => {
            let status = serde_json::from_value::<SessionInfo>(value.clone())
                .ok()
                .map(|session| {
                    if session.state == "running" || session.state == "waiting_confirmation" {
                        "started"
                    } else {
                        "failed"
                    }
                })
                .unwrap_or("started");
            Json(json!({ "status": status, "sessionId": session_id, "session": value }))
                .into_response()
        }
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "session_start_timeout", reason),
    }
}

async fn list_sessions(
    State(state): State<HubState>,
    headers: HeaderMap,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::ListSessions {
        request_id: random_id("req"),
    };
    match request_agent(&state, &query.agent_id, command, 2).await {
        Ok(value) => Json(json!({ "sessions": value })).into_response(),
        Err(_) => {
            let sessions = state
                .sessions
                .lock()
                .await
                .get(&query.agent_id)
                .map(|sessions| sessions.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            Json(json!({ "sessions": sessions })).into_response()
        }
    }
}

async fn inspect_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::InspectSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
    };
    match request_agent(&state, &query.agent_id, command, 2).await {
        Ok(value) if !value.is_null() => Json(value).into_response(),
        _ => match cached_session(&state, &query.agent_id, &session_id).await {
            Some(session) => Json(session).into_response(),
            None => api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session was not found",
            ),
        },
    }
}

async fn wait_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
    Json(body): Json<WaitRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let seconds = body.seconds.unwrap_or(0).min(MAX_WAIT_SECONDS);
    let command = HubCommand::WaitSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
        seconds,
    };
    match request_agent(&state, &query.agent_id, command, seconds + 2).await {
        Ok(value) if !value.is_null() => Json(value).into_response(),
        _ => match cached_session(&state, &query.agent_id, &session_id).await {
            Some(session) => Json(session).into_response(),
            None => api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session was not found",
            ),
        },
    }
}

async fn kill_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::KillSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
    };
    match request_agent(&state, &query.agent_id, command, 5).await {
        Ok(value) if !value.is_null() => Json(value).into_response(),
        _ => match cached_session(&state, &query.agent_id, &session_id).await {
            Some(session) => Json(session).into_response(),
            None => api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session was not found",
            ),
        },
    }
}

async fn mcp_list_servers(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<McpListServersRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let command = HubCommand::McpListServers {
        request_id: random_id("req"),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(
            StatusCode::GATEWAY_TIMEOUT,
            "mcp_list_servers_timeout",
            reason,
        ),
    }
}

async fn mcp_list_tools(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<McpListToolsRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let command = HubCommand::McpListTools {
        request_id: random_id("req"),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(
            StatusCode::GATEWAY_TIMEOUT,
            "mcp_list_tools_timeout",
            reason,
        ),
    }
}

async fn mcp_call_tool(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<McpCallToolRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let command = HubCommand::McpCallTool {
        request_id: random_id("req"),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "mcp_call_tool_timeout", reason),
    }
}

async fn connect_agent(
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
    {
        let mut agents = state.agents.lock().await;
        if let Some(old) = agents.insert(
            agent_id.clone(),
            AgentConnection {
                connection_id: connection_id.clone(),
                sender: tx.clone(),
                last_seen_at: Utc::now(),
                config_summary: None,
            },
        ) {
            let _ = old.sender.send(Message::Close(None));
        }
    }

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
            AgentMessage::Hello { config_summary } => {
                let mut agents = state.agents.lock().await;
                if let Some(connection) = agents.get_mut(&agent_id) {
                    connection.config_summary = Some(config_summary);
                }
            }
            AgentMessage::Heartbeat { sent_at } => {
                let ack = HubMessage::HeartbeatAck {
                    sent_at,
                    received_at: Utc::now(),
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
                        ConfirmationDecision::ProviderUnavailable,
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
        discard_agent_confirmations(&state, &agent_id).await;
    }
    info!(%agent_id, %connection_id, removedCurrentConnection = removed_current_connection, "agent disconnected");
}

async fn request_agent(
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

async fn handle_confirmation_request(
    state: HubState,
    agent_id: String,
    request_id: String,
    timeout_seconds: u64,
    payload: ConfirmationPayload,
) -> Result<()> {
    let remote = &state.config.remote_confirmation;
    if !remote.enabled || remote.provider != "ntfy" {
        send_confirmation_response(
            &state,
            &agent_id,
            &request_id,
            ConfirmationDecision::ProviderUnavailable,
            "remote_confirmation_disabled",
        )
        .await;
        return Ok(());
    }
    if remote.ntfy.topic.trim().is_empty()
        || remote.ntfy.server_url.trim().is_empty()
        || remote.ntfy.callback_base_url.trim().is_empty()
    {
        send_confirmation_response(
            &state,
            &agent_id,
            &request_id,
            ConfirmationDecision::ProviderUnavailable,
            "ntfy_not_configured",
        )
        .await;
        return Ok(());
    }

    let confirmation_id = random_id("confirm");
    let token = random_token();
    let token_hash = sha256_hex(&token);
    let created_at = Utc::now();
    let timeout_seconds = timeout_seconds.max(1).min(remote.timeout_seconds.max(1));
    let expires_at = created_at + chrono::Duration::seconds(timeout_seconds as i64);
    let command_preview = truncate_chars(&payload.command_preview, MAX_COMMAND_PREVIEW_CHARS);
    let pending = PendingConfirmation {
        confirmation_id: confirmation_id.clone(),
        request_id: request_id.clone(),
        agent_id: agent_id.clone(),
        token_hash,
        command_preview: command_preview.clone(),
        risk_level: payload.risk_level.clone(),
        reason: payload.reason.clone(),
        created_at,
        expires_at,
        resolved: false,
        decision: None,
    };
    state
        .pending_confirmations
        .lock()
        .await
        .insert(confirmation_id.clone(), pending);

    match publish_ntfy(
        &state,
        &confirmation_id,
        &token,
        &agent_id,
        &payload,
        &command_preview,
    )
    .await
    {
        Ok(()) => {
            info!(
                %agent_id,
                %confirmation_id,
                "remote confirmation notification sent"
            );
        }
        Err(error) => {
            warn!(%agent_id, %confirmation_id, %error, "remote confirmation notification failed");
            state
                .pending_confirmations
                .lock()
                .await
                .remove(&confirmation_id);
            send_confirmation_response(
                &state,
                &agent_id,
                &request_id,
                ConfirmationDecision::ProviderUnavailable,
                "ntfy_publish_failed",
            )
            .await;
        }
    }
    Ok(())
}

async fn publish_ntfy(
    state: &HubState,
    confirmation_id: &str,
    token: &str,
    agent_id: &str,
    payload: &ConfirmationPayload,
    command_preview: &str,
) -> Result<()> {
    let remote = &state.config.remote_confirmation;
    let ntfy = &remote.ntfy;
    let server_url = ntfy.server_url.trim_end_matches('/');
    let callback_base = ntfy.callback_base_url.trim_end_matches('/');
    let allow_url =
        format!("{callback_base}/v1/confirmations/{confirmation_id}/allow?token={token}");
    let allow_mcp_15m_url = format!(
        "{callback_base}/v1/confirmations/{confirmation_id}/allow-mcp-server-15m?token={token}"
    );
    let allow_mcp_30m_url = format!(
        "{callback_base}/v1/confirmations/{confirmation_id}/allow-mcp-server-30m?token={token}"
    );
    let deny_url = format!("{callback_base}/v1/confirmations/{confirmation_id}/deny?token={token}");
    let message = format!(
        "Agent {agent_id} wants to run:\n{command_preview}\n\nReason: {}\nRisk: {}",
        payload.reason, payload.risk_level
    );
    let actions = if payload.kind.as_deref() == Some("mcpTool") {
        json!([
            {
                "action": "http",
                "label": "Allow once",
                "url": allow_url,
                "method": "POST",
                "clear": true
            },
            {
                "action": "http",
                "label": "Allow MCP 15m",
                "url": allow_mcp_15m_url,
                "method": "POST",
                "clear": true
            },
            {
                "action": "http",
                "label": "Allow MCP 30m",
                "url": allow_mcp_30m_url,
                "method": "POST",
                "clear": true
            },
            {
                "action": "http",
                "label": "Deny",
                "url": deny_url,
                "method": "POST",
                "clear": true
            }
        ])
    } else {
        json!([
            {
                "action": "http",
                "label": "Allow",
                "url": allow_url,
                "method": "POST",
                "clear": true
            },
            {
                "action": "http",
                "label": "Deny",
                "url": deny_url,
                "method": "POST",
                "clear": true
            }
        ])
    };
    let body = json!({
        "topic": ntfy.topic,
        "title": "AgenticGPT confirmation",
        "message": message,
        "priority": 5,
        "tags": ["warning"],
        "actions": actions
    });
    let response = state.http.post(server_url).json(&body).send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("ntfy returned {}", response.status()))
    }
}

async fn confirmation_callback(
    State(state): State<HubState>,
    Path((confirmation_id, decision)): Path<(String, String)>,
    Query(query): Query<ConfirmationCallbackQuery>,
) -> Response {
    let decision = match decision.as_str() {
        "allow" => ConfirmationDecision::AllowOnce,
        "allow-mcp-server-15m" | "allow_mcp_server_15m" => ConfirmationDecision::AllowMcpServer15m,
        "allow-mcp-server-30m" | "allow_mcp_server_30m" => ConfirmationDecision::AllowMcpServer30m,
        "deny" => ConfirmationDecision::Deny,
        _ => {
            return api_error(
                StatusCode::NOT_FOUND,
                "confirmation_not_found",
                "Unknown confirmation callback action",
            )
        }
    };
    let token_hash = sha256_hex(&query.token);
    let (agent_id, request_id, status, command_preview, risk_level, confirm_reason, created_at) = {
        let mut confirmations = state.pending_confirmations.lock().await;
        let Some(pending) = confirmations.get_mut(&confirmation_id) else {
            return api_error(
                StatusCode::NOT_FOUND,
                "confirmation_not_found",
                "Confirmation was not found",
            );
        };
        if pending.resolved && Utc::now() >= pending.expires_at {
            return api_error(
                StatusCode::GONE,
                "confirmation_expired",
                "Confirmation has expired",
            );
        }
        if pending.resolved {
            return api_error(
                StatusCode::CONFLICT,
                "confirmation_resolved",
                "Confirmation has already been resolved",
            );
        }
        if Utc::now() >= pending.expires_at {
            pending.resolved = true;
            pending.decision = Some(ConfirmationDecision::Expired);
            (
                pending.agent_id.clone(),
                pending.request_id.clone(),
                StatusCode::GONE,
                pending.command_preview.clone(),
                pending.risk_level.clone(),
                pending.reason.clone(),
                pending.created_at,
            )
        } else if !constant_time_equal(&token_hash, &pending.token_hash) {
            return api_error(
                StatusCode::FORBIDDEN,
                "callback_token_invalid",
                "Invalid confirmation token",
            );
        } else {
            pending.resolved = true;
            pending.decision = Some(decision.clone());
            (
                pending.agent_id.clone(),
                pending.request_id.clone(),
                StatusCode::OK,
                pending.command_preview.clone(),
                pending.risk_level.clone(),
                pending.reason.clone(),
                pending.created_at,
            )
        }
    };

    if status == StatusCode::GONE {
        send_confirmation_response(
            &state,
            &agent_id,
            &request_id,
            ConfirmationDecision::Expired,
            "expired",
        )
        .await;
        return api_error(
            StatusCode::GONE,
            "confirmation_expired",
            "Confirmation has expired",
        );
    }

    let reason = match decision {
        ConfirmationDecision::AllowOnce => "user_allowed",
        ConfirmationDecision::AllowMcpServer15m => "user_allowed_mcp_server_15m",
        ConfirmationDecision::AllowMcpServer30m => "user_allowed_mcp_server_30m",
        ConfirmationDecision::Deny => "user_denied",
        _ => "resolved",
    };
    send_confirmation_response(&state, &agent_id, &request_id, decision.clone(), reason).await;
    info!(
        %agent_id,
        %confirmation_id,
        ?decision,
        %risk_level,
        reason = %confirm_reason,
        %created_at,
        commandPreview = %command_preview,
        "confirmation resolved by callback"
    );
    Json(json!({
        "status": "accepted",
        "decision": decision_wire_value(&decision)
    }))
    .into_response()
}

async fn cleanup_confirmations(state: HubState) {
    loop {
        sleep(Duration::from_secs(2)).await;
        let expired = {
            let now = Utc::now();
            let mut confirmations = state.pending_confirmations.lock().await;
            confirmations
                .values_mut()
                .filter(|pending| !pending.resolved && now >= pending.expires_at)
                .map(|pending| {
                    pending.resolved = true;
                    pending.decision = Some(ConfirmationDecision::Timeout);
                    (
                        pending.agent_id.clone(),
                        pending.request_id.clone(),
                        pending.confirmation_id.clone(),
                        pending.command_preview.clone(),
                        pending.risk_level.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        for (agent_id, request_id, confirmation_id, command_preview, risk_level) in expired {
            info!(
                %agent_id,
                %confirmation_id,
                %risk_level,
                commandPreview = %command_preview,
                "confirmation timed out"
            );
            send_confirmation_response(
                &state,
                &agent_id,
                &request_id,
                ConfirmationDecision::Timeout,
                "timeout",
            )
            .await;
        }
    }
}

async fn discard_agent_confirmations(state: &HubState, agent_id: &str) {
    let mut confirmations = state.pending_confirmations.lock().await;
    for pending in confirmations.values_mut() {
        if pending.agent_id == agent_id && !pending.resolved {
            pending.resolved = true;
            pending.decision = Some(ConfirmationDecision::ProviderUnavailable);
        }
    }
}

async fn send_confirmation_response(
    state: &HubState,
    agent_id: &str,
    request_id: &str,
    decision: ConfirmationDecision,
    reason: &str,
) {
    let message = HubMessage::ConfirmationResponse {
        request_id: request_id.to_string(),
        decision,
        reason: reason.to_string(),
    };
    let Ok(text) = serde_json::to_string(&message) else {
        return;
    };
    let sender = {
        let agents = state.agents.lock().await;
        agents
            .get(agent_id)
            .map(|connection| connection.sender.clone())
    };
    if let Some(sender) = sender {
        let _ = sender.send(Message::Text(text));
    }
}

fn command_request_id(command: &HubCommand) -> &str {
    match command {
        HubCommand::Exec { request_id, .. }
        | HubCommand::BatchExec { request_id, .. }
        | HubCommand::StartSession { request_id, .. }
        | HubCommand::ListSessions { request_id }
        | HubCommand::InspectSession { request_id, .. }
        | HubCommand::WaitSession { request_id, .. }
        | HubCommand::KillSession { request_id, .. }
        | HubCommand::McpListServers { request_id }
        | HubCommand::McpListTools { request_id, .. }
        | HubCommand::McpCallTool { request_id, .. } => request_id,
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
        | HubCommand::McpListServers { request_id }
        | HubCommand::McpListTools { request_id, .. }
        | HubCommand::McpCallTool { request_id, .. } => *request_id = value,
    }
}

fn require_action_auth(state: &HubState, headers: &HeaderMap) -> std::result::Result<(), Response> {
    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let token = parse_bearer_token(auth);
    if token
        .as_deref()
        .map(|token| constant_time_equal(token, state.api_key.trim()))
        .unwrap_or(false)
    {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid GPT Actions API key",
        ))
    }
}

fn parse_bearer_token(value: &str) -> Option<String> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let scheme = parts.next()?;
    let token = parts.next()?.trim();
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token.to_string())
    } else {
        None
    }
}

fn require_agent_enabled(state: &HubState, agent_id: &str) -> std::result::Result<(), Response> {
    match registry_entry(state, agent_id) {
        Ok(Some(entry)) if entry.enabled => Ok(()),
        Ok(_) => Err(api_error(
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

async fn cached_session(state: &HubState, agent_id: &str, session_id: &str) -> Option<SessionInfo> {
    state
        .sessions
        .lock()
        .await
        .get(agent_id)
        .and_then(|sessions| sessions.get(session_id).cloned())
}

async fn touch_agent(state: &HubState, agent_id: &str) {
    if let Some(connection) = state.agents.lock().await.get_mut(agent_id) {
        connection.last_seen_at = Utc::now();
    }
}

fn timeout_task_result(agent_id: &str, task_id: &str, reason: String) -> TaskResult {
    let at = Utc::now();
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

fn timeout_batch_result(
    payload: &BatchExecRequest,
    batch_id: &str,
    reason: String,
) -> BatchExecResult {
    let at = Utc::now();
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

fn api_error(status: StatusCode, code: &'static str, message: impl ToString) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorDetail {
                code,
                message: message.to_string(),
            },
        }),
    )
        .into_response()
}

fn random_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn random_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

fn decision_wire_value(decision: &ConfirmationDecision) -> &'static str {
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
}

fn default_config_summary() -> SafeConfigSummary {
    SafeConfigSummary {
        workspace_root: "unknown".to_string(),
        sandbox: SafeSandboxSummary {
            enabled: false,
            mode: "unknown".to_string(),
        },
        path_policy: SafePathPolicySummary {
            write_root_count: 0,
            read_only_root_count: 0,
            deny_root_count: 0,
        },
        policy_rule_counts: agentic_gpt_protocol::PolicyCounts {
            allow: 0,
            confirm: 0,
            deny: 0,
        },
        confirmation_provider: "unknown".to_string(),
    }
}

fn default_db_path() -> PathBuf {
    dirs_fallback_home()
        .join(".agentic_gpt")
        .join("hub.sqlite3")
}

fn default_config_path() -> PathBuf {
    dirs_fallback_home().join(".agentic_gpt").join("hub.json")
}

fn dirs_fallback_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

impl HubConfig {
    fn default_config() -> Self {
        Self {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: false,
                provider: "ntfy".to_string(),
                timeout_seconds: DEFAULT_REMOTE_CONFIRM_TIMEOUT_SECS,
                ntfy: NtfyConfig {
                    server_url: "https://ntfy.example.invalid".to_string(),
                    topic: "change-me-high-entropy-topic".to_string(),
                    callback_base_url: "https://agentic-gpt.example.invalid".to_string(),
                },
            },
        }
    }

    fn load_or_default(path: &PathBuf) -> Result<Self> {
        if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("read hub config {}", path.display()))?;
            Ok(serde_json::from_str(&text)
                .with_context(|| format!("parse hub config {}", path.display()))?)
        } else {
            Ok(Self::default_config())
        }
    }

    fn write_if_missing(&self, path: &PathBuf) -> Result<()> {
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn open_db(path: &PathBuf) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Connection::open(path).with_context(|| format!("open sqlite db {}", path.display()))
}

fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        create table if not exists agents (
            agent_id text primary key,
            display_name text not null,
            enabled integer not null,
            secret_hash text not null,
            last_seen_at text,
            capabilities_json text not null
        );
        ",
    )?;
    Ok(())
}

fn handle_agent_command(conn: &Connection, command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Add {
            agent_id,
            display_name,
            secret,
        } => {
            let capabilities = Capabilities {
                sessions: true,
                confirmation: true,
                notification_actions: true,
            };
            conn.execute(
                "insert into agents(agent_id, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values (?1, ?2, 1, ?3, null, ?4)
                 on conflict(agent_id) do update set display_name = excluded.display_name,
                     enabled = 1, secret_hash = excluded.secret_hash, capabilities_json = excluded.capabilities_json",
                params![
                    agent_id,
                    display_name,
                    sha256_hex(&secret),
                    serde_json::to_string(&capabilities)?
                ],
            )?;
            println!("agent saved");
        }
        AgentCommand::Remove { agent_id } => {
            conn.execute("delete from agents where agent_id = ?1", params![agent_id])?;
            println!("agent removed");
        }
        AgentCommand::Disable { agent_id } => {
            conn.execute(
                "update agents set enabled = 0 where agent_id = ?1",
                params![agent_id],
            )?;
            println!("agent disabled");
        }
        AgentCommand::Enable { agent_id } => {
            conn.execute(
                "update agents set enabled = 1 where agent_id = ?1",
                params![agent_id],
            )?;
            println!("agent enabled");
        }
        AgentCommand::List => {
            for entry in registry_entries_from_conn(conn)? {
                println!(
                    "{}\t{}\tenabled={}\tlastSeenAt={}",
                    entry.agent_id,
                    entry.display_name,
                    entry.enabled,
                    entry
                        .last_seen_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string())
                );
            }
        }
    }
    Ok(())
}

fn registry_entries(state: &HubState) -> Result<Vec<AgentRegistryEntry>> {
    let conn = state.db.lock().unwrap();
    registry_entries_from_conn(&conn)
}

fn registry_entries_from_conn(conn: &Connection) -> Result<Vec<AgentRegistryEntry>> {
    let mut stmt = conn.prepare(
        "select agent_id, display_name, enabled, secret_hash, last_seen_at, capabilities_json from agents order by agent_id",
    )?;
    let rows = stmt.query_map([], row_to_entry)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn registry_entry(state: &HubState, agent_id: &str) -> Result<Option<AgentRegistryEntry>> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "select agent_id, display_name, enabled, secret_hash, last_seen_at, capabilities_json from agents where agent_id = ?1",
    )?;
    let mut rows = stmt.query_map(params![agent_id], row_to_entry)?;
    rows.next().transpose().map_err(Into::into)
}

fn update_last_seen(state: &HubState, agent_id: &str) -> Result<()> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "update agents set last_seen_at = ?2 where agent_id = ?1",
        params![agent_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRegistryEntry> {
    let last_seen: Option<String> = row.get(4)?;
    let capabilities_json: String = row.get(5)?;
    Ok(AgentRegistryEntry {
        agent_id: row.get(0)?,
        display_name: row.get(1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        secret_hash: row.get(3)?,
        last_seen_at: last_seen.and_then(|value| {
            DateTime::parse_from_rfc3339(&value)
                .ok()
                .map(|value| value.with_timezone(&Utc))
        }),
        capabilities: serde_json::from_str(&capabilities_json).unwrap_or(Capabilities {
            sessions: true,
            confirmation: true,
            notification_actions: false,
        }),
    })
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_time_equal(a: &str, b: &str) -> bool {
    let max = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for index in 0..max {
        diff |= a.as_bytes().get(index).copied().unwrap_or(0) as usize
            ^ b.as_bytes().get(index).copied().unwrap_or(0) as usize;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_case_insensitively() {
        assert_eq!(parse_bearer_token("bearer  abc ").as_deref(), Some("abc"));
        assert_eq!(parse_bearer_token("Basic abc"), None);
    }

    #[test]
    fn safe_default_summary_has_no_paths_or_secrets() {
        let summary = default_config_summary();
        assert_eq!(summary.workspace_root, "unknown");
        assert_eq!(summary.sandbox.mode, "unknown");
    }
}
