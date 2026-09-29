use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessExecRequest {
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

impl ProcessExecRequest {
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
pub struct ProcessExecElement {
    pub program: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessBatchExecRequest {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub elements: Vec<ProcessExecElement>,
    pub need_confirm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
}

impl ProcessBatchExecRequest {
    pub const DEFAULT_WAIT_SECONDS: u64 = 5;
    pub const MAX_WAIT_SECONDS: u64 = 30;

    pub fn effective_wait_seconds(&self) -> u64 {
        self.wait_seconds
            .unwrap_or(Self::DEFAULT_WAIT_SECONDS)
            .min(Self::MAX_WAIT_SECONDS)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessKind {
    Command,
    Skill,
    Mcp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
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

impl ProcessState {
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

impl std::fmt::Display for ProcessState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}

pub const PROCESS_GROUP_MAX_CHARS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessGroupValidationError {
    Empty,
    TooLong,
    ControlCharacter,
}

impl ProcessGroupValidationError {
    pub fn code(self) -> &'static str {
        "process_group_invalid"
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Empty => "group must not be empty after trimming",
            Self::TooLong => "group must contain at most 32 Unicode characters",
            Self::ControlCharacter => "group must not contain control characters",
        }
    }
}

impl std::fmt::Display for ProcessGroupValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for ProcessGroupValidationError {}

pub fn normalize_process_group(
    group: Option<&str>,
) -> Result<Option<String>, ProcessGroupValidationError> {
    let Some(group) = group else {
        return Ok(None);
    };
    let trimmed = group.trim();
    if trimmed.is_empty() {
        return Err(ProcessGroupValidationError::Empty);
    }
    if trimmed.chars().count() > PROCESS_GROUP_MAX_CHARS {
        return Err(ProcessGroupValidationError::TooLong);
    }
    if trimmed.chars().any(char::is_control) {
        return Err(ProcessGroupValidationError::ControlCharacter);
    }
    Ok(Some(trimmed.to_string()))
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessCaptureStatus {
    NotStarted,
    Capturing,
    Complete,
    Incomplete,
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub agent_id: String,
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_index: Option<usize>,
    pub kind: ProcessKind,
    pub state: ProcessState,
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
    pub capture_status: ProcessCaptureStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessOutputEncoding {
    Utf8,
    Base64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputGap {
    pub start_offset: String,
    pub end_offset: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputSegment {
    pub data: String,
    pub start_offset: String,
    pub end_offset: String,
    pub encoding: ProcessOutputEncoding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<ProcessOutputGap>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputRequest {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
}

impl ProcessOutputRequest {
    pub const DEFAULT_MAX_BYTES: usize = 8 * 1024;
    pub const MAX_MAX_BYTES: usize = 32 * 1024;

    pub fn effective_max_bytes(&self) -> usize {
        self.max_bytes
            .unwrap_or(Self::DEFAULT_MAX_BYTES)
            .clamp(1, Self::MAX_MAX_BYTES)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputResponse {
    pub process_id: String,
    pub stdout: ProcessOutputSegment,
    pub stderr: ProcessOutputSegment,
    pub next_cursor: String,
    pub has_more: bool,
    pub eof: bool,
    pub capture_status: ProcessCaptureStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessStatusRequest {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
}

impl ProcessStatusRequest {
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
pub struct ProcessStatusResponse {
    #[serde(flatten)]
    pub process: ProcessInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessListItem {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub kind: ProcessKind,
    pub state: ProcessState,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub capture_status: ProcessCaptureStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessListResponse {
    pub processes: Vec<ProcessListItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ProcessKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ProcessState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl ProcessListRequest {
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
pub struct ProcessCancelRequest {
    pub process_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessCancelResponse {
    pub process_id: String,
    pub state: ProcessState,
    pub cancel_outcome: String,
    pub termination_evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProcessError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInlineStream {
    pub data: String,
    pub encoding: ProcessOutputEncoding,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInlineOutput {
    pub stdout: ProcessInlineStream,
    pub stderr: ProcessInlineStream,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputPreview {
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessResultStatus {
    Complete,
    TooLarge,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResultRequest {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
}

impl ProcessResultRequest {
    pub const DEFAULT_MAX_BYTES: usize = 8 * 1024;
    pub const MAX_MAX_BYTES: usize = 512 * 1024;

    pub fn effective_max_bytes(&self) -> usize {
        self.max_bytes
            .unwrap_or(Self::DEFAULT_MAX_BYTES)
            .clamp(1, Self::MAX_MAX_BYTES)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResultResponse {
    pub process_id: String,
    pub status: ProcessResultStatus,
    pub result_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProcessError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDetail {
    pub process: ProcessInfo,
    pub detail_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProcessError>,
    #[serde(default)]
    pub result_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResponse {
    pub status: ProcessState,
    pub completed_inline: bool,
    #[serde(flatten)]
    pub process: ProcessInfo,
    pub poll_after_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_output: Option<ProcessInlineOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<ProcessOutputPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_status: Option<ProcessResultStatus>,
    #[serde(default)]
    pub result_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProcessError>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessBatchResponse {
    pub batch_id: String,
    pub status: String,
    pub completed_inline: bool,
    pub poll_after_ms: u64,
    pub processes: Vec<ProcessResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessToolResponse {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ProcessKind>,
    pub state: ProcessState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProcessError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_status: Option<ProcessResultStatus>,
    #[serde(default)]
    pub result_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_preview: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessBatchToolResponse {
    pub batch_id: String,
    pub status: String,
    pub results: Vec<ProcessToolResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessCursor {
    pub version: u8,
    pub process_id: String,
    pub stdout_offset: u64,
    pub stderr_offset: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_list_and_status_contracts_have_bounded_defaults_without_wait_only() {
        let list: ProcessListRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(list.effective_limit(), 50);
        assert!(list.group.is_none());
        assert!(list.cursor.is_none());

        let status: ProcessStatusRequest = serde_json::from_value(serde_json::json!({
            "processId": "process-1"
        }))
        .unwrap();
        assert_eq!(status.effective_wait_seconds(), 5);
        assert_eq!(status.process_id, "process-1");
        assert!(
            serde_json::from_value::<ProcessStatusRequest>(serde_json::json!({
                "processId": "process-1",
                "waitOnly": true
            }))
            .is_err()
        );

        for (wait_seconds, expected) in [(0, 0), (5, 5), (30, 30), (31, 30), (u64::MAX, 30)] {
            let status = ProcessStatusRequest {
                process_id: "process-1".to_string(),
                wait_seconds: Some(wait_seconds),
            };
            assert_eq!(status.effective_wait_seconds(), expected);
        }
    }

    #[test]
    fn process_wire_contract_uses_process_id_and_never_status_output_fields() {
        let request = ProcessOutputRequest {
            process_id: "process-1".to_string(),
            cursor: None,
            max_bytes: None,
        };
        let request_json = serde_json::to_value(request).unwrap();
        assert_eq!(request_json["processId"], "process-1");
        assert_eq!(request_json["maxBytes"], serde_json::Value::Null);
        assert_eq!(ProcessOutputRequest::DEFAULT_MAX_BYTES, 8192);
        assert_eq!(ProcessOutputRequest::MAX_MAX_BYTES, 32768);
        assert_eq!(ProcessResultRequest::MAX_MAX_BYTES, 512 * 1024);

        let result = ProcessResultResponse {
            process_id: "process-1".to_string(),
            status: ProcessResultStatus::Unavailable,
            result_available: false,
            result: None,
            error: None,
            result_bytes: Some(600_000),
            result_sha256: Some("sha256:abc".to_string()),
            result_preview: Some("preview".to_string()),
        };
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["processId"], "process-1");
        assert_eq!(json["status"], "unavailable");
        assert_eq!(json["resultAvailable"], false);
        assert!(json.get("result").is_none());
    }
}
