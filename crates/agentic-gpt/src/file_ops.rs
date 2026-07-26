use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::OwnedMutexGuard;

use crate::config::Config;
use crate::exec;
use crate::state::AppState;

pub(crate) const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_READ_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_LINE_DISPLAY_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ResolvedPath {
    pub(crate) input: String,
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
}
