//! Internal trust, acquisition, and materialization of the Browser runtime.
mod browser_distribution_verify;

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tar::EntryType;
use tokio::io::AsyncWriteExt;
use xz2::read::XzDecoder;

use crate::browser_runtime::{
    derive_docs_root, derive_trusted_code_paths, BrowserRuntimeDescriptor,
};

const MAX_ENTRIES: usize = 10_000;
const MAX_TOTAL: u64 = 768 * 1024 * 1024;
const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 64 * 1024;
const AR_MAGIC: &[u8] = b"!<arch>\n";
const LOCK_WAIT_TOTAL: Duration = Duration::from_secs(10 * 60);
const LOCK_POLL: Duration = Duration::from_millis(50);
const LOCK_STALE_AFTER: Duration = Duration::from_secs(30 * 60);
const MAX_INRELEASE_BYTES: u64 = 256 * 1024;
const MAX_PACKAGES_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DEB_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PACKAGES_PARAGRAPHS: usize = 10_000;
const MAX_PACKAGES_FIELDS: usize = 128;
const MAX_PACKAGES_FIELD_BYTES: usize = 64 * 1024;
const OPENAI_REPOSITORY_ORIGIN: &str = "https://persistent.oaistatic.com/codex-app-prod/linux/deb/";
const OPENAI_REPOSITORY_HOST: &str = "persistent.oaistatic.com";
const INRELEASE_PATH: &str = "dists/stable/InRelease";
const STABLE_DISTRIBUTION_PREFIX: &str = "dists/stable/";
const PINNED_REPOSITORY_FINGERPRINT: &str = "3BFA0E4AE8B8CC16A2D9BA684A3B4A566C4660E4";
const PINNED_REPOSITORY_KEY: &str = include_str!("../../assets/browser/openai-linux-repo-key.asc");
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const PROVISION_OVERALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedBrowserPackage {
    pub(crate) deb_path: PathBuf,
    pub(crate) deb_sha256: String,
    pub(crate) app_version: String,
    pub(crate) channel: String,
    pub(crate) target: String,
    pub(crate) codex_home: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeMetadata {
    schema_version: u32,
    target: String,
    app_version: String,
    channel: String,
    package_sha256: String,
    installed_at: String,
    cua_node_root: String,
    chrome_root: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActiveManifest {
    target: String,
    app_version: String,
    package_sha256: String,
}

fn error(code: &'static str) -> anyhow::Error {
    anyhow!(code)
}
fn sha256(value: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(value);
    format!("{:x}", h.finalize())
}
fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-'))
}
fn identity(value: &str, code: &'static str) -> Result<()> {
    if valid_component(value) && value.len() <= 200 {
        Ok(())
    } else {
        Err(error(code))
    }
}
fn normalize_hash(value: &str) -> Result<String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        Ok(value.to_string())
    } else {
        Err(error("browser_runtime_package_hash_invalid"))
    }
}
fn host_target(target: &str) -> bool {
    matches!(
        (std::env::consts::OS, std::env::consts::ARCH, target),
        ("linux", "x86_64", "linux-x64") | ("linux", "aarch64", "linux-arm64")
    )
}
#[derive(Debug)]
pub(crate) struct BrowserDistributionResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

pub(crate) type BrowserDistributionFuture =
    Pin<Box<dyn Future<Output = Result<BrowserDistributionResponse>> + Send>>;

/// The production implementation fixes the repository origin. Tests can
/// inject a local implementation without changing that production policy.
pub(crate) trait BrowserDistributionFetcher: Send + Sync {
    fn fetch_bytes(&self, path: String, max_bytes: u64) -> BrowserDistributionFuture;
    fn stream_file(
        &self,
        path: String,
        destination: tokio::fs::File,
        expected_size: u64,
        expected_sha256: String,
    ) -> BrowserDistributionFuture;
}

struct ReqwestBrowserDistributionFetcher {
    client: reqwest::Client,
}

impl ReqwestBrowserDistributionFetcher {
    fn new() -> Result<Arc<Self>> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_REQUEST_TIMEOUT)
            .build()
            .map_err(|_| error("browser_runtime_acquisition_http_unavailable"))?;
        Ok(Arc::new(Self { client }))
    }
}

impl BrowserDistributionFetcher for ReqwestBrowserDistributionFetcher {
    fn fetch_bytes(&self, path: String, max_bytes: u64) -> BrowserDistributionFuture {
        let client = self.client.clone();
        Box::pin(async move { fetch_http_bytes(client, path, max_bytes).await })
    }

    fn stream_file(
        &self,
        path: String,
        destination: tokio::fs::File,
        expected_size: u64,
        expected_sha256: String,
    ) -> BrowserDistributionFuture {
        let client = self.client.clone();
        Box::pin(async move {
            stream_http_file(client, path, destination, expected_size, expected_sha256).await
        })
    }
}

fn fixed_repository_url(path: &str) -> Result<reqwest::Url> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('?')
        || path.contains('#')
        || path.contains(':')
        || path.contains("://")
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(error("browser_runtime_acquisition_url_invalid"));
    }
    let base = reqwest::Url::parse(OPENAI_REPOSITORY_ORIGIN)
        .map_err(|_| error("browser_runtime_acquisition_url_invalid"))?;
    let url = base
        .join(path)
        .map_err(|_| error("browser_runtime_acquisition_url_invalid"))?;
    if url.scheme() != "https"
        || url.host_str() != Some(OPENAI_REPOSITORY_HOST)
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with(base.path())
    {
        return Err(error("browser_runtime_acquisition_url_invalid"));
    }
    Ok(url)
}

fn packages_fetch_path(
    index: &browser_distribution_verify::AuthenticatedPackagesIndex,
) -> Result<String> {
    let path = format!("{STABLE_DISTRIBUTION_PREFIX}{}", index.path);
    fixed_repository_url(&path)?;
    Ok(path)
}

async fn fetch_http_bytes(
    client: reqwest::Client,
    path: String,
    max_bytes: u64,
) -> Result<BrowserDistributionResponse> {
    let url = fixed_repository_url(&path)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| error("browser_runtime_acquisition_http_failed"))?;
    fetch_http_response(response, max_bytes).await
}

async fn fetch_http_response(
    response: reqwest::Response,
    max_bytes: u64,
) -> Result<BrowserDistributionResponse> {
    if response.status() != StatusCode::OK {
        return Err(error("browser_runtime_acquisition_http_status"));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(error("browser_runtime_acquisition_response_size_limit"));
    }
    let status = response.status().as_u16();
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| error("browser_runtime_acquisition_http_failed"))?;
        let next = (body.len() as u64)
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| error("browser_runtime_acquisition_response_size_limit"))?;
        if next > max_bytes {
            return Err(error("browser_runtime_acquisition_response_size_limit"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(BrowserDistributionResponse { status, body })
}
async fn stream_http_file(
    client: reqwest::Client,
    path: String,
    destination: tokio::fs::File,
    expected_size: u64,
    expected_sha256: String,
) -> Result<BrowserDistributionResponse> {
    let url = fixed_repository_url(&path)?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| error("browser_runtime_acquisition_http_failed"))?;
    stream_http_response(response, destination, expected_size, expected_sha256).await
}

async fn stream_http_response(
    response: reqwest::Response,
    destination: tokio::fs::File,
    expected_size: u64,
    expected_sha256: String,
) -> Result<BrowserDistributionResponse> {
    if response.status() != StatusCode::OK {
        return Err(error("browser_runtime_acquisition_http_status"));
    }
    if response
        .content_length()
        .is_some_and(|length| length > expected_size || length > MAX_DEB_BYTES)
    {
        return Err(error("browser_runtime_acquisition_download_oversize"));
    }
    let mut output = destination;
    let mut stream = response.bytes_stream();
    let mut total = 0u64;
    let mut digest = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| error("browser_runtime_acquisition_http_failed"))?;
        let next = total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| error("browser_runtime_acquisition_download_oversize"))?;
        if next > expected_size || next > MAX_DEB_BYTES {
            return Err(error("browser_runtime_acquisition_download_oversize"));
        }
        output
            .write_all(&chunk)
            .await
            .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
        digest.update(&chunk);
        total = next;
    }
    if total != expected_size {
        return Err(error("browser_runtime_acquisition_download_size_mismatch"));
    }
    if format!("{:x}", digest.finalize()) != expected_sha256 {
        return Err(error("browser_runtime_acquisition_download_hash_mismatch"));
    }
    output
        .sync_all()
        .await
        .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
    Ok(BrowserDistributionResponse {
        status: StatusCode::OK.as_u16(),
        body: Vec::new(),
    })
}

async fn fetch_bounded_bytes(
    fetcher: Arc<dyn BrowserDistributionFetcher>,
    path: &str,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    let response = fetcher.fetch_bytes(path.to_owned(), max_bytes).await?;
    if response.status != StatusCode::OK.as_u16() {
        return Err(error("browser_runtime_acquisition_http_status"));
    }
    if response.body.len() as u64 > max_bytes {
        return Err(error("browser_runtime_acquisition_response_size_limit"));
    }
    Ok(response.body)
}

pub(crate) fn managed_browser_cache_root() -> Result<PathBuf> {
    dirs::home_dir()
        .ok_or_else(|| error("browser_runtime_acquisition_home_unavailable"))
        .map(|home| home.join(".agentic_gpt/cache/browser-runtime"))
}

pub(crate) fn managed_browser_codex_home() -> Result<PathBuf> {
    dirs::home_dir()
        .ok_or_else(|| error("browser_runtime_acquisition_home_unavailable"))
        .map(|home| home.join(".agentic_gpt/browser-runtime/codex-home"))
}

pub(crate) fn current_managed_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        _ => Err(error("browser_runtime_acquisition_target_unsupported")),
    }
}

fn apt_architecture(target: &str) -> Result<&'static str> {
    match target {
        "linux-x64" if host_target(target) => Ok("amd64"),
        "linux-arm64" if host_target(target) => Ok("arm64"),
        "linux-x64" | "linux-arm64" => Err(error("browser_runtime_acquisition_target_unsupported")),
        _ => Err(error("browser_runtime_acquisition_target_invalid")),
    }
}

fn ensure_managed_codex_home(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(error("browser_runtime_acquisition_codex_home_invalid"));
    }
    fs::create_dir_all(path)
        .map_err(|_| error("browser_runtime_acquisition_codex_home_unavailable"))?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| error("browser_runtime_acquisition_codex_home_unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(error("browser_runtime_acquisition_codex_home_unavailable"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| error("browser_runtime_acquisition_codex_home_unavailable"))?;
    }
    Ok(())
}

struct AcquisitionTempFile {
    path: PathBuf,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl AcquisitionTempFile {
    fn claim(path: PathBuf) -> Result<(Self, tokio::fs::File)> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
        let metadata = file
            .metadata()
            .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        let async_file = tokio::fs::File::from_std(file);
        Ok((
            Self {
                path,
                #[cfg(unix)]
                identity,
            },
            async_file,
        ))
    }
}

impl Drop for AcquisitionTempFile {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (metadata.dev(), metadata.ino()) != self.identity {
                return;
            }
        }
        if metadata.file_type().is_symlink() {
            return;
        }
        let _ = fs::remove_file(&self.path);
    }
}

async fn acquire_verified_package(
    root: &Path,
    codex_home: &Path,
    target: &str,
    architecture: &str,
    fetcher: Arc<dyn BrowserDistributionFetcher>,
) -> Result<(AcquisitionTempFile, VerifiedBrowserPackage)> {
    let inrelease =
        fetch_bounded_bytes(fetcher.clone(), INRELEASE_PATH, MAX_INRELEASE_BYTES).await?;
    let index = browser_distribution_verify::verify_pinned_inrelease(&inrelease, architecture)?;
    let packages_path = packages_fetch_path(&index)?;
    let packages = fetch_bounded_bytes(
        fetcher.clone(),
        &packages_path,
        MAX_PACKAGES_BYTES.min(index.size),
    )
    .await?;
    let selected = browser_distribution_verify::verify_and_select_chatgpt_package(
        &packages,
        &index,
        architecture,
    )?;
    fixed_repository_url(&selected.filename)?;

    let staging = root.join(".staging");
    fs::create_dir_all(&staging).map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
    let staging_metadata = fs::symlink_metadata(&staging)
        .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
    if staging_metadata.file_type().is_symlink() || !staging_metadata.is_dir() {
        return Err(error("browser_runtime_acquisition_temp_failed"));
    }
    let temporary = staging.join(format!("acquire-{}-{}.deb", std::process::id(), unique()));
    let (temporary_guard, temporary_file) = AcquisitionTempFile::claim(temporary.clone())?;
    let response = fetcher
        .stream_file(
            selected.filename.clone(),
            temporary_file,
            selected.size,
            selected.sha256.clone(),
        )
        .await?;
    if response.status != StatusCode::OK.as_u16() {
        return Err(error("browser_runtime_acquisition_http_status"));
    }
    let package = VerifiedBrowserPackage {
        deb_path: temporary,
        deb_sha256: selected.sha256,
        app_version: selected.version,
        channel: "prod".to_owned(),
        target: target.to_owned(),
        codex_home: codex_home.to_path_buf(),
    };
    Ok((temporary_guard, package))
}

async fn provision_managed_browser_runtime_inner(
    root: PathBuf,
    codex_home: PathBuf,
    target: String,
    fetcher: Arc<dyn BrowserDistributionFetcher>,
) -> Result<BrowserRuntimeDescriptor> {
    ensure_cache_root(&root)?;
    identity(&target, "browser_runtime_cache_target_invalid")?;
    if !host_target(&target) {
        return Err(error("browser_runtime_acquisition_target_unsupported"));
    }
    if !codex_home.is_absolute() {
        return Err(error("browser_runtime_acquisition_codex_home_invalid"));
    }
    ensure_managed_codex_home(&codex_home)?;
    let architecture = apt_architecture(&target)?;

    if let Ok(descriptor) = discover_managed_browser_runtime(&root, &target, &codex_home) {
        return Ok(descriptor);
    }
    let locks = root.join(".locks");
    fs::create_dir_all(&locks)
        .map_err(|_| error("browser_runtime_acquisition_lock_unavailable"))?;
    let lock_path = locks.join(format!("{target}.provision.lock"));
    let _provision_lock = Lock::acquire_async(&lock_path).await?;
    if let Ok(descriptor) = discover_managed_browser_runtime(&root, &target, &codex_home) {
        return Ok(descriptor);
    }

    let acquisition = tokio::time::timeout(
        PROVISION_OVERALL_TIMEOUT,
        acquire_verified_package(&root, &codex_home, &target, architecture, fetcher),
    )
    .await
    .map_err(|_| error("browser_runtime_acquisition_timeout"))?;
    let (temporary_guard, package) = acquisition?;
    let materializer_root = root.clone();
    let result = tokio::task::spawn_blocking(move || {
        let result = materialize_browser_runtime_at(&materializer_root, package);
        drop(temporary_guard);
        result
    })
    .await
    .map_err(|_| error("browser_runtime_acquisition_materializer_failed"))?;
    result.map_err(|_| error("browser_runtime_acquisition_materializer_failed"))
}

pub(crate) async fn provision_managed_browser_runtime(
    target: &str,
) -> Result<BrowserRuntimeDescriptor> {
    let root = managed_browser_cache_root()?;
    let codex_home = managed_browser_codex_home()?;
    let fetcher = ReqwestBrowserDistributionFetcher::new()?;
    provision_managed_browser_runtime_with_fetcher(&root, &codex_home, target, fetcher).await
}

pub(crate) async fn provision_managed_browser_runtime_with_fetcher(
    root: &Path,
    codex_home: &Path,
    target: &str,
    fetcher: Arc<dyn BrowserDistributionFetcher>,
) -> Result<BrowserRuntimeDescriptor> {
    provision_managed_browser_runtime_inner(
        root.to_path_buf(),
        codex_home.to_path_buf(),
        target.to_owned(),
        fetcher,
    )
    .await
}

/// Materialize into the default Agentic cache. This function never downloads or deletes input.
pub(crate) fn materialize_browser_runtime(
    package: VerifiedBrowserPackage,
) -> Result<BrowserRuntimeDescriptor> {
    let root = dirs::home_dir()
        .ok_or_else(|| error("browser_runtime_cache_home_unavailable"))?
        .join(".agentic_gpt/cache/browser-runtime");
    materialize_browser_runtime_at(&root, package)
}

pub(crate) fn materialize_browser_runtime_at(
    root: &Path,
    package: VerifiedBrowserPackage,
) -> Result<BrowserRuntimeDescriptor> {
    ensure_cache_root(root)?;
    identity(&package.target, "browser_runtime_cache_target_invalid")?;
    identity(
        &package.app_version,
        "browser_runtime_cache_version_invalid",
    )?;
    identity(&package.channel, "browser_runtime_cache_channel_invalid")?;
    if !package.codex_home.is_absolute() {
        return Err(error("browser_runtime_cache_codex_home_invalid"));
    }
    let digest = normalize_hash(&package.deb_sha256)?;
    if !host_target(&package.target) {
        return Err(error("browser_runtime_cache_target_unsupported"));
    }
    let actual = hash_file(&package.deb_path)?;
    if actual != digest {
        return Err(error("browser_runtime_package_hash_mismatch"));
    }
    let paths = CachePaths::new(root, &package.target, &package.app_version, &digest)?;
    fs::create_dir_all(root.join(".locks"))
        .map_err(|_| error("browser_runtime_cache_unavailable"))?;
    let _lock = Lock::acquire(&root.join(".locks").join(format!("{}.lock", package.target)))?;
    if let Ok(descriptor) = validate_artifact(&paths.final_dir, &package) {
        activate(root, &package.target, &package.app_version, &digest)?;
        return Ok(descriptor);
    }
    fs::create_dir_all(root.join(".staging"))
        .map_err(|_| error("browser_runtime_cache_unavailable"))?;
    let staging =
        root.join(".staging")
            .join(format!("install-{}-{}", std::process::id(), unique()));
    fs::create_dir(&staging).map_err(|_| error("browser_runtime_cache_staging_failed"))?;
    let result = (|| {
        extract(&package.deb_path, &digest, &staging)?;
        write_metadata(&staging, &package, &digest)?;
        validate_artifact(&staging, &package)?;
        let final_parent = paths
            .final_dir
            .parent()
            .ok_or_else(|| error("browser_runtime_cache_activation_failed"))?;
        fs::create_dir_all(final_parent)
            .map_err(|_| error("browser_runtime_cache_activation_failed"))?;
        let backup = final_parent.join(format!(".repair-old-{}", unique()));
        let had_old = if fs::symlink_metadata(&paths.final_dir).is_ok() {
            fs::rename(&paths.final_dir, &backup)
                .map_err(|_| error("browser_runtime_cache_activation_failed"))?;
            true
        } else {
            false
        };
        if fs::rename(&staging, &paths.final_dir).is_err() {
            if had_old {
                let _ = fs::rename(&backup, &paths.final_dir);
            }
            return Err(error("browser_runtime_cache_activation_failed"));
        }
        sync_dir(final_parent)?;
        let descriptor = validate_artifact(&paths.final_dir, &package)?;
        if had_old {
            let _ = remove_any(&backup);
        }
        activate(root, &package.target, &package.app_version, &digest)?;
        Ok(descriptor)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[derive(Debug)]
struct CachePaths {
    final_dir: PathBuf,
}
impl CachePaths {
    fn new(root: &Path, target: &str, version: &str, digest: &str) -> Result<Self> {
        Ok(Self {
            final_dir: root
                .join("artifacts")
                .join(target)
                .join(version)
                .join(digest),
        })
    }
}
fn unique() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn hash_file(path: &Path) -> Result<String> {
    let meta =
        fs::symlink_metadata(path).map_err(|_| error("browser_runtime_package_read_failed"))?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(error("browser_runtime_package_not_regular"));
    }
    let mut file = File::open(path).map_err(|_| error("browser_runtime_package_read_failed"))?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|_| error("browser_runtime_package_read_failed"))?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn ar_data(path: &Path) -> Result<(u64, u64)> {
    let mut f = File::open(path).map_err(|_| error("browser_runtime_package_read_failed"))?;
    let len = f
        .metadata()
        .map_err(|_| error("browser_runtime_package_format"))?
        .len();
    let mut magic = [0; 8];
    f.read_exact(&mut magic)
        .map_err(|_| error("browser_runtime_package_format"))?;
    if magic != AR_MAGIC {
        return Err(error("browser_runtime_package_format"));
    }
    let mut pos = 8;
    let mut binary = 0;
    let mut data = None;
    while pos < len {
        if len - pos < 60 {
            return Err(error("browser_runtime_package_format"));
        }
        f.seek(SeekFrom::Start(pos))
            .map_err(|_| error("browser_runtime_package_format"))?;
        let mut h = [0; 60];
        f.read_exact(&mut h)
            .map_err(|_| error("browser_runtime_package_format"))?;
        if &h[58..60] != b"`\n" {
            return Err(error("browser_runtime_package_format"));
        }
        let name = std::str::from_utf8(&h[..16])
            .map_err(|_| error("browser_runtime_package_format"))?
            .trim_end()
            .trim_end_matches('/');
        let size = std::str::from_utf8(&h[48..58])
            .map_err(|_| error("browser_runtime_package_format"))?
            .trim()
            .parse::<u64>()
            .map_err(|_| error("browser_runtime_package_format"))?;
        let end = pos
            .checked_add(60)
            .and_then(|v| v.checked_add(size))
            .ok_or_else(|| error("browser_runtime_package_format"))?;
        if end > len {
            return Err(error("browser_runtime_package_format"));
        }
        if name == "debian-binary" {
            if binary != 0 {
                return Err(error("browser_runtime_package_duplicate"));
            }
            if size > 1024 {
                return Err(error("browser_runtime_package_format"));
            }
            binary = 1;
            let mut bytes = vec![0; size as usize];
            f.read_exact(&mut bytes)
                .map_err(|_| error("browser_runtime_package_format"))?;
            if bytes != b"2.0\n" {
                return Err(error("browser_runtime_package_format"));
            }
        }
        if name == "data.tar.xz" {
            if data.is_some() {
                return Err(error("browser_runtime_package_duplicate"));
            }
            data = Some((pos + 60, size));
        }
        if name.starts_with("data.tar.") && name != "data.tar.xz" {
            return Err(error("browser_runtime_package_compression_unsupported"));
        }
        pos = end
            .checked_add(size & 1)
            .ok_or_else(|| error("browser_runtime_package_format"))?;
        if pos > len {
            return Err(error("browser_runtime_package_format"));
        }
    }
    if binary != 1 {
        return Err(error("browser_runtime_package_format"));
    }
    data.ok_or_else(|| error("browser_runtime_package_data_missing"))
}

fn extract(path: &Path, digest: &str, staging: &Path) -> Result<()> {
    let (offset, length) = ar_data(path)?;
    let mut f = File::open(path).map_err(|_| error("browser_runtime_package_read_failed"))?;
    f.seek(SeekFrom::Start(offset))
        .map_err(|_| error("browser_runtime_package_read_failed"))?;
    let decoder = XzDecoder::new(f.take(length));
    let mut archive = tar::Archive::new(decoder);
    let mut selected = HashSet::new();
    let mut directory_modes = Vec::new();
    let mut count = 0usize;
    let mut total = 0u64;
    for item in archive
        .entries()
        .map_err(|_| error("browser_runtime_package_extract_failed"))?
    {
        let entry = item.map_err(|_| error("browser_runtime_package_extract_failed"))?;
        let raw = entry.path_bytes();
        let raw =
            std::str::from_utf8(&raw).map_err(|_| error("browser_runtime_package_path_invalid"))?;
        let Some((resource_root, relative)) = selected_path(raw)? else {
            continue;
        };
        let root_name = match resource_root {
            ResourceRoot::CuaNode => "cua_node",
            ResourceRoot::Chrome => "chrome",
        };
        let cache_relative = PathBuf::from(root_name).join(&relative);
        if !selected.insert(cache_relative.clone()) {
            return Err(error("browser_runtime_package_duplicate"));
        }
        let destination = staging.join(&cache_relative);
        let ty = entry.header().entry_type();
        if ty == EntryType::symlink()
            || ty == EntryType::hard_link()
            || (!ty.is_file() && !ty.is_dir())
        {
            return Err(error("browser_runtime_package_entry_type_rejected"));
        }
        if relative.as_os_str().is_empty() && !ty.is_dir() {
            return Err(error("browser_runtime_package_entry_type_rejected"));
        }
        check_selected_limits(&mut count, &mut total, ty.is_file().then_some(entry.size()))?;
        if ty.is_dir() {
            fs::create_dir_all(&destination)
                .map_err(|_| error("browser_runtime_package_extract_failed"))?;
            directory_modes.push((destination, entry.header().mode().unwrap_or(0)));
            continue;
        }
        let size = entry.size();
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| error("browser_runtime_package_extract_failed"))?;
        }
        let mode = entry.header().mode().unwrap_or(0);
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|_| error("browser_runtime_package_extract_failed"))?;
        let mut limited = entry.take(MAX_FILE + 1);
        let copied = std::io::copy(&mut limited, &mut out)
            .map_err(|_| error("browser_runtime_package_extract_failed"))?;
        if copied != size {
            return Err(error("browser_runtime_package_extract_failed"));
        }
        out.sync_all()
            .map_err(|_| error("browser_runtime_package_extract_failed"))?;
        set_mode(&destination, mode)?;
    }
    directory_modes.sort_by(|(left, _), (right, _)| {
        right.components().count().cmp(&left.components().count())
    });
    for (path, mode) in directory_modes {
        set_mode(&path, mode)?;
    }
    let _ = digest;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceRoot {
    CuaNode,
    Chrome,
}

fn selected_path(raw: &str) -> Result<Option<(ResourceRoot, PathBuf)>> {
    let raw = raw.strip_prefix("./").unwrap_or(raw);
    const CUA: &str = "usr/lib/chatgpt/resources/cua_node";
    const CHROME: &str = "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome";
    let (resource_root, prefix) = if raw == CUA || raw.starts_with(&format!("{CUA}/")) {
        (ResourceRoot::CuaNode, CUA)
    } else if raw == CHROME || raw.starts_with(&format!("{CHROME}/")) {
        (ResourceRoot::Chrome, CHROME)
    } else {
        return Ok(None);
    };
    if raw.contains('\\') || raw.starts_with('/') {
        return Err(error("browser_runtime_package_path_invalid"));
    }
    let relative = raw.strip_prefix(prefix).unwrap_or_default();
    let relative = relative.strip_prefix('/').unwrap_or(relative);
    if relative.contains("//") {
        return Err(error("browser_runtime_package_path_invalid"));
    }
    let relative = relative.strip_suffix('/').unwrap_or(relative);
    let mut path = PathBuf::new();
    if !relative.is_empty() {
        for component in Path::new(relative).components() {
            match component {
                Component::Normal(value) if !value.is_empty() => path.push(value),
                _ => return Err(error("browser_runtime_package_path_invalid")),
            }
        }
        if relative.split('/').any(|part| part.is_empty()) {
            return Err(error("browser_runtime_package_path_invalid"));
        }
    }
    Ok(Some((resource_root, path)))
}

fn check_selected_limits(count: &mut usize, total: &mut u64, size: Option<u64>) -> Result<()> {
    *count = count
        .checked_add(1)
        .ok_or_else(|| error("browser_runtime_package_entry_limit"))?;
    if *count > MAX_ENTRIES {
        return Err(error("browser_runtime_package_entry_limit"));
    }
    if let Some(size) = size {
        if size > MAX_FILE {
            return Err(error("browser_runtime_package_size_limit"));
        }
        *total = total
            .checked_add(size)
            .filter(|value| *value <= MAX_TOTAL)
            .ok_or_else(|| error("browser_runtime_package_size_limit"))?;
    }
    Ok(())
}
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))
            .map_err(|_| error("browser_runtime_package_extract_failed"))?;
    }
    Ok(())
}

fn write_metadata(root: &Path, p: &VerifiedBrowserPackage, digest: &str) -> Result<()> {
    let metadata = RuntimeMetadata {
        schema_version: 1,
        target: p.target.clone(),
        app_version: p.app_version.clone(),
        channel: p.channel.clone(),
        package_sha256: digest.to_string(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        cua_node_root: "cua_node".into(),
        chrome_root: "chrome".into(),
    };
    let bytes = serde_json::to_vec_pretty(&metadata)
        .map_err(|_| error("browser_runtime_cache_metadata_failed"))?;
    let mut f = File::create(root.join("runtime.json"))
        .map_err(|_| error("browser_runtime_cache_metadata_failed"))?;
    f.write_all(&bytes)
        .map_err(|_| error("browser_runtime_cache_metadata_failed"))?;
    f.sync_all()
        .map_err(|_| error("browser_runtime_cache_metadata_failed"))?;
    Ok(())
}
fn regular(path: &Path) -> Result<fs::Metadata> {
    let m =
        fs::symlink_metadata(path).map_err(|_| error("browser_runtime_cache_artifact_invalid"))?;
    if m.file_type().is_symlink() || !m.is_file() {
        Err(error("browser_runtime_cache_artifact_invalid"))
    } else {
        Ok(m)
    }
}
fn directory(path: &Path) -> Result<()> {
    let m =
        fs::symlink_metadata(path).map_err(|_| error("browser_runtime_cache_artifact_invalid"))?;
    if m.file_type().is_symlink() || !m.is_dir() {
        Err(error("browser_runtime_cache_artifact_invalid"))
    } else {
        Ok(())
    }
}
fn validate_artifact(root: &Path, p: &VerifiedBrowserPackage) -> Result<BrowserRuntimeDescriptor> {
    directory(root)?;
    let runtime_metadata_path = checked_descendant(root, "runtime.json", RequiredKind::File)?;
    let metadata: RuntimeMetadata = serde_json::from_slice(&read_bounded_file(
        &runtime_metadata_path,
        MAX_METADATA_BYTES,
        "browser_runtime_cache_artifact_invalid",
    )?)
    .map_err(|_| error("browser_runtime_cache_artifact_invalid"))?;
    let digest = normalize_hash(&p.deb_sha256)?;
    if metadata.schema_version != 1
        || metadata.target != p.target
        || metadata.app_version != p.app_version
        || metadata.channel != p.channel
        || metadata.package_sha256 != digest
        || metadata.cua_node_root != "cua_node"
        || metadata.chrome_root != "chrome"
    {
        return Err(error("browser_runtime_cache_artifact_invalid"));
    }
    checked_descendant(root, "cua_node", RequiredKind::Directory)?;
    let chrome_root = checked_descendant(root, "chrome", RequiredKind::Directory)?;
    let node_modules =
        checked_descendant(root, "cua_node/lib/node_modules", RequiredKind::Directory)?;
    checked_descendant(root, "chrome/docs", RequiredKind::Directory)?;
    let node = checked_descendant(root, "cua_node/bin/node", RequiredKind::File)?;
    let repl = checked_descendant(root, "cua_node/bin/node_repl", RequiredKind::File)?;
    let client = checked_descendant(
        root,
        "chrome/scripts/browser-client.mjs",
        RequiredKind::File,
    )?;
    let service = checked_descendant(
        root,
        "chrome/scripts/browser-service.mjs",
        RequiredKind::File,
    )?;
    let node_m = regular(&node)?;
    let repl_m = regular(&repl)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if node_m.permissions().mode() & 0o111 == 0 || repl_m.permissions().mode() & 0o111 == 0 {
            return Err(error("browser_runtime_cache_required_executable"));
        }
    }
    let docs = derive_docs_root(&client)?;
    let mut trusted = derive_trusted_code_paths(&p.codex_home, std::slice::from_ref(&node_modules));
    if !trusted.contains(&chrome_root) {
        trusted.push(chrome_root);
    }
    Ok(BrowserRuntimeDescriptor {
        app_version: p.app_version.clone(),
        channel: p.channel.clone(),
        node_repl_path: repl,
        node_path: node,
        browser_client_path: client,
        browser_service_path: service,
        codex_home: p.codex_home.clone(),
        codex_cli_path: None,
        node_module_dirs: vec![node_modules],
        trusted_code_paths: trusted,
        docs_root: docs,
    })
}
pub(crate) fn discover_managed_browser_runtime(
    root: &Path,
    target: &str,
    codex_home: &Path,
) -> Result<BrowserRuntimeDescriptor> {
    identity(target, "browser_runtime_cache_target_invalid")?;
    if !codex_home.is_absolute() {
        return Err(error("browser_runtime_cache_codex_home_invalid"));
    }
    if !host_target(target) {
        return Err(error("browser_runtime_cache_target_unsupported"));
    }
    directory(root).map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    checked_descendant(root, "active", RequiredKind::Directory)
        .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    let active_path =
        checked_descendant(root, &format!("active/{target}.json"), RequiredKind::File)
            .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    let manifest: ActiveManifest = serde_json::from_slice(&read_bounded_file(
        &active_path,
        MAX_METADATA_BYTES,
        "browser_runtime_cache_active_invalid",
    )?)
    .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    if manifest.target != target {
        return Err(error("browser_runtime_cache_active_invalid"));
    }
    identity(
        &manifest.app_version,
        "browser_runtime_cache_active_invalid",
    )
    .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    let digest = normalize_hash(&manifest.package_sha256)
        .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    let artifact = root
        .join("artifacts")
        .join(target)
        .join(&manifest.app_version)
        .join(&digest);
    directory(&artifact).map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    let runtime_metadata_path = checked_descendant(&artifact, "runtime.json", RequiredKind::File)
        .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    let metadata: RuntimeMetadata = serde_json::from_slice(&read_bounded_file(
        &runtime_metadata_path,
        MAX_METADATA_BYTES,
        "browser_runtime_cache_active_invalid",
    )?)
    .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    if metadata.target != target
        || metadata.app_version != manifest.app_version
        || metadata.package_sha256 != digest
    {
        return Err(error("browser_runtime_cache_active_invalid"));
    }
    identity(&metadata.channel, "browser_runtime_cache_active_invalid")
        .map_err(|_| error("browser_runtime_cache_active_invalid"))?;
    let package = VerifiedBrowserPackage {
        deb_path: PathBuf::new(),
        deb_sha256: digest,
        app_version: manifest.app_version,
        channel: metadata.channel,
        target: target.to_string(),
        codex_home: codex_home.to_path_buf(),
    };
    validate_artifact(&artifact, &package)
        .map_err(|_| error("browser_runtime_cache_active_invalid"))
}

fn activate(root: &Path, target: &str, version: &str, digest: &str) -> Result<()> {
    let active = root.join("active");
    fs::create_dir_all(&active).map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    let tmp = active.join(format!(".{target}.tmp-{}", unique()));
    let value = ActiveManifest {
        target: target.to_string(),
        app_version: version.to_string(),
        package_sha256: digest.to_string(),
    };
    let mut f = File::create(&tmp).map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    let encoded =
        serde_json::to_vec(&value).map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    f.write_all(&encoded)
        .map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    f.sync_all()
        .map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    if fs::rename(&tmp, active.join(format!("{target}.json"))).is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(error("browser_runtime_cache_activation_failed"));
    }
    sync_dir(&active)?;
    Ok(())
}

#[derive(Clone, Copy)]
enum RequiredKind {
    File,
    Directory,
}

fn checked_descendant(root: &Path, relative: &str, kind: RequiredKind) -> Result<PathBuf> {
    let components = Path::new(relative).components().collect::<Vec<_>>();
    if components.is_empty() {
        return Err(error("browser_runtime_cache_artifact_invalid"));
    }
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(error("browser_runtime_cache_artifact_invalid"));
        };
        current.push(name);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| error("browser_runtime_cache_artifact_invalid"))?;
        if metadata.file_type().is_symlink() {
            return Err(error("browser_runtime_cache_artifact_invalid"));
        }
        let last = index + 1 == components.len();
        if !last && !metadata.is_dir() {
            return Err(error("browser_runtime_cache_artifact_invalid"));
        }
        if last {
            match kind {
                RequiredKind::File if !metadata.is_file() => {
                    return Err(error("browser_runtime_cache_artifact_invalid"));
                }
                RequiredKind::Directory if !metadata.is_dir() => {
                    return Err(error("browser_runtime_cache_artifact_invalid"));
                }
                _ => {}
            }
        }
    }
    Ok(current)
}

fn remove_any(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    let result = if metadata.file_type().is_symlink() || !metadata.is_dir() {
        fs::remove_file(path)
    } else {
        make_owned_tree_removable(path)?;
        fs::remove_dir_all(path)
    };
    result.map_err(|_| error("browser_runtime_cache_activation_failed"))
}

fn make_owned_tree_removable(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() | 0o700;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    }
    #[cfg(not(unix))]
    {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)
            .map_err(|_| error("browser_runtime_cache_activation_failed"))?;
    }
    for entry in fs::read_dir(path).map_err(|_| error("browser_runtime_cache_activation_failed"))? {
        let entry = entry.map_err(|_| error("browser_runtime_cache_activation_failed"))?;
        let child = entry.path();
        let child_metadata = fs::symlink_metadata(&child)
            .map_err(|_| error("browser_runtime_cache_activation_failed"))?;
        if child_metadata.is_dir() && !child_metadata.file_type().is_symlink() {
            make_owned_tree_removable(&child)?;
        }
    }
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| error("browser_runtime_cache_activation_failed"))
}

fn ensure_cache_root(root: &Path) -> Result<()> {
    fs::create_dir_all(root).map_err(|_| error("browser_runtime_cache_unavailable"))?;
    let metadata =
        fs::symlink_metadata(root).map_err(|_| error("browser_runtime_cache_unavailable"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(error("browser_runtime_cache_unavailable"));
    }
    Ok(())
}

fn read_bounded_file(path: &Path, max_bytes: u64, code: &'static str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error(code))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return Err(error(code));
    }
    fs::read(path).map_err(|_| error(code))
}

struct Lock {
    path: PathBuf,
    token: Vec<u8>,
    #[cfg(unix)]
    file: File,
}

fn lock_token() -> Vec<u8> {
    format!("pid={}\nnonce={}\n", std::process::id(), unique()).into_bytes()
}

#[cfg(not(unix))]
fn reclaim_stale_lock(path: &Path) -> bool {
    if !stale_lock(path) {
        return false;
    }
    let quarantine = path.with_file_name(format!(".stale-{}", unique()));
    match fs::rename(path, &quarantine) {
        Ok(()) => {
            let _ = fs::remove_file(quarantine);
            true
        }
        Err(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

fn write_lock_token(file: &mut File, token: &[u8], unavailable_code: &'static str) -> Result<()> {
    file.set_len(0)
        .and_then(|_| file.seek(SeekFrom::Start(0)))
        .and_then(|_| file.write_all(token))
        .and_then(|_| file.sync_all())
        .map_err(|_| error(unavailable_code))
}

#[cfg(unix)]
fn try_lock_file(path: &Path) -> std::io::Result<Option<File>> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(file));
    }
    let io_error = std::io::Error::last_os_error();
    if io_error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(io_error)
    }
}

#[cfg(unix)]
fn acquire_unix(
    path: &Path,
    timeout_code: &'static str,
    unavailable_code: &'static str,
) -> Result<Lock> {
    let deadline = Instant::now() + LOCK_WAIT_TOTAL;
    let token = lock_token();
    loop {
        match try_lock_file(path) {
            Ok(Some(mut file)) => {
                write_lock_token(&mut file, &token, unavailable_code)?;
                return Ok(Lock {
                    path: path.into(),
                    token,
                    file,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err(error(timeout_code));
                }
                std::thread::sleep(LOCK_POLL);
            }
            Err(_) => return Err(error(unavailable_code)),
        }
    }
}

#[cfg(unix)]
async fn acquire_unix_async(
    path: &Path,
    timeout_code: &'static str,
    unavailable_code: &'static str,
) -> Result<Lock> {
    let deadline = Instant::now() + LOCK_WAIT_TOTAL;
    let token = lock_token();
    loop {
        match try_lock_file(path) {
            Ok(Some(mut file)) => {
                write_lock_token(&mut file, &token, unavailable_code)?;
                return Ok(Lock {
                    path: path.into(),
                    token,
                    file,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err(error(timeout_code));
                }
                tokio::time::sleep(LOCK_POLL).await;
            }
            Err(_) => return Err(error(unavailable_code)),
        }
    }
}

#[cfg(not(unix))]
fn acquire_portable(
    path: &Path,
    timeout_code: &'static str,
    unavailable_code: &'static str,
) -> Result<Lock> {
    let deadline = Instant::now() + LOCK_WAIT_TOTAL;
    let token = lock_token();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                write_lock_token(&mut file, &token, unavailable_code)?;
                return Ok(Lock {
                    path: path.into(),
                    token,
                });
            }
            Err(io_error) if io_error.kind() == std::io::ErrorKind::AlreadyExists => {
                if reclaim_stale_lock(path) {
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(error(timeout_code));
                }
                std::thread::sleep(LOCK_POLL);
            }
            Err(_) => return Err(error(unavailable_code)),
        }
    }
}

#[cfg(not(unix))]
async fn acquire_portable_async(
    path: &Path,
    timeout_code: &'static str,
    unavailable_code: &'static str,
) -> Result<Lock> {
    let deadline = Instant::now() + LOCK_WAIT_TOTAL;
    let token = lock_token();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                write_lock_token(&mut file, &token, unavailable_code)?;
                return Ok(Lock {
                    path: path.into(),
                    token,
                });
            }
            Err(io_error) if io_error.kind() == std::io::ErrorKind::AlreadyExists => {
                if reclaim_stale_lock(path) {
                    continue;
                }
                if Instant::now() >= deadline {
                    return Err(error(timeout_code));
                }
                tokio::time::sleep(LOCK_POLL).await;
            }
            Err(_) => return Err(error(unavailable_code)),
        }
    }
}

impl Lock {
    fn acquire(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            return acquire_unix(
                path,
                "browser_runtime_cache_lock_timeout",
                "browser_runtime_cache_lock_unavailable",
            );
        }
        #[cfg(not(unix))]
        {
            acquire_portable(
                path,
                "browser_runtime_cache_lock_timeout",
                "browser_runtime_cache_lock_unavailable",
            )
        }
    }

    async fn acquire_async(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            return acquire_unix_async(
                path,
                "browser_runtime_acquisition_lock_timeout",
                "browser_runtime_acquisition_lock_unavailable",
            )
            .await;
        }
        #[cfg(not(unix))]
        {
            acquire_portable_async(
                path,
                "browser_runtime_acquisition_lock_timeout",
                "browser_runtime_acquisition_lock_unavailable",
            )
            .await
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let Ok(contents) = fs::read(&self.path) else {
            return;
        };
        if contents == self.token {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn stale_lock(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    if lock_owner_is_gone(path) {
        return true;
    }
    let Ok(modified) = fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age > LOCK_STALE_AFTER)
}

#[cfg(target_os = "linux")]
fn lock_owner_is_gone(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Some(pid) = text
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("pid="))
    else {
        return false;
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return false;
    };
    !Path::new("/proc").join(pid.to_string()).exists()
}
#[cfg(test)]
#[path = "browser_distribution_tests.rs"]
mod tests;
