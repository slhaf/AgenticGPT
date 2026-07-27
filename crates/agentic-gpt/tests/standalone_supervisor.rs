use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[test]
fn hidden_worker_reloads_policy_path_limit_and_mcp_without_restart() {
    let root = std::env::temp_dir().join(format!("agentic-live-reload-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let result = run_live_reload(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn supervisor_launches_real_worker_and_completes_local_mcp_call() {
    let root = std::env::temp_dir().join(format!("agentic-standalone-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let result = run_smoke(&root, "normal", false, false);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn supervisor_launches_real_room_worker_and_advertises_room_surface() {
    let root = std::env::temp_dir().join(format!("agentic-standalone-room-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let result = run_smoke(&root, "room", true, false);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn supervised_journal_mode_omits_agentic_inner_timestamp() {
    let root = std::env::temp_dir().join(format!("agentic-journal-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let result = run_smoke(&root, "normal", true, false);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn supervised_invalid_config_warning_is_supervisor_owned() {
    let root = std::env::temp_dir().join(format!("agentic-invalid-reload-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let result = run_smoke(&root, "normal", true, true);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

fn run_smoke(
    root: &Path,
    profile: &str,
    journal_mode: bool,
    invalid_config_probe: bool,
) -> Result<(), String> {
    let binary = std::env::var("CARGO_BIN_EXE_agentic-gpt")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_agentic_gpt"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/agentic-gpt")
        });
    if !binary.exists() {
        return Err(format!("agentic binary not found: {}", binary.display()));
    }

    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    let key_path = root.join("tunnel-api-key");
    let fake_tunnel = root.join("fake-tunnel.sh");
    let response_path = root.join("worker-response.jsonl");
    let worker_stderr_path = root.join("worker.stderr");
    let marker_path = root.join("tool-complete");
    let agent_id = format!("standalone-e2e-{}", Uuid::new_v4().simple());

    let init = Command::new(&binary)
        .args(["config", "--config", config_path.to_str().unwrap(), "init"])
        .output()
        .map_err(|error| format!("config init failed to spawn: {error}"))?;
    if !init.status.success() {
        return Err(format!(
            "config init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        ));
    }

    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    fs::write(&key_path, "integration-secret\n").map_err(|error| error.to_string())?;
    let mut config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    config["agentId"] = Value::String(agent_id.clone());
    config["workspaceRoot"] = Value::String(workspace.to_string_lossy().into_owned());
    config["pathPolicy"]["writeRoots"] = json!([workspace.to_string_lossy()]);
    config["tunnel"] = json!({
        "tunnelId": "tunnel_local_integration",
        "apiKey": format!("file:{}", key_path.display()),
        "client": {
            "version": null,
            "cacheDir": root.join("cache").to_string_lossy(),
            "autoDownload": false,
            "executable": fake_tunnel,
            "downloadUrl": null,
            "sha256": null
        },
        "hubReporting": { "enabled": false, "detail": "metadata" }
    });
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let (health_port, stop_health, health_thread) = start_health_server()?;
    let script = fake_tunnel_script(
        health_port,
        &response_path,
        &worker_stderr_path,
        &marker_path,
    );
    fs::write(&fake_tunnel, script).map_err(|error| error.to_string())?;
    fs::set_permissions(&fake_tunnel, fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;

    let mut supervisor_command = Command::new(&binary);
    supervisor_command
        .args([
            "run-as-standalone",
            "--config",
            config_path.to_str().unwrap(),
            "--profile",
            profile,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if journal_mode {
        supervisor_command.env("JOURNAL_STREAM", "9:9");
    }
    let mut supervisor = supervisor_command
        .spawn()
        .map_err(|error| format!("supervisor failed to spawn: {error}"))?;

    if invalid_config_probe {
        thread::sleep(Duration::from_millis(300));
        fs::write(&config_path, "{ invalid json\n").map_err(|error| error.to_string())?;
        thread::sleep(Duration::from_millis(2600));
    }

    let completed = wait_for_marker(&mut supervisor, &marker_path);
    stop_supervisor(&mut supervisor);
    stop_health.store(true, Ordering::Release);
    let _ = health_thread.join();
    let supervisor_stderr = supervisor
        .stderr
        .take()
        .map(|mut stream| {
            let mut value = String::new();
            let _ = stream.read_to_string(&mut value);
            value
        })
        .unwrap_or_default();

    if let Err(error) = completed {
        let response = read_optional(&response_path);
        let worker_stderr = read_optional(&worker_stderr_path);
        return Err(format!(
            "{error}; response={response}; worker_stderr={worker_stderr}; supervisor_stderr={supervisor_stderr}"
        ));
    }
    let response = fs::read_to_string(&response_path).map_err(|error| error.to_string())?;
    if !response.contains("standalone-e2e-ok") {
        return Err(format!(
            "worker response did not contain tool output: {response}"
        ));
    }
    let room_tool = "room.diary.append";
    if profile == "room" && !response.contains(room_tool) {
        return Err(format!(
            "room worker response did not advertise Room tools: {response}"
        ));
    }
    if profile == "normal" && response.contains(room_tool) {
        return Err(format!(
            "Normal worker response advertised a Room tool: {response}"
        ));
    }
    for expected in [
        "INFO tunnel.stdout: child-info",
        "WARN tunnel.stdout: child-warning",
        "ERROR tunnel.stderr: child-error",
        "INFO tunnel.stdout: unknown-stdout",
        "WARN tunnel.stderr: unknown-stderr",
    ] {
        if !supervisor_stderr.contains(expected) {
            return Err(format!(
                "supervisor log missing `{expected}`: {supervisor_stderr}"
            ));
        }
    }
    if supervisor_stderr.contains("tunnel.stdout: 2026-07-25T16:00:00") {
        return Err(format!(
            "forwarded child timestamp was not stripped: {supervisor_stderr}"
        ));
    }
    if journal_mode {
        let info_line = supervisor_stderr
            .lines()
            .find(|line| line.contains("INFO tunnel.stdout: child-info"))
            .ok_or("journal child info line missing")?;
        assert!(info_line.starts_with("INFO tunnel.stdout: child-info"));
        let worker_info = supervisor_stderr
            .lines()
            .find(|line| line.contains("INFO tunnel.stderr: mcp_tool; ingress=tunnel:stdio;"))
            .ok_or("journal hidden-worker INFO line missing")?;
        assert!(worker_info.starts_with("INFO tunnel.stderr: mcp_tool; ingress=tunnel:stdio;"));
        assert!(!supervisor_stderr.contains("WARN tunnel.stderr: INFO mcp_tool;"));
    }
    if invalid_config_probe {
        assert_eq!(
            supervisor_stderr
                .matches("standalone config reload failed;")
                .count(),
            1,
            "expected one supervisor-owned invalid-config warning: {supervisor_stderr}"
        );
        assert!(!supervisor_stderr.contains("standalone live config reload rejected;"));
    }
    Ok(())
}

fn run_live_reload(root: &Path) -> Result<(), String> {
    let binary = std::env::var("CARGO_BIN_EXE_agentic-gpt")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_agentic_gpt"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/agentic-gpt")
        });
    if !binary.exists() {
        return Err(format!("agentic binary not found: {}", binary.display()));
    }

    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    let init = Command::new(&binary)
        .args(["config", "--config", config_path.to_str().unwrap(), "init"])
        .output()
        .map_err(|error| format!("config init failed to spawn: {error}"))?;
    if !init.status.success() {
        return Err(format!(
            "config init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        ));
    }
    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    let path_root = root.join("path-target");
    fs::create_dir_all(&path_root).map_err(|error| error.to_string())?;
    let mut config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let agent_id = format!("live-reload-e2e-{}", Uuid::new_v4().simple());
    config["agentId"] = Value::String(agent_id.clone());
    config["workspaceRoot"] = Value::String(workspace.to_string_lossy().into_owned());
    config["pathPolicy"]["writeRoots"] =
        json!([workspace.to_string_lossy(), path_root.to_string_lossy()]);
    config["tunnel"] = json!({
        "tunnelId": "tunnel_live_reload_test",
        "apiKey": "env:AGENTIC_TEST_TUNNEL_KEY",
        "hubReporting": { "enabled": false, "detail": "metadata" }
    });
    config["policy"]["allow"] = json!([
        { "program": "/usr/bin/printf", "argsPrefix": [] },
        { "program": "/bin/sleep", "argsPrefix": [] },
        { "program": "/usr/bin/touch", "argsPrefix": [] }
    ]);
    config["policy"]["deny"] = json!([
        { "program": "/usr/bin/printf", "argsPrefix": [] }
    ]);
    config["limits"]["maxActiveJobs"] = json!(1);
    config["mcpServers"] = json!({
        "primary": {
            "enabled": true,
            "transport": "streamable-http",
            "url": "https://old.example/mcp"
        }
    });
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let token = "live-reload-token";
    let mut worker = Command::new(&binary)
        .args([
            "stdio-worker",
            "--config",
            config_path.to_str().unwrap(),
            "--profile",
            "normal",
            "--supervisor-token",
            token,
        ])
        .env("AGENTIC_GPT_SUPERVISOR_TOKEN", token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("worker failed to spawn: {error}"))?;
    let mut stdin = worker.stdin.take().ok_or("worker stdin unavailable")?;
    let stdout = worker.stdout.take().ok_or("worker stdout unavailable")?;
    let mut stdout = BufReader::new(stdout);
    let mut stderr = worker.stderr.take().ok_or("worker stderr unavailable")?;

    send_line(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "live-reload-test", "version": "1" }
            }
        }),
    )?;
    let _ = response_for(&mut stdout, 1)?;
    send_line(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )?;

    let socket_path = local_socket_path(&agent_id)?;
    wait_for_unix_socket(&socket_path, Duration::from_secs(5))?;
    let local_tools = Command::new(&binary)
        .args(["local", "list-tools", "--config"])
        .arg(&config_path)
        .output()
        .map_err(|error| format!("local peer list-tools failed to spawn: {error}"))?;
    if !local_tools.status.success() {
        return Err(format!(
            "local peer list-tools failed: {}",
            String::from_utf8_lossy(&local_tools.stderr)
        ));
    }
    let local_tools: Value =
        serde_json::from_slice(&local_tools.stdout).map_err(|error| error.to_string())?;
    let local_tools = local_tools
        .as_array()
        .ok_or("local peer tool list is not an array")?;
    assert_eq!(local_tools.len(), 24);
    assert!(local_tools
        .iter()
        .all(|tool| tool["_meta"]["surface"] == "agent-local"));

    let local_info = Command::new(&binary)
        .args(["local", "call", "agent.info", "--config"])
        .arg(&config_path)
        .args(["--arguments", "{}"])
        .output()
        .map_err(|error| format!("local peer agent.info failed to spawn: {error}"))?;
    if !local_info.status.success() {
        return Err(format!(
            "local peer agent.info failed: {}",
            String::from_utf8_lossy(&local_info.stderr)
        ));
    }
    let local_info: Value =
        serde_json::from_slice(&local_info.stdout).map_err(|error| error.to_string())?;
    assert_eq!(
        local_info["structuredContent"]["identity"]["transport"],
        "tunnel-stdio"
    );
    assert_eq!(
        local_info["structuredContent"]["connections"]["localMcp"]["status"],
        "ready"
    );

    let baseline_mcp = call_tool(&mut stdin, &mut stdout, 20, "mcp.list", json!({}))?;
    let baseline_servers = baseline_mcp["result"]["structuredContent"]["servers"]
        .as_array()
        .ok_or("baseline mcp servers missing")?;
    assert_eq!(baseline_servers.len(), 1);
    assert_eq!(baseline_servers[0]["id"], "primary");
    assert_eq!(baseline_servers[0]["url"], "https://old.example/mcp");

    let tunnel_batch = call_tool(
        &mut stdin,
        &mut stdout,
        24,
        "mcp.batch",
        json!({
            "calls": [
                {"id": "dup", "serverId": "primary", "toolName": "fake.tool", "arguments": {}},
                {"id": "dup", "serverId": "primary", "toolName": "fake.tool", "arguments": {}}
            ],
            "waitSeconds": 0
        }),
    )?;
    assert_eq!(
        tunnel_batch["result"]["structuredContent"]["error"]["code"],
        "mcp_batch_failed"
    );
    assert!(
        tunnel_batch["result"]["structuredContent"]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("mcp_batch_call_id_duplicate"))
    );
    let tunnel_batch_audit = fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))
        .map_err(|error| error.to_string())?;
    assert!(tunnel_batch_audit.contains("\"tool\":\"mcp.batch\""));
    assert!(tunnel_batch_audit.contains("\"requestSource\":\"tunnel:mcp.batch\""));
    assert!(tunnel_batch_audit.contains("\"outcome\":\"validation_rejected\""));
    assert!(tunnel_batch_audit.contains("\"errorCode\":\"mcp_batch_call_id_duplicate\""));
    assert!(!tunnel_batch_audit.contains("\"program\":\"mcp.callTool\""));

    let denied = call_tool(
        &mut stdin,
        &mut stdout,
        2,
        "process.exec",
        json!({ "program": "/usr/bin/printf", "args": ["denied"], "waitSeconds": 2 }),
    )?;
    assert_eq!(
        denied["result"]["structuredContent"]["job"]["rejectReason"],
        json!("policy_denied")
    );

    config["policy"]["deny"] = json!([]);
    config["mcpServers"]["primary"]["url"] = json!("https://new.example/mcp");
    config["mcpServers"]["local"] = json!({
        "enabled": false,
        "transport": "stdio",
        "url": "node ./local.mjs"
    });
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let allowed = call_tool(
        &mut stdin,
        &mut stdout,
        3,
        "process.exec",
        json!({ "program": "/usr/bin/printf", "args": ["reloaded"], "waitSeconds": 2 }),
    )?;
    assert_eq!(
        allowed["result"]["structuredContent"]["job"]["state"],
        json!("completed")
    );
    let changed_mcp = call_tool(&mut stdin, &mut stdout, 21, "mcp.list", json!({}))?;
    let changed_servers = changed_mcp["result"]["structuredContent"]["servers"]
        .as_array()
        .ok_or("changed mcp servers missing")?;
    assert_eq!(changed_servers.len(), 2);
    assert!(changed_servers.iter().any(|server| {
        server["id"] == "primary"
            && server["url"] == "https://new.example/mcp"
            && server["enabled"] == true
    }));
    assert!(changed_servers.iter().any(|server| {
        server["id"] == "local" && server["transport"] == "stdio" && server["enabled"] == false
    }));

    let path_target = path_root.join("reload-path-denied");
    config["pathPolicy"]["denyRoots"] = json!([path_root.to_string_lossy()]);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let path_denied = call_tool(
        &mut stdin,
        &mut stdout,
        31,
        "process.exec",
        json!({
            "program": "/usr/bin/touch",
            "args": [path_target.to_string_lossy()],
            "waitSeconds": 2
        }),
    )?;
    assert_eq!(
        path_denied["result"]["structuredContent"]["job"]["rejectReason"],
        json!("path_denied")
    );
    config["pathPolicy"]["denyRoots"] = json!([]);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let path_allowed = call_tool(
        &mut stdin,
        &mut stdout,
        32,
        "process.exec",
        json!({
            "program": "/usr/bin/touch",
            "args": [path_target.to_string_lossy()],
            "waitSeconds": 2
        }),
    )?;
    assert_eq!(
        path_allowed["result"]["structuredContent"]["job"]["state"],
        json!("completed")
    );
    assert!(path_target.exists());

    config["policy"]["deny"] = json!([
        { "program": "/usr/bin/printf", "argsPrefix": [] }
    ]);
    config["mcpServers"]["primary"]["transport"] = json!("sse");
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let retained = call_tool(
        &mut stdin,
        &mut stdout,
        33,
        "process.exec",
        json!({ "program": "/usr/bin/printf", "args": ["last-good"], "waitSeconds": 2 }),
    )?;
    assert_eq!(
        retained["result"]["structuredContent"]["job"]["state"],
        json!("completed")
    );
    let retained_mcp = call_tool(&mut stdin, &mut stdout, 22, "mcp.list", json!({}))?;
    let retained_servers = retained_mcp["result"]["structuredContent"]["servers"]
        .as_array()
        .ok_or("retained mcp servers missing")?;
    assert_eq!(retained_servers.len(), 2);
    assert!(retained_servers.iter().any(|server| {
        server["id"] == "primary"
            && server["transport"] == "streamable-http"
            && server["url"] == "https://new.example/mcp"
    }));

    config["policy"]["deny"] = json!([]);
    config["limits"]["maxActiveJobs"] = json!(1);
    config["mcpServers"]["primary"]["transport"] = json!("streamable-http");
    config["mcpServers"]["primary"]["enabled"] = json!(false);
    config["mcpServers"]
        .as_object_mut()
        .unwrap()
        .remove("local");
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let removed_mcp = call_tool(&mut stdin, &mut stdout, 23, "mcp.list", json!({}))?;
    let removed_servers = removed_mcp["result"]["structuredContent"]["servers"]
        .as_array()
        .ok_or("removed mcp servers missing")?;
    assert_eq!(removed_servers.len(), 1);
    assert_eq!(removed_servers[0]["id"], "primary");
    assert_eq!(removed_servers[0]["enabled"], false);
    assert_eq!(removed_servers[0]["url"], "https://new.example/mcp");

    let active = call_tool(
        &mut stdin,
        &mut stdout,
        4,
        "process.exec",
        json!({ "program": "/bin/sleep", "args": ["5"], "waitSeconds": 0 }),
    )?;
    let active_job_id = active["result"]["structuredContent"]["jobId"]
        .as_str()
        .ok_or("active Job id missing")?
        .to_string();
    let active_state = active["result"]["structuredContent"]["job"]["state"]
        .as_str()
        .ok_or("active Job state missing")?;
    assert!(matches!(active_state, "starting" | "running"));

    config["limits"]["maxActiveJobs"] = json!(0);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let rejected = call_tool(
        &mut stdin,
        &mut stdout,
        5,
        "process.exec",
        json!({ "program": "/usr/bin/printf", "args": ["blocked"], "waitSeconds": 2 }),
    )?;
    let reason = rejected["result"]["structuredContent"]["job"]["rejectReason"]
        .as_str()
        .ok_or("capacity rejection reason missing")?;
    assert!(reason.starts_with("max_active_jobs_reached; "));
    assert!(reason.contains("active=1; requested=1; limit=0"));

    config["limits"]["maxActiveJobs"] = json!(2);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let admitted = call_tool(
        &mut stdin,
        &mut stdout,
        6,
        "process.exec",
        json!({ "program": "/usr/bin/printf", "args": ["limit-reloaded"], "waitSeconds": 2 }),
    )?;
    assert_eq!(
        admitted["result"]["structuredContent"]["job"]["state"],
        json!("completed")
    );

    let _ = call_tool(
        &mut stdin,
        &mut stdout,
        7,
        "job.cancel",
        json!({ "jobId": active_job_id }),
    )?;
    drop(stdin);
    stop_child_gracefully(&mut worker, Duration::from_secs(5));
    wait_for_path_absent(&socket_path, Duration::from_secs(2))?;
    let mut human_logs = String::new();
    stderr
        .read_to_string(&mut human_logs)
        .map_err(|error| error.to_string())?;
    assert!(!human_logs.contains("status=started"));
    assert!(!human_logs.contains("runId="));
    assert!(!human_logs.contains("jobId="));
    assert!(human_logs.contains("status=active"));
    assert_eq!(human_logs.matches("managed_job;").count(), 1);
    assert!(!human_logs.contains(&active_job_id));
    for line in human_logs.lines().filter(|line| line.contains("run=")) {
        let run_id = line
            .split("run=")
            .nth(1)
            .and_then(|value| value.split(';').next())
            .unwrap();
        assert_eq!(run_id.len(), 12);
        assert!(run_id
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
    }
    Ok(())
}

fn write_config(path: &Path, config: &Value) -> Result<(), String> {
    fs::write(
        path,
        serde_json::to_vec_pretty(config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn send_line(stdin: &mut impl Write, value: Value) -> Result<(), String> {
    writeln!(stdin, "{}", value).map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())
}

fn response_for(reader: &mut impl BufRead, id: i64) -> Result<Value, String> {
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err(format!(
                "worker closed stdout while waiting for response {id}"
            ));
        }
        let value: Value = serde_json::from_str(line.trim()).map_err(|error| error.to_string())?;
        if value["id"] == json!(id) {
            return Ok(value);
        }
    }
}

fn call_tool(
    stdin: &mut impl Write,
    reader: &mut impl BufRead,
    id: i64,
    name: &str,
    arguments: Value,
) -> Result<Value, String> {
    send_line(
        stdin,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    )?;
    response_for(reader, id)
}

fn read_optional(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("<unavailable: {error}>"))
}

fn fake_tunnel_script(
    health_port: u16,
    response_path: &Path,
    worker_stderr_path: &Path,
    marker_path: &Path,
) -> String {
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "local-tunnel-test", "version": "1"}
        }
    });
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });
    let list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "process.exec",
            "arguments": {
                "program": "/usr/bin/printf",
                "args": ["standalone-e2e-ok"]
            }
        }
    });
    let requests = format!("{}\n{}", list, call);
    let script = format!(
        "#!/bin/sh\nset -eu\nmode=\"$1\"\nhealth_url_file=\"\"\nmcp_binding=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--health.url-file\" ]; then shift; health_url_file=\"$1\"; fi\n  if [ \"$1\" = \"--mcp.command\" ]; then shift; mcp_binding=\"$1\"; fi\n  shift\ndone\nworker=\"${{mcp_binding#*command=}}\"\nfirst=\"$(printf '%.1s' \"$worker\")\"\nif [ \"$first\" = '\"' ] || [ \"$first\" = \"'\" ]; then printf 'invalid whole-command quoting\\n' >&2; exit 23; fi\nif [ \"$mode\" = \"doctor\" ]; then exit 0; fi\nprintf 'http://127.0.0.1:{health_port}\\n' > \"$health_url_file\"\nrequest_initialize={initialize}\nrequest_initialized={initialized}\nrequest_call={call}\nprintf '%s\\n%s\\n%s\\n' \"$request_initialize\" \"$request_initialized\" \"$request_call\" | sh -c \"$worker\" > {response} 2> {worker_stderr} || true\nif grep -q 'standalone-e2e-ok' {response}; then touch {marker}; fi\nsleep 60\n",
        health_port = health_port,
        initialize = sh_quote(&initialize.to_string()),
        initialized = sh_quote(&initialized.to_string()),
        call = sh_quote(&requests),
        response = sh_quote(&response_path.to_string_lossy()),
        worker_stderr = sh_quote(&worker_stderr_path.to_string_lossy()),
        marker = sh_quote(&marker_path.to_string_lossy()),
    );
    let script = script.replace(
        "|| true\nif grep -q 'standalone-e2e-ok'",
        &format!(
            "|| true\nif [ -n \"${{JOURNAL_STREAM:-}}\" ]; then cat {} >&2; fi\nif grep -q 'standalone-e2e-ok'",
            sh_quote(&worker_stderr_path.to_string_lossy())
        ),
    );
    let child_prefix = format!("printf 'http://127.0.0.1:{health_port}\\n' > \"$health_url_file\"");
    let child_logs = format!(
        "{child_prefix}\nprintf '2026-07-25T16:00:00+00:00 INFO child-info\\n'\nprintf '2026-07-25T16:00:00+00:00 WARN child-warning\\n'\nprintf '2026-07-25T16:00:00+00:00 ERROR child-error\\n' >&2\nprintf 'unknown-stdout\\n'\nprintf 'unknown-stderr\\n' >&2"
    );
    script.replace(&child_prefix, &child_logs)
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn start_health_server() -> Result<(u16, Arc<AtomicBool>, thread::JoinHandle<()>), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread = thread::spawn(move || {
        while !thread_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => respond_health(&mut stream),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    Ok((port, stop, thread))
}

fn respond_health(stream: &mut TcpStream) {
    let mut request = [0u8; 1024];
    let _ = stream.read(&mut request);
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    let _ = stream.write_all(response);
}

fn local_socket_path(agent_id: &str) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable")?;
    Ok(PathBuf::from(home)
        .join(".agentic_gpt/runtime/agent")
        .join(agent_id)
        .join("mcp.sock"))
}

fn wait_for_unix_socket(path: &Path, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if fs::symlink_metadata(path)
            .map(|metadata| std::os::unix::fs::FileTypeExt::is_socket(&metadata.file_type()))
            .unwrap_or(false)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(format!(
        "timed out waiting for Unix MCP socket: {}",
        path.display()
    ))
}

fn wait_for_path_absent(path: &Path, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if !path.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(format!(
        "runtime path survived shutdown: {}",
        path.display()
    ))
}

fn stop_child_gracefully(child: &mut Child, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_for_marker(supervisor: &mut Child, marker: &Path) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if marker.exists() {
            return Ok(());
        }
        if let Some(status) = supervisor.try_wait().map_err(|error| error.to_string())? {
            return Err(format!("supervisor exited before tool call: {status}"));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("timed out waiting for supervised MCP tool call".to_string())
}

fn stop_supervisor(supervisor: &mut Child) {
    if supervisor.try_wait().ok().flatten().is_none() {
        unsafe {
            libc::kill(supervisor.id() as i32, libc::SIGINT);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if supervisor.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = supervisor.kill();
    }
    let _ = supervisor.wait();
}
