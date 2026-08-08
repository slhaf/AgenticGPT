use std::fmt;
use std::path::PathBuf;

use crate::cli_i18n::UiLanguage;
use crate::config::default_path_policy;
use crate::config_templates::{
    InitInput, OptionalSection, RuntimeMode, SecretValue, TunnelSecretSource,
};
use crate::WorkerProfile;

use super::validation;

const DEFAULT_TUNNEL_ID: &str = "tunnel_replace-me";
const DEFAULT_SECRET_PATH: &str = "~/.agentic_gpt/secrets/tunnel-api-key";
const DEFAULT_HUB_URL: &str = "http://localhost:8787";
const DEFAULT_HUB_TRANSPORT: &str = "websocket";
const DEFAULT_AGENT_ID: &str = "laptop";
const DEFAULT_WORKSPACE_ROOT: &str = "~/.agentic_gpt/workspace";
const DEFAULT_TUNNEL_CACHE_DIR: &str = "~/.agentic_gpt/cache/tunnel-client";
const DEFAULT_BUBBLEWRAP_PATH: &str = "bwrap";
const DEFAULT_RUNTIME_PATHS: &str = r#"["/usr","/bin","/lib","/lib64","/etc/ssl"]"#;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SetupField {
    Mode,
    Profile,
    TunnelId,
    TunnelSecretSource,
    TunnelSecretPath,
    TunnelSecretEnvironment,
    ProvisionTunnelSecret,
    TunnelSecretValue,
    HubUrl,
    HubTransport,
    AgentId,
    AgentSecret,
    DisplayName,
    WorkspaceRoot,
    WriteRoots,
    ReadOnlyRoots,
    DenyRoots,
    ConfirmationProvider,
    ConfirmationLanguage,
    MaxConcurrentTasks,
    MaxActiveJobs,
    MaxFileSearchContextLines,
    SandboxEnabled,
    BubblewrapPath,
    RequiredRuntimePaths,
    RoomTimezone,
    DiaryBoundaryHour,
    NotebookRoot,
    TunnelClientVersion,
    TunnelCacheDir,
    TunnelAutoDownload,
    TunnelExecutable,
    TunnelDownloadUrl,
    TunnelSha256,
    HubReportingEnabled,
    HubReportingDetail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SectionStatus {
    Default,
    Configured,
    NotApplicable,
}

#[derive(Default)]
pub(crate) struct SetupSeed {
    pub(crate) mode: Option<RuntimeMode>,
    pub(crate) profile: Option<WorkerProfile>,
    pub(crate) tunnel_id: Option<String>,
    pub(crate) tunnel_api_key: Option<String>,
    pub(crate) hub_url: Option<String>,
    pub(crate) hub_transport: Option<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) agent_secret: Option<SecretValue>,
}

impl fmt::Debug for SetupSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetupSeed")
            .field("mode", &self.mode)
            .field("profile", &self.profile)
            .field("tunnel_id", &self.tunnel_id)
            .field(
                "tunnel_api_key",
                &self.tunnel_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("hub_url", &self.hub_url)
            .field("hub_transport", &self.hub_transport)
            .field("agent_id", &self.agent_id)
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
    pub(crate) secret_environment: String,
    pub(crate) provision_secret_now: bool,
    pub(crate) secret_value: Option<SecretValue>,
}

#[derive(Debug)]
pub(crate) struct HubDraft {
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
    pub(crate) write_roots: String,
    pub(crate) read_only_roots: String,
    pub(crate) deny_roots: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConfirmationDraft {
    pub(crate) provider: String,
    pub(crate) language: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LimitsDraft {
    pub(crate) max_concurrent_tasks: String,
    pub(crate) max_active_jobs: String,
    pub(crate) max_file_search_context_lines: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SandboxDraft {
    pub(crate) enabled: bool,
    pub(crate) bubblewrap_path: String,
    pub(crate) required_runtime_paths: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RoomDraft {
    pub(crate) timezone: String,
    pub(crate) diary_boundary_hour: String,
    pub(crate) notebook_root: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TunnelClientDraft {
    pub(crate) version: String,
    pub(crate) cache_dir: String,
    pub(crate) auto_download: bool,
    pub(crate) executable: String,
    pub(crate) download_url: String,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HubReportingDraft {
    pub(crate) enabled: bool,
    pub(crate) detail: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OptionalDrafts {
    pub(crate) identity: Option<IdentityDraft>,
    pub(crate) workspace: Option<WorkspaceDraft>,
    pub(crate) confirmation: Option<ConfirmationDraft>,
    pub(crate) limits: Option<LimitsDraft>,
    pub(crate) sandbox: Option<SandboxDraft>,
    pub(crate) room: Option<RoomDraft>,
    pub(crate) tunnel_client: Option<TunnelClientDraft>,
    pub(crate) hub_reporting: Option<HubReportingDraft>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OptionalSectionDraft {
    Identity(IdentityDraft),
    Workspace(WorkspaceDraft),
    Confirmation(ConfirmationDraft),
    Limits(LimitsDraft),
    Sandbox(SandboxDraft),
    Room(RoomDraft),
    TunnelClient(TunnelClientDraft),
    HubReporting(HubReportingDraft),
}

impl OptionalSectionDraft {
    pub(crate) fn section(&self) -> OptionalSection {
        match self {
            Self::Identity(_) => OptionalSection::Identity,
            Self::Workspace(_) => OptionalSection::Workspace,
            Self::Confirmation(_) => OptionalSection::Confirmation,
            Self::Limits(_) => OptionalSection::Limits,
            Self::Sandbox(_) => OptionalSection::Sandbox,
            Self::Room(_) => OptionalSection::Room,
            Self::TunnelClient(_) => OptionalSection::TunnelClient,
            Self::HubReporting(_) => OptionalSection::HubReporting,
        }
    }
}

pub(crate) struct SetupSession {
    selected_mode: RuntimeMode,
    selected_profile: WorkerProfile,
    standalone: StandaloneDraft,
    hub: HubDraft,
    optional: OptionalDrafts,
    language: UiLanguage,
    config_path: PathBuf,
    tunnel_seed_error: Option<&'static str>,
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
        let selected_mode = seed.mode.unwrap_or(RuntimeMode::Standalone);
        let selected_profile = seed.profile.unwrap_or(WorkerProfile::Normal);
        let (standalone, tunnel_seed_error) =
            StandaloneDraft::from_seed(seed.tunnel_id, seed.tunnel_api_key);
        let hub = HubDraft {
            hub_url: seed.hub_url.unwrap_or_else(|| DEFAULT_HUB_URL.to_string()),
            hub_transport: seed
                .hub_transport
                .unwrap_or_else(|| DEFAULT_HUB_TRANSPORT.to_string()),
            agent_id: seed
                .agent_id
                .unwrap_or_else(|| DEFAULT_AGENT_ID.to_string()),
            agent_secret: seed.agent_secret,
        };
        Self {
            selected_mode,
            selected_profile,
            standalone,
            hub,
            optional: OptionalDrafts::default(),
            language,
            config_path,
            tunnel_seed_error,
        }
    }

    pub(crate) fn selected_mode(&self) -> RuntimeMode {
        self.selected_mode
    }

    pub(crate) fn selected_profile(&self) -> WorkerProfile {
        self.selected_profile
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

    pub(crate) fn optional_drafts(&self) -> &OptionalDrafts {
        &self.optional
    }

    pub(crate) fn set_mode(&mut self, mode: RuntimeMode) {
        self.selected_mode = mode;
    }

    pub(crate) fn set_profile(&mut self, profile: WorkerProfile) {
        self.selected_profile = profile;
    }

    pub(crate) fn available_optional_sections(&self) -> Vec<OptionalSection> {
        validation::available_optional_sections(self.selected_mode, self.selected_profile)
    }

    pub(crate) fn section_status(&self, section: OptionalSection) -> SectionStatus {
        if !validation::section_is_legal(section, self.selected_mode, self.selected_profile) {
            return SectionStatus::NotApplicable;
        }
        if self.optional.has(section) {
            SectionStatus::Configured
        } else {
            SectionStatus::Default
        }
    }

    pub(crate) fn optional_draft(&self, section: OptionalSection) -> OptionalSectionDraft {
        self.optional
            .get(section)
            .unwrap_or_else(|| default_optional_draft(self.language, section))
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

    pub(crate) fn validate_for_review(&self) -> Result<(), validation::ValidationErrors> {
        validation::validate_for_review(self)
    }

    pub(crate) fn build_active_input(&self) -> Result<InitInput, validation::ValidationErrors> {
        validation::build_active_input(self)
    }

    pub(super) fn tunnel_seed_error(&self) -> Option<&'static str> {
        self.tunnel_seed_error
    }

    pub(super) fn replace_optional(&mut self, draft: OptionalSectionDraft) {
        self.optional.set(draft);
    }
}

impl StandaloneDraft {
    fn from_seed(
        tunnel_id: Option<String>,
        tunnel_api_key: Option<String>,
    ) -> (Self, Option<&'static str>) {
        let mut draft = Self {
            tunnel_id: tunnel_id.unwrap_or_else(|| DEFAULT_TUNNEL_ID.to_string()),
            secret_source: TunnelSecretSource::File,
            secret_path: DEFAULT_SECRET_PATH.to_string(),
            secret_environment: String::new(),
            provision_secret_now: false,
            secret_value: None,
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
        (draft, error)
    }
}

impl OptionalDrafts {
    fn has(&self, section: OptionalSection) -> bool {
        match section {
            OptionalSection::Identity => self.identity.is_some(),
            OptionalSection::Workspace => self.workspace.is_some(),
            OptionalSection::Confirmation => self.confirmation.is_some(),
            OptionalSection::Limits => self.limits.is_some(),
            OptionalSection::Sandbox => self.sandbox.is_some(),
            OptionalSection::Room => self.room.is_some(),
            OptionalSection::TunnelClient => self.tunnel_client.is_some(),
            OptionalSection::HubReporting => self.hub_reporting.is_some(),
        }
    }

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
            OptionalSection::Room => self.room.clone().map(OptionalSectionDraft::Room),
            OptionalSection::TunnelClient => self
                .tunnel_client
                .clone()
                .map(OptionalSectionDraft::TunnelClient),
            OptionalSection::HubReporting => self
                .hub_reporting
                .clone()
                .map(OptionalSectionDraft::HubReporting),
        }
    }

    fn set(&mut self, draft: OptionalSectionDraft) {
        match draft {
            OptionalSectionDraft::Identity(value) => self.identity = Some(value),
            OptionalSectionDraft::Workspace(value) => self.workspace = Some(value),
            OptionalSectionDraft::Confirmation(value) => self.confirmation = Some(value),
            OptionalSectionDraft::Limits(value) => self.limits = Some(value),
            OptionalSectionDraft::Sandbox(value) => self.sandbox = Some(value),
            OptionalSectionDraft::Room(value) => self.room = Some(value),
            OptionalSectionDraft::TunnelClient(value) => self.tunnel_client = Some(value),
            OptionalSectionDraft::HubReporting(value) => self.hub_reporting = Some(value),
        }
    }
}

pub(super) fn default_optional_draft(
    language: UiLanguage,
    section: OptionalSection,
) -> OptionalSectionDraft {
    match section {
        OptionalSection::Identity => OptionalSectionDraft::Identity(IdentityDraft {
            display_name: "AgenticGPT agent".to_string(),
        }),
        OptionalSection::Workspace => {
            let workspace_root = PathBuf::from(DEFAULT_WORKSPACE_ROOT);
            let defaults = default_path_policy(&workspace_root);
            OptionalSectionDraft::Workspace(WorkspaceDraft {
                workspace_root: DEFAULT_WORKSPACE_ROOT.to_string(),
                write_roots: serialize_paths(&defaults.write_roots),
                read_only_roots: serialize_paths(&defaults.read_only_roots),
                deny_roots: serialize_paths(&defaults.deny_roots),
            })
        }
        OptionalSection::Confirmation => OptionalSectionDraft::Confirmation(ConfirmationDraft {
            provider: "default".to_string(),
            language: match language {
                UiLanguage::En => "en".to_string(),
                UiLanguage::ZhCn => "zh-CN".to_string(),
            },
        }),
        OptionalSection::Limits => OptionalSectionDraft::Limits(LimitsDraft {
            max_concurrent_tasks: "2".to_string(),
            max_active_jobs: "auto".to_string(),
            max_file_search_context_lines: "5".to_string(),
        }),
        OptionalSection::Sandbox => OptionalSectionDraft::Sandbox(SandboxDraft {
            enabled: false,
            bubblewrap_path: DEFAULT_BUBBLEWRAP_PATH.to_string(),
            required_runtime_paths: DEFAULT_RUNTIME_PATHS.to_string(),
        }),
        OptionalSection::Room => OptionalSectionDraft::Room(RoomDraft {
            timezone: "Asia/Shanghai".to_string(),
            diary_boundary_hour: "5".to_string(),
            notebook_root: String::new(),
        }),
        OptionalSection::TunnelClient => OptionalSectionDraft::TunnelClient(TunnelClientDraft {
            version: String::new(),
            cache_dir: DEFAULT_TUNNEL_CACHE_DIR.to_string(),
            auto_download: true,
            executable: String::new(),
            download_url: String::new(),
            sha256: String::new(),
        }),
        OptionalSection::HubReporting => OptionalSectionDraft::HubReporting(HubReportingDraft {
            enabled: false,
            detail: "metadata".to_string(),
        }),
    }
}

fn serialize_paths(paths: &[PathBuf]) -> String {
    serde_json::to_string(paths).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli_i18n::UiLanguage;
    use crate::config_templates::{RuntimeMode, SecretValue, TunnelSecretSource};
    use crate::WorkerProfile;

    use super::*;

    #[test]
    fn setup_defaults_to_standalone_normal_and_preserves_inactive_mode_seeds() {
        let seed = SetupSeed {
            mode: Some(RuntimeMode::Hub),
            tunnel_id: Some("tunnel_seed".into()),
            hub_url: Some("https://hub.example.com".into()),
            ..SetupSeed::default()
        };
        let mut session =
            SetupSession::new(seed, UiLanguage::En, PathBuf::from("/tmp/config.json"));

        assert_eq!(session.selected_mode(), RuntimeMode::Hub);
        assert_eq!(session.selected_profile(), WorkerProfile::Normal);
        assert_eq!(session.standalone().tunnel_id, "tunnel_seed");
        assert_eq!(session.hub().hub_url, "https://hub.example.com");

        session.set_mode(RuntimeMode::Standalone);
        assert_eq!(session.standalone().tunnel_id, "tunnel_seed");
        session.set_mode(RuntimeMode::Hub);
        assert_eq!(session.hub().hub_url, "https://hub.example.com");
    }

    #[test]
    fn tunnel_secret_reference_seeds_are_parsed_without_exposing_secret_text() {
        let file_session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_api_key: Some("file:/tmp/tunnel-secret".into()),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        );
        assert_eq!(
            file_session.standalone().secret_source,
            TunnelSecretSource::File
        );
        assert_eq!(file_session.standalone().secret_path, "/tmp/tunnel-secret");
        assert!(file_session.standalone().secret_environment.is_empty());

        let env_session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_api_key: Some("env:TUNNEL_SECRET".into()),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        );
        assert_eq!(
            env_session.standalone().secret_source,
            TunnelSecretSource::Environment
        );
        assert_eq!(env_session.standalone().secret_environment, "TUNNEL_SECRET");
        assert!(env_session.standalone().secret_path.is_empty());

        let hub_secret = SecretValue::new("hub-secret-marker");
        let hub_session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                agent_secret: Some(hub_secret),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        );
        assert!(format!("{:?}", hub_session.hub()).contains("REDACTED"));
        assert!(!format!("{:?}", hub_session.hub()).contains("hub-secret-marker"));
    }

    #[test]
    fn malformed_tunnel_secret_reference_is_reported_as_a_field_error() {
        let session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_api_key: Some("file:".into()),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        );

        let errors = session.validate_connection().unwrap_err();
        assert_eq!(errors[0].field, SetupField::TunnelSecretPath);
        assert_eq!(errors[0].code, "config_init_secret_path_invalid");
    }
}
