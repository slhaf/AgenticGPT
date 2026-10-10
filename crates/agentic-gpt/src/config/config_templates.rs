use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::cli_i18n::UiLanguage;
use crate::config::mcp_servers::McpServerConfig;
use crate::config::{
    default_path_policy, normalize_http_mcp_public_url, parse_http_mcp_allow_hosts,
    validate_hub_transport, validate_hub_url_shape, Config, ConfirmationProviderConfig,
    EventsConfig, HubReportingConfig, LimitsConfig, PathPolicyConfig, RoomConfig, SandboxConfig,
    ShellConfig, ToolNamespace, ToolsetConfig, TunnelClientConfig, TunnelConfig, WorkerProfile,
};
use crate::utils::agentic_home;

const STANDALONE_TUNNEL_ID_PLACEHOLDER: &str = "tunnel_replace-me";
const HUB_URL_PLACEHOLDER: &str = "https://hub.replace-me";
const HUB_AGENT_SECRET_PLACEHOLDER: &str = "change-me";

pub(crate) use crate::config::RuntimeMode;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum OptionalSection {
    Identity,
    Workspace,
    Confirmation,
    Limits,
    Sandbox,
    Shell,
    McpServers,
    Toolsets,
    Room,
    TunnelClient,
    HubReporting,
    Events,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TunnelSecretSource {
    File,
    Environment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingAction {
    ReplaceTunnelId,
    ProvisionTunnelSecret,
    ConfigureHubUrl,
    ReplaceAgentSecret,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct SecretValue(String);

impl SecretValue {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

pub(crate) struct InitInput {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) imported_base: Option<Config>,
    pub(crate) ui_language: UiLanguage,
    pub(crate) tunnel_id: Option<String>,
    pub(crate) tunnel_api_key: Option<String>,
    pub(crate) hub_url: Option<String>,
    pub(crate) hub_transport: Option<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) agent_secret: Option<SecretValue>,
    pub(crate) http_mcp_enabled: Option<bool>,
    pub(crate) http_mcp_host: Option<String>,
    pub(crate) http_mcp_port: Option<u16>,
    pub(crate) http_mcp_bearer_token: Option<SecretValue>,
    pub(crate) http_mcp_public_url: Option<String>,
    pub(crate) http_mcp_allow_hosts: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) path_policy: Option<PathPolicyConfig>,
    pub(crate) confirmation_provider: Option<ConfirmationProviderConfig>,
    pub(crate) confirmation_language: Option<String>,
    pub(crate) limits: Option<LimitsConfig>,
    pub(crate) sandbox: Option<SandboxConfig>,
    pub(crate) shell: Option<ShellConfig>,
    pub(crate) room: Option<RoomConfig>,
    pub(crate) tunnel_client: Option<TunnelClientConfig>,
    pub(crate) hub_reporting: Option<HubReportingConfig>,
    pub(crate) mcp_servers: Option<BTreeMap<String, McpServerConfig>>,
    pub(crate) toolsets: Option<ToolsetConfig>,
    pub(crate) events: Option<EventsConfig>,
}

impl InitInput {
    pub(crate) fn non_interactive_defaults(language: UiLanguage) -> Self {
        Self {
            mode: RuntimeMode::Standalone,
            profile: WorkerProfile::Normal,
            imported_base: None,
            ui_language: language,
            tunnel_id: None,
            tunnel_api_key: None,
            hub_url: None,
            hub_transport: None,
            agent_id: None,
            agent_secret: None,
            http_mcp_enabled: None,
            http_mcp_host: None,
            http_mcp_port: None,
            http_mcp_bearer_token: None,
            http_mcp_public_url: None,
            http_mcp_allow_hosts: None,
            display_name: None,
            workspace_root: None,
            path_policy: None,
            confirmation_provider: None,
            confirmation_language: None,
            limits: None,
            sandbox: None,
            shell: None,
            room: None,
            tunnel_client: None,
            hub_reporting: None,
            mcp_servers: None,
            toolsets: None,
            events: None,
        }
    }
}

pub(crate) struct InitBuild {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) config: Config,
    pub(crate) pending: Vec<PendingAction>,
}

pub(crate) struct InitSummary {
    pub(crate) mode: RuntimeMode,
    pub(crate) profile: WorkerProfile,
    pub(crate) config_path: PathBuf,
    pub(crate) pending: Vec<PendingAction>,
}

pub(crate) struct SecretWritePlan {
    pub(crate) path: PathBuf,
    pub(crate) value: SecretValue,
}

impl fmt::Debug for SecretWritePlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretWritePlan")
            .field("path", &self.path)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

pub(crate) fn build_config(input: InitInput) -> Result<InitBuild> {
    let InitInput {
        mode,
        profile,
        imported_base,
        ui_language,
        tunnel_id,
        http_mcp_enabled,
        http_mcp_host,
        http_mcp_port,
        http_mcp_public_url,
        http_mcp_bearer_token,
        http_mcp_allow_hosts,
        tunnel_api_key,
        hub_url,
        hub_transport,
        agent_id,
        agent_secret,
        display_name,
        workspace_root,
        path_policy,
        confirmation_provider,
        confirmation_language,
        limits,
        sandbox,
        shell,
        room,
        tunnel_client,
        hub_reporting,
        events,
        mcp_servers,
        toolsets,
    } = input;

    let has_imported_base = imported_base.is_some();
    let mut config = imported_base.unwrap_or(Config::default_config()?);
    if let Some(events) = events {
        config.events = events;
    }
    config.mode = mode;
    if let Some(enabled) = http_mcp_enabled {
        config.http_mcp.enabled = enabled;
    }
    if let Some(host) = http_mcp_host {
        config.http_mcp.host = host;
    }
    if let Some(port) = http_mcp_port {
        config.http_mcp.port = port;
    }
    if let Some(public_url) = http_mcp_public_url {
        config.http_mcp.public_url = if public_url.trim().is_empty() {
            None
        } else {
            Some(normalize_http_mcp_public_url(&public_url)?)
        };
    }
    if let Some(bearer_token) = http_mcp_bearer_token {
        config.http_mcp.bearer_token = bearer_token.expose().to_string();
    }
    if let Some(allow_hosts) = http_mcp_allow_hosts {
        config.http_mcp.allow_hosts = parse_http_mcp_allow_hosts(&allow_hosts)?;
    }
    config.profile = profile;
    if let Some(agent_id) = agent_id {
        config.agent_id = agent_id;
    }
    if let Some(toolsets) = toolsets {
        config.toolsets = toolsets;
    } else if !has_imported_base {
        config.toolsets = ToolsetConfig::for_profile(profile);
    }
    if room.is_some() && !optional_section_is_legal(OptionalSection::Room, mode, &config.toolsets) {
        return Err(anyhow!("room_config_requires_room_toolset"));
    }
    if let Some(confirmation_language) = confirmation_language {
        config.confirmation_language = confirmation_language;
    } else if !has_imported_base {
        config.confirmation_language = match ui_language {
            UiLanguage::ZhCn => "zh-CN".to_string(),
            UiLanguage::En => "en".to_string(),
        };
    }

    if let Some(display_name) = display_name {
        config.display_name = display_name;
    }

    if let Some(workspace_root) = workspace_root {
        config.workspace_root = workspace_root;
        if path_policy.is_none() {
            config.path_policy = default_path_policy(&config.workspace_root);
        }
    }
    if let Some(path_policy) = path_policy {
        config.path_policy = path_policy;
    }
    if let Some(confirmation_provider) = confirmation_provider {
        config.confirmation_provider = confirmation_provider;
    }
    if let Some(limits) = limits {
        config.limits = limits;
    }
    if let Some(sandbox) = sandbox {
        config.sandbox = sandbox;
    }
    if let Some(shell) = shell {
        config.shell = shell;
    }
    if let Some(room) = room {
        let imported_skills = config.skills.clone();
        config.room = room;
        if has_imported_base {
            config.skills = imported_skills.clone();
            config.room.skills = imported_skills;
        } else {
            config.skills = config.room.skills.clone();
        }
    }
    if let Some(mcp_servers) = mcp_servers {
        config.mcp_servers = mcp_servers;
    }

    let mut pending = Vec::new();
    match mode {
        RuntimeMode::Standalone => {
            let default_secret_path = agentic_home()?.join("secrets").join("tunnel-api-key");
            let default_secret_reference =
                secret_reference(TunnelSecretSource::File, &default_secret_path);
            let tunnel_id = non_empty_or_placeholder(tunnel_id, STANDALONE_TUNNEL_ID_PLACEHOLDER);
            if tunnel_id == STANDALONE_TUNNEL_ID_PLACEHOLDER {
                push_pending(&mut pending, PendingAction::ReplaceTunnelId);
            }
            let api_key = non_empty_or_placeholder(tunnel_api_key, &default_secret_reference);
            if api_key == default_secret_reference {
                push_pending(&mut pending, PendingAction::ProvisionTunnelSecret);
            }
            config.tunnel = Some(TunnelConfig {
                tunnel_id,
                api_key,
                client: tunnel_client.unwrap_or_default(),
                hub_reporting: hub_reporting.unwrap_or_default(),
            });
            config.validate_standalone()?;
        }
        RuntimeMode::Hub => {
            config.hub.url = match hub_url {
                Some(value) if !value.trim().is_empty() => value,
                _ => {
                    push_pending(&mut pending, PendingAction::ConfigureHubUrl);
                    HUB_URL_PLACEHOLDER.to_string()
                }
            };
            if config.hub.url == HUB_URL_PLACEHOLDER {
                push_pending(&mut pending, PendingAction::ConfigureHubUrl);
            }
            config.hub.transport = hub_transport.unwrap_or_else(|| config.hub.transport.clone());
            config.hub.agent_secret = match agent_secret {
                Some(value) if !value.expose().trim().is_empty() => value.expose().to_string(),
                _ => HUB_AGENT_SECRET_PLACEHOLDER.to_string(),
            };
            if config.hub.agent_secret.trim().is_empty()
                || config.hub.agent_secret == HUB_AGENT_SECRET_PLACEHOLDER
            {
                push_pending(&mut pending, PendingAction::ReplaceAgentSecret);
            }

            config.validate_mcp_servers()?;
            validate_hub_url_shape(&config.hub.url)?;
            validate_hub_transport(&config.hub.transport)?;
            if config.agent_id.trim().is_empty() {
                return Err(anyhow!("agent_id_required"));
            }
            if pending.is_empty() {
                config.validate_hub()?;
            }
        }
        RuntimeMode::Local => config.validate_local()?,
    }

    Ok(InitBuild {
        mode,
        profile,
        config,
        pending,
    })
}

pub(crate) fn optional_section_is_legal(
    section: OptionalSection,
    mode: RuntimeMode,
    toolsets: &ToolsetConfig,
) -> bool {
    match section {
        OptionalSection::Identity
        | OptionalSection::Workspace
        | OptionalSection::Confirmation
        | OptionalSection::Limits
        | OptionalSection::Sandbox
        | OptionalSection::Shell
        | OptionalSection::McpServers
        | OptionalSection::Toolsets
        | OptionalSection::Events => true,
        OptionalSection::Room => toolsets.is_enabled(ToolNamespace::Room),
        OptionalSection::TunnelClient | OptionalSection::HubReporting => {
            mode == RuntimeMode::Standalone
        }
    }
}

fn non_empty_or_placeholder(value: Option<String>, placeholder: &str) -> String {
    match value {
        Some(value) if !value.trim().is_empty() => value,
        _ => placeholder.to_string(),
    }
}

fn secret_reference(source: TunnelSecretSource, path: &std::path::Path) -> String {
    match source {
        TunnelSecretSource::File => format!("file:{}", path.display()),
        TunnelSecretSource::Environment => "env:AGENTIC_GPT_TUNNEL_API_KEY".to_string(),
    }
}

fn push_pending(pending: &mut Vec<PendingAction>, action: PendingAction) {
    if !pending.contains(&action) {
        pending.push(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_is_standalone_normal_with_safe_placeholders() {
        let built = build_config(InitInput::non_interactive_defaults(UiLanguage::En)).unwrap();
        assert_eq!(built.mode, RuntimeMode::Standalone);
        assert_eq!(built.profile, WorkerProfile::Normal);
        let tunnel = built.config.tunnel.as_ref().unwrap();
        assert_eq!(tunnel.tunnel_id, "tunnel_replace-me");
        assert!(tunnel.api_key.starts_with("file:"));
        assert!(built.pending.contains(&PendingAction::ReplaceTunnelId));
        assert!(built
            .pending
            .contains(&PendingAction::ProvisionTunnelSecret));
        built.config.validate_standalone().unwrap();
    }

    #[test]
    fn local_template_omits_tunnel_and_validates_locally() {
        let input = InitInput {
            mode: RuntimeMode::Local,
            profile: WorkerProfile::Normal,
            ..InitInput::non_interactive_defaults(UiLanguage::En)
        };
        let built = build_config(input).unwrap();
        assert!(built.config.tunnel.is_none());
        built.config.validate_local().unwrap();
    }

    #[test]
    fn hub_template_uses_supplied_connection_values() {
        let mut input = InitInput::non_interactive_defaults(UiLanguage::ZhCn);
        input.mode = RuntimeMode::Hub;
        input.hub_url = Some("https://hub.example.com".into());
        input.hub_transport = Some("sse".into());
        input.agent_id = Some("desk".into());
        input.agent_secret = Some(SecretValue::new("secret"));
        let built = build_config(input).unwrap();
        assert_eq!(built.config.confirmation_language, "zh-CN");
        built.config.validate_hub().unwrap();
    }

    #[test]
    fn templates_cover_all_runtime_modes_and_profiles() {
        for profile in [WorkerProfile::Normal, WorkerProfile::Room] {
            for mode in [
                RuntimeMode::Standalone,
                RuntimeMode::Hub,
                RuntimeMode::Local,
            ] {
                let mut input = InitInput::non_interactive_defaults(UiLanguage::En);
                input.mode = mode;
                input.profile = profile;
                if mode == RuntimeMode::Hub {
                    input.hub_url = Some("https://hub.example.com".to_string());
                    input.hub_transport = Some("websocket".to_string());
                    input.agent_id = Some("desk".to_string());
                    input.agent_secret = Some(SecretValue::new("hub-secret"));
                }
                let built = build_config(input).unwrap();
                assert_eq!(built.mode, mode);
                assert_eq!(built.profile, profile);
                match mode {
                    RuntimeMode::Standalone => built.config.validate_standalone().unwrap(),
                    RuntimeMode::Hub => built.config.validate_hub().unwrap(),
                    RuntimeMode::Local => built.config.validate_local().unwrap(),
                }
            }
        }
    }

    #[test]
    fn imported_base_survives_tui_managed_field_overlay() {
        let mut base = Config::default_config().unwrap();
        base.mode = RuntimeMode::Local;
        base.profile = WorkerProfile::Normal;
        base.display_name = "imported-display".to_string();
        base.hub.url = "https://inactive-hub.example.com".to_string();
        base.tunnel = Some(TunnelConfig {
            tunnel_id: "inactive-tunnel".to_string(),
            api_key: "env:IMPORTED_TUNNEL_KEY".to_string(),
            ..TunnelConfig::default()
        });
        base.room.timezone = "UTC".to_string();
        base.limits.max_concurrent_tasks = 9;
        base.extra
            .insert("futureField".to_string(), serde_json::json!(true));
        base.mcp_servers.insert(
            "imported".to_string(),
            crate::config::mcp_servers::McpServerConfig {
                enabled: true,
                transport: "stdio".to_string(),
                url: Some("node ./server.mjs".to_string()),
                auth: None,
            },
        );

        let mut input = InitInput::non_interactive_defaults(UiLanguage::En);
        input.mode = RuntimeMode::Local;
        input.profile = WorkerProfile::Normal;
        input.imported_base = Some(base);
        let built = build_config(input).unwrap();

        assert_eq!(built.config.display_name, "imported-display");
        assert_eq!(built.config.hub.url, "https://inactive-hub.example.com");
        assert_eq!(built.config.tunnel.unwrap().tunnel_id, "inactive-tunnel");
        assert_eq!(built.config.room.timezone, "UTC");
        assert_eq!(built.config.limits.max_concurrent_tasks, 9);
        assert_eq!(built.config.mcp_servers.len(), 1);
        assert_eq!(built.config.extra["futureField"], serde_json::json!(true));
    }
}
