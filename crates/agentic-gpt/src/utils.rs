use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

pub(crate) const DEFAULT_BACKUP_LIMIT: usize = 5;
pub(crate) const EXEC_TIMEOUT_SECS: u64 = 30;
pub(crate) const CONFIRM_TIMEOUT_SECS: u64 = 45;
pub(crate) const STDOUT_MAX: usize = 64 * 1024;
pub(crate) const STDERR_MAX: usize = 64 * 1024;
pub(crate) const SESSION_TAIL_MAX: usize = 64 * 1024;
pub(crate) const RECONNECT_DELAY_SECS: u64 = 3;
pub(crate) const CONNECT_TIMEOUT_SECS: u64 = 20;
pub(crate) const HEARTBEAT_INTERVAL_SECS: u64 = 15;
pub(crate) const HEARTBEAT_ACK_TIMEOUT_SECS: u64 = 45;

pub(crate) fn mcp_tool_command_preview(
    server_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> String {
    let arguments = serde_json::to_string_pretty(arguments)
        .unwrap_or_else(|_| serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_string()));
    format!(
        "MCP Tool Call\nServer: {server_id}\nTool: {tool_name}\nArguments:\n{}",
        truncate_chars(&arguments, 2000)
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
    fn compact_id_has_a_stable_twelve_hex_digit_body() {
        assert_eq!(compact_id("run_0123456789abcdef"), "0123456789ab");
        let short = compact_id("session");
        assert_eq!(short.len(), 12);
        assert!(short.chars().all(|character| character.is_ascii_hexdigit()));
        assert_eq!(short, compact_id("session"));
    }
}
