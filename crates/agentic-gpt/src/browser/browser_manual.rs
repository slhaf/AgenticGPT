use anyhow::{anyhow, Result};
use serde::Serialize;

const MAX_READ_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_SEARCH_OUTPUT_BYTES: usize = 256 * 1024;
const DEFAULT_RESULTS: usize = 50;
const DEFAULT_CONTEXT_LINES: usize = 2;
macro_rules! page {
    ($path:literal) => {
        ($path, include_str!(concat!("../../assets/browser/", $path)))
    };
}

const DOCUMENTS: &[(&str, &str)] = &[
    page!("quickstart.md"),
    page!("session-naming.md"),
    page!("api-use-behavior.md"),
    page!("accessibility.md"),
    page!("tab-claiming-chrome.md"),
    page!("tab-cleanup-chrome.md"),
    page!("browser-safety.md"),
    page!("confirmations.md"),
    page!("local-web-development.md"),
    page!("screenshots.md"),
    page!("connection-troubleshooting.md"),
    page!("browser-troubleshooting.md"),
    page!("browser-control-interruption.md"),
    page!("api.md"),
];

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

pub(crate) fn read(request: ReadRequest) -> Result<ReadResult> {
    let document = DOCUMENTS
        .iter()
        .find_map(|(path, text)| (*path == request.path).then_some(*text))
        .ok_or_else(|| anyhow!("browser_manual_read_failed"))?;
    let start = request.start_line.unwrap_or(1);
    let requested_end = request.end_line.unwrap_or(usize::MAX);
    let mut content = String::new();
    let mut end = start.saturating_sub(1);
    let mut next_start_line = None;
    for (index, line) in document.split_inclusive('\n').enumerate().skip(end) {
        let number = index + 1;
        if number > requested_end {
            break;
        }
        if line.len() > MAX_READ_OUTPUT_BYTES && content.is_empty() {
            return Err(anyhow!("browser_manual_line_too_large"));
        }
        if content.len() + line.len() > MAX_READ_OUTPUT_BYTES {
            next_start_line = Some(number);
            break;
        }
        content.push_str(line);
        end = number;
    }
    Ok(ReadResult {
        path: request.path,
        start_line: start,
        end_line: end,
        content,
        next_start_line,
    })
}

pub(crate) fn search(request: SearchRequest) -> Result<SearchResult> {
    let max_results = request.max_results.unwrap_or(DEFAULT_RESULTS);
    let context = request.context_lines.unwrap_or(DEFAULT_CONTEXT_LINES);
    let mut result = SearchResult {
        matches: Vec::new(),
        scanned_files: 0,
        scanned_bytes: 0,
        skipped: SearchSkipped::default(),
        truncated: false,
    };
    let mut output_bytes = 2;
    for &(path, document) in DOCUMENTS {
        result.scanned_files += 1;
        result.scanned_bytes += document.len() as u64;
        let lines: Vec<_> = document.lines().map(str::to_owned).collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(&request.query) {
                continue;
            }
            let candidate = SearchMatch {
                path: path.to_owned(),
                line: index + 1,
                text: line.clone(),
                before: lines[index.saturating_sub(context)..index].to_vec(),
                after: lines[index + 1..(index + 1 + context).min(lines.len())].to_vec(),
            };
            output_bytes +=
                usize::from(!result.matches.is_empty()) + serde_json::to_vec(&candidate)?.len();
            if output_bytes > MAX_SEARCH_OUTPUT_BYTES {
                result.truncated = true;
                return Ok(result);
            }
            result.matches.push(candidate);
            if result.matches.len() == max_results {
                result.truncated = true;
                return Ok(result);
            }
        }
    }
    Ok(result)
}
