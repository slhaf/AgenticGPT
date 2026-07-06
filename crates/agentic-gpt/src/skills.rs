use agentic_gpt_protocol::{
    ActiveSkill, SkillActivationRequest, SkillActivationResponse, SkillDetail, SkillPackageSummary,
    SkillReadRequest, SkillReadResponse, SkillSearchRequest, SkillSummary, SkillsActiveResponse,
    SkillsListResponse, SkillsSearchResponse,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use crate::{config::Config, state::AppState};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActiveSkillsFile {
    #[serde(default)]
    active_skills: Vec<ActiveSkillRecord>,
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
    Ok(SkillReadResponse {
        skill: package.detail(active_contains(&active, &request.id)),
    })
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
    if let Some(record) = active
        .active_skills
        .iter()
        .find(|record| record.id == request.id)
    {
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
    if changed {
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
        if validate_skill_id(id).is_err() {
            warnings.push(format!("skill_id_invalid: {id}"));
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
    let root = skills_root(config);
    let skill_dir = root.join(id);
    let skill_md_path = skill_dir.join("SKILL.md");
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
        return Ok(ActiveSkillsFile {
            active_skills: Vec::new(),
        });
    }
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
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

fn validate_skill_id(id: &str) -> Result<()> {
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

fn skills_root(config: &Config) -> PathBuf {
    config.workspace_root.join("skills")
}

fn active_file_path(config: &Config) -> PathBuf {
    config
        .workspace_root
        .join("state")
        .join("active-skills.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        state::{AppState, RunMode},
    };
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
            run_mode: RunMode::Room,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
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
            vec!["alpha", "beta"]
        );
        assert!(!response.skills[0].active);
        assert!(response.skills[1].active);
        assert_eq!(response.skills[1].name.as_deref(), Some("Beta"));
        assert_eq!(response.skills[1].tags, vec!["rust", "tools"]);
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
        assert!(active(&state).await.unwrap().active_skills.is_empty());
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
    async fn invalid_id_and_missing_skill_return_clear_errors() {
        let state = test_state();
        assert_eq!(
            read(
                &state,
                SkillReadRequest {
                    id: "../x".to_string()
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
