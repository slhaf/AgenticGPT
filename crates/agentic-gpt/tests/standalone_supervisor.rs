use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[test]
fn hidden_worker_recovers_stale_tunnel_session_before_first_call() {
    let root =
        PathBuf::from("/tmp").join(format!("as-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_stdio_resume(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn hidden_worker_reloads_policy_path_limit_and_mcp_without_restart() {
    let root =
        PathBuf::from("/tmp").join(format!("as-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_live_reload(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn hidden_worker_completes_local_mcp_call() {
    let root =
        PathBuf::from("/tmp").join(format!("as-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_smoke(&root, "normal", false, false);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn hidden_room_worker_completes_local_mcp_call() {
    let root =
        PathBuf::from("/tmp").join(format!("as-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_smoke(&root, "room", true, false);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn hidden_worker_completes_local_mcp_call_with_journal_logging_enabled() {
    let root =
        PathBuf::from("/tmp").join(format!("as-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_smoke(&root, "normal", true, false);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn hidden_worker_keeps_serving_during_invalid_config_reload() {
    let root =
        PathBuf::from("/tmp").join(format!("as-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_smoke(&root, "normal", true, true);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

#[test]
fn killing_tunnel_client_exits_its_worker_and_releases_the_local_socket() {
    let root =
        PathBuf::from("/tmp").join(format!("as-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_orphaned_worker_cleanup(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

fn run_orphaned_worker_cleanup(root: &Path) -> Result<(), String> {
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
    let startup_config_path = root.join("startup-config.json");
    let workspace = root.join("h/.agentic_gpt/workspace");
    let pid_path = root.join("worker.pid");
    let init = fixture_command(&binary, root)
        .args([
            "config",
            "--config",
            config_path.to_str().unwrap(),
            "init",
            "--non-interactive",
        ])
        .output()
        .map_err(|error| format!("config init failed to spawn: {error}"))?;
    if !init.status.success() {
        return Err(format!(
            "config init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        ));
    }

    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    let mut config: Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    let agent_id = short_agent_id(root);
    config["mode"] = Value::String("standalone".to_string());
    config["profile"] = Value::String("normal".to_string());
    config["agentId"] = Value::String(agent_id.clone());
    config["workspaceRoot"] = Value::String("~/.agentic_gpt/workspace".to_string());
    set_fixture_path_policy(&mut config, &[&workspace]);
    config["tunnel"] = json!({
        "tunnelId": "tunnel_orphan_worker_test",
        "apiKey": "env:AGENTIC_TEST_TUNNEL_KEY",
        "hubReporting": { "enabled": false, "detail": "metadata" }
    });
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        &startup_config_path,
        serde_json::to_vec(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::set_permissions(&startup_config_path, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;

    let token = "orphan-worker-test-token";
    let script = concat!(
        "exec 3<&0\n",
        "\"$1\" stdio-worker --config \"$2\" --profile normal --supervisor-token \"$3\" --startup-config \"$4\" <&3 &\n",
        "worker_pid=$!\n",
        "printf '%s\\n' \"$worker_pid\" > \"$5\"\n",
        "wait \"$worker_pid\"\n"
    );
    let mut client = GuardChild(Some(
        fixture_command(Path::new("/bin/sh"), root)
            .args([
                "-c",
                script,
                "tunnel-client-fixture",
                binary.to_str().unwrap(),
                config_path.to_str().unwrap(),
                token,
                startup_config_path.to_str().unwrap(),
                pid_path.to_str().unwrap(),
            ])
            .env("AGENTIC_GPT_SUPERVISOR_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("fake tunnel client failed to spawn: {error}"))?,
    ));
    struct GroupCleanup(Option<u32>);
    impl Drop for GroupCleanup {
        fn drop(&mut self) {
            if let Some(group) = self.0 {
                unsafe { libc::kill(-(group as i32), libc::SIGKILL) };
            }
        }
    }
    let mut group_cleanup = GroupCleanup(Some(client.id()));
    let socket_path = local_socket_path(root, &agent_id)?;
    wait_for_unix_socket(&socket_path, Duration::from_secs(10))?;
    let worker_pid = wait_for_pid_file(&pid_path, Duration::from_secs(5))?;

    if unsafe { libc::kill(client.id() as i32, libc::SIGKILL) } != 0 {
        return Err(format!(
            "failed to kill fake tunnel client: {}",
            std::io::Error::last_os_error()
        ));
    }
    let _ = client.wait();
    wait_for_process_stopped(worker_pid, Duration::from_secs(5))?;
    wait_for_socket_released(&socket_path, Duration::from_secs(5))?;
    group_cleanup.0 = None;
    Ok(())
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
    let workspace = root.join("h/.agentic_gpt/workspace");
    let key_path = root.join("h/tunnel-api-key");
    let agent_id = short_agent_id(root);

    let init = fixture_command(&binary, root)
        .args([
            "config",
            "--config",
            config_path.to_str().unwrap(),
            "init",
            "--non-interactive",
        ])
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
    config["mode"] = Value::String("standalone".to_string());
    config["profile"] = Value::String(profile.to_string());
    let mut toolsets = vec!["agent", "file", "mcp", "process", "skills"];
    if profile == "room" {
        toolsets.push("room.email");
    }
    config["toolsets"]["enabled"] = json!(toolsets);
    config["agentId"] = Value::String(agent_id.clone());
    config["workspaceRoot"] = Value::String("~/.agentic_gpt/workspace".to_string());
    set_fixture_path_policy(&mut config, &[&workspace]);
    config["shell"]["initFile"] = Value::Null;
    config["policy"]["command"]["allow"] = json!(["/usr/bin/printf **"]);
    config["tunnel"] = json!({
        "tunnelId": "tunnel_local_integration",
        "apiKey": "file:~/tunnel-api-key",
        "hubReporting": { "enabled": false, "detail": "metadata" }
    });
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let token = "standalone-smoke-token";
    let mut worker_command = fixture_command(&binary, root);
    worker_command
        .args([
            "stdio-worker",
            "--config",
            config_path.to_str().unwrap(),
            "--profile",
            profile,
            "--supervisor-token",
            token,
        ])
        .env("AGENTIC_GPT_SUPERVISOR_TOKEN", token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if journal_mode {
        worker_command.env("JOURNAL_STREAM", "9:9");
    }
    let mut worker = GuardChild(Some(
        worker_command
            .spawn()
            .map_err(|error| format!("worker failed to spawn: {error}"))?,
    ));
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
                "clientInfo": { "name": "standalone-smoke-test", "version": "1" }
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

    if invalid_config_probe {
        thread::sleep(Duration::from_millis(300));
        fs::write(&config_path, "{ invalid json\n").map_err(|error| error.to_string())?;
        thread::sleep(Duration::from_millis(2600));
    }

    send_line(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    )?;
    let listed = response_for(&mut stdout, 2)?;
    let response = call_tool(
        &mut stdin,
        &mut stdout,
        3,
        "process.exec",
        json!({ "command": "/usr/bin/printf 'standalone-e2e-ok'" }),
    )?;
    let response_text = response.to_string();
    if !response_text.contains("standalone-e2e-ok") {
        return Err(format!(
            "worker response did not contain tool output: {response_text}"
        ));
    }
    let room_tool = "room.email.list";
    if profile == "room" && !listed.to_string().contains(room_tool) {
        return Err(format!(
            "room worker response did not advertise Room tools: {listed}"
        ));
    }
    if profile == "normal" && listed.to_string().contains(room_tool) {
        return Err(format!(
            "Normal worker response advertised a Room tool: {listed}"
        ));
    }
    drop(stdin);
    stop_child_gracefully(&mut worker, Duration::from_secs(5));
    let mut worker_stderr = String::new();
    stderr
        .read_to_string(&mut worker_stderr)
        .map_err(|error| error.to_string())?;
    if journal_mode {
        let line = worker_stderr
            .lines()
            .find(|line| line.contains("mcp_tool; ingress=tunnel:stdio;"))
            .ok_or("journal worker INFO line missing")?;
        if !line.starts_with("INFO mcp_tool;") {
            return Err(format!("unexpected journal worker log: {line}"));
        }
    }
    if invalid_config_probe && worker_stderr.contains("standalone live config reload rejected;") {
        return Err(format!(
            "supervised worker duplicated config warning: {worker_stderr}"
        ));
    }
    Ok(())
}

fn run_stdio_resume(root: &Path) -> Result<(), String> {
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
    let init = fixture_command(&binary, root)
        .args([
            "config",
            "--config",
            config_path.to_str().unwrap(),
            "init",
            "--non-interactive",
        ])
        .output()
        .map_err(|error| format!("config init failed to spawn: {error}"))?;
    if !init.status.success() {
        return Err(format!(
            "config init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        ));
    }

    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    let mut config: Value =
        serde_json::from_str(&fs::read_to_string(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    config["agentId"] = Value::String(short_agent_id(root));
    config["workspaceRoot"] = Value::String(workspace.to_string_lossy().into_owned());
    set_fixture_path_policy(&mut config, &[&workspace]);
    config["tunnel"] = json!({
        "tunnelId": "tunnel_stdio_resume_test",
        "apiKey": "env:AGENTIC_TEST_TUNNEL_KEY",
        "hubReporting": { "enabled": false, "detail": "metadata" }
    });
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    let token = "stdio-resume-token";
    let mut worker = GuardChild(Some(
        fixture_command(&binary, root)
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
            .map_err(|error| format!("worker failed to spawn: {error}"))?,
    ));
    let mut stdin = worker.stdin.take().ok_or("worker stdin unavailable")?;
    let stdout = worker.stdout.take().ok_or("worker stdout unavailable")?;
    let mut stdout = BufReader::new(stdout);
    let mut stderr = worker.stderr.take().ok_or("worker stderr unavailable")?;

    send_line(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }),
    )?;
    send_line(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "tools/call",
            "params": { "name": "agent.info", "arguments": {} }
        }),
    )?;

    let mut first_line = String::new();
    stdout
        .read_line(&mut first_line)
        .map_err(|error| error.to_string())?;
    if first_line.is_empty() {
        let status = worker.try_wait().map_err(|error| error.to_string())?;
        stop_child_gracefully(&mut worker, Duration::from_secs(2));
        let mut logs = String::new();
        stderr
            .read_to_string(&mut logs)
            .map_err(|error| error.to_string())?;
        return Err(format!(
            "worker closed stdout before resumed call response; status={status:?}; stderr={logs}"
        ));
    }
    let first: Value =
        serde_json::from_str(first_line.trim()).map_err(|error| error.to_string())?;
    assert_eq!(first["id"], 0);
    assert_eq!(
        first["result"]["structuredContent"]["identity"]["profile"],
        "normal"
    );
    assert!(!first_line.contains("agentic-gpt-internal-init-"));

    send_line(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )?;
    let follow_up = response_for(&mut stdout, 1)?;
    assert!(follow_up["result"]["tools"].is_array());

    drop(stdin);
    stop_child_gracefully(&mut worker, Duration::from_secs(5));
    let mut logs = String::new();
    stderr
        .read_to_string(&mut logs)
        .map_err(|error| error.to_string())?;
    assert!(logs.contains("mcp_stdio_session_resume;"));
    assert!(logs.contains("mcp_stdio_session_resumed;"));
    assert!(!logs.contains("expect initialized request"));
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
    let init = fixture_command(&binary, root)
        .args([
            "config",
            "--config",
            config_path.to_str().unwrap(),
            "init",
            "--non-interactive",
        ])
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
    let agent_id = short_agent_id(root);
    config["agentId"] = Value::String(agent_id.clone());
    config["workspaceRoot"] = Value::String(workspace.to_string_lossy().into_owned());
    set_fixture_path_policy(&mut config, &[&workspace]);
    config["shell"]["initFile"] = Value::Null;
    set_fixture_path_policy(&mut config, &[&workspace, &path_root]);
    config["tunnel"] = json!({
        "tunnelId": "tunnel_live_reload_test",
        "apiKey": "env:AGENTIC_TEST_TUNNEL_KEY",
        "hubReporting": { "enabled": false, "detail": "metadata" }
    });
    config["policy"]["command"]["allow"] =
        json!(["/usr/bin/printf **", "/bin/sleep **", "/usr/bin/touch **"]);
    config["policy"]["command"]["deny"] = json!(["/usr/bin/printf **"]);
    config["limits"]["maxActiveProcesses"] = json!(1);
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
    let mut worker = GuardChild(Some(
        fixture_command(&binary, root)
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
            .map_err(|error| format!("worker failed to spawn: {error}"))?,
    ));
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

    let socket_path = local_socket_path(root, &agent_id)?;
    wait_for_unix_socket(&socket_path, Duration::from_secs(5))?;
    let local_tools = fixture_command(&binary, root)
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
    assert!(local_tools
        .iter()
        .all(|tool| tool["_meta"]["surface"] == "agent-local"));

    let local_info = fixture_command(&binary, root)
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
            "calls": [],
            "waitSeconds": 0
        }),
    )?;
    assert_eq!(
        tunnel_batch["result"]["structuredContent"]["error"]["code"],
        "mcp_batch_call_count_invalid"
    );
    assert!(
        tunnel_batch["result"]["structuredContent"]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("mcp_batch_call_count_invalid"))
    );
    let tunnel_batch_audit = fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))
        .map_err(|error| error.to_string())?;
    assert!(tunnel_batch_audit.contains("\"tool\":\"mcp.batch\""));
    assert!(tunnel_batch_audit.contains("\"requestSource\":\"tunnel:mcp.batch\""));
    assert!(tunnel_batch_audit.contains("\"outcome\":\"validation_rejected\""));
    assert!(tunnel_batch_audit.contains("\"errorCode\":\"mcp_batch_call_count_invalid\""));
    assert!(!tunnel_batch_audit.contains("\"program\":\"mcp.callTool\""));

    let denied = call_tool(
        &mut stdin,
        &mut stdout,
        2,
        "process.exec",
        json!({ "command": "/usr/bin/printf 'denied'", "waitSeconds": 2 }),
    )?;
    assert_eq!(
        denied["result"]["structuredContent"]["error"]["code"],
        json!("policy_denied")
    );

    config["policy"]["command"]["deny"] = json!([]);
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
        json!({ "command": "/usr/bin/printf 'reloaded'", "waitSeconds": 2 }),
    )?;
    assert_eq!(
        allowed["result"]["structuredContent"]["state"],
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
    config["policy"]["path"]["deny"] = json!([path_root.to_string_lossy().into_owned()]);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let path_denied = call_tool(
        &mut stdin,
        &mut stdout,
        31,
        "process.exec",
        json!({
            "command": format!("/usr/bin/touch {}", sh_quote(&path_target.to_string_lossy())),
            "waitSeconds": 2
        }),
    )?;
    assert_eq!(
        path_denied["result"]["structuredContent"]["error"]["code"],
        json!("path_denied")
    );
    config["policy"]["path"]["deny"] = json!([]);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let path_allowed = call_tool(
        &mut stdin,
        &mut stdout,
        32,
        "process.exec",
        json!({
            "command": format!("/usr/bin/touch {}", sh_quote(&path_target.to_string_lossy())),
            "waitSeconds": 2
        }),
    )?;
    assert_eq!(
        path_allowed["result"]["structuredContent"]["state"],
        json!("completed")
    );
    assert!(path_target.exists());

    config["policy"]["command"]["deny"] = json!(["/usr/bin/printf **"]);
    config["mcpServers"]["primary"]["transport"] = json!("sse");
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let retained = call_tool(
        &mut stdin,
        &mut stdout,
        33,
        "process.exec",
        json!({ "command": "/usr/bin/printf 'last-good'", "waitSeconds": 2 }),
    )?;
    assert_eq!(
        retained["result"]["structuredContent"]["state"],
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

    config["policy"]["command"]["deny"] = json!([]);
    config["limits"]["maxActiveProcesses"] = json!(1);
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
        json!({ "command": "/bin/sleep 5", "waitSeconds": 0 }),
    )?;
    let active_process_id = active["result"]["structuredContent"]["processId"]
        .as_str()
        .ok_or("active process id missing")?
        .to_string();
    let active_state = active["result"]["structuredContent"]["state"]
        .as_str()
        .ok_or("active process state missing")?;
    assert!(matches!(active_state, "starting" | "running"));

    config["limits"]["maxActiveProcesses"] = json!(0);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let rejected = call_tool(
        &mut stdin,
        &mut stdout,
        5,
        "process.exec",
        json!({ "command": "/usr/bin/printf 'blocked'", "waitSeconds": 2 }),
    )?;
    let reason = rejected["result"]["structuredContent"]["error"]["message"]
        .as_str()
        .ok_or("capacity rejection reason missing")?;
    assert!(reason.starts_with("max_active_processes_reached; "));
    assert!(reason.contains("active=1; requested=1; limit=0"));

    config["limits"]["maxActiveProcesses"] = json!(2);
    write_config(&config_path, &config)?;
    thread::sleep(Duration::from_millis(2300));
    let admitted = call_tool(
        &mut stdin,
        &mut stdout,
        6,
        "process.exec",
        json!({ "command": "/usr/bin/printf 'limit-reloaded'", "waitSeconds": 2 }),
    )?;
    assert_eq!(
        admitted["result"]["structuredContent"]["state"],
        json!("completed")
    );

    let _ = call_tool(
        &mut stdin,
        &mut stdout,
        7,
        "process.cancel",
        json!({ "processId": active_process_id }),
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
    assert!(!human_logs.contains("processId="));
    assert!(human_logs.contains("status=active"));
    assert_eq!(human_logs.matches("managed_process;").count(), 1);
    assert!(!human_logs.contains(&active_process_id));
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

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn short_agent_id(root: &Path) -> String {
    let fixed_path_bytes = local_socket_path(root, "x")
        .expect("HOME is required")
        .as_os_str()
        .as_bytes()
        .len()
        - 1;
    let available_id_bytes = 100usize.saturating_sub(fixed_path_bytes);
    assert!(
        available_id_bytes > 0,
        "HOME leaves no room for a local socket agent ID"
    );
    let mut agent_id = Uuid::new_v4().simple().to_string();
    agent_id.truncate(available_id_bytes.min(16));
    agent_id
}

fn local_socket_path(root: &Path, agent_id: &str) -> Result<PathBuf, String> {
    Ok(root
        .join("h")
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

fn wait_for_pid_file(path: &Path, duration: Duration) -> Result<u32, String> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if let Ok(value) = fs::read_to_string(path) {
            if let Ok(pid) = value.trim().parse() {
                return Ok(pid);
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(format!(
        "timed out waiting for worker PID: {}",
        path.display()
    ))
}

fn wait_for_process_stopped(pid: u32, duration: Duration) -> Result<(), String> {
    let process_status = PathBuf::from(format!("/proc/{pid}/stat"));
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let status = match fs::read_to_string(&process_status) {
            Ok(status) => status,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("could not inspect worker process: {error}")),
        };
        let state = status
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next());
        if state == Some("Z") {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(format!("worker process {pid} survived tunnel client exit"))
}

fn wait_for_socket_released(path: &Path, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        match UnixStream::connect(path) {
            Ok(stream) => drop(stream),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) =>
            {
                return Ok(())
            }
            Err(error) => return Err(format!("could not probe worker socket: {error}")),
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err(format!("worker socket remained active: {}", path.display()))
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
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.wait();
}

fn set_fixture_path_policy(config: &mut Value, roots: &[&Path]) {
    let paths = roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    config["policy"]["path"] = json!({
        "defaultAction": "deny",
        "readOnly": [],
        "write": paths,
        "deny": []
    });
    config["policy"]["bubblewrap"] = json!(false);
}

fn fixture_command(binary: &Path, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command.process_group(0);
    for (name, directory) in [
        ("HOME", "h"),
        ("XDG_CONFIG_HOME", "c"),
        ("XDG_DATA_HOME", "d"),
        ("XDG_CACHE_HOME", "k"),
        ("XDG_RUNTIME_DIR", "r"),
        ("TMPDIR", "t"),
    ] {
        let path = root.join(directory);
        fs::create_dir_all(&path).expect("fixture environment directory");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("private fixture environment directory");
        command.env(name, path);
    }
    command.env_remove("TMUX");
    command.env("TMUX_TMPDIR", root.join("r"));
    command.env(
        "DBUS_SESSION_BUS_ADDRESS",
        format!("unix:path={}/no-session-bus", root.display()),
    );
    command.env(
        "DBUS_SYSTEM_BUS_ADDRESS",
        format!("unix:path={}/no-system-bus", root.display()),
    );
    command
}

struct RootCleanup(PathBuf);

impl Drop for RootCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct GuardChild(Option<Child>);

impl std::ops::Deref for GuardChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.0.as_ref().expect("owned child")
    }
}

impl std::ops::DerefMut for GuardChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("owned child")
    }
}

impl Drop for GuardChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            stop_child_gracefully(child, Duration::from_secs(2));
        }
    }
}
