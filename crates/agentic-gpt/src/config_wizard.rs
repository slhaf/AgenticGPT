use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{IsTerminal, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{anyhow, Result};
use inquire::{
    Confirm, CustomType, InquireError, MultiSelect, Password, PasswordDisplayMode, Select, Text,
};

use crate::cli_i18n::UiLanguage;
use crate::config::{
    default_path_policy, write_config_with_backup, ConfirmationProviderConfig, HubReportingConfig,
    LimitsConfig, MaxActiveJobs, PathPolicyConfig, ReportingDetail, RoomConfig, SandboxConfig,
    TunnelClientConfig,
};
use crate::config_cli::ConfigInitArgs;
use crate::config_templates::{
    build_config, InitBuild, InitInput, InitSummary, OptionalSection, PendingAction, RuntimeMode,
    SecretValue, SecretWritePlan, TunnelSecretSource,
};
use crate::WorkerProfile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PromptRequest {
    SelectMode {
        default: RuntimeMode,
    },
    SelectProfile {
        default: WorkerProfile,
    },
    SelectSecretSource {
        default: TunnelSecretSource,
    },
    Text {
        id: PromptId,
        default: Option<String>,
    },
    Secret {
        id: PromptId,
    },
    Confirm {
        id: PromptId,
        default: bool,
    },
    ConfirmSummary {
        id: PromptId,
        summary: String,
        default: bool,
    },
    OptionalSections {
        available: Vec<OptionalSection>,
    },
}

// This type intentionally has no Debug implementation: answers may own secrets.
pub(crate) enum PromptAnswer {
    Mode(RuntimeMode),
    Profile(WorkerProfile),
    SecretSource(TunnelSecretSource),
    Text(String),
    Secret(SecretValue),
    Bool(bool),
    Sections(Vec<OptionalSection>),
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Scripted cancellation is exercised by wizard tests."
        )
    )]
    Cancel,
}

pub(crate) trait PromptBackend {
    fn ask(&mut self, request: PromptRequest) -> Result<PromptAnswer>;
}

pub(crate) struct InquirePromptBackend {
    language: UiLanguage,
}

impl InquirePromptBackend {
    pub(crate) fn new(language: UiLanguage) -> Self {
        Self { language }
    }
}

impl PromptBackend for InquirePromptBackend {
    fn ask(&mut self, request: PromptRequest) -> Result<PromptAnswer> {
        match request {
            PromptRequest::SelectMode { default } => {
                let options = mode_options(self.language);
                let cursor = options
                    .iter()
                    .position(|option| option.value == default)
                    .unwrap_or(0);
                Select::new(mode_prompt_label(self.language), options)
                    .with_starting_cursor(cursor)
                    .without_filtering()
                    .prompt()
                    .map(|choice| PromptAnswer::Mode(choice.value))
                    .map_err(map_inquire_error)
            }
            PromptRequest::SelectProfile { default } => {
                let options = profile_options(self.language);
                let cursor = options
                    .iter()
                    .position(|option| option.value == default)
                    .unwrap_or(0);
                Select::new(profile_prompt_label(self.language), options)
                    .with_starting_cursor(cursor)
                    .without_filtering()
                    .prompt()
                    .map(|choice| PromptAnswer::Profile(choice.value))
                    .map_err(map_inquire_error)
            }
            PromptRequest::SelectSecretSource { default } => {
                let options = secret_source_options(self.language);
                let cursor = options
                    .iter()
                    .position(|option| option.value == default)
                    .unwrap_or(0);
                Select::new(secret_source_prompt_label(self.language), options)
                    .with_starting_cursor(cursor)
                    .without_filtering()
                    .prompt()
                    .map(|choice| PromptAnswer::SecretSource(choice.value))
                    .map_err(map_inquire_error)
            }
            PromptRequest::OptionalSections { available } => {
                let options = available
                    .iter()
                    .copied()
                    .map(|section| WizardChoice {
                        value: section,
                        label: optional_section_label(section, self.language),
                    })
                    .collect();
                MultiSelect::new(optional_sections_prompt_label(self.language), options)
                    .without_filtering()
                    .prompt()
                    .map(|choices| {
                        PromptAnswer::Sections(
                            choices.into_iter().map(|choice| choice.value).collect(),
                        )
                    })
                    .map_err(map_inquire_error)
            }
            PromptRequest::Confirm { id, default } => Confirm::new(prompt_label(id, self.language))
                .with_default(default)
                .prompt()
                .map(PromptAnswer::Bool)
                .map_err(map_inquire_error),
            PromptRequest::ConfirmSummary {
                id,
                summary,
                default,
            } => {
                println!("{summary}");
                Confirm::new(prompt_label(id, self.language))
                    .with_default(default)
                    .prompt()
                    .map(PromptAnswer::Bool)
                    .map_err(map_inquire_error)
            }
            PromptRequest::Text { id, default } => {
                prompt_text(self.language, id, default).map(PromptAnswer::Text)
            }
            PromptRequest::Secret { id } => Password::new(prompt_label(id, self.language))
                .with_display_mode(PasswordDisplayMode::Hidden)
                .without_confirmation()
                .prompt()
                .map(SecretValue::new)
                .map(PromptAnswer::Secret)
                .map_err(map_inquire_error),
        }
    }
}

fn map_inquire_error(error: InquireError) -> anyhow::Error {
    match error {
        InquireError::OperationCanceled | InquireError::OperationInterrupted => {
            anyhow!("config_init_cancelled")
        }
        error => anyhow!("config_init_prompt_failed: {error}"),
    }
}

fn prompt_text(language: UiLanguage, id: PromptId, default: Option<String>) -> Result<String> {
    match id {
        PromptId::MaxConcurrentTasks => {
            prompt_usize(language, id, default).map(|value| value.to_string())
        }
        PromptId::MaxFileSearchContextLines => {
            prompt_usize(language, id, default).map(|value| value.to_string())
        }
        PromptId::DiaryBoundaryHour => {
            prompt_u32(language, id, default).map(|value| value.to_string())
        }
        PromptId::MaxActiveJobs => prompt_plain_text(language, id, default),
        _ => prompt_plain_text(language, id, default),
    }
}

fn prompt_plain_text(
    language: UiLanguage,
    id: PromptId,
    default: Option<String>,
) -> Result<String> {
    let label = prompt_label(id, language);
    let result = match default {
        Some(default) => Text::new(label).with_default(&default).prompt(),
        None => Text::new(label).prompt(),
    };
    result.map_err(map_inquire_error)
}

fn prompt_usize(language: UiLanguage, id: PromptId, default: Option<String>) -> Result<usize> {
    let mut prompt = CustomType::<usize>::new(prompt_label(id, language))
        .with_error_message(numeric_error(language));
    if let Some(default) = default {
        let default = default
            .parse::<usize>()
            .map_err(|_| anyhow!("config_init_prompt_failed: invalid numeric default"))?;
        prompt = prompt.with_default(default);
    }
    prompt.prompt().map_err(map_inquire_error)
}

fn prompt_u32(language: UiLanguage, id: PromptId, default: Option<String>) -> Result<u32> {
    let mut prompt = CustomType::<u32>::new(prompt_label(id, language))
        .with_error_message(numeric_error(language));
    if let Some(default) = default {
        let default = default
            .parse::<u32>()
            .map_err(|_| anyhow!("config_init_prompt_failed: invalid numeric default"))?;
        prompt = prompt.with_default(default);
    }
    prompt.prompt().map_err(map_inquire_error)
}

fn numeric_error(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => "Enter a non-negative integer.",
        UiLanguage::ZhCn => "请输入非负整数。",
    }
}

#[derive(Clone, Copy)]
struct WizardChoice<T> {
    value: T,
    label: &'static str,
}

impl<T> fmt::Display for WizardChoice<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label)
    }
}

fn mode_options(language: UiLanguage) -> Vec<WizardChoice<RuntimeMode>> {
    vec![
        WizardChoice {
            value: RuntimeMode::Standalone,
            label: mode_option_label(RuntimeMode::Standalone, language),
        },
        WizardChoice {
            value: RuntimeMode::Hub,
            label: mode_option_label(RuntimeMode::Hub, language),
        },
        WizardChoice {
            value: RuntimeMode::Local,
            label: mode_option_label(RuntimeMode::Local, language),
        },
    ]
}

fn profile_options(language: UiLanguage) -> Vec<WizardChoice<WorkerProfile>> {
    vec![
        WizardChoice {
            value: WorkerProfile::Normal,
            label: profile_option_label(WorkerProfile::Normal, language),
        },
        WizardChoice {
            value: WorkerProfile::Room,
            label: profile_option_label(WorkerProfile::Room, language),
        },
    ]
}

fn secret_source_options(language: UiLanguage) -> Vec<WizardChoice<TunnelSecretSource>> {
    vec![
        WizardChoice {
            value: TunnelSecretSource::File,
            label: secret_source_option_label(TunnelSecretSource::File, language),
        },
        WizardChoice {
            value: TunnelSecretSource::Environment,
            label: secret_source_option_label(TunnelSecretSource::Environment, language),
        },
    ]
}

fn mode_prompt_label(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => "Select the runtime mode",
        UiLanguage::ZhCn => "选择运行模式",
    }
}

fn profile_prompt_label(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => "Select the worker profile",
        UiLanguage::ZhCn => "选择工作配置档",
    }
}

fn secret_source_prompt_label(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => "Choose the tunnel secret source",
        UiLanguage::ZhCn => "选择隧道密钥来源",
    }
}

fn optional_sections_prompt_label(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::En => "Select optional settings to configure",
        UiLanguage::ZhCn => "选择要配置的可选设置",
    }
}

pub(crate) fn should_use_interactive_init(
    non_interactive: bool,
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
) -> bool {
    !non_interactive && stdin_is_terminal && stdout_is_terminal && stderr_is_terminal
}

pub(crate) fn process_should_use_interactive_init(non_interactive: bool) -> bool {
    should_use_interactive_init(
        non_interactive,
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
        std::io::stderr().is_terminal(),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptId {
    TunnelId,
    TunnelSecretPath,
    TunnelSecretEnvironment,
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
    ConfigureOptionalSections,
    WriteSecretNow,
    ConfirmWrite,
}

// This type intentionally has no Debug implementation: it owns the in-memory write plan.
pub(crate) struct WizardOutcome {
    pub(crate) build: InitBuild,
    pub(crate) secret_write: Option<SecretWritePlan>,
    pub(crate) summary: String,
}

const DEFAULT_TUNNEL_ID: &str = "tunnel_replace-me";
const DEFAULT_SECRET_PATH: &str = "~/.agentic_gpt/secrets/tunnel-api-key";
const DEFAULT_ENVIRONMENT_NAME: &str = "AGENTIC_GPT_TUNNEL_API_KEY";
const DEFAULT_HUB_URL: &str = "http://localhost:8787";
const DEFAULT_HUB_TRANSPORT: &str = "websocket";
const DEFAULT_AGENT_ID: &str = "laptop";
const DEFAULT_WORKSPACE_ROOT: &str = "~/.agentic_gpt/workspace";
const DEFAULT_TUNNEL_CACHE_DIR: &str = "~/.agentic_gpt/cache/tunnel-client";

pub(crate) fn available_optional_sections(
    mode: RuntimeMode,
    profile: WorkerProfile,
) -> Vec<OptionalSection> {
    let mut sections = vec![
        OptionalSection::Identity,
        OptionalSection::Workspace,
        OptionalSection::Confirmation,
        OptionalSection::Limits,
        OptionalSection::Sandbox,
    ];
    if profile == WorkerProfile::Room {
        sections.push(OptionalSection::Room);
    }
    if mode == RuntimeMode::Standalone {
        sections.push(OptionalSection::TunnelClient);
        sections.push(OptionalSection::HubReporting);
    }
    sections
}

pub(crate) fn run_wizard(
    backend: &mut impl PromptBackend,
    defaults: ConfigInitArgs,
    language: UiLanguage,
) -> Result<WizardOutcome> {
    let mode = match defaults.mode {
        Some(mode) => mode,
        None => ask_mode(backend, RuntimeMode::Standalone)?,
    };
    validate_mode_applicability(&defaults, mode)?;
    let profile = match defaults.profile {
        Some(profile) => profile,
        None => ask_profile(backend, WorkerProfile::Normal)?,
    };

    let mut input = InitInput::non_interactive_defaults(language);
    input.mode = mode;
    input.profile = profile;
    input.tunnel_id = defaults.tunnel_id;
    input.tunnel_api_key = defaults.tunnel_api_key;
    input.hub_url = defaults.hub_url;
    input.hub_transport = defaults.hub_transport;
    input.agent_id = defaults.agent_id;
    input.agent_secret = defaults.agent_secret.map(SecretValue::new);

    let mut secret_write = None;
    let mut deferred_file_secret = false;
    match mode {
        RuntimeMode::Standalone => {
            if missing_string(input.tunnel_id.as_deref()) {
                input.tunnel_id = Some(required_text(
                    ask_text(
                        backend,
                        PromptId::TunnelId,
                        Some(DEFAULT_TUNNEL_ID.to_string()),
                    )?,
                    PromptId::TunnelId,
                )?);
            }
            if missing_string(input.tunnel_api_key.as_deref()) {
                let source = ask_secret_source(backend, TunnelSecretSource::File)?;
                match source {
                    TunnelSecretSource::File => {
                        let raw_path = required_text(
                            ask_text(
                                backend,
                                PromptId::TunnelSecretPath,
                                Some(DEFAULT_SECRET_PATH.to_string()),
                            )?,
                            PromptId::TunnelSecretPath,
                        )?;
                        let path_text = raw_path
                            .strip_prefix("file:")
                            .unwrap_or(raw_path.as_str())
                            .trim();
                        if path_text.is_empty() {
                            return Err(anyhow!("config_init_secret_path_invalid"));
                        }
                        input.tunnel_api_key = Some(format!("file:{path_text}"));

                        if ask_confirm(backend, PromptId::WriteSecretNow, false)? {
                            let value = ask_secret(backend, PromptId::TunnelSecretValue)?;
                            if value.expose().trim().is_empty() {
                                return Err(anyhow!("config_init_secret_empty"));
                            }
                            secret_write = Some(SecretWritePlan {
                                path: PathBuf::from(path_text),
                                value,
                            });
                        } else {
                            deferred_file_secret = true;
                        }
                    }
                    TunnelSecretSource::Environment => {
                        let raw_name = required_text(
                            ask_text(
                                backend,
                                PromptId::TunnelSecretEnvironment,
                                Some(DEFAULT_ENVIRONMENT_NAME.to_string()),
                            )?,
                            PromptId::TunnelSecretEnvironment,
                        )?;
                        let name = raw_name
                            .strip_prefix("env:")
                            .unwrap_or(raw_name.as_str())
                            .trim();
                        if name.is_empty() {
                            return Err(anyhow!("config_init_environment_name_invalid"));
                        }
                        input.tunnel_api_key = Some(format!("env:{name}"));
                    }
                }
            }
        }
        RuntimeMode::Hub => {
            if missing_string(input.hub_url.as_deref()) {
                input.hub_url = Some(required_text(
                    ask_text(backend, PromptId::HubUrl, Some(DEFAULT_HUB_URL.to_string()))?,
                    PromptId::HubUrl,
                )?);
            }
            if missing_string(input.hub_transport.as_deref()) {
                input.hub_transport = Some(required_text(
                    ask_text(
                        backend,
                        PromptId::HubTransport,
                        Some(DEFAULT_HUB_TRANSPORT.to_string()),
                    )?,
                    PromptId::HubTransport,
                )?);
            }
            if missing_string(input.agent_id.as_deref()) {
                input.agent_id = Some(required_text(
                    ask_text(
                        backend,
                        PromptId::AgentId,
                        Some(DEFAULT_AGENT_ID.to_string()),
                    )?,
                    PromptId::AgentId,
                )?);
            }
            if input
                .agent_secret
                .as_ref()
                .map(|secret| secret.expose().trim().is_empty())
                .unwrap_or(true)
            {
                let secret = ask_secret(backend, PromptId::AgentSecret)?;
                if secret.expose().trim().is_empty() {
                    return Err(anyhow!("config_init_secret_empty"));
                }
                input.agent_secret = Some(secret);
            }
        }
        RuntimeMode::Local => {}
    }

    let available = available_optional_sections(mode, profile);
    if ask_confirm(backend, PromptId::ConfigureOptionalSections, false)? {
        let selected = ask_sections(backend, available.clone())?;
        let selected = legal_sections(selected, &available)?;
        for section in selected {
            collect_optional_section(backend, section, &mut input, language)?;
        }
    }

    let mut build = build_config(input)?;
    if secret_write.is_some() {
        build
            .pending
            .retain(|action| *action != PendingAction::ProvisionTunnelSecret);
    } else if deferred_file_secret
        && !build
            .pending
            .contains(&PendingAction::ProvisionTunnelSecret)
    {
        build.pending.push(PendingAction::ProvisionTunnelSecret);
    }
    let summary = render_summary(&build, secret_write.as_ref(), language);
    if !ask_confirm_summary(backend, PromptId::ConfirmWrite, summary.clone(), true)? {
        return Err(anyhow!("config_init_cancelled"));
    }

    Ok(WizardOutcome {
        build,
        secret_write,
        summary,
    })
}

fn validate_mode_applicability(args: &ConfigInitArgs, mode: RuntimeMode) -> Result<()> {
    if mode != RuntimeMode::Standalone
        && (args.tunnel_id.is_some() || args.tunnel_api_key.is_some())
    {
        return Err(anyhow!("config_init_option_not_applicable"));
    }
    if mode != RuntimeMode::Hub
        && (args.hub_url.is_some()
            || args.hub_transport.is_some()
            || args.agent_id.is_some()
            || args.agent_secret.is_some())
    {
        return Err(anyhow!("config_init_option_not_applicable"));
    }
    Ok(())
}

fn legal_sections(
    selected: Vec<OptionalSection>,
    available: &[OptionalSection],
) -> Result<Vec<OptionalSection>> {
    let mut result = Vec::with_capacity(selected.len());
    for section in selected {
        if !available.contains(&section) {
            return Err(anyhow!("config_init_optional_section_invalid"));
        }
        if !result.contains(&section) {
            result.push(section);
        }
    }
    Ok(result)
}

fn collect_optional_section(
    backend: &mut impl PromptBackend,
    section: OptionalSection,
    input: &mut InitInput,
    language: UiLanguage,
) -> Result<()> {
    match section {
        OptionalSection::Identity => {
            input.display_name = Some(required_text(
                ask_text(
                    backend,
                    PromptId::DisplayName,
                    Some("AgenticGPT agent".to_string()),
                )?,
                PromptId::DisplayName,
            )?);
        }
        OptionalSection::Workspace => {
            let workspace_root = PathBuf::from(required_text(
                ask_text(
                    backend,
                    PromptId::WorkspaceRoot,
                    Some(DEFAULT_WORKSPACE_ROOT.to_string()),
                )?,
                PromptId::WorkspaceRoot,
            )?);
            let defaults = default_path_policy(&workspace_root);
            let write_roots = parse_path_list(
                &required_text(
                    ask_text(
                        backend,
                        PromptId::WriteRoots,
                        Some(serialize_path_list(
                            &defaults.write_roots,
                            PromptId::WriteRoots,
                        )?),
                    )?,
                    PromptId::WriteRoots,
                )?,
                PromptId::WriteRoots,
            )?;
            let read_only_roots = parse_path_list(
                &required_text(
                    ask_text(
                        backend,
                        PromptId::ReadOnlyRoots,
                        Some(serialize_path_list(
                            &defaults.read_only_roots,
                            PromptId::ReadOnlyRoots,
                        )?),
                    )?,
                    PromptId::ReadOnlyRoots,
                )?,
                PromptId::ReadOnlyRoots,
            )?;
            let deny_roots = parse_path_list(
                &required_text(
                    ask_text(
                        backend,
                        PromptId::DenyRoots,
                        Some(serialize_path_list(
                            &defaults.deny_roots,
                            PromptId::DenyRoots,
                        )?),
                    )?,
                    PromptId::DenyRoots,
                )?,
                PromptId::DenyRoots,
            )?;
            input.workspace_root = Some(workspace_root);
            input.path_policy = Some(PathPolicyConfig {
                write_roots,
                read_only_roots,
                deny_roots,
            });
        }
        OptionalSection::Confirmation => {
            let provider = required_text(
                ask_text(
                    backend,
                    PromptId::ConfirmationProvider,
                    Some("default".to_string()),
                )?,
                PromptId::ConfirmationProvider,
            )?;
            input.confirmation_provider = Some(
                ConfirmationProviderConfig::from_legacy(&provider)
                    .map_err(|_| anyhow!("config_init_confirmation_provider_invalid"))?,
            );
            input.confirmation_language = Some(required_text(
                ask_text(
                    backend,
                    PromptId::ConfirmationLanguage,
                    Some(
                        match language {
                            UiLanguage::En => "en",
                            UiLanguage::ZhCn => "zh-CN",
                        }
                        .to_string(),
                    ),
                )?,
                PromptId::ConfirmationLanguage,
            )?);
        }
        OptionalSection::Limits => {
            let max_concurrent_tasks = parse_usize(
                &required_text(
                    ask_text(backend, PromptId::MaxConcurrentTasks, Some("2".to_string()))?,
                    PromptId::MaxConcurrentTasks,
                )?,
                PromptId::MaxConcurrentTasks,
            )?;
            let max_active_jobs = parse_max_active_jobs(&required_text(
                ask_text(backend, PromptId::MaxActiveJobs, Some("auto".to_string()))?,
                PromptId::MaxActiveJobs,
            )?)?;
            let max_file_search_context_lines = parse_usize(
                &required_text(
                    ask_text(
                        backend,
                        PromptId::MaxFileSearchContextLines,
                        Some("5".to_string()),
                    )?,
                    PromptId::MaxFileSearchContextLines,
                )?,
                PromptId::MaxFileSearchContextLines,
            )?;
            input.limits = Some(LimitsConfig {
                max_concurrent_tasks,
                max_active_jobs,
                max_file_search_context_lines,
            });
        }
        OptionalSection::Sandbox => {
            let enabled = ask_bool(backend, PromptId::SandboxEnabled, false)?;
            let bubblewrap_path = required_text(
                ask_text(backend, PromptId::BubblewrapPath, Some("bwrap".to_string()))?,
                PromptId::BubblewrapPath,
            )?;
            let runtime_paths = required_text(
                ask_text(
                    backend,
                    PromptId::RequiredRuntimePaths,
                    Some(r#"["/usr","/bin","/lib","/lib64","/etc/ssl"]"#.to_string()),
                )?,
                PromptId::RequiredRuntimePaths,
            )?;
            let required_runtime_paths = serde_json::from_str(&runtime_paths)
                .map_err(|_| anyhow!("config_init_runtime_paths_invalid"))?;
            input.sandbox = Some(SandboxConfig {
                enabled,
                bubblewrap_path,
                required_runtime_paths,
            });
        }
        OptionalSection::Room => {
            let timezone = required_text(
                ask_text(
                    backend,
                    PromptId::RoomTimezone,
                    Some("Asia/Shanghai".to_string()),
                )?,
                PromptId::RoomTimezone,
            )?;
            let diary_day_boundary_hour = parse_u32(
                &required_text(
                    ask_text(backend, PromptId::DiaryBoundaryHour, Some("5".to_string()))?,
                    PromptId::DiaryBoundaryHour,
                )?,
                PromptId::DiaryBoundaryHour,
            )?;
            let notebook_root = optional_path(ask_text(backend, PromptId::NotebookRoot, None)?);
            input.room = Some(RoomConfig {
                notebook_root,
                timezone,
                diary_day_boundary_hour,
                skills: Default::default(),
            });
        }
        OptionalSection::TunnelClient => {
            let version = optional_string(ask_text(backend, PromptId::TunnelClientVersion, None)?);
            let cache_dir = optional_path(ask_text(
                backend,
                PromptId::TunnelCacheDir,
                Some(DEFAULT_TUNNEL_CACHE_DIR.to_string()),
            )?)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TUNNEL_CACHE_DIR));
            let auto_download = ask_bool(backend, PromptId::TunnelAutoDownload, true)?;
            let executable = optional_path(ask_text(backend, PromptId::TunnelExecutable, None)?);
            let download_url =
                optional_string(ask_text(backend, PromptId::TunnelDownloadUrl, None)?);
            let sha256 = optional_string(ask_text(backend, PromptId::TunnelSha256, None)?);
            input.tunnel_client = Some(TunnelClientConfig {
                version,
                cache_dir,
                auto_download,
                executable,
                download_url,
                sha256,
            });
        }
        OptionalSection::HubReporting => {
            let enabled = ask_bool(backend, PromptId::HubReportingEnabled, false)?;
            let detail = required_text(
                ask_text(
                    backend,
                    PromptId::HubReportingDetail,
                    Some("metadata".to_string()),
                )?,
                PromptId::HubReportingDetail,
            )?;
            let detail = match detail.as_str() {
                "metadata" => ReportingDetail::Metadata,
                "full" => ReportingDetail::Full,
                _ => return Err(anyhow!("config_init_reporting_detail_invalid")),
            };
            input.hub_reporting = Some(HubReportingConfig { enabled, detail });
        }
    }
    Ok(())
}

fn ask_mode(backend: &mut impl PromptBackend, default: RuntimeMode) -> Result<RuntimeMode> {
    match ask(backend, PromptRequest::SelectMode { default })? {
        PromptAnswer::Mode(mode) => Ok(mode),
        _ => Err(anyhow!("config_init_prompt_answer_invalid: mode")),
    }
}

fn ask_profile(backend: &mut impl PromptBackend, default: WorkerProfile) -> Result<WorkerProfile> {
    match ask(backend, PromptRequest::SelectProfile { default })? {
        PromptAnswer::Profile(profile) => Ok(profile),
        _ => Err(anyhow!("config_init_prompt_answer_invalid: profile")),
    }
}

fn ask_secret_source(
    backend: &mut impl PromptBackend,
    default: TunnelSecretSource,
) -> Result<TunnelSecretSource> {
    match ask(backend, PromptRequest::SelectSecretSource { default })? {
        PromptAnswer::SecretSource(source) => Ok(source),
        _ => Err(anyhow!("config_init_prompt_answer_invalid: secret_source")),
    }
}

fn ask_text(
    backend: &mut impl PromptBackend,
    id: PromptId,
    default: Option<String>,
) -> Result<String> {
    match ask(backend, PromptRequest::Text { id, default })? {
        PromptAnswer::Text(value) => Ok(value),
        _ => Err(anyhow!(
            "config_init_prompt_answer_invalid: {}",
            prompt_key(id)
        )),
    }
}

fn ask_secret(backend: &mut impl PromptBackend, id: PromptId) -> Result<SecretValue> {
    match ask(backend, PromptRequest::Secret { id })? {
        PromptAnswer::Secret(value) => Ok(value),
        _ => Err(anyhow!(
            "config_init_prompt_answer_invalid: {}",
            prompt_key(id)
        )),
    }
}

fn ask_bool(backend: &mut impl PromptBackend, id: PromptId, default: bool) -> Result<bool> {
    match ask(backend, PromptRequest::Confirm { id, default })? {
        PromptAnswer::Bool(value) => Ok(value),
        _ => Err(anyhow!(
            "config_init_prompt_answer_invalid: {}",
            prompt_key(id)
        )),
    }
}

fn ask_confirm(backend: &mut impl PromptBackend, id: PromptId, default: bool) -> Result<bool> {
    ask_bool(backend, id, default)
}

fn ask_confirm_summary(
    backend: &mut impl PromptBackend,
    id: PromptId,
    summary: String,
    default: bool,
) -> Result<bool> {
    match ask(
        backend,
        PromptRequest::ConfirmSummary {
            id,
            summary,
            default,
        },
    )? {
        PromptAnswer::Bool(value) => Ok(value),
        _ => Err(anyhow!(
            "config_init_prompt_answer_invalid: {}",
            prompt_key(id)
        )),
    }
}

fn ask_sections(
    backend: &mut impl PromptBackend,
    available: Vec<OptionalSection>,
) -> Result<Vec<OptionalSection>> {
    match ask(backend, PromptRequest::OptionalSections { available })? {
        PromptAnswer::Sections(sections) => Ok(sections),
        _ => Err(anyhow!(
            "config_init_prompt_answer_invalid: optional_sections"
        )),
    }
}

fn ask(backend: &mut impl PromptBackend, request: PromptRequest) -> Result<PromptAnswer> {
    match backend.ask(request)? {
        PromptAnswer::Cancel => Err(anyhow!("config_init_cancelled")),
        answer => Ok(answer),
    }
}

fn required_text(value: String, id: PromptId) -> Result<String> {
    if value.trim().is_empty() {
        return Err(anyhow!(
            "config_init_required_value_missing: {}",
            prompt_key(id)
        ));
    }
    Ok(value.trim().to_string())
}

fn missing_string(value: Option<&str>) -> bool {
    value.map(|value| value.trim().is_empty()).unwrap_or(true)
}

fn optional_string(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn optional_path(value: String) -> Option<PathBuf> {
    optional_string(value).map(PathBuf::from)
}

fn serialize_path_list(paths: &[PathBuf], id: PromptId) -> Result<String> {
    serde_json::to_string(paths).map_err(|_| anyhow!(path_policy_error(id)))
}

fn parse_path_list(value: &str, id: PromptId) -> Result<Vec<PathBuf>> {
    serde_json::from_str(value.trim()).map_err(|_| anyhow!(path_policy_error(id)))
}

fn path_policy_error(id: PromptId) -> &'static str {
    match id {
        PromptId::WriteRoots => "config_init_path_policy_write_roots_invalid",
        PromptId::ReadOnlyRoots => "config_init_path_policy_read_only_roots_invalid",
        PromptId::DenyRoots => "config_init_path_policy_deny_roots_invalid",
        _ => "config_init_path_policy_invalid",
    }
}

fn parse_usize(value: &str, id: PromptId) -> Result<usize> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|_| anyhow!("config_init_number_invalid: {}", prompt_key(id)))
}

fn parse_u32(value: &str, id: PromptId) -> Result<u32> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| anyhow!("config_init_number_invalid: {}", prompt_key(id)))
}

fn parse_max_active_jobs(value: &str) -> Result<MaxActiveJobs> {
    match value.trim() {
        "auto" => Ok(MaxActiveJobs::Auto),
        value => value
            .parse::<usize>()
            .map(MaxActiveJobs::Explicit)
            .map_err(|_| anyhow!("config_init_number_invalid: max_active_jobs")),
    }
}

pub(crate) fn render_summary(
    build: &InitBuild,
    secret_write: Option<&SecretWritePlan>,
    language: UiLanguage,
) -> String {
    let (title, mode_label, profile_label, pending_label, hidden_label, secret_label) =
        match language {
            UiLanguage::En => (
                "Configuration ready",
                mode_label_en(build.mode),
                profile_label_en(build.profile),
                "Pending action",
                "[REDACTED]",
                "Secret",
            ),
            UiLanguage::ZhCn => (
                "配置已准备",
                mode_label_zh(build.mode),
                profile_label_zh(build.profile),
                "待处理操作",
                "[REDACTED]",
                "密钥",
            ),
        };

    let mut lines = vec![
        title.to_string(),
        match language {
            UiLanguage::En => format!("Mode: {mode_label}"),
            UiLanguage::ZhCn => format!("模式：{mode_label}"),
        },
        match language {
            UiLanguage::En => format!("Profile: {profile_label}"),
            UiLanguage::ZhCn => format!("配置档：{profile_label}"),
        },
    ];

    if let Some(tunnel) = build.config.tunnel.as_ref() {
        let source = secret_reference_source(&tunnel.api_key, language);
        lines.push(match language {
            UiLanguage::En => format!("Tunnel ID: {}", tunnel.tunnel_id),
            UiLanguage::ZhCn => format!("隧道 ID：{}", tunnel.tunnel_id),
        });
        lines.push(match language {
            UiLanguage::En => format!("Tunnel secret source: {source}"),
            UiLanguage::ZhCn => format!("隧道密钥来源：{source}"),
        });
    }
    if build.mode == RuntimeMode::Hub {
        lines.push(match language {
            UiLanguage::En => format!("Agent secret: {hidden_label}"),
            UiLanguage::ZhCn => format!("代理密钥：{hidden_label}"),
        });
    }
    if let Some(plan) = secret_write {
        lines.push(match language {
            UiLanguage::En => format!(
                "{secret_label} file: {} (value hidden)",
                plan.path.display()
            ),
            UiLanguage::ZhCn => format!("{secret_label} 文件：{}（值已隐藏）", plan.path.display()),
        });
    }
    for action in &build.pending {
        let action_text = pending_action_label(*action, language);
        lines.push(match language {
            UiLanguage::En => format!("{pending_label}: {action_text}"),
            UiLanguage::ZhCn => format!("{pending_label}：{action_text}"),
        });
    }
    lines.join("\n")
}

fn secret_reference_source(reference: &str, language: UiLanguage) -> &'static str {
    if reference.starts_with("env:") {
        return match language {
            UiLanguage::En => "environment variable",
            UiLanguage::ZhCn => "环境变量",
        };
    }
    if reference.starts_with("file:") {
        return match language {
            UiLanguage::En => "protected file",
            UiLanguage::ZhCn => "受保护文件",
        };
    }
    match language {
        UiLanguage::En => "configured reference",
        UiLanguage::ZhCn => "已配置引用",
    }
}

fn pending_action_label(action: PendingAction, language: UiLanguage) -> &'static str {
    match (action, language) {
        (PendingAction::ReplaceTunnelId, UiLanguage::En) => "replace tunnel ID",
        (PendingAction::ProvisionTunnelSecret, UiLanguage::En) => "provision tunnel secret",
        (PendingAction::ConfigureHubUrl, UiLanguage::En) => "configure Hub URL",
        (PendingAction::ReplaceAgentSecret, UiLanguage::En) => "replace agent secret",
        (PendingAction::ReplaceTunnelId, UiLanguage::ZhCn) => "替换隧道 ID",
        (PendingAction::ProvisionTunnelSecret, UiLanguage::ZhCn) => "配置隧道密钥",
        (PendingAction::ConfigureHubUrl, UiLanguage::ZhCn) => "配置 Hub URL",
        (PendingAction::ReplaceAgentSecret, UiLanguage::ZhCn) => "替换代理密钥",
    }
}

fn mode_label_en(mode: RuntimeMode) -> &'static str {
    match mode {
        RuntimeMode::Standalone => "Standalone",
        RuntimeMode::Hub => "Hub",
        RuntimeMode::Local => "Local",
    }
}

fn mode_label_zh(mode: RuntimeMode) -> &'static str {
    match mode {
        RuntimeMode::Standalone => "独立模式",
        RuntimeMode::Hub => "Hub 模式",
        RuntimeMode::Local => "本地模式",
    }
}

fn profile_label_en(profile: WorkerProfile) -> &'static str {
    match profile {
        WorkerProfile::Normal => "Normal",
        WorkerProfile::Room => "Room",
    }
}

fn profile_label_zh(profile: WorkerProfile) -> &'static str {
    match profile {
        WorkerProfile::Normal => "普通",
        WorkerProfile::Room => "Room",
    }
}

fn prompt_key(id: PromptId) -> &'static str {
    match id {
        PromptId::TunnelId => "tunnel_id",
        PromptId::TunnelSecretPath => "tunnel_secret_path",
        PromptId::TunnelSecretEnvironment => "tunnel_secret_environment",
        PromptId::TunnelSecretValue => "tunnel_secret_value",
        PromptId::HubUrl => "hub_url",
        PromptId::HubTransport => "hub_transport",
        PromptId::AgentId => "agent_id",
        PromptId::AgentSecret => "agent_secret",
        PromptId::DisplayName => "display_name",
        PromptId::WorkspaceRoot => "workspace_root",
        PromptId::WriteRoots => "write_roots",
        PromptId::ReadOnlyRoots => "read_only_roots",
        PromptId::DenyRoots => "deny_roots",
        PromptId::ConfirmationProvider => "confirmation_provider",
        PromptId::ConfirmationLanguage => "confirmation_language",
        PromptId::MaxConcurrentTasks => "max_concurrent_tasks",
        PromptId::MaxActiveJobs => "max_active_jobs",
        PromptId::MaxFileSearchContextLines => "max_file_search_context_lines",
        PromptId::SandboxEnabled => "sandbox_enabled",
        PromptId::BubblewrapPath => "bubblewrap_path",
        PromptId::RequiredRuntimePaths => "required_runtime_paths",
        PromptId::RoomTimezone => "room_timezone",
        PromptId::DiaryBoundaryHour => "diary_boundary_hour",
        PromptId::NotebookRoot => "notebook_root",
        PromptId::TunnelClientVersion => "tunnel_client_version",
        PromptId::TunnelCacheDir => "tunnel_cache_dir",
        PromptId::TunnelAutoDownload => "tunnel_auto_download",
        PromptId::TunnelExecutable => "tunnel_executable",
        PromptId::TunnelDownloadUrl => "tunnel_download_url",
        PromptId::TunnelSha256 => "tunnel_sha256",
        PromptId::HubReportingEnabled => "hub_reporting_enabled",
        PromptId::HubReportingDetail => "hub_reporting_detail",
        PromptId::ConfigureOptionalSections => "configure_optional_sections",
        PromptId::WriteSecretNow => "write_secret_now",
        PromptId::ConfirmWrite => "confirm_write",
    }
}

fn prompt_label(id: PromptId, language: UiLanguage) -> &'static str {
    match (id, language) {
        (PromptId::TunnelId, UiLanguage::En) => "Tunnel ID",
        (PromptId::TunnelId, UiLanguage::ZhCn) => "隧道 ID",
        (PromptId::TunnelSecretPath, UiLanguage::En) => "Protected tunnel secret file path",
        (PromptId::TunnelSecretPath, UiLanguage::ZhCn) => "受保护的隧道密钥文件路径",
        (PromptId::TunnelSecretEnvironment, UiLanguage::En) => {
            "Tunnel secret environment variable name"
        }
        (PromptId::TunnelSecretEnvironment, UiLanguage::ZhCn) => "隧道密钥环境变量名",
        (PromptId::TunnelSecretValue, UiLanguage::En) => "Tunnel API secret",
        (PromptId::TunnelSecretValue, UiLanguage::ZhCn) => "隧道 API 密钥",
        (PromptId::HubUrl, UiLanguage::En) => "Hub URL",
        (PromptId::HubUrl, UiLanguage::ZhCn) => "Hub 地址（URL）",
        (PromptId::HubTransport, UiLanguage::En) => "Hub transport (websocket or sse)",
        (PromptId::HubTransport, UiLanguage::ZhCn) => "Hub 传输方式（websocket 或 sse）",
        (PromptId::AgentId, UiLanguage::En) => "Agent ID",
        (PromptId::AgentId, UiLanguage::ZhCn) => "代理 ID",
        (PromptId::AgentSecret, UiLanguage::En) => "Agent secret",
        (PromptId::AgentSecret, UiLanguage::ZhCn) => "代理密钥",
        (PromptId::DisplayName, UiLanguage::En) => "Display name",
        (PromptId::DisplayName, UiLanguage::ZhCn) => "显示名称",
        (PromptId::WorkspaceRoot, UiLanguage::En) => "Workspace root",
        (PromptId::WorkspaceRoot, UiLanguage::ZhCn) => "工作区根目录",
        (PromptId::WriteRoots, UiLanguage::En) => "Writable roots (JSON array)",
        (PromptId::WriteRoots, UiLanguage::ZhCn) => "可写根目录（JSON 数组）",
        (PromptId::ReadOnlyRoots, UiLanguage::En) => "Read-only roots (JSON array)",
        (PromptId::ReadOnlyRoots, UiLanguage::ZhCn) => "只读根目录（JSON 数组）",
        (PromptId::DenyRoots, UiLanguage::En) => "Denied roots (JSON array)",
        (PromptId::DenyRoots, UiLanguage::ZhCn) => "拒绝访问根目录（JSON 数组）",
        (PromptId::ConfirmationProvider, UiLanguage::En) => "Confirmation provider",
        (PromptId::ConfirmationProvider, UiLanguage::ZhCn) => "确认提供方",
        (PromptId::ConfirmationLanguage, UiLanguage::En) => "Confirmation language",
        (PromptId::ConfirmationLanguage, UiLanguage::ZhCn) => "确认语言",
        (PromptId::MaxConcurrentTasks, UiLanguage::En) => "Maximum concurrent tasks",
        (PromptId::MaxConcurrentTasks, UiLanguage::ZhCn) => "最大并发任务数",
        (PromptId::MaxActiveJobs, UiLanguage::En) => "Maximum active jobs (auto or integer)",
        (PromptId::MaxActiveJobs, UiLanguage::ZhCn) => "最大活动作业数（auto 或整数）",
        (PromptId::MaxFileSearchContextLines, UiLanguage::En) => {
            "Maximum file-search context lines"
        }
        (PromptId::MaxFileSearchContextLines, UiLanguage::ZhCn) => "最大文件搜索上下文行数",
        (PromptId::SandboxEnabled, UiLanguage::En) => "Enable the sandbox",
        (PromptId::SandboxEnabled, UiLanguage::ZhCn) => "启用沙箱",
        (PromptId::BubblewrapPath, UiLanguage::En) => "Bubblewrap executable path",
        (PromptId::BubblewrapPath, UiLanguage::ZhCn) => "Bubblewrap 可执行文件路径",
        (PromptId::RequiredRuntimePaths, UiLanguage::En) => {
            "Required sandbox runtime paths (JSON array)"
        }
        (PromptId::RequiredRuntimePaths, UiLanguage::ZhCn) => "沙箱所需运行时路径（JSON 数组）",
        (PromptId::RoomTimezone, UiLanguage::En) => "Room timezone",
        (PromptId::RoomTimezone, UiLanguage::ZhCn) => "Room 时区",
        (PromptId::DiaryBoundaryHour, UiLanguage::En) => "Diary day boundary hour",
        (PromptId::DiaryBoundaryHour, UiLanguage::ZhCn) => "日记日期分界小时",
        (PromptId::NotebookRoot, UiLanguage::En) => "Notebook root (empty to disable)",
        (PromptId::NotebookRoot, UiLanguage::ZhCn) => "笔记本根目录（留空以禁用）",
        (PromptId::TunnelClientVersion, UiLanguage::En) => "Tunnel client version",
        (PromptId::TunnelClientVersion, UiLanguage::ZhCn) => "隧道客户端版本",
        (PromptId::TunnelCacheDir, UiLanguage::En) => "Tunnel client cache directory",
        (PromptId::TunnelCacheDir, UiLanguage::ZhCn) => "隧道客户端缓存目录",
        (PromptId::TunnelAutoDownload, UiLanguage::En) => {
            "Download the tunnel client automatically"
        }
        (PromptId::TunnelAutoDownload, UiLanguage::ZhCn) => "自动下载隧道客户端",
        (PromptId::TunnelExecutable, UiLanguage::En) => "Tunnel client executable path",
        (PromptId::TunnelExecutable, UiLanguage::ZhCn) => "隧道客户端可执行文件路径",
        (PromptId::TunnelDownloadUrl, UiLanguage::En) => "Tunnel client download URL",
        (PromptId::TunnelDownloadUrl, UiLanguage::ZhCn) => "隧道客户端下载 URL",
        (PromptId::TunnelSha256, UiLanguage::En) => "Tunnel client SHA-256",
        (PromptId::TunnelSha256, UiLanguage::ZhCn) => "隧道客户端 SHA-256",
        (PromptId::HubReportingEnabled, UiLanguage::En) => "Enable Hub reporting",
        (PromptId::HubReportingEnabled, UiLanguage::ZhCn) => "启用 Hub 报告",
        (PromptId::HubReportingDetail, UiLanguage::En) => "Hub reporting detail (metadata or full)",
        (PromptId::HubReportingDetail, UiLanguage::ZhCn) => "Hub 报告详细程度（metadata 或 full）",
        (PromptId::ConfigureOptionalSections, UiLanguage::En) => "Configure optional settings?",
        (PromptId::ConfigureOptionalSections, UiLanguage::ZhCn) => "是否配置可选设置？",
        (PromptId::WriteSecretNow, UiLanguage::En) => "Write the tunnel secret now?",
        (PromptId::WriteSecretNow, UiLanguage::ZhCn) => "现在写入隧道密钥？",
        (PromptId::ConfirmWrite, UiLanguage::En) => "Write this configuration?",
        (PromptId::ConfirmWrite, UiLanguage::ZhCn) => "写入此配置？",
    }
}

fn mode_option_label(mode: RuntimeMode, language: UiLanguage) -> &'static str {
    match (mode, language) {
        (RuntimeMode::Standalone, UiLanguage::En) => "Standalone (standalone)",
        (RuntimeMode::Standalone, UiLanguage::ZhCn) => "独立模式（standalone）",
        (RuntimeMode::Hub, UiLanguage::En) => "Hub (hub)",
        (RuntimeMode::Hub, UiLanguage::ZhCn) => "Hub 模式（hub）",
        (RuntimeMode::Local, UiLanguage::En) => "Local (local)",
        (RuntimeMode::Local, UiLanguage::ZhCn) => "本地模式（local）",
    }
}

fn profile_option_label(profile: WorkerProfile, language: UiLanguage) -> &'static str {
    match (profile, language) {
        (WorkerProfile::Normal, UiLanguage::En) => "Normal (normal)",
        (WorkerProfile::Normal, UiLanguage::ZhCn) => "普通（normal）",
        (WorkerProfile::Room, UiLanguage::En) => "Room (room)",
        (WorkerProfile::Room, UiLanguage::ZhCn) => "Room（room）",
    }
}

fn secret_source_option_label(source: TunnelSecretSource, language: UiLanguage) -> &'static str {
    match (source, language) {
        (TunnelSecretSource::File, UiLanguage::En) => "Protected file (file)",
        (TunnelSecretSource::File, UiLanguage::ZhCn) => "受保护文件（file）",
        (TunnelSecretSource::Environment, UiLanguage::En) => "Environment variable (env)",
        (TunnelSecretSource::Environment, UiLanguage::ZhCn) => "环境变量（env）",
    }
}

fn optional_section_label(section: OptionalSection, language: UiLanguage) -> &'static str {
    match (section, language) {
        (OptionalSection::Identity, UiLanguage::En) => "Identity and display name",
        (OptionalSection::Identity, UiLanguage::ZhCn) => "身份与显示名称",
        (OptionalSection::Workspace, UiLanguage::En) => "Workspace and path policy",
        (OptionalSection::Workspace, UiLanguage::ZhCn) => "工作区与路径策略",
        (OptionalSection::Confirmation, UiLanguage::En) => "Confirmation and language",
        (OptionalSection::Confirmation, UiLanguage::ZhCn) => "确认方式与语言",
        (OptionalSection::Limits, UiLanguage::En) => "Runtime limits",
        (OptionalSection::Limits, UiLanguage::ZhCn) => "运行时限制",
        (OptionalSection::Sandbox, UiLanguage::En) => "Sandbox",
        (OptionalSection::Sandbox, UiLanguage::ZhCn) => "沙箱",
        (OptionalSection::Room, UiLanguage::En) => "Room settings",
        (OptionalSection::Room, UiLanguage::ZhCn) => "Room 设置",
        (OptionalSection::TunnelClient, UiLanguage::En) => "Tunnel client overrides",
        (OptionalSection::TunnelClient, UiLanguage::ZhCn) => "隧道客户端覆盖设置",
        (OptionalSection::HubReporting, UiLanguage::En) => "Hub reporting",
        (OptionalSection::HubReporting, UiLanguage::ZhCn) => "Hub 报告",
    }
}

enum PriorSecretState {
    Absent,
    Existing { bytes: Vec<u8>, mode: u32 },
}

struct TemporarySecretFile {
    path: Option<PathBuf>,
}

impl Drop for TemporarySecretFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

static SECRET_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const SECRET_TEMP_ATTEMPTS: usize = 128;

pub(crate) fn commit_wizard_outcome(
    config_path: &Path,
    outcome: WizardOutcome,
) -> Result<InitSummary> {
    let WizardOutcome {
        build,
        secret_write,
        summary,
    } = outcome;
    let _summary = summary;
    let summary = InitSummary {
        mode: build.mode,
        profile: build.profile,
        config_path: config_path.to_path_buf(),
        pending: build.pending.clone(),
    };

    let Some(plan) = secret_write else {
        write_config_with_backup(config_path, &build.config)?;
        return Ok(summary);
    };

    let (target, parent, prior) = validate_and_capture_secret_target(&plan.path)?;
    if target == config_path {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }
    fs::create_dir_all(&parent).map_err(|_| anyhow!("config_init_secret_parent_invalid"))?;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
        .map_err(|_| anyhow!("config_init_secret_parent_invalid"))?;

    atomically_write_secret(&target, plan.value.expose().as_bytes(), 0o600)
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;

    if write_config_with_backup(config_path, &build.config).is_err() {
        let rollback_result = match prior {
            PriorSecretState::Absent => match fs::remove_file(&target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(anyhow!("config_init_secret_rollback_failed")),
            },
            PriorSecretState::Existing { bytes, mode } => {
                atomically_write_secret(&target, &bytes, mode)
            }
        };
        return match rollback_result {
            Ok(()) => Err(anyhow!("config_init_config_write_failed")),
            Err(_) => Err(anyhow!(
                "config_init_config_write_failed: config_init_secret_rollback_failed"
            )),
        };
    }

    Ok(summary)
}

fn validate_and_capture_secret_target(path: &Path) -> Result<(PathBuf, PathBuf, PriorSecretState)> {
    if path.as_os_str().is_empty() {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }
    let target = crate::exec::expand_pathbuf(path)
        .map_err(|_| anyhow!("config_init_secret_path_invalid"))?;
    if target.as_os_str().is_empty()
        || target
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }
    let file_name = target
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow!("config_init_secret_path_invalid"))?;
    if file_name == "." || file_name == ".." {
        return Err(anyhow!("config_init_secret_path_invalid"));
    }

    let parent = target
        .parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                parent.to_path_buf()
            }
        })
        .ok_or_else(|| anyhow!("config_init_secret_path_invalid"))?;
    if let Ok(metadata) = fs::symlink_metadata(&parent) {
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(anyhow!("config_init_secret_path_invalid"));
        }
    }

    let prior = match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(anyhow!("config_init_secret_path_invalid"));
            }
            let bytes =
                fs::read(&target).map_err(|_| anyhow!("config_init_secret_path_invalid"))?;
            PriorSecretState::Existing {
                bytes,
                mode: metadata.permissions().mode() & 0o7777,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => PriorSecretState::Absent,
        Err(_) => return Err(anyhow!("config_init_secret_path_invalid")),
    };

    Ok((target, parent, prior))
}

fn atomically_write_secret(target: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow!("config_init_secret_path_invalid"))?
        .to_string_lossy();

    let mut temporary = None;
    for _ in 0..SECRET_TEMP_ATTEMPTS {
        let counter = SECRET_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{file_name}.agentic-gpt-tmp-{}-{counter}",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(anyhow!("config_init_secret_write_failed")),
        }
    }

    let (temporary_path, mut file) =
        temporary.ok_or_else(|| anyhow!("config_init_secret_temp_unavailable"))?;
    let mut guard = TemporarySecretFile {
        path: Some(temporary_path.clone()),
    };
    file.write_all(bytes)
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    file.sync_all()
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    file.set_permissions(fs::Permissions::from_mode(mode))
        .map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    drop(file);
    fs::rename(&temporary_path, target).map_err(|_| anyhow!("config_init_secret_write_failed"))?;
    guard.path = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_cli::ConfigInitArgs;
    use crate::config_templates::{OptionalSection, RuntimeMode, SecretValue, TunnelSecretSource};
    use anyhow::{anyhow, Result};
    use std::collections::VecDeque;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("agentic-gpt-task6c-{label}-{}", std::process::id()))
    }

    fn fresh_test_root(label: &str) -> PathBuf {
        let root = test_root(label);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn test_outcome_with_secret(secret_path: &std::path::Path, value: &str) -> WizardOutcome {
        let mut input = InitInput::non_interactive_defaults(UiLanguage::En);
        input.tunnel_id = Some("tunnel_test".into());
        input.tunnel_api_key = Some(format!("file:{}", secret_path.display()));
        let build = build_config(input).unwrap();
        WizardOutcome {
            build,
            secret_write: Some(SecretWritePlan {
                path: secret_path.to_path_buf(),
                value: SecretValue::new(value),
            }),
            summary: "Configuration ready; secret value is hidden.".into(),
        }
    }

    fn test_outcome_without_secret() -> WizardOutcome {
        let build = build_config(InitInput::non_interactive_defaults(UiLanguage::En)).unwrap();
        WizardOutcome {
            build,
            secret_write: None,
            summary: "Configuration ready; secret value is hidden.".into(),
        }
    }

    struct ScriptedPromptBackend {
        answers: VecDeque<PromptAnswer>,
        requests: Vec<PromptRequest>,
    }

    impl ScriptedPromptBackend {
        fn new<const N: usize>(answers: [PromptAnswer; N]) -> Self {
            Self {
                answers: answers.into_iter().collect(),
                requests: Vec::new(),
            }
        }

        fn requests(&self) -> &[PromptRequest] {
            &self.requests
        }
    }

    impl PromptBackend for ScriptedPromptBackend {
        fn ask(&mut self, request: PromptRequest) -> Result<PromptAnswer> {
            self.requests.push(request);
            self.answers
                .pop_front()
                .ok_or_else(|| anyhow!("scripted prompt exhausted"))
        }
    }

    fn request_ids(requests: &[PromptRequest]) -> Vec<PromptId> {
        requests
            .iter()
            .filter_map(|request| match request {
                PromptRequest::Text { id, .. }
                | PromptRequest::Secret { id }
                | PromptRequest::Confirm { id, .. }
                | PromptRequest::ConfirmSummary { id, .. } => Some(*id),
                PromptRequest::SelectMode { .. }
                | PromptRequest::SelectProfile { .. }
                | PromptRequest::SelectSecretSource { .. }
                | PromptRequest::OptionalSections { .. } => None,
            })
            .collect()
    }

    #[test]
    fn default_wizard_builds_standalone_normal_and_defers_optional_sections() {
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::Mode(RuntimeMode::Standalone),
            PromptAnswer::Profile(WorkerProfile::Normal),
            PromptAnswer::Text("tunnel_virtual".into()),
            PromptAnswer::SecretSource(TunnelSecretSource::File),
            PromptAnswer::Text("~/.agentic_gpt/secrets/tunnel-api-key".into()),
            PromptAnswer::Bool(false),
            PromptAnswer::Bool(false),
            PromptAnswer::Bool(true),
        ]);
        let outcome = run_wizard(&mut backend, ConfigInitArgs::default(), UiLanguage::En).unwrap();
        assert_eq!(
            outcome.build.config.tunnel.as_ref().unwrap().tunnel_id,
            "tunnel_virtual"
        );
        assert!(outcome.secret_write.is_none());
    }

    #[test]
    fn cancelled_wizard_returns_cancelled_without_write_plan() {
        let mut backend = ScriptedPromptBackend::new([PromptAnswer::Cancel]);
        let error = match run_wizard(&mut backend, ConfigInitArgs::default(), UiLanguage::En) {
            Ok(_) => panic!("cancelled wizard unexpectedly returned an outcome"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_cancelled");
    }

    #[test]
    fn room_optional_sections_include_room_but_local_excludes_tunnel_sections() {
        let room = available_optional_sections(RuntimeMode::Standalone, WorkerProfile::Room);
        assert!(room.contains(&OptionalSection::Room));
        assert!(room.contains(&OptionalSection::TunnelClient));
        assert!(room.contains(&OptionalSection::HubReporting));

        let local = available_optional_sections(RuntimeMode::Local, WorkerProfile::Normal);
        assert!(!local.contains(&OptionalSection::Room));
        assert!(!local.contains(&OptionalSection::TunnelClient));
        assert!(!local.contains(&OptionalSection::HubReporting));
    }

    #[test]
    fn wizard_summary_never_contains_secret() {
        let marker = "wizard-secret-marker-79b2a4";
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::Mode(RuntimeMode::Standalone),
            PromptAnswer::Profile(WorkerProfile::Normal),
            PromptAnswer::Text("tunnel_virtual".into()),
            PromptAnswer::SecretSource(TunnelSecretSource::File),
            PromptAnswer::Text("~/.agentic_gpt/secrets/tunnel-api-key".into()),
            PromptAnswer::Bool(true),
            PromptAnswer::Secret(SecretValue::new(marker)),
            PromptAnswer::Bool(false),
            PromptAnswer::Bool(true),
        ]);
        let outcome = run_wizard(&mut backend, ConfigInitArgs::default(), UiLanguage::En).unwrap();
        assert!(!outcome.summary.contains(marker));
        assert!(!format!("{:?}", outcome.secret_write).contains(marker));
        assert!(!outcome
            .build
            .pending
            .contains(&PendingAction::ProvisionTunnelSecret));
    }

    #[test]
    fn empty_prompted_hub_secret_is_rejected_before_later_prompts() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Hub),
            profile: Some(WorkerProfile::Normal),
            hub_url: Some("https://hub.example.com".to_string()),
            hub_transport: Some("websocket".to_string()),
            agent_id: Some("desk".to_string()),
            ..ConfigInitArgs::default()
        };
        let mut backend =
            ScriptedPromptBackend::new([PromptAnswer::Secret(SecretValue::new("  "))]);

        let error = match run_wizard(&mut backend, args, UiLanguage::En) {
            Ok(_) => panic!("empty Hub secret unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_secret_empty");
        assert_eq!(backend.requests().len(), 1);
        assert!(matches!(
            backend.requests()[0],
            PromptRequest::Secret {
                id: PromptId::AgentSecret
            }
        ));
    }

    #[test]
    fn summary_confirmation_records_exact_redacted_summary_after_value_prompts() {
        let marker = "summary-confirmation-secret-marker";
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Hub),
            profile: Some(WorkerProfile::Normal),
            hub_url: Some("https://hub.example.com".to_string()),
            hub_transport: Some("websocket".to_string()),
            agent_id: Some("desk".to_string()),
            agent_secret: Some(marker.to_string()),
            ..ConfigInitArgs::default()
        };
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::Bool(true),
            PromptAnswer::Sections(vec![OptionalSection::Identity]),
            PromptAnswer::Text("Desk Agent".to_string()),
            PromptAnswer::Bool(true),
        ]);

        let outcome = run_wizard(&mut backend, args, UiLanguage::ZhCn).unwrap();
        let final_request = backend.requests().last().expect("final request missing");
        match final_request {
            PromptRequest::ConfirmSummary {
                id,
                summary,
                default,
            } => {
                assert_eq!(*id, PromptId::ConfirmWrite);
                assert_eq!(summary, &outcome.summary);
                assert!(*default);
                assert!(summary.contains("配置"));
                assert!(summary.contains("[REDACTED]"));
                assert!(!summary.contains(marker));
            }
            _ => panic!("final request did not carry the redacted summary"),
        }
        assert!(matches!(
            backend.requests().first(),
            Some(PromptRequest::Confirm {
                id: PromptId::ConfigureOptionalSections,
                ..
            })
        ));
        assert!(matches!(
            backend.requests().get(1),
            Some(PromptRequest::OptionalSections { .. })
        ));
        assert!(matches!(
            backend.requests().get(2),
            Some(PromptRequest::Text {
                id: PromptId::DisplayName,
                ..
            })
        ));
        assert_eq!(backend.requests().len(), 4);
    }

    #[test]
    fn deferred_custom_file_secret_has_one_pending_provision_action_and_reminder() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Standalone),
            profile: Some(WorkerProfile::Normal),
            tunnel_id: Some("tunnel_custom_file".to_string()),
            ..ConfigInitArgs::default()
        };
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::SecretSource(TunnelSecretSource::File),
            PromptAnswer::Text("file:/tmp/custom-tunnel-api-key".to_string()),
            PromptAnswer::Bool(false),
            PromptAnswer::Bool(false),
            PromptAnswer::Bool(true),
        ]);

        let outcome = run_wizard(&mut backend, args, UiLanguage::En).unwrap();
        assert_eq!(
            outcome
                .build
                .pending
                .iter()
                .filter(|action| **action == PendingAction::ProvisionTunnelSecret)
                .count(),
            1
        );
        assert_eq!(
            outcome.summary.matches("provision tunnel secret").count(),
            1
        );
    }

    #[test]
    fn explicit_defaults_skip_mode_profile_and_satisfied_required_prompts() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Standalone),
            profile: Some(WorkerProfile::Normal),
            tunnel_id: Some("tunnel_explicit".to_string()),
            tunnel_api_key: Some("env:EXPLICIT_TUNNEL_SECRET".to_string()),
            ..ConfigInitArgs::default()
        };
        let mut backend =
            ScriptedPromptBackend::new([PromptAnswer::Bool(false), PromptAnswer::Bool(true)]);

        let outcome = run_wizard(&mut backend, args, UiLanguage::En).unwrap();
        assert_eq!(outcome.build.mode, RuntimeMode::Standalone);
        assert_eq!(
            request_ids(backend.requests()),
            vec![PromptId::ConfigureOptionalSections, PromptId::ConfirmWrite]
        );
    }

    #[test]
    fn hub_prompts_only_for_missing_required_values() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Hub),
            profile: Some(WorkerProfile::Normal),
            hub_url: Some("https://hub.example.com".to_string()),
            agent_id: Some("desk".to_string()),
            ..ConfigInitArgs::default()
        };
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::Text("sse".to_string()),
            PromptAnswer::Secret(SecretValue::new("hub-secret-marker")),
            PromptAnswer::Bool(false),
            PromptAnswer::Bool(true),
        ]);

        let outcome = run_wizard(&mut backend, args, UiLanguage::En).unwrap();
        assert_eq!(outcome.build.mode, RuntimeMode::Hub);
        assert_eq!(
            request_ids(backend.requests()),
            vec![
                PromptId::HubTransport,
                PromptId::AgentSecret,
                PromptId::ConfigureOptionalSections,
                PromptId::ConfirmWrite,
            ]
        );
    }

    #[test]
    fn local_prompt_excludes_hub_and_tunnel_requests() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Local),
            profile: Some(WorkerProfile::Normal),
            ..ConfigInitArgs::default()
        };
        let mut backend =
            ScriptedPromptBackend::new([PromptAnswer::Bool(false), PromptAnswer::Bool(true)]);

        let outcome = run_wizard(&mut backend, args, UiLanguage::En).unwrap();
        assert_eq!(outcome.build.mode, RuntimeMode::Local);
        assert_eq!(
            request_ids(backend.requests()),
            vec![PromptId::ConfigureOptionalSections, PromptId::ConfirmWrite]
        );
        assert!(!backend.requests().iter().any(|request| {
            matches!(
                request,
                PromptRequest::SelectSecretSource { .. }
                    | PromptRequest::Secret { .. }
                    | PromptRequest::Text {
                        id: PromptId::TunnelId | PromptId::HubUrl,
                        ..
                    }
            )
        }));
    }

    #[test]
    fn cancellation_at_final_confirmation_is_stable() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Local),
            profile: Some(WorkerProfile::Normal),
            ..ConfigInitArgs::default()
        };
        let mut backend =
            ScriptedPromptBackend::new([PromptAnswer::Bool(false), PromptAnswer::Bool(false)]);
        let error = match run_wizard(&mut backend, args, UiLanguage::En) {
            Ok(_) => panic!("final refusal unexpectedly returned an outcome"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_cancelled");
    }

    #[test]
    fn environment_secret_reference_creates_no_write_plan() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Standalone),
            profile: Some(WorkerProfile::Normal),
            tunnel_id: Some("tunnel_environment".to_string()),
            ..ConfigInitArgs::default()
        };
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::SecretSource(TunnelSecretSource::Environment),
            PromptAnswer::Text("MY_TUNNEL_SECRET".to_string()),
            PromptAnswer::Bool(false),
            PromptAnswer::Bool(true),
        ]);

        let outcome = run_wizard(&mut backend, args, UiLanguage::En).unwrap();
        assert_eq!(
            outcome.build.config.tunnel.as_ref().unwrap().api_key,
            "env:MY_TUNNEL_SECRET"
        );
        assert!(outcome.secret_write.is_none());
    }

    #[test]
    fn illegal_optional_section_is_rejected_without_collecting_values() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Local),
            profile: Some(WorkerProfile::Normal),
            ..ConfigInitArgs::default()
        };
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::Bool(true),
            PromptAnswer::Sections(vec![OptionalSection::TunnelClient]),
        ]);
        let error = match run_wizard(&mut backend, args, UiLanguage::En) {
            Ok(_) => panic!("illegal optional section unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "config_init_optional_section_invalid");
    }

    #[test]
    fn legal_optional_sections_collect_values_without_filesystem_access() {
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Local),
            profile: Some(WorkerProfile::Normal),
            ..ConfigInitArgs::default()
        };
        let mut backend = ScriptedPromptBackend::new([
            PromptAnswer::Bool(true),
            PromptAnswer::Sections(vec![OptionalSection::Identity, OptionalSection::Workspace]),
            PromptAnswer::Text("Desk Agent".to_string()),
            PromptAnswer::Text("/tmp/agent-workspace".to_string()),
            PromptAnswer::Text(r#"["/tmp/write-root"]"#.to_string()),
            PromptAnswer::Text(r#"["/tmp/read-only-root"]"#.to_string()),
            PromptAnswer::Text(r#"["/tmp/deny-root"]"#.to_string()),
            PromptAnswer::Bool(true),
        ]);

        let outcome = run_wizard(&mut backend, args, UiLanguage::En).unwrap();
        assert_eq!(outcome.build.config.display_name, "Desk Agent");
        assert_eq!(
            outcome.build.config.workspace_root,
            PathBuf::from("/tmp/agent-workspace")
        );
        assert_eq!(
            outcome.build.config.path_policy.write_roots,
            vec![PathBuf::from("/tmp/write-root")]
        );
        assert_eq!(
            outcome.build.config.path_policy.read_only_roots,
            vec![PathBuf::from("/tmp/read-only-root")]
        );
        assert_eq!(
            outcome.build.config.path_policy.deny_roots,
            vec![PathBuf::from("/tmp/deny-root")]
        );
    }

    #[test]
    fn summaries_are_bilingual_and_secret_redacted() {
        let marker = "bilingual-secret-marker";
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Hub),
            profile: Some(WorkerProfile::Normal),
            hub_url: Some("https://hub.example.com".to_string()),
            hub_transport: Some("websocket".to_string()),
            agent_id: Some("desk".to_string()),
            agent_secret: Some(marker.to_string()),
            ..ConfigInitArgs::default()
        };
        let mut backend =
            ScriptedPromptBackend::new([PromptAnswer::Bool(false), PromptAnswer::Bool(true)]);
        let outcome = run_wizard(&mut backend, args, UiLanguage::ZhCn).unwrap();
        assert!(outcome.summary.contains("配置"));
        assert!(outcome.summary.contains("[REDACTED]"));
        assert!(!outcome.summary.contains(marker));

        let english = render_summary(
            &outcome.build,
            outcome.secret_write.as_ref(),
            UiLanguage::En,
        );
        assert!(english.contains("Configuration"));
        assert!(english.contains("[REDACTED]"));
        assert!(!english.contains(marker));
    }

    #[test]
    fn interactive_init_requires_all_terminals_and_no_non_interactive_flag() {
        let cases = [
            (true, true, true, true, false),
            (false, false, true, true, false),
            (false, true, false, true, false),
            (false, true, true, false, false),
            (false, false, false, false, false),
            (false, true, true, true, true),
        ];

        for (non_interactive, stdin, stdout, stderr, expected) in cases {
            assert_eq!(
                should_use_interactive_init(non_interactive, stdin, stdout, stderr),
                expected,
                "unexpected interactive-init decision"
            );
        }
    }

    #[test]
    fn inquire_cancellation_errors_map_to_stable_code() {
        for error in [
            InquireError::OperationCanceled,
            InquireError::OperationInterrupted,
        ] {
            assert_eq!(
                map_inquire_error(error).to_string(),
                "config_init_cancelled"
            );
        }
    }

    #[test]
    fn inquire_non_cancellation_errors_keep_a_safe_source() {
        let error = map_inquire_error(InquireError::InvalidConfiguration(
            "invalid prompt configuration".to_string(),
        ));
        assert_eq!(
            error.to_string(),
            "config_init_prompt_failed: The prompt configuration is invalid: invalid prompt configuration"
        );
    }

    #[test]
    fn bilingual_adapter_catalog_covers_all_supported_values() {
        let prompt_ids = [
            PromptId::TunnelId,
            PromptId::TunnelSecretPath,
            PromptId::TunnelSecretEnvironment,
            PromptId::TunnelSecretValue,
            PromptId::HubUrl,
            PromptId::HubTransport,
            PromptId::AgentId,
            PromptId::AgentSecret,
            PromptId::DisplayName,
            PromptId::WorkspaceRoot,
            PromptId::WriteRoots,
            PromptId::ReadOnlyRoots,
            PromptId::DenyRoots,
            PromptId::ConfirmationProvider,
            PromptId::ConfirmationLanguage,
            PromptId::MaxConcurrentTasks,
            PromptId::MaxActiveJobs,
            PromptId::MaxFileSearchContextLines,
            PromptId::SandboxEnabled,
            PromptId::BubblewrapPath,
            PromptId::RequiredRuntimePaths,
            PromptId::RoomTimezone,
            PromptId::DiaryBoundaryHour,
            PromptId::NotebookRoot,
            PromptId::TunnelClientVersion,
            PromptId::TunnelCacheDir,
            PromptId::TunnelAutoDownload,
            PromptId::TunnelExecutable,
            PromptId::TunnelDownloadUrl,
            PromptId::TunnelSha256,
            PromptId::HubReportingEnabled,
            PromptId::HubReportingDetail,
            PromptId::ConfigureOptionalSections,
            PromptId::WriteSecretNow,
            PromptId::ConfirmWrite,
        ];
        let modes = [
            RuntimeMode::Standalone,
            RuntimeMode::Hub,
            RuntimeMode::Local,
        ];
        let profiles = [WorkerProfile::Normal, WorkerProfile::Room];
        let secret_sources = [TunnelSecretSource::File, TunnelSecretSource::Environment];
        let sections = [
            OptionalSection::Identity,
            OptionalSection::Workspace,
            OptionalSection::Confirmation,
            OptionalSection::Limits,
            OptionalSection::Sandbox,
            OptionalSection::Room,
            OptionalSection::TunnelClient,
            OptionalSection::HubReporting,
        ];

        for language in [UiLanguage::En, UiLanguage::ZhCn] {
            for id in prompt_ids {
                assert!(!prompt_label(id, language).trim().is_empty());
            }
            for mode in modes {
                assert!(!mode_option_label(mode, language).trim().is_empty());
            }
            for profile in profiles {
                assert!(!profile_option_label(profile, language).trim().is_empty());
            }
            for source in secret_sources {
                assert!(!secret_source_option_label(source, language)
                    .trim()
                    .is_empty());
            }
            for section in sections {
                assert!(!optional_section_label(section, language).trim().is_empty());
            }
            assert!(!mode_prompt_label(language).trim().is_empty());
            assert!(!profile_prompt_label(language).trim().is_empty());
            assert!(!secret_source_prompt_label(language).trim().is_empty());
            assert!(!optional_sections_prompt_label(language).trim().is_empty());
        }
    }

    #[test]
    fn secret_owning_types_use_the_redacted_debug_boundary() {
        let marker = "non-debug-secret-marker";
        let answer = PromptAnswer::Secret(SecretValue::new(marker));
        let plan = SecretWritePlan {
            path: PathBuf::from("secret-path"),
            value: SecretValue::new(marker),
        };
        let redacted_plan = format!("{plan:?}");

        assert!(!redacted_plan.contains(marker));
        // PromptAnswer and WizardOutcome intentionally have no Debug implementation;
        // this test only exercises the separately redacted SecretWritePlan boundary.
        drop(answer);
    }

    #[test]
    fn commit_creates_secret_parent_0700_file_0600_and_valid_config() {
        let root = fresh_test_root("permissions-create");
        let config_path = root.join("config").join("config.json");
        let secret_path = root.join("secrets").join("tunnel-api-key");

        commit_wizard_outcome(
            &config_path,
            test_outcome_with_secret(&secret_path, "permission-secret-marker"),
        )
        .unwrap();

        assert_eq!(
            fs::metadata(secret_path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(fs::read(&secret_path).unwrap().starts_with(b"permission-"));
        let config = crate::config::Config::load(&config_path).unwrap();
        assert_eq!(
            config.tunnel.as_ref().unwrap().api_key,
            format!("file:{}", secret_path.display())
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn commit_replacement_leaves_secret_file_0600() {
        let root = fresh_test_root("permissions-replace");
        let config_path = root.join("config.json");
        let secret_path = root.join("secrets").join("tunnel-api-key");
        fs::create_dir_all(secret_path.parent().unwrap()).unwrap();
        fs::write(&secret_path, b"old-secret").unwrap();
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o640)).unwrap();

        commit_wizard_outcome(
            &config_path,
            test_outcome_with_secret(&secret_path, "replacement-secret"),
        )
        .unwrap();

        assert_eq!(
            fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            fs::read(&secret_path).unwrap() == b"replacement-secret",
            "replacement secret bytes were not committed"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn config_write_failure_restores_existing_secret_bytes_and_mode() {
        let root = fresh_test_root("rollback-existing");
        let secret_path = root.join("secrets").join("tunnel-api-key");
        fs::create_dir_all(secret_path.parent().unwrap()).unwrap();
        let original = b"original-secret";
        fs::write(&secret_path, original).unwrap();
        fs::set_permissions(&secret_path, fs::Permissions::from_mode(0o640)).unwrap();
        let config_path = root.join("config-target");
        fs::create_dir_all(&config_path).unwrap();

        let error = match commit_wizard_outcome(
            &config_path,
            test_outcome_with_secret(&secret_path, "replacement-secret"),
        ) {
            Ok(_) => panic!("config write unexpectedly succeeded"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "config_init_config_write_failed");
        assert!(
            fs::read(&secret_path).unwrap() == original,
            "existing secret bytes were not restored"
        );
        assert_eq!(
            fs::metadata(&secret_path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn config_write_failure_after_absent_secret_removes_new_target() {
        let root = fresh_test_root("rollback-absent");
        let secret_path = root.join("secrets").join("tunnel-api-key");
        let config_path = root.join("config-target");
        fs::create_dir_all(&config_path).unwrap();

        let error = match commit_wizard_outcome(
            &config_path,
            test_outcome_with_secret(&secret_path, "new-secret"),
        ) {
            Ok(_) => panic!("config write unexpectedly succeeded"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "config_init_config_write_failed");
        assert!(!secret_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rollback_leaves_no_same_directory_temporary_files() {
        let root = fresh_test_root("rollback-temp-cleanup");
        let secret_dir = root.join("secrets");
        let secret_path = secret_dir.join("tunnel-api-key");
        let config_path = root.join("config-target");
        fs::create_dir_all(&config_path).unwrap();

        let _ = commit_wizard_outcome(
            &config_path,
            test_outcome_with_secret(&secret_path, "new-secret"),
        );

        if secret_dir.exists() {
            let leftovers = fs::read_dir(&secret_dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .contains("agentic-gpt-tmp")
                })
                .count();
            assert_eq!(leftovers, 0);
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_secret_target_fails_before_config_or_secret_mutation() {
        let root = fresh_test_root("invalid-target");
        let config_path = root.join("config.json");
        fs::write(&config_path, b"existing-config").unwrap();
        let secret_path = root.join("secret-directory");
        fs::create_dir_all(&secret_path).unwrap();

        let error = match commit_wizard_outcome(
            &config_path,
            test_outcome_with_secret(&secret_path, "invalid-target-secret"),
        ) {
            Ok(_) => panic!("invalid secret target unexpectedly succeeded"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "config_init_secret_path_invalid");
        assert_eq!(fs::read(&config_path).unwrap(), b"existing-config");
        assert!(secret_path.is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_secret_outcome_writes_config_with_existing_writer() {
        let root = fresh_test_root("no-secret");
        let config_path = root.join("nested").join("config.json");

        let summary = commit_wizard_outcome(&config_path, test_outcome_without_secret()).unwrap();

        assert_eq!(summary.config_path, config_path);
        assert!(config_path.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn final_refusal_creates_neither_config_nor_secret() {
        let root = fresh_test_root("final-refusal");
        let config_path = root.join("config.json");
        let secret_path = root.join("secrets").join("tunnel-api-key");
        let args = ConfigInitArgs {
            mode: Some(RuntimeMode::Standalone),
            profile: Some(WorkerProfile::Normal),
            tunnel_id: Some("tunnel_refusal".into()),
            tunnel_api_key: Some(format!("file:{}", secret_path.display())),
            ..ConfigInitArgs::default()
        };
        let mut backend =
            ScriptedPromptBackend::new([PromptAnswer::Bool(false), PromptAnswer::Bool(false)]);

        let error = match run_wizard(&mut backend, args, UiLanguage::En) {
            Ok(_) => panic!("final refusal unexpectedly succeeded"),
            Err(error) => error,
        };

        assert_eq!(error.to_string(), "config_init_cancelled");
        assert!(!config_path.exists());
        assert!(!secret_path.exists());
        let _ = fs::remove_dir_all(root);
    }
}
