use std::sync::Arc;

use agentic_gpt_protocol::{BatchExecRequest, ExecRequest, HubCommand};
use anyhow::Result;
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ErrorData, ListToolsResult, Meta,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
    },
    service::{RequestContext, RoleServer},
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::{
    local_service,
    state::{AppState, CapabilityProfile},
};

const INSTRUCTIONS: &str = "Agentic GPT local Tunnel worker. Use process.exec for deterministic commands, session tools for managed processes, tmux for persistent workspaces, skills for the local skills workspace, and bootstrap for local startup guidance. All calls remain subject to local policy, path policy, confirmation, audit, and bounded waits.";

const NORMAL_TOOLS: &[&str] = &[
    "bootstrap",
    "bootstrap.read",
    "mcp.callTool",
    "mcp.listServers",
    "mcp.listTools",
    "process.batchExec",
    "process.exec",
    "session.inspect",
    "session.kill",
    "session.list",
    "session.start",
    "session.wait",
    "skills.active",
    "skills.activate",
    "skills.deactivate",
    "skills.install",
    "skills.install.cancel",
    "skills.install.get",
    "skills.list",
    "skills.read",
    "skills.run",
    "skills.search",
    "tmux.capturePane",
    "tmux.closeSession",
    "tmux.createSession",
    "tmux.exec",
    "tmux.listPanes",
    "tmux.listSessions",
    "tmux.pasteText",
];

const ROOM_ONLY_TOOLS: &[&str] = &[
    "room.diary.append",
    "room.diary.recent",
    "room.diary.selectExact",
    "room.notebook.append",
    "room.notebook.current",
    "room.notebook.recent",
    "room.notebook.remove",
    "room.notebook.search",
    "room.notebook.selectExact",
    "room.notebook.update",
];

pub(crate) async fn serve(state: AppState) -> Result<()> {
    let server = StdioMcpServer::new(state);
    let running = server.serve(stdio()).await?;
    let _ = running.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct StdioMcpServer {
    state: AppState,
    tools: Arc<Vec<Tool>>,
}

impl StdioMcpServer {
    fn new(state: AppState) -> Self {
        let profile = state.runtime.profile;
        let mut names = NORMAL_TOOLS.to_vec();
        if profile == CapabilityProfile::Room {
            names.extend_from_slice(ROOM_ONLY_TOOLS);
        }
        names.sort_unstable();
        let tools = names.into_iter().map(tool_descriptor).collect::<Vec<_>>();
        Self {
            state,
            tools: Arc::new(tools),
        }
    }

    async fn call(&self, request: CallToolRequestParams) -> Result<Value, ErrorData> {
        let name = request.name.to_string();
        if !self.tools.iter().any(|tool| tool.name == name) {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("Tool is not available: {name}"),
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let value = self
            .dispatch(&name, arguments)
            .await
            .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        Ok(value)
    }

    async fn dispatch(&self, name: &str, arguments: Value) -> Result<Value> {
        let request_id = request_id();
        match name {
            "process.exec" | "session.start" => {
                self.require_agent(&arguments).await?;
                let mut value = object(arguments)?;
                value.entry("args").or_insert_with(|| json!([]));
                value.entry("needConfirm").or_insert_with(|| json!(false));
                let payload: ExecRequest = from_object(value)?;
                let session_id = task_id("sess");
                let command = if name == "process.exec" {
                    HubCommand::Exec {
                        request_id,
                        task_id: task_id("task"),
                        payload,
                    }
                } else {
                    HubCommand::StartSession {
                        request_id,
                        session_id: session_id.clone(),
                        payload,
                    }
                };
                let result = local_service::dispatch(self.state.clone(), command).await?;
                if name == "session.start" {
                    Ok(session_start_result(session_id, result))
                } else {
                    Ok(result)
                }
            }
            "process.batchExec" => {
                self.require_agent(&arguments).await?;
                let mut value = object(arguments)?;
                value.entry("needConfirm").or_insert_with(|| json!(false));
                if let Some(elements) = value.get_mut("elements").and_then(Value::as_array_mut) {
                    for element in elements {
                        if let Some(element) = element.as_object_mut() {
                            element.entry("args").or_insert_with(|| json!([]));
                        }
                    }
                }
                let payload: BatchExecRequest = from_object(value)?;
                local_service::dispatch(
                    self.state.clone(),
                    HubCommand::BatchExec {
                        request_id,
                        task_id: task_id("batch"),
                        payload,
                    },
                )
                .await
                .map_err(Into::into)
            }
            "session.list" => {
                self.require_agent(&arguments).await?;
                let value = dispatch(self, HubCommand::ListSessions { request_id }).await?;
                Ok(json!({ "sessions": value }))
            }
            "session.inspect" => {
                self.require_agent(&arguments).await?;
                let value: SessionArgs = from_value(arguments)?;
                let value = dispatch(
                    self,
                    HubCommand::InspectSession {
                        request_id,
                        session_id: value.session_id,
                    },
                )
                .await?;
                Ok(session_or_not_found(value))
            }
            "session.wait" => {
                self.require_agent(&arguments).await?;
                let value: WaitArgs = from_value(arguments)?;
                let value = dispatch(
                    self,
                    HubCommand::WaitSession {
                        request_id,
                        session_id: value.session_id,
                        seconds: value.seconds.unwrap_or(0).min(30),
                    },
                )
                .await?;
                Ok(session_or_not_found(value))
            }
            "session.kill" => {
                self.require_agent(&arguments).await?;
                let value: SessionArgs = from_value(arguments)?;
                let value = dispatch(
                    self,
                    HubCommand::KillSession {
                        request_id,
                        session_id: value.session_id,
                    },
                )
                .await?;
                Ok(session_or_not_found(value))
            }
            "tmux.listSessions" => {
                self.require_agent(&arguments).await?;
                dispatch(self, HubCommand::TmuxListSessions { request_id }).await
            }
            "tmux.listPanes" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxListPanes {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.capturePane" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxCapturePane {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.pasteText" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxPasteText {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.exec" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxExec {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.createSession" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxCreateSession {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "tmux.closeSession" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::TmuxCloseSession {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "mcp.listServers" => {
                self.require_optional_agent(&arguments).await?;
                dispatch(self, HubCommand::McpListServers { request_id }).await
            }
            "mcp.listTools" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::McpListTools {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "mcp.callTool" => {
                self.require_agent(&arguments).await?;
                dispatch(
                    self,
                    HubCommand::McpCallTool {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "bootstrap" => dispatch(self, HubCommand::Bootstrap { request_id }).await,
            "bootstrap.read" => {
                dispatch(
                    self,
                    HubCommand::BootstrapRead {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.list" => dispatch(self, HubCommand::SkillsList { request_id }).await,
            "skills.read" => {
                dispatch(
                    self,
                    HubCommand::SkillsRead {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.search" => {
                dispatch(
                    self,
                    HubCommand::SkillsSearch {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.active" => dispatch(self, HubCommand::SkillsActive { request_id }).await,
            "skills.activate" => {
                dispatch(
                    self,
                    HubCommand::SkillsActivate {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.deactivate" => {
                dispatch(
                    self,
                    HubCommand::SkillsDeactivate {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.install" => {
                dispatch(
                    self,
                    HubCommand::SkillsInstall {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.install.get" => {
                dispatch(
                    self,
                    HubCommand::SkillsInstallGet {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.install.cancel" => {
                dispatch(
                    self,
                    HubCommand::SkillsInstallCancel {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "skills.run" => {
                dispatch(
                    self,
                    HubCommand::SkillsRun {
                        request_id,
                        session_id: task_id("sess"),
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.append" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookAppend {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.recent" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookRecent {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.selectExact" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookSelectExact {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.search" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookSearch {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.current" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookCurrent {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.update" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookUpdate {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.notebook.remove" => {
                dispatch(
                    self,
                    HubCommand::RoomNotebookRemove {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.diary.append" => {
                dispatch(
                    self,
                    HubCommand::RoomDiaryAppend {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.diary.recent" => {
                dispatch(
                    self,
                    HubCommand::RoomDiaryRecent {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            "room.diary.selectExact" => {
                dispatch(
                    self,
                    HubCommand::RoomDiarySelectExact {
                        request_id,
                        payload: from_value(arguments)?,
                    },
                )
                .await
            }
            _ => Err(anyhow::anyhow!("unknown stdio tool: {name}")),
        }
    }

    async fn require_agent(&self, arguments: &Value) -> Result<()> {
        let supplied = arguments
            .get("agentId")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("agentId is required"))?;
        let expected = self.state.config.read().await.agent_id.clone();
        if supplied != expected {
            return Err(anyhow::anyhow!("agentId does not identify this worker"));
        }
        Ok(())
    }

    async fn require_optional_agent(&self, arguments: &Value) -> Result<()> {
        if arguments.get("agentId").is_some() {
            self.require_agent(arguments).await?;
        }
        Ok(())
    }
}

impl ServerHandler for StdioMcpServer {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = self.call(request).await?;
        let is_error = value.get("error").is_some();
        let result = if is_error {
            CallToolResult::structured_error(value)
        } else {
            CallToolResult::structured(value)
        };
        Ok(result)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: (*self.tools).clone(),
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "agentic-gpt-stdio-worker",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionArgs {
    #[allow(dead_code)]
    agent_id: String,
    session_id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitArgs {
    #[allow(dead_code)]
    agent_id: String,
    session_id: String,
    seconds: Option<u64>,
}

async fn dispatch(server: &StdioMcpServer, command: HubCommand) -> Result<Value> {
    local_service::dispatch(server.state.clone(), command).await
}

fn session_start_result(session_id: String, value: Value) -> Value {
    let status = serde_json::from_value::<agentic_gpt_protocol::SessionInfo>(value.clone())
        .ok()
        .map(|session| {
            if matches!(
                session.state.as_str(),
                "running" | "waiting_confirmation" | "starting"
            ) {
                "started"
            } else {
                "failed"
            }
        })
        .unwrap_or("failed");
    json!({
        "status": status,
        "sessionId": session_id,
        "session": value
    })
}

fn session_or_not_found(value: Value) -> Value {
    if value.is_null() {
        json!({
            "error": {
                "code": "session_not_found",
                "message": "Session was not found"
            }
        })
    } else {
        value
    }
}

fn from_value<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(Into::into)
}

fn from_object<T: DeserializeOwned>(value: Map<String, Value>) -> Result<T> {
    from_value(Value::Object(value))
}

fn object(value: Value) -> Result<Map<String, Value>> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("tool arguments must be an object"))
}

fn request_id() -> String {
    task_id("req")
}

fn task_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn tool_descriptor(name: &str) -> Tool {
    let (properties, required) = tool_schema(name);
    let annotations = ToolAnnotations::new()
        .read_only(tool_is_read_only(name))
        .destructive(tool_is_destructive(name))
        .open_world(tool_is_open_world(name));
    Tool::new(
        name.to_string(),
        tool_description(name),
        schema(properties, required),
    )
    .with_annotations(annotations)
    .with_raw_output_schema(Arc::new(output_schema()))
    .with_meta(Meta(Map::from_iter([(
        "transport".to_string(),
        Value::String("stdio".to_string()),
    )])))
}

fn tool_schema(name: &str) -> (Map<String, Value>, &'static [&'static str]) {
    let required: &'static [&'static str] = match name {
        "process.exec" | "session.start" => &["agentId", "program"],
        "process.batchExec" => &["agentId", "elements"],
        "session.list" | "tmux.listSessions" => &["agentId"],
        "session.inspect" | "session.kill" => &["agentId", "sessionId"],
        "session.wait" => &["agentId", "sessionId"],
        "tmux.listPanes" => &["agentId"],
        "tmux.capturePane" => &["agentId", "target"],
        "tmux.pasteText" => &["agentId", "target", "text"],
        "tmux.exec" => &["agentId", "target", "program"],
        "tmux.createSession" => &["agentId", "name", "cwd"],
        "tmux.closeSession" => &["agentId", "name"],
        "mcp.listTools" => &["agentId", "serverId"],
        "mcp.callTool" => &["agentId", "serverId", "toolName"],
        "bootstrap.read" => &["id"],
        "skills.read" => &["id"],
        "skills.search" => &["query"],
        "skills.activate" | "skills.deactivate" => &["id"],
        "skills.install" => &["id", "source"],
        "skills.install.get" | "skills.install.cancel" => &["installId"],
        "skills.run" => &["id", "path"],
        "room.notebook.append" => &["scope", "significance", "abstract", "content"],
        "room.notebook.selectExact" => &["year", "month", "day"],
        "room.notebook.search" => &["query"],
        "room.notebook.current" => &["scope"],
        "room.notebook.update" | "room.notebook.remove" => &["id"],
        "room.diary.append" => &["entry"],
        "room.diary.selectExact" => &["year", "month", "day"],
        _ => &[],
    };
    (properties_for(name), required)
}

fn properties_for(name: &str) -> Map<String, Value> {
    let mut properties = Map::new();
    let mut add = |key: &str, value: Value| {
        properties.insert(key.to_string(), value);
    };
    let string = |description: &str| json!({"type": "string", "description": description});
    let number = |description: &str| json!({"type": "integer", "description": description});
    let boolean = |description: &str| json!({"type": "boolean", "description": description});
    let strings = |description: &str| json!({"type": "array", "items": {"type": "string"}, "description": description});
    if matches!(
        name,
        "process.exec"
            | "process.batchExec"
            | "session.start"
            | "session.list"
            | "session.inspect"
            | "session.wait"
            | "session.kill"
            | "tmux.listSessions"
            | "tmux.listPanes"
            | "tmux.capturePane"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.listTools"
            | "mcp.callTool"
    ) {
        add("agentId", string("Target local agent id."));
    }
    match name {
        "process.exec" | "session.start" => {
            add("program", string("Executable name or path."));
            add("args", strings("Direct argument vector."));
            add(
                "needConfirm",
                boolean("Request confirmation before execution."),
            );
            add(
                "confirmMethod",
                string("Optional confirmation provider override."),
            );
            add("workingDirectory", string("Process working directory."));
        }
        "process.batchExec" => {
            add(
                "elements",
                json!({
                    "type": "array",
                    "items": {"type": "object", "properties": {
                        "program": string("Executable name or path."),
                        "args": strings("Direct argument vector."),
                        "workingDirectory": string("Per-element working directory.")
                    }, "required": ["program"]}
                }),
            );
            add(
                "needConfirm",
                boolean("Request confirmation for the batch."),
            );
            add(
                "confirmMethod",
                string("Optional confirmation provider override."),
            );
            add(
                "workingDirectory",
                string("Default process working directory."),
            );
        }
        "session.inspect" | "session.kill" => {
            add("sessionId", string("Managed session id."));
        }
        "session.wait" => {
            add("sessionId", string("Managed session id."));
            add("seconds", number("Bounded wait in seconds, capped at 30."));
        }
        "tmux.listPanes" => add("session", string("Optional tmux session name.")),
        "tmux.capturePane" => {
            add("target", string("tmux pane target."));
            add("lines", number("History lines, default 160."));
        }
        "tmux.pasteText" => {
            add("target", string("tmux pane target."));
            add("text", string("Text to paste."));
            add("submit", boolean("Append Enter after pasting."));
            add(
                "needConfirm",
                boolean("Request confirmation before writing."),
            );
        }
        "tmux.exec" => {
            add("target", string("Shell pane target."));
            add("program", string("Program or shell builtin."));
            add("args", strings("Structured argument vector."));
            add("needConfirm", boolean("Request confirmation."));
            add("waitMs", number("Post-submit wait in milliseconds."));
            add("captureLines", number("Post-submit history lines."));
        }
        "tmux.createSession" => {
            add("name", string("tmux session name."));
            add("cwd", string("Session working directory."));
        }
        "tmux.closeSession" => {
            add("name", string("tmux session name."));
            add(
                "needConfirm",
                boolean("Request confirmation before closing."),
            );
        }
        "mcp.listServers" => add("agentId", string("Optional local agent id.")),
        "mcp.listTools" => add("serverId", string("Configured MCP server id.")),
        "mcp.callTool" => {
            add("serverId", string("Configured MCP server id."));
            add("toolName", string("Downstream MCP tool name."));
            add(
                "arguments",
                json!({"type": "object", "description": "Downstream tool arguments."}),
            );
        }
        "bootstrap.read" => add("id", string("Guide id returned by bootstrap.")),
        "skills.read" => {
            add("id", string("Skill id."));
            add("path", string("Optional package-relative resource path."));
        }
        "skills.search" => {
            add("query", string("Case-insensitive search query."));
            add("limit", number("Maximum skills returned."));
        }
        "skills.activate" | "skills.deactivate" => add("id", string("Skill id.")),
        "skills.install" => {
            add("id", string("Target skill id."));
            add(
                "source",
                json!({"type": "object", "description": "GitHub, HTTPS-file, or inline source descriptor."}),
            );
            add(
                "replaceExisting",
                boolean("Archive an existing skill before replacement."),
            );
            add(
                "activateAfterInstall",
                boolean("Optional activation choice."),
            );
            add("idempotencyKey", string("Optional retry key."));
        }
        "skills.install.get" => {
            add("installId", string("Installation job id."));
            add("waitSeconds", number("Bounded status wait, capped at 30."));
        }
        "skills.install.cancel" => add("installId", string("Installation job id.")),
        "skills.run" => {
            add("id", string("Skill id."));
            add("path", string("Package-relative executable path."));
            add("args", strings("Script argument vector."));
            add("workingDirectory", string("Optional working directory."));
            add("waitSeconds", number("Bounded inline wait, capped at 30."));
        }
        "room.notebook.append" => {
            add("datetime", string("Optional ISO-8601 timestamp."));
            add("scope", string("Path-safe notebook namespace."));
            add(
                "significance",
                json!({"type": "string", "enum": ["NORMAL", "ANCHOR"]}),
            );
            add("abstract", string("Short summary."));
            add("content", string("Full passage content."));
            add("tags", strings("Optional labels."));
        }
        "room.notebook.recent" => {
            add("scope", string("Optional scope filter."));
            add("days", number("Calendar days to scan."));
            add(
                "significance",
                json!({"type": "string", "enum": ["NORMAL", "ANCHOR"]}),
            );
            add("limit", number("Maximum passages."));
        }
        "room.notebook.selectExact" => {
            add("year", number("Calendar year."));
            add("month", number("Calendar month."));
            add("day", number("Calendar day."));
            add("scope", string("Optional scope filter."));
            add("limit", number("Maximum passages."));
        }
        "room.notebook.search" => {
            add("query", string("Search query."));
            add("scope", string("Optional scope filter."));
            add("limit", number("Maximum passages."));
        }
        "room.notebook.current" => add("scope", string("Notebook scope.")),
        "room.notebook.update" => {
            add("id", string("Passage id."));
            add(
                "significance",
                json!({"type": "string", "enum": ["NORMAL", "ANCHOR"]}),
            );
            add("abstract", string("Optional replacement summary."));
            add("content", string("Optional replacement content."));
            add("tags", strings("Optional replacement labels."));
        }
        "room.notebook.remove" => add("id", string("Passage id.")),
        "room.diary.append" => {
            add("timeHint", string("Optional daypart label."));
            add("tags", strings("Optional labels."));
            add("entry", string("Diary entry text."));
        }
        "room.diary.recent" => {
            add("days", number("Logical diary days to scan."));
            add("limit", number("Maximum entries."));
        }
        "room.diary.selectExact" => {
            add("year", number("Diary year."));
            add("month", number("Diary month."));
            add("day", number("Diary day."));
            add("limit", number("Maximum entries."));
        }
        _ => {}
    }
    properties
}

fn schema(properties: Map<String, Value>, required: &[&str]) -> Map<String, Value> {
    let mut result = Map::new();
    result.insert("type".to_string(), Value::String("object".to_string()));
    result.insert("properties".to_string(), Value::Object(properties));
    result.insert(
        "required".to_string(),
        Value::Array(
            required
                .iter()
                .map(|value| Value::String((*value).to_string()))
                .collect(),
        ),
    );
    result.insert("additionalProperties".to_string(), Value::Bool(false));
    result
}

fn output_schema() -> Map<String, Value> {
    Map::from_iter([
        ("type".to_string(), Value::String("object".to_string())),
        ("additionalProperties".to_string(), Value::Bool(true)),
    ])
}

fn tool_description(name: &str) -> String {
    match name {
        "process.exec" => "Run one short deterministic local process.".to_string(),
        "process.batchExec" => "Run multiple short local processes.".to_string(),
        "session.start" => "Start a managed local process session.".to_string(),
        "session.list" => "List local managed sessions.".to_string(),
        "session.inspect" => "Inspect a managed session.".to_string(),
        "session.wait" => "Wait for a managed session with a bounded timeout.".to_string(),
        "session.kill" => "Kill a managed session.".to_string(),
        "tmux.listSessions" => "List persistent tmux sessions.".to_string(),
        "tmux.listPanes" => "List tmux panes.".to_string(),
        "tmux.capturePane" => "Capture bounded tmux pane history.".to_string(),
        "tmux.pasteText" => "Paste text into a tmux pane.".to_string(),
        "tmux.exec" => "Submit a command to a tmux shell pane.".to_string(),
        "tmux.createSession" => "Create a tmux session.".to_string(),
        "tmux.closeSession" => "Close a tmux session.".to_string(),
        "mcp.listServers" => "List configured downstream MCP servers.".to_string(),
        "mcp.listTools" => "List tools exposed by a downstream MCP server.".to_string(),
        "mcp.callTool" => "Call a downstream MCP tool.".to_string(),
        "bootstrap" => "Load the local bootstrap manifest.".to_string(),
        "bootstrap.read" => "Read one local bootstrap guide.".to_string(),
        "skills.list" => "List local skills.".to_string(),
        "skills.read" => "Read a local skill or resource.".to_string(),
        "skills.search" => "Search local skills.".to_string(),
        "skills.active" => "List active local skills.".to_string(),
        "skills.activate" => "Activate a local skill.".to_string(),
        "skills.deactivate" => "Deactivate a local skill.".to_string(),
        "skills.install" => "Start a local skill installation.".to_string(),
        "skills.install.get" => "Get local skill installation status.".to_string(),
        "skills.install.cancel" => "Cancel a local skill installation.".to_string(),
        "skills.run" => "Run an executable from a local skill.".to_string(),
        "room.notebook.append" => "Append a Room notebook passage.".to_string(),
        "room.notebook.recent" => "Read recent Room notebook passages.".to_string(),
        "room.notebook.selectExact" => "Read Room notebook passages for a date.".to_string(),
        "room.notebook.search" => "Search Room notebook passages.".to_string(),
        "room.notebook.current" => "Read current Room notebook state.".to_string(),
        "room.notebook.update" => "Update a Room notebook passage.".to_string(),
        "room.notebook.remove" => "Remove a Room notebook passage.".to_string(),
        "room.diary.append" => "Append a Room diary entry.".to_string(),
        "room.diary.recent" => "Read recent Room diary entries.".to_string(),
        "room.diary.selectExact" => "Read Room diary entries for a date.".to_string(),
        _ => "Agentic GPT local tool.".to_string(),
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

fn tool_is_destructive(name: &str) -> bool {
    matches!(
        name,
        "session.kill"
            | "tmux.closeSession"
            | "room.notebook.remove"
            | "skills.install"
            | "skills.install.cancel"
            | "skills.run"
    )
}

fn tool_is_open_world(name: &str) -> bool {
    matches!(
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
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use rmcp::{model::CallToolRequestParams, ServiceExt};
    use tokio::io::split;
    use tokio::sync::{Mutex, RwLock};

    use super::*;
    use crate::{
        config::Config, sessions::SkillLeaseManager, skill_installs::InstallManager,
        state::RuntimeModel,
    };

    #[test]
    fn normal_and_room_tool_sets_are_exact() {
        let normal = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let room = StdioMcpServer::new(test_state(CapabilityProfile::Room));
        let normal_names = normal
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        let room_names = room
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        assert_eq!(normal_names.len(), 29);
        assert_eq!(room_names.len(), 39);
        assert!(!normal_names.iter().any(|name| name.starts_with("room.")));
        assert!(room_names.iter().any(|name| name == "room.diary.append"));
        assert!(room_names.iter().any(|name| name == "room.notebook.remove"));
        assert!(!normal_names
            .iter()
            .any(|name| name == "user.notify.deliver"));
        assert_eq!(normal_names, {
            let mut expected = NORMAL_TOOLS
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>();
            expected.sort();
            expected
        });
    }

    #[tokio::test]
    async fn absent_room_tool_is_rejected_for_normal_worker() {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let error = server
            .call(CallToolRequestParams::new("room.diary.recent"))
            .await
            .expect_err("Room-only tool must not be callable by Normal worker");
        assert_eq!(error.code, rmcp::model::ErrorCode::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn in_process_stdio_initialize_list_and_call() -> anyhow::Result<()> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = split(client_io);
        let (server_read, server_write) = split(server_io);
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Normal));
        let server_task = tokio::spawn(async move {
            let running = server.serve((server_read, server_write)).await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });

        let client = ().serve((client_read, client_write)).await?;
        let tools = client.list_all_tools().await?;
        assert_eq!(tools.len(), 29);
        let result = client
            .call_tool(
                CallToolRequestParams::new("session.list").with_arguments(Map::from_iter([(
                    "agentId".to_string(),
                    Value::String("stdio-test-agent".to_string()),
                )])),
            )
            .await?;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            result.structured_content.as_ref().unwrap()["sessions"],
            json!([])
        );
        let skills = client
            .call_tool(CallToolRequestParams::new("skills.list"))
            .await?;
        assert_eq!(skills.is_error, Some(false));
        let bootstrap = client
            .call_tool(CallToolRequestParams::new("bootstrap"))
            .await?;
        assert_eq!(bootstrap.is_error, Some(false));
        let _ = client.cancel().await;
        server_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn in_process_room_stdio_initialize_list_and_call() -> anyhow::Result<()> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = split(client_io);
        let (server_read, server_write) = split(server_io);
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Room));
        let server_task = tokio::spawn(async move {
            let running = server.serve((server_read, server_write)).await?;
            let _ = running.waiting().await?;
            anyhow::Result::<()>::Ok(())
        });

        let client = ().serve((client_read, client_write)).await?;
        let tools = client.list_all_tools().await?;
        assert_eq!(tools.len(), 39);
        let result = client
            .call_tool(CallToolRequestParams::new("room.diary.recent"))
            .await?;
        assert_eq!(result.is_error, Some(false));
        assert!(result.structured_content.as_ref().unwrap()["entries"].is_array());
        let _ = client.cancel().await;
        server_task.await??;
        Ok(())
    }

    #[tokio::test]
    async fn room_profile_dispatches_room_memory_tools() -> anyhow::Result<()> {
        let server = StdioMcpServer::new(test_state(CapabilityProfile::Room));
        let bootstrap = server.dispatch("bootstrap", json!({})).await?;
        assert!(bootstrap.get("entrypoint").is_some());
        let skills = server.dispatch("skills.list", json!({})).await?;
        assert!(skills["skills"].is_array());
        let notebook = server.dispatch("room.notebook.recent", json!({})).await?;
        assert!(notebook["passages"].is_array());
        let diary = server.dispatch("room.diary.recent", json!({})).await?;
        assert!(diary["entries"].is_array());
        Ok(())
    }

    fn test_state(profile: CapabilityProfile) -> AppState {
        let root = std::env::temp_dir().join(format!("agentic-stdio-{}", Uuid::new_v4()));
        let workspace_root = root.join("workspace");
        let mut config = Config::default_config().expect("default config");
        config.agent_id = "stdio-test-agent".to_string();
        config.workspace_root = workspace_root.clone();
        config.path_policy.write_roots = vec![workspace_root.clone()];
        config.ensure_workspace().expect("workspace");
        let bootstrap_root = workspace_root.join("bootstrap");
        std::fs::create_dir_all(&bootstrap_root).expect("bootstrap directory");
        std::fs::write(
            bootstrap_root.join("bootstrap.md"),
            "---\nid: room\nkind: entrypoint\nname: Room Bootstrap\ndescription: Test bootstrap\nschemaVersion: 1\n---\n",
        )
        .expect("bootstrap entrypoint");
        AppState {
            config_path: PathBuf::from("stdio-test-config.json"),
            config: Arc::new(RwLock::new(config)),
            runtime: RuntimeModel::tunnel(profile, false),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(SkillLeaseManager::new()),
            skill_installs: Arc::new(InstallManager::new()),
        }
    }
}
