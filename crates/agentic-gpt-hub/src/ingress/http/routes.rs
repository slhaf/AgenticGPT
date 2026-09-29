use agentic_gpt_protocol::{
    HubCommand, McpBatchRequest, McpCallToolRequest, McpListServersRequest, McpListToolsRequest,
    ProcessBatchExecRequest, ProcessCancelRequest, ProcessExecRequest, ProcessListRequest,
    ProcessOutputRequest, ProcessResultRequest, ProcessStatusRequest, TmuxCapturePaneRequest,
    TmuxCloseSessionRequest, TmuxCreateSessionRequest, TmuxExecRequest, TmuxListPanesRequest,
    TmuxPasteTextRequest,
};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::agents::dispatch::{cached_process, mcp_list_servers_all_agents, request_agent};
use crate::registry::{registry_entries, registry_entry};
use crate::runs;
use crate::state::{
    projection::{
        add_cache_metadata, filter_cached_processes, live_process_value, process_list_item,
    },
    HubState,
};
use crate::utils::{constant_time_equal, random_id};
use crate::REQUEST_TIMEOUT_SECS;

#[derive(Deserialize)]
pub(crate) struct AgentIdQuery {
    #[serde(rename = "agentId")]
    agent_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessListQuery {
    agent_id: String,
    group: Option<String>,
    kind: Option<agentic_gpt_protocol::ProcessKind>,
    state: Option<agentic_gpt_protocol::ProcessState>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessStatusQuery {
    agent_id: String,
    wait_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessOutputQuery {
    agent_id: String,
    cursor: Option<String>,
    max_bytes: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessResultQuery {
    agent_id: String,
    max_bytes: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxListPanesQuery {
    agent_id: String,
    session: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxCaptureRequest {
    agent_id: String,
    target: String,
    #[serde(default = "default_tmux_capture_lines")]
    lines: u32,
}

fn default_tmux_capture_lines() -> u32 {
    160
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxExecActionRequest {
    agent_id: String,
    target: String,
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    need_confirm: bool,
    #[serde(default = "default_tmux_exec_wait_ms")]
    wait_ms: u64,
    #[serde(default = "default_tmux_exec_capture_lines")]
    capture_lines: u32,
}

fn default_tmux_exec_wait_ms() -> u64 {
    300
}

fn default_tmux_exec_capture_lines() -> u32 {
    120
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxPasteActionRequest {
    agent_id: String,
    target: String,
    text: String,
    #[serde(default)]
    submit: bool,
    #[serde(default = "default_true")]
    need_confirm: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxCreateActionRequest {
    agent_id: String,
    name: String,
    cwd: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TmuxCloseActionRequest {
    agent_id: String,
    name: String,
    #[serde(default = "default_true")]
    need_confirm: bool,
}

fn default_true() -> bool {
    true
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
fn unavailable_process_value(
    process_id: &str,
    code: &'static str,
    reason: String,
    snapshot: Option<&crate::state::ProcessCacheSnapshot>,
) -> serde_json::Value {
    let mut body = json!({
        "processId": process_id,
        "status": "unavailable",
        "error": { "code": code, "message": reason },
        "freshness": "unknown"
    });
    if let Some(snapshot) = snapshot {
        body["cached"] = json!(process_list_item(snapshot.process.clone()));
        add_cache_metadata(&mut body, std::slice::from_ref(snapshot));
    }
    body
}

fn cached_process_status_value(
    process_id: &str,
    code: &'static str,
    reason: String,
    snapshot: &crate::state::ProcessCacheSnapshot,
) -> serde_json::Value {
    let mut body = json!({
        "processId": process_id,
        "error": { "code": code, "message": reason },
        "cached": process_list_item(snapshot.process.clone())
    });
    add_cache_metadata(&mut body, std::slice::from_ref(snapshot));
    body
}

pub(crate) async fn hub_info(State(state): State<HubState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }

    match crate::state::projection::build_hub_info_response(&state).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

pub(crate) async fn list_agents(State(state): State<HubState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let entries = match registry_entries(&state) {
        Ok(entries) => entries,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    };
    let agents = state
        .agents
        .list_agents(&entries)
        .await
        .into_iter()
        .map(|entry| {
            json!({
                "agentId": entry.agent_id,
                "alias": entry.alias,
                "displayName": entry.display_name,
                "online": entry.online,
                "transport": entry.transport.map(|transport| match transport {
                    crate::state::AgentTransport::WebSocket => "websocket",
                    crate::state::AgentTransport::Sse => "sse",
                }),
                "lastSeenAt": entry.last_seen_at,
                "capabilities": entry.capabilities,
                "configSummary": entry.config_summary,
            })
        })
        .collect::<Vec<_>>();
    Json(json!({ "agents": agents })).into_response()
}

pub(crate) async fn get_run(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    match runs::get_run(&state, &run_id) {
        Ok(Some(run)) => Json(run).into_response(),
        Ok(None) => api_error(StatusCode::NOT_FOUND, "run_not_found", "Run was not found"),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

pub(crate) async fn process_exec(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<ProcessExecRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let command = HubCommand::Exec {
        request_id: random_id("req"),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "process_exec_timeout", reason),
    }
}

pub(crate) async fn process_batch(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<ProcessBatchExecRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let command = HubCommand::ProcessBatch {
        request_id: random_id("req"),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "process_batch_timeout", reason),
    }
}

pub(crate) async fn list_processes(
    State(state): State<HubState>,
    headers: HeaderMap,
    Query(query): Query<ProcessListQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let group = match agentic_gpt_protocol::normalize_process_group(query.group.as_deref()) {
        Ok(group) => group,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, error.code(), error.message());
        }
    };
    let payload = ProcessListRequest {
        group,
        kind: query.kind,
        state: query.state,
        limit: query.limit,
        cursor: query.cursor,
    };
    let command = HubCommand::ProcessList {
        request_id: random_id("req"),
        payload: payload.clone(),
    };
    match request_agent(&state, &query.agent_id, command, 2).await {
        Ok(value) => Json(live_process_value(value)).into_response(),
        Err(reason) if payload.cursor.is_some() => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "unavailable",
                "error": {
                    "code": "process_list_cursor_unavailable",
                    "message": format!(
                        "Agent is unavailable and Hub cache cannot continue an Agent-issued cursor: {reason}"
                    )
                },
                "freshness": "unknown"
            })),
        )
            .into_response(),
        Err(_) => {
            let mut snapshots = state.process_cache.snapshots(&query.agent_id).await;
            filter_cached_processes(&mut snapshots, &payload);
            let mut body = json!({
                "processes": snapshots
                    .iter()
                    .cloned()
                    .map(|snapshot| process_list_item(snapshot.process))
                    .collect::<Vec<_>>()
            });
            add_cache_metadata(&mut body, &snapshots);
            Json(body).into_response()
        }
    }
}

pub(crate) async fn get_process_status(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(process_id): Path<String>,
    Query(query): Query<ProcessStatusQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let payload = ProcessStatusRequest {
        process_id: process_id.clone(),
        wait_seconds: query.wait_seconds,
    };
    let timeout_seconds = payload.effective_wait_seconds() + 2;
    let command = HubCommand::ProcessStatus {
        request_id: random_id("req"),
        payload,
    };
    match request_agent(&state, &query.agent_id, command, timeout_seconds).await {
        Ok(value) => Json(live_process_value(value)).into_response(),
        Err(reason) => match cached_process(&state, &query.agent_id, &process_id).await {
            Some(snapshot) => Json(cached_process_status_value(
                &process_id,
                "process_status_unavailable",
                reason,
                &snapshot,
            ))
            .into_response(),
            None => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(unavailable_process_value(
                    &process_id,
                    "process_status_unavailable",
                    reason,
                    None,
                )),
            )
                .into_response(),
        },
    }
}
fn process_agent_error_status(code: &str) -> StatusCode {
    match code {
        "invalid_process_output_cursor"
        | "process_output_cursor_ahead_of_output"
        | "process_output_max_bytes_too_small_for_next_unit" => StatusCode::BAD_REQUEST,
        "process_not_found" | "process_lost_after_restart" => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn process_agent_error_response(value: &serde_json::Value) -> Option<Response> {
    if value.get("processId").is_some()
        && value.get("status").is_some()
        && value.get("resultAvailable").is_some()
    {
        return None;
    }
    let error = value.get("error")?.as_object();
    let code = error
        .and_then(|error| error.get("code"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("agent_process_error");
    let message = error
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(code);
    Some(
        (
            process_agent_error_status(code),
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response(),
    )
}

fn process_success_response(value: serde_json::Value) -> Response {
    process_agent_error_response(&value).unwrap_or_else(|| Json(value).into_response())
}

pub(crate) async fn get_process_output(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(process_id): Path<String>,
    Query(query): Query<ProcessOutputQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::ProcessOutput {
        request_id: random_id("req"),
        payload: ProcessOutputRequest {
            process_id: process_id.clone(),
            cursor: query.cursor,
            max_bytes: query.max_bytes,
        },
    };
    match request_agent(&state, &query.agent_id, command, 5).await {
        Ok(value) => process_success_response(value),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(unavailable_process_value(
                &process_id,
                "process_output_unavailable",
                reason,
                cached_process(&state, &query.agent_id, &process_id)
                    .await
                    .as_ref(),
            )),
        )
            .into_response(),
    }
}

pub(crate) async fn get_process_result(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(process_id): Path<String>,
    Query(query): Query<ProcessResultQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::ProcessResult {
        request_id: random_id("req"),
        payload: ProcessResultRequest {
            process_id: process_id.clone(),
            max_bytes: query.max_bytes,
        },
    };
    match request_agent(&state, &query.agent_id, command, 5).await {
        Ok(value) => process_success_response(value),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(unavailable_process_value(
                &process_id,
                "process_result_unavailable",
                reason,
                cached_process(&state, &query.agent_id, &process_id)
                    .await
                    .as_ref(),
            )),
        )
            .into_response(),
    }
}

pub(crate) async fn cancel_process(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(process_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::ProcessCancel {
        request_id: random_id("req"),
        payload: ProcessCancelRequest {
            process_id: process_id.clone(),
        },
    };
    match request_agent(&state, &query.agent_id, command, 5).await {
        Ok(value) => Json(live_process_value(value)).into_response(),
        Err(reason) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "processId": process_id,
                "status": "unavailable",
                "error": {
                    "code": "process_cancel_unavailable",
                    "message": reason
                },
                "freshness": "unknown"
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn tmux_list_sessions(
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
    tmux_request(
        &state,
        &query.agent_id,
        HubCommand::TmuxListSessions {
            request_id: random_id("req"),
        },
        5,
    )
    .await
}

pub(crate) async fn tmux_list_panes(
    State(state): State<HubState>,
    headers: HeaderMap,
    Query(query): Query<TmuxListPanesQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    tmux_request(
        &state,
        &query.agent_id,
        HubCommand::TmuxListPanes {
            request_id: random_id("req"),
            payload: TmuxListPanesRequest {
                session: query.session,
            },
        },
        5,
    )
    .await
}

pub(crate) async fn tmux_capture_pane(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<TmuxCaptureRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    tmux_request(
        &state,
        &payload.agent_id,
        HubCommand::TmuxCapturePane {
            request_id: random_id("req"),
            payload: TmuxCapturePaneRequest {
                target: payload.target,
                lines: payload.lines,
            },
        },
        5,
    )
    .await
}

pub(crate) async fn tmux_exec(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<TmuxExecActionRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    tmux_request(
        &state,
        &payload.agent_id,
        HubCommand::TmuxExec {
            request_id: random_id("req"),
            payload: TmuxExecRequest {
                target: payload.target,
                program: payload.program,
                args: payload.args,
                need_confirm: payload.need_confirm,
                wait_ms: payload.wait_ms,
                capture_lines: payload.capture_lines,
            },
        },
        65,
    )
    .await
}

pub(crate) async fn tmux_paste_text(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<TmuxPasteActionRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    tmux_request(
        &state,
        &payload.agent_id,
        HubCommand::TmuxPasteText {
            request_id: random_id("req"),
            payload: TmuxPasteTextRequest {
                target: payload.target,
                text: payload.text,
                submit: payload.submit,
                need_confirm: payload.need_confirm,
            },
        },
        65,
    )
    .await
}

pub(crate) async fn tmux_create_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<TmuxCreateActionRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    tmux_request(
        &state,
        &payload.agent_id,
        HubCommand::TmuxCreateSession {
            request_id: random_id("req"),
            payload: TmuxCreateSessionRequest {
                name: payload.name,
                cwd: payload.cwd,
            },
        },
        5,
    )
    .await
}

pub(crate) async fn tmux_close_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<TmuxCloseActionRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    tmux_request(
        &state,
        &payload.agent_id,
        HubCommand::TmuxCloseSession {
            request_id: random_id("req"),
            payload: TmuxCloseSessionRequest {
                name: payload.name,
                need_confirm: payload.need_confirm,
            },
        },
        65,
    )
    .await
}

async fn tmux_request(
    state: &HubState,
    agent_id: &str,
    command: HubCommand,
    timeout_seconds: u64,
) -> Response {
    match request_agent(state, agent_id, command, timeout_seconds).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "tmux_request_timeout", reason),
    }
}

pub(crate) async fn mcp_list_servers(
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

pub(crate) async fn mcp_list_tools(
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

pub(crate) async fn mcp_call_tool(
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
    let request_timeout = payload.effective_wait_seconds() + 2;
    match request_agent(&state, &payload.agent_id, command, request_timeout).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "mcp_call_tool_timeout", reason),
    }
}

pub(crate) async fn mcp_batch(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<McpBatchRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }
    let command = HubCommand::McpBatch {
        request_id: random_id("req"),
        payload: payload.clone(),
    };
    let request_timeout = payload.effective_wait_seconds() + 2;
    match request_agent(&state, &payload.agent_id, command, request_timeout).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "mcp_batch_timeout", reason),
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn require_action_auth(
    state: &HubState,
    headers: &HeaderMap,
) -> std::result::Result<(), Response> {
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

pub(crate) fn parse_bearer_token(value: &str) -> Option<String> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let scheme = parts.next()?;
    let token = parts.next()?.trim();
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token.to_string())
    } else {
        None
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn require_agent_enabled(
    state: &HubState,
    agent_id: &str,
) -> std::result::Result<(), Response> {
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

pub(crate) fn api_error(
    status: StatusCode,
    code: &'static str,
    message: impl ToString,
) -> Response {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_status_wait_is_bounded_by_protocol_defaults() {
        for (wait_seconds, expected) in [(None, 5), (Some(0), 0), (Some(30), 30), (Some(31), 30)] {
            let query = ProcessStatusQuery {
                agent_id: "agent".to_string(),
                wait_seconds,
            };
            let payload = ProcessStatusRequest {
                process_id: "process".to_string(),
                wait_seconds: query.wait_seconds,
            };
            assert_eq!(payload.effective_wait_seconds(), expected);
        }
    }

    #[tokio::test]
    async fn process_agent_errors_become_declared_http_errors() {
        for (code, expected_status) in [
            ("invalid_process_output_cursor", StatusCode::BAD_REQUEST),
            (
                "process_output_cursor_ahead_of_output",
                StatusCode::BAD_REQUEST,
            ),
            ("process_not_found", StatusCode::NOT_FOUND),
            ("process_lost_after_restart", StatusCode::NOT_FOUND),
            (
                "process_output_snapshot_invalid",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            let response = process_success_response(json!({
                "error": { "code": code, "message": "Agent rejected the request" }
            }));
            assert_eq!(response.status(), expected_status, "{code}");
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["error"]["code"], code);
            assert_eq!(body["error"]["message"], "Agent rejected the request");
        }

        let valid_unavailable_result = json!({
            "processId": "process-1",
            "status": "unavailable",
            "resultAvailable": false,
            "error": { "code": "process_result_not_ready", "message": "Process has not completed" }
        });
        let response = process_success_response(valid_unavailable_result.clone());
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            valid_unavailable_result
        );
    }

    #[test]
    fn unavailable_process_detail_responses_expose_metadata_without_payloads() {
        let now = chrono::Utc::now();
        let process: agentic_gpt_protocol::ProcessInfo =
            serde_json::from_value(serde_json::json!({
                "agentId": "agent",
                "processId": "process-1",
                "kind": "command",
                "state": "completed",
                "createdAt": now.to_rfc3339(),
                "updatedAt": now.to_rfc3339(),
                "captureStatus": "complete"
            }))
            .unwrap();
        let snapshot = crate::state::ProcessCacheSnapshot {
            process,
            observed_at: now,
            freshness: crate::state::ProcessFreshness::Stale,
        };

        for code in ["process_output_unavailable", "process_result_unavailable"] {
            let value = unavailable_process_value(
                "process-1",
                code,
                "Agent is unavailable".to_string(),
                Some(&snapshot),
            );
            assert_eq!(value["processId"], "process-1");
            assert_eq!(value["status"], "unavailable");
            assert_eq!(value["error"]["code"], code);
            assert_eq!(value["cached"]["processId"], "process-1");
            assert_eq!(value["freshness"], "stale");
            assert!(value["observedAt"].is_string());
            for field in ["stdout", "stderr", "result"] {
                assert!(value.get(field).is_none());
                assert!(value["cached"].get(field).is_none());
            }
        }
    }
}
