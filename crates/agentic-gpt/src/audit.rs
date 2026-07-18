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
    pub(crate) session_id: Option<String>,
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
