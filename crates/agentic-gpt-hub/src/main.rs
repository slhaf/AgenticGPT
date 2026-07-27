mod agentic_result;
mod agents;
mod db;
mod instance_lock;
mod mcp_server;
mod notify;
mod oauth;
mod registry;
mod room;
mod routes;
mod runs;
mod state;
mod utils;

use agentic_gpt_protocol::{
    ConfirmationDecision, ConfirmationPayload, HubMessage, SafeBuiltinPolicyRules,
    SafeConfigSummary, SafePathPolicySummary, SafePolicyRules, SafeSandboxSummary,
};
use anyhow::{Context, Result};
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::db::{init_db, open_db};
use crate::registry::handle_agent_command;
use crate::routes::api_error;
use crate::state::{HubState, McpProfile, OutboundAgentMessage, PendingConfirmation};
use crate::utils::{constant_time_equal, random_id, random_token, sha256_hex};

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
        #[arg(
            long,
            env = "AGENTIC_GPT_HUB_MCP_PROFILE",
            value_enum,
            default_value_t = McpProfile::Full
        )]
        mcp_profile: McpProfile,
    },
    Agent {
        #[command(subcommand)]
        command: registry::AgentCommand,
    },
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

#[derive(Deserialize)]
struct ConfirmationCallbackQuery {
    token: String,
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
    match cli.command {
        HubCommandCli::Init => {
            let conn = open_db(&db_path)?;
            init_db(&conn)?;
            config.write_if_missing(&config_path)?;
            println!("initialized {}", db_path.display());
            println!("config {}", config_path.display());
        }
        HubCommandCli::Serve {
            bind,
            api_key,
            public_base_url,
            mcp_profile,
        } => {
            let _instance_lock =
                instance_lock::InstanceLock::acquire(&db_path, ".serve.lock", "hub")?;
            config.write_if_missing(&config_path)?;
            let conn = open_db(&db_path)?;
            init_db(&conn)?;
            serve(bind, api_key, public_base_url, mcp_profile, conn, config).await?;
        }
        HubCommandCli::Agent { command } => {
            let conn = open_db(&db_path)?;
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
    mcp_profile: McpProfile,
    conn: Connection,
    config: HubConfig,
) -> Result<()> {
    let state = HubState {
        api_key,
        db: Arc::new(StdMutex::new(conn)),
        config: Arc::new(config),
        mcp_profile,
        agents: Arc::new(Mutex::new(HashMap::new())),
        pending: Arc::new(Mutex::new(HashMap::new())),
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        jobs: Arc::new(Mutex::new(HashMap::new())),
        boot_generations: Arc::new(Mutex::new(HashMap::new())),
        active_room: Arc::new(Mutex::new(None)),
        http: reqwest::Client::new(),
        public_base_url: public_base_url.map(|value| value.trim_end_matches('/').to_string()),
        oauth_codes: Arc::new(Mutex::new(HashMap::new())),
        oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
        ntfy_health: Arc::new(Mutex::new(None)),
    };
    tokio::spawn(cleanup_confirmations(state.clone()));
    tokio::spawn(cleanup_runs(state.clone()));
    tokio::spawn(agents::cleanup_agent_connections(state.clone()));
    tokio::spawn(oauth::cleanup_oauth(state.clone()));
    let app = Router::new()
        .route("/v1/info", get(routes::hub_info))
        .route("/v1/agents", get(routes::list_agents))
        .route("/v1/agents/:agent_id/connect", get(agents::connect_agent))
        .route(
            "/v1/agents/:agent_id/events",
            get(agents::connect_agent_sse),
        )
        .route(
            "/v1/agents/:agent_id/messages",
            post(agents::post_agent_message),
        )
        .route("/v1/runs/:run_id", get(routes::get_run))
        .route(
            "/v1/confirmations/:confirmation_id/:decision",
            post(confirmation_callback),
        )
        .route("/v1/process/exec", post(routes::process_exec))
        .route("/v1/process/batch", post(routes::process_batch))
        .route("/v1/jobs", get(routes::list_jobs))
        .route("/v1/jobs/:job_id", get(routes::get_job))
        .route("/v1/jobs/:job_id/cancel", post(routes::cancel_job))
        .route("/v1/tmux/sessions", get(routes::tmux_list_sessions))
        .route("/v1/tmux/panes", get(routes::tmux_list_panes))
        .route("/v1/tmux/capture", post(routes::tmux_capture_pane))
        .route("/v1/tmux/exec", post(routes::tmux_exec))
        .route("/v1/tmux/paste", post(routes::tmux_paste_text))
        .route(
            "/v1/tmux/sessions/create",
            post(routes::tmux_create_session),
        )
        .route("/v1/tmux/sessions/close", post(routes::tmux_close_session))
        .route("/v1/mcp/servers", post(routes::mcp_list_servers))
        .route("/v1/mcp/tools", post(routes::mcp_list_tools))
        .route("/v1/mcp/callTool", post(routes::mcp_call_tool))
        .route("/v1/mcp/batch", post(routes::mcp_batch))
        .route("/v1/notify/channels", get(notify::notify_channels))
        .route("/v1/notify/send", post(notify::notify_send))
        .route(
            "/v1/notify/android/register",
            post(notify::android_notify_register),
        )
        .route("/v1/room/notebook/append", post(room::room_notebook_append))
        .route("/v1/room/notebook/recent", post(room::room_notebook_recent))
        .route(
            "/v1/room/notebook/selectExact",
            post(room::room_notebook_select_exact),
        )
        .route("/v1/room/notebook/search", post(room::room_notebook_search))
        .route(
            "/v1/room/notebook/current",
            post(room::room_notebook_current),
        )
        .route("/v1/room/notebook/update", post(room::room_notebook_update))
        .route("/v1/room/notebook/remove", post(room::room_notebook_remove))
        .route("/v1/room/bootstrap", post(room::room_bootstrap))
        .route("/v1/room/bootstrap/read", post(room::room_bootstrap_read))
        .route("/v1/room/skills/list", post(room::skills_list))
        .route("/v1/room/skills/read", post(room::skills_read))
        .route("/v1/room/skills/search", post(room::skills_search))
        .route("/v1/room/skills/active", post(room::skills_active))
        .route("/v1/room/skills/activate", post(room::skills_activate))
        .route("/v1/room/skills/deactivate", post(room::skills_deactivate))
        .route("/v1/room/skills/install", post(room::skills_install))
        .route(
            "/v1/room/skills/install/get",
            post(room::skills_install_get),
        )
        .route(
            "/v1/room/skills/install/cancel",
            post(room::skills_install_cancel),
        )
        .route("/v1/room/skills/run", post(room::skills_run))
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

async fn cleanup_runs(state: HubState) {
    loop {
        sleep(Duration::from_secs(30)).await;
        let older_than = Utc::now() - chrono::Duration::seconds((REQUEST_TIMEOUT_SECS * 2) as i64);
        if let Err(error) = runs::prune_expired(&state) {
            warn!(%error, "expired run cleanup failed");
        }
        match runs::mark_stale_acked_unknown(&state, older_than) {
            Ok(changed) if changed > 0 => {
                info!(changed, "marked stale acked runs unknown");
            }
            Ok(_) => {}
            Err(error) => warn!(%error, "run cleanup failed"),
        }
    }
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
    let actions = if matches!(
        payload.kind.as_deref(),
        Some("mcpTool" | "mcpBatchSingleServer")
    ) {
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
        let _ = sender.send(OutboundAgentMessage::Text(text));
    }
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
        tunnel: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::parse_bearer_token;
    use agentic_gpt_protocol::HubCommand;

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
            mcp_profile: McpProfile::Full,
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            boot_generations: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: Some("https://hub.example.invalid".to_string()),
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(Some(notify::NtfyHealthCache {
                server_url: "https://ntfy.example.invalid".to_string(),
                checked_at: Utc::now(),
                result: notify::NtfyHealthStatus::Healthy,
            }))),
        }
    }

    #[tokio::test]
    async fn hub_info_reports_safe_runtime_summary() {
        let state = test_state();
        let response = routes::build_hub_info_response(&state).await.unwrap();
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
        assert!(openapi.contains("/v1/process/exec:"));
        assert!(openapi.contains("/v1/process/batch:"));
        assert!(openapi.contains("/v1/jobs:"));
        assert!(openapi.contains("/v1/jobs/{jobId}:"));
        assert!(openapi.contains("/v1/jobs/{jobId}/cancel:"));
        assert!(openapi.contains("JobInfo:"));
        assert!(openapi.contains("JobResponse:"));
        assert!(openapi.contains("JobBatchResponse:"));
        assert!(openapi.contains("unknown_after_restart"));
        assert!(openapi.contains("/v1/mcp/callTool:"));
        assert!(openapi.contains("/v1/mcp/batch:"));
        assert!(openapi.contains("McpBatchRequest:"));
        assert!(openapi.contains("McpBatchResponse:"));
        assert!(openapi.contains("McpBatchChildResponse:"));
        assert!(openapi.contains("aggregateTruncated"));
        assert!(openapi.contains("already-started calls are never cancelled"));
        assert!(openapi.contains("McpCallToolRequest:"));
        assert!(openapi.contains("JobDetail:"));
        assert!(openapi.contains("JobResponse:"));
        assert!(!openapi.contains("allOf:"));
        assert!(openapi.contains("detailAvailable"));
        assert!(openapi.contains("resultSha256"));
        assert!(openapi.contains("resultPreview"));
        assert!(openapi.contains("Absolute downstream execution deadline"));
        assert!(openapi.contains("maximum serialized size 256 KiB"));
        assert!(openapi.contains("exceeds 512 KiB"));
        assert!(!openapi.contains("McpCallToolResponse:"));
        assert!(!openapi.contains("SkillRunResponse:"));
        for removed in [
            "/v1/exec:",
            "/v1/batchExec:",
            "/v1/sessions/start:",
            "/v1/sessions/{sessionId}:",
            "StartSessionResponse:",
            "SessionInfo:",
        ] {
            assert!(
                !openapi.contains(removed),
                "removed OpenAPI contract survived: {removed}"
            );
        }
        for tmux_path in [
            "/v1/tmux/sessions:",
            "/v1/tmux/panes:",
            "/v1/tmux/capture:",
            "/v1/tmux/exec:",
            "/v1/tmux/paste:",
            "/v1/tmux/sessions/create:",
            "/v1/tmux/sessions/close:",
        ] {
            assert!(openapi.contains(tmux_path));
        }
        assert!(openapi.contains("TmuxExecRequest:"));
        assert!(openapi.contains("isLikelyShell:"));
        assert!(openapi.contains("x-openai-isConsequential: true"));
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

    #[test]
    fn skills_commands_have_request_ids_and_run_types() {
        let mut commands = [
            HubCommand::SkillsList {
                request_id: "req-list".to_string(),
            },
            HubCommand::SkillsRead {
                request_id: "req-read".to_string(),
                payload: agentic_gpt_protocol::SkillReadRequest {
                    id: "demo".to_string(),
                    path: None,
                },
            },
            HubCommand::SkillsSearch {
                request_id: "req-search".to_string(),
                payload: agentic_gpt_protocol::SkillSearchRequest {
                    query: "demo".to_string(),
                    limit: None,
                },
            },
            HubCommand::SkillsActive {
                request_id: "req-active".to_string(),
            },
            HubCommand::SkillsActivate {
                request_id: "req-activate".to_string(),
                payload: agentic_gpt_protocol::SkillActivationRequest {
                    id: "demo".to_string(),
                },
            },
            HubCommand::SkillsDeactivate {
                request_id: "req-deactivate".to_string(),
                payload: agentic_gpt_protocol::SkillActivationRequest {
                    id: "demo".to_string(),
                },
            },
            HubCommand::SkillsInstall {
                request_id: "req-install".to_string(),
                payload: agentic_gpt_protocol::SkillInstallRequest {
                    id: "demo".to_string(),
                    source: agentic_gpt_protocol::SkillInstallSource::Files { files: vec![] },
                    replace_existing: false,
                    activate_after_install: None,
                    idempotency_key: None,
                },
            },
            HubCommand::SkillsInstallGet {
                request_id: "req-install-get".to_string(),
                payload: agentic_gpt_protocol::SkillInstallGetRequest {
                    install_id: "install-1".to_string(),
                    wait_seconds: Some(0),
                },
            },
            HubCommand::SkillsInstallCancel {
                request_id: "req-install-cancel".to_string(),
                payload: agentic_gpt_protocol::SkillInstallCancelRequest {
                    install_id: "install-1".to_string(),
                },
            },
            HubCommand::SkillsRun {
                request_id: "req-run".to_string(),
                payload: agentic_gpt_protocol::SkillRunRequest {
                    id: "demo".to_string(),
                    path: "scripts/check.sh".to_string(),
                    args: None,
                    working_directory: None,
                    wait_seconds: Some(0),
                },
            },
        ];
        let expected = [
            "skills.list",
            "skills.read",
            "skills.search",
            "skills.active",
            "skills.activate",
            "skills.deactivate",
            "skills.install",
            "skills.install.get",
            "skills.install.cancel",
            "skills.run",
        ];

        for index in 0..commands.len() {
            let command = &mut commands[index];
            let expected_type = expected[index];
            assert_eq!(crate::runs::command_type(command), expected_type);
            crate::agents::set_command_request_id(command, "req-new".to_string());
            assert_eq!(crate::agents::command_request_id(command), "req-new");
        }
    }

    #[test]
    fn bootstrap_commands_have_request_ids_and_run_types() {
        let mut commands = [
            HubCommand::RoomBootstrap {
                request_id: "req-bootstrap".to_string(),
            },
            HubCommand::RoomBootstrapRead {
                request_id: "req-bootstrap-read".to_string(),
                payload: agentic_gpt_protocol::BootstrapReadRequest {
                    id: "diary".to_string(),
                },
            },
        ];
        let expected = ["room.bootstrap", "room.bootstrap.read"];
        for index in 0..commands.len() {
            let command = &mut commands[index];
            assert_eq!(crate::runs::command_type(command), expected[index]);
            crate::agents::set_command_request_id(command, "req-new".to_string());
            assert_eq!(crate::agents::command_request_id(command), "req-new");
        }
    }

    #[test]
    fn openapi_room_skills_paths_and_schemas_do_not_include_agent_id() {
        let openapi = include_str!("../../../openapi/hub.yaml");
        for path in [
            "/v1/room/bootstrap:",
            "/v1/room/bootstrap/read:",
            "/v1/room/skills/list:",
            "/v1/room/skills/read:",
            "/v1/room/skills/search:",
            "/v1/room/skills/active:",
            "/v1/room/skills/activate:",
            "/v1/room/skills/deactivate:",
            "/v1/room/skills/install:",
            "/v1/room/skills/install/get:",
            "/v1/room/skills/install/cancel:",
            "/v1/room/skills/run:",
        ] {
            assert!(openapi.contains(path), "missing {path}");
        }
        for operation_id in [
            "roomBootstrap",
            "roomBootstrapRead",
            "roomSkillsList",
            "roomSkillsRead",
            "roomSkillsSearch",
            "roomSkillsActive",
            "roomSkillsActivate",
            "roomSkillsDeactivate",
            "roomSkillsInstall",
            "roomSkillsInstallGet",
            "roomSkillsInstallCancel",
            "roomSkillsRun",
        ] {
            assert!(openapi.contains(operation_id), "missing {operation_id}");
        }
        for schema in [
            "BootstrapReadRequest:",
            "BootstrapTextResource:",
            "BootstrapEntrypoint:",
            "BootstrapGuideSummary:",
            "BootstrapResponse:",
            "BootstrapReadResponse:",
            "SkillReadRequest:",
            "SkillSearchRequest:",
            "SkillActivationRequest:",
            "SkillSummary:",
            "SkillDetail:",
            "ActiveSkill:",
            "SkillInstallRequest:",
            "SkillInstallStatusResponse:",
            "SkillRunRequest:",
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
            assert!(!section.is_empty(), "missing schema {schema}");
            assert!(
                !section.contains("agentId"),
                "{schema} unexpectedly contains agentId"
            );
        }
    }
}
