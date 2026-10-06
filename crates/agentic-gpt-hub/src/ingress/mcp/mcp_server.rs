mod args;
pub(crate) mod transport;

use agentic_gpt_protocol::{
    normalize_process_group, BootstrapReadRequest, EventGetRequest, EventListRequest,
    EventMarkRequest, HubCommand, McpBatchCall, McpBatchRequest, McpCallToolRequest,
    McpListToolsRequest, ProcessBatchExecRequest, ProcessCancelRequest, ProcessExecElement,
    ProcessExecRequest, ProcessKind, ProcessListRequest, ProcessReadRequest, ProcessReadView,
    ProcessState, RoomDiaryActiveRequest, RoomDiaryReadRequest, RoomMaintenanceStatusRequest,
    RoomNotebookReadRequest, RoomNotebookRecentRequest, RoomNotebookSearchRequest,
    RoomStateListRequest, RoomStateReadRequest, SkillActivationRequest, SkillInstallCancelRequest,
    SkillInstallGetRequest, SkillInstallRequest, SkillReadRequest, SkillRunRequest,
    SkillSearchRequest, TmuxCapturePaneRequest, TmuxCloseSessionRequest, TmuxCreateSessionRequest,
    TmuxExecRequest, TmuxListPanesRequest, TmuxPasteTextRequest, UserNotifySendRequest,
};
use args::{
    AgentIdArgs, BootstrapReadArgs, EventGetArgs, EventListArgs, EventMarkArgs, HubRunGetArgs,
    HubRunListArgs, McpBatchArgs, McpCallToolArgs, McpListServersArgs, McpListToolsArgs,
    ProcessBatchArgs, ProcessExecArgs, ProcessIdArgs, ProcessListArgs, ProcessReadArgs,
    RoomDiaryActiveArgs, RoomDiaryReadArgs, RoomMaintenanceStatusArgs, RoomMaintenanceSubmitArgs,
    RoomNotebookReadArgs, RoomNotebookRecentArgs, RoomNotebookSearchArgs, RoomStateListArgs,
    RoomStateReadArgs, SkillActivationArgs, SkillInstallArgs, SkillInstallCancelArgs,
    SkillInstallGetArgs, SkillReadArgs, SkillRunArgs, SkillSearchArgs, TmuxCapturePaneArgs,
    TmuxCloseSessionArgs, TmuxCreateSessionArgs, TmuxExecArgs, TmuxListPanesArgs,
    TmuxListSessionsArgs, TmuxPasteTextArgs, UserNotifySendArgs,
};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ErrorData, Meta, ServerCapabilities, ServerInfo, ToolAnnotations,
};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use serde_json::{json, Map, Value};

use crate::agentic_result::AgenticResult;
use crate::agents::dispatch::{
    cache_value_with_event_panel, cached_process, mcp_list_servers_all_agents, request_agent,
};
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

const MCP_INSTRUCTIONS: &str = "Agentic GPT Hub 完整配置提供完整 Apps MCP 工具集。先用 agent.list 选择已启用的本地 Agent：进程、tmux 和指定 Agent 的下游 MCP 工具以 agentId 路由；Room、bootstrap 与 skills 工具不接收 agentId，每次调用由 Hub 捕获当时的活动 Room 连接。process.exec/process.batch 运行 command 进程，mcp.callTool/mcp.batch 运行 kind=mcp 进程；waitSeconds 只控制本次等待，不代表完成或取消。process.* 的完整执行面为 exec、batch、read、list、cancel；process.read 使用 auto 或 status 视图统一读取状态与可用产物，status 不含产物；auto 可用 cursor 续读 command/skill 输出，status 与 cursor 不可组合，kind=mcp 的下游结果不支持输出 cursor。maxBytes 是整个 ProcessResponse 的 JSON UTF-8 字节预算，省略时使用 Agent 当前 limits.processResponseBytes（出厂默认 8192），显式范围为 4096–1048576。等待超时不取消进程。mcp.callTool 的下游结果以 mcpResult 保留完整 CallToolResult；command/skill 输出用 process.read 获取。skills.run 返回实际执行 Agent 的 agentId 和 processId；之后的 process.* 必须复用这两个值，不会按当前活动 Room 自动路由。event.list/get/mark 必须指定 agentId，只操作该 Agent 的 inbox。hub.process.* 只读 Hub 进程元数据缓存，需检查 freshness/observedAt，不能等待当前状态或获取输出/结果/取消证据；process.list/read 向 Agent 请求实时观察，失败时可能返回明确标记的缓存元数据。hub.run.* 是 Hub 持久派发回执，不等于进程状态或停止证据。先用 mcp.listServers/listTools 发现下游 MCP 服务器、工具及参数 schema；mcp.batch 返回 batchId/status 和按输入顺序的紧凑子进程结果（index/id 与 ProcessResponse 字段）；included 的 mcpResult 保留完整 CallToolResult，deferred 时用对应 agentId/processId 调 process.read 续读。批次失败不回滚已开始的调用。tmux.exec 返回提交状态和短暂窗格快照，不是完成证据；需要时用 tmux.capturePane 核验输出。skills.install 是异步安装/替换，随后用 skills.install.get/cancel；内置 skill-installer 可读但只读、不可运行，调用 skills.run 前检查 origin/readOnly。发送前先查 user.notify.channels 并检查 user.notify.send 的 accepted；桌面通道可用 accepted=false/reason 表示投递失败且 isError 仍为 false，路由/Hub 投递错误才返回 error。accepted 表示通道提供方接受/处理请求，不代表用户端送达或已读；ntfy 只以发布 HTTP 成功确认。工具原生 JSON 位于 MCP structuredContent 并以 text 返回，顶层含 error 时 isError 为 true；参数/路由等 MCP 协议错误与工具 JSON 错误不同。annotations 是行为提示而非授权。";
const COORDINATOR_INSTRUCTIONS: &str = "协调者配置仅暴露 8 个 Hub 原生工具：hub.info、agent.list、hub.run.list/get、hub.process.status/list 和 user.notify.channels/send。可查看 Agent 注册/在线状态、Hub 派发回执与进程缓存快照；hub.process.* 只读可能过期的 Hub 元数据缓存（检查 freshness/observedAt），不请求 Agent 执行或读取、不等待状态，也不返回输出、结果或取消证据；hub.run.* 是回执而非实时进程状态或停止证据。在线且指定 Agent 时，hub.process.* 额外进行一次 best-effort EventPanel 元数据查询并附加 events；Agent 离线或查询失败时省略 events。这些缓存快照不代表 Agent 当前进程状态，也不同于 Full 配置中的 process.read。此配置不会向 Agent 派发执行、进程控制、tmux、下游 MCP、Room、bootstrap 或 skill 操作。发送前先查 user.notify.channels 并检查 accepted：桌面通道可能返回 accepted=false/reason 而 isError=false，路由/Hub 投递错误才返回 error。accepted 表示通道提供方接受/处理请求，不代表用户端送达或已读；ntfy 只以发布 HTTP 成功确认。结果原生 JSON 位于 MCP structuredContent 并以 text 返回；其中顶层 error 会标记 isError，参数/路由错误可作为 MCP 协议错误返回。annotations 是提示，不是授权。";

fn default_process_wait_seconds() -> u64 {
    ProcessReadRequest::DEFAULT_WAIT_SECONDS
}

fn process_read_payload(params: &ProcessReadArgs) -> ProcessReadRequest {
    let mut payload = ProcessReadRequest {
        process_id: params.process_id.clone(),
        wait_seconds: params.wait_seconds,
        view: params.view.map(ProcessReadView::from).unwrap_or_default(),
        cursor: params.cursor.clone(),
        max_bytes: params.max_bytes,
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
                | "event.mark"
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
            | "event.mark"
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
// Native cache-only process tools use the shared dispatch metadata helper.

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
        description = "读取 Hub 为指定 agentId 保留的进程元数据快照；不会请求 Agent 执行或读取进程状态。在线时额外进行一次 best-effort EventPanel 元数据查询并附加 events，Agent 离线或查询失败时省略 events。返回 processes 与 freshness（cached/stale/unknown），有观测时间时含 observedAt；快照可能过期。Agent 未注册或未启用时返回 MCP 参数错误。"
    )]
    async fn hub_process_list(
        &self,
        params: Parameters<AgentIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let agent_id = params.0.agent_id;
        self.ensure_agent_enabled(&agent_id)?;
        let value = snapshot_process_list(&self.state, &agent_id).await;
        Ok(ok_json(
            cache_value_with_event_panel(&self.state, &agent_id, value).await,
        ))
    }

    #[tool(
        name = "hub.process.status",
        description = "按 agentId 和 processId 只读取 Hub 缓存中的进程元数据；不会请求 Agent 执行、读取或等待实时状态，也不返回 stdout/stderr、下游结果正文或取消证据。适合检查最近已观测的快照，不能证明当前状态；检查 freshness/observedAt。在线时会额外进行一次 best-effort EventPanel 元数据查询并附加 events，离线或查询失败时省略。无缓存项在 structuredContent 中返回 error.code=process_not_found 与 freshness=unknown；Agent 未注册或未启用时返回 MCP 参数错误。"
    )]
    async fn hub_process_status(
        &self,
        params: Parameters<ProcessIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let value =
            snapshot_process_status(&self.state, &params.agent_id, &params.process_id).await;
        Ok(result_from_value(
            cache_value_with_event_panel(&self.state, &params.agent_id, value).await,
        ))
    }
    #[tool(
        name = "event.list",
        description = "发现指定 Agent 的待处理事件或按状态/等级筛选历史时使用。返回 items（eventId/summary/severity/createdAt/status）与可选 nextCursor；完整正文和来源用 event.get 查看。events.new 是有展示限制的提醒面板，不是完整 pending 列表；隐藏不等于 handled，仍可通过列表查询。读取不标记 handled，不读取进程状态；成功响应附目标 Agent 的 events 面板并记录实际曝光，历史仍按过期/保留策略维护。必须指定 agentId，不跨 Agent 查找或合并；非法游标或请求失败返回错误，不等于空列表。"
    )]
    async fn event_list(
        &self,
        params: Parameters<EventListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = EventListRequest {
            agent_id: params.agent_id.clone(),
            status: params.status.map(Into::into),
            severity: params.severity.map(Into::into),
            limit: params.limit,
            cursor: params.cursor,
        };
        let command = HubCommand::EventList {
            request_id: random_id("req"),
            payload,
        };
        let value = request_agent(&self.state, &params.agent_id, command, REQUEST_TIMEOUT_SECS)
            .await
            .unwrap_or_else(
                |reason| json!({ "error": { "code": "event_list_timeout", "message": reason } }),
            );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "event.get",
        description = "已知 eventId 且需要完整正文或来源时使用；ID 可来自 event.list 或事件面板。返回完整记录（message、severity/status、source.kind/ref、shownCount、expiresAt）；成功响应附目标 Agent 的 events 面板。读取不标记 handled，不操作进程或安装；详情正文读取不额外计曝光，附带面板实际展示项仍计次。历史仍按过期/保留策略维护；必须指定 agentId，不跨 Agent 查找，未知 ID 或请求失败返回错误。"
    )]
    async fn event_get(
        &self,
        params: Parameters<EventGetArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = EventGetRequest {
            agent_id: params.agent_id.clone(),
            event_id: params.event_id,
        };
        let command = HubCommand::EventGet {
            request_id: random_id("req"),
            payload,
        };
        let value = request_agent(&self.state, &params.agent_id, command, REQUEST_TIMEOUT_SECS)
            .await
            .unwrap_or_else(
                |reason| json!({ "error": { "code": "event_get_timeout", "message": reason } }),
            );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "event.mark",
        description = "事件已处理或明确决定忽略后，在指定 Agent 上将选中的 pending 事件标记为 handled，停止后续提醒；不执行或取消来源进程/安装。返回 handledIds/notFoundIds；成功响应附目标 Agent 的 events 面板。重复 ID 去重，已 handled 的 ID 幂等；过期或不存在的 ID 进入 notFoundIds，空数组不标记任何事件。不会主动删除事件记录，历史仍按过期/保留策略维护；必须指定 agentId，不跨 Agent 操作，请求失败返回错误。"
    )]
    async fn event_mark(
        &self,
        params: Parameters<EventMarkArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = EventMarkRequest {
            agent_id: params.agent_id.clone(),
            event_ids: params.event_ids,
        };
        let command = HubCommand::EventMark {
            request_id: random_id("req"),
            payload,
        };
        let value = request_agent(&self.state, &params.agent_id, command, REQUEST_TIMEOUT_SECS)
            .await
            .unwrap_or_else(
                |reason| json!({ "error": { "code": "event_mark_timeout", "message": reason } }),
            );
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.exec",
        description = "在指定 Agent 上执行一段 Bash 命令或脚本，适用于启动单项工作；多项独立工作可用 process.batch。返回 agentId/processId/state 及预算内可用输出，后续用返回的 agentId/processId 调用 process.read 或 process.cancel。普通非登录/非交互 Bash 原样执行 command（pipefail 生效，不设 set -e）；cwd 在可信初始化后生效。命令可产生文件或外部副作用；策略只分析提交的 command，不检查可信初始化中的 PATH/函数，不是运行时安全边界。waitSeconds 只等待、不取消。"
    )]
    async fn exec(&self, params: Parameters<ProcessExecArgs>) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = ProcessExecRequest {
            agent_id: params.agent_id.clone(),
            group: params.group,
            command: params.command,
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            cwd: params.cwd,
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
        description = "在指定 Agent 上一次提交多条独立 Bash 命令，适用于无需先后依赖的批量工作。整批准入后按 Agent limits.maxConcurrentTasks 控制并发，不保证执行/完成顺序；有依赖关系时用 process.exec 和显式 shell 控制流。Bash 行为与 process.exec 相同；元素 cwd 覆盖批次 cwd。返回 batchId/status 与按输入顺序排列的逐项进程信息（含 agentId/processId），用 process.read 或 process.cancel 跟进单项。waitSeconds 只等待不取消，已启动副作用不会回滚。"
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
                    command: element.command,
                    cwd: element.cwd,
                })
                .collect(),
            need_confirm: params.need_confirm.unwrap_or(false),
            confirm_method: params.confirm_method,
            cwd: params.cwd,
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
        description = "不知道 processId、重新发现已有任务或筛选指定 Agent 进程时使用；已知 ID 要读状态/输出时用 process.read。按 group/kind/state 分页列出活动或保留进程，不读取输出正文。返回 processes/nextCursor 及 freshness/observedAt；Agent 请求失败时可能回退 Hub 缓存，不能当作实时状态。离线时不能续用 Agent 签发的 cursor，会返回 status=unavailable 和 process_list_cursor_unavailable，不等于空列表。"
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
        name = "process.read",
        description = "已有 agentId/processId 时读取执行状态、增量输出或下游 MCP 结果，适合跟进 process.exec、process.batch、skills.run 或 mcp.callTool；不知道进程 ID 时先用 process.list。不启动或取消工作。view=status 只返回元数据并等待执行终态/期限；auto 优先返回未读产物，否则有界等待。返回 agentId/processId/kind/state/captureStatus 及可用 output/mcpResult。执行终态与 output.eof 不能互相推断；hasMore 仅表示当前有未读输出，gap 表示已丢失字节，captureStatus=incomplete 时不要无限等 EOF。command/skill 可用 cursor 续读；status 或 kind=mcp 不接受输出 cursor。mcpResult.value 为完整 CallToolResult；pending 不是失败，deferred 可提高 maxBytes 领取，not_retained 不可恢复。waitSeconds 到期不取消；Agent 不可达时返回 process_read_unavailable 及可能的 Hub 缓存元数据，缓存不含产物且不是实时结果。"
    )]
    async fn process_read(
        &self,
        params: Parameters<ProcessReadArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        self.ensure_agent_enabled(&params.agent_id)?;
        let payload = process_read_payload(&params);
        let timeout_seconds = payload.effective_wait_seconds() + 2;
        let command = HubCommand::ProcessRead {
            request_id: random_id("req"),
            payload,
        };
        let value =
            match request_agent(&self.state, &params.agent_id, command, timeout_seconds).await {
                Ok(value) => value,
                Err(reason) => {
                    unavailable_process_value(
                        &self.state,
                        &params.agent_id,
                        &params.process_id,
                        "process_read_unavailable",
                        reason,
                    )
                    .await
                }
            };
        Ok(result_from_value(value))
    }

    #[tool(
        name = "process.cancel",
        description = "需要停止排队中或已启动的工作时，向指定 Agent 请求取消 processId；返回 state/cancelOutcome/terminationEvidence 和可选 error，可附独立 events 面板。依据 Agent 返回的证据判断是否已观察到停止，不把请求已接收当作副作用已停止。本地命令/skill 取消针对受管理进程组，不保证脱组后代停止；MCP 向下游请求取消，不保证远端副作用停止。Agent 不可达时返回 process_cancel_unavailable 及可能的 Hub 缓存元数据，缓存不是停止证据。"
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
            Ok(value) => value,
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
        description = "发现下游 MCP 服务器：提供 agentId 时查询该已启用 Agent，否则聚合当前已启用且在线的 Agents（离线 Agent 不在结果内）。返回单 Agent 的 servers，或 agents 数组及各自 servers/error；不调用下游工具。单个在线 Agent 请求失败会保留在对应条目的 error；数据库失败作为结果内 JSON 错误返回。"
    )]
    async fn mcp_list_servers(
        &self,
        params: Parameters<McpListServersArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        if let Some(agent_id) = params.0.agent_id {
            self.ensure_agent_enabled(&agent_id)?;
            let command = HubCommand::McpListServers {
                request_id: random_id("req"),
                suppress_event_panel: false,
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
        description = "在指定 Agent 上调用一个已发现的下游 MCP 工具；调用前先用 mcp.listServers/listTools 核对 serverId、toolName、参数和副作用。Hub 将其作为 kind=mcp 的受管理进程执行，返回统一 ProcessResponse（agentId/processId/kind/state 等）及预算内的可选 mcpResult；waitSeconds 只控制此次等待。timeoutSeconds 从调用获准且取得执行槽后起算，限制下游连接/请求，不含确认等待和排队。下游副作用不会由 Hub 回滚；超时不等于取消，之后用相同 agentId/processId 调 process.read/cancel 跟进。"
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
        description = "在指定 Agent 上运行 1 至 16 个下游 MCP 调用；先核对每项 server/tool/schema。mode 决定并行或顺序，failFast 只阻止尚未开始的子项，已启动调用不取消且副作用不回滚。返回 batchId、status、可选 error 和按输入顺序的 results；每项含 index、可选 id 及紧凑 ProcessResponse 字段（包括 agentId/captureStatus 与可选 mcpResult）。mcpResult 为 included 时保留完整 CallToolResult，deferred 时用本次输入的 agentId 和对应 processId 调 process.read 续读。waitSeconds 只等内联结果，timeoutSeconds 对每个调用从获准且取得执行槽后起算，限制下游连接/请求，不含确认等待和排队。"
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
        description = "通过 channels 返回的 channel key 向用户真实投递通知；适合明确需要用户收到提示时调用，可能触达 Hub ntfy 或 Agent 桌面通道。返回 channelKey、accepted 和可选 reason/deliveryId；先检查 accepted：桌面提供方失败可只返回 accepted=false 和 reason，且 isError=false；路由/Hub 投递错误才在 structuredContent.error 中给出。accepted 只表示通道提供方接受/处理请求，ntfy 只确认发布 HTTP 成功，均不证明用户端送达或已读；失败时先查 channels 或修正通道。"
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
        description = "列出活动 Room 的工作区技能及内置 skill-installer，适合发现可读内容和候选脚本；返回 skills 元数据与 warnings，条目含 origin/readOnly。内置项 origin=builtin、readOnly=true，虽可读取但不能由 skills.run 执行；只读、不接受 agentId。"
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
        description = "读取活动 Room 中一个工作区 skill 或内置 id=skill-installer。无 path 时 SKILL.md 正文位于 skill.skillMd，resource 不返回；提供包内 path 时才返回 resource（path、encoding、content、sizeBytes、sha256），内置项只有内嵌 SKILL.md、没有其他包资源。内置项标记 origin=builtin/readOnly=true，只能阅读、不能由 skills.run 运行；查看 origin/readOnly 后再决定是否执行，不接受 agentId。"
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
        description = "按不区分大小写的子串搜索活动 Room 工作区技能及内置 skill-installer 的 id、frontmatter、tags 和 SKILL.md 正文；用于发现候选。返回 skills 元数据数组与 warnings，查看 origin/readOnly 区分可运行工作区技能与只读内置项；不改状态、不运行代码，也不接受 agentId。"
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
        description = "检查活动 Room 持久激活状态，包括可能已过期/陈旧的条目；返回 activeSkills（状态、stale 和可选 summary）。active 不代表可运行：内置 skill-installer 可能处于激活状态但 readOnly=true；执行前用 skills.list/read 检查 origin/readOnly。此工具只读、不执行 skill、不接受 agentId。"
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
        description = "在活动 Room 异步安装一个 GitHub 或显式文件源 skill；可能访问外部网络并创建/替换工作区文件，replaceExisting 会归档旧包，且可设置安装后激活。启动校验通过并受理时返回 installId、id、status、queued、deduplicated 和 pollAfterMs；立即返回不等于安装完成，使用 skills.install.get/cancel 跟进。启动校验/目标冲突等错误只含 code/message，不保证有任务标识、phase 或 retryable。安装只准备包，不运行其脚本；不接受 agentId。"
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
        description = "按 installId 查询或短暂等待活动 Room skill 安装任务；任务状态返回 status、progress、attempt、source、时间和 pollAfterMs，phase/result/error 按当前状态可选。后台失败的 error 含 code/message/retryable，phase 可缺省；启动校验或路由错误只含 code/message，不保证 phase/retryable。waitSeconds 只等待状态，不改变安装；不接受 agentId。"
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
        description = "仅运行活动 Room 中已激活、origin=workspace 且 readOnly=false 的 skill 包 scripts/ 下可执行文件；内置 skill-installer 即使 active 也不可运行。返回统一 ProcessResponse 并包含实际执行 Agent 的 agentId、processId；waitSeconds 只控制本次等待。脚本会产生实际副作用；后续 process.read/cancel 必须复用返回的 agentId 和 processId，不按当前活动 Room 自动路由；skill stdout/stderr 用 process.read，不能用下游 mcpResult。"
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
    use super::args::ProcessReadViewArgs;
    use super::*;
    use crate::config::{HubConfig, RemoteConfirmationConfig};
    use crate::db::init_db;
    use crate::state::{McpProfile, OutboundAgentMessage};
    use agentic_gpt_protocol::{
        AgentConnectionMode, AgentMessage, AgentRole, Capabilities, ProcessInfo,
        DEFAULT_PROCESS_RESPONSE_BYTES,
    };
    use axum::body::to_bytes;
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::Json;
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::{mpsc, Mutex};

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
    fn register_agent_record(state: &HubState, agent_id: &str, secret: &str) {
        let conn = state.db.lock().unwrap();
        let capabilities = Capabilities {
            processes: true,
            confirmation: true,
            notification_actions: true,
        };
        conn.execute(
            "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
             values (?1, null, ?1, 1, ?2, null, ?3)",
            rusqlite::params![
                agent_id,
                crate::utils::sha256_hex(secret),
                serde_json::to_string(&capabilities).unwrap()
            ],
        )
        .unwrap();
    }

    async fn register_online_agent(
        state: &HubState,
        agent_id: &str,
        secret: &str,
        connection_id: &str,
    ) -> mpsc::UnboundedReceiver<crate::state::OutboundAgentMessage> {
        register_agent_record(state, agent_id, secret);
        let (sender, receiver) = mpsc::unbounded_channel();
        state
            .agents
            .insert_for_test(
                agent_id,
                crate::state::AgentConnection {
                    connection_id: connection_id.to_string(),
                    sender,
                    last_seen_at: chrono::Utc::now(),
                    role: AgentRole::Normal,
                    connection_mode: AgentConnectionMode::CommandCapable,
                    hello_received: true,
                    boot_generation: Some("test-boot".to_string()),
                    transport: crate::state::AgentTransport::Sse,
                    config_summary: None,
                    notification_channels: Vec::new(),
                },
            )
            .await;
        receiver
    }

    fn agent_headers(secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-agent-secret", HeaderValue::from_str(secret).unwrap());
        headers
    }

    fn tools_call_rpc(id: u64, name: &str, arguments: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
    }

    async fn respond_to_agent_command(
        state: &HubState,
        agent_id: &str,
        secret: &str,
        connection_id: &str,
        envelope: &agentic_gpt_protocol::HubCommandEnvelope,
        data: Value,
    ) {
        let response = crate::agents::transport::post_agent_message(
            State(state.clone()),
            Path(agent_id.to_string()),
            Query(crate::agents::transport::SseConnectQuery::for_test(Some(
                connection_id.to_string(),
            ))),
            agent_headers(secret),
            Json(AgentMessage::Response {
                run_id: Some(envelope.run_id.clone()),
                request_id: envelope.request_id.clone(),
                data,
                event_sources: vec![],
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn call_event_mcp_tool(
        state: &HubState,
        agent_outbound: &mut mpsc::UnboundedReceiver<crate::state::OutboundAgentMessage>,
        other_outbound: &mut mpsc::UnboundedReceiver<crate::state::OutboundAgentMessage>,
        id: u64,
        tool: &str,
        arguments: Value,
        agent_response: Value,
    ) -> Value {
        let state_for_call = state.clone();
        let tool_name = tool.to_string();
        let request = tokio::spawn(async move {
            transport::mcp_post(
                State(state_for_call),
                Json(tools_call_rpc(id, &tool_name, arguments)),
            )
            .await
        });
        let OutboundAgentMessage::Text(text) =
            tokio::time::timeout(std::time::Duration::from_secs(5), agent_outbound.recv())
                .await
                .expect("expected an Agent command")
                .unwrap()
        else {
            panic!("expected Agent command envelope");
        };
        let envelope: agentic_gpt_protocol::HubCommandEnvelope =
            serde_json::from_str(&text).unwrap();
        match (tool, &envelope.command) {
            ("event.list", agentic_gpt_protocol::HubCommand::EventList { payload, .. }) => {
                assert_eq!(payload.agent_id, "event-agent");
                assert_eq!(payload.limit, Some(4));
            }
            ("event.get", agentic_gpt_protocol::HubCommand::EventGet { payload, .. }) => {
                assert_eq!(payload.agent_id, "event-agent");
                assert_eq!(payload.event_id, "event-1");
            }
            ("event.mark", agentic_gpt_protocol::HubCommand::EventMark { payload, .. }) => {
                assert_eq!(payload.agent_id, "event-agent");
                assert_eq!(payload.event_ids, vec!["event-1", "event-2"]);
            }
            _ => panic!("event tool routed as an unexpected HubCommand"),
        }
        assert!(other_outbound.try_recv().is_err());

        respond_to_agent_command(
            state,
            "event-agent",
            "event-secret",
            "event-connection",
            &envelope,
            agent_response,
        )
        .await;
        let response = request.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            agent_outbound.try_recv().is_err(),
            "Hub sent an extra Agent command"
        );
        assert!(
            other_outbound.try_recv().is_err(),
            "Hub queried another Agent"
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["id"], id);
        value["result"].clone()
    }

    #[tokio::test]
    async fn event_tools_forward_only_explicit_agent_responses_and_keep_error_panels() {
        let state = test_state();
        let mut event_agent =
            register_online_agent(&state, "event-agent", "event-secret", "event-connection").await;
        let mut other_agent =
            register_online_agent(&state, "other-agent", "other-secret", "other-connection").await;
        let panel = json!({
            "current": "low: 1 | medium: 0 | high: 0",
            "new": [{ "event-1 | completed": "low | 2026-10-01T00:00:00Z" }]
        });

        let listed = call_event_mcp_tool(
            &state,
            &mut event_agent,
            &mut other_agent,
            1,
            "event.list",
            json!({ "agentId": "event-agent", "status": "pending", "limit": 4 }),
            json!({
                "items": [{ "eventId": "event-1", "summary": "completed", "status": "pending" }],
                "nextCursor": null,
                "events": panel
            }),
        )
        .await;
        assert_eq!(
            listed["structuredContent"]["items"][0]["eventId"],
            "event-1"
        );
        assert_eq!(listed["structuredContent"]["events"], panel);
        assert_eq!(listed["isError"], false);

        let fetched = call_event_mcp_tool(
            &state,
            &mut event_agent,
            &mut other_agent,
            2,
            "event.get",
            json!({ "agentId": "event-agent", "eventId": "event-1" }),
            json!({
                "error": { "code": "event_not_found", "message": "Event was not found" },
                "events": panel
            }),
        )
        .await;
        assert_eq!(
            fetched["structuredContent"]["error"]["code"],
            "event_not_found"
        );
        assert_eq!(fetched["structuredContent"]["events"], panel);
        assert_eq!(fetched["isError"], true);

        let marked = call_event_mcp_tool(
            &state,
            &mut event_agent,
            &mut other_agent,
            3,
            "event.mark",
            json!({ "agentId": "event-agent", "eventIds": ["event-1", "event-2"] }),
            json!({
                "handledIds": ["event-1"],
                "notFoundIds": ["event-2"],
                "events": panel
            }),
        )
        .await;
        assert_eq!(
            marked["structuredContent"]["handledIds"],
            json!(["event-1"])
        );
        assert_eq!(marked["structuredContent"]["events"], panel);
        let missing_target = transport::mcp_post(
            State(state.clone()),
            Json(tools_call_rpc(
                4,
                "event.list",
                json!({ "status": "pending" }),
            )),
        )
        .await;
        let body = to_bytes(missing_target.into_body(), usize::MAX)
            .await
            .unwrap();
        let missing_target: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(missing_target["error"]["code"], -32602);
        assert!(event_agent.try_recv().is_err());
        assert!(other_agent.try_recv().is_err());

        assert_eq!(marked["isError"], false);
    }

    #[tokio::test]
    async fn native_cache_status_adds_one_panel_and_preserves_error_when_panel_fails() {
        let state = test_state();
        let mut outbound =
            register_online_agent(&state, "cache-agent", "cache-secret", "cache-connection").await;
        let panel = json!({ "current": "low: 0 | medium: 0 | high: 0", "new": [] });

        let call_state = state.clone();
        let call = tokio::spawn(async move {
            transport::mcp_post(
                State(call_state),
                Json(tools_call_rpc(
                    1,
                    "hub.process.status",
                    json!({ "agentId": "cache-agent", "processId": "missing-process" }),
                )),
            )
            .await
        });
        let OutboundAgentMessage::Text(text) =
            tokio::time::timeout(std::time::Duration::from_secs(5), outbound.recv())
                .await
                .expect("expected one EventPanel request")
                .unwrap()
        else {
            panic!("expected EventPanel command");
        };
        let envelope: agentic_gpt_protocol::HubCommandEnvelope =
            serde_json::from_str(&text).unwrap();
        assert!(matches!(
            envelope.command,
            agentic_gpt_protocol::HubCommand::EventPanel { .. }
        ));
        respond_to_agent_command(
            &state,
            "cache-agent",
            "cache-secret",
            "cache-connection",
            &envelope,
            panel.clone(),
        )
        .await;
        let response = call.await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["result"]["structuredContent"]["error"]["code"],
            "process_not_found"
        );
        assert_eq!(value["result"]["structuredContent"]["events"], panel);
        assert_eq!(value["result"]["isError"], true);
        assert!(
            outbound.try_recv().is_err(),
            "status requested more than one panel"
        );

        let call_state = state.clone();
        let call = tokio::spawn(async move {
            transport::mcp_post(
                State(call_state),
                Json(tools_call_rpc(
                    2,
                    "hub.process.status",
                    json!({ "agentId": "cache-agent", "processId": "missing-process" }),
                )),
            )
            .await
        });
        let OutboundAgentMessage::Text(text) =
            tokio::time::timeout(std::time::Duration::from_secs(5), outbound.recv())
                .await
                .expect("expected one EventPanel request for malformed panel response")
                .unwrap()
        else {
            panic!("expected EventPanel command");
        };
        let envelope: agentic_gpt_protocol::HubCommandEnvelope =
            serde_json::from_str(&text).unwrap();
        respond_to_agent_command(
            &state,
            "cache-agent",
            "cache-secret",
            "cache-connection",
            &envelope,
            json!({ "current": 0, "new": [] }),
        )
        .await;
        let response = call.await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["result"]["structuredContent"]["error"]["code"],
            "process_not_found"
        );
        assert!(value["result"]["structuredContent"].get("events").is_none());
        assert!(outbound.try_recv().is_err());
    }

    #[tokio::test]
    async fn offline_native_process_cache_tools_omit_events() {
        let state = test_state();
        register_agent_record(&state, "offline-agent", "offline-secret");
        for (id, name, arguments) in [
            (1, "hub.process.list", json!({ "agentId": "offline-agent" })),
            (
                2,
                "hub.process.status",
                json!({ "agentId": "offline-agent", "processId": "missing-process" }),
            ),
        ] {
            let response = transport::mcp_post(
                State(state.clone()),
                Json(tools_call_rpc(id, name, arguments)),
            )
            .await;
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert!(
                value["result"]["structuredContent"].get("events").is_none(),
                "{name} must not synthesize an offline event panel"
            );
        }
    }

    #[test]
    fn tool_read_only_hints_match_side_effect_semantics() {
        for name in [
            "agent.list",
            "event.list",
            "event.get",
            "process.list",
            "process.read",
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
            "event.mark",
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
    #[tokio::test]
    async fn coordinator_hides_event_tools_before_target_dispatch() {
        let mut state = test_state();
        state.mcp_profile = McpProfile::Coordinator;
        let server = AgenticMcpServer::new(state);
        for name in ["event.list", "event.get", "event.mark"] {
            let error = transport::call_app_tool(
                &server,
                json!({ "name": name, "arguments": { "agentId": "agent" } }),
            )
            .await
            .unwrap_err();
            assert!(
                error.contains("tool_unavailable_for_profile"),
                "{name}: {error}"
            );
        }
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
            "process.read",
            "process.cancel",
            "hub.process.list",
            "event.list",
            "event.get",
            "event.mark",
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
        assert_eq!(
            names
                .iter()
                .filter(|name| name.starts_with("process."))
                .count(),
            5
        );
        for removed in ["process.status", "process.output", "process.result"] {
            assert!(!names.iter().any(|candidate| candidate == removed));
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
    async fn process_read_mcp_dispatches_process_read_and_returns_unified_response() {
        let state = test_state();
        let mut outbound =
            register_online_agent(&state, "agent", "agent-secret", "agent-connection").await;
        let request_state = state.clone();
        let call = tokio::spawn(async move {
            transport::mcp_post(
                State(request_state),
                Json(tools_call_rpc(
                    7,
                    "process.read",
                    json!({
                        "agentId": "agent",
                        "processId": "process-1",
                        "waitSeconds": 0,
                        "view": "auto",
                        "cursor": "cursor-1"
                    }),
                )),
            )
            .await
        });
        let OutboundAgentMessage::Text(text) =
            tokio::time::timeout(std::time::Duration::from_secs(5), outbound.recv())
                .await
                .expect("process.read dispatches within the tool timeout")
                .unwrap()
        else {
            panic!("expected a reliable Agent command envelope");
        };
        let envelope: agentic_gpt_protocol::HubCommandEnvelope =
            serde_json::from_str(&text).unwrap();
        assert!(!envelope.run_id.is_empty());
        assert!(!envelope.command_hash.is_empty());
        let (request_id, payload) = match &envelope.command {
            agentic_gpt_protocol::HubCommand::ProcessRead {
                request_id,
                payload,
            } => (request_id.clone(), payload.clone()),
            command => panic!("expected ProcessRead, received {command:?}"),
        };
        assert_eq!(request_id, envelope.request_id);
        assert_eq!(payload.process_id, "process-1");
        assert_eq!(payload.wait_seconds, Some(0));
        assert_eq!(payload.view, ProcessReadView::Auto);
        assert_eq!(payload.cursor.as_deref(), Some("cursor-1"));
        assert!(payload.max_bytes.is_none());

        let response_data = json!({
            "agentId": "agent",
            "processId": "process-1",
            "kind": "command",
            "state": "running",
            "captureStatus": "capturing"
        });
        respond_to_agent_command(
            &state,
            "agent",
            "agent-secret",
            "agent-connection",
            &envelope,
            response_data.clone(),
        )
        .await;
        let response = call.await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(value["result"]["structuredContent"], response_data);
        assert_eq!(value["result"]["isError"], false);
    }

    #[tokio::test]
    async fn offline_process_read_is_unavailable_and_metadata_only() {
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

        let response = server
            .process_read(Parameters(ProcessReadArgs {
                agent_id: "agent".to_string(),
                process_id: "process-1".to_string(),
                wait_seconds: Some(0),
                view: None,
                cursor: None,
                max_bytes: None,
            }))
            .await
            .unwrap();
        let response = serde_json::to_value(response).unwrap()["structuredContent"].clone();
        assert_eq!(response["status"], "unavailable");
        assert_eq!(response["error"]["code"], "process_read_unavailable");
        assert_eq!(response["cached"]["processId"], "process-1");
        assert_eq!(response["freshness"], "stale");
        assert!(response["observedAt"].is_string());
        for field in ["output", "mcpResult", "stdout", "stderr", "result"] {
            assert!(response.get(field).is_none());
            assert!(response["cached"].get(field).is_none());
        }
    }

    #[test]
    fn process_read_arg_schema_matches_contract_and_defaults() {
        assert_eq!(
            default_process_wait_seconds(),
            ProcessReadRequest::DEFAULT_WAIT_SECONDS
        );
        let schema = serde_json::to_value(rmcp::schemars::schema_for!(ProcessReadArgs)).unwrap();
        let schema_text = schema.to_string();
        assert!(schema_text.contains("\"default\":5"));
        assert!(schema_text.contains("\"maximum\":30"));
        assert!(schema_text.contains("\"minimum\":4096"));
        assert!(schema_text.contains("\"maximum\":1048576"));
        assert!(schema_text.contains("\"auto\""));
        assert!(schema_text.contains("\"status\""));
        assert!(schema_text.contains("limits.processResponseBytes"));
        assert!(schema_text.contains(&DEFAULT_PROCESS_RESPONSE_BYTES.to_string()));
        assert!(schema["properties"].get("cursor").is_some());
        let server = AgenticMcpServer::new(test_state());
        let descriptor = transport::app_tool_descriptors(&server)
            .into_iter()
            .find(|tool| tool["name"] == "process.read")
            .expect("process.read descriptor missing");
        let descriptor_schema = &descriptor["inputSchema"];
        let descriptor_view = &descriptor_schema["properties"]["view"];
        assert_eq!(descriptor_view["default"], "auto");
        assert!(!descriptor_schema["required"]
            .as_array()
            .is_some_and(|required| required.contains(&json!("view"))));

        for args in [
            json!({"agentId": "agent", "processId": "process"}),
            json!({"agentId": "agent", "processId": "process", "view": null}),
        ] {
            let params: ProcessReadArgs = serde_json::from_value(args).unwrap();
            assert_eq!(process_read_payload(&params).view, ProcessReadView::Auto);
        }

        for (wait_seconds, expected) in [(None, 5), (Some(0), 0), (Some(31), 30)] {
            let params = ProcessReadArgs {
                agent_id: "agent".to_string(),
                process_id: "process".to_string(),
                wait_seconds,
                view: None,
                cursor: None,
                max_bytes: None,
            };
            let payload = process_read_payload(&params);
            assert_eq!(payload.wait_seconds, Some(expected));
            assert_eq!(payload.effective_wait_seconds(), expected);
            assert_eq!(payload.view, ProcessReadView::Auto);
            assert!(payload.cursor.is_none());
            assert!(payload.max_bytes.is_none());
        }
        let params = ProcessReadArgs {
            agent_id: "agent".to_string(),
            process_id: "process".to_string(),
            wait_seconds: None,
            view: Some(ProcessReadViewArgs::Status),
            cursor: None,
            max_bytes: Some(4096),
        };
        let payload = process_read_payload(&params);
        assert_eq!(payload.view, ProcessReadView::Status);
        assert_eq!(payload.max_bytes, Some(4096));
    }
}
