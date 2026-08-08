use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::cli_i18n::UiLanguage;
use crate::config_setup::{
    OptionalSectionDraft, ReviewModel, ReviewTarget, SectionStatus, SetupField, SetupSession,
};
use crate::config_templates::{InitSummary, OptionalSection, RuntimeMode, TunnelSecretSource};
use crate::tui::{
    render_action_button, render_footer, render_header, render_inline_error, render_radio_row,
    render_text_input, Theme,
};

use super::{ConfigPage, SystemError, TuiState};

pub(super) fn render(
    frame: &mut Frame,
    page: ConfigPage,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    section_draft: Option<&OptionalSectionDraft>,
    review: Option<&ReviewModel>,
    progress: (usize, usize),
) {
    match page {
        ConfigPage::Basic => render_basic(frame, session, state, theme, errors, progress),
        ConfigPage::Connection => render_connection(frame, session, state, theme, errors, progress),
        ConfigPage::OptionalCenter => {
            render_optional_center(frame, session, state, theme, progress)
        }
        ConfigPage::Optional(section) => render_optional_form(
            frame,
            section,
            session,
            state,
            theme,
            errors,
            section_draft,
            progress,
        ),
        ConfigPage::Review => {
            if let Some(review) = review {
                render_review(frame, review, state, theme, progress)
            } else {
                render_placeholder(frame, "Review", state, theme, progress)
            }
        }
        ConfigPage::Completion => render_placeholder(frame, "Done", state, theme, progress),
        ConfigPage::SystemError => render_placeholder(frame, "Error", state, theme, progress),
    }
}

fn render_basic(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    progress: (usize, usize),
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        "AgenticGPT config init",
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    let mode = session.selected_mode();
    let profile = session.selected_profile();
    let [mode_row, profile_row, error_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(body);
    render_radio_row(
        frame,
        mode_row,
        &format!("Runtime mode  {mode:?}"),
        true,
        state.focus == 0,
        theme,
    );
    render_radio_row(
        frame,
        profile_row,
        &format!("Profile       {}", format!("{profile:?}").to_lowercase()),
        true,
        state.focus == 1,
        theme,
    );
    if let Some(error) = errors.get(&SetupField::Mode) {
        render_inline_error(frame, error_row, error, theme);
    }
    render_action_button(frame, actions, "Next", state.focus >= 2, theme);
    render_footer(
        frame,
        footer,
        "Enter confirm · Tab move · Ctrl+C cancel",
        theme,
    );
}

fn render_connection(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    progress: (usize, usize),
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(6),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        "Connection settings",
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    let fields = connection_fields_for_session(session);
    let row_height = 1u16;
    let constraints = std::iter::repeat_n(Constraint::Length(row_height), fields.len())
        .chain(std::iter::once(Constraint::Min(1)))
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(body);
    for (index, field) in fields.iter().enumerate() {
        let focused = state.focus == index;
        let value = connection_value(session, *field);
        if matches!(
            field,
            SetupField::TunnelSecretSource | SetupField::ProvisionTunnelSecret
        ) {
            render_radio_row(
                frame,
                rows[index],
                &connection_label(*field, value.as_deref()),
                value.as_deref() == Some("true"),
                focused,
                theme,
            );
        } else {
            render_text_input(
                frame,
                rows[index],
                &connection_label(*field, value.as_deref()),
                value.as_deref().unwrap_or_default(),
                focused,
                matches!(
                    field,
                    SetupField::TunnelSecretValue | SetupField::AgentSecret
                ),
                theme,
            );
        }
    }
    if let Some((_, error)) = errors.iter().next() {
        render_inline_error(frame, *rows.last().unwrap_or(&body), error, theme);
    }
    render_action_button(frame, actions, "Next", state.focus >= fields.len(), theme);
    render_footer(
        frame,
        footer,
        "Enter edit · Esc back · Ctrl+C cancel",
        theme,
    );
}

pub(super) fn connection_fields_for_session(session: &SetupSession) -> Vec<SetupField> {
    match session.selected_mode() {
        RuntimeMode::Standalone => {
            let mut fields = vec![SetupField::TunnelId, SetupField::TunnelSecretSource];
            match session.standalone().secret_source {
                TunnelSecretSource::File => {
                    fields.push(SetupField::TunnelSecretPath);
                    fields.push(SetupField::ProvisionTunnelSecret);
                    if session.standalone().provision_secret_now {
                        fields.push(SetupField::TunnelSecretValue);
                    }
                }
                TunnelSecretSource::Environment => {
                    fields.push(SetupField::TunnelSecretEnvironment);
                }
            }
            fields
        }
        RuntimeMode::Hub => vec![
            SetupField::HubUrl,
            SetupField::HubTransport,
            SetupField::AgentId,
            SetupField::AgentSecret,
        ],
        RuntimeMode::Local => Vec::new(),
    }
}

fn connection_label(field: SetupField, value: Option<&str>) -> String {
    match field {
        SetupField::TunnelId => "Tunnel ID".to_string(),
        SetupField::TunnelSecretSource => format!("Secret source: {}", value.unwrap_or("file")),
        SetupField::TunnelSecretPath => "Secret file".to_string(),
        SetupField::TunnelSecretEnvironment => "Secret environment".to_string(),
        SetupField::ProvisionTunnelSecret => "Provision secret now".to_string(),
        SetupField::TunnelSecretValue => "Secret value".to_string(),
        SetupField::HubUrl => "Hub URL".to_string(),
        SetupField::HubTransport => "Transport".to_string(),
        SetupField::AgentId => "Agent ID".to_string(),
        SetupField::AgentSecret => "Agent Secret".to_string(),
        _ => format!("{field:?}"),
    }
}

pub(super) fn connection_value(session: &SetupSession, field: SetupField) -> Option<String> {
    match field {
        SetupField::TunnelId => Some(session.standalone().tunnel_id.clone()),
        SetupField::TunnelSecretSource => Some(match session.standalone().secret_source {
            TunnelSecretSource::File => "file".to_string(),
            TunnelSecretSource::Environment => "env".to_string(),
        }),
        SetupField::TunnelSecretPath => Some(session.standalone().secret_path.clone()),
        SetupField::TunnelSecretEnvironment => {
            Some(session.standalone().secret_environment.clone())
        }
        SetupField::ProvisionTunnelSecret => {
            Some(session.standalone().provision_secret_now.to_string())
        }
        SetupField::TunnelSecretValue => session
            .standalone()
            .secret_value
            .as_ref()
            .map(|secret| secret.expose().to_string()),
        SetupField::HubUrl => Some(session.hub().hub_url.clone()),
        SetupField::HubTransport => Some(session.hub().hub_transport.clone()),
        SetupField::AgentId => Some(session.hub().agent_id.clone()),
        SetupField::AgentSecret => session
            .hub()
            .agent_secret
            .as_ref()
            .map(|secret| secret.expose().to_string()),
        _ => None,
    }
}

fn render_optional_center(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    progress: (usize, usize),
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(8),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        "Optional settings",
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );

    let sections = all_optional_sections();
    let rows =
        Layout::vertical(std::iter::repeat_n(Constraint::Length(1), sections.len())).split(body);
    let focusable = session.available_optional_sections();
    for (index, section) in sections.iter().enumerate() {
        let status = session.section_status(*section);
        let applicable_index = focusable.iter().position(|candidate| candidate == section);
        let focused = applicable_index == Some(state.focus);
        let style = match status {
            SectionStatus::NotApplicable => theme.disabled,
            _ if focused => theme.focus,
            _ => theme.normal,
        };
        let prefix = if focused { "› " } else { "  " };
        let status_label = match status {
            SectionStatus::Default => "Default",
            SectionStatus::Configured => "Configured",
            SectionStatus::NotApplicable => "Not applicable",
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(section_label(*section), style),
                Span::styled(format!("  [{status_label}]"), style),
            ])),
            rows[index],
        );
    }
    render_action_button(
        frame,
        actions,
        "Finish and continue",
        state.focus >= focusable.len(),
        theme,
    );
    render_footer(
        frame,
        footer,
        "Enter open/save · Tab move · Esc back · Ctrl+C cancel",
        theme,
    );
}

fn render_optional_form(
    frame: &mut Frame,
    section: OptionalSection,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    section_draft: Option<&OptionalSectionDraft>,
    progress: (usize, usize),
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        &format!("{} settings", section_label(section)),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );

    let fallback = session.optional_draft(section);
    let draft = section_draft.unwrap_or(&fallback);
    let fields = optional_fields(section);
    let constraints = std::iter::repeat_n(Constraint::Length(1), fields.len())
        .chain(std::iter::once(Constraint::Min(1)))
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(body);
    for (index, field) in fields.iter().enumerate() {
        let value = optional_field_value(draft, *field);
        let focused = state.focus == index;
        if optional_field_is_toggle(*field) {
            render_radio_row(
                frame,
                rows[index],
                &format!("{}  {}", optional_field_label(*field), value),
                value == "true",
                focused,
                theme,
            );
        } else {
            render_text_input(
                frame,
                rows[index],
                optional_field_label(*field),
                &value,
                focused,
                false,
                theme,
            );
        }
    }
    if let Some((_, error)) = errors.iter().next() {
        render_inline_error(frame, *rows.last().unwrap_or(&body), error, theme);
    }
    render_action_button(
        frame,
        actions,
        "Save and return",
        state.focus >= fields.len(),
        theme,
    );
    render_footer(
        frame,
        footer,
        "Enter edit/toggle · Esc discard · Ctrl+C cancel",
        theme,
    );
}

fn all_optional_sections() -> [OptionalSection; 8] {
    [
        OptionalSection::Identity,
        OptionalSection::Workspace,
        OptionalSection::Confirmation,
        OptionalSection::Limits,
        OptionalSection::Sandbox,
        OptionalSection::Room,
        OptionalSection::TunnelClient,
        OptionalSection::HubReporting,
    ]
}

pub(super) fn optional_fields(section: OptionalSection) -> Vec<SetupField> {
    match section {
        OptionalSection::Identity => vec![SetupField::DisplayName],
        OptionalSection::Workspace => vec![
            SetupField::WorkspaceRoot,
            SetupField::WriteRoots,
            SetupField::ReadOnlyRoots,
            SetupField::DenyRoots,
        ],
        OptionalSection::Confirmation => {
            vec![
                SetupField::ConfirmationProvider,
                SetupField::ConfirmationLanguage,
            ]
        }
        OptionalSection::Limits => vec![
            SetupField::MaxConcurrentTasks,
            SetupField::MaxActiveJobs,
            SetupField::MaxFileSearchContextLines,
        ],
        OptionalSection::Sandbox => vec![
            SetupField::SandboxEnabled,
            SetupField::BubblewrapPath,
            SetupField::RequiredRuntimePaths,
        ],
        OptionalSection::Room => vec![
            SetupField::RoomTimezone,
            SetupField::DiaryBoundaryHour,
            SetupField::NotebookRoot,
        ],
        OptionalSection::TunnelClient => vec![
            SetupField::TunnelClientVersion,
            SetupField::TunnelCacheDir,
            SetupField::TunnelAutoDownload,
            SetupField::TunnelExecutable,
            SetupField::TunnelDownloadUrl,
            SetupField::TunnelSha256,
        ],
        OptionalSection::HubReporting => vec![
            SetupField::HubReportingEnabled,
            SetupField::HubReportingDetail,
        ],
    }
}

fn section_label(section: OptionalSection) -> &'static str {
    match section {
        OptionalSection::Identity => "Identity",
        OptionalSection::Workspace => "Workspace",
        OptionalSection::Confirmation => "Confirmation",
        OptionalSection::Limits => "Limits",
        OptionalSection::Sandbox => "Sandbox",
        OptionalSection::Room => "Room",
        OptionalSection::TunnelClient => "Tunnel client",
        OptionalSection::HubReporting => "Hub reporting",
    }
}

fn optional_field_label(field: SetupField) -> &'static str {
    match field {
        SetupField::DisplayName => "Display name",
        SetupField::WorkspaceRoot => "Workspace root",
        SetupField::WriteRoots => "Write roots (JSON)",
        SetupField::ReadOnlyRoots => "Read-only roots (JSON)",
        SetupField::DenyRoots => "Deny roots (JSON)",
        SetupField::ConfirmationProvider => "Provider",
        SetupField::ConfirmationLanguage => "Language",
        SetupField::MaxConcurrentTasks => "Max concurrent tasks",
        SetupField::MaxActiveJobs => "Max active jobs",
        SetupField::MaxFileSearchContextLines => "File-search context lines",
        SetupField::SandboxEnabled => "Sandbox enabled",
        SetupField::BubblewrapPath => "Bubblewrap path",
        SetupField::RequiredRuntimePaths => "Required runtime paths (JSON)",
        SetupField::RoomTimezone => "Timezone",
        SetupField::DiaryBoundaryHour => "Diary boundary hour",
        SetupField::NotebookRoot => "Notebook root",
        SetupField::TunnelClientVersion => "Client version",
        SetupField::TunnelCacheDir => "Cache directory",
        SetupField::TunnelAutoDownload => "Auto-download",
        SetupField::TunnelExecutable => "Executable",
        SetupField::TunnelDownloadUrl => "Download URL",
        SetupField::TunnelSha256 => "SHA-256",
        SetupField::HubReportingEnabled => "Reporting enabled",
        SetupField::HubReportingDetail => "Reporting detail",
        _ => "Value",
    }
}

pub(super) fn optional_field_is_toggle(field: SetupField) -> bool {
    matches!(
        field,
        SetupField::SandboxEnabled
            | SetupField::TunnelAutoDownload
            | SetupField::HubReportingEnabled
    )
}

pub(super) fn optional_field_value(draft: &OptionalSectionDraft, field: SetupField) -> String {
    match draft {
        OptionalSectionDraft::Identity(value) => match field {
            SetupField::DisplayName => value.display_name.clone(),
            _ => String::new(),
        },
        OptionalSectionDraft::Workspace(value) => match field {
            SetupField::WorkspaceRoot => value.workspace_root.clone(),
            SetupField::WriteRoots => value.write_roots.clone(),
            SetupField::ReadOnlyRoots => value.read_only_roots.clone(),
            SetupField::DenyRoots => value.deny_roots.clone(),
            _ => String::new(),
        },
        OptionalSectionDraft::Confirmation(value) => match field {
            SetupField::ConfirmationProvider => value.provider.clone(),
            SetupField::ConfirmationLanguage => value.language.clone(),
            _ => String::new(),
        },
        OptionalSectionDraft::Limits(value) => match field {
            SetupField::MaxConcurrentTasks => value.max_concurrent_tasks.clone(),
            SetupField::MaxActiveJobs => value.max_active_jobs.clone(),
            SetupField::MaxFileSearchContextLines => value.max_file_search_context_lines.clone(),
            _ => String::new(),
        },
        OptionalSectionDraft::Sandbox(value) => match field {
            SetupField::SandboxEnabled => value.enabled.to_string(),
            SetupField::BubblewrapPath => value.bubblewrap_path.clone(),
            SetupField::RequiredRuntimePaths => value.required_runtime_paths.clone(),
            _ => String::new(),
        },
        OptionalSectionDraft::Room(value) => match field {
            SetupField::RoomTimezone => value.timezone.clone(),
            SetupField::DiaryBoundaryHour => value.diary_boundary_hour.clone(),
            SetupField::NotebookRoot => value.notebook_root.clone(),
            _ => String::new(),
        },
        OptionalSectionDraft::TunnelClient(value) => match field {
            SetupField::TunnelClientVersion => value.version.clone(),
            SetupField::TunnelCacheDir => value.cache_dir.clone(),
            SetupField::TunnelAutoDownload => value.auto_download.to_string(),
            SetupField::TunnelExecutable => value.executable.clone(),
            SetupField::TunnelDownloadUrl => value.download_url.clone(),
            SetupField::TunnelSha256 => value.sha256.clone(),
            _ => String::new(),
        },
        OptionalSectionDraft::HubReporting(value) => match field {
            SetupField::HubReportingEnabled => value.enabled.to_string(),
            SetupField::HubReportingDetail => value.detail.clone(),
            _ => String::new(),
        },
    }
}

pub(super) fn set_optional_field(
    draft: &mut OptionalSectionDraft,
    field: SetupField,
    value: String,
) {
    match draft {
        OptionalSectionDraft::Identity(draft) if field == SetupField::DisplayName => {
            draft.display_name = value
        }
        OptionalSectionDraft::Workspace(draft) => match field {
            SetupField::WorkspaceRoot => draft.workspace_root = value,
            SetupField::WriteRoots => draft.write_roots = value,
            SetupField::ReadOnlyRoots => draft.read_only_roots = value,
            SetupField::DenyRoots => draft.deny_roots = value,
            _ => {}
        },
        OptionalSectionDraft::Confirmation(draft) => match field {
            SetupField::ConfirmationProvider => draft.provider = value,
            SetupField::ConfirmationLanguage => draft.language = value,
            _ => {}
        },
        OptionalSectionDraft::Limits(draft) => match field {
            SetupField::MaxConcurrentTasks => draft.max_concurrent_tasks = value,
            SetupField::MaxActiveJobs => draft.max_active_jobs = value,
            SetupField::MaxFileSearchContextLines => draft.max_file_search_context_lines = value,
            _ => {}
        },
        OptionalSectionDraft::Sandbox(draft) => match field {
            SetupField::BubblewrapPath => draft.bubblewrap_path = value,
            SetupField::RequiredRuntimePaths => draft.required_runtime_paths = value,
            SetupField::SandboxEnabled => draft.enabled = value == "true",
            _ => {}
        },
        OptionalSectionDraft::Room(draft) => match field {
            SetupField::RoomTimezone => draft.timezone = value,
            SetupField::DiaryBoundaryHour => draft.diary_boundary_hour = value,
            SetupField::NotebookRoot => draft.notebook_root = value,
            _ => {}
        },
        OptionalSectionDraft::TunnelClient(draft) => match field {
            SetupField::TunnelClientVersion => draft.version = value,
            SetupField::TunnelCacheDir => draft.cache_dir = value,
            SetupField::TunnelAutoDownload => draft.auto_download = value == "true",
            SetupField::TunnelExecutable => draft.executable = value,
            SetupField::TunnelDownloadUrl => draft.download_url = value,
            SetupField::TunnelSha256 => draft.sha256 = value,
            _ => {}
        },
        OptionalSectionDraft::HubReporting(draft) => match field {
            SetupField::HubReportingEnabled => draft.enabled = value == "true",
            SetupField::HubReportingDetail => draft.detail = value,
            _ => {}
        },
        _ => {}
    }
}

pub(super) fn toggle_optional_field(draft: &mut OptionalSectionDraft, field: SetupField) {
    match draft {
        OptionalSectionDraft::Sandbox(draft) if field == SetupField::SandboxEnabled => {
            draft.enabled = !draft.enabled
        }
        OptionalSectionDraft::TunnelClient(draft) if field == SetupField::TunnelAutoDownload => {
            draft.auto_download = !draft.auto_download
        }
        OptionalSectionDraft::HubReporting(draft) if field == SetupField::HubReportingEnabled => {
            draft.enabled = !draft.enabled
        }
        _ => {}
    }
}

fn render_review(
    frame: &mut Frame,
    review: &ReviewModel,
    state: &TuiState,
    theme: &Theme,
    progress: (usize, usize),
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(8),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        "Review and write",
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );

    let groups = review_groups(review);
    let mut lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Config path: ", theme.dim),
            Span::styled(review.config_path.display().to_string(), theme.normal),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("Backup: ", theme.dim),
            Span::styled(
                if review.will_backup_existing_config {
                    "existing config will be backed up"
                } else {
                    "no existing config"
                },
                theme.normal,
            ),
        ]),
    ];
    if let Some(secret) = &review.secret_write {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Secret write: ", theme.dim),
            Span::styled(
                format!("yes ({}) · value hidden", secret.path.display()),
                theme.normal,
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Secret write: ", theme.dim),
            Span::styled("no", theme.normal),
        ]));
    }
    for action in &review.pending_actions {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Pending action: ", theme.dim),
            Span::styled(format!("{action:?}"), theme.warning),
        ]));
    }
    for (index, group) in groups.iter().enumerate() {
        let focused = state.focus == index;
        let style = if focused { theme.focus } else { theme.normal };
        let prefix = if focused { "› " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(review_target_label(group.target), style),
            Span::styled(
                format!("  [{}]", section_status_label(group.status)),
                theme.dim,
            ),
        ]));
        for item in &group.items {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(format!("{}: ", item.label_key), theme.dim),
                Span::styled(&item.value, theme.normal),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), body);
    render_action_button(
        frame,
        actions,
        "Confirm and write",
        state.focus >= groups.len(),
        theme,
    );
    render_footer(
        frame,
        footer,
        "Enter open/edit · Tab move · Esc back · Ctrl+C cancel",
        theme,
    );
}

fn review_groups(review: &ReviewModel) -> Vec<&crate::config_setup::ReviewGroup> {
    let mut groups = vec![&review.basic];
    if review.mode != RuntimeMode::Local {
        groups.push(&review.connection);
    }
    groups.extend(
        review
            .optional_sections
            .iter()
            .filter(|group| group.status != SectionStatus::NotApplicable),
    );
    groups
}

fn review_target_label(target: ReviewTarget) -> &'static str {
    match target {
        ReviewTarget::Basic => "Basic settings",
        ReviewTarget::Connection => "Connection settings",
        ReviewTarget::OptionalCenter => "Optional settings",
        ReviewTarget::OptionalSection(section) => section_label(section),
    }
}

fn section_status_label(status: SectionStatus) -> &'static str {
    match status {
        SectionStatus::Default => "Default",
        SectionStatus::Configured => "Configured",
        SectionStatus::NotApplicable => "Not applicable",
    }
}

pub(super) fn render_completion(
    frame: &mut Frame,
    summary: Option<&InitSummary>,
    _finished: bool,
    language: UiLanguage,
    theme: &Theme,
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let title = match language {
        UiLanguage::ZhCn => "AgenticGPT 初始化完成",
        UiLanguage::En => "AgenticGPT initialization complete",
    };
    render_header(frame, header, title, "", theme);
    let path = summary
        .map(|summary| summary.config_path.display().to_string())
        .unwrap_or_else(|| "(unknown)".to_string());
    let guidance = match language {
        UiLanguage::ZhCn => "下一步：运行 agentic-gpt config show 查看配置。",
        UiLanguage::En => "Next: run `agentic-gpt config show` to inspect the configuration.",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Config: ", theme.dim),
                Span::styled(path, theme.normal),
            ]),
            Line::from(guidance),
        ]),
        body,
    );
    let action = match language {
        UiLanguage::ZhCn => "完成",
        UiLanguage::En => "Done",
    };
    render_action_button(frame, actions, action, true, theme);
    render_footer(frame, footer, "Enter exit", theme);
}

pub(super) fn render_system_error(frame: &mut Frame, error: Option<&SystemError>, theme: &Theme) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(frame, header, "Initialization error", "", theme);
    let (code, message) = error
        .map(|error| (error.code, error.message))
        .unwrap_or(("config_init_system_error", "Initialization failed."));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Code: ", theme.dim),
                Span::styled(code, theme.error),
            ]),
            Line::from(Span::styled(message, theme.error)),
        ]),
        body,
    );
    render_action_button(frame, actions, "Exit", true, theme);
    render_footer(frame, footer, "Enter/Esc exit · Ctrl+C cancel", theme);
}

fn render_placeholder(
    frame: &mut Frame,
    title: &str,
    state: &TuiState,
    theme: &Theme,
    progress: (usize, usize),
) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        title,
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Next phase: ", theme.dim),
            Span::styled("staged navigation surface", theme.normal),
        ])),
        body,
    );
    render_footer(frame, footer, "Esc back · Ctrl+C cancel", theme);
    let _ = state;
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession};
    use crate::config_templates::RuntimeMode;
    use crate::WorkerProfile;

    use super::super::{ConfigPage, ConfigTuiApp};

    fn content(app: &ConfigTuiApp, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn basic_page_renders_mode_profile_and_footer() {
        let app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                profile: Some(WorkerProfile::Normal),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        let rendered = content(&app, 70, 20);
        assert!(rendered.contains("Runtime mode"));
        assert!(rendered.contains("Standalone"));
        assert!(rendered.contains("normal"));
        assert!(rendered.contains("Ctrl+C"));
    }

    #[test]
    fn progress_header_matches_dynamic_mode_flow() {
        let standalone = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        assert!(content(&standalone, 70, 20).contains("1 / 4"));

        let local = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Local),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        assert!(content(&local, 70, 20).contains("1 / 3"));

        let mut hub = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        hub.handle_action(super::super::TuiAction::Next).unwrap();
        assert!(content(&hub, 70, 20).contains("2 / 4"));
    }

    #[test]
    fn resize_rerender_preserves_page_and_staged_values() {
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_id: Some("resize-staged-tunnel".into()),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        let wide = content(&app, 70, 20);
        let narrow = content(&app, 48, 12);
        assert!(wide.contains("resize-staged-tunnel"));
        assert!(narrow.contains("resize-staged-tunnel"));
        assert_eq!(app.page(), ConfigPage::Connection);
        assert_eq!(app.session().standalone().tunnel_id, "resize-staged-tunnel");
    }

    #[test]
    fn optional_center_shows_all_sections_and_not_applicable_status() {
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        app.handle_action(super::super::TuiAction::Next).unwrap();
        let rendered = content(&app, 90, 20);
        assert!(rendered.contains("Optional settings"));
        assert!(rendered.contains("Identity"));
        assert!(rendered.contains("Workspace"));
        assert!(rendered.contains("Room  [Not applicable]"));
        assert!(rendered.contains("Tunnel client"));
        assert!(rendered.contains("Finish and continue"));
    }

    #[test]
    fn workspace_form_shows_path_policy_fields() {
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        app.handle_action(super::super::TuiAction::Next).unwrap();
        app.handle_action(super::super::TuiAction::MoveNext)
            .unwrap();
        app.handle_action(super::super::TuiAction::Activate)
            .unwrap();
        let rendered = content(&app, 100, 20);
        assert!(rendered.contains("Workspace settings"));
        assert!(rendered.contains("Workspace root"));
        assert!(rendered.contains("Write roots"));
        assert!(rendered.contains("Read-only roots"));
        assert!(rendered.contains("Deny roots"));
        assert!(rendered.contains("Save and return"));
    }

    #[test]
    fn connection_pages_are_conditional_and_secret_is_not_rendered() {
        let marker = "connection-secret-marker";
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                agent_secret: Some(crate::config_templates::SecretValue::new(marker)),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        let rendered = content(&app, 70, 20);
        assert!(rendered.contains("Hub URL"));
        assert!(rendered.contains("Agent Secret"));
        assert!(!rendered.contains(marker));

        let mut local = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Local),
                ..SetupSeed::default()
            },
            UiLanguage::ZhCn,
            "/tmp/config-tui-render.json".into(),
        ));
        local.handle_action(super::super::TuiAction::Next).unwrap();
        assert_ne!(local.page(), ConfigPage::Connection);
    }

    #[test]
    fn standalone_secret_value_is_only_visible_when_provisioning() {
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        assert!(!content(&app, 70, 20).contains("Secret value"));

        app.focus_field(crate::config_setup::SetupField::ProvisionTunnelSecret);
        app.handle_action(super::super::TuiAction::Activate)
            .unwrap();
        assert!(content(&app, 70, 20).contains("Secret value"));
    }
}
