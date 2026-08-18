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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecRequest {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub program: String,
    pub args: Vec<String>,
    pub need_confirm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
}

impl ExecRequest {
    pub const DEFAULT_WAIT_SECONDS: u64 = 5;
    pub const MAX_WAIT_SECONDS: u64 = 30;

    pub fn effective_wait_seconds(&self) -> u64 {
        self.wait_seconds
            .unwrap_or(Self::DEFAULT_WAIT_SECONDS)
            .min(Self::MAX_WAIT_SECONDS)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub elements: Vec<ExecElement>,
    pub need_confirm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
}

impl BatchExecRequest {
    pub const DEFAULT_WAIT_SECONDS: u64 = 5;
    pub const MAX_WAIT_SECONDS: u64 = 30;

    pub fn effective_wait_seconds(&self) -> u64 {
        self.wait_seconds
            .unwrap_or(Self::DEFAULT_WAIT_SECONDS)
            .min(Self::MAX_WAIT_SECONDS)
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub server_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

impl McpCallToolRequest {
    pub const DEFAULT_WAIT_SECONDS: u64 = 5;
    pub const MAX_WAIT_SECONDS: u64 = 30;
    pub const DEFAULT_TIMEOUT_SECONDS: u64 = 300;
    pub const MIN_TIMEOUT_SECONDS: u64 = 1;
    pub const MAX_TIMEOUT_SECONDS: u64 = 900;

    pub fn effective_wait_seconds(&self) -> u64 {
        self.wait_seconds
            .unwrap_or(Self::DEFAULT_WAIT_SECONDS)
            .min(Self::MAX_WAIT_SECONDS)
    }

    pub fn effective_timeout_seconds(&self) -> u64 {
        self.timeout_seconds
            .unwrap_or(Self::DEFAULT_TIMEOUT_SECONDS)
            .clamp(Self::MIN_TIMEOUT_SECONDS, Self::MAX_TIMEOUT_SECONDS)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpBatchMode {
    #[default]
    Parallel,
    Sequential,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub server_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchRequest {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub calls: Vec<McpBatchCall>,
    #[serde(default)]
    pub mode: McpBatchMode,
    #[serde(default)]
    pub fail_fast: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

impl McpBatchRequest {
    pub const MIN_CALLS: usize = 1;
    pub const MAX_CALLS: usize = 16;
    pub const MAX_AGGREGATE_ARGUMENT_BYTES: usize = 2 * 1024 * 1024;
    pub const MAX_AGGREGATE_RESULT_BYTES: usize = 2 * 1024 * 1024;

    pub fn effective_wait_seconds(&self) -> u64 {
        self.wait_seconds
            .unwrap_or(McpCallToolRequest::DEFAULT_WAIT_SECONDS)
            .min(McpCallToolRequest::MAX_WAIT_SECONDS)
    }

    pub fn effective_timeout_seconds(&self) -> u64 {
        self.timeout_seconds
            .unwrap_or(McpCallToolRequest::DEFAULT_TIMEOUT_SECONDS)
            .clamp(
                McpCallToolRequest::MIN_TIMEOUT_SECONDS,
                McpCallToolRequest::MAX_TIMEOUT_SECONDS,
            )
    }
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapDocumentKind {
    Entrypoint,
    Guide,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapLoadPolicy {
    Startup,
    Contextual,
    OnDemand,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapEncoding {
    Utf8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapTextResource {
    pub path: String,
    pub encoding: BootstrapEncoding,
    pub content: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub returned_size_bytes: u64,
    pub total_lines: u64,
    pub returned_through_line: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omitted_from_line: Option<u64>,
    pub truncated: bool,
    pub last_line_complete: bool,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapEntrypoint {
    pub id: String,
    pub kind: BootstrapDocumentKind,
    pub name: String,
    pub description: String,
    pub frontmatter: serde_json::Value,
    pub resource: BootstrapTextResource,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapGuideSummary {
    pub id: String,
    pub kind: BootstrapDocumentKind,
    pub title: String,
    pub summary: String,
    pub load_policy: BootstrapLoadPolicy,
    pub priority: i32,
    pub load_when: Vec<String>,
    pub tool_bindings: Vec<String>,
    pub tags: Vec<String>,
    pub path: String,
    pub size_bytes: u64,
    pub total_lines: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResponse {
    pub schema_version: u32,
    pub revision: String,
    pub entrypoint: BootstrapEntrypoint,
    pub guides: Vec<BootstrapGuideSummary>,
    pub total_guides: usize,
    pub returned_guides: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapReadRequest {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapReadResponse {
    pub guide: BootstrapGuideSummary,
    pub frontmatter: serde_json::Value,
    pub resource: BootstrapTextResource,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillReadRequest {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillActivationRequest {
    pub id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageSummary {
    pub has_assets: bool,
    pub has_scripts: bool,
    pub has_references: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub active: bool,
    #[serde(default)]
    pub origin: SkillOrigin,
    #[serde(default)]
    pub read_only: bool,
    pub package_summary: SkillPackageSummary,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    #[default]
    Workspace,
    Builtin,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub id: String,
    pub skill_md: String,
    pub frontmatter: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub active: bool,
    #[serde(default)]
    pub origin: SkillOrigin,
    #[serde(default)]
    pub read_only: bool,
    pub package_summary: SkillPackageSummary,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSkill {
    pub id: String,
    pub activated_at: DateTime<Utc>,
    pub status: String,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SkillSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsListResponse {
    pub skills: Vec<SkillSummary>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillReadResponse {
    pub skill: SkillDetail,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<SkillResource>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillResource {
    pub path: String,
    pub encoding: SkillResourceEncoding,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillResourceEncoding {
    Utf8,
    Base64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallRequest {
    pub id: String,
    pub source: SkillInstallSource,
    #[serde(default)]
    pub replace_existing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activate_after_install: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum SkillInstallSource {
    Github {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repository: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
        ref_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Files {
        files: Vec<SkillInstallFile>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillInstallStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillInstallPhase {
    Resolving,
    Downloading,
    Extracting,
    Validating,
    WaitingForTarget,
    Committing,
    Activating,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallProgress {
    pub files_completed: u64,
    pub files_total: u64,
    pub bytes_downloaded: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallFileSummary {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub source_type: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallSourceSummary {
    pub source_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub files: Vec<SkillInstallFileSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallResult {
    pub skill: SkillSummary,
    pub source: SkillInstallSourceSummary,
    pub package_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<SkillInstallPhase>,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallStartResponse {
    pub install_id: String,
    pub id: String,
    pub status: SkillInstallStatus,
    pub queued: bool,
    pub deduplicated: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub poll_after_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallGetRequest {
    pub install_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
}

impl SkillInstallGetRequest {
    pub const DEFAULT_WAIT_SECONDS: u64 = 5;
    pub const MAX_WAIT_SECONDS: u64 = 30;

    pub fn effective_wait_seconds(&self) -> u64 {
        self.wait_seconds.unwrap_or(Self::DEFAULT_WAIT_SECONDS)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallStatusResponse {
    pub install_id: String,
    pub id: String,
    pub revision: u64,
    pub status: SkillInstallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<SkillInstallPhase>,
    pub attempt: u32,
    pub max_attempts: u32,
    pub progress: SkillInstallProgress,
    pub source: SkillInstallSourceSummary,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_requested_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SkillInstallResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SkillInstallError>,
    pub poll_after_ms: u64,
}

pub const SKILL_INSTALL_JOB_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallJobRecord {
    pub schema_version: u32,
    pub install_id: String,
    pub request: SkillInstallRequest,
    pub canonical_request_sha256: String,
    pub status: SkillInstallStatusResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallCancelRequest {
    pub install_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillInstallCancelOutcome {
    CancelRequested,
    Cancelled,
    AlreadyCancelled,
    TooLate,
    AlreadyTerminal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallCancelResponse {
    pub install_id: String,
    pub outcome: SkillInstallCancelOutcome,
    pub changed: bool,
    pub status: SkillInstallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<SkillInstallPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_requested_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRunRequest {
    pub id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
}

impl SkillRunRequest {
    pub const DEFAULT_WAIT_SECONDS: u64 = 5;
    pub const MAX_WAIT_SECONDS: u64 = 30;

    pub fn effective_wait_seconds(&self) -> u64 {
        self.wait_seconds.unwrap_or(Self::DEFAULT_WAIT_SECONDS)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsSearchResponse {
    pub skills: Vec<SkillSummary>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsActiveResponse {
    pub active_skills: Vec<ActiveSkill>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillActivationResponse {
    pub id: String,
    pub active: bool,
    pub changed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Process,
    Skill,
    Mcp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    WaitingConfirmation,
    Starting,
    Running,
    Completed,
    Failed,
    Rejected,
    CancelRequested,
    Cancelled,
    TimedOut,
    Detached,
    UnknownAfterRestart,
    Skipped,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        self.label()
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::WaitingConfirmation
                | Self::Starting
                | Self::Running
                | Self::CancelRequested
        )
    }

    pub fn is_terminal(self) -> bool {
        !self.is_active()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::WaitingConfirmation => "waiting_confirmation",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
            Self::CancelRequested => "cancel_requested",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Detached => "detached",
            Self::UnknownAfterRestart => "unknown_after_restart",
            Self::Skipped => "skipped",
        }
    }
}

impl std::fmt::Display for JobState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}

pub const JOB_GROUP_MAX_CHARS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobGroupValidationError {
    Empty,
    TooLong,
    ControlCharacter,
}

impl JobGroupValidationError {
    pub fn code(self) -> &'static str {
        "job_group_invalid"
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "group must not be empty after trimming",
            Self::TooLong => "group must contain at most 32 Unicode characters",
            Self::ControlCharacter => "group must not contain control characters",
        }
    }
}

impl std::fmt::Display for JobGroupValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for JobGroupValidationError {}

pub fn normalize_job_group(group: Option<&str>) -> Result<Option<String>, JobGroupValidationError> {
    let Some(group) = group else {
        return Ok(None);
    };
    let trimmed = group.trim();
    if trimmed.is_empty() {
        return Err(JobGroupValidationError::Empty);
    }
    if trimmed.chars().count() > JOB_GROUP_MAX_CHARS {
        return Err(JobGroupValidationError::TooLong);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(JobGroupValidationError::ControlCharacter);
    }
    Ok(Some(trimmed.to_string()))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobInfo {
    pub agent_id: String,
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_index: Option<usize>,
    pub kind: JobKind,
    pub state: JobState,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout_tail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr_tail: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_tool_name: Option<String>,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_evidence: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobError {
    pub code: String,
    pub message: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobToolResponse {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<JobKind>,
    pub state: JobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout_tail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr_tail: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub result_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub result_omitted: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobWaitResponse {
    pub job_id: String,
    pub state: JobState,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListItem {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub kind: JobKind,
    pub state: JobState,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListResponse {
    pub jobs: Vec<JobListItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCancelResponse {
    pub job_id: String,
    pub state: JobState,
    pub cancel_outcome: String,
    pub termination_evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobBatchToolResponse {
    pub batch_id: String,
    pub status: String,
    pub jobs: Vec<JobToolResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchToolChildResponse {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub job: JobToolResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchToolResponse {
    pub batch_id: String,
    pub status: McpBatchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
    pub results: Vec<McpBatchToolChildResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDetail {
    pub job: JobInfo,
    pub detail_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
    #[serde(default)]
    pub result_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResponse {
    pub status: JobState,
    pub completed_inline: bool,
    pub job_id: String,
    pub poll_after_ms: u64,
    #[serde(flatten)]
    pub detail: JobDetail,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpBatchStatus {
    Running,
    Completed,
    CompletedWithErrors,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchChildResponse {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub result_omitted: bool,
    #[serde(flatten)]
    pub detail: JobDetail,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchResponse {
    pub batch_id: String,
    pub status: McpBatchStatus,
    pub completed_inline: bool,
    pub poll_after_ms: u64,
    pub results: Vec<McpBatchChildResponse>,
    #[serde(default)]
    pub aggregate_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobBatchResponse {
    pub batch_id: String,
    pub status: String,
    pub completed_inline: bool,
    pub poll_after_ms: u64,
    pub jobs: Vec<JobInfo>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<JobKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<JobState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl JobListRequest {
    pub const DEFAULT_LIMIT: usize = 50;
    pub const MAX_LIMIT: usize = 100;

    pub fn effective_limit(&self) -> usize {
        self.limit
            .unwrap_or(Self::DEFAULT_LIMIT)
            .clamp(1, Self::MAX_LIMIT)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobGetRequest {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub wait_only: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobCancelRequest {
    pub job_id: String,
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
    #[serde(rename = "process.exec")]
    Exec {
        request_id: String,
        payload: ExecRequest,
    },
    #[serde(rename = "process.batch")]
    ProcessBatch {
        request_id: String,
        payload: BatchExecRequest,
    },
    #[serde(rename = "job.list")]
    JobList {
        request_id: String,
        payload: JobListRequest,
    },
    #[serde(rename = "job.get")]
    JobGet {
        request_id: String,
        payload: JobGetRequest,
    },
    #[serde(rename = "job.cancel")]
    JobCancel {
        request_id: String,
        payload: JobCancelRequest,
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
    #[serde(rename = "mcp.listServers")]
    McpListServers { request_id: String },
    #[serde(rename = "mcp.listTools")]
    McpListTools {
        request_id: String,
        payload: McpListToolsRequest,
    },
    #[serde(rename = "mcp.callTool")]
    McpCallTool {
        request_id: String,
        payload: McpCallToolRequest,
    },
    #[serde(rename = "mcp.batch")]
    McpBatch {
        request_id: String,
        payload: McpBatchRequest,
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
    #[serde(rename = "room.bootstrap")]
    RoomBootstrap { request_id: String },
    #[serde(rename = "room.bootstrap.read")]
    RoomBootstrapRead {
        request_id: String,
        payload: BootstrapReadRequest,
    },
    #[serde(rename = "bootstrap")]
    Bootstrap { request_id: String },
    #[serde(rename = "bootstrap.read")]
    BootstrapRead {
        request_id: String,
        payload: BootstrapReadRequest,
    },
    #[serde(rename = "skills.list")]
    SkillsList { request_id: String },
    #[serde(rename = "skills.read")]
    SkillsRead {
        request_id: String,
        payload: SkillReadRequest,
    },
    #[serde(rename = "skills.search")]
    SkillsSearch {
        request_id: String,
        payload: SkillSearchRequest,
    },
    #[serde(rename = "skills.active")]
    SkillsActive { request_id: String },
    #[serde(rename = "skills.activate")]
    SkillsActivate {
        request_id: String,
        payload: SkillActivationRequest,
    },
    #[serde(rename = "skills.deactivate")]
    SkillsDeactivate {
        request_id: String,
        payload: SkillActivationRequest,
    },
    #[serde(rename = "skills.install")]
    SkillsInstall {
        request_id: String,
        payload: SkillInstallRequest,
    },
    #[serde(rename = "skills.install.get")]
    SkillsInstallGet {
        request_id: String,
        payload: SkillInstallGetRequest,
    },
    #[serde(rename = "skills.install.cancel")]
    SkillsInstallCancel {
        request_id: String,
        payload: SkillInstallCancelRequest,
    },
    #[serde(rename = "skills.run")]
    SkillsRun {
        request_id: String,
        payload: SkillRunRequest,
    },
}

impl HubCommand {
    pub fn request_id(&self) -> &str {
        match self {
            Self::Exec { request_id, .. }
            | Self::ProcessBatch { request_id, .. }
            | Self::JobList { request_id, .. }
            | Self::JobGet { request_id, .. }
            | Self::JobCancel { request_id, .. }
            | Self::TmuxListSessions { request_id }
            | Self::TmuxListPanes { request_id, .. }
            | Self::TmuxCapturePane { request_id, .. }
            | Self::TmuxPasteText { request_id, .. }
            | Self::TmuxExec { request_id, .. }
            | Self::TmuxCreateSession { request_id, .. }
            | Self::TmuxCloseSession { request_id, .. }
            | Self::McpListServers { request_id }
            | Self::McpListTools { request_id, .. }
            | Self::McpCallTool { request_id, .. }
            | Self::McpBatch { request_id, .. }
            | Self::UserNotifyDeliver { request_id, .. }
            | Self::RoomNotebookAppend { request_id, .. }
            | Self::RoomNotebookRecent { request_id, .. }
            | Self::RoomNotebookSelectExact { request_id, .. }
            | Self::RoomNotebookSearch { request_id, .. }
            | Self::RoomNotebookCurrent { request_id, .. }
            | Self::RoomNotebookUpdate { request_id, .. }
            | Self::RoomNotebookRemove { request_id, .. }
            | Self::RoomDiaryAppend { request_id, .. }
            | Self::RoomDiaryRecent { request_id, .. }
            | Self::RoomDiarySelectExact { request_id, .. }
            | Self::RoomBootstrap { request_id }
            | Self::RoomBootstrapRead { request_id, .. }
            | Self::Bootstrap { request_id }
            | Self::BootstrapRead { request_id, .. }
            | Self::SkillsList { request_id }
            | Self::SkillsRead { request_id, .. }
            | Self::SkillsSearch { request_id, .. }
            | Self::SkillsActive { request_id }
            | Self::SkillsActivate { request_id, .. }
            | Self::SkillsDeactivate { request_id, .. }
            | Self::SkillsInstall { request_id, .. }
            | Self::SkillsInstallGet { request_id, .. }
            | Self::SkillsInstallCancel { request_id, .. }
            | Self::SkillsRun { request_id, .. } => request_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCommandEnvelope {
    pub event_id: String,
    pub run_id: String,
    pub request_id: String,
    pub command_hash: String,
    pub command: HubCommand,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Hello {
        role: AgentRole,
        #[serde(rename = "bootGeneration")]
        boot_generation: String,
        #[serde(default, rename = "connectionMode")]
        connection_mode: AgentConnectionMode,
        #[serde(rename = "configSummary")]
        config_summary: SafeConfigSummary,
        #[serde(default, rename = "notificationChannels")]
        notification_channels: Vec<NotificationChannel>,
    },
    Heartbeat {
        #[serde(rename = "sentAt")]
        sent_at: DateTime<Utc>,
    },
    JobUpdate {
        job: JobInfo,
    },
    RunReport {
        report: Box<AgentRunReport>,
    },
    Response {
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "runId")]
        run_id: Option<String>,
        #[serde(rename = "requestId")]
        request_id: String,
        data: serde_json::Value,
    },
    TransportAck {
        #[serde(rename = "eventId")]
        event_id: String,
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "commandHash")]
        command_hash: String,
    },
    TransportRunStatus {
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(rename = "requestId")]
        request_id: String,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
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
mod job_contract_tests {
    use super::*;

    fn sample_tool_response() -> JobToolResponse {
        JobToolResponse {
            job_id: "job-1".to_string(),
            group: None,
            kind: None,
            state: JobState::Running,
            elapsed_ms: Some(42),
            duration_ms: None,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            result: None,
            error: None,
            result_truncated: false,
            result_bytes: None,
            result_sha256: None,
            result_preview: None,
            result_omitted: false,
        }
    }

    #[test]
    fn job_group_validation_trims_and_bounds_readable_text() {
        assert_eq!(
            normalize_job_group(Some("  direct work  ")).unwrap(),
            Some("direct work".to_string())
        );
        assert_eq!(normalize_job_group(None).unwrap(), None);
        assert_eq!(
            normalize_job_group(Some("   ")).unwrap_err(),
            JobGroupValidationError::Empty
        );
        assert_eq!(
            normalize_job_group(Some("work\tstream")).unwrap_err(),
            JobGroupValidationError::ControlCharacter
        );
        assert!(normalize_job_group(Some(&"界".repeat(JOB_GROUP_MAX_CHARS))).is_ok());
        assert_eq!(
            normalize_job_group(Some(&"界".repeat(JOB_GROUP_MAX_CHARS + 1))).unwrap_err(),
            JobGroupValidationError::TooLong
        );
        assert_eq!(JobGroupValidationError::TooLong.code(), "job_group_invalid");
    }

    #[test]
    fn managed_job_admission_group_is_additive_and_parent_scoped() {
        let exec: ExecRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "program": "true",
            "args": [],
            "needConfirm": false
        }))
        .unwrap();
        assert_eq!(exec.group, None);

        let batch: BatchExecRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "group": "chat-direct",
            "elements": [{"program": "true", "args": []}],
            "needConfirm": false
        }))
        .unwrap();
        assert_eq!(batch.group.as_deref(), Some("chat-direct"));

        let skill: SkillRunRequest = serde_json::from_value(serde_json::json!({
            "id": "demo",
            "path": "scripts/check.sh",
            "group": "chat-direct"
        }))
        .unwrap();
        assert_eq!(skill.group.as_deref(), Some("chat-direct"));

        let mcp: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "group": "chat-direct"
        }))
        .unwrap();
        assert_eq!(mcp.group.as_deref(), Some("chat-direct"));

        let mcp_batch: McpBatchRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "group": "chat-direct",
            "calls": [{"serverId": "server", "toolName": "tool"}]
        }))
        .unwrap();
        assert_eq!(mcp_batch.group.as_deref(), Some("chat-direct"));
        assert!(serde_json::to_value(&mcp_batch.calls[0])
            .unwrap()
            .get("group")
            .is_none());
    }

    #[test]
    fn job_list_and_wait_contracts_have_frozen_defaults() {
        let list: JobListRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(list.effective_limit(), 50);
        assert_eq!(list.group, None);
        assert_eq!(list.cursor, None);

        let oversized: JobListRequest = serde_json::from_value(serde_json::json!({
            "limit": 999,
            "group": "work",
            "cursor": "opaque"
        }))
        .unwrap();
        assert_eq!(oversized.effective_limit(), 100);
        assert_eq!(oversized.group.as_deref(), Some("work"));
        assert_eq!(oversized.cursor.as_deref(), Some("opaque"));

        let get: JobGetRequest = serde_json::from_value(serde_json::json!({
            "jobId": "job-1"
        }))
        .unwrap();
        assert!(!get.wait_only);
        assert!(serde_json::to_value(&get)
            .unwrap()
            .get("waitOnly")
            .is_none());

        let wait = JobWaitResponse {
            job_id: "job-1".to_string(),
            state: JobState::Running,
            elapsed_ms: 42,
        };
        assert_eq!(
            serde_json::to_value(wait).unwrap(),
            serde_json::json!({"jobId":"job-1","state":"running","elapsedMs":42})
        );
    }

    #[test]
    fn slim_job_views_omit_routine_noise_and_keep_batch_budget_semantics() {
        let active = serde_json::to_value(sample_tool_response()).unwrap();
        assert_eq!(
            active,
            serde_json::json!({"jobId":"job-1","state":"running","elapsedMs":42})
        );

        let mut omitted = sample_tool_response();
        omitted.state = JobState::Completed;
        omitted.elapsed_ms = None;
        omitted.duration_ms = Some(7);
        omitted.result_omitted = true;
        let batch = McpBatchToolResponse {
            batch_id: "batch-1".to_string(),
            status: McpBatchStatus::Completed,
            error: None,
            results: vec![McpBatchToolChildResponse {
                index: 0,
                id: Some("first".to_string()),
                job: omitted,
            }],
        };
        let value = serde_json::to_value(batch).unwrap();
        assert_eq!(value["results"][0]["resultOmitted"], true);
        assert!(value.get("completedInline").is_none());
        assert!(value.get("pollAfterMs").is_none());
        assert!(value.get("aggregateTruncated").is_none());
    }

    #[test]
    fn job_info_can_represent_not_started_without_fabricated_timestamp() {
        let now = Utc::now();
        let info = JobInfo {
            agent_id: "agent".to_string(),
            job_id: "job-1".to_string(),
            group: Some("work".to_string()),
            batch_id: None,
            batch_call_id: None,
            batch_index: None,
            kind: JobKind::Process,
            state: JobState::Queued,
            created_at: now,
            started_at: None,
            updated_at: now,
            finished_at: None,
            program: None,
            args: Vec::new(),
            working_directory: None,
            command_preview: None,
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            truncated: false,
            reject_reason: None,
            skill_id: None,
            skill_path: None,
            installed_digest: None,
            mcp_server_id: None,
            mcp_tool_name: None,
            cancel_requested: false,
            cancel_outcome: None,
            termination_evidence: None,
        };
        let value = serde_json::to_value(info).unwrap();
        assert_eq!(value["group"], "work");
        assert!(value.get("startedAt").is_none());
    }
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

    #[test]
    fn skills_command_serde_names_are_public_interface_names() {
        let command = HubCommand::SkillsRead {
            request_id: "req".to_string(),
            payload: SkillReadRequest {
                id: "demo".to_string(),
                path: None,
            },
        };
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["type"], "skills.read");
        assert_eq!(value["requestId"], "req");
        assert_eq!(value["payload"]["id"], "demo");

        let active = ActiveSkill {
            id: "missing".to_string(),
            activated_at: Utc::now(),
            status: "missing".to_string(),
            stale: true,
            summary: None,
        };
        let serialized = serde_json::to_string(&active).unwrap();
        assert!(!serialized.contains("summary"));
    }

    #[test]
    fn skill_read_path_is_additive_and_install_source_is_discriminated() {
        let request: SkillReadRequest = serde_json::from_value(serde_json::json!({
            "id": "demo"
        }))
        .unwrap();
        assert_eq!(request.path, None);

        let source = SkillInstallSource::Github {
            repository: Some("owner/repo".to_string()),
            url: None,
            ref_name: Some("release/v1".to_string()),
            path: Some("skills/demo".to_string()),
        };
        let serialized = serde_json::to_value(source).unwrap();
        assert_eq!(serialized["type"], "github");
        assert_eq!(serialized["repository"], "owner/repo");
        assert_eq!(serialized["ref"], "release/v1");
        assert_eq!(serialized["path"], "skills/demo");

        let legacy_response = serde_json::json!({
            "skill": {
                "id": "demo",
                "skillMd": "# Demo",
                "frontmatter": {},
                "tags": [],
                "active": true,
                "packageSummary": {
                    "hasAssets": false,
                    "hasScripts": false,
                    "hasReferences": false
                },
                "warnings": []
            }
        });
        let response: SkillReadResponse = serde_json::from_value(legacy_response).unwrap();
        assert!(response.resource.is_none());
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains("resource"));
    }

    #[test]
    fn install_and_run_protocol_defaults_and_command_names_are_stable() {
        let get: SkillInstallGetRequest = serde_json::from_value(serde_json::json!({
            "installId": "install-1"
        }))
        .unwrap();
        assert_eq!(get.effective_wait_seconds(), 5);

        let run: SkillRunRequest = serde_json::from_value(serde_json::json!({
            "id": "demo",
            "path": "scripts/check.sh"
        }))
        .unwrap();
        assert_eq!(run.effective_wait_seconds(), 5);
        assert_eq!(run.args, None);

        let command = HubCommand::SkillsRun {
            request_id: "req".to_string(),
            payload: run,
        };
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["type"], "skills.run");
        assert_eq!(value["requestId"], "req");
        assert!(value.get("jobId").is_none());
        assert_eq!(value["payload"]["waitSeconds"], serde_json::Value::Null);

        let install = HubCommand::SkillsInstall {
            request_id: "req-install".to_string(),
            payload: SkillInstallRequest {
                id: "demo".to_string(),
                source: SkillInstallSource::Files { files: vec![] },
                replace_existing: false,
                activate_after_install: None,
                idempotency_key: None,
            },
        };
        assert_eq!(
            serde_json::to_value(install).unwrap()["type"],
            "skills.install"
        );
    }

    #[test]
    fn bootstrap_commands_and_enums_use_public_spellings() {
        let bootstrap = HubCommand::RoomBootstrap {
            request_id: "req-bootstrap".to_string(),
        };
        let bootstrap_value = serde_json::to_value(bootstrap).unwrap();
        assert_eq!(bootstrap_value["type"], "room.bootstrap");
        assert_eq!(bootstrap_value["requestId"], "req-bootstrap");

        let read = HubCommand::RoomBootstrapRead {
            request_id: "req-read".to_string(),
            payload: BootstrapReadRequest {
                id: "diary".to_string(),
            },
        };
        let read_value = serde_json::to_value(read).unwrap();
        assert_eq!(read_value["type"], "room.bootstrap.read");
        assert_eq!(read_value["requestId"], "req-read");
        assert_eq!(read_value["payload"]["id"], "diary");

        assert_eq!(
            serde_json::to_value(BootstrapDocumentKind::Entrypoint).unwrap(),
            "entrypoint"
        );
        assert_eq!(
            serde_json::to_value(BootstrapDocumentKind::Guide).unwrap(),
            "guide"
        );
        assert_eq!(
            serde_json::to_value(BootstrapLoadPolicy::OnDemand).unwrap(),
            "on_demand"
        );
        assert_eq!(
            serde_json::to_value(BootstrapEncoding::Utf8).unwrap(),
            "utf8"
        );

        let neutral = HubCommand::Bootstrap {
            request_id: "req-neutral".to_string(),
        };
        assert_eq!(neutral.request_id(), "req-neutral");
        assert_eq!(serde_json::to_value(neutral).unwrap()["type"], "bootstrap");

        let neutral_read = HubCommand::BootstrapRead {
            request_id: "req-neutral-read".to_string(),
            payload: BootstrapReadRequest {
                id: "guide".to_string(),
            },
        };
        assert_eq!(neutral_read.request_id(), "req-neutral-read");
        assert_eq!(
            serde_json::to_value(neutral_read).unwrap()["type"],
            "bootstrap.read"
        );
    }

    #[test]
    fn bootstrap_resource_omits_only_absent_truncation_line() {
        let resource = BootstrapTextResource {
            path: "bootstrap.md".to_string(),
            encoding: BootstrapEncoding::Utf8,
            content: "---\nid: room\n---\n".to_string(),
            media_type: "text/markdown".to_string(),
            size_bytes: 18,
            returned_size_bytes: 18,
            total_lines: 3,
            returned_through_line: 3,
            omitted_from_line: None,
            truncated: false,
            last_line_complete: true,
            sha256: "a".repeat(64),
        };
        let value = serde_json::to_value(resource).unwrap();
        assert_eq!(value["mediaType"], "text/markdown");
        assert_eq!(value["sizeBytes"], 18);
        assert_eq!(value["returnedSizeBytes"], 18);
        assert_eq!(value["totalLines"], 3);
        assert_eq!(value["returnedThroughLine"], 3);
        assert_eq!(value["truncated"], false);
        assert_eq!(value["lastLineComplete"], true);
        assert!(value.get("omittedFromLine").is_none());

        let truncated: BootstrapTextResource = serde_json::from_value(serde_json::json!({
            "path": "guides/diary.md",
            "encoding": "utf8",
            "content": "line 1\n",
            "mediaType": "text/markdown",
            "sizeBytes": 100,
            "returnedSizeBytes": 7,
            "totalLines": 20,
            "returnedThroughLine": 1,
            "omittedFromLine": 2,
            "truncated": true,
            "lastLineComplete": true,
            "sha256": "b".repeat(64)
        }))
        .unwrap();
        assert_eq!(truncated.omitted_from_line, Some(2));
    }

    #[test]
    fn bootstrap_response_and_read_request_round_trip_with_camel_case_fields() {
        let response = BootstrapResponse {
            schema_version: 1,
            revision: "r".repeat(64),
            entrypoint: BootstrapEntrypoint {
                id: "room".to_string(),
                kind: BootstrapDocumentKind::Entrypoint,
                name: "Room Bootstrap".to_string(),
                description: "Route startup guides".to_string(),
                frontmatter: serde_json::json!({"schemaVersion": 1, "future": true}),
                resource: BootstrapTextResource {
                    path: "bootstrap.md".to_string(),
                    encoding: BootstrapEncoding::Utf8,
                    content: "content\n".to_string(),
                    media_type: "text/markdown".to_string(),
                    size_bytes: 8,
                    returned_size_bytes: 8,
                    total_lines: 1,
                    returned_through_line: 1,
                    omitted_from_line: None,
                    truncated: false,
                    last_line_complete: true,
                    sha256: "a".repeat(64),
                },
            },
            guides: vec![BootstrapGuideSummary {
                id: "diary".to_string(),
                kind: BootstrapDocumentKind::Guide,
                title: "Diary conventions".to_string(),
                summary: "Keep continuity".to_string(),
                load_policy: BootstrapLoadPolicy::Contextual,
                priority: 80,
                load_when: vec!["continued context".to_string()],
                tool_bindings: vec!["room.diary.recent".to_string()],
                tags: vec!["continuity".to_string()],
                path: "guides/diary.md".to_string(),
                size_bytes: 10,
                total_lines: 2,
                sha256: "d".repeat(64),
            }],
            total_guides: 1,
            returned_guides: 1,
            warnings: vec![],
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(
            value["entrypoint"]["resource"]["mediaType"],
            "text/markdown"
        );
        assert_eq!(value["guides"][0]["loadPolicy"], "contextual");
        assert_eq!(value["guides"][0]["toolBindings"][0], "room.diary.recent");
        assert_eq!(value["totalGuides"], 1);
        assert_eq!(value["returnedGuides"], 1);

        let round_trip: BootstrapResponse = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, response);

        let request: BootstrapReadRequest = serde_json::from_value(serde_json::json!({
            "id": "diary"
        }))
        .unwrap();
        assert_eq!(request.id, "diary");
        assert_eq!(serde_json::to_value(request).unwrap()["id"], "diary");
    }

    #[test]
    fn managed_mcp_call_defaults_and_bounds_are_frozen() {
        let defaults: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "arguments": {}
        }))
        .unwrap();
        assert_eq!(defaults.effective_wait_seconds(), 5);
        assert_eq!(defaults.effective_timeout_seconds(), 300);

        let bounded: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "arguments": {},
            "waitSeconds": 999,
            "timeoutSeconds": 9999
        }))
        .unwrap();
        assert_eq!(bounded.effective_wait_seconds(), 30);
        assert_eq!(bounded.effective_timeout_seconds(), 900);

        let minimum: McpCallToolRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "serverId": "server",
            "toolName": "tool",
            "arguments": {},
            "waitSeconds": 0,
            "timeoutSeconds": 0
        }))
        .unwrap();
        assert_eq!(minimum.effective_wait_seconds(), 0);
        assert_eq!(minimum.effective_timeout_seconds(), 1);
    }

    #[test]
    fn managed_mcp_batch_defaults_bounds_and_wire_type_are_frozen() {
        let defaults: McpBatchRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "calls": [{
                "id": "first",
                "serverId": "server",
                "toolName": "tool",
                "arguments": {}
            }]
        }))
        .unwrap();
        assert_eq!(defaults.mode, McpBatchMode::Parallel);
        assert!(!defaults.fail_fast);
        assert_eq!(defaults.effective_wait_seconds(), 5);
        assert_eq!(defaults.effective_timeout_seconds(), 300);
        assert_eq!(McpBatchRequest::MIN_CALLS, 1);
        assert_eq!(McpBatchRequest::MAX_CALLS, 16);
        assert_eq!(
            McpBatchRequest::MAX_AGGREGATE_ARGUMENT_BYTES,
            2 * 1024 * 1024
        );
        assert_eq!(McpBatchRequest::MAX_AGGREGATE_RESULT_BYTES, 2 * 1024 * 1024);

        let bounded: McpBatchRequest = serde_json::from_value(serde_json::json!({
            "agentId": "agent",
            "calls": [{"serverId": "server", "toolName": "tool"}],
            "mode": "sequential",
            "failFast": true,
            "waitSeconds": 999,
            "timeoutSeconds": 9999
        }))
        .unwrap();
        assert_eq!(bounded.mode, McpBatchMode::Sequential);
        assert!(bounded.fail_fast);
        assert_eq!(bounded.effective_wait_seconds(), 30);
        assert_eq!(bounded.effective_timeout_seconds(), 900);

        let command = HubCommand::McpBatch {
            request_id: "req-batch".to_string(),
            payload: defaults,
        };
        assert_eq!(command.request_id(), "req-batch");
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["type"], "mcp.batch");
        assert_eq!(value["requestId"], "req-batch");
        assert_eq!(value["payload"]["calls"][0]["id"], "first");
    }

    #[test]
    fn hello_defaults_to_command_capable_when_generation_is_present() {
        let message: AgentMessage = serde_json::from_value(serde_json::json!({
            "type": "hello",
            "role": "normal",
            "bootGeneration": "boot-test",
            "configSummary": {
                "workspaceRoot": "/workspace",
                "sandbox": {"enabled": false, "mode": "disabled"},
                "pathPolicy": {
                    "writeRootCount": 0,
                    "readOnlyRootCount": 0,
                    "denyRootCount": 0,
                    "writeRoots": [],
                    "readOnlyRoots": [],
                    "denyRoots": []
                },
                "policyRuleCounts": {"allow": 0, "confirm": 0, "deny": 0},
                "policyRules": {
                    "allow": [], "confirm": [], "deny": [],
                    "builtins": {"confirm": [], "deny": []}
                },
                "confirmationProvider": "none"
            }
        }))
        .unwrap();
        assert!(matches!(
            message,
            AgentMessage::Hello {
                connection_mode: AgentConnectionMode::CommandCapable,
                ..
            }
        ));
    }

    #[test]
    fn hello_without_boot_generation_is_rejected() {
        let error = serde_json::from_value::<AgentMessage>(serde_json::json!({
            "type": "hello",
            "role": "normal",
            "configSummary": {
                "workspaceRoot": "/workspace",
                "sandbox": {"enabled": false, "mode": "disabled"},
                "pathPolicy": {
                    "writeRootCount": 0,
                    "readOnlyRootCount": 0,
                    "denyRootCount": 0,
                    "writeRoots": [],
                    "readOnlyRoots": [],
                    "denyRoots": []
                },
                "policyRuleCounts": {"allow": 0, "confirm": 0, "deny": 0},
                "policyRules": {
                    "allow": [], "confirm": [], "deny": [],
                    "builtins": {"confirm": [], "deny": []}
                },
                "confirmationProvider": "none"
            }
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("bootGeneration"));
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
