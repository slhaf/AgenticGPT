use serde::{Deserialize, Serialize};

use super::process_jobs::{is_false, JobDetail, JobError, JobToolResponse};

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
pub struct McpBatchToolChildResponse {
    #[serde(flatten)]
    pub job: JobToolResponse,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpBatchToolResponse {
    pub status: McpBatchStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JobError>,
    pub results: Vec<McpBatchToolChildResponse>,
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
