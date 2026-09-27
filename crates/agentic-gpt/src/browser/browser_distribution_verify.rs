use anyhow::Result;
use pgp::{
    composed::{CleartextSignedMessage, Deserializable, SignedPublicKey},
    crypto::hash::HashAlgorithm,
    types::KeyDetails,
};

use super::{
    error, identity, sha256, valid_component, MAX_DEB_BYTES, MAX_INRELEASE_BYTES,
    MAX_PACKAGES_BYTES, MAX_PACKAGES_FIELDS, MAX_PACKAGES_FIELD_BYTES, MAX_PACKAGES_PARAGRAPHS,
    PINNED_REPOSITORY_FINGERPRINT, PINNED_REPOSITORY_KEY,
};

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

pub(super) fn is_sha2_signature(hash: HashAlgorithm) -> bool {
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

pub(super) fn parse_pinned_repository_key() -> Result<SignedPublicKey> {
    let (key, _) = SignedPublicKey::from_string(PINNED_REPOSITORY_KEY)
        .map_err(|_| error("browser_runtime_acquisition_key_invalid"))?;
    require_pinned_fingerprint(&key)?;
    key.verify_bindings()
        .map_err(|_| error("browser_runtime_acquisition_key_invalid"))?;
    Ok(key)
}

pub(super) fn verify_cleartext_with_key(bytes: &[u8], key: &SignedPublicKey) -> Result<String> {
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

pub(super) fn parse_release_date(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc2822(value).is_ok()
        || chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

pub(super) fn parse_inrelease_metadata(
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
