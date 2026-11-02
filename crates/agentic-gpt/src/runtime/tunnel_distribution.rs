//! Trusted resolution and installation of the Tunnel client executable.
//!
//! Each cache lookup verifies the pinned archive and executable hashes. A
//! damaged executable can be restored from its verified archive.

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

pub(crate) const PINNED_VERSION: &str = "0.0.19";
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 32;
const MAX_REDIRECTS: usize = 5;
const DOWNLOAD_ATTEMPTS: usize = 3;
const LOCK_ATTEMPTS: usize = 400;
const LOCK_WAIT: Duration = Duration::from_millis(25);
const LOCK_STALE_AFTER: StdDuration = StdDuration::from_secs(5 * 60);

#[derive(Clone, Debug)]
struct ArtifactSpec {
    version: String,
    platform: String,
    url: String,
    archive_sha256: String,
    executable_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedClientOutcome {
    AlreadyAvailable,
    Repaired,
    Downloaded,
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
    Unavailable(String),
    Repair {
        archive: Vec<u8>,
        executable: Vec<u8>,
    },
    Valid(PathBuf),
}

const MANIFEST: &[(&str, &str, &str, &str, &str)] = &[
    (
        "0.0.19",
        "linux-amd64",
        "https://github.com/openai/tunnel-client/releases/download/v0.0.19/tunnel-client-v0.0.19-linux-amd64.zip",
        "9ee9448592345a9dd1517fb79791ab54d24f051142eb92bcbe8cd355effc9142",
        "56d0120886ce91bfe31e7a5600f6a6594199a02e9b3eac671d5da0d7480c97a3",
    ),
    (
        "0.0.19",
        "linux-arm64",
        "https://github.com/openai/tunnel-client/releases/download/v0.0.19/tunnel-client-v0.0.19-linux-arm64.zip",
        "5a19e556c4a8736ab7a6fde6f0568f6e7411fa2dff21186fc15bc30e9b98a603",
        "8a50b25d6055bc6e3e3a5c18c098637ab2be63f6e453b8ebebd02433deadba04",
    ),
];

/// Resolve a configured executable without launching or otherwise trusting it.
pub(crate) async fn resolve(config: &Config) -> Result<PathBuf> {
    let tunnel = config
        .tunnel
        .as_ref()
        .ok_or_else(|| anyhow!("tunnel_config_missing"))?;

    if let Some(executable) = &tunnel.client.executable {
        return resolve_executable(executable);
    }

    let platform = current_platform().ok_or_else(|| anyhow!("unsupported_platform"))?;
    let spec = artifact_spec(platform)?;
    resolve_managed(&tunnel.client, &spec).await
}

fn resolve_executable(path: &Path) -> Result<PathBuf> {
    let configured_path = expand_path(path)?;
    let platform = current_platform().ok_or_else(|| anyhow!("unsupported_platform"))?;
    let spec = artifact_spec(platform)?;
    verify_executable(&configured_path, &spec.executable_sha256)?;
    fs::canonicalize(&configured_path)
        .map_err(|error| executable_path_error(error, "executable_resolve_failed"))
}

async fn resolve_managed(client: &TunnelClientConfig, spec: &ArtifactSpec) -> Result<PathBuf> {
    resolve_managed_with_progress(client, spec, |_, _| {})
        .await
        .map(|(resolved, _)| resolved)
}

pub(crate) async fn download_managed<F>(
    client: &TunnelClientConfig,
    mut progress: F,
) -> Result<ManagedClientOutcome>
where
    F: FnMut(u64, Option<u64>),
{
    let platform = current_platform().ok_or_else(|| anyhow!("unsupported_platform"))?;
    let spec = artifact_spec(platform)?;
    let (_, outcome) = resolve_managed_with_progress(client, &spec, &mut progress).await?;
    Ok(outcome)
}

async fn resolve_managed_with_progress<F>(
    client: &TunnelClientConfig,
    spec: &ArtifactSpec,
    mut progress: F,
) -> Result<(PathBuf, ManagedClientOutcome)>
where
    F: FnMut(u64, Option<u64>),
{
    let cache_root = expand_path(&client.cache_dir)?;
    fs::create_dir_all(&cache_root).map_err(|_| anyhow!("cache_directory_unavailable"))?;
    let paths = cache_paths(&cache_root, spec)?;

    let initial_state = cache_state(&paths, spec);
    if let CacheState::Unavailable(error) = &initial_state {
        return Err(anyhow!("{error}"));
    }
    if let CacheState::Valid(path) = &initial_state {
        return Ok((path.clone(), ManagedClientOutcome::AlreadyAvailable));
    }

    let _lock = ArtifactLock::acquire(&paths.lock).await?;

    match cache_state(&paths, spec) {
        CacheState::Valid(path) => return Ok((path, ManagedClientOutcome::AlreadyAvailable)),
        CacheState::Repair {
            archive,
            executable,
        } => {
            let path = install_cache(&cache_root, &paths.final_dir, &archive, &executable)?;
            return Ok((path, ManagedClientOutcome::Repaired));
        }
        CacheState::Missing | CacheState::Invalid => {}
        CacheState::Unavailable(error) => return Err(anyhow!("{error}")),
    }

    let download_dir = cache_root.join(".downloads");
    fs::create_dir_all(&download_dir).map_err(|_| anyhow!("cache_directory_unavailable"))?;
    let temporary_archive = download_dir.join(format!("{}.part", Uuid::new_v4()));
    let _temporary_archive = TemporaryFile::new(temporary_archive.clone());
    progress(0, None);
    download_archive_with_retries(&spec.url, &temporary_archive, &mut progress).await?;
    let archive = read_bounded_file(&temporary_archive, MAX_ARCHIVE_BYTES)?;

    verify_archive_hash(&archive, &spec.archive_sha256)?;
    let executable = extract_executable(&archive)?;
    verify_executable_bytes(&executable, &spec.executable_sha256)?;
    let path = install_cache(&cache_root, &paths.final_dir, &archive, &executable)?;
    Ok((path, ManagedClientOutcome::Downloaded))
}

fn artifact_spec(platform: &str) -> Result<ArtifactSpec> {
    let entry = MANIFEST
        .iter()
        .find(|(entry_version, entry_platform, _, _, _)| {
            *entry_version == PINNED_VERSION && *entry_platform == platform
        })
        .ok_or_else(|| anyhow!("unsupported_platform"))?;
    Ok(ArtifactSpec {
        version: PINNED_VERSION.to_owned(),
        platform: platform.to_owned(),
        url: entry.2.to_owned(),
        archive_sha256: entry.3.to_owned(),
        executable_sha256: entry.4.to_owned(),
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

fn cache_state(paths: &CachePaths, spec: &ArtifactSpec) -> CacheState {
    let final_meta = match fs::symlink_metadata(&paths.final_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return CacheState::Missing,
        Err(error) => {
            return CacheState::Unavailable(format!("cache_artifact_metadata_failed: {error}"));
        }
    };
    if final_meta.file_type().is_symlink() || !final_meta.is_dir() {
        return CacheState::Invalid;
    }

    let archive = match read_bounded_file(&paths.archive, MAX_ARCHIVE_BYTES) {
        Ok(archive) => archive,
        Err(error) => {
            let message = error.to_string();
            return if matches!(
                message.split(':').next(),
                Some("cache_artifact_missing" | "cache_artifact_invalid" | "archive_size_limit")
            ) {
                CacheState::Invalid
            } else {
                CacheState::Unavailable(message)
            };
        }
    };
    if sha256_bytes(&archive) != spec.archive_sha256 {
        return CacheState::Invalid;
    }
    let executable_meta = match fs::symlink_metadata(&paths.executable) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return CacheState::Unavailable(format!("cache_artifact_metadata_failed: {error}"));
        }
    };
    if executable_meta.is_some_and(|meta| {
        !meta.file_type().is_symlink() && meta.is_file() && is_executable(&meta)
    }) {
        match fs::read(&paths.executable) {
            Ok(bytes) if sha256_bytes(&bytes) == spec.executable_sha256 => {
                return CacheState::Valid(paths.executable.clone());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return CacheState::Unavailable(format!("cache_artifact_read_failed: {error}"));
            }
        }
    }
    let Ok(executable) = extract_executable(&archive) else {
        return CacheState::Invalid;
    };
    if verify_executable_bytes(&executable, &spec.executable_sha256).is_err() {
        return CacheState::Invalid;
    }
    CacheState::Repair {
        archive,
        executable,
    }
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

    let mut candidate = None;
    for index in 0..archive.len() {
        let entry = archive
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
        if entry.size() > MAX_EXTRACTED_BYTES {
            return Err(anyhow!("extracted_size_limit"));
        }
        let is_candidate = entry
            .enclosed_name()
            .and_then(|path| path.file_name().map(|name| name == "tunnel-client"))
            .unwrap_or(false);
        if is_candidate {
            if candidate.is_some() {
                return Err(anyhow!("archive_duplicate_candidate"));
            }
            let mut contents = Vec::with_capacity(entry.size().min(MAX_EXTRACTED_BYTES) as usize);
            entry
                .take(MAX_EXTRACTED_BYTES + 1)
                .read_to_end(&mut contents)
                .map_err(|_| anyhow!("archive_invalid"))?;
            if contents.len() as u64 > MAX_EXTRACTED_BYTES {
                return Err(anyhow!("extracted_size_limit"));
            }
            candidate = Some(contents);
        }
    }
    candidate.ok_or_else(|| anyhow!("archive_candidate_missing"))
}

async fn download_archive<F>(
    url: &str,
    destination: &Path,
    allow_http: bool,
    mut progress: F,
) -> Result<()>
where
    F: FnMut(u64, Option<u64>),
{
    let mut current = validate_download_url(url, allow_http)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| anyhow!("download_client_init"))?;

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
            return Err(anyhow!(
                "download_http_status_{}",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
        {
            return Err(anyhow!("archive_size_limit"));
        }

        let total_length = response.content_length();
        progress(0, total_length);
        let mut file = tokio::fs::File::create(destination)
            .await
            .map_err(|_| anyhow!("download_destination_unavailable"))?;
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
            progress(total, total_length);
        }
        file.sync_all()
            .await
            .map_err(|_| anyhow!("download_write_failed"))?;
        return Ok(());
    }
    Err(anyhow!("download_redirect_limit"))
}

async fn download_archive_with_retries<F>(
    url: &str,
    destination: &Path,
    mut progress: F,
) -> Result<()>
where
    F: FnMut(u64, Option<u64>),
{
    for attempt in 0..DOWNLOAD_ATTEMPTS {
        match download_archive(url, destination, false, &mut progress).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                let message = error.to_string();
                let retryable = message == "download_failed"
                    || message
                        .strip_prefix("download_http_status_")
                        .and_then(|status| status.parse::<u16>().ok())
                        .is_some_and(|status| status >= 500);
                if !retryable || attempt + 1 == DOWNLOAD_ATTEMPTS {
                    return Err(error);
                }
                let _ = fs::remove_file(destination);
                progress(0, None);
                tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
            }
        }
    }
    unreachable!("download attempts are non-zero")
}

struct TemporaryFile(PathBuf);

impl TemporaryFile {
    fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
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
    if !url_scheme_allowed(&parsed, allow_http) {
        return Err(anyhow!("download_url_requires_https"));
    }
    if parsed.host().is_none()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(anyhow!("download_url_invalid"));
    }
    Ok(parsed)
}

fn url_scheme_allowed(url: &reqwest::Url, allow_http: bool) -> bool {
    url.scheme() == "https" || (allow_http && url.scheme() == "http")
}

fn verify_executable(path: &Path, expected_sha256: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| executable_path_error(error, "executable_metadata_failed"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!("executable_not_regular"));
    }
    if !is_executable(&metadata) {
        return Err(anyhow!("executable_not_executable"));
    }
    let bytes = fs::read(path).map_err(|_| anyhow!("executable_read_failed"))?;
    let actual_sha256 = sha256_bytes(&bytes);
    if actual_sha256 != expected_sha256 {
        return Err(anyhow!("executable_hash_mismatch"));
    }
    Ok(())
}

fn verify_executable_bytes(bytes: &[u8], expected_sha256: &str) -> Result<()> {
    if sha256_bytes(bytes) != expected_sha256 {
        return Err(anyhow!("executable_hash_mismatch"));
    }
    Ok(())
}

fn executable_path_error(error: std::io::Error, fallback: &str) -> anyhow::Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        anyhow!("executable_not_found")
    } else {
        anyhow!("{fallback}: {error}")
    }
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
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!("cache_artifact_missing")
        } else {
            anyhow!("cache_artifact_metadata_failed: {error}")
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!("cache_artifact_invalid"));
    }
    if metadata.len() > limit {
        return Err(anyhow!("archive_size_limit"));
    }
    fs::read(path).map_err(|error| anyhow!("cache_artifact_read_failed: {error}"))
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
    fn cache_revalidates_archive_and_repairs_binary() {
        let root = tempfile_dir();
        let bytes = archive(&[("tunnel-client", b"good", Some(0o100755))]);
        let spec = ArtifactSpec {
            version: "test".to_owned(),
            platform: "linux-amd64".to_owned(),
            url: "https://example.invalid/client.zip".to_owned(),
            archive_sha256: sha256_bytes(&bytes),
            executable_sha256: sha256_bytes(b"good"),
        };
        let paths = cache_paths(&root, &spec).unwrap();
        let installed = install_cache(&root, &paths.final_dir, &bytes, b"good").unwrap();
        assert_eq!(
            cache_state(&paths, &spec),
            CacheState::Valid(installed.clone())
        );
        fs::write(&installed, b"bad").unwrap();
        let CacheState::Repair {
            archive,
            executable,
        } = cache_state(&paths, &spec)
        else {
            panic!("expected damaged cached executable to be repairable");
        };
        let repaired = install_cache(&root, &paths.final_dir, &archive, &executable).unwrap();
        assert_eq!(fs::read(repaired).unwrap(), b"good");
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

    #[tokio::test]
    async fn bounded_local_download_handles_redirect_and_size_limit() {
        let body = b"small archive".to_vec();
        let (target_url, target_task) = response_server(200, &body, None).await;
        let (redirect_url, redirect_task) = response_server(302, b"", Some(&target_url)).await;
        let destination = tempfile_dir().join("download.zip");
        download_archive(&redirect_url, &destination, true, |_, _| {})
            .await
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), body);
        target_task.abort();
        redirect_task.abort();

        let oversized = vec![b'x'; (MAX_ARCHIVE_BYTES + 1) as usize];
        let (oversized_url, oversized_task) = response_server(200, &oversized, None).await;
        let result = download_archive(
            &oversized_url,
            &tempfile_dir().join("too-large.zip"),
            true,
            |_, _| {},
        )
        .await;
        assert_eq!(result.unwrap_err().to_string(), "archive_size_limit");
        oversized_task.abort();

        let (partial_url, partial_task) =
            response_server_with_length(200, b"partial", None, Some(100)).await;
        let result = download_archive(
            &partial_url,
            &tempfile_dir().join("partial.zip"),
            true,
            |_, _| {},
        )
        .await;
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
}
