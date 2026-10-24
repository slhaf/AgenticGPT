//! Opt-in image downloads and a bounded snapshot cache. HTML parsing stays in the worker.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::{Host, Url};

const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: usize = 12 * 1024 * 1024;
const MAX_RESOURCES: usize = 16;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CACHE_FILE_BYTES: u64 = (MAX_DOWNLOAD_BYTES as u64).div_ceil(3) * 4 + 128 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const CACHE_ERROR: &str = "email_render_cache_unavailable";
const PREPARED_TOO_LARGE: &str = "email_render_prepared_too_large";

type Result<T> = std::result::Result<T, String>;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CachedImages {
    created_at: u64,
    // Original URI -> validated, static raster bytes in base64. No HTML rewriting.
    images: BTreeMap<String, String>,
}

/// Called under the renderer's concurrency permit. Continuations only read snapshots.
pub(crate) async fn prepare(
    mail_id: &str,
    html: &str,
    cursor_present: bool,
) -> Result<BTreeMap<String, String>> {
    if mail_id.is_empty()
        || !mail_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return Err("email_id_invalid".to_owned());
    }
    let started = tokio::time::Instant::now();
    tokio::time::timeout(TOTAL_TIMEOUT, async move {
        let cache = Arc::new(
            tokio::task::spawn_blocking(Cache::open)
                .await
                .map_err(|_| CACHE_ERROR.to_owned())??,
        );
        let lock = Arc::new(cache.acquire_lock().await?);
        let key = format!("remote-{:x}.json", Sha256::digest(html.as_bytes()));
        let read_key = key.clone();
        if let Some(images) = cache_work(cache.clone(), lock.clone(), move |cache| {
            cache.read(&read_key)
        })
        .await?
        {
            return Ok(images);
        }
        if cursor_present {
            return Err("email_render_cache_expired".to_owned());
        }
        let sources = crate::email_render::discover(html).await?;
        let mut images = BTreeMap::new();
        let mut downloaded_bytes = 0;
        let mut stored_bytes = 0;
        let network_deadline = started + Duration::from_secs(12);
        for source in sources
            .into_iter()
            .filter(|source| remote_candidate(source))
            .take(MAX_RESOURCES)
        {
            let deadline = network_deadline.min(tokio::time::Instant::now() + REQUEST_TIMEOUT);
            if tokio::time::Instant::now() >= network_deadline {
                break;
            }
            if let Ok(Some(image)) =
                tokio::time::timeout_at(deadline, fetch_image(&source, &mut downloaded_bytes)).await
            {
                stored_bytes += image.len();
                if stored_bytes > MAX_DOWNLOAD_BYTES.div_ceil(3) * 4 {
                    break;
                }
                images.insert(source, image);
            }
        }
        cache_work(cache, lock, move |cache| {
            cache.write(&key, &images)?;
            Ok(images)
        })
        .await
    })
    .await
    .map_err(|_| "email_remote_images_timeout".to_owned())?
}

// Retain the cross-process lock even when cancellation leaves blocking disk work
// running. Atomic rename and locked cleanup protect complete snapshots.
async fn cache_work<T: Send + 'static>(
    cache: Arc<Cache>,
    lock: Arc<File>,
    work: impl FnOnce(&Cache) -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || {
        let _lock = lock;
        work(&cache)
    })
    .await
    .map_err(|_| CACHE_ERROR.to_owned())?
}

fn remote_candidate(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("//")
}

async fn fetch_image(source: &str, total: &mut usize) -> Option<String> {
    if *total >= MAX_DOWNLOAD_BYTES || source.len() > 4096 {
        return None;
    }
    // Scheme-relative URLs have no email origin; use HTTPS rather than guessing HTTP.
    let source = if source.starts_with("//") {
        format!("https:{source}")
    } else {
        source.to_owned()
    };
    let mut url = Url::parse(&source).ok()?;
    url.set_fragment(None);
    for hop in 0..=3 {
        let addresses = tokio::time::timeout(REQUEST_TIMEOUT, checked_addresses(&url))
            .await
            .ok()??;
        let host = url.host_str()?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(REQUEST_TIMEOUT)
            .resolve_to_addrs(host, &addresses)
            .build()
            .ok()?;
        let mut response = client
            .get(url.clone())
            .header(
                reqwest::header::ACCEPT,
                "image/png,image/jpeg,image/webp,image/gif",
            )
            .send()
            .await
            .ok()?;
        if response.status().is_redirection() {
            if hop == 3 {
                return None;
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)?
                .to_str()
                .ok()?;
            if location.len() > 4096 {
                return None;
            }
            url = url.join(location).ok()?;
            url.set_fragment(None);
            continue;
        }
        if !response.status().is_success() {
            return None;
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)?
            .to_str()
            .ok()?
            .split(';')
            .next()?
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            content_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return None;
        }
        let budget = MAX_IMAGE_BYTES.min(MAX_DOWNLOAD_BYTES.saturating_sub(*total));
        if response
            .content_length()
            .is_some_and(|len| len > budget as u64)
        {
            return None;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            *total = total.saturating_add(chunk.len());
            if *total > MAX_DOWNLOAD_BYTES || bytes.len().saturating_add(chunk.len()) > budget {
                return None;
            }
            bytes.extend_from_slice(&chunk);
        }
        // The shared decoder validates raster bytes and bounds decoded pixels/memory.
        return tokio::task::spawn_blocking(move || {
            let output = crate::file_ops::read_image_bytes(
                &bytes,
                crate::file_ops::MAX_IMAGE_RESPONSE_BYTES,
            )
            .ok()??;
            let image = output.images.into_iter().next()?;
            if image.bytes.len() > MAX_IMAGE_BYTES {
                return None;
            }
            Some(STANDARD.encode(image.bytes))
        })
        .await
        .ok()?;
    }
    None
}

async fn checked_addresses(url: &Url) -> Option<Vec<SocketAddr>> {
    let port = match url.scheme() {
        "http" if url.port_or_known_default() == Some(80) => 80,
        "https" if url.port_or_known_default() == Some(443) => 443,
        _ => return None,
    };
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let mut addresses = match url.host()? {
        Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        Host::Domain(host) => tokio::net::lookup_host((host, port))
            .await
            .ok()?
            .take(17)
            .collect(),
    };
    if addresses.is_empty()
        || addresses.len() > 16
        || addresses.iter().any(|addr| !public_ip(addr.ip()))
    {
        return None;
    }
    addresses.sort_unstable();
    addresses.dedup();
    Some(addresses)
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_ipv4(ip),
        IpAddr::V6(ip) => public_ipv6(ip),
    }
}

// Conservative exclusions from https://www.iana.org/assignments/iana-ipv4-special-registry
fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !matches!(a, 0 | 10 | 127 | 224..=255)
        && !(a == 100 && (64..=127).contains(&b))
        && !(a == 169 && b == 254)
        && !(a == 172 && (16..=31).contains(&b))
        && !(a == 192 && (b == 168 || (b == 0 && matches!(c, 0 | 2)) || (b == 88 && c == 99)))
        && !(a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        && !(a == 203 && b == 0 && c == 113)
}

// See https://www.iana.org/assignments/iana-ipv6-special-registry
fn public_ipv6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    // Only ordinary global unicast. This excludes mapped/translated IPv4,
    // loopback, local/link-local, multicast, and reserved address families.
    (s[0] & 0xe000) == 0x2000
        && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
        && s[0] != 0x2002
        && !(s[0] == 0x3fff && s[1] < 0x1000)
}

struct Cache {
    directory: File,
}
impl Cache {
    fn open() -> Result<Self> {
        let base = crate::utils::agentic_home().map_err(|_| CACHE_ERROR.to_string())?;
        let home = base.parent().ok_or_else(|| CACHE_ERROR.to_string())?;
        // HOME may legitimately contain symlinks. Protect the private state
        // directories below it, rather than rejecting its ancestor layout.
        let mut directory = File::open(home).map_err(|_| CACHE_ERROR.to_string())?;
        for name in [".agentic_gpt", "cache", "email"] {
            directory = open_directory(&directory, std::ffi::OsStr::new(name), true)?;
            directory
                .set_permissions(fs::Permissions::from_mode(0o700))
                .map_err(|_| CACHE_ERROR.to_string())?;
        }
        Ok(Self { directory })
    }

    fn path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd())).join(name)
    }

    async fn acquire_lock(&self) -> Result<File> {
        // Keep the directory descriptor alive while its /proc path is opened.
        let directory = self
            .directory
            .try_clone()
            .map_err(|_| CACHE_ERROR.to_string())?;
        let path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
            .join(".email-remote.lock");
        let file = tokio::task::spawn_blocking(move || {
            let _directory = directory;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
                .open(path)
                .map_err(|_| CACHE_ERROR.to_string())?;
            if !file
                .metadata()
                .map_err(|_| CACHE_ERROR.to_string())?
                .is_file()
            {
                return Err(CACHE_ERROR.to_string());
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| CACHE_ERROR.to_string())?;
            Ok::<_, String>(file)
        })
        .await
        .map_err(|_| CACHE_ERROR.to_string())??;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(file);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(CACHE_ERROR.to_string());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    fn read(&self, key: &str) -> Result<Option<BTreeMap<String, String>>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(self.path(key))
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(CACHE_ERROR.to_string()),
        };
        let metadata = file.metadata().map_err(|_| CACHE_ERROR.to_string())?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
            return Err(CACHE_ERROR.to_string());
        }
        // Older HTML snapshots could be larger than the new image-only limit.
        // Derived content can be regenerated; do not make it a permanent error.
        if metadata.len() > MAX_CACHE_FILE_BYTES {
            return Ok(None);
        }
        let mut bytes = Vec::new();
        file.take(MAX_CACHE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CACHE_ERROR.to_string())?;
        let cached: CachedImages = match serde_json::from_slice(&bytes) {
            Ok(cached) => cached,
            Err(_) => return Ok(None),
        };
        let now = now_seconds()?;
        if cached.created_at > now || now - cached.created_at >= CACHE_TTL.as_secs() {
            return Ok(None);
        }
        if cached.images.len() > MAX_RESOURCES
            || cached.images.iter().any(|(uri, image)| {
                uri.len() > 4096 || image.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4
            })
        {
            return Ok(None);
        }
        Ok(Some(cached.images))
    }

    fn cleanup(&self, incoming: u64) -> Result<()> {
        let mut entries = Vec::new();
        let now = SystemTime::now();
        for entry in fs::read_dir(self.path("")).map_err(|_| CACHE_ERROR.to_string())? {
            let entry = entry.map_err(|_| CACHE_ERROR.to_string())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let final_file = cache_name(name);
            let temp_file = name.starts_with(".email-remote-") && name.ends_with(".tmp");
            if !final_file && !temp_file {
                continue;
            }
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|_| CACHE_ERROR.to_string())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(CACHE_ERROR.to_string());
            }
            let modified = metadata.modified().map_err(|_| CACHE_ERROR.to_string())?;
            let expired = now
                .duration_since(modified)
                .map_or(true, |age| age >= CACHE_TTL);
            if temp_file || expired {
                fs::remove_file(entry.path()).map_err(|_| CACHE_ERROR.to_string())?;
            } else {
                entries.push((modified, metadata.len(), entry.path()));
            }
        }
        entries.sort_unstable_by_key(|entry| entry.0);
        let mut total: u64 = entries.iter().map(|entry| entry.1).sum();
        for (_, bytes, path) in entries {
            if total.saturating_add(incoming) <= MAX_CACHE_BYTES {
                break;
            }
            fs::remove_file(path).map_err(|_| CACHE_ERROR.to_string())?;
            total = total.saturating_sub(bytes);
        }
        Ok(())
    }

    fn write(&self, key: &str, images: &BTreeMap<String, String>) -> Result<()> {
        let value = CachedImages {
            created_at: now_seconds()?,
            images: images.clone(),
        };
        let bytes = serde_json::to_vec(&value).map_err(|_| CACHE_ERROR.to_string())?;
        if bytes.len() as u64 > MAX_CACHE_FILE_BYTES {
            return Err(PREPARED_TOO_LARGE.to_string());
        }
        self.cleanup(bytes.len() as u64)?;
        let temporary = self.path(&format!(
            ".email-remote-{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temporary)
                .map_err(|_| CACHE_ERROR.to_string())?;
            file.write_all(&bytes)
                .map_err(|_| CACHE_ERROR.to_string())?;
            file.sync_all().map_err(|_| CACHE_ERROR.to_string())?;
            fs::rename(&temporary, self.path(key)).map_err(|_| CACHE_ERROR.to_string())?;
            self.directory
                .sync_all()
                .map_err(|_| CACHE_ERROR.to_string())?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

fn cache_name(name: &str) -> bool {
    name.strip_prefix("remote-")
        .and_then(|name| name.strip_suffix(".json"))
        .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn open_directory(parent: &File, name: &std::ffi::OsStr, create: bool) -> Result<File> {
    let name =
        std::ffi::CString::new(name.as_encoded_bytes()).map_err(|_| CACHE_ERROR.to_string())?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let mut descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if descriptor < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
        let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
        if created != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
            return Err(CACHE_ERROR.to_string());
        }
        descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    }
    if descriptor < 0 {
        return Err(CACHE_ERROR.to_string());
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn now_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| CACHE_ERROR.to_string())
}
