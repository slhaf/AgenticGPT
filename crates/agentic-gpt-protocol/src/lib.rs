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
pub struct SafeConfigSummary {
    pub workspace_root: String,
    pub sandbox: SafeSandboxSummary,
    pub policy_rule_counts: PolicyCounts,
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationDecision {
    AllowOnce,
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
