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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<Value>,
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
        HubCommand::BatchExec { .. } => "process.batchExec",
        HubCommand::StartSession { .. } => "session.start",
        HubCommand::ListSessions { .. } => "session.list",
        HubCommand::InspectSession { .. } => "session.inspect",
        HubCommand::WaitSession { .. } => "session.wait",
        HubCommand::KillSession { .. } => "session.kill",
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
        HubCommand::UserNotifyDeliver { .. } => "user.notify.deliver",
        HubCommand::RoomNotebookAppend { .. } => "room.notebook.append",
        HubCommand::RoomNotebookRecent { .. } => "room.notebook.recent",
        HubCommand::RoomNotebookSelectExact { .. } => "room.notebook.selectExact",
        HubCommand::RoomNotebookSearch { .. } => "room.notebook.search",
        HubCommand::RoomNotebookCurrent { .. } => "room.notebook.current",
        HubCommand::RoomNotebookUpdate { .. } => "room.notebook.update",
        HubCommand::RoomNotebookRemove { .. } => "room.notebook.remove",
        HubCommand::RoomDiaryAppend { .. } => "room.diary.append",
        HubCommand::RoomDiaryRecent { .. } => "room.diary.recent",
        HubCommand::RoomDiarySelectExact { .. } => "room.diary.selectExact",
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
    update_status(state, run_id, "dispatched", None)
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
         set status = case when result_json is null then 'acked' else status end,
             acked_at = coalesce(acked_at, ?1),
             updated_at = ?1
         where run_id = ?2 and request_id = ?3 and agent_id = ?4 and command_hash = ?5",
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
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    let changed = conn.execute(
        "update agent_runs
         set status = ?1, reason = ?2, updated_at = ?3
         where run_id = ?4 and request_id = ?5 and agent_id = ?6",
        params![status, reason, now, run_id, request_id, agent_id],
    )?;
    Ok(changed > 0)
}

pub(crate) fn mark_timeout(state: &HubState, run_id: &str, reason: &str) -> Result<()> {
    update_status(state, run_id, "timeout_waiting_result", Some(reason))
}

fn update_status(state: &HubState, run_id: &str, status: &str, reason: Option<&str>) -> Result<()> {
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    conn.execute(
        "update agent_runs set status = ?1, reason = ?2, updated_at = ?3 where run_id = ?4",
        params![status, reason, now, run_id],
    )?;
    Ok(())
}

pub(crate) fn store_result(
    state: &HubState,
    agent_id: &str,
    run_id: Option<&str>,
    request_id: &str,
    result: &Value,
) -> Result<bool> {
    let result_json = serde_json::to_string(result)?;
    let result_hash = sha256_hex(&result_json);
    let now = Utc::now();
    let conn = state.db.lock().unwrap();
    let existing = if let Some(run_id) = run_id {
        conn.query_row(
            "select result_hash from agent_runs where run_id = ?1 and request_id = ?2 and agent_id = ?3",
            params![run_id, request_id, agent_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
    } else {
        conn.query_row(
            "select result_hash from agent_runs where request_id = ?1 and agent_id = ?2 order by created_at desc limit 1",
            params![request_id, agent_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
    };
    if let Some(Some(existing_hash)) = existing {
        if existing_hash == result_hash {
            return Ok(true);
        }
        let changed = if let Some(run_id) = run_id {
            conn.execute(
                "update agent_runs set conflict_json = ?1, updated_at = ?2 where run_id = ?3 and request_id = ?4 and agent_id = ?5",
                params![result_json, now, run_id, request_id, agent_id],
            )?
        } else {
            conn.execute(
                "update agent_runs set conflict_json = ?1, updated_at = ?2 where request_id = ?3 and agent_id = ?4",
                params![result_json, now, request_id, agent_id],
            )?
        };
        return Ok(changed > 0);
    }

    let changed = if let Some(run_id) = run_id {
        conn.execute(
            "update agent_runs
             set status = 'completed', result_json = ?1, result_hash = ?2, updated_at = ?3
             where run_id = ?4 and request_id = ?5 and agent_id = ?6",
            params![result_json, result_hash, now, run_id, request_id, agent_id],
        )?
    } else {
        conn.execute(
            "update agent_runs
             set status = 'completed', result_json = ?1, result_hash = ?2, updated_at = ?3
             where request_id = ?4 and agent_id = ?5",
            params![result_json, result_hash, now, request_id, agent_id],
        )?
    };
    Ok(changed > 0)
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
    let session = if detail == "full" {
        report
            .session
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
    let existing_agent: Option<String> = conn
        .query_row(
            "select agent_id from agent_runs where run_id = ?1",
            params![report.run_id],
            |row| row.get(0),
        )
        .optional()?;
    if existing_agent
        .as_deref()
        .is_some_and(|value| value != agent_id)
    {
        return Err(anyhow!("agent_run_owner_mismatch"));
    }
    let existing_status: Option<String> = conn
        .query_row(
            "select status from agent_runs where run_id = ?1",
            params![report.run_id],
            |row| row.get(0),
        )
        .optional()?;
    let existing_result_hash: Option<String> = conn
        .query_row(
            "select result_hash from agent_runs where run_id = ?1",
            params![report.run_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    if existing_result_hash
        .as_ref()
        .zip(result_hash.as_ref())
        .is_some_and(|(existing, incoming)| existing != incoming)
    {
        conn.execute(
            "update agent_runs set conflict_json = ?1, updated_at = ?2 where run_id = ?3",
            params![result, now, report.run_id],
        )?;
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
            source, profile, detail, session_id, duration_ms, exit_code, arguments_json, session_json
        ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
        on conflict(run_id) do update set
            status = excluded.status,
            result_json = coalesce(excluded.result_json, agent_runs.result_json),
            result_hash = coalesce(excluded.result_hash, agent_runs.result_hash),
            reason = excluded.reason,
            updated_at = excluded.updated_at,
            session_id = excluded.session_id,
            duration_ms = excluded.duration_ms,
            exit_code = excluded.exit_code,
            arguments_json = coalesce(excluded.arguments_json, agent_runs.arguments_json),
            session_json = coalesce(excluded.session_json, agent_runs.session_json)",
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
            report.session_id,
            report.duration_ms.map(|value| value as i64),
            report.exit_code,
            arguments,
            session,
        ],
    )?;
    Ok(())
}

pub(crate) fn get_run(state: &HubState, run_id: &str) -> Result<Option<AgentRun>> {
    let conn = state.db.lock().unwrap();
    conn.query_row(
        "select run_id, request_id, agent_id, command_type, command_hash, source, profile, detail,
                status, result_json, arguments_json, session_json, reason, created_at, updated_at
         from agent_runs where run_id = ?1",
        params![run_id],
        |row| {
            let result_json: Option<String> = row.get(9)?;
            let arguments_json: Option<String> = row.get(10)?;
            let session_json: Option<String> = row.get(11)?;
            Ok(AgentRun {
                run_id: row.get(0)?,
                request_id: row.get(1)?,
                agent_id: row.get(2)?,
                command_type: row.get(3)?,
                command_hash: row.get(4)?,
                source: row.get(5)?,
                profile: row.get(6)?,
                detail: row.get(7)?,
                status: row.get(8)?,
                result: result_json.and_then(|json| serde_json::from_str(&json).ok()),
                arguments: arguments_json.and_then(|json| serde_json::from_str(&json).ok()),
                session: session_json.and_then(|json| serde_json::from_str(&json).ok()),
                reason: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
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
             where (expires_at is null or expires_at > ?1)
             order by created_at desc limit 1000",
        )?;
        let rows = stmt.query_map(params![Utc::now()], |row| row.get::<_, String>(0))?;
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
    conn.execute(
        "delete from agent_runs where expires_at is not null and expires_at <= ?1",
        params![Utc::now()],
    )
    .map_err(|error| anyhow!(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::{HubConfig, McpProfile, NtfyConfig, RemoteConfirmationConfig};
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
            agents: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
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
        assert!(store_result(&state, "agent", Some(&run.run_id), "req_1", &result).unwrap());
        assert!(store_result(&state, "agent", Some(&run.run_id), "req_1", &result).unwrap());
        let stored = get_run(&state, &run.run_id).unwrap().unwrap();
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.result.unwrap(), result);
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
                session_id: None,
                exit_code: None,
                reason: None,
                arguments: Some(BoundedJsonValue {
                    value: serde_json::json!({ "agentId": "agent" }),
                    byte_count: 19,
                    sha256: "a".repeat(64),
                    truncated: false,
                }),
                result: None,
                session: None,
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
                session_id: None,
                exit_code: Some(0),
                reason: None,
                arguments: None,
                result: Some(BoundedJsonValue {
                    value: serde_json::json!({ "status": "completed" }),
                    byte_count: 23,
                    sha256: "b".repeat(64),
                    truncated: false,
                }),
                session: None,
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
                session_id: None,
                exit_code: None,
                reason: None,
                arguments: None,
                result: None,
                session: None,
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
    }
}
