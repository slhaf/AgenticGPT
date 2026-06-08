use agentic_gpt_protocol::{
    NotebookAppendRequest, NotebookAppendResponse, NotebookCurrent, NotebookCurrentRequest,
    NotebookCurrentResponse, NotebookPassagesResponse, NotebookRecentRequest,
    NotebookSearchRequest, NotebookSelectExactRequest, Passage, PassagePreview,
    PassageSignificance,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::{ensure_parent, AppState, Config};

const DEFAULT_RECENT_DAYS: u32 = 5;
const MAX_RECENT_DAYS: u32 = 30;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const CONTENT_PREVIEW_CHARS: usize = 2000;
const ABSTRACT_MAX_CHARS: usize = 240;

pub(crate) async fn append(
    state: &AppState,
    request: NotebookAppendRequest,
) -> Result<NotebookAppendResponse> {
    validate_scope(&request.scope)?;
    validate_text("abstract", &request.abstract_text, ABSTRACT_MAX_CHARS)?;
    validate_text("content", &request.content, usize::MAX)?;
    let config = state.config.read().await.clone();
    let timezone = room_timezone(&config)?;
    let root = notebook_root(&config);
    let now = Utc::now();
    let passage = Passage {
        id: format!("psg_{}", Uuid::new_v4().simple()),
        datetime: request.datetime.unwrap_or(now),
        scope: request.scope,
        significance: request.significance,
        abstract_text: request.abstract_text,
        content: request.content,
        tags: request.tags,
    };
    let path = passage_path(&root, &timezone, passage.datetime);
    let _guard = state.notebook_writes.lock().await;
    ensure_notebook_layout(&root)?;
    ensure_parent(&path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{}", serde_json::to_string(&passage)?)?;
    if matches!(passage.significance, PassageSignificance::Anchor) {
        write_current(&root, &passage)?;
    }
    Ok(NotebookAppendResponse {
        id: passage.id,
        path: path.display().to_string(),
        created: true,
        warnings: Vec::new(),
    })
}

pub(crate) async fn recent(
    state: &AppState,
    request: NotebookRecentRequest,
) -> Result<NotebookPassagesResponse> {
    if let Some(scope) = request.scope.as_deref() {
        validate_scope(scope)?;
    }
    let config = state.config.read().await.clone();
    let timezone = room_timezone(&config)?;
    let root = notebook_root(&config);
    let days = request
        .days
        .unwrap_or(DEFAULT_RECENT_DAYS)
        .clamp(1, MAX_RECENT_DAYS);
    let limit = normalized_limit(request.limit);
    let today = Utc::now().with_timezone(&timezone).date_naive();
    let start = today - Duration::days(days.saturating_sub(1) as i64);
    let dates = date_range(start, today);
    let (mut passages, warnings) = read_passages_for_dates(&root, &dates)?;
    filter_passages(
        &mut passages,
        request.scope.as_deref(),
        request.significance.as_ref(),
    );
    passages.sort_by_key(|passage| passage.datetime);
    let total = passages.len();
    if total > limit {
        passages = passages.into_iter().skip(total - limit).collect();
    }
    Ok(NotebookPassagesResponse {
        passages: passages
            .iter()
            .map(|passage| preview(passage, display_mode_for_recent(passage, &timezone)))
            .collect(),
        warnings,
    })
}

pub(crate) async fn select_exact(
    state: &AppState,
    request: NotebookSelectExactRequest,
) -> Result<NotebookPassagesResponse> {
    if let Some(scope) = request.scope.as_deref() {
        validate_scope(scope)?;
    }
    let date = NaiveDate::from_ymd_opt(request.year, request.month, request.day)
        .ok_or_else(|| anyhow!("invalid_date"))?;
    let config = state.config.read().await.clone();
    let root = notebook_root(&config);
    let (mut passages, warnings) = read_passages_for_dates(&root, &[date])?;
    filter_passages(&mut passages, request.scope.as_deref(), None);
    passages.sort_by_key(|passage| passage.datetime);
    passages.truncate(normalized_limit(request.limit));
    Ok(NotebookPassagesResponse {
        passages: passages
            .iter()
            .map(|passage| preview(passage, "full"))
            .collect(),
        warnings,
    })
}

pub(crate) async fn search(
    state: &AppState,
    request: NotebookSearchRequest,
) -> Result<NotebookPassagesResponse> {
    if request.query.trim().is_empty() {
        return Err(anyhow!("query_required"));
    }
    if let Some(scope) = request.scope.as_deref() {
        validate_scope(scope)?;
    }
    let config = state.config.read().await.clone();
    let root = notebook_root(&config);
    let query = request.query.to_lowercase();
    let limit = normalized_limit(request.limit);
    let (mut passages, warnings) = read_all_passages(&root)?;
    filter_passages(&mut passages, request.scope.as_deref(), None);
    passages.retain(|passage| passage_matches_query(passage, &query));
    passages.sort_by_key(|passage| std::cmp::Reverse(passage.datetime));
    passages.truncate(limit);
    Ok(NotebookPassagesResponse {
        passages: passages
            .iter()
            .map(|passage| preview(passage, "search"))
            .collect(),
        warnings,
    })
}

pub(crate) async fn current(
    state: &AppState,
    request: NotebookCurrentRequest,
) -> Result<NotebookCurrentResponse> {
    validate_scope(&request.scope)?;
    let config = state.config.read().await.clone();
    let root = notebook_root(&config);
    let mut warnings = Vec::new();
    match read_current(&root, &request.scope) {
        Ok(Some(current)) => Ok(NotebookCurrentResponse {
            current: Some(current),
            warnings,
        }),
        Ok(None) => current_from_latest_anchor(&root, &request.scope, warnings),
        Err(error) => {
            warnings.push(format!(
                "current_file_invalid: scope={}; error={error}",
                request.scope
            ));
            current_from_latest_anchor(&root, &request.scope, warnings)
        }
    }
}

pub(crate) fn notebook_root(config: &Config) -> PathBuf {
    config
        .room
        .notebook_root
        .clone()
        .unwrap_or_else(|| config.workspace_root.join("notebook"))
}

fn room_timezone(config: &Config) -> Result<Tz> {
    config
        .room
        .timezone
        .parse::<Tz>()
        .map_err(|_| anyhow!("invalid_room_timezone: {}", config.room.timezone))
}

fn passage_path(root: &Path, timezone: &Tz, datetime: DateTime<Utc>) -> PathBuf {
    let local = datetime.with_timezone(timezone);
    root.join("passages")
        .join(format!("{:04}", local.year()))
        .join(format!("{:02}", local.month()))
        .join(format!("{:02}.jsonl", local.day()))
}

fn date_path(root: &Path, date: NaiveDate) -> PathBuf {
    root.join("passages")
        .join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}.jsonl", date.day()))
}

fn ensure_notebook_layout(root: &Path) -> Result<()> {
    fs::create_dir_all(root.join("passages"))?;
    fs::create_dir_all(root.join("current"))?;
    let readme = root.join("README.md");
    if !readme.exists() {
        fs::write(
            readme,
            "# Room Notebook\n\nGeneric file-backed room notebook storage.\n\nPassages are stored as JSONL under `passages/YYYY/MM/DD.jsonl`, partitioned by the configured room timezone. Passage datetimes are stored as UTC ISO-8601 values. Current state files live under `current/<scope>.json`; scopes are restricted to `[A-Za-z0-9_.-]` and must not contain `..`.\n\nThis notebook is for explicit handoff passages and recoverable project state, not automatic chat logs, reminders, vector search, or task management.\n",
        )?;
    }
    Ok(())
}

fn validate_scope(scope: &str) -> Result<()> {
    if scope.is_empty()
        || scope.contains("..")
        || !scope
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err(anyhow!("invalid_scope"));
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, max_chars: usize) -> Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!("{name}_required"));
    }
    if value.chars().count() > max_chars {
        return Err(anyhow!("{name}_too_long"));
    }
    Ok(())
}

fn normalized_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn date_range(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    let mut date = start;
    while date <= end {
        dates.push(date);
        date += Duration::days(1);
    }
    dates
}

fn read_passages_for_dates(
    root: &Path,
    dates: &[NaiveDate],
) -> Result<(Vec<Passage>, Vec<String>)> {
    let mut passages = Vec::new();
    let mut warnings = Vec::new();
    for date in dates {
        read_passage_file(&date_path(root, *date), &mut passages, &mut warnings)?;
    }
    Ok((passages, warnings))
}

fn read_all_passages(root: &Path) -> Result<(Vec<Passage>, Vec<String>)> {
    let mut passages = Vec::new();
    let mut warnings = Vec::new();
    let base = root.join("passages");
    if !base.exists() {
        return Ok((passages, warnings));
    }
    for year in fs::read_dir(base)? {
        let year = year?;
        if !year.file_type()?.is_dir() {
            continue;
        }
        for month in fs::read_dir(year.path())? {
            let month = month?;
            if !month.file_type()?.is_dir() {
                continue;
            }
            for day in fs::read_dir(month.path())? {
                let day = day?;
                if day.path().extension().and_then(|value| value.to_str()) == Some("jsonl") {
                    read_passage_file(&day.path(), &mut passages, &mut warnings)?;
                }
            }
        }
    }
    Ok((passages, warnings))
}

fn read_passage_file(
    path: &Path,
    passages: &mut Vec<Passage>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let file = fs::File::open(path)?;
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Passage>(&line) {
            Ok(passage) => passages.push(passage),
            Err(error) => warnings.push(format!(
                "invalid_jsonl: path={}; line={}; error={error}",
                path.display(),
                index + 1
            )),
        }
    }
    Ok(())
}

fn filter_passages(
    passages: &mut Vec<Passage>,
    scope: Option<&str>,
    significance: Option<&PassageSignificance>,
) {
    passages.retain(|passage| {
        scope.map(|scope| passage.scope == scope).unwrap_or(true)
            && significance
                .map(|significance| same_significance(&passage.significance, significance))
                .unwrap_or(true)
    });
}

fn same_significance(left: &PassageSignificance, right: &PassageSignificance) -> bool {
    matches!(
        (left, right),
        (PassageSignificance::Normal, PassageSignificance::Normal)
            | (PassageSignificance::Anchor, PassageSignificance::Anchor)
    )
}

fn display_mode_for_recent(passage: &Passage, timezone: &Tz) -> &'static str {
    let today = Utc::now().with_timezone(timezone).date_naive();
    let passage_day = passage.datetime.with_timezone(timezone).date_naive();
    if today.signed_duration_since(passage_day).num_days() <= 2 {
        "full"
    } else if matches!(passage.significance, PassageSignificance::Anchor) {
        "anchor"
    } else {
        "abstract"
    }
}

fn preview(passage: &Passage, display_mode: &str) -> PassagePreview {
    let max = match display_mode {
        "abstract" => 0,
        "anchor" => CONTENT_PREVIEW_CHARS,
        _ => CONTENT_PREVIEW_CHARS,
    };
    let (content_preview, truncated) = truncate_preview(&passage.content, max);
    PassagePreview {
        id: passage.id.clone(),
        datetime: passage.datetime,
        scope: passage.scope.clone(),
        significance: passage.significance.clone(),
        abstract_text: passage.abstract_text.clone(),
        content_preview,
        tags: passage.tags.clone(),
        display_mode: display_mode.to_string(),
        truncated,
    }
}

fn truncate_preview(value: &str, max_chars: usize) -> (String, bool) {
    if max_chars == 0 {
        return (String::new(), !value.is_empty());
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    let truncated = value.chars().count() > max_chars;
    if truncated {
        output.push_str("...");
    }
    (output, truncated)
}

fn passage_matches_query(passage: &Passage, query: &str) -> bool {
    passage.scope.to_lowercase().contains(query)
        || passage.abstract_text.to_lowercase().contains(query)
        || passage.content.to_lowercase().contains(query)
        || passage
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
}

fn current_path(root: &Path, scope: &str) -> PathBuf {
    root.join("current").join(format!("{scope}.json"))
}

fn current_from_passage(passage: &Passage, updated_at: DateTime<Utc>) -> NotebookCurrent {
    NotebookCurrent {
        scope: passage.scope.clone(),
        updated_at,
        source_passage_id: passage.id.clone(),
        abstract_text: passage.abstract_text.clone(),
        content: passage.content.clone(),
        tags: passage.tags.clone(),
    }
}

fn write_current(root: &Path, passage: &Passage) -> Result<()> {
    validate_scope(&passage.scope)?;
    let path = current_path(root, &passage.scope);
    ensure_parent(&path)?;
    fs::write(
        path,
        serde_json::to_string_pretty(&current_from_passage(passage, Utc::now()))?,
    )?;
    Ok(())
}

fn read_current(root: &Path, scope: &str) -> Result<Option<NotebookCurrent>> {
    let path = current_path(root, scope);
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str::<NotebookCurrent>(&text)?))
}

fn current_from_latest_anchor(
    root: &Path,
    scope: &str,
    warnings: Vec<String>,
) -> Result<NotebookCurrentResponse> {
    let (mut passages, mut read_warnings) = read_all_passages(root)?;
    let mut warnings = warnings;
    warnings.append(&mut read_warnings);
    passages.retain(|passage| {
        passage.scope == scope && matches!(passage.significance, PassageSignificance::Anchor)
    });
    passages.sort_by_key(|passage| std::cmp::Reverse(passage.datetime));
    Ok(NotebookCurrentResponse {
        current: passages
            .first()
            .map(|passage| current_from_passage(passage, passage.datetime)),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_gpt_protocol::PassageSignificance;
    use chrono::TimeZone;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentic-gpt-notebook-test-{name}-{}",
            Uuid::new_v4().simple()
        ))
    }

    fn test_state(workspace_root: PathBuf) -> AppState {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace_root;
        AppState {
            config_path: PathBuf::from("test-config.json"),
            config: Arc::new(RwLock::new(config)),
            run_mode: crate::RunMode::Room,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            hub_sender: Arc::new(Mutex::new(None)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            temporary_mcp_allows: Arc::new(Mutex::new(Vec::new())),
            notebook_writes: Arc::new(Mutex::new(())),
        }
    }

    #[test]
    fn validates_scope_path_safely() {
        assert!(validate_scope("agentic.main-1").is_ok());
        assert!(validate_scope("").is_err());
        assert!(validate_scope("../x").is_err());
        assert!(validate_scope("a/b").is_err());
        assert!(validate_scope("a..b").is_err());
    }

    #[test]
    fn partitions_by_room_timezone_date() {
        let root = PathBuf::from("/tmp/notebook-test");
        let tz: Tz = "Asia/Shanghai".parse().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 6, 7, 18, 0, 0).unwrap();
        assert!(passage_path(&root, &tz, at).ends_with("passages/2026/06/08.jsonl"));
    }

    #[test]
    fn preview_tracks_truncation() {
        let passage = Passage {
            id: "psg_1".to_string(),
            datetime: Utc::now(),
            scope: "x".to_string(),
            significance: PassageSignificance::Normal,
            abstract_text: "a".to_string(),
            content: "abcdef".to_string(),
            tags: vec![],
        };
        let preview = preview(&passage, "abstract");
        assert!(preview.truncated);
        assert!(preview.content_preview.is_empty());
    }

    #[test]
    fn current_schema_round_trips_with_abstract_rename() {
        let current = NotebookCurrent {
            scope: "agentic".to_string(),
            updated_at: Utc::now(),
            source_passage_id: "psg_1".to_string(),
            abstract_text: "handoff".to_string(),
            content: "details".to_string(),
            tags: vec!["tag".to_string()],
        };
        let value = serde_json::to_value(&current).unwrap();
        assert_eq!(value["abstract"], json!("handoff"));
        assert!(value.get("abstractText").is_none());
    }

    #[tokio::test]
    async fn anchor_append_updates_current() {
        let workspace = unique_temp_dir("anchor-current").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let state = test_state(workspace.clone());
        let response = append(
            &state,
            NotebookAppendRequest {
                datetime: None,
                scope: "agentic".to_string(),
                significance: PassageSignificance::Anchor,
                abstract_text: "handoff".to_string(),
                content: "recoverable details".to_string(),
                tags: vec!["handoff".to_string()],
            },
        )
        .await
        .unwrap();
        let current = read_current(&workspace.join("notebook"), "agentic")
            .unwrap()
            .unwrap();
        assert_eq!(current.source_passage_id, response.id);
        assert_eq!(current.abstract_text, "handoff");
    }

    #[test]
    fn damaged_current_falls_back_to_latest_anchor() {
        let root = unique_temp_dir("damaged-current");
        let timezone: Tz = "Asia/Shanghai".parse().unwrap();
        let passage = Passage {
            id: "psg_anchor".to_string(),
            datetime: Utc.with_ymd_and_hms(2026, 6, 8, 1, 0, 0).unwrap(),
            scope: "agentic".to_string(),
            significance: PassageSignificance::Anchor,
            abstract_text: "fallback".to_string(),
            content: "fallback details".to_string(),
            tags: vec!["anchor".to_string()],
        };
        let path = passage_path(&root, &timezone, passage.datetime);
        ensure_parent(&path).unwrap();
        fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&passage).unwrap()),
        )
        .unwrap();
        let current_path = current_path(&root, "agentic");
        ensure_parent(&current_path).unwrap();
        fs::write(current_path, "{not json").unwrap();
        let response = current_from_latest_anchor(
            &root,
            "agentic",
            vec!["current_file_invalid: scope=agentic; error=test".to_string()],
        )
        .unwrap();
        assert!(response.warnings[0].starts_with("current_file_invalid"));
        assert_eq!(response.current.unwrap().source_passage_id, "psg_anchor");
    }
}
