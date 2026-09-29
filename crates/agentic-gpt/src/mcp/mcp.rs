pub(crate) mod batch;

use std::{
    env,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use agentic_gpt_protocol::{
    McpCallToolRequest, McpListToolsRequest, McpServerSummary, ProcessResponse, ProcessState,
};
use anyhow::{anyhow, Context, Result};
use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientInfo,
        ClientRequest, JsonObject, ServerResult,
    },
    service::{PeerRequestOptions, RunningService, ServiceError},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, ConfigureCommandExt,
        StreamableHttpClientTransport, TokioChildProcess,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::time::{sleep, sleep_until, timeout, Duration, Instant};

use crate::{
    config::mcp_servers::{McpServerAuthConfig, McpServerConfig},
    confirmation, process,
    process::{ManagedMcpSpec, TerminalEventHook},
    state::AppState,
    utils::bounded_mcp_argument_keys,
};

type McpClient = RunningService<rmcp::RoleClient, ClientInfo>;
type McpClientFuture = Pin<Box<dyn Future<Output = Result<McpClient>> + Send>>;
type McpClientFactory = Arc<dyn Fn(McpServerConfig) -> McpClientFuture + Send + Sync + 'static>;

pub(crate) async fn list_servers(state: &AppState) -> Value {
    let config = state.config.read().await;
    let servers = config
        .mcp_servers
        .iter()
        .map(|(id, server)| McpServerSummary {
            id: id.clone(),
            enabled: server.enabled,
            transport: server.transport.clone(),
            url: server.url.clone(),
        })
        .collect::<Vec<_>>();
    json!({ "servers": servers })
}

pub(crate) async fn list_tools(state: &AppState, payload: McpListToolsRequest) -> Result<Value> {
    let server = server_config(state, &payload.server_id).await?;
    let client = client(&server).await?;
    let tools = client.list_all_tools().await;
    close_client(client).await;
    Ok(json!({ "tools": tools? }))
}

pub(crate) async fn call_tool(
    state: &AppState,
    payload: McpCallToolRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> Result<ProcessResponse> {
    start_managed_call_with_factory(
        state,
        payload,
        request_source,
        terminal_event_hook,
        production_client_factory(),
    )
    .await
}

async fn start_managed_call_with_factory(
    state: &AppState,
    payload: McpCallToolRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    client_factory: McpClientFactory,
) -> Result<ProcessResponse> {
    validate_tool_name(&payload.tool_name)?;
    let arguments = tool_arguments(payload.arguments.clone())?;
    let argument_bytes = serde_json::to_vec(&payload.arguments)?.len();
    if argument_bytes > process::MAX_MCP_ARGUMENT_BYTES {
        return Err(anyhow!(
            "mcp_tool_arguments_too_large: bytes={argument_bytes}; max={}",
            process::MAX_MCP_ARGUMENT_BYTES
        ));
    }
    let argument_sha256 = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&payload.arguments)?)
    );
    let (argument_keys, argument_key_count, argument_keys_truncated) =
        bounded_mcp_argument_keys(&payload.arguments);
    let (config_revision, server) = server_config_snapshot(state, &payload.server_id).await;
    let wait_seconds = payload.effective_wait_seconds();
    let timeout_seconds = payload.effective_timeout_seconds();
    let registration = process::register_mcp_process(
        state,
        ManagedMcpSpec {
            agent_id: payload.agent_id.clone(),
            group: payload.group.clone(),
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            server_id: payload.server_id.clone(),
            tool_name: payload.tool_name.clone(),
            request_source: request_source.to_string(),
            argument_keys,
            argument_key_count,
            argument_keys_truncated,
            argument_bytes,
            argument_sha256,
            config_revision,
            terminal_event_hook,
        },
    )
    .await
    .map_err(|reason| anyhow!(reason))?;
    let process_id = registration.info.process_id.clone();
    let server = match server {
        Ok(server) => server,
        Err(reason) => {
            let code = reason
                .split_once(':')
                .map_or(reason.as_str(), |(code, _)| code)
                .to_string();
            let _ = process::set_mcp_preflight_rejection(state, &process_id).await;
            let _ = process::finish_mcp_error(
                state,
                &process_id,
                ProcessState::Rejected,
                code,
                reason,
                None,
                Some("server_config_validation"),
            )
            .await;
            return process::mcp_process_response(state, &process_id, 0)
                .await
                .map_err(|reason| anyhow!(reason));
        }
    };
    tokio::spawn(run_managed_call(
        state.clone(),
        payload,
        arguments,
        server,
        registration.cancel_requested,
        timeout_seconds,
        process_id.clone(),
        client_factory,
        None,
        None,
    ));
    process::mcp_process_response(state, &process_id, wait_seconds)
        .await
        .map_err(|reason| anyhow!(reason))
}

#[allow(clippy::too_many_arguments)]
async fn run_managed_call(
    state: AppState,
    payload: McpCallToolRequest,
    arguments: JsonObject,
    server: McpServerConfig,
    cancel_requested: Arc<AtomicBool>,
    timeout_seconds: u64,
    process_id: String,
    client_factory: McpClientFactory,
    authorization_override: Option<String>,
    fail_fast_stop: Option<Arc<AtomicBool>>,
) {
    let authorization = match authorization_override {
        Some(authorization) => authorization,
        None => {
            confirmation::authorize_mcp_tool_call_cancellable(
                &state,
                &payload.server_id,
                &payload.tool_name,
                &payload.arguments,
                cancel_requested.clone(),
            )
            .await
        }
    };
    let _ = process::set_mcp_authorization(&state, &process_id, &authorization).await;
    if authorization == "cancelled" || cancel_requested.load(Ordering::Acquire) {
        let _ = process::finish_mcp_error(
            &state,
            &process_id,
            ProcessState::Cancelled,
            "mcp_cancelled",
            "MCP process was cancelled before the downstream request started",
            Some("cancelled_before_request"),
            Some("local_cancel_before_downstream_request"),
        )
        .await;
        return;
    }
    if !mcp_authorization_allows(&authorization) {
        let _ = process::finish_mcp_error(
            &state,
            &process_id,
            ProcessState::Rejected,
            "mcp_tool_call_rejected",
            format!("MCP tool call rejected: {authorization}"),
            None,
            Some("authorization_decision"),
        )
        .await;
        return;
    }
    let _ = process::set_mcp_process_state(&state, &process_id, ProcessState::Queued).await;
    let _permit = match state
        .mcp_concurrency
        .acquire(&payload.server_id, cancel_requested.clone())
        .await
    {
        Ok(permit) => permit,
        Err(reason) if reason == "cancelled" => {
            let _ = process::finish_mcp_error(
                &state,
                &process_id,
                ProcessState::Cancelled,
                "mcp_cancelled",
                "MCP process was cancelled while waiting for an execution slot",
                Some("cancelled_while_queued"),
                Some("local_cancel_before_downstream_request"),
            )
            .await;
            return;
        }
        Err(reason) => {
            let _ = process::finish_mcp_error(
                &state,
                &process_id,
                ProcessState::Failed,
                "mcp_concurrency_failed",
                reason,
                None,
                Some("local_scheduler_error"),
            )
            .await;
            return;
        }
    };
    if fail_fast_stop
        .as_ref()
        .is_some_and(|stop| stop.load(Ordering::Acquire))
    {
        let _ = process::finish_mcp_error(
            &state,
            &process_id,
            ProcessState::Skipped,
            "mcp_batch_fail_fast_skipped",
            "MCP batch fail-fast prevented this queued child from starting",
            None,
            Some("fail_fast_before_downstream_start"),
        )
        .await;
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let _ = process::set_mcp_process_state(&state, &process_id, ProcessState::Starting).await;
    let downstream = tokio::select! {
        result = (client_factory)(server.clone()) => Some(result),
        _ = wait_for_cancel(cancel_requested.clone()) => {
            let _ = process::finish_mcp_error(
                &state,
                &process_id,
                ProcessState::Cancelled,
                "mcp_cancelled",
                "MCP process was cancelled before the downstream client connected",
                Some("cancelled_before_request"),
                Some("local_cancel_before_downstream_request"),
            ).await;
            return;
        }
        _ = sleep_until(deadline) => None,
    };
    let Some(downstream) = downstream else {
        let _ = process::finish_mcp_error(
            &state,
            &process_id,
            ProcessState::TimedOut,
            "mcp_timeout",
            "MCP execution deadline expired while connecting to the downstream server",
            None,
            Some("deadline_before_downstream_request"),
        )
        .await;
        return;
    };
    let client = match downstream {
        Ok(client) => client,
        Err(error) => {
            let _ = process::finish_mcp_error(
                &state,
                &process_id,
                ProcessState::Failed,
                "mcp_client_connect_failed",
                error.to_string(),
                None,
                Some("local_client_error"),
            )
            .await;
            return;
        }
    };
    if cancel_requested.load(Ordering::Acquire) {
        close_client(client).await;
        let _ = process::finish_mcp_error(
            &state,
            &process_id,
            ProcessState::Cancelled,
            "mcp_cancelled",
            "MCP process was cancelled before the downstream request started",
            Some("cancelled_before_request"),
            Some("local_cancel_before_downstream_request"),
        )
        .await;
        return;
    }
    let request = ClientRequest::CallToolRequest(CallToolRequest::new(
        CallToolRequestParams::new(payload.tool_name.clone()).with_arguments(arguments),
    ));
    let request_peer = client.peer().clone();
    let handle = tokio::select! {
        result = request_peer.send_cancellable_request(request, PeerRequestOptions::no_options()) => Some(result),
        _ = wait_for_cancel(cancel_requested.clone()) => {
            close_client(client).await;
            let _ = process::finish_mcp_error(
                &state,
                &process_id,
                ProcessState::Cancelled,
                "mcp_cancelled",
                "MCP process was cancelled before the downstream request id was allocated",
                Some("cancelled_before_request"),
                Some("local_cancel_before_downstream_request"),
            ).await;
            return;
        }
        _ = sleep_until(deadline) => None,
    };
    let Some(handle) = handle else {
        close_client(client).await;
        let _ = process::finish_mcp_error(
            &state,
            &process_id,
            ProcessState::TimedOut,
            "mcp_timeout",
            "MCP execution deadline expired while starting the downstream request",
            None,
            Some("deadline_before_downstream_request"),
        )
        .await;
        return;
    };
    let handle = match handle {
        Ok(handle) => handle,
        Err(error) => {
            close_client(client).await;
            let _ = process::finish_mcp_error(
                &state,
                &process_id,
                ProcessState::Failed,
                "mcp_request_start_failed",
                error.to_string(),
                None,
                Some("local_request_error"),
            )
            .await;
            return;
        }
    };
    let peer = handle.peer.clone();
    let request_id = handle.id.clone();
    let mut response = handle.rx;
    if process::attach_mcp_request(&state, &process_id, peer.clone(), request_id.clone())
        .await
        .is_err()
    {
        let _ = timeout(
            Duration::from_secs(2),
            peer.notify_cancelled(CancelledNotificationParam {
                request_id,
                reason: Some("Process no longer active".to_string()),
            }),
        )
        .await;
        close_client(client).await;
        return;
    }

    tokio::select! {
        result = &mut response => {
            let after_cancel = cancel_requested.load(Ordering::Acquire);
            finish_from_response(&state, &process_id, result, after_cancel).await;
        }
        _ = wait_for_cancel(cancel_requested.clone()) => {
            finish_after_cancel(&state, &process_id, &mut response).await;
        }
        _ = sleep_until(deadline) => {
            let notification = timeout(
                Duration::from_secs(2),
                peer.notify_cancelled(CancelledNotificationParam {
                    request_id,
                    reason: Some("Agentic MCP execution deadline expired".to_string()),
                }),
            ).await;
            let (outcome, evidence) = if matches!(notification, Ok(Ok(()))) {
                ("notification_sent", "mcp_timeout_cancel_notification_sent")
            } else {
                ("notification_failed", "mcp_timeout_cancel_notification_failed")
            };
            let _ = timeout(Duration::from_secs(2), &mut response).await;
            let _ = process::finish_mcp_error(
                &state,
                &process_id,
                ProcessState::TimedOut,
                "mcp_timeout",
                format!("MCP execution exceeded {timeout_seconds} seconds"),
                Some(outcome),
                Some(evidence),
            ).await;
        }
    }
    close_client(client).await;
}

async fn close_client(client: McpClient) {
    let _ = timeout(Duration::from_secs(2), client.cancel()).await;
}

async fn finish_after_cancel(
    state: &AppState,
    process_id: &str,
    response: &mut tokio::sync::oneshot::Receiver<Result<ServerResult, ServiceError>>,
) {
    match timeout(Duration::from_secs(2), response).await {
        Ok(result) => finish_from_response(state, process_id, result, true).await,
        Err(_) => {
            let _ = process::finish_mcp_error(
                state,
                process_id,
                ProcessState::Detached,
                "mcp_cancel_detached",
                "Cancellation notification was sent, but no downstream terminal response was observed",
                Some("notification_sent"),
                Some("mcp_cancel_notification_sent_no_terminal_response"),
            )
            .await;
        }
    }
}

async fn finish_from_response(
    state: &AppState,
    process_id: &str,
    response: Result<Result<ServerResult, ServiceError>, tokio::sync::oneshot::error::RecvError>,
    after_cancel: bool,
) {
    match response {
        Ok(Ok(ServerResult::CallToolResult(result))) => {
            let downstream_error = result.is_error == Some(true);
            let value = serde_json::to_value(result).unwrap_or_else(|_| {
                json!({
                    "isError": true,
                    "content": [{"type": "text", "text": "Result serialization failed"}]
                })
            });
            let cancel = after_cancel.then_some(("completed_after_cancel", "remote_response"));
            let _ =
                process::complete_mcp_result(state, process_id, value, downstream_error, cancel)
                    .await;
        }
        Ok(Ok(_)) => {
            let _ = process::finish_mcp_error(
                state,
                process_id,
                ProcessState::Failed,
                "mcp_unexpected_response",
                "Downstream MCP server returned an unexpected response type",
                after_cancel.then_some("response_after_cancel"),
                Some("remote_response"),
            )
            .await;
        }
        Ok(Err(error)) if after_cancel && explicit_cancel_error(&error) => {
            let _ = process::finish_mcp_error(
                state,
                process_id,
                ProcessState::Cancelled,
                "mcp_cancelled",
                error.to_string(),
                Some("cancelled"),
                Some("downstream_cancellation_response"),
            )
            .await;
        }
        Ok(Err(error)) if after_cancel => {
            let _ = process::finish_mcp_error(
                state,
                process_id,
                ProcessState::Detached,
                "mcp_cancel_detached",
                error.to_string(),
                Some("notification_sent"),
                Some("transport_or_remote_error_after_cancel"),
            )
            .await;
        }
        Ok(Err(error)) => {
            let _ = process::finish_mcp_error(
                state,
                process_id,
                ProcessState::Failed,
                "mcp_request_failed",
                error.to_string(),
                None,
                Some("downstream_or_transport_error"),
            )
            .await;
        }
        Err(_) if after_cancel => {
            let _ = process::finish_mcp_error(
                state,
                process_id,
                ProcessState::Detached,
                "mcp_cancel_detached",
                "Downstream transport closed after cancellation without terminal evidence",
                Some("notification_sent"),
                Some("transport_closed_after_cancel"),
            )
            .await;
        }
        Err(_) => {
            let _ = process::finish_mcp_error(
                state,
                process_id,
                ProcessState::Failed,
                "mcp_transport_closed",
                "Downstream MCP transport closed before returning a result",
                None,
                Some("transport_closed"),
            )
            .await;
        }
    }
}

fn explicit_cancel_error(error: &ServiceError) -> bool {
    match error {
        ServiceError::McpError(error) => error.message.to_ascii_lowercase().contains("cancel"),
        _ => false,
    }
}

async fn wait_for_cancel(cancel_requested: Arc<AtomicBool>) {
    while !cancel_requested.load(Ordering::Acquire) {
        sleep(Duration::from_millis(25)).await;
    }
}

fn validate_tool_name(tool_name: &str) -> Result<()> {
    if tool_name.is_empty() || tool_name.len() > 256 || tool_name.chars().any(char::is_control) {
        return Err(anyhow!("mcp_tool_name_invalid"));
    }
    Ok(())
}

fn mcp_authorization_allows(value: &str) -> bool {
    matches!(
        value,
        "allow_once" | "allow_mcp_server_15m" | "allow_mcp_server_30m" | "temporary_mcp_allow"
    )
}

async fn server_config_snapshot(
    state: &AppState,
    server_id: &str,
) -> (String, Result<McpServerConfig, String>) {
    let config = state.config.read().await;
    let revision = crate::config::mcp_servers::server_config_revision(&config.mcp_servers);
    let server = config.mcp_servers.get(server_id).cloned();
    drop(config);
    let result = match server {
        Some(server) => validate_selected_server(server_id, &server)
            .map(|_| server)
            .map_err(|error| error.to_string()),
        None => Err(format!("mcp_server_not_found: {server_id}")),
    };
    (revision, result)
}

fn validate_selected_server(server_id: &str, server: &McpServerConfig) -> Result<()> {
    if !server.enabled {
        return Err(anyhow!("mcp_server_disabled: {server_id}"));
    }
    match server.transport.as_str() {
        "streamable-http" => {
            if server.url.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(anyhow!("mcp_server_url_missing: {server_id}"));
            }
        }
        "stdio" => {
            if server.url.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(anyhow!("mcp_server_command_missing: {server_id}"));
            }
        }
        other => return Err(anyhow!("unsupported_mcp_transport: {other}")),
    }
    Ok(())
}

async fn server_config(state: &AppState, server_id: &str) -> Result<McpServerConfig> {
    let config = state.config.read().await;
    let server = config
        .mcp_servers
        .get(server_id)
        .cloned()
        .ok_or_else(|| anyhow!("mcp_server_not_found: {server_id}"))?;
    validate_selected_server(server_id, &server)?;
    Ok(server)
}

fn production_client_factory() -> McpClientFactory {
    Arc::new(|server| Box::pin(async move { client(&server).await }))
}

async fn client(server: &McpServerConfig) -> Result<McpClient> {
    match server.transport.as_str() {
        "streamable-http" => {
            let transport = StreamableHttpClientTransport::from_config(
                streamable_http_transport_config(server)?,
            );
            Ok(ClientInfo::default().serve(transport).await?)
        }
        "stdio" => {
            let command = server.url.clone().context("mcp_server_command_missing")?;
            let transport =
                TokioChildProcess::new(tokio::process::Command::new("sh").configure(|cmd| {
                    cmd.arg("-lc").arg(command);
                    if let Ok(home) = env::var("HOME") {
                        let path = env::var("PATH").unwrap_or_default();
                        cmd.env("PATH", format!("{home}/.local/bin:{path}"));
                    }
                }))?;
            Ok(ClientInfo::default().serve(transport).await?)
        }
        other => Err(anyhow!("unsupported_mcp_transport: {other}")),
    }
}

fn streamable_http_transport_config(
    server: &McpServerConfig,
) -> Result<StreamableHttpClientTransportConfig> {
    let url = server.url.clone().context("mcp_server_url_missing")?;
    let mut config = StreamableHttpClientTransportConfig::with_uri(url);
    if let Some(McpServerAuthConfig::Bearer { token }) = &server.auth {
        config = config.auth_header(token.clone());
    }
    Ok(config)
}

fn tool_arguments(arguments: Value) -> Result<JsonObject> {
    match arguments {
        Value::Null => Ok(JsonObject::new()),
        Value::Object(map) => Ok(map),
        other => Err(anyhow!("mcp_tool_arguments_must_be_object: {other}")),
    }
}
#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
