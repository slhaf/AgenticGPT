use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::process_jobs::JobInfo;

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
pub struct SafeTunnelSummary {
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_source: Option<String>,
    pub client_source: String,
    pub hub_reporting_enabled: bool,
    pub reporting_detail: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<SafeTunnelSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub jobs: bool,
    pub confirmation: bool,
    pub notification_actions: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    Normal,
    Room,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentConnectionMode {
    #[default]
    CommandCapable,
    ReportingOnly,
}

impl AgentConnectionMode {
    pub fn as_str(self) -> &'static str {
        self.label()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::CommandCapable => "command_capable",
            Self::ReportingOnly => "reporting_only",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundedJsonValue {
    pub value: serde_json::Value,
    pub byte_count: usize,
    pub sha256: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunReport {
    pub run_id: String,
    pub request_id: String,
    pub tool_name: String,
    pub source: String,
    pub profile: String,
    pub detail: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<BoundedJsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<BoundedJsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobInfo>,
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
    pub cached_job_count: usize,
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
