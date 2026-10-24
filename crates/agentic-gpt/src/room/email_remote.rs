//! Explicit remote-image preparation. The renderer itself never accesses the network.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine};
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, serialize, Attribute, QualName};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::{Host, Url};

const MAX_IMAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: usize = 12 * 1024 * 1024;
const MAX_RESOURCES: usize = 16;
const MAX_HTML_BYTES: usize = 16 * 1024 * 1024;
const MAX_DOM_NODES: usize = 100_000;
const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CACHE_FILE_BYTES: u64 = (MAX_HTML_BYTES as u64).div_ceil(3) * 4 + 1024;
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
const UNAVAILABLE: &str = "email-remote-image-unavailable:";
const CACHE_ERROR: &str = "email_render_cache_unavailable";
const PREPARED_TOO_LARGE: &str = "email_render_prepared_too_large";
static PREPARE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type Result<T> = std::result::Result<T, String>;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CachedHtml {
    created_at: u64,
    // Base64 bounds JSON expansion even for unusual control characters in HTML.
    html_base64: String,
}

/// Image-only network access is opt-in at the caller. A continuation never refetches.
pub(crate) async fn prepare(mail_id: &str, html: &str, cursor_present: bool) -> Result<String> {
    if mail_id.is_empty()
        || !mail_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return Err("email_id_invalid".to_string());
    }
    if html.len() > MAX_HTML_BYTES {
        return Err(PREPARED_TOO_LARGE.to_string());
    }
    let html = html.to_owned();
    let started = tokio::time::Instant::now();
    tokio::time::timeout(TOTAL_TIMEOUT, async move {
        let _guard = PREPARE_LOCK.lock().await;
        let cache = Arc::new(
            tokio::task::spawn_blocking(Cache::open)
                .await
                .map_err(|_| CACHE_ERROR.to_string())??,
        );
        let file_lock = Arc::new(cache.acquire_lock().await?);
        let key = format!("remote-{:x}.json", Sha256::digest(html.as_bytes()));
        let read_key = key.clone();
        if let Some(prepared) = cache_work(cache.clone(), file_lock.clone(), move |cache| {
            cache.read(&read_key)
        })
        .await?
        {
            return Ok(prepared);
        }
        if cursor_present {
            return Err("email_render_cache_expired".to_string());
        }
        cache_work(cache.clone(), file_lock.clone(), |cache| cache.cleanup(0)).await?;
        let source_html = html.clone();
        let sources = tokio::task::spawn_blocking(move || sources(&source_html))
            .await
            .map_err(|_| "email_render_html_invalid".to_string())??;
        let mut replacements = BTreeMap::new();
        let mut downloaded_bytes = 0;
        // Reserve time for the final DOM rewrite/cache commit. The full call is capped at 15s.
        let network_deadline = started + Duration::from_secs(12);
        for (index, source) in sources.into_iter().enumerate() {
            let deadline = network_deadline.min(tokio::time::Instant::now() + REQUEST_TIMEOUT);
            let image = if tokio::time::Instant::now() < network_deadline {
                tokio::time::timeout_at(deadline, fetch_image(&source, &mut downloaded_bytes))
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
            replacements.insert(
                source,
                image.unwrap_or_else(|| format!("{UNAVAILABLE}{}", index + 1)),
            );
        }
        let prepared = tokio::task::spawn_blocking(move || rewrite(&html, &replacements))
            .await
            .map_err(|_| "email_render_html_invalid".to_string())??;
        cache_work(cache, file_lock, move |cache| {
            cache.write(&key, &prepared)?;
            Ok(prepared)
        })
        .await
    })
    .await
    .map_err(|_| "email_remote_images_timeout".to_string())?
}

// The shared file lock stays held if a cancelled caller leaves a bounded disk
// operation finishing in the blocking pool. Another process cannot race cleanup.
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
    .map_err(|_| CACHE_ERROR.to_string())?
}

fn parse(html: &str) -> Result<RcDom> {
    if html
        .bytes()
        .filter(|byte| *byte == b'<')
        .take(MAX_DOM_NODES + 1)
        .count()
        > MAX_DOM_NODES
    {
        return Err("email_render_html_complexity_exceeded".to_string());
    }
    Ok(parse_document(RcDom::default(), Default::default()).one(html))
}

/// Use DOM attributes after HTML entity decoding. Never inspect arbitrary script/text URLs.
fn sources(html: &str) -> Result<Vec<String>> {
    let dom = parse(html)?;
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    walk(&dom.document, |handle| {
        if let NodeData::Element { name, attrs, .. } = &handle.data {
            let attrs = attrs.borrow();
            if name.local.as_ref() == "img" {
                if let Some(source) = image_source(&attrs).filter(|source| remote_candidate(source))
                {
                    push_source(&mut result, &mut seen, &source);
                }
            }
            if let Some(css) = attribute(&attrs, "style") {
                visit_css_urls(css, |_, source| {
                    if remote_candidate(source) {
                        push_source(&mut result, &mut seen, source);
                    }
                    Ok(())
                })?;
            }
            if name.local.as_ref() == "style" {
                for child in handle.children.borrow().iter() {
                    if let NodeData::Text { contents } = &child.data {
                        visit_css_urls(contents.borrow().as_ref(), |_, source| {
                            if remote_candidate(source) {
                                push_source(&mut result, &mut seen, source);
                            }
                            Ok(())
                        })?;
                    }
                }
            }
            // Legacy table/body background images are common in email.
            if let Some(source) = attribute(&attrs, "background")
                .filter(|source| legacy_background(name.local.as_ref()) && remote_candidate(source))
            {
                push_source(&mut result, &mut seen, source);
            }
        }
        Ok(())
    })?;
    Ok(result)
}

fn push_source(result: &mut Vec<String>, seen: &mut BTreeSet<String>, source: &str) {
    if result.len() < MAX_RESOURCES && seen.insert(source.to_owned()) {
        result.push(source.to_owned());
    }
}

fn walk(root: &Handle, mut visit: impl FnMut(&Handle) -> Result<()>) -> Result<()> {
    let mut stack = vec![(root.clone(), 0usize)];
    let mut count = 0;
    while let Some((handle, depth)) = stack.pop() {
        count += 1;
        if count > MAX_DOM_NODES || depth > 128 {
            return Err("email_render_html_complexity_exceeded".to_string());
        }
        visit(&handle)?;
        // Templates are inert. Only the document tree rendered by litehtml is visited.
        stack.extend(
            handle
                .children
                .borrow()
                .iter()
                .rev()
                .map(|child| (child.clone(), depth + 1)),
        );
    }
    Ok(())
}

fn attribute<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|a| a.name.ns.as_ref().is_empty() && a.name.local.as_ref() == name)
        .map(|a| a.value.as_ref().trim())
}

fn image_source(attributes: &[Attribute]) -> Option<String> {
    attribute(attributes, "src")
        .filter(|source| !source.is_empty())
        .map(str::to_owned)
        .or_else(|| attribute(attributes, "srcset").and_then(first_srcset_url))
}

/// Select the first srcset candidate when src is absent; descriptors do not affect layout here.
fn first_srcset_url(srcset: &str) -> Option<String> {
    let srcset = srcset.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
    let url = srcset
        .split_ascii_whitespace()
        .next()?
        .trim_end_matches(',');
    (!url.is_empty()).then(|| url.to_owned())
}

fn remote_candidate(source: &str) -> bool {
    let lower = source.trim_start().to_ascii_lowercase();
    lower.starts_with("http:") || lower.starts_with("https:") || lower.starts_with("//")
}

fn legacy_background(tag: &str) -> bool {
    matches!(tag, "body" | "table" | "tr" | "td" | "th")
}

fn rewritten_source(source: &str, replacements: &mut BTreeMap<String, String>) -> String {
    if remote_candidate(source) {
        let index = replacements.len() + 1;
        replacements
            .entry(source.to_owned())
            .or_insert_with(|| format!("{UNAVAILABLE}{index}"))
            .clone()
    } else {
        source.to_owned()
    }
}

fn rewrite(html: &str, replacements: &BTreeMap<String, String>) -> Result<String> {
    let dom = parse(html)?;
    let mut replacements = replacements.clone();
    let mut prepared_bytes = html.len();
    walk(&dom.document, |handle| {
        if let NodeData::Element { name, attrs, .. } = &handle.data {
            let mut attrs = attrs.borrow_mut();
            if name.local.as_ref() == "img" {
                if let Some(source) = image_source(&attrs) {
                    let original_len = source.len();
                    let source = rewritten_source(&source, &mut replacements);
                    expansion(&mut prepared_bytes, original_len, source.len())?;
                    attrs.retain(|a| !matches!(a.name.local.as_ref(), "src" | "srcset"));
                    attrs.push(Attribute {
                        name: QualName::new(None, Default::default(), "src".into()),
                        value: source.into(),
                    });
                }
            }
            for attr in attrs.iter_mut() {
                if attr.name.local.as_ref() == "background" {
                    let next = rewritten_source(attr.value.as_ref().trim(), &mut replacements);
                    expansion(&mut prepared_bytes, attr.value.len(), next.len())?;
                    attr.value = next.into();
                } else if attr.name.local.as_ref() == "style" {
                    let next = rewrite_css(attr.value.as_ref(), &mut replacements)?;
                    expansion(&mut prepared_bytes, attr.value.len(), next.len())?;
                    attr.value = next.into();
                }
            }
            if name.local.as_ref() == "style" {
                for child in handle.children.borrow().iter() {
                    if let NodeData::Text { contents } = &child.data {
                        let mut css = contents.borrow_mut();
                        let next = rewrite_css(css.as_ref(), &mut replacements)?;
                        expansion(&mut prepared_bytes, css.len(), next.len())?;
                        *css = next.into();
                    }
                }
            }
        }
        Ok(())
    })?;
    let mut output = BoundedOutput(Vec::new());
    serialize(
        &mut output,
        &SerializableHandle::from(dom.document.clone()),
        Default::default(),
    )
    .map_err(|_| PREPARED_TOO_LARGE.to_string())?;
    String::from_utf8(output.0).map_err(|_| "email_render_html_invalid".to_string())
}

fn expansion(total: &mut usize, previous: usize, next: usize) -> Result<()> {
    *total = total.saturating_sub(previous).saturating_add(next);
    if *total > MAX_HTML_BYTES {
        return Err(PREPARED_TOO_LARGE.to_string());
    }
    Ok(())
}

fn visit_css_urls(
    css: &str,
    mut visit: impl FnMut(std::ops::Range<usize>, &str) -> Result<()>,
) -> Result<()> {
    static DECLARATIONS: OnceLock<regex::Regex> = OnceLock::new();
    static URLS: OnceLock<regex::Regex> = OnceLock::new();
    // A restricted image-property subset deliberately excludes @import and font-face src.
    let declarations = DECLARATIONS.get_or_init(|| regex::Regex::new(
        r"(?is)(?:^|[;{}])\s*(?:background(?:-image)?|border-image(?:-source)?|list-style(?:-image)?|content)\s*:\s*([^;{}]*)"
    ).expect("constant CSS declaration pattern"));
    let urls = URLS.get_or_init(|| {
        regex::Regex::new(
            r#"(?is)\burl\(\s*(?:"([^"\\\r\n]*)"|'([^'\\\r\n]*)'|([^"'\\\s()]+))\s*\)"#,
        )
        .expect("constant CSS image pattern")
    });
    let visible = mask_css_comments(css);
    for declaration in declarations.captures_iter(&visible) {
        let value = declaration.get(1).expect("declaration value capture");
        for captures in urls.captures_iter(value.as_str()) {
            let whole = captures.get(0).expect("URL capture");
            let source = captures
                .get(1)
                .or_else(|| captures.get(2))
                .or_else(|| captures.get(3))
                .expect("URL value capture");
            let start = value.start() + source.start();
            let end = value.start() + source.end();
            // Comments were replaced by spaces at identical byte positions.
            visit(
                value.start() + whole.start()..value.start() + whole.end(),
                css[start..end].trim(),
            )?;
        }
    }
    Ok(())
}

fn mask_css_comments(css: &str) -> String {
    let mut bytes = css.as_bytes().to_vec();
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(quoted) = quote {
            if byte == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if byte == quoted {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            // Keep quoted URL arguments, but mask ordinary CSS strings so text
            // such as content:"url(...)" cannot trigger a network request.
            let prefix = css[..index].trim_end();
            let url_argument = prefix.len() >= 4
                && prefix.as_bytes()[prefix.len() - 4..].eq_ignore_ascii_case(b"url(");
            if url_argument {
                quote = Some(byte);
            } else {
                let mut end = index + 1;
                while end < bytes.len() {
                    if bytes[end] == b'\\' {
                        end = (end + 2).min(bytes.len());
                        continue;
                    }
                    if bytes[end] == byte {
                        end += 1;
                        break;
                    }
                    end += 1;
                }
                bytes[index..end].fill(b' ');
                index = end;
                continue;
            }
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            let end = bytes[index + 2..]
                .windows(2)
                .position(|pair| pair == b"*/")
                .map_or(bytes.len(), |offset| index + 2 + offset + 2);
            bytes[index..end].fill(b' ');
            index = end;
            continue;
        }
        index += 1;
    }
    String::from_utf8(bytes).expect("masking whole comments preserves UTF-8")
}

fn rewrite_css(css: &str, replacements: &mut BTreeMap<String, String>) -> Result<String> {
    let mut output = BoundedOutput(Vec::new());
    let mut offset = 0;
    visit_css_urls(css, |range, source| {
        output
            .write_all(&css.as_bytes()[offset..range.start])
            .map_err(|_| PREPARED_TOO_LARGE.to_string())?;
        if remote_candidate(source) {
            let next = rewritten_source(source, replacements);
            output
                .write_all(b"url(\"")
                .and_then(|_| output.write_all(next.as_bytes()))
                .and_then(|_| output.write_all(b"\")"))
                .map_err(|_| PREPARED_TOO_LARGE.to_string())?;
        } else {
            output
                .write_all(css[range.clone()].as_bytes())
                .map_err(|_| PREPARED_TOO_LARGE.to_string())?;
        }
        offset = range.end;
        Ok(())
    })?;
    output
        .write_all(&css.as_bytes()[offset..])
        .map_err(|_| PREPARED_TOO_LARGE.to_string())?;
    String::from_utf8(output.0).map_err(|_| "email_render_html_invalid".to_string())
}

struct BoundedOutput(Vec<u8>);
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_HTML_BYTES {
            return Err(io::Error::other(PREPARED_TOO_LARGE));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
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
            Some(format!(
                "data:{};base64,{}",
                image.mime_type,
                STANDARD.encode(image.bytes)
            ))
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
        let mut directory = File::open("/").map_err(|_| CACHE_ERROR.to_string())?;
        for component in home.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => directory = open_directory(&directory, name, false)?,
                _ => return Err(CACHE_ERROR.to_string()),
            }
        }
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

    fn read(&self, key: &str) -> Result<Option<String>> {
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
        if !metadata.is_file()
            || metadata.permissions().mode() & 0o777 != 0o600
            || metadata.len() > MAX_CACHE_FILE_BYTES
        {
            return Err(CACHE_ERROR.to_string());
        }
        let mut bytes = Vec::new();
        file.take(MAX_CACHE_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| CACHE_ERROR.to_string())?;
        let cached: CachedHtml = match serde_json::from_slice(&bytes) {
            Ok(cached) => cached,
            Err(_) => return Ok(None),
        };
        let now = now_seconds()?;
        if cached.created_at > now || now - cached.created_at >= CACHE_TTL.as_secs() {
            return Ok(None);
        }
        let html = match STANDARD.decode(cached.html_base64) {
            Ok(html) if html.len() <= MAX_HTML_BYTES => html,
            _ => return Ok(None),
        };
        Ok(String::from_utf8(html).ok())
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

    fn write(&self, key: &str, html: &str) -> Result<()> {
        let value = CachedHtml {
            created_at: now_seconds()?,
            html_base64: STANDARD.encode(html),
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
