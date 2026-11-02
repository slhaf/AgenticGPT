use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[test]
fn local_runtime_cli_exercises_real_unix_mcp_surface() {
    let root =
        PathBuf::from("/tmp").join(format!("al-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_local_e2e(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

const CLI_TIMEOUT: Duration = Duration::from_secs(5);
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
const RELOAD_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn incomplete_room_toolsets_remain_enabled_and_report_missing_fields_at_call_time() {
    let root =
        PathBuf::from("/tmp").join(format!("al-{}", &Uuid::new_v4().simple().to_string()[..16]));
    fs::create_dir_all(&root).unwrap();
    let _root_cleanup = RootCleanup(root.clone());
    let result = run_incomplete_room_toolsets_e2e(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

fn run_incomplete_room_toolsets_e2e(root: &Path) -> Result<(), String> {
    let binary = binary_path();
    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    let initialized = output_with_timeout(
        command(&binary, root)
            .args(["config", "--config"])
            .arg(&config_path)
            .args([
                "init",
                "--non-interactive",
                "--mode",
                "local",
                "--profile",
                "normal",
            ]),
        CLI_TIMEOUT,
    )?;
    ensure_success(&initialized, "incomplete Room config init")?;

    let agent_id = format!("local-{}", &Uuid::new_v4().simple().to_string()[..16]);
    let mut config: Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    config["agentId"] = json!(agent_id);
    config["mode"] = json!("local");
    config["workspaceRoot"] = json!(workspace);
    config["toolsets"]["enabled"] = json!(["agent", "file", "room.notebook"]);
    config["room"]["repositoryRoot"] = json!(workspace);
    // In particular, this runtime must not start a tmux-owned child process.
    config["shell"]["initFile"] = Value::Null;
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;

    for namespace in ["room.email", "room.secret"] {
        for action in ["enable", "disable", "enable"] {
            let changed = output_with_timeout(
                command(&binary, root)
                    .args(["config", "--config"])
                    .arg(&config_path)
                    .args(["toolset", action, namespace]),
                CLI_TIMEOUT,
            )?;
            ensure_success(&changed, "incomplete Room toolset mutation")?;
            let loaded = output_with_timeout(
                command(&binary, root)
                    .args(["config", "--config"])
                    .arg(&config_path)
                    .arg("show"),
                CLI_TIMEOUT,
            )?;
            ensure_success(&loaded, "incomplete Room config reload")?;
        }
    }

    let socket_path = root
        .join("h")
        .join(".agentic_gpt/runtime/agent")
        .join(&agent_id)
        .join("mcp.sock");
    let mut server = command(&binary, root)
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
    let tools = local_output_with_timeout(&binary, &config_path, ["list-tools"], CALL_TIMEOUT)?;
    let tools: Value = serde_json::from_slice(&tools.stdout).map_err(|error| error.to_string())?;
    let tools = tools.as_array().ok_or("local tool list is not an array")?;
    for name in ["room.email.list", "room.secret.list"] {
        if !tools.iter().any(|tool| tool["name"] == name) {
            return Err(format!("enabled incomplete toolset did not expose {name}"));
        }
    }

    expect_missing_room_config(
        &binary,
        &config_path,
        "room.email.list",
        "email_account_required",
        "email.account",
    )?;
    expect_missing_room_config(
        &binary,
        &config_path,
        "room.secret.list",
        "secret_not_configured",
        "room.secret.serverUrl",
    )?;

    for (key, value, tool, code, missing) in [
        (
            "email.account.imapHost",
            "imap.example.invalid",
            "room.email.list",
            "email_account_required",
            "email.account.smtpHost",
        ),
        (
            "room.secret.serverUrl",
            "https://secrets.example.invalid",
            "room.secret.list",
            "secret_not_configured",
            "room.secret.account",
        ),
    ] {
        let updated = output_with_timeout(
            command(&binary, root)
                .args(["config", "--config"])
                .arg(&config_path)
                .args(["set", key, value]),
            CLI_TIMEOUT,
        )?;
        ensure_success(&updated, "incomplete Room config set")?;
        expect_missing_room_config(&binary, &config_path, tool, code, missing)?;
    }

    let tools = local_output_with_timeout(&binary, &config_path, ["list-tools"], CALL_TIMEOUT)?;
    let tools: Value = serde_json::from_slice(&tools.stdout).map_err(|error| error.to_string())?;
    let tools = tools
        .as_array()
        .ok_or("reloaded local tool list is not an array")?;
    if !["room.email.list", "room.secret.list"]
        .iter()
        .all(|name| tools.iter().any(|tool| tool["name"] == *name))
    {
        return Err("config hot reload hid an enabled incomplete Room tool".to_string());
    }

    expect_local_tool_error(
        &binary,
        &config_path,
        "room.notebook.recent",
        "{}",
        "room_repository_root_must_be_separate",
        "room_repository_root_must_be_separate",
    )?;
    let disabled = output_with_timeout(
        command(&binary, root)
            .args(["config", "--config"])
            .arg(&config_path)
            .args(["toolset", "disable", "room.notebook"]),
        CLI_TIMEOUT,
    )?;
    ensure_success(&disabled, "disable invalid Room repository toolset")?;
    wait_for_tool_presence(&binary, &config_path, "room.notebook.recent", false)?;
    let enabled = output_with_timeout(
        command(&binary, root)
            .args(["config", "--config"])
            .arg(&config_path)
            .args(["toolset", "enable", "room.notebook"]),
        CLI_TIMEOUT,
    )?;
    ensure_success(&enabled, "re-enable invalid Room repository toolset")?;
    wait_for_tool_presence(&binary, &config_path, "room.notebook.recent", true)?;
    expect_local_tool_error(
        &binary,
        &config_path,
        "room.notebook.recent",
        "{}",
        "room_repository_root_must_be_separate",
        "room_repository_root_must_be_separate",
    )?;

    let separate_repository = root.join("separate-repository");
    fs::create_dir(&separate_repository).map_err(|error| error.to_string())?;
    let separate_repository = separate_repository
        .to_str()
        .ok_or("non-UTF8 fixture path")?;
    let updated = output_with_timeout(
        command(&binary, root)
            .args(["config", "--config"])
            .arg(&config_path)
            .args(["set", "room.repositoryRoot", separate_repository]),
        CLI_TIMEOUT,
    )?;
    ensure_success(&updated, "set separate Room repository root")?;
    expect_local_tool_success(&binary, &config_path, "room.notebook.recent")?;

    let updated = output_with_timeout(
        command(&binary, root)
            .args(["config", "--config"])
            .arg(&config_path)
            .args(["set", "browser.runtime", "{}"]),
        CLI_TIMEOUT,
    )?;
    ensure_success(&updated, "set incomplete explicit Browser runtime")?;
    let enabled = output_with_timeout(
        command(&binary, root)
            .args(["config", "--config"])
            .arg(&config_path)
            .args(["toolset", "enable", "browser"]),
        CLI_TIMEOUT,
    )?;
    ensure_success(&enabled, "enable Browser toolset without runtime")?;
    wait_for_tool_presence(&binary, &config_path, "browser.list", true)?;
    expect_browser_list_unavailable(&binary, &config_path)?;
    expect_local_tool_error(
        &binary,
        &config_path,
        "browser.manual",
        r#"{"action":"search","query":"test"}"#,
        "browser_runtime_unavailable",
        "browser_runtime_unavailable",
    )?;

    signal_and_wait(cleanup.child, libc::SIGTERM, Duration::from_secs(5))?;
    Ok(())
}

fn expect_local_tool_error(
    binary: &Path,
    config: &Path,
    tool: &str,
    arguments: &str,
    code: &str,
    message_part: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + RELOAD_TIMEOUT;
    loop {
        let tools = local_output_with_timeout(binary, config, ["list-tools"], CALL_TIMEOUT)?;
        let tools: Value =
            serde_json::from_slice(&tools.stdout).map_err(|error| error.to_string())?;
        if !tools
            .as_array()
            .is_some_and(|tools| tools.iter().any(|entry| entry["name"] == tool))
        {
            return Err(format!("enabled tool {tool} was not discovered"));
        }
        let output = output_with_timeout(
            command(binary, config.parent().expect("fixture config parent"))
                .args(["local", "--config"])
                .arg(config)
                .args(["call", tool, "--arguments", arguments]),
            CALL_TIMEOUT,
        )?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains(code) && stderr.contains(message_part) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("{tool} did not report {code}: {stderr}"));
            }
            thread::sleep(Duration::from_millis(100));
            continue;
        }
        let result: Value =
            serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
        let error = &result["structuredContent"]["error"];
        let actual_code = error["code"].as_str().unwrap_or_default();
        let message = error["message"].as_str().unwrap_or_default();
        if actual_code == code && message.contains(message_part) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{tool} did not report {code} after config reload; last response: code={actual_code}, message={message}"
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn expect_browser_list_unavailable(binary: &Path, config: &Path) -> Result<(), String> {
    let deadline = Instant::now() + RELOAD_TIMEOUT;
    loop {
        let output = local_output_with_timeout(
            binary,
            config,
            ["call", "browser.list", "--arguments", "{}"],
            CALL_TIMEOUT,
        )?;
        let result: Value =
            serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
        if result["structuredContent"]["runtimeAvailable"] == false {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "browser.list did not report runtimeAvailable=false: {}",
                result["structuredContent"]
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_tool_presence(
    binary: &Path,
    config: &Path,
    tool: &str,
    expected: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + RELOAD_TIMEOUT;
    loop {
        let output = local_output_with_timeout(binary, config, ["list-tools"], CALL_TIMEOUT)?;
        let tools: Value =
            serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
        let present = tools
            .as_array()
            .is_some_and(|tools| tools.iter().any(|entry| entry["name"] == tool));
        if present == expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{tool} discovery did not become {expected} after config reload"
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn expect_local_tool_success(binary: &Path, config: &Path, tool: &str) -> Result<(), String> {
    let deadline = Instant::now() + RELOAD_TIMEOUT;
    loop {
        let tools = local_output_with_timeout(binary, config, ["list-tools"], CALL_TIMEOUT)?;
        let tools: Value =
            serde_json::from_slice(&tools.stdout).map_err(|error| error.to_string())?;
        if !tools
            .as_array()
            .is_some_and(|tools| tools.iter().any(|entry| entry["name"] == tool))
        {
            return Err(format!("enabled tool {tool} was not discovered"));
        }
        let output = match local_output_with_timeout(
            binary,
            config,
            ["call", tool, "--arguments", "{}"],
            CALL_TIMEOUT,
        ) {
            Ok(output) => output,
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(error);
                }
                thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        let result: Value =
            serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
        if result["structuredContent"].get("error").is_none()
            && result["structuredContent"]["documents"].is_array()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{tool} did not become usable after config reload: {}",
                result["structuredContent"]
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn expect_missing_room_config(
    binary: &Path,
    config: &Path,
    tool: &str,
    code: &str,
    missing_path: &str,
) -> Result<(), String> {
    expect_local_tool_error(binary, config, tool, "{}", code, missing_path)
}

fn local_output_with_timeout<const N: usize>(
    binary: &Path,
    config: &Path,
    args: [&str; N],
    timeout: Duration,
) -> Result<Output, String> {
    output_with_timeout(
        command(binary, config.parent().expect("fixture config parent"))
            .args(["local", "--config"])
            .arg(config)
            .args(args),
        timeout,
    )
    .and_then(|output| {
        ensure_success(&output, "local CLI")?;
        Ok(output)
    })
}

fn output_with_timeout(command: &mut Command, timeout: Duration) -> Result<Output, String> {
    let capture_root = std::env::temp_dir().join(format!("agentic-cli-{}", Uuid::new_v4()));
    fs::create_dir(&capture_root).map_err(|error| error.to_string())?;
    let _capture_cleanup = RootCleanup(capture_root.clone());
    let stdout_path = capture_root.join("stdout");
    let stderr_path = capture_root.join("stderr");
    let mut child = command
        .stdout(Stdio::from(
            fs::File::create(&stdout_path).map_err(|error| error.to_string())?,
        ))
        .stderr(Stdio::from(
            fs::File::create(&stderr_path).map_err(|error| error.to_string())?,
        ))
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Ok(Output {
                    status,
                    stdout: fs::read(&stdout_path).map_err(|error| error.to_string())?,
                    stderr: fs::read(&stderr_path).map_err(|error| error.to_string())?,
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                // Every fixture command starts its own process group; kill and
                // reap the entire group if a local MCP call gets stuck.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err("fixture command timed out".to_string());
            }
            Err(error) => {
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    }
}

fn run_local_e2e(root: &Path) -> Result<(), String> {
    let binary = binary_path();
    let config_path = root.join("config.json");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
    command(&binary, root)
        .args(["config", "--config"])
        .arg(&config_path)
        .args(["init", "--non-interactive"])
        .output()
        .map_err(|error| error.to_string())?
        .status
        .success()
        .then_some(())
        .ok_or("config init failed")?;

    let agent_id = format!("local-{}", &Uuid::new_v4().simple().to_string()[..16]);
    let mut config: Value =
        serde_json::from_slice(&fs::read(&config_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    config["agentId"] = json!(agent_id);
    config["mode"] = json!("local");
    config["profile"] = json!("normal");
    config["displayName"] = json!("Local E2E");
    config["workspaceRoot"] = json!(format!("{}/", workspace.display()));
    set_fixture_path_policy(&mut config, &[&workspace]);
    let allow = config["policy"]["command"]["allow"]
        .as_array_mut()
        .ok_or("missing allow rules")?;
    allow.extend((0..140).map(|index| json!(format!("agent-info-rule-{index} **"))));
    let allow_count = allow.len();
    config["policy"]["path"]["deny"] = json!((0..130)
        .map(|index| workspace.join(format!("agent-info-private-{index}")))
        .collect::<Vec<_>>());
    config["limits"]["maxFileSearchContextLines"] = json!(7);
    config["shell"]["initFile"] = Value::Null;
    config["tunnel"] = Value::Null;
    config["policy"]["command"]["deny"] = json!(["/usr/bin/printf **"]);
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

    let socket_path = root
        .join("h")
        .join(".agentic_gpt/runtime/agent")
        .join(&agent_id)
        .join("mcp.sock");
    let mut server = command(&binary, root)
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
    if !tools.iter().any(|tool| tool["name"] == "agent.info")
        || tools.iter().any(|tool| tool["name"] == removed_tool)
        || tools
            .iter()
            .any(|tool| tool["_meta"]["surface"] != "agent-local")
        || tools.iter().any(|tool| tool["name"] == "room.diary.active")
        || tools
            .iter()
            .any(|tool| tool["name"] == "room.maintenance.submit")
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
    if info["workspaceRoot"] != workspace.to_string_lossy().as_ref()
        || info["policy"]["command"]["allowCount"] != allow_count
        || info["policy"]["path"]["writeCount"] != 1
        || info["policy"]["path"]["readOnlyCount"] != 0
        || info["policy"]["path"]["denyCount"] != 130
        || info["execution"]["fileSearchMaxContextLines"] != 7
        || info["secret"].get("syncStatus") != Some(&Value::Null)
        || info["secret"].get("syncLastTriggeredAt") != Some(&Value::Null)
        || info.to_string().contains("agent-info-rule-")
        || info.to_string().contains("agent-info-private-")
    {
        return Err("agent.info summary mismatch".to_string());
    }

    let mut ranged = GuardChild(Some(
        command(&binary, root)
            .args(["local", "--config"])
            .arg(&config_path)
            .args(["call", "file.read", "--arguments-file", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?,
    ));
    ranged
        .stdin
        .as_mut()
        .ok_or("missing local CLI stdin")?
        .write_all(br#"{"path":"ranged.txt","startLine":2,"endLine":3}"#)
        .map_err(|error| error.to_string())?;
    let ranged = ranged
        .0
        .take()
        .expect("owned CLI child")
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    ensure_success(&ranged, "ranged local call")?;
    let ranged: Value =
        serde_json::from_slice(&ranged.stdout).map_err(|error| error.to_string())?;
    let ranged = &ranged["structuredContent"];
    if ranged["content"] != "two\nthree\n" || ranged.get("metadata").is_some() {
        return Err("ranged local read mismatch".to_string());
    }

    let batch_rejected = local_output(
        &binary,
        &config_path,
        [
            "call",
            "mcp.batch",
            "--arguments",
            r#"{"calls":[],"waitSeconds":0}"#,
        ],
    )?;
    let batch_rejected: Value =
        serde_json::from_slice(&batch_rejected.stdout).map_err(|error| error.to_string())?;
    if batch_rejected["structuredContent"]["error"]["code"] != "mcp_batch_call_count_invalid"
        || batch_rejected["structuredContent"]["error"]["message"]
            .as_str()
            .is_none_or(|message| !message.starts_with("mcp_batch_call_count_invalid"))
    {
        return Err("local mcp.batch typed preflight rejection mismatch".to_string());
    }
    let batch_audit = fs::read_to_string(workspace.join(".agentic-gpt-audit.jsonl"))
        .map_err(|error| error.to_string())?;
    if !batch_audit.contains("\"tool\":\"mcp.batch\"")
        || !batch_audit.contains("\"requestSource\":\"local:mcp.batch\"")
        || !batch_audit.contains("\"outcome\":\"validation_rejected\"")
        || !batch_audit.contains("\"errorCode\":\"mcp_batch_call_count_invalid\"")
        || batch_audit.contains("\"program\":\"mcp.callTool\"")
    {
        return Err("local mcp.batch aggregate audit mismatch".to_string());
    }

    config["toolsets"]["enabled"] = json!([
        "agent",
        "file",
        "mcp",
        "process",
        "skills",
        "tmux",
        "room.bootstrap",
        "room.diary",
        "room.notebook",
        "room.state",
        "room.maintenance"
    ]);
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_millis(2300));
    let enabled_tools = local_output(&binary, &config_path, ["list-tools"])?;
    if !enabled_tools.stderr.is_empty() {
        return Err("enabled Room list-tools wrote to stderr".to_string());
    }
    let enabled_tools: Value =
        serde_json::from_slice(&enabled_tools.stdout).map_err(|error| error.to_string())?;
    let enabled_tools = enabled_tools
        .as_array()
        .ok_or("enabled Room tool list is not an array")?;
    if !enabled_tools
        .iter()
        .any(|tool| tool["name"] == "room.diary.active")
        || !enabled_tools
            .iter()
            .any(|tool| tool["name"] == "room.maintenance.submit")
    {
        return Err("enabled Room tool surface mismatch".to_string());
    }

    let active = local_output(
        &binary,
        &config_path,
        ["call", "room.diary.active", "--arguments", "{}"],
    )?;
    let active: Value =
        serde_json::from_slice(&active.stdout).map_err(|error| error.to_string())?;
    if active["structuredContent"].get("daily").is_none()
        || active["structuredContent"].get("weekly").is_none()
        || active["structuredContent"].get("monthly").is_none()
    {
        return Err("enabled Room call did not reach live server".to_string());
    }

    config["toolsets"]["enabled"] = json!(["agent", "file", "mcp", "process", "skills", "tmux"]);
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    thread::sleep(Duration::from_millis(2300));
    let disabled_tools = local_output(&binary, &config_path, ["list-tools"])?;
    if !disabled_tools.stderr.is_empty() {
        return Err("disabled Room list-tools wrote to stderr".to_string());
    }
    let disabled_tools: Value =
        serde_json::from_slice(&disabled_tools.stdout).map_err(|error| error.to_string())?;
    let disabled_tools = disabled_tools
        .as_array()
        .ok_or("disabled Room tool list is not an array")?;
    if disabled_tools
        .iter()
        .any(|tool| tool["name"] == "room.diary.active")
        || disabled_tools
            .iter()
            .any(|tool| tool["name"] == "room.maintenance.submit")
    {
        return Err("disabled Room tool surface mismatch".to_string());
    }
    let rejected_room_call = local_output_raw(
        &binary,
        &config_path,
        ["call", "room.diary.active", "--arguments", "{}"],
    )?;
    let rejected_room_stderr = String::from_utf8_lossy(&rejected_room_call.stderr);
    if rejected_room_call.status.success()
        || !rejected_room_stderr.contains("room.diary.active")
        || !rejected_room_stderr.contains("not available")
    {
        return Err("disabled Room call was not rejected".to_string());
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
            r#"{"command":"/usr/bin/printf 'guarded'","waitSeconds":1}"#,
        ],
    )?;
    let guarded: Value =
        serde_json::from_slice(&guarded.stdout).map_err(|error| error.to_string())?;
    if guarded["structuredContent"]["state"] != "rejected"
        || guarded["structuredContent"]["error"]["code"] != "policy_denied"
    {
        return Err("guarded local process was not rejected".to_string());
    }

    let second = command(&binary, root)
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
    command(binary, config.parent().expect("fixture config parent"))
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
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
    Err("local runtime did not stop after shutdown signal".to_string())
}

fn command(binary: &Path, root: &Path) -> Command {
    let mut command = fixture_command(binary, root);
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

struct LocalCleanup<'a> {
    child: &'a mut Child,
    socket_root: PathBuf,
}

impl Drop for LocalCleanup<'_> {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            // SAFETY: child.id() is the process owned by this guard.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.socket_root);
    }
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
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
