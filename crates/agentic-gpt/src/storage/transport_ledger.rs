use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use agentic_gpt_protocol::{AgentMessage, HubCommand, HubCommandEnvelope};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::instance_lock::InstanceLock;
use crate::utils::{agentic_home, ensure_parent};

const PRIVATE_MODE: u32 = 0o600;
const COMPACTION_BYTES: u64 = 256 * 1024;

static LEDGER_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[cfg(test)]
thread_local! {
    static TEST_LEDGER_PATH: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn set_test_ledger_path(path: Option<PathBuf>) {
    TEST_LEDGER_PATH.with(|current| *current.borrow_mut() = path);
}
static TEMP_COUNTER: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LedgerRecord {
    pub(crate) run_id: String,
    pub(crate) request_id: String,
    pub(crate) command_hash: String,
    pub(crate) status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<HubCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) conflict: Option<Value>,
}

struct ConflictDetails<'a> {
    run_id: &'a str,
    request_id: &'a str,
    command_hash: &'a str,
    agent_id: &'a str,
    command: Option<HubCommand>,
    result: Option<Value>,
    reason: &'a str,
}

#[derive(Clone, Debug)]
pub(crate) enum AcceptOutcome {
    FirstAccepted,
    DuplicateAccepted,
    DuplicateStarted,
    Completed(Value),
    HashMismatch,
    OwnerMismatch,
    LegacyUnowned,
}

#[derive(Clone, Debug)]
pub(crate) enum ClaimOutcome {
    Claimed,
    AlreadyStarted,
    Completed(Value),
    Missing,
    Unowned,
    OwnerMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompletionOutcome {
    Persisted,
    AlreadyCompleted,
}

pub(crate) fn latest_records() -> Result<HashMap<String, LedgerRecord>> {
    with_ledger_lock(scan_locked)
}

pub(crate) fn accept(envelope: &HubCommandEnvelope, agent_id: &str) -> Result<AcceptOutcome> {
    validate_identity(
        &envelope.run_id,
        &envelope.request_id,
        &envelope.command_hash,
        agent_id,
    )?;
    with_ledger_lock(|path| {
        let mut latest = scan_locked(path)?;
        let Some(existing) = latest.remove(&envelope.run_id) else {
            append_locked(
                path,
                &LedgerRecord {
                    run_id: envelope.run_id.clone(),
                    request_id: envelope.request_id.clone(),
                    command_hash: envelope.command_hash.clone(),
                    status: "accepted".to_string(),
                    agent_id: Some(agent_id.to_string()),
                    command: Some(envelope.command.clone()),
                    result: None,
                    reason: None,
                    conflict: None,
                },
            )?;
            return Ok(AcceptOutcome::FirstAccepted);
        };

        if existing.request_id != envelope.request_id
            || existing.command_hash != envelope.command_hash
        {
            append_conflict_locked(
                path,
                &existing,
                ConflictDetails {
                    run_id: &envelope.run_id,
                    request_id: &envelope.request_id,
                    command_hash: &envelope.command_hash,
                    agent_id,
                    command: Some(envelope.command.clone()),
                    result: None,
                    reason: "transport_identity_mismatch",
                },
            )?;
            return Ok(AcceptOutcome::HashMismatch);
        }

        let existing = if let Some(owner) = existing.agent_id.as_deref() {
            if owner != agent_id {
                append_conflict_locked(
                    path,
                    &existing,
                    ConflictDetails {
                        run_id: &envelope.run_id,
                        request_id: &envelope.request_id,
                        command_hash: &envelope.command_hash,
                        agent_id,
                        command: Some(envelope.command.clone()),
                        result: None,
                        reason: "transport_owner_mismatch",
                    },
                )?;
                return Ok(AcceptOutcome::OwnerMismatch);
            }
            existing
        } else {
            return Ok(AcceptOutcome::LegacyUnowned);
        };

        match existing.status.as_str() {
            "completed" => {
                Ok(AcceptOutcome::Completed(existing.result.ok_or_else(
                    || anyhow!("transport_completed_result_missing"),
                )?))
            }
            "started" | "running" => Ok(AcceptOutcome::DuplicateStarted),
            "accepted" => Ok(AcceptOutcome::DuplicateAccepted),
            _ => Err(anyhow!("transport_status_unusable")),
        }
    })
}

/// Atomically compare-and-claim an accepted run for execution by one Agent.
pub(crate) fn claim_started(
    run_id: &str,
    request_id: &str,
    command_hash: &str,
    agent_id: &str,
) -> Result<ClaimOutcome> {
    validate_identity(run_id, request_id, command_hash, agent_id)?;
    with_ledger_lock(|path| {
        let mut latest = scan_locked(path)?;
        let Some(mut existing) = latest.remove(run_id) else {
            return Ok(ClaimOutcome::Missing);
        };
        if existing.request_id != request_id || existing.command_hash != command_hash {
            append_conflict_locked(
                path,
                &existing,
                ConflictDetails {
                    run_id,
                    request_id,
                    command_hash,
                    agent_id,
                    command: None,
                    result: None,
                    reason: "transport_claim_identity_mismatch",
                },
            )?;
            return Ok(ClaimOutcome::OwnerMismatch);
        }
        match existing.agent_id.as_deref() {
            Some(owner) if owner == agent_id => {}
            Some(_) => {
                append_conflict_locked(
                    path,
                    &existing,
                    ConflictDetails {
                        run_id,
                        request_id,
                        command_hash,
                        agent_id,
                        command: None,
                        result: None,
                        reason: "transport_claim_owner_mismatch",
                    },
                )?;
                return Ok(ClaimOutcome::OwnerMismatch);
            }
            None => return Ok(ClaimOutcome::Unowned),
        }
        match existing.status.as_str() {
            "accepted" => {
                existing.status = "started".to_string();
                append_locked(path, &existing)?;
                Ok(ClaimOutcome::Claimed)
            }
            "started" | "running" => Ok(ClaimOutcome::AlreadyStarted),
            "completed" => {
                Ok(ClaimOutcome::Completed(existing.result.ok_or_else(
                    || anyhow!("transport_completed_result_missing"),
                )?))
            }
            _ => Err(anyhow!("transport_claim_status_unusable")),
        }
    })
}

pub(crate) fn mark_completed(
    run_id: &str,
    request_id: &str,
    command_hash: &str,
    agent_id: &str,
    result: &Value,
) -> Result<CompletionOutcome> {
    validate_identity(run_id, request_id, command_hash, agent_id)?;
    with_ledger_lock(|path| {
        let mut latest = scan_locked(path)?;
        let Some(mut existing) = latest.remove(run_id) else {
            return Err(anyhow!("transport_record_missing"));
        };
        if existing.request_id != request_id || existing.command_hash != command_hash {
            append_conflict_locked(
                path,
                &existing,
                ConflictDetails {
                    run_id,
                    request_id,
                    command_hash,
                    agent_id,
                    command: existing.command.clone(),
                    result: Some(result.clone()),
                    reason: "transport_completion_identity_mismatch",
                },
            )?;
            return Err(anyhow!("transport_completion_identity_mismatch"));
        }
        if existing.agent_id.as_deref() != Some(agent_id) {
            append_conflict_locked(
                path,
                &existing,
                ConflictDetails {
                    run_id,
                    request_id,
                    command_hash,
                    agent_id,
                    command: existing.command.clone(),
                    result: Some(result.clone()),
                    reason: "transport_completion_owner_mismatch",
                },
            )?;
            return Err(anyhow!("transport_completion_owner_mismatch"));
        }
        if existing.status == "completed" {
            if existing.result.as_ref() == Some(result) {
                return Ok(CompletionOutcome::AlreadyCompleted);
            }
            append_conflict_locked(
                path,
                &existing,
                ConflictDetails {
                    run_id,
                    request_id,
                    command_hash,
                    agent_id,
                    command: existing.command.clone(),
                    result: Some(result.clone()),
                    reason: "transport_completion_result_conflict",
                },
            )?;
            return Err(anyhow!("transport_completion_result_conflict"));
        }
        existing.status = "completed".to_string();
        existing.result = Some(result.clone());
        existing.reason = None;
        existing.conflict = None;
        append_locked(path, &existing)?;
        maybe_compact_locked(path)?;
        Ok(CompletionOutcome::Persisted)
    })
}

pub(crate) fn ack_message(envelope: &HubCommandEnvelope) -> AgentMessage {
    AgentMessage::TransportAck {
        event_id: envelope.event_id.clone(),
        run_id: envelope.run_id.clone(),
        request_id: envelope.request_id.clone(),
        command_hash: envelope.command_hash.clone(),
    }
}

pub(crate) fn completed_response(record: &LedgerRecord) -> Result<Option<AgentMessage>> {
    let Some(data) = record.result.clone() else {
        return Ok(None);
    };
    let event_sources = record
        .command
        .as_ref()
        .map(|command| {
            crate::event_notifications::initial_response_dispositions(
                crate::operation::hub_command_name(command),
                &data,
            )
        })
        .transpose()?
        .unwrap_or_default();
    Ok(Some(AgentMessage::Response {
        run_id: Some(record.run_id.clone()),
        request_id: record.request_id.clone(),
        event_sources,
        data,
    }))
}

fn with_ledger_lock<T>(operation: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let guard = LEDGER_MUTEX
        .lock()
        .map_err(|_| anyhow!("transport_ledger_lock_poisoned"))?;
    let path = ledger_path()?;
    ensure_parent(&path)?;
    reject_symlink(&path)?;
    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    reject_symlink(&lock_path)?;
    let _file_lock = InstanceLock::acquire_blocking(&path, ".lock", "transport ledger")?;
    let result = operation(&path);
    drop(guard);
    result
}

fn scan_locked(path: &Path) -> Result<HashMap<String, LedgerRecord>> {
    reject_symlink(path)?;
    let mut records = HashMap::new();
    let bytes = match read_private(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(records),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    if bytes.is_empty() {
        return Ok(records);
    }
    if !bytes.ends_with(b"\n") {
        preserve_recovery(path, bytes.as_slice(), "torn_final_line")?;
        return Err(anyhow!("transport_ledger_torn_line"));
    }
    let mut offset = 0usize;
    for line in bytes.split(|byte| *byte == b'\n') {
        let line_offset = offset;
        offset = offset.saturating_add(line.len() + 1);
        if line.is_empty() {
            continue;
        }
        let record = match serde_json::from_slice::<LedgerRecord>(line) {
            Ok(record) => record,
            Err(error) => {
                preserve_recovery(path, line, &format!("invalid_json_at_{line_offset}"))?;
                return Err(anyhow!("transport_ledger_corrupt_line:{error}"));
            }
        };
        // Conflict entries are evidence, not a replacement for the canonical run.
        if record.status != "conflict" {
            records.insert(record.run_id.clone(), record);
        }
    }
    Ok(records)
}

fn append_locked(path: &Path, record: &LedgerRecord) -> Result<()> {
    reject_symlink(path)?;
    let existed = path.exists();
    let mut options = OpenOptions::new();
    options
        .create(true)
        .append(true)
        .read(true)
        .mode(PRIVATE_MODE);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(path)
        .with_context(|| format!("open transport ledger {}", path.display()))?;
    file.set_permissions(Permissions::from_mode(PRIVATE_MODE))?;
    let encoded = serde_json::to_vec(record)?;
    file.write_all(&encoded)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    if !existed {
        sync_parent(path)?;
    }
    Ok(())
}

fn append_conflict_locked(
    path: &Path,
    canonical: &LedgerRecord,
    details: ConflictDetails<'_>,
) -> Result<()> {
    let conflict = serde_json::to_value(canonical).ok();
    append_locked(
        path,
        &LedgerRecord {
            run_id: details.run_id.to_string(),
            request_id: details.request_id.to_string(),
            command_hash: details.command_hash.to_string(),
            status: "conflict".to_string(),
            agent_id: Some(details.agent_id.to_string()),
            command: details.command,
            result: details.result,
            reason: Some(details.reason.to_string()),
            conflict,
        },
    )
}

fn validate_identity(
    run_id: &str,
    request_id: &str,
    command_hash: &str,
    agent_id: &str,
) -> Result<()> {
    if run_id.trim().is_empty()
        || request_id.trim().is_empty()
        || command_hash.trim().is_empty()
        || agent_id.trim().is_empty()
    {
        return Err(anyhow!("transport_identity_missing"));
    }
    Ok(())
}

fn maybe_compact_locked(path: &Path) -> Result<()> {
    let size = fs::metadata(path)?.len();
    if size < COMPACTION_BYTES {
        return Ok(());
    }
    compact_locked(path)
}

fn compact_locked(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    let raw = match read_private(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    if raw.is_empty() {
        return Ok(());
    }
    if !raw.ends_with(b"\n") {
        preserve_recovery(path, raw.as_slice(), "torn_final_line")?;
        return Err(anyhow!("transport_ledger_torn_line"));
    }
    let mut entries = Vec::new();
    for line in raw.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let record = match serde_json::from_slice::<LedgerRecord>(line) {
            Ok(record) => record,
            Err(error) => {
                preserve_recovery(path, line, "invalid_json_during_compaction")?;
                return Err(anyhow!("transport_ledger_corrupt_line:{error}"));
            }
        };
        entries.push((line.to_vec(), record));
    }

    let identity = |record: &LedgerRecord| {
        record.agent_id.as_ref().map(|agent_id| {
            (
                record.run_id.clone(),
                record.request_id.clone(),
                record.command_hash.clone(),
                agent_id.clone(),
            )
        })
    };
    let mut last_completed = HashMap::new();
    let mut last_completed_bytes = HashMap::new();
    for (index, (line, record)) in entries.iter().enumerate() {
        if record.status == "completed" && record.agent_id.is_some() && record.result.is_some() {
            if let Some(key) = identity(record) {
                last_completed.insert(key, index);
                last_completed_bytes.insert(line.clone(), index);
            }
        }
    }

    let mut kept = Vec::new();
    let mut changed = false;
    for (index, (line, record)) in entries.iter().enumerate() {
        let mut keep = true;
        if record.status == "completed" && record.agent_id.is_some() && record.result.is_some() {
            keep = last_completed_bytes.get(line) == Some(&index);
        } else if matches!(record.status.as_str(), "accepted" | "started" | "running") {
            if let Some(key) = identity(record) {
                if let Some(last) = last_completed.get(&key) {
                    keep = index >= *last;
                }
            }
        }
        if keep {
            kept.extend_from_slice(line);
            kept.push(b'\n');
        } else {
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }

    let backup = recovery_backup_path(path);
    reject_symlink(&backup)?;
    write_private(&backup, &raw)?;
    let temp = temporary_path(path);
    reject_symlink(&temp)?;
    write_private(&temp, &kept)?;
    fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))?;
    sync_parent(path)?;
    Ok(())
}

fn read_private(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options
        .create(true)
        .truncate(true)
        .write(true)
        .read(true)
        .mode(PRIVATE_MODE);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(path)?;
    file.set_permissions(Permissions::from_mode(PRIVATE_MODE))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn preserve_recovery(path: &Path, raw: &[u8], reason: &str) -> Result<()> {
    let recovery = recovery_path(path);
    reject_symlink(&recovery)?;
    ensure_parent(&recovery)?;
    let mut digest = Sha256::new();
    digest.update(raw);
    let digest = format!("{:x}", digest.finalize());
    let marker = format!("recovery hash={digest} reason={reason}");
    if let Ok(existing) = fs::read(&recovery) {
        if existing
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
        {
            return Ok(());
        }
    }
    let mut options = OpenOptions::new();
    options
        .create(true)
        .append(true)
        .write(true)
        .read(true)
        .mode(PRIVATE_MODE);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options.open(&recovery)?;
    file.set_permissions(Permissions::from_mode(PRIVATE_MODE))?;
    writeln!(file, "{marker} bytes={}", raw.len())?;
    file.write_all(raw)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    sync_parent(&recovery)?;
    Ok(())
}

fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(anyhow!("transport_ledger_symlink_target"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn recovery_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.recovery", path.display()))
}

fn recovery_backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.backup", path.display()))
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut value = TEMP_COUNTER
        .lock()
        .expect("transport ledger temp counter poisoned");
    *value = value.saturating_add(1);
    PathBuf::from(format!(
        "{}.tmp-{}-{}",
        path.display(),
        std::process::id(),
        *value
    ))
}

fn ledger_path() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_LEDGER_PATH.with(|current| current.borrow().clone()) {
        return Ok(path);
    }
    Ok(agentic_home()?.join("transport-runs.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_gpt_protocol::{HubCommand, ProcessExecRequest};
    use parking_lot::Mutex;
    use std::sync::{Arc, LazyLock};
    use std::thread;

    static TEST_HOME_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct TestHome {
        root: PathBuf,
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            set_test_ledger_path(None);
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn test_home() -> TestHome {
        let root =
            std::env::temp_dir().join(format!("agentic-ledger-test-{}", uuid::Uuid::new_v4()));
        let ledger_dir = root.join(".agentic_gpt");
        fs::create_dir_all(&ledger_dir).unwrap();
        set_test_ledger_path(Some(ledger_dir.join("transport-runs.jsonl")));
        TestHome { root }
    }

    fn envelope(
        run_id: &str,
        request_id: &str,
        command_hash: &str,
        command: HubCommand,
    ) -> HubCommandEnvelope {
        HubCommandEnvelope {
            event_id: format!("event-{run_id}"),
            run_id: run_id.to_string(),
            request_id: request_id.to_string(),
            command_hash: command_hash.to_string(),
            command,
        }
    }

    fn skills_command(request_id: &str) -> HubCommand {
        HubCommand::SkillsList {
            request_id: request_id.to_string(),
        }
    }

    fn exec_command(request_id: &str, agent_id: &str) -> HubCommand {
        HubCommand::Exec {
            request_id: request_id.to_string(),
            payload: ProcessExecRequest {
                agent_id: agent_id.to_string(),
                group: None,
                program: "true".to_string(),
                args: Vec::new(),
                need_confirm: false,
                confirm_method: None,
                working_directory: None,
                wait_seconds: None,
            },
        }
    }

    #[test]
    fn claim_concurrency_allows_one_started_owner() {
        let _home_lock = TEST_HOME_LOCK.lock();
        let _home = test_home();
        let command = skills_command("request-claim");
        let request = envelope("run-claim", "request-claim", "hash-claim", command);
        assert!(matches!(
            accept(&request, "agent-a").unwrap(),
            AcceptOutcome::FirstAccepted
        ));
        let request = Arc::new(request);
        let ledger = ledger_path().unwrap();
        let claims = (0..8)
            .map(|_| {
                let request = Arc::clone(&request);
                let ledger = ledger.clone();
                thread::spawn(move || {
                    set_test_ledger_path(Some(ledger));
                    claim_started(
                        &request.run_id,
                        &request.request_id,
                        &request.command_hash,
                        "agent-a",
                    )
                    .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let claims = claims
            .into_iter()
            .map(|claim| claim.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            claims
                .iter()
                .filter(|claim| matches!(claim, ClaimOutcome::Claimed))
                .count(),
            1
        );
        assert_eq!(
            claims
                .iter()
                .filter(|claim| matches!(claim, ClaimOutcome::AlreadyStarted))
                .count(),
            7
        );
    }

    #[test]
    fn torn_corruption_fails_closed_and_deduplicates_recovery_evidence() {
        let _home_lock = TEST_HOME_LOCK.lock();
        let _home = test_home();
        let ledger = ledger_path().unwrap();
        ensure_parent(&ledger).unwrap();
        fs::write(
            &ledger,
            b"{\"runId\":\"run-torn\",\"requestId\":\"request-torn\",\"commandHash\":\"hash-torn\",\"status\":\"accepted\"}\n{\"runId\":\"run-torn\",\"status\":\"started\"",
        )
        .unwrap();
        assert!(latest_records().is_err());
        let recovery = recovery_path(&ledger);
        let first_size = fs::metadata(&recovery).unwrap().len();
        assert!(first_size > 0);
        assert!(latest_records().is_err());
        assert_eq!(fs::metadata(&recovery).unwrap().len(), first_size);
        let command = skills_command("request-torn");
        let request = envelope("run-torn", "request-torn", "hash-torn", command);
        assert!(accept(&request, "agent-a").is_err());
    }

    #[test]
    fn conflicting_completion_preserves_canonical_result_and_evidence() {
        let _home_lock = TEST_HOME_LOCK.lock();
        let _home = test_home();
        let command = skills_command("request-complete");
        let request = envelope("run-complete", "request-complete", "hash-complete", command);
        accept(&request, "agent-a").unwrap();
        assert!(matches!(
            claim_started(
                &request.run_id,
                &request.request_id,
                &request.command_hash,
                "agent-a"
            )
            .unwrap(),
            ClaimOutcome::Claimed
        ));
        let canonical = serde_json::json!({"ok": true});
        assert_eq!(
            mark_completed(
                &request.run_id,
                &request.request_id,
                &request.command_hash,
                "agent-a",
                &canonical
            )
            .unwrap(),
            CompletionOutcome::Persisted
        );
        let conflicting = serde_json::json!({"ok": false});
        assert!(mark_completed(
            &request.run_id,
            &request.request_id,
            &request.command_hash,
            "agent-a",
            &conflicting
        )
        .is_err());
        assert_eq!(
            latest_records()
                .unwrap()
                .get(&request.run_id)
                .and_then(|record| record.result.clone()),
            Some(canonical)
        );
        let ledger = ledger_path().unwrap();
        let contents = fs::read_to_string(ledger).unwrap();
        assert!(contents.contains("transport_completion_result_conflict"));
    }

    #[test]
    fn conflicting_identity_is_rejected_with_evidence() {
        let _home_lock = TEST_HOME_LOCK.lock();
        let _home = test_home();
        let command = skills_command("request-identity");
        let request = envelope(
            "run-identity",
            "request-identity",
            "hash-identity",
            command.clone(),
        );
        accept(&request, "agent-a").unwrap();
        let conflicting = envelope("run-identity", "request-other", "hash-other", command);
        assert!(matches!(
            accept(&conflicting, "agent-a").unwrap(),
            AcceptOutcome::HashMismatch
        ));
        let contents = fs::read_to_string(ledger_path().unwrap()).unwrap();
        assert!(contents.contains("transport_identity_mismatch"));
    }

    #[test]
    fn unowned_legacy_is_blocked_including_explicit_agent_target() {
        let _home_lock = TEST_HOME_LOCK.lock();
        let _home = test_home();
        let legacy_command = skills_command("request-legacy");
        let legacy = envelope(
            "run-legacy",
            "request-legacy",
            "hash-legacy",
            legacy_command,
        );
        let ledger = ledger_path().unwrap();
        ensure_parent(&ledger).unwrap();
        append_json_record(
            &ledger,
            &LedgerRecord {
                run_id: legacy.run_id.clone(),
                request_id: legacy.request_id.clone(),
                command_hash: legacy.command_hash.clone(),
                status: "accepted".to_string(),
                agent_id: None,
                command: Some(legacy.command.clone()),
                result: None,
                reason: None,
                conflict: None,
            },
        );
        assert!(matches!(
            accept(&legacy, "agent-a").unwrap(),
            AcceptOutcome::LegacyUnowned
        ));

        let adopted_command = exec_command("request-adopt", "agent-a");
        let adopted = envelope(
            "run-adopt",
            "request-adopt",
            "hash-adopt",
            adopted_command.clone(),
        );
        append_json_record(
            &ledger,
            &LedgerRecord {
                run_id: adopted.run_id.clone(),
                request_id: adopted.request_id.clone(),
                command_hash: adopted.command_hash.clone(),
                status: "completed".to_string(),
                agent_id: None,
                command: Some(adopted_command),
                result: Some(serde_json::json!({"legacy": true})),
                reason: None,
                conflict: None,
            },
        );
        assert!(matches!(
            accept(&adopted, "agent-a").unwrap(),
            AcceptOutcome::LegacyUnowned
        ));
        assert_eq!(
            latest_records()
                .unwrap()
                .get(&adopted.run_id)
                .and_then(|record| record.agent_id.as_deref()),
            None
        );
    }

    #[test]
    fn missing_identity_is_rejected_without_fabricating_hash() {
        let _home_lock = TEST_HOME_LOCK.lock();
        let _home = test_home();
        let request = envelope(
            "run-missing-hash",
            "request-missing-hash",
            "",
            skills_command("request-missing-hash"),
        );
        assert!(accept(&request, "agent-a").is_err());
        assert!(!ledger_path().unwrap().exists());
        assert!(mark_completed(
            "run-missing-hash",
            "request-missing-hash",
            "",
            "agent-a",
            &serde_json::json!({"ok": true}),
        )
        .is_err());
    }

    #[test]
    fn compaction_collapses_completed_transition_history_and_retains_evidence() {
        let _home_lock = TEST_HOME_LOCK.lock();
        let _home = test_home();
        let ledger = ledger_path().unwrap();
        let command = skills_command("request-compact");
        let record = |run_id: &str,
                      request_id: &str,
                      hash: &str,
                      status: &str,
                      owner: Option<&str>,
                      result: Option<Value>|
         -> LedgerRecord {
            LedgerRecord {
                run_id: run_id.to_string(),
                request_id: request_id.to_string(),
                command_hash: hash.to_string(),
                status: status.to_string(),
                agent_id: owner.map(str::to_string),
                command: Some(command.clone()),
                result,
                reason: None,
                conflict: None,
            }
        };
        append_json_record(
            &ledger,
            &record(
                "run-done",
                "request-compact",
                "hash-compact",
                "accepted",
                Some("agent-a"),
                None,
            ),
        );
        append_json_record(
            &ledger,
            &record(
                "run-done",
                "request-compact",
                "hash-compact",
                "started",
                Some("agent-a"),
                None,
            ),
        );
        append_json_record(
            &ledger,
            &record(
                "run-done",
                "request-compact",
                "hash-compact",
                "completed",
                Some("agent-a"),
                Some(serde_json::json!({"done": true})),
            ),
        );
        append_json_record(
            &ledger,
            &record(
                "run-done",
                "request-compact",
                "hash-compact",
                "completed",
                Some("agent-a"),
                Some(serde_json::json!({"done": false})),
            ),
        );
        append_json_record(
            &ledger,
            &record(
                "run-live",
                "request-live",
                "hash-live",
                "accepted",
                Some("agent-a"),
                None,
            ),
        );
        append_json_record(
            &ledger,
            &record(
                "run-unknown",
                "request-unknown",
                "hash-unknown",
                "unknown",
                Some("agent-a"),
                None,
            ),
        );
        append_json_record(
            &ledger,
            &record(
                "run-legacy",
                "request-legacy",
                "hash-legacy",
                "accepted",
                None,
                None,
            ),
        );
        append_json_record(
            &ledger,
            &LedgerRecord {
                run_id: "run-conflict".to_string(),
                request_id: "request-conflict".to_string(),
                command_hash: "hash-conflict".to_string(),
                status: "conflict".to_string(),
                agent_id: Some("agent-a".to_string()),
                command: Some(command.clone()),
                result: Some(serde_json::json!({"conflict": true})),
                reason: Some("preserve-conflict".to_string()),
                conflict: Some(serde_json::json!({"canonical": true})),
            },
        );
        let before = fs::read(&ledger).unwrap();
        with_ledger_lock(compact_locked).unwrap();
        let after = fs::read(&ledger).unwrap();
        assert!(after.len() < before.len());
        assert!(String::from_utf8_lossy(&after).contains("\"done\":true"));
        assert!(String::from_utf8_lossy(&after).contains("\"done\":false"));
        assert!(String::from_utf8_lossy(&after).contains("\"status\":\"completed\""));
        for marker in ["run-live", "run-unknown", "run-legacy", "preserve-conflict"] {
            assert!(String::from_utf8_lossy(&after).contains(marker));
        }
        assert_eq!(fs::read(recovery_backup_path(&ledger)).unwrap(), before);
    }

    fn append_json_record(path: &Path, record: &LedgerRecord) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{}", serde_json::to_string(record).unwrap()).unwrap();
    }
}
