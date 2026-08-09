#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::cli_i18n::UiLanguage;
    use crate::config_templates::{OptionalSection, RuntimeMode, SecretValue};
    use crate::WorkerProfile;

    use super::super::model::{
        HubReportingDraft, LimitsDraft, OptionalSectionDraft, RoomDraft, SandboxDraft, SetupField,
        SetupSeed, SetupSession, WorkspaceDraft,
    };

    fn session(mode: RuntimeMode, profile: WorkerProfile) -> SetupSession {
        SetupSession::new(
            SetupSeed {
                mode: Some(mode),
                profile: Some(profile),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        )
    }

    #[test]
    fn required_connection_fields_report_concrete_domain_fields() {
        let session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_id: Some(String::new()),
                tunnel_api_key: Some("file:/tmp/tunnel-secret".into()),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        );
        assert_eq!(
            session.validate_connection().unwrap_err()[0].field,
            SetupField::TunnelId
        );
    }

    #[test]
    fn hub_connection_transport_and_secret_are_structured() {
        let mut session = session(RuntimeMode::Hub, WorkerProfile::Normal);
        session.hub_mut().hub_url = "ftp://hub.example.com".to_string();
        session.hub_mut().hub_transport = "polling".to_string();
        session.hub_mut().agent_secret = None;
        let errors = session.validate_connection().unwrap_err();
        assert_eq!(errors[0].field, SetupField::HubUrl);
        assert_eq!(errors[0].code, "hub_url_invalid");
        assert!(errors.iter().any(|error| {
            error.field == SetupField::HubTransport && error.code == "hub_transport_invalid"
        }));
        assert!(errors.iter().any(|error| {
            error.field == SetupField::AgentSecret && error.code == "config_init_secret_empty"
        }));
    }

    #[test]
    fn optional_validation_covers_paths_numbers_runtime_paths_and_reporting() {
        let mut session = session(RuntimeMode::Local, WorkerProfile::Normal);
        let workspace = OptionalSectionDraft::Workspace(WorkspaceDraft {
            workspace_root: "/tmp/workspace".to_string(),
            write_roots: "not-json".to_string(),
            read_only_roots: "[]".to_string(),
            deny_roots: "[]".to_string(),
        });
        let errors = session.save_optional_section(workspace).unwrap_err();
        assert_eq!(errors[0].field, SetupField::WriteRoots);
        assert_eq!(
            errors[0].code,
            "config_init_path_policy_write_roots_invalid"
        );

        let limits = OptionalSectionDraft::Limits(LimitsDraft {
            max_concurrent_tasks: "two".to_string(),
            max_active_jobs: "never".to_string(),
            max_file_search_context_lines: "five".to_string(),
        });
        let errors = session.save_optional_section(limits).unwrap_err();
        assert_eq!(errors[0].field, SetupField::MaxConcurrentTasks);
        assert!(errors.iter().any(|error| {
            error.field == SetupField::MaxActiveJobs
                && error.code == "config_init_number_invalid: max_active_jobs"
        }));

        let sandbox = OptionalSectionDraft::Sandbox(SandboxDraft {
            enabled: true,
            bubblewrap_path: "bwrap".to_string(),
            required_runtime_paths: "{\"/usr\":true}".to_string(),
        });
        let errors = session.save_optional_section(sandbox).unwrap_err();
        assert_eq!(errors[0].field, SetupField::RequiredRuntimePaths);
        assert_eq!(errors[0].code, "config_init_runtime_paths_invalid");

        let room = OptionalSectionDraft::Room(RoomDraft {
            timezone: "Asia/Shanghai".to_string(),
            diary_boundary_hour: "24".to_string(),
            notebook_root: String::new(),
        });
        let errors = session.save_optional_section(room).unwrap_err();
        assert_eq!(errors[0].field, SetupField::RoomTimezone);
        assert_eq!(errors[0].code, "config_init_optional_section_invalid");

        let reporting = OptionalSectionDraft::HubReporting(HubReportingDraft {
            enabled: true,
            detail: "everything".to_string(),
        });
        let errors = session.save_optional_section(reporting).unwrap_err();
        assert_eq!(errors[0].field, SetupField::HubReportingDetail);
        assert_eq!(errors[0].code, "config_init_optional_section_invalid");

        assert_eq!(
            session.section_status(OptionalSection::Room),
            super::super::model::SectionStatus::NotApplicable
        );
    }

    #[test]
    fn active_input_ignores_inactive_connection_secrets_and_restores_staged_drafts() {
        let marker = "hub-secret-active-input-marker";
        let mut session = SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                profile: Some(WorkerProfile::Normal),
                tunnel_id: Some("tunnel-staged".to_string()),
                tunnel_api_key: Some("file:/tmp/staged-tunnel-secret".to_string()),
                hub_url: Some("https://hub.example.com".to_string()),
                hub_transport: Some("websocket".to_string()),
                agent_id: Some("desk".to_string()),
                agent_secret: Some(SecretValue::new(marker)),
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config.json"),
        );

        let hub_input = session.build_active_input().unwrap();
        assert_eq!(hub_input.mode, RuntimeMode::Hub);
        assert!(hub_input.tunnel_id.is_none());
        assert!(hub_input.tunnel_api_key.is_none());
        assert_eq!(hub_input.agent_secret.unwrap().expose(), marker);

        session.set_mode(RuntimeMode::Standalone);
        let standalone_input = session.build_active_input().unwrap();
        assert_eq!(standalone_input.mode, RuntimeMode::Standalone);
        assert_eq!(standalone_input.tunnel_id.as_deref(), Some("tunnel-staged"));
        assert_eq!(
            standalone_input.tunnel_api_key.as_deref(),
            Some("file:/tmp/staged-tunnel-secret")
        );
        assert!(standalone_input.agent_secret.is_none());

        session.set_mode(RuntimeMode::Hub);
        let restored_hub = session.build_active_input().unwrap();
        assert_eq!(
            restored_hub.hub_url.as_deref(),
            Some("https://hub.example.com")
        );
        assert_eq!(restored_hub.agent_secret.unwrap().expose(), marker);
    }
}
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::config::{
    self, default_room_config, ConfirmationProviderConfig, HubReportingConfig, LimitsConfig,
    MaxActiveJobs, PathPolicyConfig, ReportingDetail, RoomConfig, SandboxConfig,
    TunnelClientConfig,
};
use crate::config_templates::{
    self, build_config, InitInput, OptionalSection, RuntimeMode, SecretValue, TunnelSecretSource,
};
use crate::mcp::{self, McpServerConfig};
use crate::WorkerProfile;

use super::model::{
    HubDraft, McpServerDraft, OptionalSectionDraft, SetupField, SetupSession, StandaloneDraft,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidationError {
    pub(crate) field: SetupField,
    pub(crate) code: &'static str,
}

pub(crate) type ValidationErrors = Vec<ValidationError>;

fn error(field: SetupField, code: &'static str) -> ValidationError {
    ValidationError { field, code }
}

pub(super) fn section_is_legal(
    section: OptionalSection,
    mode: RuntimeMode,
    profile: WorkerProfile,
) -> bool {
    config_templates::optional_section_is_legal(section, mode, profile)
}

pub(super) fn available_optional_sections(
    mode: RuntimeMode,
    profile: WorkerProfile,
) -> Vec<OptionalSection> {
    [
        OptionalSection::Identity,
        OptionalSection::Workspace,
        OptionalSection::Confirmation,
        OptionalSection::Limits,
        OptionalSection::Sandbox,
        OptionalSection::McpServers,
        OptionalSection::Room,
        OptionalSection::TunnelClient,
        OptionalSection::HubReporting,
    ]
    .into_iter()
    .filter(|section| section_is_legal(*section, mode, profile))
    .collect()
}

pub(super) fn validate_basic(_session: &SetupSession) -> Result<(), ValidationErrors> {
    // RuntimeMode and WorkerProfile are closed enums, so selecting either
    // value cannot produce a malformed basic draft. Keeping this method
    // explicit gives the frontend a stable validation boundary.
    Ok(())
}

pub(super) fn validate_connection(session: &SetupSession) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();
    match session.selected_mode() {
        RuntimeMode::Standalone => validate_standalone(
            session.standalone(),
            session.tunnel_seed_error(),
            &mut errors,
        ),
        RuntimeMode::Hub => validate_hub(session.hub(), &mut errors),
        RuntimeMode::Local => {}
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_standalone(
    draft: &StandaloneDraft,
    seed_error: Option<&'static str>,
    errors: &mut ValidationErrors,
) {
    if draft.tunnel_id.trim().is_empty() {
        errors.push(error(
            SetupField::TunnelId,
            "config_init_required_value_missing: tunnel_id",
        ));
    }

    match draft.secret_source {
        TunnelSecretSource::File => {
            if let Some(code) = seed_error.filter(|_| draft.secret_path.trim().is_empty()) {
                errors.push(error(SetupField::TunnelSecretPath, code));
            } else if normalize_file_path(&draft.secret_path).is_empty() {
                errors.push(error(
                    SetupField::TunnelSecretPath,
                    "config_init_secret_path_invalid",
                ));
            }
        }
        TunnelSecretSource::Environment => {
            let name = normalize_environment_name(&draft.secret_environment);
            if config::validate_secret_reference(&format!("env:{name}")).is_err() {
                errors.push(error(
                    SetupField::TunnelSecretEnvironment,
                    "tunnel_api_key_reference_invalid",
                ));
            }
        }
    }

    if draft.provision_secret_now {
        if draft.secret_source != TunnelSecretSource::File {
            errors.push(error(
                SetupField::TunnelSecretSource,
                "config_init_secret_source_invalid",
            ));
        }
        if draft
            .secret_value
            .as_ref()
            .map(|value| value.expose().trim().is_empty())
            .unwrap_or(true)
        {
            errors.push(error(
                SetupField::TunnelSecretValue,
                "config_init_secret_empty",
            ));
        }
    }
}

fn validate_hub(draft: &HubDraft, errors: &mut ValidationErrors) {
    if draft.hub_url.trim().is_empty() {
        errors.push(error(
            SetupField::HubUrl,
            "config_init_required_value_missing: hub_url",
        ));
    } else if config::validate_hub_url_shape(draft.hub_url.trim()).is_err() {
        errors.push(error(SetupField::HubUrl, "hub_url_invalid"));
    }
    if draft.hub_transport.trim().is_empty()
        || config::validate_hub_transport(draft.hub_transport.trim()).is_err()
    {
        errors.push(error(SetupField::HubTransport, "hub_transport_invalid"));
    }
    if draft.agent_id.trim().is_empty() {
        errors.push(error(SetupField::AgentId, "agent_id_required"));
    }
    if draft
        .agent_secret
        .as_ref()
        .map(|secret| secret.expose().trim().is_empty())
        .unwrap_or(true)
    {
        errors.push(error(SetupField::AgentSecret, "config_init_secret_empty"));
    }
}

pub(super) fn validate_field(
    session: &SetupSession,
    field: SetupField,
) -> Result<(), ValidationErrors> {
    let errors = match field {
        SetupField::Mode | SetupField::Profile | SetupField::TunnelSecretSource => Vec::new(),
        SetupField::TunnelId
        | SetupField::TunnelSecretPath
        | SetupField::TunnelSecretEnvironment
        | SetupField::ProvisionTunnelSecret
        | SetupField::TunnelSecretValue
        | SetupField::HubUrl
        | SetupField::HubTransport
        | SetupField::AgentId
        | SetupField::AgentSecret => validate_connection(session).err().unwrap_or_default(),
        SetupField::DisplayName => validate_optional(
            OptionalSection::Identity,
            &session.optional_draft(OptionalSection::Identity),
        ),
        SetupField::WorkspaceRoot
        | SetupField::WriteRoots
        | SetupField::ReadOnlyRoots
        | SetupField::DenyRoots => validate_optional(
            OptionalSection::Workspace,
            &session.optional_draft(OptionalSection::Workspace),
        ),
        SetupField::ConfirmationProvider | SetupField::ConfirmationLanguage => validate_optional(
            OptionalSection::Confirmation,
            &session.optional_draft(OptionalSection::Confirmation),
        ),
        SetupField::MaxConcurrentTasks
        | SetupField::MaxActiveJobs
        | SetupField::MaxFileSearchContextLines => validate_optional(
            OptionalSection::Limits,
            &session.optional_draft(OptionalSection::Limits),
        ),
        SetupField::SandboxEnabled
        | SetupField::BubblewrapPath
        | SetupField::RequiredRuntimePaths => validate_optional(
            OptionalSection::Sandbox,
            &session.optional_draft(OptionalSection::Sandbox),
        ),
        SetupField::McpServerId
        | SetupField::McpServerEnabled
        | SetupField::McpServerTransport
        | SetupField::McpServerEndpoint => validate_optional(
            OptionalSection::McpServers,
            &session.optional_draft(OptionalSection::McpServers),
        ),
        SetupField::RoomTimezone | SetupField::DiaryBoundaryHour | SetupField::NotebookRoot => {
            validate_optional(
                OptionalSection::Room,
                &session.optional_draft(OptionalSection::Room),
            )
        }
        SetupField::TunnelClientVersion
        | SetupField::TunnelCacheDir
        | SetupField::TunnelAutoDownload
        | SetupField::TunnelExecutable
        | SetupField::TunnelDownloadUrl
        | SetupField::TunnelSha256 => validate_optional(
            OptionalSection::TunnelClient,
            &session.optional_draft(OptionalSection::TunnelClient),
        ),
        SetupField::HubReportingEnabled | SetupField::HubReportingDetail => validate_optional(
            OptionalSection::HubReporting,
            &session.optional_draft(OptionalSection::HubReporting),
        ),
    };
    let errors: ValidationErrors = errors
        .into_iter()
        .filter(|item| item.field == field)
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub(super) fn save_optional_section(
    session: &mut SetupSession,
    draft: OptionalSectionDraft,
) -> Result<(), ValidationErrors> {
    validate_optional_draft(session, &draft)?;
    session.replace_optional(draft);
    Ok(())
}

pub(super) fn validate_optional_draft(
    session: &SetupSession,
    draft: &OptionalSectionDraft,
) -> Result<(), ValidationErrors> {
    let section = draft.section();
    if !section_is_legal(section, session.selected_mode(), session.selected_profile()) {
        return Err(vec![error(
            first_field(section),
            "config_init_optional_section_invalid",
        )]);
    }
    let errors = validate_optional(section, draft);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_optional(section: OptionalSection, draft: &OptionalSectionDraft) -> ValidationErrors {
    let mut errors = Vec::new();
    match (section, draft) {
        (OptionalSection::Identity, OptionalSectionDraft::Identity(value)) => required(
            &value.display_name,
            SetupField::DisplayName,
            "display_name",
            &mut errors,
        ),
        (OptionalSection::Workspace, OptionalSectionDraft::Workspace(value)) => {
            required(
                &value.workspace_root,
                SetupField::WorkspaceRoot,
                "workspace_root",
                &mut errors,
            );
            parse_path_list(
                &value.write_roots,
                SetupField::WriteRoots,
                "config_init_path_policy_write_roots_invalid",
                &mut errors,
            );
            parse_path_list(
                &value.read_only_roots,
                SetupField::ReadOnlyRoots,
                "config_init_path_policy_read_only_roots_invalid",
                &mut errors,
            );
            parse_path_list(
                &value.deny_roots,
                SetupField::DenyRoots,
                "config_init_path_policy_deny_roots_invalid",
                &mut errors,
            );
        }
        (OptionalSection::Confirmation, OptionalSectionDraft::Confirmation(value)) => {
            required(
                &value.provider,
                SetupField::ConfirmationProvider,
                "confirmation_provider",
                &mut errors,
            );
            if !value.provider.trim().is_empty()
                && ConfirmationProviderConfig::from_legacy(value.provider.trim()).is_err()
            {
                errors.push(error(
                    SetupField::ConfirmationProvider,
                    "config_init_confirmation_provider_invalid",
                ));
            }
            required(
                &value.language,
                SetupField::ConfirmationLanguage,
                "confirmation_language",
                &mut errors,
            );
        }
        (OptionalSection::Limits, OptionalSectionDraft::Limits(value)) => {
            parse_usize(
                &value.max_concurrent_tasks,
                SetupField::MaxConcurrentTasks,
                &mut errors,
            );
            parse_max_active_jobs(&value.max_active_jobs, &mut errors);
            match parse_usize_value(&value.max_file_search_context_lines) {
                Ok(value) if value <= config::MAX_FILE_SEARCH_CONTEXT_LINES => {}
                _ => errors.push(error(
                    SetupField::MaxFileSearchContextLines,
                    "config_init_number_invalid: max_file_search_context_lines",
                )),
            }
        }
        (OptionalSection::Sandbox, OptionalSectionDraft::Sandbox(value)) => {
            required(
                &value.bubblewrap_path,
                SetupField::BubblewrapPath,
                "bubblewrap_path",
                &mut errors,
            );
            if serde_json::from_str::<Vec<PathBuf>>(value.required_runtime_paths.trim()).is_err() {
                errors.push(error(
                    SetupField::RequiredRuntimePaths,
                    "config_init_runtime_paths_invalid",
                ));
            }
        }
        (OptionalSection::McpServers, OptionalSectionDraft::McpServers(value)) => {
            if let Err(mcp_errors) = mcp_servers_from_draft(&value.servers) {
                errors.extend(mcp_errors);
            }
        }
        (OptionalSection::Room, OptionalSectionDraft::Room(value)) => {
            required(
                &value.timezone,
                SetupField::RoomTimezone,
                "room_timezone",
                &mut errors,
            );
            match parse_u32_value(&value.diary_boundary_hour) {
                Ok(hour) if hour <= 23 => {}
                _ => errors.push(error(
                    SetupField::DiaryBoundaryHour,
                    "config_init_number_invalid: diary_boundary_hour",
                )),
            }
        }
        (OptionalSection::TunnelClient, OptionalSectionDraft::TunnelClient(value)) => {
            if let Some(url) = optional_text(&value.download_url) {
                if !url.starts_with("https://") {
                    errors.push(error(
                        SetupField::TunnelDownloadUrl,
                        "tunnel_download_url_must_use_https",
                    ));
                }
                if value.sha256.trim().is_empty() {
                    errors.push(error(
                        SetupField::TunnelSha256,
                        "tunnel_download_sha256_required",
                    ));
                }
            }
            if let Some(sha256) = optional_text(&value.sha256) {
                if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    errors.push(error(SetupField::TunnelSha256, "tunnel_sha256_invalid"));
                }
            }
        }
        (OptionalSection::HubReporting, OptionalSectionDraft::HubReporting(value)) => {
            if !matches!(value.detail.trim(), "metadata" | "full") {
                errors.push(error(
                    SetupField::HubReportingDetail,
                    "config_init_reporting_detail_invalid",
                ));
            }
        }
        _ => errors.push(error(
            first_field(section),
            "config_init_optional_section_invalid",
        )),
    }
    errors
}

fn mcp_servers_from_draft(
    servers: &[McpServerDraft],
) -> Result<BTreeMap<String, McpServerConfig>, ValidationErrors> {
    let mut ids = HashSet::new();
    let mut configs = BTreeMap::new();
    for server in servers {
        if !ids.insert(server.id.clone()) {
            return Err(vec![error(
                SetupField::McpServerId,
                "config_init_mcp_server_id_duplicate",
            )]);
        }
        configs.insert(
            server.id.clone(),
            McpServerConfig {
                enabled: server.enabled,
                transport: server.transport.clone(),
                url: Some(server.endpoint.clone()),
            },
        );
    }
    if let Err(validation_error) = mcp::validate_server_configs(&configs) {
        let code = validation_error.to_string();
        let (field, safe_code) = if code.starts_with("mcp_server_id_invalid") {
            (SetupField::McpServerId, "config_init_mcp_server_id_invalid")
        } else if code.starts_with("unsupported_mcp_transport") {
            (
                SetupField::McpServerTransport,
                "config_init_mcp_transport_invalid",
            )
        } else {
            (
                SetupField::McpServerEndpoint,
                "config_init_mcp_endpoint_invalid",
            )
        };
        return Err(vec![error(field, safe_code)]);
    }
    Ok(configs)
}

fn required(value: &str, field: SetupField, key: &'static str, errors: &mut ValidationErrors) {
    if value.trim().is_empty() {
        errors.push(error(
            field,
            match key {
                "display_name" => "config_init_required_value_missing: display_name",
                "workspace_root" => "config_init_required_value_missing: workspace_root",
                "confirmation_provider" => {
                    "config_init_required_value_missing: confirmation_provider"
                }
                "confirmation_language" => {
                    "config_init_required_value_missing: confirmation_language"
                }
                "bubblewrap_path" => "config_init_required_value_missing: bubblewrap_path",
                "room_timezone" => "config_init_required_value_missing: room_timezone",
                _ => "config_init_required_value_missing",
            },
        ));
    }
}

fn parse_path_list(
    value: &str,
    field: SetupField,
    code: &'static str,
    errors: &mut ValidationErrors,
) {
    if serde_json::from_str::<Vec<PathBuf>>(value.trim()).is_err() {
        errors.push(error(field, code));
    }
}

fn parse_usize(value: &str, field: SetupField, errors: &mut ValidationErrors) {
    if parse_usize_value(value).is_err() {
        errors.push(error(field, number_code(field)));
    }
}

fn parse_max_active_jobs(value: &str, errors: &mut ValidationErrors) {
    if value.trim() != "auto" && parse_usize_value(value).is_err() {
        errors.push(error(
            SetupField::MaxActiveJobs,
            "config_init_number_invalid: max_active_jobs",
        ));
    }
}

fn parse_usize_value(value: &str) -> Result<usize, ()> {
    value.trim().parse::<usize>().map_err(|_| ())
}

fn parse_u32_value(value: &str) -> Result<u32, ()> {
    value.trim().parse::<u32>().map_err(|_| ())
}

fn number_code(field: SetupField) -> &'static str {
    match field {
        SetupField::MaxConcurrentTasks => "config_init_number_invalid: max_concurrent_tasks",
        SetupField::MaxFileSearchContextLines => {
            "config_init_number_invalid: max_file_search_context_lines"
        }
        _ => "config_init_number_invalid",
    }
}

fn optional_text(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn normalize_file_path(value: &str) -> &str {
    value
        .trim()
        .strip_prefix("file:")
        .unwrap_or_else(|| value.trim())
        .trim()
}

fn normalize_environment_name(value: &str) -> String {
    value
        .trim()
        .strip_prefix("env:")
        .unwrap_or_else(|| value.trim())
        .trim()
        .to_string()
}

pub(super) fn validate_for_review(session: &SetupSession) -> Result<(), ValidationErrors> {
    let mut errors = Vec::new();
    if let Err(mut basic) = validate_basic(session) {
        errors.append(&mut basic);
    }
    if let Err(mut connection) = validate_connection(session) {
        errors.append(&mut connection);
    }
    for section in session.available_optional_sections() {
        if let Some(draft) = configured_draft(session, section) {
            errors.extend(validate_optional(section, &draft));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let input = build_active_input_unchecked(session)?;
    if let Err(build_error) = build_config(input) {
        return Err(vec![map_build_error(session, build_error.to_string())]);
    }
    Ok(())
}

fn configured_draft(
    session: &SetupSession,
    section: OptionalSection,
) -> Option<OptionalSectionDraft> {
    let drafts = session.optional_drafts();
    match section {
        OptionalSection::Identity => drafts.identity.clone().map(OptionalSectionDraft::Identity),
        OptionalSection::Workspace => drafts
            .workspace
            .clone()
            .map(OptionalSectionDraft::Workspace),
        OptionalSection::Confirmation => drafts
            .confirmation
            .clone()
            .map(OptionalSectionDraft::Confirmation),
        OptionalSection::Limits => drafts.limits.clone().map(OptionalSectionDraft::Limits),
        OptionalSection::Sandbox => drafts.sandbox.clone().map(OptionalSectionDraft::Sandbox),
        OptionalSection::McpServers => drafts
            .mcp_servers
            .clone()
            .map(OptionalSectionDraft::McpServers),
        OptionalSection::Room => drafts.room.clone().map(OptionalSectionDraft::Room),
        OptionalSection::TunnelClient => drafts
            .tunnel_client
            .clone()
            .map(OptionalSectionDraft::TunnelClient),
        OptionalSection::HubReporting => drafts
            .hub_reporting
            .clone()
            .map(OptionalSectionDraft::HubReporting),
    }
}

pub(super) fn build_active_input(session: &SetupSession) -> Result<InitInput, ValidationErrors> {
    validate_for_review(session)?;
    build_active_input_unchecked(session)
}

pub(super) fn build_active_input_unchecked(
    session: &SetupSession,
) -> Result<InitInput, ValidationErrors> {
    let mut input = InitInput::non_interactive_defaults(session.language());
    input.mode = session.selected_mode();
    input.profile = session.selected_profile();

    match session.selected_mode() {
        RuntimeMode::Standalone => {
            input.tunnel_id = Some(session.standalone().tunnel_id.clone());
            let draft = session.standalone();
            let reference = match draft.secret_source {
                TunnelSecretSource::File => {
                    format!("file:{}", normalize_file_path(&draft.secret_path))
                }
                TunnelSecretSource::Environment => {
                    format!(
                        "env:{}",
                        normalize_environment_name(&draft.secret_environment)
                    )
                }
            };
            input.tunnel_api_key = Some(reference);
        }
        RuntimeMode::Hub => {
            input.hub_url = Some(session.hub().hub_url.clone());
            input.hub_transport = Some(session.hub().hub_transport.clone());
            input.agent_id = Some(session.hub().agent_id.clone());
            input.agent_secret = session
                .hub()
                .agent_secret
                .as_ref()
                .map(|secret| SecretValue::new(secret.expose()));
        }
        RuntimeMode::Local => {}
    }

    for section in session.available_optional_sections() {
        if let Some(draft) = configured_draft(session, section) {
            apply_optional_draft(&mut input, section, &draft)?;
        }
    }
    Ok(input)
}

fn apply_optional_draft(
    input: &mut InitInput,
    section: OptionalSection,
    draft: &OptionalSectionDraft,
) -> Result<(), ValidationErrors> {
    match (section, draft) {
        (OptionalSection::Identity, OptionalSectionDraft::Identity(value)) => {
            input.display_name = Some(value.display_name.trim().to_string());
        }
        (OptionalSection::Workspace, OptionalSectionDraft::Workspace(value)) => {
            let parse = |text: &str, field: SetupField, code: &'static str| {
                serde_json::from_str::<Vec<PathBuf>>(text.trim())
                    .map_err(|_| vec![error(field, code)])
            };
            let path_policy = PathPolicyConfig {
                write_roots: parse(
                    &value.write_roots,
                    SetupField::WriteRoots,
                    "config_init_path_policy_write_roots_invalid",
                )?,
                read_only_roots: parse(
                    &value.read_only_roots,
                    SetupField::ReadOnlyRoots,
                    "config_init_path_policy_read_only_roots_invalid",
                )?,
                deny_roots: parse(
                    &value.deny_roots,
                    SetupField::DenyRoots,
                    "config_init_path_policy_deny_roots_invalid",
                )?,
            };
            input.workspace_root = Some(PathBuf::from(value.workspace_root.trim()));
            input.path_policy = Some(path_policy);
        }
        (OptionalSection::Confirmation, OptionalSectionDraft::Confirmation(value)) => {
            input.confirmation_provider = Some(
                ConfirmationProviderConfig::from_legacy(value.provider.trim()).map_err(|_| {
                    vec![error(
                        SetupField::ConfirmationProvider,
                        "config_init_confirmation_provider_invalid",
                    )]
                })?,
            );
            input.confirmation_language = Some(value.language.trim().to_string());
        }
        (OptionalSection::Limits, OptionalSectionDraft::Limits(value)) => {
            let max_concurrent_tasks =
                value
                    .max_concurrent_tasks
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| {
                        vec![error(
                            SetupField::MaxConcurrentTasks,
                            "config_init_number_invalid: max_concurrent_tasks",
                        )]
                    })?;
            let max_active_jobs = if value.max_active_jobs.trim() == "auto" {
                MaxActiveJobs::Auto
            } else {
                MaxActiveJobs::Explicit(value.max_active_jobs.trim().parse().map_err(|_| {
                    vec![error(
                        SetupField::MaxActiveJobs,
                        "config_init_number_invalid: max_active_jobs",
                    )]
                })?)
            };
            let max_file_search_context_lines = value
                .max_file_search_context_lines
                .trim()
                .parse::<usize>()
                .map_err(|_| {
                    vec![error(
                        SetupField::MaxFileSearchContextLines,
                        "config_init_number_invalid: max_file_search_context_lines",
                    )]
                })?;
            input.limits = Some(LimitsConfig {
                max_concurrent_tasks,
                max_active_jobs,
                max_file_search_context_lines,
            });
        }
        (OptionalSection::Sandbox, OptionalSectionDraft::Sandbox(value)) => {
            let required_runtime_paths = serde_json::from_str(&value.required_runtime_paths)
                .map_err(|_| {
                    vec![error(
                        SetupField::RequiredRuntimePaths,
                        "config_init_runtime_paths_invalid",
                    )]
                })?;
            input.sandbox = Some(SandboxConfig {
                enabled: value.enabled,
                bubblewrap_path: value.bubblewrap_path.trim().to_string(),
                required_runtime_paths,
            });
        }
        (OptionalSection::McpServers, OptionalSectionDraft::McpServers(value)) => {
            input.mcp_servers = Some(mcp_servers_from_draft(&value.servers)?);
        }
        (OptionalSection::Room, OptionalSectionDraft::Room(value)) => {
            let diary_day_boundary_hour =
                value.diary_boundary_hour.trim().parse().map_err(|_| {
                    vec![error(
                        SetupField::DiaryBoundaryHour,
                        "config_init_number_invalid: diary_boundary_hour",
                    )]
                })?;
            input.room = Some(RoomConfig {
                notebook_root: optional_path(&value.notebook_root),
                timezone: value.timezone.trim().to_string(),
                diary_day_boundary_hour,
                skills: default_room_config().skills,
            });
        }
        (OptionalSection::TunnelClient, OptionalSectionDraft::TunnelClient(value)) => {
            input.tunnel_client = Some(TunnelClientConfig {
                version: optional_string(&value.version),
                cache_dir: if value.cache_dir.trim().is_empty() {
                    PathBuf::from("~/.agentic_gpt/cache/tunnel-client")
                } else {
                    PathBuf::from(value.cache_dir.trim())
                },
                auto_download: value.auto_download,
                executable: optional_path(&value.executable),
                download_url: optional_string(&value.download_url),
                sha256: optional_string(&value.sha256),
            });
        }
        (OptionalSection::HubReporting, OptionalSectionDraft::HubReporting(value)) => {
            let detail = match value.detail.trim() {
                "metadata" => ReportingDetail::Metadata,
                "full" => ReportingDetail::Full,
                _ => {
                    return Err(vec![error(
                        SetupField::HubReportingDetail,
                        "config_init_reporting_detail_invalid",
                    )])
                }
            };
            input.hub_reporting = Some(HubReportingConfig {
                enabled: value.enabled,
                detail,
            });
        }
        _ => {
            return Err(vec![error(
                first_field(section),
                "config_init_optional_section_invalid",
            )])
        }
    }
    Ok(())
}

fn optional_string(value: &str) -> Option<String> {
    optional_text(value).map(ToString::to_string)
}

fn optional_path(value: &str) -> Option<PathBuf> {
    optional_string(value).map(PathBuf::from)
}

fn map_build_error(session: &SetupSession, code: String) -> ValidationError {
    let (field, safe_code) = match code.as_str() {
        "tunnel_id_required" => (SetupField::TunnelId, "tunnel_id_required"),
        "tunnel_api_key_reference_invalid" => (
            SetupField::TunnelSecretPath,
            "tunnel_api_key_reference_invalid",
        ),
        "tunnel_api_key_reference_plaintext_rejected" => (
            SetupField::TunnelSecretPath,
            "tunnel_api_key_reference_plaintext_rejected",
        ),
        "hub_url_invalid" => (SetupField::HubUrl, "hub_url_invalid"),
        "hub_transport_invalid" => (SetupField::HubTransport, "hub_transport_invalid"),
        "agent_id_required" => (SetupField::AgentId, "agent_id_required"),
        "agent_secret_required" => (SetupField::AgentSecret, "agent_secret_required"),
        "tunnel_download_url_must_use_https" => (
            SetupField::TunnelDownloadUrl,
            "tunnel_download_url_must_use_https",
        ),
        "tunnel_download_sha256_required" => {
            (SetupField::TunnelSha256, "tunnel_download_sha256_required")
        }
        "tunnel_sha256_invalid" => (SetupField::TunnelSha256, "tunnel_sha256_invalid"),
        "room_config_requires_room_profile" => {
            (SetupField::Profile, "room_config_requires_room_profile")
        }
        _ => (
            match session.selected_mode() {
                RuntimeMode::Standalone => SetupField::TunnelId,
                RuntimeMode::Hub => SetupField::HubUrl,
                RuntimeMode::Local => SetupField::Mode,
            },
            "config_init_build_invalid",
        ),
    };
    error(field, safe_code)
}

fn first_field(section: OptionalSection) -> SetupField {
    match section {
        OptionalSection::Identity => SetupField::DisplayName,
        OptionalSection::Workspace => SetupField::WorkspaceRoot,
        OptionalSection::Confirmation => SetupField::ConfirmationProvider,
        OptionalSection::Limits => SetupField::MaxConcurrentTasks,
        OptionalSection::Sandbox => SetupField::SandboxEnabled,
        OptionalSection::McpServers => SetupField::McpServerId,
        OptionalSection::Room => SetupField::RoomTimezone,
        OptionalSection::TunnelClient => SetupField::TunnelClientVersion,
        OptionalSection::HubReporting => SetupField::HubReportingDetail,
    }
}
