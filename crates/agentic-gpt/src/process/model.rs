//! Local process state. Connection identity is supplied only when reporting over the wire.
use agentic_gpt_protocol::{
    McpBatchStatus, ProcessCaptureStatus, ProcessError, ProcessKind, ProcessState,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessInfo {
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
pub(crate) struct ProcessDetail {
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
pub(crate) struct McpBatchChildResponse {
    pub index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub result_omitted: bool,
    #[serde(flatten)]
    pub process: ProcessDetail,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpBatchResponse {
    pub batch_id: String,
    pub status: McpBatchStatus,
    pub results: Vec<McpBatchChildResponse>,
    #[serde(default)]
    pub aggregate_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProcessError>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl ProcessInfo {
    pub(crate) fn into_wire(self, agent_id: &str) -> agentic_gpt_protocol::ProcessInfo {
        agentic_gpt_protocol::ProcessInfo {
            agent_id: agent_id.to_owned(),
            process_id: self.process_id,
            group: self.group,
            batch_id: self.batch_id,
            batch_call_id: self.batch_call_id,
            batch_index: self.batch_index,
            kind: self.kind,
            state: self.state,
            created_at: self.created_at,
            started_at: self.started_at,
            updated_at: self.updated_at,
            finished_at: self.finished_at,
            program: self.program,
            args: self.args,
            working_directory: self.working_directory,
            command_preview: self.command_preview,
            exit_code: self.exit_code,
            reject_reason: self.reject_reason,
            skill_id: self.skill_id,
            skill_path: self.skill_path,
            installed_digest: self.installed_digest,
            mcp_server_id: self.mcp_server_id,
            mcp_tool_name: self.mcp_tool_name,
            cancel_requested: self.cancel_requested,
            cancel_outcome: self.cancel_outcome,
            termination_evidence: self.termination_evidence,
            capture_status: self.capture_status,
            capture_error: self.capture_error,
        }
    }
}
