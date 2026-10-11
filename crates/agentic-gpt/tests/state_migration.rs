#![cfg(unix)]

use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{symlink, FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

// Frozen historical fixtures: event v3 and process v1, before local identity removal.
const OLD_EVENTS: &str = r#"
PRAGMA user_version=3;
CREATE TABLE events (
 event_id TEXT PRIMARY KEY NOT NULL, message TEXT NOT NULL, severity TEXT NOT NULL,
 created_at TEXT NOT NULL, status TEXT NOT NULL, source_kind TEXT NOT NULL,
 source_ref TEXT NOT NULL, shown_count INTEGER NOT NULL DEFAULT 0,
 expires_at TEXT, terminal_at TEXT
);
CREATE INDEX idx_events_list ON events(status,severity,created_at DESC,event_id DESC);
CREATE INDEX idx_events_pending ON events(status,severity,created_at,event_id);
CREATE TABLE internal_sources (
 source_kind TEXT NOT NULL, source_ref TEXT NOT NULL, response_state TEXT NOT NULL,
 settled_at TEXT, policy_json TEXT NOT NULL, completion_seen INTEGER NOT NULL DEFAULT 0,
 completion_event_type TEXT, completion_message TEXT, completed_at TEXT,
 notified INTEGER NOT NULL DEFAULT 0, origin_run_id TEXT, origin_request_id TEXT,
 origin_command_hash TEXT, PRIMARY KEY(source_kind,source_ref)
);
CREATE INDEX idx_internal_sources_recovery
 ON internal_sources(response_state,completion_seen,source_kind,source_ref);
CREATE TABLE event_store_metadata (
 id INTEGER PRIMARY KEY CHECK(id=1), owner_fingerprint TEXT NOT NULL
);
INSERT INTO event_store_metadata VALUES(1,'legacy-owner');
"#;

const OLD_PROCESSES: &str = r#"
PRAGMA user_version=1;
CREATE TABLE processes (
 process_id TEXT PRIMARY KEY NOT NULL, agent_id TEXT NOT NULL, group_name TEXT,
 kind TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL, started_at TEXT,
 updated_at TEXT NOT NULL, finished_at TEXT, info_json TEXT NOT NULL, detail_json TEXT,
 stdout_bytes BLOB NOT NULL DEFAULT X'', stdout_start_offset TEXT NOT NULL DEFAULT '0',
 stdout_end_offset TEXT NOT NULL DEFAULT '0', stderr_bytes BLOB NOT NULL DEFAULT X'',
 stderr_start_offset TEXT NOT NULL DEFAULT '0', stderr_end_offset TEXT NOT NULL DEFAULT '0',
 capture_status TEXT NOT NULL, capture_error TEXT
);
CREATE INDEX idx_processes_created_id ON processes(created_at DESC,process_id DESC);
CREATE INDEX idx_processes_group_created ON processes(group_name,created_at DESC,process_id DESC);
CREATE INDEX idx_processes_kind_created ON processes(kind,created_at DESC,process_id DESC);
CREATE INDEX idx_processes_state_created ON processes(state,created_at DESC,process_id DESC);
CREATE INDEX idx_processes_finished ON processes(finished_at);
CREATE TABLE process_event_completions (
 process_id TEXT PRIMARY KEY NOT NULL, event_type TEXT, message TEXT, completed_at TEXT
);
"#;

#[test]
fn migrated_history_survives_identity_change_through_real_local_agent() {
    let fixture = Fixture::new();
    let legacy = fixture.seed("before", "saved-process");
    let other = fixture.seed("other", "other-process");
    let original = files(&legacy);
    let other_original = files(&other);
    let ledger = fixture.agentic().join("transport-runs.jsonl");
    fs::write(&ledger, "{\"agentId\":\"remote\",\"runId\":\"receipt\"}\n").unwrap();

    let preview = fixture.migration(&["--dry-run"]);
    success(&preview);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("dry-run:"));
    assert!(new_objects(&fixture.state()).is_empty());
    assert_eq!(files(&legacy), original);

    let migrated = fixture.migration(&[]);
    success(&migrated);
    let report = String::from_utf8_lossy(&migrated.stdout);
    assert!(report.contains("migrated:"));
    assert!(report.contains("legacy jobs database"));
    assert!(report.contains("unknown legacy objects"));
    assert_eq!(files(&legacy), original);
    assert_eq!(files(&other), other_original);
    assert_eq!(
        fs::read_to_string(&ledger).unwrap(),
        "{\"agentId\":\"remote\",\"runId\":\"receipt\"}\n"
    );
    assert_migrated_database(&fixture.state());
    assert_eq!(
        fs::read(fixture.state().join("skill-installs/journals/keep.txt")).unwrap(),
        b"journal association"
    );
    assert_eq!(
        fs::read(fixture.state().join("skill-installs/staging/keep.txt")).unwrap(),
        b"staged payload"
    );
    assert_private(&fixture.state(), true);
    for name in new_objects(&fixture.state()) {
        assert_private(&fixture.state().join(name), true);
    }

    // Skip must happen before even reading a requested config, and preserve target bytes.
    let target_before = files_without_agent(&fixture.state());
    let skip = fixture.migration(&["--config", "/does-not-exist/fixture.json"]);
    success(&skip);
    assert!(String::from_utf8_lossy(&skip.stdout).contains("skip:"));
    assert_eq!(files_without_agent(&fixture.state()), target_before);

    {
        let _agent = fixture.start("before");
        fixture.assert_history("before");
    }
    fixture.set_identity("after");
    {
        let _agent = fixture.start("after");
        fixture.assert_history("after");
    }
    assert_eq!(files(&legacy), original);
    assert_eq!(files(&other), other_original);
    assert!(!fixture.state().join("agent/after").exists());
}

#[test]
fn migration_rejects_live_and_invalid_sources_and_rolls_back_publication_errors() {
    let fixture = Fixture::new();
    let legacy = fixture.seed("before", "saved-process");
    let database = legacy.join("events.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection.pragma_update(None, "user_version", 99).unwrap();
    drop(connection);
    fixture.assert_rejected();
    let connection = Connection::open(&database).unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    drop(connection);
    let original = files(&legacy);

    let link = legacy.join("skill-installs/staging/link");
    symlink(&fixture.config, &link).unwrap();
    fixture.assert_rejected();
    fs::remove_file(link).unwrap();
    for lock in [
        PathBuf::from(format!("{}.run.lock", fixture.config.display())),
        fixture.agentic().join(".agent-state-migration.lock"),
    ] {
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock)
            .unwrap();
        // SAFETY: held owns this descriptor and keeps it alive until the child exits.
        assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
        fixture.assert_rejected();
        drop(held);
    }
    let socket = fixture.socket("before");
    fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(&socket).unwrap();
    fixture.assert_rejected();
    drop(listener);
    fs::remove_file(socket).unwrap();

    // Exercise real renames, including a signal delivered after rename already succeeded
    // and a second signal during rollback. Neither may leave any published object.
    let injected = r#"
import importlib.util, os, signal, sys
spec=importlib.util.spec_from_file_location('migration',sys.argv[1])
module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
rename=module.os.rename; calls=0; mode=sys.argv[3]
def interrupted_rename(source,target):
    global calls
    calls+=1
    if mode=='before-second' and calls==2:
        raise OSError('injected second publication failure')
    if mode=='signals' and calls==2:
        os.kill(os.getpid(),signal.SIGTERM)
    result=rename(source,target)
    if calls==1 and mode=='after-first':
        raise module.MigrationError('injected failure after successful rename')
    if calls==1 and mode=='signals':
        os.kill(os.getpid(),signal.SIGTERM)
    return result
module.os.rename=interrupted_rename
sys.argv=[sys.argv[1],'--config',sys.argv[2]]
sys.exit(module.main())
"#;
    for (mode, message) in [
        ("before-second", "injected second publication failure"),
        ("after-first", "injected failure after successful rename"),
        ("signals", "migration interrupted by signal"),
    ] {
        let mut command = fixture.command(Path::new("python3"));
        command
            .arg("-c")
            .arg(injected)
            .arg(script())
            .arg(&fixture.config)
            .arg(mode);
        let result = output(command);
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(new_objects(&fixture.state()).is_empty(), "{mode}");
        assert_eq!(files(&legacy), original, "{mode}");
        assert!(fs::read_dir(fixture.agentic()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".state-migration-")));
    }
}

#[test]
fn migration_rejects_unreadable_process_json_without_publishing_any_state() {
    let fixture = Fixture::new();
    let legacy = fixture.seed("before", "saved-process");
    let database = legacy.join("process.sqlite3");
    let connection = Connection::open(&database).unwrap();
    let (valid_info, valid_detail): (String, String) = connection
        .query_row("SELECT info_json,detail_json FROM processes", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    drop(connection);
    let info: Value = serde_json::from_str(&valid_info).unwrap();
    let detail: Value = serde_json::from_str(&valid_detail).unwrap();
    let mut malformed = vec![(json!({}), detail.clone())];
    for (field, bad_value) in [
        ("processId", json!("different-process")),
        ("kind", json!("unsupported-kind")),
        ("state", json!("unsupported-state")),
        ("captureStatus", json!("unsupported-capture")),
        ("createdAt", json!("2026-02-31T12:00:00Z")),
        ("updatedAt", json!("2026-10-11T12:00:00")),
        ("finishedAt", json!(false)),
        ("group", json!([])),
        ("args", json!([1])),
        ("cancelRequested", json!(1)),
        ("batchIndex", json!(-1)),
        ("exitCode", json!(2147483648_i64)),
    ] {
        let mut invalid = info.clone();
        invalid[field] = bad_value;
        malformed.push((invalid, detail.clone()));
    }
    for (field, bad_value) in [
        ("process", json!({})),
        ("detailAvailable", Value::Null),
        ("resultAvailable", json!("yes")),
        ("resultBytes", json!(true)),
        ("resultSha256", json!([])),
        ("error", json!({"code": "failure", "message": 1})),
    ] {
        let mut invalid = detail.clone();
        invalid[field] = bad_value;
        malformed.push((info.clone(), invalid));
    }
    for (invalid_info, invalid_detail) in malformed {
        let connection = Connection::open(&database).unwrap();
        connection
            .execute(
                "UPDATE processes SET info_json=?1,detail_json=?2",
                params![invalid_info.to_string(), invalid_detail.to_string()],
            )
            .unwrap();
        drop(connection);
        let original = files(&legacy);
        fixture.assert_rejected();
        assert_eq!(files(&legacy), original);
        assert!(fs::read_dir(fixture.agentic()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".state-migration-")));
    }
}

fn assert_migrated_database(state: &Path) {
    let events = Connection::open(state.join("events.sqlite3")).unwrap();
    assert_eq!(
        events
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        events
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='event_store_metadata'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let origin: (String, String, String) = events
        .query_row(
            "SELECT origin_run_id,origin_request_id,origin_command_hash FROM internal_sources",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        origin,
        (
            "remote-run".into(),
            "remote-request".into(),
            "remote-hash".into()
        )
    );
    let processes = Connection::open(state.join("process.sqlite3")).unwrap();
    assert_eq!(
        processes
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    let (info, detail, stdout): (String, String, Vec<u8>) = processes
        .query_row(
            "SELECT info_json,detail_json,stdout_bytes FROM processes",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    let info: Value = serde_json::from_str(&info).unwrap();
    let detail: Value = serde_json::from_str(&detail).unwrap();
    assert!(info.get("agentId").is_none());
    assert!(detail["process"].get("agentId").is_none());
    assert_eq!(info["metadata"]["agentId"], "business-owner");
    assert_eq!(detail["result"]["agentId"], "remote-result-owner");
    assert_eq!(stdout, b"persisted stdout\n");
    assert_eq!(
        processes
            .query_row("SELECT message FROM process_event_completions", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "saved completion"
    );
    assert_eq!(processes.query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name LIKE 'idx_processes_%'", [], |row| row.get::<_, i64>(0)).unwrap(), 5);
    let mut statement = processes.prepare("PRAGMA table_info(processes)").unwrap();
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(!columns.iter().any(|column| column == "agent_id"));
}

struct Fixture {
    root: PathBuf,
    config: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from("/tmp").join(format!(
            "asm-{}",
            &Uuid::new_v4().simple().to_string()[..12]
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            config: root.join("config.json"),
            root,
        };
        for directory in ["h", "c", "d", "k", "r", "t", "w"] {
            let path = fixture.root.join(directory);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut command = fixture.command(Path::new(env!("CARGO_BIN_EXE_agentic-gpt")));
        command
            .args(["config", "--config"])
            .arg(&fixture.config)
            .args(["init", "--non-interactive"]);
        success(&output(command));
        fixture.set_identity("before");
        fixture
    }

    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command.process_group(0);
        for (name, directory) in [
            ("HOME", "h"),
            ("XDG_CONFIG_HOME", "c"),
            ("XDG_DATA_HOME", "d"),
            ("XDG_CACHE_HOME", "k"),
            ("XDG_RUNTIME_DIR", "r"),
            ("TMPDIR", "t"),
        ] {
            command.env(name, self.root.join(directory));
        }
        command.env("PYTHONDONTWRITEBYTECODE", "1");
        command.env_remove("AGENTIC_SUPERVISOR_TOKEN");
        command.env_remove("TMUX");
        command.env("TMUX_TMPDIR", self.root.join("r"));
        command
    }

    fn agentic(&self) -> PathBuf {
        self.root.join("h/.agentic_gpt")
    }
    fn state(&self) -> PathBuf {
        self.agentic().join("state")
    }
    fn socket(&self, identity: &str) -> PathBuf {
        self.agentic()
            .join("runtime/agent")
            .join(identity)
            .join("mcp.sock")
    }

    fn set_identity(&self, identity: &str) {
        let mut config: Value = serde_json::from_slice(&fs::read(&self.config).unwrap()).unwrap();
        config["agentId"] = json!(identity);
        config["mode"] = json!("local");
        config["profile"] = json!("normal");
        config["workspaceRoot"] = json!(self.root.join("w"));
        config["shell"]["initFile"] = Value::Null;
        config["tunnel"] = Value::Null;
        config["mcpServers"] = json!({});
        config["httpMcp"]["enabled"] = json!(false);
        config["browser"]["managed"]["enabled"] = json!(false);
        fs::write(&self.config, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    }

    fn seed(&self, identity: &str, process_id: &str) -> PathBuf {
        let root = self.state().join("agent").join(identity);
        fs::create_dir_all(root.join("skill-installs/journals")).unwrap();
        fs::create_dir_all(root.join("skill-installs/staging")).unwrap();
        fs::write(
            root.join("skill-installs/journals/keep.txt"),
            "journal association",
        )
        .unwrap();
        fs::write(
            root.join("skill-installs/staging/keep.txt"),
            "staged payload",
        )
        .unwrap();
        fs::write(root.join("active-skills.json"), "{\"activeSkills\":[]}").unwrap();
        fs::write(root.join("jobs.sqlite3"), b"legacy jobs bytes").unwrap();
        fs::write(root.join("unrelated.txt"), "unknown legacy bytes").unwrap();
        let now = Utc::now().to_rfc3339();
        let events = Connection::open(root.join("events.sqlite3")).unwrap();
        events.execute_batch(OLD_EVENTS).unwrap();
        events.execute("INSERT INTO events(event_id,message,severity,created_at,status,source_kind,source_ref) VALUES('saved-event','saved event','low',?1,'pending','process',?2)", params![now, process_id]).unwrap();
        events.execute("INSERT INTO internal_sources(source_kind,source_ref,response_state,policy_json,origin_run_id,origin_request_id,origin_command_hash) VALUES('process',?1,'suppressed','{\"low_ttl_seconds\":86400,\"overrides\":{}}','remote-run','remote-request','remote-hash')", [process_id]).unwrap();
        let processes = Connection::open(root.join("process.sqlite3")).unwrap();
        processes.execute_batch(OLD_PROCESSES).unwrap();
        // Omitted args/cancelRequested/resultAvailable exercise the model's valid defaults.
        let info = json!({"agentId":identity,"processId":process_id,"group":"migrated", "kind":"command","state":"completed","createdAt":now,"startedAt":now,"updatedAt":now,"finishedAt":now,"program":"printf","exitCode":0,"terminationEvidence":"process_exit","captureStatus":"complete","metadata":{"agentId":"business-owner"}});
        let detail = json!({"process":info,"detailAvailable":true,"result":{"agentId":"remote-result-owner"}});
        processes.execute("INSERT INTO processes(process_id,agent_id,group_name,kind,state,created_at,started_at,updated_at,finished_at,info_json,detail_json,stdout_bytes,stdout_end_offset,stderr_bytes,stderr_end_offset,capture_status) VALUES(?1,?2,'migrated','command','completed',?3,?3,?3,?3,?4,?5,?6,'17',?7,'17','complete')", params![process_id, identity, now, info.to_string(), detail.to_string(), b"persisted stdout\n".as_slice(), b"persisted stderr\n".as_slice()]).unwrap();
        processes.execute("INSERT INTO process_event_completions VALUES(?1,'process.completed','saved completion',?2)", params![process_id, now]).unwrap();
        root
    }

    fn migration(&self, args: &[&str]) -> Output {
        let mut command = self.command(Path::new("python3"));
        command
            .arg(script())
            .arg("--config")
            .arg(&self.config)
            .args(args);
        output(command)
    }

    fn assert_rejected(&self) {
        let result = self.migration(&[]);
        assert!(
            !result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(new_objects(&self.state()).is_empty());
    }

    fn start(&self, identity: &str) -> Agent {
        let log_path = self.root.join(format!("agent-{identity}.stderr"));
        let log = File::create(&log_path).unwrap();
        let mut command = self.command(Path::new(env!("CARGO_BIN_EXE_agentic-gpt")));
        command
            .args(["run", "--config"])
            .arg(&self.config)
            .stdout(Stdio::null())
            .stderr(log);
        let mut agent = Agent {
            child: command.spawn().unwrap(),
            socket: self.socket(identity),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if fs::symlink_metadata(&agent.socket).is_ok_and(|meta| meta.file_type().is_socket()) {
                return agent;
            }
            assert!(
                agent.child.try_wait().unwrap().is_none(),
                "Agent exited: {}",
                fs::read_to_string(&log_path).unwrap()
            );
            assert!(
                Instant::now() < deadline,
                "Agent readiness timeout: {}",
                fs::read_to_string(&log_path).unwrap()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn call(&self, tool: &str, arguments: Value) -> Value {
        let mut command = self.command(Path::new(env!("CARGO_BIN_EXE_agentic-gpt")));
        command
            .args(["local", "--config"])
            .arg(&self.config)
            .args(["call", tool, "--arguments"])
            .arg(arguments.to_string());
        let result = output(command);
        success(&result);
        let result: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_ne!(result["isError"], true, "{result}");
        assert!(result["structuredContent"]["error"].is_null(), "{result}");
        result["structuredContent"].clone()
    }

    fn assert_history(&self, identity: &str) {
        let list = self.call("process.list", json!({"group":"migrated"}));
        let processes = list["processes"].as_array().unwrap();
        assert_eq!(processes.len(), 1, "{list}");
        assert_eq!(processes[0]["processId"], "saved-process");
        let read = self.call(
            "process.read",
            json!({"processId":"saved-process","waitSeconds":0}),
        );
        assert_eq!(read["agentId"], identity, "{read}");
        assert_eq!(read["state"], "completed", "{read}");
        assert_eq!(
            read["output"]["stdout"]["data"], "persisted stdout\n",
            "{read}"
        );
        assert_eq!(
            read["output"]["stderr"]["data"], "persisted stderr\n",
            "{read}"
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Agent {
    child: Child,
    socket: PathBuf,
}
impl Drop for Agent {
    fn drop(&mut self) {
        // SAFETY: the test owns this process group and waits for it before deleting HOME.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.socket);
    }
}

struct ChildGuard(Option<Child>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            // SAFETY: this guard owns the child and its independently created process group.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn output(mut command: Command) -> Output {
    let mut guard = ChildGuard(Some(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    ));
    let deadline = Instant::now() + Duration::from_secs(20);
    while guard.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "fixture command timed out: {command:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    guard.0.take().unwrap().wait_with_output().unwrap()
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/migrate-agent-state.py")
}

fn new_objects(state: &Path) -> Vec<&'static str> {
    [
        "events.sqlite3",
        "process.sqlite3",
        "active-skills.json",
        "skill-installs",
    ]
    .into_iter()
    .filter(|name| state.join(name).exists())
    .collect()
}

fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn files_without_agent(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    files(root)
        .into_iter()
        .filter(|(path, _)| !path.starts_with("agent"))
        .collect()
}

fn assert_private(path: &Path, recurse: bool) {
    let metadata = fs::symlink_metadata(path).unwrap();
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        if metadata.is_dir() { 0o700 } else { 0o600 },
        "{}",
        path.display()
    );
    if recurse && metadata.is_dir() {
        for child in fs::read_dir(path).unwrap() {
            let child = child.unwrap().path();
            if child.file_name().unwrap() != "agent" {
                assert_private(&child, true);
            }
        }
    }
}
