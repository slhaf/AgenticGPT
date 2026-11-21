use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, Result};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use futures_util::StreamExt;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use tokio::{
    net::TcpListener,
    sync::{watch, Notify},
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, HttpMcpConfig},
    http_oauth::{self, HttpMcpAuthState, HttpMcpHostPolicy},
    state::AppState,
    stdio_server::AgentMcpServer,
    utils::{log_info, log_warn},
};

pub(crate) const HTTP_MCP_PATH: &str = "/mcp";
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
const REBIND_WAIT: Duration = Duration::from_millis(500);
const STOP_WAIT: Duration = Duration::from_secs(2);

/// One HTTP listener/session generation. Retirement closes admission immediately,
/// while accepted tool calls and their POST response bodies keep the old sessions
/// alive. A body finishing is distinct from the tool handler merely returning.
#[derive(Default)]
pub(crate) struct HttpOwner {
    retired: AtomicBool,
    pending: AtomicUsize,
    idle: Notify,
}

pub(crate) struct HttpRequestLease(Arc<HttpOwner>);

impl Drop for HttpRequestLease {
    fn drop(&mut self) {
        if self.0.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_one();
        }
    }
}

impl HttpOwner {
    pub(crate) fn begin(self: &Arc<Self>) -> Option<HttpRequestLease> {
        if self.retired.load(Ordering::Acquire) {
            return None;
        }
        self.pending.fetch_add(1, Ordering::AcqRel);
        let lease = HttpRequestLease(self.clone());
        // Close the race with retirement between the check and increment.
        if self.retired.load(Ordering::Acquire) {
            drop(lease);
            None
        } else {
            Some(lease)
        }
    }

    fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }

    async fn drained(&self) {
        loop {
            let notified = self.idle.notified();
            if self.pending.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

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
    applied: HttpMcpConfig,
    auth: HttpMcpAuthState,
    owner: Arc<HttpOwner>,
    accept_stop: CancellationToken,
    session_stop: CancellationToken,
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
        let error = reconcile(&state, &mut active, &desired)
            .await
            .err()
            .map(|error| error_code(&error));
        let applied = active
            .as_ref()
            .map(|server: &ActiveHttpServer| server.applied.clone())
            .or_else(|| (!desired.enabled && error.is_none()).then(|| desired.clone()));
        publish_attempt(&state, applied, error, Some(&desired)).await;

        let event = if let Some(server) = active.as_mut() {
            tokio::select! {
                _ = &mut server.task => WaitEvent::Finished,
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
                stop_active(&mut active);
                publish_attempt(&state, None, Some("http_mcp_owner_stopped".into()), None).await;
                return Ok(());
            }
            WaitEvent::Finished => {
                if let Some(server) = active.take() {
                    server.owner.retire();
                    server.session_stop.cancel();
                }
                publish_attempt(
                    &state,
                    None,
                    Some("http_mcp_server_task_failed".into()),
                    None,
                )
                .await;
                return Err(anyhow!("http_mcp_server_task_failed"));
            }
        }
    }
}

async fn publish_attempt(
    state: &AppState,
    applied: Option<HttpMcpConfig>,
    error: Option<String>,
    target: Option<&HttpMcpConfig>,
) {
    *state.config_runtime.http_applied.write().await = applied;
    *state.config_runtime.http_error.write().await = error;
    *state.config_runtime.http_attempted_config.write().await = target.cloned();
    state.config_runtime.http_attempted.notify_one();
}

async fn reconcile(
    state: &AppState,
    active: &mut Option<ActiveHttpServer>,
    desired: &HttpMcpConfig,
) -> Result<()> {
    config::validate_http_mcp_config(desired)?;
    if !desired.enabled {
        if active.is_some() {
            stop_active(active);
            log_info("standalone HTTP MCP ingress disabled; accepted requests are draining".into());
        }
        return Ok(());
    }
    let resolved_token = match config::resolve_secret_reference(&desired.bearer_token) {
        Ok(token) => token,
        Err(_) => {
            // Revoking the credential also revokes admission through the old
            // listener. Accepted responses still drain before sessions close.
            stop_active(active);
            return Err(anyhow!("http_mcp_bearer_token_unavailable"));
        }
    };
    let endpoint = EndpointKey::from(desired);
    if let Some(server) = active.as_mut() {
        if server.endpoint == endpoint {
            server.auth.replace_resolved_token(resolved_token);
            server.applied = desired.clone();
            return Ok(());
        }
    }
    let same_address = active.as_ref().is_some_and(|server| {
        server.endpoint.host == endpoint.host && server.endpoint.port == endpoint.port
    });
    if same_address {
        // Host/origin policy belongs to the listener generation. Free its accept
        // socket without cancelling accepted requests. If rebinding fails, the
        // old socket cannot be restored; report unavailable, never old applied.
        stop_active(active);
    }
    let binding = async {
        loop {
            match TcpListener::bind((desired.host.as_str(), desired.port)).await {
                Ok(listener) => return Ok(listener),
                Err(error) if same_address && error.kind() == std::io::ErrorKind::AddrInUse => {
                    sleep(Duration::from_millis(10)).await
                }
                Err(error) => return Err(error),
            }
        }
    };
    let listener = match timeout(REBIND_WAIT, binding).await {
        Ok(Ok(listener)) => listener,
        Ok(Err(error)) => {
            log_warn(format!(
                "standalone HTTP MCP listener bind failed; host={}; port={}; errorCode={}",
                desired.host,
                desired.port,
                error_code(&error)
            ));
            return Err(anyhow!("http_mcp_listener_bind_failed"));
        }
        Err(_) => {
            return Err(anyhow!(if same_address {
                "http_mcp_same_address_rebind_failed"
            } else {
                "http_mcp_listener_bind_timeout"
            }))
        }
    };
    let next = spawn_server(listener, state.clone(), desired, endpoint, resolved_token);
    stop_active(active);
    *active = Some(next);
    log_info(format!("standalone HTTP MCP ingress ready; transport=streamable-http; host={}; port={}; path={HTTP_MCP_PATH}", desired.host, desired.port));
    Ok(())
}

fn spawn_server(
    listener: TcpListener,
    state: AppState,
    desired: &HttpMcpConfig,
    endpoint: EndpointKey,
    resolved_token: String,
) -> ActiveHttpServer {
    let accept_stop = CancellationToken::new();
    let session_stop = CancellationToken::new();
    let owner = Arc::new(HttpOwner::default());
    let auth = HttpMcpAuthState::new(resolved_token, endpoint.public_url.clone());
    let host_policy =
        HttpMcpHostPolicy::new(endpoint.public_url.clone(), desired.allow_hosts.clone());
    let task_auth = auth.clone();
    let task_accept_stop = accept_stop.clone();
    let task_session_stop = session_stop.clone();
    let task_owner = owner.clone();
    let allow_hosts = desired.allow_hosts.clone();
    let task = tokio::spawn(serve_listener(
        listener,
        state,
        task_auth,
        host_policy,
        allow_hosts,
        task_accept_stop,
        task_session_stop,
        task_owner,
    ));
    ActiveHttpServer {
        endpoint,
        applied: desired.clone(),
        auth,
        owner,
        accept_stop,
        session_stop,
        task,
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve_listener(
    listener: TcpListener,
    state: AppState,
    auth: HttpMcpAuthState,
    host_policy: HttpMcpHostPolicy,
    allow_hosts: Option<Vec<String>>,
    accept_stop: CancellationToken,
    session_stop: CancellationToken,
    owner: Arc<HttpOwner>,
) -> Result<()> {
    let mut server_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(true)
        .with_cancellation_token(session_stop.clone());
    server_config = match allow_hosts {
        None => server_config.disable_allowed_hosts(),
        Some(hosts) if hosts.len() == 1 && hosts[0] == "*" => server_config.disable_allowed_hosts(),
        Some(hosts) => server_config.with_allowed_hosts(hosts),
    };
    let session_manager = Arc::new(LocalSessionManager::default());
    let service_owner = owner.clone();
    let service: StreamableHttpService<AgentMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                Ok(AgentMcpServer::with_http_owner(
                    state.clone(),
                    service_owner.clone(),
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
    let router = http_oauth::routes(auth.clone())
        .merge(mcp_router)
        .layer(middleware::from_fn_with_state(
            host_policy,
            http_oauth::require_host_origin,
        ))
        .layer(middleware::from_fn_with_state(owner, retain_response));
    let cleanup_task = tokio::spawn(http_oauth::cleanup(auth, session_stop.clone()));
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(accept_stop.cancelled_owned())
        .await;
    session_stop.cancel();
    let _ = cleanup_task.await;
    result.map_err(|_| anyhow!("http_mcp_server_task_failed"))
}

async fn retain_response(
    State(owner): State<Arc<HttpOwner>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(lease) = owner.begin() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "http_mcp_endpoint_retired").into_response();
    };
    let retain_body = request.method() == Method::POST;
    let response = next.run(request).await;
    if !retain_body {
        return response;
    }
    let (parts, body) = response.into_parts();
    let stream = futures_util::stream::unfold(
        (body.into_data_stream(), lease),
        |(mut body, lease)| async move { body.next().await.map(|item| (item, (body, lease))) },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}

fn stop_active(active: &mut Option<ActiveHttpServer>) {
    let Some(mut server) = active.take() else {
        return;
    };
    server.owner.retire();
    server.accept_stop.cancel();
    tokio::spawn(async move {
        // A disconnected POST may drop its body while its tool still writes.
        // The independent tool lease prevents cancelling that accepted work.
        server.owner.drained().await;
        server.session_stop.cancel();
        if timeout(STOP_WAIT, &mut server.task).await.is_err() {
            // Do not abort accepted work or a slow final response. Dropping a
            // JoinHandle detaches the graceful shutdown; the update stays bounded.
            log_warn("standalone HTTP MCP old listener is still draining".into());
        }
    });
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
