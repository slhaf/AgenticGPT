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
use serde::{
    ser::{SerializeMap, SerializeStruct},
    Deserialize, Deserializer, Serialize, Serializer,
};
use serde_json::Value;
use std::fmt;

use crate::mcp::McpServerConfig;
use crate::policy::{builtin_rules, paths_match, PolicyDecision};
use crate::state::CapabilityProfile;
use crate::utils::{agentic_home, ensure_parent, hostname_fallback, DEFAULT_BACKUP_LIMIT};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RuntimeMode {
    Standalone,
    Hub,
    Local,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub(crate) enum WorkerProfile {
    Normal,
    Room,
}

impl WorkerProfile {
    pub(crate) fn capability_profile(self) -> CapabilityProfile {
        match self {
            Self::Normal => CapabilityProfile::Normal,
            Self::Room => CapabilityProfile::Room,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HubConfig {
    pub(crate) url: String,
    #[serde(default = "default_hub_transport")]
    pub(crate) transport: String,
    pub(crate) agent_secret: String,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8787".to_string(),
            transport: default_hub_transport(),
            agent_secret: "change-me".to_string(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Config {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) agent_id: String,
    pub(crate) display_name: String,
    pub(crate) hub: HubConfig,
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

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "freedesktop" => Ok(Self::Freedesktop),
            "ntfy" => Ok(Self::Ntfy),
            other => Err(format!("unknown confirmation channel: {other}")),
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

    pub(crate) fn from_channel_names<'a>(
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<Self, String> {
        let channels = names
            .into_iter()
            .map(ConfirmationChannel::parse)
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_channels(channels)
    }

    pub(crate) fn channel_names(&self) -> Vec<&'static str> {
        self.channels
            .iter()
            .map(|channel| channel.as_str())
            .collect()
    }

    pub(crate) fn channels_json(&self) -> String {
        serde_json::to_string(&self.channel_names()).expect("confirmation channel names serialize")
    }

    pub(crate) fn fallback_label(&self) -> String {
        if self.channels.is_empty() {
            "none".to_string()
        } else {
            self.channel_names().join(" → ")
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
        if object.contains_key("provider") {
            return Err(de::Error::custom(
                "confirmationProvider.provider is legacy; run config import",
            ));
        }
        let channels = object
            .get("channels")
            .ok_or_else(|| de::Error::custom("confirmationProvider.channels is required"))?;
        let values = channels
            .as_array()
            .ok_or_else(|| de::Error::custom("confirmationProvider.channels must be an array"))?;
        let names = values
            .iter()
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    de::Error::custom("confirmationProvider.channels entries must be strings")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_channel_names(names).map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SandboxConfig {
    pub(crate) enabled: bool,
    pub(crate) bubblewrap_path: String,
    pub(crate) required_runtime_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
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

pub(crate) fn validate_max_file_search_context_lines(value: usize) -> Result<()> {
    if value > MAX_FILE_SEARCH_CONTEXT_LINES {
        return Err(anyhow!(
            "maxFileSearchContextLines must be between 0 and {MAX_FILE_SEARCH_CONTEXT_LINES}"
        ));
    }
    Ok(())
}

fn deserialize_max_file_search_context_lines<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let value = usize::deserialize(deserializer)?;
    validate_max_file_search_context_lines(value).map_err(de::Error::custom)?;
    Ok(value)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MaxActiveJobs {
    #[default]
    Auto,
    Explicit(usize),
}

pub(crate) fn parse_max_active_jobs(value: &str) -> Result<MaxActiveJobs> {
    if value == "auto" {
        return Ok(MaxActiveJobs::Auto);
    }
    Ok(MaxActiveJobs::Explicit(value.parse::<usize>()?))
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
            mode: RuntimeMode::Standalone,
            profile: WorkerProfile::Normal,
            agent_id: "laptop".to_string(),
            display_name: hostname_fallback(),
            hub: HubConfig::default(),
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
        let value: Value = serde_json::from_str(&text)?;
        let object = value
            .as_object()
            .ok_or_else(|| anyhow!("config_json_object_required"))?;
        if !object.contains_key("mode") || !object.contains_key("profile") {
            return Err(anyhow!(
                "config_requires_mode_profile: run config import to migrate, or config init to reinitialize"
            ));
        }
        if ["hubUrl", "hubTransport", "agentSecret", "workerUrl"]
            .iter()
            .any(|key| object.contains_key(*key))
        {
            return Err(anyhow!("config_requires_nested_hub: run config import"));
        }
        if object
            .get("confirmationProvider")
            .and_then(Value::as_object)
            .is_some_and(|provider| provider.contains_key("provider"))
        {
            return Err(anyhow!(
                "config_requires_confirmation_channels: run config import"
            ));
        }
        if object
            .get("room")
            .and_then(Value::as_object)
            .is_some_and(|room| room.contains_key("skills"))
        {
            return Err(anyhow!(
                "config_requires_top_level_skills: run config import"
            ));
        }
        let mode = serde_json::from_value::<RuntimeMode>(
            object.get("mode").cloned().expect("mode was checked"),
        )
        .map_err(|_| anyhow!("config_mode_invalid"))?;
        let profile = serde_json::from_value::<WorkerProfile>(
            object.get("profile").cloned().expect("profile was checked"),
        )
        .map_err(|_| anyhow!("config_profile_invalid"))?;
        let has_path_policy = object.contains_key("pathPolicy");
        let defaults = Self::default_config()?;
        let workspace_root = object
            .get("workspaceRoot")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_else(|| defaults.workspace_root.clone());
        let mut defaults = defaults;
        defaults.mode = mode;
        defaults.profile = profile;
        defaults.path_policy = default_path_policy(&workspace_root);
        if object.get("tunnel").is_some_and(Value::is_object) {
            defaults.tunnel = Some(TunnelConfig::default());
        } else {
            defaults.tunnel = None;
        }
        let mut effective = serde_json::to_value(defaults)?;
        merge_json_values(&mut effective, value);
        let mut config: Self = serde_json::from_value(effective)?;
        if !has_path_policy {
            config.path_policy = default_path_policy(&config.workspace_root);
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

    /// Best-effort migration reader used only by `config import`.
    ///
    /// Runtime loading deliberately stays strict: this function is the explicit
    /// boundary where old top-level Hub fields, missing selectors, and malformed
    /// recognized fields may be dealt with and reported to the user.
    pub(crate) fn import(path: &Path) -> Result<ConfigImport> {
        let text = fs::read_to_string(path)
            .map_err(|error| anyhow!("config_import_source_read_failed: {error}"))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| anyhow!("config_import_json_invalid: {error}"))?;
        let mut object = value
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow!("config_import_json_object_required"))?;
        let mut warnings = Vec::new();

        let import_defaults = Self::default_config()?;
        let mode = match object.get("mode").cloned() {
            Some(value) => match serde_json::from_value::<RuntimeMode>(value) {
                Ok(mode) => mode,
                Err(_) => {
                    warnings.push("mode (invalid value; using init default)".to_string());
                    import_defaults.mode
                }
            },
            None => import_defaults.mode,
        };
        let profile = match object.get("profile").cloned() {
            Some(value) => match serde_json::from_value::<WorkerProfile>(value) {
                Ok(profile) => profile,
                Err(_) => {
                    warnings.push("profile (invalid value; using init default)".to_string());
                    import_defaults.profile
                }
            },
            None => import_defaults.profile,
        };

        let mut hub = match object.remove("hub") {
            Some(Value::Object(hub)) => hub,
            Some(_) => {
                warnings.push("hub (expected an object; defaulted during import)".to_string());
                serde_json::Map::new()
            }
            None => serde_json::Map::new(),
        };
        for (legacy_key, nested_key) in [
            ("hubUrl", "url"),
            ("workerUrl", "url"),
            ("hubTransport", "transport"),
            ("agentSecret", "agentSecret"),
        ] {
            if let Some(value) = object.remove(legacy_key) {
                if hub.contains_key(nested_key) {
                    warnings.push(format!(
                        "{legacy_key} (ignored because hub.{nested_key} is also present)"
                    ));
                } else {
                    hub.insert(nested_key.to_string(), value);
                }
            }
        }
        if !hub.is_empty() {
            object.insert("hub".to_string(), Value::Object(hub));
        }

        if let Some(value) = object.remove("confirmationProvider") {
            match value {
                Value::Object(mut provider) => {
                    if let Some(legacy) = provider.remove("provider") {
                        if provider.contains_key("channels") {
                            warnings.push(
                                "confirmationProvider.provider (ignored because channels is also present)"
                                    .to_string(),
                            );
                            object.insert(
                                "confirmationProvider".to_string(),
                                Value::Object(provider),
                            );
                        } else if let Some(name) = legacy.as_str() {
                            match ConfirmationProviderConfig::from_legacy(name) {
                                Ok(canonical) => {
                                    object.insert(
                                        "confirmationProvider".to_string(),
                                        serde_json::to_value(canonical)?,
                                    );
                                }
                                Err(_) => warnings.push(
                                    "confirmationProvider.provider (invalid legacy value; retained init default)"
                                        .to_string(),
                                ),
                            }
                        } else {
                            warnings.push(
                                "confirmationProvider.provider (expected a string; retained init default)"
                                    .to_string(),
                            );
                        }
                    } else {
                        object.insert("confirmationProvider".to_string(), Value::Object(provider));
                    }
                }
                other => {
                    object.insert("confirmationProvider".to_string(), other);
                }
            }
        }

        let legacy_room_skills = object
            .get_mut("room")
            .and_then(Value::as_object_mut)
            .and_then(|room| room.remove("skills"));
        if let Some(legacy_skills) = legacy_room_skills {
            if object.contains_key("skills") {
                warnings.push(
                    "room.skills (ignored because top-level skills is also present)".to_string(),
                );
            } else {
                object.insert("skills".to_string(), legacy_skills);
            }
        }

        object.insert("mode".to_string(), serde_json::to_value(mode)?);
        object.insert("profile".to_string(), serde_json::to_value(profile)?);

        if let Some(Value::Object(tunnel)) = object.get_mut("tunnel") {
            let invalid_api_key = tunnel.get("apiKey").is_some_and(|value| {
                value
                    .as_str()
                    .is_none_or(|reference| validate_secret_reference(reference).is_err())
            });
            if invalid_api_key {
                warnings.push(
                    "tunnel.apiKey (invalid secret reference; use file:PATH or env:NAME; cleared for import)"
                        .to_string(),
                );
                tunnel.insert("apiKey".to_string(), Value::String(String::new()));
            }
        }

        let known = [
            "mode",
            "profile",
            "agentId",
            "displayName",
            "hub",
            "workspaceRoot",
            "backupLimit",
            "confirmationProvider",
            "confirmationLanguage",
            "sandbox",
            "mcpServers",
            "pathPolicy",
            "policy",
            "limits",
            "skills",
            "room",
            "tunnel",
        ];
        let original = object.clone();
        for key in known
            .iter()
            .copied()
            .filter(|key| *key != "mode" && *key != "profile")
        {
            if !object.contains_key(key) {
                continue;
            }
            let mut candidate = original.clone();
            for other in known.iter().copied() {
                if other != key && other != "mode" && other != "profile" {
                    candidate.remove(other);
                }
            }
            if materialize_import_value(&Value::Object(candidate)).is_err() {
                warnings.push(format!(
                    "{key} (could not be imported; retained default instead)"
                ));
                object.remove(key);
            }
        }

        let normalized = Value::Object(object);
        let config = materialize_import_value(&normalized)
            .map_err(|error| anyhow!("config_import_no_usable_fields: {error}"))?;
        Ok(ConfigImport { config, warnings })
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
            confirmation_provider: self.confirmation_provider.fallback_label(),
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

    pub(crate) fn validate_hub(&self) -> Result<()> {
        self.validate_local()?;
        validate_hub_url_shape(&self.hub.url)?;
        validate_hub_transport(&self.hub.transport)?;
        if self.agent_id.trim().is_empty() {
            return Err(anyhow!("agent_id_required"));
        }
        if self.hub.agent_secret.trim().is_empty() || self.hub.agent_secret == "change-me" {
            return Err(anyhow!("agent_secret_required"));
        }
        Ok(())
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

#[derive(Debug)]
pub(crate) struct ConfigImport {
    pub(crate) config: Config,
    pub(crate) warnings: Vec<String>,
}

fn materialize_import_value(value: &Value) -> Result<Config> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("config_import_json_object_required"))?;
    let mut defaults = Config::default_config()?;
    let mode = serde_json::from_value::<RuntimeMode>(
        object
            .get("mode")
            .cloned()
            .ok_or_else(|| anyhow!("config_import_mode_missing"))?,
    )?;
    let profile = serde_json::from_value::<WorkerProfile>(
        object
            .get("profile")
            .cloned()
            .ok_or_else(|| anyhow!("config_import_profile_missing"))?,
    )?;
    defaults.mode = mode;
    defaults.profile = profile;
    let workspace_root = object
        .get("workspaceRoot")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| defaults.workspace_root.clone());
    defaults.path_policy = default_path_policy(&workspace_root);
    defaults.tunnel = object
        .get("tunnel")
        .is_some_and(Value::is_object)
        .then(TunnelConfig::default);
    let mut effective = serde_json::to_value(defaults)?;
    merge_json_values(&mut effective, value.clone());
    let mut config: Config = serde_json::from_value(effective)?;
    if !object.contains_key("pathPolicy") {
        config.path_policy = default_path_policy(&config.workspace_root);
    }
    config.room.skills = config.skills.clone();
    Ok(config)
}

fn merge_json_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(&key) {
                    merge_json_values(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn sparse_defaults(config: &Config) -> Result<Config> {
    let mut defaults = Config::default_config()?;
    defaults.mode = config.mode;
    defaults.profile = config.profile;
    defaults.path_policy = default_path_policy(&config.workspace_root);
    // Sparse persistence is independent of the currently active runtime mode.
    // If a tunnel section is configured, compare it against tunnel defaults so
    // inactive-but-saved tunnel settings stay sparse instead of materializing
    // reconstructable client/reporting defaults.
    defaults.tunnel = config.tunnel.as_ref().map(|_| TunnelConfig::default());
    Ok(defaults)
}

fn prune_sparse_value(value: &mut Value, defaults: &Value, root: bool) {
    let (Value::Object(value), Value::Object(defaults)) = (value, defaults) else {
        return;
    };
    let keys = value.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        if root && matches!(key.as_str(), "mode" | "profile") {
            continue;
        }
        let Some(current) = value.get(&key) else {
            continue;
        };
        let Some(default) = defaults.get(&key) else {
            // Unknown flattened fields are deliberately retained, including empty
            // objects and nulls, so future fields survive a current-version write.
            continue;
        };
        if current == default {
            value.remove(&key);
            continue;
        }
        if current.is_object() && default.is_object() {
            if let Some(current) = value.get_mut(&key) {
                prune_sparse_value(current, default, false);
            }
            if value
                .get(&key)
                .and_then(Value::as_object)
                .is_some_and(|object| object.is_empty())
            {
                value.remove(&key);
            }
        }
    }
}

pub(crate) fn sparse_config_value(config: &Config, redact_secrets: bool) -> Result<Value> {
    let defaults = sparse_defaults(config)?;
    let defaults_value = serde_json::to_value(defaults)?;
    let mut value = serde_json::to_value(config)?;
    value
        .as_object_mut()
        .ok_or_else(|| anyhow!("config_projection_object_required"))?;
    prune_sparse_value(&mut value, &defaults_value, true);

    if redact_secrets {
        if let Some(secret) = value
            .as_object_mut()
            .and_then(|object| object.get_mut("hub"))
            .and_then(Value::as_object_mut)
            .and_then(|hub| hub.get_mut("agentSecret"))
        {
            *secret = Value::String("[REDACTED]".to_string());
        }
    }
    Ok(value)
}

struct OrderedConfigRoot<'a>(&'a Value);

impl Serialize for OrderedConfigRoot<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let object = self
            .0
            .as_object()
            .ok_or_else(|| serde::ser::Error::custom("config_projection_object_required"))?;
        let mut map = serializer.serialize_map(Some(object.len()))?;
        for key in ["agentId", "displayName", "mode", "profile"] {
            if let Some(value) = object.get(key) {
                map.serialize_entry(key, value)?;
            }
        }
        for (key, value) in object {
            if !matches!(key.as_str(), "agentId" | "displayName" | "mode" | "profile") {
                map.serialize_entry(key, value)?;
            }
        }
        map.end()
    }
}

fn ordered_config_value_json(value: &Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(&OrderedConfigRoot(value))?)
}

pub(crate) fn ordered_config_json(config: &Config) -> Result<String> {
    ordered_config_value_json(&serde_json::to_value(config)?)
}

pub(crate) fn sparse_config_json(config: &Config, redact_secrets: bool) -> Result<String> {
    let value = sparse_config_value(config, redact_secrets)?;
    ordered_config_value_json(&value)
}

pub(crate) fn validate_hub_url_shape(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).map_err(|_| anyhow!("hub_url_invalid"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(anyhow!("hub_url_invalid"));
    }
    Ok(())
}

pub(crate) fn validate_hub_transport(value: &str) -> Result<()> {
    if !matches!(value, "websocket" | "sse") {
        return Err(anyhow!("hub_transport_invalid"));
    }
    Ok(())
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
    let serialized = sparse_config_json(config, false)?;
    ensure_parent(path)?;
    if path.exists() {
        let backup_dir = path.parent().unwrap().join("backups");
        fs::create_dir_all(&backup_dir)?;
        let backup = backup_dir.join(format!("config.{}.json", Utc::now().timestamp_millis()));
        fs::copy(path, backup)?;
        prune_backups(&backup_dir, config.backup_limit)?;
    }
    fs::write(path, serialized)?;
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
        let path = temp_config_path();
        fs::write(&path, source).unwrap();
        let config = Config::load(&path).unwrap();
        config.validate_mcp_servers().unwrap();
        config.validate_standalone().unwrap();
        assert_eq!(config.mode, RuntimeMode::Standalone);
        assert_eq!(config.profile, WorkerProfile::Normal);
        assert_eq!(config.agent_id, "laptop");
        assert_eq!(config.limits.max_active_jobs, MaxActiveJobs::Auto);
        assert_eq!(
            config.limits.max_file_search_context_lines,
            DEFAULT_MAX_FILE_SEARCH_CONTEXT_LINES
        );
        assert_eq!(config.mcp_servers.len(), 2);
        assert!(config.mcp_servers.values().all(|server| !server.enabled));
        assert_eq!(value["hub"]["agentSecret"], "change-me-before-use");
        assert_eq!(value["tunnel"]["tunnelId"], "tunnel_replace-me");
        assert!(value["tunnel"]["apiKey"]
            .as_str()
            .is_some_and(|reference| reference.starts_with("file:")));
        assert!(value["tunnel"].get("hubReporting").is_none());
        assert!(!config.tunnel.as_ref().unwrap().hub_reporting.enabled);
        assert!(value["limits"].get("maxActiveSessions").is_none());
        assert!(value["limits"].get("sessionIdleTimeoutSecs").is_none());
        assert!(!source.contains("AGENTIC_GPT_API_KEY="));
        assert!(!source.contains("integration-secret"));
        let _ = fs::remove_file(path);
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
    fn confirmation_provider_disk_shape_rejects_legacy_provider() {
        assert!(serde_json::from_value::<ConfirmationProviderConfig>(json!({
            "provider": "none"
        }))
        .is_err());
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
    fn legacy_confirmation_labels_map_to_canonical_fallback_order() {
        let provider = ConfirmationProviderConfig::from_legacy("hub").unwrap();
        assert_eq!(provider.fallback_label(), "ntfy");
        let provider = ConfirmationProviderConfig::from_legacy("freedesktop-then-hub").unwrap();
        assert_eq!(provider.fallback_label(), "freedesktop → ntfy");
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
    fn strict_load_rejects_legacy_room_skills() {
        let path = temp_config_path();
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["room"]["skills"] = json!({ "maxFiles": 7, "allowedHosts": ["example.test"] });
        value.as_object_mut().unwrap().remove("skills");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let error = Config::load(&path).unwrap_err().to_string();
        assert!(error.contains("config_requires_top_level_skills"));
        assert!(error.contains("config import"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn explicit_import_prefers_top_level_skills_over_legacy_room_skills() {
        let path = temp_config_path();
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value["skills"]["maxFiles"] = json!(11);
        value["room"]["skills"] = json!({ "maxFiles": 3 });
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let imported = Config::import(&path).unwrap();
        assert_eq!(imported.config.skills.max_files, 11);
        assert!(imported
            .warnings
            .iter()
            .any(|warning| warning.starts_with("room.skills ")));
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

    #[test]
    fn sparse_projection_always_keeps_selectors_and_omits_reconstructable_defaults() {
        let mut config = Config::default_config().unwrap();
        config.mode = RuntimeMode::Local;
        config.profile = WorkerProfile::Normal;

        let value = sparse_config_value(&config, false).unwrap();
        assert_eq!(value["mode"], json!("local"));
        assert_eq!(value["profile"], json!("normal"));
        for key in [
            "agentId",
            "displayName",
            "hub",
            "workspaceRoot",
            "pathPolicy",
            "policy",
            "limits",
            "skills",
            "room",
            "tunnel",
        ] {
            assert!(
                value.get(key).is_none(),
                "default field was retained: {key}"
            );
        }
    }

    #[test]
    fn sparse_projection_preserves_custom_workspace_root_but_reconstructs_its_path_defaults() {
        let mut config = Config::default_config().unwrap();
        config.mode = RuntimeMode::Local;
        config.workspace_root = temp_config_path().with_extension("workspace");
        config.path_policy = default_path_policy(&config.workspace_root);

        let value = sparse_config_value(&config, false).unwrap();
        assert_eq!(
            value["workspaceRoot"],
            serde_json::to_value(&config.workspace_root).unwrap()
        );
        assert!(value.get("pathPolicy").is_none());

        let path = temp_config_path();
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.workspace_root, config.workspace_root);
        assert_eq!(loaded.path_policy, config.path_policy);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn sparse_projection_keeps_inactive_sections_and_redacts_config_secrets() {
        let mut config = Config::default_config().unwrap();
        config.mode = RuntimeMode::Hub;
        config.profile = WorkerProfile::Room;
        config.hub.agent_secret = "hub-secret-marker".to_string();
        config.tunnel = Some(TunnelConfig {
            tunnel_id: "illegal-tunnel".to_string(),
            api_key: "file:/tmp/tunnel-secret".to_string(),
            ..TunnelConfig::default()
        });

        let value = sparse_config_value(&config, true).unwrap();
        assert!(value.get("tunnel").is_some());
        assert_eq!(value["hub"]["agentSecret"], json!("[REDACTED]"));
        assert_eq!(value["tunnel"]["apiKey"], json!("file:/tmp/tunnel-secret"));
        assert!(!serde_json::to_string(&value)
            .unwrap()
            .contains("hub-secret-marker"));
    }

    #[test]
    fn sparse_load_reconstructs_workspace_dependent_path_policy_and_unknown_fields() {
        let root = temp_config_path();
        let workspace = root.with_extension("workspace");
        let expected = default_path_policy(&workspace);
        let value = json!({
            "mode": "local",
            "profile": "normal",
            "workspaceRoot": workspace,
            "futureField": {"enabled": true}
        });
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&root).unwrap();
        assert_eq!(loaded.mode, RuntimeMode::Local);
        assert_eq!(loaded.profile, WorkerProfile::Normal);
        assert_eq!(loaded.workspace_root, workspace);
        assert_eq!(loaded.path_policy, expected);
        assert_eq!(loaded.extra["futureField"], json!({"enabled": true}));
        let _ = fs::remove_file(root);
    }

    #[test]
    fn sparse_load_preserves_inactive_mode_and_profile_sections() {
        let root = temp_config_path();
        let value = json!({
            "mode": "local",
            "profile": "normal",
            "tunnel": {"tunnelId": "stale"},
            "room": {"timezone": "UTC", "diaryDayBoundaryHour": 1}
        });
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load(&root).unwrap();
        assert_eq!(
            loaded
                .tunnel
                .as_ref()
                .map(|tunnel| tunnel.tunnel_id.as_str()),
            Some("stale")
        );
        assert_eq!(loaded.room.timezone, "UTC");
        assert_eq!(loaded.room.diary_day_boundary_hour, 1);
        let _ = fs::remove_file(root);
    }

    #[test]
    fn sparse_projection_preserves_explicit_inactive_hub_tunnel_and_room_data() {
        let mut config = Config::default_config().unwrap();
        config.mode = RuntimeMode::Local;
        config.profile = WorkerProfile::Normal;
        config.hub.url = "https://inactive-hub.example.com".to_string();
        config.hub.transport = "sse".to_string();
        config.hub.agent_secret = "inactive-hub-secret".to_string();
        config.tunnel = Some(TunnelConfig {
            tunnel_id: "inactive-tunnel".to_string(),
            api_key: "env:INACTIVE_TUNNEL_KEY".to_string(),
            ..TunnelConfig::default()
        });
        config.room.timezone = "UTC".to_string();
        config.extra.insert("futureField".to_string(), json!(true));

        let value = sparse_config_value(&config, false).unwrap();
        assert_eq!(
            value["hub"]["url"],
            json!("https://inactive-hub.example.com")
        );
        assert_eq!(value["tunnel"]["tunnelId"], json!("inactive-tunnel"));
        assert_eq!(value["tunnel"]["apiKey"], json!("env:INACTIVE_TUNNEL_KEY"));
        assert!(value["tunnel"].get("client").is_none());
        assert!(value["tunnel"].get("hubReporting").is_none());
        assert_eq!(value["room"]["timezone"], json!("UTC"));
        assert_eq!(value["futureField"], json!(true));

        let root = temp_config_path();
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        let loaded = Config::load(&root).unwrap();
        assert_eq!(loaded.hub.url, config.hub.url);
        assert_eq!(loaded.tunnel.unwrap().tunnel_id, "inactive-tunnel");
        assert_eq!(loaded.room.timezone, "UTC");
        let _ = fs::remove_file(root);
    }

    #[test]
    fn strict_load_rejects_legacy_confirmation_provider_shape() {
        let root = temp_config_path();
        let value = json!({
            "mode": "local",
            "profile": "normal",
            "confirmationProvider": {"provider": "none"}
        });
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let error = Config::load(&root).unwrap_err().to_string();
        assert!(error.contains("config_requires_confirmation_channels"));
        assert!(error.contains("config import"));
        let _ = fs::remove_file(root);
    }

    #[test]
    fn loading_a_legacy_file_without_selectors_returns_migration_error() {
        let root = temp_config_path();
        let mut value = serde_json::to_value(Config::default_config().unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("mode");
        value.as_object_mut().unwrap().remove("profile");
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let error = Config::load(&root).unwrap_err().to_string();
        assert!(error.contains("config_requires_mode_profile"));
        assert!(error.contains("config import"));
        assert!(error.contains("config init"));
        let _ = fs::remove_file(root);
    }

    #[test]
    fn strict_load_rejects_legacy_top_level_hub_fields_even_with_selectors() {
        let root = temp_config_path();
        let value = json!({
            "mode": "hub",
            "profile": "normal",
            "hubUrl": "https://legacy.example.com",
            "hubTransport": "sse",
            "agentSecret": "legacy-secret"
        });
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let error = Config::load(&root).unwrap_err().to_string();
        assert!(error.contains("config_requires_nested_hub"));
        assert!(error.contains("run config import"));
        let _ = fs::remove_file(root);
    }

    #[test]
    fn explicit_import_maps_legacy_hub_and_preserves_recognized_and_unknown_fields() {
        let root = temp_config_path();
        let value = json!({
            "displayName": "imported-agent",
            "hubUrl": "https://legacy.example.com",
            "hubTransport": "sse",
            "agentSecret": "legacy-secret",
            "confirmationProvider": {"provider": "none"},
            "tunnel": {
                "tunnelId": "tunnel-imported",
                "apiKey": "env:IMPORTED_TUNNEL_KEY"
            },
            "limits": {
                "maxConcurrentTasks": 7,
                "maxActiveJobs": 4
            },
            "mcpServers": {
                "imported": {
                    "enabled": true,
                    "transport": "stdio",
                    "url": "node ./server.mjs"
                }
            },
            "policy": {
                "allow": [{"program": "git", "argsPrefix": ["status"]}],
                "confirm": [],
                "deny": []
            },
            "room": {
                "timezone": "UTC",
                "diaryDayBoundaryHour": 3,
                "skills": {"maxFiles": 7, "allowedHosts": ["example.test"]}
            },
            "futureField": {"preserve": true}
        });
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let imported = Config::import(&root).unwrap();
        assert!(
            imported.warnings.is_empty(),
            "unexpected warnings: {:?}",
            imported.warnings
        );
        assert_eq!(imported.config.mode, RuntimeMode::Standalone);
        assert_eq!(imported.config.profile, WorkerProfile::Normal);
        assert!(imported.config.confirmation_provider.channels.is_empty());
        assert_eq!(imported.config.skills.max_files, 7);
        assert_eq!(imported.config.skills.allowed_hosts, vec!["example.test"]);
        assert_eq!(imported.config.hub.url, "https://legacy.example.com");
        assert_eq!(imported.config.hub.transport, "sse");
        assert_eq!(imported.config.hub.agent_secret, "legacy-secret");
        assert_eq!(
            imported.config.tunnel.as_ref().unwrap().tunnel_id,
            "tunnel-imported"
        );
        assert_eq!(imported.config.limits.max_concurrent_tasks, 7);
        assert_eq!(imported.config.mcp_servers.len(), 1);
        assert_eq!(imported.config.policy.allow[0].program, "git");
        assert_eq!(imported.config.room.timezone, "UTC");
        assert_eq!(
            imported.config.extra["futureField"],
            json!({"preserve": true})
        );
        let _ = fs::remove_file(root);
    }

    #[test]
    fn explicit_import_reports_unimportable_recognized_fields_and_keeps_other_values() {
        let root = temp_config_path();
        let value = json!({
            "displayName": "still-imported",
            "limits": {"maxActiveSessions": 4},
            "futureField": "preserve-me"
        });
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let imported = Config::import(&root).unwrap();
        assert!(imported
            .warnings
            .iter()
            .any(|warning| warning.starts_with("limits ")));
        assert_eq!(imported.config.display_name, "still-imported");
        assert_eq!(imported.config.extra["futureField"], json!("preserve-me"));
        assert_eq!(imported.config.limits.max_concurrent_tasks, 2);
        let _ = fs::remove_file(root);
    }

    #[test]
    fn explicit_import_clears_plaintext_tunnel_secret_and_reports_it() {
        let root = temp_config_path();
        let value = json!({
            "mode": "standalone",
            "profile": "normal",
            "tunnel": {
                "tunnelId": "imported-tunnel",
                "apiKey": "plaintext-secret-marker"
            },
            "displayName": "still-imported"
        });
        fs::write(&root, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let imported = Config::import(&root).unwrap();
        assert!(imported
            .warnings
            .iter()
            .any(|warning| warning.starts_with("tunnel.apiKey ")));
        assert_eq!(imported.config.display_name, "still-imported");
        assert_eq!(imported.config.tunnel.as_ref().unwrap().api_key, "");
        assert!(!serde_json::to_string(&imported.config)
            .unwrap()
            .contains("plaintext-secret-marker"));
        let _ = fs::remove_file(root);
    }

    #[test]
    fn durable_writer_uses_sparse_projection_and_preserves_unknown_fields() {
        let path = temp_config_path();
        let mut config = Config::default_config().unwrap();
        config.mode = RuntimeMode::Local;
        config.profile = WorkerProfile::Normal;
        config.extra.insert("futureField".to_string(), json!(true));

        write_config_with_backup(&path, &config).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["mode"], json!("local"));
        assert_eq!(value["profile"], json!("normal"));
        assert_eq!(value["futureField"], json!(true));
        assert!(value.get("limits").is_none());
        assert!(value.get("pathPolicy").is_none());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn hub_validation_rejects_invalid_url_and_transport_with_stable_errors() {
        for (url, transport, expected) in [
            ("ftp://hub.example.com", "websocket", "hub_url_invalid"),
            (
                "https://hub.example.com",
                "polling",
                "hub_transport_invalid",
            ),
        ] {
            let mut config = Config::default_config().unwrap();
            config.hub.url = url.to_string();
            config.hub.transport = transport.to_string();
            config.hub.agent_secret = "configured-secret".to_string();
            let error = config.validate_hub().unwrap_err();
            assert_eq!(
                error.to_string(),
                expected,
                "Hub validation error code changed"
            );
        }
    }
}
