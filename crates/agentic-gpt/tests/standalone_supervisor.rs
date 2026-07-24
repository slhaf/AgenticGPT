use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
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
fn supervisor_launches_real_worker_and_completes_local_mcp_call() {
    let root = std::env::temp_dir().join(format!("agentic-standalone-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let result = run_smoke(&root);
    let _ = fs::remove_dir_all(&root);
    result.unwrap();
}

fn run_smoke(root: &Path) -> Result<(), String> {
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
        &agent_id,
        &response_path,
        &worker_stderr_path,
        &marker_path,
    );
    fs::write(&fake_tunnel, script).map_err(|error| error.to_string())?;
    fs::set_permissions(&fake_tunnel, fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;

    let mut supervisor = Command::new(&binary)
        .args([
            "run-as-standalone",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("supervisor failed to spawn: {error}"))?;

    let completed = wait_for_marker(&mut supervisor, &marker_path);
    stop_supervisor(&mut supervisor);
    stop_health.store(true, Ordering::Release);
    let _ = health_thread.join();

    if let Err(error) = completed {
        let response = read_optional(&response_path);
        let worker_stderr = read_optional(&worker_stderr_path);
        let supervisor_stderr = supervisor
            .stderr
            .take()
            .map(|mut stream| {
                let mut value = String::new();
                let _ = stream.read_to_string(&mut value);
                value
            })
            .unwrap_or_default();
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
    Ok(())
}

fn read_optional(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("<unavailable: {error}>"))
}

fn fake_tunnel_script(
    health_port: u16,
    agent_id: &str,
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
    let call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "process.exec",
            "arguments": {
                "agentId": agent_id,
                "program": "/usr/bin/printf",
                "args": ["standalone-e2e-ok"]
            }
        }
    });
    format!(
        "#!/bin/sh\nset -eu\nmode=\"$1\"\nhealth_url_file=\"\"\nmcp_binding=\"\"\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"--health.url-file\" ]; then shift; health_url_file=\"$1\"; fi\n  if [ \"$1\" = \"--mcp.command\" ]; then shift; mcp_binding=\"$1\"; fi\n  shift\ndone\nworker=\"${{mcp_binding#*command=}}\"\nfirst=\"$(printf '%.1s' \"$worker\")\"\nif [ \"$first\" = '\"' ] || [ \"$first\" = \"'\" ]; then printf 'invalid whole-command quoting\\n' >&2; exit 23; fi\nif [ \"$mode\" = \"doctor\" ]; then exit 0; fi\nprintf 'http://127.0.0.1:{health_port}\\n' > \"$health_url_file\"\nrequest_initialize={initialize}\nrequest_initialized={initialized}\nrequest_call={call}\nprintf '%s\\n%s\\n%s\\n' \"$request_initialize\" \"$request_initialized\" \"$request_call\" | sh -c \"$worker\" > {response} 2> {worker_stderr} || true\nif grep -q 'standalone-e2e-ok' {response}; then touch {marker}; fi\nsleep 60\n",
        health_port = health_port,
        initialize = sh_quote(&initialize.to_string()),
        initialized = sh_quote(&initialized.to_string()),
        call = sh_quote(&call.to_string()),
        response = sh_quote(&response_path.to_string_lossy()),
        worker_stderr = sh_quote(&worker_stderr_path.to_string_lossy()),
        marker = sh_quote(&marker_path.to_string_lossy()),
    )
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
