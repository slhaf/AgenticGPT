use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomDiaryLayer {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomDiaryLayerIssue {
    Missing,
    Unreadable,
    InvalidUtf8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomDiaryLayerResult {
    pub layer: RoomDiaryLayer,
    pub period: String,
    pub path: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<RoomDiaryLayerIssue>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomDiaryActiveRequest {}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomDiaryActiveResponse {
    pub daily: RoomDiaryLayerResult,
    pub weekly: RoomDiaryLayerResult,
    pub monthly: RoomDiaryLayerResult,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomDiaryReadRequest {
    pub layer: RoomDiaryLayer,
    pub period: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomDiaryReadResponse {
    pub document: RoomDiaryLayerResult,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomNotebookPreview {
    pub path: String,
    pub title: String,
    pub content_preview: String,
    pub truncated: bool,
    pub effective_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomNotebookRecentRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomNotebookSearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomNotebookReadRequest {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomNotebookReadResponse {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomNotebookResultsResponse {
    pub documents: Vec<RoomNotebookPreview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomStateListRequest {}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomStateEntity {
    pub entity: String,
    pub path: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomStateListResponse {
    pub entities: Vec<RoomStateEntity>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomStateReadRequest {
    pub entity: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomStateReadResponse {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomMaintenanceSlot {
    #[serde(rename = "diary.daily")]
    DiaryDaily,
    #[serde(rename = "diary.weekly")]
    DiaryWeekly,
    #[serde(rename = "diary.monthly")]
    DiaryMonthly,
    Notebook,
    Entity,
}

impl RoomMaintenanceSlot {
    pub const ALL: [Self; 5] = [
        Self::DiaryDaily,
        Self::DiaryWeekly,
        Self::DiaryMonthly,
        Self::Notebook,
        Self::Entity,
    ];
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomMaintenanceRequestItem {
    pub slot: RoomMaintenanceSlot,
    pub payload: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomMaintenanceExecutionMode {
    Local,
    Workflow,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomMaintenanceSubmitRequest {
    pub items: Vec<RoomMaintenanceRequestItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<RoomMaintenanceExecutionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u8>,
}

impl RoomMaintenanceSubmitRequest {
    pub const MIN_ITEMS: usize = 1;
    pub const MAX_ITEMS: usize = 5;
    pub const MAX_WAIT_SECONDS: u8 = 30;

    pub fn has_valid_item_count(&self) -> bool {
        (Self::MIN_ITEMS..=Self::MAX_ITEMS).contains(&self.items.len())
    }

    pub fn effective_wait_seconds(&self) -> u8 {
        self.wait_seconds.unwrap_or(0).min(Self::MAX_WAIT_SECONDS)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomMaintenanceSubmissionState {
    Applied,
    Submitted,
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomMaintenanceSyncOutcome {
    NotRequested,
    Succeeded,
    Failed,
    Unavailable,
    Pending,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceSubmitResponse {
    pub mode: RoomMaintenanceExecutionMode,
    pub state: RoomMaintenanceSubmissionState,
    pub local_applied: bool,
    pub sync: RoomMaintenanceSyncOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomMaintenanceStatusRequest {}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceRepositoryStatus {
    pub root: String,
    pub initialized: bool,
    pub top_level: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clean: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceSchemaStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    pub supported: bool,
    pub ready: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceScaffoldStatus {
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceLocalExecutorStatus {
    pub ready: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceWorkflowStatus {
    pub available: bool,
    pub ready: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceRemoteStatus {
    pub configured: bool,
    pub available: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceSyncStatus {
    pub upstream_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_head: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceSlotStatus {
    pub slot: RoomMaintenanceSlot,
    pub occupied: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMaintenanceStatusResponse {
    pub repository: RoomMaintenanceRepositoryStatus,
    pub schema: RoomMaintenanceSchemaStatus,
    pub scaffold: RoomMaintenanceScaffoldStatus,
    pub local_executor: RoomMaintenanceLocalExecutorStatus,
    pub configured_mode: RoomMaintenanceExecutionMode,
    pub auto_push: bool,
    pub workflow: RoomMaintenanceWorkflowStatus,
    pub remote: RoomMaintenanceRemoteStatus,
    pub sync: RoomMaintenanceSyncStatus,
    pub slots: Vec<RoomMaintenanceSlotStatus>,
}
