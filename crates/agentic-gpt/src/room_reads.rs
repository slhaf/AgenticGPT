use agentic_gpt_protocol::{
    RoomDiaryActiveRequest, RoomDiaryActiveResponse, RoomDiaryLayer, RoomDiaryLayerIssue,
    RoomDiaryLayerResult, RoomDiaryReadRequest, RoomDiaryReadResponse, RoomNotebookPreview,
    RoomNotebookReadRequest, RoomNotebookReadResponse, RoomNotebookRecentRequest,
    RoomNotebookResultsResponse, RoomNotebookSearchRequest, RoomStateEntity, RoomStateListRequest,
    RoomStateListResponse, RoomStateReadRequest, RoomStateReadResponse,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, Utc};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use crate::{room_repository, state::AppState};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const MAX_PREVIEW_CHARS: usize = 2_000;
const MAX_WARNINGS: usize = 32;

pub(crate) async fn diary_active(
    state: &AppState,
    _request: RoomDiaryActiveRequest,
) -> Result<RoomDiaryActiveResponse> {
    let root = read_root(state).await?;
    Ok(RoomDiaryActiveResponse {
        daily: read_diary_layer(
            root.as_deref(),
            RoomDiaryLayer::Daily,
            "current",
            "Diary/Daily/current.md",
        ),
        weekly: read_diary_layer(
            root.as_deref(),
            RoomDiaryLayer::Weekly,
            "current",
            "Diary/Weekly/current.md",
        ),
        monthly: read_diary_layer(
            root.as_deref(),
            RoomDiaryLayer::Monthly,
            "current",
            "Diary/Monthly/current.md",
        ),
    })
}

pub(crate) async fn diary_read(
    state: &AppState,
    request: RoomDiaryReadRequest,
) -> Result<RoomDiaryReadResponse> {
    let (period, relative) = diary_relative_path(request.layer, &request.period)?;
    let root = read_root(state).await?;
    let document = read_diary_layer(root.as_deref(), request.layer, &period, &relative);
    Ok(RoomDiaryReadResponse { document })
}

pub(crate) async fn notebook_recent(
    state: &AppState,
    request: RoomNotebookRecentRequest,
) -> Result<RoomNotebookResultsResponse> {
    let limit = normalize_limit(request.limit)?;
    let Some(root) = read_root(state).await? else {
        return Ok(RoomNotebookResultsResponse::default());
    };
    let (paths, mut warnings) = notebook_paths(&root)?;
    let mut documents = Vec::with_capacity(paths.len().min(limit));
    for relative in paths {
        match notebook_document(&root, &relative) {
            Ok((preview, _)) => documents.push(preview),
            Err(error) => push_warning(&mut warnings, format!("{relative}: {error}")),
        }
    }
    sort_recent_documents(&mut documents, limit);
    Ok(RoomNotebookResultsResponse {
        documents,
        warnings,
    })
}

pub(crate) async fn notebook_search(
    state: &AppState,
    request: RoomNotebookSearchRequest,
) -> Result<RoomNotebookResultsResponse> {
    let query = request.query.trim();
    if query.is_empty() {
        return Err(anyhow!("room_notebook_query_required"));
    }
    if query.chars().count() > 256 {
        return Err(anyhow!("room_notebook_query_too_long"));
    }
    let limit = normalize_limit(request.limit)?;
    let Some(root) = read_root(state).await? else {
        return Ok(RoomNotebookResultsResponse::default());
    };
    let (paths, mut warnings) = notebook_paths(&root)?;
    let query = query.to_lowercase();
    let mut documents = Vec::with_capacity(limit);
    for relative in paths {
        match notebook_document(&root, &relative) {
            Ok((preview, body))
                if relative.to_lowercase().contains(&query)
                    || preview.title.to_lowercase().contains(&query)
                    || body.to_lowercase().contains(&query) =>
            {
                documents.push(preview);
                if documents.len() == limit {
                    break;
                }
            }
            Ok(_) => {}
            Err(error) => push_warning(&mut warnings, format!("{relative}: {error}")),
        }
    }
    documents.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(RoomNotebookResultsResponse {
        documents,
        warnings,
    })
}

pub(crate) async fn notebook_read(
    state: &AppState,
    request: RoomNotebookReadRequest,
) -> Result<RoomNotebookReadResponse> {
    let relative = validate_notebook_path(&request.path)?;
    let Some(root) = read_root(state).await? else {
        return Err(anyhow!("room_notebook_not_found"));
    };
    let content = room_repository::read_bounded_markdown(&root, &relative)?;
    if content.truncated {
        return Err(anyhow!("room_markdown_too_large"));
    }
    Ok(RoomNotebookReadResponse {
        path: relative,
        content: content.content,
    })
}

pub(crate) async fn state_list(
    state: &AppState,
    _request: RoomStateListRequest,
) -> Result<RoomStateListResponse> {
    let Some(root) = read_root(state).await? else {
        return Ok(RoomStateListResponse::default());
    };
    let entities_root = match room_repository::repository_path(&root, "State/entities") {
        Ok(path) => path,
        Err(error) => return Err(error),
    };
    let metadata = match fs::symlink_metadata(&entities_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(anyhow!("room_repository_symlink_escape"));
        }
        Ok(metadata) if metadata.is_dir() => metadata,
        Ok(_) => return Err(anyhow!("room_state_entities_not_directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RoomStateListResponse::default());
        }
        Err(error) => return Err(error.into()),
    };
    let _ = metadata;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&entities_root)? {
        let entry = entry?;
        let path = entry.path();
        let file_metadata = fs::symlink_metadata(&path)?;
        if file_metadata.file_type().is_symlink() || !file_metadata.is_file() {
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        entries.push(RoomStateEntity {
            entity: path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string(),
            path: format!("State/entities/{name}"),
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(RoomStateListResponse { entities: entries })
}

pub(crate) async fn state_read(
    state: &AppState,
    request: RoomStateReadRequest,
) -> Result<RoomStateReadResponse> {
    let relative = state_relative_path(&request.entity)?;
    let Some(root) = read_root(state).await? else {
        return Err(anyhow!("room_state_entity_not_found"));
    };
    let content = room_repository::read_bounded_markdown(&root, &relative)?;
    if content.truncated {
        return Err(anyhow!("room_markdown_too_large"));
    }
    Ok(RoomStateReadResponse {
        path: relative,
        content: content.content,
    })
}

async fn read_root(state: &AppState) -> Result<Option<PathBuf>> {
    let config = state.config.read().await.clone();
    let status = room_repository::inspect_repository(&config)?;
    match fs::symlink_metadata(&status.root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(anyhow!("room_repository_symlink_escape"))
        }
        Ok(metadata) if metadata.is_dir() => Ok(Some(status.root)),
        Ok(_) => Err(anyhow!("room_repository_root_not_directory")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn diary_relative_path(layer: RoomDiaryLayer, period: &str) -> Result<(String, String)> {
    if period == "current" {
        return Ok((
            period.to_string(),
            format!("Diary/{}/current.md", diary_directory(layer)),
        ));
    }
    match layer {
        RoomDiaryLayer::Daily => {
            parse_date(period)?;
            Ok((period.to_string(), format!("Diary/Daily/{period}.md")))
        }
        RoomDiaryLayer::Weekly | RoomDiaryLayer::Monthly => {
            let (start, end) = parse_period_range(period)?;
            Ok((
                period.to_string(),
                format!("Diary/{}/{start}--{end}.md", diary_directory(layer)),
            ))
        }
    }
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || !value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return Err(anyhow!("room_diary_period_invalid"));
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| anyhow!("room_diary_period_invalid"))?;
    if date.format("%Y-%m-%d").to_string() != value {
        return Err(anyhow!("room_diary_period_invalid"));
    }
    Ok(date)
}

fn parse_period_range(value: &str) -> Result<(NaiveDate, NaiveDate)> {
    let Some((start, end)) = value.split_once("--") else {
        return Err(anyhow!("room_diary_period_invalid"));
    };
    if end.contains("--") {
        return Err(anyhow!("room_diary_period_invalid"));
    }
    let start = parse_date(start)?;
    let end = parse_date(end)?;
    if start > end {
        return Err(anyhow!("room_diary_period_invalid"));
    }
    Ok((start, end))
}

fn diary_directory(layer: RoomDiaryLayer) -> &'static str {
    match layer {
        RoomDiaryLayer::Daily => "Daily",
        RoomDiaryLayer::Weekly => "Weekly",
        RoomDiaryLayer::Monthly => "Monthly",
    }
}

fn read_diary_layer(
    root: Option<&Path>,
    layer: RoomDiaryLayer,
    period: &str,
    relative: &str,
) -> RoomDiaryLayerResult {
    let unavailable = |issue| RoomDiaryLayerResult {
        layer,
        period: period.to_string(),
        path: relative.to_string(),
        available: false,
        content: None,
        issue: Some(issue),
    };
    let Some(root) = root else {
        return unavailable(RoomDiaryLayerIssue::Missing);
    };
    let path = match room_repository::repository_path(root, relative) {
        Ok(path) => path,
        Err(_) => return unavailable(RoomDiaryLayerIssue::Unreadable),
    };
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            unavailable(RoomDiaryLayerIssue::Missing)
        }
        Err(_) => unavailable(RoomDiaryLayerIssue::Unreadable),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            unavailable(RoomDiaryLayerIssue::Unreadable)
        }
        Ok(_) => match room_repository::read_bounded_markdown(root, relative) {
            Ok(value) if !value.truncated => RoomDiaryLayerResult {
                layer,
                period: period.to_string(),
                path: relative.to_string(),
                available: true,
                content: Some(value.content),
                issue: None,
            },
            Ok(_) => unavailable(RoomDiaryLayerIssue::Unreadable),
            Err(error) if error.to_string().contains("room_file_not_utf8") => {
                unavailable(RoomDiaryLayerIssue::InvalidUtf8)
            }
            Err(_) => unavailable(RoomDiaryLayerIssue::Unreadable),
        },
    }
}

fn normalize_limit(limit: Option<usize>) -> Result<usize> {
    match limit {
        None => Ok(DEFAULT_LIMIT),
        Some(value) if (1..=MAX_LIMIT).contains(&value) => Ok(value),
        Some(_) => Err(anyhow!("room_read_limit_invalid")),
    }
}

fn notebook_paths(root: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let notebook_root = match room_repository::repository_path(root, "Notebook") {
        Ok(path) => path,
        Err(error) => return Err(error),
    };
    match fs::symlink_metadata(&notebook_root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Vec::new()))
        }
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(anyhow!("room_repository_symlink_escape"));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(anyhow!("room_notebook_root_not_directory"));
        }
        Ok(_) => {}
    }
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    walk_notebook(root, &notebook_root, "Notebook", &mut paths, &mut warnings)?;
    paths.sort();
    Ok((paths, warnings))
}

fn walk_notebook(
    root: &Path,
    directory: &Path,
    relative_directory: &str,
    paths: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            push_warning(warnings, "Notebook contains a non-UTF-8 path".to_string());
            continue;
        };
        let relative = format!("{relative_directory}/{name}");
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            push_warning(warnings, format!("{relative}: symlink skipped"));
            continue;
        }
        if metadata.is_dir() {
            walk_notebook(root, &path, &relative, paths, warnings)?;
        } else if metadata.is_file()
            && path.extension().and_then(|extension| extension.to_str()) == Some("md")
        {
            if room_repository::repository_path(root, &relative).is_ok() {
                paths.push(relative);
            } else {
                push_warning(warnings, format!("{relative}: unsafe path skipped"));
            }
        }
    }
    Ok(())
}

fn notebook_document(root: &Path, relative: &str) -> Result<(RoomNotebookPreview, String)> {
    let bounded = room_repository::read_bounded_markdown(root, relative)?;
    let path = room_repository::repository_path(root, relative)?;
    let title = markdown_title(&bounded.content, relative);
    let content_preview = bounded
        .content
        .chars()
        .take(MAX_PREVIEW_CHARS)
        .collect::<String>();
    let truncated =
        bounded.truncated || content_preview.chars().count() < bounded.content.chars().count();
    let preview = RoomNotebookPreview {
        path: relative.to_string(),
        title,
        content_preview,
        truncated,
        effective_at: effective_at(root, relative, &path),
    };
    Ok((preview, bounded.content))
}

fn sort_recent_documents(documents: &mut Vec<RoomNotebookPreview>, limit: usize) {
    documents.sort_by(|left, right| {
        right
            .effective_at
            .cmp(&left.effective_at)
            .then_with(|| left.path.cmp(&right.path))
    });
    documents.truncate(limit);
}

fn markdown_title(content: &str, relative: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_prefix("# ")
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(str::to_string)
        .or_else(|| {
            Path::new(relative)
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| relative.to_string())
}

fn effective_at(root: &Path, relative: &str, path: &Path) -> DateTime<Utc> {
    let working = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| DateTime::<Utc>::from(SystemTime::UNIX_EPOCH));
    let clean_tracked = Command::new("git")
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignored",
            "--",
            relative,
        ])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|status| status.is_empty());
    if !clean_tracked {
        return working;
    }
    Command::new("git")
        .args(["log", "-1", "--format=%ct", "--", relative])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<i64>().ok())
        .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
        .unwrap_or(working)
}

fn validate_notebook_path(value: &str) -> Result<String> {
    if !value.starts_with("Notebook/") || value.contains("//") {
        return Err(anyhow!("room_notebook_path_invalid"));
    }
    room_repository::validate_repository_relative(value)?;
    if Path::new(value)
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("md")
    {
        return Err(anyhow!("room_markdown_path_required"));
    }
    Ok(value.to_string())
}

fn state_relative_path(entity: &str) -> Result<String> {
    if entity.is_empty()
        || entity.contains(['/', '\\', '\0'])
        || entity == "."
        || entity == ".."
        || Path::new(entity).components().count() != 1
    {
        return Err(anyhow!("room_state_entity_invalid"));
    }
    room_repository::validate_repository_relative(&format!("State/entities/{entity}.md"))?;
    Ok(format!("State/entities/{entity}.md"))
}

fn push_warning(warnings: &mut Vec<String>, warning: String) {
    if warnings.len() < MAX_WARNINGS {
        warnings.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::state::{CapabilityProfile, RuntimeModel};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn test_state(workspace_root: PathBuf) -> AppState {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace_root;
        AppState {
            config_path: PathBuf::from("room-reads-test-config.json"),
            config: Arc::new(RwLock::new(config)),
            private_state: crate::private_state::PrivateStatePaths::for_test(
                std::env::temp_dir().join(format!(
                    "agentic-room-reads-private-{}",
                    uuid::Uuid::new_v4().simple()
                )),
            ),
            job_history: crate::job_history::JobHistoryStore::disabled(
                std::env::temp_dir().join("agentic-room-reads-test-jobs.sqlite3"),
            ),
            runtime: RuntimeModel::local(CapabilityProfile::Room),
            started_at: Utc::now(),
            boot_generation: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            supervised: false,
            file_locks: Arc::new(Mutex::new(HashMap::new())),
            jobs: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            reporting_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            mcp_concurrency: Arc::new(crate::jobs::McpConcurrency::new()),
            notebook_writes: Arc::new(Mutex::new(())),
            room_repository_writes: Arc::new(Mutex::new(())),
            skills_writes: Arc::new(Mutex::new(())),
            skill_leases: Arc::new(crate::jobs::SkillLeaseManager::new()),
            skill_installs: Arc::new(crate::skill_installs::InstallManager::new()),
        }
    }

    #[test]
    fn diary_periods_use_strict_direct_layer_paths() {
        assert_eq!(
            diary_relative_path(RoomDiaryLayer::Daily, "2026-09-11").unwrap(),
            (
                "2026-09-11".to_string(),
                "Diary/Daily/2026-09-11.md".to_string()
            )
        );
        assert_eq!(
            diary_relative_path(RoomDiaryLayer::Weekly, "2026-09-07--2026-09-13").unwrap(),
            (
                "2026-09-07--2026-09-13".to_string(),
                "Diary/Weekly/2026-09-07--2026-09-13.md".to_string()
            )
        );
        assert!(diary_relative_path(RoomDiaryLayer::Daily, "2026-02-30").is_err());
        assert!(diary_relative_path(RoomDiaryLayer::Monthly, "2026-10-01--2026-09-30").is_err());
        assert!(diary_relative_path(RoomDiaryLayer::Daily, "Diary/secret.md").is_err());
    }

    #[test]
    fn semantic_paths_reject_arbitrary_repository_files() {
        assert!(validate_notebook_path("Notebook/topic.md").is_ok());
        assert!(validate_notebook_path("State/entities/project.md").is_err());
        assert!(validate_notebook_path("Notebook/../secret.md").is_err());
        assert!(state_relative_path("project").is_ok());
        assert!(state_relative_path("project.v2").is_ok());
        assert!(state_relative_path("../project").is_err());
    }
    #[tokio::test]
    async fn state_list_entities_round_trip_for_dotted_stem() {
        let workspace_root = std::env::temp_dir().join(format!(
            "agentic-room-state-round-trip-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let entities_root = workspace_root.join("room/State/entities");
        fs::create_dir_all(&entities_root).unwrap();
        fs::write(entities_root.join("project.v2.md"), "# Project V2\n").unwrap();

        let state = test_state(workspace_root.clone());
        let listed = state_list(&state, RoomStateListRequest::default())
            .await
            .unwrap();
        assert_eq!(listed.entities.len(), 1);
        assert_eq!(listed.entities[0].entity, "project.v2");
        assert_eq!(listed.entities[0].path, "State/entities/project.v2.md");

        let read = state_read(
            &state,
            RoomStateReadRequest {
                entity: listed.entities[0].entity.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(read.path, listed.entities[0].path);
        assert_eq!(read.content, "# Project V2\n");

        let _ = fs::remove_dir_all(workspace_root);
    }

    #[test]
    fn notebook_recent_orders_by_recency_before_limit() {
        let root = std::env::temp_dir().join(format!(
            "agentic-room-notebook-recent-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(root.join("Notebook")).unwrap();
        fs::write(root.join("Notebook/alpha.md"), "# Older\n").unwrap();
        fs::write(root.join("Notebook/zeta.md"), "# Newer\n").unwrap();

        let (paths, warnings) = notebook_paths(&root).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            paths,
            vec![
                "Notebook/alpha.md".to_string(),
                "Notebook/zeta.md".to_string()
            ]
        );
        let mut documents = paths
            .into_iter()
            .map(|relative| notebook_document(&root, &relative).map(|(preview, _)| preview))
            .collect::<Result<Vec<_>>>()
            .unwrap();
        let older = DateTime::<Utc>::from_timestamp(1_000, 0).unwrap();
        let newer = DateTime::<Utc>::from_timestamp(2_000, 0).unwrap();
        documents[0].effective_at = older;
        documents[1].effective_at = newer;

        sort_recent_documents(&mut documents, 1);
        assert_eq!(documents[0].path, "Notebook/zeta.md");

        let mut tied = vec![
            notebook_document(&root, "Notebook/zeta.md").unwrap().0,
            notebook_document(&root, "Notebook/alpha.md").unwrap().0,
        ];
        for document in &mut tied {
            document.effective_at = newer;
        }
        sort_recent_documents(&mut tied, 2);
        assert_eq!(
            tied.iter()
                .map(|document| document.path.as_str())
                .collect::<Vec<_>>(),
            vec!["Notebook/alpha.md", "Notebook/zeta.md"]
        );

        let _ = fs::remove_dir_all(root);
    }
}
