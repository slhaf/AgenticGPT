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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillRunSessionState {
    Starting,
    WaitingConfirmation,
    Running,
    Exited,
    Failed,
    Killed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRunResponse {
    pub agent_id: String,
    pub session_id: String,
    pub completed_inline: bool,
    pub poll_after_ms: u64,
    pub session: SessionInfo,
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
    #[serde(rename = "process.exec")]
    Exec {
        request_id: String,
        task_id: String,
        payload: ExecRequest,
    },
    #[serde(rename = "process.batchExec")]
    BatchExec {
        request_id: String,
        task_id: String,
        payload: BatchExecRequest,
    },
    #[serde(rename = "session.start")]
    StartSession {
        request_id: String,
        session_id: String,
        payload: ExecRequest,
    },
    #[serde(rename = "session.list")]
    ListSessions { request_id: String },
    #[serde(rename = "session.inspect")]
    InspectSession {
        request_id: String,
        session_id: String,
    },
    #[serde(rename = "session.wait")]
    WaitSession {
        request_id: String,
        session_id: String,
        seconds: u64,
    },
    #[serde(rename = "session.kill")]
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
        session_id: String,
        payload: SkillRunRequest,
    },
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
            session_id: "session-1".to_string(),
            payload: run,
        };
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["type"], "skills.run");
        assert_eq!(value["requestId"], "req");
        assert_eq!(value["sessionId"], "session-1");
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
