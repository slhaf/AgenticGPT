use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use rmcp::{model::CallToolRequestParams, ServiceExt};
use serde_json::{json, Map, Value};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::config::Config;
use crate::state::AppState;
use crate::stdio_server::{AgentMcpServer, RequestIngress};
use crate::utils::{agentic_home, log_warn};

const MAX_SOCKET_PATH_BYTES: usize = 100;
const MAX_LOCAL_CONNECTIONS: usize = 16;
const LOCAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct LocalMcpListener {
    listener: UnixListener,
    path: PathBuf,
    _guard: SocketGuard,
}

impl LocalMcpListener {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) async fn serve(self, state: AppState) -> Result<()> {
        let permits = Arc::new(Semaphore::new(MAX_LOCAL_CONNECTIONS));
        loop {
            let (stream, _) = self
                .listener
                .accept()
                .await
                .map_err(|_| anyhow!("local_mcp_accept_failed"))?;
            if stream
                .peer_cred()
                .map(|credentials| credentials.uid() != effective_uid())
                .unwrap_or(true)
            {
                log_warn("local MCP connection rejected; peer UID mismatch".to_string());
                continue;
            }
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                log_warn("local MCP connection rejected; capacity exhausted".to_string());
                continue;
            };
            let server = AgentMcpServer::with_ingress(state.clone(), RequestIngress::LocalUnix);
            tokio::spawn(async move {
                let _permit = permit;
                let outcome = async {
                    let running = server.serve(stream).await?;
                    let _ = running.waiting().await?;
                    anyhow::Result::<()>::Ok(())
                }
                .await;
                if let Err(error) = outcome {
                    log_warn(format!(
                        "local MCP connection ended with error; errorCode={}",
                        bounded_error_code(&error.to_string())
                    ));
                }
            });
        }
    }
}

pub(crate) async fn bind(agent_id: &str) -> Result<LocalMcpListener> {
    let path = socket_path(agent_id)?;
    bind_at(path).await
}

pub(crate) fn socket_path(agent_id: &str) -> Result<PathBuf> {
    validate_agent_id(agent_id)?;
    let path = agentic_home()?
        .join("runtime")
        .join("agent")
        .join(agent_id)
        .join("mcp.sock");
    validate_socket_path(&path)?;
    Ok(path)
}

pub(crate) fn status(agent_id: &str, enabled: bool) -> Value {
    if !enabled {
        return json!({
            "enabled": false,
            "transport": "unix",
            "status": "disabled",
            "path": null,
        });
    }
    match socket_path(agent_id) {
        Ok(path) => {
            let ready = local_socket_is_private(&path);
            json!({
                "enabled": true,
                "transport": "unix",
                "status": if ready { "ready" } else { "unavailable" },
                "path": path.to_string_lossy(),
            })
        }
        Err(_) => json!({
            "enabled": true,
            "transport": "unix",
            "status": "unavailable",
            "path": null,
        }),
    }
}

pub(crate) async fn list_tools(config_path: &Path) -> Result<Value> {
    let client = connect(config_path).await?;
    let tools = client.list_all_tools().await?;
    let value = serde_json::to_value(tools)?;
    let _ = client.cancel().await;
    Ok(value)
}

pub(crate) async fn call_tool(
    config_path: &Path,
    tool: String,
    arguments: Map<String, Value>,
) -> Result<Value> {
    let client = connect(config_path).await?;
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
        .await?;
    let value = serde_json::to_value(result)?;
    let _ = client.cancel().await;
    Ok(value)
}

async fn connect(
    config_path: &Path,
) -> Result<rmcp::service::RunningService<rmcp::RoleClient, ()>> {
    let config = Config::load(config_path)?;
    let path = socket_path(&config.agent_id)?;
    let stream = timeout(LOCAL_CONNECT_TIMEOUT, UnixStream::connect(&path))
        .await
        .map_err(|_| anyhow!("local_mcp_connect_timeout"))?
        .map_err(|_| anyhow!("local_mcp_unavailable"))?;
    let credentials = stream
        .peer_cred()
        .map_err(|_| anyhow!("local_mcp_peer_credentials_unavailable"))?;
    if credentials.uid() != effective_uid() {
        return Err(anyhow!("local_mcp_peer_uid_mismatch"));
    }
    ().serve(stream)
        .await
        .map_err(|_| anyhow!("local_mcp_initialize_failed"))
}

async fn bind_at(path: PathBuf) -> Result<LocalMcpListener> {
    validate_socket_path(&path)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("local_mcp_runtime_path_invalid"))?;
    fs::create_dir_all(parent).map_err(|_| anyhow!("local_mcp_runtime_directory_unavailable"))?;
    let parent_metadata =
        fs::symlink_metadata(parent).map_err(|_| anyhow!("local_mcp_runtime_path_unsafe"))?;
    if !parent_metadata.file_type().is_dir() || parent_metadata.uid() != effective_uid() {
        return Err(anyhow!("local_mcp_runtime_path_unsafe"));
    }
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
        .map_err(|_| anyhow!("local_mcp_runtime_permissions_failed"))?;
    remove_stale_socket(&path).await?;
    let listener = UnixListener::bind(&path).map_err(|error| {
        if error.kind() == io::ErrorKind::AddrInUse {
            anyhow!("local_mcp_socket_in_use")
        } else {
            anyhow!("local_mcp_bind_failed")
        }
    })?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|_| anyhow!("local_mcp_socket_permissions_failed"))?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| anyhow!("local_mcp_socket_unavailable"))?;
    let guard = SocketGuard {
        path: path.clone(),
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    Ok(LocalMcpListener {
        listener,
        path,
        _guard: guard,
    })
}

async fn remove_stale_socket(path: &Path) -> Result<()> {
    let before = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(anyhow!("local_mcp_socket_metadata_failed")),
    };
    if !before.file_type().is_socket() || before.uid() != effective_uid() {
        return Err(anyhow!("local_mcp_socket_path_unsafe"));
    }
    match UnixStream::connect(path).await {
        Ok(_) => return Err(anyhow!("local_mcp_socket_in_use")),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) => {}
        Err(_) => return Err(anyhow!("local_mcp_socket_probe_failed")),
    }
    let after = fs::symlink_metadata(path).map_err(|_| anyhow!("local_mcp_socket_changed"))?;
    if after.dev() != before.dev() || after.ino() != before.ino() || !after.file_type().is_socket()
    {
        return Err(anyhow!("local_mcp_socket_changed"));
    }
    fs::remove_file(path).map_err(|_| anyhow!("local_mcp_stale_socket_cleanup_failed"))
}

fn local_socket_is_private(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(parent_metadata) = fs::symlink_metadata(parent) else {
        return false;
    };
    let Ok(socket_metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    parent_metadata.file_type().is_dir()
        && parent_metadata.uid() == effective_uid()
        && parent_metadata.permissions().mode() & 0o777 == 0o700
        && socket_metadata.file_type().is_socket()
        && socket_metadata.uid() == effective_uid()
        && socket_metadata.permissions().mode() & 0o777 == 0o600
}

fn validate_agent_id(agent_id: &str) -> Result<()> {
    if agent_id.is_empty()
        || !agent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(anyhow!("local_mcp_agent_id_invalid"));
    }
    Ok(())
}

fn validate_socket_path(path: &Path) -> Result<()> {
    if path.as_os_str().as_bytes().len() > MAX_SOCKET_PATH_BYTES {
        return Err(anyhow!("local_mcp_socket_path_too_long"));
    }
    Ok(())
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not mutate memory.
    unsafe { libc::geteuid() }
}

fn bounded_error_code(value: &str) -> String {
    value
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .find(|part| !part.is_empty())
        .unwrap_or("local_mcp_connection_failed")
        .chars()
        .take(64)
        .collect()
}

struct SocketGuard {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
            && fs::remove_file(&self.path).is_ok()
        {
            if let Some(parent) = self.path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("agentic-local-mcp-{}", uuid::Uuid::new_v4()))
            .join(name)
    }

    #[tokio::test]
    async fn bind_is_private_rejects_second_listener_and_cleans_on_drop() {
        let path = temp_path("mcp.sock");
        let first = bind_at(path.clone()).await.unwrap();
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(bind_at(path.clone())
            .await
            .err()
            .unwrap()
            .to_string()
            .starts_with("local_mcp_socket_in_use"));
        let parent = path.parent().unwrap().to_path_buf();
        drop(first);
        assert!(!path.exists());
        assert!(!parent.exists());
    }

    #[tokio::test]
    async fn bind_replaces_owned_stale_socket_but_rejects_regular_file() {
        let path = temp_path("mcp.sock");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(stale);
        let listener = bind_at(path.clone()).await.unwrap();
        drop(listener);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not a socket").unwrap();
        assert!(bind_at(path.clone())
            .await
            .err()
            .unwrap()
            .to_string()
            .starts_with("local_mcp_socket_path_unsafe"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn bind_rejects_symlink_runtime_directory() {
        let root = std::env::temp_dir().join(format!(
            "agentic-local-mcp-symlink-{}",
            uuid::Uuid::new_v4()
        ));
        let target = root.join("target");
        let linked = root.join("linked");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &linked).unwrap();
        let error = bind_at(linked.join("mcp.sock"))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.starts_with("local_mcp_runtime_path_unsafe"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn socket_path_rejects_invalid_identity_and_oversized_path() {
        assert!(socket_path("bad/id").is_err());
        let long = PathBuf::from("/tmp").join("x".repeat(MAX_SOCKET_PATH_BYTES));
        assert!(validate_socket_path(&long).is_err());
    }
}
