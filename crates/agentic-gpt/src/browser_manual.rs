use anyhow::{anyhow, Result};
use serde::Serialize;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_READ_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_SEARCH_FILES: usize = 512;
const MAX_SEARCH_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RESULTS: usize = 100;
const MAX_CONTEXT_LINES: usize = 5;
const DEFAULT_RESULTS: usize = 50;
const DEFAULT_CONTEXT_LINES: usize = 2;
const MAX_QUERY_BYTES: usize = 4 * 1024;
const MAX_SEARCH_OUTPUT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Default)]
pub(crate) struct ReadRequest {
    pub(crate) path: String,
    pub(crate) start_line: Option<usize>,
    pub(crate) end_line: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SearchRequest {
    pub(crate) query: String,
    pub(crate) max_results: Option<usize>,
    pub(crate) context_lines: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadResult {
    pub(crate) path: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_start_line: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchMatch {
    pub(crate) path: String,
    pub(crate) line: usize,
    pub(crate) text: String,
    pub(crate) before: Vec<String>,
    pub(crate) after: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchSkipped {
    pub(crate) too_large: usize,
    pub(crate) non_utf8: usize,
    pub(crate) symlink: usize,
    pub(crate) unreadable: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchResult {
    pub(crate) matches: Vec<SearchMatch>,
    pub(crate) scanned_files: usize,
    pub(crate) scanned_bytes: u64,
    pub(crate) skipped: SearchSkipped,
    pub(crate) truncated: bool,
}

pub(crate) fn read(docs_root: &Path, request: ReadRequest) -> Result<ReadResult> {
    let relative = validate_relative_path(&request.path)?;
    validate_range(request.start_line, request.end_line)?;
    let root = checked_root(docs_root)?;
    let path = checked_child(&root, &relative)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| anyhow!("browser_manual_read_failed"))?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow!("browser_manual_symlink_rejected"));
    }
    if !metadata.is_file() {
        return Err(anyhow!("browser_manual_not_a_file"));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(anyhow!("browser_manual_file_too_large"));
    }
    let bytes = fs::read(&path).map_err(|_| anyhow!("browser_manual_read_failed"))?;
    let text = String::from_utf8(bytes).map_err(|_| anyhow!("browser_manual_non_utf8"))?;
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    let start = request.start_line.unwrap_or(1);
    let requested_end = request.end_line.unwrap_or(usize::MAX);
    let mut content = String::new();
    let mut returned_end = start.saturating_sub(1);
    let mut next = None;
    for (index, line) in lines.iter().enumerate().skip(start.saturating_sub(1)) {
        let number = index + 1;
        if number > requested_end {
            break;
        }
        if line.len() > MAX_READ_OUTPUT_BYTES && content.is_empty() {
            return Err(anyhow!("browser_manual_line_too_large"));
        }
        if content.len().saturating_add(line.len()) > MAX_READ_OUTPUT_BYTES {
            next = Some(number);
            break;
        }
        content.push_str(line);
        returned_end = number;
    }
    Ok(ReadResult {
        path: relative_to_string(&relative)?,
        start_line: start,
        end_line: returned_end,
        content,
        next_start_line: next,
    })
}

pub(crate) fn search(docs_root: &Path, request: SearchRequest) -> Result<SearchResult> {
    if request.query.trim().is_empty() || request.query.len() > MAX_QUERY_BYTES {
        return Err(anyhow!("browser_manual_invalid_query"));
    }
    let max_results = request.max_results.unwrap_or(DEFAULT_RESULTS);
    if !(1..=MAX_RESULTS).contains(&max_results) {
        return Err(anyhow!("browser_manual_invalid_bounds"));
    }
    let context = request.context_lines.unwrap_or(DEFAULT_CONTEXT_LINES);
    if context > MAX_CONTEXT_LINES {
        return Err(anyhow!("browser_manual_invalid_bounds"));
    }
    let root = checked_root(docs_root)?;
    let mut files = Vec::new();
    let mut skipped = SearchSkipped::default();
    collect_files(&root, &mut files, &mut skipped)?;
    files.sort();
    let mut matches = Vec::new();
    let mut scanned_files = 0;
    let mut scanned_bytes = 0;
    let mut truncated = false;
    for path in files {
        if scanned_files >= MAX_SEARCH_FILES || scanned_bytes >= MAX_SEARCH_BYTES {
            truncated = true;
            break;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(_) => {
                skipped.unreadable += 1;
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            skipped.symlink += 1;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() > MAX_FILE_BYTES {
            skipped.too_large += 1;
            continue;
        }
        if scanned_bytes.saturating_add(metadata.len()) > MAX_SEARCH_BYTES {
            truncated = true;
            break;
        }
        let bytes = match fs::read(&path) {
            Ok(value) => value,
            Err(_) => {
                skipped.unreadable += 1;
                continue;
            }
        };
        scanned_files += 1;
        scanned_bytes += bytes.len() as u64;
        let text = match String::from_utf8(bytes) {
            Ok(value) => value,
            Err(_) => {
                skipped.non_utf8 += 1;
                continue;
            }
        };
        let lines = text.lines().map(ToString::to_string).collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(&request.query) {
                continue;
            }
            let candidate = SearchMatch {
                path: relative_to_string(path.strip_prefix(&root).unwrap_or(&path))?,
                line: index + 1,
                text: line.clone(),
                before: lines[index.saturating_sub(context)..index].to_vec(),
                after: lines[index + 1..(index + 1 + context).min(lines.len())].to_vec(),
            };
            let mut trial = matches.clone();
            trial.push(candidate);
            let size = serde_json::to_vec(&trial)
                .map_err(|_| anyhow!("browser_manual_search_failed"))?
                .len();
            if trial.len() > max_results {
                truncated = true;
                break;
            }
            if size > MAX_SEARCH_OUTPUT_BYTES {
                truncated = true;
                break;
            }
            matches.push(trial.pop().expect("candidate was pushed"));
        }
        if matches.len() >= max_results || truncated {
            if matches.len() >= max_results {
                truncated = true;
            }
            break;
        }
    }
    Ok(SearchResult {
        matches,
        scanned_files,
        scanned_bytes,
        skipped,
        truncated,
    })
}

fn checked_root(root: &Path) -> Result<PathBuf> {
    let metadata =
        fs::symlink_metadata(root).map_err(|_| anyhow!("browser_manual_docs_unavailable"))?;
    if metadata.file_type().is_symlink() {
        return Err(anyhow!("browser_manual_symlink_rejected"));
    }
    if !metadata.is_dir() {
        return Err(anyhow!("browser_manual_docs_not_directory"));
    }
    Ok(root.to_path_buf())
}

fn validate_relative_path(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if value.is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(anyhow!("browser_manual_invalid_path"));
    }
    Ok(path.to_path_buf())
}

fn validate_range(start: Option<usize>, end: Option<usize>) -> Result<()> {
    if start == Some(0) || end == Some(0) || matches!((start, end), (Some(a), Some(b)) if a > b) {
        return Err(anyhow!("browser_manual_invalid_range"));
    }
    Ok(())
}

fn checked_child(root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(anyhow!("browser_manual_invalid_path"));
        };
        path.push(part);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| anyhow!("browser_manual_read_failed"))?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!("browser_manual_symlink_rejected"));
        }
    }
    Ok(path)
}

fn relative_to_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| anyhow!("browser_manual_invalid_path"))
}

fn collect_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    skipped: &mut SearchSkipped,
) -> Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => {
            skipped.unreadable += 1;
            return Ok(());
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped.unreadable += 1;
                continue;
            }
        };
        let path = entry.path();
        let kind = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                skipped.unreadable += 1;
                continue;
            }
        };
        if kind.file_type().is_symlink() {
            skipped.symlink += 1;
            continue;
        }
        if kind.is_dir() {
            collect_files(&path, files, skipped)?;
        } else if kind.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static IDS: AtomicUsize = AtomicUsize::new(0);
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "agentic-browser-manual-{}-{}",
            std::process::id(),
            IDS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn read_ranges_and_rejects_bad_paths() {
        let root = fixture();
        fs::write(root.join("a.md"), "one\ntwo\nthree\n").unwrap();
        let result = read(
            &root,
            ReadRequest {
                path: "a.md".into(),
                start_line: Some(2),
                end_line: Some(3),
            },
        )
        .unwrap();
        assert_eq!(result.content, "two\nthree\n");
        assert_eq!(result.end_line, 3);
        assert!(!result.path.contains(root.to_str().unwrap()));
        for path in ["", ".", "../a.md", "/tmp/a.md"] {
            assert_eq!(
                read(
                    &root,
                    ReadRequest {
                        path: path.into(),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .to_string(),
                "browser_manual_invalid_path"
            );
        }
        cleanup(&root);
    }

    #[test]
    fn search_is_literal_nested_and_sorted() {
        let root = fixture();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("z.md"), "before\nneedle\nafter\n").unwrap();
        fs::write(root.join("nested/a.md"), "needle\n").unwrap();
        let result = search(
            &root,
            SearchRequest {
                query: "needle".into(),
                max_results: None,
                context_lines: Some(1),
            },
        )
        .unwrap();
        assert_eq!(
            result
                .matches
                .iter()
                .map(|m| m.path.as_str())
                .collect::<Vec<_>>(),
            vec!["nested/a.md", "z.md"]
        );
        assert_eq!(result.matches[1].before, vec!["before"]);
        assert_eq!(result.matches[1].after, vec!["after"]);
        cleanup(&root);
    }

    #[test]
    fn read_reports_continuation_and_rejects_giant_line() {
        let root = fixture();
        let content = format!(
            "{}\n{}\n",
            "x".repeat(MAX_READ_OUTPUT_BYTES / 2),
            "y".repeat(MAX_READ_OUTPUT_BYTES / 2)
        );
        fs::write(root.join("large.md"), content).unwrap();
        let result = read(
            &root,
            ReadRequest {
                path: "large.md".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.next_start_line, Some(2));
        assert!(result.content.len() <= MAX_READ_OUTPUT_BYTES);
        fs::write(root.join("giant.md"), vec![b'x'; MAX_READ_OUTPUT_BYTES + 1]).unwrap();
        assert_eq!(
            read(
                &root,
                ReadRequest {
                    path: "giant.md".into(),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string(),
            "browser_manual_line_too_large"
        );
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_root_component_and_final_symlinks() {
        use std::os::unix::fs::symlink;
        let root = fixture();
        fs::create_dir(root.join("dir")).unwrap();
        fs::write(root.join("dir/file.md"), "safe").unwrap();
        symlink(root.join("dir"), root.join("dir-link")).unwrap();
        symlink(root.join("dir/file.md"), root.join("file-link.md")).unwrap();
        for path in ["dir-link/file.md", "file-link.md"] {
            assert_eq!(
                read(
                    &root,
                    ReadRequest {
                        path: path.into(),
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .to_string(),
                "browser_manual_symlink_rejected"
            );
        }
        let root_link = root.with_extension("root-link");
        symlink(&root, &root_link).unwrap();
        assert_eq!(
            read(
                &root_link,
                ReadRequest {
                    path: "dir/file.md".into(),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string(),
            "browser_manual_symlink_rejected"
        );
        cleanup(&root);
        let _ = fs::remove_file(root_link);
    }

    #[test]
    fn search_rejects_query_and_context_bounds() {
        let root = fixture();
        assert_eq!(
            search(
                &root,
                SearchRequest {
                    query: " ".into(),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string(),
            "browser_manual_invalid_query"
        );
        assert_eq!(
            search(
                &root,
                SearchRequest {
                    query: "x".into(),
                    max_results: Some(0),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string(),
            "browser_manual_invalid_bounds"
        );
        assert_eq!(
            search(
                &root,
                SearchRequest {
                    query: "x".into(),
                    context_lines: Some(MAX_CONTEXT_LINES + 1),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string(),
            "browser_manual_invalid_bounds"
        );
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn search_skips_symlink_and_non_utf8() {
        use std::os::unix::fs::symlink;
        let root = fixture();
        fs::write(root.join("ok.md"), "needle").unwrap();
        fs::write(root.join("bad.md"), [0xff, 0xfe]).unwrap();
        symlink(root.join("ok.md"), root.join("link.md")).unwrap();
        let result = search(
            &root,
            SearchRequest {
                query: "needle".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.skipped.symlink, 1);
        assert_eq!(result.skipped.non_utf8, 1);
        cleanup(&root);
    }
}
