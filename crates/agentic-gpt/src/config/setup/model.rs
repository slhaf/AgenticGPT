use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use crate::cli_i18n::UiLanguage;
use crate::config::{
    seed_path_policy, sparse_config_json, Config, EventsConfig, HttpMcpConfig, PathDefaultAction,
    ShellInitFile, ToolNamespace, ToolsetConfig, DEFAULT_HTTP_MCP_ALLOW_HOSTS,
    DEFAULT_HTTP_MCP_HOST, DEFAULT_HTTP_MCP_PORT, DEFAULT_SHELL_INIT_FILE, INTERNAL_EVENT_TYPES,
};
use crate::config_templates::{
    build_config, InitInput, OptionalSection, RuntimeMode, SecretValue, TunnelSecretSource,
};
use crate::tui::forms::OrderedMultiSelectState;
use crate::WorkerProfile;
use agentic_gpt_protocol::DEFAULT_PROCESS_RESPONSE_BYTES;

use super::validation;

const DEFAULT_SECRET_PATH: &str = "~/.agentic_gpt/secrets/tunnel-api-key";
const DEFAULT_HUB_URL: &str = "http://localhost:8787";
const DEFAULT_HUB_TRANSPORT: &str = "websocket";
const DEFAULT_AGENT_ID: &str = "laptop";
const DEFAULT_WORKSPACE_ROOT: &str = "~/.agentic_gpt/workspace";
const DEFAULT_TUNNEL_CACHE_DIR: &str = "~/.agentic_gpt/cache/tunnel-client";
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum SetupField {
    Mode,
    Profile,
    TunnelId,
    TunnelSecretSource,
    TunnelSecretPath,
    TunnelSecretEnvironment,
    ProvisionTunnelSecret,
    TunnelSecretValue,
    HttpMcpEnabled,
    HttpMcpHost,
    HttpMcpPort,
    HttpMcpPublicUrl,
    HttpMcpBearerToken,
    HttpMcpAllowHosts,
    HubUrl,
    HubTransport,
    AgentId,
    AgentSecret,
    HubEnabled,
    DisplayName,
    WorkspaceRoot,
    PathDefaultAction,
    PathReadOnlyPaths,
    PathWritePaths,
    PathDenyPaths,
    ConfirmationChannels,
    ConfirmationLanguage,
    MaxConcurrentTasks,
    MaxActiveProcesses,
    MaxFileSearchContextLines,
    ProcessResponseBytes,
    Bubblewrap,
    ShellInitFileMode,
    ShellInitFilePath,
    McpServerId,
    McpServerEnabled,
    McpServerTransport,
    McpServerEndpoint,
    McpServerBearerAuth,
    McpServerBearerToken,
    EmailImapHost,
    EmailImapPort,
    EmailImapTls,
    EmailSmtpHost,
    EmailSmtpPort,
    EmailSmtpTls,
    EmailUsername,
    EmailPassword,
    EmailFrom,
    EmailSyncInterval,
    EmailRetentionDays,
    EmailMaxStorageBytes,
    EmailSendPolicy,
    EmailTrashPolicy,
    RoomSecretServerUrl,
    RoomSecretAccount,
    RoomSecretClientId,
    RoomSecretClientSecret,
    RoomSecretMasterPassword,
    RoomSecretGetPolicy,
    RoomSecretUpsertPolicy,
    RoomSecretDeletePolicy,
    RoomSecretSyncWaitTimeoutMs,
    RoomTimezone,
    DiaryBoundaryHour,
    RepositoryRoot,
    RoomMaintenanceMode,
    RoomMaintenanceAutoPush,
    TunnelCacheDir,
    TunnelExecutable,
    HubReportingEnabled,
    HubReportingDetail,
    EventsLowTtlSeconds,
    EventProcessCompletedLevel,
    EventProcessFailedLevel,
    EventProcessRejectedLevel,
    EventProcessCancelledLevel,
    EventProcessTimedOutLevel,
    EventProcessDetachedLevel,
    EventProcessUnknownAfterRestartLevel,
    EventProcessSkippedLevel,
    EventSkillInstallCompletedLevel,
    EventSkillInstallFailedLevel,
    EventSkillInstallCancelledLevel,
    Toolsets,
    Catalog(usize),
}
impl SetupField {
    pub(crate) fn room_secret_key(self) -> Option<&'static str> {
        match self {
            Self::RoomSecretServerUrl => Some("serverUrl"),
            Self::RoomSecretAccount => Some("account"),
            Self::RoomSecretClientId => Some("clientId"),
            Self::RoomSecretClientSecret => Some("clientSecret"),
            Self::RoomSecretMasterPassword => Some("masterPassword"),
            Self::RoomSecretGetPolicy => Some("getPolicy"),
            Self::RoomSecretUpsertPolicy => Some("upsertPolicy"),
            Self::RoomSecretDeletePolicy => Some("deletePolicy"),
            Self::RoomSecretSyncWaitTimeoutMs => Some("syncWaitTimeoutMs"),
            _ => None,
        }
    }

    pub(crate) fn room_secret_is_sensitive(self) -> bool {
        matches!(
            self,
            Self::RoomSecretServerUrl
                | Self::RoomSecretAccount
                | Self::RoomSecretClientId
                | Self::RoomSecretClientSecret
                | Self::RoomSecretMasterPassword
        )
    }

    pub(crate) fn internal_event_type(self) -> Option<&'static str> {
        match self {
            Self::EventProcessCompletedLevel => Some("process.completed"),
            Self::EventProcessFailedLevel => Some("process.failed"),
            Self::EventProcessRejectedLevel => Some("process.rejected"),
            Self::EventProcessCancelledLevel => Some("process.cancelled"),
            Self::EventProcessTimedOutLevel => Some("process.timed_out"),
            Self::EventProcessDetachedLevel => Some("process.detached"),
            Self::EventProcessUnknownAfterRestartLevel => Some("process.unknown_after_restart"),
            Self::EventProcessSkippedLevel => Some("process.skipped"),
            Self::EventSkillInstallCompletedLevel => Some("skill_install.completed"),
            Self::EventSkillInstallFailedLevel => Some("skill_install.failed"),
            Self::EventSkillInstallCancelledLevel => Some("skill_install.cancelled"),
            _ => None,
        }
    }

    pub(crate) fn internal_event_override(event_type: &str) -> Option<Self> {
        match event_type {
            "process.completed" => Some(Self::EventProcessCompletedLevel),
            "process.failed" => Some(Self::EventProcessFailedLevel),
            "process.rejected" => Some(Self::EventProcessRejectedLevel),
            "process.cancelled" => Some(Self::EventProcessCancelledLevel),
            "process.timed_out" => Some(Self::EventProcessTimedOutLevel),
            "process.detached" => Some(Self::EventProcessDetachedLevel),
            "process.unknown_after_restart" => Some(Self::EventProcessUnknownAfterRestartLevel),
            "process.skipped" => Some(Self::EventProcessSkippedLevel),
            "skill_install.completed" => Some(Self::EventSkillInstallCompletedLevel),
            "skill_install.failed" => Some(Self::EventSkillInstallFailedLevel),
            "skill_install.cancelled" => Some(Self::EventSkillInstallCancelledLevel),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SectionStatus {
    Default,
    Configured,
    Disabled,
    NotApplicable,
}

#[derive(Default)]
pub(crate) struct SetupSeed {
    pub(crate) mode: Option<RuntimeMode>,
    pub(crate) profile: Option<WorkerProfile>,
    pub(crate) imported_base: Option<Config>,
    pub(crate) edit_existing: bool,
    pub(crate) tunnel_id: Option<String>,
    pub(crate) tunnel_api_key: Option<String>,
    pub(crate) hub_url: Option<String>,
    pub(crate) hub_transport: Option<String>,
    pub(crate) hub_enabled: Option<bool>,
    pub(crate) agent_id: Option<String>,
    pub(crate) agent_secret: Option<SecretValue>,
    pub(crate) http_mcp_enabled: Option<bool>,
    pub(crate) http_mcp_host: Option<String>,
    pub(crate) http_mcp_port: Option<u16>,
    pub(crate) http_mcp_public_url: Option<String>,
    pub(crate) http_mcp_bearer_token: Option<SecretValue>,
    pub(crate) http_mcp_allow_hosts: Option<String>,
}

impl fmt::Debug for SetupSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetupSeed")
            .field("mode", &self.mode)
            .field("profile", &self.profile)
            .field(
                "imported_base",
                &self.imported_base.as_ref().map(|_| "[REDACTED_BASE]"),
            )
            .field("tunnel_id", &self.tunnel_id)
            .field(
                "tunnel_api_key",
                &self.tunnel_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("hub_url", &self.hub_url)
            .field("hub_transport", &self.hub_transport)
            .field("hub_enabled", &self.hub_enabled)
            .field("agent_id", &self.agent_id)
            .field(
                "http_mcp_bearer_token",
                &self.http_mcp_bearer_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("http_mcp_public_url", &self.http_mcp_public_url)
            .field(
                "agent_secret",
                &self.agent_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct StandaloneDraft {
    pub(crate) tunnel_id: String,
    pub(crate) secret_source: TunnelSecretSource,
    pub(crate) secret_path: String,
    pub(crate) http_mcp_enabled: bool,
    pub(crate) http_mcp_host: String,
    pub(crate) http_mcp_port: String,
    pub(crate) public_url: String,
    pub(crate) http_mcp_bearer_token: Option<SecretValue>,
    pub(crate) http_mcp_allow_hosts: String,
    pub(crate) secret_environment: String,
    pub(crate) provision_secret_now: bool,
    pub(crate) secret_value: Option<SecretValue>,
}

#[derive(Debug)]
pub(crate) struct HubDraft {
    pub(crate) enabled: bool,
    pub(crate) hub_url: String,
    pub(crate) hub_transport: String,
    pub(crate) agent_id: String,
    pub(crate) agent_secret: Option<SecretValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IdentityDraft {
    pub(crate) display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceDraft {
    pub(crate) workspace_root: String,
    pub(crate) default_action: String,
    pub(crate) read_only_paths: String,
    pub(crate) write_paths: String,
    pub(crate) deny_paths: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConfirmationDraft {
    pub(crate) channels: String,
    pub(crate) language: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LimitsDraft {
    pub(crate) max_concurrent_tasks: String,
    pub(crate) max_active_processes: String,
    pub(crate) max_file_search_context_lines: String,
    pub(crate) process_response_bytes: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SandboxDraft {
    pub(crate) enabled: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShellDraft {
    pub(crate) init_file_mode: String,
    pub(crate) init_file_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpServerDraft {
    pub(crate) id: String,
    pub(crate) enabled: bool,
    pub(crate) transport: String,
    pub(crate) endpoint: String,
    pub(crate) bearer_auth: bool,
    pub(crate) bearer_token: Option<SecretValue>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolsetsDraft {
    pub(crate) selection: OrderedMultiSelectState,
}

impl ToolsetsDraft {
    fn from_config(config: &ToolsetConfig) -> Self {
        Self {
            selection: OrderedMultiSelectState::new(
                tool_namespace_options(),
                config
                    .enabled_names()
                    .into_iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
        }
    }
    pub(crate) fn to_config(&self) -> Result<ToolsetConfig, ()> {
        let mut config = ToolsetConfig::room();
        for namespace in ToolNamespace::all().iter().copied() {
            config.disable(namespace);
        }
        for name in self.selection.selected() {
            let namespace = ToolNamespace::parse(name).map_err(|_| ())?;
            config.enable(namespace);
        }
        Ok(config)
    }

    pub(crate) fn toggle_room_group(&mut self) {
        let room_names: Vec<String> = ToolNamespace::all()
            .iter()
            .copied()
            .filter(|namespace| namespace.is_room())
            .map(|namespace| namespace.as_str().to_string())
            .collect();
        let select = !room_names
            .iter()
            .all(|name| self.selection.selected().contains(name));
        let mut selected: Vec<String> = self
            .selection
            .selected()
            .iter()
            .filter(|name| !room_names.contains(name))
            .cloned()
            .collect();
        if select {
            selected.extend(room_names);
        }
        self.selection = OrderedMultiSelectState::new(tool_namespace_options(), selected);
    }
}

fn tool_namespace_options() -> Vec<String> {
    ToolNamespace::all()
        .iter()
        .map(|namespace| namespace.as_str().to_string())
        .collect()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct McpServersDraft {
    pub(crate) servers: Vec<McpServerDraft>,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct EmailDraft {
    pub(crate) values: BTreeMap<SetupField, String>,
}
impl fmt::Debug for EmailDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut values = self.values.clone();
        values.insert(SetupField::EmailPassword, "[REDACTED]".to_string());
        formatter
            .debug_struct("EmailDraft")
            .field("values", &values)
            .finish()
    }
}
impl EmailDraft {
    const FIELDS: &'static [(SetupField, &'static str, &'static str)] = &[
        (SetupField::EmailImapHost, "/account/imapHost", ""),
        (SetupField::EmailImapPort, "/account/imapPort", "993"),
        (SetupField::EmailImapTls, "/account/imapTls", "implicit"),
        (SetupField::EmailSmtpHost, "/account/smtpHost", ""),
        (SetupField::EmailSmtpPort, "/account/smtpPort", "465"),
        (SetupField::EmailSmtpTls, "/account/smtpTls", "implicit"),
        (SetupField::EmailUsername, "/account/username", ""),
        (SetupField::EmailPassword, "/account/password", ""),
        (SetupField::EmailFrom, "/account/from", ""),
        (SetupField::EmailSyncInterval, "/syncIntervalSeconds", ""),
        (SetupField::EmailRetentionDays, "/retentionDays", ""),
        (SetupField::EmailMaxStorageBytes, "/maxStorageBytes", ""),
        (SetupField::EmailSendPolicy, "/sendPolicy", ""),
        (SetupField::EmailTrashPolicy, "/trashPolicy", ""),
    ];
    pub(crate) fn from_config(config: &crate::config::EmailConfig) -> Self {
        let value = serde_json::to_value(config).expect("email configuration serializes");
        let values = Self::FIELDS
            .iter()
            .map(|(field, pointer, fallback)| {
                let text = value
                    .pointer(pointer)
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string())
                    })
                    .unwrap_or_else(|| (*fallback).to_string());
                (*field, text)
            })
            .collect();
        Self { values }
    }
    pub(crate) fn value(&self, field: SetupField) -> String {
        self.values.get(&field).cloned().unwrap_or_default()
    }
    pub(crate) fn to_config(
        &self,
    ) -> Result<crate::config::EmailConfig, (SetupField, &'static str)> {
        let mut value = serde_json::to_value(crate::config::EmailConfig::default())
            .expect("email configuration serializes");
        let mut account = serde_json::Map::new();
        for (field, pointer, _) in Self::FIELDS {
            let text = self.value(*field);
            let parsed = match field {
                SetupField::EmailImapPort | SetupField::EmailSmtpPort => serde_json::Value::from(
                    text.parse::<u16>()
                        .map_err(|_| (*field, "email_account_port_invalid"))?,
                ),
                SetupField::EmailSyncInterval
                | SetupField::EmailRetentionDays
                | SetupField::EmailMaxStorageBytes => serde_json::Value::from(
                    text.parse::<u64>()
                        .map_err(|_| (*field, "config_init_number_invalid"))?,
                ),
                SetupField::EmailImapTls | SetupField::EmailSmtpTls => {
                    serde_json::from_value::<crate::config::EmailTls>(serde_json::Value::String(
                        text.clone(),
                    ))
                    .map_err(|_| (*field, "email_tls_invalid"))?;
                    serde_json::Value::String(text)
                }
                SetupField::EmailSendPolicy | SetupField::EmailTrashPolicy => {
                    serde_json::from_value::<crate::config::EmailPolicy>(
                        serde_json::Value::String(text.clone()),
                    )
                    .map_err(|_| (*field, "email_policy_invalid"))?;
                    serde_json::Value::String(text)
                }
                _ => serde_json::Value::String(text),
            };
            if let Some(key) = pointer.strip_prefix("/account/") {
                account.insert(key.to_string(), parsed);
            } else {
                value[&pointer[1..]] = parsed;
            }
        }
        if [
            SetupField::EmailImapHost,
            SetupField::EmailSmtpHost,
            SetupField::EmailUsername,
            SetupField::EmailPassword,
            SetupField::EmailFrom,
        ]
        .iter()
        .any(|field| !self.value(*field).is_empty())
        {
            value["account"] = serde_json::Value::Object(account);
        }
        let config: crate::config::EmailConfig = serde_json::from_value(value)
            .map_err(|_| (SetupField::EmailImapHost, "email_config_invalid"))?;
        config.validate().map_err(|error| {
            let (field, code) = match error.to_string().as_str() {
                "email_sync_interval_seconds_invalid" => (
                    SetupField::EmailSyncInterval,
                    "email_sync_interval_seconds_invalid",
                ),
                "email_retention_days_invalid" => (
                    SetupField::EmailRetentionDays,
                    "email_retention_days_invalid",
                ),
                "email_max_storage_bytes_invalid" => (
                    SetupField::EmailMaxStorageBytes,
                    "email_max_storage_bytes_invalid",
                ),
                "email_account_password_reference_invalid" => (
                    SetupField::EmailPassword,
                    "email_account_password_reference_invalid",
                ),
                "email_account_from_invalid" => {
                    (SetupField::EmailFrom, "email_account_from_invalid")
                }
                "email_account_username_invalid" => {
                    (SetupField::EmailUsername, "email_account_username_invalid")
                }
                "email_account_host_invalid" => {
                    (SetupField::EmailImapHost, "email_account_host_invalid")
                }
                "email_account_port_invalid" => {
                    (SetupField::EmailImapPort, "email_account_port_invalid")
                }
                _ => (SetupField::EmailImapHost, "email_config_invalid"),
            };
            (field, code)
        })?;
        Ok(config)
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct RoomSecretDraft {
    pub(crate) values: BTreeMap<SetupField, String>,
}

impl Default for RoomSecretDraft {
    fn default() -> Self {
        Self::from_config(&crate::config::RoomSecretConfig::default())
    }
}

impl fmt::Debug for RoomSecretDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let values: BTreeMap<_, _> = self
            .values
            .iter()
            .map(|(field, value)| {
                (
                    *field,
                    if field.room_secret_is_sensitive() {
                        "[REDACTED]"
                    } else {
                        value.as_str()
                    },
                )
            })
            .collect();
        formatter
            .debug_struct("RoomSecretDraft")
            .field("values", &values)
            .finish()
    }
}

impl RoomSecretDraft {
    pub(crate) const FIELDS: &'static [SetupField] = &[
        SetupField::RoomSecretServerUrl,
        SetupField::RoomSecretAccount,
        SetupField::RoomSecretClientId,
        SetupField::RoomSecretClientSecret,
        SetupField::RoomSecretMasterPassword,
        SetupField::RoomSecretGetPolicy,
        SetupField::RoomSecretUpsertPolicy,
        SetupField::RoomSecretDeletePolicy,
        SetupField::RoomSecretSyncWaitTimeoutMs,
    ];

    pub(crate) fn from_config(config: &crate::config::RoomSecretConfig) -> Self {
        let value = serde_json::to_value(config).expect("Room Secret configuration serializes");
        let values = Self::FIELDS
            .iter()
            .map(|field| {
                let value = &value[field.room_secret_key().expect("Room Secret field")];
                let text = value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string());
                (*field, text)
            })
            .collect();
        Self { values }
    }

    pub(crate) fn value(&self, field: SetupField) -> String {
        self.values.get(&field).cloned().unwrap_or_default()
    }

    pub(crate) fn to_config(
        &self,
    ) -> Result<crate::config::RoomSecretConfig, (SetupField, &'static str)> {
        let mut value = serde_json::Map::new();
        for field in Self::FIELDS {
            let text = self.value(*field);
            let parsed = match field {
                SetupField::RoomSecretSyncWaitTimeoutMs => serde_json::Value::from(
                    text.parse::<u64>()
                        .map_err(|_| (*field, "room_secret_sync_wait_timeout_ms_invalid"))?,
                ),
                SetupField::RoomSecretGetPolicy
                | SetupField::RoomSecretUpsertPolicy
                | SetupField::RoomSecretDeletePolicy => {
                    if !matches!(text.as_str(), "allow" | "confirm") {
                        return Err((*field, "room_secret_policy_invalid"));
                    }
                    serde_json::Value::String(text)
                }
                _ => serde_json::Value::String(text),
            };
            value.insert(
                field
                    .room_secret_key()
                    .expect("Room Secret field")
                    .to_string(),
                parsed,
            );
        }
        let config: crate::config::RoomSecretConfig =
            serde_json::from_value(serde_json::Value::Object(value)).map_err(|_| {
                (
                    SetupField::RoomSecretServerUrl,
                    "room_secret_config_invalid",
                )
            })?;
        config
            .validate()
            .map_err(|error| match error.to_string().as_str() {
                "room_secret_server_url_invalid" => (
                    SetupField::RoomSecretServerUrl,
                    "room_secret_server_url_invalid",
                ),
                "room_secret_account_invalid" => {
                    (SetupField::RoomSecretAccount, "room_secret_account_invalid")
                }
                "room_secret_client_id_reference_invalid" => (
                    SetupField::RoomSecretClientId,
                    "room_secret_client_id_reference_invalid",
                ),
                "room_secret_client_secret_reference_invalid" => (
                    SetupField::RoomSecretClientSecret,
                    "room_secret_client_secret_reference_invalid",
                ),
                "room_secret_master_password_reference_invalid" => (
                    SetupField::RoomSecretMasterPassword,
                    "room_secret_master_password_reference_invalid",
                ),
                _ => (
                    SetupField::RoomSecretSyncWaitTimeoutMs,
                    "room_secret_sync_wait_timeout_ms_invalid",
                ),
            })?;
        Ok(config)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoomDraft {
    pub(crate) timezone: String,
    pub(crate) diary_boundary_hour: String,
    pub(crate) repository_root: String,
    pub(crate) maintenance_mode: String,
    pub(crate) maintenance_auto_push: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TunnelClientDraft {
    pub(crate) cache_dir: String,
    pub(crate) executable: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HubReportingDraft {
    pub(crate) enabled: bool,
    pub(crate) detail: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EventsDraft {
    pub(crate) low_ttl_seconds: String,
    pub(crate) internal_overrides: BTreeMap<String, String>,
}

impl EventsDraft {
    fn from_config(config: &EventsConfig) -> Self {
        let internal_overrides = INTERNAL_EVENT_TYPES
            .iter()
            .map(|event_type| {
                let level = config
                    .internal_overrides
                    .get(*event_type)
                    .map(|level| level.as_str())
                    .unwrap_or("inherit");
                ((*event_type).to_string(), level.to_string())
            })
            .collect();
        Self {
            low_ttl_seconds: config.low_ttl_seconds.to_string(),
            internal_overrides,
        }
    }
}

impl Default for EventsDraft {
    fn default() -> Self {
        Self::from_config(&EventsConfig::default())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OptionalDrafts {
    pub(crate) identity: Option<IdentityDraft>,
    pub(crate) workspace: Option<WorkspaceDraft>,
    pub(crate) confirmation: Option<ConfirmationDraft>,
    pub(crate) limits: Option<LimitsDraft>,
    pub(crate) sandbox: Option<SandboxDraft>,
    pub(crate) toolsets: Option<ToolsetsDraft>,
    pub(crate) shell: Option<ShellDraft>,
    pub(crate) mcp_servers: Option<McpServersDraft>,
    pub(crate) room: Option<RoomDraft>,
    pub(crate) room_secret: Option<RoomSecretDraft>,
    pub(crate) email: Option<EmailDraft>,
    pub(crate) tunnel_client: Option<TunnelClientDraft>,
    pub(crate) hub_reporting: Option<HubReportingDraft>,
    pub(crate) events: Option<EventsDraft>,
    pub(crate) catalog: BTreeMap<OptionalSection, CatalogDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OptionalSectionDraft {
    Identity(IdentityDraft),
    Workspace(WorkspaceDraft),
    Confirmation(ConfirmationDraft),
    Limits(LimitsDraft),
    Sandbox(SandboxDraft),
    Toolsets(ToolsetsDraft),
    McpServers(McpServersDraft),
    Room(RoomDraft),
    RoomSecret(RoomSecretDraft),
    Email(EmailDraft),
    TunnelClient(TunnelClientDraft),
    HubReporting(HubReportingDraft),
    Shell(ShellDraft),
    Events(EventsDraft),
    Catalog(CatalogDraft),
}

struct StandaloneSeed<'a> {
    tunnel_id: Option<String>,
    tunnel_api_key: Option<String>,
    http_mcp_enabled: Option<bool>,
    http_mcp_host: Option<String>,
    http_mcp_port: Option<u16>,
    http_mcp_public_url: Option<String>,
    http_mcp_bearer_token: Option<SecretValue>,
    http_mcp_allow_hosts: Option<String>,
    imported_http_mcp: Option<&'a HttpMcpConfig>,
}

impl OptionalSectionDraft {
    pub(crate) fn section(&self) -> OptionalSection {
        match self {
            Self::Identity(_) => OptionalSection::Identity,
            Self::Workspace(_) => OptionalSection::Workspace,
            Self::Confirmation(_) => OptionalSection::Confirmation,
            Self::Limits(_) => OptionalSection::Limits,
            Self::Sandbox(_) => OptionalSection::Sandbox,
            Self::Toolsets(_) => OptionalSection::Toolsets,
            Self::McpServers(_) => OptionalSection::McpServers,
            Self::Room(_) => OptionalSection::Room,
            Self::RoomSecret(_) => OptionalSection::RoomSecret,
            Self::Email(_) => OptionalSection::Email,
            Self::TunnelClient(_) => OptionalSection::TunnelClient,
            Self::HubReporting(_) => OptionalSection::HubReporting,
            Self::Events(_) => OptionalSection::Events,
            Self::Catalog(value) => value.section,
            Self::Shell(_) => OptionalSection::Shell,
        }
    }
}

pub(crate) struct SetupSession {
    selected_mode: RuntimeMode,
    selected_profile: WorkerProfile,
    imported_base: Option<Config>,
    pub(super) edit_existing: bool,
    standalone: StandaloneDraft,
    hub: HubDraft,
    optional: OptionalDrafts,
    language: UiLanguage,
    config_path: PathBuf,
    tunnel_seed_error: Option<&'static str>,
    http_mcp_seed_error: Option<&'static str>,
}

impl fmt::Debug for SetupSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetupSession")
            .field("selected_mode", &self.selected_mode)
            .field("selected_profile", &self.selected_profile)
            .field("standalone", &self.standalone)
            .field("hub", &self.hub)
            .field("optional", &self.optional)
            .field("language", &self.language)
            .field("config_path", &self.config_path)
            .finish()
    }
}

impl SetupSession {
    pub(crate) fn new(seed: SetupSeed, language: UiLanguage, config_path: PathBuf) -> Self {
        let imported_base = seed.imported_base;
        let selected_mode = seed
            .mode
            .or_else(|| imported_base.as_ref().map(|config| config.mode))
            .unwrap_or(RuntimeMode::Standalone);
        let selected_profile = seed
            .profile
            .or_else(|| imported_base.as_ref().map(|config| config.profile))
            .unwrap_or(WorkerProfile::Normal);
        let imported_tunnel_id = imported_base
            .as_ref()
            .and_then(|config| config.tunnel.as_ref())
            .map(|tunnel| tunnel.tunnel_id.clone());
        let imported_tunnel_api_key = imported_base
            .as_ref()
            .and_then(|config| config.tunnel.as_ref())
            .map(|tunnel| tunnel.api_key.clone());
        let imported_http_mcp = imported_base.as_ref().map(|config| &config.http_mcp);
        let (standalone, tunnel_seed_error, http_mcp_seed_error) =
            StandaloneDraft::from_seed(StandaloneSeed {
                tunnel_id: seed.tunnel_id.or(imported_tunnel_id),
                tunnel_api_key: seed.tunnel_api_key.or(imported_tunnel_api_key),
                http_mcp_enabled: seed.http_mcp_enabled,
                http_mcp_host: seed.http_mcp_host,
                http_mcp_port: seed.http_mcp_port,
                http_mcp_public_url: seed.http_mcp_public_url,
                http_mcp_bearer_token: seed.http_mcp_bearer_token,
                http_mcp_allow_hosts: seed.http_mcp_allow_hosts,
                imported_http_mcp,
            });
        let imported_hub = imported_base.as_ref().map(|config| &config.hub);
        let hub = HubDraft {
            enabled: seed
                .hub_enabled
                .unwrap_or_else(|| imported_hub.map(|hub| hub.enabled).unwrap_or(false)),
            hub_url: seed
                .hub_url
                .or_else(|| imported_hub.map(|hub| hub.url.clone()))
                .unwrap_or_else(|| DEFAULT_HUB_URL.to_string()),
            hub_transport: seed
                .hub_transport
                .or_else(|| imported_hub.map(|hub| hub.transport.clone()))
                .unwrap_or_else(|| DEFAULT_HUB_TRANSPORT.to_string()),
            agent_id: seed
                .agent_id
                .or_else(|| imported_base.as_ref().map(|config| config.agent_id.clone()))
                .unwrap_or_else(|| DEFAULT_AGENT_ID.to_string()),
            agent_secret: seed
                .agent_secret
                .or_else(|| imported_hub.map(|hub| SecretValue::new(hub.agent_secret.clone()))),
        };
        let optional = imported_base
            .as_ref()
            .map(optional_drafts_from_config)
            .unwrap_or_default();
        Self {
            selected_mode,
            selected_profile,
            edit_existing: seed.edit_existing,
            imported_base,
            standalone,
            hub,
            optional,
            language,
            config_path,
            tunnel_seed_error,
            http_mcp_seed_error,
        }
    }

    pub(crate) fn selected_mode(&self) -> RuntimeMode {
        self.selected_mode
    }

    pub(crate) fn selected_profile(&self) -> WorkerProfile {
        self.selected_profile
    }

    pub(crate) fn is_editing_existing(&self) -> bool {
        self.edit_existing
    }

    pub(crate) fn imported_base(&self) -> Option<&Config> {
        self.imported_base.as_ref()
    }

    pub(crate) fn language(&self) -> UiLanguage {
        self.language
    }

    pub(crate) fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }

    pub(crate) fn standalone(&self) -> &StandaloneDraft {
        &self.standalone
    }

    pub(crate) fn standalone_mut(&mut self) -> &mut StandaloneDraft {
        &mut self.standalone
    }

    pub(crate) fn hub(&self) -> &HubDraft {
        &self.hub
    }

    pub(crate) fn hub_mut(&mut self) -> &mut HubDraft {
        &mut self.hub
    }
    pub(crate) fn optional_draft(&self, section: OptionalSection) -> OptionalSectionDraft {
        self.optional.get(section).unwrap_or_else(|| {
            default_optional_draft_for_profile(self.language, section, self.selected_profile)
        })
    }

    pub(crate) fn auxiliary_hub_connection_enabled(&self) -> bool {
        self.selected_mode != RuntimeMode::Hub && self.hub.enabled
    }
    pub(crate) fn optional_drafts(&self) -> &OptionalDrafts {
        &self.optional
    }
    pub(crate) fn effective_toolsets(&self) -> ToolsetConfig {
        self.optional
            .toolsets
            .as_ref()
            .and_then(|draft| draft.to_config().ok())
            .unwrap_or_else(|| ToolsetConfig::for_profile(self.selected_profile))
    }

    pub(crate) fn set_mode(&mut self, mode: RuntimeMode) {
        self.selected_mode = mode;
    }

    pub(crate) fn set_profile(&mut self, profile: WorkerProfile) {
        self.selected_profile = profile;
    }

    pub(crate) fn available_optional_sections(&self) -> Vec<OptionalSection> {
        let toolsets = self.effective_toolsets();
        validation::available_optional_sections(self.selected_mode, &toolsets)
    }

    pub(crate) fn section_status(&self, section: OptionalSection) -> SectionStatus {
        self.section_status_with_override(section, None)
    }

    pub(crate) fn section_status_with_override(
        &self,
        section: OptionalSection,
        override_draft: Option<&OptionalSectionDraft>,
    ) -> SectionStatus {
        let draft = override_draft
            .filter(|draft| draft.section() == section)
            .cloned()
            .unwrap_or_else(|| self.optional_draft(section));
        let toolsets = match override_draft {
            Some(OptionalSectionDraft::Toolsets(value)) => value
                .to_config()
                .unwrap_or_else(|_| self.effective_toolsets()),
            _ => self.effective_toolsets(),
        };
        let policy_enabled = match override_draft {
            Some(OptionalSectionDraft::Catalog(value))
                if value.section == OptionalSection::SecurityPolicy =>
            {
                catalog_bool_value(value, &["policy", "enabled"])
                    .unwrap_or_else(|| self.policy_enabled())
            }
            _ => self.policy_enabled(),
        };
        if !validation::section_is_legal(section, self.selected_mode, &toolsets) {
            return SectionStatus::NotApplicable;
        }

        if self.section_is_closed(section, &draft, &toolsets, policy_enabled) {
            return SectionStatus::Disabled;
        }

        let changed_toolset = match section {
            OptionalSection::Browser => {
                self.toolset_differs_from_default(ToolNamespace::Browser, &toolsets)
            }
            OptionalSection::Skills => {
                self.toolset_differs_from_default(ToolNamespace::Skills, &toolsets)
            }
            OptionalSection::ConfigManagement => {
                self.toolset_differs_from_default(ToolNamespace::Config, &toolsets)
            }
            OptionalSection::McpServers => {
                self.toolset_differs_from_default(ToolNamespace::Mcp, &toolsets)
            }
            OptionalSection::Email => {
                self.toolset_differs_from_default(ToolNamespace::RoomEmail, &toolsets)
            }
            OptionalSection::RoomSecret => {
                self.toolset_differs_from_default(ToolNamespace::RoomSecret, &toolsets)
            }
            OptionalSection::Room => ToolNamespace::all()
                .iter()
                .copied()
                .filter(|namespace| namespace.uses_room_repository())
                .any(|namespace| self.toolset_differs_from_default(namespace, &toolsets)),
            OptionalSection::Toolsets => {
                toolsets != ToolsetConfig::for_profile(self.selected_profile)
            }
            _ => false,
        };
        if changed_toolset || !self.draft_matches_default(section, &draft) {
            SectionStatus::Configured
        } else {
            SectionStatus::Default
        }
    }

    fn policy_enabled(&self) -> bool {
        self.optional
            .catalog
            .get(&OptionalSection::SecurityPolicy)
            .and_then(|draft| catalog_bool_value(draft, &["policy", "enabled"]))
            .unwrap_or(true)
    }

    fn toolset_differs_from_default(
        &self,
        namespace: ToolNamespace,
        toolsets: &ToolsetConfig,
    ) -> bool {
        toolsets.is_enabled(namespace)
            != ToolsetConfig::for_profile(self.selected_profile).is_enabled(namespace)
    }

    fn section_is_closed(
        &self,
        section: OptionalSection,
        draft: &OptionalSectionDraft,
        toolsets: &ToolsetConfig,
        policy_enabled: bool,
    ) -> bool {
        match section {
            OptionalSection::Browser => !toolsets.is_enabled(ToolNamespace::Browser),
            OptionalSection::Skills => !toolsets.is_enabled(ToolNamespace::Skills),
            OptionalSection::ConfigManagement => !toolsets.is_enabled(ToolNamespace::Config),
            OptionalSection::McpServers => {
                !toolsets.is_enabled(ToolNamespace::Mcp)
                    || matches!(draft, OptionalSectionDraft::McpServers(value)
                        if !value.servers.is_empty() && value.servers.iter().all(|server| !server.enabled))
            }
            OptionalSection::Email => !toolsets.is_enabled(ToolNamespace::RoomEmail),
            OptionalSection::RoomSecret => !toolsets.is_enabled(ToolNamespace::RoomSecret),
            OptionalSection::Room => !ToolNamespace::all()
                .iter()
                .copied()
                .filter(|namespace| namespace.uses_room_repository())
                .any(|namespace| toolsets.is_enabled(namespace)),
            OptionalSection::Toolsets => toolsets.enabled.is_empty(),
            OptionalSection::SecurityPolicy => !policy_enabled,
            OptionalSection::Sandbox => {
                !policy_enabled
                    || matches!(draft, OptionalSectionDraft::Sandbox(value) if !value.enabled)
            }
            OptionalSection::HubReporting => {
                matches!(draft, OptionalSectionDraft::HubReporting(value) if !value.enabled)
            }
            _ => false,
        }
    }

    fn draft_matches_default(
        &self,
        section: OptionalSection,
        draft: &OptionalSectionDraft,
    ) -> bool {
        let default =
            default_optional_draft_for_profile(self.language, section, self.selected_profile);
        if let OptionalSectionDraft::Identity(current) = draft {
            let ui_default = default_optional_draft_for_profile(
                self.language,
                OptionalSection::Identity,
                self.selected_profile,
            );
            let persisted_default = Config::default_config()
                .expect("default config")
                .display_name;
            if self.imported_base.is_none() && self.optional.get(section).is_none() {
                return current.display_name.trim() == persisted_default
                    || matches!(ui_default, OptionalSectionDraft::Identity(value)
                        if current.display_name.trim() == value.display_name);
            }
            return current.display_name.trim() == persisted_default;
        }
        if section == OptionalSection::SecurityPolicy {
            let mut config = Config::default_config().expect("default config");
            config.profile = self.selected_profile;
            config.policy.command = crate::config::seed_command_policy(self.selected_profile);
            let default = CatalogDraft::from_config(section, &config);
            if let OptionalSectionDraft::Catalog(current) = draft {
                let mut effective = self
                    .imported_base
                    .as_ref()
                    .map(|config| CatalogDraft::from_config(section, config))
                    .unwrap_or_else(|| default.clone());
                for (index, value) in &current.values {
                    if current.initial.get(index) != Some(value) {
                        effective.values.insert(*index, value.clone());
                    }
                }
                return catalog_values_match(&effective, &default);
            }
        }
        match (draft, &default) {
            (
                OptionalSectionDraft::Workspace(current),
                OptionalSectionDraft::Workspace(default),
            ) => {
                current.default_action == default.default_action
                    && paths_match(&current.read_only_paths, &default.read_only_paths)
                    && paths_match(&current.write_paths, &default.write_paths)
                    && paths_match(&current.deny_paths, &default.deny_paths)
                    && paths_match_one(&current.workspace_root, &default.workspace_root)
            }
            (OptionalSectionDraft::Limits(current), OptionalSectionDraft::Limits(default)) => {
                limits_match(current, default)
            }
            (OptionalSectionDraft::Room(current), OptionalSectionDraft::Room(default)) => {
                current.timezone == default.timezone
                    && numeric_text_matches(
                        &current.diary_boundary_hour,
                        &default.diary_boundary_hour,
                    )
                    && paths_match_one(&current.repository_root, &default.repository_root)
                    && current.maintenance_mode == default.maintenance_mode
                    && current.maintenance_auto_push == default.maintenance_auto_push
            }
            (OptionalSectionDraft::Events(current), OptionalSectionDraft::Events(default)) => {
                numeric_text_matches(&current.low_ttl_seconds, &default.low_ttl_seconds)
                    && current.internal_overrides == default.internal_overrides
            }
            (OptionalSectionDraft::Toolsets(current), OptionalSectionDraft::Toolsets(default)) => {
                current.to_config().ok() == default.to_config().ok()
            }
            (OptionalSectionDraft::Catalog(current), OptionalSectionDraft::Catalog(default)) => {
                catalog_values_match(current, default)
            }
            (
                OptionalSectionDraft::TunnelClient(current),
                OptionalSectionDraft::TunnelClient(default),
            ) => {
                paths_match_one(&current.cache_dir, &default.cache_dir)
                    && optional_path_matches(&current.executable, &default.executable)
            }
            _ => *draft == default,
        }
    }

    pub(crate) fn validate_basic(&self) -> Result<(), validation::ValidationErrors> {
        validation::validate_basic(self)
    }

    pub(crate) fn validate_connection(&self) -> Result<(), validation::ValidationErrors> {
        validation::validate_connection(self)
    }

    pub(crate) fn validate_field(
        &self,
        field: SetupField,
    ) -> Result<(), validation::ValidationErrors> {
        validation::validate_field(self, field)
    }

    pub(crate) fn save_optional_section(
        &mut self,
        draft: OptionalSectionDraft,
    ) -> Result<(), validation::ValidationErrors> {
        validation::save_optional_section(self, draft)
    }

    pub(crate) fn validate_optional_draft(
        &self,
        draft: &OptionalSectionDraft,
    ) -> Result<(), validation::ValidationErrors> {
        validation::validate_optional_draft(self, draft)
    }

    pub(crate) fn save_optional_section_for_review(
        &mut self,
        draft: OptionalSectionDraft,
    ) -> Result<(), validation::ValidationErrors> {
        validation::validate_optional_draft(self, &draft)?;
        self.optional.set(draft);
        Ok(())
    }

    pub(crate) fn validate_for_review(&self) -> Result<(), validation::ValidationErrors> {
        validation::validate_for_review(self)
    }

    pub(crate) fn build_active_input(&self) -> Result<InitInput, validation::ValidationErrors> {
        validation::build_active_input(self)
    }

    pub(crate) fn redacted_config_json(&self) -> anyhow::Result<String> {
        let input = self
            .build_active_input()
            .map_err(|_| anyhow::anyhow!("config_init_preview_invalid"))?;
        let built = build_config(input)?;
        sparse_config_json(&built.config, true)
    }

    pub(super) fn tunnel_seed_error(&self) -> Option<&'static str> {
        self.tunnel_seed_error
    }
    pub(super) fn http_mcp_seed_error(&self) -> Option<&'static str> {
        self.http_mcp_seed_error
    }

    pub(super) fn replace_optional(&mut self, draft: OptionalSectionDraft) {
        self.optional.set(draft);
    }
}

impl StandaloneDraft {
    fn from_seed(seed: StandaloneSeed<'_>) -> (Self, Option<&'static str>, Option<&'static str>) {
        let StandaloneSeed {
            tunnel_id,
            tunnel_api_key,
            http_mcp_enabled,
            http_mcp_host,
            http_mcp_port,
            http_mcp_public_url,
            http_mcp_bearer_token,
            http_mcp_allow_hosts,
            imported_http_mcp,
        } = seed;
        let mut draft = Self {
            tunnel_id: tunnel_id.unwrap_or_default(),
            secret_source: TunnelSecretSource::File,
            secret_path: DEFAULT_SECRET_PATH.to_string(),
            secret_environment: String::new(),
            provision_secret_now: false,
            secret_value: None,
            http_mcp_enabled: http_mcp_enabled
                .or_else(|| imported_http_mcp.map(|config| config.enabled))
                .unwrap_or(false),
            http_mcp_host: http_mcp_host
                .or_else(|| imported_http_mcp.map(|config| config.host.clone()))
                .unwrap_or_else(|| DEFAULT_HTTP_MCP_HOST.to_string()),
            http_mcp_port: http_mcp_port
                .or_else(|| imported_http_mcp.map(|config| config.port))
                .unwrap_or(DEFAULT_HTTP_MCP_PORT)
                .to_string(),
            public_url: http_mcp_public_url
                .or_else(|| imported_http_mcp.and_then(|config| config.public_url.clone()))
                .unwrap_or_default(),
            http_mcp_bearer_token: None,
            http_mcp_allow_hosts: http_mcp_allow_hosts
                .or_else(|| {
                    imported_http_mcp
                        .map(|config| serde_json::to_string(&config.allow_hosts).unwrap())
                })
                .unwrap_or_else(|| {
                    serde_json::to_string(&Some(
                        DEFAULT_HTTP_MCP_ALLOW_HOSTS
                            .iter()
                            .map(|host| (*host).to_string())
                            .collect::<Vec<_>>(),
                    ))
                    .unwrap()
                }),
        };
        let mut error = None;
        if let Some(reference) = tunnel_api_key {
            if let Some(path) = reference.strip_prefix("file:") {
                draft.secret_source = TunnelSecretSource::File;
                draft.secret_path = path.trim().to_string();
                draft.secret_environment.clear();
                if draft.secret_path.is_empty() {
                    error = Some("config_init_secret_path_invalid");
                }
            } else if let Some(name) = reference.strip_prefix("env:") {
                draft.secret_source = TunnelSecretSource::Environment;
                draft.secret_environment = name.trim().to_string();
                draft.secret_path.clear();
            } else if reference.trim().is_empty() {
                draft.secret_source = TunnelSecretSource::File;
                draft.secret_path.clear();
                error = Some("config_init_secret_path_invalid");
            } else {
                // Do not copy an unrecognised reference into a renderable
                // buffer: the CLI contract accepts only file:/PATH or env:NAME.
                // The user can replace the empty field after seeing the safe
                // validation error.
                draft.secret_source = TunnelSecretSource::File;
                draft.secret_path.clear();
                draft.secret_environment.clear();
                error = Some("tunnel_api_key_reference_plaintext_rejected");
            }
        }

        let seeded_http_token = http_mcp_bearer_token.or_else(|| {
            imported_http_mcp
                .filter(|config| !config.bearer_token.is_empty())
                .map(|config| SecretValue::new(config.bearer_token.clone()))
        });
        let (http_token, http_error) = match seeded_http_token {
            Some(token) if token.expose().is_empty() => (Some(token), None),
            Some(token)
                if crate::config::validate_http_mcp_bearer_token(token.expose()).is_ok() =>
            {
                (Some(token), None)
            }
            Some(_) => (None, Some("http_mcp_bearer_token_reference_invalid")),
            None => (None, None),
        };
        draft.http_mcp_bearer_token = http_token;
        (draft, error, http_error)
    }
}

impl OptionalDrafts {
    fn get(&self, section: OptionalSection) -> Option<OptionalSectionDraft> {
        match section {
            OptionalSection::Identity => self.identity.clone().map(OptionalSectionDraft::Identity),
            OptionalSection::Workspace => {
                self.workspace.clone().map(OptionalSectionDraft::Workspace)
            }
            OptionalSection::Confirmation => self
                .confirmation
                .clone()
                .map(OptionalSectionDraft::Confirmation),
            OptionalSection::Limits => self.limits.clone().map(OptionalSectionDraft::Limits),
            OptionalSection::Sandbox => self.sandbox.clone().map(OptionalSectionDraft::Sandbox),
            OptionalSection::Shell => self.shell.clone().map(OptionalSectionDraft::Shell),
            OptionalSection::Toolsets => self.toolsets.clone().map(OptionalSectionDraft::Toolsets),
            OptionalSection::McpServers => self
                .mcp_servers
                .clone()
                .map(OptionalSectionDraft::McpServers),
            OptionalSection::Room => self.room.clone().map(OptionalSectionDraft::Room),
            OptionalSection::RoomSecret => self
                .room_secret
                .clone()
                .map(OptionalSectionDraft::RoomSecret),
            OptionalSection::Email => self.email.clone().map(OptionalSectionDraft::Email),
            OptionalSection::TunnelClient => self
                .tunnel_client
                .clone()
                .map(OptionalSectionDraft::TunnelClient),
            OptionalSection::HubReporting => self
                .hub_reporting
                .clone()
                .map(OptionalSectionDraft::HubReporting),
            OptionalSection::Events => self.events.clone().map(OptionalSectionDraft::Events),
            OptionalSection::Browser
            | OptionalSection::Skills
            | OptionalSection::SecurityPolicy
            | OptionalSection::ConfigManagement => self
                .catalog
                .get(&section)
                .cloned()
                .map(OptionalSectionDraft::Catalog),
        }
    }

    fn set(&mut self, draft: OptionalSectionDraft) {
        match draft {
            OptionalSectionDraft::Identity(value) => self.identity = Some(value),
            OptionalSectionDraft::Workspace(value) => self.workspace = Some(value),
            OptionalSectionDraft::Confirmation(value) => self.confirmation = Some(value),
            OptionalSectionDraft::Limits(value) => self.limits = Some(value),
            OptionalSectionDraft::Sandbox(value) => self.sandbox = Some(value),
            OptionalSectionDraft::Shell(value) => self.shell = Some(value),
            OptionalSectionDraft::Toolsets(value) => self.toolsets = Some(value),
            OptionalSectionDraft::McpServers(value) => self.mcp_servers = Some(value),
            OptionalSectionDraft::Room(value) => self.room = Some(value),
            OptionalSectionDraft::RoomSecret(value) => self.room_secret = Some(value),
            OptionalSectionDraft::Email(value) => self.email = Some(value),
            OptionalSectionDraft::TunnelClient(value) => self.tunnel_client = Some(value),
            OptionalSectionDraft::HubReporting(value) => self.hub_reporting = Some(value),
            OptionalSectionDraft::Events(value) => self.events = Some(value),
            OptionalSectionDraft::Catalog(value) => {
                self.catalog.insert(value.section, value);
            }
        }
    }
}

pub(crate) fn default_optional_draft(
    language: UiLanguage,
    section: OptionalSection,
) -> OptionalSectionDraft {
    default_optional_draft_for_profile(language, section, WorkerProfile::Normal)
}

pub(crate) fn default_optional_draft_for_profile(
    language: UiLanguage,
    section: OptionalSection,
    profile: WorkerProfile,
) -> OptionalSectionDraft {
    match section {
        OptionalSection::Browser
        | OptionalSection::Skills
        | OptionalSection::SecurityPolicy
        | OptionalSection::ConfigManagement => OptionalSectionDraft::Catalog(
            CatalogDraft::from_config(section, &Config::default_config().expect("default config")),
        ),
        OptionalSection::Identity => OptionalSectionDraft::Identity(IdentityDraft {
            display_name: "AgenticGPT agent".to_string(),
        }),
        OptionalSection::Workspace => {
            let workspace_root = PathBuf::from(DEFAULT_WORKSPACE_ROOT);
            let defaults = seed_path_policy(&workspace_root);
            OptionalSectionDraft::Workspace(WorkspaceDraft {
                workspace_root: DEFAULT_WORKSPACE_ROOT.to_string(),
                default_action: path_default_action_label(defaults.default_action).to_string(),
                read_only_paths: serialize_paths(&defaults.read_only),
                write_paths: serialize_paths(&defaults.write),
                deny_paths: serialize_paths(&defaults.deny),
            })
        }
        OptionalSection::Confirmation => OptionalSectionDraft::Confirmation(ConfirmationDraft {
            channels: r#"["freedesktop","ntfy"]"#.to_string(),
            language: match language {
                UiLanguage::En => "en".to_string(),
                UiLanguage::ZhCn => "zh-CN".to_string(),
            },
        }),
        OptionalSection::Limits => OptionalSectionDraft::Limits(LimitsDraft {
            max_concurrent_tasks: "2".to_string(),
            max_active_processes: "auto".to_string(),
            max_file_search_context_lines: "5".to_string(),
            process_response_bytes: DEFAULT_PROCESS_RESPONSE_BYTES.to_string(),
        }),
        OptionalSection::Sandbox => OptionalSectionDraft::Sandbox(SandboxDraft { enabled: false }),
        OptionalSection::Shell => OptionalSectionDraft::Shell(ShellDraft {
            init_file_mode: "default".to_string(),
            init_file_path: DEFAULT_SHELL_INIT_FILE.to_string(),
        }),
        OptionalSection::Toolsets => OptionalSectionDraft::Toolsets(ToolsetsDraft::from_config(
            &ToolsetConfig::for_profile(profile),
        )),
        OptionalSection::McpServers => OptionalSectionDraft::McpServers(McpServersDraft::default()),
        OptionalSection::Email => OptionalSectionDraft::Email(EmailDraft::from_config(
            &crate::config::EmailConfig::default(),
        )),
        OptionalSection::Room => OptionalSectionDraft::Room(RoomDraft {
            timezone: "Asia/Shanghai".to_string(),
            diary_boundary_hour: "5".to_string(),
            repository_root: String::new(),
            maintenance_mode: "local".to_string(),
            maintenance_auto_push: false,
        }),
        OptionalSection::RoomSecret => OptionalSectionDraft::RoomSecret(RoomSecretDraft::default()),
        OptionalSection::TunnelClient => OptionalSectionDraft::TunnelClient(TunnelClientDraft {
            cache_dir: DEFAULT_TUNNEL_CACHE_DIR.to_string(),
            executable: String::new(),
        }),
        OptionalSection::HubReporting => OptionalSectionDraft::HubReporting(HubReportingDraft {
            enabled: false,
            detail: "metadata".to_string(),
        }),
        OptionalSection::Events => {
            OptionalSectionDraft::Events(EventsDraft::from_config(&EventsConfig::default()))
        }
    }
}

fn serialize_paths(paths: &[PathBuf]) -> String {
    serde_json::to_string(paths).unwrap_or_else(|_| "[]".to_string())
}

fn optional_drafts_from_config(config: &Config) -> OptionalDrafts {
    let tunnel = config.tunnel.as_ref();
    OptionalDrafts {
        identity: Some(IdentityDraft {
            display_name: config.display_name.clone(),
        }),
        workspace: Some(WorkspaceDraft {
            workspace_root: config.workspace_root.to_string_lossy().into_owned(),
            default_action: path_default_action_label(config.policy.path.default_action)
                .to_string(),
            read_only_paths: serialize_paths(&config.policy.path.read_only),
            write_paths: serialize_paths(&config.policy.path.write),
            deny_paths: serialize_paths(&config.policy.path.deny),
        }),
        confirmation: Some(ConfirmationDraft {
            channels: config.confirmation_provider.channels_json(),
            language: config.confirmation_language.clone(),
        }),
        limits: Some(LimitsDraft {
            max_concurrent_tasks: config.limits.max_concurrent_tasks.to_string(),
            max_active_processes: config.limits.max_active_processes.configured_label(),
            max_file_search_context_lines: config.limits.max_file_search_context_lines.to_string(),
            process_response_bytes: config.limits.process_response_bytes.to_string(),
        }),
        sandbox: Some(SandboxDraft {
            enabled: config.policy.bubblewrap,
        }),
        shell: Some(match &config.shell.init_file {
            ShellInitFile::Default => ShellDraft {
                init_file_mode: "default".to_string(),
                init_file_path: DEFAULT_SHELL_INIT_FILE.to_string(),
            },
            ShellInitFile::Disabled => ShellDraft {
                init_file_mode: "disabled".to_string(),
                init_file_path: DEFAULT_SHELL_INIT_FILE.to_string(),
            },
            ShellInitFile::Path(path) => ShellDraft {
                init_file_mode: "path".to_string(),
                init_file_path: path.clone(),
            },
        }),
        toolsets: Some(ToolsetsDraft::from_config(&config.toolsets)),
        mcp_servers: Some(McpServersDraft {
            servers: config
                .mcp_servers
                .iter()
                .map(|(id, server)| McpServerDraft {
                    id: id.clone(),
                    enabled: server.enabled,
                    transport: server.transport.clone(),
                    endpoint: server.url.clone().unwrap_or_default(),
                    bearer_auth: matches!(
                        server.auth,
                        Some(crate::config::mcp_servers::McpServerAuthConfig::Bearer { .. })
                    ),
                    bearer_token: server.auth.as_ref().map(
                        |crate::config::mcp_servers::McpServerAuthConfig::Bearer { token }| {
                            SecretValue::new(token.clone())
                        },
                    ),
                })
                .collect(),
        }),
        email: Some(EmailDraft::from_config(&config.email)),
        room: Some(RoomDraft {
            timezone: config.room.timezone.clone(),
            diary_boundary_hour: config.room.diary_day_boundary_hour.to_string(),
            repository_root: config
                .room
                .repository_root
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            maintenance_mode: format!("{:?}", config.room.maintenance.mode).to_lowercase(),
            maintenance_auto_push: config.room.maintenance.auto_push,
        }),
        room_secret: Some(RoomSecretDraft::from_config(&config.room.secret)),
        tunnel_client: tunnel.map(|tunnel| TunnelClientDraft {
            cache_dir: tunnel.client.cache_dir.to_string_lossy().into_owned(),
            executable: tunnel
                .client
                .executable
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }),
        hub_reporting: tunnel.map(|tunnel| HubReportingDraft {
            enabled: tunnel.hub_reporting.enabled,
            detail: tunnel.hub_reporting.detail.to_string(),
        }),
        events: Some(EventsDraft::from_config(&config.events)),
        catalog: [
            OptionalSection::Browser,
            OptionalSection::Skills,
            OptionalSection::SecurityPolicy,
            OptionalSection::ConfigManagement,
        ]
        .into_iter()
        .map(|section| (section, CatalogDraft::from_config(section, config)))
        .collect(),
    }
}

fn path_default_action_label(action: PathDefaultAction) -> &'static str {
    match action {
        PathDefaultAction::Allow => "allow",
        PathDefaultAction::Deny => "deny",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli_i18n::UiLanguage;
    use crate::config_templates::{OptionalSection, RuntimeMode, SecretValue};
    use crate::WorkerProfile;

    use super::*;

    #[test]
    fn preview_does_not_disclose_transaction_secret_material() {
        let config_path = PathBuf::from("/tmp/config-preview.json");
        let secret_marker = "transaction-only-tunnel-secret";
        let secret_path = PathBuf::from("/tmp/preview-tunnel-secret");
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                profile: Some(WorkerProfile::Room),
                tunnel_id: Some("tunnel-preview".to_string()),
                tunnel_api_key: Some(format!("file:{}", secret_path.display())),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            config_path,
        );
        session.standalone_mut().provision_secret_now = true;
        session.standalone_mut().secret_value = Some(SecretValue::new(secret_marker));

        let preview: serde_json::Value =
            serde_json::from_str(&session.redacted_config_json().unwrap()).unwrap();
        assert!(!serde_json::to_string(&preview)
            .unwrap()
            .contains(secret_marker));
    }

    #[test]
    fn imported_base_seeds_reviewable_fields_without_requiring_an_editor_for_every_field() {
        let mut base = crate::config::Config::default_config().unwrap();
        base.mode = RuntimeMode::Local;
        base.display_name = "imported-display".to_string();
        base.limits.max_concurrent_tasks = 8;
        base.mcp_servers.insert(
            "imported".to_string(),
            crate::config::mcp_servers::McpServerConfig {
                enabled: true,
                transport: "stdio".to_string(),
                url: Some("node ./server.mjs".to_string()),
                auth: None,
            },
        );
        base.extra
            .insert("futureField".to_string(), serde_json::json!(true));
        let session = SetupSession::new(
            SetupSeed {
                imported_base: Some(base),
                mode: Some(RuntimeMode::Local),
                profile: Some(WorkerProfile::Normal),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/imported-review.json"),
        );

        match session.optional_draft(OptionalSection::Identity) {
            OptionalSectionDraft::Identity(draft) => {
                assert_eq!(draft.display_name, "imported-display")
            }
            other => panic!("unexpected draft: {other:?}"),
        }
        match session.optional_draft(OptionalSection::Limits) {
            OptionalSectionDraft::Limits(draft) => assert_eq!(draft.max_concurrent_tasks, "8"),
            other => panic!("unexpected draft: {other:?}"),
        }
        match session.optional_draft(OptionalSection::McpServers) {
            OptionalSectionDraft::McpServers(draft) => assert_eq!(draft.servers.len(), 1),
            other => panic!("unexpected draft: {other:?}"),
        }
        let preview = session.redacted_config_json().unwrap();
        assert!(preview.contains("futureField"));
        assert!(preview.contains("maxConcurrentTasks"));
    }

    #[test]
    fn mcp_server_draft_defaults_empty_and_saves_as_configured() {
        let mut session = SetupSession::new(
            SetupSeed::default(),
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        );
        assert!(matches!(
            session.optional_draft(OptionalSection::McpServers),
            OptionalSectionDraft::McpServers(McpServersDraft { ref servers }) if servers.is_empty()
        ));

        session
            .save_optional_section(OptionalSectionDraft::McpServers(McpServersDraft {
                servers: vec![McpServerDraft {
                    id: "secured_tools".into(),
                    enabled: true,
                    transport: "streamable-http".into(),
                    endpoint: "https://example.test/mcp".into(),
                    bearer_auth: true,
                    bearer_token: Some(SecretValue::new("mcp-token-marker")),
                }],
            }))
            .unwrap();

        assert_eq!(
            session.section_status(OptionalSection::McpServers),
            SectionStatus::Configured
        );
        let draft = session.optional_draft(OptionalSection::McpServers);
        let OptionalSectionDraft::McpServers(draft) = draft else {
            panic!("expected MCP draft");
        };
        assert!(draft.servers[0].bearer_auth);
        assert_eq!(
            draft.servers[0].bearer_token.as_ref().unwrap().expose(),
            "mcp-token-marker"
        );
        assert!(!format!("{draft:?}").contains("mcp-token-marker"));
    }
}

pub(crate) fn catalog_entries() -> Vec<crate::config_catalog::ConfigEntry> {
    static ENTRIES: std::sync::OnceLock<Vec<crate::config_catalog::ConfigEntry>> =
        std::sync::OnceLock::new();
    ENTRIES
        .get_or_init(|| {
            crate::config_catalog::entries()
                .expect("configuration catalog is validated before opening the editor")
                .into_iter()
                .filter(|entry| {
                    let path = entry.path.join(".");
                    !entry.path.iter().any(|part| part == "*")
                        && (entry.kind != "object" || path == "browser.runtime")
                        && entry.kind != "map"
                        && (path.starts_with("browser.")
                            || path.starts_with("skills.")
                            || path.starts_with("policy.command.")
                            || path == "policy.enabled"
                            || path == "backupLimit"
                            || path == "config.updatePolicy")
                })
                .collect()
        })
        .clone()
}

fn catalog_section(entry: &crate::config_catalog::ConfigEntry) -> OptionalSection {
    let path = entry.path.join(".");
    if path.starts_with("browser.") {
        OptionalSection::Browser
    } else if path.starts_with("skills.") {
        OptionalSection::Skills
    } else if path.starts_with("policy.command.") || path == "policy.enabled" {
        OptionalSection::SecurityPolicy
    } else {
        OptionalSection::ConfigManagement
    }
}

fn catalog_bool_value(draft: &CatalogDraft, path: &[&str]) -> Option<bool> {
    let index = catalog_entries().iter().position(|entry| {
        entry
            .path
            .iter()
            .map(|part| part.as_str())
            .eq(path.iter().copied())
    })?;
    serde_json::from_str(draft.values.get(&index)?).ok()
}

fn catalog_values_match(current: &CatalogDraft, default: &CatalogDraft) -> bool {
    if current.values.keys().ne(default.values.keys()) {
        return false;
    }
    let entries = catalog_entries();
    current.values.iter().all(|(index, current)| {
        let Some(default) = default.values.get(index) else {
            return false;
        };
        match entries.get(*index).map(|entry| entry.kind.as_str()) {
            Some("port" | "non-negative-integer") => numeric_text_matches(current, default),
            Some("auto-or-non-negative-integer") => auto_numeric_text_matches(current, default),
            _ => current == default,
        }
    })
}

fn limits_match(current: &LimitsDraft, default: &LimitsDraft) -> bool {
    numeric_text_matches(&current.max_concurrent_tasks, &default.max_concurrent_tasks)
        && auto_numeric_text_matches(&current.max_active_processes, &default.max_active_processes)
        && numeric_text_matches(
            &current.max_file_search_context_lines,
            &default.max_file_search_context_lines,
        )
        && numeric_text_matches(
            &current.process_response_bytes,
            &default.process_response_bytes,
        )
}

fn numeric_text_matches(current: &str, default: &str) -> bool {
    match (
        current.trim().parse::<u128>(),
        default.trim().parse::<u128>(),
    ) {
        (Ok(current), Ok(default)) => current == default,
        _ => current == default,
    }
}

fn auto_numeric_text_matches(current: &str, default: &str) -> bool {
    let current = current.trim();
    let default = default.trim();
    match (current, default) {
        ("auto", "auto") => true,
        ("auto", _) | (_, "auto") => false,
        _ => numeric_text_matches(current, default),
    }
}

fn paths_match(current: &str, default: &str) -> bool {
    match (
        serde_json::from_str::<Vec<PathBuf>>(current),
        serde_json::from_str::<Vec<PathBuf>>(default),
    ) {
        (Ok(current), Ok(default)) => normalize_paths(current) == normalize_paths(default),
        _ => current == default,
    }
}

fn paths_match_one(current: &str, default: &str) -> bool {
    normalize_path(PathBuf::from(current)) == normalize_path(PathBuf::from(default))
}

fn optional_path_matches(current: &str, default: &str) -> bool {
    if current.is_empty() || default.is_empty() {
        current == default
    } else {
        paths_match_one(current, default)
    }
}

fn normalize_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.into_iter().map(normalize_path).collect()
}

fn normalize_path(path: PathBuf) -> PathBuf {
    crate::exec::expand_pathbuf(&path).unwrap_or(path)
}

pub(crate) fn catalog_section_at(index: usize) -> OptionalSection {
    catalog_entries()
        .get(index)
        .map(catalog_section)
        .unwrap_or(OptionalSection::ConfigManagement)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CatalogDraft {
    pub(crate) section: OptionalSection,
    pub(crate) values: BTreeMap<usize, String>,
    initial: BTreeMap<usize, String>,
}

impl CatalogDraft {
    fn from_config(section: OptionalSection, config: &Config) -> Self {
        let value = serde_json::to_value(config).expect("config serializes");
        let values: BTreeMap<usize, String> = catalog_entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| catalog_section(entry) == section)
            .map(|(index, entry)| {
                let mut current = &value;
                for part in &entry.path {
                    current = &current[part];
                }
                let missing = current.is_null();
                let current = if current.is_null() {
                    entry.default_value.as_ref().unwrap_or(&entry.example)
                } else {
                    current
                };
                let text = if entry.kind == "object" {
                    if missing {
                        "automatic".to_owned()
                    } else {
                        "explicit".to_owned()
                    }
                } else {
                    current
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| current.to_string())
                };
                (index, text)
            })
            .collect();
        Self {
            section,
            initial: values.clone(),
            values,
        }
    }

    pub(crate) fn changes(&self) -> Result<Vec<agentic_gpt_protocol::ConfigChange>, usize> {
        catalog_entries()
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                catalog_section(entry) == self.section
                    && self.values.get(index) != self.initial.get(index)
            })
            .map(|(index, entry)| {
                if entry.kind == "object" {
                    return Ok(
                        if self
                            .values
                            .get(&index)
                            .is_some_and(|value| value == "automatic")
                        {
                            agentic_gpt_protocol::ConfigChange::Unset {
                                path: entry.path.clone(),
                            }
                        } else {
                            agentic_gpt_protocol::ConfigChange::Set {
                                path: entry.path.clone(),
                                value: serde_json::json!({}),
                            }
                        },
                    );
                }

                crate::config_catalog::parse_cli_change(
                    &entry.path.join("."),
                    self.values
                        .get(&index)
                        .map(String::as_str)
                        .unwrap_or("null"),
                )
                .map_err(|_| index)
            })
            .collect()
    }
}
