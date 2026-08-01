use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use agentic_gpt_protocol::{
    JobError, JobResponse, JobState, McpBatchChildResponse, McpBatchMode, McpBatchRequest,
    McpBatchResponse, McpBatchStatus, McpCallToolRequest, McpListToolsRequest, McpServerSummary,
};
use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use futures_util::stream::BoxStream;
use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    model::{
        CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientInfo,
        ClientRequest, JsonObject, ServerResult,
    },
    service::{PeerRequestOptions, RunningService, ServiceError},
    transport::{
        streamable_http_client::{
            SseError, StreamableHttpClient, StreamableHttpClientTransportConfig,
            StreamableHttpError, StreamableHttpPostResponse,
        },
        ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess,
    },
    ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    task::JoinSet,
    time::{sleep, sleep_until, timeout, Duration, Instant},
};

use crate::{
    audit::{write_mcp_batch_audit, McpBatchAuditRecord},
    config::{write_config_with_backup, Config},
    confirmation::{self, McpBatchConfirmationItem},
    jobs,
    jobs::{ManagedMcpSpec, TerminalEventHook},
    utils::bounded_mcp_argument_keys,
    AppState,
};

#[derive(Subcommand)]
pub(crate) enum McpConfigCommand {
    List,
    Add {
        server_id: String,
        url: String,
        #[arg(long, default_value = "streamable-http")]
        transport: String,
        #[arg(long, default_value_t = true)]
        enabled: bool,
    },
    Remove {
        server_id: String,
    },
    Enable {
        server_id: String,
    },
    Disable {
        server_id: String,
    },
}

type McpClient = RunningService<rmcp::RoleClient, ClientInfo>;
type McpClientFuture = Pin<Box<dyn Future<Output = Result<McpClient>> + Send>>;
type McpClientFactory =
    Arc<dyn Fn(ResolvedMcpServerConfig) -> McpClientFuture + Send + Sync + 'static>;

#[derive(Clone)]
struct PreparedMcpBatchCall {
    index: usize,
    id: Option<String>,
    payload: McpCallToolRequest,
    arguments: JsonObject,
    server: ResolvedMcpServerConfig,
    config_revision: String,
    argument_keys: Vec<String>,
    argument_key_count: usize,
    argument_keys_truncated: bool,
    argument_bytes: usize,
    argument_sha256: String,
    temporary_allowed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServerConfig {
    pub(crate) enabled: bool,
    pub(crate) transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone)]
struct ResolvedMcpServerConfig {
    transport: String,
    url: Option<String>,
    headers: HashMap<HeaderName, HeaderValue>,
}

#[derive(Clone)]
struct RedactedHttpClient(reqwest::Client);

#[derive(Debug)]
struct RedactedHttpClientError;

impl std::fmt::Display for RedactedHttpClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("downstream HTTP client error")
    }
}

impl std::error::Error for RedactedHttpClientError {}

fn redact_streamable_http_error<E>(
    _error: StreamableHttpError<E>,
) -> StreamableHttpError<RedactedHttpClientError>
where
    E: std::error::Error + Send + Sync + 'static,
{
    StreamableHttpError::UnexpectedServerResponse("downstream MCP request failed".into())
}

impl StreamableHttpClient for RedactedHttpClient {
    type Error = RedactedHttpClientError;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: rmcp::model::ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        <reqwest::Client as StreamableHttpClient>::post_message(
            &self.0,
            uri,
            message,
            session_id,
            auth_token,
            custom_headers,
        )
        .await
        .map_err(redact_streamable_http_error)
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        <reqwest::Client as StreamableHttpClient>::delete_session(
            &self.0,
            uri,
            session,
            auth_token,
            custom_headers,
        )
        .await
        .map_err(redact_streamable_http_error)
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<
        BoxStream<'static, Result<sse_stream::Sse, SseError>>,
        StreamableHttpError<Self::Error>,
    > {
        <reqwest::Client as StreamableHttpClient>::get_stream(
            &self.0,
            uri,
            session_id,
            last_event_id,
            auth_token,
            custom_headers,
        )
        .await
        .map_err(redact_streamable_http_error)
    }
}

pub(crate) fn mutate_servers(config_path: PathBuf, command: McpConfigCommand) -> Result<()> {
    let mut config = Config::load_or_default(&config_path)?;
    match command {
        McpConfigCommand::List => {
            println!("{}", serde_json::to_string_pretty(&config.mcp_servers)?);
            return Ok(());
        }
        McpConfigCommand::Add {
            server_id,
            url,
            transport,
            enabled,
        } => {
            config.mcp_servers.insert(
                server_id,
                McpServerConfig {
                    enabled,
                    transport,
                    url: Some(url),
                    headers: BTreeMap::new(),
                },
            );
        }
        McpConfigCommand::Remove { server_id } => {
            config.mcp_servers.remove(&server_id);
        }
        McpConfigCommand::Enable { server_id } => {
            let server = config
                .mcp_servers
                .get_mut(&server_id)
                .ok_or_else(|| anyhow!("mcp server not found: {server_id}"))?;
            server.enabled = true;
        }
        McpConfigCommand::Disable { server_id } => {
            let server = config
                .mcp_servers
                .get_mut(&server_id)
                .ok_or_else(|| anyhow!("mcp server not found: {server_id}"))?;
            server.enabled = false;
        }
    }
    validate_server_configs(&config.mcp_servers)?;
    write_config_with_backup(&config_path, &config)
}

pub(crate) fn validate_server_configs(servers: &BTreeMap<String, McpServerConfig>) -> Result<()> {
    for (server_id, server) in servers {
        if server_id.is_empty()
            || server_id.len() > 64
            || server_id.trim() != server_id
            || !server_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(anyhow!("mcp_server_id_invalid: {server_id}"));
        }
        let raw_endpoint = server.url.as_deref().unwrap_or_default();
        let endpoint = raw_endpoint.trim();
        match server.transport.as_str() {
            "streamable-http" => {
                validate_header_config(server_id, server)?;
                if endpoint.is_empty() {
                    return Err(anyhow!("mcp_server_url_missing: {server_id}"));
                }
                if endpoint != raw_endpoint {
                    return Err(anyhow!("mcp_server_url_invalid: {server_id}"));
                }
                let url = reqwest::Url::parse(endpoint)
                    .map_err(|_| anyhow!("mcp_server_url_invalid: {server_id}"))?;
                if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                    return Err(anyhow!("mcp_server_url_invalid: {server_id}"));
                }
                validate_header_endpoint_security(server_id, server, &url)?;
            }
            "stdio" => {
                validate_header_config(server_id, server)?;
                if endpoint.is_empty() {
                    return Err(anyhow!("mcp_server_command_missing: {server_id}"));
                }
                if endpoint != raw_endpoint || endpoint.chars().any(|character| character == '\0') {
                    return Err(anyhow!("mcp_server_command_invalid: {server_id}"));
                }
            }
            other => return Err(anyhow!("unsupported_mcp_transport: {server_id}: {other}")),
        }
    }
    Ok(())
}

const RESERVED_CUSTOM_HEADERS: &[&str] = &[
    "accept",
    "content-type",
    "mcp-session-id",
    "last-event-id",
    "mcp-protocol-version",
];

fn validate_header_config(server_id: &str, server: &McpServerConfig) -> Result<()> {
    if server.headers.is_empty() {
        return Ok(());
    }
    if server.transport != "streamable-http" {
        return Err(anyhow!("mcp_header_transport_invalid: {server_id}"));
    }
    let mut names = HashSet::new();
    for (name, reference) in &server.headers {
        let parsed = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| anyhow!("mcp_header_name_invalid: {server_id}"))?;
        let normalized = parsed.as_str().to_ascii_lowercase();
        if !names.insert(normalized) {
            return Err(anyhow!("mcp_header_duplicate: {server_id}"));
        }
        if RESERVED_CUSTOM_HEADERS
            .iter()
            .any(|reserved| parsed.as_str().eq_ignore_ascii_case(reserved))
        {
            return Err(anyhow!("mcp_header_reserved: {server_id}"));
        }
        crate::secrets::validate_reference(reference).map_err(|error| {
            anyhow!(match error {
                crate::secrets::SecretReferenceError::PlaintextRejected => {
                    format!("mcp_header_reference_plaintext_rejected: {server_id}")
                }
                crate::secrets::SecretReferenceError::InvalidReference => {
                    format!("mcp_header_reference_invalid: {server_id}")
                }
                crate::secrets::SecretReferenceError::Unavailable
                | crate::secrets::SecretReferenceError::InvalidValue => {
                    format!("mcp_header_reference_invalid: {server_id}")
                }
            })
        })?;
    }
    Ok(())
}

fn validate_header_endpoint_security(
    server_id: &str,
    server: &McpServerConfig,
    url: &reqwest::Url,
) -> Result<()> {
    if !server.headers.is_empty() && url.scheme() != "https" && !url_is_loopback(url) {
        return Err(anyhow!("mcp_header_https_required: {server_id}"));
    }
    Ok(())
}

fn url_is_loopback(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub(crate) fn server_config_revision(servers: &BTreeMap<String, McpServerConfig>) -> String {
    let bytes = serde_json::to_vec(servers).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

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
    let client = client(&server)
        .await
        .map_err(|_| anyhow!("mcp_client_connect_failed"))?;
    let tools = client
        .list_all_tools()
        .await
        .map_err(|_| anyhow!("mcp_request_failed"));
    close_client(client).await;
    Ok(json!({ "tools": tools? }))
}

pub(crate) async fn call_tool(
    state: &AppState,
    payload: McpCallToolRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> Result<Value> {
    let response = start_managed_call_with_factory(
        state,
        payload,
        request_source,
        terminal_event_hook,
        production_client_factory(),
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

pub(crate) async fn batch(
    state: &AppState,
    payload: McpBatchRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
) -> Result<Value> {
    let response = start_managed_batch_with_factory(
        state,
        payload,
        request_source,
        terminal_event_hook,
        production_client_factory(),
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn start_managed_batch_with_factory(
    state: &AppState,
    payload: McpBatchRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    client_factory: McpClientFactory,
) -> Result<McpBatchResponse> {
    let started = Instant::now();
    let batch_id = format!("batch_{}", uuid::Uuid::new_v4().simple());
    let prepared = match prepare_mcp_batch(state, &payload).await {
        Ok(prepared) => prepared,
        Err(error) => {
            write_batch_rejection_audit(
                state,
                &batch_id,
                request_source,
                &payload,
                "validation_rejected",
                &batch_error_code(&error.to_string()),
                started,
            )
            .await;
            return Err(error);
        }
    };
    let server_count = prepared
        .iter()
        .map(|call| call.payload.server_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let specs = prepared
        .iter()
        .map(|call| ManagedMcpSpec {
            agent_id: payload.agent_id.clone(),
            batch_id: Some(batch_id.clone()),
            batch_call_id: call.id.clone(),
            batch_index: Some(call.index),
            server_id: call.payload.server_id.clone(),
            tool_name: call.payload.tool_name.clone(),
            request_source: request_source.to_string(),
            argument_keys: call.argument_keys.clone(),
            argument_key_count: call.argument_key_count,
            argument_keys_truncated: call.argument_keys_truncated,
            argument_bytes: call.argument_bytes,
            argument_sha256: call.argument_sha256.clone(),
            config_revision: call.config_revision.clone(),
            terminal_event_hook: terminal_event_hook.clone(),
        })
        .collect::<Vec<_>>();
    let registrations = match jobs::register_mcp_batch(state, specs).await {
        Ok(registrations) => registrations,
        Err(reason) => {
            write_batch_rejection_audit(
                state,
                &batch_id,
                request_source,
                &payload,
                "capacity_rejected",
                &batch_error_code(&reason),
                started,
            )
            .await;
            return Err(anyhow!(reason));
        }
    };
    let child_refs = registrations
        .iter()
        .enumerate()
        .map(|(index, registration)| {
            (
                index,
                prepared[index].id.clone(),
                registration.info.job_id.clone(),
            )
        })
        .collect::<Vec<_>>();

    let confirmation_items = prepared
        .iter()
        .filter(|call| !call.temporary_allowed)
        .map(|call| McpBatchConfirmationItem {
            index: call.index,
            id: call.id.clone(),
            server_id: call.payload.server_id.clone(),
            tool_name: call.payload.tool_name.clone(),
            argument_keys: call.argument_keys.clone(),
            argument_key_count: call.argument_key_count,
            argument_keys_truncated: call.argument_keys_truncated,
            argument_bytes: call.argument_bytes,
            argument_sha256: call.argument_sha256.clone(),
        })
        .collect::<Vec<_>>();
    let confirmation_servers = confirmation_items
        .iter()
        .map(|item| item.server_id.as_str())
        .collect::<HashSet<_>>();
    let temporary_server = (confirmation_servers.len() == 1)
        .then(|| {
            confirmation_items
                .first()
                .map(|item| item.server_id.as_str())
        })
        .flatten();
    let batch_cancel = Arc::new(AtomicBool::new(false));
    let watcher = tokio::spawn(watch_batch_cancellation(
        registrations
            .iter()
            .map(|registration| registration.cancel_requested.clone())
            .collect(),
        batch_cancel.clone(),
    ));
    let confirmation_result = confirmation::authorize_mcp_batch_cancellable(
        state,
        &confirmation_items,
        temporary_server,
        batch_cancel,
    )
    .await;
    watcher.abort();
    for (index, registration) in registrations.iter().enumerate() {
        let child_authorization = if prepared[index].temporary_allowed {
            "temporary_mcp_allow"
        } else {
            confirmation_result.as_str()
        };
        let _ = jobs::set_mcp_authorization(state, &registration.info.job_id, child_authorization)
            .await;
    }

    if !mcp_authorization_allows(&confirmation_result) {
        let terminal = if confirmation_result == "cancelled" {
            JobState::Cancelled
        } else {
            JobState::Rejected
        };
        for registration in &registrations {
            let _ = jobs::finish_mcp_error(
                state,
                &registration.info.job_id,
                terminal,
                "mcp_batch_rejected",
                format!("MCP batch did not start: {confirmation_result}"),
                (terminal == JobState::Cancelled).then_some("batch_cancelled_before_start"),
                Some("aggregate_authorization_decision"),
            )
            .await;
        }
        let mut response = build_mcp_batch_response(
            state,
            &batch_id,
            &child_refs,
            0,
            Some(McpBatchStatus::Rejected),
        )
        .await?;
        response.error = Some(JobError {
            code: "mcp_batch_rejected".to_string(),
            message: format!("MCP batch did not start: {confirmation_result}"),
        });
        apply_batch_result_budget(&mut response)?;
        write_batch_audit(
            state,
            &batch_id,
            request_source,
            &payload,
            server_count,
            confirmation_items.len(),
            Some(confirmation_result),
            &child_refs,
            &response,
            started,
        )
        .await;
        return Ok(response);
    }

    let mode = payload.mode;
    let fail_fast = payload.fail_fast;
    let timeout_seconds = payload.effective_timeout_seconds();
    let coordinator_state = state.clone();
    let coordinator_batch_id = batch_id.clone();
    let coordinator_source = request_source.to_string();
    let coordinator_payload = payload.clone();
    let coordinator_refs = child_refs.clone();
    let coordinator_confirmation = confirmation_result.clone();
    tokio::spawn(async move {
        run_mcp_batch_coordinator(
            coordinator_state.clone(),
            prepared,
            registrations,
            mode,
            fail_fast,
            timeout_seconds,
            client_factory,
            coordinator_confirmation.clone(),
        )
        .await;
        if let Ok(response) = build_mcp_batch_response(
            &coordinator_state,
            &coordinator_batch_id,
            &coordinator_refs,
            0,
            None,
        )
        .await
        {
            write_batch_audit(
                &coordinator_state,
                &coordinator_batch_id,
                &coordinator_source,
                &coordinator_payload,
                server_count,
                confirmation_items.len(),
                Some(coordinator_confirmation),
                &coordinator_refs,
                &response,
                started,
            )
            .await;
        }
    });

    build_mcp_batch_response(
        state,
        &batch_id,
        &child_refs,
        payload.effective_wait_seconds(),
        None,
    )
    .await
}

async fn prepare_mcp_batch(
    state: &AppState,
    payload: &McpBatchRequest,
) -> Result<Vec<PreparedMcpBatchCall>> {
    if !(McpBatchRequest::MIN_CALLS..=McpBatchRequest::MAX_CALLS).contains(&payload.calls.len()) {
        return Err(anyhow!(
            "mcp_batch_call_count_invalid: calls={}; min={}; max={}",
            payload.calls.len(),
            McpBatchRequest::MIN_CALLS,
            McpBatchRequest::MAX_CALLS
        ));
    }
    let mut ids = HashSet::new();
    let mut aggregate_bytes = 0usize;
    let mut prepared = Vec::with_capacity(payload.calls.len());
    for (index, call) in payload.calls.iter().enumerate() {
        if let Some(id) = call.id.as_deref() {
            validate_batch_call_id(id)?;
            if !ids.insert(id.to_string()) {
                return Err(anyhow!("mcp_batch_call_id_duplicate: {id}"));
            }
        }
        validate_tool_name(&call.tool_name)?;
        let arguments = tool_arguments(call.arguments.clone())?;
        let encoded = serde_json::to_vec(&call.arguments)?;
        if encoded.len() > jobs::MAX_MCP_ARGUMENT_BYTES {
            return Err(anyhow!(
                "mcp_tool_arguments_too_large: index={index}; bytes={}; max={}",
                encoded.len(),
                jobs::MAX_MCP_ARGUMENT_BYTES
            ));
        }
        aggregate_bytes = aggregate_bytes.saturating_add(encoded.len());
        if aggregate_bytes > McpBatchRequest::MAX_AGGREGATE_ARGUMENT_BYTES {
            return Err(anyhow!(
                "mcp_batch_arguments_too_large: bytes={aggregate_bytes}; max={}",
                McpBatchRequest::MAX_AGGREGATE_ARGUMENT_BYTES
            ));
        }
        let argument_sha256 = format!("sha256:{:x}", Sha256::digest(&encoded));
        let (argument_keys, argument_key_count, argument_keys_truncated) =
            bounded_mcp_argument_keys(&call.arguments);
        let (config_revision, server) = server_config_snapshot(state, &call.server_id).await;
        let server = server.map_err(|reason| anyhow!(reason))?;
        let temporary_allowed = confirmation::temporary_mcp_allowed(state, &call.server_id).await;
        prepared.push(PreparedMcpBatchCall {
            index,
            id: call.id.clone(),
            payload: McpCallToolRequest {
                agent_id: payload.agent_id.clone(),
                server_id: call.server_id.clone(),
                tool_name: call.tool_name.clone(),
                arguments: call.arguments.clone(),
                wait_seconds: Some(0),
                timeout_seconds: payload.timeout_seconds,
            },
            arguments,
            server,
            config_revision,
            argument_keys,
            argument_key_count,
            argument_keys_truncated,
            argument_bytes: encoded.len(),
            argument_sha256,
            temporary_allowed,
        });
    }
    Ok(prepared)
}

#[allow(clippy::too_many_arguments)]
async fn run_mcp_batch_coordinator(
    state: AppState,
    prepared: Vec<PreparedMcpBatchCall>,
    registrations: Vec<jobs::ManagedMcpRegistration>,
    mode: McpBatchMode,
    fail_fast: bool,
    timeout_seconds: u64,
    client_factory: McpClientFactory,
    confirmation_result: String,
) {
    let stop = Arc::new(AtomicBool::new(false));
    match mode {
        McpBatchMode::Sequential => {
            for (call, registration) in prepared.into_iter().zip(registrations) {
                if fail_fast && stop.load(Ordering::Acquire) {
                    mark_batch_child_skipped(&state, &registration.info.job_id).await;
                    continue;
                }
                run_managed_call(
                    state.clone(),
                    call.payload,
                    call.arguments,
                    call.server,
                    registration.cancel_requested,
                    timeout_seconds,
                    registration.info.job_id.clone(),
                    client_factory.clone(),
                    Some(if call.temporary_allowed {
                        "temporary_mcp_allow".to_string()
                    } else {
                        confirmation_result.clone()
                    }),
                    fail_fast.then(|| stop.clone()),
                )
                .await;
                if fail_fast
                    && jobs::get_job(&state, &registration.info.job_id, 0)
                        .await
                        .is_ok_and(|job| hard_batch_failure(job.state))
                {
                    stop.store(true, Ordering::Release);
                }
            }
        }
        McpBatchMode::Parallel => {
            let mut tasks = JoinSet::new();
            for (call, registration) in prepared.into_iter().zip(registrations) {
                let task_state = state.clone();
                let task_factory = client_factory.clone();
                let task_stop = stop.clone();
                let task_confirmation = confirmation_result.clone();
                tasks.spawn(async move {
                    run_managed_call(
                        task_state.clone(),
                        call.payload,
                        call.arguments,
                        call.server,
                        registration.cancel_requested,
                        timeout_seconds,
                        registration.info.job_id.clone(),
                        task_factory,
                        Some(if call.temporary_allowed {
                            "temporary_mcp_allow".to_string()
                        } else {
                            task_confirmation
                        }),
                        fail_fast.then(|| task_stop.clone()),
                    )
                    .await;
                    if fail_fast
                        && jobs::get_job(&task_state, &registration.info.job_id, 0)
                            .await
                            .is_ok_and(|job| hard_batch_failure(job.state))
                    {
                        task_stop.store(true, Ordering::Release);
                    }
                });
            }
            while tasks.join_next().await.is_some() {}
        }
    }
}

async fn mark_batch_child_skipped(state: &AppState, job_id: &str) {
    let _ = jobs::finish_mcp_error(
        state,
        job_id,
        JobState::Skipped,
        "mcp_batch_fail_fast_skipped",
        "MCP batch fail-fast prevented this queued child from starting",
        None,
        Some("fail_fast_before_downstream_start"),
    )
    .await;
}

fn hard_batch_failure(state: JobState) -> bool {
    matches!(
        state,
        JobState::Failed
            | JobState::Rejected
            | JobState::Cancelled
            | JobState::TimedOut
            | JobState::Detached
            | JobState::UnknownAfterRestart
    )
}

async fn watch_batch_cancellation(flags: Vec<Arc<AtomicBool>>, batch_cancel: Arc<AtomicBool>) {
    loop {
        if flags.iter().any(|flag| flag.load(Ordering::Acquire)) {
            batch_cancel.store(true, Ordering::Release);
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
}

async fn build_mcp_batch_response(
    state: &AppState,
    batch_id: &str,
    child_refs: &[(usize, Option<String>, String)],
    wait_seconds: u64,
    forced_status: Option<McpBatchStatus>,
) -> Result<McpBatchResponse> {
    let deadline = Instant::now() + Duration::from_secs(wait_seconds.min(30));
    let mut details = Vec::new();
    loop {
        details.clear();
        let mut all_terminal = true;
        for (index, id, job_id) in child_refs {
            let detail = jobs::get_job_detail(state, job_id, 0)
                .await
                .map_err(|reason| anyhow!(reason))?;
            all_terminal &= detail.job.state.is_terminal();
            details.push(McpBatchChildResponse {
                index: *index,
                id: id.clone(),
                detail,
            });
        }
        if all_terminal || wait_seconds == 0 || Instant::now() >= deadline {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    let completed_inline = details
        .iter()
        .all(|result| result.detail.job.state.is_terminal());
    let status = forced_status.unwrap_or_else(|| {
        if !completed_inline {
            McpBatchStatus::Running
        } else if details
            .iter()
            .all(|result| result.detail.job.state == JobState::Completed)
        {
            McpBatchStatus::Completed
        } else {
            McpBatchStatus::CompletedWithErrors
        }
    });
    let mut response = McpBatchResponse {
        batch_id: batch_id.to_string(),
        status,
        completed_inline,
        poll_after_ms: if completed_inline { 0 } else { 1_000 },
        results: details,
        aggregate_truncated: false,
        aggregate_bytes: None,
        error: None,
    };
    apply_batch_result_budget(&mut response)?;
    Ok(response)
}

fn apply_batch_result_budget(response: &mut McpBatchResponse) -> Result<()> {
    let mut bytes = serde_json::to_vec(response)?.len();
    if bytes > McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES {
        response.aggregate_truncated = true;
        for index in (0..response.results.len()).rev() {
            let removed = {
                let child = &mut response.results[index];
                if child.detail.result.take().is_some() {
                    child.detail.result_truncated = true;
                    true
                } else {
                    false
                }
            };
            if removed {
                bytes = serde_json::to_vec(response)?.len();
                if bytes <= McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES {
                    break;
                }
            }
        }
    }
    let mut previous = None;
    for _ in 0..4 {
        let current = serde_json::to_vec(response)?.len();
        response.aggregate_bytes = Some(current);
        if previous == Some(current) {
            break;
        }
        previous = Some(current);
    }
    let final_bytes = serde_json::to_vec(response)?.len();
    response.aggregate_bytes = Some(final_bytes);
    if final_bytes > McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES {
        return Err(anyhow!(
            "mcp_batch_result_too_large_after_clipping: bytes={final_bytes}; max={}",
            McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES
        ));
    }
    Ok(())
}

async fn write_batch_rejection_audit(
    state: &AppState,
    batch_id: &str,
    request_source: &str,
    payload: &McpBatchRequest,
    outcome: &str,
    error_code: &str,
    started: Instant,
) {
    let config = state.config.read().await.clone();
    let server_count = payload
        .calls
        .iter()
        .map(|call| call.server_id.as_str())
        .collect::<HashSet<_>>()
        .len();
    let _ = write_mcp_batch_audit(
        &config,
        McpBatchAuditRecord {
            time: chrono::Utc::now(),
            tool: "mcp.batch".to_string(),
            batch_id: batch_id.to_string(),
            request_source: request_source.to_string(),
            call_count: payload.calls.len(),
            server_count,
            mode: match payload.mode {
                McpBatchMode::Parallel => "parallel",
                McpBatchMode::Sequential => "sequential",
            }
            .to_string(),
            fail_fast: payload.fail_fast,
            confirmation_required_count: 0,
            confirmation_result: None,
            child_job_ids: Vec::new(),
            outcome: outcome.to_string(),
            error_code: Some(error_code.to_string()),
            duration_ms: started.elapsed().as_millis(),
            truncated: false,
        },
    );
}

fn batch_error_code(message: &str) -> String {
    let end = message.find([':', ';']).unwrap_or(message.len());
    message[..end].chars().take(128).collect()
}

#[allow(clippy::too_many_arguments)]
async fn write_batch_audit(
    state: &AppState,
    batch_id: &str,
    request_source: &str,
    payload: &McpBatchRequest,
    server_count: usize,
    confirmation_required_count: usize,
    confirmation_result: Option<String>,
    child_refs: &[(usize, Option<String>, String)],
    response: &McpBatchResponse,
    started: Instant,
) {
    let config = state.config.read().await.clone();
    let _ = write_mcp_batch_audit(
        &config,
        McpBatchAuditRecord {
            time: chrono::Utc::now(),
            tool: "mcp.batch".to_string(),
            batch_id: batch_id.to_string(),
            request_source: request_source.to_string(),
            call_count: payload.calls.len(),
            server_count,
            mode: match payload.mode {
                McpBatchMode::Parallel => "parallel",
                McpBatchMode::Sequential => "sequential",
            }
            .to_string(),
            fail_fast: payload.fail_fast,
            confirmation_required_count,
            confirmation_result,
            child_job_ids: child_refs
                .iter()
                .map(|(_, _, job_id)| job_id.clone())
                .collect(),
            outcome: match response.status {
                McpBatchStatus::Running => "running",
                McpBatchStatus::Completed => "completed",
                McpBatchStatus::CompletedWithErrors => "completed_with_errors",
                McpBatchStatus::Rejected => "rejected",
            }
            .to_string(),
            error_code: response.error.as_ref().map(|error| error.code.clone()),
            duration_ms: started.elapsed().as_millis(),
            truncated: response.aggregate_truncated,
        },
    );
}

fn validate_batch_call_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 64
        || !id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
    {
        return Err(anyhow!("mcp_batch_call_id_invalid: {id}"));
    }
    Ok(())
}

async fn start_managed_call_with_factory(
    state: &AppState,
    payload: McpCallToolRequest,
    request_source: &str,
    terminal_event_hook: Option<TerminalEventHook>,
    client_factory: McpClientFactory,
) -> Result<JobResponse> {
    validate_tool_name(&payload.tool_name)?;
    let arguments = tool_arguments(payload.arguments.clone())?;
    let argument_bytes = serde_json::to_vec(&payload.arguments)?.len();
    if argument_bytes > jobs::MAX_MCP_ARGUMENT_BYTES {
        return Err(anyhow!(
            "mcp_tool_arguments_too_large: bytes={argument_bytes}; max={}",
            jobs::MAX_MCP_ARGUMENT_BYTES
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
    let registration = jobs::register_mcp_job(
        state,
        ManagedMcpSpec {
            agent_id: payload.agent_id.clone(),
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
    let job_id = registration.info.job_id.clone();
    let server = match server {
        Ok(server) => server,
        Err(reason) => {
            let code = reason
                .split_once(':')
                .map_or(reason.as_str(), |(code, _)| code)
                .to_string();
            let _ = jobs::set_mcp_preflight_rejection(state, &job_id).await;
            let _ = jobs::finish_mcp_error(
                state,
                &job_id,
                JobState::Rejected,
                code,
                reason,
                None,
                Some("server_config_validation"),
            )
            .await;
            return jobs::mcp_job_response(state, &job_id, 0)
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
        job_id.clone(),
        client_factory,
        None,
        None,
    ));
    jobs::mcp_job_response(state, &job_id, wait_seconds)
        .await
        .map_err(|reason| anyhow!(reason))
}

#[allow(clippy::too_many_arguments)]
async fn run_managed_call(
    state: AppState,
    payload: McpCallToolRequest,
    arguments: JsonObject,
    server: ResolvedMcpServerConfig,
    cancel_requested: Arc<AtomicBool>,
    timeout_seconds: u64,
    job_id: String,
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
    let _ = jobs::set_mcp_authorization(&state, &job_id, &authorization).await;
    if authorization == "cancelled" || cancel_requested.load(Ordering::Acquire) {
        let _ = jobs::finish_mcp_error(
            &state,
            &job_id,
            JobState::Cancelled,
            "mcp_cancelled",
            "MCP Job was cancelled before the downstream request started",
            Some("cancelled_before_request"),
            Some("local_cancel_before_downstream_request"),
        )
        .await;
        return;
    }
    if !mcp_authorization_allows(&authorization) {
        let _ = jobs::finish_mcp_error(
            &state,
            &job_id,
            JobState::Rejected,
            "mcp_tool_call_rejected",
            format!("MCP tool call rejected: {authorization}"),
            None,
            Some("authorization_decision"),
        )
        .await;
        return;
    }
    let _ = jobs::set_mcp_job_state(&state, &job_id, JobState::Queued).await;
    let _permit = match state
        .mcp_concurrency
        .acquire(&payload.server_id, cancel_requested.clone())
        .await
    {
        Ok(permit) => permit,
        Err(reason) if reason == "cancelled" => {
            let _ = jobs::finish_mcp_error(
                &state,
                &job_id,
                JobState::Cancelled,
                "mcp_cancelled",
                "MCP Job was cancelled while waiting for an execution slot",
                Some("cancelled_while_queued"),
                Some("local_cancel_before_downstream_request"),
            )
            .await;
            return;
        }
        Err(reason) => {
            let _ = jobs::finish_mcp_error(
                &state,
                &job_id,
                JobState::Failed,
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
        let _ = jobs::finish_mcp_error(
            &state,
            &job_id,
            JobState::Skipped,
            "mcp_batch_fail_fast_skipped",
            "MCP batch fail-fast prevented this queued child from starting",
            None,
            Some("fail_fast_before_downstream_start"),
        )
        .await;
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let _ = jobs::set_mcp_job_state(&state, &job_id, JobState::Starting).await;
    let downstream = tokio::select! {
        result = (client_factory)(server.clone()) => Some(result),
        _ = wait_for_cancel(cancel_requested.clone()) => {
            let _ = jobs::finish_mcp_error(
                &state,
                &job_id,
                JobState::Cancelled,
                "mcp_cancelled",
                "MCP Job was cancelled before the downstream client connected",
                Some("cancelled_before_request"),
                Some("local_cancel_before_downstream_request"),
            ).await;
            return;
        }
        _ = sleep_until(deadline) => None,
    };
    let Some(downstream) = downstream else {
        let _ = jobs::finish_mcp_error(
            &state,
            &job_id,
            JobState::TimedOut,
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
        Err(_error) => {
            let _ = jobs::finish_mcp_error(
                &state,
                &job_id,
                JobState::Failed,
                "mcp_client_connect_failed",
                "Downstream MCP client connection failed",
                None,
                Some("local_client_error"),
            )
            .await;
            return;
        }
    };
    if cancel_requested.load(Ordering::Acquire) {
        close_client(client).await;
        let _ = jobs::finish_mcp_error(
            &state,
            &job_id,
            JobState::Cancelled,
            "mcp_cancelled",
            "MCP Job was cancelled before the downstream request started",
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
            let _ = jobs::finish_mcp_error(
                &state,
                &job_id,
                JobState::Cancelled,
                "mcp_cancelled",
                "MCP Job was cancelled before the downstream request id was allocated",
                Some("cancelled_before_request"),
                Some("local_cancel_before_downstream_request"),
            ).await;
            return;
        }
        _ = sleep_until(deadline) => None,
    };
    let Some(handle) = handle else {
        close_client(client).await;
        let _ = jobs::finish_mcp_error(
            &state,
            &job_id,
            JobState::TimedOut,
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
        Err(_error) => {
            close_client(client).await;
            let _ = jobs::finish_mcp_error(
                &state,
                &job_id,
                JobState::Failed,
                "mcp_request_start_failed",
                "Downstream MCP request could not be started",
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
    if jobs::attach_mcp_request(&state, &job_id, peer.clone(), request_id.clone())
        .await
        .is_err()
    {
        let _ = timeout(
            Duration::from_secs(2),
            peer.notify_cancelled(CancelledNotificationParam {
                request_id,
                reason: Some("Job no longer active".to_string()),
            }),
        )
        .await;
        close_client(client).await;
        return;
    }

    tokio::select! {
        result = &mut response => {
            let after_cancel = cancel_requested.load(Ordering::Acquire);
            finish_from_response(&state, &job_id, result, after_cancel).await;
        }
        _ = wait_for_cancel(cancel_requested.clone()) => {
            finish_after_cancel(&state, &job_id, &mut response).await;
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
            let _ = jobs::finish_mcp_error(
                &state,
                &job_id,
                JobState::TimedOut,
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
    job_id: &str,
    response: &mut tokio::sync::oneshot::Receiver<Result<ServerResult, ServiceError>>,
) {
    match timeout(Duration::from_secs(2), response).await {
        Ok(result) => finish_from_response(state, job_id, result, true).await,
        Err(_) => {
            let _ = jobs::finish_mcp_error(
                state,
                job_id,
                JobState::Detached,
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
    job_id: &str,
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
            let _ = jobs::complete_mcp_result(state, job_id, value, downstream_error, cancel).await;
        }
        Ok(Ok(_)) => {
            let _ = jobs::finish_mcp_error(
                state,
                job_id,
                JobState::Failed,
                "mcp_unexpected_response",
                "Downstream MCP server returned an unexpected response type",
                after_cancel.then_some("response_after_cancel"),
                Some("remote_response"),
            )
            .await;
        }
        Ok(Err(error)) if after_cancel && explicit_cancel_error(&error) => {
            let _ = jobs::finish_mcp_error(
                state,
                job_id,
                JobState::Cancelled,
                "mcp_cancelled",
                "Downstream MCP request was cancelled",
                Some("cancelled"),
                Some("downstream_cancellation_response"),
            )
            .await;
        }
        Ok(Err(_error)) if after_cancel => {
            let _ = jobs::finish_mcp_error(
                state,
                job_id,
                JobState::Detached,
                "mcp_cancel_detached",
                "Downstream MCP request ended without a terminal response",
                Some("notification_sent"),
                Some("transport_or_remote_error_after_cancel"),
            )
            .await;
        }
        Ok(Err(_error)) => {
            let _ = jobs::finish_mcp_error(
                state,
                job_id,
                JobState::Failed,
                "mcp_request_failed",
                "Downstream MCP request failed",
                None,
                Some("downstream_or_transport_error"),
            )
            .await;
        }
        Err(_) if after_cancel => {
            let _ = jobs::finish_mcp_error(
                state,
                job_id,
                JobState::Detached,
                "mcp_cancel_detached",
                "Downstream transport closed after cancellation without terminal evidence",
                Some("notification_sent"),
                Some("transport_closed_after_cancel"),
            )
            .await;
        }
        Err(_) => {
            let _ = jobs::finish_mcp_error(
                state,
                job_id,
                JobState::Failed,
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
) -> (String, Result<ResolvedMcpServerConfig, String>) {
    let config = state.config.read().await;
    let revision = server_config_revision(&config.mcp_servers);
    let server = config.mcp_servers.get(server_id).cloned();
    drop(config);
    let result = match server {
        Some(server) => {
            resolve_selected_server(server_id, &server).map_err(|error| error.to_string())
        }
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
            validate_header_config(server_id, server)?;
            let endpoint = server.url.as_deref().unwrap_or_default().trim();
            if endpoint.is_empty() {
                return Err(anyhow!("mcp_server_url_missing: {server_id}"));
            }
            let url = reqwest::Url::parse(endpoint)
                .map_err(|_| anyhow!("mcp_server_url_invalid: {server_id}"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
                return Err(anyhow!("mcp_server_url_invalid: {server_id}"));
            }
            validate_header_endpoint_security(server_id, server, &url)?;
        }
        "stdio" => {
            validate_header_config(server_id, server)?;
            if server.url.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(anyhow!("mcp_server_command_missing: {server_id}"));
            }
        }
        other => return Err(anyhow!("unsupported_mcp_transport: {other}")),
    }
    Ok(())
}

fn resolve_selected_server(
    server_id: &str,
    server: &McpServerConfig,
) -> Result<ResolvedMcpServerConfig> {
    validate_selected_server(server_id, server)?;
    Ok(ResolvedMcpServerConfig {
        transport: server.transport.clone(),
        url: server.url.clone(),
        headers: resolve_headers(server_id, &server.headers)?,
    })
}

fn resolve_headers(
    server_id: &str,
    headers: &BTreeMap<String, String>,
) -> Result<HashMap<HeaderName, HeaderValue>> {
    let mut resolved = HashMap::with_capacity(headers.len());
    for (name, reference) in headers {
        let parsed_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| anyhow!("mcp_header_name_invalid: {server_id}"))?;
        let value = crate::secrets::resolve_reference(reference)
            .map_err(|error| mcp_header_secret_error(server_id, error))?;
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(anyhow!("mcp_header_value_invalid: {server_id}"));
        }
        let mut parsed_value = HeaderValue::from_str(&value)
            .map_err(|_| anyhow!("mcp_header_value_invalid: {server_id}"))?;
        parsed_value.set_sensitive(true);
        resolved.insert(parsed_name, parsed_value);
    }
    Ok(resolved)
}

fn mcp_header_secret_error(
    server_id: &str,
    error: crate::secrets::SecretReferenceError,
) -> anyhow::Error {
    let code = match error {
        crate::secrets::SecretReferenceError::PlaintextRejected => {
            "mcp_header_reference_plaintext_rejected"
        }
        crate::secrets::SecretReferenceError::InvalidReference => "mcp_header_reference_invalid",
        crate::secrets::SecretReferenceError::Unavailable => "mcp_header_secret_unavailable",
        crate::secrets::SecretReferenceError::InvalidValue => "mcp_header_value_invalid",
    };
    anyhow!("{code}: {server_id}")
}

async fn server_config(state: &AppState, server_id: &str) -> Result<ResolvedMcpServerConfig> {
    let config = state.config.read().await;
    let server = config
        .mcp_servers
        .get(server_id)
        .cloned()
        .ok_or_else(|| anyhow!("mcp_server_not_found: {server_id}"))?;
    drop(config);
    resolve_selected_server(server_id, &server)
}

fn production_client_factory() -> McpClientFactory {
    Arc::new(|server| Box::pin(async move { client(&server).await }))
}

async fn client(server: &ResolvedMcpServerConfig) -> Result<McpClient> {
    match server.transport.as_str() {
        "streamable-http" => {
            let url = server.url.clone().context("mcp_server_url_missing")?;
            let config = StreamableHttpClientTransportConfig::with_uri(url.clone())
                .custom_headers(server.headers.clone());
            if server.headers.is_empty() {
                let transport = StreamableHttpClientTransport::from_config(config);
                return Ok(ClientInfo::default().serve(transport).await?);
            }
            let endpoint =
                reqwest::Url::parse(&url).map_err(|_| anyhow!("mcp_server_url_invalid"))?;
            let mut builder = reqwest::Client::builder()
                .pool_max_idle_per_host(0)
                .redirect(reqwest::redirect::Policy::none());
            if url_is_loopback(&endpoint) {
                builder = builder.no_proxy();
            }
            let http_client = builder
                .build()
                .map_err(|_| anyhow!("mcp_http_client_init_failed"))?;
            let transport =
                StreamableHttpClientTransport::with_client(RedactedHttpClient(http_client), config);
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

fn tool_arguments(arguments: Value) -> Result<JsonObject> {
    match arguments {
        Value::Null => Ok(JsonObject::new()),
        Value::Object(map) => Ok(map),
        other => Err(anyhow!("mcp_tool_arguments_must_be_object: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    static PROXY_ENV_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

    async fn lock_proxy_environment() -> tokio::sync::MutexGuard<'static, ()> {
        PROXY_ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
    }

    fn server(transport: &str, endpoint: Option<&str>) -> McpServerConfig {
        McpServerConfig {
            enabled: true,
            transport: transport.to_string(),
            url: endpoint.map(str::to_string),
            headers: BTreeMap::new(),
        }
    }

    #[derive(Clone)]
    enum FakeBehavior {
        Fast,
        Delayed(u64),
        ToolError,
        Large,
        MediumLarge,
        WaitForCancel,
        IgnoreCancel,
    }

    #[derive(Default)]
    struct FakeConcurrency {
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    impl FakeConcurrency {
        fn enter(self: &Arc<Self>) -> FakeConcurrencyGuard {
            let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
            let _ = self
                .max_active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (active > current).then_some(active)
                });
            FakeConcurrencyGuard {
                tracker: self.clone(),
            }
        }

        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::Acquire)
        }
    }

    struct FakeConcurrencyGuard {
        tracker: Arc<FakeConcurrency>,
    }

    impl Drop for FakeConcurrencyGuard {
        fn drop(&mut self) {
            self.tracker.active.fetch_sub(1, Ordering::AcqRel);
        }
    }

    #[derive(Clone)]
    struct FakeMcpServer {
        behavior: FakeBehavior,
        calls: Arc<std::sync::Mutex<Vec<Value>>>,
        request_ids: Arc<std::sync::Mutex<Vec<String>>>,
        cancelled_ids: Arc<std::sync::Mutex<Vec<String>>>,
        context_cancelled: Arc<AtomicBool>,
        concurrency: Arc<FakeConcurrency>,
        events: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl FakeMcpServer {
        fn new(behavior: FakeBehavior) -> Self {
            Self::with_concurrency(behavior, Arc::new(FakeConcurrency::default()))
        }

        fn with_concurrency(behavior: FakeBehavior, concurrency: Arc<FakeConcurrency>) -> Self {
            Self {
                behavior,
                calls: Arc::new(std::sync::Mutex::new(Vec::new())),
                request_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
                cancelled_ids: Arc::new(std::sync::Mutex::new(Vec::new())),
                context_cancelled: Arc::new(AtomicBool::new(false)),
                concurrency,
                events: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }
    }

    impl rmcp::ServerHandler for FakeMcpServer {
        fn call_tool(
            &self,
            request: rmcp::model::CallToolRequestParams,
            context: rmcp::service::RequestContext<rmcp::RoleServer>,
        ) -> impl Future<Output = Result<rmcp::model::CallToolResult, rmcp::ErrorData>> + Send + '_
        {
            let behavior = self.behavior.clone();
            let calls = self.calls.clone();
            let request_ids = self.request_ids.clone();
            let context_cancelled = self.context_cancelled.clone();
            let concurrency = self.concurrency.clone();
            let events = self.events.clone();
            async move {
                let arguments = Value::Object(request.arguments.unwrap_or_default());
                let label = arguments
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("-")
                    .to_string();
                calls.lock().unwrap().push(arguments);
                request_ids.lock().unwrap().push(context.id.to_string());
                events.lock().unwrap().push(format!("start:{label}"));
                let _guard = concurrency.enter();
                let result = match behavior {
                    FakeBehavior::Fast => {
                        Ok(rmcp::model::CallToolResult::structured(json!({"ok": true})))
                    }
                    FakeBehavior::Delayed(milliseconds) => {
                        sleep(Duration::from_millis(milliseconds)).await;
                        Ok(rmcp::model::CallToolResult::structured(json!({
                            "delayed": true
                        })))
                    }
                    FakeBehavior::ToolError => {
                        Ok(rmcp::model::CallToolResult::structured_error(json!({
                            "code": "fake_error",
                            "message": "fake tool error"
                        })))
                    }
                    FakeBehavior::Large => Ok(rmcp::model::CallToolResult::structured(json!({
                        "blob": "大".repeat(220_000)
                    }))),
                    FakeBehavior::MediumLarge => {
                        Ok(rmcp::model::CallToolResult::structured(json!({
                            "blob": "x".repeat(240_000)
                        })))
                    }
                    FakeBehavior::WaitForCancel => {
                        context.ct.cancelled().await;
                        context_cancelled.store(true, Ordering::Release);
                        Err(rmcp::ErrorData::internal_error(
                            "cancelled by client".to_string(),
                            None,
                        ))
                    }
                    FakeBehavior::IgnoreCancel => {
                        sleep(Duration::from_secs(10)).await;
                        Ok(rmcp::model::CallToolResult::structured(
                            json!({"late": true}),
                        ))
                    }
                };
                events.lock().unwrap().push(format!("end:{label}"));
                result
            }
        }

        fn on_cancelled(
            &self,
            notification: rmcp::model::CancelledNotificationParam,
            _context: rmcp::service::NotificationContext<rmcp::RoleServer>,
        ) -> impl Future<Output = ()> + Send + '_ {
            self.cancelled_ids
                .lock()
                .unwrap()
                .push(notification.request_id.to_string());
            std::future::ready(())
        }

        fn get_info(&self) -> rmcp::model::ServerInfo {
            rmcp::model::ServerInfo::new(
                rmcp::model::ServerCapabilities::builder()
                    .enable_tools()
                    .build(),
            )
            .with_server_info(rmcp::model::Implementation::new("fake-mcp", "test"))
        }
    }

    fn fake_factory(server: FakeMcpServer) -> McpClientFactory {
        Arc::new(move |_config| {
            let server = server.clone();
            Box::pin(async move {
                let (client_io, server_io) = tokio::io::duplex(64 * 1024);
                tokio::spawn(async move {
                    if let Ok(running) = server.serve(server_io).await {
                        let _ = running.waiting().await;
                    }
                });
                Ok(ClientInfo::default().serve(client_io).await?)
            })
        })
    }

    fn routing_factory(
        servers: std::collections::HashMap<String, FakeMcpServer>,
    ) -> McpClientFactory {
        Arc::new(move |config| {
            let key = config.url.unwrap_or_default();
            let server = servers.get(&key).cloned();
            Box::pin(async move {
                let server = server.ok_or_else(|| anyhow!("fake_server_not_found: {key}"))?;
                let (client_io, server_io) = tokio::io::duplex(64 * 1024);
                tokio::spawn(async move {
                    if let Ok(running) = server.serve(server_io).await {
                        let _ = running.waiting().await;
                    }
                });
                Ok(ClientInfo::default().serve(client_io).await?)
            })
        })
    }

    async fn managed_test_state(max_active_jobs: usize) -> (AppState, PathBuf) {
        use std::collections::HashMap;
        use tokio::sync::{Mutex, RwLock};

        let root = std::env::temp_dir().join(format!(
            "agentic-managed-mcp-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace;
        config.limits.max_active_jobs = crate::config::MaxActiveJobs::Explicit(max_active_jobs);
        config.mcp_servers.insert(
            "fake".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "stdio".to_string(),
                url: Some("fake-command".to_string()),
                headers: BTreeMap::new(),
            },
        );
        let state = AppState {
            config_path: root.join("config.json"),
            config: Arc::new(RwLock::new(config)),
            runtime: crate::state::RuntimeModel::local(crate::state::CapabilityProfile::Normal),
            started_at: chrono::Utc::now(),
            boot_generation: "mcpboot00001".to_string(),
            supervised: false,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::jobs::McpConcurrency::new()),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(crate::jobs::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        };
        crate::confirmation::allow_mcp_server_for_test(&state, "fake").await;
        (state, root)
    }

    async fn add_fake_server(state: &AppState, server_id: &str, temporary_allow: bool) {
        state.config.write().await.mcp_servers.insert(
            server_id.to_string(),
            McpServerConfig {
                enabled: true,
                transport: "stdio".to_string(),
                url: Some(server_id.to_string()),
                headers: BTreeMap::new(),
            },
        );
        if temporary_allow {
            crate::confirmation::allow_mcp_server_for_test(state, server_id).await;
        }
    }

    fn batch_call(
        id: Option<&str>,
        server_id: &str,
        label: &str,
    ) -> agentic_gpt_protocol::McpBatchCall {
        agentic_gpt_protocol::McpBatchCall {
            id: id.map(str::to_string),
            server_id: server_id.to_string(),
            tool_name: "fake.tool".to_string(),
            arguments: json!({"label": label}),
        }
    }

    fn batch_request(
        calls: Vec<agentic_gpt_protocol::McpBatchCall>,
        mode: McpBatchMode,
        fail_fast: bool,
        wait_seconds: u64,
    ) -> McpBatchRequest {
        McpBatchRequest {
            agent_id: "test-agent".to_string(),
            calls,
            mode,
            fail_fast,
            wait_seconds: Some(wait_seconds),
            timeout_seconds: Some(30),
        }
    }

    fn managed_request(arguments: Value, wait_seconds: u64) -> McpCallToolRequest {
        managed_request_with_timeout(arguments, wait_seconds, 30)
    }

    fn managed_request_with_timeout(
        arguments: Value,
        wait_seconds: u64,
        timeout_seconds: u64,
    ) -> McpCallToolRequest {
        McpCallToolRequest {
            agent_id: "test-agent".to_string(),
            server_id: "fake".to_string(),
            tool_name: "fake.tool".to_string(),
            arguments,
            wait_seconds: Some(wait_seconds),
            timeout_seconds: Some(timeout_seconds),
        }
    }

    async fn wait_for_fake_request(server: &FakeMcpServer) {
        for _ in 0..200 {
            if !server.request_ids.lock().unwrap().is_empty() {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
        panic!("fake downstream request did not start");
    }

    #[tokio::test]
    async fn managed_mcp_fast_result_uses_real_rmcp_transport() {
        let (state, root) = managed_test_state(2).await;
        let fake = FakeMcpServer::new(FakeBehavior::Fast);
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({"value": 1, "secret": "super-secret-value"}), 5),
            "local:mcp.callTool",
            None,
            fake_factory(fake.clone()),
        )
        .await
        .unwrap();
        assert!(response.completed_inline);
        assert_eq!(response.status, JobState::Completed);
        assert_eq!(response.detail.job.kind, agentic_gpt_protocol::JobKind::Mcp);
        assert_eq!(
            response.detail.result.as_ref().unwrap()["structuredContent"]["ok"],
            true
        );
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            &[json!({"value": 1, "secret": "super-secret-value"})]
        );
        let audit =
            std::fs::read_to_string(root.join("workspace").join(".agentic-gpt-audit.jsonl"))
                .unwrap();
        assert!(audit.contains("\"program\":\"mcp.callTool\""));
        assert!(audit.contains("\"mcpServerId\":\"fake\""));
        assert!(audit.contains("\"mcpToolName\":\"fake.tool\""));
        assert!(audit.contains("\"argumentKeys\":[\"secret\",\"value\"]"));
        assert!(audit.contains("\"argumentKeyCount\":2"));
        assert!(audit.contains("\"argumentKeysTruncated\":false"));
        assert!(audit.contains("\"argumentSha256\":\"sha256:"));
        assert!(audit.contains("\"resultSha256\":\"sha256:"));
        assert!(!audit.contains("super-secret-value"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_deferred_result_is_retained_for_job_get() {
        let (state, root) = managed_test_state(2).await;
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 0),
            "local:mcp.callTool",
            None,
            fake_factory(FakeMcpServer::new(FakeBehavior::Delayed(150))),
        )
        .await
        .unwrap();
        assert!(!response.completed_inline);
        assert!(response.status.is_active());
        let detail = crate::jobs::get_job_detail(&state, &response.job_id, 2)
            .await
            .unwrap();
        assert_eq!(detail.job.state, JobState::Completed);
        assert_eq!(detail.result.unwrap()["structuredContent"]["delayed"], true);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_cancel_while_waiting_for_hub_confirmation_cleans_pending_sender() {
        let (state, root) = managed_test_state(1).await;
        state.temporary_mcp_allows.lock().await.clear();
        state
            .config
            .write()
            .await
            .confirmation_provider
            .set_legacy("hub")
            .unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        *state.hub_sender.lock().await = Some(sender);
        let fake = FakeMcpServer::new(FakeBehavior::Fast);
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({"secret": "must-not-appear-in-confirmation"}), 0),
            "local:mcp.callTool",
            None,
            fake_factory(fake.clone()),
        )
        .await
        .unwrap();
        let message = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        match message {
            agentic_gpt_protocol::AgentMessage::ConfirmationRequest { payload, .. } => {
                assert!(payload
                    .command_preview
                    .contains("Argument keys (showing 1 of 1, truncated=false): [secret]"));
                assert!(payload
                    .command_preview
                    .contains("Argument SHA-256: sha256:"));
                assert!(!payload
                    .command_preview
                    .contains("must-not-appear-in-confirmation"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
        assert_eq!(state.pending_confirmations.lock().await.len(), 1);
        let cancelled = crate::jobs::cancel_job(&state, &response.job_id)
            .await
            .unwrap();
        assert_eq!(cancelled.job.state, JobState::Cancelled);
        for _ in 0..100 {
            if state.pending_confirmations.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(state.pending_confirmations.lock().await.is_empty());
        assert!(fake.calls.lock().unwrap().is_empty());
        let detail = crate::jobs::get_job_detail(&state, &response.job_id, 1)
            .await
            .unwrap();
        assert_eq!(detail.job.state, JobState::Cancelled);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_tool_error_and_large_result_are_truthful() {
        let (state, root) = managed_test_state(2).await;
        let error = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 5),
            "local:mcp.callTool",
            None,
            fake_factory(FakeMcpServer::new(FakeBehavior::ToolError)),
        )
        .await
        .unwrap();
        assert_eq!(error.status, JobState::Failed);
        assert_eq!(error.detail.error.as_ref().unwrap().code, "mcp_tool_error");
        assert_eq!(error.detail.result.as_ref().unwrap()["isError"], true);

        let large = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 5),
            "local:mcp.callTool",
            None,
            fake_factory(FakeMcpServer::new(FakeBehavior::Large)),
        )
        .await
        .unwrap();
        assert_eq!(large.status, JobState::Completed);
        assert!(large.detail.result.is_none());
        assert!(large.detail.result_truncated);
        assert!(large.detail.result_bytes.unwrap() > jobs::MAX_MCP_RESULT_BYTES);
        assert!(large
            .detail
            .result_sha256
            .as_deref()
            .unwrap()
            .starts_with("sha256:"));
        assert!(large
            .detail
            .result_preview
            .as_deref()
            .unwrap()
            .contains("blob"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_timeout_sends_exact_cancel_notification() {
        let (state, root) = managed_test_state(1).await;
        let fake = FakeMcpServer::new(FakeBehavior::WaitForCancel);
        let response = start_managed_call_with_factory(
            &state,
            managed_request_with_timeout(json!({}), 3, 1),
            "local:mcp.callTool",
            None,
            fake_factory(fake.clone()),
        )
        .await
        .unwrap();
        assert_eq!(response.status, JobState::TimedOut);
        assert_eq!(
            response.detail.job.termination_evidence.as_deref(),
            Some("mcp_timeout_cancel_notification_sent")
        );
        assert!(fake.context_cancelled.load(Ordering::Acquire));
        assert_eq!(
            fake.cancelled_ids.lock().unwrap().as_slice(),
            fake.request_ids.lock().unwrap().as_slice()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_user_cancel_observes_remote_cancellation() {
        let (state, root) = managed_test_state(1).await;
        let fake = FakeMcpServer::new(FakeBehavior::WaitForCancel);
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 0),
            "local:mcp.callTool",
            None,
            fake_factory(fake.clone()),
        )
        .await
        .unwrap();
        wait_for_fake_request(&fake).await;
        let cancelling = crate::jobs::cancel_job(&state, &response.job_id)
            .await
            .unwrap();
        assert!(matches!(
            cancelling.job.state,
            JobState::CancelRequested | JobState::Detached
        ));
        let detail = crate::jobs::get_job_detail(&state, &response.job_id, 3)
            .await
            .unwrap();
        assert_eq!(detail.job.state, JobState::Detached);
        assert_eq!(
            detail.job.cancel_outcome.as_deref(),
            Some("notification_sent")
        );
        assert_eq!(
            detail.job.termination_evidence.as_deref(),
            Some("transport_or_remote_error_after_cancel")
        );
        assert!(fake.context_cancelled.load(Ordering::Acquire));
        assert_eq!(
            fake.cancelled_ids.lock().unwrap().as_slice(),
            fake.request_ids.lock().unwrap().as_slice()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_cancel_without_terminal_evidence_becomes_detached() {
        let (state, root) = managed_test_state(1).await;
        let fake = FakeMcpServer::new(FakeBehavior::IgnoreCancel);
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 0),
            "local:mcp.callTool",
            None,
            fake_factory(fake.clone()),
        )
        .await
        .unwrap();
        wait_for_fake_request(&fake).await;
        let cancelling = crate::jobs::cancel_job(&state, &response.job_id)
            .await
            .unwrap();
        assert_eq!(
            cancelling.job.cancel_outcome.as_deref(),
            Some("notification_sent")
        );
        let detail = crate::jobs::get_job_detail(&state, &response.job_id, 4)
            .await
            .unwrap();
        assert_eq!(detail.job.state, JobState::Detached);
        assert_eq!(
            detail.job.termination_evidence.as_deref(),
            Some("transport_or_remote_error_after_cancel")
        );
        assert_eq!(fake.cancelled_ids.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_shares_capacity_and_rejects_oversized_arguments() {
        let (state, root) = managed_test_state(1).await;
        let fake = FakeMcpServer::new(FakeBehavior::WaitForCancel);
        let first = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 0),
            "local:mcp.callTool",
            None,
            fake_factory(fake.clone()),
        )
        .await
        .unwrap();
        wait_for_fake_request(&fake).await;
        let capacity = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 0),
            "local:mcp.callTool",
            None,
            fake_factory(FakeMcpServer::new(FakeBehavior::Fast)),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(capacity.starts_with("max_active_jobs_reached"));
        let oversized = start_managed_call_with_factory(
            &state,
            managed_request(
                json!({"blob": "x".repeat(jobs::MAX_MCP_ARGUMENT_BYTES + 1)}),
                0,
            ),
            "local:mcp.callTool",
            None,
            fake_factory(FakeMcpServer::new(FakeBehavior::Fast)),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(oversized.starts_with("mcp_tool_arguments_too_large"));
        assert_eq!(
            crate::jobs::list_jobs(&state, agentic_gpt_protocol::JobListRequest::default(),)
                .await
                .len(),
            1
        );
        let _ = crate::jobs::cancel_job(&state, &first.job_id).await;
        let _ = crate::jobs::get_job_detail(&state, &first.job_id, 3).await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_batch_preflight_and_capacity_fail_atomically_before_confirmation() {
        let (state, root) = managed_test_state(20).await;
        let fake = FakeMcpServer::new(FakeBehavior::Fast);
        let factory = fake_factory(fake.clone());

        let duplicate = batch_request(
            vec![
                batch_call(Some("dup"), "fake", "a"),
                batch_call(Some("dup"), "fake", "b"),
            ],
            McpBatchMode::Parallel,
            false,
            0,
        );
        assert!(start_managed_batch_with_factory(
            &state,
            duplicate,
            "local:mcp.batch",
            None,
            factory.clone(),
        )
        .await
        .unwrap_err()
        .to_string()
        .starts_with("mcp_batch_call_id_duplicate"));

        let empty = batch_request(Vec::new(), McpBatchMode::Parallel, false, 0);
        assert!(start_managed_batch_with_factory(
            &state,
            empty,
            "local:mcp.batch",
            None,
            factory.clone(),
        )
        .await
        .unwrap_err()
        .to_string()
        .starts_with("mcp_batch_call_count_invalid"));

        let too_many = batch_request(
            (0..17)
                .map(|index| batch_call(Some(&format!("call-{index}")), "fake", "x"))
                .collect(),
            McpBatchMode::Parallel,
            false,
            0,
        );
        assert!(start_managed_batch_with_factory(
            &state,
            too_many,
            "local:mcp.batch",
            None,
            factory.clone(),
        )
        .await
        .unwrap_err()
        .to_string()
        .starts_with("mcp_batch_call_count_invalid"));

        let non_object = McpBatchRequest {
            calls: vec![agentic_gpt_protocol::McpBatchCall {
                id: Some("non-object".to_string()),
                server_id: "fake".to_string(),
                tool_name: "fake.tool".to_string(),
                arguments: json!([1, 2, 3]),
            }],
            ..batch_request(Vec::new(), McpBatchMode::Parallel, false, 0)
        };
        assert!(start_managed_batch_with_factory(
            &state,
            non_object,
            "local:mcp.batch",
            None,
            factory.clone(),
        )
        .await
        .unwrap_err()
        .to_string()
        .starts_with("mcp_tool_arguments_must_be_object"));

        let aggregate = McpBatchRequest {
            calls: (0..9)
                .map(|index| agentic_gpt_protocol::McpBatchCall {
                    id: Some(format!("large-{index}")),
                    server_id: "fake".to_string(),
                    tool_name: "fake.tool".to_string(),
                    arguments: json!({"blob": "x".repeat(245_000)}),
                })
                .collect(),
            ..batch_request(Vec::new(), McpBatchMode::Parallel, false, 0)
        };
        assert!(start_managed_batch_with_factory(
            &state,
            aggregate,
            "local:mcp.batch",
            None,
            factory,
        )
        .await
        .unwrap_err()
        .to_string()
        .starts_with("mcp_batch_arguments_too_large"));

        assert!(
            crate::jobs::list_jobs(&state, agentic_gpt_protocol::JobListRequest::default(),)
                .await
                .is_empty()
        );
        assert!(state.pending_confirmations.lock().await.is_empty());
        assert!(fake.calls.lock().unwrap().is_empty());

        let rejection_audit =
            std::fs::read_to_string(root.join("workspace").join(".agentic-gpt-audit.jsonl"))
                .unwrap();
        let rejection_records = rejection_audit
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rejection_records.len(), 5);
        assert!(rejection_records.iter().all(|record| {
            record["tool"] == "mcp.batch"
                && record["outcome"] == "validation_rejected"
                && record["childJobIds"] == json!([])
        }));
        assert!(!rejection_audit.contains("\"program\":\"mcp.callTool\""));

        let (capacity_state, capacity_root) = managed_test_state(1).await;
        let capacity_fake = FakeMcpServer::new(FakeBehavior::Fast);
        let capacity = batch_request(
            vec![
                batch_call(Some("a"), "fake", "a"),
                batch_call(Some("b"), "fake", "b"),
            ],
            McpBatchMode::Parallel,
            false,
            0,
        );
        assert!(start_managed_batch_with_factory(
            &capacity_state,
            capacity,
            "local:mcp.batch",
            None,
            fake_factory(capacity_fake.clone()),
        )
        .await
        .unwrap_err()
        .to_string()
        .starts_with("max_active_jobs_reached"));
        assert!(crate::jobs::list_jobs(
            &capacity_state,
            agentic_gpt_protocol::JobListRequest::default(),
        )
        .await
        .is_empty());
        assert!(capacity_state.pending_confirmations.lock().await.is_empty());
        assert!(capacity_fake.calls.lock().unwrap().is_empty());
        let capacity_audit = std::fs::read_to_string(
            capacity_root
                .join("workspace")
                .join(".agentic-gpt-audit.jsonl"),
        )
        .unwrap();
        let capacity_records = capacity_audit
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(capacity_records.len(), 1);
        assert_eq!(capacity_records[0]["tool"], "mcp.batch");
        assert_eq!(capacity_records[0]["outcome"], "capacity_rejected");
        assert_eq!(capacity_records[0]["errorCode"], "max_active_jobs_reached");
        assert_eq!(capacity_records[0]["childJobIds"], json!([]));
        assert!(!capacity_audit.contains("\"program\":\"mcp.callTool\""));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(capacity_root);
    }

    #[tokio::test]
    async fn mcp_batch_sequential_fail_fast_preserves_order_and_audit_correlation() {
        let (state, root) = managed_test_state(10).await;
        add_fake_server(&state, "error", true).await;
        add_fake_server(&state, "fast", true).await;
        let error = FakeMcpServer::new(FakeBehavior::ToolError);
        let fast = FakeMcpServer::new(FakeBehavior::Fast);
        let response = start_managed_batch_with_factory(
            &state,
            batch_request(
                vec![
                    batch_call(Some("first"), "error", "first"),
                    batch_call(Some("second"), "fast", "second"),
                    batch_call(Some("third"), "fast", "third"),
                ],
                McpBatchMode::Sequential,
                true,
                5,
            ),
            "local:mcp.batch",
            None,
            routing_factory(std::collections::HashMap::from([
                ("error".to_string(), error.clone()),
                ("fast".to_string(), fast.clone()),
            ])),
        )
        .await
        .unwrap();

        assert!(response.completed_inline);
        assert_eq!(response.status, McpBatchStatus::CompletedWithErrors);
        assert_eq!(
            response
                .results
                .iter()
                .map(|result| result.id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("first"), Some("second"), Some("third")]
        );
        assert_eq!(response.results[0].detail.job.state, JobState::Failed);
        assert_eq!(response.results[1].detail.job.state, JobState::Skipped);
        assert_eq!(response.results[2].detail.job.state, JobState::Skipped);
        for (index, result) in response.results.iter().enumerate() {
            assert_eq!(
                result.detail.job.batch_id.as_deref(),
                Some(response.batch_id.as_str())
            );
            assert_eq!(result.detail.job.batch_index, Some(index));
            assert_eq!(result.detail.job.batch_call_id, result.id);
        }
        assert_eq!(error.calls.lock().unwrap().len(), 1);
        assert!(fast.calls.lock().unwrap().is_empty());

        let audit_path = root.join("workspace").join(".agentic-gpt-audit.jsonl");
        let audit = loop {
            if let Ok(audit) = std::fs::read_to_string(&audit_path) {
                if audit.contains("\"tool\":\"mcp.batch\"") {
                    break audit;
                }
            }
            sleep(Duration::from_millis(10)).await;
        };
        let records = audit
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            records
                .iter()
                .filter(|record| record["tool"] == "mcp.batch")
                .count(),
            1
        );
        let child_records = records
            .iter()
            .filter(|record| record["program"] == "mcp.callTool")
            .collect::<Vec<_>>();
        assert_eq!(child_records.len(), 3);
        assert!(child_records
            .iter()
            .all(|record| record["batchId"] == response.batch_id));
        assert_eq!(
            child_records
                .iter()
                .map(|record| record["batchIndex"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(!audit.contains("first\":\""));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_batch_parallel_enforces_per_server_and_global_concurrency() {
        let (state, root) = managed_test_state(20).await;
        let same_server = FakeMcpServer::new(FakeBehavior::Delayed(120));
        let response = start_managed_batch_with_factory(
            &state,
            batch_request(
                (0..5)
                    .map(|index| {
                        batch_call(
                            Some(&format!("same-{index}")),
                            "fake",
                            &format!("same-{index}"),
                        )
                    })
                    .collect(),
                McpBatchMode::Parallel,
                false,
                5,
            ),
            "local:mcp.batch",
            None,
            fake_factory(same_server.clone()),
        )
        .await
        .unwrap();
        assert_eq!(response.status, McpBatchStatus::Completed);
        assert_eq!(same_server.concurrency.max_active(), 2);

        let (global_state, global_root) = managed_test_state(20).await;
        let tracker = Arc::new(FakeConcurrency::default());
        let mut routes = std::collections::HashMap::new();
        let mut calls = Vec::new();
        for index in 0..10 {
            let server_id = format!("server-{index}");
            add_fake_server(&global_state, &server_id, true).await;
            routes.insert(
                server_id.clone(),
                FakeMcpServer::with_concurrency(FakeBehavior::Delayed(120), tracker.clone()),
            );
            calls.push(batch_call(
                Some(&format!("global-{index}")),
                &server_id,
                &format!("global-{index}"),
            ));
        }
        let response = start_managed_batch_with_factory(
            &global_state,
            batch_request(calls, McpBatchMode::Parallel, false, 5),
            "local:mcp.batch",
            None,
            routing_factory(routes),
        )
        .await
        .unwrap();
        assert_eq!(response.status, McpBatchStatus::Completed);
        assert_eq!(tracker.max_active(), jobs::MCP_GLOBAL_CONCURRENCY);
        assert_eq!(global_state.mcp_concurrency.active(), 0);
        assert_eq!(global_state.mcp_concurrency.queued(), 0);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(global_root);
    }

    #[tokio::test]
    async fn mcp_batch_clips_late_results_to_the_aggregate_budget() {
        let (state, root) = managed_test_state(20).await;
        let fake = FakeMcpServer::new(FakeBehavior::MediumLarge);
        let response = start_managed_batch_with_factory(
            &state,
            batch_request(
                (0..6)
                    .map(|index| {
                        batch_call(
                            Some(&format!("large-{index}")),
                            "fake",
                            &format!("large-{index}"),
                        )
                    })
                    .collect(),
                McpBatchMode::Parallel,
                false,
                10,
            ),
            "local:mcp.batch",
            None,
            fake_factory(fake),
        )
        .await
        .unwrap();
        assert_eq!(response.status, McpBatchStatus::Completed);
        assert!(response.aggregate_truncated);
        assert!(response.aggregate_bytes.unwrap() <= McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES);
        assert_eq!(
            serde_json::to_vec(&response).unwrap().len(),
            response.aggregate_bytes.unwrap()
        );
        let retained = response
            .results
            .iter()
            .filter(|result| result.detail.result.is_some())
            .count();
        let clipped = response
            .results
            .iter()
            .filter(|result| result.detail.result.is_none() && result.detail.result_truncated)
            .count();
        assert!(retained > 0);
        assert!(clipped > 0);
        assert!(response.results.last().unwrap().detail.result.is_none());
        assert!(response
            .results
            .last()
            .unwrap()
            .detail
            .result_sha256
            .as_deref()
            .unwrap()
            .starts_with("sha256:"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_batch_single_server_uses_one_confirmation_and_can_grant_temporary_allow() {
        let (state, root) = managed_test_state(10).await;
        state.temporary_mcp_allows.lock().await.clear();
        state
            .config
            .write()
            .await
            .confirmation_provider
            .set_legacy("hub")
            .unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        *state.hub_sender.lock().await = Some(sender);
        let fake = FakeMcpServer::new(FakeBehavior::Fast);
        let task_state = state.clone();
        let task_fake = fake.clone();
        let task = tokio::spawn(async move {
            start_managed_batch_with_factory(
                &task_state,
                batch_request(
                    vec![
                        batch_call(Some("first"), "fake", "secret-first-value"),
                        batch_call(Some("second"), "fake", "secret-second-value"),
                    ],
                    McpBatchMode::Parallel,
                    false,
                    5,
                ),
                "local:mcp.batch",
                None,
                fake_factory(task_fake),
            )
            .await
        });
        let message = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let request_id = match message {
            agentic_gpt_protocol::AgentMessage::ConfirmationRequest {
                request_id,
                payload,
                ..
            } => {
                assert_eq!(payload.program, "mcp.batch");
                assert_eq!(payload.kind.as_deref(), Some("mcpBatchSingleServer"));
                assert_eq!(payload.server_id.as_deref(), Some("fake"));
                assert_eq!(payload.tool_name, None);
                assert!(payload
                    .command_preview
                    .contains("Calls requiring confirmation: 2"));
                assert!(payload.command_preview.contains("#0 id=first"));
                assert!(payload.command_preview.contains("#1 id=second"));
                assert!(!payload.command_preview.contains("secret-first-value"));
                assert!(!payload.command_preview.contains("secret-second-value"));
                request_id
            }
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(receiver.try_recv().is_err());
        state
            .pending_confirmations
            .lock()
            .await
            .remove(&request_id)
            .unwrap()
            .send("allow_mcp_server_15m".to_string())
            .unwrap();
        let response = timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.status, McpBatchStatus::Completed);
        assert_eq!(fake.calls.lock().unwrap().len(), 2);
        assert!(confirmation::temporary_mcp_allowed(&state, "fake").await);
        assert!(state.pending_confirmations.lock().await.is_empty());
        assert!(receiver.try_recv().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_batch_multi_server_uses_one_non_scoped_confirmation_and_rejects_all() {
        let (state, root) = managed_test_state(10).await;
        state.temporary_mcp_allows.lock().await.clear();
        add_fake_server(&state, "second", false).await;
        state
            .config
            .write()
            .await
            .confirmation_provider
            .set_legacy("hub")
            .unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        *state.hub_sender.lock().await = Some(sender);
        let first = FakeMcpServer::new(FakeBehavior::Fast);
        let second = FakeMcpServer::new(FakeBehavior::Fast);
        let task_state = state.clone();
        let task_first = first.clone();
        let task_second = second.clone();
        let task = tokio::spawn(async move {
            start_managed_batch_with_factory(
                &task_state,
                batch_request(
                    vec![
                        batch_call(Some("first"), "fake", "first-value"),
                        batch_call(Some("second"), "second", "second-value"),
                    ],
                    McpBatchMode::Parallel,
                    false,
                    5,
                ),
                "local:mcp.batch",
                None,
                routing_factory(std::collections::HashMap::from([
                    ("fake-command".to_string(), task_first),
                    ("second".to_string(), task_second),
                ])),
            )
            .await
        });
        let message = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let request_id = match message {
            agentic_gpt_protocol::AgentMessage::ConfirmationRequest {
                request_id,
                payload,
                ..
            } => {
                assert_eq!(payload.kind.as_deref(), Some("mcpBatch"));
                assert_eq!(payload.server_id, None);
                assert_eq!(payload.tool_name, None);
                assert!(payload
                    .command_preview
                    .contains("Calls requiring confirmation: 2"));
                assert!(!payload.command_preview.contains("first-value"));
                assert!(!payload.command_preview.contains("second-value"));
                request_id
            }
            other => panic!("unexpected message: {other:?}"),
        };
        assert!(receiver.try_recv().is_err());
        state
            .pending_confirmations
            .lock()
            .await
            .remove(&request_id)
            .unwrap()
            .send("deny".to_string())
            .unwrap();
        let response = timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.status, McpBatchStatus::Rejected);
        assert!(response.completed_inline);
        assert_eq!(response.error.as_ref().unwrap().code, "mcp_batch_rejected");
        assert!(response
            .results
            .iter()
            .all(|result| result.detail.job.state == JobState::Rejected));
        assert!(first.calls.lock().unwrap().is_empty());
        assert!(second.calls.lock().unwrap().is_empty());
        assert!(!confirmation::temporary_mcp_allowed(&state, "fake").await);
        assert!(!confirmation::temporary_mcp_allowed(&state, "second").await);
        assert!(state.pending_confirmations.lock().await.is_empty());
        assert!(receiver.try_recv().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_batch_child_cancel_during_aggregate_confirmation_cancels_all_before_start() {
        let (state, root) = managed_test_state(10).await;
        state.temporary_mcp_allows.lock().await.clear();
        state
            .config
            .write()
            .await
            .confirmation_provider
            .set_legacy("hub")
            .unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        *state.hub_sender.lock().await = Some(sender);
        let fake = FakeMcpServer::new(FakeBehavior::Fast);
        let task_state = state.clone();
        let task_fake = fake.clone();
        let task = tokio::spawn(async move {
            start_managed_batch_with_factory(
                &task_state,
                batch_request(
                    vec![
                        batch_call(Some("first"), "fake", "first"),
                        batch_call(Some("second"), "fake", "second"),
                    ],
                    McpBatchMode::Parallel,
                    false,
                    5,
                ),
                "local:mcp.batch",
                None,
                fake_factory(task_fake),
            )
            .await
        });
        let message = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        match message {
            agentic_gpt_protocol::AgentMessage::ConfirmationRequest { payload, .. } => {
                assert_eq!(payload.kind.as_deref(), Some("mcpBatchSingleServer"));
            }
            other => panic!("unexpected message: {other:?}"),
        }
        let jobs =
            crate::jobs::list_jobs(&state, agentic_gpt_protocol::JobListRequest::default()).await;
        assert_eq!(jobs.len(), 2);
        assert!(jobs
            .iter()
            .all(|job| job.state == JobState::WaitingConfirmation));
        let cancelled_job_id = jobs[0].job_id.clone();
        let cancelled = crate::jobs::cancel_job(&state, &cancelled_job_id)
            .await
            .unwrap();
        assert_eq!(cancelled.job.state, JobState::Cancelled);

        let response = timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.status, McpBatchStatus::Rejected);
        assert!(response.completed_inline);
        assert_eq!(response.error.as_ref().unwrap().code, "mcp_batch_rejected");
        assert!(response
            .results
            .iter()
            .all(|result| result.detail.job.state == JobState::Cancelled));
        let directly_cancelled = response
            .results
            .iter()
            .find(|result| result.detail.job.job_id == cancelled_job_id)
            .unwrap();
        assert_eq!(
            directly_cancelled
                .detail
                .job
                .termination_evidence
                .as_deref(),
            Some("local_cancel_before_downstream_request")
        );
        assert!(response
            .results
            .iter()
            .filter(|result| { result.detail.job.job_id != cancelled_job_id })
            .all(|result| {
                result.detail.job.termination_evidence.as_deref()
                    == Some("aggregate_authorization_decision")
            }));
        for _ in 0..100 {
            if state.pending_confirmations.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
        assert!(state.pending_confirmations.lock().await.is_empty());
        assert!(fake.calls.lock().unwrap().is_empty());
        assert!(receiver.try_recv().is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    struct MockHttpRequest {
        method: String,
        headers: String,
        body: Value,
    }

    async fn read_http_request(stream: &mut TcpStream) -> Option<MockHttpRequest> {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 8192];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position;
            }
        };
        let header_text = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
        let content_length = header_text
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        let body_start = header_end + 4;
        while bytes.len() < body_start + content_length {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        let body = serde_json::from_slice(&bytes[body_start..body_start + content_length]).ok()?;
        let method = header_text
            .lines()
            .next()?
            .split_whitespace()
            .next()?
            .to_owned();
        Some(MockHttpRequest {
            method,
            headers: header_text,
            body,
        })
    }

    async fn spawn_http_mock() -> (
        String,
        Arc<std::sync::Mutex<Vec<MockHttpRequest>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let captured = captured.clone();
                tokio::spawn(async move {
                    let Some(request) = read_http_request(&mut stream).await else {
                        return;
                    };
                    let method = request.body["method"].as_str();
                    let response = if request.method == "GET" {
                        "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    } else if method == Some("notifications/initialized") {
                        "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    } else {
                        let result = match method {
                            Some("initialize") => json!({
                                "protocolVersion": "2025-06-18",
                                "capabilities": {"tools": {}},
                                "serverInfo": {"name": "header-mock", "version": "test"}
                            }),
                            Some("tools/list") => json!({"tools": []}),
                            Some("tools/call") => json!({
                                "content": [{"type": "text", "text": "ok"}],
                                "isError": false
                            }),
                            _ => json!({}),
                        };
                        let body = json!({
                            "jsonrpc": "2.0",
                            "id": request.body.get("id").cloned().unwrap_or(Value::Null),
                            "result": result,
                        })
                        .to_string();
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: test-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        )
                    };
                    captured.lock().unwrap().push(request);
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        (url, requests, task)
    }

    async fn spawn_failing_http_mock(
        canary: &str,
        fail_initialize: bool,
    ) -> (
        String,
        Arc<std::sync::Mutex<Vec<MockHttpRequest>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let canary = canary.to_string();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let captured = captured.clone();
                let canary = canary.clone();
                tokio::spawn(async move {
                    let Some(request) = read_http_request(&mut stream).await else {
                        return;
                    };
                    let method = request.body["method"].as_str();
                    let fail = method == Some("tools/call")
                        || (fail_initialize && method == Some("initialize"));
                    let response = if fail {
                        let body = format!("downstream failure canary={canary}");
                        format!(
                            "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer error=\"{canary}\"\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        )
                    } else if request.method == "GET" {
                        "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    } else if method == Some("notifications/initialized") {
                        "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    } else {
                        let result = match method {
                            Some("initialize") => json!({
                                "protocolVersion": "2025-06-18",
                                "capabilities": {"tools": {}},
                                "serverInfo": {"name": "header-failure-mock", "version": "test"}
                            }),
                            Some("tools/list") => json!({"tools": []}),
                            _ => json!({}),
                        };
                        let body = json!({
                            "jsonrpc": "2.0",
                            "id": request.body.get("id").cloned().unwrap_or(Value::Null),
                            "result": result,
                        })
                        .to_string();
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: failure-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        )
                    };
                    captured.lock().unwrap().push(request);
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        (url, requests, task)
    }

    async fn spawn_proxy_probe() -> (
        String,
        Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let captured = captured.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let Ok(read) =
                            timeout(Duration::from_millis(500), stream.read(&mut chunk)).await
                        else {
                            break;
                        };
                        let Ok(read) = read else {
                            break;
                        };
                        if read == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&chunk[..read]);
                        if bytes.windows(4).any(|window| window == b"\r\n\r\n")
                            || bytes.len() >= 16 * 1024
                        {
                            break;
                        }
                    }
                    captured.lock().unwrap().push(bytes);
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });
        (url, requests, task)
    }

    async fn spawn_cross_origin_redirect_mocks() -> (
        String,
        Arc<std::sync::Mutex<Vec<MockHttpRequest>>>,
        tokio::task::JoinHandle<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_url = format!("http://{}/mcp", target_listener.local_addr().unwrap());
        let target_requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_target = target_requests.clone();
        let target_task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = target_listener.accept().await else {
                    break;
                };
                let captured_target = captured_target.clone();
                tokio::spawn(async move {
                    let Some(request) = read_http_request(&mut stream).await else {
                        return;
                    };
                    captured_target.lock().unwrap().push(request);
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });

        let redirect_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_url = format!("http://{}/mcp", redirect_listener.local_addr().unwrap());
        let redirect_task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = redirect_listener.accept().await else {
                    break;
                };
                let location = target_url.clone();
                tokio::spawn(async move {
                    if read_http_request(&mut stream).await.is_some() {
                        let response = format!(
                            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                    }
                });
            }
        });
        (redirect_url, target_requests, redirect_task, target_task)
    }

    #[tokio::test]
    async fn streamable_http_header_redirect_does_not_forward_to_other_origin() {
        let root =
            std::env::temp_dir().join(format!("agentic-mcp-redirect-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let env_name = format!("AGENTIC_MCP_REDIRECT_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&env_name, "Bearer redirect-secret");
        let (url, target_requests, redirect_task, target_task) =
            spawn_cross_origin_redirect_mocks().await;
        let mut raw = server("streamable-http", Some(&url));
        raw.headers
            .insert("Authorization".to_string(), format!("env:{env_name}"));
        let resolved = resolve_selected_server("redirect", &raw).unwrap();
        let result = client(&resolved).await;
        assert!(result.is_err());
        redirect_task.abort();
        target_task.abort();
        assert!(target_requests.lock().unwrap().is_empty());
        std::env::remove_var(&env_name);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn streamable_http_headers_resolve_from_env_and_file_and_reach_requests() {
        let root = std::env::temp_dir().join(format!("agentic-mcp-http-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let file_secret = root.join("tenant-header");
        std::fs::write(&file_secret, "file-tenant\r\n").unwrap();
        let env_name = format!("AGENTIC_MCP_HEADER_{}", uuid::Uuid::new_v4().simple());
        let env_secret = "Bearer env-token";
        std::env::set_var(&env_name, env_secret);
        let (url, requests, task) = spawn_http_mock().await;
        let mut raw = server("streamable-http", Some(&url));
        raw.headers
            .insert("Authorization".to_string(), format!("env:{env_name}"));
        raw.headers.insert(
            "X-Tenant".to_string(),
            format!("file:{}", file_secret.display()),
        );

        let resolved = resolve_selected_server("headers", &raw).unwrap();
        assert!(resolved
            .headers
            .get(&HeaderName::from_static("authorization"))
            .unwrap()
            .is_sensitive());
        let client = client(&resolved).await.unwrap();
        assert!(client.list_all_tools().await.unwrap().is_empty());
        close_client(client).await;
        task.abort();
        let requests = requests.lock().unwrap();
        let post_requests = requests
            .iter()
            .filter(|request| request.method == "POST")
            .collect::<Vec<_>>();
        assert!(post_requests.len() >= 3);
        for request in post_requests {
            let headers = request.headers.to_ascii_lowercase();
            assert!(headers.contains("authorization: bearer env-token"));
            assert!(headers.contains("x-tenant: file-tenant"));
        }
        let serialized = serde_json::to_string(&raw).unwrap();
        assert!(!serialized.contains(env_secret));
        assert!(!serialized.contains("file-tenant"));
        std::env::remove_var(&env_name);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn streamable_http_without_headers_remains_compatible() {
        let _proxy_env_guard = lock_proxy_environment().await;
        let (url, requests, task) = spawn_http_mock().await;
        let raw = server("streamable-http", Some(&url));
        let resolved = resolve_selected_server("no-headers", &raw).unwrap();
        let client = client(&resolved).await.unwrap();
        assert!(client.list_all_tools().await.unwrap().is_empty());
        close_client(client).await;
        task.abort();
        let requests = requests.lock().unwrap();
        assert!(requests.iter().any(|request| request.method == "POST"));
        assert!(requests.iter().all(|request| {
            let headers = request.headers.to_ascii_lowercase();
            !headers.contains("authorization:") && !headers.contains("x-tenant:")
        }));
    }

    #[tokio::test]
    async fn managed_mcp_static_headers_do_not_leak_into_audit_or_job_output() {
        let (state, root) = managed_test_state(2).await;
        let (url, requests, task) = spawn_http_mock().await;
        let env_name = format!("AGENTIC_MCP_AUDIT_{}", uuid::Uuid::new_v4().simple());
        let secret = "Bearer audit-secret";
        std::env::set_var(&env_name, secret);
        {
            let mut config = state.config.write().await;
            let server = config.mcp_servers.get_mut("fake").unwrap();
            server.transport = "streamable-http".to_string();
            server.url = Some(url);
            server
                .headers
                .insert("Authorization".to_string(), format!("env:{env_name}"));
        }
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 5),
            "local:mcp.callTool",
            None,
            production_client_factory(),
        )
        .await
        .unwrap();
        assert_eq!(response.status, JobState::Completed);
        let detail = crate::jobs::get_job_detail(&state, &response.job_id, 0)
            .await
            .unwrap();
        let audit =
            std::fs::read_to_string(root.join("workspace").join(".agentic-gpt-audit.jsonl"))
                .unwrap();
        let serialized_config = serde_json::to_string(&state.config.read().await.clone()).unwrap();
        assert!(!serde_json::to_string(&detail).unwrap().contains(secret));
        assert!(!audit.contains(secret));
        assert!(!serialized_config.contains(secret));
        task.abort();
        let requests = requests.lock().unwrap();
        assert!(requests.iter().any(|request| {
            request.method == "POST"
                && request
                    .headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer audit-secret")
        }));
        std::env::remove_var(&env_name);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn managed_mcp_downstream_failures_redact_body_headers_and_worker_diagnostics() {
        let (state, root) = managed_test_state(2).await;
        let canary = "mcp-downstream-canary";
        let (url, requests, task) = spawn_failing_http_mock(canary, false).await;
        let env_name = format!("AGENTIC_MCP_FAILURE_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&env_name, "Bearer failure-token");
        {
            let mut config = state.config.write().await;
            let server = config.mcp_servers.get_mut("fake").unwrap();
            server.transport = "streamable-http".to_string();
            server.url = Some(url);
            server
                .headers
                .insert("Authorization".to_string(), format!("env:{env_name}"));
        }
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 5),
            "local:mcp.callTool",
            None,
            production_client_factory(),
        )
        .await
        .unwrap();
        assert_eq!(response.status, JobState::Failed);
        let detail = crate::jobs::get_job_detail(&state, &response.job_id, 0)
            .await
            .unwrap();
        let audit =
            std::fs::read_to_string(root.join("workspace").join(".agentic-gpt-audit.jsonl"))
                .unwrap();
        assert_eq!(detail.job.state, JobState::Failed);
        assert_eq!(detail.error.as_ref().unwrap().code, "mcp_request_failed");
        assert!(!serde_json::to_string(&response).unwrap().contains(canary));
        assert!(!serde_json::to_string(&detail).unwrap().contains(canary));
        assert!(!audit.contains(canary));

        let (init_url, init_requests, init_task) = spawn_failing_http_mock(canary, true).await;
        let mut init_raw = server("streamable-http", Some(&init_url));
        init_raw
            .headers
            .insert("Authorization".to_string(), format!("env:{env_name}"));
        let init_resolved = resolve_selected_server("init-failure", &init_raw).unwrap();
        let init_error = client(&init_resolved).await.unwrap_err();
        let worker_diagnostic = format!("worker stderr: {init_error:?}");
        assert!(!worker_diagnostic.contains(canary));

        task.abort();
        init_task.abort();
        assert!(requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| { request.method == "POST" && request.body["method"] == "tools/call" }));
        assert!(init_requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| { request.method == "POST" && request.body["method"] == "initialize" }));
        std::env::remove_var(&env_name);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_snapshots_keep_resolved_headers_across_reload() {
        let (state, root) = managed_test_state(2).await;
        let (url, requests, task) = spawn_http_mock().await;
        let old_name = format!("AGENTIC_MCP_OLD_{}", uuid::Uuid::new_v4().simple());
        let new_name = format!("AGENTIC_MCP_NEW_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&old_name, "Bearer old-token");
        std::env::set_var(&new_name, "Bearer new-token");
        {
            let mut config = state.config.write().await;
            let server = config.mcp_servers.get_mut("fake").unwrap();
            server.transport = "streamable-http".to_string();
            server.url = Some(url.clone());
            server
                .headers
                .insert("Authorization".to_string(), format!("env:{old_name}"));
            std::fs::write(
                &state.config_path,
                serde_json::to_vec_pretty(&*config).unwrap(),
            )
            .unwrap();
        }
        let (old_revision, old_server) = server_config_snapshot(&state, "fake").await;
        let old_server = old_server.unwrap();

        let mut reloaded = state.config.read().await.clone();
        reloaded
            .mcp_servers
            .get_mut("fake")
            .unwrap()
            .headers
            .insert("Authorization".to_string(), format!("env:{new_name}"));
        std::fs::write(
            &state.config_path,
            serde_json::to_vec_pretty(&reloaded).unwrap(),
        )
        .unwrap();
        crate::reload_standalone_live_config_once(&state)
            .await
            .unwrap();

        let (new_revision, new_server) = server_config_snapshot(&state, "fake").await;
        let new_server = new_server.unwrap();
        assert_ne!(old_revision, new_revision);
        assert_eq!(
            old_server
                .headers
                .get(&HeaderName::from_static("authorization"))
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer old-token"
        );
        assert_eq!(
            new_server
                .headers
                .get(&HeaderName::from_static("authorization"))
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer new-token"
        );

        let old_client = client(&old_server).await.unwrap();
        assert!(old_client.list_all_tools().await.unwrap().is_empty());
        close_client(old_client).await;
        let new_client = client(&new_server).await.unwrap();
        assert!(new_client.list_all_tools().await.unwrap().is_empty());
        close_client(new_client).await;

        let serialized_config = serde_json::to_string(&state.config.read().await.clone()).unwrap();
        task.abort();
        let requests = requests.lock().unwrap();
        let post_headers = requests
            .iter()
            .filter(|request| request.method == "POST")
            .map(|request| request.headers.to_ascii_lowercase())
            .collect::<Vec<_>>();
        assert!(post_headers
            .iter()
            .any(|headers| headers.contains("authorization: bearer old-token")));
        assert!(post_headers
            .iter()
            .any(|headers| headers.contains("authorization: bearer new-token")));
        assert!(!serialized_config.contains("Bearer old-token"));
        assert!(!serialized_config.contains("Bearer new-token"));
        std::env::remove_var(&old_name);
        std::env::remove_var(&new_name);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mcp_revision_ignores_rotated_secret_source_value() {
        let (state, root) = managed_test_state(2).await;
        let env_name = format!("AGENTIC_MCP_ROTATED_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&env_name, "Bearer first-token");
        {
            let mut config = state.config.write().await;
            let server = config.mcp_servers.get_mut("fake").unwrap();
            server.transport = "streamable-http".to_string();
            server.url = Some("http://127.0.0.1:1/mcp".to_string());
            server
                .headers
                .insert("Authorization".to_string(), format!("env:{env_name}"));
        }
        let (first_revision, first_server) = server_config_snapshot(&state, "fake").await;
        let first_server = first_server.unwrap();
        std::env::set_var(&env_name, "Bearer second-token");
        let (second_revision, second_server) = server_config_snapshot(&state, "fake").await;
        let second_server = second_server.unwrap();
        assert_eq!(first_revision, second_revision);
        assert_eq!(
            first_server
                .headers
                .get(&HeaderName::from_static("authorization"))
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer first-token"
        );
        assert_eq!(
            second_server
                .headers
                .get(&HeaderName::from_static("authorization"))
                .unwrap()
                .to_str()
                .unwrap(),
            "Bearer second-token"
        );
        std::env::remove_var(&env_name);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn missing_mcp_header_secret_is_safe_in_job_error() {
        let (state, root) = managed_test_state(2).await;
        let env_name = format!("AGENTIC_MCP_JOB_MISSING_{}", uuid::Uuid::new_v4().simple());
        {
            let mut config = state.config.write().await;
            let server = config.mcp_servers.get_mut("fake").unwrap();
            server.transport = "streamable-http".to_string();
            server.url = Some("http://127.0.0.1:1/mcp".to_string());
            server
                .headers
                .insert("Authorization".to_string(), format!("env:{env_name}"));
        }
        let response = start_managed_call_with_factory(
            &state,
            managed_request(json!({}), 0),
            "local:mcp.callTool",
            None,
            fake_factory(FakeMcpServer::new(FakeBehavior::Fast)),
        )
        .await
        .unwrap();
        assert_eq!(response.status, JobState::Rejected);
        let error = response.detail.error.unwrap();
        assert_eq!(error.code, "mcp_header_secret_unavailable");
        assert_eq!(error.message, "mcp_header_secret_unavailable: fake");
        assert!(!error.message.contains(&env_name));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_header_resolution_errors_do_not_include_sources_or_values() {
        let env_name = format!("AGENTIC_MCP_MISSING_{}", uuid::Uuid::new_v4().simple());
        let mut missing = server("streamable-http", Some("https://example.test/mcp"));
        missing
            .headers
            .insert("Authorization".to_string(), format!("env:{env_name}"));
        let error = match resolve_selected_server("headers", &missing) {
            Ok(_) => panic!("missing secret unexpectedly resolved"),
            Err(error) => error.to_string(),
        };
        assert_eq!(error, "mcp_header_secret_unavailable: headers");
        assert!(!error.contains(&env_name));

        let root = std::env::temp_dir().join(format!("agentic-mcp-error-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("value");
        std::fs::write(&path, "Bearer super-token\nline\n").unwrap();
        let mut invalid = server("streamable-http", Some("https://example.test/mcp"));
        invalid.headers.insert(
            "Authorization".to_string(),
            format!("file:{}", path.display()),
        );
        let error = match resolve_selected_server("headers", &invalid) {
            Ok(_) => panic!("invalid secret unexpectedly resolved"),
            Err(error) => error.to_string(),
        };
        assert_eq!(error, "mcp_header_value_invalid: headers");
        assert!(!error.contains(path.to_string_lossy().as_ref()));
        assert!(!error.contains("super-token"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn header_validation_rejects_literals_reserved_duplicates_and_stdio() {
        let mut literal = server("streamable-http", Some("https://example.test/mcp"));
        literal
            .headers
            .insert("Authorization".to_string(), "literal-secret".to_string());
        assert!(
            validate_server_configs(&BTreeMap::from([("literal".to_string(), literal,)]))
                .unwrap_err()
                .to_string()
                .starts_with("mcp_header_reference_plaintext_rejected")
        );

        let mut insecure = server("streamable-http", Some("http://example.test/mcp"));
        insecure.headers.insert(
            "Authorization".to_string(),
            "env:AGENTIC_MCP_HEADER".to_string(),
        );
        assert!(
            validate_server_configs(&BTreeMap::from([("insecure".to_string(), insecure,)]))
                .unwrap_err()
                .to_string()
                .starts_with("mcp_header_https_required")
        );

        let mut invalid_name = server("streamable-http", Some("https://example.test/mcp"));
        invalid_name.headers.insert(
            "Bad Header".to_string(),
            "env:AGENTIC_MCP_HEADER".to_string(),
        );
        assert!(validate_server_configs(&BTreeMap::from([(
            "invalid-name".to_string(),
            invalid_name,
        )]))
        .unwrap_err()
        .to_string()
        .starts_with("mcp_header_name_invalid"));

        for name in [
            "Accept",
            "Content-Type",
            "Mcp-Session-Id",
            "Last-Event-Id",
            "MCP-Protocol-Version",
        ] {
            let mut reserved = server("streamable-http", Some("https://example.test/mcp"));
            reserved
                .headers
                .insert(name.to_string(), "env:AGENTIC_MCP_HEADER".to_string());
            assert!(validate_server_configs(&BTreeMap::from([
                ("reserved".to_string(), reserved,)
            ]))
            .unwrap_err()
            .to_string()
            .starts_with("mcp_header_reserved"));
        }

        let mut duplicate = server("streamable-http", Some("https://example.test/mcp"));
        duplicate
            .headers
            .insert("X-Trace".to_string(), "env:AGENTIC_MCP_HEADER".to_string());
        duplicate.headers.insert(
            "x-trace".to_string(),
            "env:AGENTIC_MCP_HEADER_2".to_string(),
        );
        assert!(
            validate_server_configs(&BTreeMap::from([("duplicate".to_string(), duplicate,)]))
                .unwrap_err()
                .to_string()
                .starts_with("mcp_header_duplicate")
        );

        let mut invalid_stdio = server("stdio", Some("node ./server.mjs"));
        invalid_stdio
            .headers
            .insert("X-Trace".to_string(), "env:AGENTIC_MCP_HEADER".to_string());
        assert!(
            validate_server_configs(&BTreeMap::from([("stdio".to_string(), invalid_stdio,)]))
                .unwrap_err()
                .to_string()
                .starts_with("mcp_header_transport_invalid")
        );
    }

    #[test]
    fn loopback_url_detection_accepts_ipv4_ipv6_and_localhost() {
        for endpoint in [
            "http://127.0.0.1:3000/mcp",
            "http://[::1]:3000/mcp",
            "http://localhost:3000/mcp",
        ] {
            let url = reqwest::Url::parse(endpoint).unwrap();
            assert!(url_is_loopback(&url), "endpoint={endpoint}");
        }
        let mut ipv6 = server("streamable-http", Some("http://[::1]:3000/mcp"));
        ipv6.headers.insert(
            "Authorization".to_string(),
            "env:AGENTIC_MCP_IPV6".to_string(),
        );
        assert!(validate_server_configs(&BTreeMap::from([("ipv6".to_string(), ipv6)])).is_ok());
        let external = reqwest::Url::parse("http://192.0.2.1:3000/mcp").unwrap();
        assert!(!url_is_loopback(&external));
    }

    #[tokio::test]
    async fn loopback_header_clients_bypass_system_proxy() {
        let _proxy_env_guard = lock_proxy_environment().await;
        let (url, target_requests, target_task) = spawn_http_mock().await;
        let (proxy_url, proxy_requests, proxy_task) = spawn_proxy_probe().await;
        let saved_proxy_environment = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ]
        .into_iter()
        .map(|name| (name, std::env::var_os(name)))
        .collect::<Vec<_>>();
        for (name, _) in &saved_proxy_environment {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", &proxy_url);
        let env_name = format!("AGENTIC_MCP_PROXY_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&env_name, "Bearer proxy-canary");
        let mut raw = server("streamable-http", Some(&url));
        raw.headers
            .insert("Authorization".to_string(), format!("env:{env_name}"));
        let resolved = resolve_selected_server("proxy", &raw).unwrap();
        let client = client(&resolved).await.unwrap();
        assert!(client.list_all_tools().await.unwrap().is_empty());
        close_client(client).await;
        std::env::remove_var(&env_name);
        for (name, value) in saved_proxy_environment {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        proxy_task.abort();
        target_task.abort();
        assert!(proxy_requests.lock().unwrap().is_empty());
        assert!(target_requests.lock().unwrap().iter().any(|request| {
            request.method == "POST"
                && request
                    .headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer proxy-canary")
        }));
    }

    #[test]
    fn server_config_validation_is_complete_and_typed() {
        let mut servers = BTreeMap::new();
        servers.insert(
            "http-main".to_string(),
            server("streamable-http", Some("http://127.0.0.1:3000/mcp")),
        );
        servers.insert(
            "stdio_main".to_string(),
            server("stdio", Some("node ./server.mjs")),
        );
        assert!(validate_server_configs(&servers).is_ok());

        for (id, config, code) in [
            (
                "bad id",
                server("stdio", Some("echo ok")),
                "mcp_server_id_invalid",
            ),
            (
                "missing-http",
                server("streamable-http", None),
                "mcp_server_url_missing",
            ),
            (
                "bad-http",
                server("streamable-http", Some("file:///tmp/mcp.sock")),
                "mcp_server_url_invalid",
            ),
            (
                "spaced-http",
                server("streamable-http", Some(" https://example.test/mcp")),
                "mcp_server_url_invalid",
            ),
            (
                "missing-command",
                server("stdio", Some("  ")),
                "mcp_server_command_missing",
            ),
            (
                "bad-command",
                server("stdio", Some("echo\0bad")),
                "mcp_server_command_invalid",
            ),
            (
                "spaced-command",
                server("stdio", Some(" echo ok")),
                "mcp_server_command_invalid",
            ),
            (
                "unsupported",
                server("sse", Some("https://example.test/mcp")),
                "unsupported_mcp_transport",
            ),
        ] {
            let mut candidate = BTreeMap::new();
            candidate.insert(id.to_string(), config);
            let error = validate_server_configs(&candidate).unwrap_err().to_string();
            assert!(error.starts_with(code), "error={error}");
        }
    }

    #[test]
    fn config_cli_rejects_invalid_server_without_writing_and_accepts_valid_server() {
        let root = std::env::temp_dir().join(format!(
            "agentic-mcp-config-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.json");
        let config = Config::default_config().unwrap();
        std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        let before = std::fs::read(&path).unwrap();

        let error = mutate_servers(
            path.clone(),
            McpConfigCommand::Add {
                server_id: "invalid".to_string(),
                url: "https://example.test/mcp".to_string(),
                transport: "sse".to_string(),
                enabled: true,
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.starts_with("unsupported_mcp_transport"));
        assert_eq!(std::fs::read(&path).unwrap(), before);

        mutate_servers(
            path.clone(),
            McpConfigCommand::Add {
                server_id: "valid-http".to_string(),
                url: "https://example.test/mcp".to_string(),
                transport: "streamable-http".to_string(),
                enabled: true,
            },
        )
        .unwrap();
        let written = Config::load(&path).unwrap();
        assert_eq!(
            written.mcp_servers["valid-http"].url.as_deref(),
            Some("https://example.test/mcp")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn server_config_revision_is_deterministic_and_content_sensitive() {
        let mut first = BTreeMap::new();
        first.insert(
            "b".to_string(),
            server("streamable-http", Some("https://b.example/mcp")),
        );
        first.insert("a".to_string(), server("stdio", Some("node a.mjs")));
        let mut second = BTreeMap::new();
        second.insert("a".to_string(), server("stdio", Some("node a.mjs")));
        second.insert(
            "b".to_string(),
            server("streamable-http", Some("https://b.example/mcp")),
        );
        assert_eq!(
            server_config_revision(&first),
            server_config_revision(&second)
        );
        second.get_mut("b").unwrap().enabled = false;
        assert_ne!(
            server_config_revision(&first),
            server_config_revision(&second)
        );
    }
}
