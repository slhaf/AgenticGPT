use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::OwnedMutexGuard;

use crate::config::Config;
use crate::exec;
use crate::state::AppState;

pub(crate) const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_READ_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_LINE_DISPLAY_BYTES: usize = 4 * 1024;
pub(crate) const MAX_SEARCH_FILES: usize = 10_000;
pub(crate) const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_SEARCH_RESULTS: usize = 200;
pub(crate) const MAX_SEARCH_CONTEXT: usize = 5;
pub(crate) const MAX_SEARCH_OUTPUT_BYTES: usize = 256 * 1024;

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
        locks
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    lock.lock_owned().await
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
    let bytes = fs::read(&resolved.path)
        .map_err(|_| FileError::new("file_read_failed", "file could not be read"))?;
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
}

pub(crate) fn search(options: SearchOptions<'_>) -> std::result::Result<Value, FileError> {
    if options.query.trim().is_empty() {
        return Err(FileError::new(
            "file_search_query_empty",
            "query must not be empty",
        ));
    }
    if options.context_lines > MAX_SEARCH_CONTEXT {
        return Err(FileError::new(
            "file_context_limit_exceeded",
            "contextLines exceeds the bound",
        ));
    }
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
        .build()
        .collect::<Vec<_>>();
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
        scanned_files += 1;
        if scanned_files > MAX_SEARCH_FILES {
            truncated = true;
            truncation_reason = Some("scan_files");
            break;
        }
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
        if scanned_bytes.saturating_add(metadata.len()) > MAX_SEARCH_BYTES {
            truncated = true;
            truncation_reason = Some("scan_bytes");
            break;
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
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
                let before = (index.saturating_sub(options.context_lines)..index)
                    .map(|line_index| line_display(lines[line_index]))
                    .collect::<Vec<_>>();
                let after = ((index + 1)
                    ..=(index + options.context_lines).min(lines.len().saturating_sub(1)))
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
                let candidate_bytes = serde_json::to_vec(&value).unwrap_or_default().len();
                let current_bytes = serde_json::to_vec(&matches).unwrap_or_default().len();
                if matches.len() >= options.max_results {
                    truncated = true;
                    truncation_reason = Some("max_results");
                    break;
                }
                if current_bytes.saturating_add(candidate_bytes) > MAX_SEARCH_OUTPUT_BYTES {
                    truncated = true;
                    truncation_reason = Some("output_bytes");
                    break;
                }
                matches.push(value);
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
        "matches": matches,
        "matchCount": match_count,
        "scannedFiles": scanned_files.min(MAX_SEARCH_FILES),
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
    let mut line_number = 1;
    let mut returned_through = None;
    let mut truncated = false;
    let mut last_complete = true;
    for line in text.split_inclusive('\n') {
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
        line_number += 1;
    }
    let complete_end = returned_through == Some(end) || returned_through == Some(line_count(text));
    if !complete_end && returned_through.is_some() && !last_complete {
        truncated = true;
    } else if returned_through.is_some() && returned_through.unwrap_or(0) < end {
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
    let record = crate::audit::FileAuditRecord {
        time: Utc::now(),
        tool: "file.edit".to_string(),
        action: "edit".to_string(),
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
        let bytes = fs::read(&target.path)
            .map_err(|_| FileError::new("file_read_failed", "target could not be read"))?;
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
            if content.as_bytes().len() > MAX_FILE_BYTES as usize {
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
        let current = fs::read(&target.path).map_err(|_| {
            FileError::new("file_revision_conflict", "target changed before commit")
        })?;
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
            Ok(()) => fs::remove_file(&temp),
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
        if file.write_all(bytes).is_err() || file.sync_all().is_err() {
            let _ = fs::remove_file(&temp);
            return Err(FileError::new(
                "file_write_failed",
                "temporary file could not be synced",
            ));
        }
        if let Some(permissions) = permissions {
            let _ = fs::set_permissions(&temp, permissions.clone());
        }
        return Ok(temp);
    }
    Err(FileError::new(
        "file_write_failed",
        "temporary file could not be created",
    ))
}

fn sync_parent(target: &Path) {
    if let Some(parent) = target.parent() {
        if let Ok(file) = fs::File::open(parent) {
            let _ = file.sync_all();
        }
    }
}

fn bounded_diff(before: &str, after: &str) -> (String, bool, Value) {
    let before_lines = logical_lines(before);
    let after_lines = logical_lines(after);
    let mut prefix = 0;
    while prefix < before_lines.len()
        && prefix < after_lines.len()
        && before_lines[prefix] == after_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < before_lines.len().saturating_sub(prefix)
        && suffix < after_lines.len().saturating_sub(prefix)
        && before_lines[before_lines.len() - 1 - suffix]
            == after_lines[after_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let removed = before_lines.len().saturating_sub(prefix + suffix);
    let added = after_lines.len().saturating_sub(prefix + suffix);
    let mut diff = format!(
        "--- before\n+++ after\n@@ -{},{} +{},{} @@\n",
        prefix + 1,
        removed,
        prefix + 1,
        added
    );
    for line in before_lines.iter().skip(prefix).take(removed) {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in after_lines.iter().skip(prefix).take(added) {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }
    let truncated = diff.len() > 64 * 1024;
    if truncated {
        diff = utf8_prefix(&diff, 64 * 1024).to_string();
    }
    (diff, truncated, json!({"added": added, "removed": removed}))
}

fn logical_lines(text: &str) -> Vec<String> {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    text.split(newline)
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
        .take_while(|line| !(line.is_empty() && text.ends_with(newline)))
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
    while index < patch_lines.len() {
        let header = patch_lines[index];
        if !header.starts_with("@@ ") {
            return Err(FileError::new(
                "file_patch_invalid",
                "expected a unified hunk",
            ));
        }
        let (old_start, old_len, new_len) = parse_hunk_header(header)?;
        index += 1;
        let start = old_start
            .checked_sub(1)
            .and_then(|base| (base as isize).checked_add(offset))
            .filter(|start| *start >= 0)
            .map(|start| start as usize)
            .ok_or_else(|| FileError::new("file_patch_conflict", "hunk location does not match"))?;
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
        while index < patch_lines.len() && !patch_lines[index].starts_with("@@ ") {
            let line = patch_lines[index];
            index += 1;
            let (kind, content) = line.split_at(1);
            match kind {
                " " => {
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
                "-" => {
                    if lines.get(cursor).map(String::as_str) != Some(content) {
                        return Err(FileError::new(
                            "file_patch_conflict",
                            "removed text does not match",
                        ));
                    }
                    cursor += 1;
                    consumed_old += 1;
                }
                "+" => {
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
    Ok(lines.join(newline)
        + if original.ends_with(newline) {
            newline
        } else {
            ""
        })
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

fn parse_hunk_header(header: &str) -> std::result::Result<(usize, usize, usize), FileError> {
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
    let (old_start, old_len) = parse_range(old)?;
    let (_, new_len) = parse_range(new)?;
    Ok((old_start, old_len, new_len))
}

fn parse_range(value: &str) -> std::result::Result<(usize, usize), FileError> {
    let mut values = value.split(',');
    let start = values
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| FileError::new("file_patch_invalid", "invalid hunk range"))?;
    if start == 0 {
        return Err(FileError::new(
            "file_patch_invalid",
            "hunk range starts at zero",
        ));
    }
    let len = values
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    Ok((start, len))
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
        })
        .unwrap();
        assert_eq!(regex["matchCount"], 1);
        assert_eq!(regex["truncated"], true);
        assert_eq!(regex["truncationReason"], "max_results");
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
        })
        .unwrap_err();
        assert_eq!(invalid_glob.code, "file_invalid_glob");
        let _ = fs::remove_dir_all(root);
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
    }
}
