use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyCounts {
    pub allow: usize,
    pub confirm: usize,
    pub deny: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeSandboxSummary {
    pub enabled: bool,
    pub mode: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafePathRoot {
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafePathPolicySummary {
    pub write_root_count: usize,
    pub read_only_root_count: usize,
    pub deny_root_count: usize,
    pub write_roots: Vec<SafePathRoot>,
    pub read_only_roots: Vec<SafePathRoot>,
    pub deny_roots: Vec<SafePathRoot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeRule {
    pub program: String,
    pub args_prefix: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeBuiltinPolicyRules {
    pub confirm: Vec<SafeRule>,
    pub deny: Vec<SafeRule>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafePolicyRules {
    pub allow: Vec<SafeRule>,
    pub confirm: Vec<SafeRule>,
    pub deny: Vec<SafeRule>,
    pub builtins: SafeBuiltinPolicyRules,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeConfigSummary {
    pub workspace_root: String,
    pub sandbox: SafeSandboxSummary,
    pub path_policy: SafePathPolicySummary,
    pub policy_rule_counts: PolicyCounts,
    pub policy_rules: SafePolicyRules,
    pub confirmation_provider: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub sessions: bool,
    pub confirmation: bool,
    pub notification_actions: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    Normal,
    Room,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubInfoRemoteConfirmation {
    pub enabled: bool,
    pub provider: String,
    pub timeout_seconds: u64,
    pub ntfy_configured: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubInfoAgents {
    pub registered_count: usize,
    pub enabled_count: usize,
    pub online_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubInfoCounts {
    pub pending_request_count: usize,
    pub pending_confirmation_count: usize,
    pub cached_session_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubInfoResponse {
    pub service: String,
    pub version: String,
    pub public_base_url: Option<String>,
    pub request_timeout_seconds: u64,
    pub max_wait_seconds: u64,
    pub remote_confirmation: HubInfoRemoteConfirmation,
    pub agents: HubInfoAgents,
    pub counts: HubInfoCounts,
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRegistryEntry {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub display_name: String,
    pub enabled: bool,
    pub secret_hash: String,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecRequest {
    pub agent_id: String,
    pub program: String,
    pub args: Vec<String>,
    pub need_confirm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecElement {
    pub program: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExecRequest {
    pub agent_id: String,
    pub elements: Vec<ExecElement>,
    pub need_confirm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSummary {
    pub id: String,
    pub enabled: bool,
    pub transport: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpListServersRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpListToolsRequest {
    pub agent_id: String,
    pub server_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCallToolRequest {
    pub agent_id: String,
    pub server_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationChannel {
    pub key: String,
    pub display_name: String,
    pub available: bool,
    pub kind: String,
    pub supports_actions: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationAction {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifySendRequest {
    pub channel_key: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifySendResponse {
    pub channel_key: String,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifyDeliveryRequest {
    pub channel_key: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub actions: Vec<NotificationAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserNotifyDeliveryResponse {
    pub channel_key: String,
    pub delivered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PassageSignificance {
    Normal,
    Anchor,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Passage {
    pub id: String,
    pub datetime: DateTime<Utc>,
    pub scope: String,
    pub significance: PassageSignificance,
    #[serde(rename = "abstract")]
    pub abstract_text: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PassagePreview {
    pub id: String,
    pub datetime: DateTime<Utc>,
    pub scope: String,
    pub significance: PassageSignificance,
    #[serde(rename = "abstract")]
    pub abstract_text: String,
    pub content_preview: String,
    pub tags: Vec<String>,
    pub display_mode: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookCurrent {
    pub scope: String,
    pub updated_at: DateTime<Utc>,
    pub source_passage_id: String,
    #[serde(rename = "abstract")]
    pub abstract_text: String,
    pub content: String,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookAppendRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datetime: Option<DateTime<Utc>>,
    pub scope: String,
    pub significance: PassageSignificance,
    #[serde(rename = "abstract")]
    pub abstract_text: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookAppendResponse {
    pub id: String,
    pub path: String,
    pub created: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookRecentRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub significance: Option<PassageSignificance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookSelectExactRequest {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookSearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookCurrentRequest {
    pub scope: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookUpdateRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub significance: Option<PassageSignificance>,
    #[serde(rename = "abstract", default, skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookUpdateResponse {
    pub updated: bool,
    pub id: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookRemoveRequest {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookRemoveResponse {
    pub removed: bool,
    pub id: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookPassagesResponse {
    pub passages: Vec<PassagePreview>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookCurrentResponse {
    pub current: Option<NotebookCurrent>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryEntry {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub date: String,
    pub time_hint: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub entry: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryAppendRequest {
    #[serde(default)]
    pub time_hint: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub entry: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryAppendResponse {
    pub id: String,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub date: String,
    pub created: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryRecentRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiarySelectExactRequest {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiaryEntriesResponse {
    pub entries: Vec<DiaryEntry>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub agent_id: String,
    pub task_id: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchElementResult {
    pub index: usize,
    pub program: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    pub result: TaskResult,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExecResult {
    pub agent_id: String,
    pub batch_id: String,
    pub status: String,
    pub results: Vec<BatchElementResult>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub agent_id: String,
    pub session_id: String,
    pub state: String,
    pub program: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    pub command_preview: String,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationPayload {
    pub program: String,
    pub args: Vec<String>,
    pub command_preview: String,
    pub risk_level: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxListPanesRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxCapturePaneRequest {
    pub target: String,
    #[serde(default = "default_tmux_capture_lines")]
    pub lines: u32,
}

fn default_tmux_capture_lines() -> u32 {
    160
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxPasteTextRequest {
    pub target: String,
    pub text: String,
    #[serde(default)]
    pub submit: bool,
    #[serde(default = "default_true")]
    pub need_confirm: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxExecRequest {
    pub target: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub need_confirm: bool,
    #[serde(default = "default_tmux_exec_wait_ms")]
    pub wait_ms: u64,
    #[serde(default = "default_tmux_exec_capture_lines")]
    pub capture_lines: u32,
}

fn default_tmux_exec_wait_ms() -> u64 {
    300
}

fn default_tmux_exec_capture_lines() -> u32 {
    120
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxCreateSessionRequest {
    pub name: String,
    pub cwd: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxCloseSessionRequest {
    pub name: String,
    #[serde(default = "default_true")]
    pub need_confirm: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationDecision {
    AllowOnce,
    #[serde(rename = "allow_mcp_server_15m")]
    AllowMcpServer15m,
    #[serde(rename = "allow_mcp_server_30m")]
    AllowMcpServer30m,
    Deny,
    Timeout,
    ProviderUnavailable,
    CallbackTokenInvalid,
    Expired,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HubCommand {
    #[serde(rename = "exec")]
    Exec {
        request_id: String,
        task_id: String,
        payload: ExecRequest,
    },
    #[serde(rename = "batchExec")]
    BatchExec {
        request_id: String,
        task_id: String,
        payload: BatchExecRequest,
    },
    #[serde(rename = "startSession")]
    StartSession {
        request_id: String,
        session_id: String,
        payload: ExecRequest,
    },
    #[serde(rename = "listSessions")]
    ListSessions { request_id: String },
    #[serde(rename = "inspectSession")]
    InspectSession {
        request_id: String,
        session_id: String,
    },
    #[serde(rename = "waitSession")]
    WaitSession {
        request_id: String,
        session_id: String,
        seconds: u64,
    },
    #[serde(rename = "killSession")]
    KillSession {
        request_id: String,
        session_id: String,
    },
    #[serde(rename = "tmux.listSessions")]
    TmuxListSessions { request_id: String },
    #[serde(rename = "tmux.listPanes")]
    TmuxListPanes {
        request_id: String,
        payload: TmuxListPanesRequest,
    },
    #[serde(rename = "tmux.capturePane")]
    TmuxCapturePane {
        request_id: String,
        payload: TmuxCapturePaneRequest,
    },
    #[serde(rename = "tmux.pasteText")]
    TmuxPasteText {
        request_id: String,
        payload: TmuxPasteTextRequest,
    },
    #[serde(rename = "tmux.exec")]
    TmuxExec {
        request_id: String,
        payload: TmuxExecRequest,
    },
    #[serde(rename = "tmux.createSession")]
    TmuxCreateSession {
        request_id: String,
        payload: TmuxCreateSessionRequest,
    },
    #[serde(rename = "tmux.closeSession")]
    TmuxCloseSession {
        request_id: String,
        payload: TmuxCloseSessionRequest,
    },
    #[serde(rename = "mcpListServers")]
    McpListServers { request_id: String },
    #[serde(rename = "mcpListTools")]
    McpListTools {
        request_id: String,
        payload: McpListToolsRequest,
    },
    #[serde(rename = "mcpCallTool")]
    McpCallTool {
        request_id: String,
        payload: McpCallToolRequest,
    },
    #[serde(rename = "user.notify.deliver")]
    UserNotifyDeliver {
        request_id: String,
        payload: UserNotifyDeliveryRequest,
    },
    #[serde(rename = "room.notebook.append")]
    RoomNotebookAppend {
        request_id: String,
        payload: NotebookAppendRequest,
    },
    #[serde(rename = "room.notebook.recent")]
    RoomNotebookRecent {
        request_id: String,
        payload: NotebookRecentRequest,
    },
    #[serde(rename = "room.notebook.selectExact")]
    RoomNotebookSelectExact {
        request_id: String,
        payload: NotebookSelectExactRequest,
    },
    #[serde(rename = "room.notebook.search")]
    RoomNotebookSearch {
        request_id: String,
        payload: NotebookSearchRequest,
    },
    #[serde(rename = "room.notebook.current")]
    RoomNotebookCurrent {
        request_id: String,
        payload: NotebookCurrentRequest,
    },
    #[serde(rename = "room.notebook.update")]
    RoomNotebookUpdate {
        request_id: String,
        payload: NotebookUpdateRequest,
    },
    #[serde(rename = "room.notebook.remove")]
    RoomNotebookRemove {
        request_id: String,
        payload: NotebookRemoveRequest,
    },
    #[serde(rename = "room.diary.append")]
    RoomDiaryAppend {
        request_id: String,
        payload: DiaryAppendRequest,
    },
    #[serde(rename = "room.diary.recent")]
    RoomDiaryRecent {
        request_id: String,
        payload: DiaryRecentRequest,
    },
    #[serde(rename = "room.diary.selectExact")]
    RoomDiarySelectExact {
        request_id: String,
        payload: DiarySelectExactRequest,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Hello {
        role: AgentRole,
        #[serde(rename = "configSummary")]
        config_summary: SafeConfigSummary,
        #[serde(default, rename = "notificationChannels")]
        notification_channels: Vec<NotificationChannel>,
    },
    Heartbeat {
        #[serde(rename = "sentAt")]
        sent_at: DateTime<Utc>,
    },
    SessionUpdate {
        session: SessionInfo,
    },
    Response {
        #[serde(rename = "requestId")]
        request_id: String,
        data: serde_json::Value,
    },
    ConfirmationRequest {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "timeoutSeconds")]
        timeout_seconds: u64,
        payload: ConfirmationPayload,
    },
}

#[cfg(test)]
mod tmux_tests {
    use super::*;

    #[test]
    fn paste_and_close_default_to_confirmation() {
        let paste: TmuxPasteTextRequest = serde_json::from_value(serde_json::json!({
            "target": "%0",
            "text": "status"
        }))
        .unwrap();
        let close: TmuxCloseSessionRequest =
            serde_json::from_value(serde_json::json!({ "name": "agentic" })).unwrap();
        assert!(paste.need_confirm);
        assert!(close.need_confirm);
        assert!(!paste.submit);
    }

    #[test]
    fn tmux_exec_defaults_to_structured_non_forced_confirmation_request() {
        let request: TmuxExecRequest = serde_json::from_value(serde_json::json!({
            "target": "%0",
            "program": "git",
            "args": ["status"]
        }))
        .unwrap();
        assert_eq!(request.program, "git");
        assert_eq!(request.args, ["status"]);
        assert!(!request.need_confirm);
        assert_eq!(request.wait_ms, 300);
        assert_eq!(request.capture_lines, 120);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HubMessage {
    HeartbeatAck {
        #[serde(rename = "sentAt")]
        sent_at: DateTime<Utc>,
        #[serde(rename = "receivedAt")]
        received_at: DateTime<Utc>,
    },
    ConfirmationResponse {
        #[serde(rename = "requestId")]
        request_id: String,
        decision: ConfirmationDecision,
        reason: String,
    },
}
