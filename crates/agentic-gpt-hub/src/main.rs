mod mcp_server;
mod oauth;

use agentic_gpt_protocol::{
    AgentMessage, AgentRegistryEntry, AgentRole, BatchExecRequest, BatchExecResult, Capabilities,
    ConfirmationDecision, ConfirmationPayload, ExecRequest, HubCommand, HubInfoAgents,
    HubInfoCounts, HubInfoRemoteConfirmation, HubInfoResponse, HubMessage, McpCallToolRequest,
    McpListServersRequest, McpListToolsRequest, NotebookAppendRequest, NotebookCurrentRequest,
    NotebookRecentRequest, NotebookRemoveRequest, NotebookSearchRequest,
    NotebookSelectExactRequest, NotebookUpdateRequest, NotificationChannel, SafeBuiltinPolicyRules,
    SafeConfigSummary, SafePathPolicySummary, SafePolicyRules, SafeSandboxSummary, SessionInfo,
    TaskResult, UserNotifyDeliveryRequest, UserNotifySendRequest, UserNotifySendResponse,
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
        alias: Option<String>,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        secret: String,
    },
    Alias {
        #[command(subcommand)]
        command: AgentAliasCommand,
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

#[derive(Subcommand)]
enum AgentAliasCommand {
    Set {
        #[arg(long)]
        agent_id: String,
        #[arg(long)]
        alias: String,
    },
    Clear {
        #[arg(long)]
        agent_id: String,
    },
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
    active_room: Arc<Mutex<Option<ActiveRoomConnection>>>,
    http: reqwest::Client,
    public_base_url: Option<String>,
    oauth_codes: Arc<Mutex<HashMap<String, oauth::OAuthAuthorizationCode>>>,
    oauth_tokens: Arc<Mutex<HashMap<String, oauth::OAuthAccessToken>>>,
    ntfy_health: Arc<Mutex<Option<NtfyHealthCache>>>,
}

#[derive(Clone)]
struct AgentConnection {
    connection_id: String,
    sender: mpsc::UnboundedSender<Message>,
    last_seen_at: DateTime<Utc>,
    role: AgentRole,
    config_summary: Option<SafeConfigSummary>,
    notification_channels: Vec<NotificationChannel>,
}

#[derive(Clone, Debug)]
struct ActiveRoomConnection {
    agent_id: String,
    connection_id: String,
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
struct NtfyHealthCache {
    server_url: String,
    checked_at: DateTime<Utc>,
    result: NtfyHealthStatus,
}

#[derive(Clone, Debug)]
enum NtfyHealthStatus {
    Healthy,
    Unhealthy,
    Failed,
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
#[serde(rename_all = "camelCase")]
struct UserNotifySendBody {
    channel: String,
    title: String,
    body: String,
    #[serde(default)]
    actions: Vec<agentic_gpt_protocol::NotificationAction>,
    #[serde(default)]
    priority: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidRegisterRequest {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    capabilities: Value,
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
        active_room: Arc::new(Mutex::new(None)),
        http: reqwest::Client::new(),
        public_base_url: public_base_url.map(|value| value.trim_end_matches('/').to_string()),
        oauth_codes: Arc::new(Mutex::new(HashMap::new())),
        oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
        ntfy_health: Arc::new(Mutex::new(None)),
    };
    tokio::spawn(cleanup_confirmations(state.clone()));
    tokio::spawn(oauth::cleanup_oauth(state.clone()));
    let app = Router::new()
        .route("/v1/info", get(hub_info))
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
        .route("/v1/notify/channels", get(notify_channels))
        .route("/v1/notify/send", post(notify_send))
        .route("/v1/notify/android/register", post(android_notify_register))
        .route("/v1/room/notebook/append", post(room_notebook_append))
        .route("/v1/room/notebook/recent", post(room_notebook_recent))
        .route(
            "/v1/room/notebook/selectExact",
            post(room_notebook_select_exact),
        )
        .route("/v1/room/notebook/search", post(room_notebook_search))
        .route("/v1/room/notebook/current", post(room_notebook_current))
        .route("/v1/room/notebook/update", post(room_notebook_update))
        .route("/v1/room/notebook/remove", post(room_notebook_remove))
        .route("/mcp", get(mcp_server::mcp_get).post(mcp_server::mcp_post))
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

async fn hub_info(State(state): State<HubState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }

    match build_hub_info_response(&state).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

async fn build_hub_info_response(state: &HubState) -> Result<HubInfoResponse> {
    let entries = registry_entries(state)?;
    let registered_count = entries.len();
    let enabled_count = entries.iter().filter(|entry| entry.enabled).count();
    let online_count = state.agents.lock().await.len();
    let pending_request_count = state.pending.lock().await.len();
    let pending_confirmation_count = state
        .pending_confirmations
        .lock()
        .await
        .values()
        .filter(|confirmation| !confirmation.resolved)
        .count();
    let cached_session_count = state.sessions.lock().await.values().map(HashMap::len).sum();

    let remote = &state.config.remote_confirmation;
    let ntfy = &remote.ntfy;
    Ok(HubInfoResponse {
        service: "agentic-gpt-hub".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        public_base_url: state.public_base_url.clone(),
        request_timeout_seconds: REQUEST_TIMEOUT_SECS,
        max_wait_seconds: MAX_WAIT_SECONDS,
        remote_confirmation: HubInfoRemoteConfirmation {
            enabled: remote.enabled,
            provider: remote.provider.clone(),
            timeout_seconds: remote.timeout_seconds,
            ntfy_configured: !ntfy_not_configured(ntfy)
                && !ntfy.callback_base_url.trim().is_empty(),
        },
        agents: HubInfoAgents {
            registered_count,
            enabled_count,
            online_count,
        },
        counts: HubInfoCounts {
            pending_request_count,
            pending_confirmation_count,
            cached_session_count,
        },
        generated_at: Utc::now(),
    })
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
                "alias": entry.alias,
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
    if let Some(agent_id) = payload.agent_id.as_deref() {
        if let Err(response) = require_agent_enabled(&state, agent_id) {
            return response;
        }
        let command = HubCommand::McpListServers {
            request_id: random_id("req"),
        };
        return match request_agent(&state, agent_id, command, REQUEST_TIMEOUT_SECS).await {
            Ok(value) => Json(value).into_response(),
            Err(reason) => api_error(
                StatusCode::GATEWAY_TIMEOUT,
                "mcp_list_servers_timeout",
                reason,
            ),
        };
    }

    match mcp_list_servers_all_agents(&state).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", reason),
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

async fn notify_channels(State(state): State<HubState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match notification_channels(&state).await {
        Ok(channels) => Json(json!({ "channels": channels })).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

async fn notify_send(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<UserNotifySendBody>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let request = UserNotifySendRequest {
        channel_key: payload.channel,
        title: payload.title,
        body: payload.body,
        actions: payload.actions,
        priority: payload.priority,
    };
    match send_user_notification(&state, request).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => notify_route_error_response(error),
    }
}

async fn android_notify_register(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<AndroidRegisterRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match register_android_endpoint(&state, payload) {
        Ok(value) => Json(value).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

async fn room_notebook_append(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookAppendRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookAppend {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_append_timeout",
    )
    .await
}

async fn room_notebook_recent(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookRecentRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookRecent {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_recent_timeout",
    )
    .await
}

async fn room_notebook_select_exact(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookSelectExactRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookSelectExact {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_select_exact_timeout",
    )
    .await
}

async fn room_notebook_search(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookSearchRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookSearch {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_search_timeout",
    )
    .await
}

async fn room_notebook_current(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookCurrentRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookCurrent {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_current_timeout",
    )
    .await
}

async fn room_notebook_update(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookUpdateRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookUpdate {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_update_timeout",
    )
    .await
}

async fn room_notebook_remove(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<NotebookRemoveRequest>,
) -> Response {
    forward_room_command(
        state,
        headers,
        HubCommand::RoomNotebookRemove {
            request_id: random_id("req"),
            payload,
        },
        "room_notebook_remove_timeout",
    )
    .await
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NotifyChannelKey {
    AgentFreedesktop { alias: String },
    HubNtfy,
    AndroidNotice,
    AndroidAlarm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AndroidEndpointState {
    NotConnected,
    DeliveryNotImplemented,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NotifyRouteError {
    InvalidChannel(String),
    AgentNotFound(String),
    ChannelUnavailable {
        channel_key: String,
        reason: &'static str,
    },
    DeliveryFailed {
        channel_key: String,
        reason: String,
    },
    Db(String),
}

pub(crate) async fn notification_channels(state: &HubState) -> Result<Vec<NotificationChannel>> {
    let entries = registry_entries(state)?;
    let by_id = entries
        .into_iter()
        .map(|entry| (entry.agent_id.clone(), entry))
        .collect::<HashMap<_, _>>();
    let online = state.agents.lock().await;
    let mut channels = Vec::new();
    for (agent_id, connection) in online.iter() {
        let Some(entry) = by_id.get(agent_id) else {
            continue;
        };
        if !entry.enabled {
            continue;
        }
        let alias = entry.alias.as_deref().unwrap_or(&entry.agent_id);
        for channel in &connection.notification_channels {
            if channel.kind == "freedesktop" {
                channels.push(NotificationChannel {
                    key: format!("agent::{alias}::freedesktop"),
                    display_name: format!("{} desktop notification", entry.display_name),
                    available: true,
                    kind: "freedesktop".to_string(),
                    supports_actions: channel.supports_actions,
                    reason: None,
                    agent_id: Some(entry.agent_id.clone()),
                });
            }
        }
    }

    let ntfy_state = ntfy_channel_state(state).await;
    channels.push(NotificationChannel {
        key: "hub::ntfy".to_string(),
        display_name: "ntfy notification".to_string(),
        available: ntfy_state.reason.is_none(),
        kind: "ntfy".to_string(),
        supports_actions: false,
        reason: ntfy_state.reason.map(str::to_string),
        agent_id: None,
    });

    let android_reason = match android_endpoint_state(state) {
        AndroidEndpointState::NotConnected => "android_endpoint_not_connected",
        AndroidEndpointState::DeliveryNotImplemented => "android_delivery_not_implemented",
    };
    channels.push(NotificationChannel {
        key: "hub::android::notice".to_string(),
        display_name: "Android normal notification".to_string(),
        available: false,
        kind: "android_notice".to_string(),
        supports_actions: true,
        reason: Some(android_reason.to_string()),
        agent_id: None,
    });
    channels.push(NotificationChannel {
        key: "hub::android::alarm".to_string(),
        display_name: "Android alarm notification".to_string(),
        available: false,
        kind: "android_alarm".to_string(),
        supports_actions: true,
        reason: Some(android_reason.to_string()),
        agent_id: None,
    });
    Ok(channels)
}

pub(crate) async fn send_user_notification(
    state: &HubState,
    request: UserNotifySendRequest,
) -> std::result::Result<UserNotifySendResponse, NotifyRouteError> {
    match parse_notify_channel_key(&request.channel_key)? {
        NotifyChannelKey::AgentFreedesktop { alias } => {
            let agent_id = resolve_agent_alias(state, &alias)?;
            let delivery = UserNotifyDeliveryRequest {
                channel_key: request.channel_key.clone(),
                title: request.title.clone(),
                body: request.body.clone(),
                actions: request.actions.clone(),
                priority: request.priority.clone(),
            };
            let command = HubCommand::UserNotifyDeliver {
                request_id: random_id("req"),
                payload: delivery,
            };
            match request_agent(state, &agent_id, command, REQUEST_TIMEOUT_SECS).await {
                Ok(value) => {
                    if value.get("error").is_some() {
                        return Err(NotifyRouteError::DeliveryFailed {
                            channel_key: request.channel_key,
                            reason: value.to_string(),
                        });
                    }
                    Ok(UserNotifySendResponse {
                        channel_key: request.channel_key,
                        accepted: value
                            .get("delivered")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        delivery_id: None,
                        reason: value
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                }
                Err(reason) => Err(NotifyRouteError::DeliveryFailed {
                    channel_key: request.channel_key,
                    reason,
                }),
            }
        }
        NotifyChannelKey::HubNtfy => {
            if ntfy_not_configured(&state.config.remote_confirmation.ntfy) {
                return Err(NotifyRouteError::ChannelUnavailable {
                    channel_key: request.channel_key,
                    reason: "ntfy_not_configured",
                });
            }
            publish_ntfy_notification(state, &request)
                .await
                .map_err(|error| NotifyRouteError::DeliveryFailed {
                    channel_key: request.channel_key.clone(),
                    reason: error.to_string(),
                })?;
            Ok(UserNotifySendResponse {
                channel_key: request.channel_key,
                accepted: true,
                delivery_id: None,
                reason: None,
            })
        }
        NotifyChannelKey::AndroidNotice | NotifyChannelKey::AndroidAlarm => {
            let reason = match android_endpoint_state(state) {
                AndroidEndpointState::NotConnected => "android_endpoint_not_connected",
                AndroidEndpointState::DeliveryNotImplemented => "android_delivery_not_implemented",
            };
            Err(NotifyRouteError::ChannelUnavailable {
                channel_key: request.channel_key,
                reason,
            })
        }
    }
}

pub(crate) fn parse_notify_channel_key(
    key: &str,
) -> std::result::Result<NotifyChannelKey, NotifyRouteError> {
    let parts = key.split("::").collect::<Vec<_>>();
    match parts.as_slice() {
        ["agent", alias, "freedesktop"] if !alias.trim().is_empty() => {
            Ok(NotifyChannelKey::AgentFreedesktop {
                alias: alias.to_string(),
            })
        }
        ["hub", "ntfy"] => Ok(NotifyChannelKey::HubNtfy),
        ["hub", "android", "notice"] => Ok(NotifyChannelKey::AndroidNotice),
        ["hub", "android", "alarm"] => Ok(NotifyChannelKey::AndroidAlarm),
        _ => Err(NotifyRouteError::InvalidChannel(key.to_string())),
    }
}

pub(crate) fn resolve_agent_alias(
    state: &HubState,
    alias: &str,
) -> std::result::Result<String, NotifyRouteError> {
    let entries =
        registry_entries(state).map_err(|error| NotifyRouteError::Db(error.to_string()))?;
    entries
        .into_iter()
        .find(|entry| {
            entry.enabled
                && (entry.alias.as_deref() == Some(alias)
                    || (entry.alias.is_none() && entry.agent_id == alias))
        })
        .map(|entry| entry.agent_id)
        .ok_or_else(|| NotifyRouteError::AgentNotFound(alias.to_string()))
}

pub(crate) async fn publish_ntfy_notification(
    state: &HubState,
    request: &UserNotifySendRequest,
) -> Result<()> {
    let ntfy = &state.config.remote_confirmation.ntfy;
    let server_url = ntfy.server_url.trim_end_matches('/');
    let body = json!({
        "topic": ntfy.topic,
        "title": request.title,
        "message": request.body,
        "priority": ntfy_priority(request.priority.as_deref()),
    });
    let response = state.http.post(server_url).json(&body).send().await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("ntfy returned {}", response.status()))
    }
}

pub(crate) fn android_endpoint_state(state: &HubState) -> AndroidEndpointState {
    if android_endpoint_exists(state).unwrap_or(false) {
        AndroidEndpointState::DeliveryNotImplemented
    } else {
        AndroidEndpointState::NotConnected
    }
}

struct NtfyChannelState {
    reason: Option<&'static str>,
}

async fn ntfy_channel_state(state: &HubState) -> NtfyChannelState {
    let ntfy = &state.config.remote_confirmation.ntfy;
    if ntfy_not_configured(ntfy) {
        return NtfyChannelState {
            reason: Some("ntfy_not_configured"),
        };
    }
    match cached_ntfy_health(state).await {
        NtfyHealthStatus::Healthy => NtfyChannelState { reason: None },
        NtfyHealthStatus::Unhealthy => NtfyChannelState {
            reason: Some("ntfy_unhealthy"),
        },
        NtfyHealthStatus::Failed => NtfyChannelState {
            reason: Some("ntfy_health_check_failed"),
        },
    }
}

async fn cached_ntfy_health(state: &HubState) -> NtfyHealthStatus {
    const NTFY_HEALTH_CACHE_TTL_SECS: i64 = 45;

    let server_url = state
        .config
        .remote_confirmation
        .ntfy
        .server_url
        .trim_end_matches('/')
        .to_string();
    let now = Utc::now();
    if let Some(cached) = state.ntfy_health.lock().await.clone() {
        if cached.server_url == server_url
            && now.signed_duration_since(cached.checked_at).num_seconds()
                < NTFY_HEALTH_CACHE_TTL_SECS
        {
            return cached.result;
        }
    }

    let result = check_ntfy_health(state, &server_url).await;
    *state.ntfy_health.lock().await = Some(NtfyHealthCache {
        server_url,
        checked_at: now,
        result: result.clone(),
    });
    result
}

async fn check_ntfy_health(state: &HubState, server_url: &str) -> NtfyHealthStatus {
    let url = format!("{server_url}/v1/health");
    let response = match timeout(Duration::from_secs(3), state.http.get(url).send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => return NtfyHealthStatus::Failed,
        Err(_) => return NtfyHealthStatus::Failed,
    };
    if !response.status().is_success() {
        return NtfyHealthStatus::Failed;
    }
    match response.json::<Value>().await {
        Ok(value) if value.get("healthy").and_then(Value::as_bool) == Some(true) => {
            NtfyHealthStatus::Healthy
        }
        Ok(_) => NtfyHealthStatus::Unhealthy,
        Err(_) => NtfyHealthStatus::Failed,
    }
}

fn ntfy_not_configured(ntfy: &NtfyConfig) -> bool {
    let server_url = ntfy.server_url.trim().trim_end_matches('/');
    let topic = ntfy.topic.trim();
    server_url.is_empty()
        || topic.is_empty()
        || (server_url == "https://ntfy.example.invalid" && topic == "change-me-high-entropy-topic")
}

fn ntfy_priority(priority: Option<&str>) -> i32 {
    match priority {
        Some("min") | Some("low") => 2,
        Some("high") => 4,
        Some("urgent") | Some("alarm") => 5,
        _ => 3,
    }
}

fn android_endpoint_exists(state: &HubState) -> Result<bool> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "select exists(select 1 from notification_endpoints where kind = 'android' and enabled = 1)",
    )?;
    let exists: i64 = stmt.query_row([], |row| row.get(0))?;
    Ok(exists != 0)
}

fn register_android_endpoint(state: &HubState, payload: AndroidRegisterRequest) -> Result<Value> {
    let endpoint_id = random_id("android");
    let token = random_token();
    let token_hash = sha256_hex(&token);
    let now = Utc::now().to_rfc3339();
    let display_name = payload
        .display_name
        .unwrap_or_else(|| "Android endpoint".to_string());
    let capabilities = if payload.capabilities.is_null() {
        json!({})
    } else {
        payload.capabilities
    };
    let conn = state.db.lock().unwrap();
    conn.execute(
        "insert into notification_endpoints(endpoint_id, kind, display_name, capabilities_json, token_hash, enabled, last_seen_at, created_at)
         values (?1, 'android', ?2, ?3, ?4, 1, ?5, ?5)",
        params![
            endpoint_id,
            display_name,
            serde_json::to_string(&capabilities)?,
            token_hash,
            now
        ],
    )?;
    Ok(json!({
        "endpointId": endpoint_id,
        "token": token,
        "status": "registered",
        "deliveryImplemented": false
    }))
}

fn notify_route_error_response(error: NotifyRouteError) -> Response {
    match error {
        NotifyRouteError::InvalidChannel(channel) => api_error(
            StatusCode::BAD_REQUEST,
            "invalid_notify_channel",
            format!("Invalid notification channel: {channel}"),
        ),
        NotifyRouteError::AgentNotFound(alias) => api_error(
            StatusCode::NOT_FOUND,
            "agent_alias_not_found",
            format!("No enabled agent found for alias: {alias}"),
        ),
        NotifyRouteError::ChannelUnavailable {
            channel_key,
            reason,
        } => api_error(
            StatusCode::BAD_REQUEST,
            reason,
            format!("Notification channel {channel_key} is unavailable: {reason}"),
        ),
        NotifyRouteError::DeliveryFailed {
            channel_key,
            reason,
        } => api_error(
            StatusCode::BAD_GATEWAY,
            "notify_delivery_failed",
            format!("Notification delivery failed for {channel_key}: {reason}"),
        ),
        NotifyRouteError::Db(reason) => {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", reason)
        }
    }
}

async fn forward_room_command(
    state: HubState,
    headers: HeaderMap,
    command: HubCommand,
    timeout_code: &'static str,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match request_active_room(&state, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(RoomRouteError::NotActive) => api_error(
            StatusCode::NOT_FOUND,
            "room_not_active",
            "no active room agent",
        ),
        Err(RoomRouteError::StateConflict) => api_error(
            StatusCode::CONFLICT,
            "room_state_conflict",
            "active room state is inconsistent",
        ),
        Err(RoomRouteError::Timeout(reason)) => {
            api_error(StatusCode::GATEWAY_TIMEOUT, timeout_code, reason)
        }
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
            } => match register_connection_role(&state, &agent_id, &connection_id, role).await {
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
            },
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
        release_active_room_if_current(&state, &agent_id, &connection_id).await;
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RoomRouteError {
    NotActive,
    StateConflict,
    Timeout(String),
}

pub(crate) async fn request_active_room(
    state: &HubState,
    command: HubCommand,
    timeout_secs: u64,
) -> std::result::Result<Value, RoomRouteError> {
    let active = state
        .active_room
        .lock()
        .await
        .clone()
        .ok_or(RoomRouteError::NotActive)?;
    let valid = {
        let agents = state.agents.lock().await;
        agents
            .get(&active.agent_id)
            .map(|connection| {
                connection.connection_id == active.connection_id
                    && connection.role == AgentRole::Room
            })
            .unwrap_or(false)
    };
    if !valid {
        return Err(RoomRouteError::StateConflict);
    }
    request_agent(state, &active.agent_id, command, timeout_secs)
        .await
        .map_err(RoomRouteError::Timeout)
}

async fn register_connection_role(
    state: &HubState,
    agent_id: &str,
    connection_id: &str,
    role: AgentRole,
) -> std::result::Result<(), &'static str> {
    match role {
        AgentRole::Normal => {
            release_active_room_for_agent(state, agent_id).await;
            Ok(())
        }
        AgentRole::Room => {
            let mut active = state.active_room.lock().await;
            match active.as_ref() {
                None => {
                    *active = Some(ActiveRoomConnection {
                        agent_id: agent_id.to_string(),
                        connection_id: connection_id.to_string(),
                    });
                    Ok(())
                }
                Some(current) if current.agent_id == agent_id => {
                    *active = Some(ActiveRoomConnection {
                        agent_id: agent_id.to_string(),
                        connection_id: connection_id.to_string(),
                    });
                    Ok(())
                }
                Some(_) => Err("room_already_active"),
            }
        }
    }
}

async fn replace_agent_connection(
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
                last_seen_at: Utc::now(),
                role: AgentRole::Normal,
                config_summary: None,
                notification_channels: Vec::new(),
            },
        )
    };
    if let Some(old) = old {
        release_active_room_if_current(state, agent_id, &old.connection_id).await;
        let _ = old.sender.send(Message::Close(None));
    }
}

async fn release_active_room_if_current(state: &HubState, agent_id: &str, connection_id: &str) {
    let mut active = state.active_room.lock().await;
    let should_release = active
        .as_ref()
        .map(|current| current.agent_id == agent_id && current.connection_id == connection_id)
        .unwrap_or(false);
    if should_release {
        *active = None;
    }
}

async fn release_active_room_for_agent(state: &HubState, agent_id: &str) {
    let mut active = state.active_room.lock().await;
    if active
        .as_ref()
        .map(|current| current.agent_id == agent_id)
        .unwrap_or(false)
    {
        *active = None;
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
        | HubCommand::McpCallTool { request_id, .. }
        | HubCommand::UserNotifyDeliver { request_id, .. }
        | HubCommand::RoomNotebookAppend { request_id, .. }
        | HubCommand::RoomNotebookRecent { request_id, .. }
        | HubCommand::RoomNotebookSelectExact { request_id, .. }
        | HubCommand::RoomNotebookSearch { request_id, .. }
        | HubCommand::RoomNotebookCurrent { request_id, .. }
        | HubCommand::RoomNotebookUpdate { request_id, .. }
        | HubCommand::RoomNotebookRemove { request_id, .. } => request_id,
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
        | HubCommand::McpCallTool { request_id, .. }
        | HubCommand::UserNotifyDeliver { request_id, .. }
        | HubCommand::RoomNotebookAppend { request_id, .. }
        | HubCommand::RoomNotebookRecent { request_id, .. }
        | HubCommand::RoomNotebookSelectExact { request_id, .. }
        | HubCommand::RoomNotebookSearch { request_id, .. }
        | HubCommand::RoomNotebookCurrent { request_id, .. }
        | HubCommand::RoomNotebookUpdate { request_id, .. }
        | HubCommand::RoomNotebookRemove { request_id, .. } => *request_id = value,
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
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: Vec::new(),
        },
        policy_rule_counts: agentic_gpt_protocol::PolicyCounts {
            allow: 0,
            confirm: 0,
            deny: 0,
        },
        policy_rules: SafePolicyRules {
            allow: Vec::new(),
            confirm: Vec::new(),
            deny: Vec::new(),
            builtins: SafeBuiltinPolicyRules {
                confirm: Vec::new(),
                deny: Vec::new(),
            },
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
        create table if not exists notification_endpoints (
            endpoint_id text primary key,
            kind text not null,
            display_name text,
            capabilities_json text not null,
            token_hash text not null,
            enabled integer not null,
            last_seen_at text,
            created_at text not null
        );
        ",
    )?;
    ensure_column(conn, "agents", "alias", "alias text")?;
    conn.execute_batch(
        "create unique index if not exists agents_alias_unique on agents(alias) where alias is not null;",
    )?;
    Ok(())
}

fn handle_agent_command(conn: &Connection, command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Add {
            agent_id,
            alias,
            display_name,
            secret,
        } => {
            let alias = normalize_alias(alias.as_deref())?;
            let capabilities = Capabilities {
                sessions: true,
                confirmation: true,
                notification_actions: true,
            };
            conn.execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values (?1, ?2, ?3, 1, ?4, null, ?5)
                 on conflict(agent_id) do update set display_name = excluded.display_name,
                     alias = coalesce(excluded.alias, agents.alias),
                     enabled = 1, secret_hash = excluded.secret_hash, capabilities_json = excluded.capabilities_json",
                params![
                    agent_id,
                    alias,
                    display_name,
                    sha256_hex(&secret),
                    serde_json::to_string(&capabilities)?
                ],
            )?;
            println!("agent saved");
        }
        AgentCommand::Alias { command } => match command {
            AgentAliasCommand::Set { agent_id, alias } => {
                let alias = normalize_alias(Some(&alias))?;
                conn.execute(
                    "update agents set alias = ?2 where agent_id = ?1",
                    params![agent_id, alias],
                )?;
                println!("agent alias saved");
            }
            AgentAliasCommand::Clear { agent_id } => {
                conn.execute(
                    "update agents set alias = null where agent_id = ?1",
                    params![agent_id],
                )?;
                println!("agent alias cleared");
            }
        },
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
                    "{}\talias={}\t{}\tenabled={}\tlastSeenAt={}",
                    entry.agent_id,
                    entry.alias.as_deref().unwrap_or("-"),
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
        "select agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json from agents order by agent_id",
    )?;
    let rows = stmt.query_map([], row_to_entry)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn registry_entry(state: &HubState, agent_id: &str) -> Result<Option<AgentRegistryEntry>> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "select agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json from agents where agent_id = ?1",
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
    let last_seen: Option<String> = row.get(5)?;
    let capabilities_json: String = row.get(6)?;
    Ok(AgentRegistryEntry {
        agent_id: row.get(0)?,
        alias: row.get(1)?,
        display_name: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        secret_hash: row.get(4)?,
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

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("pragma table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    conn.execute(&format!("alter table {table} add column {definition}"), [])?;
    Ok(())
}

fn normalize_alias(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let alias = value.trim();
    if alias.is_empty() {
        return Ok(None);
    }
    let valid = alias
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'));
    if !valid {
        return Err(anyhow::anyhow!(
            "alias may only contain ASCII letters, digits, underscore, or hyphen"
        ));
    }
    Ok(Some(alias.to_string()))
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

    fn test_hub_config() -> HubConfig {
        HubConfig {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: true,
                provider: "ntfy".to_string(),
                timeout_seconds: 45,
                ntfy: NtfyConfig {
                    server_url: "https://ntfy.example.invalid".to_string(),
                    topic: "secret-topic-for-test".to_string(),
                    callback_base_url: "https://callback.example.invalid".to_string(),
                },
            },
        }
    }

    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(test_hub_config()),
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: Some("https://hub.example.invalid".to_string()),
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(Some(NtfyHealthCache {
                server_url: "https://ntfy.example.invalid".to_string(),
                checked_at: Utc::now(),
                result: NtfyHealthStatus::Healthy,
            }))),
        }
    }

    async fn insert_connection(
        state: &HubState,
        agent_id: &str,
        connection_id: &str,
        role: AgentRole,
    ) -> mpsc::UnboundedReceiver<Message> {
        let (tx, rx) = mpsc::unbounded_channel();
        state.agents.lock().await.insert(
            agent_id.to_string(),
            AgentConnection {
                connection_id: connection_id.to_string(),
                sender: tx,
                last_seen_at: Utc::now(),
                role,
                config_summary: None,
                notification_channels: Vec::new(),
            },
        );
        rx
    }

    async fn replace_connection(
        state: &HubState,
        agent_id: &str,
        connection_id: &str,
    ) -> mpsc::UnboundedReceiver<Message> {
        let (tx, rx) = mpsc::unbounded_channel();
        replace_agent_connection(state, agent_id, connection_id, tx).await;
        rx
    }

    #[tokio::test]
    async fn hub_info_reports_safe_runtime_summary() {
        let state = test_state();
        let response = build_hub_info_response(&state).await.unwrap();
        let value = serde_json::to_value(response).unwrap();
        let text = serde_json::to_string(&value).unwrap();

        assert_eq!(value["service"], "agentic-gpt-hub");
        assert_eq!(value["publicBaseUrl"], "https://hub.example.invalid");
        assert_eq!(value["remoteConfirmation"]["enabled"], true);
        assert_eq!(value["remoteConfirmation"]["provider"], "ntfy");
        assert_eq!(value["remoteConfirmation"]["ntfyConfigured"], true);
        assert_eq!(value["agents"]["registeredCount"], 0);
        assert_eq!(value["agents"]["onlineCount"], 0);
        assert_eq!(value["counts"]["pendingRequestCount"], 0);
        assert!(!text.contains("secret-topic-for-test"));
        assert!(!text.contains("callback.example.invalid"));
        assert!(!text.contains("test-api-key"));
    }

    #[test]
    fn openapi_documents_info_and_path_policy() {
        let openapi_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../openapi/hub.yaml");
        let openapi = std::fs::read_to_string(openapi_path).unwrap();

        assert!(openapi.contains("/v1/info:"));
        assert!(openapi.contains("HubInfoResponse:"));
        assert!(openapi.contains("pathPolicy:"));
        assert!(openapi.contains("writeRootCount"));
        assert!(openapi.contains("readOnlyRootCount"));
        assert!(openapi.contains("denyRootCount"));
        assert!(openapi.contains("writeRoots"));
        assert!(openapi.contains("readOnlyRoots"));
        assert!(openapi.contains("denyRoots"));
        assert!(openapi.contains("SafePathRoot:"));
        assert!(openapi.contains("policyRules:"));
        assert!(openapi.contains("SafePolicyRules:"));
        assert!(openapi.contains("SafeRule:"));
        assert!(openapi.contains("argsPrefix"));
        assert!(openapi.contains("workingDirectory"));
        assert!(openapi.contains("Optional command working directory"));
        assert!(openapi.contains("Optional default working directory for all batch elements"));
    }

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
        assert!(summary.path_policy.write_roots.is_empty());
        assert!(summary.path_policy.read_only_roots.is_empty());
        assert!(summary.path_policy.deny_roots.is_empty());
        assert!(summary.policy_rules.allow.is_empty());
        assert!(summary.policy_rules.confirm.is_empty());
        assert!(summary.policy_rules.deny.is_empty());
        assert!(summary.policy_rules.builtins.confirm.is_empty());
        assert!(summary.policy_rules.builtins.deny.is_empty());
    }

    #[test]
    fn openapi_room_notebook_schemas_do_not_include_agent_id_or_deferred_apis() {
        let openapi = include_str!("../../../openapi/hub.yaml");
        for schema in [
            "NotebookAppendRequest:",
            "NotebookRecentRequest:",
            "NotebookSelectExactRequest:",
            "NotebookSearchRequest:",
            "NotebookCurrentRequest:",
            "NotebookUpdateRequest:",
            "NotebookRemoveRequest:",
        ] {
            let mut in_section = false;
            let mut section = String::new();
            for line in openapi.lines() {
                if line.trim() == schema {
                    in_section = true;
                    section.push_str(line);
                    section.push('\n');
                    continue;
                }
                if in_section && line.starts_with("    ") && !line.starts_with("      ") {
                    break;
                }
                if in_section {
                    section.push_str(line);
                    section.push('\n');
                }
            }
            assert!(
                !section.contains("agentId"),
                "{schema} unexpectedly contains agentId"
            );
        }
        for forbidden in ["recentWeek", "recentMonth", "selectPast"] {
            assert!(!openapi.contains(forbidden));
        }
        assert!(openapi.contains("roomNotebookUpdate"));
        assert!(openapi.contains("roomNotebookRemove"));
    }

    #[tokio::test]
    async fn first_room_agent_becomes_active_room() {
        let state = test_state();
        let _rx = insert_connection(&state, "room", "conn1", AgentRole::Normal).await;
        register_connection_role(&state, "room", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let active = state.active_room.lock().await.clone().unwrap();
        assert_eq!(active.agent_id, "room");
        assert_eq!(active.connection_id, "conn1");
    }

    #[tokio::test]
    async fn second_different_room_agent_is_rejected() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room-a", "conn1", AgentRole::Room).await;
        register_connection_role(&state, "room-a", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = insert_connection(&state, "room-b", "conn2", AgentRole::Normal).await;
        assert_eq!(
            register_connection_role(&state, "room-b", "conn2", AgentRole::Room).await,
            Err("room_already_active")
        );
    }

    #[tokio::test]
    async fn same_room_agent_reconnect_replaces_old_room_connection() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "new").await;
        assert!(state.active_room.lock().await.is_none());
        register_connection_role(&state, "room", "new", AgentRole::Room)
            .await
            .unwrap();
        let active = state.active_room.lock().await.clone().unwrap();
        assert_eq!(active.connection_id, "new");
    }

    #[tokio::test]
    async fn same_agent_normal_hello_releases_old_active_room() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "normal").await;
        register_connection_role(&state, "room", "normal", AgentRole::Normal)
            .await
            .unwrap();
        assert!(state.active_room.lock().await.is_none());
    }

    #[tokio::test]
    async fn same_agent_replacement_without_hello_does_not_leave_stale_active_room() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "new-no-hello").await;
        assert!(state.active_room.lock().await.is_none());
        release_active_room_if_current(&state, "room", "new-no-hello").await;
        assert!(state.active_room.lock().await.is_none());
    }

    #[tokio::test]
    async fn room_api_after_replacement_without_hello_returns_not_active() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = replace_connection(&state, "room", "new-no-hello").await;
        let result = request_active_room(
            &state,
            HubCommand::RoomNotebookCurrent {
                request_id: "req".to_string(),
                payload: NotebookCurrentRequest {
                    scope: "agentic".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }

    #[tokio::test]
    async fn stale_room_disconnect_does_not_release_new_room_connection() {
        let state = test_state();
        let _rx1 = insert_connection(&state, "room", "old", AgentRole::Room).await;
        register_connection_role(&state, "room", "old", AgentRole::Room)
            .await
            .unwrap();
        let _rx2 = insert_connection(&state, "room", "new", AgentRole::Room).await;
        register_connection_role(&state, "room", "new", AgentRole::Room)
            .await
            .unwrap();
        release_active_room_if_current(&state, "room", "old").await;
        let active = state.active_room.lock().await.clone().unwrap();
        assert_eq!(active.connection_id, "new");
    }

    #[tokio::test]
    async fn room_api_without_active_room_returns_not_active() {
        let state = test_state();
        let result = request_active_room(
            &state,
            HubCommand::RoomNotebookCurrent {
                request_id: "req".to_string(),
                payload: NotebookCurrentRequest {
                    scope: "agentic".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }

    #[tokio::test]
    async fn update_remove_room_api_without_active_room_returns_not_active() {
        let state = test_state();
        let update = request_active_room(
            &state,
            HubCommand::RoomNotebookUpdate {
                request_id: "req-update".to_string(),
                payload: NotebookUpdateRequest {
                    id: "psg_missing".to_string(),
                    significance: None,
                    abstract_text: Some("updated".to_string()),
                    content: None,
                    tags: None,
                },
            },
            1,
        )
        .await;
        assert_eq!(update.unwrap_err(), RoomRouteError::NotActive);
        let remove = request_active_room(
            &state,
            HubCommand::RoomNotebookRemove {
                request_id: "req-remove".to_string(),
                payload: NotebookRemoveRequest {
                    id: "psg_missing".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(remove.unwrap_err(), RoomRouteError::NotActive);
    }

    #[tokio::test]
    async fn room_api_routes_to_active_room_connection() {
        let state = test_state();
        let mut rx = insert_connection(&state, "room", "conn1", AgentRole::Room).await;
        register_connection_role(&state, "room", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            request_active_room(
                &request_state,
                HubCommand::RoomNotebookCurrent {
                    request_id: "req".to_string(),
                    payload: NotebookCurrentRequest {
                        scope: "agentic".to_string(),
                    },
                },
                5,
            )
            .await
            .unwrap()
        });
        let Message::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text command");
        };
        let command = serde_json::from_str::<HubCommand>(&text).unwrap();
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::RoomNotebookCurrent { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({ "current": null, "warnings": [] }))
            .unwrap();
        let value = task.await.unwrap();
        assert_eq!(value["current"], Value::Null);
    }

    #[tokio::test]
    async fn update_remove_room_api_routes_to_active_room_connection() {
        let state = test_state();
        let mut rx = insert_connection(&state, "room", "conn1", AgentRole::Room).await;
        register_connection_role(&state, "room", "conn1", AgentRole::Room)
            .await
            .unwrap();
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            request_active_room(
                &request_state,
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
                5,
            )
            .await
            .unwrap()
        });
        let Message::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text command");
        };
        let command = serde_json::from_str::<HubCommand>(&text).unwrap();
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::RoomNotebookUpdate { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({ "updated": true, "id": "psg_1", "warnings": [] }))
            .unwrap();
        let value = task.await.unwrap();
        assert_eq!(value["updated"], true);

        let request_state = state.clone();
        let task = tokio::spawn(async move {
            request_active_room(
                &request_state,
                HubCommand::RoomNotebookRemove {
                    request_id: "req-remove".to_string(),
                    payload: NotebookRemoveRequest {
                        id: "psg_1".to_string(),
                    },
                },
                5,
            )
            .await
            .unwrap()
        });
        let Message::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected text command");
        };
        let command = serde_json::from_str::<HubCommand>(&text).unwrap();
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::RoomNotebookRemove { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({ "removed": true, "id": "psg_1", "warnings": [] }))
            .unwrap();
        let value = task.await.unwrap();
        assert_eq!(value["removed"], true);
    }

    #[tokio::test]
    async fn normal_agent_is_not_room_api_fallback() {
        let state = test_state();
        let _rx = insert_connection(&state, "normal", "conn1", AgentRole::Normal).await;
        let result = request_active_room(
            &state,
            HubCommand::RoomNotebookCurrent {
                request_id: "req".to_string(),
                payload: NotebookCurrentRequest {
                    scope: "agentic".to_string(),
                },
            },
            1,
        )
        .await;
        assert_eq!(result.unwrap_err(), RoomRouteError::NotActive);
    }

    #[test]
    fn parses_notify_channel_keys() {
        assert_eq!(
            parse_notify_channel_key("agent::laptop::freedesktop").unwrap(),
            NotifyChannelKey::AgentFreedesktop {
                alias: "laptop".to_string()
            }
        );
        assert_eq!(
            parse_notify_channel_key("hub::ntfy").unwrap(),
            NotifyChannelKey::HubNtfy
        );
        assert_eq!(
            parse_notify_channel_key("hub::android::notice").unwrap(),
            NotifyChannelKey::AndroidNotice
        );
        assert!(parse_notify_channel_key("agent::::freedesktop").is_err());
        assert!(parse_notify_channel_key("room::notify").is_err());
    }

    #[test]
    fn agent_alias_is_nullable_and_unique_when_present() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let capabilities = serde_json::to_string(&Capabilities {
            sessions: true,
            confirmation: true,
            notification_actions: false,
        })
        .unwrap();
        conn.execute(
            "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
             values ('a', null, 'A', 1, 'hash-a', null, ?1)",
            params![capabilities],
        )
        .unwrap();
        conn.execute(
            "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
             values ('b', null, 'B', 1, 'hash-b', null, ?1)",
            params![capabilities],
        )
        .unwrap();
        conn.execute(
            "update agents set alias = 'laptop' where agent_id = 'a'",
            [],
        )
        .unwrap();
        let duplicate = conn.execute(
            "update agents set alias = 'laptop' where agent_id = 'b'",
            [],
        );
        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn notification_channels_include_agent_ntfy_and_android_placeholders() {
        let state = test_state();
        {
            let conn = state.db.lock().unwrap();
            let capabilities = serde_json::to_string(&Capabilities {
                sessions: true,
                confirmation: true,
                notification_actions: false,
            })
            .unwrap();
            conn.execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values ('agentic-gpt-slhaf-laptop', 'laptop', 'Laptop', 1, 'hash', null, ?1)",
                params![capabilities],
            )
            .unwrap();
        }
        let _rx = insert_connection(
            &state,
            "agentic-gpt-slhaf-laptop",
            "conn1",
            AgentRole::Normal,
        )
        .await;
        state
            .agents
            .lock()
            .await
            .get_mut("agentic-gpt-slhaf-laptop")
            .unwrap()
            .notification_channels
            .push(NotificationChannel {
                key: "agent::agentic-gpt-slhaf-laptop::freedesktop".to_string(),
                display_name: "Desktop".to_string(),
                available: true,
                kind: "freedesktop".to_string(),
                supports_actions: false,
                reason: None,
                agent_id: Some("agentic-gpt-slhaf-laptop".to_string()),
            });

        let channels = notification_channels(&state).await.unwrap();
        assert!(channels
            .iter()
            .any(|channel| channel.key == "agent::laptop::freedesktop"));
        assert!(channels.iter().any(|channel| channel.key == "hub::ntfy"));
        let android = channels
            .iter()
            .find(|channel| channel.key == "hub::android::notice")
            .unwrap();
        assert!(!android.available);
        assert_eq!(
            android.reason.as_deref(),
            Some("android_endpoint_not_connected")
        );
    }

    #[tokio::test]
    async fn ntfy_default_placeholder_is_not_configured() {
        let mut state = test_state();
        state.config = Arc::new(HubConfig::default_config());
        let channels = notification_channels(&state).await.unwrap();
        let ntfy = channels
            .iter()
            .find(|channel| channel.key == "hub::ntfy")
            .unwrap();
        assert!(!ntfy.available);
        assert_eq!(ntfy.reason.as_deref(), Some("ntfy_not_configured"));
    }

    #[tokio::test]
    async fn ntfy_health_cache_controls_listing_reason() {
        let state = test_state();
        *state.ntfy_health.lock().await = Some(NtfyHealthCache {
            server_url: "https://ntfy.example.invalid".to_string(),
            checked_at: Utc::now(),
            result: NtfyHealthStatus::Unhealthy,
        });
        let channels = notification_channels(&state).await.unwrap();
        let ntfy = channels
            .iter()
            .find(|channel| channel.key == "hub::ntfy")
            .unwrap();
        assert!(!ntfy.available);
        assert_eq!(ntfy.reason.as_deref(), Some("ntfy_unhealthy"));

        *state.ntfy_health.lock().await = Some(NtfyHealthCache {
            server_url: "https://ntfy.example.invalid".to_string(),
            checked_at: Utc::now(),
            result: NtfyHealthStatus::Failed,
        });
        let channels = notification_channels(&state).await.unwrap();
        let ntfy = channels
            .iter()
            .find(|channel| channel.key == "hub::ntfy")
            .unwrap();
        assert!(!ntfy.available);
        assert_eq!(ntfy.reason.as_deref(), Some("ntfy_health_check_failed"));
    }

    #[tokio::test]
    async fn android_registered_endpoint_still_reports_delivery_not_implemented() {
        let state = test_state();
        register_android_endpoint(
            &state,
            AndroidRegisterRequest {
                display_name: Some("Phone".to_string()),
                capabilities: json!({ "channels": ["notice", "alarm"] }),
            },
        )
        .unwrap();
        let channels = notification_channels(&state).await.unwrap();
        let android = channels
            .iter()
            .find(|channel| channel.key == "hub::android::alarm")
            .unwrap();
        assert!(!android.available);
        assert_eq!(
            android.reason.as_deref(),
            Some("android_delivery_not_implemented")
        );
        let error = send_user_notification(
            &state,
            UserNotifySendRequest {
                channel_key: "hub::android::alarm".to_string(),
                title: "Wake".to_string(),
                body: "Up".to_string(),
                actions: Vec::new(),
                priority: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            NotifyRouteError::ChannelUnavailable {
                channel_key: "hub::android::alarm".to_string(),
                reason: "android_delivery_not_implemented"
            }
        );
    }

    #[tokio::test]
    async fn user_notify_send_routes_agent_channel_by_alias() {
        let state = test_state();
        {
            let conn = state.db.lock().unwrap();
            let capabilities = serde_json::to_string(&Capabilities {
                sessions: true,
                confirmation: true,
                notification_actions: false,
            })
            .unwrap();
            conn.execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values ('agentic-gpt-slhaf-laptop', 'laptop', 'Laptop', 1, 'hash', null, ?1)",
                params![capabilities],
            )
            .unwrap();
        }
        let mut rx = insert_connection(
            &state,
            "agentic-gpt-slhaf-laptop",
            "conn1",
            AgentRole::Normal,
        )
        .await;
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            send_user_notification(
                &request_state,
                UserNotifySendRequest {
                    channel_key: "agent::laptop::freedesktop".to_string(),
                    title: "Hello".to_string(),
                    body: "World".to_string(),
                    actions: Vec::new(),
                    priority: None,
                },
            )
            .await
            .unwrap()
        });
        let Message::Text(text) = rx.recv().await.unwrap() else {
            panic!("expected command");
        };
        let command = serde_json::from_str::<HubCommand>(&text).unwrap();
        let request_id = command_request_id(&command).to_string();
        assert!(matches!(command, HubCommand::UserNotifyDeliver { .. }));
        let sender = state.pending.lock().await.remove(&request_id).unwrap();
        sender
            .send(json!({
                "channelKey": "agent::laptop::freedesktop",
                "delivered": true
            }))
            .unwrap();
        let response = task.await.unwrap();
        assert!(response.accepted);
        assert_eq!(response.channel_key, "agent::laptop::freedesktop");
    }
}
