use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
        self.wait_seconds
            .unwrap_or(Self::DEFAULT_WAIT_SECONDS)
            .min(Self::MAX_WAIT_SECONDS)
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
        self.wait_seconds
            .unwrap_or(Self::DEFAULT_WAIT_SECONDS)
            .min(Self::MAX_WAIT_SECONDS)
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
