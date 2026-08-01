use std::collections::{HashMap, HashSet};
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

use crate::audit::{
    write_batch_audit, write_file_audit, BatchAuditRecord, ChangedLines, FileAuditRecord,
};
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

#[cfg(test)]
static INJECT_BATCH_STAGE_FAILURE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
pub(crate) fn inject_batch_stage_failure(target: &Path) {
    *INJECT_BATCH_STAGE_FAILURE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        Some(fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf()));
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
    let revision = revision(&bytes);
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
    result["revision"] = json!(revision);
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

#[allow(dead_code)]
pub(crate) fn search(options: SearchOptions<'_>) -> std::result::Result<Value, FileError> {
    search_with_context_limit(
        options,
        crate::config::DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES,
    )
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
    let mut skipped = json!({
        "tooLarge": 0,
        "nonUtf8": 0,
        "symlink": 0,
        "unreadable": 0,
    });
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
        let file_type = entry.file_type();
        if file_type.is_some_and(|kind| kind.is_symlink()) {
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
        let read_limit = remaining_bytes.min(MAX_FILE_BYTES);
        let bytes = match read_bounded(path, read_limit) {
            Ok(BoundedRead::Complete(bytes)) => bytes,
            Ok(BoundedRead::Exceeded) if read_limit < MAX_FILE_BYTES => {
                truncated = true;
                truncation_reason = Some("scan_bytes");
                break;
            }
            Ok(BoundedRead::Exceeded) => {
                increment_skip(&mut skipped, "tooLarge");
                continue;
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
    let match_count = matches.len();
    Ok(json!({
        "query": options.query,
        "mode": match options.mode { SearchMode::Literal => "literal", SearchMode::Regex => "regex" },
        "requestedContextLines": options.context_lines,
        "effectiveContextLines": effective_context_lines,
        "contextLinesClipped": context_lines_clipped,
        "warnings": warnings,
        "matches": matches,
        "matchCount": match_count,
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
        let glob = Glob::new(pattern)
            .map_err(|_| FileError::new("file_invalid_glob", "invalid glob pattern"))?;
        builder.add(glob);
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
        let selected = line_number >= start && line_number <= end;
        if selected {
            let available = MAX_READ_OUTPUT_BYTES.saturating_sub(output.len());
            if line.len() <= available {
                output.push_str(line);
                returned_through = Some(line_number);
            } else {
                let prefix = utf8_prefix(line, available);
                output.push_str(prefix);
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditMode {
    Replace,
    Patch,
    Write,
}

#[derive(Clone, Debug)]
pub(crate) struct EditRequest {
    pub(crate) mode: EditMode,
    pub(crate) path: String,
    pub(crate) expected_revision: Option<String>,
    pub(crate) expected_absent: Option<bool>,
    pub(crate) old_text: Option<String>,
    pub(crate) new_text: Option<String>,
    pub(crate) expected_matches: Option<usize>,
    pub(crate) patch: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) dry_run: bool,
    pub(crate) need_confirm: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct BatchOperation {
    pub(crate) id: Option<String>,
    pub(crate) kind: String,
    pub(crate) path: String,
    pub(crate) include_content: bool,
    pub(crate) start_line: Option<usize>,
    pub(crate) end_line: Option<usize>,
    pub(crate) query: Option<String>,
    pub(crate) search_mode: Option<String>,
    pub(crate) case_sensitive: bool,
    pub(crate) include: Vec<String>,
    pub(crate) exclude: Vec<String>,
    pub(crate) context_lines: usize,
    pub(crate) max_results: usize,
    pub(crate) hidden: bool,
    pub(crate) respect_gitignore: bool,
    pub(crate) edit_mode: Option<String>,
    pub(crate) expected_revision: Option<String>,
    pub(crate) expected_absent: Option<bool>,
    pub(crate) old_text: Option<String>,
    pub(crate) new_text: Option<String>,
    pub(crate) expected_matches: Option<usize>,
    pub(crate) patch: Option<String>,
    pub(crate) content: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BatchRequest {
    pub(crate) operations: Vec<BatchOperation>,
    pub(crate) dry_run: bool,
    pub(crate) need_confirm: bool,
}

pub(crate) async fn edit(state: &AppState, request: EditRequest) -> Value {
    let started = std::time::Instant::now();
    let config = state.config.read().await.clone();
    let mut value = match edit_inner(state, &config, &request).await {
        Ok(value) => value,
        Err(error) => error.value(),
    };
    let error_code = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let resolved_path = value
        .get("resolvedPath")
        .and_then(Value::as_str)
        .unwrap_or(&request.path)
        .to_string();
    let before_revision = value
        .get("beforeRevision")
        .and_then(Value::as_str)
        .map(str::to_string);
    let after_revision = value
        .get("afterRevision")
        .and_then(Value::as_str)
        .map(str::to_string);
    let confirmation_result = value
        .get("confirmation")
        .and_then(|confirmation| confirmation.get("result"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let replacement_count = value
        .get("replacementCount")
        .and_then(Value::as_u64)
        .map(|count| count as usize);
    let changed_lines = value.get("changedLines").and_then(|lines| {
        Some(crate::audit::ChangedLines {
            added: lines.get("added")?.as_u64()? as usize,
            removed: lines.get("removed")?.as_u64()? as usize,
        })
    });
    let outcome = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed")
        .to_string();
    let committed = Some(!request.dry_run && matches!(outcome.as_str(), "created" | "updated"));
    let record = crate::audit::FileAuditRecord {
        time: Utc::now(),
        tool: "file.edit".to_string(),
        action: "edit".to_string(),
        batch_id: None,
        group_id: None,
        operation_index: None,
        operation_id: None,
        path: resolved_path,
        mode: Some(edit_mode_label(request.mode).to_string()),
        requested_confirmation: request.need_confirm,
        confirmation_result,
        before_revision,
        after_revision,
        outcome,
        error_code,
        duration_ms: started.elapsed().as_millis(),
        replacement_count,
        changed_lines,
        committed,
    };
    if request.dry_run {
        return value;
    }
    let audit_status = match crate::audit::write_file_audit(&config, record) {
        Ok(()) => "written",
        Err(_) => "failed",
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("auditStatus".to_string(), json!(audit_status));
    }
    value
}

async fn edit_inner(
    state: &AppState,
    config: &Config,
    request: &EditRequest,
) -> std::result::Result<Value, FileError> {
    let (target, existed) = match resolve_path(config, &request.path, Access::Write) {
        Ok(target) => (target, true),
        Err(error) if error.code == "file_not_found" => {
            let target = resolve_absent_path(config, &request.path)?;
            (target, false)
        }
        Err(error) => return Err(error),
    };
    let _lock = lock_target(state, &target.path).await;
    revalidate_target(config, &target, existed)?;
    if existed && request.expected_absent == Some(true) {
        return Err(FileError::new(
            "file_already_exists",
            "target already exists",
        ));
    }
    let (before_bytes, before_text, before_revision, before_mode) = if existed {
        let metadata = fs::metadata(&target.path)
            .map_err(|_| FileError::new("file_not_found", "target was not found"))?;
        if !metadata.is_file() {
            return Err(FileError::new(
                "file_not_regular",
                "target is not a regular file",
            ));
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(FileError::new(
                "file_too_large",
                "file exceeds the 8 MiB edit bound",
            ));
        }
        let bytes = match read_bounded(&target.path, MAX_FILE_BYTES)
            .map_err(|_| FileError::new("file_read_failed", "target could not be read"))?
        {
            BoundedRead::Complete(bytes) => bytes,
            BoundedRead::Exceeded => {
                return Err(FileError::new(
                    "file_too_large",
                    "file exceeds the 8 MiB edit bound",
                ))
            }
        };
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| FileError::new("file_not_utf8", "file content is not UTF-8 text"))?;
        let revision = revision(&bytes);
        validate_expected_revision(request.expected_revision.as_deref(), &revision)?;
        let mode = fs::metadata(&target.path)
            .ok()
            .map(|meta| meta.permissions());
        (Some(bytes), Some(text), Some(revision), mode)
    } else {
        if request.mode != EditMode::Write || request.expected_absent != Some(true) {
            return Err(FileError::new(
                "file_revision_required",
                "new files require expectedAbsent: true",
            ));
        }
        if request.expected_revision.is_some() {
            return Err(FileError::new(
                "file_revision_invalid",
                "expectedAbsent and expectedRevision are mutually exclusive",
            ));
        }
        (None, None, None, None)
    };
    let mut replacement_count = 0usize;
    let candidate = match request.mode {
        EditMode::Replace => {
            let old_text = request.old_text.as_deref().ok_or_else(|| {
                FileError::new("file_match_count_mismatch", "oldText is required")
            })?;
            if old_text.is_empty() {
                return Err(FileError::new(
                    "file_match_count_mismatch",
                    "oldText must not be empty",
                ));
            }
            let new_text = request.new_text.as_deref().unwrap_or_default();
            let source = before_text.as_deref().unwrap_or_default();
            let matches = source.match_indices(old_text).count();
            let expected = request.expected_matches.unwrap_or(1);
            if expected == 0 {
                return Err(FileError::new(
                    "file_match_count_mismatch",
                    "expectedMatches must be positive",
                ));
            }
            if matches != expected {
                return Err(FileError::new(
                    "file_match_count_mismatch",
                    &format!("expected {expected} matches but found {matches}"),
                ));
            }
            replacement_count = matches;
            source.replace(old_text, new_text)
        }
        EditMode::Patch => {
            let patch = request
                .patch
                .as_deref()
                .ok_or_else(|| FileError::new("file_patch_invalid", "patch is required"))?;
            apply_unified_patch(
                before_text.as_deref().ok_or_else(|| {
                    FileError::new("file_patch_invalid", "patch requires an existing file")
                })?,
                patch,
                &target.path,
            )?
        }
        EditMode::Write => {
            let content = request
                .content
                .as_deref()
                .ok_or_else(|| FileError::new("file_write_failed", "content is required"))?;
            if content.len() > MAX_FILE_BYTES as usize {
                return Err(FileError::new(
                    "file_too_large",
                    "candidate exceeds the 8 MiB edit bound",
                ));
            }
            content.to_string()
        }
    };
    let candidate_bytes = candidate.as_bytes();
    if candidate_bytes.len() > MAX_FILE_BYTES as usize {
        return Err(FileError::new(
            "file_too_large",
            "candidate exceeds the 8 MiB edit bound",
        ));
    }
    let after_revision = revision(candidate_bytes);
    let before_size = before_bytes.as_ref().map_or(0, Vec::len);
    let unchanged = before_bytes.as_deref() == Some(candidate_bytes);
    let (diff, diff_truncated, changed_lines) =
        bounded_diff(before_text.as_deref().unwrap_or(""), &candidate);
    let mut response = json!({
        "path": request.path,
        "resolvedPath": target.path.to_string_lossy(),
        "mode": edit_mode_label(request.mode),
        "status": if unchanged { "unchanged" } else if request.dry_run { "dry-run" } else if existed { "updated" } else { "created" },
        "beforeRevision": before_revision,
        "afterRevision": after_revision,
        "beforeSizeBytes": before_size,
        "afterSizeBytes": candidate_bytes.len(),
        "replacementCount": replacement_count,
        "diff": diff,
        "diffTruncated": diff_truncated,
        "changedLines": changed_lines,
    });
    if unchanged || request.dry_run {
        return Ok(response);
    }
    if request.need_confirm {
        let confirmation = crate::confirmation::request_confirmation(
            state,
            config,
            None,
            "file.edit",
            &[
                request.path.clone(),
                edit_mode_label(request.mode).to_string(),
            ],
        )
        .await;
        response["confirmation"] = json!({"requested": true, "result": confirmation});
        if confirmation != "allow_once" {
            let code = if matches!(
                confirmation.as_str(),
                "provider_unavailable" | "confirmation_provider_unavailable"
            ) {
                "file_confirmation_unavailable"
            } else {
                "file_confirmation_denied"
            };
            return Err(FileError::new(code, "file mutation was not confirmed"));
        }
    }
    revalidate_target(config, &target, existed)?;
    if existed {
        let current = match read_bounded(&target.path, MAX_FILE_BYTES)
            .map_err(|_| FileError::new("file_revision_conflict", "target changed before commit"))?
        {
            BoundedRead::Complete(bytes) => bytes,
            BoundedRead::Exceeded => {
                return Err(FileError::new(
                    "file_revision_conflict",
                    "target changed before commit",
                ))
            }
        };
        if Some(revision(&current)) != before_revision {
            return Err(FileError::new(
                "file_revision_conflict",
                "target changed before commit",
            ));
        }
    } else if fs::symlink_metadata(&target.requested).is_ok() {
        return Err(FileError::new(
            "file_already_exists",
            "target appeared before commit",
        ));
    }
    let temp = stage_temp(&target.path, candidate_bytes, before_mode.as_ref())?;
    let commit_result = if existed {
        fs::rename(&temp, &target.path)
    } else {
        match fs::hard_link(&temp, &target.requested) {
            Ok(()) => {
                let _ = fs::remove_file(&temp);
                Ok(())
            }
            Err(error) => Err(error),
        }
    };
    if let Err(error) = commit_result {
        let _ = fs::remove_file(&temp);
        return Err(FileError::new(
            "file_write_failed",
            &format!("atomic file commit failed: {error}"),
        ));
    }
    sync_parent(&target.path);
    response["status"] = json!(if existed { "updated" } else { "created" });
    response["confirmation"] = response
        .get("confirmation")
        .cloned()
        .unwrap_or_else(|| json!({"requested": false, "result": null}));
    Ok(response)
}

const MAX_BATCH_OPERATIONS: usize = 32;
const MAX_BATCH_EDITS: usize = 16;
const MAX_BATCH_REQUEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_BATCH_ORIGINAL_BYTES: usize = 32 * 1024 * 1024;
const MAX_BATCH_CANDIDATE_BYTES: usize = 32 * 1024 * 1024;
const MAX_BATCH_SCAN_FILES: usize = 20_000;
const MAX_BATCH_SCAN_BYTES: u64 = 128 * 1024 * 1024;
const MAX_BATCH_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
struct BatchSearchBudget {
    file_limit: usize,
    byte_limit: u64,
    aggregate_file_limited: bool,
    aggregate_byte_limited: bool,
}

fn remaining_batch_search_budget(scan_files: usize, scan_bytes: u64) -> BatchSearchBudget {
    let remaining_files = MAX_BATCH_SCAN_FILES.saturating_sub(scan_files);
    let remaining_bytes = MAX_BATCH_SCAN_BYTES.saturating_sub(scan_bytes);
    BatchSearchBudget {
        file_limit: remaining_files.min(MAX_SEARCH_FILES),
        byte_limit: remaining_bytes.min(MAX_SEARCH_BYTES),
        aggregate_file_limited: remaining_files < MAX_SEARCH_FILES,
        aggregate_byte_limited: remaining_bytes < MAX_SEARCH_BYTES,
    }
}

fn aggregate_search_limit_hit(value: &Value, budget: BatchSearchBudget) -> bool {
    match value["truncationReason"].as_str() {
        Some("scan_files") => budget.aggregate_file_limited,
        Some("scan_bytes") => budget.aggregate_byte_limited,
        _ => false,
    }
}

struct BatchFileGroup {
    group_id: String,
    target: ResolvedPath,
    existed: bool,
    before_bytes: Option<Vec<u8>>,
    before_revision: Option<String>,
    before_mode: Option<fs::Permissions>,
    candidate: Option<Vec<u8>>,
    candidate_revision: Option<String>,
    operation_indices: Vec<usize>,
}

struct BatchPreparedGroup {
    group_id: String,
    target: ResolvedPath,
    existed: bool,
    before_revision: Option<String>,
    before_size_bytes: usize,
    before_mode: Option<fs::Permissions>,
    candidate: Vec<u8>,
    after_revision: String,
    operation_indices: Vec<usize>,
    temp: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct BatchGroupRecord {
    group_id: String,
    target: String,
    operation_indices: Vec<usize>,
    base_revision: Option<String>,
    final_revision: Option<String>,
    status: String,
    committed: bool,
    failure_count: usize,
    error_code: Option<String>,
}

impl BatchGroupRecord {
    fn from_group(group: &BatchFileGroup) -> Self {
        Self {
            group_id: group.group_id.clone(),
            target: group.target.path.to_string_lossy().to_string(),
            operation_indices: group.operation_indices.clone(),
            base_revision: None,
            final_revision: None,
            status: "pending".to_string(),
            committed: false,
            failure_count: 0,
            error_code: None,
        }
    }

    fn synthetic(index: usize, operation: &BatchOperation) -> Self {
        Self {
            group_id: format!("file_group_{index}"),
            target: operation.path.clone(),
            operation_indices: vec![index],
            base_revision: None,
            final_revision: None,
            status: "failed".to_string(),
            committed: false,
            failure_count: 1,
            error_code: None,
        }
    }

    fn refresh_failure_count(&mut self, results: &[Value]) {
        self.failure_count = self
            .operation_indices
            .iter()
            .filter(|index| {
                matches!(
                    results[**index]["status"].as_str(),
                    Some("failed" | "skipped")
                )
            })
            .count();
    }

    fn value(&self, operations: &[BatchOperation]) -> Value {
        let operation_ids = self
            .operation_indices
            .iter()
            .filter_map(|index| operations[*index].id.clone())
            .collect::<Vec<_>>();
        let mut value = json!({
            "groupId": self.group_id,
            "target": self.target,
            "operationIndexes": self.operation_indices,
            "operationIds": operation_ids,
            "baseRevision": self.base_revision,
            "finalRevision": self.final_revision,
            "status": self.status,
            "committed": self.committed,
            "failureCount": self.failure_count,
        });
        if let Some(error_code) = &self.error_code {
            value["errorCode"] = json!(error_code);
        }
        value
    }
}

fn batch_error(code: &str, message: &str) -> Value {
    FileError::new(code, message).value()
}

fn operation_envelope(
    index: usize,
    operation: &BatchOperation,
    status: &str,
    result: Value,
) -> Value {
    let mut envelope = json!({
        "index": index,
        "type": operation.kind,
        "status": status,
        "result": result,
    });
    if let Some(id) = operation.id.as_deref() {
        envelope["id"] = json!(id);
    }
    envelope
}

fn operation_error_envelope(
    index: usize,
    operation: &BatchOperation,
    status: &str,
    error: Value,
) -> Value {
    let mut envelope = json!({
        "index": index,
        "type": operation.kind,
        "status": status,
        "error": error.get("error").cloned().unwrap_or(error),
    });
    if let Some(id) = operation.id.as_deref() {
        envelope["id"] = json!(id);
    }
    envelope
}

fn batch_edit_request(operation: &BatchOperation) -> std::result::Result<EditRequest, FileError> {
    let mode = match operation.edit_mode.as_deref() {
        Some("replace") => EditMode::Replace,
        Some("patch") => EditMode::Patch,
        Some("write") => EditMode::Write,
        _ => {
            return Err(FileError::new(
                "file_invalid_mode",
                "edit mode must be replace, patch, or write",
            ))
        }
    };
    Ok(EditRequest {
        mode,
        path: operation.path.clone(),
        expected_revision: operation.expected_revision.clone(),
        expected_absent: operation.expected_absent,
        old_text: operation.old_text.clone(),
        new_text: operation.new_text.clone(),
        expected_matches: operation.expected_matches,
        patch: operation.patch.clone(),
        content: operation.content.clone(),
        dry_run: false,
        need_confirm: false,
    })
}

fn batch_request_text_bytes(operation: &BatchOperation) -> usize {
    operation.query.as_deref().unwrap_or("").len()
        + operation.include.iter().map(String::len).sum::<usize>()
        + operation.exclude.iter().map(String::len).sum::<usize>()
        + operation.old_text.as_deref().unwrap_or("").len()
        + operation.new_text.as_deref().unwrap_or("").len()
        + operation.patch.as_deref().unwrap_or("").len()
        + operation.content.as_deref().unwrap_or("").len()
}

fn valid_batch_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn load_batch_group_base(group: &mut BatchFileGroup) -> std::result::Result<(), FileError> {
    if !group.existed {
        group.before_bytes = None;
        group.before_revision = None;
        group.before_mode = None;
        group.candidate = None;
        group.candidate_revision = None;
        return Ok(());
    }
    let metadata = fs::metadata(&group.target.path)
        .map_err(|_| FileError::new("file_not_found", "target was not found"))?;
    if !metadata.is_file() {
        return Err(FileError::new(
            "file_not_regular",
            "target is not a regular file",
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(FileError::new(
            "file_too_large",
            "file exceeds the 8 MiB edit bound",
        ));
    }
    let bytes = match read_bounded(&group.target.path, MAX_FILE_BYTES)
        .map_err(|_| FileError::new("file_read_failed", "target could not be read"))?
    {
        BoundedRead::Complete(bytes) => bytes,
        BoundedRead::Exceeded => {
            return Err(FileError::new(
                "file_too_large",
                "file exceeds the 8 MiB edit bound",
            ))
        }
    };
    String::from_utf8(bytes.clone())
        .map_err(|_| FileError::new("file_not_utf8", "file content is not UTF-8 text"))?;
    let base_revision = revision(&bytes);
    group.before_bytes = Some(bytes.clone());
    group.before_revision = Some(base_revision.clone());
    group.before_mode = Some(metadata.permissions());
    group.candidate = Some(bytes);
    group.candidate_revision = Some(base_revision);
    Ok(())
}

fn validate_batch_group_guards(
    group: &BatchFileGroup,
    operations: &[BatchOperation],
) -> std::result::Result<(), FileError> {
    if group.existed {
        let mut expected_revision = None::<&str>;
        for index in &group.operation_indices {
            let operation = &operations[*index];
            if operation.expected_absent == Some(true) {
                return Err(FileError::new(
                    "file_already_exists",
                    "target already exists",
                ));
            }
            if let Some(value) = operation.expected_revision.as_deref() {
                if !is_revision(value) {
                    return Err(FileError::new(
                        "file_revision_invalid",
                        "expectedRevision is invalid",
                    ));
                }
                if expected_revision.is_some_and(|expected| expected != value) {
                    return Err(FileError::new(
                        "file_batch_guard_conflict",
                        "same-file edit guards must reference one base revision",
                    ));
                }
                expected_revision = Some(value);
            }
        }
        let expected_revision = expected_revision.ok_or_else(|| {
            FileError::new(
                "file_revision_required",
                "existing-file mutations require expectedRevision",
            )
        })?;
        let actual_revision = group.before_revision.as_deref().ok_or_else(|| {
            FileError::new("file_revision_conflict", "target revision is unavailable")
        })?;
        return validate_expected_revision(Some(expected_revision), actual_revision);
    }

    let mut expected_absent = None::<bool>;
    for index in &group.operation_indices {
        let operation = &operations[*index];
        if operation.expected_revision.is_some() {
            return Err(FileError::new(
                "file_revision_invalid",
                "expectedAbsent and expectedRevision are mutually exclusive",
            ));
        }
        if let Some(value) = operation.expected_absent {
            if expected_absent.is_some_and(|expected| expected != value) {
                return Err(FileError::new(
                    "file_batch_guard_conflict",
                    "same-file create guards must agree on expectedAbsent",
                ));
            }
            expected_absent = Some(value);
        }
    }
    if expected_absent != Some(true) {
        return Err(FileError::new(
            "file_revision_required",
            "new files require expectedAbsent: true",
        ));
    }
    let first = batch_edit_request(&operations[group.operation_indices[0]])?;
    if first.mode != EditMode::Write {
        return Err(FileError::new(
            "file_revision_required",
            "new file groups must begin with a write operation",
        ));
    }
    Ok(())
}

fn apply_batch_operation(
    group: &mut BatchFileGroup,
    operation: &BatchOperation,
) -> std::result::Result<Value, FileError> {
    let request = batch_edit_request(operation)?;
    let before_bytes = group.candidate.as_deref();
    let before_text = before_bytes
        .map(|bytes| String::from_utf8(bytes.to_vec()))
        .transpose()
        .map_err(|_| FileError::new("file_not_utf8", "candidate is not UTF-8 text"))?;
    let before_revision = group.candidate_revision.clone();
    let mut replacement_count = 0usize;
    let candidate = match request.mode {
        EditMode::Replace => {
            let old_text = request.old_text.as_deref().ok_or_else(|| {
                FileError::new("file_match_count_mismatch", "oldText is required")
            })?;
            if old_text.is_empty() {
                return Err(FileError::new(
                    "file_match_count_mismatch",
                    "oldText must not be empty",
                ));
            }
            let source = before_text.as_deref().ok_or_else(|| {
                FileError::new(
                    "file_match_count_mismatch",
                    "replace requires an existing file",
                )
            })?;
            let matches = source.match_indices(old_text).count();
            let expected = request.expected_matches.unwrap_or(1);
            if expected == 0 || matches != expected {
                return Err(FileError::new(
                    "file_match_count_mismatch",
                    &format!("expected {expected} matches but found {matches}"),
                ));
            }
            replacement_count = matches;
            source.replace(old_text, request.new_text.as_deref().unwrap_or_default())
        }
        EditMode::Patch => apply_unified_patch(
            before_text.as_deref().ok_or_else(|| {
                FileError::new("file_patch_invalid", "patch requires an existing file")
            })?,
            request
                .patch
                .as_deref()
                .ok_or_else(|| FileError::new("file_patch_invalid", "patch is required"))?,
            &group.target.path,
        )?,
        EditMode::Write => request
            .content
            .as_deref()
            .ok_or_else(|| FileError::new("file_write_failed", "content is required"))?
            .to_string(),
    };
    let candidate = candidate.into_bytes();
    if candidate.len() > MAX_FILE_BYTES as usize {
        return Err(FileError::new(
            "file_too_large",
            "candidate exceeds the 8 MiB edit bound",
        ));
    }
    let after_revision = revision(&candidate);
    let unchanged = before_bytes == Some(candidate.as_slice());
    let (diff, diff_truncated, changed_lines) = bounded_diff(
        before_text.as_deref().unwrap_or(""),
        std::str::from_utf8(&candidate).unwrap_or(""),
    );
    let response = json!({
        "path": request.path,
        "resolvedPath": group.target.path.to_string_lossy(),
        "groupId": group.group_id,
        "mode": edit_mode_label(request.mode),
        "status": if unchanged { "unchanged" } else if before_bytes.is_some() { "updated" } else { "created" },
        "beforeRevision": before_revision,
        "afterRevision": after_revision,
        "beforeSizeBytes": before_bytes.map_or(0, |bytes| bytes.len()),
        "afterSizeBytes": candidate.len(),
        "replacementCount": replacement_count,
        "diff": diff,
        "diffTruncated": diff_truncated,
        "changedLines": changed_lines,
    });
    group.candidate = Some(candidate);
    group.candidate_revision = response["afterRevision"].as_str().map(str::to_string);
    Ok(response)
}

fn mark_group_failure(
    results: &mut [Value],
    operations: &[BatchOperation],
    indices: &[usize],
    group_id: &str,
    failed_index: usize,
    error: Value,
    skipped: (&str, &str),
) {
    for index in indices {
        if *index == failed_index {
            results[*index] = annotate_group_result(
                operation_error_envelope(*index, &operations[*index], "failed", error.clone()),
                group_id,
                false,
            );
        } else {
            results[*index] = annotate_group_result(
                operation_error_envelope(
                    *index,
                    &operations[*index],
                    "skipped",
                    batch_error(skipped.0, skipped.1),
                ),
                group_id,
                false,
            );
        }
    }
}

fn mark_group_all_failure(
    results: &mut [Value],
    operations: &[BatchOperation],
    indices: &[usize],
    group_id: &str,
    code: &str,
    message: &str,
) {
    for index in indices {
        results[*index] = annotate_group_result(
            operation_error_envelope(
                *index,
                &operations[*index],
                "failed",
                batch_error(code, message),
            ),
            group_id,
            false,
        );
    }
}

fn annotate_group_result(mut result: Value, group_id: &str, committed: bool) -> Value {
    result["groupId"] = json!(group_id);
    result["committed"] = json!(committed);
    if let Some(payload) = result.get_mut("result").and_then(Value::as_object_mut) {
        payload.insert("groupId".to_string(), json!(group_id));
        payload.insert("committed".to_string(), json!(committed));
    }
    result
}

fn annotate_group_operations(
    results: &mut [Value],
    indices: &[usize],
    group_id: &str,
    committed: bool,
) {
    for index in indices {
        if !results[*index].is_null() {
            results[*index] = annotate_group_result(results[*index].take(), group_id, committed);
        }
    }
}

fn prepared_group_changed(group: &BatchPreparedGroup) -> bool {
    if group.existed {
        group.before_revision.as_deref() != Some(group.after_revision.as_str())
    } else {
        true
    }
}

fn attach_batch_group_evidence(
    response: &mut Value,
    group_records: &mut [BatchGroupRecord],
    operations: &[BatchOperation],
    results: &[Value],
) {
    for record in group_records.iter_mut() {
        record.refresh_failure_count(results);
    }
    group_records.sort_by_key(|record| {
        record
            .operation_indices
            .first()
            .copied()
            .unwrap_or(usize::MAX)
    });
    let total = group_records.len();
    let committed = group_records
        .iter()
        .filter(|record| record.committed)
        .count();
    let failed = group_records
        .iter()
        .filter(|record| record.status == "failed")
        .count();
    let unchanged = group_records
        .iter()
        .filter(|record| record.status == "unchanged")
        .count();
    let dry_run = group_records
        .iter()
        .filter(|record| record.status == "dry-run")
        .count();
    let failure_count = results
        .iter()
        .filter(|result| matches!(result["status"].as_str(), Some("failed" | "skipped")))
        .count();
    response["groups"] = Value::Array(
        group_records
            .iter()
            .map(|record| record.value(operations))
            .collect(),
    );
    response["groupCounts"] = json!({
        "total": total,
        "committed": committed,
        "failed": failed,
        "unchanged": unchanged,
        "dryRun": dry_run,
    });
    response["failureCount"] = json!(failure_count);
    response["results"] = Value::Array(results.to_vec());
}

pub(crate) async fn batch(state: &AppState, request: BatchRequest) -> Value {
    let started = std::time::Instant::now();
    let config = state.config.read().await.clone();
    let batch_id = format!("file_batch_{}", uuid::Uuid::new_v4().simple());
    let mut response = json!({
        "batchId": batch_id,
        "status": "rejected",
        "startedAt": Utc::now(),
        "updatedAt": Utc::now(),
        "operationCount": request.operations.len(),
        "editCount": 0,
        "effectiveMutationCount": 0,
        "confirmation": {"requested": false, "result": Value::Null},
        "results": [],
        "groups": [],
        "groupCounts": {
            "total": 0,
            "committed": 0,
            "failed": 0,
            "unchanged": 0,
        },
        "failureCount": 0,
        "truncated": false,
        "truncationReason": Value::Null,
    });
    if request.operations.is_empty() {
        response["error"] =
            batch_error("file_batch_empty", "operations must not be empty")["error"].clone();
        return finalize_batch(response, &config, started, &request);
    }
    if request.operations.len() > MAX_BATCH_OPERATIONS {
        response["error"] = batch_error(
            "file_batch_too_many_operations",
            "operations exceed the 32-entry bound",
        )["error"]
            .clone();
        return finalize_batch(response, &config, started, &request);
    }
    let mut ids = HashSet::new();
    let mut edit_count = 0usize;
    let mut request_bytes = 0usize;
    let mut validation_errors = vec![None::<Value>; request.operations.len()];
    for (index, operation) in request.operations.iter().enumerate() {
        if let Some(id) = operation.id.as_deref() {
            if !valid_batch_id(id) {
                validation_errors[index] = Some(batch_error(
                    "file_batch_invalid_id",
                    "operation id is invalid",
                ));
            } else if !ids.insert(id.to_string()) {
                validation_errors[index] = Some(batch_error(
                    "file_batch_duplicate_id",
                    "operation ids must be unique",
                ));
            }
        }
        if !matches!(operation.kind.as_str(), "read" | "search" | "edit") {
            validation_errors[index] = Some(batch_error(
                "file_batch_invalid_type",
                "operation type must be read, search, or edit",
            ));
        }
        if operation.kind == "edit" {
            edit_count += 1;
            if operation.edit_mode.is_none() {
                validation_errors[index] =
                    Some(batch_error("file_invalid_mode", "edit mode is required"));
            }
        }
        request_bytes = request_bytes.saturating_add(batch_request_text_bytes(operation));
    }
    if edit_count > MAX_BATCH_EDITS {
        response["error"] = batch_error(
            "file_batch_too_many_edits",
            "edits exceed the 16-entry bound",
        )["error"]
            .clone();
        return finalize_batch(response, &config, started, &request);
    }
    if request_bytes > MAX_BATCH_REQUEST_BYTES {
        response["error"] = batch_error(
            "file_batch_request_too_large",
            "aggregate request text exceeds the 16 MiB bound",
        )["error"]
            .clone();
        return finalize_batch(response, &config, started, &request);
    }
    response["editCount"] = json!(edit_count);
    let mut results = vec![Value::Null; request.operations.len()];
    let mut scan_files = 0usize;
    let mut scan_bytes = 0u64;
    for (index, operation) in request.operations.iter().enumerate() {
        if let Some(error) = validation_errors[index].clone() {
            results[index] = operation_error_envelope(index, operation, "failed", error);
            continue;
        }
        let mut search_budget = None;
        let result = match operation.kind.as_str() {
            "read" => match resolve_path(&config, &operation.path, Access::Read) {
                Ok(path) => read(
                    &path,
                    operation.include_content,
                    operation.start_line,
                    operation.end_line,
                ),
                Err(error) => Err(error),
            },
            "search" => {
                let mode = match operation.search_mode.as_deref() {
                    Some("regex") => SearchMode::Regex,
                    Some("literal") | None => SearchMode::Literal,
                    _ => {
                        results[index] = operation_error_envelope(
                            index,
                            operation,
                            "failed",
                            batch_error("file_invalid_mode", "mode must be literal or regex"),
                        );
                        continue;
                    }
                };
                let path = match resolve_path(&config, &operation.path, Access::Read) {
                    Ok(path) => path,
                    Err(error) if error.code == "file_not_found" => {
                        results[index] = operation_error_envelope(
                            index,
                            operation,
                            "failed",
                            batch_error("file_search_path_not_found", "search path was not found"),
                        );
                        continue;
                    }
                    Err(error) => {
                        results[index] =
                            operation_error_envelope(index, operation, "failed", error.value());
                        continue;
                    }
                };
                let budget = remaining_batch_search_budget(scan_files, scan_bytes);
                search_budget = Some(budget);
                search_with_context_limit(
                    SearchOptions {
                        root: &path,
                        query: operation.query.as_deref().unwrap_or_default(),
                        mode,
                        case_sensitive: operation.case_sensitive,
                        include: &operation.include,
                        exclude: &operation.exclude,
                        context_lines: operation.context_lines,
                        max_results: operation.max_results,
                        hidden: operation.hidden,
                        respect_gitignore: operation.respect_gitignore,
                        scan_file_limit: budget.file_limit,
                        scan_byte_limit: budget.byte_limit,
                    },
                    config.limits.max_file_search_context_lines,
                )
            }
            "edit" => continue,
            _ => unreachable!(),
        };
        match result {
            Ok(value) => {
                let mut aggregate_limit_hit = false;
                if operation.kind == "search" {
                    scan_files = scan_files
                        .saturating_add(value["scannedFiles"].as_u64().unwrap_or(0) as usize);
                    scan_bytes =
                        scan_bytes.saturating_add(value["scannedBytes"].as_u64().unwrap_or(0));
                    aggregate_limit_hit = search_budget
                        .is_some_and(|budget| aggregate_search_limit_hit(&value, budget));
                }
                if aggregate_limit_hit {
                    let error = batch_error(
                        "file_batch_scan_limit_exceeded",
                        "aggregate search scan exceeds the batch bound",
                    );
                    response["error"] = error["error"].clone();
                    results[index] = operation_error_envelope(index, operation, "failed", error);
                } else {
                    results[index] = operation_envelope(index, operation, "completed", value);
                }
            }
            Err(error) => {
                results[index] =
                    operation_error_envelope(index, operation, "failed", error.value());
            }
        }
    }
    debug_assert!(scan_files <= MAX_BATCH_SCAN_FILES);
    debug_assert!(scan_bytes <= MAX_BATCH_SCAN_BYTES);
    if edit_count == 0 {
        let failed = results
            .iter()
            .filter(|value| value["status"] == "failed")
            .count();
        response["status"] = json!(if failed == 0 {
            "completed"
        } else {
            "completed_with_errors"
        });
        let mut no_groups = Vec::new();
        attach_batch_group_evidence(&mut response, &mut no_groups, &request.operations, &results);
        return finalize_batch(response, &config, started, &request);
    }
    let mut groups = Vec::<BatchFileGroup>::new();
    let mut group_indexes = HashMap::<PathBuf, usize>::new();
    let mut group_record_indexes = HashMap::<String, usize>::new();
    let mut group_records = Vec::<BatchGroupRecord>::new();
    for (index, operation) in request.operations.iter().enumerate() {
        if operation.kind != "edit" {
            continue;
        }
        if validation_errors[index].is_some() {
            let record = BatchGroupRecord::synthetic(index, operation);
            let group_id = record.group_id.clone();
            if !results[index].is_null() {
                results[index] = annotate_group_result(results[index].take(), &group_id, false);
            }
            group_record_indexes.insert(group_id, group_records.len());
            group_records.push(record);
            continue;
        }
        let resolved = match resolve_path(&config, &operation.path, Access::Write) {
            Ok(path) => Some((path, true)),
            Err(error) if error.code == "file_not_found" => {
                match resolve_absent_path(&config, &operation.path) {
                    Ok(path) => Some((path, false)),
                    Err(error) => {
                        results[index] =
                            operation_error_envelope(index, operation, "failed", error.value());
                        let record = BatchGroupRecord::synthetic(index, operation);
                        let group_id = record.group_id.clone();
                        results[index] =
                            annotate_group_result(results[index].take(), &group_id, false);
                        group_record_indexes.insert(group_id, group_records.len());
                        group_records.push(record);
                        None
                    }
                }
            }
            Err(error) => {
                results[index] =
                    operation_error_envelope(index, operation, "failed", error.value());
                let record = BatchGroupRecord::synthetic(index, operation);
                let group_id = record.group_id.clone();
                results[index] = annotate_group_result(results[index].take(), &group_id, false);
                group_record_indexes.insert(group_id, group_records.len());
                group_records.push(record);
                None
            }
        };
        if let Some((target, existed)) = resolved {
            if let Some(group_index) = group_indexes.get(&target.path).copied() {
                groups[group_index].operation_indices.push(index);
                let group_id = groups[group_index].group_id.clone();
                if let Some(record_index) = group_record_indexes.get(&group_id).copied() {
                    group_records[record_index].operation_indices.push(index);
                }
            } else {
                let group_id = format!("file_group_{index}");
                group_indexes.insert(target.path.clone(), groups.len());
                let group = BatchFileGroup {
                    group_id: group_id.clone(),
                    target,
                    existed,
                    before_bytes: None,
                    before_revision: None,
                    before_mode: None,
                    candidate: None,
                    candidate_revision: None,
                    operation_indices: vec![index],
                };
                group_record_indexes.insert(group_id, group_records.len());
                group_records.push(BatchGroupRecord::from_group(&group));
                groups.push(group);
            }
        }
    }
    groups.sort_by(|left, right| left.target.path.cmp(&right.target.path));
    let mut locks = Vec::new();
    for group in &groups {
        locks.push(lock_target(state, &group.target.path).await);
    }
    let mut prepared = Vec::new();
    let mut original_bytes = 0usize;
    let mut candidate_bytes = 0usize;
    for mut group in groups {
        let record_index = group_record_indexes[&group.group_id];
        let first_index = group.operation_indices[0];
        let mut group_error = None::<(usize, Value)>;
        if let Err(error) = revalidate_target(&config, &group.target, group.existed) {
            group_error = Some((first_index, error.value()));
        }
        if group_error.is_none() {
            if let Err(error) = load_batch_group_base(&mut group) {
                group_error = Some((first_index, error.value()));
            }
        }
        if group_error.is_none() {
            if let Err(error) = validate_batch_group_guards(&group, &request.operations) {
                group_error = Some((first_index, error.value()));
            }
        }
        if group_error.is_none() {
            let operation_indices = group.operation_indices.clone();
            for index in operation_indices {
                match apply_batch_operation(&mut group, &request.operations[index]) {
                    Ok(value) => {
                        results[index] = operation_envelope(
                            index,
                            &request.operations[index],
                            "completed",
                            value,
                        );
                    }
                    Err(error) => {
                        group_error = Some((index, error.value()));
                        break;
                    }
                }
            }
        }
        if let Some((failed_index, error)) = group_error {
            mark_group_failure(
                &mut results,
                &request.operations,
                &group.operation_indices,
                &group.group_id,
                failed_index,
                error,
                ("file_batch_group_rejected", "file group preflight failed"),
            );
            group_records[record_index].status = "failed".to_string();
            group_records[record_index].error_code = results[failed_index]
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string);
            continue;
        }
        let final_candidate = match group.candidate.take() {
            Some(candidate) => candidate,
            None => {
                let error = batch_error(
                    "file_batch_candidate_missing",
                    "file group did not produce a candidate",
                );
                mark_group_failure(
                    &mut results,
                    &request.operations,
                    &group.operation_indices,
                    &group.group_id,
                    *group.operation_indices.last().unwrap_or(&first_index),
                    error,
                    (
                        "file_batch_group_rejected",
                        "file group candidate planning failed",
                    ),
                );
                group_records[record_index].status = "failed".to_string();
                group_records[record_index].error_code =
                    Some("file_batch_candidate_missing".to_string());
                continue;
            }
        };
        let before_revision = group.before_revision.clone();
        let before_size_bytes = group.before_bytes.as_ref().map_or(0, Vec::len);
        let next_original_bytes = original_bytes.saturating_add(before_size_bytes);
        let next_candidate_bytes = candidate_bytes.saturating_add(final_candidate.len());
        if next_original_bytes > MAX_BATCH_ORIGINAL_BYTES
            || next_candidate_bytes > MAX_BATCH_CANDIDATE_BYTES
        {
            let error = batch_error(
                "file_batch_candidate_limit_exceeded",
                "aggregate batch file bytes exceed the bound",
            );
            mark_group_failure(
                &mut results,
                &request.operations,
                &group.operation_indices,
                &group.group_id,
                *group.operation_indices.last().unwrap_or(&first_index),
                error,
                (
                    "file_batch_group_rejected",
                    "file group candidate exceeds the batch bound",
                ),
            );
            group_records[record_index].status = "failed".to_string();
            group_records[record_index].error_code =
                Some("file_batch_candidate_limit_exceeded".to_string());
            continue;
        }
        original_bytes = next_original_bytes;
        candidate_bytes = next_candidate_bytes;
        let after_revision = group
            .candidate_revision
            .take()
            .unwrap_or_else(|| revision(&final_candidate));
        group_records[record_index].base_revision = before_revision.clone();
        group_records[record_index].final_revision = Some(after_revision.clone());
        group_records[record_index].status =
            if group.existed && before_revision.as_deref() == Some(after_revision.as_str()) {
                "unchanged".to_string()
            } else {
                "pending".to_string()
            };
        prepared.push(BatchPreparedGroup {
            group_id: group.group_id,
            target: group.target,
            existed: group.existed,
            before_revision,
            before_size_bytes,
            before_mode: group.before_mode,
            candidate: final_candidate,
            after_revision,
            operation_indices: group.operation_indices,
            temp: None,
        });
    }
    let effective: Vec<usize> = request
        .operations
        .iter()
        .enumerate()
        .filter(|(index, operation)| {
            operation.kind == "edit" && results[*index]["result"]["status"] != "unchanged"
        })
        .map(|(index, _)| index)
        .collect();
    let has_changed_groups = prepared.iter().any(prepared_group_changed);
    response["effectiveMutationCount"] = json!(effective.len());
    let mut commit_attempted = false;
    if !request.dry_run && has_changed_groups {
        for item in prepared
            .iter_mut()
            .filter(|item| prepared_group_changed(item))
        {
            let record_index = group_record_indexes[&item.group_id];
            match stage_batch_temp(
                &item.target.path,
                &item.candidate,
                item.before_mode.as_ref(),
            ) {
                Ok(temp) => {
                    item.temp = Some(temp);
                    group_records[record_index].status = "staged".to_string();
                }
                Err(error) => {
                    let error_value = error.value();
                    mark_group_failure(
                        &mut results,
                        &request.operations,
                        &item.operation_indices,
                        &item.group_id,
                        *item.operation_indices.first().unwrap_or(&0),
                        error_value,
                        ("file_batch_group_rejected", "file group staging failed"),
                    );
                    group_records[record_index].status = "failed".to_string();
                    group_records[record_index].error_code = Some(error.code);
                }
            }
        }
    }
    let staged_changed_groups = prepared
        .iter()
        .any(|item| prepared_group_changed(item) && item.temp.is_some());
    if request.need_confirm && !request.dry_run && staged_changed_groups {
        let preview = prepared
            .iter()
            .filter(|item| prepared_group_changed(item) && item.temp.is_some())
            .map(|item| {
                let last_index = *item.operation_indices.last().unwrap_or(&0);
                let changed = &results[last_index]["result"]["changedLines"];
                let mode = batch_edit_request(&request.operations[last_index])
                    .map(|request| edit_mode_label(request.mode))
                    .unwrap_or("edit");
                let operation_ids = item
                    .operation_indices
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "group:{}:{}:ops={}:{}:{}->{}:{}:{}:+{}-{}",
                    last_index,
                    item.target.path.display(),
                    operation_ids,
                    mode,
                    item.before_revision.as_deref().unwrap_or("absent"),
                    item.after_revision,
                    item.before_size_bytes,
                    item.candidate.len(),
                    changed["added"].as_u64().unwrap_or(0),
                    changed["removed"].as_u64().unwrap_or(0),
                )
            })
            .collect::<Vec<_>>();
        let confirmation =
            crate::confirmation::request_confirmation(state, &config, None, "file.batch", &preview)
                .await;
        response["confirmation"] = json!({"requested": true, "result": confirmation});
        if confirmation != "allow_once" {
            let code = if confirmation == "confirmation_provider_unavailable" {
                "file_batch_confirmation_unavailable"
            } else {
                "file_batch_confirmation_denied"
            };
            for item in prepared.iter_mut() {
                if prepared_group_changed(item) && item.temp.is_some() {
                    if let Some(temp) = item.temp.take() {
                        let _ = fs::remove_file(temp);
                    }
                    let record_index = group_record_indexes[&item.group_id];
                    mark_group_all_failure(
                        &mut results,
                        &request.operations,
                        &item.operation_indices,
                        &item.group_id,
                        code,
                        "batch mutation was not confirmed",
                    );
                    group_records[record_index].status = "failed".to_string();
                    group_records[record_index].error_code = Some(code.to_string());
                }
            }
            for item in prepared.iter_mut() {
                if let Some(temp) = item.temp.take() {
                    let _ = fs::remove_file(temp);
                }
            }
            response["status"] = json!("rejected");
            for item in &prepared {
                if !prepared_group_changed(item) {
                    annotate_group_operations(
                        &mut results,
                        &item.operation_indices,
                        &item.group_id,
                        false,
                    );
                }
            }
            attach_batch_group_evidence(
                &mut response,
                &mut group_records,
                &request.operations,
                &results,
            );
            return finalize_batch(response, &config, started, &request);
        }
    }
    if request.dry_run {
        for item in &prepared {
            let record_index = group_record_indexes[&item.group_id];
            for index in &item.operation_indices {
                if let Some(mut value) = results[*index].get("result").cloned() {
                    if value["status"] != "unchanged" {
                        value["status"] = json!("dry-run");
                    }
                    results[*index] = annotate_group_result(
                        operation_envelope(*index, &request.operations[*index], "completed", value),
                        &item.group_id,
                        false,
                    );
                }
            }
            group_records[record_index].status = if prepared_group_changed(item) {
                "dry-run".to_string()
            } else {
                "unchanged".to_string()
            };
        }
        response["status"] = json!("dry-run");
        attach_batch_group_evidence(
            &mut response,
            &mut group_records,
            &request.operations,
            &results,
        );
        return finalize_batch(response, &config, started, &request);
    }
    for item in prepared
        .iter_mut()
        .filter(|item| prepared_group_changed(item) && item.temp.is_some())
    {
        {
            commit_attempted = true;
            let record_index = group_record_indexes[&item.group_id];
            let mut commit_error = None::<FileError>;
            if let Err(error) = revalidate_target(&config, &item.target, item.existed) {
                commit_error = Some(error);
            } else if item.existed {
                match read_bounded(&item.target.path, MAX_FILE_BYTES) {
                    Ok(BoundedRead::Complete(bytes))
                        if Some(revision(&bytes)) == item.before_revision => {}
                    _ => {
                        commit_error = Some(FileError::new(
                            "file_revision_conflict",
                            "target changed before batch commit",
                        ));
                    }
                }
            } else if fs::symlink_metadata(&item.target.requested).is_ok() {
                commit_error = Some(FileError::new(
                    "file_already_exists",
                    "target appeared before batch commit",
                ));
            }
            let temp = item.temp.take();
            if commit_error.is_none() {
                let temp = temp.ok_or_else(|| {
                    FileError::new("file_batch_commit_failed", "batch staging file is missing")
                });
                match temp {
                    Ok(temp) => {
                        let result = if item.existed {
                            fs::rename(&temp, &item.target.path)
                        } else {
                            match fs::hard_link(&temp, &item.target.requested) {
                                Ok(()) => {
                                    let _ = fs::remove_file(&temp);
                                    Ok(())
                                }
                                Err(error) => Err(error),
                            }
                        };
                        if let Err(error) = result {
                            let _ = fs::remove_file(&temp);
                            commit_error = Some(FileError::new(
                                "file_batch_commit_failed",
                                &format!("atomic file commit failed: {error}"),
                            ));
                        }
                    }
                    Err(error) => commit_error = Some(error),
                }
            } else if let Some(temp) = temp {
                let _ = fs::remove_file(temp);
            }
            if let Some(error) = commit_error {
                let error_code = error.code.clone();
                mark_group_failure(
                    &mut results,
                    &request.operations,
                    &item.operation_indices,
                    &item.group_id,
                    *item.operation_indices.first().unwrap_or(&0),
                    error.value(),
                    ("file_batch_group_rejected", "file group commit failed"),
                );
                group_records[record_index].status = "failed".to_string();
                group_records[record_index].error_code = Some(error_code);
            } else {
                sync_parent(&item.target.path);
                group_records[record_index].status = "committed".to_string();
                group_records[record_index].committed = true;
                annotate_group_operations(
                    &mut results,
                    &item.operation_indices,
                    &item.group_id,
                    true,
                );
            }
        }
    }
    for item in &prepared {
        let record_index = group_record_indexes[&item.group_id];
        if !prepared_group_changed(item) {
            group_records[record_index].status = "unchanged".to_string();
            annotate_group_operations(&mut results, &item.operation_indices, &item.group_id, false);
        }
        if let Some(temp) = &item.temp {
            let _ = fs::remove_file(temp);
        }
    }
    let operation_failures = results
        .iter()
        .any(|result| matches!(result["status"].as_str(), Some("failed" | "skipped")));
    let successful_groups = group_records
        .iter()
        .filter(|record| matches!(record.status.as_str(), "committed" | "unchanged"))
        .count();
    response["status"] = json!(if operation_failures {
        if commit_attempted || successful_groups > 0 {
            "completed_with_errors"
        } else {
            "rejected"
        }
    } else {
        "completed"
    });
    attach_batch_group_evidence(
        &mut response,
        &mut group_records,
        &request.operations,
        &results,
    );
    finalize_batch(response, &config, started, &request)
}

fn finalize_batch(
    mut response: Value,
    config: &Config,
    started: std::time::Instant,
    request: &BatchRequest,
) -> Value {
    response["updatedAt"] = json!(Utc::now());
    let batch_id = response["batchId"]
        .as_str()
        .unwrap_or("file_batch_unknown")
        .to_string();
    let mut audits_written = true;
    if !request.dry_run {
        if let Some(results) = response["results"].as_array() {
            for (index, operation) in request.operations.iter().enumerate() {
                if operation.kind != "edit" {
                    continue;
                }
                let value = results
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| json!({"status":"failed"}));
                let payload = value.get("result").cloned().unwrap_or_default();
                let group_id = value
                    .get("groupId")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("groupId").and_then(Value::as_str))
                    .map(str::to_string);
                let committed = value
                    .get("committed")
                    .and_then(Value::as_bool)
                    .or_else(|| payload.get("committed").and_then(Value::as_bool));
                let mode = operation.edit_mode.as_deref().map(|mode| mode.to_string());
                let changed_lines = payload.get("changedLines").and_then(|lines| {
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
                        action: "batch-edit".to_string(),
                        batch_id: Some(batch_id.clone()),
                        group_id,
                        operation_index: Some(index),
                        operation_id: operation.id.clone(),
                        path: payload
                            .get("resolvedPath")
                            .and_then(Value::as_str)
                            .unwrap_or(&operation.path)
                            .to_string(),
                        mode,
                        requested_confirmation: request.need_confirm,
                        confirmation_result: response["confirmation"]["result"]
                            .as_str()
                            .map(str::to_string),
                        before_revision: payload
                            .get("beforeRevision")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        after_revision: payload
                            .get("afterRevision")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        outcome: value
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("failed")
                            .to_string(),
                        error_code: value
                            .get("error")
                            .and_then(|error| error.get("code"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        duration_ms: started.elapsed().as_millis(),
                        replacement_count: payload
                            .get("replacementCount")
                            .and_then(Value::as_u64)
                            .map(|value| value as usize),
                        changed_lines,
                        committed,
                    },
                )
                .is_err()
                {
                    audits_written = false;
                }
            }
        }
    }
    response["auditStatus"] = json!("pending");
    let mut finalized = truncate_batch_response(response);
    let status = finalized["status"]
        .as_str()
        .unwrap_or("rejected")
        .to_string();
    let group_counts = &finalized["groupCounts"];
    let truncated = finalized["truncated"].as_bool().unwrap_or(false);
    if write_batch_audit(
        config,
        BatchAuditRecord {
            time: Utc::now(),
            tool: "file.batch".to_string(),
            action: "batch".to_string(),
            batch_id,
            operation_count: request.operations.len(),
            edit_count: request
                .operations
                .iter()
                .filter(|operation| operation.kind == "edit")
                .count(),
            group_count: group_counts["total"].as_u64().unwrap_or(0) as usize,
            committed_group_count: group_counts["committed"].as_u64().unwrap_or(0) as usize,
            failed_group_count: group_counts["failed"].as_u64().unwrap_or(0) as usize,
            unchanged_group_count: group_counts["unchanged"].as_u64().unwrap_or(0) as usize,
            failure_count: finalized["failureCount"].as_u64().unwrap_or(0) as usize,
            confirmation_result: finalized["confirmation"]["result"]
                .as_str()
                .map(str::to_string),
            outcome: status,
            duration_ms: started.elapsed().as_millis(),
            truncated,
        },
    )
    .is_err()
    {
        audits_written = false;
    }
    finalized["auditStatus"] = json!(if audits_written { "written" } else { "failed" });
    finalized
}

fn truncate_batch_response(mut response: Value) -> Value {
    let mut bytes = serde_json::to_vec(&response).unwrap_or_default();
    if bytes.len() <= MAX_BATCH_OUTPUT_BYTES {
        return response;
    }
    response["truncated"] = json!(true);
    response["truncationReason"] = json!("output_bytes");
    let result_count = response["results"].as_array().map_or(0, Vec::len);
    for index in 0..result_count {
        if let Some(result) = response["results"].get_mut(index) {
            if let Some(payload) = result.get_mut("result").and_then(Value::as_object_mut) {
                payload.remove("content");
                payload.remove("matches");
                payload.remove("diff");
                payload.insert("resultTruncated".to_string(), json!(true));
            }
            bytes = serde_json::to_vec(&response).unwrap_or_default();
            if bytes.len() <= MAX_BATCH_OUTPUT_BYTES {
                break;
            }
        }
    }
    if bytes.len() > MAX_BATCH_OUTPUT_BYTES {
        response["results"] = response["results"].as_array().map(|results| {
            Value::Array(results.iter().map(|result| {
                let mut compact = json!({"index": result["index"], "type": result["type"], "status": result["status"]});
                if result.get("id").is_some() { compact["id"] = result["id"].clone(); }
                if result.get("groupId").is_some() { compact["groupId"] = result["groupId"].clone(); }
                if result.get("committed").is_some() { compact["committed"] = result["committed"].clone(); }
                if result.get("error").is_some() { compact["error"] = result["error"].clone(); }
                compact
            }).collect())
        }).unwrap_or_else(|| json!([]));
    }
    response
}

fn edit_mode_label(mode: EditMode) -> &'static str {
    match mode {
        EditMode::Replace => "replace",
        EditMode::Patch => "patch",
        EditMode::Write => "write",
    }
}

fn validate_expected_revision(
    expected: Option<&str>,
    actual: &str,
) -> std::result::Result<(), FileError> {
    let expected = expected.ok_or_else(|| {
        FileError::new(
            "file_revision_required",
            "existing-file mutations require expectedRevision",
        )
    })?;
    if !is_revision(expected) {
        return Err(FileError::new(
            "file_revision_invalid",
            "expectedRevision is invalid",
        ));
    }
    if expected != actual {
        return Err(FileError::new(
            "file_revision_conflict",
            "target revision does not match",
        ));
    }
    Ok(())
}

fn is_revision(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn revalidate_target(
    config: &Config,
    target: &ResolvedPath,
    existed: bool,
) -> std::result::Result<(), FileError> {
    if existed {
        let current = resolve_path(config, &target.input, Access::Write)?;
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
            FileError::new("file_parent_not_found", "parent directory was not found")
        })?;
        let canonical_parent = fs::canonicalize(parent).map_err(|_| {
            FileError::new("file_parent_not_found", "parent directory was not found")
        })?;
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
        if file.write_all(bytes).is_err() {
            let _ = fs::remove_file(&temp);
            return Err(FileError::new(
                "file_write_failed",
                "temporary file could not be written",
            ));
        }
        if let Some(permissions) = permissions {
            if file.set_permissions(permissions.clone()).is_err() {
                let _ = fs::remove_file(&temp);
                return Err(FileError::new(
                    "file_write_failed",
                    "temporary file permissions could not be preserved",
                ));
            }
        }
        if file.sync_all().is_err() {
            let _ = fs::remove_file(&temp);
            return Err(FileError::new(
                "file_write_failed",
                "temporary file could not be synced",
            ));
        }
        return Ok(temp);
    }
    Err(FileError::new(
        "file_write_failed",
        "temporary file could not be created",
    ))
}

fn stage_batch_temp(
    target: &Path,
    bytes: &[u8],
    permissions: Option<&fs::Permissions>,
) -> std::result::Result<PathBuf, FileError> {
    #[cfg(test)]
    let injected_failure = {
        let mut expected_target = INJECT_BATCH_STAGE_FAILURE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let matches_target = expected_target.as_ref().is_some_and(|expected| {
            fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf()) == *expected
        });
        if matches_target {
            expected_target.take();
        }
        matches_target
    };
    #[cfg(test)]
    if injected_failure {
        return Err(FileError::new(
            "file_batch_stage_failed",
            "injected batch staging failure",
        ));
    }
    stage_temp(target, bytes, permissions)
}

fn sync_parent(target: &Path) {
    if let Some(parent) = target.parent() {
        if let Ok(file) = fs::File::open(parent) {
            let _ = file.sync_all();
        }
    }
}

const MAX_DIFF_BYTES: usize = 64 * 1024;
const DIFF_CONTEXT_LINES: usize = 3;
const NO_NEWLINE_MARKER: &str = "\\ No newline at end of file";

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

fn logical_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n')
        .map(|line| {
            let line = line.strip_suffix('\n').unwrap_or(line);
            line.strip_suffix('\r').unwrap_or(line).to_string()
        })
        .collect()
}

fn apply_unified_patch(
    original: &str,
    patch: &str,
    target: &Path,
) -> std::result::Result<String, FileError> {
    let patch_lines = patch.lines().collect::<Vec<_>>();
    if patch_lines.is_empty() {
        return Err(FileError::new("file_patch_invalid", "patch is empty"));
    }
    let mut index = 0;
    if patch_lines
        .first()
        .is_some_and(|line| line.starts_with("--- "))
    {
        if patch_lines.len() < 2 || !patch_lines[1].starts_with("+++ ") {
            return Err(FileError::new(
                "file_patch_invalid",
                "patch headers are incomplete",
            ));
        }
        let old = normalize_patch_header(&patch_lines[0][4..]);
        let new = normalize_patch_header(&patch_lines[1][4..]);
        if old != new || !header_matches_target(&new, target) {
            return Err(FileError::new(
                "file_patch_invalid",
                "patch target does not match requested file",
            ));
        }
        index = 2;
    }
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines = logical_lines(original);
    let mut offset = 0isize;
    let mut old_missing_newline = false;
    let mut new_missing_newline = false;
    let mut last_new_range_end = None;
    while index < patch_lines.len() {
        let header = patch_lines[index];
        if !header.starts_with("@@ ") {
            return Err(FileError::new(
                "file_patch_invalid",
                "expected a unified hunk",
            ));
        }
        let (old_start, old_len, new_start, new_len) = parse_hunk_header(header)?;
        index += 1;
        let old_index = range_index(old_start, old_len)?;
        let new_index = range_index(new_start, new_len)?;
        let start = (old_index as isize)
            .checked_add(offset)
            .filter(|start| *start >= 0)
            .map(|start| start as usize)
            .ok_or_else(|| FileError::new("file_patch_conflict", "hunk location does not match"))?;
        if new_index != start {
            return Err(FileError::new(
                "file_patch_invalid",
                "old and new hunk ranges are inconsistent",
            ));
        }
        last_new_range_end = Some(new_index.saturating_add(new_len));
        if start > lines.len() {
            return Err(FileError::new(
                "file_patch_conflict",
                "hunk location does not match",
            ));
        }
        let mut cursor = start;
        let mut consumed_old = 0usize;
        let mut replacement = Vec::new();
        let mut consumed_new = 0usize;
        let mut previous_kind = None;
        while index < patch_lines.len() && !patch_lines[index].starts_with("@@ ") {
            let line = patch_lines[index];
            index += 1;
            if line.starts_with("--- ") || line.starts_with("+++ ") {
                return Err(FileError::new(
                    "file_patch_invalid",
                    "multiple file headers are not supported",
                ));
            }
            if line == NO_NEWLINE_MARKER {
                match previous_kind {
                    Some('-') => old_missing_newline = true,
                    Some('+') => new_missing_newline = true,
                    Some(' ') => {
                        old_missing_newline = true;
                        new_missing_newline = true;
                    }
                    _ => {
                        return Err(FileError::new(
                            "file_patch_invalid",
                            "newline marker must follow a hunk line",
                        ))
                    }
                }
                previous_kind = None;
                continue;
            }
            let (kind, content) = match line.as_bytes().first() {
                Some(b' ') => (' ', &line[1..]),
                Some(b'-') => ('-', &line[1..]),
                Some(b'+') => ('+', &line[1..]),
                _ => return Err(FileError::new("file_patch_invalid", "invalid hunk line")),
            };
            previous_kind = Some(kind);
            match kind {
                ' ' => {
                    if lines.get(cursor).map(String::as_str) != Some(content) {
                        return Err(FileError::new(
                            "file_patch_conflict",
                            "context does not match",
                        ));
                    }
                    cursor += 1;
                    consumed_old += 1;
                    replacement.push(content.to_string());
                    consumed_new += 1;
                }
                '-' => {
                    if lines.get(cursor).map(String::as_str) != Some(content) {
                        return Err(FileError::new(
                            "file_patch_conflict",
                            "removed text does not match",
                        ));
                    }
                    cursor += 1;
                    consumed_old += 1;
                }
                '+' => {
                    replacement.push(content.to_string());
                    consumed_new += 1;
                }
                _ => return Err(FileError::new("file_patch_invalid", "invalid hunk line")),
            }
        }
        if consumed_old != old_len || consumed_new != new_len {
            return Err(FileError::new(
                "file_patch_invalid",
                "hunk line counts do not match",
            ));
        }
        let replacement_end = start + consumed_old;
        lines.splice(start..replacement_end, replacement);
        offset += new_len as isize - old_len as isize;
    }
    let hunk_reaches_new_eof = last_new_range_end == Some(lines.len());
    let has_final_newline = if lines.is_empty() || new_missing_newline {
        false
    } else if old_missing_newline || hunk_reaches_new_eof {
        true
    } else {
        original.ends_with('\n')
    };
    Ok(lines.join(newline) + if has_final_newline { newline } else { "" })
}

fn normalize_patch_header(value: &str) -> String {
    value
        .split('\t')
        .next()
        .unwrap_or(value)
        .trim_start_matches("a/")
        .trim_start_matches("b/")
        .to_string()
}

fn header_matches_target(header: &str, target: &Path) -> bool {
    let target = target
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let target_name = Path::new(&target)
        .file_name()
        .and_then(|name| name.to_str());
    header == target
        || target.ends_with(&format!("/{header}"))
        || (!header.contains('/') && Some(header) == target_name)
}

fn parse_hunk_header(header: &str) -> std::result::Result<(usize, usize, usize, usize), FileError> {
    let body = header
        .strip_prefix("@@ ")
        .and_then(|body| body.split(" @@").next())
        .ok_or_else(|| FileError::new("file_patch_invalid", "invalid hunk header"))?;
    let mut fields = body.split_whitespace();
    let old = fields
        .next()
        .and_then(|value| value.strip_prefix('-'))
        .ok_or_else(|| FileError::new("file_patch_invalid", "invalid old hunk range"))?;
    let new = fields
        .next()
        .and_then(|value| value.strip_prefix('+'))
        .ok_or_else(|| FileError::new("file_patch_invalid", "invalid new hunk range"))?;
    if fields.next().is_some() {
        return Err(FileError::new(
            "file_patch_invalid",
            "invalid hunk header fields",
        ));
    }
    let (old_start, old_len) = parse_range(old)?;
    let (new_start, new_len) = parse_range(new)?;
    Ok((old_start, old_len, new_start, new_len))
}

fn parse_range(value: &str) -> std::result::Result<(usize, usize), FileError> {
    let mut values = value.split(',');
    let start = values
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| FileError::new("file_patch_invalid", "invalid hunk range"))?;
    let len = match values.next() {
        Some(value) => value
            .parse()
            .map_err(|_| FileError::new("file_patch_invalid", "invalid hunk range"))?,
        None => 1,
    };
    if values.next().is_some() || (start == 0 && len != 0) {
        return Err(FileError::new("file_patch_invalid", "invalid hunk range"));
    }
    Ok((start, len))
}

fn range_index(start: usize, len: usize) -> std::result::Result<usize, FileError> {
    if len == 0 {
        Ok(start)
    } else {
        start
            .checked_sub(1)
            .ok_or_else(|| FileError::new("file_patch_invalid", "invalid hunk range"))
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
    fn reads_metadata_and_preserves_exact_revision_and_newlines() {
        let root =
            std::env::temp_dir().join(format!("file-read-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("sample.txt");
        fs::write(&path, "one\r\ntwo\n").unwrap();
        let config = config(&root);
        let resolved = resolve_path(&config, "sample.txt", Access::Read).unwrap();
        let value = read(&resolved, true, None, None).unwrap();
        assert_eq!(value["content"], "one\r\ntwo\n");
        assert_eq!(value["totalLines"], 2);
        assert_eq!(value["revision"], revision(b"one\r\ntwo\n"));
        let metadata = read(&resolved, false, None, None).unwrap();
        assert!(metadata.get("content").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ranges_are_bounded_and_utf8_safe() {
        let root =
            std::env::temp_dir().join(format!("file-range-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("sample.txt");
        fs::write(&path, "a\nβ\nccc\n").unwrap();
        let config = config(&root);
        let resolved = resolve_path(&config, "sample.txt", Access::Read).unwrap();
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
        let write_error = check_policy(&config, &read_resolved.path, Access::Write).unwrap_err();
        assert_eq!(write_error.code, "path_readonly");
        let denied_resolved = resolve_path(
            &config,
            &denied.join("a.txt").to_string_lossy(),
            Access::Read,
        )
        .unwrap_err();
        assert_eq!(denied_resolved.code, "path_denied");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn metadata_mode_describes_binary_and_content_mode_rejects_it() {
        let root =
            std::env::temp_dir().join(format!("file-binary-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("binary.dat");
        fs::write(&path, [0_u8, 159, 146, 150]).unwrap();
        let config = config(&root);
        let resolved = resolve_path(&config, "binary.dat", Access::Read).unwrap();
        let metadata = read(&resolved, false, None, None).unwrap();
        assert!(metadata.get("revision").is_none());
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
        let path = root.join("large.txt");
        let bytes = vec![b'x'; MAX_FILE_BYTES as usize + 1];
        fs::write(&path, bytes).unwrap();
        let config = config(&root);
        let resolved = resolve_path(&config, "large.txt", Access::Read).unwrap();
        let metadata = read(&resolved, false, None, None).unwrap();
        assert_eq!(metadata["sizeBytes"], MAX_FILE_BYTES + 1);
        assert!(metadata.get("revision").is_none());
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
        match read_bounded(&path, 5).unwrap() {
            BoundedRead::Complete(bytes) => assert_eq!(bytes, b"12345"),
            BoundedRead::Exceeded => panic!("exact-bound read must complete"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_allowed_only_when_the_canonical_target_stays_inside_policy() {
        let root =
            std::env::temp_dir().join(format!("file-symlink-{}", uuid::Uuid::new_v4().simple()));
        let outside = root.join("outside");
        let workspace = root.join("workspace");
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
        let error = resolve_path(&config, "escape-link.txt", Access::Read).unwrap_err();
        assert_eq!(error.code, "path_denied");
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
        let config = config(&root);
        let resolved = resolve_path(&config, ".", Access::Read).unwrap();
        let include = vec!["**/*.rs".to_string()];
        let empty = Vec::new();
        let value = search(SearchOptions {
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
        })
        .unwrap();
        assert_eq!(value["matchCount"], 2);
        assert_eq!(value["matches"][0]["line"], 2);
        assert_eq!(value["matches"][0]["column"], 1);
        assert_eq!(value["matches"][0]["before"][0], "before");
        assert_eq!(value["matches"][0]["after"][0], "after");
        assert_eq!(value["skippedFiles"]["nonUtf8"], 1);

        let regex = search(SearchOptions {
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
        })
        .unwrap();
        assert_eq!(regex["matchCount"], 1);
        assert_eq!(regex["truncated"], true);
        assert_eq!(regex["truncationReason"], "max_results");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_streams_file_and_byte_limits_without_overshoot() {
        let root = std::env::temp_dir().join(format!(
            "file-search-bounds-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("a.txt"),
            "needle
",
        )
        .unwrap();
        fs::write(
            root.join("b.txt"),
            "needle
",
        )
        .unwrap();
        let config = config(&root);
        let resolved = resolve_path(&config, ".", Access::Read).unwrap();
        let empty = Vec::new();
        let files = search(SearchOptions {
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
        })
        .unwrap();
        assert_eq!(files["scannedFiles"], 1);
        assert_eq!(files["truncated"], true);
        assert_eq!(files["truncationReason"], "scan_files");

        let file = resolve_path(&config, "a.txt", Access::Read).unwrap();
        let bytes = search(SearchOptions {
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
        })
        .unwrap();
        assert_eq!(bytes["scannedBytes"], 0);
        assert_eq!(bytes["truncated"], true);
        assert_eq!(bytes["truncationReason"], "scan_bytes");

        let output_path = root.join("large-output.txt");
        let long_line = format!("needle{}\n", "x".repeat(MAX_LINE_DISPLAY_BYTES));
        fs::write(&output_path, long_line.repeat(80)).unwrap();
        let output_file = resolve_path(&config, "large-output.txt", Access::Read).unwrap();
        let output = search(SearchOptions {
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
        })
        .unwrap();
        assert_eq!(output["truncationReason"], "output_bytes");
        assert!(serde_json::to_vec(&output["matches"]).unwrap().len() <= MAX_SEARCH_OUTPUT_BYTES);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn batch_search_budget_exposes_only_the_remaining_aggregate_capacity() {
        let budget =
            remaining_batch_search_budget(MAX_BATCH_SCAN_FILES - 3, MAX_BATCH_SCAN_BYTES - 7);
        assert_eq!(budget.file_limit, 3);
        assert_eq!(budget.byte_limit, 7);
        assert!(budget.aggregate_file_limited);
        assert!(budget.aggregate_byte_limited);
        assert!(aggregate_search_limit_hit(
            &json!({"truncationReason":"scan_files"}),
            budget,
        ));
        assert!(aggregate_search_limit_hit(
            &json!({"truncationReason":"scan_bytes"}),
            budget,
        ));
        assert!(!aggregate_search_limit_hit(
            &json!({"truncationReason":"max_results"}),
            budget,
        ));
    }

    #[test]
    fn search_rejects_invalid_patterns_and_enforces_bounds() {
        let root = std::env::temp_dir().join(format!(
            "file-search-invalid-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "x\n").unwrap();
        let config = config(&root);
        let resolved = resolve_path(&config, ".", Access::Read).unwrap();
        let empty = Vec::new();
        let invalid_regex = search(SearchOptions {
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
        })
        .unwrap_err();
        assert_eq!(invalid_regex.code, "file_invalid_regex");
        let invalid_glob = search(SearchOptions {
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
        })
        .unwrap_err();
        assert_eq!(invalid_glob.code, "file_invalid_glob");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_diff_preserves_blank_lines_and_emits_disjoint_hunks() {
        let before = "top\n\nold-a\nkeep-1\nkeep-2\nkeep-3\nkeep-4\nkeep-5\nkeep-6\nkeep-7\nkeep-8\nold-b\n\nbottom\n";
        let after = "top\n\nnew-a\nkeep-1\nkeep-2\nkeep-3\nkeep-4\nkeep-5\nkeep-6\nkeep-7\nkeep-8\nnew-b\n\nbottom\n";
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
        assert!(newline.contains(NO_NEWLINE_MARKER));
        assert!(!newline.is_empty());

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

    #[test]
    fn generated_diff_hunks_round_trip_through_patch_parser() {
        let target = Path::new("/workspace/example.txt");
        let before = "first\nkeep-1\nkeep-2\nkeep-3\nkeep-4\nkeep-5\nkeep-6\nkeep-7\nlast";
        let after = "FIRST\nkeep-1\nkeep-2\nkeep-3\nkeep-4\nkeep-5\nkeep-6\nkeep-7\nLAST\n";
        let (diff, truncated, changed) = bounded_diff(before, after);
        assert!(!truncated);
        assert_eq!(changed, json!({"added": 2, "removed": 2}));
        assert_eq!(diff.matches("@@ -").count(), 2);
        let hunks = diff.lines().skip(2).collect::<Vec<_>>().join("\n") + "\n";
        assert_eq!(apply_unified_patch(before, &hunks, target).unwrap(), after);
    }

    #[test]
    fn unified_patch_accepts_standard_zero_ranges_and_newline_markers() {
        let target = Path::new("/workspace/example.txt");
        assert_eq!(
            apply_unified_patch("", "@@ -0,0 +1,1 @@\n+first\n", target).unwrap(),
            "first\n"
        );
        assert_eq!(
            apply_unified_patch("only\n", "@@ -1,1 +0,0 @@\n-only\n", target).unwrap(),
            ""
        );
        assert_eq!(
            apply_unified_patch("one\n", "@@ -1 +1 @@\n-one\n+ONE\n", target).unwrap(),
            "ONE\n"
        );
        assert_eq!(
            apply_unified_patch(
                "one",
                "@@ -1 +1 @@\n-one\n\\ No newline at end of file\n+one\n",
                target,
            )
            .unwrap(),
            "one\n"
        );
        assert_eq!(
            apply_unified_patch(
                "one\n",
                "@@ -1 +1 @@\n-one\n+one\n\\ No newline at end of file\n",
                target,
            )
            .unwrap(),
            "one"
        );
    }

    #[test]
    fn unified_patch_rejects_bare_and_malformed_hunk_ranges() {
        let target = Path::new("/workspace/example.txt");
        for patch in [
            "@@\n-old\n+new\n",
            "@@ -1,x +1,1 @@\n-old\n+new\n",
            "@@ -1,1,1 +1,1 @@\n-old\n+new\n",
            "@@ -0,1 +1,1 @@\n-old\n+new\n",
            "@@ -1,1 +2,1 @@\n-old\n+new\n",
            "@@ -1,1 +1,1 @@\n\\ No newline at end of file\n-old\n+new\n",
            "@@ -1,1 +1,1 @@\n\n",
            "@@ -1,1 +1,1 @@\n你不是合法的 hunk 行\n",
        ] {
            assert_eq!(
                apply_unified_patch("old\n", patch, target)
                    .unwrap_err()
                    .code,
                "file_patch_invalid",
                "patch should be rejected: {patch:?}"
            );
        }
    }

    #[test]
    fn unified_patch_is_exact_single_file_and_preserves_crlf() {
        let target = Path::new("/workspace/src/main.rs");
        let original = "one\r\ntwo\r\nthree\r\n";
        let patch =
            "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n";
        assert_eq!(
            apply_unified_patch(original, patch, target).unwrap(),
            "one\r\nTWO\r\nthree\r\n"
        );
        let fuzzy = "@@ -1,3 +1,3 @@\n one\n-wrong\n+TWO\n three\n";
        assert_eq!(
            apply_unified_patch(original, fuzzy, target)
                .unwrap_err()
                .code,
            "file_patch_conflict"
        );
        let wrong_target = "--- a/other.rs\n+++ b/other.rs\n@@ -1,1 +1,1 @@\n-one\n+ONE\n";
        assert_eq!(
            apply_unified_patch(original, wrong_target, target)
                .unwrap_err()
                .code,
            "file_patch_invalid"
        );
        let multi_file = "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,1 +1,1 @@\n-one\n+ONE\n--- a/src/other.rs\n+++ b/src/other.rs\n@@ -1,1 +1,1 @@\n-two\n+TWO\n";
        assert_eq!(
            apply_unified_patch(original, multi_file, target)
                .unwrap_err()
                .code,
            "file_patch_invalid"
        );
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
}
