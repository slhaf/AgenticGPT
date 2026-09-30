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

const MCP_INSTRUCTIONS: &str = "Agentic GPT Hub 完整配置提供 49 个工具。先用 agent.list 选择已启用的本地 Agent：进程、tmux 和指定 Agent 的下游 MCP 工具以 agentId 路由；Room、bootstrap 与 skills 工具不接受 agentId，每次请求由 Hub 捕获当时的活动 Room 连接。process.exec/process.batch 与 mcp.callTool/mcp.batch 创建 Agent 受管理进程；waitSeconds 只控制本次内联等待，不代表完成或取消，后续用 process.status/output/result/cancel。hub.process.* 只读 Hub 进程缓存，需检查 freshness/observedAt；process.list/status 向 Agent 请求实时状态（请求失败时可能回退缓存）。hub.run.* 查询 Hub 持久派发回执，不等于进程状态，也不证明进程完成或停止。先用 mcp.listServers/listTools 发现下游 MCP 服务器、工具及参数 schema，再调用会在下游真实执行的 mcp.callTool/batch；批次失败不回滚已开始的调用。tmux.exec 返回提交状态和短暂窗格快照，不是完成证据；需要时用 tmux.capturePane 核验输出，先用 tmux.listPanes 找目标。skills.install 是异步安装/替换，随后用 skills.install.get/cancel；安装不等于运行，skills.run 只运行已激活的 skill 并按进程工具追踪。发送前先查 user.notify.channels；user.notify.send 会真实投递，accepted 不表示用户已看到。工具原生 JSON 位于 MCP structuredContent，并以 text 内容返回；structuredContent 顶层含 error 时 isError 为 true；参数/路由等 MCP 协议错误与工具返回的 JSON 错误不同。annotations 是行为提示而非授权。";
const COORDINATOR_INSTRUCTIONS: &str = "协调者配置仅暴露 8 个 Hub 原生工具：hub.info、agent.list、hub.run.list/get、hub.process.status/list 和 user.notify.channels/send。可查看 Agent 注册/在线状态、Hub 派发回执与当前进程缓存快照；hub.process.* 是可能过期的 Hub 缓存（检查 freshness/observedAt），hub.run.* 是回执而非实时进程状态或停止证据。此配置不会向 Agent 派发执行、进程控制、tmux、下游 MCP、Room、bootstrap 或 skill 操作。user.notify.channels 只列通道能力/可用性；user.notify.send 会真实投递，accepted 不表示用户已看到。结果原生 JSON 位于 MCP structuredContent 并以 text 返回；其中顶层 error 会标记 isError，参数/路由错误可作为 MCP 协议错误返回。annotations 是提示，不是授权。";

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
            "process.exec"
                | "process.batch"
                | "process.cancel"
                | "tmux.pasteText"
                | "tmux.exec"
                | "tmux.closeSession"
                | "mcp.batch"
                | "mcp.callTool"
                | "room.maintenance.submit"
                | "skills.deactivate"
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
        description = "查询 Hub 本地运行与容量概况，适合确认服务版本、超时上限、Agent 数量、待处理请求/确认及进程缓存规模；返回 service/version、remoteConfirmation、agents、counts 和 generatedAt，不派发命令。数据库或序列化失败会作为 MCP 错误返回。"
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
        description = "发现已注册 Agent 及其可用性，供后续需要 agentId 的工具选择目标；返回 agents 数组，条目含 agentId、alias、displayName、online、connectionMode、lastSeenAt、capabilities 和 configSummary。只读 Hub registry/连接状态，不选择 Room Agent；数据库错误为 MCP 错误。"
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
        description = "按 runId 查询一条 Hub 持久派发回执，适合从超时错误中的 runId 恢复检查；返回 request/Agent/命令身份、status、时间、reason，以及可选 result、processId/process 和 resultRetained/resultOmitted。它不是实时进程状态、进程输出或停止证明；未找到返回 MCP 参数错误 run_not_found，数据库错误返回 MCP 内部错误。"
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
        description = "检索 Hub 持久派发回执历史，适合按 agentId、source、status 或 sinceSeconds 回顾运行；返回 runs 和实际 limit（默认 20、上限 100）。这些是回执/结果保留记录，不是实时进程列表；不派发命令，数据库错误作为 MCP 内部错误返回。"
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
        description = "读取 Hub 为指定 agentId 保留的进程元数据快照；仅在查看缓存状态且不要求实时性时使用，不会联系 Agent。返回 processes 与 freshness（cached/stale/unknown），有观测时间时含 observedAt；不含进程输出/结构化结果，快照可能过期。Agent 未注册或未启用时返回 MCP 参数错误。"
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
        description = "按 agentId 和 processId 读取一条 Hub 缓存快照，不会联系 Agent；适合离线时查看最近已观测元数据，不能证明当前状态。返回进程字段以及 freshness/observedAt；无缓存项在 structuredContent 中返回 error.code=process_not_found 与 freshness=unknown。Agent 未注册或未启用时返回 MCP 参数错误。"
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
        description = "在指定 Agent 上启动一个受管理进程，适合运行命令；program 与 args 是直接可执行文件和参数数组，不自动拆分 shell 字符串，shell 语法须显式调用 bash/sh。返回 processId、status、completedInline 等初始进程结果，waitSeconds 只限制内联等待。执行会产生实际副作用并仍受 Agent 本地策略/确认约束；process_exec_timeout 是结果内 JSON 错误，超时不表示未启动或已取消，可用 process.status/output/result/cancel 跟进，回执则用 hub.run.get 查询。"
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
        description = "在同一批次准入边界内启动多个 Agent 受管理进程；每项可用 workingDirectory 覆盖批次默认目录。返回 batchId、status、completedInline 和逐项 results/processId；已启动项的副作用不会回滚。waitSeconds 只等初始结果，process_batch_timeout 不会取消子进程；按返回的进程标识用 process 工具跟进。"
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
        description = "列出指定 Agent 的活动或保留进程，可按 group/kind/state 过滤并以 cursor 续页；优先用于在线实时发现。返回 processes、nextCursor，以及 freshness/observedAt；Agent 请求失败时 Hub 可能回退缓存快照，需据 freshness 判断。离线时不能续用 Agent 签发的 cursor，会返回 status=unavailable 和 process_list_cursor_unavailable；该错误不等于空列表。"
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
        description = "查询指定进程的 Agent 实时状态；waitSeconds 可短暂等待状态变化，但不会取消进程。返回 ProcessInfo 元数据及可选 waitElapsedMs，不含 stdout、stderr 或结果正文。Agent 不可达时 structuredContent 含 process_status_unavailable，可能附带 Hub 缓存摘要但不视为实时状态；未注册/启用的 Agent 返回 MCP 参数错误。"
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
        description = "以非消费式分页读取 Agent 保存的 stdout/stderr；cursor 按原始字节续读，maxBytes 限制本页，非法 UTF-8 会用 base64 编码保留。返回 processId、stdout/stderr 分段的 data/offset/encoding（可能含 gap）、nextCursor、hasMore、eof 与 captureStatus。Agent 不可达时返回 process_output_unavailable；Hub 缓存只补元数据，不提供输出正文。"
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
        description = "读取 Agent 保留的完整结构化进程结果；仅用于取结果，不取代 process.output 的 stdout/stderr，也不从 Hub 缓存补内容。返回 processId、status（complete/too_large/unavailable）、resultAvailable，以及可选 result、error、resultBytes、resultSha256、resultPreview。结果缺失/过大按状态说明；Agent 不可达时返回 process_result_unavailable。"
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
        description = "请求 Agent 取消指定进程，并检查实际终止证据；返回 state、cancelOutcome、terminationEvidence，失败时可含 error。请求取消不保证远端已停止；Agent 不可达时返回 process_cancel_unavailable，并可能附 Hub 缓存元数据，不能把缓存当停止证据。"
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
        description = "发现指定 Agent 上持久 tmux 会话，适合在命令前确认会话是否存在；返回 sessions（会话名、窗口数、附着状态及活动/创建时间），不改变会话。tmux Hub 请求超时作为 MCP 错误返回。"
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
        description = "发现指定 Agent 的 tmux panes，可选按 session 收窄；返回 panes（paneId、session、currentPath/currentCommand、shell、dead 和 copy-mode 提示），供后续 capturePane/exec/pasteText 选择目标。只读；Hub 请求超时作为 MCP 错误返回。"
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
        description = "读取指定 tmux pane 最近的有界历史，适合在 tmux.exec 后检查输出；返回 capture 文本，不推进或消费 pane。它是某一时刻的屏幕快照，不单独证明命令完成；目标无效或 Hub 请求超时会返回错误。"
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
        description = "把文本输入到非 shell 的 tmux pane（如 TUI/REPL）；shell pane 会以 tmux_shell_paste_forbidden 拒绝，应改用 tmux.exec。submit=true 会追加 Enter，可能触发界面动作；confirmation/local policy 仍适用。成功返回 status=completed，表示已处理输入而非下游任务完成；失败返回 error。"
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
        description = "向指定 Agent 的活动 shell 窗格提交一个 program/args 命令；仅接受存活且可用的 shell 窗格，并执行本地预检、策略与必要确认。成功返回 status=submitted 及可选 snapshot/warning；waitMs 仅决定短暂观察时间，snapshot 不是命令完成证据，之后用 tmux.capturePane 核实。命令会作用于持久窗格/工作区；错误以结果内 JSON 返回，Hub 请求超时为 MCP 错误。"
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
        description = "在指定 Agent 的允许工作目录中创建或复用持久 tmux 会话，适合需要跨调用保留 shell/TUI 状态的工作；返回 session 和 created。会话与其窗格会持续存在并承载后续命令，cwd 必须在 Agent 允许范围内；Hub 请求超时作为 MCP 错误返回。"
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
        description = "关闭指定 Agent 的持久 tmux session；只有在明确需要结束该工作区时调用。关闭会终止其中窗口/窗格并可能中断正在运行的工作，默认请求确认且仍服从本地策略。成功返回 session 和 closed=true；失败返回错误，Hub 请求超时作为 MCP 错误返回。"
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
        description = "发现下游 MCP servers：提供 agentId 时查询该 Agent，否则聚合当前在线 Agents。返回单 Agent 的 servers，或 agents 数组及各自 servers/error；不调用下游工具。聚合范围仅在线 Agent，单个请求失败会保留在对应条目的 error；数据库失败作为结果内 JSON 错误返回。"
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
        description = "按 agentId 和 mcp.listServers 返回的 serverId 查询下游工具清单；返回 tools 及工具名、描述与 input schema，用于确认参数后再调用。只读取发现信息，不执行工具；Agent 超时以结果内 error 返回。"
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
        description = "在指定 Agent 上调用一个已发现的下游 MCP 工具；调用前先用 mcp.listServers/listTools 核对 serverId、toolName、参数和副作用。Hub 将其作为受管理 MCP 进程执行，返回 processId/status/completedInline 与可用的下游 CallToolResult（保留 content/isError）；waitSeconds 只等内联结果，timeoutSeconds 是执行期限。下游副作用不会由 Hub 回滚；超时不等于取消，可用 process 工具跟进。"
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
        description = "在指定 Agent 上运行 1 至 16 个下游 MCP 调用；先核对每项 server/tool/schema。mode 决定并行或顺序，failFast 只阻止尚未开始的子项，已启动调用不取消且副作用不回滚。返回 batchId/status 和逐项结果/进程标识；waitSeconds 只等内联结果，timeoutSeconds 是子调用期限，超时后用 process 工具跟进。"
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
        description = "查询当前 Hub 可用的通知通道及能力，发送前用其选择 channel key；返回 channels，含 key、displayName、available、kind、supportsActions、reason 和可选 agentId。只读、不发送通知；当前 Android 通道会说明不可用原因，数据库错误作为 MCP 内部错误返回。"
    )]
    async fn user_notify_channels(&self) -> Result<CallToolResult, ErrorData> {
        let channels = notification_channels(&self.state)
            .await
            .map_err(|error| mcp_internal_error("db_error", error.to_string()))?;
        Ok(ok_json(json!({ "channels": channels })))
    }

    #[tool(
        name = "user.notify.send",
        description = "通过 channels 返回的 channel key 向用户真实投递通知；适合明确需要用户收到提示时调用，可能触达 Hub ntfy 或 Agent 桌面通道。返回 channelKey、accepted 和可选 reason/deliveryId；accepted 不证明用户已看到。无效/不可用通道、Agent alias 无效或投递失败会在 structuredContent.error 中给出类别，可恢复时先查 channels 或修正通道。"
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
        description = "读取本次调用所捕获的活动 Room Agent 当前 daily/weekly/monthly 日记概览，适合先了解近期记录；返回 daily、weekly、monthly 三层各自的路径、可用性、内容或 issue 提示。只读，不接受 agentId；无活动 Room、活动状态冲突或超时分别返回 room_not_active、room_state_conflict 或专用 timeout 错误。"
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
        description = "读取本次请求捕获的活动 Room 中一个精确日记周期；layer 选择 daily/weekly/monthly，period 使用 Room 本地逻辑周期。返回 document（路径、内容/可用性或 issue）；不修改日记，也不接受 agentId。无活动 Room、状态冲突或超时会以 structuredContent.error 返回。"
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
        description = "在活动 Room 的 Notebook 中发现最近文档，适合在搜索或精读前浏览；limit 限制返回数量。返回 documents 数组（每项含 path、title、contentPreview、truncated、effectiveAt）及可能的 warnings，不返回完整正文、不修改文件，也不接受 agentId。"
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
        description = "在活动 Room 的 Notebook 路径、H1 标题和正文中做不区分大小写的子串搜索；适合先定位候选文档，再用 room.notebook.read 精读。返回 documents 数组（path、title、contentPreview、truncated、effectiveAt）及可能的 warnings，不修改文件、不接受 agentId；query 长度受 schema 限制。"
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
        description = "按 Notebook 相对 Markdown path 读取一个精确文档；只能使用 Room 返回/发现的 Notebook/ 下路径，任意仓库路径会被拒绝。返回 path 和完整 content；只读、不接受 agentId，错误和无活动 Room 状态按结果中的 error 说明。"
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
        description = "列出活动 Room 的确定性 State 实体文档，适合发现可读取的当前实体；返回 entities 数组（entity 标识和 path），不修改状态，也不接受 agentId。无活动 Room、状态冲突或超时会返回对应 error。"
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
        description = "按 State/entities/ 下的精确实体文件名读取一个活动 Room 状态实体；只使用 room.state.list 返回的实体名，不可传任意仓库路径。返回文档路径与内容，只读、不接受 agentId；无活动 Room 或无效实体会以 error 返回。"
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
        description = "在提交维护前检查活动 Room 的维护就绪状态：返回 repository、schema、scaffold、localExecutor、configuredMode、autoPush、workflow、remote、sync 和 slots（五个语义槽位的占用）。只读、不接受 agentId，适合先判断仓库与执行路径能否维护。"
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
        description = "向当前活动 Room 提交 1 至 5 个互不重复的 diary.daily/weekly/monthly、notebook 或 entity 槽位维护请求；Hub/Agent 会先按 Room 仓库验证整组输入。返回 mode、state、localApplied、sync、revision 等执行/同步状态。local 模式会改写经验证的 Room 文件，并可能按 autoPush 配置同步/推送；workflow 模式会提交配置的工作流；副作用不自动回滚，waitSeconds 只等待有限时间。"
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
        description = "加载本次请求捕获的活动 Room bootstrap 指引，适合开始 Room 工作前了解可用流程；返回 guides、entrypoint、revision、schemaVersion、totalGuides/returnedGuides 和 warnings，不修改文件、不接受 agentId。需要一篇完整指引时用 room.bootstrap.read；无活动 Room或路由失败会返回 error。"
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
        description = "按 room.bootstrap 返回的 id 读取活动 Room 的一篇已验证 bootstrap 指引；返回 guide、frontmatter、resource 和 warnings，不修改文件、不接受 agentId。不存在的 id、无活动 Room 或路由超时会返回对应 error。"
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
        description = "room.bootstrap 的简短名称别名：读取当前活动 Room 的 bootstrap 指引清单/摘要；只读、不接受 agentId。需要单篇内容时用 bootstrap.read；它与 room.bootstrap 使用相同的路由和错误语义。"
    )]
    async fn bootstrap(&self) -> Result<CallToolResult, ErrorData> {
        self.room_bootstrap().await
    }

    #[tool(
        name = "bootstrap.read",
        description = "room.bootstrap.read 的简短名称别名：用 bootstrap 返回的 id 读取一篇活动 Room 指引及内容；只读、不接受 agentId，错误语义与 room.bootstrap.read 相同。"
    )]
    async fn bootstrap_read(
        &self,
        params: Parameters<BootstrapReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.room_bootstrap_read(params).await
    }

    #[tool(
        name = "skills.list",
        description = "列出活动 Room 工作区中的本地 skills 及 active 标记，适合发现安装内容和后续可运行项；返回 skills 元数据数组和可能的 warnings。只读，不安装/激活或执行 skill，也不接受 agentId。"
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
        description = "读取活动 Room 中一个 skill 包；id 必须是 workspace skills/ 下的技能目录，path 可选且相对包目录，不传时返回兼容的 SKILL.md 内容。返回 skill 详情及可选 resource（path、encoding、content、sizeBytes、sha256）；用于查看指令/资源，不会激活或执行 skill，也不接受 agentId。"
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
        description = "按不区分大小写的子串搜索活动 Room 本地 skill 的 id、frontmatter、tags 和 SKILL.md 正文；用于从已安装 skill 中发现候选。返回有界匹配元数据，不改状态、不运行代码，也不接受 agentId。"
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
        description = "检查活动 Room 中 skill 的持久激活状态，包括可能已过期/陈旧的条目；适合在运行前确认当前状态。返回 active 状态清单，不修改、不执行 skill，也不接受 agentId。"
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
        description = "在活动 Room 持久激活状态中加入指定 skill；用于允许后续 skills.run 使用该技能，不安装包也不执行脚本。返回更新后的激活状态；这是状态写入而非只读操作，不接受 agentId。"
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
        description = "从活动 Room 持久激活状态中移除指定 skill；这不会卸载 skill 包或充当受管理进程取消。返回更新后的激活状态；属于状态修改，不接受 agentId。"
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
        description = "在活动 Room 异步安装一个 GitHub 或显式文件源 skill；可能访问外部网络并创建/替换工作区文件，replaceExisting 会归档旧包，且可设置安装后激活。返回 installId、id、status、queued、deduplicated 和 pollAfterMs；立即返回不等于安装完成，使用 skills.install.get/cancel 跟进。安装只准备包，不运行其脚本；不接受 agentId。"
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
        description = "按 installId 查询或短暂等待活动 Room skill 安装任务；返回 status/phase/progress、attempt、source、时间、可选 result/error 和 pollAfterMs，适合跟踪 skills.install。waitSeconds 只等待状态，不改变安装；不存在的任务和路由错误会返回 error，不接受 agentId。"
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
        description = "请求在提交前协作取消一项活动 Room skill 安装；返回 outcome、changed、status 及可选 phase/cancelRequestedAt。取消是请求而非保证，若安装已进入提交阶段可能返回 too_late/already_terminal；此工具不会卸载已安装包，不接受 agentId。"
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
        description = "运行活动 Room skill 包 scripts/ 下的指定可执行文件，并作为 Agent 受管理进程追踪；可先用 skills.active/read 确认状态与脚本。返回 processId、status、completedInline 等初始结果，waitSeconds 只控制内联等待；脚本会产生实际副作用，使用 process.status/output/result/cancel 跟进，不接受 agentId。"
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
        assert_eq!(batch["annotations"]["destructiveHint"], true);
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
