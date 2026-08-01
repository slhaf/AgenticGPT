use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use agentic_gpt_protocol::{
    PolicyCounts, SafeBuiltinPolicyRules, SafeConfigSummary, SafePathPolicySummary, SafePathRoot,
    SafePolicyRules, SafeRule, SafeSandboxSummary, SafeTunnelSummary,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::de::{self, Visitor};
use serde::{ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::mcp::McpServerConfig;
use crate::policy::{builtin_rules, paths_match, PolicyDecision};
use crate::state::CapabilityProfile;
use crate::utils::{
    agentic_home, ensure_parent, hostname_fallback, log_warn, DEFAULT_BACKUP_LIMIT,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    pub(crate) agent_id: String,
    pub(crate) display_name: String,
    #[serde(alias = "workerUrl")]
    pub(crate) hub_url: String,
    #[serde(default = "default_hub_transport")]
    pub(crate) hub_transport: String,
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
    #[serde(default)]
    pub(crate) skills: RoomSkillsConfig,
    #[serde(default = "default_room_config")]
    pub(crate) room: RoomConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tunnel: Option<TunnelConfig>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfirmationChannel {
    Freedesktop,
    Ntfy,
}

impl ConfirmationChannel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Freedesktop => "freedesktop",
            Self::Ntfy => "ntfy",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConfirmationProviderConfig {
    pub(crate) channels: Vec<ConfirmationChannel>,
}

impl ConfirmationProviderConfig {
    pub(crate) fn default_channels() -> Vec<ConfirmationChannel> {
        vec![ConfirmationChannel::Freedesktop, ConfirmationChannel::Ntfy]
    }

    pub(crate) fn from_legacy(value: &str) -> Result<Self, String> {
        let normalized = value.trim();
        let channels = match normalized {
            "none" => Vec::new(),
            "freedesktop" => vec![ConfirmationChannel::Freedesktop],
            "hub" | "ntfy" => vec![ConfirmationChannel::Ntfy],
            "freedesktop-then-hub" | "freedesktopThenHub" | "freedesktop-then-ntfy" | "default" => {
                Self::default_channels()
            }
            other => return Err(format!("unknown confirmation channel: {other}")),
        };
        Ok(Self { channels })
    }

    pub(crate) fn from_channels(channels: Vec<ConfirmationChannel>) -> Result<Self, String> {
        if channels
            .iter()
            .enumerate()
            .any(|(index, channel)| channels[..index].contains(channel))
        {
            return Err("duplicate confirmation channel".to_string());
        }
        Ok(Self { channels })
    }

    pub(crate) fn set_legacy(&mut self, value: &str) -> Result<(), String> {
        self.channels = Self::from_legacy(value)?.channels;
        Ok(())
    }

    pub(crate) fn display_label(&self) -> String {
        match self.channels.as_slice() {
            [] => "none".to_string(),
            [ConfirmationChannel::Freedesktop] => "freedesktop".to_string(),
            [ConfirmationChannel::Ntfy] => "ntfy".to_string(),
            [ConfirmationChannel::Freedesktop, ConfirmationChannel::Ntfy] => {
                "freedesktop-then-ntfy".to_string()
            }
            _ => self
                .channels
                .iter()
                .map(|channel| channel.as_str())
                .collect::<Vec<_>>()
                .join("-then-"),
        }
    }
}

impl Serialize for ConfirmationProviderConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let channels = self
            .channels
            .iter()
            .map(|channel| channel.as_str())
            .collect::<Vec<_>>();
        let mut state = serializer.serialize_struct("ConfirmationProviderConfig", 1)?;
        state.serialize_field("channels", &channels)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ConfirmationProviderConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| de::Error::custom("confirmationProvider must be an object"))?;
        if let Some(channels) = object.get("channels") {
            let values = channels.as_array().ok_or_else(|| {
                de::Error::custom("confirmationProvider.channels must be an array")
            })?;
            let mut parsed = Vec::with_capacity(values.len());
            for value in values {
                let name = value.as_str().ok_or_else(|| {
                    de::Error::custom("confirmationProvider.channels entries must be strings")
                })?;
                let channel = match name {
                    "freedesktop" => ConfirmationChannel::Freedesktop,
                    "ntfy" => ConfirmationChannel::Ntfy,
                    other => {
                        return Err(de::Error::custom(format!(
                            "unknown confirmation channel: {other}"
                        )))
                    }
                };
                if parsed.contains(&channel) {
                    return Err(de::Error::custom("duplicate confirmation channel"));
                }
                parsed.push(channel);
            }
            return Self::from_channels(parsed).map_err(de::Error::custom);
        }
        let provider = object
            .get("provider")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                de::Error::custom("confirmationProvider requires channels or legacy provider")
            })?;
        Self::from_legacy(provider).map_err(de::Error::custom)
    }
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LimitsConfig {
    pub(crate) max_concurrent_tasks: usize,
    pub(crate) max_active_jobs: MaxActiveJobs,
    #[serde(
        default = "default_max_file_search_context_lines",
        deserialize_with = "deserialize_max_file_search_context_lines"
    )]
    pub(crate) max_file_search_context_lines: usize,
}

pub(crate) const DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES: usize = 5;
pub(crate) const MAX_FILE_SEARCH_CONTEXT_LINES: usize = 100;

fn default_max_file_search_context_lines() -> usize {
    DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES
}

fn deserialize_max_file_search_context_lines<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    if value > MAX_FILE_SEARCH_CONTEXT_LINES {
        return Err(de::Error::custom(format!(
            "maxFileSearchContextLines must be between 0 and {MAX_FILE_SEARCH_CONTEXT_LINES}"
        )));
    }
    Ok(value)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MaxActiveJobs {
    #[default]
    Auto,
    Explicit(usize),
}

impl Serialize for MaxActiveJobs {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Explicit(value) => serializer.serialize_u64(*value as u64),
        }
    }
}

impl<'de> Deserialize<'de> for MaxActiveJobs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MaxActiveJobsVisitor;

        impl<'de> Visitor<'de> for MaxActiveJobsVisitor {
            type Value = MaxActiveJobs;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a non-negative integer or the string \"auto\"")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let value = usize::try_from(value)
                    .map_err(|_| E::custom("maxActiveJobs is too large for this platform"))?;
                Ok(MaxActiveJobs::Explicit(value))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(E::custom("maxActiveJobs must not be negative"));
                }
                self.visit_u64(value as u64)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value == "auto" {
                    Ok(MaxActiveJobs::Auto)
                } else {
                    Err(E::custom("maxActiveJobs must be an integer or \"auto\""))
                }
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(MaxActiveJobsVisitor)
    }
}

impl MaxActiveJobs {
    pub(crate) const MIN_AUTO: usize = 6;
    pub(crate) const MAX_AUTO: usize = 24;

    pub(crate) fn configured_label(self) -> String {
        match self {
            Self::Auto => "auto".to_string(),
            Self::Explicit(value) => value.to_string(),
        }
    }

    pub(crate) fn resolve(self) -> ResolvedMaxActiveJobs {
        let available_parallelism = std::thread::available_parallelism()
            .ok()
            .map(std::num::NonZeroUsize::get);
        self.resolve_with_parallelism(available_parallelism)
    }

    fn resolve_with_parallelism(
        self,
        available_parallelism: Option<usize>,
    ) -> ResolvedMaxActiveJobs {
        let resolved = match self {
            Self::Explicit(value) => value,
            Self::Auto => available_parallelism
                .map(|parallelism| parallelism.saturating_mul(3).saturating_add(1) / 2)
                .unwrap_or(Self::MIN_AUTO)
                .clamp(Self::MIN_AUTO, Self::MAX_AUTO),
        };
        ResolvedMaxActiveJobs {
            configured: self,
            resolved,
            available_parallelism,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedMaxActiveJobs {
    pub(crate) configured: MaxActiveJobs,
    pub(crate) resolved: usize,
    pub(crate) available_parallelism: Option<usize>,
}

impl ResolvedMaxActiveJobs {
    pub(crate) fn diagnostic(self) -> String {
        format!(
            "maxActiveJobs={}; resolvedMaxActiveJobs={}",
            self.configured.configured_label(),
            self.resolved
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoomConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notebook_root: Option<PathBuf>,
    pub(crate) timezone: String,
    #[serde(default = "default_diary_day_boundary_hour")]
    pub(crate) diary_day_boundary_hour: u32,
    #[serde(default, skip_serializing)]
    pub(crate) skills: RoomSkillsConfig,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TunnelConfig {
    pub(crate) tunnel_id: String,
    pub(crate) api_key: String,
    #[serde(default)]
    pub(crate) client: TunnelClientConfig,
    #[serde(default)]
    pub(crate) hub_reporting: HubReportingConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TunnelClientConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) version: Option<String>,
    #[serde(default = "default_tunnel_cache_dir")]
    pub(crate) cache_dir: PathBuf,
    #[serde(default = "default_true")]
    pub(crate) auto_download: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) executable: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HubReportingConfig {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) detail: ReportingDetail,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ReportingDetail {
    #[default]
    Metadata,
    Full,
}

impl std::fmt::Display for ReportingDetail {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Metadata => "metadata",
            Self::Full => "full",
        })
    }
}

impl Default for TunnelClientConfig {
    fn default() -> Self {
        Self {
            version: None,
            cache_dir: default_tunnel_cache_dir(),
            auto_download: true,
            executable: None,
            download_url: None,
            sha256: None,
        }
    }
}

impl Default for HubReportingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            detail: ReportingDetail::Metadata,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoomSkillsConfig {
    #[serde(default = "default_skill_max_files")]
    pub(crate) max_files: usize,
    #[serde(default = "default_skill_max_file_bytes")]
    pub(crate) max_file_bytes: u64,
    #[serde(default = "default_skill_max_package_bytes")]
    pub(crate) max_package_bytes: u64,
    #[serde(default = "default_skill_max_skill_md_bytes")]
    pub(crate) max_skill_md_bytes: u64,
    #[serde(default = "default_skill_max_inline_bytes")]
    pub(crate) max_inline_bytes: u64,
    #[serde(default = "default_skill_connect_timeout_secs")]
    pub(crate) connect_timeout_secs: u64,
    #[serde(default = "default_skill_request_timeout_secs")]
    pub(crate) request_timeout_secs: u64,
    #[serde(default = "default_skill_idle_timeout_secs")]
    pub(crate) idle_timeout_secs: u64,
    #[serde(default = "default_skill_max_redirects")]
    pub(crate) max_redirects: usize,
    #[serde(default = "default_skill_max_concurrent_installs")]
    pub(crate) max_concurrent_installs: usize,
    #[serde(default = "default_skill_max_parallel_downloads")]
    pub(crate) max_parallel_downloads: usize,
    #[serde(default = "default_skill_max_attempts")]
    pub(crate) max_attempts: u32,
    #[serde(default = "default_skill_total_deadline_secs")]
    pub(crate) total_deadline_secs: u64,
    #[serde(default)]
    pub(crate) allowed_hosts: Vec<String>,
}
impl Config {
    pub(crate) fn default_config() -> Result<Self> {
        let base = agentic_home()?;
        Ok(Self {
            agent_id: "laptop".to_string(),
            display_name: hostname_fallback(),
            hub_url: "http://localhost:8787".to_string(),
            hub_transport: default_hub_transport(),
            agent_secret: "change-me".to_string(),
            workspace_root: base.join("workspace"),
            backup_limit: DEFAULT_BACKUP_LIMIT,
            confirmation_provider: ConfirmationProviderConfig {
                channels: ConfirmationProviderConfig::default_channels(),
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
                max_active_jobs: MaxActiveJobs::Auto,
                max_file_search_context_lines: DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES,
            },
            skills: RoomSkillsConfig::default(),
            room: default_room_config(),
            tunnel: None,
            extra: BTreeMap::new(),
        })
    }

    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let has_path_policy = value.get("pathPolicy").is_some();
        let has_top_level_skills = value.get("skills").is_some();
        let legacy_skills = value
            .get("room")
            .and_then(|room| room.get("skills"))
            .cloned();
        let mut config: Self = serde_json::from_value(value)?;
        if !has_path_policy {
            config.path_policy = default_path_policy(&config.workspace_root);
        }
        if has_top_level_skills {
            if legacy_skills.is_some() {
                log_warn(
                    "both skills and legacy room.skills are configured; top-level skills wins"
                        .to_string(),
                );
            }
        } else if let Some(legacy_skills) = legacy_skills {
            config.skills = serde_json::from_value(legacy_skills)?;
        }
        config.room.skills = config.skills.clone();
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
                    confirm: safe_rules(&builtin_rules(
                        CapabilityProfile::Normal,
                        PolicyDecision::Confirm,
                    )),
                    deny: safe_rules(&builtin_rules(
                        CapabilityProfile::Normal,
                        PolicyDecision::Deny,
                    )),
                },
            },
            confirmation_provider: self.confirmation_provider.display_label(),
            tunnel: self.tunnel.as_ref().map(|tunnel| SafeTunnelSummary {
                configured: true,
                tunnel_id: (!tunnel.tunnel_id.trim().is_empty()).then(|| tunnel.tunnel_id.clone()),
                api_key_source: secret_reference_kind(&tunnel.api_key),
                client_source: if tunnel.client.executable.is_some() {
                    "executable".to_string()
                } else if tunnel.client.download_url.is_some() {
                    "custom-url".to_string()
                } else {
                    "managed".to_string()
                },
                hub_reporting_enabled: tunnel.hub_reporting.enabled,
                reporting_detail: tunnel.hub_reporting.detail.to_string(),
            }),
        }
    }

    pub(crate) fn validate_mcp_servers(&self) -> Result<()> {
        crate::mcp::validate_server_configs(&self.mcp_servers)
    }

    pub(crate) fn validate_local(&self) -> Result<()> {
        self.validate_mcp_servers()
    }

    pub(crate) fn validate_standalone(&self) -> Result<()> {
        self.validate_local()?;
        let tunnel = self
            .tunnel
            .as_ref()
            .ok_or_else(|| anyhow!("tunnel_config_required"))?;
        if tunnel.tunnel_id.trim().is_empty() {
            return Err(anyhow!("tunnel_id_required"));
        }
        validate_secret_reference(&tunnel.api_key)?;
        if let Some(url) = tunnel.client.download_url.as_deref() {
            if !url.starts_with("https://") {
                return Err(anyhow!("tunnel_download_url_must_use_https"));
            }
            if tunnel.client.sha256.is_none() {
                return Err(anyhow!("tunnel_download_sha256_required"));
            }
        }
        if let Some(sha256) = tunnel.client.sha256.as_deref() {
            if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(anyhow!("tunnel_sha256_invalid"));
            }
        }
        match tunnel.hub_reporting.detail {
            ReportingDetail::Metadata | ReportingDetail::Full => {}
        }
        Ok(())
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
        skills: RoomSkillsConfig::default(),
    }
}

impl Default for RoomSkillsConfig {
    fn default() -> Self {
        Self {
            max_files: default_skill_max_files(),
            max_file_bytes: default_skill_max_file_bytes(),
            max_package_bytes: default_skill_max_package_bytes(),
            max_skill_md_bytes: default_skill_max_skill_md_bytes(),
            max_inline_bytes: default_skill_max_inline_bytes(),
            connect_timeout_secs: default_skill_connect_timeout_secs(),
            request_timeout_secs: default_skill_request_timeout_secs(),
            idle_timeout_secs: default_skill_idle_timeout_secs(),
            max_redirects: default_skill_max_redirects(),
            max_concurrent_installs: default_skill_max_concurrent_installs(),
            max_parallel_downloads: default_skill_max_parallel_downloads(),
            max_attempts: default_skill_max_attempts(),
            total_deadline_secs: default_skill_total_deadline_secs(),
            allowed_hosts: Vec::new(),
        }
    }
}

fn default_skill_max_files() -> usize {
    256
}
fn default_skill_max_file_bytes() -> u64 {
    10 * 1024 * 1024
}
fn default_skill_max_package_bytes() -> u64 {
    50 * 1024 * 1024
}
fn default_skill_max_skill_md_bytes() -> u64 {
    256 * 1024
}
fn default_skill_max_inline_bytes() -> u64 {
    2 * 1024 * 1024
}
fn default_skill_connect_timeout_secs() -> u64 {
    10
}
fn default_skill_request_timeout_secs() -> u64 {
    120
}
fn default_skill_idle_timeout_secs() -> u64 {
    30
}
fn default_skill_max_redirects() -> usize {
    5
}
fn default_skill_max_concurrent_installs() -> usize {
    2
}
fn default_skill_max_parallel_downloads() -> usize {
    4
}
fn default_skill_max_attempts() -> u32 {
    3
}
fn default_skill_total_deadline_secs() -> u64 {
    600
}

fn default_true() -> bool {
    true
}

fn default_tunnel_cache_dir() -> PathBuf {
    agentic_home()
        .map(|home| home.join("cache").join("tunnel-client"))
        .unwrap_or_else(|_| PathBuf::from("~/.agentic_gpt/cache/tunnel-client"))
}

pub(crate) fn validate_secret_reference(reference: &str) -> Result<()> {
    if let Some(name) = reference.strip_prefix("env:") {
        if name.is_empty()
            || !name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_'
                    || byte.is_ascii_alphanumeric() && (index > 0 || byte.is_ascii_alphabetic())
            })
        {
            return Err(anyhow!("tunnel_api_key_reference_invalid"));
        }
        return Ok(());
    }
    if let Some(path) = reference.strip_prefix("file:") {
        if path.trim().is_empty() {
            return Err(anyhow!("tunnel_api_key_reference_invalid"));
        }
        return Ok(());
    }
    Err(anyhow!("tunnel_api_key_reference_plaintext_rejected"))
}

fn secret_reference_kind(reference: &str) -> Option<String> {
    if reference.starts_with("env:") {
        Some("env".to_string())
    } else if reference.starts_with("file:") {
        Some("file".to_string())
    } else if reference.is_empty() {
        None
    } else {
        Some("invalid".to_string())
    }
}

pub(crate) fn default_diary_day_boundary_hour() -> u32 {
    5
}

pub(crate) fn default_confirmation_language() -> String {
    "zh-CN".to_string()
}

pub(crate) fn default_hub_transport() -> String {
    "websocket".to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn temp_config_path() -> PathBuf {
        std::env::temp_dir().join(format!("agentic-config-{}.json", Uuid::new_v4().simple()))
    }

    #[test]
    fn checked_in_v09_config_example_is_strict_and_safe_to_copy() {
        let source = include_str!("../../../config.example.json");
        let value: serde_json::Value = serde_json::from_str(source).unwrap();
        let config: Config = serde_json::from_value(value.clone()).unwrap();
        config.validate_mcp_servers().unwrap();
        config.validate_standalone().unwrap();
        assert_eq!(config.agent_id, "laptop");
        assert_eq!(config.limits.max_active_jobs, MaxActiveJobs::Auto);
        assert_eq!(
            config.limits.max_file_search_context_lines,
            DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES
        );
        assert_eq!(config.mcp_servers.len(), 2);
        assert!(config.mcp_servers.values().all(|server| !server.enabled));
        assert_eq!(value["agentSecret"], "change-me-before-use");
        assert_eq!(value["tunnel"]["tunnelId"], "tunnel_replace-me");
        assert!(value["tunnel"]["apiKey"]
            .as_str()
            .is_some_and(|reference| reference.starts_with("file:")));
        assert_eq!(value["tunnel"]["hubReporting"]["enabled"], false);
        assert!(value["limits"].get("maxActiveSessions").is_none());
        assert!(value["limits"].get("sessionIdleTimeoutSecs").is_none());
        assert!(!source.contains("AGENTIC_GPT_API_KEY="));
        assert!(!source.contains("integration-secret"));
    }

    #[test]
    fn max_active_jobs_supports_auto_and_explicit_round_trips() {
        let auto: MaxActiveJobs = serde_json::from_value(json!("auto")).unwrap();
        let explicit: MaxActiveJobs = serde_json::from_value(json!(12)).unwrap();
        assert_eq!(auto, MaxActiveJobs::Auto);
        assert_eq!(explicit, MaxActiveJobs::Explicit(12));
        assert_eq!(serde_json::to_value(auto).unwrap(), json!("auto"));
        assert_eq!(serde_json::to_value(explicit).unwrap(), json!(12));
        assert!(serde_json::from_value::<MaxActiveJobs>(json!(-1)).is_err());
        assert!(serde_json::from_value::<MaxActiveJobs>(json!("AUTO")).is_err());
    }

    #[test]
    fn file_search_context_limit_defaults_and_rejects_invalid_values() {
        let base = |value: serde_json::Value| {
            serde_json::from_value::<LimitsConfig>(json!({
                "maxConcurrentTasks": 2,
                "maxActiveJobs": "auto",
                "maxFileSearchContextLines": value,
            }))
        };

        let defaults = serde_json::from_value::<LimitsConfig>(json!({
            "maxConcurrentTasks": 2,
            "maxActiveJobs": "auto",
        }))
        .unwrap();
        assert_eq!(
            defaults.max_file_search_context_lines,
            DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES
        );
        for value in [0, 5, 20, MAX_FILE_SEARCH_CONTEXT_LINES] {
            assert_eq!(
                base(json!(value)).unwrap().max_file_search_context_lines,
                value
            );
        }
        assert!(base(json!(-1)).is_err());
        assert!(base(json!(1.5)).is_err());
        assert!(base(json!(MAX_FILE_SEARCH_CONTEXT_LINES + 1)).is_err());
    }

    #[test]
    fn limits_reject_removed_max_active_sessions_field() {
        let error = serde_json::from_value::<LimitsConfig>(json!({
            "maxConcurrentTasks": 2,
            "maxActiveSessions": 4
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("unknown field `maxActiveSessions`"));
        assert!(error.contains("maxActiveJobs"));

        let error = serde_json::from_value::<LimitsConfig>(json!({
            "maxConcurrentTasks": 2,
            "maxActiveJobs": 4,
            "jobIdleTimeoutSecs": 900
        }))
        .unwrap_err()
        .to_string();
        assert!(error.contains("unknown field `jobIdleTimeoutSecs`"));
    }

    #[test]
    fn auto_max_active_jobs_uses_the_frozen_formula() {
        for (parallelism, expected) in [(1, 6), (4, 6), (8, 12), (12, 18), (16, 24), (20, 24)] {
            let resolved = MaxActiveJobs::Auto.resolve_with_parallelism(Some(parallelism));
            assert_eq!(resolved.resolved, expected, "parallelism={parallelism}");
        }
        assert_eq!(
            MaxActiveJobs::Auto.resolve_with_parallelism(None).resolved,
            6
        );
        assert_eq!(
            MaxActiveJobs::Explicit(4)
                .resolve_with_parallelism(Some(20))
                .resolved,
            4
        );
    }

    #[test]
    fn new_default_config_serializes_auto_limit() {
        let value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        assert_eq!(value["limits"]["maxActiveJobs"], json!("auto"));
        assert_eq!(
            value["limits"]["maxFileSearchContextLines"],
            json!(DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES)
        );
        assert_eq!(
            value["confirmationProvider"]["channels"],
            json!(["freedesktop", "ntfy"])
        );
        assert!(value["confirmationProvider"].get("provider").is_none());
    }

    #[test]
    fn confirmation_provider_accepts_legacy_aliases_and_canonicalizes() {
        for (legacy, expected) in [
            ("none", json!([])),
            ("freedesktop", json!(["freedesktop"])),
            ("hub", json!(["ntfy"])),
            ("ntfy", json!(["ntfy"])),
            ("freedesktop-then-hub", json!(["freedesktop", "ntfy"])),
            ("freedesktopThenHub", json!(["freedesktop", "ntfy"])),
            ("freedesktop-then-ntfy", json!(["freedesktop", "ntfy"])),
            ("default", json!(["freedesktop", "ntfy"])),
        ] {
            let parsed = serde_json::from_value::<ConfirmationProviderConfig>(json!({
                "provider": legacy
            }))
            .unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap()["channels"], expected);
        }
    }

    #[test]
    fn confirmation_provider_rejects_duplicate_or_unknown_channels() {
        assert!(serde_json::from_value::<ConfirmationProviderConfig>(json!({
            "channels": ["ntfy", "ntfy"]
        }))
        .is_err());
        assert!(serde_json::from_value::<ConfirmationProviderConfig>(json!({
            "channels": ["email"]
        }))
        .is_err());
    }

    #[test]
    fn confirmation_provider_display_label_is_truthful() {
        let mut provider = ConfirmationProviderConfig::from_legacy("hub").unwrap();
        assert_eq!(provider.display_label(), "ntfy");
        provider.set_legacy("freedesktop-then-hub").unwrap();
        assert_eq!(provider.display_label(), "freedesktop-then-ntfy");
    }

    #[test]
    fn explicit_limit_stays_numeric_after_config_load_and_write() {
        let path = temp_config_path();
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["limits"]["maxActiveJobs"] = json!(4);
        value["futureField"] = json!({"preserve": true});
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.limits.max_active_jobs, MaxActiveJobs::Explicit(4));
        let written = serde_json::to_value(loaded).unwrap();
        assert_eq!(written["limits"]["maxActiveJobs"], json!(4));
        assert_eq!(written["futureField"]["preserve"], json!(true));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn legacy_skills_are_loaded_and_written_at_top_level() {
        let path = temp_config_path();
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["futureField"] = json!({ "preserve": true });
        value["room"]["skills"] = json!({ "maxFiles": 7, "allowedHosts": ["example.test"] });
        value.as_object_mut().unwrap().remove("skills");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let config = Config::load(&path).unwrap();
        assert_eq!(config.skills.max_files, 7);
        assert_eq!(config.skills.allowed_hosts, vec!["example.test"]);
        let written = serde_json::to_value(config).unwrap();
        assert_eq!(written["skills"]["maxFiles"], 7);
        assert!(written["room"].get("skills").is_none());
        assert_eq!(written["futureField"]["preserve"], true);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn top_level_skills_win_over_legacy_values() {
        let path = temp_config_path();
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["skills"]["maxFiles"] = json!(11);
        value["room"]["skills"] = json!({ "maxFiles": 3 });
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let config = Config::load(&path).unwrap();
        assert_eq!(config.skills.max_files, 11);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn mcp_server_semantics_are_validated_before_standalone_use() {
        let mut config = Config::default_config().unwrap();
        config.mcp_servers.insert(
            "valid-http".to_string(),
            McpServerConfig {
                enabled: true,
                transport: "streamable-http".to_string(),
                url: Some("https://example.test/mcp".to_string()),
            },
        );
        assert!(config.validate_mcp_servers().is_ok());
        config.mcp_servers.get_mut("valid-http").unwrap().transport = "sse".to_string();
        assert!(config
            .validate_mcp_servers()
            .unwrap_err()
            .to_string()
            .starts_with("unsupported_mcp_transport"));
    }

    #[test]
    fn tunnel_secret_references_are_strict_and_safe_summary_is_redacted() {
        assert!(validate_secret_reference("env:AGENTIC_TUNNEL_API_KEY").is_ok());
        assert!(validate_secret_reference("file:/run/secrets/tunnel").is_ok());
        assert!(validate_secret_reference("env:1BAD").is_err());
        assert!(validate_secret_reference("plaintext-secret").is_err());

        let mut config = Config::default_config().unwrap();
        config.tunnel = Some(TunnelConfig {
            tunnel_id: "tunnel_demo".to_string(),
            api_key: "env:AGENTIC_TUNNEL_API_KEY".to_string(),
            ..TunnelConfig::default()
        });
        let summary = serde_json::to_string(&config.safe_summary()).unwrap();
        assert!(summary.contains("tunnel_demo"));
        assert!(summary.contains("\"apiKeySource\":\"env\""));
        assert!(!summary.contains("AGENTIC_TUNNEL_API_KEY"));
        assert!(config.validate_standalone().is_ok());
        config.tunnel.as_mut().unwrap().api_key = "secret".to_string();
        assert!(config.validate_standalone().is_err());
    }
}
