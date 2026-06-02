use agentic_gpt_protocol::{
    BatchExecRequest, ExecElement, ExecRequest, HubCommand, McpCallToolRequest,
    McpListToolsRequest, SessionInfo,
};
use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ErrorData, Meta, ServerCapabilities, ServerInfo, ToolAnnotations,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, tower::StreamableHttpService,
};
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::{
    cached_session, default_config_summary, random_id, registry_entries, registry_entry,
    request_agent, timeout_batch_result, timeout_task_result, HubState, MAX_WAIT_SECONDS,
    REQUEST_TIMEOUT_SECS,
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

pub(crate) type AgenticMcpService = StreamableHttpService<AgenticMcpServer, LocalSessionManager>;

pub(crate) fn service(state: HubState) -> AgenticMcpService {
    StreamableHttpService::new(
        move || Ok(AgenticMcpServer::new(state.clone())),
        Default::default(),
        StreamableHttpServerConfig::default()
            .with_stateful_mode(false)
            .with_json_response(true)
            .with_sse_keep_alive(None),
    )
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
                })
                .collect(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
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
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
struct BatchExecElementArgs {
    program: String,
    #[serde(default)]
    args: Option<Vec<String>>,
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

fn mcp_invalid_params(code: &'static str, message: &'static str) -> ErrorData {
    ErrorData::invalid_params(message, Some(json!({ "code": code })))
}

fn mcp_internal_error(code: &'static str, message: String) -> ErrorData {
    ErrorData::internal_error(message, Some(json!({ "code": code })))
}
