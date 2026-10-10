use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use uuid::Uuid;

pub const MAX_PAYLOAD_SIZE: usize = 8 * 1024 * 1024;
pub const BRIDGE_DIR: &str = "/tmp/codex-browser-use";
pub const LOG_PATH: &str = "/tmp/codex-browser-use/agentic-browser-host.log";
pub const ID_PREFIX: &str = "agentic-browser-host:";

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    TooLarge(u32),
    Json(serde_json::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::TooLarge(size) => write!(formatter, "frame too large: {size}"),
            Self::Json(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for FrameError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

fn read_exact_or_eof(reader: &mut (impl Read + ?Sized), buffer: &mut [u8]) -> io::Result<bool> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => return Ok(false),
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

pub fn read_frame(reader: &mut (impl Read + ?Sized)) -> Result<Option<Value>, FrameError> {
    let mut header = [0_u8; 4];
    if !read_exact_or_eof(reader, &mut header)? {
        return Ok(None);
    }

    let size = u32::from_ne_bytes(header);
    if size as usize > MAX_PAYLOAD_SIZE {
        return Err(FrameError::TooLarge(size));
    }

    let mut payload = vec![0_u8; size as usize];
    if !read_exact_or_eof(reader, &mut payload)? {
        return Ok(None);
    }

    Ok(Some(serde_json::from_slice(&payload)?))
}

pub fn write_frame(writer: &mut (impl Write + ?Sized), message: &Value) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(message)?;
    let size = u32::try_from(payload.len()).map_err(|_| {
        FrameError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame length exceeds uint32",
        ))
    })?;
    writer.write_all(&size.to_ne_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

#[derive(Clone)]
struct Logger {
    path: PathBuf,
}

impl Logger {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn log(&self, message: impl AsRef<str>) {
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| writeln!(file, "{}", message.as_ref().trim_end()));
        let _ = result;
    }
}

type ClientId = u64;
type LockedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

struct ClientEntry {
    writer: LockedWriter,
    connection: Option<UnixStream>,
}

struct PendingRoute {
    client_id: ClientId,
    original_id: Value,
    method: Option<String>,
}

struct Registry {
    clients: HashMap<ClientId, ClientEntry>,
    pending: HashMap<String, PendingRoute>,
    next_client_id: ClientId,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            clients: HashMap::new(),
            pending: HashMap::new(),
            next_client_id: 1,
        }
    }
}

pub struct Host {
    socket_path: PathBuf,
    logger: Logger,
    extension_writer: LockedWriter,
    registry: Mutex<Registry>,
    standalone_compat: bool,
}

impl Host {
    pub fn new(
        socket_path: impl Into<PathBuf>,
        log_path: impl Into<PathBuf>,
        extension_writer: Box<dyn Write + Send>,
    ) -> Self {
        Self::new_with_standalone_compat(socket_path, log_path, extension_writer, false)
    }

    fn new_with_standalone_compat(
        socket_path: impl Into<PathBuf>,
        log_path: impl Into<PathBuf>,
        extension_writer: Box<dyn Write + Send>,
        standalone_compat: bool,
    ) -> Self {
        Self {
            socket_path: socket_path.into(),
            logger: Logger::new(log_path),
            extension_writer: Arc::new(Mutex::new(extension_writer)),
            registry: Mutex::new(Registry::default()),
            standalone_compat,
        }
    }

    fn register_client(
        &self,
        writer: Box<dyn Write + Send>,
        connection: Option<UnixStream>,
    ) -> ClientId {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let client_id = registry.next_client_id;
        registry.next_client_id += 1;
        registry.clients.insert(
            client_id,
            ClientEntry {
                writer: Arc::new(Mutex::new(writer)),
                connection,
            },
        );
        client_id
    }

    fn client_count(&self) -> usize {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clients
            .len()
    }

    fn drop_client(&self, client_id: ClientId) {
        let entry = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = registry.clients.remove(&client_id);
            registry
                .pending
                .retain(|_, route| route.client_id != client_id);
            entry
        };

        if let Some(connection) = entry.and_then(|entry| entry.connection) {
            let _ = connection.shutdown(std::net::Shutdown::Both);
        }
    }

    fn client_writer(&self, client_id: ClientId) -> Option<LockedWriter> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clients
            .get(&client_id)
            .map(|entry| Arc::clone(&entry.writer))
    }

    fn write_locked(writer: &LockedWriter, message: &Value) -> Result<(), FrameError> {
        let mut writer = writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_frame(&mut **writer, message)
    }

    fn send_extension(&self, message: &Value) -> Result<(), FrameError> {
        self.logger.log(format!(
            "to-extension method={} id={}",
            log_value(message.get("method")),
            log_value(message.get("id"))
        ));
        Self::write_locked(&self.extension_writer, message)
    }

    fn broadcast(&self, message: &Value) {
        let clients: Vec<_> = self
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clients
            .iter()
            .map(|(&client_id, entry)| (client_id, Arc::clone(&entry.writer)))
            .collect();

        let mut dead = Vec::new();
        for (client_id, writer) in clients {
            if Self::write_locked(&writer, message).is_err() {
                dead.push(client_id);
            }
        }
        for client_id in dead {
            self.drop_client(client_id);
        }
    }

    fn handle_client_message(&self, client_id: ClientId, message: Value) -> Result<(), FrameError> {
        let Some(object) = message.as_object() else {
            return Ok(());
        };
        if !object.contains_key("method") {
            return Ok(());
        }

        if object.get("method").and_then(Value::as_str) == Some("bridge.getStatus")
            && object.contains_key("id")
        {
            let response = json!({
                "jsonrpc": "2.0",
                "id": object.get("id").expect("id checked").clone(),
                "result": {
                    "ok": true,
                    "socketPath": self.socket_path,
                    "clients": self.client_count(),
                },
            });
            if let Some(writer) = self.client_writer(client_id) {
                Self::write_locked(&writer, &response)?;
            }
            return Ok(());
        }

        if !object.contains_key("id") {
            return self.send_extension(&message);
        }

        let bridge_id = format!("{ID_PREFIX}{client_id}:{}", Uuid::new_v4().simple());
        let original_id = object.get("id").expect("id checked").clone();
        let method = object
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending
            .insert(
                bridge_id.clone(),
                PendingRoute {
                    client_id,
                    original_id,
                    method,
                },
            );

        let mut outbound = object.clone();
        outbound.insert("id".to_owned(), Value::String(bridge_id));
        self.send_extension(&Value::Object(outbound))
    }

    fn handle_extension_message(&self, message: Value) -> Result<(), FrameError> {
        let Some(object) = message.as_object() else {
            return Ok(());
        };
        self.logger.log(format!(
            "from-extension method={} id={}",
            log_value(object.get("method")),
            log_value(object.get("id"))
        ));

        let has_id = object.contains_key("id");
        let has_method = object.contains_key("method");
        if has_id && !has_method {
            self.route_extension_response(object);
            return Ok(());
        }
        if has_method && !has_id {
            self.broadcast(&message);
            return Ok(());
        }
        if !has_method || !has_id {
            return Ok(());
        }

        let response = extension_request_response(object);
        self.send_extension(&response)
    }

    fn route_extension_response(&self, object: &Map<String, Value>) {
        let Some(bridge_id) = object.get("id").and_then(Value::as_str) else {
            return;
        };
        let routed = {
            let mut registry = self
                .registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let route = registry.pending.remove(bridge_id);
            route.and_then(|route| {
                registry
                    .clients
                    .get(&route.client_id)
                    .map(|entry| (route, Arc::clone(&entry.writer)))
            })
        };

        let Some((route, writer)) = routed else {
            return;
        };
        let mut response = object.clone();
        if self.standalone_compat && route.method.as_deref() == Some("getInfo") {
            if let Some(result) = response.get_mut("result").and_then(Value::as_object_mut) {
                result.remove("agentRequestHeaderEnabled");
            }
        }
        response.insert("id".to_owned(), route.original_id);
        if Self::write_locked(&writer, &Value::Object(response)).is_err() {
            self.drop_client(route.client_id);
        }
    }
}

fn extension_request_response(request: &Map<String, Value>) -> Value {
    let id = request.get("id").expect("request id checked").clone();
    let method = request.get("method").and_then(Value::as_str);
    match method {
        Some("ping") => json!({"jsonrpc": "2.0", "id": id, "result": "pong"}),
        Some("ensureCodexAppServer" | "codexRuntime/ensure" | "codexRuntime/restart") => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "ok": false,
                "error": "ChatGPT app server is not provided by Agentic browser host PoC.",
            },
        }),
        Some("codexRuntime/hello") => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "manifestSchemaVersion": 2,
                "nativeHostProtocolVersion": 2,
                "supportedProtocolVersions": [2],
                "supportedMethods": [],
            },
        }),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!(
                    "No Agentic PoC handler for {}",
                    log_value(request.get("method"))
                ),
            },
        }),
    }
}

fn log_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".to_owned(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

fn handle_client(host: Arc<Host>, connection: UnixStream) {
    let result = (|| -> Result<(), FrameError> {
        let control = connection.try_clone()?;
        let writer = connection.try_clone()?;
        let client_id = host.register_client(Box::new(writer), Some(control));
        host.logger.log(format!("client connected id={client_id}"));

        let result = (|| {
            let mut reader = connection;
            while let Some(message) = read_frame(&mut reader)? {
                host.handle_client_message(client_id, message)?;
            }
            Ok(())
        })();

        if let Err(error) = &result {
            host.logger
                .log(format!("client error id={client_id}: {error}"));
        }
        host.drop_client(client_id);
        host.logger.log(format!("client closed id={client_id}"));
        result
    })();
    let _ = result;
}

fn prepare_socket(path: &Path, logger: &Logger) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))?;
    logger.log(format!("listening socket={}", path.display()));
    Ok(listener)
}

fn socket_path_for_pid(pid: u32) -> PathBuf {
    Path::new(BRIDGE_DIR).join(format!("agentic-browser-host-{pid}.sock"))
}

fn standalone_compat_enabled() -> bool {
    std::env::var_os("AGENTIC_BROWSER_HOST_STANDALONE_COMPAT").as_deref()
        == Some(std::ffi::OsStr::new("1"))
}

pub fn run() {
    let socket_path = socket_path_for_pid(std::process::id());
    let logger = Logger::new(LOG_PATH);
    let origin: Vec<_> = std::env::args().skip(1).collect();
    logger.log(format!(
        "native host started pid={} origin={origin:?}",
        std::process::id()
    ));

    let standalone_compat = standalone_compat_enabled();
    logger.log(format!("standalone compatibility={standalone_compat}"));
    let host = Arc::new(Host::new_with_standalone_compat(
        &socket_path,
        LOG_PATH,
        Box::new(io::stdout()),
        standalone_compat,
    ));
    match prepare_socket(&socket_path, &logger) {
        Ok(listener) => {
            let socket_host = Arc::clone(&host);
            let socket_logger = logger.clone();
            thread::spawn(move || {
                for connection in listener.incoming() {
                    match connection {
                        Ok(connection) => {
                            let client_host = Arc::clone(&socket_host);
                            thread::spawn(move || handle_client(client_host, connection));
                        }
                        Err(error) => {
                            socket_logger.log(format!("socket accept error: {error}"));
                            break;
                        }
                    }
                }
            });
        }
        Err(error) => logger.log(format!("socket server error: {error}")),
    }

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let result = (|| -> Result<(), FrameError> {
        while let Some(message) = read_frame(&mut input)? {
            host.handle_extension_message(message)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        logger.log(format!("native host fatal: {error}"));
    }

    match fs::remove_file(&socket_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => logger.log(format!("socket cleanup error: {error}")),
    }
    logger.log("native host stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[derive(Clone, Default)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SharedBuffer {
        fn messages(&self) -> Vec<Value> {
            let bytes = self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let mut cursor = Cursor::new(bytes);
            let mut messages = Vec::new();
            while let Some(message) = read_frame(&mut cursor).unwrap() {
                messages.push(message);
            }
            messages
        }
    }

    fn test_host() -> (Host, SharedBuffer) {
        let extension = SharedBuffer::default();
        (
            Host::new(
                "/tmp/test-browser-host.sock",
                "/dev/null",
                Box::new(extension.clone()),
            ),
            extension,
        )
    }

    fn compat_test_host() -> (Host, SharedBuffer) {
        let extension = SharedBuffer::default();
        (
            Host::new_with_standalone_compat(
                "/tmp/test-browser-host.sock",
                "/dev/null",
                Box::new(extension.clone()),
                true,
            ),
            extension,
        )
    }

    #[test]
    fn zero_malformed_and_non_utf8_payloads_are_errors() {
        let cases = [
            0_u32.to_ne_bytes().to_vec(),
            framed_payload(b"{"),
            framed_payload(&[0xff]),
        ];
        for input in cases {
            assert!(matches!(
                read_frame(&mut Cursor::new(input)),
                Err(FrameError::Json(_))
            ));
        }
    }

    fn framed_payload(payload: &[u8]) -> Vec<u8> {
        let mut frame = (payload.len() as u32).to_ne_bytes().to_vec();
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn dropping_client_removes_its_pending_routes() {
        let (host, extension) = test_host();
        let client = SharedBuffer::default();
        let client_id = host.register_client(Box::new(client.clone()), None);
        host.handle_client_message(
            client_id,
            json!({"jsonrpc": "2.0", "id": 42, "method": "getInfo"}),
        )
        .unwrap();
        let bridge_id = extension.messages()[0]["id"].as_str().unwrap().to_owned();

        host.drop_client(client_id);

        let registry = host
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(registry.clients.is_empty());
        assert!(registry.pending.is_empty());
        drop(registry);
        host.handle_extension_message(json!({"id": bridge_id, "result": "stale"}))
            .unwrap();
        assert!(client.messages().is_empty());
    }

    #[test]
    fn identifier_free_notifications_forward_and_broadcast() {
        let (host, extension) = test_host();
        let first_client = SharedBuffer::default();
        let second_client = SharedBuffer::default();
        let first_id = host.register_client(Box::new(first_client.clone()), None);
        host.register_client(Box::new(second_client.clone()), None);
        let client_notification = json!({"method": "client.event", "params": {"value": 1}});
        let extension_notification = json!({"method": "extension.event", "params": [2]});

        host.handle_client_message(first_id, client_notification.clone())
            .unwrap();
        host.handle_extension_message(extension_notification.clone())
            .unwrap();

        assert_eq!(extension.messages(), vec![client_notification]);
        assert_eq!(
            first_client.messages(),
            vec![extension_notification.clone()]
        );
        assert_eq!(second_client.messages(), vec![extension_notification]);
    }

    #[test]
    fn extension_requests_receive_baseline_responses() {
        let (host, extension) = test_host();
        for (id, method) in [
            (1, "ping"),
            (2, "ensureCodexAppServer"),
            (3, "codexRuntime/ensure"),
            (4, "codexRuntime/restart"),
            (5, "codexRuntime/hello"),
            (6, "unrecognized"),
        ] {
            host.handle_extension_message(json!({"id": id, "method": method}))
                .unwrap();
        }

        let negative = json!({
            "ok": false,
            "error": "ChatGPT app server is not provided by Agentic browser host PoC.",
        });
        assert_eq!(
            extension.messages(),
            vec![
                json!({"jsonrpc": "2.0", "id": 1, "result": "pong"}),
                json!({"jsonrpc": "2.0", "id": 2, "result": negative}),
                json!({"jsonrpc": "2.0", "id": 3, "result": negative}),
                json!({"jsonrpc": "2.0", "id": 4, "result": negative}),
                json!({
                    "jsonrpc": "2.0",
                    "id": 5,
                    "result": {
                        "manifestSchemaVersion": 2,
                        "nativeHostProtocolVersion": 2,
                        "supportedProtocolVersions": [2],
                        "supportedMethods": [],
                    },
                }),
                json!({
                    "jsonrpc": "2.0",
                    "id": 6,
                    "error": {
                        "code": -32601,
                        "message": "No Agentic PoC handler for unrecognized",
                    },
                }),
            ]
        );
    }

    #[test]
    fn rewritten_request_ids_restore_to_the_originating_clients() {
        let (host, extension) = test_host();
        let first_client = SharedBuffer::default();
        let second_client = SharedBuffer::default();
        let first_id = host.register_client(Box::new(first_client.clone()), None);
        let second_id = host.register_client(Box::new(second_client.clone()), None);

        host.handle_client_message(
            first_id,
            json!({"jsonrpc": "2.0", "id": 11, "method": "getInfo"}),
        )
        .unwrap();
        host.handle_client_message(
            second_id,
            json!({"jsonrpc": "2.0", "id": "two", "method": "getUserTabs"}),
        )
        .unwrap();

        let outbound = extension.messages();
        let first_bridge_id = outbound[0]["id"].as_str().unwrap();
        let second_bridge_id = outbound[1]["id"].as_str().unwrap();
        assert!(first_bridge_id.starts_with("agentic-browser-host:1:"));
        assert!(second_bridge_id.starts_with("agentic-browser-host:2:"));
        assert_eq!(first_bridge_id.len(), "agentic-browser-host:1:".len() + 32);
        assert_eq!(second_bridge_id.len(), "agentic-browser-host:2:".len() + 32);

        host.handle_extension_message(json!({"id": second_bridge_id, "result": "second"}))
            .unwrap();
        host.handle_extension_message(json!({"id": first_bridge_id, "result": "first"}))
            .unwrap();

        assert_eq!(
            first_client.messages(),
            vec![json!({"id": 11, "result": "first"})]
        );
        assert_eq!(
            second_client.messages(),
            vec![json!({"id": "two", "result": "second"})]
        );
    }

    #[test]
    fn standalone_compat_hides_agent_request_header_capability_for_get_info() {
        let (host, extension) = compat_test_host();
        let client = SharedBuffer::default();
        let client_id = host.register_client(Box::new(client.clone()), None);

        host.handle_client_message(
            client_id,
            json!({"jsonrpc": "2.0", "id": 1, "method": "getInfo"}),
        )
        .unwrap();
        let bridge_id = extension.messages()[0]["id"].as_str().unwrap().to_owned();
        host.handle_extension_message(json!({
            "jsonrpc": "2.0",
            "id": bridge_id,
            "result": {
                "family": "chrome",
                "agentRequestHeaderEnabled": false,
            },
        }))
        .unwrap();

        assert_eq!(
            client.messages(),
            vec![json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {"family": "chrome"},
            })]
        );
    }
}
