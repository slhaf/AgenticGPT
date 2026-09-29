mod args;
pub(crate) mod transport;

use agentic_gpt_protocol::{
    normalize_process_group, BootstrapReadRequest, HubCommand, McpBatchCall, McpBatchRequest,
    McpCallToolRequest, McpListToolsRequest, ProcessBatchExecRequest, ProcessCancelRequest,
    ProcessExecElement, ProcessExecRequest, ProcessKind, ProcessListRequest, ProcessOutputRequest,
    ProcessResultRequest, ProcessState, ProcessStatusRequest, RoomDiaryActiveRequest,
    RoomDiaryReadRequest, RoomMaintenanceStatusRequest, RoomNotebookReadRequest,
    RoomNotebookRecentRequest, RoomNotebookSearchRequest, RoomStateListRequest,
    RoomStateReadRequest, SkillActivationRequest, SkillInstallCancelRequest,
    SkillInstallGetRequest, SkillInstallRequest, SkillReadRequest, SkillRunRequest,
    SkillSearchRequest, TmuxCapturePaneRequest, TmuxCloseSessionRequest, TmuxCreateSessionRequest,
    TmuxExecRequest, TmuxListPanesRequest, TmuxPasteTextRequest, UserNotifySendRequest,
};
use args::{
    AgentIdArgs, BootstrapReadArgs, HubRunGetArgs, HubRunListArgs, McpBatchArgs, McpCallToolArgs,
    McpListServersArgs, McpListToolsArgs, ProcessBatchArgs, ProcessExecArgs, ProcessIdArgs,
    ProcessListArgs, ProcessOutputArgs, ProcessResultArgs, ProcessStatusArgs, RoomDiaryActiveArgs,
    RoomDiaryReadArgs, RoomMaintenanceStatusArgs, RoomMaintenanceSubmitArgs, RoomNotebookReadArgs,
    RoomNotebookRecentArgs, RoomNotebookSearchArgs, RoomStateListArgs, RoomStateReadArgs,
    SkillActivationArgs, SkillInstallArgs, SkillInstallCancelArgs, SkillInstallGetArgs,
    SkillReadArgs, SkillRunArgs, SkillSearchArgs, TmuxCapturePaneArgs, TmuxCloseSessionArgs,
    TmuxCreateSessionArgs, TmuxExecArgs, TmuxListPanesArgs, TmuxListSessionsArgs,
    TmuxPasteTextArgs, UserNotifySendArgs,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ErrorData, Meta, ServerCapabilities, ServerInfo, ToolAnnotations,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde_json::{json, Map, Value};

use crate::agentic_result::AgenticResult;
use crate::agents::dispatch::{cached_process, mcp_list_servers_all_agents, request_agent};
use crate::notify::{notification_channels, send_user_notification, NotifyRouteError};
use crate::registry::{registry_entries, registry_entry};
use crate::room::control::{request_active_room, RoomRouteError};
use crate::runs;
use crate::state::{
    projection::{
        add_cache_metadata, build_hub_info_response, filter_cached_processes, live_process_value,
        process_list_item,
    },
    HubState, McpProfile,
};
use crate::utils::random_id;
use crate::REQUEST_TIMEOUT_SECS;
const ROOM_TRANSPORT_MARGIN_SECS: u64 = 5;

const MCP_INSTRUCTIONS: &str = "Agentic GPT Hub exposes process.exec and process.batch for managed execution, plus process.status, process.list, process.output, process.result, and process.cancel for lifecycle follow-up. Status and list report metadata only; output uses resumable lossless pages, and result reports whether the complete structured value is retained. hub.process.status and hub.process.list are cache-only snapshots with explicit freshness. mcp.callTool and mcp.batch use the same managed process lifecycle for downstream MCP calls. Use tmux as the persistent shared workspace for stateful development, iterative debugging, TUIs, and user-agent handoff. For tmux work, discover the active session and panes before issuing commands. skills.install remains a distinct installation workflow.";
const COORDINATOR_INSTRUCTIONS: &str = "Agentic GPT Hub coordinator profile. This connector exposes only Hub-native agent status, retained run history, current cached process snapshots, and notification tools. It never dispatches execution, process-control, tmux, downstream MCP, skills, bootstrap, diary, or notebook commands to an Agent.";

fn default_process_wait_seconds() -> u64 {
    ProcessStatusRequest::DEFAULT_WAIT_SECONDS
}

fn process_status_payload(params: &ProcessStatusArgs) -> ProcessStatusRequest {
    let mut payload = ProcessStatusRequest {
        process_id: params.process_id.clone(),
        wait_seconds: params.wait_seconds,
    };
    payload.wait_seconds = Some(payload.effective_wait_seconds());
    payload
}

fn default_standard_wait_seconds() -> u64 {
    5
}

fn default_room_wait_seconds() -> u8 {
    0
}

fn default_room_notebook_limit() -> usize {
    20
}

fn default_process_list_limit() -> usize {
    ProcessListRequest::DEFAULT_LIMIT
}

fn default_process_output_max_bytes() -> usize {
    ProcessOutputRequest::DEFAULT_MAX_BYTES
}

fn default_process_result_max_bytes() -> usize {
    ProcessResultRequest::DEFAULT_MAX_BYTES
}

const COORDINATOR_TOOLS: &[&str] = &[
    "hub.info",
    "agent.list",
    "hub.run.list",
    "hub.run.get",
    "hub.process.status",
    "hub.process.list",
    "user.notify.channels",
    "user.notify.send",
];

#[derive(Clone)]
pub(crate) struct AgenticMcpServer {
    state: HubState,
    profile: McpProfile,
    tool_router: ToolRouter<Self>,
}

impl AgenticMcpServer {
    pub(crate) fn new(state: HubState) -> Self {
        let mut tool_router = Self::tool_router();
        decorate_tool_descriptors(&mut tool_router);
        let profile = state.mcp_profile;
        Self {
            state,
            profile,
            tool_router,
        }
    }

    fn profile_allows_tool(&self, name: &str) -> bool {
        self.profile == McpProfile::Full || COORDINATOR_TOOLS.contains(&name)
    }

    fn generated_tool(&self, name: &str) -> bool {
        self.tool_router.get(name).is_some()
    }

    // Profile filtering narrows the generated router; annotations only describe callable tools.
    fn allows_tool(&self, name: &str) -> bool {
        self.profile_allows_tool(name) && self.generated_tool(name)
    }

    fn instructions(&self) -> &'static str {
        match self.profile {
            McpProfile::Full => MCP_INSTRUCTIONS,
            McpProfile::Coordinator => COORDINATOR_INSTRUCTIONS,
        }
    }
}

fn decorate_tool_descriptors(tool_router: &mut ToolRouter<AgenticMcpServer>) {
    for route in tool_router.map.values_mut() {
        let name = route.attr.name.as_ref();
        let open_world = matches!(
            name,
            "process.exec"
                | "process.batch"
                | "mcp.batch"
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
            "process.cancel"
                | "tmux.closeSession"
                | "room.maintenance.submit"
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
            | "process.batch"
            | "process.cancel"
            | "tmux.pasteText"
            | "tmux.exec"
            | "tmux.createSession"
            | "tmux.closeSession"
            | "mcp.batch"
            | "mcp.callTool"
            | "user.notify.send"
            | "room.maintenance.submit"
            | "skills.activate"
            | "skills.deactivate"
            | "skills.install"
            | "skills.install.cancel"
            | "skills.run"
    )
}

fn parse_process_kind(value: Option<&str>) -> Result<Option<ProcessKind>, ErrorData> {
    match value {
        None => Ok(None),
        Some("command") => Ok(Some(ProcessKind::Command)),
        Some("skill") => Ok(Some(ProcessKind::Skill)),
        Some("mcp") => Ok(Some(ProcessKind::Mcp)),
        Some(_) => Err(mcp_invalid_params(
            "process_kind_invalid",
            "kind must be command, skill, or mcp",
        )),
    }
}

fn parse_process_state(value: Option<&str>) -> Result<Option<ProcessState>, ErrorData> {
    let state = match value {
        None => return Ok(None),
        Some("queued") => ProcessState::Queued,
        Some("waiting_confirmation") => ProcessState::WaitingConfirmation,
        Some("starting") => ProcessState::Starting,
        Some("running") => ProcessState::Running,
        Some("completed") => ProcessState::Completed,
        Some("failed") => ProcessState::Failed,
        Some("rejected") => ProcessState::Rejected,
        Some("cancel_requested") => ProcessState::CancelRequested,
        Some("cancelled") => ProcessState::Cancelled,
        Some("timed_out") => ProcessState::TimedOut,
        Some("detached") => ProcessState::Detached,
        Some("unknown_after_restart") => ProcessState::UnknownAfterRestart,
        Some("skipped") => ProcessState::Skipped,
        Some(_) => {
            return Err(mcp_invalid_params(
                "process_state_invalid",
                "unknown process state",
            ))
        }
    };
    Ok(Some(state))
}

fn object_schema() -> Map<String, Value> {
    let mut schema = Map::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("additionalProperties".to_string(), Value::Bool(true));
    schema
}

async fn snapshot_process_list(state: &HubState, agent_id: &str) -> Value {
    let snapshots = state.process_cache.snapshots(agent_id).await;
    let mut value = json!({
        "processes": snapshots
            .iter()
            .cloned()
            .map(|snapshot| process_list_item(snapshot.process))
            .collect::<Vec<_>>()
    });
    add_cache_metadata(&mut value, &snapshots);
    value
}

async fn snapshot_process_list_filtered(
    state: &HubState,
    agent_id: &str,
    request: &ProcessListRequest,
    unavailable_reason: &str,
) -> Value {
    if request.cursor.is_some() {
        return json!({
            "status": "unavailable",
            "error": {
                "code": "process_list_cursor_unavailable",
                "message": format!(
                    "Agent is unavailable and Hub cache cannot continue an Agent-issued cursor: {unavailable_reason}"
                )
            },
            "freshness": "unknown"
        });
    }
    let mut snapshots = state.process_cache.snapshots(agent_id).await;
    filter_cached_processes(&mut snapshots, request);
    let mut value = json!({
        "processes": snapshots
            .iter()
            .cloned()
            .map(|snapshot| process_list_item(snapshot.process))
            .collect::<Vec<_>>()
    });
    add_cache_metadata(&mut value, &snapshots);
    value
}

async fn cached_process_summary(
    state: &HubState,
    agent_id: &str,
    process_id: &str,
) -> Option<Value> {
    cached_process(state, agent_id, process_id)
        .await
        .and_then(|snapshot| {
            let mut value =
                serde_json::to_value(process_list_item(snapshot.process.clone())).ok()?;
            add_cache_metadata(&mut value, std::slice::from_ref(&snapshot));
            Some(value)
        })
}

async fn snapshot_process_status(state: &HubState, agent_id: &str, process_id: &str) -> Value {
    match cached_process(state, agent_id, process_id).await {
        Some(snapshot) => {
            let mut value = serde_json::to_value(snapshot.process.clone())
                .expect("ProcessInfo is serializable");
            add_cache_metadata(&mut value, std::slice::from_ref(&snapshot));
            value
        }
        None => {
            json!({ "error": { "code": "process_not_found", "message": "Process was not found" }, "freshness": "unknown" })
        }
    }
}

async fn unavailable_process_value(
    state: &HubState,
    agent_id: &str,
    process_id: &str,
    code: &'static str,
    reason: String,
) -> Value {
    let mut value = json!({
        "processId": process_id,
        "status": "unavailable",
        "error": { "code": code, "message": reason },
        "freshness": "unknown"
    });
    if let Some(cached) = cached_process_summary(state, agent_id, process_id).await {
        if let Some(freshness) = cached.get("freshness").cloned() {
            value["freshness"] = freshness;
        }
        if let Some(observed_at) = cached.get("observedAt").cloned() {
            value["observedAt"] = observed_at;
        }
        value["cached"] = cached;
    }
    value
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AgenticMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "agentic-gpt-hub",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.instructions())
    }
}

#[tool_router(router = tool_router)]
impl AgenticMcpServer {
    #[tool(
        name = "hub.info",
        description = "Inspect Hub runtime health and bounded capacity summaries; read-only."
    )]
    async fn hub_info(&self) -> Result<CallToolResult, ErrorData> {
        let info = build_hub_info_response(&self.state)
            .await
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(serde_json::to_value(info).map_err(|error| {
            mcp_internal_error("serialization_error", error.to_string())
        })?))
    }

    #[tool(
        name = "agent.list",
        description = "List registered local agents and availability; read-only discovery."
    )]
    async fn list_agents(&self) -> Result<CallToolResult, ErrorData> {
        let entries = registry_entries(&self.state)
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        let agents = self
            .state
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
                    "connectionMode": entry.connection_mode.map(|mode| mode.label()),
                    "lastSeenAt": entry.last_seen_at,
                    "capabilities": entry.capabilities,
                    "configSummary": entry.config_summary,
                })
            })
            .collect::<Vec<_>>();
        Ok(ok_json(json!({ "agents": agents })))
    }

    #[tool(
        name = "hub.run.get",
        description = "Read one retained Hub command run by id."
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
        name = "hub.run.list",
        description = "List retained Hub and Agent-originated run records; read-only."
    )]
    async fn hub_run_list(
        &self,
        params: Parameters<HubRunListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let limit = params.limit.unwrap_or(20).clamp(1, 100);
        let since = params
            .since_seconds
            .map(|seconds| chrono::Utc::now() - chrono::Duration::seconds(seconds as i64));
        let runs = runs::list_runs(
            &self.state,
            params.agent_id.as_deref(),
            params.source.as_deref(),
            params.status.as_deref(),
            since,
            limit,
        )
        .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(json!({ "runs": runs, "limit": limit })))
    }

    #[tool(
        name = "hub.process.list",
        description = "List cached process status metadata for one local Agent without dispatching; freshness and observation time are explicit."
    )]
    async fn hub_process_list(
        &self,
        params: Parameters<AgentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let agent_id = params.0.agent_id;
        self.ensure_agent_enabled(&agent_id)?;
        Ok(ok_json(snapshot_process_list(&self.state, &agent_id).await))
    }

    #[tool(
        name = "hub.process.status",
        description = "Read one cached process status snapshot without dispatching; freshness and observation time are explicit."
    )]
    async fn hub_process_status(
        &self,
        params: Parameters<ProcessIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        Ok(result_from_value(
            snapshot_process_status(&self.state, &params.agent_id, &params.process_id).await,
        ))
    }

    #[tool(
        name = "process.exec",
        description = "Start one managed process on a local Agent; use process.status, process.output, process.result, and process.cancel for lifecycle follow-up."
    )]
    async fn exec(&self, params: Parameters<ProcessExecArgs>) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = ProcessExecRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            program: params.program,
            args: params.args.unwrap_or_default(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            working_directory: params.working_directory,
            wait_seconds: params.wait_seconds,
        };
        let command = HubCommand::Exec {
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
            |reason| json!({ "error": { "code": "process_exec_timeout", "message": reason } }),
        );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.batch",
        description = "Start multiple managed processes under one admission boundary; started side effects are not rolled back."
    )]
    async fn batch_exec(
        &self,
        params: Parameters<ProcessBatchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = ProcessBatchExecRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            elements: params
                .elements
                .into_iter()
                .map(|element| ProcessExecElement {
                    program: element.program,
                    args: element.args.unwrap_or_default(),
                    working_directory: element.working_directory,
                })
                .collect(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            working_directory: params.working_directory,
            wait_seconds: params.wait_seconds,
        };
        let command = HubCommand::ProcessBatch {
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
            |reason| json!({ "error": { "code": "process_batch_timeout", "message": reason } }),
        );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.list",
        description = "List active or retained managed processes for one local Agent with optional filters and cursor pagination; offline cache snapshots carry explicit freshness."
    )]
    async fn process_list(
        &self,
        params: Parameters<ProcessListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let group = normalize_process_group(params.group.as_deref())
            .map_err(|error| mcp_invalid_params(error.code(), error.message()))?;
        let payload = ProcessListRequest {
            group,
            kind: parse_process_kind(params.kind.as_deref())?,
            state: parse_process_state(params.state.as_deref())?,
            limit: params.limit,
            cursor: params.cursor,
        };
        let command = HubCommand::ProcessList {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 2).await {
            Ok(value) => live_process_value(value),
            Err(reason) => {
                snapshot_process_list_filtered(&self.state, &params.agent_id, &payload, &reason)
                    .await
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.status",
        description = "Inspect or briefly wait for process status metadata only; status never includes stdout, stderr, or result bodies."
    )]
    async fn process_status(
        &self,
        params: Parameters<ProcessStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = process_status_payload(&params);
        let wait_seconds = payload.effective_wait_seconds();
        let command = HubCommand::ProcessStatus {
            request_id: random_id("req"),
            payload,
        };
        let value =
            match request_agent(&self.state, &params.agent_id, command, wait_seconds + 2).await {
                Ok(value) => live_process_value(value),
                Err(reason) => {
                    let mut value = unavailable_process_value(
                        &self.state,
                        &params.agent_id,
                        &params.process_id,
                        "process_status_unavailable",
                        reason,
                    )
                    .await;
                    if let Some(object) = value.as_object_mut() {
                        object.remove("status");
                    }
                    value
                }
            };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.output",
        description = "Read a non-consuming lossless output page. Cursor offsets are raw bytes, and invalid UTF-8 is preserved with base64 encoding."
    )]
    async fn process_output(
        &self,
        params: Parameters<ProcessOutputArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::ProcessOutput {
            request_id: random_id("req"),
            payload: ProcessOutputRequest {
                process_id: params.process_id.clone(),
                cursor: params.cursor,
                max_bytes: params.max_bytes,
            },
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 5).await {
            Ok(value) => value,
            Err(reason) => {
                unavailable_process_value(
                    &self.state,
                    &params.agent_id,
                    &params.process_id,
                    "process_output_unavailable",
                    reason,
                )
                .await
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.result",
        description = "Read an explicitly retained complete structured result or an unavailable status; the Hub metadata cache never supplies result content."
    )]
    async fn process_result(
        &self,
        params: Parameters<ProcessResultArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::ProcessResult {
            request_id: random_id("req"),
            payload: ProcessResultRequest {
                process_id: params.process_id.clone(),
                max_bytes: params.max_bytes,
            },
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 5).await {
            Ok(value) => value,
            Err(reason) => {
                unavailable_process_value(
                    &self.state,
                    &params.agent_id,
                    &params.process_id,
                    "process_result_unavailable",
                    reason,
                )
                .await
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.cancel",
        description = "Request managed process cancellation and return observed termination evidence; a remote stop is not assumed."
    )]
    async fn process_cancel(
        &self,
        params: Parameters<ProcessIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let command = HubCommand::ProcessCancel {
            request_id: random_id("req"),
            payload: ProcessCancelRequest {
                process_id: params.process_id.clone(),
            },
        };
        let value = match request_agent(&self.state, &params.agent_id, command, 5).await {
            Ok(value) => live_process_value(value),
            Err(reason) => {
                unavailable_process_value(
                    &self.state,
                    &params.agent_id,
                    &params.process_id,
                    "process_cancel_unavailable",
                    reason,
                )
                .await
            }
        };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "tmux.listSessions",
        description = "List persistent tmux sessions on a local Agent; read-only."
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
        description = "List tmux panes and shell/TUI hints; read-only."
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
        description = "Capture bounded tmux pane history; use it to observe tmux.exec progress."
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
        description = "Paste text into a non-shell tmux pane or TUI; shell panes are rejected."
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
        description = "Submit one structured command to a tmux shell pane; returned snapshot is not proof of completion."
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
        description = "Create or reuse one persistent tmux session in an allowed working directory."
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
        description = "Close one persistent tmux session; destructive and confirmation-aware."
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
        description = "List downstream MCP servers, optionally scoped to one Agent; read-only discovery."
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
        description = "List tools exposed by one downstream MCP server; read-only discovery."
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
        description = "Run one downstream MCP tool as a managed process; use process tools for lifecycle follow-up."
    )]
    async fn mcp_call_tool(
        &self,
        params: Parameters<McpCallToolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = McpCallToolRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            server_id: params.server_id,
            tool_name: params.tool_name,
            arguments: params.arguments.unwrap_or_else(|| json!({})),
            wait_seconds: params.wait_seconds,
            timeout_seconds: params.timeout_seconds,
        };
        let command = HubCommand::McpCallTool {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let request_timeout = payload.effective_wait_seconds() + 2;
        let value = request_agent(&self.state, &payload.agent_id, command, request_timeout)
            .await
            .unwrap_or_else(
                |reason| json!({ "error": { "code": "mcp_call_tool_timeout", "message": reason } }),
            );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "mcp.batch",
        description = "Run multiple downstream MCP calls as managed processes under one admission boundary; downstream side effects are not rolled back."
    )]
    async fn mcp_batch(
        &self,
        params: Parameters<McpBatchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = McpBatchRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            calls: params
                .calls
                .into_iter()
                .map(|call| McpBatchCall {
                    id: None,
                    server_id: call.server_id,
                    tool_name: call.tool_name,
                    arguments: call.arguments.unwrap_or_else(|| json!({})),
                })
                .collect(),
            mode: params.mode.unwrap_or_default().into(),
            fail_fast: params.fail_fast.unwrap_or(false),
            wait_seconds: params.wait_seconds,
            timeout_seconds: params.timeout_seconds,
        };
        let command = HubCommand::McpBatch {
            request_id: random_id("req"),
            payload: payload.clone(),
        };
        let request_timeout = payload.effective_wait_seconds() + 2;
        let value = request_agent(&self.state, &payload.agent_id, command, request_timeout)
            .await
            .unwrap_or_else(
                |reason| json!({ "error": { "code": "mcp_batch_timeout", "message": reason } }),
            );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "user.notify.channels",
        description = "List Hub-native notification channels; read-only."
    )]
    async fn user_notify_channels(&self) -> Result<CallToolResult, ErrorData> {
        let channels = notification_channels(&self.state)
            .await
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(json!({ "channels": channels })))
    }

    #[tool(
        name = "user.notify.send",
        description = "Send one Hub-native user notification."
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
        name = "room.diary.active",
        description = "Read the active daily, weekly, and monthly Room diary documents; read-only, semantic, and bounded."
    )]
    async fn room_diary_active(
        &self,
        _params: Parameters<RoomDiaryActiveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomDiaryActive {
                request_id: random_id("req"),
                payload: RoomDiaryActiveRequest {},
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_diary_active_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.diary.read",
        description = "Read one exact Room diary document by validated semantic layer and period; read-only, semantic, and bounded."
    )]
    async fn room_diary_read(
        &self,
        params: Parameters<RoomDiaryReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = RoomDiaryReadRequest {
            layer: params.layer.into(),
            period: params.period,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomDiaryRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_diary_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.recent",
        description = "Read bounded recent Room notebook Markdown previews; read-only, semantic, and bounded semantic discovery."
    )]
    async fn room_notebook_recent(
        &self,
        params: Parameters<RoomNotebookRecentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = RoomNotebookRecentRequest {
            limit: params.0.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookRecent {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_notebook_recent_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.search",
        description = "Search Room notebook Markdown by bounded case-insensitive substring fields; read-only, semantic, and bounded discovery."
    )]
    async fn room_notebook_search(
        &self,
        params: Parameters<RoomNotebookSearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let payload = RoomNotebookSearchRequest {
            query: params.query,
            limit: params.limit,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookSearch {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_notebook_search_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.notebook.read",
        description = "Read one exact Room notebook Markdown document under the validated Notebook root; read-only, semantic, and bounded."
    )]
    async fn room_notebook_read(
        &self,
        params: Parameters<RoomNotebookReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = RoomNotebookReadRequest {
            path: params.0.path,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomNotebookRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_notebook_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.state.list",
        description = "List deterministic Room state entity documents; read-only, semantic, and bounded."
    )]
    async fn room_state_list(
        &self,
        _params: Parameters<RoomStateListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomStateList {
                request_id: random_id("req"),
                payload: RoomStateListRequest {},
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_state_list_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.state.read",
        description = "Read one exact Room state entity Markdown document by validated entity name; read-only, semantic, and bounded."
    )]
    async fn room_state_read(
        &self,
        params: Parameters<RoomStateReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = RoomStateReadRequest {
            entity: params.0.entity,
        };
        let value = request_active_room(
            &self.state,
            HubCommand::RoomStateRead {
                request_id: random_id("req"),
                payload,
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_state_read_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.maintenance.status",
        description = "Inspect Room maintenance readiness, repository state, schema/scaffold support, executor configuration, workflow/remote availability, synchronization heads, and deterministic occupancy for all five semantic slots; read-only and non-destructive."
    )]
    async fn room_maintenance_status(
        &self,
        _params: Parameters<RoomMaintenanceStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let value = request_active_room(
            &self.state,
            HubCommand::RoomMaintenanceStatus {
                request_id: random_id("req"),
                payload: RoomMaintenanceStatusRequest {},
            },
            REQUEST_TIMEOUT_SECS,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_maintenance_status_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.maintenance.submit",
        description = "Apply one to five unique Room maintenance slot requests after exact validation; destructive but confined to the validated Room repository, with optional local/workflow mode and bounded workflow wait; not open-world."
    )]
    async fn room_maintenance_submit(
        &self,
        params: Parameters<RoomMaintenanceSubmitArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let payload = params.0.into_protocol();
        let timeout_secs = REQUEST_TIMEOUT_SECS
            .max(u64::from(payload.effective_wait_seconds()) + ROOM_TRANSPORT_MARGIN_SECS);
        let value = request_active_room(
            &self.state,
            HubCommand::RoomMaintenanceSubmit {
                request_id: random_id("req"),
                payload,
            },
            timeout_secs,
        )
        .await
        .unwrap_or_else(|error| {
            room_route_error_value_with_timeout(error, "room_maintenance_submit_timeout")
        });
        Ok(result_from_value(value))
    }

    #[tool(
        name = "room.bootstrap",
        description = "Load Room bootstrap guidance; read-only."
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
        description = "Read one validated Room bootstrap guide by id."
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
        name = "bootstrap",
        description = "Load Room bootstrap guidance; read-only alias."
    )]
    async fn bootstrap(&self) -> Result<CallToolResult, ErrorData> {
        self.room_bootstrap().await
    }

    #[tool(
        name = "bootstrap.read",
        description = "Read one Room bootstrap guide by id; read-only alias."
    )]
    async fn bootstrap_read(
        &self,
        params: Parameters<BootstrapReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.room_bootstrap_read(params).await
    }

    #[tool(
        name = "skills.list",
        description = "List local Room skills with active state; read-only discovery."
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
        .unwrap_or_else(|error| room_route_error_value_with_timeout(error, "skills_list_timeout"));
        Ok(result_from_value(slim_skills_list_response(value)))
    }

    #[tool(
        name = "skills.read",
        description = "Read one local Room skill package or resource."
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
        description = "Search local Room skill metadata and content; read-only."
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
        description = "List active Room skill state, including stale entries; read-only."
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
        description = "Mark one valid Room skill active; executes nothing."
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
        description = "Remove active state for one Room skill; executes nothing."
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
        description = "Start one asynchronous Room skill installation; use install get/cancel for follow-up."
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
        description = "Inspect or briefly wait for one Room skill installation; read-only lifecycle inspection."
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
        description = "Request cooperative cancellation of one Room skill installation before commit."
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
        description = "Run an executable from an active Room skill as a managed process."
    )]
    async fn skills_run(
        &self,
        params: Parameters<SkillRunArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let value = request_active_room(
            &self.state,
            HubCommand::SkillsRun {
                request_id: random_id("req"),
                payload: SkillRunRequest {
                    id: params.id,
                    path: params.path,
                    group: params.group,
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

fn ok_json(value: Value) -> CallToolResult {
    AgenticResult::from_native_value(value).into_call_tool_result()
}

fn remove_empty_warnings(value: &mut Value) {
    if value
        .get("warnings")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        if let Some(object) = value.as_object_mut() {
            object.remove("warnings");
        }
    }
}

fn slim_skills_list_response(mut value: Value) -> Value {
    remove_empty_warnings(&mut value);
    if let Some(skills) = value.get_mut("skills").and_then(Value::as_array_mut) {
        for skill in skills {
            remove_empty_warnings(skill);
        }
    }
    value
}

fn result_from_value(value: Value) -> CallToolResult {
    AgenticResult::from_native_value(value).into_call_tool_result()
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
    use crate::config::{HubConfig, RemoteConfirmationConfig};
    use crate::db::init_db;
    use crate::state::McpProfile;
    use agentic_gpt_protocol::ProcessInfo;
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::Json;
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
            mcp_profile: McpProfile::Full,
            agents: Arc::new(crate::agents::lifecycle::Connections::new()),
            dispatch: Arc::new(crate::agents::dispatch::Dispatch::new()),
            confirmations: Arc::new(crate::confirmation::Confirmations::new()),
            process_cache: Arc::new(crate::state::ProcessCache::new()),
            boot_generations: Arc::new(Mutex::new(HashMap::new())),
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
            "process.list",
            "process.status",
            "process.output",
            "process.result",
            "hub.process.status",
            "hub.process.list",
            "tmux.listSessions",
            "tmux.listPanes",
            "tmux.capturePane",
            "hub.run.get",
            "room.diary.active",
            "room.diary.read",
            "room.notebook.recent",
            "room.notebook.search",
            "room.notebook.read",
            "room.state.list",
            "room.state.read",
            "room.maintenance.status",
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
            "process.batch",
            "process.cancel",
            "tmux.exec",
            "tmux.pasteText",
            "tmux.createSession",
            "tmux.closeSession",
            "mcp.batch",
            "mcp.callTool",
            "user.notify.send",
            "room.maintenance.submit",
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
    fn coordinator_profile_exposes_only_native_tools() {
        let mut state = test_state();
        state.mcp_profile = McpProfile::Coordinator;
        let server = AgenticMcpServer::new(state);
        let mut names = transport::app_tool_descriptors(&server)
            .into_iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "agent.list",
                "hub.info",
                "hub.process.list",
                "hub.process.status",
                "hub.run.get",
                "hub.run.list",
                "user.notify.channels",
                "user.notify.send",
            ]
        );
    }

    #[tokio::test]
    async fn coordinator_rejects_hidden_execution_tools_before_dispatch() {
        let mut state = test_state();
        state.mcp_profile = McpProfile::Coordinator;
        let server = AgenticMcpServer::new(state);
        let error = transport::call_app_tool(
            &server,
            json!({ "name": "process.exec", "arguments": { "agentId": "agent" } }),
        )
        .await
        .unwrap_err();
        assert!(error.contains("tool_unavailable_for_profile"));
        for name in [
            "room.diary.active",
            "room.diary.read",
            "room.notebook.recent",
            "room.notebook.search",
            "room.notebook.read",
            "room.state.list",
            "room.state.read",
            "room.maintenance.status",
            "room.maintenance.submit",
        ] {
            let error = transport::call_app_tool(&server, json!({ "name": name, "arguments": {} }))
                .await
                .unwrap_err();
            assert!(
                error.contains("tool_unavailable_for_profile"),
                "{name}: {error}"
            );
        }
        let run_count: i64 = server
            .state
            .db
            .lock()
            .unwrap()
            .query_row("select count(*) from agent_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(run_count, 0);
    }

    #[test]
    fn full_profile_keeps_bootstrap_aliases_and_execution_surface() {
        let server = AgenticMcpServer::new(test_state());
        let names = transport::app_tool_descriptors(&server)
            .into_iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "bootstrap"));
        assert!(names.iter().any(|name| name == "bootstrap.read"));
        for name in [
            "process.exec",
            "process.batch",
            "process.list",
            "process.status",
            "process.output",
            "process.result",
            "process.cancel",
            "hub.process.list",
            "hub.process.status",
            "room.diary.active",
            "room.diary.read",
            "room.notebook.recent",
            "room.notebook.search",
            "room.notebook.read",
            "room.state.list",
            "room.state.read",
            "room.maintenance.status",
            "room.maintenance.submit",
        ] {
            assert!(
                names.iter().any(|candidate| candidate == name),
                "missing {name}"
            );
        }
    }

    #[test]
    fn mcp_batch_descriptor_freezes_bounds_and_side_effect_annotations() {
        let server = AgenticMcpServer::new(test_state());
        let tools = transport::app_tool_descriptors(&server);
        let batch = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("mcp.batch"))
            .expect("mcp.batch descriptor missing");
        assert_eq!(batch["annotations"]["readOnlyHint"], false);
        assert_eq!(batch["annotations"]["destructiveHint"], false);
        assert_eq!(batch["annotations"]["openWorldHint"], true);
        let calls = &batch["inputSchema"]["properties"]["calls"];
        assert_eq!(calls["type"], "array");
        assert_eq!(calls["minItems"], 1);
        assert_eq!(calls["maxItems"], 16);
        assert_eq!(
            batch["inputSchema"]["properties"]["waitSeconds"]["minimum"],
            0
        );
        assert_eq!(
            batch["inputSchema"]["properties"]["waitSeconds"]["maximum"],
            30
        );
        assert_eq!(
            batch["inputSchema"]["properties"]["timeoutSeconds"]["minimum"],
            1
        );
        assert_eq!(
            batch["inputSchema"]["properties"]["timeoutSeconds"]["maximum"],
            900
        );
        assert!(batch["inputSchema"]["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("agentId"))
                && required.contains(&json!("calls"))));
    }

    #[test]
    fn skill_install_and_run_tools_are_exposed_with_stable_annotations() {
        let server = AgenticMcpServer::new(test_state());
        let tools = transport::app_tool_descriptors(&server);
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
        for tool in transport::app_tool_descriptors(&server) {
            let name = tool["name"].as_str().unwrap();
            let result =
                transport::call_app_tool(&server, json!({ "name": name, "arguments": {} })).await;
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
            let response = transport::mcp_post(
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
    fn room_mcp_input_schemas_do_not_include_agent_id() {
        let schemas = [
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomDiaryActiveArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomDiaryReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookRecentArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookSearchArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomNotebookReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomStateListArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomStateReadArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomMaintenanceStatusArgs)).unwrap(),
            serde_json::to_string(&rmcp::schemars::schema_for!(RoomMaintenanceSubmitArgs)).unwrap(),
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
        let value = json!({ "processes": [] });

        let result = result_from_value(value.clone());
        let serialized = serde_json::to_value(result).unwrap();

        assert_eq!(serialized["structuredContent"], value);
        assert_eq!(serialized["isError"], false);
        assert_eq!(serialized["content"][0]["type"], "text");
    }

    #[tokio::test]
    async fn mcp_tools_call_wire_response_uses_agentic_result_shape() {
        let response = transport::mcp_post(
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

    #[tokio::test]
    async fn offline_process_output_and_result_are_unavailable_and_cache_only() {
        let state = test_state();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values ('agent', null, 'Agent', 1, 'hash', null, ?1)",
                [json!({
                    "processes": true,
                    "confirmation": true,
                    "notificationActions": false
                })
                .to_string()],
            )
            .unwrap();
        }
        let now = chrono::Utc::now().to_rfc3339();
        let process: ProcessInfo = serde_json::from_value(json!({
            "agentId": "agent",
            "processId": "process-1",
            "kind": "command",
            "state": "completed",
            "createdAt": now.clone(),
            "updatedAt": now,
            "captureStatus": "complete"
        }))
        .unwrap();
        state
            .process_cache
            .record("agent", "old-connection", None, process)
            .await;
        state
            .process_cache
            .mark_connection_stale("agent", "old-connection")
            .await;
        let server = AgenticMcpServer::new(state);

        let output = server
            .process_output(Parameters(ProcessOutputArgs {
                agent_id: "agent".to_string(),
                process_id: "process-1".to_string(),
                cursor: None,
                max_bytes: Some(1024),
            }))
            .await
            .unwrap();
        let output = serde_json::to_value(output).unwrap()["structuredContent"].clone();
        assert_eq!(output["status"], "unavailable");
        assert_eq!(output["error"]["code"], "process_output_unavailable");
        assert_eq!(output["cached"]["processId"], "process-1");
        assert_eq!(output["freshness"], "stale");

        let result = server
            .process_result(Parameters(ProcessResultArgs {
                agent_id: "agent".to_string(),
                process_id: "process-1".to_string(),
                max_bytes: Some(1024),
            }))
            .await
            .unwrap();
        let result = serde_json::to_value(result).unwrap()["structuredContent"].clone();
        assert_eq!(result["status"], "unavailable");
        assert_eq!(result["error"]["code"], "process_result_unavailable");
        assert_eq!(result["cached"]["processId"], "process-1");
        assert_eq!(result["freshness"], "stale");
        for value in [&output, &result] {
            for field in ["stdout", "stderr", "result"] {
                assert!(value.get(field).is_none());
                assert!(value["cached"].get(field).is_none());
            }
        }
    }

    #[test]
    fn process_lifecycle_arg_schemas_match_protocol_bounds() {
        assert_eq!(
            default_process_wait_seconds(),
            ProcessStatusRequest::DEFAULT_WAIT_SECONDS
        );
        let status_schema =
            serde_json::to_string(&rmcp::schemars::schema_for!(ProcessStatusArgs)).unwrap();
        assert!(status_schema.contains("\"default\":5"));
        assert!(status_schema.contains("\"maximum\":30"));
        let output_schema =
            serde_json::to_string(&rmcp::schemars::schema_for!(ProcessOutputArgs)).unwrap();
        assert!(output_schema.contains("\"default\":8192"));
        assert!(output_schema.contains("\"maximum\":32768"));
        let result_schema =
            serde_json::to_string(&rmcp::schemars::schema_for!(ProcessResultArgs)).unwrap();
        assert!(result_schema.contains("\"default\":8192"));
        assert!(result_schema.contains("\"maximum\":524288"));
        for (wait_seconds, expected) in [(None, 5), (Some(0), 0), (Some(31), 30)] {
            let params = ProcessStatusArgs {
                agent_id: "agent".to_string(),
                process_id: "process".to_string(),
                wait_seconds,
            };
            let payload = process_status_payload(&params);
            assert_eq!(payload.wait_seconds, Some(expected));
            assert_eq!(payload.effective_wait_seconds(), expected);
        }
    }
}
