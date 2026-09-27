use anyhow::Result;
use axum::http::Request;
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use rusqlite::Connection;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use crate::state::{HubState, McpProfile};
use crate::REQUEST_TIMEOUT_SECS;
use crate::{
    agents, confirmation, mcp_server, notify, oauth, room, routes, runs, state, HubConfig,
};

pub(crate) async fn serve(
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
        .route("/v1/room/diary/active", post(room::http::room_diary_active))
        .route("/v1/room/diary/read", post(room::http::room_diary_read))
        .route(
            "/v1/room/notebook/recent",
            post(room::http::room_notebook_recent),
        )
        .route(
            "/v1/room/notebook/search",
            post(room::http::room_notebook_search),
        )
        .route(
            "/v1/room/notebook/read",
            post(room::http::room_notebook_read),
        )
        .route("/v1/room/state/list", post(room::http::room_state_list))
        .route("/v1/room/state/read", post(room::http::room_state_read))
        .route(
            "/v1/room/maintenance/status",
            post(room::http::room_maintenance_status),
        )
        .route(
            "/v1/room/maintenance/submit",
            post(room::http::room_maintenance_submit),
        )
        .route("/v1/room/bootstrap", post(room::http::room_bootstrap))
        .route(
            "/v1/room/bootstrap/read",
            post(room::http::room_bootstrap_read),
        )
        .route("/v1/room/skills/list", post(room::http::skills_list))
        .route("/v1/room/skills/read", post(room::http::skills_read))
        .route("/v1/room/skills/search", post(room::http::skills_search))
        .route("/v1/room/skills/active", post(room::http::skills_active))
        .route(
            "/v1/room/skills/activate",
            post(room::http::skills_activate),
        )
        .route(
            "/v1/room/skills/deactivate",
            post(room::http::skills_deactivate),
        )
        .route("/v1/room/skills/install", post(room::http::skills_install))
        .route(
            "/v1/room/skills/install/get",
            post(room::http::skills_install_get),
        )
        .route(
            "/v1/room/skills/install/cancel",
            post(room::http::skills_install_cancel),
        )
        .route("/v1/room/skills/run", post(room::http::skills_run))
        .route(
            "/mcp",
            get(mcp_server::transport::mcp_get).post(mcp_server::transport::mcp_post),
        )
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
            mcp_server::transport::require_auth_on_mcp_path,
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
