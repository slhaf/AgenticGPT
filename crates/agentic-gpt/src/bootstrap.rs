use agentic_gpt_protocol::{
    BootstrapDocumentKind, BootstrapEncoding, BootstrapEntrypoint, BootstrapGuideSummary,
    BootstrapLoadPolicy, BootstrapReadRequest, BootstrapReadResponse, BootstrapResponse,
    BootstrapTextResource,
};
use anyhow::{anyhow, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::{config::Config, state::AppState};

const BOOTSTRAP_DIR: &str = "bootstrap";
const ENTRYPOINT_NAME: &str = "bootstrap.md";
const GUIDES_DIR: &str = "guides";
const ENTRYPOINT_MAX_BYTES: usize = 65_536;
const GUIDE_MAX_BYTES: usize = 262_144;
const MAX_RETURNED_GUIDES: usize = 64;
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone)]
struct GuideDocument {
    id: String,
    path: String,
    retained_bytes: Option<Vec<u8>>,
    frontmatter: Value,
    summary: BootstrapGuideSummary,
}

struct LoadedPackage {
    entrypoint: BootstrapEntrypoint,
    guides: Vec<GuideDocument>,
    warnings: Vec<String>,
    revision: String,
}

pub(crate) async fn load(state: &AppState) -> Result<BootstrapResponse> {
    let config = state.config.read().await.clone();
    load_config(&config)
}

pub(crate) async fn read(
    state: &AppState,
    request: BootstrapReadRequest,
) -> Result<BootstrapReadResponse> {
    let config = state.config.read().await.clone();
    read_config(&config, &request)
}

fn load_config(config: &Config) -> Result<BootstrapResponse> {
    let package = load_package(config, None)?;
    let total_guides = package.guides.len();
    let returned_guides = total_guides.min(MAX_RETURNED_GUIDES);
    let mut warnings = package.warnings;
    if total_guides > returned_guides {
        warnings.push(format!(
            "guides_truncated: total={total_guides}; returned={returned_guides}"
        ));
    }
    Ok(BootstrapResponse {
        schema_version: SCHEMA_VERSION,
        revision: package.revision,
        entrypoint: package.entrypoint,
        guides: package
            .guides
            .iter()
            .take(returned_guides)
            .map(|guide| guide.summary.clone())
            .collect(),
        total_guides,
        returned_guides,
        warnings,
    })
}

fn read_config(config: &Config, request: &BootstrapReadRequest) -> Result<BootstrapReadResponse> {
    let package = load_package(config, Some(&request.id))?;
    let guide = package
        .guides
        .iter()
        .find(|guide| guide.id == request.id)
        .ok_or_else(|| anyhow!("guide_not_found"))?;
    let bytes = guide
        .retained_bytes
        .as_deref()
        .ok_or_else(|| anyhow!("bootstrap_read_failed"))?;
    let (resource, warning) =
        build_resource(bytes, &guide.path, GUIDE_MAX_BYTES, "guide_truncated")?;
    Ok(BootstrapReadResponse {
        guide: guide.summary.clone(),
        frontmatter: guide.frontmatter.clone(),
        resource,
        warnings: warning.into_iter().collect(),
    })
}

fn load_package(config: &Config, retain_guide_id: Option<&str>) -> Result<LoadedPackage> {
    let bootstrap_root = config.workspace_root.join(BOOTSTRAP_DIR);
    ensure_directory(&bootstrap_root, "bootstrap_not_found", "bootstrap_invalid")?;

    let entrypoint_path = bootstrap_root.join(ENTRYPOINT_NAME);
    let entrypoint_bytes = read_required_file(&entrypoint_path)?;
    let entrypoint_text =
        String::from_utf8(entrypoint_bytes.clone()).map_err(|_| anyhow!("bootstrap_invalid"))?;
    let entrypoint_frontmatter =
        parse_required_frontmatter(&entrypoint_text).map_err(|_| anyhow!("bootstrap_invalid"))?;
    let (entrypoint_id, name, description) =
        validate_entrypoint(&entrypoint_frontmatter).map_err(|_| anyhow!("bootstrap_invalid"))?;
    let (entrypoint_resource, entrypoint_warning) = build_resource(
        &entrypoint_bytes,
        ENTRYPOINT_NAME,
        ENTRYPOINT_MAX_BYTES,
        "entrypoint_truncated",
    )?;
    let entrypoint = BootstrapEntrypoint {
        id: entrypoint_id,
        kind: BootstrapDocumentKind::Entrypoint,
        name,
        description,
        frontmatter: entrypoint_frontmatter,
        resource: entrypoint_resource,
    };

    let (guides, mut warnings) = scan_guides(&bootstrap_root, retain_guide_id)?;
    if let Some(warning) = entrypoint_warning {
        warnings.insert(0, warning);
    }
    let revision = package_revision(&entrypoint_bytes, &guides);
    Ok(LoadedPackage {
        entrypoint,
        guides,
        warnings,
        revision,
    })
}

fn ensure_directory(path: &Path, missing_code: &str, invalid_code: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!(missing_code.to_string())
        } else {
            anyhow!(invalid_code.to_string())
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!(invalid_code.to_string()));
    }
    Ok(())
}

fn read_required_file(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!("bootstrap_not_found")
        } else {
            anyhow!("bootstrap_invalid")
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(anyhow!("bootstrap_invalid"));
    }
    fs::read(path).map_err(|_| anyhow!("bootstrap_read_failed"))
}

fn scan_guides(
    bootstrap_root: &Path,
    retain_guide_id: Option<&str>,
) -> Result<(Vec<GuideDocument>, Vec<String>)> {
    let guides_root = bootstrap_root.join(GUIDES_DIR);
    let metadata = match fs::symlink_metadata(&guides_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()));
        }
        Err(_) => {
            return Ok((
                Vec::new(),
                vec![format!(
                    "guide_dir_entry_unreadable: path={}",
                    display_path(&guides_root)
                )],
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Ok((
            Vec::new(),
            vec![format!(
                "guides_dir_symlink_ignored: path={}",
                display_path(&guides_root)
            )],
        ));
    }
    if !metadata.is_dir() {
        return Ok((
            Vec::new(),
            vec![format!(
                "guide_dir_entry_unreadable: path={}; reason=not_directory",
                display_path(&guides_root)
            )],
        ));
    }

    let mut entries = match fs::read_dir(&guides_root) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .collect::<Vec<_>>(),
        Err(_) => {
            return Ok((
                Vec::new(),
                vec![format!(
                    "guide_dir_entry_unreadable: path={}",
                    display_path(&guides_root)
                )],
            ));
        }
    };
    entries.sort_by(|left, right| display_path(left).cmp(&display_path(right)));

    let mut guides = Vec::new();
    let mut warnings = Vec::new();
    for path in entries {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_name.starts_with('.') || path.extension().and_then(|ext| ext.to_str()) != Some("md")
        {
            continue;
        }
        let relative_path = format!("{GUIDES_DIR}/{file_name}");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                warnings.push(format!("guide_unreadable: path={relative_path}"));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            warnings.push(format!("guide_symlink_ignored: path={relative_path}"));
            continue;
        }
        if !metadata.is_file() {
            warnings.push(format!(
                "guide_unreadable: path={relative_path}; reason=not_regular_file"
            ));
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                warnings.push(format!("guide_unreadable: path={relative_path}"));
                continue;
            }
        };
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(_) => {
                warnings.push(format!("guide_non_utf8: path={relative_path}"));
                continue;
            }
        };
        let frontmatter = match parse_required_frontmatter(&text) {
            Ok(frontmatter) => frontmatter,
            Err(error) => {
                warnings.push(format!(
                    "guide_frontmatter_invalid: path={relative_path}; detail={error}"
                ));
                continue;
            }
        };
        let metadata = match validate_guide(&frontmatter, &relative_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.push(format!(
                    "guide_metadata_invalid: path={relative_path}; detail={error}"
                ));
                continue;
            }
        };
        let size_bytes = bytes.len() as u64;
        let total_lines = logical_line_count(&bytes);
        let sha256 = hex_sha256(&bytes);
        guides.push(GuideDocument {
            id: metadata.id.clone(),
            path: relative_path,
            retained_bytes: (retain_guide_id == Some(metadata.id.as_str())).then_some(bytes),
            frontmatter,
            summary: BootstrapGuideSummary {
                id: metadata.id,
                kind: BootstrapDocumentKind::Guide,
                title: metadata.title,
                summary: metadata.summary,
                load_policy: metadata.load_policy,
                priority: metadata.priority,
                load_when: metadata.load_when,
                tool_bindings: metadata.tool_bindings,
                tags: metadata.tags,
                path: metadata.path,
                size_bytes,
                total_lines,
                sha256,
            },
        });
    }

    let mut ids = BTreeMap::<String, usize>::new();
    for guide in &guides {
        *ids.entry(guide.id.clone()).or_default() += 1;
    }
    guides.retain(|guide| {
        if ids.get(&guide.id).copied().unwrap_or_default() > 1 {
            warnings.push(format!(
                "guide_duplicate_id: id={}; path={}",
                guide.id, guide.path
            ));
            false
        } else {
            true
        }
    });
    warnings.sort();
    guides.sort_by(|left, right| {
        right
            .summary
            .priority
            .cmp(&left.summary.priority)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok((guides, warnings))
}

struct GuideMetadata {
    id: String,
    title: String,
    summary: String,
    load_policy: BootstrapLoadPolicy,
    priority: i32,
    load_when: Vec<String>,
    tool_bindings: Vec<String>,
    tags: Vec<String>,
    path: String,
}

fn validate_entrypoint(frontmatter: &Value) -> Result<(String, String, String)> {
    let id = required_string(frontmatter, "id")?;
    validate_id(&id)?;
    if required_string(frontmatter, "kind")? != "entrypoint" {
        return Err(anyhow!("kind"));
    }
    let name = required_string(frontmatter, "name")?;
    let description = required_string(frontmatter, "description")?;
    let schema_version = frontmatter
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("schemaVersion"))?;
    if schema_version != SCHEMA_VERSION as u64 {
        return Err(anyhow!("schemaVersion_unsupported"));
    }
    Ok((id, name, description))
}

fn validate_guide(frontmatter: &Value, path: &str) -> Result<GuideMetadata> {
    let id = required_string(frontmatter, "id")?;
    validate_id(&id)?;
    if required_string(frontmatter, "kind")? != "guide" {
        return Err(anyhow!("kind"));
    }
    let title = required_string(frontmatter, "title")?;
    let summary = required_string(frontmatter, "summary")?;
    let load_policy = match frontmatter.get("loadPolicy") {
        None => BootstrapLoadPolicy::OnDemand,
        Some(Value::String(value)) => match value.as_str() {
            "startup" => BootstrapLoadPolicy::Startup,
            "contextual" => BootstrapLoadPolicy::Contextual,
            "on_demand" => BootstrapLoadPolicy::OnDemand,
            _ => return Err(anyhow!("loadPolicy")),
        },
        Some(_) => return Err(anyhow!("loadPolicy")),
    };
    let priority = match frontmatter.get("priority") {
        None => 0,
        Some(value) => value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| anyhow!("priority"))?,
    };
    let load_when = string_array(frontmatter, "loadWhen")?;
    let tool_bindings = string_array(frontmatter, "toolBindings")?;
    let tags = string_array(frontmatter, "tags")?;
    Ok(GuideMetadata {
        id,
        title,
        summary,
        load_policy,
        priority,
        load_when,
        tool_bindings,
        tags,
        path: path.to_string(),
    })
}

fn required_string(frontmatter: &Value, key: &str) -> Result<String> {
    let value = frontmatter
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!(key.to_string()))?;
    if value.trim().is_empty() {
        return Err(anyhow!(key.to_string()));
    }
    Ok(value.to_string())
}

fn string_array(frontmatter: &Value, key: &str) -> Result<Vec<String>> {
    let Some(value) = frontmatter.get(key) else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| anyhow!(key.to_string()))?;
    values
        .iter()
        .map(|value| {
            let value = value.as_str().ok_or_else(|| anyhow!(key.to_string()))?;
            if value.trim().is_empty() {
                return Err(anyhow!(key.to_string()));
            }
            Ok(value.to_string())
        })
        .collect()
}

fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id == "." || id == ".." {
        return Err(anyhow!("id"));
    }
    if !id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(anyhow!("id"));
    }
    Ok(())
}

fn parse_required_frontmatter(text: &str) -> Result<Value> {
    let normalized = text.replace("\r\n", "\n");
    if !normalized.starts_with("---\n") {
        return Err(anyhow!("missing"));
    }
    let rest = &normalized[4..];
    let end = rest
        .find("\n---")
        .filter(|index| {
            let after_marker = *index + "\n---".len();
            after_marker == rest.len() || rest[after_marker..].starts_with('\n')
        })
        .ok_or_else(|| anyhow!("unclosed"))?;
    let yaml = &rest[..end];
    let value = serde_yaml::from_str::<serde_yaml::Value>(yaml)
        .map_err(|error| anyhow!(error.to_string()))?;
    let value = serde_json::to_value(value).map_err(|error| anyhow!(error.to_string()))?;
    if !value.is_object() {
        return Err(anyhow!("expected_object"));
    }
    Ok(value)
}

fn build_resource(
    bytes: &[u8],
    path: &str,
    max_bytes: usize,
    warning_code: &str,
) -> Result<(BootstrapTextResource, Option<String>)> {
    let size_bytes = bytes.len() as u64;
    let sha256 = hex_sha256(bytes);
    let total_lines = logical_line_count(bytes);
    let (returned_bytes, truncated, last_line_complete) = if bytes.len() <= max_bytes {
        (bytes, false, true)
    } else {
        let mut boundary = max_bytes;
        while boundary > 0 && std::str::from_utf8(&bytes[..boundary]).is_err() {
            boundary -= 1;
        }
        let bounded = &bytes[..boundary];
        let complete_line_boundary = bounded
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|index| index + 1);
        match complete_line_boundary {
            Some(end) => (&bytes[..end], true, true),
            None => (bounded, true, false),
        }
    };
    let content = std::str::from_utf8(returned_bytes)
        .map_err(|_| anyhow!("bootstrap_invalid"))?
        .to_string();
    let returned_size_bytes = returned_bytes.len() as u64;
    let returned_through_line = logical_line_count(returned_bytes);
    let omitted_from_line = if truncated {
        Some(if last_line_complete {
            returned_through_line + 1
        } else {
            returned_through_line.max(1)
        })
    } else {
        None
    };
    let warning = truncated.then(|| {
        format!(
            "{warning_code}: path={path}; sizeBytes={size_bytes}; returnedSizeBytes={returned_size_bytes}; omittedFromLine={}",
            omitted_from_line.unwrap_or(1)
        )
    });
    Ok((
        BootstrapTextResource {
            path: path.to_string(),
            encoding: BootstrapEncoding::Utf8,
            content,
            media_type: "text/markdown".to_string(),
            size_bytes,
            returned_size_bytes,
            total_lines,
            returned_through_line,
            omitted_from_line,
            truncated,
            last_line_complete,
            sha256,
        },
        warning,
    ))
}

fn package_revision(entrypoint_bytes: &[u8], guides: &[GuideDocument]) -> String {
    let mut canonical = Vec::new();
    canonical.extend_from_slice(b"agentic-room-bootstrap-v1\0");
    canonical.extend_from_slice(b"schemaVersion\0");
    canonical.extend_from_slice(SCHEMA_VERSION.to_string().as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(b"entrypoint\0bootstrap.md\0");
    canonical.extend_from_slice(hex_sha256(entrypoint_bytes).as_bytes());
    canonical.push(0);
    for guide in guides {
        canonical.extend_from_slice(b"guide\0");
        canonical.extend_from_slice(guide.id.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(guide.path.as_bytes());
        canonical.push(0);
        canonical.extend_from_slice(guide.summary.sha256.as_bytes());
        canonical.push(0);
    }
    hex_sha256(&canonical)
}

fn logical_line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
    if bytes.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    fn test_config(name: &str) -> (Config, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "agentic-bootstrap-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root.clone();
        (config, root)
    }

    fn write_entrypoint(root: &Path, extra: &str) {
        let bootstrap = root.join(BOOTSTRAP_DIR);
        fs::create_dir_all(&bootstrap).unwrap();
        fs::write(
            bootstrap.join(ENTRYPOINT_NAME),
            format!(
                "---\nid: room\nkind: entrypoint\nname: Room Bootstrap\ndescription: Route guides\nschemaVersion: 1\n---\n{extra}"
            ),
        )
        .unwrap();
    }

    fn write_guide(root: &Path, file_name: &str, frontmatter: &str, body: &str) {
        let guides = root.join(BOOTSTRAP_DIR).join(GUIDES_DIR);
        fs::create_dir_all(&guides).unwrap();
        fs::write(
            guides.join(file_name),
            format!("---\n{frontmatter}\n---\n{body}"),
        )
        .unwrap();
    }

    #[test]
    fn missing_package_and_invalid_entrypoint_are_fail_closed() {
        let (config, root) = test_config("missing");
        assert_eq!(
            load_config(&config).unwrap_err().to_string(),
            "bootstrap_not_found"
        );

        write_entrypoint(&root, "");
        fs::write(
            root.join(BOOTSTRAP_DIR).join(ENTRYPOINT_NAME),
            "---\nid: room\nkind: guide\n---\n",
        )
        .unwrap();
        assert_eq!(
            load_config(&config).unwrap_err().to_string(),
            "bootstrap_invalid"
        );
    }

    #[test]
    fn valid_entrypoint_defaults_and_crlf_are_supported() {
        let (mut config, root) = test_config("entrypoint");
        write_entrypoint(&root, "hello\n");
        write_guide(
            &root,
            "diary.md",
            "id: diary\r\nkind: guide\r\ntitle: Diary\r\nsummary: Keep context",
            "body\n",
        );
        let response = load_config(&config).unwrap();
        assert_eq!(response.schema_version, 1);
        assert_eq!(response.entrypoint.id, "room");
        assert_eq!(
            response.guides[0].load_policy,
            BootstrapLoadPolicy::OnDemand
        );
        assert_eq!(response.guides[0].priority, 0);
        assert!(response
            .entrypoint
            .frontmatter
            .get("schemaVersion")
            .is_some());
        assert_eq!(response.guides[0].path, "guides/diary.md");
        config.workspace_root = root;
    }

    #[test]
    fn flat_discovery_ignores_hidden_non_markdown_and_nested_entries() {
        let (config, root) = test_config("discovery");
        write_entrypoint(&root, "");
        write_guide(
            &root,
            "valid.md",
            "id: valid\nkind: guide\ntitle: Valid\nsummary: Valid guide",
            "body",
        );
        let guides = root.join(BOOTSTRAP_DIR).join(GUIDES_DIR);
        fs::write(guides.join("ignored.txt"), "not a guide").unwrap();
        fs::write(guides.join(".hidden.md"), "not a guide").unwrap();
        fs::create_dir_all(guides.join("nested")).unwrap();
        fs::write(guides.join("nested").join("nested.md"), "not a guide").unwrap();
        let response = load_config(&config).unwrap();
        assert_eq!(response.total_guides, 1);
        assert_eq!(response.guides[0].id, "valid");
    }

    #[test]
    fn invalid_guides_are_excluded_and_duplicate_ids_are_order_independent() {
        let (config, root) = test_config("invalid-guides");
        write_entrypoint(&root, "");
        write_guide(
            &root,
            "a.md",
            "id: duplicate\nkind: guide\ntitle: A\nsummary: A",
            "a",
        );
        write_guide(
            &root,
            "b.md",
            "id: duplicate\nkind: guide\ntitle: B\nsummary: B",
            "b",
        );
        write_guide(
            &root,
            "invalid.md",
            "id: invalid id\nkind: guide\ntitle: Bad\nsummary: Bad",
            "bad",
        );
        fs::write(
            root.join(BOOTSTRAP_DIR).join(GUIDES_DIR).join("binary.md"),
            [0xff, 0xfe, 0xfd],
        )
        .unwrap();
        let response = load_config(&config).unwrap();
        assert_eq!(response.total_guides, 0);
        assert!(response
            .warnings
            .iter()
            .any(|warning| warning.starts_with("guide_duplicate_id:")));
        assert!(response
            .warnings
            .iter()
            .any(|warning| warning.starts_with("guide_non_utf8:")));
        assert!(response
            .warnings
            .iter()
            .any(|warning| warning.starts_with("guide_metadata_invalid:")));
    }

    #[test]
    fn guides_sort_by_priority_then_id_and_manifest_caps_at_64_but_read_keeps_all() {
        let (config, root) = test_config("ordering");
        write_entrypoint(&root, "");
        for index in 0..65 {
            let id = format!("guide{index:02}");
            write_guide(
                &root,
                &format!("{id}.md"),
                &format!(
                    "id: {id}\nkind: guide\ntitle: {id}\nsummary: Summary\npriority: {}",
                    if index == 64 { 100 } else { 0 }
                ),
                "body\n",
            );
        }
        let response = load_config(&config).unwrap();
        assert_eq!(response.total_guides, 65);
        assert_eq!(response.returned_guides, 64);
        assert_eq!(response.guides[0].id, "guide64");
        assert!(response
            .warnings
            .iter()
            .any(|warning| warning.starts_with("guides_truncated:")));
        let read = read_config(
            &config,
            &BootstrapReadRequest {
                id: "guide63".to_string(),
            },
        )
        .unwrap();
        assert_eq!(read.guide.id, "guide63");
    }

    #[test]
    fn invalid_and_valid_guides_have_different_revision_membership() {
        let (config, root) = test_config("revision");
        write_entrypoint(&root, "");
        write_guide(
            &root,
            "valid.md",
            "id: valid\nkind: guide\ntitle: Valid\nsummary: Valid",
            "one",
        );
        write_guide(
            &root,
            "invalid.md",
            "id: invalid id\nkind: guide\ntitle: Invalid\nsummary: Invalid",
            "one",
        );
        let first = load_config(&config).unwrap().revision;
        fs::write(
            root.join(BOOTSTRAP_DIR).join(GUIDES_DIR).join("invalid.md"),
            "not frontmatter",
        )
        .unwrap();
        assert_eq!(load_config(&config).unwrap().revision, first);
        fs::write(
            root.join(BOOTSTRAP_DIR).join(GUIDES_DIR).join("valid.md"),
            "---\nid: valid\nkind: guide\ntitle: Valid\nsummary: Valid\n---\ntwo",
        )
        .unwrap();
        assert_ne!(load_config(&config).unwrap().revision, first);
    }

    #[test]
    fn resource_truncation_is_line_aware_and_utf8_safe() {
        let bytes = "a\nβββ\nlast".as_bytes();
        let (resource, warning) =
            build_resource(bytes, "guides/demo.md", 5, "guide_truncated").unwrap();
        assert_eq!(resource.content, "a\n");
        assert_eq!(resource.total_lines, 3);
        assert_eq!(resource.returned_through_line, 1);
        assert_eq!(resource.omitted_from_line, Some(2));
        assert!(resource.last_line_complete);
        assert!(warning.unwrap().starts_with("guide_truncated:"));

        let (resource, _) =
            build_resource("ββββ".as_bytes(), "guides/demo.md", 5, "guide_truncated").unwrap();
        assert_eq!(resource.content, "ββ");
        assert!(!resource.last_line_complete);
        assert_eq!(resource.returned_through_line, 1);
        assert_eq!(resource.omitted_from_line, Some(1));
        assert_eq!(resource.returned_size_bytes, resource.content.len() as u64);
    }

    #[test]
    fn entrypoint_and_guide_symlinks_do_not_get_read_as_content() {
        let (config, root) = test_config("symlink");
        fs::create_dir_all(root.join(BOOTSTRAP_DIR).join(GUIDES_DIR)).unwrap();
        fs::write(root.join("real.md"), "content").unwrap();
        symlink(
            root.join("real.md"),
            root.join(BOOTSTRAP_DIR).join(ENTRYPOINT_NAME),
        )
        .unwrap();
        assert_eq!(
            load_config(&config).unwrap_err().to_string(),
            "bootstrap_invalid"
        );

        fs::remove_file(root.join(BOOTSTRAP_DIR).join(ENTRYPOINT_NAME)).unwrap();
        write_entrypoint(&root, "");
        symlink(
            root.join("real.md"),
            root.join(BOOTSTRAP_DIR).join(GUIDES_DIR).join("link.md"),
        )
        .unwrap();
        let response = load_config(&config).unwrap();
        assert_eq!(response.total_guides, 0);
        assert!(response
            .warnings
            .iter()
            .any(|warning| warning.starts_with("guide_symlink_ignored:")));
    }
}
