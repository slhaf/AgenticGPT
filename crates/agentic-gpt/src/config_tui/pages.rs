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
use crate::config_templates::{
    InitSummary, OptionalSection, PendingAction, RuntimeMode, TunnelSecretSource,
};
use crate::tui::{
    render_action_button, render_footer, render_header, render_inline_error, render_radio_row,
    render_text_input_with_cursor, Theme,
};

use super::{ConfigPage, SystemError, TuiState};

fn t(language: UiLanguage, en: &'static str, zh: &'static str) -> &'static str {
    match language {
        UiLanguage::En => en,
        UiLanguage::ZhCn => zh,
    }
}

fn localized_error(code: &str, language: UiLanguage) -> String {
    let key = code.split(':').next().unwrap_or(code).trim();
    let (en, zh) = match key {
        "config_init_required_value_missing" => ("Required value is missing.", "必填项不能为空。"),
        "config_init_secret_empty" => ("Secret must not be empty.", "密钥不能为空。"),
        "config_init_secret_path_invalid" => ("Secret path is invalid.", "密钥路径无效。"),
        "config_init_secret_source_invalid" => ("Secret source is invalid.", "密钥来源无效。"),
        "config_init_number_invalid" => ("Enter a valid number.", "请输入有效数字。"),
        "config_init_path_policy_write_roots_invalid" => (
            "Write roots must be a JSON array.",
            "写入根目录必须是 JSON 数组。",
        ),
        "config_init_path_policy_read_only_roots_invalid" => (
            "Read-only roots must be a JSON array.",
            "只读根目录必须是 JSON 数组。",
        ),
        "config_init_path_policy_deny_roots_invalid" => (
            "Deny roots must be a JSON array.",
            "拒绝根目录必须是 JSON 数组。",
        ),
        "config_init_runtime_paths_invalid" => (
            "Required runtime paths must be a JSON array.",
            "运行时路径必须是 JSON 数组。",
        ),
        "config_init_confirmation_provider_invalid" => {
            ("Confirmation provider is invalid.", "确认提供方无效。")
        }
        "config_init_reporting_detail_invalid" => {
            ("Reporting detail is invalid.", "报告详细程度无效。")
        }
        "config_init_optional_section_invalid" => {
            ("Optional section is invalid.", "可选配置区块无效。")
        }
        "config_init_build_invalid" => ("Configuration could not be built.", "配置无法生成。"),
        _ => ("Input is invalid.", "输入值无效。"),
    };
    t(language, en, zh).to_string()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    frame: &mut Frame,
    page: ConfigPage,
    session: &SetupSession,
    state: &TuiState,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    section_draft: Option<&OptionalSectionDraft>,
    review: Option<&ReviewModel>,
    progress: (usize, usize),
) {
    match page {
        ConfigPage::Basic => render_basic(frame, session, state, language, theme, errors, progress),
        ConfigPage::Connection => {
            render_connection(frame, session, state, language, theme, errors, progress)
        }
        ConfigPage::OptionalCenter => {
            render_optional_center(frame, session, state, language, theme, progress)
        }
        ConfigPage::Optional(section) => render_optional_form(
            frame,
            section,
            session,
            state,
            language,
            theme,
            errors,
            section_draft,
            progress,
        ),
        ConfigPage::Review => {
            if let Some(review) = review {
                render_review(frame, review, state, language, theme, progress)
            } else {
                render_placeholder(
                    frame,
                    t(language, "Review", "检查与写入"),
                    state,
                    language,
                    theme,
                    progress,
                )
            }
        }
        ConfigPage::Completion => render_placeholder(
            frame,
            t(language, "Done", "完成"),
            state,
            language,
            theme,
            progress,
        ),
        ConfigPage::SystemError => render_system_error(frame, None, language, theme),
    }
}

fn render_basic(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    language: UiLanguage,
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
        t(language, "AgenticGPT config init", "AgenticGPT 配置初始化"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    let mode = session.selected_mode();
    let profile = session.selected_profile();
    let [mode_row, mode_hint, profile_row, profile_hint, error_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(body);
    render_radio_row(
        frame,
        mode_row,
        &format!("{}  {mode:?}", t(language, "Runtime mode", "运行模式")),
        true,
        state.focus == 0,
        theme,
    );
    frame.render_widget(
        ratatui::widgets::Paragraph::new(t(
            language,
            "  Choices: Standalone / Hub / Local",
            "  可选：Standalone / Hub / Local",
        ))
        .style(theme.dim),
        mode_hint,
    );
    render_radio_row(
        frame,
        profile_row,
        &format!(
            "{}       {}",
            t(language, "Profile", "能力配置"),
            format!("{profile:?}").to_lowercase()
        ),
        true,
        state.focus == 1,
        theme,
    );
    frame.render_widget(
        ratatui::widgets::Paragraph::new(t(
            language,
            "  Choices: normal / Room",
            "  可选：normal / Room",
        ))
        .style(theme.dim),
        profile_hint,
    );
    if let Some(error) = errors.get(&SetupField::Mode) {
        let message = localized_error(error, language);
        render_inline_error(frame, error_row, &message, theme);
    }
    render_action_button(
        frame,
        actions,
        t(language, "Next", "下一步"),
        state.focus >= 2,
        theme,
    );
    render_footer(
        frame,
        footer,
        t(
            language,
            "Enter choose · Tab move · Ctrl+C cancel",
            "Enter 选择 · Tab 移动 · Ctrl+C 取消",
        ),
        theme,
    );
}

fn render_connection(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    language: UiLanguage,
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
        t(language, "Connection settings", "连接设置"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    let fields = connection_fields_for_session(session);
    let constraints = fields
        .iter()
        .flat_map(|field| {
            std::iter::once(Constraint::Length(1))
                .chain(errors.contains_key(field).then_some(Constraint::Length(1)))
        })
        .chain(std::iter::once(Constraint::Min(1)))
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(body);
    let mut row_index = 0;
    for (index, field) in fields.iter().enumerate() {
        let focused = state.focus == index;
        let confirmed_value = connection_value(session, *field);
        let value = confirmed_value.as_deref().unwrap_or_default();
        if matches!(
            field,
            SetupField::TunnelSecretSource | SetupField::ProvisionTunnelSecret
        ) {
            render_radio_row(
                frame,
                rows[row_index],
                &connection_label(*field, Some(value), language),
                match field {
                    SetupField::TunnelSecretSource => true,
                    SetupField::ProvisionTunnelSecret => value == "true",
                    _ => false,
                },
                focused,
                theme,
            );
        } else {
            render_text_input_with_cursor(
                frame,
                rows[row_index],
                &connection_label(*field, Some(value), language),
                current_input_value(state, *field, value),
                focused,
                matches!(
                    field,
                    SetupField::TunnelSecretValue | SetupField::AgentSecret
                ),
                editing_cursor(state, *field),
                theme,
            );
        }
        row_index += 1;
        if let Some(error) = errors.get(field) {
            let message = localized_error(error, language);
            render_inline_error(frame, rows[row_index], &message, theme);
            row_index += 1;
        }
    }
    render_action_button(
        frame,
        actions,
        t(language, "Next", "下一步"),
        state.focus >= fields.len(),
        theme,
    );
    render_footer(
        frame,
        footer,
        t(
            language,
            "Enter edit · Esc back · Ctrl+C cancel",
            "Enter 编辑 · Esc 返回 · Ctrl+C 取消",
        ),
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

fn connection_label(field: SetupField, value: Option<&str>, language: UiLanguage) -> String {
    match field {
        SetupField::TunnelId => t(language, "Tunnel ID", "隧道 ID").to_string(),
        SetupField::TunnelSecretSource => format!(
            "{}: {}",
            t(language, "Secret source", "密钥来源"),
            value.unwrap_or("file")
        ),
        SetupField::TunnelSecretPath => t(language, "Secret file", "密钥文件").to_string(),
        SetupField::TunnelSecretEnvironment => {
            t(language, "Secret environment", "密钥环境变量").to_string()
        }
        SetupField::ProvisionTunnelSecret => {
            t(language, "Provision secret now", "立即写入密钥").to_string()
        }
        SetupField::TunnelSecretValue => t(language, "Secret value", "密钥值").to_string(),
        SetupField::HubUrl => t(language, "Hub URL", "Hub 地址").to_string(),
        SetupField::HubTransport => t(language, "Transport", "传输方式").to_string(),
        SetupField::AgentId => t(language, "Agent ID", "代理 ID").to_string(),
        SetupField::AgentSecret => t(language, "Agent Secret", "代理密钥").to_string(),
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
    language: UiLanguage,
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
        t(language, "Optional settings", "可选配置"),
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
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(section_label(*section, language), style),
                Span::styled(
                    format!("  [{}]", section_status_label(status, language)),
                    style,
                ),
            ])),
            rows[index],
        );
    }
    render_action_button(
        frame,
        actions,
        t(language, "Finish and continue", "完成并继续"),
        state.focus >= focusable.len(),
        theme,
    );
    render_footer(
        frame,
        footer,
        t(
            language,
            "Enter open/save · Tab move · Esc back · Ctrl+C cancel",
            "Enter 打开/保存 · Tab 移动 · Esc 返回 · Ctrl+C 取消",
        ),
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_optional_form(
    frame: &mut Frame,
    section: OptionalSection,
    session: &SetupSession,
    state: &TuiState,
    language: UiLanguage,
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
        &format!(
            "{} {}",
            section_label(section, language),
            t(language, "settings", "配置")
        ),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );

    let fallback = session.optional_draft(section);
    let draft = section_draft.unwrap_or(&fallback);
    let fields = optional_fields(section);
    let constraints = fields
        .iter()
        .flat_map(|field| {
            std::iter::once(Constraint::Length(1))
                .chain(errors.contains_key(field).then_some(Constraint::Length(1)))
        })
        .chain(std::iter::once(Constraint::Min(1)))
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(body);
    let mut row_index = 0;
    for (index, field) in fields.iter().enumerate() {
        let value = optional_field_value(draft, *field);
        let value = current_input_value(state, *field, &value);
        let focused = state.focus == index;
        if optional_field_is_toggle(*field) {
            render_radio_row(
                frame,
                rows[row_index],
                &format!("{}  {}", optional_field_label(*field, language), value),
                value == "true",
                focused,
                theme,
            );
        } else {
            render_text_input_with_cursor(
                frame,
                rows[row_index],
                optional_field_label(*field, language),
                value,
                focused,
                false,
                editing_cursor(state, *field),
                theme,
            );
        }
        row_index += 1;
        if let Some(error) = errors.get(field) {
            let message = localized_error(error, language);
            render_inline_error(frame, rows[row_index], &message, theme);
            row_index += 1;
        }
    }
    render_action_button(
        frame,
        actions,
        t(language, "Save and return", "保存并返回"),
        state.focus >= fields.len(),
        theme,
    );
    render_footer(
        frame,
        footer,
        t(
            language,
            "Enter edit/toggle · Esc discard · Ctrl+C cancel",
            "Enter 编辑/切换 · Esc 放弃 · Ctrl+C 取消",
        ),
        theme,
    );
}

fn current_input_value<'a>(state: &'a TuiState, field: SetupField, confirmed: &'a str) -> &'a str {
    state
        .editing
        .as_ref()
        .filter(|editing| editing.field == field)
        .map(|editing| editing.buffer.as_str())
        .unwrap_or(confirmed)
}

fn editing_cursor(state: &TuiState, field: SetupField) -> Option<usize> {
    state
        .editing
        .as_ref()
        .filter(|editing| editing.field == field)
        .map(|editing| editing.cursor)
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

fn section_label(section: OptionalSection, language: UiLanguage) -> &'static str {
    match section {
        OptionalSection::Identity => t(language, "Identity", "身份"),
        OptionalSection::Workspace => t(language, "Workspace", "工作区"),
        OptionalSection::Confirmation => t(language, "Confirmation", "确认"),
        OptionalSection::Limits => t(language, "Limits", "限制"),
        OptionalSection::Sandbox => t(language, "Sandbox", "沙箱"),
        OptionalSection::Room => t(language, "Room", "Room"),
        OptionalSection::TunnelClient => t(language, "Tunnel client", "隧道客户端"),
        OptionalSection::HubReporting => t(language, "Hub reporting", "Hub 报告"),
    }
}

fn optional_field_label(field: SetupField, language: UiLanguage) -> &'static str {
    match field {
        SetupField::DisplayName => t(language, "Display name", "显示名称"),
        SetupField::WorkspaceRoot => t(language, "Workspace root", "工作区根目录"),
        SetupField::WriteRoots => t(language, "Write roots (JSON)", "写入根目录（JSON）"),
        SetupField::ReadOnlyRoots => t(language, "Read-only roots (JSON)", "只读根目录（JSON）"),
        SetupField::DenyRoots => t(language, "Deny roots (JSON)", "拒绝根目录（JSON）"),
        SetupField::ConfirmationProvider => t(language, "Provider", "提供方"),
        SetupField::ConfirmationLanguage => t(language, "Language", "语言"),
        SetupField::MaxConcurrentTasks => t(language, "Max concurrent tasks", "最大并发任务"),
        SetupField::MaxActiveJobs => t(language, "Max active jobs", "最大活动作业"),
        SetupField::MaxFileSearchContextLines => {
            t(language, "File-search context lines", "文件搜索上下文行数")
        }
        SetupField::SandboxEnabled => t(language, "Sandbox enabled", "启用沙箱"),
        SetupField::BubblewrapPath => t(language, "Bubblewrap path", "Bubblewrap 路径"),
        SetupField::RequiredRuntimePaths => t(
            language,
            "Required runtime paths (JSON)",
            "必需运行时路径（JSON）",
        ),
        SetupField::RoomTimezone => t(language, "Timezone", "时区"),
        SetupField::DiaryBoundaryHour => t(language, "Diary boundary hour", "日记边界小时"),
        SetupField::NotebookRoot => t(language, "Notebook root", "笔记本根目录"),
        SetupField::TunnelClientVersion => t(language, "Client version", "客户端版本"),
        SetupField::TunnelCacheDir => t(language, "Cache directory", "缓存目录"),
        SetupField::TunnelAutoDownload => t(language, "Auto-download", "自动下载"),
        SetupField::TunnelExecutable => t(language, "Executable", "可执行文件"),
        SetupField::TunnelDownloadUrl => t(language, "Download URL", "下载地址"),
        SetupField::TunnelSha256 => t(language, "SHA-256", "SHA-256"),
        SetupField::HubReportingEnabled => t(language, "Reporting enabled", "启用报告"),
        SetupField::HubReportingDetail => t(language, "Reporting detail", "报告详细程度"),
        _ => t(language, "Value", "值"),
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
    language: UiLanguage,
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
        t(language, "Review and write", "检查并写入"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );

    let groups = review_groups(review);
    let mut lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled(t(language, "Config path: ", "配置路径："), theme.dim),
            Span::styled(review.config_path.display().to_string(), theme.normal),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(t(language, "Backup: ", "备份："), theme.dim),
            Span::styled(
                if review.will_backup_existing_config {
                    t(
                        language,
                        "existing config will be backed up",
                        "将备份现有配置",
                    )
                } else {
                    t(language, "no existing config", "没有现有配置")
                },
                theme.normal,
            ),
        ]),
    ];
    if let Some(secret) = &review.secret_write {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(t(language, "Secret write: ", "密钥写入："), theme.dim),
            Span::styled(
                format!(
                    "{} ({}) · {}",
                    t(language, "yes", "是"),
                    secret.path.display(),
                    t(language, "value hidden", "值已隐藏")
                ),
                theme.normal,
            ),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(t(language, "Secret write: ", "密钥写入："), theme.dim),
            Span::styled(t(language, "no", "否"), theme.normal),
        ]));
    }
    for action in &review.pending_actions {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(t(language, "Pending action: ", "待处理动作："), theme.dim),
            Span::styled(pending_action_label(*action, language), theme.warning),
        ]));
    }
    let mut group_line_indices = Vec::with_capacity(groups.len());
    for (index, group) in groups.iter().enumerate() {
        group_line_indices.push(lines.len());
        let focused = state.focus == index;
        let style = if focused { theme.focus } else { theme.normal };
        let prefix = if focused { "› " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(review_target_label(group.target, language), style),
            Span::styled(
                format!("  [{}]", section_status_label(group.status, language)),
                theme.dim,
            ),
        ]));
        for item in &group.items {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("{}: ", review_item_label(item.label_key, language)),
                    theme.dim,
                ),
                Span::styled(&item.value, theme.normal),
            ]));
        }
    }
    let focused_line = group_line_indices
        .get(state.focus)
        .copied()
        .unwrap_or_else(|| lines.len().saturating_sub(1));
    let body_height = usize::from(body.height);
    let max_scroll = lines.len().saturating_sub(body_height);
    let scroll = focused_line
        .saturating_sub(body_height / 2)
        .min(max_scroll)
        .min(usize::from(u16::MAX)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), body);
    render_action_button(
        frame,
        actions,
        t(language, "Confirm and write", "确认并写入"),
        state.focus >= groups.len(),
        theme,
    );
    render_footer(
        frame,
        footer,
        t(
            language,
            "Enter open/edit · Tab move · Esc back · Ctrl+C cancel",
            "Enter 打开/编辑 · Tab 移动 · Esc 返回 · Ctrl+C 取消",
        ),
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

fn review_target_label(target: ReviewTarget, language: UiLanguage) -> &'static str {
    match target {
        ReviewTarget::Basic => t(language, "Basic settings", "基础设置"),
        ReviewTarget::Connection => t(language, "Connection settings", "连接设置"),
        ReviewTarget::OptionalCenter => t(language, "Optional settings", "可选配置"),
        ReviewTarget::OptionalSection(section) => section_label(section, language),
    }
}

fn section_status_label(status: SectionStatus, language: UiLanguage) -> &'static str {
    match status {
        SectionStatus::Default => t(language, "Default", "默认"),
        SectionStatus::Configured => t(language, "Configured", "已配置"),
        SectionStatus::NotApplicable => t(language, "Not applicable", "不适用"),
    }
}

fn pending_action_label(action: PendingAction, language: UiLanguage) -> &'static str {
    match action {
        PendingAction::ReplaceTunnelId => t(language, "Replace tunnel ID", "替换隧道 ID"),
        PendingAction::ProvisionTunnelSecret => {
            t(language, "Provision tunnel secret", "写入隧道密钥")
        }
        PendingAction::ConfigureHubUrl => t(language, "Configure Hub URL", "配置 Hub 地址"),
        PendingAction::ReplaceAgentSecret => t(language, "Replace agent secret", "替换代理密钥"),
    }
}

fn review_item_label(label_key: &str, language: UiLanguage) -> &'static str {
    match label_key {
        "mode" => t(language, "Mode", "模式"),
        "profile" => t(language, "Profile", "能力配置"),
        "tunnel_id" => t(language, "Tunnel ID", "隧道 ID"),
        "tunnel_secret_source" => t(language, "Secret source", "密钥来源"),
        "tunnel_secret_reference" => t(language, "Secret reference", "密钥引用"),
        "hub_url" => t(language, "Hub URL", "Hub 地址"),
        "hub_transport" => t(language, "Hub transport", "Hub 传输方式"),
        "agent_id" => t(language, "Agent ID", "代理 ID"),
        "agent_secret" => t(language, "Agent secret", "代理密钥"),
        "connection" => t(language, "Connection", "连接"),
        "display_name" => t(language, "Display name", "显示名称"),
        "workspace_root" => t(language, "Workspace root", "工作区根目录"),
        "write_roots" => t(language, "Write roots", "写入根目录"),
        "read_only_roots" => t(language, "Read-only roots", "只读根目录"),
        "deny_roots" => t(language, "Deny roots", "拒绝根目录"),
        "confirmation_provider" => t(language, "Provider", "提供方"),
        "confirmation_language" => t(language, "Language", "语言"),
        "max_concurrent_tasks" => t(language, "Max concurrent tasks", "最大并发任务"),
        "max_active_jobs" => t(language, "Max active jobs", "最大活动作业"),
        "max_file_search_context_lines" => {
            t(language, "File-search context lines", "文件搜索上下文行数")
        }
        "sandbox_enabled" => t(language, "Sandbox enabled", "启用沙箱"),
        "bubblewrap_path" => t(language, "Bubblewrap path", "Bubblewrap 路径"),
        "required_runtime_paths" => t(language, "Required runtime paths", "必需运行时路径"),
        "room_timezone" => t(language, "Timezone", "时区"),
        "diary_boundary_hour" => t(language, "Diary boundary hour", "日记边界小时"),
        "notebook_root" => t(language, "Notebook root", "笔记本根目录"),
        "tunnel_client_version" => t(language, "Client version", "客户端版本"),
        "tunnel_cache_dir" => t(language, "Cache directory", "缓存目录"),
        "tunnel_auto_download" => t(language, "Auto-download", "自动下载"),
        "tunnel_executable" => t(language, "Executable", "可执行文件"),
        "tunnel_download_url" => t(language, "Download URL", "下载地址"),
        "tunnel_sha256" => t(language, "SHA-256", "SHA-256"),
        "hub_reporting_enabled" => t(language, "Reporting enabled", "启用报告"),
        "hub_reporting_detail" => t(language, "Reporting detail", "报告详细程度"),
        _ => t(language, "Value", "值"),
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
                Span::styled(t(language, "Config: ", "配置："), theme.dim),
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
    render_footer(
        frame,
        footer,
        t(language, "Enter exit", "Enter 退出"),
        theme,
    );
}

pub(super) fn render_system_error(
    frame: &mut Frame,
    error: Option<&SystemError>,
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
    render_header(
        frame,
        header,
        t(language, "Initialization error", "初始化错误"),
        "",
        theme,
    );
    let (code, message) = error.map(|error| (error.code, error.message)).unwrap_or((
        "config_init_system_error",
        t(language, "Initialization failed.", "初始化失败。"),
    ));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(t(language, "Code: ", "代码："), theme.dim),
                Span::styled(code, theme.error),
            ]),
            Line::from(Span::styled(message, theme.error)),
        ]),
        body,
    );
    render_action_button(frame, actions, t(language, "Exit", "退出"), true, theme);
    render_footer(
        frame,
        footer,
        t(
            language,
            "Enter/Esc exit · Ctrl+C cancel",
            "Enter/Esc 退出 · Ctrl+C 取消",
        ),
        theme,
    );
}

fn render_placeholder(
    frame: &mut Frame,
    title: &str,
    state: &TuiState,
    language: UiLanguage,
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
            Span::styled(t(language, "Next phase: ", "下一阶段："), theme.dim),
            Span::styled(
                t(language, "staged navigation surface", "分阶段导航界面"),
                theme.normal,
            ),
        ])),
        body,
    );
    render_footer(
        frame,
        footer,
        t(
            language,
            "Esc back · Ctrl+C cancel",
            "Esc 返回 · Ctrl+C 取消",
        ),
        theme,
    );
    let _ = state;
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession};
    use crate::config_templates::RuntimeMode;
    use crate::config_tui::TuiAction;
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
        assert!(rendered.contains("Hub"));
        assert!(rendered.contains("Local"));
        assert!(rendered.contains("normal"));
        assert!(rendered.contains("Room"));
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

    #[test]
    fn critical_pages_keep_primary_actions_in_a_small_terminal() {
        for mode in [
            RuntimeMode::Standalone,
            RuntimeMode::Hub,
            RuntimeMode::Local,
        ] {
            let mut app = ConfigTuiApp::new(SetupSession::new(
                SetupSeed {
                    mode: Some(mode),
                    profile: Some(WorkerProfile::Normal),
                    tunnel_id: Some("small-terminal-tunnel".into()),
                    tunnel_api_key: Some("file:/tmp/small-terminal-secret".into()),
                    agent_secret: Some(crate::config_templates::SecretValue::new(
                        "small-terminal-agent-secret",
                    )),
                    ..SetupSeed::default()
                },
                UiLanguage::En,
                "/tmp/config-tui-small-terminal.json".into(),
            ));

            let basic = content(&app, 36, 12);
            assert!(basic.contains("Next"), "basic action missing for {mode:?}");

            app.handle_action(TuiAction::Next).unwrap();
            if app.page() == ConfigPage::Connection {
                let connection = content(&app, 36, 12);
                assert!(
                    connection.contains("Next"),
                    "connection action missing for {mode:?}"
                );
                app.handle_action(TuiAction::Next).unwrap();
            }

            assert_eq!(app.page(), ConfigPage::OptionalCenter);
            let optional = content(&app, 36, 12);
            assert!(optional.contains("Finish and continue"));
            app.handle_action(TuiAction::Next).unwrap();

            assert_eq!(app.page(), ConfigPage::Review);
            let review = content(&app, 36, 12);
            assert!(review.contains("Confirm and write"));
        }
    }
}
