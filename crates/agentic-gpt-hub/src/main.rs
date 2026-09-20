mod agentic_result;
mod agents;
mod confirmation;
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
    SafeBuiltinPolicyRules, SafeConfigSummary, SafePathPolicySummary, SafePolicyRules,
    SafeSandboxSummary,
};
use anyhow::{Context, Result};
use axum::http::Request;
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
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
use crate::state::{HubState, McpProfile};

const REQUEST_TIMEOUT_SECS: u64 = 35;
const MAX_WAIT_SECONDS: u64 = 30;
const DEFAULT_REMOTE_CONFIRM_TIMEOUT_SECS: u64 = 45;

#[derive(Parser)]
#[command(name = "agentic-gpt-hub")]
#[command(version)]
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
        agents: Arc::new(agents::lifecycle::Connections::new()),
        dispatch: Arc::new(agents::dispatch::Dispatch::new()),
        confirmations: Arc::new(confirmation::Confirmations::new()),
        job_cache: Arc::new(state::JobCache::new()),
        boot_generations: Arc::new(Mutex::new(HashMap::new())),
        active_room: Arc::new(Mutex::new(None)),
        http: reqwest::Client::new(),
        public_base_url: public_base_url.map(|value| value.trim_end_matches('/').to_string()),
        oauth_codes: Arc::new(Mutex::new(HashMap::new())),
        oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
        ntfy_health: Arc::new(Mutex::new(None)),
    };
    tokio::spawn(confirmation::cleanup(state.clone()));
    tokio::spawn(cleanup_runs(state.clone()));
    tokio::spawn(cleanup_job_cache(state.clone()));
    tokio::spawn(agents::lifecycle::cleanup_agent_connections(state.clone()));
    tokio::spawn(oauth::cleanup_oauth(state.clone()));
    let app = Router::new()
        .route("/v1/info", get(routes::hub_info))
        .route("/v1/agents", get(routes::list_agents))
        .route(
            "/v1/agents/:agent_id/connect",
            get(agents::transport::connect_agent),
        )
        .route(
            "/v1/agents/:agent_id/events",
            get(agents::transport::connect_agent_sse),
        )
        .route(
            "/v1/agents/:agent_id/messages",
            post(agents::transport::post_agent_message),
        )
        .route("/v1/runs/:run_id", get(routes::get_run))
        .route(
            "/v1/confirmations/:confirmation_id/:decision",
            post(confirmation::callback),
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

async fn cleanup_job_cache(state: HubState) {
    loop {
        sleep(Duration::from_secs(15)).await;
        state.job_cache.sweep(Utc::now()).await;
    }
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
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("refusing symlinked hub config path {}", path.display())
            }
            Ok(_) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("read hub config {}", path.display()))?;
                Ok(serde_json::from_str(&text)
                    .with_context(|| format!("parse hub config {}", path.display()))?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::default_config())
            }
            Err(error) => Err(error.into()),
        }
    }

    fn write_if_missing(&self, path: &PathBuf) -> Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    anyhow::bail!("refusing symlinked hub config path {}", path.display());
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = unique_config_sibling(path);
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true).read(true);
        set_private_file_mode(&mut options);
        let payload = serde_json::to_vec_pretty(self)?;
        {
            let mut file = options.open(&temp)?;
            std::io::Write::write_all(&mut file, &payload)?;
            file.sync_all()?;
        }
        match std::fs::hard_link(&temp, path) {
            Ok(()) => {
                std::fs::File::open(path)?.sync_all()?;
                sync_config_parent(path)?;
                std::fs::remove_file(&temp)?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&temp);
                match std::fs::symlink_metadata(path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        anyhow::bail!("refusing symlinked hub config path {}", path.display())
                    }
                    Ok(_) => Ok(()),
                    Err(error) => Err(error.into()),
                }
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                Err(error.into())
            }
        }
    }
}

fn unique_config_sibling(path: &std::path::Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".tmp.{}.{}", std::process::id(), nonce));
    PathBuf::from(value)
}

fn sync_config_parent(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_file_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_mode(_options: &mut std::fs::OpenOptions) {}

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
            agents: Arc::new(agents::lifecycle::Connections::new()),
            dispatch: Arc::new(agents::dispatch::Dispatch::new()),
            confirmations: Arc::new(confirmation::Confirmations::new()),
            job_cache: Arc::new(state::JobCache::new()),
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

    #[test]
    fn cli_version_uses_crate_version() {
        let error = match Cli::try_parse_from(["agentic-gpt-hub", "--version"]) {
            Ok(_) => panic!("--version unexpectedly parsed as a runnable command"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
        let rendered = error.to_string();
        assert!(rendered.contains("agentic-gpt-hub 0.9.1"));
        assert!(rendered.contains(env!("CARGO_PKG_VERSION")));
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
    fn ntfy_mcp_confirmation_stays_within_three_action_limit() {
        let actions = confirmation::ntfy_confirmation_actions(
            "https://hub.example",
            "confirm-1",
            "token-1",
            Some("mcpTool"),
        );
        let actions = actions.as_array().unwrap();
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0]["label"], "Allow once");
        assert_eq!(actions[1]["label"], "Allow MCP 30m");
        assert_eq!(actions[2]["label"], "Deny");
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
    fn skills_commands_have_run_types() {
        let commands = [
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
                    group: None,
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

        for (index, command) in commands.iter().enumerate() {
            let expected_type = expected[index];
            assert_eq!(crate::runs::command_type(command), expected_type);
        }
    }

    #[test]
    fn bootstrap_commands_have_run_types() {
        let commands = [
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
        for (index, command) in commands.iter().enumerate() {
            assert_eq!(crate::runs::command_type(command), expected[index]);
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
