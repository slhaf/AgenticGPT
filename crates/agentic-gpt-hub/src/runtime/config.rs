use agentic_gpt_protocol::{
    SafeBuiltinPolicyRules, SafeConfigSummary, SafePathPolicySummary, SafePolicyRules,
    SafeSandboxSummary,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(crate) const REQUEST_TIMEOUT_SECS: u64 = 35;
pub(crate) const MAX_WAIT_SECONDS: u64 = 30;
pub(crate) const DEFAULT_REMOTE_CONFIRM_TIMEOUT_SECS: u64 = 45;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HubConfig {
    pub(crate) remote_confirmation: RemoteConfirmationConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteConfirmationConfig {
    pub(crate) enabled: bool,
    pub(crate) provider: String,
    pub(crate) timeout_seconds: u64,
    pub(crate) ntfy: NtfyConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NtfyConfig {
    pub(crate) server_url: String,
    pub(crate) topic: String,
    pub(crate) callback_base_url: String,
}

pub(crate) fn default_db_path() -> PathBuf {
    dirs_fallback_home()
        .join(".agentic_gpt")
        .join("hub.sqlite3")
}

pub(crate) fn default_config_path() -> PathBuf {
    dirs_fallback_home().join(".agentic_gpt").join("hub.json")
}

fn dirs_fallback_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

impl HubConfig {
    pub(crate) fn default_config() -> Self {
        Self {
            remote_confirmation: RemoteConfirmationConfig {
                enabled: false,
                provider: "ntfy".to_string(),
                timeout_seconds: DEFAULT_REMOTE_CONFIRM_TIMEOUT_SECS,
                ntfy: NtfyConfig {
                    server_url: "https://ntfy.example.invalid".to_string(),
                    topic: "change-me-high-entropy-topic".to_string(),
                    callback_base_url: "https://agentic-gpt.example.invalid".to_string(),
                },
            },
        }
    }

    pub(crate) fn load_or_default(path: &PathBuf) -> Result<Self> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("refusing symlinked hub config path {}", path.display())
            }
            Ok(_) => {
                let text = std::fs::read_to_string(path)
                    .with_context(|| format!("read hub config {}", path.display()))?;
                Ok(serde_json::from_str(&text)
                    .with_context(|| format!("parse hub config {}", path.display()))?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::default_config())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn write_if_missing(&self, path: &PathBuf) -> Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    anyhow::bail!("refusing symlinked hub config path {}", path.display());
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = unique_config_sibling(path);
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true).read(true);
        set_private_file_mode(&mut options);
        let payload = serde_json::to_vec_pretty(self)?;
        {
            let mut file = options.open(&temp)?;
            std::io::Write::write_all(&mut file, &payload)?;
            file.sync_all()?;
        }
        match std::fs::hard_link(&temp, path) {
            Ok(()) => {
                std::fs::File::open(path)?.sync_all()?;
                sync_config_parent(path)?;
                std::fs::remove_file(&temp)?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&temp);
                match std::fs::symlink_metadata(path) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        anyhow::bail!("refusing symlinked hub config path {}", path.display())
                    }
                    Ok(_) => Ok(()),
                    Err(error) => Err(error.into()),
                }
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                Err(error.into())
            }
        }
    }
}

fn unique_config_sibling(path: &std::path::Path) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".tmp.{}.{}", std::process::id(), nonce));
    PathBuf::from(value)
}

fn sync_config_parent(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_file_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_mode(_options: &mut std::fs::OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::default_config_summary;

    #[test]
    fn safe_default_summary_has_no_paths_or_secrets() {
        let summary = default_config_summary();
        assert_eq!(summary.workspace_root, "unknown");
        assert_eq!(summary.sandbox.mode, "unknown");
        assert!(summary.path_policy.write_roots.is_empty());
        assert!(summary.path_policy.read_only_roots.is_empty());
        assert!(summary.path_policy.deny_roots.is_empty());
        assert!(summary.policy_rules.allow.is_empty());
        assert!(summary.policy_rules.confirm.is_empty());
        assert!(summary.policy_rules.deny.is_empty());
        assert!(summary.policy_rules.builtins.confirm.is_empty());
        assert!(summary.policy_rules.builtins.deny.is_empty());
    }
}

pub(crate) fn default_config_summary() -> SafeConfigSummary {
    SafeConfigSummary {
        workspace_root: "unknown".to_string(),
        sandbox: SafeSandboxSummary {
            enabled: false,
            mode: "unknown".to_string(),
        },
        path_policy: SafePathPolicySummary {
            write_root_count: 0,
            read_only_root_count: 0,
            deny_root_count: 0,
            write_roots: Vec::new(),
            read_only_roots: Vec::new(),
            deny_roots: Vec::new(),
        },
        policy_rule_counts: agentic_gpt_protocol::PolicyCounts {
            allow: 0,
            confirm: 0,
            deny: 0,
        },
        policy_rules: SafePolicyRules {
            allow: Vec::new(),
            confirm: Vec::new(),
            deny: Vec::new(),
            builtins: SafeBuiltinPolicyRules {
                confirm: Vec::new(),
                deny: Vec::new(),
            },
        },
        confirmation_provider: "unknown".to_string(),
        tunnel: None,
    }
}
