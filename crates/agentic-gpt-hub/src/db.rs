use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::PathBuf;

pub(crate) fn open_db(path: &PathBuf) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Connection::open(path).with_context(|| format!("open sqlite db {}", path.display()))
}

pub(crate) fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        create table if not exists agents (
            agent_id text primary key,
            display_name text not null,
            enabled integer not null,
            secret_hash text not null,
            last_seen_at text,
            capabilities_json text not null
        );
        create table if not exists notification_endpoints (
            endpoint_id text primary key,
            kind text not null,
            display_name text,
            capabilities_json text not null,
            token_hash text not null,
            enabled integer not null,
            last_seen_at text,
            created_at text not null
        );
        create table if not exists agent_runs (
            run_id text primary key,
            request_id text not null,
            agent_id text not null,
            command_type text not null,
            command_json text not null,
            command_hash text not null,
            status text not null,
            acked_at text,
            result_json text,
            result_hash text,
            conflict_json text,
            reason text,
            created_at text not null,
            updated_at text not null,
            expires_at text
        );
        ",
    )?;
    ensure_column(conn, "agents", "alias", "alias text")?;
    ensure_column(conn, "agent_runs", "source", "source text")?;
    ensure_column(conn, "agent_runs", "profile", "profile text")?;
    ensure_column(conn, "agent_runs", "detail", "detail text")?;
    ensure_column(conn, "agent_runs", "session_id", "session_id text")?;
    ensure_column(conn, "agent_runs", "duration_ms", "duration_ms integer")?;
    ensure_column(conn, "agent_runs", "exit_code", "exit_code integer")?;
    ensure_column(conn, "agent_runs", "arguments_json", "arguments_json text")?;
    ensure_column(conn, "agent_runs", "session_json", "session_json text")?;
    conn.execute_batch(
        "create unique index if not exists agents_alias_unique on agents(alias) where alias is not null;",
    )?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("pragma table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    conn.execute(&format!("alter table {table} add column {definition}"), [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_gpt_protocol::Capabilities;
    use rusqlite::params;

    #[test]
    fn agent_alias_is_nullable_and_unique_when_present() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let capabilities = serde_json::to_string(&Capabilities {
            sessions: true,
            confirmation: true,
            notification_actions: false,
        })
        .unwrap();
        conn.execute(
            "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
             values ('a', null, 'A', 1, 'hash-a', null, ?1)",
            params![capabilities],
        )
        .unwrap();
        conn.execute(
            "insert into agents(agent_id, alias, display_name, enabled, secret_hash, last_seen_at, capabilities_json)
             values ('b', null, 'B', 1, 'hash-b', null, ?1)",
            params![capabilities],
        )
        .unwrap();
        conn.execute(
            "update agents set alias = 'laptop' where agent_id = 'a'",
            [],
        )
        .unwrap();
        let duplicate = conn.execute(
            "update agents set alias = 'laptop' where agent_id = 'b'",
            [],
        );
        assert!(duplicate.is_err());
    }
}
