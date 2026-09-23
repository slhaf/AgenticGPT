use agentic_gpt_protocol::{
    normalize_job_group, BatchExecRequest, ExecRequest, HubCommand, JobCancelRequest,
    JobGetRequest, JobKind, JobListRequest, JobState, McpBatchRequest, McpCallToolRequest,
    McpListServersRequest, McpListToolsRequest, TmuxCapturePaneRequest, TmuxCloseSessionRequest,
    TmuxCreateSessionRequest, TmuxExecRequest, TmuxListPanesRequest, TmuxPasteTextRequest,
};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::agents::dispatch::{cached_job, mcp_list_servers_all_agents, request_agent};
use crate::registry::{registry_entries, registry_entry};
use crate::runs;
use crate::state::{
    projection::{add_cache_metadata, filter_cached_jobs, job_list_item, live_job_value},
    HubState,
};
use crate::utils::{constant_time_equal, random_id};
use crate::{MAX_WAIT_SECONDS, REQUEST_TIMEOUT_SECS};

#[derive(Deserialize)]
pub(crate) struct AgentIdQuery {
    #[serde(rename = "agentId")]
    agent_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JobListQuery {
    agent_id: String,
    group: Option<String>,
    kind: Option<JobKind>,
    state: Option<JobState>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JobGetQuery {
    agent_id: String,
    wait_seconds: Option<u64>,
    #[serde(default)]
    wait_only: bool,
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
    Json(payload): Json<ExecRequest>,
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
    Json(payload): Json<BatchExecRequest>,
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

pub(crate) async fn list_jobs(
    State(state): State<HubState>,
    headers: HeaderMap,
    Query(query): Query<JobListQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let group = match normalize_job_group(query.group.as_deref()) {
        Ok(group) => group,
        Err(error) => {
            return api_error(StatusCode::BAD_REQUEST, error.code(), error.message());
        }
    };
    let payload = JobListRequest {
        group,
        kind: query.kind,
        state: query.state,
        limit: query.limit,
        cursor: query.cursor,
    };
    let command = HubCommand::JobList {
        request_id: random_id("req"),
        payload: payload.clone(),
    };
    match request_agent(&state, &query.agent_id, command, 2).await {
        Ok(value) => Json(live_job_value(value)).into_response(),
        Err(reason) if payload.cursor.is_some() => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": {
                    "code": "job_list_cursor_unavailable",
                    "message": format!(
                        "Agent is unavailable and Hub cache cannot continue an Agent-issued cursor: {reason}"
                    )
                },
                "freshness": "unknown"
            })),
        )
            .into_response(),
        Err(_) => {
            let mut snapshots = state.job_cache.snapshots(&query.agent_id).await;
            filter_cached_jobs(&mut snapshots, &payload);
            let mut body = json!({
                "jobs": snapshots
                    .iter()
                    .cloned()
                    .map(|snapshot| job_list_item(snapshot.job))
                    .collect::<Vec<_>>()
            });
            add_cache_metadata(&mut body, &snapshots);
            Json(body).into_response()
        }
    }
}

pub(crate) async fn get_job(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Query(query): Query<JobGetQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::JobGet {
        request_id: random_id("req"),
        payload: JobGetRequest {
            job_id: job_id.clone(),
            wait_only: query.wait_only,
            wait_seconds: query
                .wait_seconds
                .map(|seconds| seconds.min(MAX_WAIT_SECONDS)),
        },
    };
    let timeout_seconds = query.wait_seconds.unwrap_or(0).min(MAX_WAIT_SECONDS) + 2;
    match request_agent(&state, &query.agent_id, command, timeout_seconds).await {
        Ok(value) => Json(live_job_value(value)).into_response(),
        Err(reason) => match cached_job(&state, &query.agent_id, &job_id).await {
            Some(snapshot) => {
                let mut body = json!({
                    "error": {
                        "code": "job_get_unavailable",
                        "message": reason
                    },
                    "cached": job_list_item(snapshot.job.clone())
                });
                add_cache_metadata(&mut body, std::slice::from_ref(&snapshot));
                Json(body).into_response()
            }
            None => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": {
                        "code": "job_get_unavailable",
                        "message": reason
                    },
                    "freshness": "unknown"
                })),
            )
                .into_response(),
        },
    }
}

pub(crate) async fn cancel_job(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::JobCancel {
        request_id: random_id("req"),
        payload: JobCancelRequest {
            job_id: job_id.clone(),
        },
    };
    match request_agent(&state, &query.agent_id, command, 5).await {
        Ok(value) => Json(live_job_value(value)).into_response(),
        Err(reason) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "code": "job_cancel_unavailable",
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
