use agentic_gpt_protocol::{
    McpBatchMode, NotificationAction, RoomDiaryLayer, RoomMaintenanceExecutionMode,
    RoomMaintenanceRequestItem, RoomMaintenanceSlot, RoomMaintenanceSubmitRequest,
    SkillInstallFile, SkillInstallSource,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    default_job_list_limit, default_job_wait_seconds, default_room_notebook_limit,
    default_room_wait_seconds, default_standard_wait_seconds, default_wait_only,
};

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentIdArgs {
    #[schemars(
        description = "Target local agent id. Room notebook tools do not use agentId; they route to the active Room Agent."
    )]
    pub(super) agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct HubRunGetArgs {
    #[schemars(description = "Run id returned by a timed-out Hub request.")]
    pub(super) run_id: String,
}

#[derive(Debug, Default, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct HubRunListArgs {
    #[serde(default)]
    #[schemars(description = "Optional agent id filter.")]
    pub(super) agent_id: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional source filter such as hub or tunnel.")]
    pub(super) source: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional status filter such as started, completed, failed, or timeout_waiting_result."
    )]
    pub(super) status: Option<String>,
    #[serde(default)]
    #[schemars(description = "Only include records created within this many seconds.")]
    pub(super) since_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(description = "Result count, default 20 and capped at 100.")]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(description = "Optional human-readable workstream key inherited by the Job.")]
    pub(super) group: Option<String>,
    #[schemars(
        description = "Executable name or path. For shell syntax, use bash or sh with args such as ['-lc', '...']."
    )]
    pub(super) program: String,
    #[serde(default)]
    #[schemars(
        description = "Argument vector passed directly to the program; this is not a shell-split string."
    )]
    pub(super) args: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(
        description = "Request confirmation before execution. Local policy may still allow, confirm, or deny regardless of this flag."
    )]
    pub(super) need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional per-request confirmation provider override. Omit or use default to follow local agent config."
    )]
    pub(super) confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Process working directory. Relative values resolve from the agent workspace root; prefer this over cd in shell commands."
    )]
    pub(super) working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    pub(super) wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct BatchExecArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional human-readable workstream key inherited by every child Job."
    )]
    pub(super) group: Option<String>,
    #[schemars(
        description = "Commands to run. Each element can override the top-level workingDirectory."
    )]
    pub(super) elements: Vec<BatchExecElementArgs>,
    #[serde(default)]
    #[schemars(
        description = "Request confirmation for the batch. Local policy may still allow, confirm, or deny regardless of this flag."
    )]
    pub(super) need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional per-request confirmation provider override for all batch elements."
    )]
    pub(super) confirm_method: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Default process working directory for all batch elements. Relative values resolve from the agent workspace root."
    )]
    pub(super) working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    pub(super) wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct BatchExecElementArgs {
    #[schemars(description = "Executable name or path for this batch element.")]
    pub(super) program: String,
    #[serde(default)]
    #[schemars(description = "Argument vector passed directly to the program.")]
    pub(super) args: Option<Vec<String>>,
    #[serde(default)]
    #[schemars(
        description = "Per-element process working directory. Overrides the batch workingDirectory."
    )]
    pub(super) working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct JobIdArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(
        description = "Managed Job id returned by process.exec, process.batch, or skills.run."
    )]
    pub(super) job_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct JobGetArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(description = "Managed Job id.")]
    pub(super) job_id: String,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_job_wait_seconds",
        description = "Bounded wait in seconds; defaults to 5 and is capped at 30."
    )]
    pub(super) wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        default = "default_wait_only",
        description = "While waiting, suppress active intermediate detail; defaults to false; terminal completion still returns normal detail."
    )]
    pub(super) wait_only: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct JobListArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(description = "Exact human-readable workstream filter.")]
    pub(super) group: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional Job kind: process, skill, or mcp.")]
    pub(super) kind: Option<String>,
    #[serde(default)]
    #[schemars(description = "Optional Job state filter.")]
    pub(super) state: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_job_list_limit",
        description = "Maximum retained Jobs; defaults to 50 and is capped at 100."
    )]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    #[schemars(description = "Opaque cursor returned by a prior job.list response.")]
    pub(super) cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxListSessionsArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxListPanesArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(description = "Optional tmux session name to scope pane listing.")]
    pub(super) session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxCapturePaneArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(description = "tmux target such as session:window.pane or a pane id like %0.")]
    pub(super) target: String,
    #[serde(default)]
    #[schemars(
        description = "Number of recent tmux history lines to capture. Defaults to 160 and caps at 5000."
    )]
    pub(super) lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxPasteTextArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(description = "tmux target such as session:window.pane or a pane id like %0.")]
    pub(super) target: String,
    #[schemars(description = "Text to paste into the tmux pane.")]
    pub(super) text: String,
    #[serde(default)]
    #[schemars(description = "Append Enter after pasting the text. Defaults to false.")]
    pub(super) submit: Option<bool>,
    #[serde(default)]
    #[schemars(description = "Request local confirmation before pasting. Defaults to true.")]
    pub(super) need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxExecArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(description = "Shell pane target such as session:window.pane or %0.")]
    pub(super) target: String,
    #[schemars(description = "Program or shell builtin to execute as one command.")]
    pub(super) program: String,
    #[serde(default)]
    #[schemars(description = "Structured argument vector; shell operators are not interpreted.")]
    pub(super) args: Vec<String>,
    #[serde(default)]
    #[schemars(description = "Force local confirmation in addition to configured policy.")]
    pub(super) need_confirm: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Milliseconds to wait before returning the post-submit pane snapshot. Defaults to 300 and caps at 5000."
    )]
    pub(super) wait_ms: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Number of tmux history lines to include in the post-submit snapshot. Defaults to 120, caps at 5000, and 0 disables the snapshot."
    )]
    pub(super) capture_lines: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxCreateSessionArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(description = "tmux session name.")]
    pub(super) name: String,
    #[schemars(description = "Session cwd, subject to the local agent path policy.")]
    pub(super) cwd: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TmuxCloseSessionArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(description = "tmux session name.")]
    pub(super) name: String,
    #[serde(default)]
    #[schemars(description = "Request local confirmation before closing. Defaults to true.")]
    pub(super) need_confirm: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpListServersArgs {
    #[serde(default)]
    #[schemars(
        description = "Optional target local agent id. Omit to list MCP servers for all currently connected agents."
    )]
    pub(super) agent_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpListToolsArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[schemars(description = "MCP server id returned by mcp.listServers.")]
    pub(super) server_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpCallToolArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(description = "Optional human-readable workstream key inherited by the Job.")]
    pub(super) group: Option<String>,
    #[schemars(description = "MCP server id returned by mcp.listServers.")]
    pub(super) server_id: String,
    #[schemars(description = "Tool name returned by mcp.listTools.")]
    pub(super) tool_name: String,
    #[serde(default)]
    #[schemars(
        description = "JSON object arguments forwarded to the MCP tool; maximum serialized size 256 KiB."
    )]
    pub(super) arguments: Option<Value>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    pub(super) wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Absolute downstream execution deadline in seconds, default 300 and capped at 900."
    )]
    pub(super) timeout_seconds: Option<u64>,
}

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

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpBatchCallArgs {
    #[schemars(description = "Configured MCP server id.")]
    pub(super) server_id: String,
    #[schemars(description = "Downstream MCP tool name.")]
    pub(super) tool_name: String,
    #[serde(default)]
    #[schemars(description = "JSON object arguments; maximum serialized size 256 KiB per call.")]
    pub(super) arguments: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct McpBatchArgs {
    #[schemars(description = "Target local agent id.")]
    pub(super) agent_id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional human-readable workstream key inherited by every child Job."
    )]
    pub(super) group: Option<String>,
    #[schemars(
        length(min = 1, max = 16),
        description = "Ordered 1..16 downstream MCP calls; aggregate serialized arguments are capped at 2 MiB."
    )]
    pub(super) calls: Vec<McpBatchCallArgs>,
    #[serde(default)]
    #[schemars(description = "Execution mode: parallel by default, or sequential.")]
    pub(super) mode: Option<McpBatchModeArgs>,
    #[serde(default)]
    #[schemars(
        description = "When true, prevent not-yet-started children from starting after a hard child failure; already-started calls are never cancelled."
    )]
    pub(super) fail_fast: Option<bool>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    pub(super) wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 900),
        description = "Per-child downstream execution deadline in seconds after scheduling, default 300 and capped at 900."
    )]
    pub(super) timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserNotifySendArgs {
    #[schemars(description = "Notification channel key returned by user.notify.channels.")]
    pub(super) channel: String,
    #[schemars(description = "Notification title.")]
    pub(super) title: String,
    #[schemars(description = "Notification body.")]
    pub(super) body: String,
    #[serde(default)]
    #[schemars(description = "Optional notification actions. Phase A does not deliver actions.")]
    pub(super) actions: Option<Vec<UserNotifyActionArgs>>,
    #[serde(default)]
    #[schemars(description = "Optional priority such as low, normal, high, urgent, or alarm.")]
    pub(super) priority: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct UserNotifyActionArgs {
    #[schemars(description = "Stable action id. Android ack will report this as actionId.")]
    pub(super) id: String,
    #[schemars(description = "Human-readable action label.")]
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

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomDiaryActiveArgs {}

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
    #[schemars(description = "Room diary temporal layer.")]
    pub(super) layer: RoomDiaryLayerArgs,
    #[schemars(
        pattern(r"^(current|\d{4}-\d{2}-\d{2}(--\d{4}-\d{2}-\d{2})?)$"),
        description = "Room-local logical period: daily uses current or YYYY-MM-DD; weekly/monthly use current or YYYY-MM-DD--YYYY-MM-DD."
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
        description = "Maximum bounded recent Notebook previews returned; defaults to 20."
    )]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomNotebookSearchArgs {
    #[schemars(
        length(min = 1, max = 256),
        description = "Case-insensitive bounded substring query over Notebook paths, H1 titles, and bodies."
    )]
    pub(super) query: String,
    #[serde(default)]
    #[schemars(
        range(min = 1, max = 100),
        default = "default_room_notebook_limit",
        description = "Maximum bounded Notebook previews returned; defaults to 20."
    )]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomNotebookReadArgs {
    #[schemars(
        description = "Exact Notebook-relative Markdown path returned or discovered under Notebook/; arbitrary repository paths are rejected."
    )]
    pub(super) path: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomStateListArgs {}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomStateReadArgs {
    #[schemars(
        description = "State entity filename stem resolved under State/entities/; arbitrary repository paths are rejected."
    )]
    pub(super) entity: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomMaintenanceStatusArgs {}

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

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomMaintenanceItemArgs {
    #[schemars(description = "Unique Room semantic slot to maintain.")]
    pub(super) slot: RoomMaintenanceSlotArgs,
    #[schemars(
        description = "Slot-specific maintenance payload; validated by the Room maintenance executor."
    )]
    pub(super) payload: Value,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RoomMaintenanceSubmitArgs {
    #[schemars(
        length(min = 1, max = 5),
        description = "One to five maintenance requests; each slot may appear at most once. The set is validated against the Room repository before any mutation."
    )]
    pub(super) items: Vec<RoomMaintenanceItemArgs>,
    #[serde(default)]
    #[schemars(
        description = "Optional execution mode override; local applies in the validated Room repository, workflow submits through the configured Room workflow."
    )]
    pub(super) mode: Option<RoomMaintenanceModeArgs>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_room_wait_seconds",
        description = "Optional bounded wait for workflow consumption and local fast-forward, from 0 through 30 seconds."
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
    #[schemars(description = "Guide id returned by room.bootstrap.")]
    pub(super) id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillReadArgs {
    #[schemars(description = "Skill id, matching one workspace skills/ directory name.")]
    pub(super) id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional package-relative file path. Omit to read the legacy SKILL.md response."
    )]
    pub(super) path: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillSearchArgs {
    #[schemars(
        description = "Case-insensitive substring query over id, frontmatter, tags, and SKILL.md content."
    )]
    pub(super) query: String,
    #[serde(default)]
    #[schemars(description = "Maximum skills returned. Defaults to 20 and caps at 100.")]
    pub(super) limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillActivationArgs {
    #[schemars(description = "Skill id, matching one workspace skills/ directory name.")]
    pub(super) id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillInstallArgs {
    #[schemars(description = "Target skill id. One installation job targets exactly one id.")]
    pub(super) id: String,
    #[schemars(description = "GitHub, HTTPS-file, or inline-content source descriptor.")]
    pub(super) source: SkillInstallSourceArgs,
    #[serde(default)]
    #[schemars(
        description = "Archive an existing workspace skill before replacement. Defaults to false."
    )]
    pub(super) replace_existing: bool,
    #[serde(default)]
    #[schemars(
        description = "Optional explicit activation choice; new skills default active and replacement preserves its prior state."
    )]
    pub(super) activate_after_install: Option<bool>,
    #[serde(default)]
    #[schemars(
        description = "Optional idempotency key for safe retries of the same install request."
    )]
    pub(super) idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub(super) enum SkillInstallSourceArgs {
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

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillInstallFileArgs {
    pub(super) path: String,
    #[serde(default)]
    pub(super) url: Option<String>,
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) content_base64: Option<String>,
    #[serde(default)]
    pub(super) sha256: Option<String>,
    #[serde(default)]
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
    pub(super) install_id: String,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded status wait in seconds; defaults to 5 and is capped at 30."
    )]
    pub(super) wait_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillInstallCancelArgs {
    pub(super) install_id: String,
}

#[derive(Debug, Deserialize, Serialize, rmcp::schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct SkillRunArgs {
    pub(super) id: String,
    #[schemars(description = "Package-relative executable path under scripts/.")]
    pub(super) path: String,
    #[serde(default)]
    #[schemars(description = "Optional human-readable workstream key inherited by the Job.")]
    pub(super) group: Option<String>,
    #[serde(default)]
    pub(super) args: Option<Vec<String>>,
    #[serde(default)]
    pub(super) working_directory: Option<String>,
    #[serde(default)]
    #[schemars(
        range(min = 0, max = 30),
        default = "default_standard_wait_seconds",
        description = "Bounded inline wait in seconds; defaults to 5 and is capped at 30."
    )]
    pub(super) wait_seconds: Option<u64>,
}
