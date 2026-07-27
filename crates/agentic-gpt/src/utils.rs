use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};

pub(crate) const DEFAULT_BACKUP_LIMIT: usize = 5;
pub(crate) const CONFIRM_TIMEOUT_SECS: u64 = 45;
pub(crate) const JOB_TAIL_MAX: usize = 64 * 1024;
pub(crate) const RECONNECT_DELAY_SECS: u64 = 3;
pub(crate) const CONNECT_TIMEOUT_SECS: u64 = 20;
pub(crate) const HEARTBEAT_INTERVAL_SECS: u64 = 15;
pub(crate) const HEARTBEAT_ACK_TIMEOUT_SECS: u64 = 45;
const MCP_ARGUMENT_KEY_LIMIT: usize = 32;
const MCP_ARGUMENT_KEY_CHAR_LIMIT: usize = 80;

pub(crate) fn bounded_mcp_argument_keys(
    arguments: &serde_json::Value,
) -> (Vec<String>, usize, bool) {
    let mut keys = arguments
        .as_object()
        .map(|arguments| arguments.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    let total = keys.len();
    let mut truncated = total > MCP_ARGUMENT_KEY_LIMIT;
    keys.truncate(MCP_ARGUMENT_KEY_LIMIT);
    for key in &mut keys {
        let bounded = truncate_chars(key, MCP_ARGUMENT_KEY_CHAR_LIMIT);
        truncated |= bounded != *key;
        *key = bounded;
    }
    (keys, total, truncated)
}

pub(crate) fn mcp_tool_command_preview(
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let bytes = serde_json::to_vec(arguments).unwrap_or_default();
    let (keys, total_keys, keys_truncated) = bounded_mcp_argument_keys(arguments);
    let sha256 = format!("sha256:{:x}", Sha256::digest(&bytes));
    format!(
        "MCP Tool Call\nServer: {server_id}\nTool: {tool_name}\nArgument keys (showing {} of {total_keys}, truncated={keys_truncated}): [{}]\nArgument bytes: {}\nArgument SHA-256: {sha256}",
        keys.len(),
        keys.join(", "),
        bytes.len()
    )
}

pub(crate) fn risk_level(program: &str) -> String {
    if risky_file_mutation(program) {
        "HIGH"
    } else if matches!(
        program,
        "curl" | "wget" | "docker" | "systemctl" | "service"
    ) {
        "MEDIUM"
    } else {
        "LOW"
    }
    .to_string()
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

pub(crate) fn risky_file_mutation(program: &str) -> bool {
    matches!(
        program,
        "rm" | "mv" | "chmod" | "chown" | "git" | "python" | "node"
    )
}

pub(crate) fn config_path(path: Option<PathBuf>) -> PathBuf {
    path.unwrap_or_else(|| {
        agentic_home()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("config.json")
    })
}

pub(crate) fn agentic_home() -> Result<PathBuf> {
    let home = dirs::home_dir().context("home directory not found")?;
    Ok(home.join(".agentic_gpt"))
}

pub(crate) fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub(crate) fn hostname_fallback() -> String {
    std::env::var("HOSTNAME").unwrap_or_else(|_| "agentic-gpt-linux".to_string())
}

pub(crate) fn command_preview(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .map(|part| {
            if part.contains(char::is_whitespace) {
                format!("{part:?}")
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn log_info(message: String) {
    log_line("INFO", message);
}

pub(crate) fn log_warn(message: String) {
    log_line("WARN", message);
}

pub(crate) fn log_error(message: String) {
    log_line("ERROR", message);
}

pub(crate) fn log_line(level: &str, message: String) {
    eprintln!(
        "{}",
        render_log_line(level, &message, journald_active(), Utc::now())
    );
}

fn journald_active() -> bool {
    std::env::var_os("JOURNAL_STREAM").is_some() || std::env::var_os("INVOCATION_ID").is_some()
}

fn render_log_line(
    level: &str,
    message: &str,
    journald: bool,
    timestamp: chrono::DateTime<Utc>,
) -> String {
    if journald {
        format!("{level} {message}")
    } else {
        format!("{} {level} {message}", timestamp.to_rfc3339())
    }
}

pub(crate) fn compact_id(value: &str) -> String {
    let body = value
        .rsplit_once('_')
        .map(|(_, body)| body)
        .unwrap_or(value);
    let prefix = body.chars().take(12).collect::<String>();
    if prefix.chars().count() == 12
        && prefix
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return prefix.to_ascii_lowercase();
    }

    let hash = value.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{:012x}", hash & 0x0000_ffff_ffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_rendering_omits_inner_timestamp_only_in_journal_mode() {
        let timestamp = Utc::now();
        let journal = render_log_line("INFO", "component: ready", true, timestamp);
        assert_eq!(journal, "INFO component: ready");

        let foreground = render_log_line("INFO", "component: ready", false, timestamp);
        assert!(foreground.starts_with(&format!("{} INFO ", timestamp.to_rfc3339())));
    }

    #[test]
    fn mcp_confirmation_preview_is_sorted_bounded_metadata_without_values() {
        let preview = mcp_tool_command_preview(
            "server",
            "tool",
            &serde_json::json!({
                "zeta": "super-secret-value",
                "alpha": 1
            }),
        );
        assert!(preview.contains("Argument keys (showing 2 of 2, truncated=false): [alpha, zeta]"));
        assert!(preview.contains("Argument bytes:"));
        assert!(preview.contains("Argument SHA-256: sha256:"));
        assert!(!preview.contains("super-secret-value"));
        assert!(!preview.contains("\"alpha\":1"));
    }

    #[test]
    fn mcp_argument_key_summary_is_counted_and_bounded() {
        let mut arguments = serde_json::Map::new();
        for index in 0..40 {
            arguments.insert(
                format!("key-{index:02}-{}", "x".repeat(100)),
                serde_json::json!("hidden-value"),
            );
        }
        let value = serde_json::Value::Object(arguments);
        let (keys, total, truncated) = bounded_mcp_argument_keys(&value);
        assert_eq!(total, 40);
        assert_eq!(keys.len(), 32);
        assert!(truncated);
        assert!(keys.iter().all(|key| key.chars().count() <= 83));
        let preview = mcp_tool_command_preview("server", "tool", &value);
        assert!(preview.contains("showing 32 of 40, truncated=true"));
        assert!(!preview.contains("hidden-value"));
        assert!(preview.len() < 4_096);
    }

    #[test]
    fn compact_id_has_a_stable_twelve_hex_digit_body() {
        assert_eq!(compact_id("run_0123456789abcdef"), "0123456789ab");
        let short = compact_id("job");
        assert_eq!(short.len(), 12);
        assert!(short.chars().all(|character| character.is_ascii_hexdigit()));
        assert_eq!(short, compact_id("job"));
    }
}
