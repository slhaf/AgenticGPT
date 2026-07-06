use agentic_gpt_protocol::HubCommand;
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
    pub(crate) status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
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
        HubCommand::Exec { .. } => "exec",
        HubCommand::BatchExec { .. } => "batchExec",
        HubCommand::StartSession { .. } => "startSession",
        HubCommand::ListSessions { .. } => "listSessions",
        HubCommand::InspectSession { .. } => "inspectSession",
        HubCommand::WaitSession { .. } => "waitSession",
        HubCommand::KillSession { .. } => "killSession",
        HubCommand::TmuxListSessions { .. } => "tmux.listSessions",
        HubCommand::TmuxListPanes { .. } => "tmux.listPanes",
        HubCommand::TmuxCapturePane { .. } => "tmux.capturePane",
        HubCommand::TmuxPasteText { .. } => "tmux.pasteText",
        HubCommand::TmuxExec { .. } => "tmux.exec",
        HubCommand::TmuxCreateSession { .. } => "tmux.createSession",
        HubCommand::TmuxCloseSession { .. } => "tmux.closeSession",
        HubCommand::McpListServers { .. } => "mcpListServers",
        HubCommand::McpListTools { .. } => "mcpListTools",
        HubCommand::McpCallTool { .. } => "mcpCallTool",
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
        HubCommand::SkillsList { .. } => "skills.list",
        HubCommand::SkillsRead { .. } => "skills.read",
        HubCommand::SkillsSearch { .. } => "skills.search",
        HubCommand::SkillsActive { .. } => "skills.active",
        HubCommand::SkillsActivate { .. } => "skills.activate",
        HubCommand::SkillsDeactivate { .. } => "skills.deactivate",
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

pub(crate) fn get_run(state: &HubState, run_id: &str) -> Result<Option<AgentRun>> {
    let conn = state.db.lock().unwrap();
    conn.query_row(
        "select run_id, request_id, agent_id, command_type, command_hash, status,
                result_json, reason, created_at, updated_at
         from agent_runs where run_id = ?1",
        params![run_id],
        |row| {
            let result_json: Option<String> = row.get(6)?;
            Ok(AgentRun {
                run_id: row.get(0)?,
                request_id: row.get(1)?,
                agent_id: row.get(2)?,
                command_type: row.get(3)?,
                command_hash: row.get(4)?,
                status: row.get(5)?,
                result: result_json.and_then(|json| serde_json::from_str(&json).ok()),
                reason: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
            })
        },
    )
    .optional()
    .map_err(|error| anyhow!(error))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::{HubConfig, NtfyConfig, RemoteConfirmationConfig};
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
}
