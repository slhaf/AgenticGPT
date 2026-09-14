use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use axum::{middleware, Router};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle, time::sleep};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, HttpMcpConfig},
    http_oauth::{self, HttpMcpAuthState, HttpMcpHostPolicy},
    state::AppState,
    stdio_server::{AgentMcpServer, RequestIngress},
    utils::{log_info, log_warn},
};

pub(crate) const HTTP_MCP_PATH: &str = "/mcp";
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
struct EndpointKey {
    host: String,
    port: u16,
    public_url: Option<String>,
    allow_hosts: Option<Vec<String>>,
}

impl From<&HttpMcpConfig> for EndpointKey {
    fn from(config: &HttpMcpConfig) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
            public_url: config.public_url.clone(),
            allow_hosts: config.allow_hosts.clone(),
        }
    }
}

struct ActiveHttpServer {
    endpoint: EndpointKey,
    auth: HttpMcpAuthState,
    cancellation: CancellationToken,
    task: JoinHandle<Result<()>>,
}

enum WaitEvent {
    Changed(bool),
    Finished,
    Shutdown,
    Retry,
}

pub(crate) async fn run(
    state: AppState,
    mut updates: watch::Receiver<HttpMcpConfig>,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut active = None;
    loop {
        let desired = updates.borrow().clone();
        reconcile(&state, &mut active, &desired).await?;

        let event = if let Some(server) = active.as_mut() {
            tokio::select! {
                _result = &mut server.task => WaitEvent::Finished,
                changed = updates.changed() => WaitEvent::Changed(changed.is_ok()),
                _ = shutdown.cancelled() => WaitEvent::Shutdown,
                _ = sleep(RETRY_INTERVAL) => WaitEvent::Retry,
            }
        } else {
            tokio::select! {
                changed = updates.changed() => WaitEvent::Changed(changed.is_ok()),
                _ = shutdown.cancelled() => WaitEvent::Shutdown,
                _ = sleep(RETRY_INTERVAL) => WaitEvent::Retry,
            }
        };

        match event {
            WaitEvent::Changed(true) | WaitEvent::Retry => {}
            WaitEvent::Changed(false) | WaitEvent::Shutdown => {
                stop_active(&mut active).await;
                return Ok(());
            }
            WaitEvent::Finished => {
                stop_active(&mut active).await;
                return Err(anyhow!("http_mcp_server_task_failed"));
            }
        }
    }
}

async fn reconcile(
    state: &AppState,
    active: &mut Option<ActiveHttpServer>,
    desired: &HttpMcpConfig,
) -> Result<()> {
    if let Err(error) = config::validate_http_mcp_config(desired) {
        log_warn(format!(
            "standalone HTTP MCP configuration rejected; errorCode={}",
            error_code(&error)
        ));
        return Ok(());
    }

    if !desired.enabled {
        if active.is_some() {
            stop_active(active).await;
            log_info("standalone HTTP MCP ingress disabled".to_string());
        }
        return Ok(());
    }

    let Some(resolved_token) = resolve_token(desired) else {
        if active.is_some() {
            stop_active(active).await;
            log_warn("standalone HTTP MCP ingress stopped; bearer token unavailable".to_string());
        }
        return Ok(());
    };
    let endpoint = EndpointKey::from(desired);

    if let Some(server) = active.as_mut() {
        if server.endpoint == endpoint {
            server.auth.replace_resolved_token(resolved_token).await;
            return Ok(());
        }
    }

    let rebind_same_address = active.as_ref().is_some_and(|server| {
        server.endpoint.host == endpoint.host && server.endpoint.port == endpoint.port
    });
    if rebind_same_address {
        stop_active(active).await;
    }

    let listener = match TcpListener::bind((desired.host.as_str(), desired.port)).await {
        Ok(listener) => listener,
        Err(error) => {
            log_warn(format!(
                "standalone HTTP MCP listener bind failed; host={}; port={}; errorCode={}",
                desired.host,
                desired.port,
                error_code(&error)
            ));
            return Ok(());
        }
    };

    let next = spawn_server(
        listener,
        state.clone(),
        desired,
        endpoint.clone(),
        resolved_token,
    );
    if active.is_some() {
        stop_active(active).await;
    }
    *active = Some(next);
    log_info(format!(
        "standalone HTTP MCP ingress ready; transport=streamable-http; host={}; port={}; path={HTTP_MCP_PATH}",
        desired.host, desired.port
    ));
    Ok(())
}

fn resolve_token(config: &HttpMcpConfig) -> Option<String> {
    match config::resolve_secret_reference(&config.bearer_token) {
        Ok(token) => Some(token),
        Err(error) => {
            log_warn(format!(
                "standalone HTTP MCP bearer token unavailable; errorCode={}",
                error_code(&error)
            ));
            None
        }
    }
}

fn spawn_server(
    listener: TcpListener,
    state: AppState,
    desired: &HttpMcpConfig,
    endpoint: EndpointKey,
    resolved_token: String,
) -> ActiveHttpServer {
    let cancellation = CancellationToken::new();
    let auth = HttpMcpAuthState::new(resolved_token, endpoint.public_url.clone());
    let host_policy =
        HttpMcpHostPolicy::new(endpoint.public_url.clone(), desired.allow_hosts.clone());
    let task_auth = auth.clone();
    let task_cancellation = cancellation.clone();
    let allow_hosts = desired.allow_hosts.clone();
    let task = tokio::spawn(async move {
        serve_listener(
            listener,
            state,
            task_auth,
            host_policy,
            allow_hosts,
            task_cancellation,
        )
        .await
    });
    ActiveHttpServer {
        endpoint,
        auth,
        cancellation,
        task,
    }
}

async fn serve_listener(
    listener: TcpListener,
    state: AppState,
    auth: HttpMcpAuthState,
    host_policy: HttpMcpHostPolicy,
    allow_hosts: Option<Vec<String>>,
    cancellation: CancellationToken,
) -> Result<()> {
    let mut server_config = StreamableHttpServerConfig::default()
        .with_stateful_mode(true)
        .with_cancellation_token(cancellation.clone());
    server_config = match allow_hosts {
        None => server_config.disable_allowed_hosts(),
        Some(hosts) if hosts.len() == 1 && hosts[0] == "*" => server_config.disable_allowed_hosts(),
        Some(hosts) => server_config.with_allowed_hosts(hosts),
    };

    let session_manager = Arc::new(LocalSessionManager::default());
    let service: StreamableHttpService<AgentMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(AgentMcpServer::with_ingress(
                    state.clone(),
                    RequestIngress::Http,
                ))
            },
            session_manager,
            server_config,
        );
    let mcp_router =
        Router::new()
            .nest_service(HTTP_MCP_PATH, service)
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                http_oauth::require_bearer,
            ));
    let router =
        http_oauth::routes(auth.clone())
            .merge(mcp_router)
            .layer(middleware::from_fn_with_state(
                host_policy,
                http_oauth::require_host_origin,
            ));

    let cleanup_task = tokio::spawn(http_oauth::cleanup(auth, cancellation.clone()));
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(cancellation.clone().cancelled_owned())
        .await;
    cancellation.cancel();
    let _ = cleanup_task.await;
    result.map_err(|_| anyhow!("http_mcp_server_task_failed"))
}

async fn stop_active(active: &mut Option<ActiveHttpServer>) {
    if let Some(server) = active.take() {
        server.cancellation.cancel();
        let _ = server.task.await;
    }
}

fn error_code(error: &impl std::fmt::Display) -> String {
    error
        .to_string()
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .find(|part| !part.is_empty())
        .unwrap_or("http_mcp_failed")
        .chars()
        .take(64)
        .collect()
}
