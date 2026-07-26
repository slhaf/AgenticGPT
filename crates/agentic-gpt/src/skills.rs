use agentic_gpt_protocol::{
    ActiveSkill, SkillActivationRequest, SkillActivationResponse, SkillDetail, SkillOrigin,
    SkillPackageSummary, SkillReadRequest, SkillReadResponse, SkillRunRequest, SkillSearchRequest,
    SkillSummary, SkillsActiveResponse, SkillsListResponse, SkillsSearchResponse,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

use crate::{config::Config, state::AppState};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const BUILTIN_INSTALLER_ID: &str = "skill-installer";
const MAX_RESOURCE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActiveSkillsFile {
    #[serde(default)]
    active_skills: Vec<ActiveSkillRecord>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    disabled_defaults: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActiveSkillRecord {
    id: String,
    activated_at: DateTime<Utc>,
}

#[derive(Clone)]
struct SkillPackage {
    id: String,
    origin: SkillOrigin,
    read_only: bool,
    package_root: Option<PathBuf>,
    skill_md: String,
    frontmatter: Value,
    name: Option<String>,
    description: Option<String>,
    version: Option<String>,
    tags: Vec<String>,
    package_summary: SkillPackageSummary,
    warnings: Vec<String>,
}

pub(crate) async fn list(state: &AppState) -> Result<SkillsListResponse> {
    let config = state.config.read().await.clone();
    let active = read_active_file(&config)?;
    let mut warnings = Vec::new();
    let mut skills = scan_skills(&config, &active, &mut warnings)?
        .into_values()
        .map(|package| package.summary(true, active_contains(&active, &package.id)))
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(SkillsListResponse { skills, warnings })
}

pub(crate) async fn read(state: &AppState, request: SkillReadRequest) -> Result<SkillReadResponse> {
    validate_skill_id(&request.id)?;
    let config = state.config.read().await.clone();
    let active = read_active_file(&config)?;
    let package = load_skill(&config, &request.id, active_contains(&active, &request.id))?;
    let resource = request
        .path
        .as_deref()
        .map(|path| read_resource(&package, path))
        .transpose()?;
    Ok(SkillReadResponse {
        skill: package.detail(active_contains(&active, &request.id)),
        resource,
    })
}

pub(crate) async fn resolve_run_program(
    state: &AppState,
    request: &SkillRunRequest,
) -> Result<PathBuf> {
    validate_skill_id(&request.id)?;
    let config = state.config.read().await.clone();
    let active = read_active_file(&config)?;
    if !active_contains(&active, &request.id) {
        return Err(anyhow!("skill_inactive"));
    }
    let package = load_skill(&config, &request.id, true)?;
    if package.origin != SkillOrigin::Workspace || package.read_only {
        return Err(anyhow!("skill_not_runnable"));
    }
    let root = package
        .package_root
        .as_ref()
        .ok_or_else(|| anyhow!("skill_not_runnable"))?;
    let relative =
        normalize_resource_path(&request.path).map_err(|_| anyhow!("invalid_script_path"))?;
    let mut components = relative.components();
    if components.next() != Some(Component::Normal(std::ffi::OsStr::new("scripts"))) {
        return Err(anyhow!("script_path_forbidden"));
    }
    reject_symlink(root, "script_symlink")?;
    let mut candidate = root.clone();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(anyhow!("invalid_script_path"));
        };
        candidate.push(part);
        reject_symlink(&candidate, "script_symlink")?;
    }
    let metadata = fs::metadata(&candidate).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow!("script_not_found")
        } else {
            anyhow!("script_read_failed")
        }
    })?;
    if !metadata.is_file() {
        return Err(anyhow!("script_not_runnable"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(anyhow!("script_not_executable"));
        }
    }
    let canonical_root = fs::canonicalize(root).map_err(|_| anyhow!("skill_not_runnable"))?;
    let canonical_candidate =
        fs::canonicalize(&candidate).map_err(|_| anyhow!("script_not_runnable"))?;
    if !canonical_candidate.starts_with(&canonical_root) {
        return Err(anyhow!("script_path_forbidden"));
    }
    Ok(canonical_candidate)
}

pub(crate) async fn search(
    state: &AppState,
    request: SkillSearchRequest,
) -> Result<SkillsSearchResponse> {
    let query = request.query.trim();
    if query.is_empty() {
        return Err(anyhow!("query_required"));
    }
    let query = query.to_lowercase();
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let config = state.config.read().await.clone();
    let active = read_active_file(&config)?;
    let mut warnings = Vec::new();
    let mut skills = scan_skills(&config, &active, &mut warnings)?
        .into_values()
        .filter(|package| package.matches_query(&query))
        .map(|package| package.summary(true, active_contains(&active, &package.id)))
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| left.id.cmp(&right.id));
    skills.truncate(limit);
    Ok(SkillsSearchResponse { skills, warnings })
}

pub(crate) async fn active(state: &AppState) -> Result<SkillsActiveResponse> {
    let config = state.config.read().await.clone();
    let active = read_active_file(&config)?;
    let mut warnings = Vec::new();
    let skills = scan_skills(&config, &active, &mut warnings)?;
    let active_skills = active
        .active_skills
        .iter()
        .map(|record| {
            let summary = skills
                .get(&record.id)
                .map(|package| package.summary(true, true));
            ActiveSkill {
                id: record.id.clone(),
                activated_at: record.activated_at,
                status: if summary.is_some() {
                    "active".to_string()
                } else {
                    "missing".to_string()
                },
                stale: summary.is_none(),
                summary,
            }
        })
        .collect();
    Ok(SkillsActiveResponse {
        active_skills,
        warnings,
    })
}

pub(crate) async fn activate(
    state: &AppState,
    request: SkillActivationRequest,
) -> Result<SkillActivationResponse> {
    validate_skill_id(&request.id)?;
    let config = state.config.read().await.clone();
    load_skill(&config, &request.id, false)?;
    let _guard = state.skills_writes.lock().await;
    let mut active = read_active_file(&config)?;
    let removed_disabled_default = active.disabled_defaults.iter().any(|id| id == &request.id);
    active.disabled_defaults.retain(|id| id != &request.id);
    if let Some(record) = active
        .active_skills
        .iter()
        .find(|record| record.id == request.id)
    {
        if removed_disabled_default {
            write_active_file(&config, &active)?;
        }
        return Ok(SkillActivationResponse {
            id: request.id,
            active: true,
            changed: false,
            activated_at: Some(record.activated_at),
        });
    }
    let activated_at = Utc::now();
    active.active_skills.push(ActiveSkillRecord {
        id: request.id.clone(),
        activated_at,
    });
    active
        .active_skills
        .sort_by(|left, right| left.id.cmp(&right.id));
    write_active_file(&config, &active)?;
    Ok(SkillActivationResponse {
        id: request.id,
        active: true,
        changed: true,
        activated_at: Some(activated_at),
    })
}

pub(crate) async fn deactivate(
    state: &AppState,
    request: SkillActivationRequest,
) -> Result<SkillActivationResponse> {
    validate_skill_id(&request.id)?;
    let config = state.config.read().await.clone();
    let _guard = state.skills_writes.lock().await;
    let mut active = read_active_file(&config)?;
    let before = active.active_skills.len();
    active
        .active_skills
        .retain(|record| record.id != request.id);
    let changed = active.active_skills.len() != before;
    let default_disabled = is_default_active_builtin(&request.id);
    let tombstone_added =
        default_disabled && !active.disabled_defaults.iter().any(|id| id == &request.id);
    if tombstone_added {
        active.disabled_defaults.push(request.id.clone());
        active.disabled_defaults.sort();
    }
    if changed || tombstone_added {
        write_active_file(&config, &active)?;
    }
    Ok(SkillActivationResponse {
        id: request.id,
        active: false,
        changed,
        activated_at: None,
    })
}

impl SkillPackage {
    fn summary(&self, _valid: bool, active: bool) -> SkillSummary {
        SkillSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            version: self.version.clone(),
            tags: self.tags.clone(),
            active,
            origin: self.origin,
            read_only: self.read_only,
            package_summary: self.package_summary.clone(),
            warnings: self.warnings.clone(),
        }
    }

    fn detail(self, active: bool) -> SkillDetail {
        SkillDetail {
            id: self.id,
            skill_md: self.skill_md,
            frontmatter: self.frontmatter,
            name: self.name,
            description: self.description,
            version: self.version,
            tags: self.tags,
            active,
            origin: self.origin,
            read_only: self.read_only,
            package_summary: self.package_summary,
            warnings: self.warnings,
        }
    }

    fn matches_query(&self, query: &str) -> bool {
        self.id.to_lowercase().contains(query)
            || self.skill_md.to_lowercase().contains(query)
            || self.frontmatter.to_string().to_lowercase().contains(query)
            || self
                .tags
                .iter()
                .any(|tag| tag.to_lowercase().contains(query))
    }
}

fn scan_skills(
    config: &Config,
    active: &ActiveSkillsFile,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<String, SkillPackage>> {
    let root = skills_root(config);
    let mut skills = BTreeMap::new();
    skills.insert(BUILTIN_INSTALLER_ID.to_string(), builtin_skill_package()?);
    if !root.exists() {
        return Ok(skills);
    }
    for entry in fs::read_dir(&root)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("skill_dir_entry_unreadable: {error}"));
                continue;
            }
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            warnings.push(format!("skill_dir_name_invalid: {}", path.display()));
            continue;
        };
        if id.starts_with('.') {
            continue;
        }
        if validate_skill_id(id).is_err() {
            warnings.push(format!("skill_id_invalid: {id}"));
            continue;
        }
        if id == BUILTIN_INSTALLER_ID {
            warnings.push("skill_id_reserved: skill-installer".to_string());
            continue;
        }
        match load_skill(config, id, active_contains(active, id)) {
            Ok(package) => {
                skills.insert(id.to_string(), package);
            }
            Err(error) if error.to_string() == "not_found" => {}
            Err(error) => warnings.push(format!("skill_unreadable: id={id}; error={error}")),
        }
    }
    Ok(skills)
}

fn load_skill(config: &Config, id: &str, _active: bool) -> Result<SkillPackage> {
    validate_skill_id(id)?;
    if id == BUILTIN_INSTALLER_ID {
        return builtin_skill_package();
    }
    let root = skills_root(config);
    let skill_dir = root.join(id);
    reject_symlink(&skill_dir, "skill_symlink")?;
    let skill_md_path = skill_dir.join("SKILL.md");
    reject_symlink(&skill_md_path, "skill_symlink")?;
    if !skill_md_path.is_file() {
        return Err(anyhow!("not_found"));
    }
    let skill_md = fs::read_to_string(&skill_md_path)?;
    let (frontmatter, mut warnings) = parse_frontmatter(&skill_md);
    let name = string_field(&frontmatter, "name");
    let description = string_field(&frontmatter, "description");
    let version = string_field(&frontmatter, "version");
    let tags = frontmatter
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if frontmatter.get("tags").is_some() && tags.is_empty() {
        warnings.push("frontmatter_tags_ignored: expected string array".to_string());
    }
    Ok(SkillPackage {
        id: id.to_string(),
        origin: SkillOrigin::Workspace,
        read_only: false,
        package_root: Some(skill_dir.clone()),
        skill_md,
        frontmatter,
        name,
        description,
        version,
        tags,
        package_summary: SkillPackageSummary {
            has_assets: skill_dir.join("assets").is_dir(),
            has_scripts: skill_dir.join("scripts").is_dir(),
            has_references: skill_dir.join("references").is_dir(),
        },
        warnings,
    })
}

fn builtin_skill_package() -> Result<SkillPackage> {
    let skill_md = include_str!("../skills/skill-installer/SKILL.md").to_string();
    let (frontmatter, warnings) = parse_frontmatter(&skill_md);
    Ok(SkillPackage {
        id: BUILTIN_INSTALLER_ID.to_string(),
        origin: SkillOrigin::Builtin,
        read_only: true,
        package_root: None,
        name: string_field(&frontmatter, "name"),
        description: string_field(&frontmatter, "description"),
        version: string_field(&frontmatter, "version"),
        tags: frontmatter
            .get("tags")
            .and_then(Value::as_array)
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        skill_md,
        frontmatter,
        package_summary: SkillPackageSummary::default(),
        warnings,
    })
}

fn read_resource(
    package: &SkillPackage,
    path: &str,
) -> Result<agentic_gpt_protocol::SkillResource> {
    let relative = normalize_resource_path(path)?;
    let bytes = if relative == Path::new("SKILL.md") {
        package.skill_md.as_bytes().to_vec()
    } else {
        let root = package
            .package_root
            .as_ref()
            .ok_or_else(|| anyhow!("resource_unavailable"))?;
        reject_symlink(root, "resource_symlink")?;
        let mut cursor = root.clone();
        for component in relative.components() {
            let Component::Normal(part) = component else {
                return Err(anyhow!("invalid_resource_path"));
            };
            cursor.push(part);
            reject_symlink(&cursor, "resource_symlink")?;
        }
        let metadata = fs::metadata(&cursor).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                anyhow!("resource_not_found")
            } else {
                anyhow!("resource_read_failed")
            }
        })?;
        if !metadata.is_file() {
            return Err(anyhow!("not_a_file"));
        }
        if metadata.len() > MAX_RESOURCE_BYTES {
            return Err(anyhow!("resource_too_large"));
        }
        fs::read(cursor).map_err(|_| anyhow!("resource_read_failed"))?
    };
    if bytes.len() as u64 > MAX_RESOURCE_BYTES {
        return Err(anyhow!("resource_too_large"));
    }
    let sha256 = hex_sha256(&bytes);
    let (encoding, content) = match String::from_utf8(bytes.clone()) {
        Ok(text) => (agentic_gpt_protocol::SkillResourceEncoding::Utf8, text),
        Err(_) => (
            agentic_gpt_protocol::SkillResourceEncoding::Base64,
            BASE64.encode(&bytes),
        ),
    };
    Ok(agentic_gpt_protocol::SkillResource {
        path: relative.to_string_lossy().replace('\\', "/"),
        encoding,
        content,
        media_type: None,
        size_bytes: bytes.len() as u64,
        sha256,
    })
}

fn normalize_resource_path(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.contains('\0') || path.contains('\\') {
        return Err(anyhow!("invalid_resource_path"));
    }
    let mut relative = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(anyhow!("invalid_resource_path"));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(anyhow!("invalid_resource_path"));
    }
    Ok(relative)
}

fn reject_symlink(path: &Path, error: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(anyhow!("{error}")),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(anyhow!("{error}")),
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_frontmatter(skill_md: &str) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();
    let normalized = skill_md.replace("\r\n", "\n");
    if !normalized.starts_with("---\n") {
        return (Value::Object(Default::default()), warnings);
    }
    let Some(end) = normalized[4..].find("\n---\n") else {
        warnings.push("frontmatter_unclosed".to_string());
        return (Value::Object(Default::default()), warnings);
    };
    let yaml = &normalized[4..4 + end];
    match serde_yaml::from_str::<serde_yaml::Value>(yaml) {
        Ok(value) => match serde_json::to_value(value) {
            Ok(value) if value.is_object() => (value, warnings),
            Ok(_) => {
                warnings.push("frontmatter_ignored: expected object".to_string());
                (Value::Object(Default::default()), warnings)
            }
            Err(error) => {
                warnings.push(format!("frontmatter_invalid: {error}"));
                (Value::Object(Default::default()), warnings)
            }
        },
        Err(error) => {
            warnings.push(format!("frontmatter_invalid: {error}"));
            (Value::Object(Default::default()), warnings)
        }
    }
}

fn string_field(frontmatter: &Value, key: &str) -> Option<String> {
    frontmatter
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn active_contains(active: &ActiveSkillsFile, id: &str) -> bool {
    active.active_skills.iter().any(|record| record.id == id)
}

fn read_active_file(config: &Config) -> Result<ActiveSkillsFile> {
    let path = active_file_path(config);
    if !path.exists() {
        let active = ActiveSkillsFile {
            active_skills: Vec::new(),
            disabled_defaults: Vec::new(),
        };
        return reconcile_default_activation(config, active);
    }
    let text = fs::read_to_string(path)?;
    reconcile_default_activation(config, serde_json::from_str(&text)?)
}

fn reconcile_default_activation(
    config: &Config,
    mut active: ActiveSkillsFile,
) -> Result<ActiveSkillsFile> {
    if is_default_active_builtin(BUILTIN_INSTALLER_ID)
        && !active
            .disabled_defaults
            .iter()
            .any(|id| id == BUILTIN_INSTALLER_ID)
        && !active_contains(&active, BUILTIN_INSTALLER_ID)
    {
        active.active_skills.push(ActiveSkillRecord {
            id: BUILTIN_INSTALLER_ID.to_string(),
            activated_at: Utc::now(),
        });
        active
            .active_skills
            .sort_by(|left, right| left.id.cmp(&right.id));
        write_active_file(config, &active)?;
    }
    Ok(active)
}

fn is_default_active_builtin(id: &str) -> bool {
    id == BUILTIN_INSTALLER_ID
}

fn write_active_file(config: &Config, active: &ActiveSkillsFile) -> Result<()> {
    let path = active_file_path(config);
    let state_dir = path
        .parent()
        .ok_or_else(|| anyhow!("active_state_parent_missing"))?;
    fs::create_dir_all(state_dir)?;
    let tmp = state_dir.join(format!(".active-skills-{}.tmp", Uuid::new_v4().simple()));
    fs::write(&tmp, serde_json::to_vec_pretty(active)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub(crate) fn validate_skill_id(id: &str) -> Result<()> {
    if id.is_empty() || id == "." || id == ".." {
        return Err(anyhow!("invalid_id"));
    }
    if id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-')
    {
        Ok(())
    } else {
        Err(anyhow!("invalid_id"))
    }
}

pub(crate) fn skills_root(config: &Config) -> PathBuf {
    config.workspace_root.join("skills")
}

fn active_file_path(config: &Config) -> PathBuf {
    config
        .workspace_root
        .join("state")
        .join("active-skills.json")
}

pub(crate) async fn is_active(state: &AppState, id: &str) -> Result<bool> {
    let config = state.config.read().await.clone();
    let active = read_active_file(&config)?;
    Ok(active_contains(&active, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, state::AppState};
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn test_state() -> AppState {
        let root = std::env::temp_dir().join(format!("agentic-skills-{}", Uuid::new_v4().simple()));
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root;
        AppState {
            config_path: PathBuf::from("test-config.json"),
            config: Arc::new(RwLock::new(config)),
            runtime: crate::state::RuntimeModel::hub(crate::state::CapabilityProfile::Room),
            started_at: chrono::Utc::now(),
            supervised: false,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(crate::sessions::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        }
    }

    async fn workspace_root(state: &AppState) -> PathBuf {
        state.config.read().await.workspace_root.clone()
    }

    fn write_skill(root: &Path, id: &str, body: &str) {
        let dir = root.join("skills").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }

    #[tokio::test]
    async fn list_reads_only_valid_first_level_skills_sorted_with_active() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(
            &root,
            "beta",
            "---\nname: Beta\ntags: [rust, tools]\n---\nBody",
        );
        write_skill(&root, "alpha", "# Alpha");
        fs::create_dir_all(root.join("skills").join("missing-md")).unwrap();

        activate(
            &state,
            SkillActivationRequest {
                id: "beta".to_string(),
            },
        )
        .await
        .unwrap();

        let response = list(&state).await.unwrap();
        assert_eq!(
            response
                .skills
                .iter()
                .map(|skill| skill.id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta", BUILTIN_INSTALLER_ID]
        );
        assert!(!response.skills[0].active);
        assert!(response.skills[1].active);
        assert_eq!(response.skills[1].name.as_deref(), Some("Beta"));
        assert_eq!(response.skills[1].tags, vec!["rust", "tools"]);
        assert!(response.skills[2].active);
        assert_eq!(response.skills[2].origin, SkillOrigin::Builtin);
        assert!(response.skills[2].read_only);
    }

    #[tokio::test]
    async fn read_returns_frontmatter_package_summary_and_warnings() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(&root, "demo", "---\nname: Demo\n: bad\n---\nBody");
        fs::create_dir_all(root.join("skills/demo/assets")).unwrap();

        let response = read(
            &state,
            SkillReadRequest {
                id: "demo".to_string(),
                path: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(response.skill.id, "demo");
        assert!(response.skill.skill_md.contains("Body"));
        assert!(response.skill.package_summary.has_assets);
        assert!(response
            .skill
            .warnings
            .iter()
            .any(|warning| warning.starts_with("frontmatter_invalid")));
    }

    #[tokio::test]
    async fn search_matches_id_frontmatter_tags_and_body_case_insensitively_with_limit() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(&root, "alpha", "---\ndescription: Needle\n---\nBody");
        write_skill(&root, "beta", "---\ntags: [Needle]\n---\nBody");
        write_skill(&root, "needle-id", "# Title");

        let response = search(
            &state,
            SkillSearchRequest {
                query: "needle".to_string(),
                limit: Some(2),
            },
        )
        .await
        .unwrap();

        assert_eq!(response.skills.len(), 2);
        assert_eq!(response.skills[0].id, "alpha");
        assert_eq!(
            search(
                &state,
                SkillSearchRequest {
                    query: " ".to_string(),
                    limit: None,
                },
            )
            .await
            .unwrap_err()
            .to_string(),
            "query_required"
        );
    }

    #[tokio::test]
    async fn active_marks_deleted_skill_stale_without_summary_and_deactivate_cleans_it() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(&root, "gone", "# Gone");
        activate(
            &state,
            SkillActivationRequest {
                id: "gone".to_string(),
            },
        )
        .await
        .unwrap();
        fs::remove_file(root.join("skills/gone/SKILL.md")).unwrap();

        let response = active(&state).await.unwrap();
        assert_eq!(response.active_skills[0].status, "missing");
        assert!(response.active_skills[0].stale);
        assert!(response.active_skills[0].summary.is_none());

        let response = deactivate(
            &state,
            SkillActivationRequest {
                id: "gone".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(response.changed);
        let active = active(&state).await.unwrap();
        assert_eq!(active.active_skills.len(), 1);
        assert_eq!(active.active_skills[0].id, BUILTIN_INSTALLER_ID);
    }

    #[tokio::test]
    async fn activation_is_idempotent_and_state_file_only_saves_id_and_time() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(&root, "demo", "# Demo");

        let first = activate(
            &state,
            SkillActivationRequest {
                id: "demo".to_string(),
            },
        )
        .await
        .unwrap();
        let second = activate(
            &state,
            SkillActivationRequest {
                id: "demo".to_string(),
            },
        )
        .await
        .unwrap();

        assert!(first.changed);
        assert!(!second.changed);
        assert_eq!(first.activated_at, second.activated_at);
        let text = fs::read_to_string(root.join("state/active-skills.json")).unwrap();
        assert!(text.contains("activeSkills"));
        assert!(text.contains("activatedAt"));
        assert!(!text.contains("summary"));
    }

    #[tokio::test]
    async fn builtin_installer_is_default_active_and_deactivation_survives_restart() {
        let state = test_state();
        let root = workspace_root(&state).await;

        let first = list(&state).await.unwrap();
        let installer = first
            .skills
            .iter()
            .find(|skill| skill.id == BUILTIN_INSTALLER_ID)
            .unwrap();
        assert!(installer.active);
        assert_eq!(installer.origin, SkillOrigin::Builtin);
        assert!(installer.read_only);
        let detail = read(
            &state,
            SkillReadRequest {
                id: BUILTIN_INSTALLER_ID.to_string(),
                path: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(detail.skill.origin, SkillOrigin::Builtin);
        assert!(detail.skill.skill_md.contains("skills.install"));
        assert_eq!(
            search(
                &state,
                SkillSearchRequest {
                    query: "installer".to_string(),
                    limit: None,
                },
            )
            .await
            .unwrap()
            .skills
            .iter()
            .filter(|skill| skill.id == BUILTIN_INSTALLER_ID)
            .count(),
            1
        );

        write_skill(&root, BUILTIN_INSTALLER_ID, "# shadow");
        let shadowed = read(
            &state,
            SkillReadRequest {
                id: BUILTIN_INSTALLER_ID.to_string(),
                path: None,
            },
        )
        .await
        .unwrap();
        assert!(shadowed.skill.skill_md.contains("skills.install"));

        deactivate(
            &state,
            SkillActivationRequest {
                id: BUILTIN_INSTALLER_ID.to_string(),
            },
        )
        .await
        .unwrap();
        let text = fs::read_to_string(root.join("state/active-skills.json")).unwrap();
        assert!(text.contains("disabledDefaults"));
        assert!(text.contains(BUILTIN_INSTALLER_ID));
        assert!(
            !list(&state)
                .await
                .unwrap()
                .skills
                .iter()
                .find(|skill| skill.id == BUILTIN_INSTALLER_ID)
                .unwrap()
                .active
        );

        activate(
            &state,
            SkillActivationRequest {
                id: BUILTIN_INSTALLER_ID.to_string(),
            },
        )
        .await
        .unwrap();
        let text = fs::read_to_string(root.join("state/active-skills.json")).unwrap();
        assert!(!text.contains("disabledDefaults"));
    }

    #[tokio::test]
    async fn read_supports_bounded_utf8_and_base64_package_resources() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(&root, "demo", "# Demo");
        let package_root = root.join("skills/demo");
        fs::write(package_root.join("notes.txt"), "hello").unwrap();
        fs::write(package_root.join("data.bin"), [0_u8, 255_u8, 1_u8]).unwrap();

        let text = read(
            &state,
            SkillReadRequest {
                id: "demo".to_string(),
                path: Some("notes.txt".to_string()),
            },
        )
        .await
        .unwrap();
        let resource = text.resource.unwrap();
        assert_eq!(
            resource.encoding,
            agentic_gpt_protocol::SkillResourceEncoding::Utf8
        );
        assert_eq!(resource.content, "hello");
        assert_eq!(resource.size_bytes, 5);

        let binary = read(
            &state,
            SkillReadRequest {
                id: "demo".to_string(),
                path: Some("data.bin".to_string()),
            },
        )
        .await
        .unwrap();
        let resource = binary.resource.unwrap();
        assert_eq!(
            resource.encoding,
            agentic_gpt_protocol::SkillResourceEncoding::Base64
        );
        assert_eq!(resource.content, "AP8B");
        assert_eq!(resource.size_bytes, 3);
    }

    #[tokio::test]
    async fn read_rejects_resource_escape_directories_and_symlinks() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(&root, "demo", "# Demo");
        let package_root = root.join("skills/demo");
        fs::create_dir(package_root.join("docs")).unwrap();
        fs::write(package_root.join("docs/info.txt"), "info").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("state"), package_root.join("link")).unwrap();

        for path in ["../state/secret", "docs", "link/secret"] {
            let error = read(
                &state,
                SkillReadRequest {
                    id: "demo".to_string(),
                    path: Some(path.to_string()),
                },
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(
                ["invalid_resource_path", "not_a_file", "resource_symlink"]
                    .contains(&error.as_str()),
                "unexpected error for {path}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn run_resolution_requires_active_workspace_executable_under_scripts() {
        let state = test_state();
        let root = workspace_root(&state).await;
        write_skill(&root, "demo", "# Demo");
        let scripts = root.join("skills/demo/scripts");
        fs::create_dir_all(&scripts).unwrap();
        let script = scripts.join("check.sh");
        fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let request = SkillRunRequest {
            id: "demo".to_string(),
            path: "scripts/check.sh".to_string(),
            args: None,
            working_directory: None,
            wait_seconds: None,
        };
        assert_eq!(
            resolve_run_program(&state, &request)
                .await
                .unwrap_err()
                .to_string(),
            "skill_inactive"
        );
        activate(
            &state,
            SkillActivationRequest {
                id: "demo".to_string(),
            },
        )
        .await
        .unwrap();
        let resolved = resolve_run_program(&state, &request).await.unwrap();
        assert_eq!(
            resolved.file_name().and_then(|name| name.to_str()),
            Some("check.sh")
        );
        assert!(resolve_run_program(
            &state,
            &SkillRunRequest {
                id: "demo".to_string(),
                path: "SKILL.md".to_string(),
                args: None,
                working_directory: None,
                wait_seconds: None,
            },
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn invalid_id_and_missing_skill_return_clear_errors() {
        let state = test_state();
        assert_eq!(
            read(
                &state,
                SkillReadRequest {
                    id: "../x".to_string(),
                    path: None,
                },
            )
            .await
            .unwrap_err()
            .to_string(),
            "invalid_id"
        );
        assert_eq!(
            activate(
                &state,
                SkillActivationRequest {
                    id: "missing".to_string()
                },
            )
            .await
            .unwrap_err()
            .to_string(),
            "not_found"
        );
    }
}
