use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rmcp::handler::server::wrapper::Parameters;
use serde::Deserialize;
use serde_json::{json, Value};

use super::AgenticMcpServer;
use crate::state::HubState;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

pub(crate) async fn mcp_get(State(state): State<HubState>) -> Response {
    let server = AgenticMcpServer::new(state);
    Json(json!({
        "name": "agentic-gpt-hub",
        "profile": server.profile.label(),
        "tools": app_tool_descriptors(&server)
    }))
    .into_response()
}

pub(crate) async fn mcp_post(State(state): State<HubState>, Json(rpc): Json<Value>) -> Response {
    let request = match serde_json::from_value::<JsonRpcRequest>(rpc) {
        Ok(request) => request,
        Err(error) => return rpc_error(None, -32700, format!("Invalid JSON-RPC request: {error}")),
    };
    let id = request.id.clone();
    let server = AgenticMcpServer::new(state);
    match request.method.as_str() {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": negotiated_protocol_version(request.params.as_ref()),
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "agentic-gpt-hub",
                    "title": "Agentic GPT Hub",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": server.instructions(),
                "profile": server.profile.label()
            }),
        ),
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({ "tools": app_tool_descriptors(&server) })),
        "tools/call" => {
            match call_app_tool(&server, request.params.unwrap_or_else(|| json!({}))).await {
                Ok(result) => rpc_result(id, result),
                Err(error) => rpc_error(id, -32602, error),
            }
        }
        _ => rpc_error(id, -32601, "Method not found"),
    }
}

pub(super) async fn call_app_tool(
    server: &AgenticMcpServer,
    params: Value,
) -> Result<Value, String> {
    let object = params
        .as_object()
        .ok_or_else(|| "tools/call params must be an object".to_string())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "tools/call params.name is required".to_string())?;
    if !server.allows_tool(name) {
        if !server.profile_allows_tool(name) {
            return Err(format!("tool_unavailable_for_profile: {name}"));
        }
        return Err(format!("Unknown tool: {name}"));
    }
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match name {
        "agent.list" => server.list_agents().await,
        "process.exec" => server.exec(Parameters(decode_args(arguments)?)).await,
        "process.batch" => server.batch_exec(Parameters(decode_args(arguments)?)).await,
        "job.list" => server.job_list(Parameters(decode_args(arguments)?)).await,
        "job.get" => server.job_get(Parameters(decode_args(arguments)?)).await,
        "job.cancel" => server.job_cancel(Parameters(decode_args(arguments)?)).await,
        "tmux.listSessions" => {
            server
                .tmux_list_sessions(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.listPanes" => {
            server
                .tmux_list_panes(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.capturePane" => {
            server
                .tmux_capture_pane(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.pasteText" => {
            server
                .tmux_paste_text(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.exec" => server.tmux_exec(Parameters(decode_args(arguments)?)).await,
        "tmux.createSession" => {
            server
                .tmux_create_session(Parameters(decode_args(arguments)?))
                .await
        }
        "tmux.closeSession" => {
            server
                .tmux_close_session(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.listServers" => {
            server
                .mcp_list_servers(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.listTools" => {
            server
                .mcp_list_tools(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.callTool" => {
            server
                .mcp_call_tool(Parameters(decode_args(arguments)?))
                .await
        }
        "mcp.batch" => server.mcp_batch(Parameters(decode_args(arguments)?)).await,
        "hub.info" => server.hub_info().await,
        "user.notify.channels" => server.user_notify_channels().await,
        "hub.run.get" => {
            server
                .hub_run_get(Parameters(decode_args(arguments)?))
                .await
        }
        "hub.run.list" => {
            server
                .hub_run_list(Parameters(decode_args(arguments)?))
                .await
        }
        "hub.job.list" => {
            server
                .hub_job_list(Parameters(decode_args(arguments)?))
                .await
        }
        "hub.job.get" => {
            server
                .hub_job_get(Parameters(decode_args(arguments)?))
                .await
        }
        "user.notify.send" => {
            server
                .user_notify_send(Parameters(decode_args(arguments)?))
                .await
        }
        "room.diary.active" => {
            server
                .room_diary_active(Parameters(decode_args(arguments)?))
                .await
        }
        "room.diary.read" => {
            server
                .room_diary_read(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.recent" => {
            server
                .room_notebook_recent(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.search" => {
            server
                .room_notebook_search(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.read" => {
            server
                .room_notebook_read(Parameters(decode_args(arguments)?))
                .await
        }
        "room.state.list" => {
            server
                .room_state_list(Parameters(decode_args(arguments)?))
                .await
        }
        "room.state.read" => {
            server
                .room_state_read(Parameters(decode_args(arguments)?))
                .await
        }
        "room.maintenance.status" => {
            server
                .room_maintenance_status(Parameters(decode_args(arguments)?))
                .await
        }
        "room.maintenance.submit" => {
            server
                .room_maintenance_submit(Parameters(decode_args(arguments)?))
                .await
        }
        "room.bootstrap" => server.room_bootstrap().await,
        "room.bootstrap.read" => {
            server
                .room_bootstrap_read(Parameters(decode_args(arguments)?))
                .await
        }
        "bootstrap" => server.bootstrap().await,
        "bootstrap.read" => {
            server
                .bootstrap_read(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.list" => server.skills_list().await,
        "skills.read" => {
            server
                .skills_read(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.search" => {
            server
                .skills_search(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.active" => server.skills_active().await,
        "skills.activate" => {
            server
                .skills_activate(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.deactivate" => {
            server
                .skills_deactivate(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.install" => {
            server
                .skills_install(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.install.get" => {
            server
                .skills_install_get(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.install.cancel" => {
            server
                .skills_install_cancel(Parameters(decode_args(arguments)?))
                .await
        }
        "skills.run" => server.skills_run(Parameters(decode_args(arguments)?)).await,
        _ => return Err(format!("Unknown tool: {name}")),
    }
    .map_err(|error| error.to_string())?;
    serde_json::to_value(result).map_err(|error| error.to_string())
}

fn decode_args<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Invalid tool arguments: {error}"))
}
pub(super) fn app_tool_descriptors(server: &AgenticMcpServer) -> Vec<Value> {
    server
        .tool_router
        .list_all()
        .into_iter()
        .filter(|tool| server.allows_tool(tool.name.as_ref()))
        .map(|tool| {
            let mut value = serde_json::to_value(tool).unwrap_or_else(|_| json!({}));
            if let Some(object) = value.as_object_mut() {
                let security_schemes = json!([{ "type": "oauth2", "scopes": ["agentic:mcp"] }]);
                object.insert("securitySchemes".to_string(), security_schemes.clone());
                object
                    .entry("_meta".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(meta) = object.get_mut("_meta").and_then(Value::as_object_mut) {
                    meta.insert("securitySchemes".to_string(), security_schemes);
                    meta.insert(
                        "openai/toolInvocation/invoking".to_string(),
                        json!("Running…"),
                    );
                    meta.insert("openai/toolInvocation/invoked".to_string(), json!("Done"));
                }
            }
            value
        })
        .collect()
}

fn negotiated_protocol_version(params: Option<&Value>) -> &'static str {
    match params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str)
    {
        Some("2025-03-26") => "2025-03-26",
        Some("2025-06-18") => "2025-06-18",
        _ => "2025-06-18",
    }
}

fn rpc_result(id: Option<Value>, result: Value) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result }))
        .into_response()
}

fn rpc_error(id: Option<Value>, code: i64, message: impl ToString) -> Response {
    Json(json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message.to_string() }
    }))
    .into_response()
}

pub(crate) async fn require_auth_on_mcp_path(
    State(state): State<HubState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    if request.uri().path().starts_with("/mcp")
        && !crate::oauth::is_valid_mcp_bearer(&state, &headers).await
    {
        return crate::oauth::mcp_unauthorized_response(&state, &headers);
    }
    next.run(request).await
}
