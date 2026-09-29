use agentic_gpt_protocol::{AgentRunReport, HubCommand};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::HubState;
use crate::utils::{random_id, sha256_hex};

const RUN_TTL_HOURS: i64 = 24;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentRun {
    pub(crate) run_id: String,
    pub(crate) request_id: String,
    pub(crate) agent_id: String,
    pub(crate) command_type: String,
    pub(crate) command_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
    pub(crate) status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    /// Whether the authoritative result payload is still retained in Hub storage.
    pub(crate) result_retained: bool,
    /// True when a completed result payload was compacted after its retention window.
    pub(crate) result_omitted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) process_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) process: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedRun {
    pub(crate) run_id: String,
    pub(crate) request_id: String,
    pub(crate) command_hash: String,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StoreResultOutcome {
    Stored { command_hash: String },
    Duplicate { command_hash: String },
    Conflict,
    Unmatched,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingReplay {
    pub(crate) run_id: String,
    pub(crate) request_id: String,
    pub(crate) command_hash: String,
    pub(crate) command: HubCommand,
}

pub(crate) fn command_type(command: &HubCommand) -> &'static str {
    match command {
        HubCommand::Exec { .. } => "process.exec",
        HubCommand::ProcessBatch { .. } => "process.batch",
        HubCommand::ProcessList { .. } => "process.list",
        HubCommand::ProcessStatus { .. } => "process.status",
        HubCommand::ProcessOutput { .. } => "process.output",
        HubCommand::ProcessResult { .. } => "process.result",
        HubCommand::ProcessCancel { .. } => "process.cancel",
        HubCommand::TmuxListSessions { .. } => "tmux.listSessions",
        HubCommand::TmuxListPanes { .. } => "tmux.listPanes",
        HubCommand::TmuxCapturePane { .. } => "tmux.capturePane",
        HubCommand::TmuxPasteText { .. } => "tmux.pasteText",
        HubCommand::TmuxExec { .. } => "tmux.exec",
        HubCommand::TmuxCreateSession { .. } => "tmux.createSession",
        HubCommand::TmuxCloseSession { .. } => "tmux.closeSession",
        HubCommand::McpListServers { .. } => "mcp.listServers",
        HubCommand::McpListTools { .. } => "mcp.listTools",
        HubCommand::McpCallTool { .. } => "mcp.callTool",
        HubCommand::McpBatch { .. } => "mcp.batch",
        HubCommand::UserNotifyDeliver { .. } => "user.notify.deliver",
        HubCommand::RoomDiaryActive { .. } => "room.diary.active",
        HubCommand::RoomDiaryRead { .. } => "room.diary.read",
        HubCommand::RoomNotebookRecent { .. } => "room.notebook.recent",
        HubCommand::RoomNotebookSearch { .. } => "room.notebook.search",
        HubCommand::RoomNotebookRead { .. } => "room.notebook.read",
        HubCommand::RoomStateList { .. } => "room.state.list",
        HubCommand::RoomStateRead { .. } => "room.state.read",
        HubCommand::RoomMaintenanceStatus { .. } => "room.maintenance.status",
        HubCommand::RoomMaintenanceSubmit { .. } => "room.maintenance.submit",
        HubCommand::RoomBootstrap { .. } => "room.bootstrap",
        HubCommand::RoomBootstrapRead { .. } => "room.bootstrap.read",
        HubCommand::Bootstrap { .. } => "bootstrap",
        HubCommand::BootstrapRead { .. } => "bootstrap.read",
        HubCommand::SkillsList { .. } => "skills.list",
        HubCommand::SkillsRead { .. } => "skills.read",
        HubCommand::SkillsSearch { .. } => "skills.search",
        HubCommand::SkillsActive { .. } => "skills.active",
        HubCommand::SkillsActivate { .. } => "skills.activate",
        HubCommand::SkillsDeactivate { .. } => "skills.deactivate",
        HubCommand::SkillsInstall { .. } => "skills.install",
        HubCommand::SkillsInstallGet { .. } => "skills.install.get",
        HubCommand::SkillsInstallCancel { .. } => "skills.install.cancel",
        HubCommand::SkillsRun { .. } => "skills.run",
    }
}

pub(crate) fn prepare_run(
    state: &HubState,
    agent_id: &str,
    request_id: &str,
    command: &HubCommand,
) -> Result<PreparedRun> {
    let command_json = serde_json::to_string(command)?;
    let command_hash = sha256_hex(&command_json);
    let run_id = random_id("run");
    let now = Utc::now();
    let expires_at = now + Duration::hours(RUN_TTL_HOURS);
    let conn = state.db.lock().unwrap();
    conn.execute(
        "insert into agent_runs(
            run_id, request_id, agent_id, command_type, command_json, command_hash,
            status, created_at, updated_at, expires_at
        ) values (?1, ?2, ?3, ?4, ?5, ?6, 'created', ?7, ?7, ?8)",
        params![
            run_id,
            request_id,
            agent_id,
            command_type(command),
            command_json,
            command_hash,
            now,
            expires_at
        ],
    )?;
    Ok(PreparedRun {
        run_id,
        request_id: request_id.to_string(),
        command_hash,
    })
}

pub(crate) fn pending_unacked(state: &HubState, agent_id: &str) -> Result<Vec<PendingReplay>> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "select run_id, request_id, command_hash, command_json
         from agent_runs
         where agent_id = ?1
           and acked_at is null
           and result_json is null
           and status in ('created', 'dispatched', 'timeout_waiting_result')
         order by created_at asc",
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        let command_json: String = row.get(3)?;
        let command = serde_json::from_str::<HubCommand>(&command_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        Ok(PendingReplay {
            run_id: row.get(0)?,
            request_id: row.get(1)?,
            command_hash: row.get(2)?,
            command,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| anyhow!(error))
}

pub(crate) fn mark_dispatched(state: &HubState, run_id: &str) -> Result<()> {
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    conn.execute(
        "update agent_runs
         set status = 'dispatched', updated_at = ?1
         where run_id = ?2 and result_json is null and status = 'created'",
        params![now, run_id],
    )?;
    Ok(())
}

pub(crate) fn mark_acked(
    state: &HubState,
    agent_id: &str,
    run_id: &str,
    request_id: &str,
    command_hash: &str,
) -> Result<bool> {
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    let changed = conn.execute(
        "update agent_runs
         set status = case
                 when result_json is null
                      and status in ('created', 'dispatched', 'timeout_waiting_result')
                 then 'acked'
                 else status
             end,
             acked_at = coalesce(acked_at, ?1),
             updated_at = case
                 when result_json is null
                      and status in ('created', 'dispatched', 'timeout_waiting_result')
                 then ?1
                 else updated_at
             end
         where run_id = ?2
           and request_id = ?3
           and agent_id = ?4
           and command_hash = ?5",
        params![now, run_id, request_id, agent_id, command_hash],
    )?;
    Ok(changed > 0)
}

pub(crate) fn mark_status(
    state: &HubState,
    agent_id: &str,
    run_id: &str,
    request_id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<bool> {
    if !matches!(status, "started" | "running" | "failed" | "unknown") {
        return Ok(false);
    }

    let conn = state.db.lock().unwrap();
    let current: Option<(String, bool)> = conn
        .query_row(
            "select status, result_json is not null
             from agent_runs
             where run_id = ?1 and request_id = ?2 and agent_id = ?3",
            params![run_id, request_id, agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((current_status, has_result)) = current else {
        return Ok(false);
    };
    if has_result {
        return Ok(true);
    }

    let should_update = match current_status.as_str() {
        "created" | "dispatched" | "timeout_waiting_result" | "acked" => true,
        "started" => matches!(status, "running" | "failed" | "unknown"),
        "running" => matches!(status, "failed" | "unknown"),
        "failed" | "unknown" | "completed" | "not_sent" => false,
        _ => false,
    };
    if !should_update {
        return Ok(true);
    }

    let now = Utc::now();
    conn.execute(
        "update agent_runs
         set status = ?1, reason = ?2, updated_at = ?3
         where run_id = ?4
           and request_id = ?5
           and agent_id = ?6
           and result_json is null",
        params![status, reason, now, run_id, request_id, agent_id],
    )?;
    Ok(true)
}

pub(crate) fn mark_timeout(state: &HubState, run_id: &str, reason: &str) -> Result<()> {
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    conn.execute(
        "update agent_runs
         set status = 'timeout_waiting_result', reason = ?1, updated_at = ?2
         where run_id = ?3
           and result_json is null
           and status in ('created', 'dispatched')",
        params![reason, now, run_id],
    )?;
    Ok(())
}

pub(crate) fn mark_not_sent(state: &HubState, run_id: &str, reason: &str) -> Result<()> {
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    conn.execute(
        "update agent_runs
         set status = 'not_sent', reason = ?1, updated_at = ?2
         where run_id = ?3
           and result_json is null
           and status in ('created', 'dispatched')",
        params![reason, now, run_id],
    )?;
    Ok(())
}

pub(crate) fn store_result(
    state: &HubState,
    agent_id: &str,
    run_id: &str,
    request_id: &str,
    result: &Value,
) -> Result<StoreResultOutcome> {
    let result_json = serde_json::to_string(result)?;
    let result_hash = sha256_hex(&result_json);
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    let Some((command_hash, existing_result_json, existing_result_hash)) = conn
        .query_row(
            "select command_hash, result_json, result_hash
             from agent_runs
             where run_id = ?1 and request_id = ?2 and agent_id = ?3",
            params![run_id, request_id, agent_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(StoreResultOutcome::Unmatched);
    };

    if existing_result_json.is_some() || existing_result_hash.is_some() {
        let canonical_hash = existing_result_hash
            .or_else(|| existing_result_json.as_deref().map(sha256_hex))
            .expect("result JSON or hash is present");
        if canonical_hash == result_hash {
            return Ok(StoreResultOutcome::Duplicate { command_hash });
        }
        conn.execute(
            "update agent_runs
             set conflict_json = ?1, updated_at = ?2
             where run_id = ?3 and request_id = ?4 and agent_id = ?5",
            params![result_json, now, run_id, request_id, agent_id],
        )?;
        return Ok(StoreResultOutcome::Conflict);
    }

    conn.execute(
        "update agent_runs
         set status = 'completed', result_json = ?1, result_hash = ?2, updated_at = ?3
         where run_id = ?4
           and request_id = ?5
           and agent_id = ?6
           and result_json is null",
        params![result_json, result_hash, now, run_id, request_id, agent_id],
    )?;
    Ok(StoreResultOutcome::Stored { command_hash })
}

pub(crate) fn upsert_agent_report(
    state: &HubState,
    agent_id: &str,
    report: AgentRunReport,
) -> Result<()> {
    if report.run_id.trim().is_empty() || report.request_id.trim().is_empty() {
        return Err(anyhow!("invalid_agent_run_report"));
    }
    if !matches!(report.status.as_str(), "started" | "completed" | "failed") {
        return Err(anyhow!("invalid_agent_run_status"));
    }
    if !matches!(report.detail.as_str(), "metadata" | "full") {
        return Err(anyhow!("invalid_agent_run_detail"));
    }
    let now = Utc::now();
    let detail = report.detail.clone();
    let result = if detail == "full" {
        report
            .result
            .map(|value| serde_json::to_string(&value.value))
            .transpose()?
    } else {
        None
    };
    let arguments = if detail == "full" {
        report
            .arguments
            .map(|value| serde_json::to_string(&value.value))
            .transpose()?
    } else {
        None
    };
    let process = if detail == "full" {
        report
            .process
            .map(|value| serde_json::to_string(&value))
            .transpose()?
    } else {
        None
    };
    let result_hash = result.as_deref().map(sha256_hex);
    let command_json = serde_json::json!({
        "source": report.source,
        "toolName": report.tool_name,
        "profile": report.profile,
    });
    let command_json = serde_json::to_string(&command_json)?;
    let command_hash = sha256_hex(&command_json);
    let conn = state.db.lock().unwrap();
    let existing_identity: Option<(String, String)> = conn
        .query_row(
            "select agent_id, request_id from agent_runs where run_id = ?1",
            params![report.run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((existing_agent, existing_request_id)) = existing_identity {
        if existing_agent != agent_id {
            return Err(anyhow!("agent_run_owner_mismatch"));
        }
        if existing_request_id != report.request_id {
            return Err(anyhow!("agent_run_request_mismatch"));
        }
    }
    let existing_status: Option<String> = conn
        .query_row(
            "select status from agent_runs where run_id = ?1",
            params![report.run_id],
            |row| row.get(0),
        )
        .optional()?;
    let existing_result: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "select result_json, result_hash from agent_runs where run_id = ?1",
            params![report.run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((existing_result_json, existing_result_hash)) = existing_result {
        if existing_result_json.is_some() || existing_result_hash.is_some() {
            let canonical_hash = existing_result_hash
                .or_else(|| existing_result_json.as_deref().map(sha256_hex))
                .expect("result JSON or hash is present");
            if result_hash
                .as_ref()
                .is_some_and(|incoming| incoming != &canonical_hash)
            {
                conn.execute(
                    "update agent_runs set conflict_json = ?1, updated_at = ?2 where run_id = ?3",
                    params![result, now, report.run_id],
                )?;
            }
            return Ok(());
        }
    }
    if matches!(existing_status.as_deref(), Some("unknown" | "not_sent")) {
        return Ok(());
    }
    if matches!(existing_status.as_deref(), Some("completed")) && report.status != "completed" {
        return Ok(());
    }
    if matches!(existing_status.as_deref(), Some("completed" | "failed"))
        && report.status == "started"
    {
        return Ok(());
    }
    let expires_at = report.started_at + Duration::hours(RUN_TTL_HOURS);
    conn.execute(
        "insert into agent_runs(
            run_id, request_id, agent_id, command_type, command_json, command_hash,
            status, result_json, result_hash, reason, created_at, updated_at, expires_at,
            source, profile, detail, process_id, duration_ms, exit_code, arguments_json, process_json
        ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
        on conflict(run_id) do update set
            status = excluded.status,
            result_json = coalesce(excluded.result_json, agent_runs.result_json),
            result_hash = coalesce(excluded.result_hash, agent_runs.result_hash),
            reason = excluded.reason,
            updated_at = excluded.updated_at,
            process_id = excluded.process_id,
            duration_ms = excluded.duration_ms,
            exit_code = excluded.exit_code,
            arguments_json = coalesce(excluded.arguments_json, agent_runs.arguments_json),
            process_json = coalesce(excluded.process_json, agent_runs.process_json)",
        params![
            report.run_id,
            report.request_id,
            agent_id,
            report.tool_name,
            command_json,
            command_hash,
            report.status,
            result,
            result_hash,
            report.reason,
            report.started_at,
            report.updated_at.max(now),
            expires_at,
            report.source,
            report.profile,
            detail,
            report.process_id,
            report.duration_ms.map(|value| value as i64),
            report.exit_code,
            arguments,
            process,
        ],
    )?;
    Ok(())
}
pub(crate) fn get_run(state: &HubState, run_id: &str) -> Result<Option<AgentRun>> {
    let conn = state.db.lock().unwrap();
    conn.query_row(
        "select run_id, request_id, agent_id, command_type, command_hash, source, profile, detail,
                status, result_json, result_hash, arguments_json, process_id, process_json, reason, created_at,
                updated_at, expires_at
         from agent_runs where run_id = ?1",
        params![run_id],
        |row| {
            let result_json: Option<String> = row.get(9)?;
            let result_hash: Option<String> = row.get(10)?;
            let arguments_json: Option<String> = row.get(11)?;
            let process_id: Option<String> = row.get(12)?;
            let process_json: Option<String> = row.get(13)?;
            let status: String = row.get(8)?;
            let expires_at: Option<DateTime<Utc>> = row.get(17)?;
            let result_retained = result_json.is_some();
            let result_omitted = !result_retained
                && result_hash.is_some()
                && status == "completed"
                && expires_at.is_some_and(|value| value <= Utc::now());
            Ok(AgentRun {
                run_id: row.get(0)?,
                request_id: row.get(1)?,
                agent_id: row.get(2)?,
                command_type: row.get(3)?,
                command_hash: row.get(4)?,
                source: row.get(5)?,
                profile: row.get(6)?,
                detail: row.get(7)?,
                status,
                result: result_json.and_then(|json| serde_json::from_str(&json).ok()),
                result_retained,
                result_omitted,
                arguments: arguments_json.and_then(|json| serde_json::from_str(&json).ok()),
                process_id,
                process: process_json.and_then(|json| serde_json::from_str(&json).ok()),
                reason: row.get(14)?,
                created_at: row.get(15)?,
                updated_at: row.get(16)?,
            })
        },
    )
    .optional()
    .map_err(|error| anyhow!(error))
}

pub(crate) fn list_runs(
    state: &HubState,
    agent_id: Option<&str>,
    source: Option<&str>,
    status: Option<&str>,
    since: Option<DateTime<Utc>>,
    limit: usize,
) -> Result<Vec<AgentRun>> {
    let ids = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "select run_id from agent_runs
             order by created_at desc limit 1000",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut runs = Vec::new();
    for run_id in ids {
        let Some(run) = get_run(state, &run_id)? else {
            continue;
        };
        if agent_id.is_some_and(|value| run.agent_id != value)
            || source.is_some_and(|value| run.source.as_deref() != Some(value))
            || status.is_some_and(|value| run.status != value)
            || since.is_some_and(|value| run.created_at < value)
        {
            continue;
        }
        runs.push(run);
        if runs.len() >= limit {
            break;
        }
    }
    Ok(runs)
}

pub(crate) fn mark_stale_acked_unknown(
    state: &HubState,
    older_than: DateTime<Utc>,
) -> Result<usize> {
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    conn.execute(
        "update agent_runs
         set status = 'unknown',
             reason = 'acked_result_timeout',
             updated_at = ?1
         where acked_at is not null
           and result_json is null
           and status in ('acked', 'started', 'running')
           and updated_at < ?2",
        params![now, older_than],
    )
    .map_err(|error| anyhow!(error))
}

pub(crate) fn prune_expired(state: &HubState) -> Result<usize> {
    let conn = state.db.lock().unwrap();
    // Keep every identity/status/hash tombstone. Only completed payloads with no
    // conflict evidence are compacted after the existing 24-hour window.
    let expiring: i64 = conn.query_row(
        "select count(*) from agent_runs
         where expires_at is not null
           and expires_at <= ?1
           and status = 'completed'
           and result_json is not null
           and result_hash is not null
           and conflict_json is null",
        params![Utc::now()],
        |row| row.get(0),
    )?;
    if expiring == 0 {
        return Ok(0);
    }
    crate::db::backup_before_retention(&conn)?;
    conn.execute(
        "update agent_runs
         set result_json = null,
             arguments_json = null,
             process_json = null
         where expires_at is not null
           and expires_at <= ?1
           and status = 'completed'
           and result_json is not null
           and result_hash is not null
           and conflict_json is null",
        params![Utc::now()],
    )
    .map_err(|error| anyhow!(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteConfirmationConfig;
    use crate::db::init_db;
    use crate::state::McpProfile;
    use crate::{HubConfig, NtfyConfig};
    use agentic_gpt_protocol::{AgentRunReport, BoundedJsonValue};
    use rusqlite::Connection;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::Mutex;

    fn test_state() -> HubState {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        HubState {
            api_key: "test-api-key".to_string(),
            db: Arc::new(StdMutex::new(conn)),
            config: Arc::new(HubConfig {
                remote_confirmation: RemoteConfirmationConfig {
                    enabled: false,
                    provider: "none".to_string(),
                    timeout_seconds: 45,
                    ntfy: NtfyConfig {
                        server_url: String::new(),
                        topic: String::new(),
                        callback_base_url: String::new(),
                    },
                },
            }),
            mcp_profile: McpProfile::Full,
            agents: Arc::new(crate::agents::lifecycle::Connections::new()),
            dispatch: Arc::new(crate::agents::dispatch::Dispatch::new()),
            confirmations: Arc::new(crate::confirmation::Confirmations::new()),
            process_cache: Arc::new(crate::state::ProcessCache::new()),
            boot_generations: Arc::new(Mutex::new(HashMap::new())),
            active_room: Arc::new(Mutex::new(None)),
            http: reqwest::Client::new(),
            public_base_url: None,
            oauth_codes: Arc::new(Mutex::new(HashMap::new())),
            oauth_tokens: Arc::new(Mutex::new(HashMap::new())),
            ntfy_health: Arc::new(Mutex::new(None)),
        }
    }

    #[test]
    fn stores_late_result_idempotently_by_run_id_and_request_id() {
        let state = test_state();
        let command = HubCommand::McpListServers {
            request_id: "req_1".to_string(),
        };
        let run = prepare_run(&state, "agent", "req_1", &command).unwrap();
        assert!(mark_acked(&state, "agent", &run.run_id, "req_1", &run.command_hash).unwrap());
        let result = serde_json::json!({ "servers": [] });
        assert_eq!(
            store_result(&state, "agent", &run.run_id, "req_1", &result).unwrap(),
            StoreResultOutcome::Stored {
                command_hash: run.command_hash.clone()
            }
        );
        assert_eq!(
            store_result(&state, "agent", &run.run_id, "req_1", &result).unwrap(),
            StoreResultOutcome::Duplicate {
                command_hash: run.command_hash.clone()
            }
        );
        let stored = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.result.unwrap(), result);
    }
    #[test]
    fn completed_result_survives_dispatch_and_wait_timeout_updates() {
        let state = test_state();
        let command = HubCommand::McpListServers {
            request_id: "req_completed_guard".to_string(),
        };
        let run = prepare_run(&state, "agent", "req_completed_guard", &command).unwrap();
        assert!(mark_status(
            &state,
            "agent",
            &run.run_id,
            "req_completed_guard",
            "running",
            Some("before_result"),
        )
        .unwrap());
        let result = serde_json::json!({ "servers": ["canonical"] });
        assert!(matches!(
            store_result(&state, "agent", &run.run_id, "req_completed_guard", &result,).unwrap(),
            StoreResultOutcome::Stored { .. }
        ));
        let completed = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.result, Some(result.clone()));
        assert_eq!(completed.reason.as_deref(), Some("before_result"));

        mark_dispatched(&state, &run.run_id).unwrap();
        let after_dispatch = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(after_dispatch.status, completed.status);
        assert_eq!(after_dispatch.result, completed.result);
        assert_eq!(after_dispatch.reason, completed.reason);
        assert_eq!(after_dispatch.updated_at, completed.updated_at);

        mark_timeout(&state, &run.run_id, "late_timeout").unwrap();
        let after_timeout = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(after_timeout.status, completed.status);
        assert_eq!(after_timeout.result, completed.result);
        assert_eq!(after_timeout.reason, completed.reason);
        assert_eq!(after_timeout.updated_at, completed.updated_at);

        let unfinished = prepare_run(
            &state,
            "agent",
            "req_unfinished_guard",
            &HubCommand::McpListServers {
                request_id: "req_unfinished_guard".to_string(),
            },
        )
        .unwrap();
        mark_dispatched(&state, &unfinished.run_id).unwrap();
        assert_eq!(
            get_run(&state, &unfinished.run_id).unwrap().unwrap().status,
            "dispatched"
        );
        mark_timeout(&state, &unfinished.run_id, "process_exec_timeout").unwrap();
        let timed_out = get_run(&state, &unfinished.run_id).unwrap().unwrap();
        assert_eq!(timed_out.status, "timeout_waiting_result");
        assert_eq!(timed_out.reason.as_deref(), Some("process_exec_timeout"));
    }
    #[test]
    fn wp1_transport_status_preserves_completed_result() {
        let state = test_state();
        let command = HubCommand::McpListServers {
            request_id: "req_wp1_completed_status".to_string(),
        };
        let run = prepare_run(&state, "agent", "req_wp1_completed_status", &command).unwrap();
        let result = serde_json::json!({ "servers": ["canonical"] });
        assert!(matches!(
            store_result(
                &state,
                "agent",
                &run.run_id,
                "req_wp1_completed_status",
                &result,
            )
            .unwrap(),
            StoreResultOutcome::Stored { .. }
        ));
        let completed = get_run(&state, &run.run_id).unwrap().unwrap();

        assert!(mark_status(
            &state,
            "agent",
            &run.run_id,
            "req_wp1_completed_status",
            "failed",
            Some("late_failure"),
        )
        .unwrap());

        let after_status = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(after_status.status, "completed");
        assert_eq!(after_status.result, Some(result));
        assert_eq!(after_status.reason, completed.reason);
        assert_eq!(after_status.updated_at, completed.updated_at);
    }
    #[test]
    fn wp1_matching_stale_status_is_idempotent_and_foreign_is_rejected() {
        let state = test_state();
        let command = HubCommand::McpListServers {
            request_id: "req_wp1_status_tuple".to_string(),
        };
        let run = prepare_run(&state, "agent", "req_wp1_status_tuple", &command).unwrap();
        assert!(mark_status(
            &state,
            "agent",
            &run.run_id,
            "req_wp1_status_tuple",
            "started",
            Some("remote_started"),
        )
        .unwrap());
        let before = get_run(&state, &run.run_id).unwrap().unwrap();

        assert!(mark_status(
            &state,
            "agent",
            &run.run_id,
            "req_wp1_status_tuple",
            "started",
            Some("stale_started"),
        )
        .unwrap());
        assert!(!mark_status(
            &state,
            "foreign",
            &run.run_id,
            "req_wp1_status_tuple",
            "running",
            Some("foreign"),
        )
        .unwrap());

        let after = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(after.status, before.status);
        assert_eq!(after.reason, before.reason);
        assert_eq!(after.updated_at, before.updated_at);
    }

    #[test]
    fn wp1_remote_progress_survives_dispatch_timeout_and_late_ack() {
        let state = test_state();
        let started = prepare_run(
            &state,
            "agent",
            "req_wp1_started_progress",
            &HubCommand::McpListServers {
                request_id: "req_wp1_started_progress".to_string(),
            },
        )
        .unwrap();
        assert!(mark_status(
            &state,
            "agent",
            &started.run_id,
            "req_wp1_started_progress",
            "started",
            Some("remote_started"),
        )
        .unwrap());
        let started_before = get_run(&state, &started.run_id).unwrap().unwrap();

        mark_dispatched(&state, &started.run_id).unwrap();
        mark_timeout(&state, &started.run_id, "process_exec_timeout").unwrap();
        assert!(mark_acked(
            &state,
            "agent",
            &started.run_id,
            "req_wp1_started_progress",
            &started.command_hash,
        )
        .unwrap());

        let started_after = get_run(&state, &started.run_id).unwrap().unwrap();
        assert_eq!(started_after.status, "started");
        assert_eq!(started_after.reason.as_deref(), Some("remote_started"));
        assert_eq!(started_after.updated_at, started_before.updated_at);

        let late_progress = prepare_run(
            &state,
            "agent",
            "req_wp1_late_progress",
            &HubCommand::McpListServers {
                request_id: "req_wp1_late_progress".to_string(),
            },
        )
        .unwrap();
        mark_dispatched(&state, &late_progress.run_id).unwrap();
        mark_timeout(&state, &late_progress.run_id, "process_exec_timeout").unwrap();
        let timeout_before_progress = get_run(&state, &late_progress.run_id).unwrap().unwrap();
        assert!(mark_status(
            &state,
            "agent",
            &late_progress.run_id,
            "req_wp1_late_progress",
            "started",
            Some("remote_started_after_timeout"),
        )
        .unwrap());
        let late_started = get_run(&state, &late_progress.run_id).unwrap().unwrap();
        assert_eq!(late_started.status, "started");
        assert_eq!(
            late_started.reason.as_deref(),
            Some("remote_started_after_timeout")
        );
        assert!(late_started.updated_at >= timeout_before_progress.updated_at);
        assert!(mark_status(
            &state,
            "agent",
            &late_progress.run_id,
            "req_wp1_late_progress",
            "running",
            Some("remote_running_after_timeout"),
        )
        .unwrap());
        let late_running = get_run(&state, &late_progress.run_id).unwrap().unwrap();
        assert_eq!(late_running.status, "running");
        assert_eq!(
            late_running.reason.as_deref(),
            Some("remote_running_after_timeout")
        );
        assert!(late_running.updated_at >= late_started.updated_at);

        let running = prepare_run(
            &state,
            "agent",
            "req_wp1_running_progress",
            &HubCommand::McpListServers {
                request_id: "req_wp1_running_progress".to_string(),
            },
        )
        .unwrap();
        assert!(mark_status(
            &state,
            "agent",
            &running.run_id,
            "req_wp1_running_progress",
            "running",
            Some("remote_running"),
        )
        .unwrap());
        let running_before = get_run(&state, &running.run_id).unwrap().unwrap();

        assert!(mark_status(
            &state,
            "agent",
            &running.run_id,
            "req_wp1_running_progress",
            "started",
            Some("regressed"),
        )
        .unwrap());

        let running_after = get_run(&state, &running.run_id).unwrap().unwrap();
        assert_eq!(running_after.status, "running");
        assert_eq!(running_after.reason.as_deref(), Some("remote_running"));
        assert_eq!(running_after.updated_at, running_before.updated_at);

        let acked = prepare_run(
            &state,
            "agent",
            "req_wp1_acked_progress",
            &HubCommand::McpListServers {
                request_id: "req_wp1_acked_progress".to_string(),
            },
        )
        .unwrap();
        assert!(mark_acked(
            &state,
            "agent",
            &acked.run_id,
            "req_wp1_acked_progress",
            &acked.command_hash,
        )
        .unwrap());
        let acked_before = get_run(&state, &acked.run_id).unwrap().unwrap();
        mark_dispatched(&state, &acked.run_id).unwrap();
        mark_timeout(&state, &acked.run_id, "process_exec_timeout").unwrap();
        let acked_after = get_run(&state, &acked.run_id).unwrap().unwrap();
        assert_eq!(acked_after.status, "acked");
        assert_eq!(acked_after.reason, acked_before.reason);
        assert_eq!(acked_after.updated_at, acked_before.updated_at);
    }

    #[test]
    fn wp1_failed_and_unknown_runs_accept_only_late_results() {
        for (status, reason) in [
            ("failed", "command_hash_mismatch"),
            ("unknown", "agent_restarted_before_completion"),
        ] {
            let state = test_state();
            let request_id = format!("req_wp1_{status}_late_result");
            let command = HubCommand::McpListServers {
                request_id: request_id.clone(),
            };
            let run = prepare_run(&state, "agent", &request_id, &command).unwrap();
            assert!(mark_status(
                &state,
                "agent",
                &run.run_id,
                &request_id,
                status,
                Some(reason),
            )
            .unwrap());

            let result = serde_json::json!({ "status": status });
            assert!(matches!(
                store_result(&state, "agent", &run.run_id, &request_id, &result).unwrap(),
                StoreResultOutcome::Stored { .. }
            ));
            let stored = get_run(&state, &run.run_id).unwrap().unwrap();
            assert_eq!(stored.status, "completed");
            assert_eq!(stored.result, Some(result));
        }
    }

    #[test]
    fn wp1_agent_report_identity_and_terminal_result_are_preserved() {
        let state = test_state();
        let started = Utc::now();
        let initial = AgentRunReport {
            run_id: "run_wp1_report_identity".to_string(),
            request_id: "req_wp1_report_identity".to_string(),
            tool_name: "process.exec".to_string(),
            source: "tunnel".to_string(),
            profile: "normal".to_string(),
            detail: "metadata".to_string(),
            status: "started".to_string(),
            started_at: started,
            updated_at: started,
            duration_ms: None,
            process_id: None,
            exit_code: None,
            reason: None,
            arguments: None,
            result: None,
            process: None,
        };
        upsert_agent_report(&state, "agent", initial.clone()).unwrap();

        let mut foreign_request = initial.clone();
        foreign_request.request_id = "req_wp1_foreign".to_string();
        let error = upsert_agent_report(&state, "agent", foreign_request).unwrap_err();
        assert_eq!(error.to_string(), "agent_run_request_mismatch");

        let mut completed = initial.clone();
        completed.detail = "full".to_string();
        completed.status = "completed".to_string();
        completed.updated_at = started + Duration::seconds(1);
        completed.result = Some(BoundedJsonValue {
            value: serde_json::json!({ "ok": true }),
            byte_count: 11,
            sha256: "c".repeat(64),
            truncated: false,
        });
        upsert_agent_report(&state, "agent", completed.clone()).unwrap();
        let canonical = get_run(&state, &completed.run_id).unwrap().unwrap();

        let mut stale = completed;
        stale.status = "failed".to_string();
        stale.reason = Some("late_failure".to_string());
        stale.result = None;
        upsert_agent_report(&state, "agent", stale).unwrap();

        let stored = get_run(&state, "run_wp1_report_identity").unwrap().unwrap();
        assert_eq!(stored.request_id, "req_wp1_report_identity");
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.result, canonical.result);
        assert_eq!(stored.reason, canonical.reason);
        assert_eq!(stored.updated_at, canonical.updated_at);
    }

    #[test]
    fn stale_acked_runs_become_unknown() {
        let state = test_state();
        let command = HubCommand::McpListServers {
            request_id: "req_1".to_string(),
        };
        let run = prepare_run(&state, "agent", "req_1", &command).unwrap();
        assert!(mark_acked(&state, "agent", &run.run_id, "req_1", &run.command_hash).unwrap());
        assert_eq!(mark_stale_acked_unknown(&state, Utc::now()).unwrap(), 1);
        let stored = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(stored.status, "unknown");
        assert_eq!(stored.reason.as_deref(), Some("acked_result_timeout"));
    }

    #[test]
    fn agent_reports_upsert_idempotently_and_keeps_full_bounded_detail() {
        let state = test_state();
        let started = Utc::now();
        upsert_agent_report(
            &state,
            "agent",
            AgentRunReport {
                run_id: "run_agent_1".to_string(),
                request_id: "req_agent_1".to_string(),
                tool_name: "process.exec".to_string(),
                source: "tunnel".to_string(),
                profile: "normal".to_string(),
                detail: "full".to_string(),
                status: "started".to_string(),
                started_at: started,
                updated_at: started,
                duration_ms: None,
                process_id: None,
                exit_code: None,
                reason: None,
                arguments: Some(BoundedJsonValue {
                    value: serde_json::json!({ "agentId": "agent" }),
                    byte_count: 19,
                    sha256: "a".repeat(64),
                    truncated: false,
                }),
                result: None,
                process: None,
            },
        )
        .unwrap();
        upsert_agent_report(
            &state,
            "agent",
            AgentRunReport {
                run_id: "run_agent_1".to_string(),
                request_id: "req_agent_1".to_string(),
                tool_name: "process.exec".to_string(),
                source: "tunnel".to_string(),
                profile: "normal".to_string(),
                detail: "full".to_string(),
                status: "completed".to_string(),
                started_at: started,
                updated_at: started + Duration::seconds(1),
                duration_ms: Some(1000),
                process_id: Some("process-agent-1".to_string()),
                exit_code: Some(0),
                reason: None,
                arguments: None,
                result: Some(BoundedJsonValue {
                    value: serde_json::json!({ "status": "completed" }),
                    byte_count: 23,
                    sha256: "b".repeat(64),
                    truncated: false,
                }),
                process: Some(
                    serde_json::from_value(serde_json::json!({
                        "agentId": "agent",
                        "processId": "process-agent-1",
                        "kind": "command",
                        "state": "completed",
                        "createdAt": started.to_rfc3339(),
                        "updatedAt": (started + Duration::seconds(1)).to_rfc3339(),
                        "captureStatus": "complete"
                    }))
                    .unwrap(),
                ),
            },
        )
        .unwrap();
        upsert_agent_report(
            &state,
            "agent",
            AgentRunReport {
                run_id: "run_agent_1".to_string(),
                request_id: "req_agent_1".to_string(),
                tool_name: "process.exec".to_string(),
                source: "tunnel".to_string(),
                profile: "normal".to_string(),
                detail: "full".to_string(),
                status: "started".to_string(),
                started_at: started,
                updated_at: started,
                duration_ms: None,
                process_id: None,
                exit_code: None,
                reason: None,
                arguments: None,
                result: None,
                process: None,
            },
        )
        .unwrap();
        let stored = get_run(&state, "run_agent_1").unwrap().unwrap();
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.source.as_deref(), Some("tunnel"));
        assert_eq!(stored.detail.as_deref(), Some("full"));
        assert_eq!(
            stored.arguments,
            Some(serde_json::json!({ "agentId": "agent" }))
        );
        assert_eq!(
            stored.result,
            Some(serde_json::json!({ "status": "completed" }))
        );
        assert_eq!(stored.process_id.as_deref(), Some("process-agent-1"));
        assert_eq!(
            stored.process.as_ref().unwrap()["processId"],
            "process-agent-1"
        );
    }
}
