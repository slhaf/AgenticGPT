use agentic_gpt_protocol::{
    EventSeverity, EventStatus, McpBatchMode, NotificationAction, RoomDiaryLayer,
    RoomMaintenanceExecutionMode, RoomMaintenanceRequestItem, RoomMaintenanceSlot,
    RoomMaintenanceSubmitRequest, SkillInstallFile, SkillInstallSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    default_process_list_limit, default_process_wait_seconds, default_room_notebook_limit,
    default_room_wait_seconds, default_standard_wait_seconds,
};

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentIdArgs {
    #[schemars(
        description = "Hub 中已启用的本地 Agent ID；可从 agent.list 获取。Room 工具另路由到当前活动的 Room Agent，不接收此字段。"
    )]
    pub(super) agent_id: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum EventStatusArgs {
    Pending,
    Handled,
    Expired,
}

impl From<EventStatusArgs> for EventStatus {
    fn from(value: EventStatusArgs) -> Self {
        match value {
            EventStatusArgs::Pending => Self::Pending,
            EventStatusArgs::Handled => Self::Handled,
            EventStatusArgs::Expired => Self::Expired,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum EventSeverityArgs {
    Low,
    Medium,
    High,
}

impl From<EventSeverityArgs> for EventSeverity {
    fn from(value: EventSeverityArgs) -> Self {
        match value {
            EventSeverityArgs::Low => Self::Low,
            EventSeverityArgs::Medium => Self::Medium,
            EventSeverityArgs::High => Self::High,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct EventListArgs {
    #[schemars(
        description = "目标本地 Agent ID；必须指定 Hub 中已启用的 Agent，不会自动选择其他 Agent。"
    )]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "可选事件状态筛选：pending、handled 或 expired；省略或 null 时为 pending，不表示列出所有状态。"
    )]
    pub(super) status: Option<EventStatusArgs>,
    #[serde(default)]
    #[schemars(
        description = "可选通知优先级筛选：low、medium 或 high；省略或 null 时不筛选，等级不表示来源任务成功或失败。"
    )]
    pub(super) severity: Option<EventSeverityArgs>,
    #[serde(default)]
    #[schemars(description = "每页结果数；省略或 null 时为20，Agent 将显式值限制到1–100。")]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    #[schemars(
        description = "上一页 event.list 返回的 nextCursor；原样续页并保持同一 agentId、status、severity，limit 可调整。非法或筛选不匹配的游标会报错。"
    )]
    pub(super) cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct EventGetArgs {
    #[schemars(
        description = "目标本地 Agent ID；必须指定 Hub 中已启用的 Agent，不会自动选择其他 Agent。"
    )]
    pub(super) agent_id: String,
    #[schemars(description = "event.list 或事件面板中的事件 ID；不是进程或安装 ID。")]
    pub(super) event_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct EventMarkArgs {
    #[schemars(
        description = "目标本地 Agent ID；必须指定 Hub 中已启用的 Agent，不会自动选择其他 Agent。"
    )]
    pub(super) agent_id: String,
    #[schemars(
        description = "已处理或决定忽略的事件 ID，最多512项；重复/已 handled 的 ID 幂等，过期或未知 ID 进入 notFoundIds。空数组不标记任何事件，不表示确认全部。"
    )]
    pub(super) event_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct HubRunGetArgs {
    #[schemars(
        description = "从 hub.run.list 返回的运行记录 ID；不存在或已清理的记录会返回 run_not_found。"
    )]
    pub(super) run_id: String,
}

#[derive(Debug, Default, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct HubRunListArgs {
    #[serde(default)]
    #[schemars(description = "可选的本地 Agent ID 精确筛选；省略或传 null 时不按 Agent 限制。")]
    pub(super) agent_id: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "可选的来源字符串精确筛选，按记录中的 source 原值匹配；例如 hub 或 tunnel，不是固定枚举。"
    )]
    pub(super) source: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "可选的状态字符串精确筛选，按记录中的 status 原值匹配；例如 started、completed、failed 或 timeout_waiting_result，不是固定枚举。"
    )]
    pub(super) status: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "只保留最近这段时间内创建的记录；单位为秒，省略或传 null 时不设时间下限。"
    )]
    pub(super) since_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(description = "返回记录数上限；省略或传 null 时为 20，运行时限制在 1–100。")]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProcessExecArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "可选工作流分组名，会随托管进程记录；首尾空白会去除，去除后不能为空、最多 32 个 Unicode 字符且不能含控制字符。"
    )]
    pub(super) group: Option<String>,
    #[schemars(description = "要执行的 Bash 命令原文；Agent 按原文执行，不按 argv 拆分或改写。")]
    pub(super) command: String,
    #[serde(default)]
    #[schemars(
        description = "是否在请求中要求确认；省略或传 null 时为 false。最终 Allow、Confirm 或 Deny 仍由 Agent 本地策略决定，true 不能覆盖策略拒绝，false 也不能绕过策略确认。"
    )]
    pub(super) need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "可选的确认提供方覆盖；省略、null 或 default 沿用 Agent 本地配置的提供方链，其他值按受支持的旧式提供方名称解析；不能绕过本地策略。"
    )]
    pub(super) confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "可选当前工作目录；省略或 null 使用 Agent 工作区根目录。Bash 初始化完成后会切换至此目录。"
    )]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "启动后等待进程完成的内联等待秒数；省略或传 null 时为 5，范围 0–30，超过 30 时运行时按 30 处理；0 表示不等待完成。"
    )]
    pub(super) wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProcessBatchArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "可选工作流分组名，继承给批次中的每个托管进程；按与 process.exec 相同的规则去除首尾空白并校验非空、最多 32 个 Unicode 字符且无控制字符。"
    )]
    pub(super) group: Option<String>,
    #[schemars(
        description = "必填的独立 Bash 命令列表；整个批次先准入、后按配置并发启动。按输入顺序返回逐项进程信息，不保证执行/完成顺序。元素 cwd（非 null）覆盖批次 cwd；空数组不启动进程。"
    )]
    pub(super) elements: Vec<ProcessBatchElementArgs>,
    #[serde(default)]
    #[schemars(
        description = "是否为批次请求确认；省略或传 null 时为 false。Agent 本地策略仍可对各元素要求确认或拒绝；需要确认的元素通过批次确认流程处理。"
    )]
    pub(super) need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "可选的批次级确认提供方覆盖，传给所有需要确认的元素；省略、null 或 default 使用 Agent 本地配置，其他值按受支持的旧式提供方名称解析，不能绕过本地策略。"
    )]
    pub(super) confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "所有元素的默认工作目录；省略或 null 使用 Agent 工作区根目录。元素 cwd（非 null）覆盖此值。"
    )]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "开始批次后等待所有子进程完成的内联等待秒数；省略或传 null 时为 5，范围 0–30，超过 30 时运行时按 30 处理；0 表示不等待完成。"
    )]
    pub(super) wait_seconds: Option<u64>,
}

/// 批次中的单条命令；可单独指定工作目录覆盖批次默认目录。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProcessBatchElementArgs {
    #[schemars(description = "要执行的 Bash 命令原文；不按 argv 拆分或改写。")]
    pub(super) command: String,
    #[serde(default)]
    #[schemars(
        description = "可选的元素级工作目录；非 null 时覆盖批次 cwd，省略或 null 时继承批次目录。"
    )]
    pub(super) cwd: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProcessIdArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[schemars(
        description = "托管进程 ID；使用 process.exec、process.batch、skills.run 或 mcp.callTool 返回的 ID。"
    )]
    pub(super) process_id: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProcessReadViewArgs {
    #[default]
    Auto,
    Status,
}

fn default_process_read_view() -> ProcessReadViewArgs {
    ProcessReadViewArgs::Auto
}

impl From<ProcessReadViewArgs> for agentic_gpt_protocol::ProcessReadView {
    fn from(value: ProcessReadViewArgs) -> Self {
        match value {
            ProcessReadViewArgs::Auto => Self::Auto,
            ProcessReadViewArgs::Status => Self::Status,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProcessReadArgs {
    #[schemars(
        description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。Room skill 首响返回的 agentId 必须原样复用，不会按当前活动 Room 自动路由。"
    )]
    pub(super) agent_id: String,
    #[schemars(
        description = "要读取的托管进程 ID；使用 process.exec、process.batch、skills.run 或 mcp.callTool 返回的 processId。"
    )]
    pub(super) process_id: String,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_process_wait_seconds",
        description = "等待可观察变化的最长秒数；省略或传 null 时为 5，范围 0–30，超过 30 时按 30 处理；0 立即读取。auto 有输出 backlog 时优先返回，status 只等待执行终态或期限；等待超时不取消进程。"
    )]
    pub(super) wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        default = "default_process_read_view",
        description = "观察视图：auto（默认）读取有界输出或 kind=mcp 结果并可等待；status 仅返回状态元数据，不含输出或结构化结果。"
    )]
    pub(super) view: Option<ProcessReadViewArgs>,
    #[serde(default)]
    #[schemars(
        description = "auto 视图的非消费式输出游标，用于续读 command/skill 输出；status 不能与 cursor 同时使用。kind=mcp 的下游结果不支持输出 cursor，应读取 mcpResult。"
    )]
    pub(super) cursor: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 4096, max = 1048576),
        description = "整个紧凑 ProcessResponse 的 JSON UTF-8 字节预算，不含传输封套和独立 events 面板，不切分 MCP 结果；省略时不在 Hub 覆盖 Agent 当前 limits.processResponseBytes（出厂默认 8192）；显式值范围 4096–1048576。"
    )]
    pub(super) max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProcessListArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "可选的分组精确筛选；按去除首尾空白后的名称匹配，不能为空、最多 32 个 Unicode 字符且不能含控制字符。"
    )]
    pub(super) group: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "可选的进程类型筛选；仅接受 command、skill 或 mcp，省略或传 null 时不筛选。"
    )]
    pub(super) kind: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "可选的进程状态精确筛选；仅接受 queued、waiting_confirmation、starting、running、completed、failed、rejected、cancel_requested、cancelled、timed_out、detached、unknown_after_restart 或 skipped。"
    )]
    pub(super) state: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_process_list_limit",
        description = "最多返回的保留进程数；默认 50，范围 1–100。"
    )]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    #[schemars(
        description = "上一页 process.list 响应中的 nextCursor；不透明游标，原样传回以继续分页。"
    )]
    pub(super) cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxListSessionsArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxListPanesArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "可选的 tmux 会话名精确筛选；省略或传 null 时列出该 Agent 下所有会话的窗格。"
    )]
    pub(super) session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxCapturePaneArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[schemars(
        description = "要捕获的 tmux 窗格目标，例如 session:window.pane 或窗格 ID（如 %0）；目标必须存在。"
    )]
    pub(super) target: String,
    #[serde(default)]
    #[schemars(
        description = "捕获的最近 tmux 历史行数；省略或传 null 时为 160，运行时最多取 5000 行。"
    )]
    pub(super) lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxPasteTextArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[schemars(
        description = "要粘贴的 tmux 窗格目标，例如 session:window.pane 或窗格 ID（如 %0）；shell 窗格会被拒绝。"
    )]
    pub(super) target: String,
    #[schemars(
        description = "粘贴到非 shell 窗格或 TUI 的原文；这是有副作用的输入，可改变目标应用状态。"
    )]
    pub(super) text: String,
    #[serde(default)]
    #[schemars(description = "是否在粘贴文本后追加 Enter；省略或传 null 时为 false。")]
    pub(super) submit: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "是否先请求本地确认；省略或传 null 时为 true，显式 false 会跳过此确认。"
    )]
    pub(super) need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxExecArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[schemars(
        description = "必须是可用的 tmux shell 窗格目标，例如 session:window.pane 或 %0；非 shell、已退出或处于 copy mode 的窗格会被拒绝。"
    )]
    pub(super) target: String,
    #[schemars(
        description = "提交到 shell 窗格的程序名或内建命令；命令会先经过本地路径/执行预检和策略判定。"
    )]
    pub(super) program: String,
    #[serde(default)]
    #[schemars(
        description = "可选的结构化参数数组；程序及每个参数会分别 shell 引号转义，数组元素中的 shell 运算符按字面参数处理。需要 shell 语法时显式调用 bash 或 sh 并使用 -lc。"
    )]
    pub(super) args: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "是否额外请求本地确认；省略或传 null 时为 false，但本地执行策略仍可要求确认或拒绝。"
    )]
    pub(super) need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "提交命令后、读取窗格快照前等待的毫秒数；省略或传 null 时为 300，运行时最多等待 5000 毫秒。"
    )]
    pub(super) wait_ms: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "提交后的窗格快照最多包含的历史行数；省略或传 null 时为 120，运行时最多取 5000 行；0 禁用快照。快照不证明命令已完成。"
    )]
    pub(super) capture_lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxCreateSessionArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[schemars(description = "要创建或复用的 tmux 会话名；不能为空或含控制字符。")]
    pub(super) name: String,
    #[schemars(
        description = "会话工作目录；相对路径从 Agent 工作区根目录解析，必须是现存目录并通过本地路径策略。"
    )]
    pub(super) cwd: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxCloseSessionArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[schemars(description = "要关闭的 tmux 会话名；会终止该会话及其窗格中的工作。")]
    pub(super) name: String,
    #[serde(default)]
    #[schemars(
        description = "是否在关闭会话前请求本地确认；省略或传 null 时为 true，显式 false 会跳过此确认。"
    )]
    pub(super) need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpListServersArgs {
    #[serde(default)]
    #[schemars(
        description = "可选的本地 Agent ID；提供时只查询该 Agent。省略或传 null 时，Hub 仅聚合当前已启用且在线的 Agent 的 MCP 服务器；已注册但离线的 Agent 不包含在结果中。"
    )]
    pub(super) agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpListToolsArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[schemars(description = "mcp.listServers 返回的 MCP 服务器 ID。")]
    pub(super) server_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpCallToolArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "可选工作流分组名，会记录在对应托管进程上；首尾空白会去除，去除后不能为空、最多 32 个 Unicode 字符且不能含控制字符。"
    )]
    pub(super) group: Option<String>,
    #[schemars(description = "目标 MCP 服务器 ID；应使用同一 Agent 的 mcp.listServers 返回值。")]
    pub(super) server_id: String,
    #[schemars(description = "下游 MCP 工具名；应使用 mcp.listTools 返回的名称。")]
    pub(super) tool_name: String,
    #[serde(default)]
    #[schemars(
        description = "转发给下游工具的 JSON 对象；省略或传 null 时使用空对象 {}，序列化后最多 256 KiB，非对象参数会被拒绝。"
    )]
    pub(super) arguments: Option<Value>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "启动后等待托管调用结果的内联等待秒数；省略或传 null 时为 5，范围 0–30，超过 30 时运行时按 30 处理；0 不等待结果。"
    )]
    pub(super) wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "下游连接/请求的截止时长，单位秒；从调用获准并取得并发执行槽后开始，不含确认和等待执行槽的排队时间。省略或传 null 时为 300，运行时限制在 1–900 秒（小于 1 的值按 1 处理）。"
    )]
    pub(super) timeout_seconds: Option<u64>,
}

/// 下游 MCP 批次的调度方式：parallel 并发执行（默认），sequential 按 calls 输入顺序执行；并发子调用可能各自产生副作用。
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum McpBatchModeArgs {
    #[default]
    Parallel,
    Sequential,
}

impl From<McpBatchModeArgs> for McpBatchMode {
    fn from(value: McpBatchModeArgs) -> Self {
        match value {
            McpBatchModeArgs::Parallel => Self::Parallel,
            McpBatchModeArgs::Sequential => Self::Sequential,
        }
    }
}

/// 批次中的单个下游 MCP 工具调用；省略参数时按空对象处理。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpBatchCallArgs {
    #[schemars(description = "配置中的 MCP 服务器 ID；使用该 Agent 的 mcp.listServers 返回值。")]
    pub(super) server_id: String,
    #[schemars(description = "下游 MCP 工具名；使用 mcp.listTools 返回的名称。")]
    pub(super) tool_name: String,
    #[serde(default)]
    #[schemars(
        description = "转发给下游工具的 JSON 对象；省略或传 null 时使用空对象 {}，每次调用序列化后最多 256 KiB。"
    )]
    pub(super) arguments: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpBatchArgs {
    #[schemars(description = "目标本地 Agent ID；必须是 Hub 中已启用的 Agent。")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "可选工作流分组名，继承给批次中每个托管进程；首尾空白会去除，去除后不能为空、最多 32 个 Unicode 字符且不能含控制字符。"
    )]
    pub(super) group: Option<String>,
    #[schemars(
        length(min = 1, max = 16),
        description = "必填的下游调用列表；必须有 1–16 项，每项参数序列化后最多 256 KiB，全部参数合计最多 2 MiB。调用可能各自产生外部副作用，批次不回滚已执行调用。"
    )]
    pub(super) calls: Vec<McpBatchCallArgs>,
    #[serde(default)]
    #[schemars(description = "调用调度方式；省略或传 null 时为 parallel，也可指定 sequential。")]
    pub(super) mode: Option<McpBatchModeArgs>,
    #[serde(default)]
    #[schemars(
        description = "是否快速停止后续调度；省略或传 null 时为 false。设为 true 后，硬失败发生时不再启动尚未开始的子调用，但不会取消已启动的调用。"
    )]
    pub(super) fail_fast: Option<bool>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "批次开始后等待整体结果的内联等待秒数；省略或传 null 时为 5，范围 0–30，超过 30 时运行时按 30 处理；0 不等待结果。"
    )]
    pub(super) wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 900),
        description = "每个子调用在获准并取得下游并发执行槽后适用的连接/请求截止时长，单位秒；不含确认和等待执行槽的排队时间。省略或传 null 时为 300，范围 1–900，超过上限时运行时按 900 处理。"
    )]
    pub(super) timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserNotifySendArgs {
    #[schemars(
        description = "通知目标 channel key；从 user.notify.channels 选择 available 为 true 的条目。"
    )]
    pub(super) channel: String,
    #[schemars(description = "通知标题文本；按目标通道能力发送，不由此字段保证显示长度。")]
    pub(super) title: String,
    #[schemars(description = "通知正文文本；按目标通道能力发送，不由此字段保证显示长度。")]
    pub(super) body: String,
    #[serde(default)]
    #[schemars(
        description = "可选的动作 ID/标签列表；字段会被接受，但当前桌面、ntfy 和 Android 投递实现均不投递动作，也不会产生 actionId 确认回传。"
    )]
    pub(super) actions: Option<Vec<UserNotifyActionArgs>>,
    #[serde(default)]
    #[schemars(
        description = "可选的通道优先级提示。Hub 的 ntfy 通道将 min/low 映射为低、high 映射为高、urgent/alarm 映射为最高，其余或省略均按 normal 处理；桌面通道忽略此值。"
    )]
    pub(super) priority: Option<String>,
}

/// 通知动作的稳定标识和显示标签；当前投递实现不发送动作，也不会回传动作 ID。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserNotifyActionArgs {
    #[schemars(description = "动作的稳定 ID；当前投递实现不会触发该动作或回传 actionId。")]
    pub(super) id: String,
    #[schemars(description = "供通知界面显示的动作标签；当前投递实现不会显示动作。")]
    pub(super) label: String,
}

impl From<UserNotifyActionArgs> for NotificationAction {
    fn from(value: UserNotifyActionArgs) -> Self {
        Self {
            id: value.id,
            label: value.label,
        }
    }
}

/// 无输入字段；请传空对象，附加字段会因 deny_unknown_fields 被拒绝。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomDiaryActiveArgs {}

/// Room 日记层取值为 daily、weekly 或 monthly：daily 使用 current 或有效 YYYY-MM-DD，weekly/monthly 使用 current 或有序的 YYYY-MM-DD--YYYY-MM-DD 范围。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum RoomDiaryLayerArgs {
    Daily,
    Weekly,
    Monthly,
}

impl From<RoomDiaryLayerArgs> for RoomDiaryLayer {
    fn from(value: RoomDiaryLayerArgs) -> Self {
        match value {
            RoomDiaryLayerArgs::Daily => Self::Daily,
            RoomDiaryLayerArgs::Weekly => Self::Weekly,
            RoomDiaryLayerArgs::Monthly => Self::Monthly,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomDiaryReadArgs {
    #[schemars(description = "日记时间层；取值为 daily、weekly 或 monthly。")]
    pub(super) layer: RoomDiaryLayerArgs,
    #[schemars(
        pattern(r"^(current|\d{4}-\d{2}-\d{2}(--\d{4}-\d{2}-\d{2})?)$"),
        description = "Room 本地日记周期：daily 使用 current 或有效的 YYYY-MM-DD；weekly/monthly 使用 current 或 YYYY-MM-DD--YYYY-MM-DD，范围起始日期不得晚于结束日期。"
    )]
    pub(super) period: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomNotebookRecentArgs {
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_room_notebook_limit",
        description = "返回的近期 Notebook 预览数；省略或传 null 时为 20，范围 1–100。"
    )]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomNotebookSearchArgs {
    #[schemars(
        length(min = 1, max = 256),
        description = "不区分大小写的子串查询，匹配 Notebook 路径、一级标题和正文；长度为 1–256 个字符。"
    )]
    pub(super) query: String,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_room_notebook_limit",
        description = "最多返回的 Notebook 预览数；省略或传 null 时为 20，范围 1–100。"
    )]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomNotebookReadArgs {
    #[schemars(
        description = "要读取的精确 Notebook 相对 Markdown 路径；路径必须位于 Notebook/ 下、以 .md 结尾且不能越出 Room 仓库。优先使用 recent/search 返回的路径。"
    )]
    pub(super) path: String,
}

/// 无输入字段；请传空对象，附加字段会因 deny_unknown_fields 被拒绝。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomStateListArgs {}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomStateReadArgs {
    #[schemars(
        description = "State/entities/ 下实体文件的文件名主干，不含目录或 .md；不能为空、含路径分隔符或为 . / ..，例如 project.v2。任意仓库路径会被拒绝。"
    )]
    pub(super) entity: String,
}

/// 无输入字段；请传空对象，附加字段会因 deny_unknown_fields 被拒绝。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomMaintenanceStatusArgs {}

/// Room 维护目标槽位：diary.daily/weekly/monthly 分别指向对应层的 current.md；notebook 按 payload.path 定位，entity 按 payload.entity 定位。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum RoomMaintenanceSlotArgs {
    #[serde(rename = "diary.daily")]
    DiaryDaily,
    #[serde(rename = "diary.weekly")]
    DiaryWeekly,
    #[serde(rename = "diary.monthly")]
    DiaryMonthly,
    Notebook,
    Entity,
}

impl From<RoomMaintenanceSlotArgs> for RoomMaintenanceSlot {
    fn from(value: RoomMaintenanceSlotArgs) -> Self {
        match value {
            RoomMaintenanceSlotArgs::DiaryDaily => Self::DiaryDaily,
            RoomMaintenanceSlotArgs::DiaryWeekly => Self::DiaryWeekly,
            RoomMaintenanceSlotArgs::DiaryMonthly => Self::DiaryMonthly,
            RoomMaintenanceSlotArgs::Notebook => Self::Notebook,
            RoomMaintenanceSlotArgs::Entity => Self::Entity,
        }
    }
}

/// Room 维护执行方式：local 在已验证仓库中直接应用，workflow 提交给已配置的 Room workflow。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(super) enum RoomMaintenanceModeArgs {
    Local,
    Workflow,
}

impl From<RoomMaintenanceModeArgs> for RoomMaintenanceExecutionMode {
    fn from(value: RoomMaintenanceModeArgs) -> Self {
        match value {
            RoomMaintenanceModeArgs::Local => Self::Local,
            RoomMaintenanceModeArgs::Workflow => Self::Workflow,
        }
    }
}

/// 一个 Room 维护语义槽位及其原样交给执行器的 JSON 请求载荷。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomMaintenanceItemArgs {
    #[schemars(description = "本项要维护的唯一语义槽位；一个提交中同一 slot 不得重复。")]
    pub(super) slot: RoomMaintenanceSlotArgs,
    #[schemars(
        description = "随 slot 变化的 JSON 请求载荷，按原值交给 Room 维护执行器校验；序列化后每项最多 64 KiB。Notebook 槽位要求 payload.path 位于 Notebook/ 且以 .md 结尾（最多 240 字节）；Entity 槽位要求 payload.entity 是单个文件名主干（最多 160 字节，不能含 /、\\ 或 NUL，也不能是 . 或 ..）。日记槽位的其余结构由执行器校验。"
    )]
    pub(super) payload: Value,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomMaintenanceSubmitArgs {
    #[schemars(
        length(min = 1, max = 5),
        description = "必填的维护项列表，须有 1–5 项且 slot 互不重复；整组请求会先针对已初始化、干净且可用的 Room 仓库校验，再执行限定在对应语义目标内的维护。"
    )]
    pub(super) items: Vec<RoomMaintenanceItemArgs>,
    #[serde(default)]
    #[schemars(
        description = "可选执行方式覆盖；省略或传 null 时使用 Room 本地配置。local 在通过校验的 Room 仓库中直接应用；workflow 提交给已配置的工作流。"
    )]
    pub(super) mode: Option<RoomMaintenanceModeArgs>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_room_wait_seconds",
        description = "等待 workflow 消费请求及可能的本地快进等待时长，单位秒；省略或传 null 时为 0，范围 0–30。仅 workflow 模式使用此等待，local 模式直接执行。"
    )]
    pub(super) wait_seconds: Option<u8>,
}

impl RoomMaintenanceSubmitArgs {
    pub(super) fn into_protocol(self) -> RoomMaintenanceSubmitRequest {
        RoomMaintenanceSubmitRequest {
            items: self
                .items
                .into_iter()
                .map(|item| RoomMaintenanceRequestItem {
                    slot: item.slot.into(),
                    payload: item.payload,
                })
                .collect(),
            mode: self.mode.map(RoomMaintenanceExecutionMode::from),
            wait_seconds: self.wait_seconds,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct BootstrapReadArgs {
    #[schemars(
        description = "room.bootstrap 返回的引导指南 ID；不存在的 ID 会由 Room 路由报告错误。"
    )]
    pub(super) id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillReadArgs {
    #[schemars(
        description = "Room 技能 ID；可为工作区 skills/ 目录名或内置 skill-installer。内置项只读且不可运行；ID 不能为空、不能为 . 或 ..，且只允许 ASCII 字母、数字、下划线、点和连字符。"
    )]
    pub(super) id: String,
    #[serde(default)]
    #[schemars(
        description = "可选的包内相对资源路径；省略或传 null 时返回旧版 SKILL.md 响应。工作区技能可读取包内资源，内置 skill-installer 仅提供内嵌 SKILL.md。指定路径不得是绝对路径、含 . 或 .. 路径段、反斜杠或 NUL；拒绝符号链接，文件内容上限为 1 MiB。"
    )]
    pub(super) path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillSearchArgs {
    #[schemars(
        description = "查询先去除首尾空白且不能为空，再做不区分大小写的子串匹配；匹配技能 ID、frontmatter、标签和 SKILL.md 内容。"
    )]
    pub(super) query: String,
    #[serde(default)]
    #[schemars(description = "最多返回的技能数；省略或传 null 时为 20，运行时限制在 1–100。")]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillActivationArgs {
    #[schemars(
        description = "要启用或停用的 Room 技能目录 ID；不能为空、不能为 . 或 ..，且只允许 ASCII 字母、数字、下划线、点和连字符。"
    )]
    pub(super) id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillInstallArgs {
    #[schemars(
        description = "目标技能目录 ID：非空，仅含 ASCII 字母、数字、下划线、点或连字符；.、..、以点开头以及保留名 skill-installer 不允许。一次安装只处理一个技能 ID。"
    )]
    pub(super) id: String,
    #[schemars(
        description = "必填的安装来源联合体；type 为 github（仓库）或 files（显式文件列表），两种形式不能混用。"
    )]
    pub(super) source: SkillInstallSourceArgs,
    #[serde(default)]
    #[schemars(
        description = "是否替换已存在技能；省略时为 false。设为 true 会先归档旧技能；若目标已存在而此值为 false，安装请求会被拒绝。"
    )]
    pub(super) replace_existing: bool,
    #[serde(default)]
    #[schemars(
        description = "可选的安装后启用选择；true 会确保启用，false、null 或省略时新技能仍默认启用，替换时保留旧技能原有启用状态。"
    )]
    pub(super) activate_after_install: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "可选幂等键，必须为 1–128 字节；相同键和相同请求仅在任务记录保留期间复用，相同键配不同请求在保留期间返回 idempotency_conflict。终态记录最多保留 7 天且仅保留最近 100 条；记录清理后相同键不再复用或冲突检测，可能发起新安装或因目标已存在而失败。"
    )]
    pub(super) idempotency_key: Option<String>,
}

/// 带 type 标签的安装来源联合体：从 GitHub 仓库读取，或显式提供一组文件。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub(super) enum SkillInstallSourceArgs {
    /// type 为 github。repository 与 url 必须且只能提供一个；ref/path 可覆盖 URL 中解析出的分支/子目录。
    Github {
        #[serde(default)]
        #[schemars(
            description = "GitHub owner/repository 标识；与 url 互斥且必须提供其中之一，例如 octo/demo。"
        )]
        repository: Option<String>,
        #[serde(default)]
        #[schemars(
            description = "HTTPS GitHub URL；主机必须是 github.com，不能带凭据、查询参数或片段；可为仓库根 URL 或 tree/blob 引用 URL，并与 repository 互斥。"
        )]
        url: Option<String>,
        #[serde(rename = "ref", default)]
        #[schemars(
            description = "可选分支、标签或提交引用；覆盖 URL 中解析出的 ref，repository 形式下指定要安装的引用。"
        )]
        ref_name: Option<String>,
        #[serde(default)]
        #[schemars(description = "可选的仓库内子目录/文件路径；覆盖 URL 中解析出的路径。")]
        path: Option<String>,
    },
    /// type 为 files。显式提供的文件必须非空且数量不超过本地技能配置上限。
    Files {
        #[schemars(
            description = "要安装的文件列表；必须非空且不超过本地配置的文件数上限，路径须唯一并且每个文件恰有一种内容来源。"
        )]
        files: Vec<SkillInstallFileArgs>,
    },
}

impl SkillInstallSourceArgs {
    pub(super) fn into_protocol(self) -> SkillInstallSource {
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

/// 单个技能包文件；path 必填，内容来源为 url、content、contentBase64 三者之一。
#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillInstallFileArgs {
    #[schemars(
        description = "包内相对文件路径；不得是绝对路径、含 . 或 .. 路径段、反斜杠或 NUL，最长 240 字节、最多 16 层；同一安装中大小写折叠后不得重复或形成父子路径冲突。"
    )]
    pub(super) path: String,
    #[serde(default)]
    #[schemars(
        description = "HTTPS 文件下载地址；不得含用户名/密码，且须与 content、contentBase64 恰有一个被提供。"
    )]
    pub(super) url: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "作为 UTF-8 文本写入文件的内联内容；与 url、contentBase64 互斥，内联总量受本地技能配置上限限制。"
    )]
    pub(super) content: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Base64 编码的原始文件字节；与 url、content 互斥，解码后的内联总量受本地技能配置上限限制。"
    )]
    pub(super) content_base64: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "可选的预期 SHA-256 十六进制摘要；对最终文件字节校验，比较时不区分大小写，不匹配会使安装失败。"
    )]
    pub(super) sha256: Option<String>,
    #[serde(default)]
    #[schemars(description = "是否将文件标记为可执行；省略或传 null 时为 false。")]
    pub(super) executable: Option<bool>,
}

impl SkillInstallFileArgs {
    pub(super) fn into_protocol(self) -> SkillInstallFile {
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
pub(super) struct SkillInstallGetArgs {
    #[schemars(description = "skills.install 返回的安装任务 ID。")]
    pub(super) install_id: String,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "等待安装状态变化的秒数；省略或传 null 时为 5，范围 0–30，超过 30 时运行时按 30 处理；0 表示不等待。"
    )]
    pub(super) wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillInstallCancelArgs {
    #[schemars(
        description = "skills.install 返回的安装任务 ID；此工具请求在提交点之前协作取消该安装。"
    )]
    pub(super) install_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillRunArgs {
    #[schemars(description = "要运行的 Room 技能 ID；必须是已启用且可运行的工作区技能。")]
    pub(super) id: String,
    #[schemars(
        description = "技能包内 scripts/ 下的相对可执行文件路径；目标必须存在、可执行且不能经过符号链接逃逸。"
    )]
    pub(super) path: String,
    #[serde(default)]
    #[schemars(
        description = "可选工作流分组名，记录在托管进程上；首尾空白会去除，去除后不能为空、最多 32 个 Unicode 字符且不能含控制字符。"
    )]
    pub(super) group: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "传给技能脚本的 argv 字符串数组；每项是独立参数，不按 shell 字符串拆分。省略或传 null 等同空数组。"
    )]
    pub(super) args: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(
        description = "可选进程工作目录；省略或传 null 时使用 Agent 工作区根目录，相对路径从该根目录解析，且必须通过本地路径策略。"
    )]
    pub(super) working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "启动技能脚本后等待完成的内联等待秒数；省略或传 null 时为 5，范围 0–30，超过 30 时运行时按 30 处理；0 表示不等待完成。"
    )]
    pub(super) wait_seconds: Option<u64>,
}
