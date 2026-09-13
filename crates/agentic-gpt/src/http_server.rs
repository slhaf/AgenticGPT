use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use tokio::{
    net::TcpListener,
    sync::{watch, RwLock},
    task::{JoinError, JoinHandle},
    time::sleep,
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{self, HttpMcpConfig},
    state::AppState,
    stdio_server::{AgentMcpServer, RequestIngress},
    utils::{log_info, log_warn},
};

pub(crate) const HTTP_MCP_PATH: &str = "/mcp";
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct HttpMcpAuthState {
    resolved_token: Arc<RwLock<Option<String>>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct EndpointKey {
    host: String,
    port: u16,
    allow_hosts: Option<Vec<String>>,
}

impl From<&HttpMcpConfig> for EndpointKey {
    fn from(config: &HttpMcpConfig) -> Self {
        Self {
            host: config.host.clone(),
            port: config.port,
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
    Finished(std::result::Result<Result<()>, JoinError>),
    Retry,
}

pub(crate) async fn run(
    state: AppState,
    mut updates: watch::Receiver<HttpMcpConfig>,
) -> Result<()> {
    let mut active = None;
    loop {
        let desired = updates.borrow().clone();
        reconcile(&state, &mut active, &desired).await?;

        let event = if let Some(server) = active.as_mut() {
            tokio::select! {
                result = &mut server.task => WaitEvent::Finished(result),
                changed = updates.changed() => WaitEvent::Changed(changed.is_ok()),
                _ = sleep(RETRY_INTERVAL) => WaitEvent::Retry,
            }
        } else {
            tokio::select! {
                changed = updates.changed() => WaitEvent::Changed(changed.is_ok()),
                _ = sleep(RETRY_INTERVAL) => WaitEvent::Retry,
            }
        };

        match event {
            WaitEvent::Changed(true) | WaitEvent::Retry => {}
            WaitEvent::Changed(false) => {
                stop_active(&mut active).await;
                return Ok(());
            }
            WaitEvent::Finished(result) => {
                stop_active(&mut active).await;
                if result.is_err() || result.is_ok_and(|result| result.is_err()) {
                    return Err(anyhow!("http_mcp_server_task_failed"));
                }
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
            let changed =
                server.auth.resolved_token.read().await.as_deref() != Some(resolved_token.as_str());
            if changed {
                *server.auth.resolved_token.write().await = Some(resolved_token);
            }
            return Ok(());
        }
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
    let auth = HttpMcpAuthState {
        resolved_token: Arc::new(RwLock::new(Some(resolved_token))),
    };
    let task_auth = auth.clone();
    let task_cancellation = cancellation.clone();
    let allow_hosts = desired.allow_hosts.clone();
    let task = tokio::spawn(async move {
        serve_listener(listener, state, task_auth, allow_hosts, task_cancellation).await
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
    let router = Router::new()
        .nest_service(HTTP_MCP_PATH, service)
        .layer(middleware::from_fn_with_state(auth, require_bearer));

    axum::serve(listener, router)
        .with_graceful_shutdown(cancellation.cancelled_owned())
        .await
        .map_err(|_| anyhow!("http_mcp_server_task_failed"))
}

async fn require_bearer(
    State(auth): State<HttpMcpAuthState>,
    request: Request,
    next: Next,
) -> Response {
    let presented = parse_bearer_token(request.headers());
    let authorized = {
        let expected = auth.resolved_token.read().await;
        expected
            .as_deref()
            .zip(presented)
            .is_some_and(|(expected, presented)| constant_time_eq(expected, presented))
    };
    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response()
    }
}

fn parse_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(char::is_whitespace)?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty()).then_some(token)
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for (left, right) in left.as_bytes().iter().zip(right.as_bytes()) {
        difference |= usize::from(left ^ right);
    }
    difference == 0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_parser_accepts_case_insensitive_scheme_and_trimmed_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "bEaReR   token-value  ".parse().unwrap(),
        );
        assert_eq!(parse_bearer_token(&headers), Some("token-value"));
    }

    #[test]
    fn bearer_parser_rejects_missing_basic_empty_and_malformed_values() {
        for value in [
            "",
            "Basic token",
            "Bearer",
            "Bearer   ",
            "Bearer\ttoken\textra",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
            if value == "Bearer\ttoken\textra" {
                assert_eq!(parse_bearer_token(&headers), Some("token\textra"));
            } else {
                assert_eq!(parse_bearer_token(&headers), None);
            }
        }
    }

    #[test]
    fn constant_time_comparison_checks_length_and_bytes() {
        assert!(constant_time_eq("token", "token"));
        assert!(!constant_time_eq("token", "Token"));
        assert!(!constant_time_eq("token", "token-extra"));
    }
}
