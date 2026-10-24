use std::cell::Cell;
use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

// Protect the real Local MCP -> email store -> renderer/file writer path. Owner
// tests and direct render-worker tests do not cover this public combination.
#[test]
fn local_email_api_preserves_native_images_cursors_and_attachment_writes() {
    let mut fixture = Fixture::new();
    let config_path = fixture.root.join("config.json");
    let config_arg = config_path.to_str().unwrap();
    fixture.run(&[
        "config",
        "--config",
        config_arg,
        "init",
        "--non-interactive",
        "--mode",
        "local",
        "--profile",
        "room",
    ]);
    let workspace = fixture.root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let password = fixture.root.join("password");
    fs::write(&password, "unused-fixture-password").unwrap();
    fs::set_permissions(&password, fs::Permissions::from_mode(0o600)).unwrap();
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["agentId"] = json!("email-api");
    config["workspaceRoot"] = json!(workspace);
    // Browser, Tmux, Skills and Process are disabled; rendering must stand alone.
    config["toolsets"]["enabled"] = json!(["agent", "room", "file"]);
    config["confirmationProvider"] = json!({"channels": []});
    config["policy"]["path"] =
        json!({"defaultAction":"deny","readOnly":[],"write":[workspace],"deny":[]});
    config["email"] = json!({"enabled":true,"syncIntervalSeconds":86400,"sendPolicy":"deny","trashPolicy":"deny","account":{
        "imapHost":"127.0.0.1","imapPort":9,"smtpHost":"127.0.0.1","smtpPort":9,
        "username":"fixture","password":format!("file:{}",password.display()),"from":"sender@example.invalid"}});
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    fixture.agent = Some(fixture.spawn(&["run", "--config", config_arg], "agent"));
    let socket = fixture
        .root
        .join("h/.agentic_gpt/runtime/agent/email-api/mcp.sock");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "Agent socket was not created");
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(
        data(&fixture.call("agent.info", json!({})))["connections"]["localMcp"]["status"],
        "ready"
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let remote = format!("http://{}/must-not-connect", listener.local_addr().unwrap());
    let id = format!("mail_{}", Uuid::new_v4().simple());
    let text = format!(
        "Email fixture text. {} needle-final",
        "Long paragraph for cursor traversal. ".repeat(1800)
    );
    let html = format!(
        r#"<html><body><img src="{remote}" width="20" height="20"><div style="height:1800px;background:#aa3322">first slice</div><div style="height:1800px;background:#225599">second slice</div></body></html>"#
    );
    fixture.seed(&id, &text, &html);

    let listed = fixture.call("room.email.list", json!({}));
    assert_eq!(data(&listed)["items"][0]["id"], id);
    let first_text = fixture.call("room.email.read", json!({"id":id}));
    assert_eq!(
        data(&first_text)["to"],
        json!(["\"Receiver\" <receiver@example.invalid>"])
    );
    assert_eq!(
        data(&first_text)["cc"],
        json!(["\"Copy\" <copy@example.invalid>"])
    );
    let text_cursor = data(&first_text)["nextCursor"]
        .as_str()
        .expect("text continuation");
    // The first read fills metadata for legacy/externally seeded rows. Later
    // text pages must use the persisted body and metadata without opening EML.
    let raw_path = fixture
        .root
        .join("h/.agentic_gpt/state/email")
        .join(format!("{id}.eml"));
    let held_raw = raw_path.with_extension("held");
    fs::rename(&raw_path, &held_raw).unwrap();
    let second_text = fixture.call("room.email.read", json!({"id":id,"cursor":text_cursor}));
    fs::rename(held_raw, raw_path).unwrap();
    assert_eq!(
        data(&second_text)["attachments"],
        data(&first_text)["attachments"]
    );
    let joined = format!(
        "{}{}",
        data(&first_text)["body"].as_str().unwrap(),
        data(&second_text)["body"].as_str().unwrap()
    );
    assert!(
        text.starts_with(&joined) && joined.len() > 24 * 1024,
        "text cursor repeated/skipped content"
    );
    let found = fixture.call("room.email.search", json!({"query":"Fixture topic"}));
    assert_eq!(data(&found)["items"][0]["ref"], id);
    let found_body = fixture.call("room.email.search", json!({"id":id,"query":"needle-final"}));
    let found_cursor = data(&found_body)["items"][0]["ref"].as_str().unwrap();
    let hit = fixture.call("room.email.read", json!({"id":id,"cursor":found_cursor}));
    assert!(data(&hit)["body"]
        .as_str()
        .unwrap()
        .contains("needle-final"));

    let first_image = fixture.call("room.email.read", json!({"id":id,"view":"image"}));
    let image_bytes = STANDARD.decode(native_image(&first_image)).unwrap();
    let image = image::load_from_memory(&image_bytes).unwrap();
    assert_eq!(image.width(), 1024);
    assert!(
        data(&first_image).get("images").is_none(),
        "Base64 duplicated in structured content"
    );
    let next = data(&first_image)["nextCursor"]
        .as_str()
        .expect("image continuation");
    let second_image = fixture.call(
        "room.email.read",
        json!({"id":id,"view":"image","cursor":next}),
    );
    assert_ne!(
        native_image(&first_image),
        native_image(&second_image),
        "image cursor repeated the first slice"
    );
    let second_png = STANDARD.decode(native_image(&second_image)).unwrap();
    let first_pixels = image.to_rgba8();
    let second_pixels = image::load_from_memory(&second_png).unwrap().to_rgba8();
    assert!(first_pixels
        .pixels()
        .any(|pixel| pixel.0 == [170, 51, 34, 255]));
    assert!(second_pixels
        .pixels()
        .any(|pixel| pixel.0 == [34, 85, 153, 255]));

    let remote_image = fixture.call("room.email.read", json!({"id":id,"view":"imageWithRemote"}));
    assert!(data(&remote_image)["blockedRemoteImages"].as_u64().unwrap() >= 1);
    let remote_cursor = data(&remote_image)["nextCursor"]
        .as_str()
        .expect("remote image continuation");
    let cache = fs::read_dir(fixture.root.join("h/.agentic_gpt/cache/email"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("remote-")
        })
        .unwrap();
    let cached = fs::read(&cache).unwrap();
    let continued = fixture.call(
        "room.email.read",
        json!({"id":id,"view":"imageWithRemote","cursor":remote_cursor}),
    );
    assert_ne!(native_image(&remote_image), native_image(&continued));
    assert_eq!(
        cached,
        fs::read(&cache).unwrap(),
        "continuation regenerated the prepared page"
    );
    // A rebuilt snapshot can change image dimensions and page boundaries. An
    // old cursor must not continue against different downloaded image bytes.
    let mut replaced: Value = serde_json::from_slice(&cached).unwrap();
    replaced["images"][&remote] = json!(native_image(&remote_image));
    fs::write(&cache, serde_json::to_vec(&replaced).unwrap()).unwrap();
    let changed = fixture.call(
        "room.email.read",
        json!({"id":id,"view":"imageWithRemote","cursor":remote_cursor}),
    );
    assert_eq!(
        changed["structuredContent"]["error"]["code"],
        "email_render_cache_expired"
    );
    fs::remove_file(&cache).unwrap();
    let expired = fixture.call(
        "room.email.read",
        json!({"id":id,"view":"imageWithRemote","cursor":remote_cursor}),
    );
    assert_eq!(
        expired["structuredContent"]["error"]["code"],
        "email_render_cache_expired"
    );
    // A previous-version derived snapshot must rebuild from page one, while a
    // continuation must never silently re-download resources or change layout.
    fs::write(
        &cache,
        serde_json::to_vec(&json!({
            "createdAt": chrono::Utc::now().timestamp() as u64,
            "htmlBase64": STANDARD.encode(b"old cached HTML")
        }))
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o600)).unwrap();
    let rebuilt = fixture.call("room.email.read", json!({"id":id,"view":"imageWithRemote"}));
    assert_eq!(native_image(&rebuilt), native_image(&remote_image));
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
        "remote mode connected to a forbidden local URL"
    );

    let attachment = data(&first_text)["attachments"][0]["id"].as_str().unwrap();
    let download = json!({"id":id,"attachmentId":attachment,"path":"downloads/fixture.bin"});
    let saved = fixture.call("room.email.attachment", download.clone());
    assert_eq!(data(&saved)["path"], "downloads/fixture.bin");
    assert_eq!(
        fs::read(workspace.join("downloads/fixture.bin")).unwrap(),
        b"attachment fixture bytes"
    );
    let repeated = fixture.call("room.email.attachment", download);
    assert_eq!(
        repeated["structuredContent"]["error"]["code"],
        "file_already_exists"
    );
    assert_eq!(
        fs::read(workspace.join("downloads/fixture.bin")).unwrap(),
        b"attachment fixture bytes"
    );
    for (tool, arguments) in [
        (
            "room.email.send",
            json!({"to":["receiver@example.invalid"],"subject":"fixture","text":"must not send"}),
        ),
        ("room.email.operate", json!({"id":id,"action":"trash"})),
    ] {
        let denied = fixture.call(tool, arguments);
        assert_eq!(
            denied["structuredContent"]["error"]["code"],
            "email_policy_denied"
        );
    }
}

fn data(result: &Value) -> &Value {
    assert_ne!(result["isError"], true, "MCP error: {result}");
    &result["structuredContent"]
}

fn native_image(result: &Value) -> &str {
    data(result);
    result["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|content| content["type"] == "image" && content["mimeType"] == "image/png")
        .and_then(|content| content["data"].as_str())
        .expect("native MCP PNG image block")
}

struct Fixture {
    root: PathBuf,
    agent: Option<Process>,
    sequence: Cell<usize>,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from("/tmp")
            .join(format!("ea-{}", &Uuid::new_v4().simple().to_string()[..12]));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            root,
            agent: None,
            sequence: Cell::new(0),
        };
        for directory in ["h", "c", "d", "k", "s", "r", "t"] {
            fs::create_dir(fixture.root.join(directory)).unwrap();
            fs::set_permissions(
                fixture.root.join(directory),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
        }
        fixture
    }
    fn spawn(&self, args: &[&str], label: &str) -> Process {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agentic-gpt"));
        command.args(args).process_group(0).stdin(Stdio::null());
        for (key, directory) in [
            ("HOME", "h"),
            ("XDG_CONFIG_HOME", "c"),
            ("XDG_DATA_HOME", "d"),
            ("XDG_CACHE_HOME", "k"),
            ("XDG_STATE_HOME", "s"),
            ("XDG_RUNTIME_DIR", "r"),
            ("TMPDIR", "t"),
            ("TMUX_TMPDIR", "r"),
        ] {
            command.env(key, self.root.join(directory));
        }
        command
            .env_remove("TMUX")
            .env_remove("AGENTIC_SUPERVISOR_TOKEN")
            .env("TOKIO_WORKER_THREADS", "2");
        command.stdout(fs::File::create(self.root.join(format!("{label}.out"))).unwrap());
        command.stderr(fs::File::create(self.root.join(format!("{label}.err"))).unwrap());
        Process(command.spawn().unwrap())
    }
    fn run(&self, args: &[&str]) -> Vec<u8> {
        let sequence = self.sequence.get();
        self.sequence.set(sequence + 1);
        let label = format!("call-{sequence}");
        let mut process = self.spawn(args, &label);
        let deadline = Instant::now() + Duration::from_secs(50);
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "CLI failed: {}",
                    fs::read_to_string(self.root.join(format!("{label}.err"))).unwrap()
                );
                return fs::read(self.root.join(format!("{label}.out"))).unwrap();
            }
            assert!(Instant::now() < deadline, "CLI timed out: {args:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn call(&self, tool: &str, arguments: Value) -> Value {
        if !tool.starts_with("room.email.") {
            return self.call_raw(tool, arguments);
        }
        // A fresh real medium event makes each email result's shared frame
        // observable, including image projections and business-error responses.
        let reference = format!("email-{}", &Uuid::new_v4().simple().to_string()[..12]);
        let injected = self.call_raw(
            "privateevent.inject",
            json!({"message":reference,"ref":reference,"severity":"medium"}),
        );
        let event_id = data(&injected)["eventId"].as_str().unwrap();
        let result = self.call_raw(tool, arguments);
        let structured = &result["structuredContent"];
        let text = result["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|content| content["type"] == "text")
            .filter_map(|content| content["text"].as_str())
            .filter_map(|text| serde_json::from_str::<Value>(text).ok())
            .find(|value| value.get("events").is_some())
            .expect("email result text retained the shared event panel");
        for value in [structured, &text] {
            let panel = &value["events"];
            assert!(
                panel["current"].is_string(),
                "missing shared panel: {result}"
            );
            assert!(
                panel["new"].as_array().unwrap().iter().any(|item| {
                    item.as_object()
                        .unwrap()
                        .keys()
                        .any(|key| key.starts_with(event_id) && key.contains(&reference))
                }),
                "{tool} dropped the newly injected event from its shared frame"
            );
        }
        result
    }
    fn call_raw(&self, tool: &str, arguments: Value) -> Value {
        serde_json::from_slice(&self.run(&[
            "local",
            "--config",
            self.root.join("config.json").to_str().unwrap(),
            "call",
            tool,
            "--arguments",
            &arguments.to_string(),
        ]))
        .unwrap()
    }
    fn seed(&self, id: &str, text: &str, html: &str) {
        let raw = format!("From: Sender <sender@example.invalid>\r\nTo: Receiver <receiver@example.invalid>\r\nCc: Copy <copy@example.invalid>\r\nSubject: Fixture topic\r\nMessage-ID: <fixture@example.invalid>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=mixed\r\n\r\n--mixed\r\nContent-Type: multipart/alternative; boundary=alternative\r\n\r\n--alternative\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}\r\n--alternative\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--alternative--\r\n--mixed\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=fixture.bin\r\n\r\nattachment fixture bytes\r\n--mixed--\r\n", text.replace('\n', "\r\n"));
        let account = format!("{:x}", Sha256::digest(b"127.0.0.1\x009\x00fixture"));
        let state = self.root.join("h/.agentic_gpt/state");
        let connection = Connection::open(state.join("email.sqlite3")).unwrap();
        connection.busy_timeout(Duration::from_secs(5)).unwrap();
        connection.execute("INSERT OR IGNORE INTO accounts(account,validity,baseline,last_uid,initial_complete) VALUES(?,1,1,1,1)", [&account]).unwrap();
        connection.execute("INSERT INTO mail(id,account,validity,uid,inbox,sender,recipients,cc,subject,received_at,unread,status,snippet,body,format,message_id,raw_bytes,body_bytes) VALUES(?,?,1,1,1,?,?,?,'Fixture topic',?,1,'received','Email fixture text.',?,'plain','<fixture@example.invalid>',?,?)", params![id,account,"Sender <sender@example.invalid>","Receiver <receiver@example.invalid>","Copy <copy@example.invalid>",chrono::Utc::now().timestamp_millis(),text,raw.len() as i64,text.len() as i64]).unwrap();
        connection
            .execute(
                "UPDATE stats SET value=? WHERE key='content_bytes'",
                [(raw.len() + text.len()) as i64],
            )
            .unwrap();
        // Insert first: a concurrent owner recovery must not delete a fixture raw file as orphaned.
        let path = state.join("email").join(format!("{id}.eml"));
        fs::write(&path, raw).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.agent.take());
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_some() {
            return;
        }
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.0.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}
