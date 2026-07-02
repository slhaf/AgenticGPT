use agentic_gpt_protocol::{
    BatchExecRequest, ExecRequest, HubCommand, HubInfoAgents, HubInfoCounts,
    HubInfoRemoteConfirmation, HubInfoResponse, McpCallToolRequest, McpListServersRequest,
    McpListToolsRequest, SessionInfo, TmuxCapturePaneRequest, TmuxCloseSessionRequest,
    TmuxCreateSessionRequest, TmuxExecRequest, TmuxListPanesRequest, TmuxPasteTextRequest,
};
use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use crate::agents::{
    cached_session, mcp_list_servers_all_agents, request_agent, timeout_batch_result,
    timeout_task_result,
};
use crate::registry::{registry_entries, registry_entry};
use crate::runs;
use crate::state::HubState;
use crate::utils::{constant_time_equal, random_id};
use crate::{default_config_summary, notify, MAX_WAIT_SECONDS, REQUEST_TIMEOUT_SECS};

#[derive(Deserialize)]
pub(crate) struct AgentIdQuery {
    #[serde(rename = "agentId")]
    agent_id: String,
}

#[derive(Deserialize)]
pub(crate) struct WaitRequest {
    seconds: Option<u64>,
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

    match build_hub_info_response(&state).await {
        Ok(response) => Json(response).into_response(),
        Err(error) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    }
}

pub(crate) async fn build_hub_info_response(state: &HubState) -> Result<HubInfoResponse> {
    let entries = registry_entries(state)?;
    let registered_count = entries.len();
    let enabled_count = entries.iter().filter(|entry| entry.enabled).count();
    let online_count = state.agents.lock().await.len();
    let pending_request_count = state.pending.lock().await.len();
    let pending_confirmation_count = state
        .pending_confirmations
        .lock()
        .await
        .values()
        .filter(|confirmation| !confirmation.resolved)
        .count();
    let cached_session_count = state.sessions.lock().await.values().map(HashMap::len).sum();

    let remote = &state.config.remote_confirmation;
    let ntfy = &remote.ntfy;
    Ok(HubInfoResponse {
        service: "agentic-gpt-hub".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        public_base_url: state.public_base_url.clone(),
        request_timeout_seconds: REQUEST_TIMEOUT_SECS,
        max_wait_seconds: MAX_WAIT_SECONDS,
        remote_confirmation: HubInfoRemoteConfirmation {
            enabled: remote.enabled,
            provider: remote.provider.clone(),
            timeout_seconds: remote.timeout_seconds,
            ntfy_configured: !notify::ntfy_not_configured(ntfy)
                && !ntfy.callback_base_url.trim().is_empty(),
        },
        agents: HubInfoAgents {
            registered_count,
            enabled_count,
            online_count,
        },
        counts: HubInfoCounts {
            pending_request_count,
            pending_confirmation_count,
            cached_session_count,
        },
        generated_at: Utc::now(),
    })
}

pub(crate) async fn list_agents(State(state): State<HubState>, headers: HeaderMap) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    let entries = match registry_entries(&state) {
        Ok(entries) => entries,
        Err(error) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "db_error", error),
    };
    let online = state.agents.lock().await;
    let agents = entries
        .into_iter()
        .filter(|entry| entry.enabled)
        .map(|entry| {
            let status = online.get(&entry.agent_id);
            json!({
                "agentId": entry.agent_id,
                "alias": entry.alias,
                "displayName": entry.display_name,
                "online": status.is_some(),
                "transport": status.map(|s| match s.transport {
                    crate::state::AgentTransport::WebSocket => "websocket",
                    crate::state::AgentTransport::Sse => "sse",
                }),
                "lastSeenAt": status.map(|s| s.last_seen_at).or(entry.last_seen_at),
                "capabilities": entry.capabilities,
                "configSummary": status.and_then(|s| s.config_summary.clone()).unwrap_or_else(default_config_summary)
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

pub(crate) async fn exec(
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
    let task_id = random_id("task");
    let command = HubCommand::Exec {
        request_id: random_id("req"),
        task_id: task_id.clone(),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => {
            Json(timeout_task_result(&payload.agent_id, &task_id, reason)).into_response()
        }
    }
}

pub(crate) async fn batch_exec(
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
    let batch_id = random_id("batch");
    let command = HubCommand::BatchExec {
        request_id: random_id("req"),
        task_id: batch_id.clone(),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => Json(timeout_batch_result(&payload, &batch_id, reason)).into_response(),
    }
}

pub(crate) async fn start_session(
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
    let session_id = random_id("sess");
    let command = HubCommand::StartSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
        payload: payload.clone(),
    };
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => {
            let status = serde_json::from_value::<SessionInfo>(value.clone())
                .ok()
                .map(|session| {
                    if session.state == "running" || session.state == "waiting_confirmation" {
                        "started"
                    } else {
                        "failed"
                    }
                })
                .unwrap_or("started");
            Json(json!({ "status": status, "sessionId": session_id, "session": value }))
                .into_response()
        }
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "session_start_timeout", reason),
    }
}

pub(crate) async fn list_sessions(
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
    let command = HubCommand::ListSessions {
        request_id: random_id("req"),
    };
    match request_agent(&state, &query.agent_id, command, 2).await {
        Ok(value) => Json(json!({ "sessions": value })).into_response(),
        Err(_) => {
            let sessions = state
                .sessions
                .lock()
                .await
                .get(&query.agent_id)
                .map(|sessions| sessions.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            Json(json!({ "sessions": sessions })).into_response()
        }
    }
}

pub(crate) async fn inspect_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::InspectSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
    };
    match request_agent(&state, &query.agent_id, command, 2).await {
        Ok(value) if !value.is_null() => Json(value).into_response(),
        _ => match cached_session(&state, &query.agent_id, &session_id).await {
            Some(session) => Json(session).into_response(),
            None => api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session was not found",
            ),
        },
    }
}

pub(crate) async fn wait_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
    Json(body): Json<WaitRequest>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let seconds = body.seconds.unwrap_or(0).min(MAX_WAIT_SECONDS);
    let command = HubCommand::WaitSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
        seconds,
    };
    match request_agent(&state, &query.agent_id, command, seconds + 2).await {
        Ok(value) if !value.is_null() => Json(value).into_response(),
        _ => match cached_session(&state, &query.agent_id, &session_id).await {
            Some(session) => Json(session).into_response(),
            None => api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session was not found",
            ),
        },
    }
}

pub(crate) async fn kill_session(
    State(state): State<HubState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<AgentIdQuery>,
) -> Response {
    if let Err(response) = require_action_auth(&state, &headers) {
        return response;
    }
    if let Err(response) = require_agent_enabled(&state, &query.agent_id) {
        return response;
    }
    let command = HubCommand::KillSession {
        request_id: random_id("req"),
        session_id: session_id.clone(),
    };
    match request_agent(&state, &query.agent_id, command, 5).await {
        Ok(value) if !value.is_null() => Json(value).into_response(),
        _ => match cached_session(&state, &query.agent_id, &session_id).await {
            Some(session) => Json(session).into_response(),
            None => api_error(
                StatusCode::NOT_FOUND,
                "session_not_found",
                "Session was not found",
            ),
        },
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
    match request_agent(&state, &payload.agent_id, command, REQUEST_TIMEOUT_SECS).await {
        Ok(value) => Json(value).into_response(),
        Err(reason) => api_error(StatusCode::GATEWAY_TIMEOUT, "mcp_call_tool_timeout", reason),
    }
}

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
