use agentic_gpt_protocol::{AgentRegistryEntry, Capabilities};
use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Subcommand;
use rusqlite::{params, Connection};

use crate::state::HubState;
use crate::utils::sha256_hex;

#[derive(Subcommand)]
pub(crate) enum AgentCommand {
    Add {
        #[arg(long)]
        agent_id: String,
        #[arg(long)]
        alias: Option<String>,
        #[arg(long)]
        display_name: String,
        #[arg(long)]
        secret: String,
    },
    Alias {
        #[command(subcommand)]
        command: AgentAliasCommand,
    },
    Remove {
        #[arg(long)]
        agent_id: String,
    },
    Disable {
        #[arg(long)]
        agent_id: String,
    },
    Enable {
        #[arg(long)]
        agent_id: String,
    },
    List,
}

#[derive(Subcommand)]
pub(crate) enum AgentAliasCommand {
    Set {
        #[arg(long)]
        agent_id: String,
        #[arg(long)]
        alias: String,
    },
    Clear {
        #[arg(long)]
        agent_id: String,
    },
}

pub(crate) fn handle_agent_command(conn: &Connection, command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::Add {
            agent_id,
            alias,
            display_name,
            secret,
        } => {
            let alias = normalize_alias(alias.as_deref())?;
            let capabilities = Capabilities {
                sessions: true,
                confirmation: true,
                notification_actions: true,
            };
            conn.execute(
                "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
                 values (?1, ?2, ?3, 1, ?4, null, ?5)
                 on conflict(agent_id) do update set display_name = excluded.display_name,
                     alias = coalesce(excluded.alias, agents.alias),
                     enabled = 1, secret_hash = excluded.secret_hash, capabilities_json = excluded.capabilities_json",
                params![
                    agent_id,
                    alias,
                    display_name,
                    sha256_hex(&secret),
                    serde_json::to_string(&capabilities)?
                ],
            )?;
            println!("agent saved");
        }
        AgentCommand::Alias { command } => match command {
            AgentAliasCommand::Set { agent_id, alias } => {
                let alias = normalize_alias(Some(&alias))?;
                conn.execute(
                    "update agents set alias = ?2 where agent_id = ?1",
                    params![agent_id, alias],
                )?;
                println!("agent alias saved");
            }
            AgentAliasCommand::Clear { agent_id } => {
                conn.execute(
                    "update agents set alias = null where agent_id = ?1",
                    params![agent_id],
                )?;
                println!("agent alias cleared");
            }
        },
        AgentCommand::Remove { agent_id } => {
            conn.execute("delete from agents where agent_id = ?1", params![agent_id])?;
            println!("agent removed");
        }
        AgentCommand::Disable { agent_id } => {
            conn.execute(
                "update agents set enabled = 0 where agent_id = ?1",
                params![agent_id],
            )?;
            println!("agent disabled");
        }
        AgentCommand::Enable { agent_id } => {
            conn.execute(
                "update agents set enabled = 1 where agent_id = ?1",
                params![agent_id],
            )?;
            println!("agent enabled");
        }
        AgentCommand::List => {
            for entry in registry_entries_from_conn(conn)? {
                println!(
                    "{}\talias={}\t{}\tenabled={}\tlastSeenAt={}",
                    entry.agent_id,
                    entry.alias.as_deref().unwrap_or("-"),
                    entry.display_name,
                    entry.enabled,
                    entry
                        .last_seen_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string())
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn registry_entries(state: &HubState) -> Result<Vec<AgentRegistryEntry>> {
    let conn = state.db.lock().unwrap();
    registry_entries_from_conn(&conn)
}

fn registry_entries_from_conn(conn: &Connection) -> Result<Vec<AgentRegistryEntry>> {
    let mut stmt = conn.prepare(
        "select agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json from agents order by agent_id",
    )?;
    let rows = stmt.query_map([], row_to_entry)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(crate) fn registry_entry(
    state: &HubState,
    agent_id: &str,
) -> Result<Option<AgentRegistryEntry>> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "select agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json from agents where agent_id = ?1",
    )?;
    let mut rows = stmt.query_map(params![agent_id], row_to_entry)?;
    rows.next().transpose().map_err(Into::into)
}

pub(crate) fn update_last_seen(state: &HubState, agent_id: &str) -> Result<()> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "update agents set last_seen_at = ?2 where agent_id = ?1",
        params![agent_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRegistryEntry> {
    let last_seen: Option<String> = row.get(5)?;
    let capabilities_json: String = row.get(6)?;
    Ok(AgentRegistryEntry {
        agent_id: row.get(0)?,
        alias: row.get(1)?,
        display_name: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        secret_hash: row.get(4)?,
        last_seen_at: last_seen.and_then(|value| {
            DateTime::parse_from_rfc3339(&value)
                .ok()
                .map(|value| value.with_timezone(&Utc))
        }),
        capabilities: serde_json::from_str(&capabilities_json).unwrap_or(Capabilities {
            sessions: true,
            confirmation: true,
            notification_actions: false,
        }),
    })
}

fn normalize_alias(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let alias = value.trim();
    if alias.is_empty() {
        return Ok(None);
    }
    let valid = alias
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'));
    if !valid {
        return Err(anyhow::anyhow!(
            "alias may only contain ASCII letters, digits, underscore, or hyphen"
        ));
    }
    Ok(Some(alias.to_string()))
}
