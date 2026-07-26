use agentic_gpt_protocol::{
    DiaryAppendRequest, DiaryAppendResponse, DiaryEntriesResponse, DiaryEntry, DiaryRecentRequest,
    DiarySelectExactRequest,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::{config::Config, state::AppState, utils::ensure_parent};

const DEFAULT_RECENT_DAYS: u32 = 3;
const MAX_RECENT_DAYS: u32 = 30;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const DEFAULT_TIME_HINT: &str = "unknown";

pub(crate) async fn append(
    state: &AppState,
    request: DiaryAppendRequest,
) -> Result<DiaryAppendResponse> {
    validate_entry(&request.entry)?;
    validate_time_hint(request.time_hint.as_deref())?;
    let config = state.config.read().await.clone();
    let timezone = room_timezone(&config)?;
    let root = diary_root(&config);
    let now = Utc::now();
    let local = now.with_timezone(&timezone);
    let diary_date = diary_date_for_local(local, config.room.diary_day_boundary_hour)?;
    let date = diary_date.to_string();
    let entry = DiaryEntry {
        id: format!("dia_{}", Uuid::new_v4().simple()),
        created_at: now,
        date: date.clone(),
        time_hint: request
            .time_hint
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_TIME_HINT.to_string()),
        tags: request.tags,
        entry: request.entry,
    };
    let path = diary_path(&root, diary_date);
    let _guard = state.notebook_writes.lock().await;
    ensure_diary_layout(&root)?;
    ensure_parent(&path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    Ok(DiaryAppendResponse {
        id: entry.id,
        path: path.display().to_string(),
        created_at: entry.created_at,
        date,
        created: true,
        warnings: Vec::new(),
    })
}

pub(crate) async fn recent(
    state: &AppState,
    request: DiaryRecentRequest,
) -> Result<DiaryEntriesResponse> {
    let config = state.config.read().await.clone();
    let timezone = room_timezone(&config)?;
    let root = diary_root(&config);
    let days = request
        .days
        .unwrap_or(DEFAULT_RECENT_DAYS)
        .clamp(1, MAX_RECENT_DAYS);
    let limit = normalized_limit(request.limit);
    let local_now = Utc::now().with_timezone(&timezone);
    let today = diary_date_for_local(local_now, config.room.diary_day_boundary_hour)?;
    let start = today - Duration::days(days.saturating_sub(1) as i64);
    let dates = date_range(start, today);
    let (mut entries, warnings) = read_entries_for_dates(&root, &dates)?;
    entries.sort_by_key(|entry| entry.created_at);
    let total = entries.len();
    if total > limit {
        entries = entries.into_iter().skip(total - limit).collect();
    }
    Ok(DiaryEntriesResponse { entries, warnings })
}

pub(crate) async fn select_exact(
    state: &AppState,
    request: DiarySelectExactRequest,
) -> Result<DiaryEntriesResponse> {
    let date = NaiveDate::from_ymd_opt(request.year, request.month, request.day)
        .ok_or_else(|| anyhow!("invalid_date"))?;
    let config = state.config.read().await.clone();
    let root = diary_root(&config);
    let (mut entries, warnings) = read_entries_for_dates(&root, &[date])?;
    entries.sort_by_key(|entry| entry.created_at);
    entries.truncate(normalized_limit(request.limit));
    Ok(DiaryEntriesResponse { entries, warnings })
}

pub(crate) fn diary_root(config: &Config) -> PathBuf {
    config.workspace_root.join("diary")
}

fn room_timezone(config: &Config) -> Result<Tz> {
    config
        .room
        .timezone
        .parse::<Tz>()
        .map_err(|_| anyhow!("invalid_room_timezone: {}", config.room.timezone))
}

fn diary_date_for_local(local: DateTime<Tz>, boundary_hour: u32) -> Result<NaiveDate> {
    if boundary_hour > 23 {
        return Err(anyhow!(
            "invalid_diary_day_boundary_hour: {boundary_hour}; expected 0..=23"
        ));
    }
    let date = if local.hour() < boundary_hour {
        local.date_naive() - Duration::days(1)
    } else {
        local.date_naive()
    };
    Ok(date)
}

fn diary_path(root: &Path, date: NaiveDate) -> PathBuf {
    root.join(format!("{:04}", date.year()))
        .join(format!("{:02}", date.month()))
        .join(format!("{:02}.jsonl", date.day()))
}

fn ensure_diary_layout(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    let readme = root.join("README.md");
    if !readme.exists() {
        fs::write(
            readme,
            "# Room Diary\n\nFile-backed room diary storage.\n\nEntries are stored as JSONL under `YYYY/MM/DD.jsonl`, partitioned by the logical diary day in the configured room timezone. Local times before `room.diaryDayBoundaryHour` are assigned to the previous diary date. Entry timestamps are stored as UTC ISO-8601 values. Diary entries are concrete event narratives for cross-day continuity, not task handoff passages or automatic chat logs.\n",
        )?;
    }
    Ok(())
}

fn validate_entry(entry: &str) -> Result<()> {
    if entry.trim().is_empty() {
        return Err(anyhow!("entry_required"));
    }
    Ok(())
}

fn validate_time_hint(time_hint: Option<&str>) -> Result<()> {
    if let Some(value) = time_hint {
        if value.trim().is_empty() {
            return Ok(());
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(anyhow!("invalid_time_hint"));
        }
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

fn read_entries_for_dates(
    root: &Path,
    dates: &[NaiveDate],
) -> Result<(Vec<DiaryEntry>, Vec<String>)> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for date in dates {
        read_entry_file(&diary_path(root, *date), &mut entries, &mut warnings)?;
    }
    Ok((entries, warnings))
}

fn read_entry_file(
    path: &Path,
    entries: &mut Vec<DiaryEntry>,
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
        match serde_json::from_str::<DiaryEntry>(&line) {
            Ok(entry) => entries.push(entry),
            Err(error) => warnings.push(format!(
                "invalid_jsonl: path={}; line={}; error={error}",
                path.display(),
                index + 1
            )),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::utils::ensure_parent;
    use crate::AppState;
    use chrono::TimeZone;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};
    use uuid::Uuid;

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agentic-gpt-diary-test-{name}-{}",
            Uuid::new_v4().simple()
        ))
    }

    fn test_state(workspace_root: PathBuf) -> AppState {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = workspace_root;
        AppState {
            config_path: PathBuf::from("/tmp/test-config.json"),
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

    #[test]
    fn diary_date_uses_sleep_day_boundary() {
        let timezone: Tz = "Asia/Shanghai".parse().unwrap();
        let before_boundary = timezone.with_ymd_and_hms(2026, 6, 23, 4, 59, 0).unwrap();
        let at_boundary = timezone.with_ymd_and_hms(2026, 6, 23, 5, 0, 0).unwrap();

        assert_eq!(
            diary_date_for_local(before_boundary, 5).unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 22).unwrap()
        );
        assert_eq!(
            diary_date_for_local(at_boundary, 5).unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 23).unwrap()
        );
    }

    #[test]
    fn diary_date_rejects_invalid_boundary_hour() {
        let timezone: Tz = "Asia/Shanghai".parse().unwrap();
        let local = timezone.with_ymd_and_hms(2026, 6, 23, 1, 0, 0).unwrap();
        let error = diary_date_for_local(local, 24).unwrap_err().to_string();
        assert!(error.contains("invalid_diary_day_boundary_hour"));
    }

    #[test]
    fn diary_root_uses_workspace_root() {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = PathBuf::from("/tmp/room-workspace");
        assert_eq!(
            diary_root(&config),
            PathBuf::from("/tmp/room-workspace/diary")
        );
    }

    #[tokio::test]
    async fn append_and_select_exact_round_trip() {
        let workspace = unique_temp_dir("append-select").join("workspace");
        let state = test_state(workspace.clone());
        let response = append(
            &state,
            DiaryAppendRequest {
                time_hint: Some("evening".to_string()),
                tags: vec!["agentic".to_string()],
                entry: "用户决定 diary V0 直接使用 workspaceRoot，不新增 root 配置。".to_string(),
            },
        )
        .await
        .unwrap();
        let diary_date = NaiveDate::parse_from_str(&response.date, "%Y-%m-%d").unwrap();
        let exact = select_exact(
            &state,
            DiarySelectExactRequest {
                year: diary_date.year(),
                month: diary_date.month(),
                day: diary_date.day(),
                limit: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(exact.entries.len(), 1);
        assert_eq!(exact.entries[0].id, response.id);
        assert_eq!(exact.entries[0].time_hint, "evening");
        assert_eq!(exact.entries[0].tags, vec!["agentic"]);
        assert!(exact.entries[0].entry.contains("workspaceRoot"));
        assert!(workspace.join("diary").exists());
    }

    #[tokio::test]
    async fn recent_returns_limited_latest_entries() {
        let workspace = unique_temp_dir("recent").join("workspace");
        let state = test_state(workspace);
        for index in 0..3 {
            append(
                &state,
                DiaryAppendRequest {
                    time_hint: None,
                    tags: Vec::new(),
                    entry: format!("entry {index}"),
                },
            )
            .await
            .unwrap();
        }
        let response = recent(
            &state,
            DiaryRecentRequest {
                days: Some(1),
                limit: Some(2),
            },
        )
        .await
        .unwrap();
        assert_eq!(response.entries.len(), 2);
        assert!(response.entries[0].entry.contains("entry 1"));
        assert!(response.entries[1].entry.contains("entry 2"));
    }

    #[test]
    fn damaged_jsonl_returns_warning() {
        let root = unique_temp_dir("damaged").join("diary");
        let path = root.join("2026").join("06").join("22.jsonl");
        ensure_parent(&path).unwrap();
        fs::write(&path, "{not json}\n").unwrap();
        let (entries, warnings) =
            read_entries_for_dates(&root, &[NaiveDate::from_ymd_opt(2026, 6, 22).unwrap()])
                .unwrap();
        assert!(entries.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with("invalid_jsonl"));
    }
}
