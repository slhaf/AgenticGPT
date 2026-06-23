use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentic_gpt_protocol::{
    PolicyCounts, SafeBuiltinPolicyRules, SafeConfigSummary, SafePathPolicySummary, SafePathRoot,
    SafePolicyRules, SafeRule, SafeSandboxSummary,
};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::mcp::McpServerConfig;
use crate::policy::{builtin_rules, paths_match, PolicyDecision};
use crate::state::RunMode;
use crate::utils::{agentic_home, ensure_parent, hostname_fallback, DEFAULT_BACKUP_LIMIT};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    pub(crate) agent_id: String,
    pub(crate) display_name: String,
    #[serde(alias = "workerUrl")]
    pub(crate) hub_url: String,
    pub(crate) agent_secret: String,
    pub(crate) workspace_root: PathBuf,
    pub(crate) backup_limit: usize,
    pub(crate) confirmation_provider: ConfirmationProviderConfig,
    #[serde(default = "default_confirmation_language")]
    pub(crate) confirmation_language: String,
    pub(crate) sandbox: SandboxConfig,
    #[serde(default)]
    pub(crate) mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    pub(crate) path_policy: PathPolicyConfig,
    pub(crate) policy: PolicyConfig,
    pub(crate) limits: LimitsConfig,
    #[serde(default = "default_room_config")]
    pub(crate) room: RoomConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfirmationProviderConfig {
    pub(crate) provider: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SandboxConfig {
    pub(crate) enabled: bool,
    pub(crate) bubblewrap_path: String,
    pub(crate) required_runtime_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathPolicyConfig {
    #[serde(default)]
    pub(crate) write_roots: Vec<PathBuf>,
    #[serde(default)]
    pub(crate) read_only_roots: Vec<PathBuf>,
    #[serde(default)]
    pub(crate) deny_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PolicyConfig {
    pub(crate) allow: Vec<Rule>,
    pub(crate) confirm: Vec<Rule>,
    pub(crate) deny: Vec<Rule>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rule {
    pub(crate) program: String,
    pub(crate) args_prefix: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LimitsConfig {
    pub(crate) max_concurrent_tasks: usize,
    pub(crate) max_active_sessions: usize,
    pub(crate) session_idle_timeout_secs: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoomConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notebook_root: Option<PathBuf>,
    pub(crate) timezone: String,
    #[serde(default = "default_diary_day_boundary_hour")]
    pub(crate) diary_day_boundary_hour: u32,
}
impl Config {
    pub(crate) fn default_config() -> Result<Self> {
        let base = agentic_home()?;
        Ok(Self {
            agent_id: "laptop".to_string(),
            display_name: hostname_fallback(),
            hub_url: "http://localhost:8787".to_string(),
            agent_secret: "change-me".to_string(),
            workspace_root: base.join("workspace"),
            backup_limit: DEFAULT_BACKUP_LIMIT,
            confirmation_provider: ConfirmationProviderConfig {
                provider: "freedesktop-then-hub".to_string(),
            },
            confirmation_language: default_confirmation_language(),
            mcp_servers: BTreeMap::new(),
            sandbox: SandboxConfig {
                enabled: false,
                bubblewrap_path: "bwrap".to_string(),
                required_runtime_paths: vec![
                    PathBuf::from("/usr"),
                    PathBuf::from("/bin"),
                    PathBuf::from("/lib"),
                    PathBuf::from("/lib64"),
                    PathBuf::from("/etc/ssl"),
                ],
            },
            path_policy: default_path_policy(&base.join("workspace")),
            policy: PolicyConfig::default(),
            limits: LimitsConfig {
                max_concurrent_tasks: 2,
                max_active_sessions: 4,
                session_idle_timeout_secs: 3600,
            },
            room: default_room_config(),
        })
    }

    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let has_path_policy = value.get("pathPolicy").is_some();
        let mut config: Self = serde_json::from_value(value)?;
        if !has_path_policy {
            config.path_policy = default_path_policy(&config.workspace_root);
        }
        Ok(config)
    }

    pub(crate) fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Self::default_config()
        }
    }

    pub(crate) fn ensure_workspace(&self) -> Result<()> {
        fs::create_dir_all(&self.workspace_root)?;
        Ok(())
    }

    pub(crate) fn safe_summary(&self) -> SafeConfigSummary {
        let write_roots = safe_write_roots(self);
        SafeConfigSummary {
            workspace_root: workspace_root_summary(&self.workspace_root),
            sandbox: SafeSandboxSummary {
                enabled: self.sandbox.enabled,
                mode: if self.sandbox.enabled {
                    "bubblewrap"
                } else {
                    "disabled"
                }
                .to_string(),
            },
            path_policy: SafePathPolicySummary {
                write_root_count: write_roots.len(),
                read_only_root_count: self.path_policy.read_only_roots.len(),
                deny_root_count: self.path_policy.deny_roots.len(),
                write_roots,
                read_only_roots: safe_path_roots(
                    &self.path_policy.read_only_roots,
                    &self.workspace_root,
                    "configured",
                ),
                deny_roots: safe_path_roots(
                    &self.path_policy.deny_roots,
                    &self.workspace_root,
                    "configured",
                ),
            },
            policy_rule_counts: PolicyCounts {
                allow: self.policy.allow.len(),
                confirm: self.policy.confirm.len(),
                deny: self.policy.deny.len(),
            },
            policy_rules: SafePolicyRules {
                allow: safe_rules(&self.policy.allow),
                confirm: safe_rules(&self.policy.confirm),
                deny: safe_rules(&self.policy.deny),
                builtins: SafeBuiltinPolicyRules {
                    confirm: safe_rules(&builtin_rules(RunMode::Normal, PolicyDecision::Confirm)),
                    deny: safe_rules(&builtin_rules(RunMode::Normal, PolicyDecision::Deny)),
                },
            },
            confirmation_provider: self.confirmation_provider.provider.clone(),
        }
    }
}

fn workspace_root_summary(workspace_root: &Path) -> String {
    if *workspace_root == agentic_home().unwrap_or_default().join("workspace") {
        "default".to_string()
    } else {
        workspace_root
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("workspace:{name}"))
            .unwrap_or_else(|| "configured".to_string())
    }
}

fn safe_write_roots(config: &Config) -> Vec<SafePathRoot> {
    let mut has_workspace_root = false;
    let mut roots = config
        .path_policy
        .write_roots
        .iter()
        .map(|root| {
            let is_workspace_root = paths_match(root, &config.workspace_root);
            has_workspace_root |= is_workspace_root;
            SafePathRoot {
                path: safe_path_display(root, &config.workspace_root),
                source: if is_workspace_root {
                    "workspaceRoot"
                } else {
                    "configured"
                }
                .to_string(),
            }
        })
        .collect::<Vec<_>>();
    if !has_workspace_root {
        roots.insert(
            0,
            SafePathRoot {
                path: "workspace".to_string(),
                source: "workspaceRoot".to_string(),
            },
        );
    }
    roots
}

fn safe_path_roots(roots: &[PathBuf], workspace_root: &Path, source: &str) -> Vec<SafePathRoot> {
    roots
        .iter()
        .map(|root| SafePathRoot {
            path: safe_path_display(root, workspace_root),
            source: source.to_string(),
        })
        .collect()
}

fn safe_path_display(path: &Path, workspace_root: &Path) -> String {
    if paths_match(path, workspace_root) {
        return "workspace".to_string();
    }
    let raw = path.to_string_lossy().to_string();
    if raw == "~" || raw.starts_with("~/") {
        return raw;
    }
    if let Some(home) = dirs::home_dir() {
        if let Ok(relative) = path.strip_prefix(&home) {
            if relative.as_os_str().is_empty() {
                return "~".to_string();
            }
            return format!("~/{}", relative.to_string_lossy());
        }
    }
    raw
}

fn safe_rules(rules: &[Rule]) -> Vec<SafeRule> {
    rules
        .iter()
        .map(|rule| SafeRule {
            program: rule.program.clone(),
            args_prefix: rule.args_prefix.clone(),
        })
        .collect()
}
pub(crate) fn default_room_config() -> RoomConfig {
    RoomConfig {
        notebook_root: None,
        timezone: "Asia/Shanghai".to_string(),
        diary_day_boundary_hour: default_diary_day_boundary_hour(),
    }
}

pub(crate) fn default_diary_day_boundary_hour() -> u32 {
    5
}

pub(crate) fn default_confirmation_language() -> String {
    "zh-CN".to_string()
}

pub(crate) fn normalize_confirmation_language(language: &str) -> String {
    match language {
        "zh" | "zh-CN" | "zh_CN" | "cn" | "中文" => "zh-CN".to_string(),
        "en" | "en-US" | "en_US" | "English" | "english" => "en".to_string(),
        other => {
            if other.to_lowercase().starts_with("zh") {
                "zh-CN".to_string()
            } else {
                "en".to_string()
            }
        }
    }
}

pub(crate) fn confirmation_language_is_zh(config: &Config) -> bool {
    normalize_confirmation_language(&config.confirmation_language) == "zh-CN"
}

pub(crate) fn default_path_policy(workspace_root: &Path) -> PathPolicyConfig {
    PathPolicyConfig {
        write_roots: vec![
            workspace_root.to_path_buf(),
            PathBuf::from("~/Documents"),
            PathBuf::from("~/Downloads"),
            PathBuf::from("/tmp"),
        ],
        read_only_roots: vec![
            PathBuf::from("~/.cache"),
            PathBuf::from("~/.local/share"),
            PathBuf::from("/etc/os-release"),
            PathBuf::from("/proc/meminfo"),
            PathBuf::from("/proc/cpuinfo"),
            PathBuf::from("/proc/loadavg"),
            PathBuf::from("/proc/uptime"),
            PathBuf::from("/sys/class/power_supply"),
            PathBuf::from("/sys/class/thermal"),
        ],
        deny_roots: vec![
            PathBuf::from("~/.ssh"),
            PathBuf::from("~/.gnupg"),
            PathBuf::from("~/.local/share/keyrings"),
            PathBuf::from("~/.password-store"),
            PathBuf::from("~/.mozilla"),
            PathBuf::from("~/.config/google-chrome"),
            PathBuf::from("~/.config/chromium"),
            PathBuf::from("~/.config/Code/User/globalStorage"),
            PathBuf::from("~/.npmrc"),
            PathBuf::from("~/.pypirc"),
            PathBuf::from("~/.cargo/credentials"),
            PathBuf::from("~/.docker/config.json"),
            PathBuf::from("~/.kube"),
            PathBuf::from("~/.aws"),
            PathBuf::from("~/.config/gcloud"),
            PathBuf::from("~/.config/gh"),
            PathBuf::from("~/.config/hub"),
            PathBuf::from("~/.config/clash"),
            PathBuf::from("~/.config/clash-verge"),
        ],
    }
}
pub(crate) fn write_config_with_backup(path: &Path, config: &Config) -> Result<()> {
    ensure_parent(path)?;
    if path.exists() {
        let backup_dir = path.parent().unwrap().join("backups");
        fs::create_dir_all(&backup_dir)?;
        let backup = backup_dir.join(format!("config.{}.json", Utc::now().timestamp_millis()));
        fs::copy(path, backup)?;
        prune_backups(&backup_dir, config.backup_limit)?;
    }
    fs::write(path, serde_json::to_string_pretty(config)?)?;
    Ok(())
}

fn prune_backups(dir: &Path, limit: usize) -> Result<()> {
    let mut entries = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("config."))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).ok());
    while entries.len() > limit {
        if let Some(entry) = entries.first() {
            fs::remove_file(entry.path())?;
        }
        entries.remove(0);
    }
    Ok(())
}
