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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecElement {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExecRequest {
    pub agent_id: String,
    pub elements: Vec<ExecElement>,
    pub need_confirm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_method: Option<String>,
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
    pub agent_id: String,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Hello {
        #[serde(rename = "configSummary")]
        config_summary: SafeConfigSummary,
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
