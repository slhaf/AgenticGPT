//! Internal, offline materialization of the Browser runtime from a verified `.deb`.
use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use tar::EntryType;
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
    checked_descendant(root, "chrome", RequiredKind::Directory)?;
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
    let trusted = derive_trusted_code_paths(&p.codex_home, std::slice::from_ref(&node_modules));
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
}
impl Lock {
    fn acquire(path: &Path) -> Result<Self> {
        let deadline = Instant::now() + LOCK_WAIT_TOTAL;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "pid={}", std::process::id());
                    return Ok(Self { path: path.into() });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if stale_lock(path) {
                        let _ = fs::remove_file(path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err(error("browser_runtime_cache_lock_timeout"));
                    }
                    std::thread::sleep(LOCK_POLL)
                }
                Err(_) => return Err(error("browser_runtime_cache_lock_unavailable")),
            }
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
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
    let Some(pid) = text.trim().strip_prefix("pid=") else {
        return false;
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return false;
    };
    !Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tar::Builder;
    use xz2::write::XzEncoder;

    fn current_target() -> &'static str {
        match std::env::consts::ARCH {
            "aarch64" => "linux-arm64",
            _ => "linux-x64",
        }
    }

    fn encoded_tar(build: impl FnOnce(&mut Builder<XzEncoder<&mut Vec<u8>>>)) -> Vec<u8> {
        let mut compressed = Vec::new();
        {
            let encoder = XzEncoder::new(&mut compressed, 1);
            let mut tar = Builder::new(encoder);
            build(&mut tar);
            tar.into_inner().unwrap().finish().unwrap();
        }
        compressed
    }

    fn append_file(tar: &mut Builder<XzEncoder<&mut Vec<u8>>>, name: &str, data: &[u8], mode: u32) {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        tar.append(&header, Cursor::new(data)).unwrap();
    }

    fn append_directory(tar: &mut Builder<XzEncoder<&mut Vec<u8>>>, name: &str, mode: u32) {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_entry_type(EntryType::Directory);
        header.set_size(0);
        header.set_mode(mode);
        header.set_cksum();
        tar.append(&header, std::io::empty()).unwrap();
    }

    fn append_special(
        tar: &mut Builder<XzEncoder<&mut Vec<u8>>>,
        name: &str,
        entry_type: EntryType,
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_entry_type(entry_type);
        header.set_size(0);
        header.set_mode(0o777);
        if entry_type == EntryType::symlink() || entry_type == EntryType::hard_link() {
            header.set_link_name("outside").unwrap();
        }
        header.set_cksum();
        tar.append(&header, std::io::empty()).unwrap();
    }

    fn tar_fixture() -> Vec<u8> {
        encoded_tar(|tar| {
            append_directory(tar, "usr/lib/chatgpt/resources/cua_node/lib", 0o555);
            for (name, data, mode) in [
                (
                    "usr/lib/chatgpt/resources/cua_node/bin/node",
                    b"node".as_slice(),
                    0o6755,
                ),
                (
                    "usr/lib/chatgpt/resources/cua_node/bin/node_repl",
                    b"repl".as_slice(),
                    0o2755,
                ),
                (
                    "usr/lib/chatgpt/resources/cua_node/lib/node_modules/package.json",
                    b"{}".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/cua_node/shared.txt",
                    b"cua-shared".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/scripts/browser-client.mjs",
                    b"client".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/scripts/browser-service.mjs",
                    b"service".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/docs/index.md",
                    b"docs".as_slice(),
                    0o644,
                ),
                (
                    "usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/chrome/shared.txt",
                    b"chrome-shared".as_slice(),
                    0o644,
                ),
                ("usr/share/unrelated.txt", b"ignored".as_slice(), 0o644),
            ] {
                append_file(tar, name, data, mode);
            }
        })
    }
    fn ar_member(name: &str, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:>10}`\n",
            name,
            0,
            0,
            0,
            0,
            body.len()
        );
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(b'\n');
        }
        out
    }
    fn deb() -> Vec<u8> {
        let mut out = AR_MAGIC.to_vec();
        out.extend(ar_member("debian-binary", b"2.0\n"));
        out.extend(ar_member("control.tar.xz", b"not used"));
        out.extend(ar_member("data.tar.xz", &tar_fixture()));
        out
    }
    fn package(root: &Path, bytes: &[u8]) -> VerifiedBrowserPackage {
        let path = root.join("input.deb");
        fs::write(&path, bytes).unwrap();
        VerifiedBrowserPackage {
            deb_path: path,
            deb_sha256: sha256(bytes),
            app_version: "1.2.3".into(),
            channel: "prod".into(),
            target: current_target().into(),
            codex_home: root.join("codex"),
        }
    }
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "browser-distribution-{}-{}",
            std::process::id(),
            unique()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn target_names_are_frozen() {
        let _default_entrypoint: fn(VerifiedBrowserPackage) -> Result<BrowserRuntimeDescriptor> =
            materialize_browser_runtime;
        assert!(valid_component("linux-x64"));
        assert!(valid_component("linux-arm64"));
        assert!(!valid_component("../x"));
        assert!(!valid_component("bad value"));
        assert_eq!(
            normalize_hash(&"A".repeat(64)).unwrap_err().to_string(),
            "browser_runtime_package_hash_invalid"
        );
    }

    #[test]
    fn synthetic_deb_materializes_only_selected_resources() {
        let root = temp();
        let p = package(&root, &deb());
        let descriptor = materialize_browser_runtime_at(&root.join("cache"), p).unwrap();
        assert_eq!(descriptor.codex_cli_path, None);
        assert!(descriptor.node_path.exists());
        assert!(descriptor.browser_client_path.exists());
        assert_eq!(
            descriptor.docs_root,
            descriptor
                .browser_client_path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("docs")
        );
        let cua_root = descriptor.node_path.parent().unwrap().parent().unwrap();
        let chrome_root = descriptor
            .browser_client_path
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        assert_eq!(
            descriptor.node_module_dirs,
            vec![cua_root.join("lib/node_modules")]
        );
        assert_eq!(descriptor.trusted_code_paths[0], root.join("codex"));
        assert_eq!(
            fs::read(cua_root.join("shared.txt")).unwrap(),
            b"cua-shared"
        );
        assert_eq!(
            fs::read(chrome_root.join("shared.txt")).unwrap(),
            b"chrome-shared"
        );
        assert!(!cua_root.join("usr/share/unrelated.txt").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let node_mode = fs::metadata(&descriptor.node_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(node_mode & 0o777, 0o755);
            assert_eq!(node_mode & 0o7000, 0);
            let lib_mode = fs::metadata(cua_root.join("lib"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(lib_mode & 0o777, 0o555);
        }
        assert!(discover_managed_browser_runtime(
            &root.join("cache"),
            current_target(),
            &root.join("codex")
        )
        .is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hash_mismatch_is_rejected_before_archive_processing() {
        let root = temp();
        let mut p = package(&root, &deb());
        p.deb_sha256 = "0".repeat(64);
        assert_eq!(
            materialize_browser_runtime_at(&root.join("cache"), p)
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_hash_mismatch"
        );
        assert!(!root.join("cache/.staging").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_data_member_is_rejected() {
        let root = temp();
        let data = tar_fixture();
        let mut bytes = AR_MAGIC.to_vec();
        bytes.extend(ar_member("debian-binary", b"2.0\n"));
        bytes.extend(ar_member("data.tar.xz", &data));
        bytes.extend(ar_member("data.tar.xz", &data));
        let mut p = package(&root, &bytes);
        p.deb_sha256 = sha256(&bytes);
        assert_eq!(
            materialize_browser_runtime_at(&root.join("cache"), p)
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_duplicate"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ar_parser_rejects_malformed_truncated_duplicate_and_unsupported_members() {
        let root = temp();
        let path = root.join("probe.deb");

        let mut odd = AR_MAGIC.to_vec();
        odd.extend(ar_member("debian-binary", b"2.0\n"));
        odd.extend(ar_member("control.tar.xz", b"x"));
        odd.extend(ar_member("data.tar.xz", &tar_fixture()));
        fs::write(&path, &odd).unwrap();
        assert!(ar_data(&path).is_ok());

        let mut unsupported = AR_MAGIC.to_vec();
        unsupported.extend(ar_member("debian-binary", b"2.0\n"));
        unsupported.extend(ar_member("data.tar.gz", b"x"));
        fs::write(&path, &unsupported).unwrap();
        assert_eq!(
            ar_data(&path).unwrap_err().to_string(),
            "browser_runtime_package_compression_unsupported"
        );

        let mut duplicate_binary = AR_MAGIC.to_vec();
        duplicate_binary.extend(ar_member("debian-binary", b"2.0\n"));
        duplicate_binary.extend(ar_member("debian-binary", b"2.0\n"));
        duplicate_binary.extend(ar_member("data.tar.xz", &tar_fixture()));
        fs::write(&path, &duplicate_binary).unwrap();
        assert_eq!(
            ar_data(&path).unwrap_err().to_string(),
            "browser_runtime_package_duplicate"
        );

        let mut truncated = odd;
        truncated.pop();
        fs::write(&path, &truncated).unwrap();
        assert_eq!(
            ar_data(&path).unwrap_err().to_string(),
            "browser_runtime_package_format"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selected_paths_reject_traversal_and_empty_aliases() {
        let prefix = "usr/lib/chatgpt/resources/cua_node";
        for path in [
            format!("{prefix}/../escape"),
            format!("{prefix}//double"),
            format!("{prefix}/a/../../escape"),
        ] {
            assert_eq!(
                selected_path(&path).unwrap_err().to_string(),
                "browser_runtime_package_path_invalid"
            );
        }
        assert!(selected_path("../usr/share/unrelated").unwrap().is_none());
        assert!(
            selected_path("usr\\lib\\chatgpt\\resources\\cua_node\\bin\\node")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn selected_links_special_types_and_duplicate_files_are_rejected() {
        for entry_type in [
            EntryType::symlink(),
            EntryType::hard_link(),
            EntryType::fifo(),
        ] {
            let root = temp();
            let tar = encoded_tar(|builder| {
                append_special(builder, "usr/lib/chatgpt/resources/cua_node", entry_type);
            });
            let mut bytes = AR_MAGIC.to_vec();
            bytes.extend(ar_member("debian-binary", b"2.0\n"));
            bytes.extend(ar_member("data.tar.xz", &tar));
            let p = package(&root, &bytes);
            assert_eq!(
                materialize_browser_runtime_at(&root.join("cache"), p)
                    .unwrap_err()
                    .to_string(),
                "browser_runtime_package_entry_type_rejected"
            );
            let _ = fs::remove_dir_all(root);
        }

        let root = temp();
        let duplicate = encoded_tar(|builder| {
            append_file(
                builder,
                "usr/lib/chatgpt/resources/cua_node/bin/node",
                b"first",
                0o755,
            );
            append_file(
                builder,
                "usr/lib/chatgpt/resources/cua_node/bin/node",
                b"second",
                0o755,
            );
        });
        let mut bytes = AR_MAGIC.to_vec();
        bytes.extend(ar_member("debian-binary", b"2.0\n"));
        bytes.extend(ar_member("data.tar.xz", &duplicate));
        let p = package(&root, &bytes);
        assert_eq!(
            materialize_browser_runtime_at(&root.join("cache"), p)
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_duplicate"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selected_entry_and_size_bounds_are_enforced_without_large_fixtures() {
        let mut count = MAX_ENTRIES;
        let mut total = 0;
        assert_eq!(
            check_selected_limits(&mut count, &mut total, None)
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_entry_limit"
        );

        let mut count = 0;
        let mut total = 0;
        assert_eq!(
            check_selected_limits(&mut count, &mut total, Some(MAX_FILE + 1))
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_size_limit"
        );

        let mut count = 0;
        let mut total = MAX_TOTAL;
        assert_eq!(
            check_selected_limits(&mut count, &mut total, Some(1))
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_size_limit"
        );
    }

    #[test]
    fn identical_install_is_idempotent_and_corrupt_artifact_repairs() {
        let root = temp();
        let cache = root.join("cache");
        let bytes = deb();
        let p = package(&root, &bytes);
        let first = materialize_browser_runtime_at(&cache, p.clone()).unwrap();
        let artifact = first
            .node_path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let before = fs::read(artifact.join("runtime.json")).unwrap();
        assert_eq!(
            materialize_browser_runtime_at(&cache, p.clone()).unwrap(),
            first
        );
        assert_eq!(fs::read(artifact.join("runtime.json")).unwrap(), before);
        fs::remove_file(&first.browser_client_path).unwrap();
        assert!(materialize_browser_runtime_at(&cache, p).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_manifest_corruption_fails_closed() {
        let root = temp();
        let cache = root.join("cache");
        let bytes = deb();
        let p = package(&root, &bytes);
        materialize_browser_runtime_at(&cache, p).unwrap();
        let active = cache
            .join("active")
            .join(format!("{}.json", current_target()));
        fs::write(&active, b"{}\n").unwrap();
        assert_eq!(
            discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
                .unwrap_err()
                .to_string(),
            "browser_runtime_cache_active_invalid"
        );

        fs::write(
            &active,
            serde_json::json!({
                "target": if current_target() == "linux-x64" { "linux-arm64" } else { "linux-x64" },
                "appVersion": "1.2.3",
                "packageSha256": "0".repeat(64)
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
                .unwrap_err()
                .to_string(),
            "browser_runtime_cache_active_invalid"
        );

        fs::write(
            &active,
            serde_json::json!({
                "target": current_target(),
                "appVersion": "../escape",
                "packageSha256": "0".repeat(64)
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
                .unwrap_err()
                .to_string(),
            "browser_runtime_cache_active_invalid"
        );

        fs::write(&active, vec![b'x'; (MAX_METADATA_BYTES + 1) as usize]).unwrap();
        assert_eq!(
            discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
                .unwrap_err()
                .to_string(),
            "browser_runtime_cache_active_invalid"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_critical_parent_in_cached_artifact_fails_closed() {
        use std::os::unix::fs::symlink;

        let root = temp();
        let cache = root.join("cache");
        let p = package(&root, &deb());
        let descriptor = materialize_browser_runtime_at(&cache, p).unwrap();
        let artifact = descriptor
            .node_path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let bin = artifact.join("cua_node/bin");
        let outside = root.join("outside-bin");
        fs::rename(&bin, &outside).unwrap();
        symlink(&outside, &bin).unwrap();
        assert_eq!(
            discover_managed_browser_runtime(&cache, current_target(), &root.join("codex"))
                .unwrap_err()
                .to_string(),
            "browser_runtime_cache_active_invalid"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn target_locks_serialize_without_global_lock() {
        let root = temp();
        let first_path = root.join("same.lock");
        let other_path = root.join("other.lock");
        let first = Lock::acquire(&first_path).unwrap();
        assert!(Lock::acquire(&other_path).is_ok());
        let blocked = std::thread::spawn(move || Lock::acquire(&first_path).is_ok());
        std::thread::sleep(std::time::Duration::from_millis(10));
        drop(first);
        assert!(blocked.join().unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dead_lock_owner_is_recovered_without_waiting_for_age_timeout() {
        let root = temp();
        let path = root.join("dead.lock");
        fs::write(&path, b"pid=4294967295\n").unwrap();
        let lock = Lock::acquire(&path).unwrap();
        assert!(path.exists());
        drop(lock);
        assert!(!path.exists());
        let _ = fs::remove_dir_all(root);
    }
}
