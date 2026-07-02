use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use agentic_gpt_protocol::{AgentMessage, HubCommand, HubCommandEnvelope};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::utils::{agentic_home, ensure_parent};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LedgerRecord {
    pub(crate) run_id: String,
    pub(crate) request_id: String,
    pub(crate) command_hash: String,
    pub(crate) status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<HubCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum AcceptOutcome {
    FirstAccepted,
    DuplicateAccepted,
    DuplicateStarted,
    Completed(Value),
    HashMismatch,
}

pub(crate) fn latest_records() -> Result<HashMap<String, LedgerRecord>> {
    let path = ledger_path()?;
    let mut records = HashMap::new();
    if !path.exists() {
        return Ok(records);
    }
    let file = fs::File::open(path)?;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<LedgerRecord>(&line) {
            records.insert(record.run_id.clone(), record);
        }
    }
    Ok(records)
}

pub(crate) fn accept(envelope: &HubCommandEnvelope) -> Result<AcceptOutcome> {
    let latest = latest_records()?;
    if let Some(existing) = latest.get(&envelope.run_id) {
        if existing.request_id != envelope.request_id
            || existing.command_hash != envelope.command_hash
        {
            return Ok(AcceptOutcome::HashMismatch);
        }
        return match existing.status.as_str() {
            "completed" => Ok(AcceptOutcome::Completed(
                existing.result.clone().unwrap_or(Value::Null),
            )),
            "started" | "running" => Ok(AcceptOutcome::DuplicateStarted),
            _ => Ok(AcceptOutcome::DuplicateAccepted),
        };
    }
    append(&LedgerRecord {
        run_id: envelope.run_id.clone(),
        request_id: envelope.request_id.clone(),
        command_hash: envelope.command_hash.clone(),
        status: "accepted".to_string(),
        command: Some(envelope.command.clone()),
        result: None,
        reason: None,
    })?;
    Ok(AcceptOutcome::FirstAccepted)
}

pub(crate) fn mark_started(run_id: &str) -> Result<()> {
    if let Some(mut record) = latest_records()?.remove(run_id) {
        record.status = "started".to_string();
        append(&record)?;
    }
    Ok(())
}

pub(crate) fn mark_completed(run_id: &str, request_id: &str, result: &Value) -> Result<()> {
    let mut record = latest_records()?.remove(run_id).unwrap_or(LedgerRecord {
        run_id: run_id.to_string(),
        request_id: request_id.to_string(),
        command_hash: String::new(),
        status: String::new(),
        command: None,
        result: None,
        reason: None,
    });
    record.status = "completed".to_string();
    record.result = Some(result.clone());
    append(&record)?;
    Ok(())
}

pub(crate) fn ack_message(envelope: &HubCommandEnvelope) -> AgentMessage {
    AgentMessage::TransportAck {
        event_id: envelope.event_id.clone(),
        run_id: envelope.run_id.clone(),
        request_id: envelope.request_id.clone(),
        command_hash: envelope.command_hash.clone(),
    }
}

pub(crate) fn completed_response(record: &LedgerRecord) -> Option<AgentMessage> {
    Some(AgentMessage::Response {
        run_id: Some(record.run_id.clone()),
        request_id: record.request_id.clone(),
        data: record.result.clone()?,
    })
}

fn append(record: &LedgerRecord) -> Result<()> {
    let path = ledger_path()?;
    ensure_parent(&path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

fn ledger_path() -> Result<PathBuf> {
    Ok(agentic_home()?.join("transport-runs.jsonl"))
}
