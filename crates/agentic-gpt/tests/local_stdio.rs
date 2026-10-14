use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn two_stdio_bridges_share_agent_and_keep_same_request_ids_isolated() {
    let fixture = Fixture::new();
    let (mut agent, socket_path) = fixture.start_agent();
    let mut first = fixture.start_bridge();
    let mut second = fixture.start_bridge();

    first.send_initialize("first-client");
    second.send_initialize("second-client");
    first.finish_initialize();
    second.finish_initialize();
    first.send_request(2, "tools/list", json!({}));
    second.send_request(2, "tools/list", json!({}));
    let first_tools = first.receive_response(2);
    let second_tools = second.receive_response(2);
    assert!(tool_names(&first_tools).contains(&"agent.info"));
    assert!(tool_names(&second_tools).contains(&"agent.info"));

    // Identical request ids on separate bridge connections must remain scoped
    // to their own MCP session, even when the response shapes differ.
    first.send_request(7, "tools/list", json!({}));
    second.send_request(7, "tools/call", json!({"name":"agent.info","arguments":{}}));
    let first_result = first.receive_response(7);
    let second_result = second.receive_response(7);
    assert!(first_result["result"]["tools"].is_array());
    let second_info = tool_result(&second_result);
    let agent_id = second_info["identity"]["agentId"]
        .as_str()
        .expect("agent.info exposes the live agent identity");
    assert_eq!(agent_id, fixture.agent_id);

    first.send_request(8, "tools/call", json!({"name":"agent.info","arguments":{}}));
    second.send_request(8, "tools/call", json!({"name":"agent.info","arguments":{}}));
    let first_info = first.receive_response(8);
    let second_info = second.receive_response(8);
    assert_eq!(tool_result(&first_info)["identity"]["agentId"], agent_id);
    assert_eq!(tool_result(&second_info)["identity"]["agentId"], agent_id);

    let process = first.request(
        10,
        "tools/call",
        json!({
            "name":"process.exec",
            "arguments":{"command":"/usr/bin/printf 'bridge-shared-output'","waitSeconds":2}
        }),
    );
    let process = tool_result(&process);
    let process_id = process["processId"]
        .as_str()
        .expect("process.exec returns the Agent-owned process id")
        .to_string();
    assert_eq!(process["state"], "completed");

    let read = second.request(
        11,
        "tools/call",
        json!({"name":"process.read","arguments":{"processId":process_id}}),
    );
    let read = tool_result(&read);
    assert_eq!(read["processId"], process_id);
    assert_eq!(read["output"]["stdout"]["data"], "bridge-shared-output");
    let audit = fs::read_to_string(fixture.workspace_path().join(".agentic-gpt-audit.jsonl"))
        .expect("local process.exec writes an audit record");
    assert!(audit.contains("\"requestSource\":\"local:process.exec\""));

    first.close_stdin();
    first.wait(WAIT);
    assert!(agent.child_mut().try_wait().unwrap().is_none());

    let still_live = second.request(9, "tools/call", json!({"name":"agent.info","arguments":{}}));
    assert_eq!(tool_result(&still_live)["identity"]["agentId"], agent_id);
    assert!(fs::symlink_metadata(&socket_path)
        .unwrap()
        .file_type()
        .is_socket());
}

#[test]
fn stdin_eof_without_initialize_exits_bridge_and_leaves_agent_running() {
    let fixture = Fixture::new();
    let (mut agent, _) = fixture.start_agent();
    let mut bridge = fixture.start_bridge();

    bridge.close_stdin();
    let status = bridge.wait(WAIT);
    assert!(status.success(), "bridge exited with {status}");
    assert!(agent.child_mut().try_wait().unwrap().is_none());
}

#[test]
fn daemon_disconnect_exits_bridge_while_client_stdin_remains_open() {
    let fixture = Fixture::new();
    let (mut agent, _) = fixture.start_agent();
    let mut bridge = fixture.start_bridge();
    bridge.initialize("disconnect-client");

    agent.terminate(libc::SIGTERM, WAIT);
    bridge.wait(WAIT);
}

#[test]
fn missing_daemon_reports_error_without_stdout_or_runtime_creation() {
    let fixture = Fixture::new();
    let mut command = fixture.command();
    command
        .args(["local", "stdio", "--config"])
        .arg(&fixture.config_path);
    let output = output_with_timeout(&mut command, WAIT);

    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "stdout must contain MCP bytes only"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("local_mcp_unavailable"));
    assert!(!fixture.agent_runtime_root().exists());
}

#[test]
fn bridge_forwards_fragmented_unix_bytes_without_preinitializing_or_reframing() {
    let fixture = Fixture::new();
    let socket_path = fixture.install_fake_socket();
    let listener = UnixListener::bind(&socket_path).expect("bind fake local peer");
    listener
        .set_nonblocking(true)
        .expect("set fake peer listener nonblocking");
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)).unwrap();

    let client_bytes = [
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"raw\",\"version\":\"1\"}}}\n".as_slice(),
        b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"padding\":\"{}\"}}\n",
            "x".repeat(96 * 1024)
        )
        .as_bytes(),
    ]
    .concat();
    let response_bytes = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n";
    let expected_client = client_bytes.clone();
    let expected_response = response_bytes.to_vec();
    let peer = JoinOnDrop::new(thread::spawn(move || {
        let deadline = Instant::now() + WAIT;
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "bridge did not connect to fake peer"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept bridge connection: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(WAIT))
            .expect("set peer read timeout");
        let mut received = vec![0; expected_client.len()];
        stream
            .read_exact(&mut received)
            .expect("read full client byte stream");
        assert_eq!(received, expected_client);
        for fragment in expected_response.chunks(11) {
            stream
                .write_all(fragment)
                .expect("write fragmented response");
            thread::sleep(Duration::from_millis(2));
        }
    }));

    let mut bridge = fixture.start_bridge();
    bridge
        .stdin
        .as_mut()
        .expect("bridge stdin remains open")
        .write_all(&client_bytes)
        .expect("send complete client stream");
    bridge
        .stdin
        .as_mut()
        .expect("bridge stdin remains open")
        .flush()
        .expect("flush client stream");

    let mut received = Vec::new();
    while received.len() < response_bytes.len() {
        received.extend_from_slice(&bridge.receive_bytes(WAIT));
    }
    assert_eq!(received, response_bytes);
    peer.join().expect("fake Unix peer completed");
    bridge.wait(WAIT);
}

fn tool_names(response: &Value) -> Vec<&str> {
    response["result"]["tools"]
        .as_array()
        .expect("tools/list result")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect()
}

fn tool_result(response: &Value) -> Value {
    response["result"]["structuredContent"].clone()
}

struct Fixture {
    root: PathBuf,
    config_path: PathBuf,
    agent_id: String,
}

impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from("/tmp")
            .join(format!("ls-{}", &Uuid::new_v4().simple().to_string()[..16]));
        fs::create_dir_all(&root).expect("create isolated fixture root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("make fixture root private");
        let fixture = Self {
            config_path: root.join("config.json"),
            agent_id: format!("s-{}", &Uuid::new_v4().simple().to_string()[..8]),
            root,
        };
        fixture.initialize_config();
        fixture
    }

    fn initialize_config(&self) {
        let mut command = self.command();
        command
            .args(["config", "--config"])
            .arg(&self.config_path)
            .args(["init", "--non-interactive"]);
        let output = output_with_timeout(&mut command, WAIT);
        assert!(
            output.status.success(),
            "config init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut config: Value =
            serde_json::from_slice(&fs::read(&self.config_path).expect("read isolated config"))
                .expect("parse isolated config");
        config["agentId"] = json!(self.agent_id);
        config["mode"] = json!("local");
        config["profile"] = json!("normal");
        config["displayName"] = json!("Local stdio test");
        config["workspaceRoot"] = json!(self.root.join("workspace"));
        config["shell"]["initFile"] = Value::Null;
        config["tunnel"] = Value::Null;
        config["policy"]["command"]["allow"] = json!(["/usr/bin/printf **"]);
        config["toolsets"]["enabled"] = json!(["agent", "process"]);
        fs::create_dir_all(self.root.join("workspace")).expect("create workspace");
        fs::write(
            &self.config_path,
            serde_json::to_vec_pretty(&config).expect("serialize isolated config"),
        )
        .expect("write isolated config");
    }

    fn start_agent(&self) -> (ChildGuard, PathBuf) {
        let socket_path = self.agent_socket_path();
        let mut child = self
            .command()
            .args(["run", "--config"])
            .arg(&self.config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start local Agent");
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            if fs::symlink_metadata(&socket_path)
                .map(|metadata| metadata.file_type().is_socket())
                .unwrap_or(false)
            {
                return (ChildGuard::new(child), socket_path);
            }
            if let Some(status) = child.try_wait().expect("check Agent process") {
                panic!("local Agent exited before socket startup: {status}");
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("local Agent socket did not become ready");
    }

    fn start_bridge(&self) -> BridgeClient {
        let mut command = self.command();
        command
            .args(["local", "stdio", "--config"])
            .arg(&self.config_path);
        BridgeClient::spawn(command)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agentic-gpt"));
        command.process_group(0);
        for (name, subdir) in [
            ("HOME", "h"),
            ("XDG_CONFIG_HOME", "xdg-config"),
            ("XDG_DATA_HOME", "xdg-data"),
            ("XDG_CACHE_HOME", "xdg-cache"),
            ("XDG_RUNTIME_DIR", "xdg-runtime"),
            ("TMPDIR", "tmp"),
        ] {
            let path = self.root.join(subdir);
            fs::create_dir_all(&path).expect("create isolated environment directory");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("make isolated environment directory private");
            command.env(name, path);
        }
        command.env_remove("DBUS_SESSION_BUS_ADDRESS");
        let dbus = self.root.join("dbus");
        fs::create_dir_all(&dbus).expect("create isolated D-Bus directory");
        fs::set_permissions(&dbus, fs::Permissions::from_mode(0o700))
            .expect("make isolated D-Bus directory private");
        command.env(
            "DBUS_SESSION_BUS_ADDRESS",
            format!("unix:path={}", dbus.join("bus").display()),
        );
        command.env_remove("AGENTIC_GPT_SUPERVISOR_TOKEN");
        command.env_remove("TMUX");
        command
    }

    fn agent_runtime_root(&self) -> PathBuf {
        self.root
            .join("h/.agentic_gpt/runtime/agent")
            .join(&self.agent_id)
    }

    fn workspace_path(&self) -> PathBuf {
        self.root.join("workspace")
    }

    fn agent_socket_path(&self) -> PathBuf {
        self.agent_runtime_root().join("mcp.sock")
    }

    fn install_fake_socket(&self) -> PathBuf {
        let path = self.agent_socket_path();
        fs::create_dir_all(path.parent().expect("socket parent"))
            .expect("create private socket parent");
        fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700))
            .expect("set private socket parent permissions");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child is owned")
    }

    fn wait(&mut self, duration: Duration) -> ExitStatus {
        wait_child(self.child_mut(), duration)
    }

    fn terminate(&mut self, signal: i32, duration: Duration) {
        let child = self.child_mut();
        // SAFETY: this guard owns the process id passed to kill.
        unsafe {
            libc::kill(child.id() as i32, signal);
        }
        self.wait(duration);
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = &mut self.child else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            // SAFETY: each child starts in a new process group owned by this fixture.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
        }
        let _ = child.wait();
    }
}

struct BridgeClient {
    child: ChildGuard,
    stdin: Option<ChildStdin>,
    stdout: Receiver<Vec<u8>>,
    buffered: Vec<u8>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl BridgeClient {
    fn spawn(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start local stdio bridge");
        let stdout = child.stdout.take().expect("bridge stdout");
        let (sender, receiver) = mpsc::channel();
        let stdout_thread = thread::spawn(move || forward_stdout(stdout, sender));
        let stderr = child.stderr.take().expect("bridge stderr");
        let stderr_capture = Arc::new(Mutex::new(Vec::new()));
        let stderr_thread_capture = stderr_capture.clone();
        let stderr_thread = thread::spawn(move || {
            let mut reader = stderr;
            let mut buffer = [0; 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => stderr_thread_capture
                        .lock()
                        .expect("stderr capture lock")
                        .extend_from_slice(&buffer[..read]),
                }
            }
        });
        let stdin = child.stdin.take();
        Self {
            child: ChildGuard::new(child),
            stdin,
            stdout: receiver,
            buffered: Vec::new(),
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            stderr: stderr_capture,
        }
    }

    fn initialize(&mut self, name: &str) {
        self.send_initialize(name);
        self.finish_initialize();
    }

    fn send_initialize(&mut self, name: &str) {
        self.send_request(
            1,
            "initialize",
            json!({
                "protocolVersion":"2025-06-18",
                "capabilities":{},
                "clientInfo":{"name":name,"version":"1"}
            }),
        );
    }

    fn finish_initialize(&mut self) {
        let response = self.receive_response(1);
        assert_eq!(response["id"], 1);
        assert!(response["result"]["serverInfo"].is_object());
        self.notify("notifications/initialized", json!({}));
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send_request(id, method, params);
        self.receive_response(id)
    }

    fn send_request(&mut self, id: u64, method: &str, params: Value) {
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.write_json(&message);
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.write_json(&json!({"jsonrpc":"2.0","method":method,"params":params}));
    }

    fn write_json(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().expect("bridge stdin is open");
        serde_json::to_writer(&mut *stdin, message).expect("serialize MCP message");
        stdin.write_all(b"\n").expect("write MCP frame");
        stdin.flush().expect("flush MCP frame");
    }

    fn receive_response(&mut self, id: u64) -> Value {
        loop {
            let line = self.receive_line(WAIT);
            let message: Value = serde_json::from_slice(&line).expect("bridge stdout MCP JSON");
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
        }
    }

    fn receive_line(&mut self, duration: Duration) -> Vec<u8> {
        let deadline = Instant::now() + duration;
        loop {
            if let Some(newline) = self.buffered.iter().position(|byte| *byte == b'\n') {
                return self.buffered.drain(..=newline).collect();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "timed out waiting for bridge stdout");
            match self.stdout.recv_timeout(remaining) {
                Ok(bytes) => self.buffered.extend(bytes),
                Err(RecvTimeoutError::Timeout) => panic!(
                    "timed out waiting for bridge stdout (buffered {} bytes: {:?}); bridge stderr: {}",
                    self.buffered.len(),
                    String::from_utf8_lossy(&self.buffered[..self.buffered.len().min(64)]),
                    String::from_utf8_lossy(&self.stderr.lock().expect("stderr capture lock"))
                ),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("bridge stdout closed before a complete MCP frame")
                }
            }
        }
    }

    fn receive_bytes(&mut self, duration: Duration) -> Vec<u8> {
        match self.stdout.recv_timeout(duration) {
            Ok(bytes) => bytes,
            Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for bridge bytes"),
            Err(RecvTimeoutError::Disconnected) => {
                panic!("bridge stdout closed before expected bytes arrived")
            }
        }
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
    }

    fn wait(&mut self, duration: Duration) -> ExitStatus {
        let status = self.child.wait(duration);
        self.join_output_threads();
        status
    }

    fn join_output_threads(&mut self) {
        if let Some(handle) = self.stdout_thread.take() {
            handle.join().expect("bridge stdout reader exited");
        }
        if let Some(handle) = self.stderr_thread.take() {
            handle.join().expect("bridge stderr reader exited");
        }
    }
}

impl Drop for BridgeClient {
    fn drop(&mut self) {
        self.close_stdin();
        if let Some(child) = &mut self.child.child {
            if child.try_wait().ok().flatten().is_none() {
                // SAFETY: this guard owns the bridge's process group.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        self.join_output_threads();
    }
}

fn forward_stdout(mut stdout: ChildStdout, sender: mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0; 8192];
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                if sender.send(buffer[..read].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

fn wait_child(child: &mut Child, duration: Duration) -> ExitStatus {
    let deadline = Instant::now() + duration;
    loop {
        if let Some(status) = child.try_wait().expect("check child status") {
            return status;
        }
        if Instant::now() >= deadline {
            // SAFETY: caller owns the process; kill and reap before failing.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            panic!("child did not exit within {duration:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn output_with_timeout(command: &mut Command, duration: Duration) -> std::process::Output {
    let mut child = ChildGuard::new(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start bounded CLI command"),
    );
    wait_child(child.child_mut(), duration);
    child
        .child
        .take()
        .expect("owned command child")
        .wait_with_output()
        .expect("collect bounded CLI command output")
}

struct JoinOnDrop(Option<JoinHandle<()>>);

impl JoinOnDrop {
    fn new(handle: JoinHandle<()>) -> Self {
        Self(Some(handle))
    }

    fn join(mut self) -> thread::Result<()> {
        self.0.take().expect("owned thread").join()
    }
}

impl Drop for JoinOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = handle.join();
        }
    }
}
