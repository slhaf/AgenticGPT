use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{anyhow, Result};
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserAuditRecord {
    pub(crate) time: DateTime<Utc>,
    pub(crate) tool: String,
    pub(crate) request_source: String,
    pub(crate) lease_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) runtime_app_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) code_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) idle_timeout_seconds: Option<u64>,
    pub(crate) outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_code: Option<String>,
    pub(crate) duration_ms: u128,
}

const AUDIT_MAX_BYTES: u64 = 8 * 1024 * 1024;
fn append_record(config: &Config, record: Vec<u8>) -> Result<()> {
    let audit_path = config.workspace_root.join(".agentic-gpt-audit.jsonl");
    append_record_at(&audit_path, record)
}

fn append_record_at(audit_path: &Path, record: Vec<u8>) -> Result<()> {
    if record.len() as u64 + 1 > AUDIT_MAX_BYTES {
        return Err(anyhow!(
            "audit_record_too_large; bytes={}; limit={AUDIT_MAX_BYTES}",
            record.len() + 1
        ));
    }

    let _lock = acquire_audit_lock(&audit_path)?;
    let current_len = fs::metadata(&audit_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if current_len.saturating_add(record.len() as u64 + 1) > AUDIT_MAX_BYTES {
        let backup = audit_path.with_file_name(".agentic-gpt-audit.jsonl.1");
        match fs::remove_file(&backup) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if current_len > 0 {
            fs::rename(&audit_path, &backup)?;
        }
    }

    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&audit_path)?;
    file.seek(SeekFrom::End(0))?;
    let length = file.stream_position()?;
    if length > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0_u8; 1];
        file.read_exact(&mut last)?;
        file.seek(SeekFrom::End(0))?;
        if last[0] != b'\n' {
            file.write_all(b"\n")?;
        }
    }
    file.write_all(&record)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn acquire_audit_lock(path: &std::path::Path) -> Result<File> {
    let lock_path = path.with_file_name(".agentic-gpt-audit.jsonl.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(file)
}

pub(crate) fn write_audit(config: &Config, record: AuditRecord) -> Result<()> {
    append_record(config, serde_json::to_vec(&record)?)
}

pub(crate) fn write_mcp_batch_audit(config: &Config, record: McpBatchAuditRecord) -> Result<()> {
    append_record(config, serde_json::to_vec(&record)?)
}

pub(crate) fn write_file_audit(config: &Config, record: FileAuditRecord) -> Result<()> {
    append_record(config, serde_json::to_vec(&record)?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use uuid::Uuid;

    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("agentic-audit-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn oversized_audit_record_is_rejected_without_truncating_existing_data() {
        let root = temp_dir();
        let path = root.join(".agentic-gpt-audit.jsonl");
        append_record_at(&path, b"original".to_vec()).unwrap();
        let oversized = vec![b'x'; AUDIT_MAX_BYTES as usize];
        let error = append_record_at(&path, oversized).unwrap_err();
        assert!(error.to_string().contains("audit_record_too_large"));
        assert_eq!(fs::read(&path).unwrap(), b"original\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn audit_rotation_keeps_one_bounded_backup() {
        let root = temp_dir();
        let path = root.join(".agentic-gpt-audit.jsonl");
        append_record_at(&path, vec![b'a'; AUDIT_MAX_BYTES as usize - 1]).unwrap();
        append_record_at(&path, b"next".to_vec()).unwrap();
        let backup = root.join(".agentic-gpt-audit.jsonl.1");
        assert_eq!(fs::metadata(&backup).unwrap().len(), AUDIT_MAX_BYTES);
        assert_eq!(fs::read(&path).unwrap(), b"next\n");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_audit_appends_remain_complete_json_lines() {
        let root = temp_dir();
        let path = Arc::new(root.join(".agentic-gpt-audit.jsonl"));
        let mut workers = Vec::new();
        for worker in 0..4 {
            let path = path.clone();
            workers.push(thread::spawn(move || {
                for line in 0..32 {
                    let record = format!(r#"{{"worker":{worker},"line":{line}}}"#);
                    append_record_at(&path, record.into_bytes()).unwrap();
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let lines = fs::read_to_string(&*path).unwrap();
        assert_eq!(lines.lines().count(), 128);
        for line in lines.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(value.get("worker").is_some());
            assert!(value.get("line").is_some());
        }
        let _ = fs::remove_dir_all(root);
    }
}

pub(crate) fn write_browser_audit(config: &Config, record: BrowserAuditRecord) -> Result<()> {
    append_record(config, serde_json::to_vec(&record)?)
}
