use anyhow::{bail, Context, Result};
use rusqlite::{Connection, TransactionBehavior};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CURRENT_SCHEMA_VERSION: i64 = 3;
const SQLITE_BUSY_TIMEOUT_MS: u64 = 5_000;

pub(crate) fn open_db(path: &PathBuf) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    ensure_not_symlink(path)?;
    if !path.exists() {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true).read(true);
        set_private_mode(&mut options);
        match options.open(path) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure_not_symlink(path)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let conn =
        Connection::open(path).with_context(|| format!("open sqlite db {}", path.display()))?;
    configure_connection(&conn)?;
    Ok(conn)
}

fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

pub(crate) fn init_db(conn: &Connection) -> Result<()> {
    let version = schema_version(conn)?;
    if version > CURRENT_SCHEMA_VERSION {
        bail!(
            "unsupported hub database schema version {version}; maximum supported {CURRENT_SCHEMA_VERSION}"
        );
    }

    let staged_backup = if version < CURRENT_SCHEMA_VERSION {
        stage_backup(conn, "pre-migration")?
    } else {
        None
    };
    let transaction =
        match rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate) {
            Ok(transaction) => transaction,
            Err(error) => {
                discard_staged_backup(staged_backup);
                return Err(error.into());
            }
        };
    let locked_version = match schema_version(&transaction) {
        Ok(version) => version,
        Err(error) => {
            drop(transaction);
            discard_staged_backup(staged_backup);
            return Err(error);
        }
    };
    if locked_version > CURRENT_SCHEMA_VERSION {
        drop(transaction);
        discard_staged_backup(staged_backup);
        bail!(
            "unsupported hub database schema version {locked_version}; maximum supported {CURRENT_SCHEMA_VERSION}"
        );
    }
    if locked_version >= CURRENT_SCHEMA_VERSION {
        drop(transaction);
        discard_staged_backup(staged_backup);
        return Ok(());
    }
    if let Some(staged_backup) = staged_backup {
        staged_backup.publish()?;
    }
    transaction.execute_batch(
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
    rename_legacy_column(&transaction, "agent_runs", "job_id", "process_id")?;
    rename_legacy_column(&transaction, "agent_runs", "job_json", "process_json")?;
    ensure_column(&transaction, "agents", "alias", "alias text")?;
    ensure_column(&transaction, "agent_runs", "source", "source text")?;
    ensure_column(&transaction, "agent_runs", "profile", "profile text")?;
    ensure_column(&transaction, "agent_runs", "detail", "detail text")?;
    ensure_column(&transaction, "agent_runs", "process_id", "process_id text")?;
    ensure_column(
        &transaction,
        "agent_runs",
        "duration_ms",
        "duration_ms integer",
    )?;
    ensure_column(&transaction, "agent_runs", "exit_code", "exit_code integer")?;
    ensure_column(
        &transaction,
        "agent_runs",
        "arguments_json",
        "arguments_json text",
    )?;
    ensure_column(
        &transaction,
        "agent_runs",
        "process_json",
        "process_json text",
    )?;
    transaction.execute_batch(
        "create unique index if not exists agents_alias_unique on agents(alias) where alias is not null;",
    )?;
    crate::event_feedback::init_transaction(&transaction)?;
    transaction.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

pub(crate) fn backup_before_retention(conn: &Connection) -> Result<()> {
    backup_database(conn, "pre-retention")
}

struct StagedBackup {
    staging_dir: PathBuf,
    temporary: PathBuf,
    backup: PathBuf,
}

impl StagedBackup {
    fn publish(self) -> Result<()> {
        if let Err(error) = fs::rename(&self.temporary, &self.backup) {
            let _ = fs::remove_dir_all(&self.staging_dir);
            return Err(error.into());
        }
        if let Err(error) = fs::remove_dir_all(&self.staging_dir) {
            return Err(error.into());
        }
        sync_parent(&self.backup)?;
        Ok(())
    }

    fn discard(self) {
        let _ = fs::remove_dir_all(&self.staging_dir);
    }
}

fn discard_staged_backup(staged: Option<StagedBackup>) {
    if let Some(staged) = staged {
        staged.discard();
    }
}

fn backup_database(conn: &Connection, label: &str) -> Result<()> {
    let Some(staged) = stage_backup(conn, label)? else {
        return Ok(());
    };
    staged.publish()
}

fn stage_backup(conn: &Connection, label: &str) -> Result<Option<StagedBackup>> {
    let Some(path) = connection_path(conn)? else {
        return Ok(None);
    };
    if !path.exists() || fs::metadata(&path)?.len() == 0 {
        return Ok(None);
    }
    // VACUUM INTO uses SQLite's consistent snapshot machinery, so concurrent
    // writers cannot leave a partially copied page in the recovery image.
    let backup = {
        let mut value = path.as_os_str().to_os_string();
        value.push(format!(".{label}.bak"));
        PathBuf::from(value)
    };
    // The snapshot is created inside a private directory because VACUUM INTO
    // chooses the file mode itself. This prevents a permissive umask from
    // exposing result/command payloads before chmod runs.
    let staging_dir = unique_sibling(&backup, "staging");
    create_private_dir(&staging_dir)?;
    let temporary = staging_dir.join("snapshot.sqlite");
    let escaped = temporary.to_string_lossy().replace('\'', "''");
    if let Err(error) = conn.execute_batch(&format!("VACUUM INTO '{escaped}'")) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error.into());
    }
    if let Err(error) = set_private_path_mode(&temporary) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    if let Err(error) = File::open(&temporary).and_then(|file| file.sync_all()) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error.into());
    }
    Ok(Some(StagedBackup {
        staging_dir,
        temporary,
        backup,
    }))
}

fn connection_path(conn: &Connection) -> Result<Option<PathBuf>> {
    let Some(path) = conn.path() else {
        return Ok(None);
    };
    if path.is_empty() || path == ":memory:" {
        Ok(None)
    } else {
        Ok(Some(PathBuf::from(path)))
    }
}

fn unique_sibling(path: &Path, suffix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".{suffix}.{}.{}", std::process::id(), nonce));
    PathBuf::from(value)
}

fn create_private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn ensure_not_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing symlinked hub database path {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn set_private_path_mode(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_path_mode(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_mode(_options: &mut OpenOptions) {}

fn ensure_column(
    conn: &rusqlite::Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
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

fn rename_legacy_column(
    conn: &rusqlite::Transaction<'_>,
    table: &str,
    old_column: &str,
    new_column: &str,
) -> Result<()> {
    let columns = {
        let mut stmt = conn.prepare(&format!("pragma table_info({table})"))?;
        let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
        columns.collect::<std::result::Result<Vec<_>, _>>()?
    };
    if columns.iter().any(|column| column == old_column)
        && !columns.iter().any(|column| column == new_column)
    {
        conn.execute(
            &format!("alter table {table} rename column {old_column} to {new_column}"),
            [],
        )?;
    }
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
            processes: true,
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

    #[test]
    fn migration_sets_explicit_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }
    #[test]
    fn migration_renames_legacy_run_process_fields_without_losing_data() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table agent_runs (
                 run_id text primary key,
                 job_id text,
                 job_json text
             );
             insert into agent_runs values ('run', 'process-1', '{\"state\":\"completed\"}');
             pragma user_version = 1;",
        )
        .unwrap();

        init_db(&conn).unwrap();

        let (process_id, process_json): (String, String) = conn
            .query_row(
                "select process_id, process_json from agent_runs where run_id = 'run'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(process_id, "process-1");
        assert_eq!(process_json, "{\"state\":\"completed\"}");
        let columns = {
            let mut stmt = conn.prepare("pragma table_info(agent_runs)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<std::result::Result<Vec<_>, _>>().unwrap()
        };
        assert!(!columns.iter().any(|column| column == "job_id"));
        assert!(!columns.iter().any(|column| column == "job_json"));
    }

    #[test]
    fn newer_schema_version_is_rejected_without_changes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION + 1)
            .unwrap();
        assert!(init_db(&conn).is_err());
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION + 1);
    }

    #[test]
    fn failed_legacy_migration_rolls_back_schema_changes() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table agents (
                 agent_id text primary key,
                 display_name text not null,
                 enabled integer not null,
                 secret_hash text not null,
                 last_seen_at text,
                 capabilities_json text not null,
                 alias text
             );
             insert into agents values ('a', 'A', 1, 'hash-a', null, '{}', 'duplicate');
             insert into agents values ('b', 'B', 1, 'hash-b', null, '{}', 'duplicate');",
        )
        .unwrap();
        assert!(init_db(&conn).is_err());
        let table_count: i64 = conn
            .query_row(
                "select count(*) from sqlite_master
                 where type = 'table' and name in ('notification_endpoints', 'agent_runs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 0);
    }
}
