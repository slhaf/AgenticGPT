use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use rmcp::model::CallToolResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const API_VERSION: u32 = 1;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

type Reply = Result<Value, BrowserError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct BrowserError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl BrowserError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    fn disconnected() -> Self {
        Self::new("browser_disconnected", "Browser connection closed")
    }

    fn expired() -> Self {
        Self::new("browser_lease_expired", "Browser lease is no longer active")
    }

    fn invalid_result() -> Self {
        Self::new(
            "browser_result_invalid",
            "Browser returned an invalid result",
        )
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserInfo {
    pub(crate) browser_id: String,
    pub(crate) browser_name: String,
    pub(crate) extension_version: String,
    pub(crate) api_version: u32,
    pub(crate) connected: bool,
    pub(crate) busy: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LeaseInfo {
    name: String,
    browser_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserList {
    listener_ready: bool,
    runtime_available: bool,
    browsers: Vec<BrowserInfo>,
    leases: Vec<LeaseInfo>,
}

#[derive(Clone)]
struct Connection {
    id: Uuid,
    info: BrowserInfo,
    requests: mpsc::Sender<Request>,
    stop: CancellationToken,
}

struct Lease {
    connection: Uuid,
    browser_id: String,
    id: Option<String>,
    acquiring: bool,
}

#[derive(Default)]
struct Registry {
    connections: BTreeMap<String, Connection>,
    offline_browsers: BTreeMap<String, BrowserInfo>,
    leases: BTreeMap<String, Lease>,
}

impl Registry {
    fn disconnect(&mut self, connection: &Connection) {
        let browser_id = &connection.info.browser_id;
        if self.connections.get(browser_id).map(|entry| entry.id) == Some(connection.id) {
            let mut info = self
                .connections
                .remove(browser_id)
                .expect("matched connection")
                .info;
            info.connected = false;
            info.busy = false;
            self.offline_browsers.insert(browser_id.clone(), info);
        }
        self.leases
            .retain(|_, lease| lease.connection != connection.id);
    }

    fn lease_ended(&mut self, connection: Uuid, lease_id: &str) {
        self.leases.retain(|_, lease| {
            if lease.connection == connection && lease.id.as_deref() == Some(lease_id) {
                lease.id = None;
                return lease.acquiring;
            }
            true
        });
    }

    fn acquire_failed(&mut self, name: &str, connection: Uuid) {
        if let Some(lease) = self
            .leases
            .get_mut(name)
            .filter(|lease| lease.connection == connection)
        {
            if lease.id.is_some() {
                lease.acquiring = false;
            } else {
                self.leases.remove(name);
            }
        }
    }
}

struct Request {
    message: Value,
    effect: RequestEffect,
    reply: oneshot::Sender<Reply>,
}

enum RequestEffect {
    Acquire(String),
    Lease { id: String, release: bool },
}

impl RequestEffect {
    fn apply(self, registry: &mut Registry, connection: &Connection, reply: Reply) -> Reply {
        match self {
            Self::Acquire(name) => {
                let reply = reply.and_then(|result| {
                    let id = result
                        .get("leaseId")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or_else(BrowserError::invalid_result)?;
                    let lease = registry
                        .leases
                        .get_mut(&name)
                        .filter(|lease| lease.connection == connection.id)
                        .ok_or_else(BrowserError::expired)?;
                    lease.id = Some(id.into());
                    lease.acquiring = false;
                    Ok(result)
                });
                if reply.is_err() {
                    registry.acquire_failed(&name, connection.id);
                }
                reply
            }
            Self::Lease { id, release } => {
                if (release
                    && reply
                        .as_ref()
                        .is_ok_and(|value| value.get("released") == Some(&Value::Bool(true))))
                    || reply
                        .as_ref()
                        .is_err_and(|error| error.code == "browser_lease_expired")
                {
                    registry.lease_ended(connection.id, &id);
                }
                reply
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
struct ListenerConfig {
    address: SocketAddr,
    token_digest: [u8; 32],
    agent_id: String,
}

struct Listener {
    config: ListenerConfig,
    stop: CancellationToken,
    task: JoinHandle<TcpListener>,
}

#[derive(Default)]
pub(crate) struct BrowserRuntime {
    registry: Arc<Mutex<Registry>>,
    listener: tokio::sync::Mutex<Option<Listener>>,
}

impl BrowserRuntime {
    pub(crate) async fn configure(
        &self,
        address: SocketAddr,
        token: Option<&str>,
        agent_id: &str,
    ) -> anyhow::Result<()> {
        let mut current = self.listener.lock().await;
        let Some(token) = token else {
            if let Some(listener) = current.take() {
                listener.stop.cancel();
                listener.task.await.context("stop browser listener")?;
            }
            return Ok(());
        };
        let config = ListenerConfig {
            address,
            token_digest: Sha256::digest(token.as_bytes()).into(),
            agent_id: agent_id.into(),
        };
        if current
            .as_ref()
            .is_some_and(|listener| listener.config == config)
        {
            return Ok(());
        }
        let socket = if current
            .as_ref()
            .is_some_and(|listener| listener.config.address == address)
        {
            let listener = current
                .take()
                .expect("same address has an existing listener");
            listener.stop.cancel();
            listener.task.await.context("stop browser listener")?
        } else {
            let socket = TcpListener::bind(address)
                .await
                .with_context(|| format!("bind browser listener on {address}"))?;
            if let Some(listener) = current.take() {
                listener.stop.cancel();
                listener.task.await.context("stop browser listener")?;
            }
            socket
        };
        let stop = CancellationToken::new();
        *current = Some(Listener {
            config: config.clone(),
            task: tokio::spawn(run_listener(
                socket,
                config,
                self.registry.clone(),
                stop.clone(),
            )),
            stop,
        });
        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        if let Some(listener) = self.listener.lock().await.take() {
            listener.stop.cancel();
            let _ = listener.task.await;
        }
    }

    pub(crate) async fn list(&self) -> BrowserList {
        let listener_ready = self
            .listener
            .lock()
            .await
            .as_ref()
            .is_some_and(|listener| !listener.task.is_finished());
        let registry = self.registry.lock();
        let mut browsers: Vec<_> = registry
            .connections
            .values()
            .map(|connection| &connection.info)
            .chain(registry.offline_browsers.values())
            .cloned()
            .collect();
        browsers.sort_unstable_by(|first, second| first.browser_id.cmp(&second.browser_id));
        BrowserList {
            listener_ready,
            runtime_available: !registry.connections.is_empty(),
            browsers,
            leases: registry
                .leases
                .iter()
                .filter(|(_, lease)| lease.id.is_some())
                .map(|(name, lease)| LeaseInfo {
                    name: name.clone(),
                    browser_id: lease.browser_id.clone(),
                })
                .collect(),
        }
    }

    pub(crate) fn browser_info(&self, browser_id: &str) -> Option<BrowserInfo> {
        self.registry
            .lock()
            .connections
            .get(browser_id)
            .map(|connection| connection.info.clone())
    }

    pub(crate) fn lease_browser_info(&self, name: &str) -> Option<BrowserInfo> {
        let registry = self.registry.lock();
        let lease = registry.leases.get(name)?;
        registry
            .connections
            .get(&lease.browser_id)
            .map(|connection| connection.info.clone())
    }

    pub(crate) async fn acquire(
        &self,
        browser_id: &str,
        name: &str,
        idle_timeout_seconds: u64,
    ) -> Result<BrowserInfo, BrowserError> {
        let (connection, reply) = {
            let mut registry = self.registry.lock();
            let connection = registry
                .connections
                .get(browser_id)
                .cloned()
                .ok_or_else(|| {
                    BrowserError::new("browser_not_connected", "Selected browser is not connected")
                })?;
            if let Some(lease) = registry.leases.get_mut(name) {
                if lease.connection != connection.id || lease.acquiring {
                    return Err(BrowserError::new(
                        "browser_busy",
                        "Lease name is already in use",
                    ));
                }
                lease.acquiring = true;
            } else {
                registry.leases.insert(
                    name.into(),
                    Lease {
                        connection: connection.id,
                        browser_id: browser_id.into(),
                        id: None,
                        acquiring: true,
                    },
                );
            }
            let reply = connection.send(
                json!({"type":"acquire", "name":name, "idleTimeoutSeconds":idle_timeout_seconds}),
                RequestEffect::Acquire(name.into()),
            );
            if reply.is_err() {
                registry.acquire_failed(name, connection.id);
            }
            (connection, reply?)
        };
        connection.wait(reply, CONTROL_TIMEOUT).await?;
        Ok(connection.info)
    }

    pub(crate) async fn repl(
        &self,
        name: &str,
        code: &str,
        timeout_ms: u64,
    ) -> Result<CallToolResult, BrowserError> {
        let reply = self
            .command(
                name,
                json!({"type":"repl", "code":code, "timeoutMs":timeout_ms}),
                false,
                Duration::from_millis(timeout_ms) + CONTROL_TIMEOUT,
            )
            .await?;
        serde_json::from_value(reply).map_err(|_| BrowserError::invalid_result())
    }

    pub(crate) async fn reset(&self, name: &str) -> Result<(), BrowserError> {
        self.command(name, json!({"type":"reset"}), false, CONTROL_TIMEOUT)
            .await
            .map(|_| ())
    }

    pub(crate) async fn release(&self, name: &str) -> Result<bool, BrowserError> {
        match self
            .command(name, json!({"type":"release"}), true, CONTROL_TIMEOUT)
            .await
        {
            Ok(reply) => reply
                .get("released")
                .and_then(Value::as_bool)
                .ok_or_else(BrowserError::invalid_result),
            Err(error) if error.code == "browser_lease_expired" => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn command(
        &self,
        name: &str,
        mut message: Value,
        release: bool,
        timeout: Duration,
    ) -> Reply {
        let (connection, reply) = {
            let registry = self.registry.lock();
            let lease = registry
                .leases
                .get(name)
                .ok_or_else(BrowserError::expired)?;
            let id = lease.id.as_ref().ok_or_else(|| {
                BrowserError::new("browser_busy", "Browser lease is being acquired")
            })?;
            let connection = registry
                .connections
                .get(&lease.browser_id)
                .cloned()
                .ok_or_else(BrowserError::disconnected)?;
            message["leaseId"] = json!(id);
            let reply = connection.send(
                message,
                RequestEffect::Lease {
                    id: id.clone(),
                    release,
                },
            )?;
            (connection, reply)
        };
        connection.wait(reply, timeout).await
    }
}

impl Connection {
    fn send(
        &self,
        mut message: Value,
        effect: RequestEffect,
    ) -> Result<oneshot::Receiver<Reply>, BrowserError> {
        if self.stop.is_cancelled() {
            return Err(BrowserError::disconnected());
        }
        message["requestId"] = json!(Uuid::new_v4().to_string());
        let (reply, receive) = oneshot::channel();
        self.requests
            .try_send(Request {
                message,
                effect,
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    BrowserError::new("browser_busy", "Browser request queue is full")
                }
                mpsc::error::TrySendError::Closed(_) => BrowserError::disconnected(),
            })?;
        Ok(receive)
    }

    async fn wait(&self, reply: oneshot::Receiver<Reply>, timeout: Duration) -> Reply {
        match tokio::time::timeout(timeout, reply).await {
            Ok(result) => result.unwrap_or_else(|_| Err(BrowserError::disconnected())),
            Err(_) => {
                self.stop.cancel();
                Err(BrowserError::new(
                    "browser_response_timeout",
                    "Browser did not respond",
                ))
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Incoming {
    Hello {
        token: String,
        browser_id: String,
        browser_name: String,
        api_version: u32,
        extension_version: String,
    },
    Ping,
    Status {
        busy: bool,
    },
    Result {
        request_id: String,
        #[serde(flatten)]
        reply: WireReply,
    },
    LeaseEnded {
        lease_id: String,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WireReply {
    Success {
        result: serde_json::Map<String, Value>,
    },
    Failure {
        error: BrowserError,
    },
}

async fn run_listener(
    socket: TcpListener,
    config: ListenerConfig,
    registry: Arc<Mutex<Registry>>,
    stop: CancellationToken,
) -> TcpListener {
    let config = Arc::new(config);
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {},
            accepted = socket.accept() => match accepted {
                Ok((socket, _)) => {
                    connections.spawn(serve_connection(socket, config.clone(), registry.clone(), stop.child_token()));
                }
                Err(error) => {
                    crate::utils::log_warn(format!("browser listener accept failed: {error}"));
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }
    connections.shutdown().await;
    *registry.lock() = Registry::default();
    socket
}

async fn serve_connection(
    stream: TcpStream,
    config: Arc<ListenerConfig>,
    registry: Arc<Mutex<Registry>>,
    stop: CancellationToken,
) {
    let handshake = async {
        let websocket_config = WebSocketConfig::default()
            .max_message_size(Some(16 * 1024 * 1024))
            .max_frame_size(Some(16 * 1024 * 1024));
        let mut socket =
            tokio_tungstenite::accept_async_with_config(stream, Some(websocket_config))
                .await
                .ok()?;
        let hello = match socket.next().await {
            Some(Ok(Message::Text(text))) => serde_json::from_str::<Incoming>(&text).ok(),
            _ => None,
        };
        let info = validate_hello(hello, &config);
        let response = match &info {
            Ok(_) => json!({"type":"welcome", "apiVersion":API_VERSION, "agentId":config.agent_id}),
            Err(error) => json!({"type":"error", "error":error}),
        };
        socket
            .send(Message::Text(response.to_string().into()))
            .await
            .ok()?;
        Some((socket, info.ok()?))
    };
    let result = tokio::select! {
        _ = stop.cancelled() => None,
        result = tokio::time::timeout(Duration::from_secs(10), handshake) => result.ok().flatten(),
    };
    let Some((mut socket, info)) = result else {
        return;
    };
    let (requests, mut incoming_requests) = mpsc::channel::<Request>(32);
    let connection = Connection {
        id: Uuid::new_v4(),
        info,
        requests,
        stop: stop.clone(),
    };
    {
        let mut registry = registry.lock();
        registry
            .offline_browsers
            .remove(&connection.info.browser_id);
        if let Some(old) = registry
            .connections
            .insert(connection.info.browser_id.clone(), connection.clone())
        {
            registry.disconnect(&old);
            old.stop.cancel();
        }
    }
    let session = async {
        let mut pending = BTreeMap::<String, Request>::new();
        loop {
            tokio::select! {
                request = incoming_requests.recv(), if pending.len() < 32 => {
                    let Some(request) = request else { break };
                    let id = request.message["requestId"].as_str().expect("assigned request ID").to_owned();
                    if socket.send(Message::Text(request.message.to_string().into())).await.is_err() { break }
                    pending.insert(id, request);
                }
                message = tokio::time::timeout(Duration::from_secs(90), socket.next()) => {
                    match message {
                        Ok(Some(Ok(Message::Text(text)))) => {
                            let Ok(message) = serde_json::from_str::<Incoming>(&text) else { break };
                            match message {
                                Incoming::Ping => {
                                    if socket.send(Message::Text(r#"{"type":"pong"}"#.into())).await.is_err() { break }
                                }
                                Incoming::Status { busy } => {
                                    if let Some(entry) = registry.lock().connections.get_mut(&connection.info.browser_id).filter(|entry| entry.id == connection.id) {
                                        entry.info.busy = busy;
                                    }
                                }
                                Incoming::Result { request_id, reply } => {
                                    if let Some(request) = pending.remove(&request_id) {
                                        let reply = match reply {
                                            WireReply::Success { result } => Ok(Value::Object(result)),
                                            WireReply::Failure { error } => Err(error),
                                        };
                                        let outcome = request.effect.apply(&mut registry.lock(), &connection, reply);
                                        let _ = request.reply.send(outcome);
                                    }
                                }
                                Incoming::LeaseEnded { lease_id } => registry.lock().lease_ended(connection.id, &lease_id),
                                Incoming::Hello { .. } => break,
                            }
                        }
                        Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => {
                            if socket.flush().await.is_err() { break }
                        }
                        _ => break,
                    }
                }
            }
        }
    };
    tokio::select! {
        biased;
        _ = stop.cancelled() => {},
        _ = session => {},
    }
    registry.lock().disconnect(&connection);
}

fn validate_hello(
    hello: Option<Incoming>,
    config: &ListenerConfig,
) -> Result<BrowserInfo, BrowserError> {
    let Some(Incoming::Hello {
        token,
        browser_id,
        browser_name,
        api_version,
        extension_version,
    }) = hello
    else {
        return Err(BrowserError::new(
            "browser_hello_required",
            "Expected a browser hello message",
        ));
    };
    let digest = Sha256::digest(token.as_bytes());
    let difference = digest
        .iter()
        .zip(config.token_digest)
        .fold(0u8, |value, (actual, expected)| value | (actual ^ expected));
    if difference != 0 {
        return Err(BrowserError::new(
            "browser_auth_failed",
            "Pairing token was rejected",
        ));
    }
    if api_version != API_VERSION {
        return Err(BrowserError::new(
            "browser_api_version_unsupported",
            "Browser API version is unsupported",
        ));
    }
    if [&browser_id, &browser_name, &extension_version]
        .iter()
        .any(|value| value.trim().is_empty() || value.len() > 256)
    {
        return Err(BrowserError::new(
            "browser_hello_invalid",
            "Browser identity fields are invalid",
        ));
    }
    Ok(BrowserInfo {
        browser_id,
        browser_name,
        extension_version,
        api_version,
        connected: true,
        busy: true,
    })
}
