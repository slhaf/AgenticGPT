use agentic_gpt_protocol::{
    EventGetRequest, EventListRequest, EventMarkRequest, EventSeverity, EventStatus, HubCommand,
    McpBatchRequest, McpCallToolRequest, McpListServersRequest, McpListToolsRequest,
    ProcessBatchExecRequest, ProcessCancelRequest, ProcessExecRequest, ProcessListRequest,
    ProcessReadRequest, ProcessReadView, TmuxCapturePaneRequest, TmuxCloseSessionRequest,
    TmuxCreateSessionRequest, TmuxExecRequest, TmuxListPanesRequest, TmuxPasteTextRequest,
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

pub(crate) fn process_routes() -> axum::Router<HubState> {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/v1/process/exec", post(process_exec))
        .route("/v1/process/batch", post(process_batch))
        .route("/v1/process", get(list_processes))
        .route("/v1/process/:process_id/read", get(get_process_read))
        .route("/v1/process/:process_id/cancel", post(cancel_process))
}

#[derive(Deserialize)]
pub(crate) struct AgentIdQuery {
    #[serde(rename = "agentId")]
    agent_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventListQuery {
    agent_id: String,
    status: Option<EventStatus>,
    severity: Option<EventSeverity>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventMarkHttpRequest {
    agent_id: String,
    event_ids: Vec<String>,
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
pub(crate) struct ProcessReadQuery {
    agent_id: String,
    wait_seconds: Option<u64>,
    view: Option<ProcessReadView>,
    cursor: Option<String>,
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

pub(crate) async fn list_events(
    State(state): State<HubState>,
    headers: HeaderMap,
    Query(query): Query<EventListQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }

    let command = HubCommand::EventList {
        request_id: random_id("req"),
        payload: EventListRequest {
            agent_id: query.agent_id.clone(),
            status: query.status,
            severity: query.severity,
            limit: query.limit,
            cursor: query.cursor,
        },
    };
    match request_agent(&state, &query.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => process_success_response(value),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "event_list_timeout", reason),
    }
}

pub(crate) async fn get_event(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(event_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }

    let command = HubCommand::EventGet {
        request_id: random_id("req"),
        payload: EventGetRequest {
            agent_id: query.agent_id.clone(),
            event_id,
        },
    };
    match request_agent(&state, &query.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => process_success_response(value),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "event_get_timeout", reason),
    }
}

pub(crate) async fn mark_events(
    State(state): State<HubState>,
    headers: HeaderMap,
    Json(payload): Json<EventMarkHttpRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &payload.agent_id) {
        return response;
    }

    let command = HubCommand::EventMark {
        request_id: random_id("req"),
        payload: EventMarkRequest {
            agent_id: payload.agent_id.clone(),
            event_ids: payload.event_ids,
        },
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => process_success_response(value),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "event_mark_timeout", reason),
    }
}

pub(crate) async fn get_process_read(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(process_id): Path<String>,
    Query(query): Query<ProcessReadQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let payload = ProcessReadRequest {
        process_id: process_id.clone(),
        wait_seconds: query.wait_seconds,
        view: query.view.unwrap_or_default(),
        cursor: query.cursor,
        max_bytes: query.max_bytes,
    };
    let timeout_seconds = payload.effective_wait_seconds() + 2;
    let command = HubCommand::ProcessRead {
        request_id: random_id("req"),
        payload,
    };
    match request_agent(&state, &query.agent_id, command, timeout_seconds).await {
        Ok(value) => process_success_response(value),
        Err(reason) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(unavailable_process_value(
                &process_id,
                "process_read_unavailable",
                reason,
                cached_process(&state, &query.agent_id, &process_id)
                    .await
                    .as_ref(),
            )),
        )
            .into_response(),
    }
}

fn process_agent_error_status(code: &str) -> StatusCode {
    match code {
        "invalid_process_output_cursor"
        | "process_output_cursor_ahead_of_output"
        | "process_output_max_bytes_too_small_for_next_unit"
        | "process_read_cursor_with_status_view"
        | "process_read_cursor_not_supported_for_mcp"
        | "process_read_max_bytes_out_of_range"
        | "process_response_budget_too_small"
        | "event_cursor_invalid"
        | "event_cursor_scope_mismatch"
        | "event_mark_too_many_ids"
        | "event_id_invalid"
        | "event_agent_mismatch"
        | "event_store_agent_scope_invalid" => StatusCode::BAD_REQUEST,
        "process_not_found" | "process_lost_after_restart" | "event_not_found" => {
            StatusCode::NOT_FOUND
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn process_agent_error_response(value: &mut serde_json::Value) -> Option<StatusCode> {
    if value.get("agentId").is_some()
        && value.get("processId").is_some()
        && value.get("kind").is_some()
        && value.get("state").is_some()
        && value.get("captureStatus").is_some()
    {
        return None;
    }
    let error_value = value.get_mut("error")?;
    if !error_value.is_object() {
        *error_value = json!({});
    }
    let error = error_value.as_object_mut()?;
    let code = error
        .get("code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("agent_process_error")
        .to_string();
    let message = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&code)
        .to_string();
    let status = process_agent_error_status(&code);
    error.insert("code".to_string(), serde_json::Value::String(code));
    error.insert("message".to_string(), serde_json::Value::String(message));
    Some(status)
}

fn process_success_response(mut value: serde_json::Value) -> Response {
    match process_agent_error_response(&mut value) {
        Some(status) => (status, Json(value)).into_response(),
        None => Json(value).into_response(),
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
        Ok(value) => Json(value).into_response(),
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
            suppress_event_panel: false,
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
    use rusqlite::params;
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::mpsc;

    fn http_test_state() -> HubState {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::init_db(&conn).unwrap();
        crate::event_feedback::init(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(crate::HubConfig {
                remote_confirmation: crate::config::RemoteConfirmationConfig {
                    enabled: false,
                    provider: "none".to_string(),
                    timeout_seconds: 45,
                    ntfy: crate::NtfyConfig {
                        server_url: String::new(),
                        topic: String::new(),
                        callback_base_url: String::new(),
                    },
                },
            }),
            mcp_profile: crate::state::McpProfile::Full,
            agents: Arc::new(crate::agents::lifecycle::Connections::new()),
            dispatch: Arc::new(crate::agents::dispatch::Dispatch::new()),
            confirmations: Arc::new(crate::confirmation::Confirmations::new()),
            process_cache: Arc::new(crate::state::ProcessCache::new()),
            boot_generations: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            active_room: Arc::new(tokio::sync::Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: None,
            oauth_codes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    fn register_http_agent(state: &HubState) {
        let capabilities = agentic_gpt_protocol::Capabilities {
            processes: true,
            confirmation: true,
            notification_actions: true,
        };
        state
            .db
            .lock()
            .unwrap()
            .execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values (?1, null, ?1, 1, ?2, null, ?3)",
                params![
                    "agent",
                    crate::utils::sha256_hex("agent-secret"),
                    serde_json::to_string(&capabilities).unwrap()
                ],
            )
            .unwrap();
    }

    fn http_action_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer test-api-key"),
        );
        headers
    }

    fn http_agent_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-agent-secret",
            axum::http::HeaderValue::from_static("agent-secret"),
        );
        headers
    }

    fn event_list_query(agent_id: &str, cursor: Option<&str>) -> EventListQuery {
        EventListQuery {
            agent_id: agent_id.to_string(),
            status: None,
            severity: None,
            limit: None,
            cursor: cursor.map(str::to_string),
        }
    }

    async fn insert_http_agent_connection(
        state: &HubState,
    ) -> mpsc::UnboundedReceiver<crate::state::OutboundAgentMessage> {
        let (sender, receiver) = mpsc::unbounded_channel();
        state
            .agents
            .insert_for_test(
                "agent",
                crate::state::AgentConnection {
                    connection_id: "current".to_string(),
                    sender,
                    last_seen_at: chrono::Utc::now(),
                    role: agentic_gpt_protocol::AgentRole::Normal,
                    connection_mode: agentic_gpt_protocol::AgentConnectionMode::CommandCapable,
                    hello_received: true,
                    boot_generation: Some("testboot".to_string()),
                    transport: crate::state::AgentTransport::Sse,
                    config_summary: None,
                    notification_channels: Vec::new(),
                },
            )
            .await;
        receiver
    }

    async fn respond_with_agent_error<F>(
        state: HubState,
        outbound: &mut mpsc::UnboundedReceiver<crate::state::OutboundAgentMessage>,
        request: F,
        data: serde_json::Value,
    ) -> Response
    where
        F: Future<Output = Response> + Send + 'static,
    {
        let task = tokio::spawn(request);
        let message = outbound.recv().await.expect("Hub sends an Agent command");
        let crate::state::OutboundAgentMessage::Text(text) = message else {
            panic!("expected a text command envelope");
        };
        let envelope: agentic_gpt_protocol::HubCommandEnvelope =
            serde_json::from_str(&text).unwrap();
        let transport_response = crate::agents::transport::post_agent_message(
            State(state.clone()),
            Path("agent".to_string()),
            Query(crate::agents::transport::SseConnectQuery::for_test(Some(
                "current".to_string(),
            ))),
            http_agent_headers(),
            Json(agentic_gpt_protocol::AgentMessage::Response {
                run_id: Some(envelope.run_id),
                request_id: envelope.request_id,
                data,
                event_sources: Vec::new(),
            }),
        )
        .await;
        assert_eq!(transport_response.status(), StatusCode::OK);
        task.await.unwrap()
    }

    async fn response_json(response: Response) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn event_http_routes_require_authorization_and_an_enabled_agent() {
        let state = http_test_state();

        for response in [
            list_events(
                State(state.clone()),
                HeaderMap::new(),
                Query(event_list_query("agent", None)),
            )
            .await,
            get_event(
                State(state.clone()),
                HeaderMap::new(),
                Path("event-1".to_string()),
                Query(AgentIdQuery {
                    agent_id: "agent".to_string(),
                }),
            )
            .await,
            mark_events(
                State(state.clone()),
                HeaderMap::new(),
                Json(EventMarkHttpRequest {
                    agent_id: "agent".to_string(),
                    event_ids: vec!["event-1".to_string()],
                }),
            )
            .await,
        ] {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        for response in [
            list_events(
                State(state.clone()),
                http_action_headers(),
                Query(event_list_query("agent", None)),
            )
            .await,
            get_event(
                State(state.clone()),
                http_action_headers(),
                Path("event-1".to_string()),
                Query(AgentIdQuery {
                    agent_id: "agent".to_string(),
                }),
            )
            .await,
            mark_events(
                State(state.clone()),
                http_action_headers(),
                Json(EventMarkHttpRequest {
                    agent_id: "agent".to_string(),
                    event_ids: vec!["event-1".to_string()],
                }),
            )
            .await,
        ] {
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        assert_eq!(state.dispatch.pending_count().await, 0);
        register_http_agent(&state);
        state
            .db
            .lock()
            .unwrap()
            .execute(
                "update agents set enabled = 0 where agent_id = ?1",
                params!["agent"],
            )
            .unwrap();
        for response in [
            list_events(
                State(state.clone()),
                http_action_headers(),
                Query(event_list_query("agent", None)),
            )
            .await,
            get_event(
                State(state.clone()),
                http_action_headers(),
                Path("event-1".to_string()),
                Query(AgentIdQuery {
                    agent_id: "agent".to_string(),
                }),
            )
            .await,
            mark_events(
                State(state.clone()),
                http_action_headers(),
                Json(EventMarkHttpRequest {
                    agent_id: "agent".to_string(),
                    event_ids: vec!["event-1".to_string()],
                }),
            )
            .await,
        ] {
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        assert_eq!(state.dispatch.pending_count().await, 0);
    }

    #[tokio::test]
    async fn online_event_domain_errors_keep_the_agent_panel_and_http_status() {
        let state = http_test_state();
        register_http_agent(&state);
        let mut outbound = insert_http_agent_connection(&state).await;
        let panel = json!({
            "current": "low: 1 | medium: 0 | high: 0",
            "new": [{
                "event-1 | Process completed": "low | 2026-10-01T12:00:00Z"
            }]
        });

        let request_state = state.clone();
        let response = respond_with_agent_error(
            state.clone(),
            &mut outbound,
            async move {
                get_event(
                    State(request_state),
                    http_action_headers(),
                    Path("missing-event".to_string()),
                    Query(AgentIdQuery {
                        agent_id: "agent".to_string(),
                    }),
                )
                .await
            },
            json!({
                "error": { "code": "event_not_found", "message": "event_not_found" },
                "events": panel
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "event_not_found");
        assert_eq!(body["events"]["current"], "low: 1 | medium: 0 | high: 0");
        assert_eq!(
            body["events"]["new"][0]["event-1 | Process completed"],
            "low | 2026-10-01T12:00:00Z"
        );

        let panel = json!({
            "current": "low: 1 | medium: 0 | high: 0",
            "new": [{
                "event-1 | Process completed": "low | 2026-10-01T12:00:00Z"
            }]
        });
        let request_state = state.clone();
        let response = respond_with_agent_error(
            state.clone(),
            &mut outbound,
            async move {
                list_events(
                    State(request_state),
                    http_action_headers(),
                    Query(event_list_query("agent", Some("not-a-valid-cursor"))),
                )
                .await
            },
            json!({
                "error": { "code": "event_cursor_invalid", "message": "event_cursor_invalid" },
                "events": panel
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "event_cursor_invalid");
        assert_eq!(body["events"]["current"], "low: 1 | medium: 0 | high: 0");
        let panel = json!({
            "current": "low: 1 | medium: 0 | high: 0",
            "new": [{
                "event-1 | Process completed": "low | 2026-10-01T12:00:00Z"
            }]
        });
        let event_ids = (0..513).map(|index| format!("event-{index}")).collect();
        let request_state = state.clone();
        let response = respond_with_agent_error(
            state.clone(),
            &mut outbound,
            async move {
                mark_events(
                    State(request_state),
                    http_action_headers(),
                    Json(EventMarkHttpRequest {
                        agent_id: "agent".to_string(),
                        event_ids,
                    }),
                )
                .await
            },
            json!({
                "error": { "code": "event_mark_too_many_ids", "message": "event_mark_too_many_ids" },
                "events": panel
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "event_mark_too_many_ids");
        assert_eq!(body["events"]["current"], "low: 1 | medium: 0 | high: 0");
    }

    #[tokio::test]
    async fn process_read_resolves_reliable_response_and_preserves_contract_shape() {
        let state = http_test_state();
        register_http_agent(&state);
        let mut outbound = insert_http_agent_connection(&state).await;
        let response_data = json!({
            "agentId": "agent",
            "processId": "process-1",
            "kind": "command",
            "state": "running",
            "captureStatus": "capturing",
            "output": {
                "stdout": {
                    "data": "ok",
                    "startOffset": "0",
                    "endOffset": "2",
                    "encoding": "utf8"
                },
                "stderr": {
                    "data": "",
                    "startOffset": "0",
                    "endOffset": "0",
                    "encoding": "utf8"
                },
                "nextCursor": "next-page",
                "hasMore": true,
                "eof": false
            }
        });
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            get_process_read(
                State(request_state),
                http_action_headers(),
                Path("process-1".to_string()),
                Query(ProcessReadQuery {
                    agent_id: "agent".to_string(),
                    wait_seconds: Some(3),
                    view: Some(ProcessReadView::Auto),
                    cursor: Some("prior-page".to_string()),
                    max_bytes: Some(4096),
                }),
            )
            .await
        });
        let message = outbound
            .recv()
            .await
            .expect("Hub sends a ProcessRead command");
        let crate::state::OutboundAgentMessage::Text(text) = message else {
            panic!("expected a reliable text command envelope");
        };
        let envelope: agentic_gpt_protocol::HubCommandEnvelope =
            serde_json::from_str(&text).unwrap();
        assert!(!envelope.event_id.is_empty());
        assert!(!envelope.run_id.is_empty());
        assert!(!envelope.command_hash.is_empty());
        let expected_request_id = envelope.request_id.clone();
        let run_id = envelope.run_id.clone();
        let payload = match envelope.command {
            HubCommand::ProcessRead {
                request_id,
                payload,
            } => {
                assert_eq!(request_id, expected_request_id);
                payload
            }
            command => panic!("expected ProcessRead, received {command:?}"),
        };
        assert_eq!(payload.process_id, "process-1");
        assert_eq!(payload.wait_seconds, Some(3));
        assert_eq!(payload.view, ProcessReadView::Auto);
        assert_eq!(payload.cursor.as_deref(), Some("prior-page"));
        assert_eq!(payload.max_bytes, Some(4096));

        let transport_response = crate::agents::transport::post_agent_message(
            State(state),
            Path("agent".to_string()),
            Query(crate::agents::transport::SseConnectQuery::for_test(Some(
                "current".to_string(),
            ))),
            http_agent_headers(),
            Json(agentic_gpt_protocol::AgentMessage::Response {
                run_id: Some(run_id),
                request_id: expected_request_id,
                data: response_data.clone(),
                event_sources: Vec::new(),
            }),
        )
        .await;
        assert_eq!(transport_response.status(), StatusCode::OK);
        let response = task.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body, response_data);
        assert!(body.get("freshness").is_none());
        assert!(body.get("observedAt").is_none());
    }

    #[tokio::test]
    async fn process_http_routes_reject_removed_read_paths() {
        let app = process_routes().with_state(http_test_state());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::new();

        let read = client
            .get(format!(
                "http://{address}/v1/process/process-1/read?agentId=agent"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::UNAUTHORIZED);
        for path in [
            "/v1/process/process-1",
            "/v1/process/process-1/output",
            "/v1/process/process-1/result",
        ] {
            let response = client
                .get(format!("http://{address}{path}?agentId=agent"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        server.abort();
    }
}
