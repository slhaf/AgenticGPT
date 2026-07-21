use agentic_gpt_protocol::{
    BatchExecRequest, BootstrapReadRequest, DiaryAppendRequest, DiaryRecentRequest,
    DiarySelectExactRequest, ExecElement, ExecRequest, HubCommand, McpCallToolRequest,
    McpListToolsRequest, NotebookAppendRequest, NotebookCurrentRequest, NotebookRecentRequest,
    NotebookRemoveRequest, NotebookSearchRequest, NotebookSelectExactRequest,
    NotebookUpdateRequest, NotificationAction, PassageSignificance, SessionInfo,
    SkillActivationRequest, SkillInstallCancelRequest, SkillInstallFile, SkillInstallGetRequest,
    SkillInstallRequest, SkillInstallSource, SkillReadRequest, SkillRunRequest, SkillSearchRequest,
    TmuxCapturePaneRequest, TmuxCloseSessionRequest, TmuxCreateSessionRequest, TmuxExecRequest,
    TmuxListPanesRequest, TmuxPasteTextRequest, UserNotifySendRequest,
};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ErrorData, Meta, ServerCapabilities, ServerInfo, ToolAnnotations,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::agentic_result::AgenticResult;
use crate::agents::{
    cached_session, mcp_list_servers_all_agents, request_agent, timeout_batch_result,
    timeout_task_result,
};
use crate::notify::{notification_channels, send_user_notification, NotifyRouteError};
use crate::registry::{registry_entries, registry_entry};
use crate::room::{request_active_room, RoomRouteError};
use crate::runs;
use crate::state::HubState;
use crate::utils::random_id;
use crate::{default_config_summary, MAX_WAIT_SECONDS, REQUEST_TIMEOUT_SECS};

const MCP_INSTRUCTIONS: &str = "Agentic GPT Hub provides three execution layers. Use process.exec for short, one-shot inspection, detection, and deterministic tasks where an exit code is required. Use session.start for long-running or background managed processes whose lifecycle and output should be observed through session tools. Use tmux as the persistent shared workspace for stateful development, iterative debugging, TUIs, and user-agent handoff. For tmux work, discover the workspace with tmux.listSessions and tmux.listPanes, inspect it with tmux.capturePane, then use tmux.exec for shell panes or tmux.pasteText for non-shell panes. tmux.exec confirms submission to the interactive shell and returns a bounded post-submit pane snapshot; it is still not proof of command completion, so use process.exec when a deterministic exit status is required. At Room session start, call room.bootstrap, then call room.bootstrap.read for relevant guide ids listed in its manifest. Room skills are managed only by the active Room Agent: use skills.install for asynchronous GitHub/HTTPS/inline installation, then skills.install.get with waitSeconds (default 5) and pollAfterMs, and skills.install.cancel when needed. Use skills.read with an optional package-relative path for bounded resources. Use skills.run for active installed scripts; it returns terminal output inline when it completes within waitSeconds (default 5), otherwise follow the returned sessionId with session.inspect/session.wait/session.kill. Commands remain subject to Agentic local policy, path policy, confirmation, and audit.";

#[derive(Clone)]
pub(crate) struct AgenticMcpServer {
    state: HubState,
    tool_router: ToolRouter<Self>,
}

impl AgenticMcpServer {
    pub(crate) fn new(state: HubState) -> Self {
        let mut tool_router = Self::tool_router();
        decorate_tool_descriptors(&mut tool_router);
        Self { state, tool_router }
    }
}

fn decorate_tool_descriptors(tool_router: &mut ToolRouter<AgenticMcpServer>) {
    for route in tool_router.map.values_mut() {
        let name = route.attr.name.as_ref();
        let open_world = matches!(
            name,
            "process.exec"
                | "process.batchExec"
                | "session.start"
                | "mcp.callTool"
                | "tmux.pasteText"
                | "tmux.exec"
                | "tmux.createSession"
                | "tmux.closeSession"
                | "skills.install"
                | "skills.run"
        );
        let read_only = tool_is_read_only(name);
        let destructive = matches!(
            name,
            "session.kill"
                | "tmux.closeSession"
                | "room.notebook.remove"
                | "skills.install"
                | "skills.install.cancel"
                | "skills.run"
        );
        route.attr.annotations = Some(
            ToolAnnotations::new()
                .read_only(read_only)
                .destructive(destructive)
                .open_world(open_world),
        );
        route.attr.output_schema = Some(std::sync::Arc::new(object_schema()));
        let mut meta = Map::new();
        meta.insert(
            "securitySchemes".to_string(),
            json!([{ "type": "oauth2", "scopes": ["agentic:mcp"] }]),
        );
        route.attr.meta = Some(Meta(meta));
    }
}

fn tool_is_read_only(name: &str) -> bool {
    !matches!(
        name,
        "process.exec"
            | "process.batchExec"
            | "session.start"
            | "session.kill"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.callTool"
            | "user.notify.send"
            | "room.notebook.append"
            | "room.notebook.update"
            | "room.notebook.remove"
            | "room.diary.append"
            | "skills.activate"
            | "skills.deactivate"
            | "skills.install"
            | "skills.install.cancel"
            | "skills.run"
    )
}

fn object_schema() -> Map<String, Value> {
    let mut schema = Map::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("additionalProperties".to_string(), Value::Bool(true));
    schema
}

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
                "instructions": MCP_INSTRUCTIONS
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

async fn call_app_tool(server: &AgenticMcpServer, params: Value) -> Result<Value, String> {
    let object = params
        .as_object()
        .ok_or_else(|| "tools/call params must be an object".to_string())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "tools/call params.name is required".to_string())?;
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match name {
        "agent.list" => server.list_agents().await,
        "process.exec" => server.exec(Parameters(decode_args(arguments)?)).await,
        "process.batchExec" => server.batch_exec(Parameters(decode_args(arguments)?)).await,
        "session.start" => {
            server
                .start_session(Parameters(decode_args(arguments)?))
                .await
        }
        "session.list" => {
            server
                .list_sessions(Parameters(decode_args(arguments)?))
                .await
        }
        "session.inspect" => {
            server
                .inspect_session(Parameters(decode_args(arguments)?))
                .await
        }
        "session.wait" => {
            server
                .wait_session(Parameters(decode_args(arguments)?))
                .await
        }
        "session.kill" => {
            server
                .kill_session(Parameters(decode_args(arguments)?))
                .await
        }
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
        "user.notify.channels" => server.user_notify_channels().await,
        "hub.run.get" => {
            server
                .hub_run_get(Parameters(decode_args(arguments)?))
                .await
        }
        "user.notify.send" => {
            server
                .user_notify_send(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.append" => {
            server
                .room_notebook_append(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.recent" => {
            server
                .room_notebook_recent(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.selectExact" => {
            server
                .room_notebook_select_exact(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.search" => {
            server
                .room_notebook_search(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.current" => {
            server
                .room_notebook_current(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.update" => {
            server
                .room_notebook_update(Parameters(decode_args(arguments)?))
                .await
        }
        "room.notebook.remove" => {
            server
                .room_notebook_remove(Parameters(decode_args(arguments)?))
                .await
        }
        "room.diary.append" => {
            server
                .room_diary_append(Parameters(decode_args(arguments)?))
                .await
        }
        "room.diary.recent" => {
            server
                .room_diary_recent(Parameters(decode_args(arguments)?))
                .await
        }
        "room.diary.selectExact" => {
            server
                .room_diary_select_exact(Parameters(decode_args(arguments)?))
                .await
        }
        "room.bootstrap" => server.room_bootstrap().await,
        "room.bootstrap.read" => {
            server
                .room_bootstrap_read(Parameters(decode_args(arguments)?))
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

fn app_tool_descriptors(server: &AgenticMcpServer) -> Vec<Value> {
    server
        .tool_router
        .list_all()
        .into_iter()
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

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgenticMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "agentic-gpt-hub",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(MCP_INSTRUCTIONS)
    }
}

#[tool_router(router = tool_router)]
impl AgenticMcpServer {
    #[tool(
        name = "agent.list",
        description = "List registered local agents, their online status, capabilities, and safe config summaries. Use this before choosing an agentId."
    )]
    async fn list_agents(&self) -> Result<CallToolResult, ErrorData> {
        let entries = registry_entries(&self.state)
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        let online = self.state.agents.lock().await;
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
                    "lastSeenAt": status.map(|s| s.last_seen_at).or(entry.last_seen_at),
                    "capabilities": entry.capabilities,
                    "configSummary": status.and_then(|s| s.config_summary.clone()).unwrap_or_else(default_config_summary)
                })
            })
            .collect::<Vec<_>>();
        Ok(ok_json(json!({ "agents": agents })))
    }

    #[tool(
        name = "hub.run.get",
        description = "Return persisted status and optional late result for one Hub-to-Agent command run by runId."
    )]
    async fn hub_run_get(
        &self,
        params: Parameters<HubRunGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        match runs::get_run(&self.state, &params.run_id)
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?
        {
            Some(run) => Ok(ok_json(serde_json::to_value(run).map_err(|error| {
                mcp_internal_error("serialization_error", error.to_string())
            })?)),
            None => Err(mcp_invalid_params("run_not_found", "Run was not found")),
        }
    }

    #[tool(
        name = "process.exec",
        description = "Run one short, one-shot inspection, detection, or deterministic command on a local Agentic agent and return its exit status. workingDirectory is the process CWD. Use session.start for long-running managed processes and tmux for persistent collaborative work."
    )]
    async fn exec(&self, params: Parameters<ExecArgs>) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = ExecRequest {
            agent_id: params.agent_id.clone(),
            program: params.program,
            args: params.args.unwrap_or_default(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            working_directory: params.working_directory,
        };
        let task_id = random_id("task");
        let command = HubCommand::Exec {
            request_id: random_id("req"),
            task_id: task_id.clone(),
            payload: payload.clone(),
        };
        let value = match request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        {
            Ok(value) => value,
            Err(reason) => {
                serde_json::to_value(timeout_task_result(&payload.agent_id, &task_id, reason))
                    .unwrap_or_else(|error| json!({ "error": error.to_string() }))
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.batchExec",
        description = "Run multiple short commands on a local Agentic agent. Top-level workingDirectory is the default process CWD for all elements; per-element workingDirectory overrides it."
    )]
    async fn batch_exec(
        &self,
        params: Parameters<BatchExecArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = BatchExecRequest {
            agent_id: params.agent_id.clone(),
            elements: params
                .elements
                .into_iter()
                .map(|element| ExecElement {
                    program: element.program,
                    args: element.args.unwrap_or_default(),
                    working_directory: element.working_directory,
                })
                .collect(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            working_directory: params.working_directory,
        };
        let batch_id = random_id("batch");
        let command = HubCommand::BatchExec {
            request_id: random_id("req"),
            task_id: batch_id.clone(),
            payload: payload.clone(),
        };
        let value = match request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        {
            Ok(value) => value,
            Err(reason) => serde_json::to_value(timeout_batch_result(&payload, &batch_id, reason))
                .unwrap_or_else(|error| json!({ "error": error.to_string() })),
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "session.start",
        description = "Start a long-running or background managed process on a local Agentic agent. workingDirectory is the process CWD; observe lifecycle and output with session.wait or session.inspect. Use tmux instead when the work needs a persistent interactive workspace or user-agent handoff."
    )]
    async fn start_session(
        &self,
        params: Parameters<ExecArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = ExecRequest {
            agent_id: params.agent_id.clone(),
            program: params.program,
            args: params.args.unwrap_or_default(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            working_directory: params.working_directory,
        };
        let session_id = random_id("sess");
        let command = HubCommand::StartSession {
            request_id: random_id("req"),
            session_id: session_id.clone(),
            payload: payload.clone(),
        };
        let value = match request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        {
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
                json!({ "status": status, "sessionId": session_id, "session": value })
            }
            Err(reason) => {
                json!({ "error": { "code": "session_start_timeout", "message": reason } })
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "session.list",
        description = "List running or recently cached command sessions for a local Agentic agent."
    )]
    async fn list_sessions(
        &self,
        params: Parameters<AgentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let agent_id = params.0.agent_id;
        self.ensure_agent_enabled(&agent_id)?;
        let command = HubCommand::ListSessions {
            request_id: random_id("req"),
        };
        let value = match request_agent(&self.state, &agent_id, command, 2).await {
            Ok(value) => json!({ "sessions": value }),
            Err(_) => {
                let sessions = self
                    .state
                    .sessions
                    .lock()
                    .await
                    .get(&agent_id)
                    .map(|sessions| sessions.values().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                json!({ "sessions": sessions })
            }
        };
        Ok(ok_json(value))
    }

    #[tool(
        name = "session.inspect",
        description = "Inspect one command session by id and return current state plus recent stdout/stderr tails."
    )]
    async fn inspect_session(
        &self,
        params: Parameters<SessionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::InspectSession {
            request_id: random_id("req"),
            session_id: params.session_id.clone(),
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 2).await {
            Ok(value) if !value.is_null() => value,
            _ => match cached_session(&self.state, &params.agent_id, &params.session_id).await {
                Some(session) => serde_json::to_value(session)
                    .unwrap_or_else(|error| json!({ "error": error.to_string() })),
                None => {
                    json!({ "error": { "code": "session_not_found", "message": "Session was not found" } })
                }
            },
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "session.wait",
        description = "Wait up to seconds for a session update, capped at 30 seconds. Use 0 or omit seconds to return cached state immediately."
    )]
    async fn wait_session(
        &self,
        params: Parameters<WaitSessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let seconds = params.seconds.unwrap_or(0).min(MAX_WAIT_SECONDS);
        let command = HubCommand::WaitSession {
            request_id: random_id("req"),
            session_id: params.session_id.clone(),
            seconds,
        };
        let value = match request_agent(&self.state, &params.agent_id, command, seconds + 2).await {
            Ok(value) if !value.is_null() => value,
            _ => match cached_session(&self.state, &params.agent_id, &params.session_id).await {
                Some(session) => serde_json::to_value(session)
                    .unwrap_or_else(|error| json!({ "error": error.to_string() })),
                None => {
                    json!({ "error": { "code": "session_not_found", "message": "Session was not found" } })
                }
            },
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "session.kill",
        description = "Kill a running command session by id on a local Agentic agent."
    )]
    async fn kill_session(
        &self,
        params: Parameters<SessionIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::KillSession {
            request_id: random_id("req"),
            session_id: params.session_id.clone(),
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 5).await {
            Ok(value) if !value.is_null() => value,
            _ => match cached_session(&self.state, &params.agent_id, &params.session_id).await {
                Some(session) => serde_json::to_value(session)
                    .unwrap_or_else(|error| json!({ "error": error.to_string() })),
                None => {
                    json!({ "error": { "code": "session_not_found", "message": "Session was not found" } })
                }
            },
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.listSessions",
        description = "List persistent tmux shared workspaces on a local Agentic agent. Start tmux workflows here, then use tmux.listPanes to locate the relevant pane."
    )]
    async fn tmux_list_sessions(
        &self,
        params: Parameters<TmuxListSessionsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxListSessions {
            request_id: random_id("req"),
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.listPanes",
        description = "List tmux panes, optionally scoped to one session. Returns target ids, cwd, foreground command, size, process/mode state, and isLikelyShell. Inspect this before choosing tmux.exec for a shell pane or tmux.pasteText for a non-shell pane."
    )]
    async fn tmux_list_panes(
        &self,
        params: Parameters<TmuxListPanesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxListPanes {
            request_id: random_id("req"),
            payload: TmuxListPanesRequest {
                session: params.session,
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.capturePane",
        description = "Capture recent tmux pane history to inspect a shared shell or TUI before acting and to observe progress afterward. This is the primary observation mechanism for tmux.exec, which does not report command completion."
    )]
    async fn tmux_capture_pane(
        &self,
        params: Parameters<TmuxCapturePaneArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxCapturePane {
            request_id: random_id("req"),
            payload: TmuxCapturePaneRequest {
                target: params.target,
                lines: params.lines.unwrap_or(160),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.pasteText",
        description = "Paste text into a non-shell tmux pane or TUI, optionally appending Enter. Shell panes are rejected; use tmux.exec there. Defaults to confirmation because this writes into an interactive workspace."
    )]
    async fn tmux_paste_text(
        &self,
        params: Parameters<TmuxPasteTextArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxPasteText {
            request_id: random_id("req"),
            payload: TmuxPasteTextRequest {
                target: params.target,
                text: params.text,
                submit: params.submit.unwrap_or(false),
                need_confirm: params.need_confirm.unwrap_or(true),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 65)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.exec",
        description = "Submit one structured program and argument vector atomically to an existing tmux shell pane for persistent collaborative work. The pane cwd, command policy, path policy, confirmation, and audit apply. Returns a bounded post-submit pane snapshot after waitMs; this is not proof of completion, so use exec when an exit status is required."
    )]
    async fn tmux_exec(
        &self,
        params: Parameters<TmuxExecArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxExec {
            request_id: random_id("req"),
            payload: TmuxExecRequest {
                target: params.target,
                program: params.program,
                args: params.args,
                need_confirm: params.need_confirm.unwrap_or(false),
                wait_ms: params.wait_ms.unwrap_or(300),
                capture_lines: params.capture_lines.unwrap_or(120),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 65)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.createSession",
        description = "Create an idempotent persistent tmux workspace when no suitable session exists. Prefer reusing the default agentic session; cwd is checked by the local path policy."
    )]
    async fn tmux_create_session(
        &self,
        params: Parameters<TmuxCreateSessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxCreateSession {
            request_id: random_id("req"),
            payload: TmuxCreateSessionRequest {
                name: params.name,
                cwd: params.cwd,
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 5)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.closeSession",
        description = "Close a persistent tmux workspace only when it is explicitly no longer needed. Defaults to local confirmation."
    )]
    async fn tmux_close_session(
        &self,
        params: Parameters<TmuxCloseSessionArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::TmuxCloseSession {
            request_id: random_id("req"),
            payload: TmuxCloseSessionRequest {
                name: params.name,
                need_confirm: params.need_confirm.unwrap_or(true),
            },
        };
        let value = request_agent(&self.state, &params.agent_id, command, 65)
            .await
            .map_err(|reason| mcp_internal_error("tmux_request_timeout", reason))?;
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.listServers",
        description = "List MCP servers configured inside local Agentic agents. If agentId is omitted, returns MCP servers for all connected agents grouped by agent. If agentId is provided, returns that agent's servers."
    )]
    async fn mcp_list_servers(
        &self,
        params: Parameters<McpListServersArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Some(agent_id) = params.0.agent_id {
            self.ensure_agent_enabled(&agent_id)?;
            let command = HubCommand::McpListServers {
                request_id: random_id("req"),
            };
            let value = request_agent(&self.state, &agent_id, command, REQUEST_TIMEOUT_SECS)
                .await
                .unwrap_or_else(|reason| json!({ "error": { "code": "mcp_list_servers_timeout", "message": reason } }));
            return Ok(result_from_value(value));
        }

        let value = mcp_list_servers_all_agents(&self.state)
            .await
            .unwrap_or_else(|reason| json!({ "error": { "code": "db_error", "message": reason } }));
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.listTools",
        description = "List tools exposed by one MCP server configured inside a local Agentic agent. Use mcp.callTool with the returned serverId and tool name."
    )]
    async fn mcp_list_tools(
        &self,
        params: Parameters<McpListToolsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = McpListToolsRequest {
            agent_id: params.agent_id.clone(),
            server_id: params.server_id,
        };
        let command = HubCommand::McpListTools {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let value = request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(
            |reason| json!({ "error": { "code": "mcp_list_tools_timeout", "message": reason } }),
        );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.callTool",
        description = "Call a tool on an MCP server configured inside a local Agentic agent. Arguments are forwarded as JSON; local Agentic confirmation and policy still apply."
    )]
    async fn mcp_call_tool(
        &self,
        params: Parameters<McpCallToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = McpCallToolRequest {
            agent_id: params.agent_id.clone(),
            server_id: params.server_id,
            tool_name: params.tool_name,
            arguments: params.arguments.unwrap_or_else(|| json!({})),
        };
        let command = HubCommand::McpCallTool {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let value = request_agent(
            &self.state,
            &payload.agent_id,
            command,
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(
            |reason| json!({ "error": { "code": "mcp_call_tool_timeout", "message": reason } }),
        );
        Ok(result_from_mcp_tool_value(value))
    }

    #[tool(
        name = "user.notify.channels",
        description = "List Hub-native user notification channels. This does not use the active Room Agent."
    )]
    async fn user_notify_channels(&self) -> Result<CallToolResult, ErrorData> {
        let channels = notification_channels(&self.state)
            .await
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(json!({ "channels": channels })))
    }

    #[tool(
        name = "user.notify.send",
        description = "Send a Hub-native notification to the user through a selected channel. This does not use the active Room Agent."
    )]
    async fn user_notify_send(
        &self,
        params: Parameters<UserNotifySendArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let request = UserNotifySendRequest {
            channel_key: params.channel,
            title: params.title,
            body: params.body,
            actions: params
                .actions
                .unwrap_or_default()
                .into_iter()
                .map(Into::into)
                .collect(),
            priority: params.priority,
        };
        let value = send_user_notification(&self.state, request)
            .await
            .map(serde_json::to_value)
            .unwrap_or_else(|error| {
                Ok(json!({
                    "error": {
                        "code": notify_route_error_code(&error),
                        "message": notify_route_error_message(&error)
                    }
                }))
            })
            .unwrap_or_else(|error| json!({ "error": { "code": "serialization_error", "message": error.to_string() } }));
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.append",
        description = "Append an explicit notebook passage to the active Room Agent. ANCHOR passages update current state for their scope. No agentId is used."
    )]
    async fn room_notebook_append(
        &self,
        params: Parameters<RoomNotebookAppendArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = NotebookAppendRequest {
            datetime: params
                .datetime
                .map(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .map(|datetime| datetime.with_timezone(&chrono::Utc))
                        .map_err(|error| mcp_invalid_params("invalid_datetime", error.to_string()))
                })
                .transpose()?,
            scope: params.scope,
            significance: parse_significance(&params.significance)?,
            abstract_text: params.abstract_text,
            content: params.content,
            tags: params.tags.unwrap_or_default(),
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookAppend {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.recent",
        description = "Return recent notebook passages from the active Room Agent. Supports optional scope and significance filters. No agentId is used."
    )]
    async fn room_notebook_recent(
        &self,
        params: Parameters<RoomNotebookRecentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = NotebookRecentRequest {
            scope: params.scope,
            days: params.days,
            significance: params
                .significance
                .as_deref()
                .map(parse_significance)
                .transpose()?,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookRecent {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.selectExact",
        description = "Return notebook passages for one exact room-timezone calendar day from the active Room Agent. No agentId is used."
    )]
    async fn room_notebook_select_exact(
        &self,
        params: Parameters<RoomNotebookSelectExactArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = NotebookSelectExactRequest {
            year: params.year,
            month: params.month,
            day: params.day,
            scope: params.scope,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookSelectExact {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.search",
        description = "Search notebook passages in the active Room Agent by substring over abstract, content, scope, and tags. No vector search is used in V1."
    )]
    async fn room_notebook_search(
        &self,
        params: Parameters<RoomNotebookSearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = NotebookSearchRequest {
            query: params.query,
            scope: params.scope,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookSearch {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.current",
        description = "Return current recoverable notebook state for one scope, derived from current state or latest ANCHOR. No agentId is used."
    )]
    async fn room_notebook_current(
        &self,
        params: Parameters<RoomNotebookCurrentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = NotebookCurrentRequest {
            scope: params.scope,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookCurrent {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.update",
        description = "Update editable fields of one notebook passage in the active Room Agent. Scope and datetime are immutable; current state is refreshed when anchors change."
    )]
    async fn room_notebook_update(
        &self,
        params: Parameters<RoomNotebookUpdateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = NotebookUpdateRequest {
            id: params.id,
            significance: params
                .significance
                .as_deref()
                .map(parse_significance)
                .transpose()?,
            abstract_text: params.abstract_text,
            content: params.content,
            tags: params.tags,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookUpdate {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.remove",
        description = "Physically remove one notebook passage from the active Room Agent. If it was current, current state falls back to the latest ANCHOR or becomes null."
    )]
    async fn room_notebook_remove(
        &self,
        params: Parameters<RoomNotebookRemoveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = NotebookRemoveRequest { id: params.id };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookRemove {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.diary.append",
        description = "Append one diary entry to the active Room Agent. The entry is stored under the current logical diary day in workspace diary storage. No agentId is used."
    )]
    async fn room_diary_append(
        &self,
        params: Parameters<RoomDiaryAppendArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = DiaryAppendRequest {
            time_hint: params.time_hint,
            tags: params.tags.unwrap_or_default(),
            entry: params.entry,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomDiaryAppend {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.diary.recent",
        description = "Return recent diary entries from the active Room Agent. Scans recent logical diary days in workspace diary storage. No agentId is used."
    )]
    async fn room_diary_recent(
        &self,
        params: Parameters<RoomDiaryRecentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = DiaryRecentRequest {
            days: params.days,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomDiaryRecent {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.diary.selectExact",
        description = "Return diary entries for one exact logical diary date from the active Room Agent workspace diary storage. No agentId is used."
    )]
    async fn room_diary_select_exact(
        &self,
        params: Parameters<RoomDiarySelectExactArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = DiarySelectExactRequest {
            year: params.year,
            month: params.month,
            day: params.day,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomDiarySelectExact {
                request_id: random_id("req"),
                payload: payload.clone(),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.bootstrap",
        description = "Load the active Room Agent session bootstrap entrypoint and deterministic guide manifest. Call this at Room session start; it takes no agentId."
    )]
    async fn room_bootstrap(&self) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomBootstrap {
                request_id: random_id("req"),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_bootstrap_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.bootstrap.read",
        description = "Read one valid bootstrap guide by id from the active Room Agent workspace, including typed metadata, raw frontmatter, and bounded Markdown content. No agentId is used."
    )]
    async fn room_bootstrap_read(
        &self,
        params: Parameters<BootstrapReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomBootstrapRead {
                request_id: random_id("req"),
                payload: BootstrapReadRequest { id: params.0.id },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_bootstrap_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.list",
        description = "List valid skills in the active Room Agent workspace. Scans workspace skills/*/SKILL.md and marks active skills. No agentId is used."
    )]
    async fn skills_list(&self) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsList {
                request_id: random_id("req"),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.read",
        description = "Read one skill package from the active Room Agent workspace by id, including SKILL.md, parsed frontmatter, package summary, warnings, and active status. No agentId is used."
    )]
    async fn skills_read(
        &self,
        params: Parameters<SkillReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = SkillReadRequest {
            id: params.0.id,
            path: params.0.path,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.search",
        description = "Search skills in the active Room Agent workspace by case-insensitive substring over id, frontmatter, tags, and SKILL.md content. No agentId is used."
    )]
    async fn skills_search(
        &self,
        params: Parameters<SkillSearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = SkillSearchRequest {
            query: params.query,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsSearch {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.active",
        description = "Return active skills for the active Room Agent workspace. Deleted skills remain listed as stale/missing until deactivated. No agentId is used."
    )]
    async fn skills_active(&self) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsActive {
                request_id: random_id("req"),
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.activate",
        description = "Mark an existing valid skill as active in the active Room Agent workspace. This grants no permissions and executes nothing. No agentId is used."
    )]
    async fn skills_activate(
        &self,
        params: Parameters<SkillActivationArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = SkillActivationRequest { id: params.0.id };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsActivate {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.deactivate",
        description = "Remove active state for a skill id in the active Room Agent workspace. This succeeds even for stale/missing skills. No agentId is used."
    )]
    async fn skills_deactivate(
        &self,
        params: Parameters<SkillActivationArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = SkillActivationRequest { id: params.0.id };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsDeactivate {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.install",
        description = "Start an asynchronous installation of one Room skill from public GitHub, HTTPS file entries, or inline content. Returns an installId immediately; poll with skills.install.get. No agentId is used."
    )]
    async fn skills_install(
        &self,
        params: Parameters<SkillInstallArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = SkillInstallRequest {
            id: params.id,
            source: params.source.into_protocol(),
            replace_existing: params.replace_existing,
            activate_after_install: params.activate_after_install,
            idempotency_key: params.idempotency_key,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsInstall {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.install.get",
        description = "Return the latest asynchronous Room skill installation status. Waits up to waitSeconds (default 5, maximum 30) and returns pollAfterMs guidance. No agentId is used."
    )]
    async fn skills_install_get(
        &self,
        params: Parameters<SkillInstallGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsInstallGet {
                request_id: random_id("req"),
                payload: SkillInstallGetRequest {
                    install_id: params.install_id,
                    wait_seconds: params.wait_seconds,
                },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.install.cancel",
        description = "Request cooperative cancellation of a Room skill installation. Cancellation is idempotent and reports tooLate once atomic commit has begun. No agentId is used."
    )]
    async fn skills_install_cancel(
        &self,
        params: Parameters<SkillInstallCancelArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsInstallCancel {
                request_id: random_id("req"),
                payload: SkillInstallCancelRequest {
                    install_id: params.0.install_id,
                },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }

    #[tool(
        name = "skills.run",
        description = "Run an executable scripts/ file from an active workspace skill through the Room Agent managed-session engine. Returns terminal output inline when it finishes within waitSeconds (default 5), otherwise returns the session id for session.inspect/session.wait/session.kill. No agentId is used."
    )]
    async fn skills_run(
        &self,
        params: Parameters<SkillRunArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let session_id = random_id("sess");
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsRun {
                request_id: random_id("req"),
                session_id,
                payload: SkillRunRequest {
                    id: params.id,
                    path: params.path,
                    args: params.args,
                    working_directory: params.working_directory,
                    wait_seconds: params.wait_seconds,
                },
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(room_route_error_value);
        Ok(result_from_value(value))
    }
}

impl AgenticMcpServer {
    fn ensure_agent_enabled(&self, agent_id: &str) -> Result<(), ErrorData> {
        match registry_entry(&self.state, agent_id) {
            Ok(Some(entry)) if entry.enabled => Ok(()),
            Ok(_) => Err(mcp_invalid_params(
                "agent_not_found",
                "Agent is not registered or enabled",
            )),
            Err(error) => Err(mcp_internal_error("db_error", error.to_string())),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct AgentIdArgs {
    #[schemars(
        description = "Target local agent id. Room notebook tools do not use agentId; they route to the active Room Agent."
    )]
    agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct HubRunGetArgs {
    #[schemars(description = "Run id returned by a timed-out Hub request.")]
    run_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExecArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(
        description = "Executable name or path. For shell syntax, use bash or sh with args such as ['-lc', '...']."
    )]
    program: String,
    #[serde(default)]
    #[schemars(
        description = "Argument vector passed directly to the program; this is not a shell-split string."
    )]
    args: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(
        description = "Request confirmation before execution. Local policy may still allow, confirm, or deny regardless of this flag."
    )]
    need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional per-request confirmation provider override. Omit or use default to follow local agent config."
    )]
    confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Process working directory. Relative values resolve from the agent workspace root; prefer this over cd in shell commands."
    )]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BatchExecArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(
        description = "Commands to run. Each element can override the top-level workingDirectory."
    )]
    elements: Vec<BatchExecElementArgs>,
    #[serde(default)]
    #[schemars(
        description = "Request confirmation for the batch. Local policy may still allow, confirm, or deny regardless of this flag."
    )]
    need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional per-request confirmation provider override for all batch elements."
    )]
    confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Default process working directory for all batch elements. Relative values resolve from the agent workspace root."
    )]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BatchExecElementArgs {
    #[schemars(description = "Executable name or path for this batch element.")]
    program: String,
    #[serde(default)]
    #[schemars(description = "Argument vector passed directly to the program.")]
    args: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(
        description = "Per-element process working directory. Overrides the batch workingDirectory."
    )]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SessionIdArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "Session id returned by session.start or session.list.")]
    session_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct WaitSessionArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "Session id returned by session.start or session.list.")]
    session_id: String,
    #[serde(default)]
    #[schemars(
        description = "Maximum seconds to wait for new session output or state, capped at 30. Use 0 or omit for immediate cached state."
    )]
    seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxListSessionsArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxListPanesArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[serde(default)]
    #[schemars(description = "Optional tmux session name to scope pane listing.")]
    session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxCapturePaneArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux target such as session:window.pane or a pane id like %0.")]
    target: String,
    #[serde(default)]
    #[schemars(
        description = "Number of recent tmux history lines to capture. Defaults to 160 and caps at 5000."
    )]
    lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxPasteTextArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux target such as session:window.pane or a pane id like %0.")]
    target: String,
    #[schemars(description = "Text to paste into the tmux pane.")]
    text: String,
    #[serde(default)]
    #[schemars(description = "Append Enter after pasting the text. Defaults to false.")]
    submit: Option<bool>,
    #[serde(default)]
    #[schemars(description = "Request local confirmation before pasting. Defaults to true.")]
    need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxExecArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "Shell pane target such as session:window.pane or %0.")]
    target: String,
    #[schemars(description = "Program or shell builtin to execute as one command.")]
    program: String,
    #[serde(default)]
    #[schemars(description = "Structured argument vector; shell operators are not interpreted.")]
    args: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Force local confirmation in addition to configured policy.")]
    need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Milliseconds to wait before returning the post-submit pane snapshot. Defaults to 300 and caps at 5000."
    )]
    wait_ms: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Number of tmux history lines to include in the post-submit snapshot. Defaults to 120, caps at 5000, and 0 disables the snapshot."
    )]
    capture_lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxCreateSessionArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux session name.")]
    name: String,
    #[schemars(description = "Session cwd, subject to the local agent path policy.")]
    cwd: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct TmuxCloseSessionArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "tmux session name.")]
    name: String,
    #[serde(default)]
    #[schemars(description = "Request local confirmation before closing. Defaults to true.")]
    need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpListServersArgs {
    #[serde(default)]
    #[schemars(
        description = "Optional target local agent id. Omit to list MCP servers for all currently connected agents."
    )]
    agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpListToolsArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "MCP server id returned by mcp.listServers.")]
    server_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpCallToolArgs {
    #[schemars(description = "Target local agent id.")]
    agent_id: String,
    #[schemars(description = "MCP server id returned by mcp.listServers.")]
    server_id: String,
    #[schemars(description = "Tool name returned by mcp.listTools.")]
    tool_name: String,
    #[serde(default)]
    #[schemars(description = "JSON object arguments forwarded to the MCP tool.")]
    arguments: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UserNotifySendArgs {
    #[schemars(description = "Notification channel key returned by user.notify.channels.")]
    channel: String,
    #[schemars(description = "Notification title.")]
    title: String,
    #[schemars(description = "Notification body.")]
    body: String,
    #[serde(default)]
    #[schemars(description = "Optional notification actions. Phase A does not deliver actions.")]
    actions: Option<Vec<UserNotifyActionArgs>>,
    #[serde(default)]
    #[schemars(description = "Optional priority such as low, normal, high, urgent, or alarm.")]
    priority: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UserNotifyActionArgs {
    #[schemars(description = "Stable action id. Android ack will report this as actionId.")]
    id: String,
    #[schemars(description = "Human-readable action label.")]
    label: String,
}

impl From<UserNotifyActionArgs> for NotificationAction {
    fn from(value: UserNotifyActionArgs) -> Self {
        Self {
            id: value.id,
            label: value.label,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookAppendArgs {
    #[serde(default)]
    #[schemars(
        description = "Optional ISO-8601 datetime. Stored as UTC; file partitioning uses the configured room timezone."
    )]
    datetime: Option<String>,
    #[schemars(
        description = "Path-safe notebook namespace such as agentic or monopoly; used for current state and filtering."
    )]
    scope: String,
    #[schemars(
        description = "NORMAL for ordinary passages; ANCHOR for passages that should update current state for the scope."
    )]
    significance: String,
    #[serde(rename = "abstract")]
    #[schemars(description = "Short summary used in timelines and previews.")]
    abstract_text: String,
    #[schemars(description = "Full recoverable passage content.")]
    content: String,
    #[serde(default)]
    #[schemars(description = "Optional labels included in simple search.")]
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookRecentArgs {
    #[serde(default)]
    #[schemars(description = "Optional path-safe notebook scope filter.")]
    scope: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Number of recent room-timezone calendar days to scan. Defaults to 5 and is capped at 30."
    )]
    days: Option<u32>,
    #[serde(default)]
    #[schemars(description = "Optional significance filter: NORMAL or ANCHOR.")]
    significance: Option<String>,
    #[serde(default)]
    #[schemars(description = "Maximum passages returned, capped by the server.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookSelectExactArgs {
    #[schemars(description = "Year in the configured room timezone calendar.")]
    year: i32,
    #[schemars(description = "Month in the configured room timezone calendar, 1-12.")]
    month: u32,
    #[schemars(description = "Day of month in the configured room timezone calendar.")]
    day: u32,
    #[serde(default)]
    #[schemars(description = "Optional path-safe notebook scope filter.")]
    scope: Option<String>,
    #[serde(default)]
    #[schemars(description = "Maximum passages returned, capped by the server.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookSearchArgs {
    #[schemars(
        description = "Case-insensitive substring query over abstract, content, scope, and tags."
    )]
    query: String,
    #[serde(default)]
    #[schemars(description = "Optional path-safe notebook scope filter.")]
    scope: Option<String>,
    #[serde(default)]
    #[schemars(description = "Maximum passages returned, capped by the server.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookCurrentArgs {
    #[schemars(description = "Path-safe notebook scope whose current state should be returned.")]
    scope: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookUpdateArgs {
    #[schemars(description = "Passage id returned by append, recent, search, or selectExact.")]
    id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional new significance: NORMAL or ANCHOR. Scope and datetime cannot be changed."
    )]
    significance: Option<String>,
    #[serde(rename = "abstract", default)]
    #[schemars(description = "Optional new short summary used in timelines and previews.")]
    abstract_text: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional new full recoverable passage content.")]
    content: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional replacement tag list included in simple search.")]
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookRemoveArgs {
    #[schemars(
        description = "Passage id returned by append, recent, search, or selectExact. Removal is physical in V1."
    )]
    id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomDiaryAppendArgs {
    #[serde(default)]
    #[schemars(
        description = "Optional daypart label such as morning, noon, afternoon, evening, bedtime, or unknown. Stored as metadata only; the date is derived from the current logical diary day."
    )]
    time_hint: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional labels included with the diary entry.")]
    tags: Option<Vec<String>>,
    #[schemars(description = "Diary entry text to append.")]
    entry: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomDiaryRecentArgs {
    #[serde(default)]
    #[schemars(
        description = "Number of recent logical diary days to scan. Defaults to 3 and is capped at 30."
    )]
    days: Option<u32>,
    #[serde(default)]
    #[schemars(description = "Maximum diary entries returned, capped by the server.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomDiarySelectExactArgs {
    #[schemars(description = "Year in the logical diary date.")]
    year: i32,
    #[schemars(description = "Month in the logical diary date, 1-12.")]
    month: u32,
    #[schemars(description = "Day of month in the logical diary date.")]
    day: u32,
    #[serde(default)]
    #[schemars(description = "Maximum diary entries returned, capped by the server.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BootstrapReadArgs {
    #[schemars(description = "Guide id returned by room.bootstrap.")]
    id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillReadArgs {
    #[schemars(description = "Skill id, matching one workspace skills/ directory name.")]
    id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional package-relative file path. Omit to read the legacy SKILL.md response."
    )]
    path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillSearchArgs {
    #[schemars(
        description = "Case-insensitive substring query over id, frontmatter, tags, and SKILL.md content."
    )]
    query: String,
    #[serde(default)]
    #[schemars(description = "Maximum skills returned. Defaults to 20 and caps at 100.")]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillActivationArgs {
    #[schemars(description = "Skill id, matching one workspace skills/ directory name.")]
    id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallArgs {
    #[schemars(description = "Target skill id. One installation job targets exactly one id.")]
    id: String,
    #[schemars(description = "GitHub, HTTPS-file, or inline-content source descriptor.")]
    source: SkillInstallSourceArgs,
    #[serde(default)]
    #[schemars(
        description = "Archive an existing workspace skill before replacement. Defaults to false."
    )]
    replace_existing: bool,
    #[serde(default)]
    #[schemars(
        description = "Optional explicit activation choice; new skills default active and replacement preserves its prior state."
    )]
    activate_after_install: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional idempotency key for safe retries of the same install request."
    )]
    idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
enum SkillInstallSourceArgs {
    Github {
        #[serde(default)]
        repository: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(rename = "ref", default)]
        ref_name: Option<String>,
        #[serde(default)]
        path: Option<String>,
    },
    Files {
        files: Vec<SkillInstallFileArgs>,
    },
}

impl SkillInstallSourceArgs {
    fn into_protocol(self) -> SkillInstallSource {
        match self {
            Self::Github {
                repository,
                url,
                ref_name,
                path,
            } => SkillInstallSource::Github {
                repository,
                url,
                ref_name,
                path,
            },
            Self::Files { files } => SkillInstallSource::Files {
                files: files
                    .into_iter()
                    .map(SkillInstallFileArgs::into_protocol)
                    .collect(),
            },
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallFileArgs {
    path: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    content_base64: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    executable: Option<bool>,
}

impl SkillInstallFileArgs {
    fn into_protocol(self) -> SkillInstallFile {
        SkillInstallFile {
            path: self.path,
            url: self.url,
            content: self.content,
            content_base64: self.content_base64,
            sha256: self.sha256,
            executable: self.executable,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallGetArgs {
    install_id: String,
    #[serde(default)]
    #[schemars(description = "Seconds to wait for a newer status revision, 0-30. Defaults to 5.")]
    wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillInstallCancelArgs {
    install_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SkillRunArgs {
    id: String,
    #[schemars(description = "Package-relative executable path under scripts/.")]
    path: String,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    working_directory: Option<String>,
    #[serde(default)]
    #[schemars(description = "Bounded inline wait in seconds, 0-30. Defaults to 5.")]
    wait_seconds: Option<u64>,
}

fn ok_json(value: Value) -> CallToolResult {
    AgenticResult::from_native_value(value).into_call_tool_result()
}

fn result_from_value(value: Value) -> CallToolResult {
    AgenticResult::from_native_value(value).into_call_tool_result()
}

fn result_from_mcp_tool_value(value: Value) -> CallToolResult {
    AgenticResult::from_mcp_or_native_value(value).into_call_tool_result()
}

fn room_route_error_value(error: RoomRouteError) -> Value {
    room_route_error_value_with_timeout(error, "room_notebook_timeout")
}

fn room_route_error_value_with_timeout(error: RoomRouteError, timeout_code: &'static str) -> Value {
    match error {
        RoomRouteError::NotActive => json!({
            "error": { "code": "room_not_active", "message": "no active room agent" }
        }),
        RoomRouteError::StateConflict => json!({
            "error": { "code": "room_state_conflict", "message": "active room state is inconsistent" }
        }),
        RoomRouteError::Timeout(reason) => json!({
            "error": { "code": timeout_code, "message": reason }
        }),
    }
}

fn parse_significance(value: &str) -> Result<PassageSignificance, ErrorData> {
    match value {
        "NORMAL" => Ok(PassageSignificance::Normal),
        "ANCHOR" => Ok(PassageSignificance::Anchor),
        _ => Err(mcp_invalid_params(
            "invalid_significance",
            "significance must be NORMAL or ANCHOR",
        )),
    }
}

fn mcp_invalid_params(code: &'static str, message: impl ToString) -> ErrorData {
    ErrorData::invalid_params(message.to_string(), Some(json!({ "code": code })))
}

fn mcp_internal_error(code: &'static str, message: String) -> ErrorData {
    ErrorData::internal_error(message, Some(json!({ "code": code })))
}

fn notify_route_error_code(error: &NotifyRouteError) -> &'static str {
    match error {
        NotifyRouteError::InvalidChannel(_) => "invalid_notify_channel",
        NotifyRouteError::AgentNotFound(_) => "agent_alias_not_found",
        NotifyRouteError::ChannelUnavailable { reason, .. } => reason,
        NotifyRouteError::DeliveryFailed { .. } => "notify_delivery_failed",
        NotifyRouteError::Db(_) => "db_error",
    }
}

fn notify_route_error_message(error: &NotifyRouteError) -> String {
    match error {
        NotifyRouteError::InvalidChannel(channel) => {
            format!("Invalid notification channel: {channel}")
        }
        NotifyRouteError::AgentNotFound(alias) => {
            format!("No enabled agent found for alias: {alias}")
        }
        NotifyRouteError::ChannelUnavailable {
            channel_key,
            reason,
        } => format!("Notification channel {channel_key} is unavailable: {reason}"),
        NotifyRouteError::DeliveryFailed {
            channel_key,
            reason,
        } => format!("Notification delivery failed for {channel_key}: {reason}"),
        NotifyRouteError::Db(reason) => reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::{HubConfig, RemoteConfirmationConfig};
    use axum::body::to_bytes;
    use axum::extract::State;
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn test_hub_config() -> HubConfig {
        HubConfig {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: true,
                provider: "ntfy".to_string(),
                timeout_seconds: 45,
                ntfy: crate::NtfyConfig {
                    server_url: "https://ntfy.example.invalid".to_string(),
                    topic: "secret-topic-for-test".to_string(),
                    callback_base_url: "https://callback.example.invalid".to_string(),
                },
            },
        }
    }

    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(test_hub_config()),
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: Some("https://hub.example.invalid".to_string()),
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(Some(crate::notify::NtfyHealthCache {
                server_url: "https://ntfy.example.invalid".to_string(),
                checked_at: chrono::Utc::now(),
                result: crate::notify::NtfyHealthStatus::Healthy,
            }))),
        }
    }

    #[test]
    fn tool_read_only_hints_match_side_effect_semantics() {
        for name in [
            "agent.list",
            "session.list",
            "tmux.listSessions",
            "tmux.listPanes",
            "tmux.capturePane",
            "hub.run.get",
            "room.notebook.search",
            "room.diary.recent",
            "room.diary.selectExact",
            "room.bootstrap",
            "room.bootstrap.read",
            "skills.list",
            "skills.read",
            "skills.search",
            "skills.active",
            "skills.install.get",
        ] {
            assert!(tool_is_read_only(name), "{name} should be read-only");
        }
        for name in [
            "process.exec",
            "session.start",
            "session.kill",
            "tmux.exec",
            "tmux.pasteText",
            "tmux.createSession",
            "tmux.closeSession",
            "mcp.callTool",
            "user.notify.send",
            "room.notebook.append",
            "room.notebook.update",
            "room.notebook.remove",
            "room.diary.append",
            "skills.activate",
            "skills.deactivate",
            "skills.install",
            "skills.install.cancel",
            "skills.run",
        ] {
            assert!(!tool_is_read_only(name), "{name} should not be read-only");
        }
    }

    #[test]
    fn skill_install_and_run_tools_are_exposed_with_stable_annotations() {
        let server = AgenticMcpServer::new(test_state());
        let tools = app_tool_descriptors(&server);
        let mut names = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        names.sort_unstable();
        for name in [
            "room.bootstrap",
            "room.bootstrap.read",
            "skills.install",
            "skills.install.get",
            "skills.install.cancel",
            "skills.run",
        ] {
            assert!(names.contains(&name), "missing MCP tool {name}");
        }
        let install = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("skills.install"))
            .unwrap();
        assert_eq!(install["annotations"]["readOnlyHint"], false);
        assert_eq!(install["annotations"]["destructiveHint"], true);
        let get = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("skills.install.get"))
            .unwrap();
        assert_eq!(get["annotations"]["readOnlyHint"], true);

        for name in ["room.bootstrap", "room.bootstrap.read"] {
            let tool = tools
                .iter()
                .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
                .unwrap_or_else(|| panic!("missing MCP tool {name}"));
            assert_eq!(tool["annotations"]["readOnlyHint"], true);
            assert_eq!(tool["annotations"]["destructiveHint"], false);
            assert_eq!(tool["annotations"]["openWorldHint"], false);
        }
    }

    #[tokio::test]
    async fn every_advertised_tool_is_accepted_by_apps_dispatcher() {
        let server = AgenticMcpServer::new(test_state());
        for tool in app_tool_descriptors(&server) {
            let name = tool["name"].as_str().unwrap();
            let result = call_app_tool(&server, json!({ "name": name, "arguments": {} })).await;
            if let Err(error) = result {
                assert!(
                    !error.starts_with("Unknown tool:"),
                    "advertised tool {name} is not accepted by tools/call: {error}"
                );
            }
        }
    }

    #[tokio::test]
    async fn apps_bootstrap_tools_are_callable_through_tools_call() {
        for (name, arguments) in [
            ("room.bootstrap", json!({})),
            ("room.bootstrap.read", json!({ "id": "missing" })),
        ] {
            let response = mcp_post(
                State(test_state()),
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": { "name": name, "arguments": arguments }
                })),
            )
            .await;
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert!(
                value.get("error").is_none(),
                "{name} was not dispatched: {value}"
            );
            assert_eq!(value["result"]["isError"], true);
            assert_eq!(
                value["result"]["structuredContent"]["error"]["code"],
                "room_not_active"
            );
        }
    }

    #[test]
    fn bootstrap_timeout_values_preserve_operation_specific_codes() {
        for (code, expected) in [
            ("room_bootstrap_timeout", "room_bootstrap_timeout"),
            ("room_bootstrap_read_timeout", "room_bootstrap_read_timeout"),
        ] {
            let value = room_route_error_value_with_timeout(
                RoomRouteError::Timeout("timed out".to_string()),
                code,
            );
            let result = serde_json::to_value(result_from_value(value)).unwrap();
            assert_eq!(result["isError"], true);
            assert_eq!(result["structuredContent"]["error"]["code"], expected);
        }
    }

    #[test]
    fn tmux_paste_schema_exposes_confirmation_default_field() {
        let schema =
            serde_json::to_string(&rmcp::schemars::schema_for!(TmuxPasteTextArgs)).unwrap();
        assert!(schema.contains("needConfirm"));
        assert!(schema.contains("submit"));
    }

    #[test]
    fn tmux_exec_schema_exposes_snapshot_fields() {
        let schema = serde_json::to_string(&rmcp::schemars::schema_for!(TmuxExecArgs)).unwrap();
        assert!(schema.contains("waitMs"));
        assert!(schema.contains("captureLines"));
    }

    #[test]
    fn room_notebook_mcp_input_schemas_do_not_include_agent_id() {
        let schemas = [
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookAppendArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookRecentArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookSelectExactArgs))
                .unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookSearchArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookCurrentArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookUpdateArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookRemoveArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomDiaryAppendArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomDiaryRecentArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomDiarySelectExactArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(BootstrapReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillSearchArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillActivationArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillInstallArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillInstallGetArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillInstallCancelArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(SkillRunArgs)).unwrap(),
        ];
        for schema in schemas {
            assert!(!schema.contains("agentId"));
            assert!(!schema.contains("agent_id"));
        }
    }

    #[test]
    fn native_tool_values_use_agentic_result_shape() {
        let value = json!({ "sessions": [] });

        let result = result_from_value(value.clone());
        let serialized = serde_json::to_value(result).unwrap();

        assert_eq!(serialized["structuredContent"], value);
        assert_eq!(serialized["isError"], false);
        assert_eq!(serialized["content"][0]["type"], "text");
    }

    #[test]
    fn mcp_tool_values_pass_through_mcp_result_envelope() {
        let value = json!({
            "content": [{ "type": "image", "data": "aW1hZ2U=", "mimeType": "image/png" }],
            "_meta": { "hidden": "widget-only" },
            "isError": false
        });

        let result = result_from_mcp_tool_value(value);
        let serialized = serde_json::to_value(result).unwrap();

        assert_eq!(serialized["content"][0]["type"], "image");
        assert_eq!(serialized["content"][0]["mimeType"], "image/png");
        assert_eq!(serialized["_meta"]["hidden"], "widget-only");
        assert!(serialized.get("structuredContent").is_none());
    }

    #[tokio::test]
    async fn mcp_tools_call_wire_response_uses_agentic_result_shape() {
        let response = mcp_post(
            State(test_state()),
            Json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "agent.list",
                    "arguments": {}
                }
            })),
        )
        .await;

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 1);
        assert_eq!(value["result"]["content"][0]["type"], "text");
        assert_eq!(value["result"]["structuredContent"]["agents"], json!([]));
        assert_eq!(value["result"]["isError"], false);
    }
}
