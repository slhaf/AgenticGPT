use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
use std::sync::Mutex;

use anyhow::Result;
use chrono::{DateTime, Utc};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use tokio::sync::OwnedMutexGuard;

use agentic_apply_patch::{apply_update, parse_patch, ApplyPatchFileUpdateMode, Hunk};

use crate::audit::{write_file_audit, ChangedLines, FileAuditRecord};
use crate::config::Config;
use crate::exec;
use crate::state::AppState;

pub(crate) const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_READ_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_LINE_DISPLAY_BYTES: usize = 4 * 1024;
pub(crate) const MAX_SEARCH_FILES: usize = 10_000;
pub(crate) const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_SEARCH_RESULTS: usize = 200;
pub(crate) const MAX_SEARCH_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_BATCH_OPERATIONS: usize = 32;
pub(crate) const MAX_BATCH_SCAN_FILES: usize = 20_000;
pub(crate) const MAX_BATCH_SCAN_BYTES: u64 = 128 * 1024 * 1024;
pub(crate) const MAX_BATCH_OUTPUT_BYTES: usize = 1024 * 1024;

#[cfg(test)]
static INJECT_EXTERNAL_CHANGE: Mutex<Option<(PathBuf, Vec<u8>)>> = Mutex::new(None);

#[cfg(test)]
static INJECT_MOVE_SOURCE_REMOVE_FAILURE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
static INJECT_COMMIT_FAILURE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
pub(crate) fn inject_external_change(path: &Path, contents: &[u8]) {
    *INJECT_EXTERNAL_CHANGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some((path.to_path_buf(), contents.to_vec()));
}

#[cfg(test)]
pub(crate) fn inject_move_source_remove_failure(path: &Path) {
    *INJECT_MOVE_SOURCE_REMOVE_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path.to_path_buf());
}

#[cfg(test)]
pub(crate) fn inject_commit_failure(path: &Path) {
    *INJECT_COMMIT_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path.to_path_buf());
}

#[cfg(test)]
fn should_inject_commit_failure(change: &PlannedChange) -> bool {
    let mut failure = INJECT_COMMIT_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if failure
        .as_ref()
        .is_some_and(|path| path == &change.target.path)
    {
        *failure = None;
        true
    } else {
        false
    }
}

#[cfg(test)]
fn take_external_change_for(changes: &[PlannedChange]) -> Option<(PathBuf, Vec<u8>)> {
    let mut pending = INJECT_EXTERNAL_CHANGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let matches = pending.as_ref().is_some_and(|(path, _)| {
        changes.iter().any(|change| {
            change
                .source
                .as_ref()
                .is_some_and(|source| source.path == *path)
                || change.target.path == *path
        })
    });
    if matches {
        pending.take()
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedPath {
    pub(crate) input: String,
    pub(crate) requested: PathBuf,
    pub(crate) path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Access {
    Read,
    Write,
}

pub(crate) async fn lock_target(state: &AppState, path: &Path) -> OwnedMutexGuard<()> {
    let lock = {
        let mut locks = state.file_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(path).and_then(std::sync::Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
            lock
        }
    };
    lock.lock_owned().await
}

enum BoundedRead {
    Complete(Vec<u8>),
    Exceeded,
}

fn read_bounded(path: &Path, limit: u64) -> std::io::Result<BoundedRead> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        Ok(BoundedRead::Exceeded)
    } else {
        Ok(BoundedRead::Complete(bytes))
    }
}

pub(crate) fn resolve_path(
    config: &Config,
    input: &str,
    access: Access,
) -> std::result::Result<ResolvedPath, FileError> {
    if input.trim().is_empty() {
        return Err(FileError::new("file_path_empty", "path must not be empty"));
    }
    let raw = PathBuf::from(input);
    let expanded = exec::expand_pathbuf(&raw)
        .map_err(|_| FileError::new("file_path_invalid", "path could not be expanded"))?;
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        config.workspace_root.join(expanded)
    };
    let metadata = fs::symlink_metadata(&candidate).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            FileError::new("file_not_found", "path was not found")
        } else {
            FileError::new("file_path_unreadable", "path metadata could not be read")
        }
    })?;
    let resolved = if metadata.file_type().is_symlink() {
        fs::canonicalize(&candidate).map_err(|_| {
            FileError::new(
                "file_symlink_rejected",
                "symlink target could not be resolved",
            )
        })?
    } else {
        fs::canonicalize(&candidate)
            .map_err(|_| FileError::new("file_path_unreadable", "path could not be resolved"))?
    };
    check_policy(config, &resolved, access)?;
    if is_reserved_path(config, &resolved) {
        return Err(FileError::new(
            "file_reserved_path",
            "Agentic runtime path is reserved",
        ));
    }
    Ok(ResolvedPath {
        input: input.to_string(),
        requested: candidate,
        path: resolved,
    })
}

pub(crate) fn resolve_absent_path(
    config: &Config,
    input: &str,
) -> std::result::Result<ResolvedPath, FileError> {
    if input.trim().is_empty() {
        return Err(FileError::new("file_path_empty", "path must not be empty"));
    }
    let raw = PathBuf::from(input);
    let expanded = exec::expand_pathbuf(&raw)
        .map_err(|_| FileError::new("file_path_invalid", "path could not be expanded"))?;
    let candidate = if expanded.is_absolute() {
        expanded
    } else {
        config.workspace_root.join(expanded)
    };
    if fs::symlink_metadata(&candidate).is_ok() {
        return Err(FileError::new(
            "file_already_exists",
            "target already exists",
        ));
    }
    let resolved = exec::canonicalize_existing_or_parent(&candidate)
        .map_err(|_| FileError::new("file_parent_not_found", "parent directory was not found"))?;
    if !resolved.parent().is_some_and(Path::is_dir) {
        return Err(FileError::new(
            "file_parent_not_found",
            "parent directory was not found",
        ));
    }
    check_policy(config, &resolved, Access::Write)?;
    if is_reserved_path(config, &resolved) {
        return Err(FileError::new(
            "file_reserved_path",
            "Agentic runtime path is reserved",
        ));
    }
    Ok(ResolvedPath {
        input: input.to_string(),
        requested: candidate,
        path: resolved,
    })
}

fn check_policy(
    config: &Config,
    resolved: &Path,
    access: Access,
) -> std::result::Result<(), FileError> {
    let write_roots = normalized_roots(
        config
            .path_policy
            .write_roots
            .iter()
            .chain(std::iter::once(&config.workspace_root)),
    )?;
    let read_roots = normalized_roots(config.path_policy.read_only_roots.iter())?;
    let deny_roots = normalized_roots(config.path_policy.deny_roots.iter())?;
    if deny_roots.iter().any(|root| resolved.starts_with(root)) {
        return Err(FileError::new(
            "path_denied",
            "path is denied by path policy",
        ));
    }
    if access == Access::Write
        && read_roots.iter().any(|root| resolved.starts_with(root))
        && !write_roots.iter().any(|root| resolved.starts_with(root))
    {
        return Err(FileError::new(
            "path_readonly",
            "path is read-only under path policy",
        ));
    }
    if !write_roots.iter().any(|root| resolved.starts_with(root))
        && !read_roots.iter().any(|root| resolved.starts_with(root))
    {
        return Err(FileError::new(
            "path_denied",
            "path is outside configured path policy roots",
        ));
    }
    Ok(())
}

fn normalized_roots<'a>(
    roots: impl Iterator<Item = &'a PathBuf>,
) -> std::result::Result<Vec<PathBuf>, FileError> {
    roots
        .map(|root| {
            let expanded = exec::expand_pathbuf(root).map_err(|_| {
                FileError::new("path_policy_error", "path root could not be expanded")
            })?;
            exec::canonicalize_existing_or_parent(&expanded)
                .map_err(|_| FileError::new("path_policy_error", "path root could not be resolved"))
        })
        .collect()
}

fn is_reserved_path(config: &Config, path: &Path) -> bool {
    path == config.workspace_root.join(".agentic-gpt-audit.jsonl")
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".agentic-file-tmp-"))
}

pub(crate) fn read(
    resolved: &ResolvedPath,
    include_content: bool,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> std::result::Result<Value, FileError> {
    let metadata = fs::metadata(&resolved.path)
        .map_err(|_| FileError::new("file_not_found", "path was not found"))?;
    let modified_at = metadata
        .modified()
        .ok()
        .map(|time| DateTime::<Utc>::from(time).to_rfc3339());
    let file_type = if metadata.is_dir() {
        "directory"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(FileError::new(
            "file_not_regular",
            "path is not a regular file or directory",
        ));
    }
    let mut result = json!({
        "path": resolved.input,
        "resolvedPath": resolved.path.to_string_lossy(),
        "type": file_type,
        "sizeBytes": metadata.len(),
        "modifiedAt": modified_at,
        "encoding": Value::Null,
    });
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        if include_content && metadata.is_file() && metadata.len() > MAX_FILE_BYTES {
            return Err(FileError::new(
                "file_too_large",
                "file exceeds the 8 MiB content bound",
            ));
        }
        return Ok(result);
    }
    let bytes = match read_bounded(&resolved.path, MAX_FILE_BYTES)
        .map_err(|_| FileError::new("file_read_failed", "file could not be read"))?
    {
        BoundedRead::Complete(bytes) => bytes,
        BoundedRead::Exceeded if !include_content => return Ok(result),
        BoundedRead::Exceeded => {
            return Err(FileError::new(
                "file_too_large",
                "file exceeds the 8 MiB content bound",
            ))
        }
    };
    result["sizeBytes"] = json!(bytes.len());
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) if !include_content => return Ok(result),
        Err(_) => {
            return Err(FileError::new(
                "file_not_utf8",
                "file content is not UTF-8 text",
            ))
        }
    };
    let total_lines = line_count(&text);
    result["encoding"] = json!("utf-8");
    result["totalLines"] = json!(total_lines);
    if !include_content {
        return Ok(result);
    }
    let start = start_line.unwrap_or(1);
    let end = end_line.unwrap_or(total_lines.max(1));
    if start == 0 || end == 0 || end < start {
        return Err(FileError::new(
            "file_invalid_line_range",
            "line range is invalid",
        ));
    }
    let (content, returned_through, returned_bytes, truncated, last_complete, next_start) =
        bounded_lines(&text, start, end);
    result["content"] = json!(content);
    result["startLine"] = json!(start);
    result["returnedThroughLine"] = returned_through.map_or(Value::Null, |line| json!(line));
    result["returnedBytes"] = json!(returned_bytes);
    result["truncated"] = json!(truncated);
    result["lastLineComplete"] = json!(last_complete);
    if let Some(next_start) = next_start {
        result["nextStartLine"] = json!(next_start);
    }
    Ok(result)
}

pub(crate) fn revision(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchMode {
    Literal,
    Regex,
}

pub(crate) struct SearchOptions<'a> {
    pub(crate) root: &'a ResolvedPath,
    pub(crate) query: &'a str,
    pub(crate) mode: SearchMode,
    pub(crate) case_sensitive: bool,
    pub(crate) include: &'a [String],
    pub(crate) exclude: &'a [String],
    pub(crate) context_lines: usize,
    pub(crate) max_results: usize,
    pub(crate) hidden: bool,
    pub(crate) respect_gitignore: bool,
    pub(crate) scan_file_limit: usize,
    pub(crate) scan_byte_limit: u64,
}

pub(crate) fn search_with_context_limit(
    options: SearchOptions<'_>,
    configured_max_context_lines: usize,
) -> std::result::Result<Value, FileError> {
    if options.query.trim().is_empty() {
        return Err(FileError::new(
            "file_search_query_empty",
            "query must not be empty",
        ));
    }
    let effective_context_lines = options
        .context_lines
        .min(configured_max_context_lines)
        .min(crate::config::MAX_FILE_SEARCH_CONTEXT_LINES);
    let context_lines_clipped = options.context_lines != effective_context_lines;
    let warnings = if context_lines_clipped {
        json!(["context_lines_clipped_to_configured_limit"])
    } else {
        json!([])
    };
    if options.max_results == 0 || options.max_results > MAX_SEARCH_RESULTS {
        return Err(FileError::new(
            "file_result_limit_exceeded",
            "maxResults is outside the bound",
        ));
    }
    let include_set = compile_globs(options.include)?;
    let exclude_set = compile_globs(options.exclude)?;
    let pattern = match options.mode {
        SearchMode::Literal => regex::escape(options.query),
        SearchMode::Regex => options.query.to_string(),
    };
    let pattern = if options.case_sensitive {
        pattern
    } else {
        format!("(?i:{pattern})")
    };
    let matcher = Regex::new(&pattern)
        .map_err(|_| FileError::new("file_invalid_regex", "query is not a valid regex"))?;
    let mut matches = Vec::new();
    let mut matches_bytes = 2usize;
    let mut scanned_files = 0usize;
    let mut scanned_bytes = 0u64;
    let mut skipped = json!({"tooLarge": 0, "nonUtf8": 0, "symlink": 0, "unreadable": 0});
    let mut truncated = false;
    let mut truncation_reason = None::<&str>;
    let root_is_file = options.root.path.is_file();
    let entries = WalkBuilder::new(&options.root.path)
        .hidden(!options.hidden)
        .git_ignore(options.respect_gitignore)
        .git_global(options.respect_gitignore)
        .git_exclude(options.respect_gitignore)
        .follow_links(false)
        .build();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                increment_skip(&mut skipped, "unreadable");
                continue;
            }
        };
        let path = entry.path();
        if path == options.root.path && path.is_dir() {
            continue;
        }
        if entry.file_type().is_some_and(|kind| kind.is_symlink()) {
            increment_skip(&mut skipped, "symlink");
            continue;
        }
        if !path.is_file() {
            continue;
        }
        if scanned_files >= options.scan_file_limit {
            truncated = true;
            truncation_reason = Some("scan_files");
            break;
        }
        scanned_files += 1;
        let relative = if root_is_file {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            path.strip_prefix(&options.root.path)
                .unwrap_or(path)
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/")
        };
        if include_set
            .as_ref()
            .is_some_and(|set| !set.is_match(&relative))
            || exclude_set
                .as_ref()
                .is_some_and(|set| set.is_match(&relative))
        {
            continue;
        }
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => {
                increment_skip(&mut skipped, "unreadable");
                continue;
            }
        };
        if metadata.len() > MAX_FILE_BYTES {
            increment_skip(&mut skipped, "tooLarge");
            continue;
        }
        let remaining_bytes = options.scan_byte_limit.saturating_sub(scanned_bytes);
        if metadata.len() > remaining_bytes {
            truncated = true;
            truncation_reason = Some("scan_bytes");
            break;
        }
        let bytes = match read_bounded(path, remaining_bytes.min(MAX_FILE_BYTES)) {
            Ok(BoundedRead::Complete(bytes)) => bytes,
            Ok(BoundedRead::Exceeded) => {
                truncated = true;
                truncation_reason = Some("scan_bytes");
                break;
            }
            Err(_) => {
                increment_skip(&mut skipped, "unreadable");
                continue;
            }
        };
        scanned_bytes += bytes.len() as u64;
        if bytes.contains(&0) {
            increment_skip(&mut skipped, "nonUtf8");
            continue;
        }
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                increment_skip(&mut skipped, "nonUtf8");
                continue;
            }
        };
        let lines = text.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            for found in matcher.find_iter(line) {
                let before = (index.saturating_sub(effective_context_lines)..index)
                    .map(|line_index| line_display(lines[line_index]))
                    .collect::<Vec<_>>();
                let after = ((index + 1)
                    ..=(index + effective_context_lines).min(lines.len().saturating_sub(1)))
                    .map(|line_index| line_display(lines[line_index]))
                    .collect::<Vec<_>>();
                let value = json!({
                    "path": &relative,
                    "line": index + 1,
                    "column": line[..found.start()].chars().count() + 1,
                    "lineText": line_display(line),
                    "before": before,
                    "after": after,
                });
                if matches.len() >= options.max_results {
                    truncated = true;
                    truncation_reason = Some("max_results");
                    break;
                }
                let candidate_bytes = serde_json::to_vec(&value).unwrap_or_default().len();
                let next_matches_bytes = matches_bytes
                    .saturating_add(candidate_bytes)
                    .saturating_add(usize::from(!matches.is_empty()));
                if next_matches_bytes > MAX_SEARCH_OUTPUT_BYTES {
                    truncated = true;
                    truncation_reason = Some("output_bytes");
                    break;
                }
                matches.push(value);
                matches_bytes = next_matches_bytes;
            }
            if truncation_reason.is_some() {
                break;
            }
        }
        if truncation_reason.is_some() {
            break;
        }
    }
    Ok(json!({
        "query": options.query,
        "mode": match options.mode { SearchMode::Literal => "literal", SearchMode::Regex => "regex" },
        "requestedContextLines": options.context_lines,
        "effectiveContextLines": effective_context_lines,
        "contextLinesClipped": context_lines_clipped,
        "warnings": warnings,
        "matches": matches,
        "matchCount": matches.len(),
        "scannedFiles": scanned_files,
        "scannedBytes": scanned_bytes,
        "skippedFiles": skipped,
        "truncated": truncated,
        "truncationReason": truncation_reason,
    }))
}

fn compile_globs(patterns: &[String]) -> std::result::Result<Option<GlobSet>, FileError> {
    if patterns.len() > 16 {
        return Err(FileError::new(
            "file_invalid_glob",
            "too many glob patterns",
        ));
    }
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(
            Glob::new(pattern)
                .map_err(|_| FileError::new("file_invalid_glob", "invalid glob pattern"))?,
        );
    }
    builder
        .build()
        .map(Some)
        .map_err(|_| FileError::new("file_invalid_glob", "invalid glob pattern"))
}

fn increment_skip(skipped: &mut Value, key: &str) {
    if let Some(value) = skipped.get(key).and_then(Value::as_u64) {
        skipped[key] = json!(value.saturating_add(1));
    }
}

fn line_display(line: &str) -> String {
    utf8_prefix(line, MAX_LINE_DISPLAY_BYTES).to_string()
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.as_bytes()
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + usize::from(!text.ends_with('\n'))
    }
}

fn bounded_lines(
    text: &str,
    start: usize,
    end: usize,
) -> (String, Option<usize>, usize, bool, bool, Option<usize>) {
    if text.is_empty() || start > line_count(text) {
        return (String::new(), None, 0, false, true, None);
    }
    let mut output = String::new();
    let mut returned_through = None;
    let mut truncated = false;
    let mut last_complete = true;
    for (line_number, line) in (1..).zip(text.split_inclusive('\n')) {
        if line_number >= start && line_number <= end {
            let available = MAX_READ_OUTPUT_BYTES.saturating_sub(output.len());
            if line.len() <= available {
                output.push_str(line);
                returned_through = Some(line_number);
            } else {
                output.push_str(utf8_prefix(line, available));
                truncated = true;
                last_complete = false;
                break;
            }
        }
        if line_number >= end {
            break;
        }
    }
    if returned_through.is_some_and(|line| line < end) {
        truncated = true;
    }
    let next_start = (truncated && last_complete)
        .then(|| returned_through.map(|line| line + 1))
        .flatten();
    (
        output.clone(),
        returned_through,
        output.len(),
        truncated,
        last_complete,
        next_start,
    )
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[derive(Clone, Debug)]
pub(crate) struct FileError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl FileError {
    pub(crate) fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }

    pub(crate) fn value(&self) -> Value {
        json!({"error": {"code": self.code, "message": self.message}})
    }
}

pub(crate) fn to_result(result: std::result::Result<Value, FileError>) -> Result<Value> {
    Ok(result.unwrap_or_else(|error| error.value()))
}

#[derive(Clone, Debug)]
pub(crate) struct ReadRequest {
    pub(crate) path: String,
    pub(crate) include_content: bool,
    pub(crate) start_line: Option<usize>,
    pub(crate) end_line: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct SearchRequest {
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) mode: Option<String>,
    pub(crate) case_sensitive: bool,
    pub(crate) include: Vec<String>,
    pub(crate) exclude: Vec<String>,
    pub(crate) context_lines: usize,
    pub(crate) max_results: usize,
    pub(crate) hidden: bool,
    pub(crate) respect_gitignore: bool,
}

pub(crate) async fn read_batch(state: &AppState, requests: &[ReadRequest]) -> Value {
    let config = state.config.read().await.clone();
    let results = requests
        .iter()
        .enumerate()
        .map(|(index, request)| {
            let value = resolve_path(&config, &request.path, Access::Read).and_then(|resolved| {
                read(
                    &resolved,
                    request.include_content,
                    request.start_line,
                    request.end_line,
                )
            });
            envelope(index, value)
        })
        .collect::<Vec<_>>();
    finalize_read_search_batch(results)
}

pub(crate) async fn search_batch(state: &AppState, requests: &[SearchRequest]) -> Value {
    let config = state.config.read().await.clone();
    let mut scanned_files = 0usize;
    let mut scanned_bytes = 0u64;
    let mut results = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        let remaining_batch_files = MAX_BATCH_SCAN_FILES.saturating_sub(scanned_files);
        let remaining_batch_bytes = MAX_BATCH_SCAN_BYTES.saturating_sub(scanned_bytes);
        let result = match request.mode.as_deref() {
            None | Some("literal") => Ok(SearchMode::Literal),
            Some("regex") => Ok(SearchMode::Regex),
            Some(_) => Err(FileError::new(
                "file_invalid_mode",
                "mode must be literal or regex",
            )),
        }
        .and_then(|mode| {
            resolve_path(&config, &request.path, Access::Read).map(|resolved| (resolved, mode))
        })
        .map_err(|error| {
            if error.code == "file_not_found" {
                FileError::new("file_search_path_not_found", "search path was not found")
            } else {
                error
            }
        })
        .and_then(|(resolved, mode)| {
            search_with_context_limit(
                SearchOptions {
                    root: &resolved,
                    query: &request.query,
                    mode,
                    case_sensitive: request.case_sensitive,
                    include: &request.include,
                    exclude: &request.exclude,
                    context_lines: request.context_lines,
                    max_results: request.max_results,
                    hidden: request.hidden,
                    respect_gitignore: request.respect_gitignore,
                    scan_file_limit: MAX_SEARCH_FILES.min(remaining_batch_files),
                    scan_byte_limit: MAX_SEARCH_BYTES.min(remaining_batch_bytes),
                },
                config.limits.max_file_search_context_lines,
            )
        });
        let value = match result {
            Ok(value) => {
                scanned_files = scanned_files
                    .saturating_add(value["scannedFiles"].as_u64().unwrap_or(0) as usize);
                scanned_bytes =
                    scanned_bytes.saturating_add(value["scannedBytes"].as_u64().unwrap_or(0));
                let aggregate_limit_hit = (value["truncationReason"] == "scan_files"
                    && remaining_batch_files <= MAX_SEARCH_FILES
                    && value["scannedFiles"].as_u64().unwrap_or(0) >= remaining_batch_files as u64)
                    || (value["truncationReason"] == "scan_bytes"
                        && remaining_batch_bytes <= MAX_SEARCH_BYTES
                        && value["scannedBytes"].as_u64().unwrap_or(0) >= remaining_batch_bytes);
                if aggregate_limit_hit {
                    Err(FileError::new(
                        "file_batch_scan_limit_exceeded",
                        "aggregate search scan exceeds the batch bound",
                    ))
                } else {
                    Ok(value)
                }
            }
            Err(error) => Err(error),
        };
        results.push(envelope(index, value));
    }
    finalize_read_search_batch(results)
}

fn envelope(index: usize, result: std::result::Result<Value, FileError>) -> Value {
    match result {
        Ok(result) => json!({"index": index, "status": "completed", "result": result}),
        Err(error) => {
            json!({"index": index, "status": "failed", "error": error.value()["error"].clone()})
        }
    }
}

fn finalize_read_search_batch(results: Vec<Value>) -> Value {
    let failed = results
        .iter()
        .filter(|value| value["status"] == "failed")
        .count();
    let mut response = json!({
        "status": if failed == 0 { "completed" } else { "completed_with_errors" },
        "results": results,
    });
    truncate_batch_output(&mut response);
    response
}

fn truncate_batch_output(response: &mut Value) {
    let mut bytes = serde_json::to_vec(response).unwrap_or_default();
    if bytes.len() <= MAX_BATCH_OUTPUT_BYTES {
        return;
    }
    response["truncated"] = json!(true);
    response["truncationReason"] = json!("output_bytes");
    let result_count = response["results"].as_array().map_or(0, Vec::len);
    for index in 0..result_count {
        if let Some(payload) = response["results"][index]
            .get_mut("result")
            .and_then(Value::as_object_mut)
        {
            payload.remove("content");
            payload.remove("matches");
            payload.remove("diff");
            payload.insert("resultTruncated".to_string(), json!(true));
        }
        bytes = serde_json::to_vec(response).unwrap_or_default();
        if bytes.len() <= MAX_BATCH_OUTPUT_BYTES {
            return;
        }
    }
    if bytes.len() > MAX_BATCH_OUTPUT_BYTES {
        if let Some(results) = response["results"].as_array_mut() {
            for result in results {
                let mut compact = json!({
                    "index": result["index"],
                    "status": result["status"],
                });
                if result.get("error").is_some() {
                    compact["error"] = result["error"].clone();
                }
                *result = compact;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EditRequest {
    pub(crate) patch: String,
    pub(crate) need_confirm: bool,
}

#[derive(Clone, Debug)]
struct PlannedChange {
    action: &'static str,
    source: Option<ResolvedPath>,
    target: ResolvedPath,
    destination: Option<ResolvedPath>,
    before_bytes: Option<Vec<u8>>,
    before_revision: Option<String>,
    before_mode: Option<fs::Permissions>,
    candidate: Option<Vec<u8>>,
    after_revision: Option<String>,
    diff: String,
    diff_truncated: bool,
    changed_lines: Value,
    temp: Option<PathBuf>,
}

pub(crate) async fn edit(state: &AppState, request: EditRequest) -> Value {
    let started = std::time::Instant::now();
    let config = state.config.read().await.clone();
    let result = apply_patch_inner(state, &config, &request).await;
    match result {
        Ok(response) => {
            let audit_status = write_patch_audits(
                &config,
                &response,
                request.need_confirm,
                started.elapsed().as_millis(),
            );
            let mut value = slim_edit_response(&response);
            add_audit_warning(&mut value, audit_status);
            value
        }
        Err(error) => {
            let audit_status = write_patch_failure_audit(
                &config,
                &error.code,
                request.need_confirm,
                started.elapsed().as_millis(),
            );
            let mut value = error.value();
            add_audit_warning(&mut value, audit_status);
            value
        }
    }
}

async fn apply_patch_inner(
    state: &AppState,
    config: &Config,
    request: &EditRequest,
) -> std::result::Result<Value, FileError> {
    let parsed = parse_patch(&request.patch)
        .map_err(|error| FileError::new("file_patch_invalid", &error.to_string()))?;
    if parsed.hunks.is_empty() {
        return Err(FileError::new(
            "file_patch_invalid",
            "patch contains no files",
        ));
    }
    let mut changes = Vec::new();
    let mut lock_paths = Vec::new();
    let mut normalized_paths = Vec::new();
    for hunk in parsed.hunks {
        let path = hunk.path().to_string_lossy().to_string();
        match hunk {
            Hunk::AddFile { contents, .. } => {
                let target = resolve_absent_path(config, &path)?;
                if contents.len() > MAX_FILE_BYTES as usize {
                    return Err(FileError::new(
                        "file_too_large",
                        "candidate exceeds the 8 MiB edit bound",
                    ));
                }
                let candidate = contents.into_bytes();
                let (diff, diff_truncated, changed_lines) =
                    bounded_diff("", &String::from_utf8_lossy(&candidate));
                lock_paths.push(target.path.clone());
                normalized_paths.push(target.path.clone());
                changes.push(PlannedChange {
                    action: "add",
                    source: None,
                    target,
                    destination: None,
                    before_bytes: None,
                    before_revision: None,
                    before_mode: None,
                    after_revision: Some(revision(&candidate)),
                    candidate: Some(candidate),
                    diff,
                    diff_truncated,
                    changed_lines,
                    temp: None,
                });
            }
            Hunk::DeleteFile { .. } => {
                let source = resolve_path(config, &path, Access::Write)?;
                let (bytes, text, mode) = load_edit_source(&source)?;
                let (diff, diff_truncated, changed_lines) = bounded_diff(&text, "");
                lock_paths.push(source.path.clone());
                normalized_paths.push(source.path.clone());
                changes.push(PlannedChange {
                    action: "delete",
                    source: Some(source.clone()),
                    target: source,
                    destination: None,
                    before_revision: Some(revision(&bytes)),
                    before_bytes: Some(bytes),
                    before_mode: mode,
                    candidate: None,
                    after_revision: None,
                    diff,
                    diff_truncated,
                    changed_lines,
                    temp: None,
                });
            }
            Hunk::UpdateFile {
                path: source_path,
                move_path,
                chunks,
            } => {
                let source_name = source_path.to_string_lossy().to_string();
                let source = resolve_path(config, &source_name, Access::Write)?;
                let (bytes, text, mode) = load_edit_source(&source)?;
                let candidate_text = apply_update(
                    &text,
                    &source_name,
                    &chunks,
                    ApplyPatchFileUpdateMode::PreserveLineEndings,
                )
                .map_err(|error| FileError::new("file_patch_conflict", &error.to_string()))?;
                let (diff, diff_truncated, changed_lines) = bounded_diff(&text, &candidate_text);
                let candidate = candidate_text.into_bytes();
                if candidate.len() > MAX_FILE_BYTES as usize {
                    return Err(FileError::new(
                        "file_too_large",
                        "candidate exceeds the 8 MiB edit bound",
                    ));
                }
                let destination = move_path
                    .map(|path| resolve_absent_path(config, &path.to_string_lossy()))
                    .transpose()?;
                let target = destination.clone().unwrap_or_else(|| source.clone());
                lock_paths.push(source.path.clone());
                normalized_paths.push(source.path.clone());
                if let Some(destination) = &destination {
                    lock_paths.push(destination.path.clone());
                    normalized_paths.push(destination.path.clone());
                }
                changes.push(PlannedChange {
                    action: if destination.is_some() {
                        "move"
                    } else {
                        "update"
                    },
                    source: Some(source.clone()),
                    target,
                    destination,
                    before_revision: Some(revision(&bytes)),
                    before_bytes: Some(bytes),
                    before_mode: mode,
                    after_revision: Some(revision(&candidate)),
                    candidate: Some(candidate),
                    diff,
                    diff_truncated,
                    changed_lines,
                    temp: None,
                });
            }
        }
    }
    reject_overlapping_paths(&normalized_paths)?;
    lock_paths.sort();
    lock_paths.dedup();
    let mut locks = Vec::new();
    for path in &lock_paths {
        locks.push(lock_target(state, path).await);
    }

    for change in &changes {
        if let Some(source) = &change.source {
            revalidate_target(config, source, true)?;
            let current = load_edit_source(source)?.0;
            if Some(revision(&current)) != change.before_revision {
                return Err(FileError::new(
                    "file_revision_conflict",
                    "source changed before commit",
                ));
            }
        }
        if change.action == "add" || change.action == "move" {
            revalidate_target(config, &change.target, false)?;
        }
    }

    let mut response_changes = changes.iter().map(change_value).collect::<Vec<_>>();
    let effective = changes.iter().filter(|change| is_effective(change)).count();

    if effective > 0 {
        for change in &mut changes {
            if is_effective(change) {
                if let Some(candidate) = &change.candidate {
                    match stage_temp(&change.target.path, candidate, change.before_mode.as_ref()) {
                        Ok(temp) => change.temp = Some(temp),
                        Err(error) => {
                            cleanup_temps(&mut changes);
                            return Err(error);
                        }
                    }
                }
            }
        }
    }

    if effective > 0 && request.need_confirm {
        let preview = changes
            .iter()
            .filter(|change| is_effective(change))
            .map(|change| {
                format!(
                    "{}:{}:+{}-{}",
                    change.action,
                    change.target.path.display(),
                    change.changed_lines["added"],
                    change.changed_lines["removed"]
                )
            })
            .collect::<Vec<_>>();
        let confirmation =
            crate::confirmation::request_confirmation(state, config, None, "file.edit", &preview)
                .await;
        if confirmation != "allow_once" {
            cleanup_temps(&mut changes);
            let code = if confirmation == "confirmation_provider_unavailable" {
                "file_confirmation_unavailable"
            } else {
                "file_confirmation_denied"
            };
            return Err(FileError::new(code, "file mutation was not confirmed"));
        }
    }
    // Revalidate again after staging and immediately before the first commit.
    #[cfg(test)]
    if let Some((path, contents)) = take_external_change_for(&changes) {
        let _ = fs::write(path, contents);
    }
    for change in &changes {
        if let Some(source) = &change.source {
            revalidate_target(config, source, true)?;
            let current = load_edit_source(source)?.0;
            if Some(revision(&current)) != change.before_revision {
                cleanup_temps(&mut changes);
                return Err(FileError::new(
                    "file_revision_conflict",
                    "source changed before commit",
                ));
            }
        }
        if change.action == "add" || change.action == "move" {
            revalidate_target(config, &change.target, false)?;
        }
    }

    let mut committed_count = 0usize;
    for (change_index, change) in changes.iter_mut().enumerate() {
        if !is_effective(change) {
            response_changes[change_index]["status"] = json!("unchanged");
            continue;
        }
        #[cfg(test)]
        let injected_failure = should_inject_commit_failure(change);
        #[cfg(not(test))]
        let injected_failure = false;
        let result = if injected_failure {
            Err(FileError::new(
                "file_write_failed",
                "injected commit failure",
            ))
        } else {
            match change.action {
                "add" => commit_create(change),
                "update" => commit_replace(change),
                "delete" => commit_delete(change),
                "move" => commit_move(change),
                _ => unreachable!(),
            }
        };
        if let Err(error) = result {
            cleanup_temps(&mut changes);
            if committed_count == 0 {
                return Err(error);
            }
            response_changes[change_index]["status"] = json!("failed");
            response_changes[change_index]["error"] = error.value()["error"].clone();
            for (value, later_change) in response_changes
                .iter_mut()
                .zip(&changes)
                .skip(change_index + 1)
            {
                if is_effective(later_change) {
                    value["status"] = json!("skipped-not-attempted");
                } else {
                    value["status"] = json!("unchanged");
                }
            }
            let mut response = patch_response("completed_with_errors", response_changes, effective);
            response["summary"]["planned"] = json!(effective);
            response["summary"]["changed"] = json!(committed_count);
            response["summary"]["committed"] = json!(committed_count);
            response["summary"]["failed"] = json!(1);
            response["summary"]["skipped"] = json!(changes
                .iter()
                .skip(change_index + 1)
                .filter(|later| is_effective(later))
                .count());
            if effective > 0 && request.need_confirm {
                response["confirmation"] = json!({"requested": true, "result": "allow_once"});
            }
            return Ok(response);
        }
        let status = match change.action {
            "add" => "created",
            "delete" => "deleted",
            "move" => "moved",
            _ => "updated",
        };
        response_changes[change_index]["status"] = json!(status);
        committed_count += 1;
        if let Some(parent) = change.target.path.parent() {
            sync_parent(parent);
        }
        if let Some(source) = &change.source {
            sync_parent(source.path.parent().unwrap_or(Path::new("")));
        }
    }
    cleanup_temps(&mut changes);
    for (value, change) in response_changes.iter_mut().zip(&changes) {
        value["status"] = json!(if is_effective(change) {
            match change.action {
                "add" => "created",
                "delete" => "deleted",
                "move" => "moved",
                _ => "updated",
            }
        } else {
            "unchanged"
        });
    }
    let mut response = patch_response("completed", response_changes, effective);
    if effective > 0 && request.need_confirm {
        response["confirmation"] = json!({"requested": true, "result": "allow_once"});
    }
    Ok(response)
}

fn load_edit_source(
    source: &ResolvedPath,
) -> std::result::Result<(Vec<u8>, String, Option<fs::Permissions>), FileError> {
    let metadata = fs::metadata(&source.path)
        .map_err(|_| FileError::new("file_not_found", "source was not found"))?;
    if !metadata.is_file() {
        return Err(FileError::new(
            "file_not_regular",
            "source is not a regular file",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(FileError::new(
            "file_too_large",
            "source exceeds the 8 MiB edit bound",
        ));
    }
    let bytes = match read_bounded(&source.path, MAX_FILE_BYTES)
        .map_err(|_| FileError::new("file_read_failed", "source could not be read"))?
    {
        BoundedRead::Complete(bytes) => bytes,
        BoundedRead::Exceeded => {
            return Err(FileError::new(
                "file_too_large",
                "source exceeds the 8 MiB edit bound",
            ))
        }
    };
    let text = String::from_utf8(bytes.clone())
        .map_err(|_| FileError::new("file_not_utf8", "source is not UTF-8 text"))?;
    Ok((bytes, text, Some(metadata.permissions())))
}

fn reject_overlapping_paths(paths: &[PathBuf]) -> std::result::Result<(), FileError> {
    let mut paths = paths.to_vec();
    paths.sort();
    for pair in paths.windows(2) {
        if pair[1] == pair[0] || pair[1].starts_with(&pair[0]) {
            return Err(FileError::new(
                "file_patch_ambiguous_paths",
                "patch source and destination paths overlap or repeat",
            ));
        }
    }
    Ok(())
}

fn is_effective(change: &PlannedChange) -> bool {
    match (&change.before_bytes, &change.candidate) {
        (Some(before), Some(candidate)) => before != candidate || change.action == "move",
        (None, Some(_)) | (Some(_), None) => true,
        (None, None) => false,
    }
}

fn change_value(change: &PlannedChange) -> Value {
    let requested = change
        .source
        .as_ref()
        .map(|source| source.input.as_str())
        .unwrap_or(&change.target.input);
    let mut value = json!({
        "requestedPath": requested,
        "resolvedPath": change.target.path.to_string_lossy(),
        "action": change.action,
        "status": "pending",
        "diff": change.diff,
        "diffTruncated": change.diff_truncated,
        "changedLines": change.changed_lines,
        "beforeRevision": change.before_revision,
        "afterRevision": change.after_revision,
    });
    if let Some(destination) = &change.destination {
        value["destinationRequestedPath"] = json!(destination.input);
        value["destinationResolvedPath"] = json!(destination.path.to_string_lossy());
    }
    value
}

fn slim_edit_response(response: &Value) -> Value {
    let status = response["status"].as_str().unwrap_or("completed");
    let changes = response["changes"].as_array().cloned().unwrap_or_default();
    if status == "completed" {
        let changes = changes
            .iter()
            .filter_map(slim_committed_change)
            .collect::<Vec<_>>();
        return json!({
            "status": "completed",
            "changed": changes.len(),
            "changes": changes,
        });
    }

    let changes = changes.iter().map(slim_evidence_change).collect::<Vec<_>>();
    let mut value = json!({
        "status": status,
        "changes": changes,
    });
    if status == "completed_with_errors" {
        value["summary"] = json!({
            "committed": response["summary"]["committed"],
            "failed": response["summary"]["failed"],
            "skipped": response["summary"]["skipped"],
        });
    }
    value
}

fn slim_committed_change(change: &Value) -> Option<Value> {
    let action = change["status"].as_str()?;
    if !matches!(action, "created" | "updated" | "deleted" | "moved") {
        return None;
    }
    let mut value = json!({
        "path": change["requestedPath"],
        "action": action,
    });
    if action == "moved" {
        value["destination"] = change["destinationRequestedPath"].clone();
    }
    Some(value)
}

fn slim_evidence_change(change: &Value) -> Value {
    let status = change["status"].as_str().unwrap_or("failed");
    let mut value = json!({
        "path": change["requestedPath"],
        "status": status,
    });
    if !change["destinationRequestedPath"].is_null() {
        value["destination"] = change["destinationRequestedPath"].clone();
    }
    if status == "failed" {
        value["error"] = change["error"].clone();
    }
    value
}

fn add_audit_warning(value: &mut Value, audit_status: &str) {
    if audit_status == "failed" {
        value["warnings"] = json!([{
            "code": "file_audit_failed",
            "message": "file edit audit record could not be written"
        }]);
    }
}

fn patch_response(status: &str, changes: Vec<Value>, effective: usize) -> Value {
    json!({
        "status": status,
        "summary": {"total": changes.len(), "changed": effective, "unchanged": changes.len().saturating_sub(effective)},
        "changes": changes,
    })
}

fn cleanup_temps(changes: &mut [PlannedChange]) {
    for change in changes {
        if let Some(temp) = change.temp.take() {
            let _ = fs::remove_file(temp);
        }
    }
}

fn commit_create(change: &mut PlannedChange) -> std::result::Result<(), FileError> {
    let temp = change
        .temp
        .take()
        .ok_or_else(|| FileError::new("file_write_failed", "staged file is missing"))?;
    fs::hard_link(&temp, &change.target.requested).map_err(|error| {
        FileError::new("file_write_failed", &format!("file commit failed: {error}"))
    })?;
    let _ = fs::remove_file(temp);
    Ok(())
}

fn commit_replace(change: &mut PlannedChange) -> std::result::Result<(), FileError> {
    let temp = change
        .temp
        .take()
        .ok_or_else(|| FileError::new("file_write_failed", "staged file is missing"))?;
    fs::rename(temp, &change.target.path).map_err(|error| {
        FileError::new(
            "file_write_failed",
            &format!("atomic file commit failed: {error}"),
        )
    })
}

fn commit_delete(change: &mut PlannedChange) -> std::result::Result<(), FileError> {
    fs::remove_file(&change.target.path).map_err(|error| {
        FileError::new("file_write_failed", &format!("file delete failed: {error}"))
    })
}

fn commit_move(change: &mut PlannedChange) -> std::result::Result<(), FileError> {
    let destination = change
        .destination
        .as_ref()
        .ok_or_else(|| FileError::new("file_write_failed", "move destination is missing"))?;
    let temp = change
        .temp
        .take()
        .ok_or_else(|| FileError::new("file_write_failed", "staged file is missing"))?;
    fs::rename(temp, &destination.requested).map_err(|error| {
        FileError::new(
            "file_write_failed",
            &format!("move destination commit failed: {error}"),
        )
    })?;
    let source = change
        .source
        .as_ref()
        .ok_or_else(|| FileError::new("file_write_failed", "move source is missing"))?;
    #[cfg(test)]
    let injected_failure = INJECT_MOVE_SOURCE_REMOVE_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
        .is_some_and(|path| path == source.path);
    #[cfg(not(test))]
    let injected_failure = false;
    let source_result = if injected_failure {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected source removal failure",
        ))
    } else {
        fs::remove_file(&source.path)
    };
    if let Err(error) = source_result {
        let _ = fs::remove_file(&destination.requested);
        return Err(FileError::new(
            "file_write_failed",
            &format!("move source delete failed: {error}"),
        ));
    }
    Ok(())
}

fn revalidate_target(
    config: &Config,
    target: &ResolvedPath,
    existed: bool,
) -> std::result::Result<(), FileError> {
    if existed {
        let current = resolve_path(config, &target.input, Access::Write).map_err(|_| {
            FileError::new("file_revision_conflict", "source changed before commit")
        })?;
        if current.path != target.path {
            return Err(FileError::new(
                "file_symlink_rejected",
                "target path changed before commit",
            ));
        }
    } else {
        if fs::symlink_metadata(&target.requested).is_ok() {
            return Err(FileError::new(
                "file_already_exists",
                "target already exists",
            ));
        }
        let parent = target.requested.parent().ok_or_else(|| {
            FileError::new("file_parent_not_found", "target parent was not found")
        })?;
        let canonical_parent = fs::canonicalize(parent)
            .map_err(|_| FileError::new("file_parent_not_found", "target parent was not found"))?;
        if canonical_parent != target.path.parent().unwrap_or(Path::new("")) {
            return Err(FileError::new(
                "file_symlink_rejected",
                "parent path changed before commit",
            ));
        }
    }
    Ok(())
}

fn stage_temp(
    target: &Path,
    bytes: &[u8],
    permissions: Option<&fs::Permissions>,
) -> std::result::Result<PathBuf, FileError> {
    let parent = target
        .parent()
        .ok_or_else(|| FileError::new("file_parent_not_found", "target parent was not found"))?;
    for _ in 0..3 {
        let temp = parent.join(format!(
            ".agentic-file-tmp-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => file,
            Err(_) => continue,
        };
        use std::io::Write;
        if file.write_all(bytes).is_err()
            || permissions
                .is_some_and(|permissions| file.set_permissions(permissions.clone()).is_err())
            || file.sync_all().is_err()
        {
            let _ = fs::remove_file(&temp);
            return Err(FileError::new(
                "file_write_failed",
                "temporary file could not be staged",
            ));
        }
        return Ok(temp);
    }
    Err(FileError::new(
        "file_write_failed",
        "temporary file could not be created",
    ))
}

fn sync_parent(parent: &Path) {
    if let Ok(file) = fs::File::open(parent) {
        let _ = file.sync_all();
    }
}

const MAX_DIFF_BYTES: usize = 64 * 1024;
const DIFF_CONTEXT_LINES: usize = 3;

fn bounded_diff(before: &str, after: &str) -> (String, bool, Value) {
    let text_diff = TextDiff::from_lines(before, after);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in text_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    let mut diff = text_diff
        .unified_diff()
        .context_radius(DIFF_CONTEXT_LINES)
        .header("before", "after")
        .to_string();
    let truncated = diff.len() > MAX_DIFF_BYTES;
    if truncated {
        diff = utf8_prefix(&diff, MAX_DIFF_BYTES).to_string();
    }
    (diff, truncated, json!({"added": added, "removed": removed}))
}

fn write_patch_audits(
    config: &Config,
    response: &Value,
    need_confirm: bool,
    duration_ms: u128,
) -> &'static str {
    let mut ok = true;
    if let Some(changes) = response["changes"].as_array() {
        for change in changes {
            let outcome = change["status"].as_str().unwrap_or("failed");
            let error_code = change["error"]["code"].as_str().map(str::to_string);
            let before_revision = change["beforeRevision"].as_str().map(str::to_string);
            let after_revision = change["afterRevision"].as_str().map(str::to_string);
            let confirmation_result = response["confirmation"]["result"]
                .as_str()
                .map(str::to_string);
            let changed_lines = change["changedLines"].as_object().and_then(|lines| {
                Some(ChangedLines {
                    added: lines.get("added")?.as_u64()? as usize,
                    removed: lines.get("removed")?.as_u64()? as usize,
                })
            });
            if write_file_audit(
                config,
                FileAuditRecord {
                    time: Utc::now(),
                    tool: "file.edit".to_string(),
                    action: "apply-patch".to_string(),
                    batch_id: None,
                    group_id: None,
                    operation_index: None,
                    operation_id: None,
                    path: change["resolvedPath"].as_str().unwrap_or("").to_string(),
                    mode: change["action"].as_str().map(str::to_string),
                    requested_confirmation: need_confirm,
                    confirmation_result,
                    before_revision,
                    after_revision,
                    outcome: outcome.to_string(),
                    error_code,
                    duration_ms,
                    replacement_count: None,
                    changed_lines,
                    committed: Some(matches!(
                        outcome,
                        "created" | "updated" | "deleted" | "moved"
                    )),
                },
            )
            .is_err()
            {
                ok = false;
            }
        }
    }
    if ok {
        "written"
    } else {
        "failed"
    }
}

fn write_patch_failure_audit(
    config: &Config,
    error_code: &str,
    need_confirm: bool,
    duration_ms: u128,
) -> &'static str {
    let result = write_file_audit(
        config,
        FileAuditRecord {
            time: Utc::now(),
            tool: "file.edit".to_string(),
            action: "apply-patch".to_string(),
            batch_id: None,
            group_id: None,
            operation_index: None,
            operation_id: None,
            path: "<patch>".to_string(),
            mode: None,
            requested_confirmation: need_confirm,
            confirmation_result: None,
            before_revision: None,
            after_revision: None,
            outcome: "failed".to_string(),
            error_code: Some(error_code.to_string()),
            duration_ms,
            replacement_count: None,
            changed_lines: None,
            committed: Some(false),
        },
    );
    if result.is_ok() {
        "written"
    } else {
        "failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, PathPolicyConfig};

    fn config(root: &Path) -> Config {
        let mut config = Config::default_config().unwrap();
        config.workspace_root = root.to_path_buf();
        config.path_policy = PathPolicyConfig {
            write_roots: vec![root.to_path_buf()],
            read_only_roots: Vec::new(),
            deny_roots: Vec::new(),
        };
        config
    }

    #[test]
    fn reads_metadata_without_exposing_revision_and_preserves_newlines() {
        let root =
            std::env::temp_dir().join(format!("file-read-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("sample.txt");
        fs::write(&path, "one\r\ntwo\n").unwrap();
        let resolved = resolve_path(&config(&root), "sample.txt", Access::Read).unwrap();
        let value = read(&resolved, true, None, None).unwrap();
        assert_eq!(value["content"], "one\r\ntwo\n");
        assert_eq!(value["totalLines"], 2);
        assert!(value.get("revision").is_none());
    }

    #[test]
    fn ranges_are_bounded_and_utf8_safe() {
        let root =
            std::env::temp_dir().join(format!("file-range-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("sample.txt"), "a\nβ\nccc\n").unwrap();
        let resolved = resolve_path(&config(&root), "sample.txt", Access::Read).unwrap();
        let value = read(&resolved, true, Some(2), Some(2)).unwrap();
        assert_eq!(value["content"], "β\n");
        assert_eq!(value["startLine"], 2);
        assert_eq!(value["returnedThroughLine"], 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn deny_and_readonly_policy_precedence_is_enforced() {
        let root =
            std::env::temp_dir().join(format!("file-policy-{}", uuid::Uuid::new_v4().simple()));
        let workspace = root.join("workspace");
        let read_only = root.join("readonly");
        let denied = root.join("denied");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&read_only).unwrap();
        fs::create_dir_all(&denied).unwrap();
        fs::write(read_only.join("a.txt"), "a").unwrap();
        fs::write(denied.join("a.txt"), "a").unwrap();
        let mut config = config(&workspace);
        config.path_policy.read_only_roots = vec![read_only.clone()];
        config.path_policy.deny_roots = vec![denied.clone()];
        let read_resolved = resolve_path(
            &config,
            &read_only.join("a.txt").to_string_lossy(),
            Access::Read,
        )
        .unwrap();
        assert_eq!(
            read(&read_resolved, false, None, None).unwrap()["type"],
            "file"
        );
        assert_eq!(
            check_policy(&config, &read_resolved.path, Access::Write)
                .unwrap_err()
                .code,
            "path_readonly"
        );
        assert_eq!(
            resolve_path(
                &config,
                &denied.join("a.txt").to_string_lossy(),
                Access::Read,
            )
            .unwrap_err()
            .code,
            "path_denied"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn metadata_mode_describes_binary_and_content_mode_rejects_it() {
        let root =
            std::env::temp_dir().join(format!("file-binary-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("binary.dat"), [0_u8, 159, 146, 150]).unwrap();
        let resolved = resolve_path(&config(&root), "binary.dat", Access::Read).unwrap();
        let metadata = read(&resolved, false, None, None).unwrap();
        assert_eq!(metadata["encoding"], Value::Null);
        assert!(metadata.get("content").is_none());
        assert_eq!(
            read(&resolved, true, None, None).unwrap_err().code,
            "file_not_utf8"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn large_files_keep_metadata_bounded() {
        let root =
            std::env::temp_dir().join(format!("file-large-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("large.txt"),
            vec![b'x'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let resolved = resolve_path(&config(&root), "large.txt", Access::Read).unwrap();
        assert_eq!(
            read(&resolved, false, None, None).unwrap()["sizeBytes"],
            MAX_FILE_BYTES + 1
        );
        assert_eq!(
            read(&resolved, true, None, None).unwrap_err().code,
            "file_too_large"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_reader_never_returns_more_than_the_requested_limit() {
        let root = std::env::temp_dir().join(format!(
            "file-bounded-read-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("sample.txt");
        fs::write(&path, b"12345").unwrap();
        assert!(matches!(
            read_bounded(&path, 4).unwrap(),
            BoundedRead::Exceeded
        ));
        assert!(matches!(
            read_bounded(&path, 5).unwrap(),
            BoundedRead::Complete(bytes) if bytes == b"12345"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_allowed_only_when_the_canonical_target_stays_inside_policy() {
        let root =
            std::env::temp_dir().join(format!("file-symlink-{}", uuid::Uuid::new_v4().simple()));
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(workspace.join("inside.txt"), "inside").unwrap();
        fs::write(outside.join("outside.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(
            workspace.join("inside.txt"),
            workspace.join("inside-link.txt"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            outside.join("outside.txt"),
            workspace.join("escape-link.txt"),
        )
        .unwrap();
        let config = config(&workspace);
        assert!(resolve_path(&config, "inside-link.txt", Access::Read).is_ok());
        assert_eq!(
            resolve_path(&config, "escape-link.txt", Access::Read)
                .unwrap_err()
                .code,
            "path_denied"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_supports_literal_regex_glob_context_and_skip_accounting() {
        let root =
            std::env::temp_dir().join(format!("file-search-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(
            root.join("src/a.rs"),
            "before\nNeedle here\nafter\nNeedle twice\n",
        )
        .unwrap();
        fs::write(root.join("src/b.txt"), "Needle text\n").unwrap();
        fs::write(root.join(".hidden.rs"), "Needle hidden\n").unwrap();
        fs::write(root.join("binary.rs"), [0_u8, 1, 2]).unwrap();
        fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        fs::write(root.join("ignored.rs"), "Needle ignored\n").unwrap();
        let resolved = resolve_path(&config(&root), ".", Access::Read).unwrap();
        let include = vec!["**/*.rs".to_string()];
        let empty = Vec::new();
        let value = search_with_context_limit(
            SearchOptions {
                root: &resolved,
                query: "needle",
                mode: SearchMode::Literal,
                case_sensitive: false,
                include: &include,
                exclude: &empty,
                context_lines: 1,
                max_results: 50,
                hidden: false,
                respect_gitignore: true,
                scan_file_limit: MAX_SEARCH_FILES,
                scan_byte_limit: MAX_SEARCH_BYTES,
            },
            20,
        )
        .unwrap();
        assert_eq!(value["matchCount"], 2);
        assert_eq!(value["matches"][0]["line"], 2);
        assert_eq!(value["matches"][0]["column"], 1);
        assert_eq!(value["matches"][0]["before"][0], "before");
        assert_eq!(value["matches"][0]["after"][0], "after");
        assert_eq!(value["skippedFiles"]["nonUtf8"], 1);

        let regex = search_with_context_limit(
            SearchOptions {
                root: &resolved,
                query: r"Needle \w+",
                mode: SearchMode::Regex,
                case_sensitive: true,
                include: &empty,
                exclude: &empty,
                context_lines: 0,
                max_results: 1,
                hidden: true,
                respect_gitignore: false,
                scan_file_limit: MAX_SEARCH_FILES,
                scan_byte_limit: MAX_SEARCH_BYTES,
            },
            20,
        )
        .unwrap();
        assert_eq!(regex["matchCount"], 1);
        assert_eq!(regex["truncationReason"], "max_results");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_streams_file_byte_and_output_limits_without_overshoot() {
        let root = std::env::temp_dir().join(format!(
            "file-search-bounds-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "needle\n").unwrap();
        fs::write(root.join("b.txt"), "needle\n").unwrap();
        let resolved = resolve_path(&config(&root), ".", Access::Read).unwrap();
        let empty = Vec::new();
        let files = search_with_context_limit(
            SearchOptions {
                root: &resolved,
                query: "needle",
                mode: SearchMode::Literal,
                case_sensitive: true,
                include: &empty,
                exclude: &empty,
                context_lines: 0,
                max_results: 50,
                hidden: false,
                respect_gitignore: true,
                scan_file_limit: 1,
                scan_byte_limit: MAX_SEARCH_BYTES,
            },
            20,
        )
        .unwrap();
        assert_eq!(files["scannedFiles"], 1);
        assert_eq!(files["truncationReason"], "scan_files");

        let file = resolve_path(&config(&root), "a.txt", Access::Read).unwrap();
        let bytes = search_with_context_limit(
            SearchOptions {
                root: &file,
                query: "needle",
                mode: SearchMode::Literal,
                case_sensitive: true,
                include: &empty,
                exclude: &empty,
                context_lines: 0,
                max_results: 50,
                hidden: false,
                respect_gitignore: true,
                scan_file_limit: MAX_SEARCH_FILES,
                scan_byte_limit: 3,
            },
            20,
        )
        .unwrap();
        assert_eq!(bytes["scannedBytes"], 0);
        assert_eq!(bytes["truncationReason"], "scan_bytes");

        let long_line = format!("needle{}\n", "x".repeat(MAX_LINE_DISPLAY_BYTES));
        fs::write(root.join("large-output.txt"), long_line.repeat(80)).unwrap();
        let output_file = resolve_path(&config(&root), "large-output.txt", Access::Read).unwrap();
        let output = search_with_context_limit(
            SearchOptions {
                root: &output_file,
                query: "needle",
                mode: SearchMode::Literal,
                case_sensitive: true,
                include: &empty,
                exclude: &empty,
                context_lines: 0,
                max_results: MAX_SEARCH_RESULTS,
                hidden: false,
                respect_gitignore: true,
                scan_file_limit: MAX_SEARCH_FILES,
                scan_byte_limit: MAX_SEARCH_BYTES,
            },
            20,
        )
        .unwrap();
        assert_eq!(output["truncationReason"], "output_bytes");
        assert!(serde_json::to_vec(&output["matches"]).unwrap().len() <= MAX_SEARCH_OUTPUT_BYTES);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_rejects_invalid_patterns_and_enforces_bounds() {
        let root = std::env::temp_dir().join(format!(
            "file-search-invalid-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "x\n").unwrap();
        let resolved = resolve_path(&config(&root), ".", Access::Read).unwrap();
        let empty = Vec::new();
        let invalid_regex = search_with_context_limit(
            SearchOptions {
                root: &resolved,
                query: "[",
                mode: SearchMode::Regex,
                case_sensitive: true,
                include: &empty,
                exclude: &empty,
                context_lines: 0,
                max_results: 50,
                hidden: false,
                respect_gitignore: true,
                scan_file_limit: MAX_SEARCH_FILES,
                scan_byte_limit: MAX_SEARCH_BYTES,
            },
            20,
        )
        .unwrap_err();
        assert_eq!(invalid_regex.code, "file_invalid_regex");
        let invalid_glob = search_with_context_limit(
            SearchOptions {
                root: &resolved,
                query: "x",
                mode: SearchMode::Literal,
                case_sensitive: true,
                include: &["[".to_string()],
                exclude: &empty,
                context_lines: 0,
                max_results: 50,
                hidden: false,
                respect_gitignore: true,
                scan_file_limit: MAX_SEARCH_FILES,
                scan_byte_limit: MAX_SEARCH_BYTES,
            },
            20,
        )
        .unwrap_err();
        assert_eq!(invalid_glob.code, "file_invalid_glob");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_diff_preserves_blank_lines_and_emits_disjoint_hunks() {
        let before =
            "top\n\nold-a\nkeep-1\nkeep-2\nkeep-3\nkeep-4\nkeep-5\nkeep-6\nkeep-7\nkeep-8\nold-b\n\nbottom\n";
        let after =
            "top\n\nnew-a\nkeep-1\nkeep-2\nkeep-3\nkeep-4\nkeep-5\nkeep-6\nkeep-7\nkeep-8\nnew-b\n\nbottom\n";
        let (diff, truncated, changed) = bounded_diff(before, after);
        assert!(!truncated);
        assert_eq!(changed, json!({"added": 2, "removed": 2}));
        assert!(diff.contains("-old-a"));
        assert!(diff.contains("+new-a"));
        assert!(diff.contains("-old-b"));
        assert!(diff.contains("+new-b"));
        assert_eq!(diff.matches("@@ -").count(), 2);
    }

    #[test]
    fn bounded_diff_counts_create_delete_crlf_and_final_newline() {
        let (created, truncated, changed) = bounded_diff("", "one\n\ntwo\n");
        assert!(!truncated);
        assert_eq!(changed, json!({"added": 3, "removed": 0}));
        assert!(created.contains("+one"));
        assert!(created.contains("+two"));

        let (deleted, truncated, changed) = bounded_diff("one\n\ntwo\n", "");
        assert!(!truncated);
        assert_eq!(changed, json!({"added": 0, "removed": 3}));
        assert!(deleted.contains("-one"));
        assert!(deleted.contains("-two"));

        let (crlf, truncated, changed) =
            bounded_diff("one\r\nold\r\nthree\r\n", "one\r\nnew\r\nthree\r\n");
        assert!(!truncated);
        assert_eq!(changed, json!({"added": 1, "removed": 1}));
        assert!(crlf.contains("-old\r\n"));
        assert!(crlf.contains("+new\r\n"));

        let (newline, truncated, changed) = bounded_diff("one", "one\n");
        assert!(!truncated);
        assert_eq!(changed, json!({"added": 1, "removed": 1}));
        assert!(newline.contains("No newline at end of file"));

        let (unchanged, truncated, changed) = bounded_diff("same\n", "same\n");
        assert!(!truncated);
        assert_eq!(changed, json!({"added": 0, "removed": 0}));
        assert!(unchanged.is_empty());
    }

    #[test]
    fn bounded_diff_truncates_utf8_after_computing_complete_counts() {
        let before = (0..4_000)
            .map(|index| format!("旧内容-{index:04}-abcdefghijk\n"))
            .collect::<String>();
        let after = (0..4_000)
            .map(|index| format!("新内容-{index:04}-ABCDEFGHIJK\n"))
            .collect::<String>();
        let (diff, truncated, changed) = bounded_diff(&before, &after);
        assert!(truncated);
        assert!(diff.len() <= MAX_DIFF_BYTES);
        assert!(diff.is_char_boundary(diff.len()));
        assert_eq!(changed, json!({"added": 4_000, "removed": 4_000}));
    }

    #[cfg(unix)]
    #[test]
    fn absent_commit_uses_no_replace_and_overwrite_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("file-atomic-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let config = config(&root);
        let absent = resolve_absent_path(&config, "new.txt").unwrap();
        let temp = stage_temp(&absent.path, b"new", None).unwrap();
        fs::write(&absent.requested, b"raced").unwrap();
        assert!(fs::hard_link(&temp, &absent.requested).is_err());
        assert_eq!(fs::read(&absent.requested).unwrap(), b"raced");
        let _ = fs::remove_file(temp);

        let existing = root.join("existing.txt");
        fs::write(&existing, b"old").unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o640)).unwrap();
        let resolved = resolve_path(&config, "existing.txt", Access::Write).unwrap();
        let temp = stage_temp(
            &resolved.path,
            b"new",
            Some(&fs::metadata(&existing).unwrap().permissions()),
        )
        .unwrap();
        fs::rename(temp, &resolved.path).unwrap();
        assert_eq!(
            fs::metadata(&existing).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn apply_patch_parser_handles_add_delete_update_and_move_without_fs_access() {
        let parsed = parse_patch("*** Begin Patch\n*** Add File: add.txt\n+hello\n*** Delete File: gone.txt\n*** Update File: old.txt\n*** Move to: new.txt\n@@\n-old\n+new\n*** End Patch").unwrap();
        assert_eq!(parsed.hunks.len(), 3);
    }

    #[test]
    fn rejects_duplicate_and_ancestor_patch_paths() {
        let duplicate = PathBuf::from("/workspace/file.txt");
        let error = reject_overlapping_paths(&[duplicate.clone(), duplicate]).unwrap_err();
        assert_eq!(error.code, "file_patch_ambiguous_paths");

        let error = reject_overlapping_paths(&[
            PathBuf::from("/workspace/tree"),
            PathBuf::from("/workspace/tree/file.txt"),
        ])
        .unwrap_err();
        assert_eq!(error.code, "file_patch_ambiguous_paths");
    }

    #[test]
    fn move_source_removal_failure_compensates_destination() {
        let root =
            std::env::temp_dir().join(format!("file-move-compensate-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let config = config(&root);
        let source = root.join("source.txt");
        fs::write(&source, "source\n").unwrap();
        let source = resolve_path(&config, "source.txt", Access::Write).unwrap();
        let destination = resolve_absent_path(&config, "destination.txt").unwrap();
        let candidate = b"destination\n".to_vec();
        let temp = stage_temp(&destination.path, &candidate, None).unwrap();
        let mut change = PlannedChange {
            action: "move",
            source: Some(source.clone()),
            target: destination.clone(),
            destination: Some(destination.clone()),
            before_bytes: Some(b"source\n".to_vec()),
            before_revision: Some(revision(b"source\n")),
            before_mode: None,
            candidate: Some(candidate),
            after_revision: None,
            diff: String::new(),
            diff_truncated: false,
            changed_lines: json!({"added": 0, "removed": 0}),
            temp: Some(temp),
        };
        inject_move_source_remove_failure(&source.path);

        let error = commit_move(&mut change).unwrap_err();
        assert_eq!(error.code, "file_write_failed");
        assert!(source.path.is_file());
        assert!(!destination.requested.exists());
        let _ = fs::remove_dir_all(root);
    }
}
