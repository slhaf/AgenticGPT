use std::fs::OpenOptions;
use std::io::Write;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::config::Config;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditRecord {
    pub(crate) task_id: Option<String>,
    pub(crate) job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) batch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) batch_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) batch_index: Option<usize>,
    pub(crate) time: DateTime<Utc>,
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) working_directory: Option<String>,
    pub(crate) need_confirm: bool,
    pub(crate) policy_decision: String,
    pub(crate) confirmation_result: Option<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) duration_ms: u128,
    pub(crate) truncated: bool,
    pub(crate) request_source: String,
    pub(crate) reject_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) skill_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) skill_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) installed_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mcp_server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mcp_tool_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) argument_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) argument_key_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) argument_keys_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) argument_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) argument_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) config_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) terminal_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) termination_evidence: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileAuditRecord {
    pub(crate) time: DateTime<Utc>,
    pub(crate) tool: String,
    pub(crate) action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) batch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) group_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation_id: Option<String>,
    pub(crate) path: String,
    pub(crate) mode: Option<String>,
    pub(crate) requested_confirmation: bool,
    pub(crate) confirmation_result: Option<String>,
    pub(crate) before_revision: Option<String>,
    pub(crate) after_revision: Option<String>,
    pub(crate) outcome: String,
    pub(crate) error_code: Option<String>,
    pub(crate) duration_ms: u128,
    pub(crate) replacement_count: Option<usize>,
    pub(crate) changed_lines: Option<ChangedLines>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) committed: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangedLines {
    pub(crate) added: usize,
    pub(crate) removed: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BatchAuditRecord {
    pub(crate) time: DateTime<Utc>,
    pub(crate) tool: String,
    pub(crate) action: String,
    pub(crate) batch_id: String,
    pub(crate) operation_count: usize,
    pub(crate) edit_count: usize,
    pub(crate) group_count: usize,
    pub(crate) committed_group_count: usize,
    pub(crate) failed_group_count: usize,
    pub(crate) unchanged_group_count: usize,
    pub(crate) failure_count: usize,
    pub(crate) confirmation_result: Option<String>,
    pub(crate) outcome: String,
    pub(crate) duration_ms: u128,
    pub(crate) truncated: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpBatchAuditRecord {
    pub(crate) time: DateTime<Utc>,
    pub(crate) tool: String,
    pub(crate) batch_id: String,
    pub(crate) request_source: String,
    pub(crate) call_count: usize,
    pub(crate) server_count: usize,
    pub(crate) mode: String,
    pub(crate) fail_fast: bool,
    pub(crate) confirmation_required_count: usize,
    pub(crate) confirmation_result: Option<String>,
    pub(crate) child_job_ids: Vec<String>,
    pub(crate) outcome: String,
    pub(crate) error_code: Option<String>,
    pub(crate) duration_ms: u128,
    pub(crate) truncated: bool,
}

pub(crate) fn write_audit(config: &Config, record: AuditRecord) -> Result<()> {
    let audit_path = config.workspace_root.join(".agentic-gpt-audit.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    Ok(())
}

pub(crate) fn write_mcp_batch_audit(config: &Config, record: McpBatchAuditRecord) -> Result<()> {
    let audit_path = config.workspace_root.join(".agentic-gpt-audit.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    Ok(())
}

pub(crate) fn write_file_audit(config: &Config, record: FileAuditRecord) -> Result<()> {
    let audit_path = config.workspace_root.join(".agentic-gpt-audit.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    Ok(())
}

pub(crate) fn write_batch_audit(config: &Config, record: BatchAuditRecord) -> Result<()> {
    let audit_path = config.workspace_root.join(".agentic-gpt-audit.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path)?;
    writeln!(file, "{}", serde_json::to_string(&record)?)?;
    Ok(())
}
