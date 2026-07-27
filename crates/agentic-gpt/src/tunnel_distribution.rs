//! Trusted resolution and installation of the Tunnel client executable.
//!
//! The resolver deliberately treats a downloaded archive as the trusted unit:
//! the pinned SHA-256 is for the archive, and every cache lookup re-hashes that
//! archive before deriving the executable bytes that will be launched.

use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, SystemTime};

use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use reqwest::header::LOCATION;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::time::Duration;
use uuid::Uuid;
use zip::ZipArchive;

use crate::config::{Config, TunnelClientConfig};

const PINNED_VERSION: &str = "0.0.10";
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 32;
const MAX_REDIRECTS: usize = 5;
const LOCK_ATTEMPTS: usize = 400;
const LOCK_WAIT: Duration = Duration::from_millis(25);
const LOCK_STALE_AFTER: StdDuration = StdDuration::from_secs(5 * 60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TunnelClientSource {
    ExecutableOverride,
    ManagedCache,
    CustomDownload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedTunnelClient {
    pub(crate) path: PathBuf,
    pub(crate) version: String,
    pub(crate) platform: String,
    pub(crate) sha256: String,
    pub(crate) source: TunnelClientSource,
}

#[derive(Clone, Debug)]
struct ArtifactSpec {
    version: String,
    platform: String,
    url: String,
    archive_sha256: String,
    source: TunnelClientSource,
}

#[derive(Clone, Debug)]
struct CachePaths {
    final_dir: PathBuf,
    executable: PathBuf,
    archive: PathBuf,
    lock: PathBuf,
}

#[derive(Debug, PartialEq)]
enum CacheState {
    Missing,
    Invalid,
    Repair {
        archive: Vec<u8>,
        executable: Vec<u8>,
    },
    Valid(PathBuf),
}

const MANIFEST: &[(&str, &str, &str, &str)] = &[
    (
        "0.0.10",
        "linux-amd64",
        "https://github.com/openai/tunnel-client/releases/download/v0.0.10/tunnel-client-v0.0.10-linux-amd64.zip",
        "b9e0388a343f2d7adeff3992f411a0bd3d916a64bc56534aac5fd15ac1b20cd5",
    ),
    (
        "0.0.10",
        "linux-arm64",
        "https://github.com/openai/tunnel-client/releases/download/v0.0.10/tunnel-client-v0.0.10-linux-arm64.zip",
        "b842a9b2352eebd80514cf01a1fbb1c0d400a7d24a4015e85a7ea5f1aeaa5b30",
    ),
];

/// Resolve a configured executable without launching or otherwise trusting it.
pub(crate) async fn resolve(config: &Config) -> Result<ResolvedTunnelClient> {
    let tunnel = config
        .tunnel
        .as_ref()
        .ok_or_else(|| anyhow!("tunnel_config_missing"))?;

    if let Some(executable) = &tunnel.client.executable {
        return resolve_executable(executable, tunnel.client.sha256.as_deref());
    }

    let platform = current_platform().ok_or_else(|| anyhow!("unsupported_platform"))?;
    let spec = artifact_spec(&tunnel.client, platform)?;
    resolve_managed(&tunnel.client, &spec).await
}

fn resolve_executable(path: &Path, expected_sha256: Option<&str>) -> Result<ResolvedTunnelClient> {
    let configured_path = expand_path(path)?;
    let actual_sha256 = verify_executable(&configured_path, expected_sha256)?;
    let path = fs::canonicalize(&configured_path).map_err(|_| anyhow!("executable_not_found"))?;
    Ok(ResolvedTunnelClient {
        path,
        version: "override".to_owned(),
        platform: current_platform().unwrap_or("unknown").to_owned(),
        sha256: actual_sha256,
        source: TunnelClientSource::ExecutableOverride,
    })
}

async fn resolve_managed(
    client: &TunnelClientConfig,
    spec: &ArtifactSpec,
) -> Result<ResolvedTunnelClient> {
    let cache_root = expand_path(&client.cache_dir)?;
    fs::create_dir_all(&cache_root).map_err(|_| anyhow!("cache_directory_unavailable"))?;
    let paths = cache_paths(&cache_root, spec)?;

    let initial_state = cache_state(&paths, &spec.archive_sha256);
    if let CacheState::Valid(path) = &initial_state {
        return resolved(spec, path.clone());
    }
    if !client.auto_download && matches!(initial_state, CacheState::Missing | CacheState::Invalid) {
        return Err(anyhow!("cached_artifact_missing_or_invalid"));
    }

    let _lock = ArtifactLock::acquire(&paths.lock).await?;

    match cache_state(&paths, &spec.archive_sha256) {
        CacheState::Valid(path) => return resolved(spec, path),
        CacheState::Repair {
            archive,
            executable,
        } => {
            let path = install_cache(&cache_root, &paths.final_dir, &archive, &executable)?;
            return resolved(spec, path);
        }
        CacheState::Missing | CacheState::Invalid => {}
    }

    let download_dir = cache_root.join(".downloads");
    fs::create_dir_all(&download_dir).map_err(|_| anyhow!("cache_directory_unavailable"))?;
    let temporary_archive = download_dir.join(format!("{}.part", Uuid::new_v4()));
    let download_result = download_archive(&spec.url, &temporary_archive, false).await;
    if let Err(error) = download_result {
        let _ = fs::remove_file(&temporary_archive);
        return Err(error);
    }

    let archive = match read_bounded_file(&temporary_archive, MAX_ARCHIVE_BYTES) {
        Ok(archive) => archive,
        Err(error) => {
            let _ = fs::remove_file(&temporary_archive);
            return Err(error);
        }
    };
    let _ = fs::remove_file(&temporary_archive);

    verify_archive_hash(&archive, &spec.archive_sha256)?;
    let executable = extract_executable(&archive)?;
    let path = install_cache(&cache_root, &paths.final_dir, &archive, &executable)?;
    resolved(spec, path)
}

fn resolved(spec: &ArtifactSpec, path: PathBuf) -> Result<ResolvedTunnelClient> {
    let executable_sha256 = sha256_file(&path)?;
    Ok(ResolvedTunnelClient {
        path,
        version: spec.version.clone(),
        platform: spec.platform.clone(),
        sha256: executable_sha256,
        source: spec.source.clone(),
    })
}

fn artifact_spec(client: &TunnelClientConfig, platform: &str) -> Result<ArtifactSpec> {
    if let Some(url) = client.download_url.as_deref() {
        let url = validate_download_url(url, false)?.to_string();
        let archive_sha256 = normalize_sha256(
            client
                .sha256
                .as_deref()
                .ok_or_else(|| anyhow!("custom_download_sha256_missing"))?,
        )?;
        let version = client
            .version
            .as_deref()
            .map(normalize_version)
            .transpose()?
            .unwrap_or_else(|| "custom".to_owned());
        return Ok(ArtifactSpec {
            version,
            platform: platform.to_owned(),
            url,
            archive_sha256,
            source: TunnelClientSource::CustomDownload,
        });
    }

    if client.sha256.is_some() {
        return Err(anyhow!("sha256_requires_custom_download"));
    }

    let version = client
        .version
        .as_deref()
        .map(normalize_version)
        .transpose()?
        .unwrap_or_else(|| PINNED_VERSION.to_owned());
    let entry = MANIFEST
        .iter()
        .find(|(entry_version, entry_platform, _, _)| {
            *entry_version == version && *entry_platform == platform
        })
        .ok_or_else(|| anyhow!("unsupported_tunnel_client_version"))?;
    Ok(ArtifactSpec {
        version: version.clone(),
        platform: platform.to_owned(),
        url: entry.2.to_owned(),
        archive_sha256: entry.3.to_owned(),
        source: TunnelClientSource::ManagedCache,
    })
}

fn cache_paths(cache_root: &Path, spec: &ArtifactSpec) -> Result<CachePaths> {
    let identity = cache_root
        .join("artifacts")
        .join(cache_component(&spec.version)?)
        .join(cache_component(&spec.platform)?)
        .join(cache_component(&spec.archive_sha256)?);
    let lock = cache_root.join(".locks").join(format!(
        "{}-{}-{}.lock",
        cache_component(&spec.version)?,
        cache_component(&spec.platform)?,
        cache_component(&spec.archive_sha256)?
    ));
    Ok(CachePaths {
        executable: identity.join("tunnel-client"),
        archive: identity.join("artifact.zip"),
        final_dir: identity,
        lock,
    })
}

fn cache_state(paths: &CachePaths, expected_archive_sha256: &str) -> CacheState {
    let Ok(final_meta) = fs::symlink_metadata(&paths.final_dir) else {
        return CacheState::Missing;
    };
    if final_meta.file_type().is_symlink() || !final_meta.is_dir() {
        return CacheState::Invalid;
    }

    let Ok(archive) = read_bounded_file(&paths.archive, MAX_ARCHIVE_BYTES) else {
        return CacheState::Invalid;
    };
    if sha256_bytes(&archive) != expected_archive_sha256 {
        return CacheState::Invalid;
    }
    let Ok(executable) = extract_executable(&archive) else {
        return CacheState::Invalid;
    };

    let Ok(executable_meta) = fs::symlink_metadata(&paths.executable) else {
        return CacheState::Repair {
            archive,
            executable,
        };
    };
    if executable_meta.file_type().is_symlink()
        || !executable_meta.is_file()
        || !is_executable(&executable_meta)
    {
        return CacheState::Repair {
            archive,
            executable,
        };
    }
    let Ok(cached_executable) = fs::read(&paths.executable) else {
        return CacheState::Repair {
            archive,
            executable,
        };
    };
    if cached_executable != executable {
        return CacheState::Repair {
            archive,
            executable,
        };
    }
    CacheState::Valid(paths.executable.clone())
}

fn install_cache(
    cache_root: &Path,
    final_dir: &Path,
    archive: &[u8],
    executable: &[u8],
) -> Result<PathBuf> {
    if archive.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(anyhow!("archive_size_limit"));
    }
    if executable.len() as u64 > MAX_EXTRACTED_BYTES {
        return Err(anyhow!("extracted_size_limit"));
    }
    if let Some(parent) = final_dir.parent() {
        fs::create_dir_all(parent).map_err(|_| anyhow!("cache_directory_unavailable"))?;
    }
    let staging_root = cache_root.join(".staging");
    fs::create_dir_all(&staging_root).map_err(|_| anyhow!("cache_directory_unavailable"))?;
    let staging = staging_root.join(Uuid::new_v4().to_string());
    fs::create_dir(&staging).map_err(|_| anyhow!("cache_install_failed"))?;

    let result = (|| -> Result<PathBuf> {
        write_synced(&staging.join("artifact.zip"), archive, false)?;
        write_synced(&staging.join("tunnel-client"), executable, true)?;

        let backup = staging_root.join(format!("{}.old", Uuid::new_v4()));
        let had_old = match fs::symlink_metadata(final_dir) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(anyhow!("cache_path_unsafe"));
            }
            Ok(_) => {
                fs::rename(final_dir, &backup).map_err(|_| anyhow!("cache_install_failed"))?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(anyhow!("cache_install_failed")),
        };

        if fs::rename(&staging, final_dir).is_err() {
            if had_old {
                let _ = fs::rename(&backup, final_dir);
            }
            return Err(anyhow!("cache_install_failed"));
        }
        if had_old {
            let _ = fs::remove_dir_all(backup);
        }
        Ok(final_dir.join("tunnel-client"))
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn write_synced(path: &Path, contents: &[u8], executable: bool) -> Result<()> {
    let mut file = File::create(path).map_err(|_| anyhow!("cache_install_failed"))?;
    file.write_all(contents)
        .map_err(|_| anyhow!("cache_install_failed"))?;
    file.sync_all()
        .map_err(|_| anyhow!("cache_install_failed"))?;
    if executable {
        set_executable(path)?;
    }
    Ok(())
}

fn extract_executable(archive_bytes: &[u8]) -> Result<Vec<u8>> {
    if archive_bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(anyhow!("archive_size_limit"));
    }
    let mut archive =
        ZipArchive::new(Cursor::new(archive_bytes)).map_err(|_| anyhow!("archive_invalid"))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(anyhow!("archive_entry_limit"));
    }

    let mut extracted_bytes = 0u64;
    let mut candidate = None;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| anyhow!("archive_invalid"))?;
        let name = entry.name().to_owned();
        if name.contains('\\') || entry.enclosed_name().is_none() {
            return Err(anyhow!("archive_path_traversal"));
        }
        if entry.is_symlink() {
            return Err(anyhow!("archive_symlink_rejected"));
        }
        if let Some(mode) = entry.unix_mode() {
            let file_type = mode & 0o170000;
            if file_type != 0 && file_type != 0o100000 && file_type != 0o040000 {
                return Err(anyhow!("archive_file_type_rejected"));
            }
        }
        if entry.is_dir() {
            continue;
        }
        if !entry.is_file() {
            return Err(anyhow!("archive_file_type_rejected"));
        }
        if entry.size() > MAX_EXTRACTED_BYTES
            || extracted_bytes.saturating_add(entry.size()) > MAX_EXTRACTED_BYTES
        {
            return Err(anyhow!("extracted_size_limit"));
        }
        extracted_bytes += entry.size();

        let is_candidate = entry
            .enclosed_name()
            .and_then(|path| path.file_name().map(|name| name == "tunnel-client"))
            .unwrap_or(false);
        if !is_candidate {
            return Err(anyhow!("archive_layout_rejected"));
        }
        if candidate.is_some() {
            return Err(anyhow!("archive_duplicate_candidate"));
        }
        let mut contents = Vec::with_capacity(entry.size().min(MAX_EXTRACTED_BYTES) as usize);
        entry
            .read_to_end(&mut contents)
            .map_err(|_| anyhow!("archive_invalid"))?;
        candidate = Some(contents);
    }
    candidate.ok_or_else(|| anyhow!("archive_candidate_missing"))
}

async fn download_archive(url: &str, destination: &Path, allow_http: bool) -> Result<()> {
    let mut current = validate_download_url(url, allow_http)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| anyhow!("download_client_init"))?;
    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|_| anyhow!("download_destination_unavailable"))?;

    for redirect_count in 0..=MAX_REDIRECTS {
        let response = client
            .get(current.clone())
            .send()
            .await
            .map_err(|_| anyhow!("download_failed"))?;
        if response.status().is_redirection() {
            if redirect_count == MAX_REDIRECTS {
                return Err(anyhow!("download_redirect_limit"));
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| anyhow!("download_redirect_invalid"))?;
            current = current
                .join(location)
                .map_err(|_| anyhow!("download_redirect_invalid"))?;
            if !url_scheme_allowed(&current, allow_http) {
                return Err(anyhow!("download_redirect_requires_https"));
            }
            continue;
        }
        if !response.status().is_success() {
            return Err(anyhow!("download_http_error"));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
        {
            return Err(anyhow!("archive_size_limit"));
        }

        let mut total = 0u64;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| anyhow!("download_failed"))?;
            total = total
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| anyhow!("archive_size_limit"))?;
            if total > MAX_ARCHIVE_BYTES {
                return Err(anyhow!("archive_size_limit"));
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| anyhow!("download_write_failed"))?;
        }
        file.sync_all()
            .await
            .map_err(|_| anyhow!("download_write_failed"))?;
        return Ok(());
    }
    Err(anyhow!("download_redirect_limit"))
}

struct ArtifactLock {
    path: PathBuf,
}

impl ArtifactLock {
    async fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| anyhow!("cache_directory_unavailable"))?;
        }
        for _ in 0..LOCK_ATTEMPTS {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "pid={}", std::process::id());
                    return Ok(Self {
                        path: path.to_owned(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if stale_lock(path) {
                        let _ = fs::remove_file(path);
                    } else {
                        tokio::time::sleep(LOCK_WAIT).await;
                    }
                }
                Err(_) => return Err(anyhow!("cache_lock_unavailable")),
            }
        }
        Err(anyhow!("cache_lock_timeout"))
    }
}

impl Drop for ArtifactLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn stale_lock(path: &Path) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age > LOCK_STALE_AFTER)
}

fn validate_download_url(url: &str, allow_http: bool) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).map_err(|_| anyhow!("download_url_invalid"))?;
    if !url_scheme_allowed(&parsed, allow_http)
        || parsed.host().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(anyhow!("download_url_requires_https"));
    }
    Ok(parsed)
}

fn url_scheme_allowed(url: &reqwest::Url, allow_http: bool) -> bool {
    url.scheme() == "https" || (allow_http && url.scheme() == "http")
}

fn verify_executable(path: &Path, expected_sha256: Option<&str>) -> Result<String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| anyhow!("executable_not_found"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!("executable_not_regular"));
    }
    if !is_executable(&metadata) {
        return Err(anyhow!("executable_not_executable"));
    }
    let actual_sha256 = sha256_file(path)?;
    if let Some(expected) = expected_sha256 {
        let expected = normalize_sha256(expected)?;
        if expected != actual_sha256 {
            return Err(anyhow!("executable_hash_mismatch"));
        }
    }
    Ok(actual_sha256)
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|_| anyhow!("executable_read_failed"))?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn verify_archive_hash(archive: &[u8], expected_sha256: &str) -> Result<()> {
    if sha256_bytes(archive) != expected_sha256 {
        return Err(anyhow!("archive_hash_mismatch"));
    }
    Ok(())
}

fn read_bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| anyhow!("cache_artifact_missing"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!("cache_artifact_invalid"));
    }
    if metadata.len() > limit {
        return Err(anyhow!("archive_size_limit"));
    }
    fs::read(path).map_err(|_| anyhow!("cache_artifact_invalid"))
}

fn normalize_sha256(value: &str) -> Result<String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!("sha256_invalid"));
    }
    Ok(value.to_ascii_lowercase())
}

fn normalize_version(value: &str) -> Result<String> {
    let value = value.strip_prefix('v').unwrap_or(value);
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(anyhow!("tunnel_client_version_invalid"));
    }
    Ok(value.to_owned())
}

fn cache_component(value: &str) -> Result<&str> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(anyhow!("cache_identity_invalid"));
    }
    Ok(value)
}

fn expand_path(path: &Path) -> Result<PathBuf> {
    let value = path.to_string_lossy();
    if value == "~" {
        return dirs::home_dir().ok_or_else(|| anyhow!("home_directory_unavailable"));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("home_directory_unavailable"))?;
        return Ok(home.join(rest));
    }
    Ok(path.to_owned())
}

fn current_platform() -> Option<&'static str> {
    platform_for(std::env::consts::OS, std::env::consts::ARCH)
}

fn platform_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("linux-amd64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        _ => None,
    }
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
        .map_err(|_| anyhow!("cache_install_failed"))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use zip::write::{SimpleFileOptions, ZipWriter};
    use zip::CompressionMethod;

    fn archive(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut output);
        for (name, contents, mode) in entries {
            let mut options =
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            if let Some(mode) = mode {
                options = options.unix_permissions(*mode);
            }
            writer.start_file(*name, options).unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
        output.into_inner()
    }

    #[test]
    fn manifest_and_platforms_are_pinned() {
        assert_eq!(platform_for("linux", "x86_64"), Some("linux-amd64"));
        assert_eq!(platform_for("linux", "aarch64"), Some("linux-arm64"));
        assert_eq!(platform_for("windows", "x86_64"), None);
        assert_eq!(MANIFEST.len(), 2);
        assert_eq!(MANIFEST[0].3.len(), 64);
        assert_eq!(MANIFEST[1].3.len(), 64);
        assert_eq!(
            MANIFEST[0].2,
            "https://github.com/openai/tunnel-client/releases/download/v0.0.10/tunnel-client-v0.0.10-linux-amd64.zip"
        );
        assert_eq!(
            MANIFEST[0].3,
            "b9e0388a343f2d7adeff3992f411a0bd3d916a64bc56534aac5fd15ac1b20cd5"
        );
        assert_eq!(
            MANIFEST[1].3,
            "b842a9b2352eebd80514cf01a1fbb1c0d400a7d24a4015e85a7ea5f1aeaa5b30"
        );
    }

    #[test]
    fn artifact_selection_requires_pinned_or_explicit_trust() {
        let default_client = TunnelClientConfig::default();
        let spec = artifact_spec(&default_client, "linux-amd64").unwrap();
        assert_eq!(spec.version, PINNED_VERSION);
        assert_eq!(spec.source, TunnelClientSource::ManagedCache);

        let unknown = TunnelClientConfig {
            version: Some("9.9.9".to_owned()),
            ..TunnelClientConfig::default()
        };
        assert_eq!(
            artifact_spec(&unknown, "linux-amd64")
                .unwrap_err()
                .to_string(),
            "unsupported_tunnel_client_version"
        );

        let mut custom = TunnelClientConfig {
            download_url: Some("http://127.0.0.1/client.zip".to_owned()),
            sha256: Some("a".repeat(64)),
            ..TunnelClientConfig::default()
        };
        assert_eq!(
            artifact_spec(&custom, "linux-amd64")
                .unwrap_err()
                .to_string(),
            "download_url_requires_https"
        );
        custom.download_url = Some("https://example.invalid/client.zip".to_owned());
        assert_eq!(
            artifact_spec(&custom, "linux-amd64").unwrap().source,
            TunnelClientSource::CustomDownload
        );
    }

    #[test]
    fn executable_override_checks_permissions_and_optional_hash() {
        let root = tempfile_dir();
        let path = root.join("tunnel-client");
        fs::write(&path, b"#!/bin/sh\nexit 0\n").unwrap();
        assert_eq!(
            verify_executable(&path, None).unwrap_err().to_string(),
            "executable_not_executable"
        );
        set_executable(&path).unwrap();
        let digest = sha256_file(&path).unwrap();
        assert_eq!(verify_executable(&path, Some(&digest)).unwrap(), digest);
        assert_eq!(
            verify_executable(&path, Some(&"0".repeat(64)))
                .unwrap_err()
                .to_string(),
            "executable_hash_mismatch"
        );
        let link = root.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(
            verify_executable(&link, None).unwrap_err().to_string(),
            "executable_not_regular"
        );
    }

    #[test]
    fn archive_rejects_traversal_symlinks_duplicates_and_extra_files() {
        let traversal = archive(&[("../tunnel-client", b"x", Some(0o100755))]);
        assert_eq!(
            extract_executable(&traversal).unwrap_err().to_string(),
            "archive_path_traversal"
        );

        let mut symlink = archive(&[("tunnel-client", b"target", Some(0o644))]);
        mark_zip_entry_as_unix_symlink(&mut symlink);
        assert_eq!(
            extract_executable(&symlink).unwrap_err().to_string(),
            "archive_symlink_rejected"
        );

        let duplicate = archive(&[
            ("one/tunnel-client", b"one", Some(0o100755)),
            ("two/tunnel-client", b"two", Some(0o100755)),
        ]);
        assert_eq!(
            extract_executable(&duplicate).unwrap_err().to_string(),
            "archive_duplicate_candidate"
        );

        let extra = archive(&[
            ("tunnel-client", b"one", Some(0o100755)),
            ("README", b"extra", Some(0o100644)),
        ]);
        assert_eq!(
            extract_executable(&extra).unwrap_err().to_string(),
            "archive_layout_rejected"
        );
    }

    #[test]
    fn cache_revalidates_archive_and_repairs_binary() {
        let root = tempfile_dir();
        let bytes = archive(&[("tunnel-client", b"good", Some(0o100755))]);
        let spec = ArtifactSpec {
            version: "test".to_owned(),
            platform: "linux-amd64".to_owned(),
            url: "https://example.invalid/client.zip".to_owned(),
            archive_sha256: sha256_bytes(&bytes),
            source: TunnelClientSource::ManagedCache,
        };
        let paths = cache_paths(&root, &spec).unwrap();
        let installed = install_cache(&root, &paths.final_dir, &bytes, b"good").unwrap();
        assert_eq!(
            cache_state(&paths, &spec.archive_sha256),
            CacheState::Valid(installed.clone())
        );
        fs::write(&installed, b"bad").unwrap();
        assert!(matches!(
            cache_state(&paths, &spec.archive_sha256),
            CacheState::Repair { .. }
        ));
        let replacement = archive(&[("tunnel-client", b"replacement", Some(0o100755))]);
        let replacement_executable = extract_executable(&replacement).unwrap();
        let installed = install_cache(
            &root,
            &paths.final_dir,
            &replacement,
            &replacement_executable,
        )
        .unwrap();
        assert_eq!(fs::read(installed).unwrap(), b"replacement");
    }

    #[tokio::test]
    async fn offline_cache_and_auto_download_false_are_deterministic() {
        let root = tempfile_dir();
        let archive = archive(&[("tunnel-client", b"offline", Some(0o100755))]);
        let spec = ArtifactSpec {
            version: "offline".to_owned(),
            platform: "linux-amd64".to_owned(),
            url: "https://example.invalid/client.zip".to_owned(),
            archive_sha256: sha256_bytes(&archive),
            source: TunnelClientSource::CustomDownload,
        };
        let paths = cache_paths(&root, &spec).unwrap();
        let executable = extract_executable(&archive).unwrap();
        install_cache(&root, &paths.final_dir, &archive, &executable).unwrap();
        let client = TunnelClientConfig {
            cache_dir: root.clone(),
            auto_download: false,
            ..TunnelClientConfig::default()
        };
        assert_eq!(
            resolve_managed(&client, &spec).await.unwrap().path,
            paths.executable
        );

        let empty_root = tempfile_dir();
        let absent_client = TunnelClientConfig {
            cache_dir: empty_root,
            auto_download: false,
            ..TunnelClientConfig::default()
        };
        assert_eq!(
            resolve_managed(&absent_client, &spec)
                .await
                .unwrap_err()
                .to_string(),
            "cached_artifact_missing_or_invalid"
        );
    }

    #[test]
    fn archive_hash_mismatch_is_checked_before_install() {
        let archive = archive(&[("tunnel-client", b"bytes", Some(0o100755))]);
        assert_eq!(
            verify_archive_hash(&archive, &"0".repeat(64))
                .unwrap_err()
                .to_string(),
            "archive_hash_mismatch"
        );
    }

    #[tokio::test]
    async fn artifact_lock_serializes_concurrent_installers() {
        let root = tempfile_dir();
        let lock_path = root.join("artifact.lock");
        let first = ArtifactLock::acquire(&lock_path).await.unwrap();
        let waiting_path = lock_path.clone();
        let waiting = tokio::spawn(async move { ArtifactLock::acquire(&waiting_path).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(first);
        assert!(waiting.await.unwrap().is_ok());
    }

    #[test]
    fn download_url_requires_https_outside_tests() {
        assert!(validate_download_url("http://127.0.0.1/client.zip", false).is_err());
        assert!(validate_download_url("http://127.0.0.1/client.zip", true).is_ok());
        assert!(
            validate_download_url("https://user:pass@example.invalid/client.zip", false).is_err()
        );
    }

    #[tokio::test]
    async fn bounded_local_download_handles_redirect_and_size_limit() {
        let body = b"small archive".to_vec();
        let (target_url, target_task) = response_server(200, &body, None).await;
        let (redirect_url, redirect_task) = response_server(302, b"", Some(&target_url)).await;
        let destination = tempfile_dir().join("download.zip");
        download_archive(&redirect_url, &destination, true)
            .await
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), body);
        target_task.abort();
        redirect_task.abort();

        let oversized = vec![b'x'; (MAX_ARCHIVE_BYTES + 1) as usize];
        let (oversized_url, oversized_task) = response_server(200, &oversized, None).await;
        let result =
            download_archive(&oversized_url, &tempfile_dir().join("too-large.zip"), true).await;
        assert_eq!(result.unwrap_err().to_string(), "archive_size_limit");
        oversized_task.abort();

        let (partial_url, partial_task) =
            response_server_with_length(200, b"partial", None, Some(100)).await;
        let result =
            download_archive(&partial_url, &tempfile_dir().join("partial.zip"), true).await;
        assert_eq!(result.unwrap_err().to_string(), "download_failed");
        partial_task.abort();
    }

    async fn response_server(
        status: u16,
        body: &[u8],
        location: Option<&str>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        response_server_with_length(status, body, location, None).await
    }

    async fn response_server_with_length(
        status: u16,
        body: &[u8],
        location: Option<&str>,
        declared_length: Option<usize>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address: SocketAddr = listener.local_addr().unwrap();
        let body = Arc::new(body.to_vec());
        let location = location.map(str::to_owned);
        let declared_length = declared_length.unwrap_or(body.len());
        let task = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await;
                let reason = if status == 200 { "OK" } else { "Found" };
                let mut response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    declared_length
                );
                if let Some(location) = location {
                    response.push_str(&format!("Location: {location}\r\n"));
                }
                response.push_str("\r\n");
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });
        (format!("http://{address}/client.zip"), task)
    }

    fn tempfile_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!("agentic-tunnel-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn mark_zip_entry_as_unix_symlink(bytes: &mut [u8]) {
        let signature = [0x50, 0x4b, 0x01, 0x02];
        let offset = bytes
            .windows(signature.len())
            .position(|window| window == signature)
            .unwrap();
        // Central-directory "version made by": Unix, and external attributes
        // carry the POSIX symlink file type in the upper 16 bits.
        bytes[offset + 4..offset + 6].copy_from_slice(&0x0314u16.to_le_bytes());
        bytes[offset + 38..offset + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
    }
}
