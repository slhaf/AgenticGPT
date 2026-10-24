//! Private, bounded CLI transport. Raw provider output never becomes an error.
use super::{Error, Result};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use tokio_util::sync::CancellationToken;

pub(super) const VERSION: &str = "2026.10.0";
const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

pub(super) struct Cli {
    pub(super) root: PathBuf,
    stop: CancellationToken,
    active: parking_lot::Mutex<usize>,
    idle: tokio::sync::Notify,
    #[cfg(test)]
    pub(super) binary: parking_lot::RwLock<PathBuf>,
}
impl Cli {
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            stop: CancellationToken::new(),
            active: parking_lot::Mutex::new(0),
            idle: tokio::sync::Notify::new(),
            #[cfg(test)]
            binary: parking_lot::RwLock::new("bw".into()),
        }
    }
    pub(super) async fn run(
        self: &Arc<Self>,
        args: &[&str],
        env: BTreeMap<String, String>,
        input: Option<Vec<u8>>,
        write: bool,
        sync: bool,
    ) -> Result<Vec<u8>> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let cli = self.clone();
        {
            let mut active = self.active.lock();
            if self.stop.is_cancelled() {
                return Err(Error::Unavailable);
            }
            *active += 1;
        }
        // Detached from request cancellation: the owned task always reaps the child.
        tokio::spawn(async move {
            let result = cli.execute(&args, env, input, write, sync).await;
            *cli.active.lock() -= 1;
            cli.idle.notify_waiters();
            result
        })
        .await
        .map_err(|_| Error::Unavailable)?
    }
    async fn execute(
        &self,
        args: &[String],
        env: BTreeMap<String, String>,
        input: Option<Vec<u8>>,
        write: bool,
        sync: bool,
    ) -> Result<Vec<u8>> {
        #[cfg(test)]
        let binary = self.binary.read().clone();
        #[cfg(not(test))]
        let binary = "bw";
        let mut command = Command::new(binary);
        command
            .args(args)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("home/config"))
            .env("XDG_CACHE_HOME", self.root.join("home/cache"))
            .env("XDG_DATA_HOME", self.root.join("home/data"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("BITWARDENCLI_APPDATA_DIR", &self.root)
            .env("BW_NOINTERACTION", "true")
            .env("LANG", "C.UTF-8")
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Network configuration is inherited explicitly; no ambient BW/session/Node hooks.
        for key in [
            "PATH",
            "HTTPS_PROXY",
            "HTTP_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "https_proxy",
            "http_proxy",
            "all_proxy",
            "no_proxy",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NODE_EXTRA_CA_CERTS",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|_| Error::Unavailable)?;
        let process_id = child.id();
        let mut stdin = child.stdin.take().ok_or(Error::Unavailable)?;
        let stdout = child.stdout.take().ok_or(Error::Unavailable)?;
        let stderr = child.stderr.take().ok_or(Error::Unavailable)?;
        let work = async {
            let send = async {
                if let Some(bytes) = input {
                    stdin
                        .write_all(&bytes)
                        .await
                        .map_err(|_| Error::Unavailable)?;
                }
                drop(stdin);
                Ok::<_, Error>(())
            };
            let (_, out, err) = tokio::try_join!(send, bounded(stdout), bounded(stderr))
                .map_err(|e| if write { Error::WriteUnknown } else { e })?;
            let status = child.wait().await.map_err(|_| {
                if write {
                    Error::WriteUnknown
                } else {
                    Error::Unavailable
                }
            })?;
            if status.success() {
                Ok(out)
            } else {
                Err(classify(&err, &out, write))
            }
        };
        let result = tokio::select! {
            result=async { if sync {work.await} else {tokio::time::timeout(Duration::from_secs(20),work).await.unwrap_or(Err(if write {Error::WriteUnknown} else {Error::Unavailable}))} }=>result,
            _=self.stop.cancelled()=>Err(if write {Error::WriteUnknown} else {Error::Unavailable}),
        };
        // Kill descendants in the private process group, then reap even on cap/timeout/cancel.
        if result.is_err() {
            #[cfg(unix)]
            if let Some(pid) = process_id {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill().await;
        }
        let _ = child.wait().await;
        result
    }
    pub(super) async fn stop(&self) {
        self.stop.cancel();
        loop {
            let notified = self.idle.notified();
            if *self.active.lock() == 0 {
                break;
            }
            notified.await;
        }
    }
}
async fn bounded(mut stream: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|_| Error::Unavailable)?;
        if n == 0 {
            return Ok(bytes);
        }
        if bytes.len() + n > OUTPUT_LIMIT {
            return Err(Error::Unavailable);
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}
fn classify(stderr: &[u8], stdout: &[u8], write: bool) -> Error {
    // Markers are inspected transiently; they are never returned or persisted.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(stderr),
        String::from_utf8_lossy(stdout)
    )
    .to_ascii_lowercase();
    if text.contains("invalid master password")
        || text.contains("vault is locked")
        || text.contains("vault is not unlocked")
    {
        return Error::Locked;
    }
    if text.contains("invalid api key")
        || text.contains("invalid client")
        || text.contains("invalid_grant")
        || text.contains("not logged in")
        || text.contains("unauthorized")
        || text.contains("401 unauthorized")
    {
        return Error::Auth;
    }
    if write {
        if text.contains("not found") {
            return Error::NotFound;
        }
        if text.contains("400 bad request")
            || text.contains("403 forbidden")
            || text.contains("no permission")
        {
            return Error::Unavailable;
        }
        return Error::WriteUnknown;
    }
    if [
        "certificate",
        "tls",
        "ssl",
        "http status",
        "status code",
        "429",
        "500",
        "502",
        "503",
        "504",
    ]
    .iter()
    .any(|v| text.contains(v))
    {
        return Error::Unavailable;
    }
    if [
        "getaddrinfo enotfound",
        "getaddrinfo eai_again",
        "connect econnrefused",
        "connect enetunreach",
        "connect ehostunreach",
        "connect etimedout",
        "read econnreset",
        "socket hang up",
    ]
    .iter()
    .any(|v| text.contains(v))
    {
        return Error::Network;
    }
    Error::Unavailable
}
pub(super) fn json(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice(bytes).map_err(|_| Error::Unavailable)
}
pub(super) fn private_dir(path: &Path) -> anyhow::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_dir() {
            anyhow::bail!("secret_private_state_unavailable");
        }
    }
    std::fs::create_dir_all(path)
        .map_err(|_| anyhow::anyhow!("secret_private_state_unavailable"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| anyhow::anyhow!("secret_private_state_unavailable"))?;
    }
    Ok(())
}
