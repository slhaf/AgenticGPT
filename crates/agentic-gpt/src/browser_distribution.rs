//! Internal trust, acquisition, and materialization of the Browser runtime.
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use pgp::{
    composed::{CleartextSignedMessage, Deserializable, SignedPublicKey},
    crypto::hash::HashAlgorithm,
    types::KeyDetails,
};
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
const PINNED_REPOSITORY_KEY: &str = include_str!("../assets/browser/openai-linux-repo-key.asc");
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthenticatedPackagesIndex {
    pub(crate) path: String,
    pub(crate) sha256: String,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedChatgptPackage {
    pub(crate) filename: String,
    pub(crate) version: String,
    pub(crate) architecture: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
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

fn is_sha2_signature(hash: HashAlgorithm) -> bool {
    matches!(
        hash,
        HashAlgorithm::Sha256 | HashAlgorithm::Sha384 | HashAlgorithm::Sha512
    )
}

fn require_pinned_fingerprint(key: &SignedPublicKey) -> Result<()> {
    if format!("{:X}", key.fingerprint()) != PINNED_REPOSITORY_FINGERPRINT {
        return Err(error("browser_runtime_acquisition_key_fingerprint_invalid"));
    }
    Ok(())
}

pub(crate) fn parse_pinned_repository_key() -> Result<SignedPublicKey> {
    let (key, _) = SignedPublicKey::from_string(PINNED_REPOSITORY_KEY)
        .map_err(|_| error("browser_runtime_acquisition_key_invalid"))?;
    require_pinned_fingerprint(&key)?;
    key.verify_bindings()
        .map_err(|_| error("browser_runtime_acquisition_key_invalid"))?;
    Ok(key)
}

fn verify_cleartext_with_key(bytes: &[u8], key: &SignedPublicKey) -> Result<String> {
    if bytes.len() as u64 > MAX_INRELEASE_BYTES {
        return Err(error("browser_runtime_acquisition_inrelease_size_limit"));
    }
    let input = std::str::from_utf8(bytes)
        .map_err(|_| error("browser_runtime_acquisition_inrelease_invalid"))?;
    let (message, _) = CleartextSignedMessage::from_string(input)
        .map_err(|_| error("browser_runtime_acquisition_inrelease_invalid"))?;
    if message.signatures().is_empty() {
        return Err(error(
            "browser_runtime_acquisition_inrelease_signature_invalid",
        ));
    }
    if message
        .signatures()
        .iter()
        .any(|signature| !signature.hash_alg().is_some_and(is_sha2_signature))
    {
        return Err(error(
            "browser_runtime_acquisition_inrelease_signature_hash_unsupported",
        ));
    }
    let signature = message
        .verify(key)
        .map_err(|_| error("browser_runtime_acquisition_inrelease_signature_invalid"))?;
    if !signature.hash_alg().is_some_and(is_sha2_signature) {
        return Err(error(
            "browser_runtime_acquisition_inrelease_signature_hash_unsupported",
        ));
    }
    Ok(message.signed_text())
}

pub(crate) fn verify_pinned_inrelease(
    bytes: &[u8],
    architecture: &str,
) -> Result<AuthenticatedPackagesIndex> {
    let key = parse_pinned_repository_key()?;
    verify_inrelease_with_key(bytes, &key, architecture)
}

fn verify_inrelease_with_key(
    bytes: &[u8],
    key: &SignedPublicKey,
    architecture: &str,
) -> Result<AuthenticatedPackagesIndex> {
    require_pinned_fingerprint(key)?;
    let cleartext = verify_cleartext_with_key(bytes, key)?;
    parse_inrelease_metadata(&cleartext, architecture)
}

fn valid_release_field_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn lowercase_sha256(value: &str, code: &'static str) -> Result<String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(value.to_owned())
    } else {
        Err(error(code))
    }
}

fn decimal_u64(value: &str, code: &'static str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(code));
    }
    value.parse::<u64>().map_err(|_| error(code))
}

fn parse_release_date(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc2822(value).is_ok()
        || chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

fn parse_inrelease_metadata(
    cleartext: &str,
    architecture: &str,
) -> Result<AuthenticatedPackagesIndex> {
    if !matches!(architecture, "amd64" | "arm64") {
        return Err(error("browser_runtime_acquisition_target_unsupported"));
    }
    let target_path = format!("main/binary-{architecture}/Packages");
    let mut suite = None;
    let mut codename = None;
    let mut date_seen = false;
    let mut section = None;
    let mut target = None;

    for raw_line in cleartext.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            section = None;
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if section.as_deref() != Some("SHA256") {
                continue;
            }
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 {
                return Err(error(
                    "browser_runtime_acquisition_inrelease_metadata_invalid",
                ));
            }
            let digest = lowercase_sha256(
                fields[0],
                "browser_runtime_acquisition_inrelease_metadata_invalid",
            )?;
            let size = decimal_u64(
                fields[1],
                "browser_runtime_acquisition_inrelease_metadata_invalid",
            )?;
            if fields[2] == target_path {
                if target.is_some() || size == 0 || size > MAX_PACKAGES_BYTES {
                    return Err(error(
                        "browser_runtime_acquisition_inrelease_metadata_invalid",
                    ));
                }
                target = Some(AuthenticatedPackagesIndex {
                    path: target_path.clone(),
                    sha256: digest,
                    size,
                });
            }
            continue;
        }

        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| error("browser_runtime_acquisition_inrelease_metadata_invalid"))?;
        if !valid_release_field_name(name) {
            return Err(error(
                "browser_runtime_acquisition_inrelease_metadata_invalid",
            ));
        }
        let value = value.trim();
        if value.contains('\0') {
            return Err(error(
                "browser_runtime_acquisition_inrelease_metadata_invalid",
            ));
        }
        section = Some(name.to_owned());
        match name {
            "Suite" => {
                if suite.replace(value.to_owned()).is_some() || value != "stable" {
                    return Err(error(
                        "browser_runtime_acquisition_inrelease_metadata_invalid",
                    ));
                }
            }
            "Codename" => {
                if codename.replace(value.to_owned()).is_some() || value != "stable" {
                    return Err(error(
                        "browser_runtime_acquisition_inrelease_metadata_invalid",
                    ));
                }
            }
            "Date" => {
                if date_seen || !parse_release_date(value) {
                    return Err(error(
                        "browser_runtime_acquisition_inrelease_metadata_invalid",
                    ));
                }
                date_seen = true;
            }
            "SHA256" if !value.is_empty() => {
                return Err(error(
                    "browser_runtime_acquisition_inrelease_metadata_invalid",
                ));
            }
            _ => {}
        }
    }

    if (suite.is_none() && codename.is_none()) || !date_seen {
        return Err(error(
            "browser_runtime_acquisition_inrelease_metadata_invalid",
        ));
    }
    target.ok_or_else(|| error("browser_runtime_acquisition_inrelease_packages_missing"))
}

fn verify_packages_bytes(bytes: &[u8], expected_size: u64, expected_sha256: &str) -> Result<()> {
    let expected_sha256 = lowercase_sha256(
        expected_sha256,
        "browser_runtime_acquisition_packages_hash_invalid",
    )?;
    if expected_size > MAX_PACKAGES_BYTES {
        return Err(error("browser_runtime_acquisition_packages_size_limit"));
    }
    if bytes.len() as u64 != expected_size {
        return Err(error("browser_runtime_acquisition_packages_size_mismatch"));
    }
    if sha256(bytes) != expected_sha256 {
        return Err(error("browser_runtime_acquisition_packages_hash_mismatch"));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct PackagesField {
    name: String,
    value: String,
    continued: bool,
}

fn parse_packages_paragraph(lines: &[&str]) -> Result<Vec<PackagesField>> {
    let mut fields: Vec<PackagesField> = Vec::new();
    let mut current: Option<usize> = None;
    for line in lines {
        if line.len() > MAX_PACKAGES_FIELD_BYTES {
            return Err(error("browser_runtime_acquisition_packages_field_limit"));
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            let Some(index) = current else {
                return Err(error("browser_runtime_acquisition_packages_invalid"));
            };
            fields[index].continued = true;
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| error("browser_runtime_acquisition_packages_invalid"))?;
        if !valid_release_field_name(name) || value.contains('\0') {
            return Err(error("browser_runtime_acquisition_packages_invalid"));
        }
        if fields.len() >= MAX_PACKAGES_FIELDS {
            return Err(error("browser_runtime_acquisition_packages_field_limit"));
        }
        fields.push(PackagesField {
            name: name.to_owned(),
            value: value.trim().to_owned(),
            continued: false,
        });
        current = Some(fields.len() - 1);
    }
    if fields.is_empty() {
        return Err(error("browser_runtime_acquisition_packages_invalid"));
    }
    Ok(fields)
}

fn package_field<'a>(
    fields: &'a [PackagesField],
    name: &str,
    required: bool,
) -> Result<Option<&'a str>> {
    let mut found = fields.iter().filter(|field| field.name == name);
    let Some(field) = found.next() else {
        if required {
            return Err(error("browser_runtime_acquisition_package_field_invalid"));
        }
        return Ok(None);
    };
    if found.next().is_some() || field.continued {
        return Err(error("browser_runtime_acquisition_package_field_invalid"));
    }
    Ok(Some(field.value.as_str()))
}

fn validate_package_filename(value: &str) -> Result<String> {
    if value.is_empty()
        || value.len() > 512
        || value.starts_with('/')
        || value.contains('\\')
        || value.contains('?')
        || value.contains('#')
        || value.contains("://")
    {
        return Err(error(
            "browser_runtime_acquisition_package_filename_invalid",
        ));
    }
    let components = value.split('/').collect::<Vec<_>>();
    if components.len() < 5
        || components[..4] != ["pool", "main", "c", "chatgpt"]
        || !components.last().is_some_and(|name| name.ends_with(".deb"))
    {
        return Err(error(
            "browser_runtime_acquisition_package_filename_invalid",
        ));
    }
    if components
        .iter()
        .any(|component| !valid_component(component))
    {
        return Err(error(
            "browser_runtime_acquisition_package_filename_invalid",
        ));
    }
    Ok(value.to_owned())
}

pub(crate) fn select_chatgpt_package(
    bytes: &[u8],
    architecture: &str,
) -> Result<VerifiedChatgptPackage> {
    if !matches!(architecture, "amd64" | "arm64") {
        return Err(error("browser_runtime_acquisition_target_unsupported"));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| error("browser_runtime_acquisition_packages_invalid"))?;
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();
    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
                if paragraphs.len() > MAX_PACKAGES_PARAGRAPHS {
                    return Err(error(
                        "browser_runtime_acquisition_packages_paragraph_limit",
                    ));
                }
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
        if paragraphs.len() > MAX_PACKAGES_PARAGRAPHS {
            return Err(error(
                "browser_runtime_acquisition_packages_paragraph_limit",
            ));
        }
    }

    let mut matching = Vec::new();
    for paragraph in paragraphs {
        let fields = parse_packages_paragraph(&paragraph)?;
        let Some(package) = package_field(&fields, "Package", false)? else {
            continue;
        };
        if package != "chatgpt" {
            continue;
        }
        let architecture_value = package_field(&fields, "Architecture", true)?
            .ok_or_else(|| error("browser_runtime_acquisition_package_field_invalid"))?;
        if architecture_value != architecture {
            continue;
        }
        let version = package_field(&fields, "Version", true)?
            .ok_or_else(|| error("browser_runtime_acquisition_package_field_invalid"))?;
        identity(
            version,
            "browser_runtime_acquisition_package_version_invalid",
        )?;
        let filename = package_field(&fields, "Filename", true)?
            .ok_or_else(|| error("browser_runtime_acquisition_package_field_invalid"))?;
        let filename = validate_package_filename(filename)?;
        let size_text = package_field(&fields, "Size", true)?
            .ok_or_else(|| error("browser_runtime_acquisition_package_field_invalid"))?;
        let size = decimal_u64(
            size_text,
            "browser_runtime_acquisition_package_size_invalid",
        )?;
        if size == 0 || size > MAX_DEB_BYTES {
            return Err(error("browser_runtime_acquisition_package_size_invalid"));
        }
        let sha256 = package_field(&fields, "SHA256", true)?
            .ok_or_else(|| error("browser_runtime_acquisition_package_field_invalid"))?;
        let sha256 = lowercase_sha256(sha256, "browser_runtime_acquisition_package_hash_invalid")?;
        matching.push(VerifiedChatgptPackage {
            filename,
            version: version.to_owned(),
            architecture: architecture_value.to_owned(),
            size,
            sha256,
        });
    }
    match matching.len() {
        1 => Ok(matching.remove(0)),
        0 => Err(error("browser_runtime_acquisition_package_missing")),
        _ => Err(error("browser_runtime_acquisition_package_ambiguous")),
    }
}

pub(crate) fn verify_and_select_chatgpt_package(
    bytes: &[u8],
    index: &AuthenticatedPackagesIndex,
    architecture: &str,
) -> Result<VerifiedChatgptPackage> {
    verify_packages_bytes(bytes, index.size, &index.sha256)?;
    select_chatgpt_package(bytes, architecture)
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

fn packages_fetch_path(index: &AuthenticatedPackagesIndex) -> Result<String> {
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
    let index = verify_pinned_inrelease(&inrelease, architecture)?;
    let packages_path = packages_fetch_path(&index)?;
    let packages = fetch_bounded_bytes(
        fetcher.clone(),
        &packages_path,
        MAX_PACKAGES_BYTES.min(index.size),
    )
    .await?;
    let selected = verify_and_select_chatgpt_package(&packages, &index, architecture)?;
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
mod tests {
    use super::*;
    use std::io::Cursor;
    use tar::Builder;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use xz2::write::XzEncoder;

    const TEST_PUBLIC_KEY: &str = r#"-----BEGIN PGP PUBLIC KEY BLOCK-----
Version: GnuPG v2

mQENBFdqP+gBCACoG81sddF9ZZx6TsN7lenDxs53wqJt1bawToXZl6qJ+hO8tMNy
/aUgaqqi8pC5FX3HlSRQhjRFDQXq4jR3+jecFK3wfwmxTKXMKhN52zhSIWClED47
56B3wPITuwAG2WYFccClhrtWX8j+wBK5IpLVVAXQnORZLOP7fzUZkO1KDu3bP4D6
f1LQyatIzgS08dnlp6WvsJDrkUeGY6R5smpx9f+PqBVfUVpmckbMOR+BYPJtplZT
0lez4qWsmdWbqU7ZhdvnhMGcTbBbk9WbfR1IJptigeuK8vTU24Jp2FQj6iIBV9OD
jUfr11EJ6W3TvHaWuddd0hfX9DnyH9rghrDFABEBAAG0FnN0ZXZlLmJpa29AZXhh
bXBsZS5uZXSJATcEEwEIACEFAldqP+gCGwMFCwkIBwIGFQgJCgsCBBYCAwECHgEC
F4AACgkQqkPx3Mf+0bd5kggAphS7UDycKadfaRH5JENmKXeI+UUd+E0iERwv7eXq
RcgjNK1oHQSXN+ejDEXzZv2fcCRB7rWEvEXL0pCtPveyzDAQJdhZTRVgmfCXTr1m
9pJfVC3B20jgx6ZxZO8jKDL+bqvufWJczWDT0iHP0Jv04SqASLRs2JRPy+a+w3GJ
+DzG8orfAKiIE1Qycovr8Ol+jdo9ZV9blRA8/j4eqZYg4b7AOf8/mDyXsx3xzSPV
uwkDSluhaOrsV8N0suZ51rfdpapv6VJsXlyQbceJwwgSt2A1n2Sw3ZINwpO7BODy
wO6J44751+qY4cmap4NItyqGQTT6TUEL9ANfrZFmPWmFWLkBDQRXaj/oAQgA255C
UJxFEKLVwEoSgwZqXd94AhjGUbMY6NXdFj5cCq0JmWZrbpT/5OblTrymiH1iLmI0
ymo+/s8vh6NtB98dhr1syH3asNQfXZRfF+u5X5hLDNPF4sUelsl4+EUef0Hbc9U+
e+8F8A9TMxELSqQ8Ul3Hu42hc+/ugkc1G/8++Sv/f60TqWcUR2GmuiAvkuS1WmdA
TMhwPr7vMfssV0X0mboz32//b/UfuOyctso5FM+bRaKrEJDQ2WDg57yqnaqsKEga
jW0jElpAVIn792W6YWKOk4auYSpO5f7BVs40Z+bxKGxiH87z9fnmlYAsQwPOOxZw
WaCSrReeheK6c6emAQARAQABiQEfBBgBCAAJBQJXaj/oAhsMAAoJEKpD8dzH/tG3
baoH/0KI3pIUiIYiLESGXqF+s/W2BmGNwdkYldcyFwkXz84VXoG0B3k7nrwT2DOJ
AEeToavzd3J+aZ4PmxBRAMtDhah0wsMXrwCI8y9Stmm6PIssnu9IP9+jgr4IkKIR
UB/Wn6nzgseaNd7vN4JChCyLSvF+vLd3D56Wzq+hBjybaE+zcEusVLdKYDm2i0YC
pkBkmSuC18lLxhNC8oSCCvVOiyw+TqGHhLnrpA4nGi0MLjAR3OgJ5d/TclYgkLcp
yOupg9GplQsAZUFfQPrY80SJuN9ijBp4xtA1U+WCGKh4ySv1+odpRjPX3eOGUFKZ
sJRKpZupoGWfVN78wm1nPLBKTvM=
=6N/A
-----END PGP PUBLIC KEY BLOCK-----"#;

    const TEST_SIGNED_CLEARTEXT: &str = r#"-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

You are scrupulously honest, frank, and straightforward.  Therefore you
have few friends.
-----BEGIN PGP SIGNATURE-----
Version: GnuPG v2

iQE0BAEBCAAeBQJXakWmFxxzdGV2ZS5iaWtvQGV4YW1wbGUubmV0AAoJEKpD8dzH
/tG3OiIH/18NlMSXXRFRrxXq9OZySzJxgLI7BjGilRTqb4ALeFzNjmCwu3Y+Gkdg
t7NjYjSe0erWiKYDEmALICwcpmSmXHA//gol3QkHJKIlKQGXJP1qLvIde5+lnK8K
YVwLKLBQBQtlGMkMXPdUEn9PgzSoBFoFIqrzQmAdLO3yijSdm0Mzl9wyIhtbUXk+
VgX2d/6DRIwcKcFoX2QbFlM/z1kdrS6cOYFbJWavEpLDz9ON8Q8a8uqcBiqRlSpW
eGOMMsysJs+44+qX6uE3hu2KJE9xvHwhSjJOxqtw8dN3KZ1+8IkxsDrvDAhn+Klf
Hbtj647f/iTOF88o1ihO7goDi93Bpv4=
=xAv4
-----END PGP SIGNATURE-----"#;

    #[test]
    fn pinned_repository_key_has_exact_fingerprint() {
        let key = parse_pinned_repository_key().unwrap();
        assert_eq!(
            format!("{:X}", key.fingerprint()),
            PINNED_REPOSITORY_FINGERPRINT
        );
    }

    #[test]
    fn cleartext_signature_verifies_and_tampering_fails() {
        let (key, _) = SignedPublicKey::from_string(TEST_PUBLIC_KEY).unwrap();
        let authenticated =
            verify_cleartext_with_key(TEST_SIGNED_CLEARTEXT.as_bytes(), &key).unwrap();
        assert!(authenticated.contains("scrupulously honest"));

        let tampered = TEST_SIGNED_CLEARTEXT.replace("honest", "dishonest");
        assert_eq!(
            verify_cleartext_with_key(tampered.as_bytes(), &key)
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_inrelease_signature_invalid"
        );
    }

    #[test]
    fn cleartext_signature_hash_policy_is_sha2_only() {
        let (key, _) = SignedPublicKey::from_string(TEST_PUBLIC_KEY).unwrap();
        assert!(verify_cleartext_with_key(TEST_SIGNED_CLEARTEXT.as_bytes(), &key).is_ok());
        assert!(!is_sha2_signature(HashAlgorithm::Md5));
        assert!(!is_sha2_signature(HashAlgorithm::Sha1));
        assert!(!is_sha2_signature(HashAlgorithm::Sha224));
        assert!(!is_sha2_signature(HashAlgorithm::Sha3_256));
        assert!(is_sha2_signature(HashAlgorithm::Sha256));
        assert!(is_sha2_signature(HashAlgorithm::Sha384));
        assert!(is_sha2_signature(HashAlgorithm::Sha512));
    }

    #[test]
    fn authenticated_inrelease_metadata_selects_exact_target() {
        let amd64 = "a".repeat(64);
        let arm64 = "b".repeat(64);
        let body = format!(
            "Suite: stable\nCodename: stable\nDate: Tue, 1 Jul 2003 10:52:37 +0200\n\
             SHA256:\n {amd64} 42 main/binary-amd64/Packages\n {arm64} 43 main/binary-arm64/Packages\n"
        );
        let index = parse_inrelease_metadata(&body, "amd64").unwrap();
        assert_eq!(
            index,
            AuthenticatedPackagesIndex {
                path: "main/binary-amd64/Packages".into(),
                sha256: amd64,
                size: 42,
            }
        );
        let fetch_path = packages_fetch_path(&index).unwrap();
        assert_eq!(fetch_path, "dists/stable/main/binary-amd64/Packages");
    }

    #[test]
    fn release_date_parser_accepts_rfc2822_timestamp() {
        assert!(parse_release_date("Tue, 1 Jul 2003 10:52:37 +0200"));
        assert!(!parse_release_date("Tue, 1 Jul 2003 10:52:37 XYZ"));
    }

    #[test]
    fn authenticated_inrelease_metadata_rejects_duplicates_and_contradictions() {
        let digest = "a".repeat(64);
        let duplicate = format!(
            "Suite: stable\nCodename: stable\nSHA256:\n {digest} 1 main/binary-amd64/Packages\n\
             {digest} 1 main/binary-amd64/Packages\n"
        );
        assert_eq!(
            parse_inrelease_metadata(&duplicate, "amd64")
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_inrelease_metadata_invalid"
        );
        let contradictory = format!(
            "Suite: testing\nCodename: stable\nSHA256:\n {digest} 1 main/binary-amd64/Packages\n"
        );
        assert_eq!(
            parse_inrelease_metadata(&contradictory, "amd64")
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_inrelease_metadata_invalid"
        );
    }
    #[derive(Clone, Default)]
    struct TestFetcher {
        responses: Arc<std::sync::Mutex<Vec<(String, u16, Vec<u8>)>>>,
        package: Arc<std::sync::Mutex<Option<(u16, Vec<u8>)>>>,
    }

    impl TestFetcher {
        fn response(&self, path: &str, status: u16, body: &[u8]) {
            self.responses
                .lock()
                .unwrap()
                .push((path.to_owned(), status, body.to_vec()));
        }
    }

    impl BrowserDistributionFetcher for TestFetcher {
        fn fetch_bytes(&self, path: String, _max_bytes: u64) -> BrowserDistributionFuture {
            let response = self
                .responses
                .lock()
                .unwrap()
                .iter()
                .find(|(candidate, _, _)| candidate == &path)
                .map(|(_, status, body)| BrowserDistributionResponse {
                    status: *status,
                    body: body.clone(),
                });
            Box::pin(async move {
                response.ok_or_else(|| error("browser_runtime_acquisition_http_failed"))
            })
        }

        fn stream_file(
            &self,
            _path: String,
            destination: tokio::fs::File,
            _expected_size: u64,
            _expected_sha256: String,
        ) -> BrowserDistributionFuture {
            let package = self.package.lock().unwrap().clone();
            Box::pin(async move {
                let Some((status, body)) = package else {
                    return Err(error("browser_runtime_acquisition_http_failed"));
                };
                let mut output = destination;
                output
                    .write_all(&body)
                    .await
                    .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
                output
                    .sync_all()
                    .await
                    .map_err(|_| error("browser_runtime_acquisition_temp_failed"))?;
                Ok(BrowserDistributionResponse {
                    status,
                    body: Vec::new(),
                })
            })
        }
    }

    async fn local_http_response(
        status: u16,
        body: &[u8],
        declared_length: Option<usize>,
        location: Option<&str>,
    ) -> (reqwest::Response, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_vec();
        let declared_length = declared_length.unwrap_or(body.len());
        let location = location.map(str::to_owned);
        let task = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await;
                let reason = if status == 200 { "OK" } else { "Found" };
                let mut response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {declared_length}\r\n\
                     Connection: close\r\n"
                );
                if let Some(location) = location {
                    response.push_str(&format!("Location: {location}\r\n"));
                }
                response.push_str("\r\n");
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            }
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let response = client
            .get(format!("http://{address}/fixture"))
            .send()
            .await
            .unwrap();
        (response, task)
    }

    #[tokio::test]
    async fn injected_fetcher_rejects_status_and_metadata_bounds() {
        let fetcher = TestFetcher::default();
        fetcher.response(INRELEASE_PATH, 302, b"redirect");
        assert_eq!(
            fetch_bounded_bytes(
                Arc::new(fetcher.clone()),
                INRELEASE_PATH,
                MAX_INRELEASE_BYTES
            )
            .await
            .unwrap_err()
            .to_string(),
            "browser_runtime_acquisition_http_status"
        );
        fetcher.response(
            "oversized",
            200,
            &vec![b'x'; (MAX_INRELEASE_BYTES + 1) as usize],
        );
        assert_eq!(
            fetch_bounded_bytes(Arc::new(fetcher), "oversized", MAX_INRELEASE_BYTES)
                .await
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_response_size_limit"
        );
    }

    #[test]
    fn fixed_repository_url_rejects_untrusted_path_forms() {
        let url = fixed_repository_url(INRELEASE_PATH).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some(OPENAI_REPOSITORY_HOST));
        for path in [
            "../evil",
            "pool/main/c/chatgpt/../evil.deb",
            "https://evil.example/package.deb",
            "pool/main/c/chatgpt/package.deb?redirect=1",
            "pool/main/c/chatgpt/package.deb#fragment",
        ] {
            assert_eq!(
                fixed_repository_url(path).unwrap_err().to_string(),
                "browser_runtime_acquisition_url_invalid"
            );
        }
    }

    #[tokio::test]
    async fn local_http_status_redirect_and_body_bounds_are_rejected() {
        let (redirect, task) = local_http_response(
            302,
            b"redirect",
            None,
            Some("http://127.0.0.1:9/should-not-follow"),
        )
        .await;
        assert_eq!(
            fetch_http_response(redirect, MAX_INRELEASE_BYTES)
                .await
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_http_status"
        );
        task.await.unwrap();

        let body = vec![b'x'; 4];
        let (oversized, task) = local_http_response(200, &body, None, None).await;
        assert_eq!(
            fetch_http_response(oversized, 3)
                .await
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_response_size_limit"
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn streamed_package_checks_size_hash_and_temp_cleanup() {
        let root = temp();
        let body = b"small package bytes";
        let destination = root.join("package.deb");
        let (guard, destination_file) = AcquisitionTempFile::claim(destination.clone()).unwrap();
        let (response, task) = local_http_response(200, body, None, None).await;
        stream_http_response(response, destination_file, body.len() as u64, sha256(body))
            .await
            .unwrap();
        task.await.unwrap();
        assert_eq!(fs::read(&destination).unwrap(), body);
        drop(guard);
        assert!(!destination.exists());

        let mismatch = root.join("mismatch.deb");
        let (guard, mismatch_file) = AcquisitionTempFile::claim(mismatch.clone()).unwrap();
        let (response, task) = local_http_response(200, body, None, None).await;
        assert_eq!(
            stream_http_response(response, mismatch_file, body.len() as u64, "a".repeat(64),)
                .await
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_download_hash_mismatch"
        );
        task.await.unwrap();
        drop(guard);
        assert!(!mismatch.exists());

        let overshoot = root.join("overshoot.deb");
        let (guard, overshoot_file) = AcquisitionTempFile::claim(overshoot.clone()).unwrap();
        let (response, task) = local_http_response(200, body, None, None).await;
        assert_eq!(
            stream_http_response(
                response,
                overshoot_file,
                body.len() as u64 - 1,
                sha256(body),
            )
            .await
            .unwrap_err()
            .to_string(),
            "browser_runtime_acquisition_download_oversize"
        );
        task.await.unwrap();
        drop(guard);
        assert!(!overshoot.exists());

        let short = root.join("short.deb");
        let (guard, short_file) = AcquisitionTempFile::claim(short.clone()).unwrap();
        let (response, task) = local_http_response(200, body, None, None).await;
        assert_eq!(
            stream_http_response(response, short_file, body.len() as u64 + 1, sha256(body),)
                .await
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_download_size_mismatch"
        );
        task.await.unwrap();
        drop(guard);
        assert!(!short.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn provisioning_lock_rechecks_cache_after_waiting() {
        let root = temp();
        let cache = root.join("cache");
        let codex_home = root.join("codex");
        fs::create_dir_all(cache.join(".locks")).unwrap();
        let lock_path = cache
            .join(".locks")
            .join(format!("{}.provision.lock", current_target()));
        let held = Lock::acquire(&lock_path).unwrap();
        let fetcher = Arc::new(TestFetcher::default());
        let waiting_cache = cache.clone();
        let waiting_codex_home = codex_home.clone();
        let waiting_target = current_target().to_owned();
        let waiting_fetcher = fetcher.clone();
        let waiting = tokio::spawn(async move {
            provision_managed_browser_runtime_with_fetcher(
                &waiting_cache,
                &waiting_codex_home,
                &waiting_target,
                waiting_fetcher,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let bytes = deb();
        let package = package(&root, &bytes);
        let expected = materialize_browser_runtime_at(&cache, package).unwrap();
        drop(held);

        let actual = waiting.await.unwrap().unwrap();
        assert_eq!(actual, expected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn acquisition_temp_cleanup_handles_materializer_success_and_failure() {
        let root = temp();
        let cache = root.join("cache");
        let bytes = deb();
        let input = root.join("input.deb");
        fs::write(&input, &bytes).unwrap();
        let successful = root.join("successful.part");
        let (guard, successful_file) = AcquisitionTempFile::claim(successful.clone()).unwrap();
        drop(successful_file);
        fs::write(&successful, &bytes).unwrap();
        let package = VerifiedBrowserPackage {
            deb_path: input.clone(),
            deb_sha256: sha256(&bytes),
            app_version: "1.2.3".into(),
            channel: "prod".into(),
            target: current_target().into(),
            codex_home: root.join("codex"),
        };
        assert!(materialize_browser_runtime_at(&cache, package).is_ok());
        drop(guard);
        assert!(!successful.exists());

        let failed = root.join("failed.part");
        let (guard, failed_file) = AcquisitionTempFile::claim(failed.clone()).unwrap();
        drop(failed_file);
        fs::write(&failed, &bytes).unwrap();
        let package = VerifiedBrowserPackage {
            deb_path: failed.clone(),
            deb_sha256: "b".repeat(64),
            app_version: "1.2.3".into(),
            channel: "prod".into(),
            target: current_target().into(),
            codex_home: root.join("codex"),
        };
        assert_eq!(
            materialize_browser_runtime_at(&cache, package)
                .unwrap_err()
                .to_string(),
            "browser_runtime_package_hash_mismatch"
        );
        drop(guard);
        assert!(!failed.exists());
        let _ = fs::remove_dir_all(root);
    }

    fn package_paragraph(
        version: &str,
        architecture: &str,
        filename: &str,
        size: &str,
        digest: &str,
    ) -> String {
        format!(
            "Package: chatgpt\nVersion: {version}\nArchitecture: {architecture}\n\
             Filename: {filename}\nSize: {size}\nSHA256: {digest}\n"
        )
    }

    #[test]
    fn packages_digest_and_size_are_verified_before_selection() {
        let digest = "a".repeat(64);
        let bytes = package_paragraph(
            "1.2.3",
            "amd64",
            "pool/main/c/chatgpt/chatgpt_1.2.3_amd64.deb",
            "3",
            &digest,
        )
        .into_bytes();
        let index = AuthenticatedPackagesIndex {
            path: "main/binary-amd64/Packages".into(),
            sha256: sha256(&bytes),
            size: bytes.len() as u64,
        };
        assert_eq!(
            verify_and_select_chatgpt_package(&bytes, &index, "amd64")
                .unwrap()
                .version,
            "1.2.3"
        );
        let mut wrong_hash = index.clone();
        wrong_hash.sha256 = "b".repeat(64);
        assert_eq!(
            verify_and_select_chatgpt_package(&bytes, &wrong_hash, "amd64")
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_packages_hash_mismatch"
        );
        let mut wrong_size = index;
        wrong_size.size += 1;
        assert_eq!(
            verify_and_select_chatgpt_package(&bytes, &wrong_size, "amd64")
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_packages_size_mismatch"
        );
    }

    #[test]
    #[ignore = "manual real-package materialization smoke"]
    fn real_package_materializes_from_explicit_fixture_path() {
        let deb_path = std::env::var_os("AGENTIC_BROWSER_REAL_DEB")
            .map(PathBuf::from)
            .expect("AGENTIC_BROWSER_REAL_DEB must point to a verified official .deb");
        let deb_sha256 = std::env::var("AGENTIC_BROWSER_REAL_DEB_SHA256")
            .expect("AGENTIC_BROWSER_REAL_DEB_SHA256 must contain the verified package digest");
        let app_version = std::env::var("AGENTIC_BROWSER_REAL_APP_VERSION")
            .expect("AGENTIC_BROWSER_REAL_APP_VERSION must contain the verified package version");
        let scratch = temp();
        let cache_root = std::env::var_os("AGENTIC_BROWSER_REAL_CACHE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| scratch.join("cache"));
        let package = VerifiedBrowserPackage {
            deb_path,
            deb_sha256,
            app_version: app_version.clone(),
            channel: "prod".into(),
            target: current_target().into(),
            codex_home: scratch.join("codex-home"),
        };
        let descriptor = materialize_browser_runtime_at(&cache_root, package).unwrap();
        assert_eq!(descriptor.app_version, app_version);
        assert_eq!(descriptor.channel, "prod");
        assert!(descriptor.node_repl_path.is_file());
        assert!(descriptor.browser_client_path.is_file());
        assert!(descriptor.docs_root.is_dir());
        let chrome_root = descriptor
            .browser_service_path
            .parent()
            .and_then(Path::parent)
            .expect("browser service remains inside the selected chrome bundle");
        assert!(descriptor
            .trusted_code_paths
            .iter()
            .any(|path| path == chrome_root));
        let _ = fs::remove_dir_all(scratch);
    }

    #[test]
    fn package_selector_rejects_ambiguity_duplicate_fields_and_unsafe_names() {
        let digest = "a".repeat(64);
        let first = package_paragraph(
            "1.2.3",
            "amd64",
            "pool/main/c/chatgpt/chatgpt_1.2.3_amd64.deb",
            "3",
            &digest,
        );
        assert_eq!(
            select_chatgpt_package(format!("{first}\n{first}").as_bytes(), "amd64")
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_package_ambiguous"
        );
        let duplicate = format!("{first}SHA256: {digest}\n");
        assert_eq!(
            select_chatgpt_package(duplicate.as_bytes(), "amd64")
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_package_field_invalid"
        );
        let unsafe_filename = package_paragraph(
            "1.2.3",
            "amd64",
            "pool/main/c/chatgpt/../escape.deb",
            "3",
            &digest,
        );
        assert_eq!(
            select_chatgpt_package(unsafe_filename.as_bytes(), "amd64")
                .unwrap_err()
                .to_string(),
            "browser_runtime_acquisition_package_filename_invalid"
        );
    }

    #[test]
    fn package_selector_validates_required_identity_fields() {
        let digest = "a".repeat(64);
        for (version, size, hash, expected) in [
            (
                "bad/version",
                "3",
                digest.as_str(),
                "browser_runtime_acquisition_package_version_invalid",
            ),
            (
                "1.2.3",
                "0",
                digest.as_str(),
                "browser_runtime_acquisition_package_size_invalid",
            ),
            (
                "1.2.3",
                "3",
                "ABC",
                "browser_runtime_acquisition_package_hash_invalid",
            ),
        ] {
            let paragraph = package_paragraph(
                version,
                "amd64",
                "pool/main/c/chatgpt/chatgpt_1.2.3_amd64.deb",
                size,
                hash,
            );
            assert_eq!(
                select_chatgpt_package(paragraph.as_bytes(), "amd64")
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
    }
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
        assert!(descriptor
            .trusted_code_paths
            .iter()
            .any(|path| path == chrome_root));
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
        assert_eq!(
            selected_path(&format!("{prefix}/bin/")).unwrap().unwrap().1,
            PathBuf::from("bin")
        );
        for path in [
            format!("{prefix}/../escape"),
            format!("{prefix}//double"),
            format!("{prefix}/bin//"),
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
