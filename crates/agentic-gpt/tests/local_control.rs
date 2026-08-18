use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[test]
fn local_runtime_cli_exercises_real_unix_mcp_surface() {
    let root = std::env::temp_dir().join(format!("agentic-local-control-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let result = run_local_e2e(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

fn run_local_e2e(root: &Path) -> Result<(), String> {
    let binary = binary_path();
    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    command(&binary)
        .args(["config", "--config"])
        .arg(&config_path)
        .args(["init", "--non-interactive"])
        .output()
        .map_err(|error| error.to_string())?
        .status
        .success()
        .then_some(())
        .ok_or("config init failed")?;

    let agent_id = format!("local-e2e-{}", Uuid::new_v4().simple());
    let mut config: Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    config["agentId"] = json!(agent_id);
    config["mode"] = json!("local");
    config["profile"] = json!("normal");
    config["displayName"] = json!("Local E2E");
    config["workspaceRoot"] = json!(workspace);
    config["tunnel"] = Value::Null;
    config["policy"]["deny"] = json!([{
        "program": "/usr/bin/printf",
        "argsPrefix": []
    }]);
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
    fs::write(workspace.join("ranged.txt"), "one\ntwo\nthree\nfour\n")
        .map_err(|error| error.to_string())?;

    let socket_path = home_dir()
        .join(".agentic_gpt/runtime/agent")
        .join(&agent_id)
        .join("mcp.sock");
    let mut server = command(&binary)
        .args(["run", "--config"])
        .arg(&config_path)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let cleanup = LocalCleanup {
        child: &mut server,
        socket_root: socket_path.parent().unwrap().to_path_buf(),
    };

    wait_for_socket(&socket_path, Duration::from_secs(5))?;
    let directory_mode = fs::metadata(socket_path.parent().unwrap())
        .map_err(|error| error.to_string())?
        .permissions()
        .mode()
        & 0o777;
    let socket_metadata = fs::symlink_metadata(&socket_path).map_err(|error| error.to_string())?;
    if directory_mode != 0o700
        || socket_metadata.permissions().mode() & 0o777 != 0o600
        || !socket_metadata.file_type().is_socket()
    {
        return Err("local socket permissions/type mismatch".to_string());
    }

    let tools = local_output(&binary, &config_path, ["list-tools"])?;
    if !tools.stderr.is_empty() {
        return Err("local list-tools wrote to stderr".to_string());
    }
    let tools: Value = serde_json::from_slice(&tools.stdout).map_err(|error| error.to_string())?;
    let tools = tools.as_array().ok_or("tool list is not an array")?;
    let removed_tool = ["file", "batch"].join(".");
    if tools.len() != 23
        || !tools.iter().any(|tool| tool["name"] == "agent.info")
        || tools.iter().any(|tool| tool["name"] == removed_tool)
        || tools
            .iter()
            .any(|tool| tool["_meta"]["surface"] != "agent-local")
    {
        return Err("unexpected local tool surface".to_string());
    }

    let info = local_output(
        &binary,
        &config_path,
        ["call", "agent.info", "--arguments", "{}"],
    )?;
    let info: Value = serde_json::from_slice(&info.stdout).map_err(|error| error.to_string())?;
    let info = &info["structuredContent"];
    if info["identity"]["transport"] != "local-unix"
        || info["identity"]["profile"] != "normal"
        || info["connections"]["localMcp"]["status"] != "ready"
        || info["connections"]["localMcp"]["path"]
            .as_str()
            .is_none_or(|path| !path.ends_with("/mcp.sock"))
        || info["connections"]["hubReporting"]["status"] != "disabled"
    {
        return Err("agent.info local readiness mismatch".to_string());
    }

    let mut ranged = command(&binary)
        .args(["local", "--config"])
        .arg(&config_path)
        .args(["call", "file.read", "--arguments-file", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    ranged
        .stdin
        .as_mut()
        .ok_or("missing local CLI stdin")?
        .write_all(br#"{"path":"ranged.txt","includeContent":true,"startLine":2,"endLine":3}"#)
        .map_err(|error| error.to_string())?;
    let ranged = ranged
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    ensure_success(&ranged, "ranged local call")?;
    let ranged: Value =
        serde_json::from_slice(&ranged.stdout).map_err(|error| error.to_string())?;
    let ranged = &ranged["structuredContent"];
    if ranged["content"] != "two\nthree\n"
        || ranged["startLine"] != 2
        || ranged["returnedThroughLine"] != 3
    {
        return Err("ranged local read mismatch".to_string());
    }

    let batch_rejected = local_output(
        &binary,
        &config_path,
        [
            "call",
            "mcp.batch",
            "--arguments",
            r#"{"calls":[{"id":"dup","serverId":"primary","toolName":"fake.tool","arguments":{}},{"id":"dup","serverId":"primary","toolName":"fake.tool","arguments":{}}],"waitSeconds":0}"#,
        ],
    )?;
    let batch_rejected: Value =
        serde_json::from_slice(&batch_rejected.stdout).map_err(|error| error.to_string())?;
    if batch_rejected["structuredContent"]["error"]["code"] != "mcp_batch_failed"
        || batch_rejected["structuredContent"]["error"]["message"]
            .as_str()
            .is_none_or(|message| !message.starts_with("mcp_batch_call_id_duplicate"))
    {
        return Err("local mcp.batch typed preflight rejection mismatch".to_string());
    }
    let batch_audit = fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))
        .map_err(|error| error.to_string())?;
    if !batch_audit.contains("\"tool\":\"mcp.batch\"")
        || !batch_audit.contains("\"requestSource\":\"local:mcp.batch\"")
        || !batch_audit.contains("\"outcome\":\"validation_rejected\"")
        || !batch_audit.contains("\"errorCode\":\"mcp_batch_call_id_duplicate\"")
        || batch_audit.contains("\"program\":\"mcp.callTool\"")
    {
        return Err("local mcp.batch aggregate audit mismatch".to_string());
    }

    config["mcpServers"]["primary"]["url"] = json!("https://new.example/mcp");
    config["mcpServers"]["local"] = json!({
        "enabled": false,
        "transport": "stdio",
        "url": "node ./local.mjs"
    });
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_millis(2300));
    let reloaded = local_output(
        &binary,
        &config_path,
        ["call", "mcp.list", "--arguments", "{}"],
    )?;
    let reloaded: Value =
        serde_json::from_slice(&reloaded.stdout).map_err(|error| error.to_string())?;
    let servers = reloaded["structuredContent"]["servers"]
        .as_array()
        .ok_or("local reloaded server list missing")?;
    if servers.len() != 2
        || !servers.iter().any(|server| {
            server["id"] == "primary"
                && server["url"] == "https://new.example/mcp"
                && server["enabled"] == true
        })
        || !servers.iter().any(|server| {
            server["id"] == "local" && server["transport"] == "stdio" && server["enabled"] == false
        })
    {
        return Err("local runtime MCP hot reload mismatch".to_string());
    }

    let guarded = local_output(
        &binary,
        &config_path,
        [
            "call",
            "process.exec",
            "--arguments",
            r#"{"program":"/usr/bin/printf","args":["guarded"],"waitSeconds":1}"#,
        ],
    )?;
    let guarded: Value =
        serde_json::from_slice(&guarded.stdout).map_err(|error| error.to_string())?;
    if guarded["structuredContent"]["job"]["state"] != "rejected"
        || guarded["structuredContent"]["job"]["rejectReason"] != "policy_denied"
    {
        return Err("guarded local process was not rejected".to_string());
    }

    let second = command(&binary)
        .args(["run", "--config"])
        .arg(&config_path)
        .output()
        .map_err(|error| error.to_string())?;
    if second.status.success()
        || !String::from_utf8_lossy(&second.stderr).contains("already running")
    {
        return Err("second local runtime was not rejected by the run lock".to_string());
    }

    signal_and_wait(cleanup.child, libc::SIGTERM, Duration::from_secs(5))?;
    if socket_path.exists() {
        return Err("local socket survived clean shutdown".to_string());
    }
    let stderr = cleanup
        .child
        .stderr
        .take()
        .map(|mut stderr| {
            let mut value = String::new();
            let _ = std::io::Read::read_to_string(&mut stderr, &mut value);
            value
        })
        .unwrap_or_default();
    if !stderr.contains("ingress=local:unix") || !stderr.contains("errorCode=policy_denied") {
        return Err("local ingress source/policy audit log missing".to_string());
    }

    let unavailable = local_output_raw(&binary, &config_path, ["list-tools"])?;
    if unavailable.status.success()
        || !String::from_utf8_lossy(&unavailable.stderr).contains("local_mcp_unavailable")
    {
        return Err("stopped local runtime did not report typed unavailability".to_string());
    }
    Ok(())
}

fn local_output<const N: usize>(
    binary: &Path,
    config: &Path,
    args: [&str; N],
) -> Result<Output, String> {
    let output = local_output_raw(binary, config, args)?;
    ensure_success(&output, "local CLI")?;
    Ok(output)
}

fn local_output_raw<const N: usize>(
    binary: &Path,
    config: &Path,
    args: [&str; N],
) -> Result<Output, String> {
    command(binary)
        .args(["local", "--config"])
        .arg(config)
        .args(args)
        .output()
        .map_err(|error| error.to_string())
}

fn ensure_success(output: &Output, label: &str) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn wait_for_socket(path: &Path, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("local socket did not become ready".to_string())
}

fn signal_and_wait(child: &mut Child, signal: i32, duration: Duration) -> Result<(), String> {
    // SAFETY: child.id() is a live process id owned by this test.
    unsafe {
        libc::kill(child.id() as i32, signal);
    }
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("local runtime did not stop after shutdown signal".to_string())
}

fn command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command.env_remove("AGENTIC_SUPERVISOR_TOKEN");
    command
}

fn binary_path() -> PathBuf {
    std::env::var("CARGO_BIN_EXE_agentic-gpt")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_agentic_gpt"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/agentic-gpt")
        })
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME is required for local control E2E")
}

struct LocalCleanup<'a> {
    child: &'a mut Child,
    socket_root: PathBuf,
}

impl Drop for LocalCleanup<'_> {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            // SAFETY: child.id() is the process owned by this guard.
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
        let _ = fs::remove_dir_all(&self.socket_root);
    }
}
