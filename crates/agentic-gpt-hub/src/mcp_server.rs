use agentic_gpt_protocol::{
    BatchExecRequest, ExecElement, ExecRequest, HubCommand, McpCallToolRequest,
    McpListToolsRequest, NotebookAppendRequest, NotebookCurrentRequest, NotebookRecentRequest,
    NotebookSearchRequest, NotebookSelectExactRequest, PassageSignificance, SessionInfo,
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

use crate::{
    cached_session, default_config_summary, random_id, registry_entries, registry_entry,
    request_active_room, request_agent, timeout_batch_result, timeout_task_result, HubState,
    RoomRouteError, MAX_WAIT_SECONDS, REQUEST_TIMEOUT_SECS,
};

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
        let open_world = matches!(name, "exec" | "batchExec" | "startSession" | "mcpCallTool");
        route.attr.annotations = Some(
            ToolAnnotations::new()
                .read_only(true)
                .destructive(false)
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
                "instructions": "Agentic GPT Hub tools. Commands are routed to registered local agents and remain subject to Agentic local policy, path policy, confirmation, and audit."
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
        "listAgents" => server.list_agents().await,
        "exec" => server.exec(Parameters(decode_args(arguments)?)).await,
        "batchExec" => server.batch_exec(Parameters(decode_args(arguments)?)).await,
        "startSession" => {
            server
                .start_session(Parameters(decode_args(arguments)?))
                .await
        }
        "listSessions" => {
            server
                .list_sessions(Parameters(decode_args(arguments)?))
                .await
        }
        "inspectSession" => {
            server
                .inspect_session(Parameters(decode_args(arguments)?))
                .await
        }
        "waitSession" => {
            server
                .wait_session(Parameters(decode_args(arguments)?))
                .await
        }
        "killSession" => {
            server
                .kill_session(Parameters(decode_args(arguments)?))
                .await
        }
        "mcpListServers" => {
            server
                .mcp_list_servers(Parameters(decode_args(arguments)?))
                .await
        }
        "mcpListTools" => {
            server
                .mcp_list_tools(Parameters(decode_args(arguments)?))
                .await
        }
        "mcpCallTool" => {
            server
                .mcp_call_tool(Parameters(decode_args(arguments)?))
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
            .with_instructions(
                "Agentic GPT Hub tools. Commands are routed to registered local agents and remain subject to Agentic local policy, path policy, confirmation, and audit.",
            )
    }
}

#[tool_router(router = tool_router)]
impl AgenticMcpServer {
    #[tool(
        name = "listAgents",
        description = "List registered Agentic local agents and online status."
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
        name = "exec",
        description = "Run a short command on a local Agentic agent. Long commands should use startSession."
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
        name = "batchExec",
        description = "Run multiple short commands on a local Agentic agent."
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
        name = "startSession",
        description = "Start a long-running command session on a local Agentic agent."
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
        name = "listSessions",
        description = "List running or recently cached sessions for a local Agentic agent."
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
        name = "inspectSession",
        description = "Inspect a session by id for a local Agentic agent."
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
        name = "waitSession",
        description = "Wait briefly for a session to update, then return current session state."
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
        name = "killSession",
        description = "Kill a running session by id for a local Agentic agent."
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
        name = "mcpListServers",
        description = "List MCP servers configured inside a local Agentic agent."
    )]
    async fn mcp_list_servers(
        &self,
        params: Parameters<AgentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let agent_id = params.0.agent_id;
        self.ensure_agent_enabled(&agent_id)?;
        let command = HubCommand::McpListServers {
            request_id: random_id("req"),
        };
        let value = request_agent(&self.state, &agent_id, command, REQUEST_TIMEOUT_SECS)
            .await
            .unwrap_or_else(|reason| json!({ "error": { "code": "mcp_list_servers_timeout", "message": reason } }));
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcpListTools",
        description = "List tools exposed by one MCP server configured inside a local Agentic agent."
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
        name = "mcpCallTool",
        description = "Call a tool on an MCP server configured inside a local Agentic agent. Local Agentic confirmation policy applies."
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
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.append",
        description = "Append an explicit passage to the generic room notebook."
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
        description = "Return recent passages from the generic room notebook."
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
        description = "Return passages for one exact room-timezone calendar day."
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
        description = "Search the generic room notebook with simple bounded JSONL scanning."
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
        description = "Return the current recoverable state for one generic room notebook scope."
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
    agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExecArgs {
    agent_id: String,
    program: String,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    need_confirm: Option<bool>,
    #[serde(default)]
    confirm_method: Option<String>,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BatchExecArgs {
    agent_id: String,
    elements: Vec<BatchExecElementArgs>,
    #[serde(default)]
    need_confirm: Option<bool>,
    #[serde(default)]
    confirm_method: Option<String>,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BatchExecElementArgs {
    program: String,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SessionIdArgs {
    agent_id: String,
    session_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct WaitSessionArgs {
    agent_id: String,
    session_id: String,
    #[serde(default)]
    seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpListToolsArgs {
    agent_id: String,
    server_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct McpCallToolArgs {
    agent_id: String,
    server_id: String,
    tool_name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookAppendArgs {
    #[serde(default)]
    datetime: Option<String>,
    scope: String,
    significance: String,
    #[serde(rename = "abstract")]
    abstract_text: String,
    content: String,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookRecentArgs {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    days: Option<u32>,
    #[serde(default)]
    significance: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookSelectExactArgs {
    year: i32,
    month: u32,
    day: u32,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookSearchArgs {
    query: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RoomNotebookCurrentArgs {
    scope: String,
}

fn ok_json(value: Value) -> CallToolResult {
    CallToolResult::structured(value)
}

fn result_from_value(value: Value) -> CallToolResult {
    if value.get("error").is_some() {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    }
}

fn room_route_error_value(error: RoomRouteError) -> Value {
    match error {
        RoomRouteError::NotActive => json!({
            "error": { "code": "room_not_active", "message": "no active room agent" }
        }),
        RoomRouteError::StateConflict => json!({
            "error": { "code": "room_state_conflict", "message": "active room state is inconsistent" }
        }),
        RoomRouteError::Timeout(reason) => json!({
            "error": { "code": "room_notebook_timeout", "message": reason }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_notebook_mcp_input_schemas_do_not_include_agent_id() {
        let schemas = [
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookAppendArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookRecentArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookSelectExactArgs))
                .unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookSearchArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookCurrentArgs)).unwrap(),
        ];
        for schema in schemas {
            assert!(!schema.contains("agentId"));
            assert!(!schema.contains("agent_id"));
        }
    }
}
