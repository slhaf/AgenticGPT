use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
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
use crate::tui::forms::{
    boolean_row_line, choice_input_row_line, choice_row_line, editable_list_item_line,
    input_row_line, long_form_input_value_line, numeric_input_value_line, render_long_form_input,
    subsection_heading_line, EditableListState,
};
use crate::tui::{
    action_line, inline_error_line, labeled_heading_line, render_action_button,
    render_contextual_footer, render_footer, render_header, render_horizontal_rule,
    render_inspector, render_surface, render_surface_header, surface_choice_line,
    surface_status_line, Theme,
};
use crate::WorkerProfile;

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
    section_dirty: bool,
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
            section_dirty,
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
    let full = frame.area();
    let content = if full.width >= 60 && full.height >= 16 {
        full.inner(Margin {
            horizontal: 2,
            vertical: 1,
        })
    } else {
        full
    };
    let [header, top_rule, body, bottom_rule, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(10),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(content);
    render_surface_header(
        frame,
        header,
        t(language, "AgenticGPT config init", "AgenticGPT 配置初始化"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    render_horizontal_rule(frame, top_rule, theme);

    let [left, _, right] = Layout::horizontal([
        Constraint::Percentage(43),
        Constraint::Length(2),
        Constraint::Min(24),
    ])
    .areas(body);
    render_basic_controls(frame, left, session, state, language, theme, errors);
    render_surface(frame, right, theme);
    let inspector = right.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let (title, body) = basic_inspector_copy(state.focus, language);
    render_inspector(frame, inspector, title, body, theme);

    render_horizontal_rule(frame, bottom_rule, theme);
    render_contextual_footer(
        frame,
        footer,
        &[
            ("↑↓ j/k", t(language, "move", "移动")),
            ("Enter/l", t(language, "choose", "选择")),
            ("Esc/h", t(language, "back", "返回")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ],
        theme,
    );
}

const BASIC_MODE_FOCUS_COUNT: usize = 3;
const BASIC_PROFILE_FOCUS_START: usize = BASIC_MODE_FOCUS_COUNT;
const BASIC_PROFILE_FOCUS_COUNT: usize = 2;
const BASIC_ROOM_FOCUS: usize = BASIC_PROFILE_FOCUS_START + 1;
pub(super) const BASIC_NEXT_FOCUS: usize = BASIC_PROFILE_FOCUS_START + BASIC_PROFILE_FOCUS_COUNT;

pub(super) fn basic_focus_len() -> usize {
    BASIC_NEXT_FOCUS + 1
}

pub(super) fn basic_focus_field(focus: usize) -> Option<SetupField> {
    match focus {
        0..BASIC_MODE_FOCUS_COUNT => Some(SetupField::Mode),
        BASIC_PROFILE_FOCUS_START..BASIC_NEXT_FOCUS => Some(SetupField::Profile),
        _ => None,
    }
}

pub(super) fn basic_mode_for_focus(focus: usize) -> Option<RuntimeMode> {
    match focus {
        0 => Some(RuntimeMode::Standalone),
        1 => Some(RuntimeMode::Hub),
        2 => Some(RuntimeMode::Local),
        _ => None,
    }
}

pub(super) fn basic_profile_for_focus(focus: usize) -> Option<WorkerProfile> {
    match focus {
        BASIC_PROFILE_FOCUS_START => Some(WorkerProfile::Normal),
        BASIC_ROOM_FOCUS => Some(WorkerProfile::Room),
        _ => None,
    }
}

fn render_basic_controls(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    session: &SetupSession,
    state: &TuiState,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
) {
    let mut lines = vec![labeled_heading_line(
        t(language, "Runtime mode", "运行模式"),
        area.width,
        theme,
    )];
    lines.push(Line::raw(""));
    for (index, (mode, label)) in [
        (
            RuntimeMode::Standalone,
            t(language, "Standalone", "Standalone"),
        ),
        (RuntimeMode::Hub, t(language, "Hub", "Hub")),
        (RuntimeMode::Local, t(language, "Local", "Local")),
    ]
    .into_iter()
    .enumerate()
    {
        lines.push(surface_choice_line(
            label,
            session.selected_mode() == mode,
            state.focus == index,
            theme,
        ));
    }

    lines.push(Line::raw(""));
    lines.push(labeled_heading_line(
        t(language, "Profile", "能力配置"),
        area.width,
        theme,
    ));
    lines.push(Line::raw(""));
    for (index, (profile, label)) in [
        (WorkerProfile::Normal, t(language, "Normal", "Normal")),
        (WorkerProfile::Room, t(language, "Room", "Room")),
    ]
    .into_iter()
    .enumerate()
    {
        lines.push(surface_choice_line(
            label,
            session.selected_profile() == profile,
            state.focus == BASIC_PROFILE_FOCUS_START + index,
            theme,
        ));
    }

    if let Some(error) = errors.get(&SetupField::Mode) {
        lines.push(inline_error_line(&localized_error(error, language), theme));
    }
    if let Some(error) = errors.get(&SetupField::Profile) {
        lines.push(inline_error_line(&localized_error(error, language), theme));
    }

    let target_next_y = area.height.saturating_sub(1) as usize;
    if lines.len() > target_next_y {
        lines.truncate(target_next_y);
    } else {
        lines.extend((0..target_next_y.saturating_sub(lines.len())).map(|_| Line::raw("")));
    }
    lines.push(action_line(
        t(language, "Next", "下一步"),
        state.focus == BASIC_NEXT_FOCUS,
        theme,
    ));
    frame.render_widget(Paragraph::new(lines), area);
}

fn basic_inspector_copy(
    focus: usize,
    language: UiLanguage,
) -> (&'static str, &'static [&'static str]) {
    match focus {
        0 => (
            t(language, "Standalone", "Standalone"),
            match language {
                UiLanguage::En => &[
                    "Agent runs independently and exposes capabilities through the Tunnel.",
                    "Suitable for a resident Agent with remote access.",
                ],
                UiLanguage::ZhCn => &[
                    "Agent 独立运行，并通过 Tunnel 暴露能力。",
                    "适合：单机常驻 Agent 与远程接入。",
                ],
            },
        ),
        1 => (
            t(language, "Hub", "Hub"),
            match language {
                UiLanguage::En => &[
                    "Connect to a remote AgenticGPT Hub for connection management and dispatch.",
                    "Suitable for centrally managing multiple Agents.",
                ],
                UiLanguage::ZhCn => &[
                    "连接远程 AgenticGPT Hub，由 Hub 管理连接与调度。",
                    "适合：集中管理多个 Agent。",
                ],
            },
        ),
        2 => (
            t(language, "Local", "Local"),
            match language {
                UiLanguage::En => &[
                    "Provide MCP capabilities locally without Hub or Tunnel.",
                    "Suitable for local development and personal use.",
                ],
                UiLanguage::ZhCn => &[
                    "仅在本机提供 MCP 能力，不连接 Hub 或 Tunnel。",
                    "适合：本地开发与个人使用。",
                ],
            },
        ),
        3 => (
            t(language, "Normal", "Normal"),
            match language {
                UiLanguage::En => &[
                    "Enable the general Agent capability set.",
                    "Keep the configuration and runtime surface compact.",
                ],
                UiLanguage::ZhCn => &["启用通用 Agent 能力集。", "保持配置和运行面最精简。"],
            },
        ),
        4 => (
            t(language, "Room", "Room"),
            match language {
                UiLanguage::En => &[
                    "Enable Room capabilities on top of the Normal profile.",
                    "Includes Diary, Notebook, and other long-lived context features.",
                ],
                UiLanguage::ZhCn => &[
                    "在 Normal 基础上启用 Room 能力。",
                    "包括 Diary、Notebook 等长期上下文功能。",
                ],
            },
        ),
        _ => (
            t(language, "Next", "下一步"),
            match language {
                UiLanguage::En => &[
                    "Confirm the committed choices and continue to connection settings.",
                    "All values remain in memory until the final commit.",
                ],
                UiLanguage::ZhCn => &[
                    "确认已提交的选择并进入连接设置。",
                    "所有值仍只保存在内存中。",
                ],
            },
        ),
    }
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
    let [header, top_rule, body, bottom_rule, footer] = surface_shell_areas(frame.area());
    render_surface_header(
        frame,
        header,
        t(language, "AgenticGPT config init", "AgenticGPT 配置初始化"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    render_horizontal_rule(frame, top_rule, theme);
    let [left, _, right] = surface_columns(body);

    let items = connection_focus_items(session);
    let mut cursor = left.y;
    render_surface_heading(
        frame,
        left,
        &mut cursor,
        t(language, "Connection", "连接"),
        theme,
    );
    render_surface_blank(left, &mut cursor, frame);

    let mut index = 0usize;
    while index < items.len() {
        match items[index] {
            ConnectionFocusItem::SecretSource(TunnelSecretSource::File) => {
                if let Some(row) = next_surface_row(left, &mut cursor) {
                    frame.render_widget(
                        Paragraph::new(subsection_heading_line(
                            &connection_label(SetupField::TunnelSecretSource, None, language),
                            theme,
                        )),
                        row,
                    );
                }
                for (offset, (source, label)) in [
                    (TunnelSecretSource::File, "file"),
                    (TunnelSecretSource::Environment, "env"),
                ]
                .into_iter()
                .enumerate()
                {
                    if let Some(row) = next_surface_row(left, &mut cursor) {
                        frame.render_widget(
                            Paragraph::new(choice_row_line(
                                label,
                                state.focus == index + offset,
                                session.standalone().secret_source == source,
                                12,
                                theme,
                            )),
                            row,
                        );
                    }
                }
                if let Some(error) = errors.get(&SetupField::TunnelSecretSource) {
                    if let Some(row) = next_surface_row(left, &mut cursor) {
                        frame.render_widget(
                            Paragraph::new(inline_error_line(
                                &localized_error(error, language),
                                theme,
                            )),
                            row,
                        );
                    }
                }
                render_surface_blank(left, &mut cursor, frame);
                index += 2;
                continue;
            }
            ConnectionFocusItem::SecretSource(TunnelSecretSource::Environment) => {
                index += 1;
                continue;
            }
            ConnectionFocusItem::Field(field) => {
                let focused = state.focus == index;
                let confirmed_value = connection_value(session, field);
                let value = confirmed_value.as_deref().unwrap_or_default();
                let label = connection_label(field, None, language);
                if field == SetupField::ProvisionTunnelSecret {
                    if let Some(row) = next_surface_row(left, &mut cursor) {
                        frame.render_widget(
                            Paragraph::new(boolean_row_line(
                                &label,
                                if value == "true" {
                                    t(language, "on", "开")
                                } else {
                                    t(language, "off", "关")
                                },
                                focused,
                                24,
                                theme,
                            )),
                            row,
                        );
                    }
                } else if let Some(field_area) = next_surface_rows(left, &mut cursor, 2) {
                    let edit_cursor = editing_cursor(state, field);
                    render_long_form_input(
                        frame,
                        field_area,
                        &label,
                        current_input_value(state, field, value),
                        focused,
                        edit_cursor.is_some(),
                        edit_cursor,
                        matches!(
                            field,
                            SetupField::TunnelSecretValue | SetupField::AgentSecret
                        ),
                        false,
                        theme,
                    );
                }
                if let Some(error) = errors.get(&field) {
                    if let Some(row) = next_surface_row(left, &mut cursor) {
                        frame.render_widget(
                            Paragraph::new(inline_error_line(
                                &localized_error(error, language),
                                theme,
                            )),
                            row,
                        );
                    }
                }
            }
        }
        index += 1;
    }

    render_surface_action(
        frame,
        left,
        t(language, "Next", "下一步"),
        state.focus >= items.len(),
        theme,
    );

    let field = connection_focus_field(session, state.focus);
    if right.width > 0 {
        render_surface(frame, right, theme);
        let inspector = right.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let title = field
            .map(|field| connection_label(field, None, language))
            .unwrap_or_else(|| t(language, "Next", "下一步").to_string());
        let inspector_body = connection_inspector_body(field, language);
        render_inspector(frame, inspector, &title, inspector_body, theme);
    }
    render_horizontal_rule(frame, bottom_rule, theme);
    render_connection_footer(
        frame,
        footer,
        state,
        session,
        field,
        state.focus >= items.len(),
        language,
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_connection_footer(
    frame: &mut Frame,
    area: Rect,
    state: &TuiState,
    session: &SetupSession,
    field: Option<SetupField>,
    action_focused: bool,
    language: UiLanguage,
    theme: &Theme,
) {
    if state.editing.is_some() {
        render_contextual_footer(
            frame,
            area,
            &[
                ("Enter", t(language, "confirm", "确认")),
                ("Esc", t(language, "discard", "放弃")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            theme,
        );
        return;
    }
    let action = if action_focused {
        t(language, "continue", "继续")
    } else if connection_secret_source_for_focus(session, state.focus).is_some() {
        t(language, "choose", "选择")
    } else if field == Some(SetupField::ProvisionTunnelSecret) {
        t(language, "toggle", "切换")
    } else {
        t(language, "edit", "编辑")
    };
    render_contextual_footer(
        frame,
        area,
        &[
            ("↑↓ j/k", t(language, "move", "移动")),
            ("Enter/l", action),
            ("Esc/h", t(language, "back", "返回")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ],
        theme,
    );
}

fn surface_shell_areas(area: Rect) -> [Rect; 5] {
    let content = if area.width >= 60 && area.height >= 16 {
        area.inner(Margin {
            horizontal: 2,
            vertical: 1,
        })
    } else {
        area
    };
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(content)
}

fn surface_columns(body: Rect) -> [Rect; 3] {
    if body.width < 66 {
        [body, Rect::default(), Rect::default()]
    } else {
        Layout::horizontal([
            Constraint::Min(40),
            Constraint::Length(2),
            Constraint::Min(24),
        ])
        .areas(body)
    }
}

fn next_surface_row(area: Rect, cursor: &mut u16) -> Option<Rect> {
    next_surface_rows(area, cursor, 1)
}

fn next_surface_rows(area: Rect, cursor: &mut u16, height: u16) -> Option<Rect> {
    let action_y = area.y + area.height.saturating_sub(1);
    if height == 0 || cursor.saturating_add(height) > action_y {
        return None;
    }
    let rows = Rect {
        x: area.x,
        y: *cursor,
        width: area.width,
        height,
    };
    *cursor = cursor.saturating_add(height);
    Some(rows)
}

fn render_surface_heading(
    frame: &mut Frame,
    area: Rect,
    cursor: &mut u16,
    label: &str,
    theme: &Theme,
) {
    if let Some(row) = next_surface_row(area, cursor) {
        frame.render_widget(
            Paragraph::new(labeled_heading_line(label, area.width, theme)),
            row,
        );
    }
}

fn render_surface_blank(area: Rect, cursor: &mut u16, frame: &mut Frame) {
    if let Some(row) = next_surface_row(area, cursor) {
        frame.render_widget(Paragraph::new(""), row);
    }
}

fn render_surface_action(frame: &mut Frame, area: Rect, label: &str, focused: bool, theme: &Theme) {
    let row = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(action_line(label, focused, theme)), row);
}

fn render_surface_action_dock(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    focused: bool,
    theme: &Theme,
) {
    if area.height < 2 {
        render_surface_action(frame, area, label, focused, theme);
        return;
    }
    let separator = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(2),
        width: area.width,
        height: 1,
    };
    let action = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    render_horizontal_rule(frame, separator, theme);
    frame.render_widget(Paragraph::new(action_line(label, focused, theme)), action);
}

fn optional_action_label(dirty: bool, language: UiLanguage) -> &'static str {
    if dirty {
        t(language, "Save and return", "保存并返回")
    } else {
        t(language, "Return", "返回")
    }
}

fn render_surface_footer(
    frame: &mut Frame,
    area: Rect,
    editing: bool,
    action_focused: bool,
    language: UiLanguage,
    theme: &Theme,
) {
    let hints: Vec<(&str, &str)> = if editing {
        vec![
            ("Enter", t(language, "confirm", "确认")),
            ("Esc", t(language, "discard", "放弃")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ]
    } else if action_focused {
        vec![
            ("Enter/l", t(language, "continue", "继续")),
            ("Esc/h", t(language, "back", "返回")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ]
    } else {
        vec![
            ("↑↓ j/k", t(language, "move", "移动")),
            ("Enter/l", t(language, "edit", "编辑")),
            ("Esc/h", t(language, "back", "返回")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ]
    };
    render_contextual_footer(frame, area, &hints, theme);
}

fn connection_inspector_body(
    field: Option<SetupField>,
    language: UiLanguage,
) -> &'static [&'static str] {
    match field {
        Some(SetupField::TunnelId) => match language {
            UiLanguage::En => &[
                "Stable identifier used by the Tunnel.",
                "Keep it short and reusable.",
            ],
            UiLanguage::ZhCn => &["Tunnel 使用的稳定标识。", "建议保持简短并长期复用。"],
        },
        Some(SetupField::TunnelSecretSource) => match language {
            UiLanguage::En => &[
                "Choose a file or environment reference.",
                "Secret contents stay masked.",
            ],
            UiLanguage::ZhCn => &["选择文件或环境变量引用。", "密钥内容始终保持隐藏。"],
        },
        Some(SetupField::TunnelSecretPath | SetupField::TunnelSecretEnvironment) => {
            match language {
                UiLanguage::En => &[
                    "Only the reference is shown here.",
                    "The secret itself is never rendered.",
                ],
                UiLanguage::ZhCn => &["这里仅显示引用位置。", "密钥本身不会被渲染。"],
            }
        }
        Some(SetupField::ProvisionTunnelSecret | SetupField::TunnelSecretValue) => match language {
            UiLanguage::En => &[
                "Provisioning is staged until final write.",
                "Input remains masked while editing.",
            ],
            UiLanguage::ZhCn => &["写入动作会暂存到最终提交。", "编辑时输入始终隐藏。"],
        },
        Some(SetupField::HubUrl) => match language {
            UiLanguage::En => &[
                "Remote Hub endpoint used for dispatch.",
                "Validation runs before leaving this page.",
            ],
            UiLanguage::ZhCn => &["用于调度的远程 Hub 地址。", "离开页面前会执行验证。"],
        },
        Some(SetupField::HubTransport) => match language {
            UiLanguage::En => &[
                "Transport used to reach the Hub.",
                "Keep the existing setup semantics.",
            ],
            UiLanguage::ZhCn => &["访问 Hub 所使用的传输方式。", "保持现有配置语义。"],
        },
        Some(SetupField::AgentId) => match language {
            UiLanguage::En => &[
                "Stable identity presented to the Hub.",
                "It is safe to edit before commit.",
            ],
            UiLanguage::ZhCn => &["向 Hub 呈现的稳定身份。", "提交前可以安全编辑。"],
        },
        Some(SetupField::AgentSecret) => match language {
            UiLanguage::En => &[
                "Credential for Hub authentication.",
                "The value is never displayed.",
            ],
            UiLanguage::ZhCn => &["用于 Hub 身份验证的凭据。", "值不会显示出来。"],
        },
        _ => match language {
            UiLanguage::En => &[
                "Selections are staged in memory.",
                "Validation remains authoritative.",
            ],
            UiLanguage::ZhCn => &["选择会暂存在内存中。", "验证逻辑保持不变。"],
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectionFocusItem {
    Field(SetupField),
    SecretSource(TunnelSecretSource),
}

impl ConnectionFocusItem {
    fn field(self) -> SetupField {
        match self {
            Self::Field(field) => field,
            Self::SecretSource(_) => SetupField::TunnelSecretSource,
        }
    }
}

pub(super) fn connection_focus_items(session: &SetupSession) -> Vec<ConnectionFocusItem> {
    match session.selected_mode() {
        RuntimeMode::Standalone => {
            let mut items = vec![
                ConnectionFocusItem::Field(SetupField::TunnelId),
                ConnectionFocusItem::SecretSource(TunnelSecretSource::File),
                ConnectionFocusItem::SecretSource(TunnelSecretSource::Environment),
            ];
            match session.standalone().secret_source {
                TunnelSecretSource::File => {
                    items.push(ConnectionFocusItem::Field(SetupField::TunnelSecretPath));
                    items.push(ConnectionFocusItem::Field(
                        SetupField::ProvisionTunnelSecret,
                    ));
                    if session.standalone().provision_secret_now {
                        items.push(ConnectionFocusItem::Field(SetupField::TunnelSecretValue));
                    }
                }
                TunnelSecretSource::Environment => {
                    items.push(ConnectionFocusItem::Field(
                        SetupField::TunnelSecretEnvironment,
                    ));
                }
            }
            items
        }
        RuntimeMode::Hub => vec![
            ConnectionFocusItem::Field(SetupField::HubUrl),
            ConnectionFocusItem::Field(SetupField::HubTransport),
            ConnectionFocusItem::Field(SetupField::AgentId),
            ConnectionFocusItem::Field(SetupField::AgentSecret),
        ],
        RuntimeMode::Local => Vec::new(),
    }
}

pub(super) fn connection_focus_len(session: &SetupSession) -> usize {
    connection_focus_items(session).len() + 1
}

pub(super) fn connection_focus_field(session: &SetupSession, focus: usize) -> Option<SetupField> {
    connection_focus_items(session)
        .get(focus)
        .copied()
        .map(ConnectionFocusItem::field)
}

pub(super) fn connection_secret_source_for_focus(
    session: &SetupSession,
    focus: usize,
) -> Option<TunnelSecretSource> {
    match connection_focus_items(session).get(focus).copied() {
        Some(ConnectionFocusItem::SecretSource(source)) => Some(source),
        _ => None,
    }
}

pub(super) fn connection_field_index(session: &SetupSession, field: SetupField) -> Option<usize> {
    let items = connection_focus_items(session);
    if field == SetupField::TunnelSecretSource {
        let selected = session.standalone().secret_source;
        return items.iter().position(
            |item| matches!(item, ConnectionFocusItem::SecretSource(source) if *source == selected),
        );
    }
    items.iter().position(|item| item.field() == field)
}

fn connection_label(field: SetupField, _value: Option<&str>, language: UiLanguage) -> String {
    match field {
        SetupField::TunnelId => t(language, "Tunnel ID", "隧道 ID").to_string(),
        SetupField::TunnelSecretSource => t(language, "Secret source", "密钥来源").to_string(),
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
    let [header, top_rule, body, bottom_rule, footer] = surface_shell_areas(frame.area());
    render_surface_header(
        frame,
        header,
        t(language, "AgenticGPT config init", "AgenticGPT 配置初始化"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    render_horizontal_rule(frame, top_rule, theme);
    let [left, _, right] = surface_columns(body);

    let sections = all_optional_sections();
    let focusable = session.available_optional_sections();
    let mut cursor = left.y;
    render_surface_heading(
        frame,
        left,
        &mut cursor,
        t(language, "Optional settings", "可选配置"),
        theme,
    );
    render_surface_blank(left, &mut cursor, frame);
    for section in sections {
        let status = session.section_status(section);
        let applicable_index = focusable.iter().position(|candidate| *candidate == section);
        let focused = applicable_index == Some(state.focus);
        if let Some(row) = next_surface_row(left, &mut cursor) {
            frame.render_widget(
                Paragraph::new(surface_status_line(
                    section_label(section, language),
                    section_status_label(status, language),
                    focused,
                    status == SectionStatus::NotApplicable,
                    theme,
                )),
                row,
            );
        }
    }
    render_surface_action(
        frame,
        left,
        t(language, "Finish and continue", "完成并继续"),
        state.focus >= focusable.len(),
        theme,
    );
    if right.width > 0 {
        render_surface(frame, right, theme);
        let inspector = right.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let section = focusable.get(state.focus).copied();
        let title = section
            .map(|section| section_label(section, language))
            .unwrap_or_else(|| t(language, "Optional settings", "可选配置"));
        let body = optional_center_inspector_body(section, language);
        render_inspector(frame, inspector, title, body, theme);
    }
    render_horizontal_rule(frame, bottom_rule, theme);
    render_surface_footer(
        frame,
        footer,
        false,
        state.focus >= focusable.len(),
        language,
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
    section_dirty: bool,
    progress: (usize, usize),
) {
    if section == OptionalSection::Workspace {
        render_workspace_form(
            frame,
            session,
            state,
            language,
            theme,
            errors,
            section_draft,
            section_dirty,
            progress,
        );
        return;
    }

    let [header, top_rule, body, bottom_rule, footer] = surface_shell_areas(frame.area());
    render_surface_header(
        frame,
        header,
        t(language, "AgenticGPT config init", "AgenticGPT 配置初始化"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    render_horizontal_rule(frame, top_rule, theme);
    let [left, _, right] = surface_columns(body);

    let fallback = session.optional_draft(section);
    let draft = section_draft.unwrap_or(&fallback);
    let focus_items = optional_focus_items(section, draft);
    let action_focused = state.focus >= focus_items.len();
    let section_heading = format!(
        "{} {}",
        section_label(section, language),
        t(language, "settings", "配置")
    );
    let mut lines = vec![
        labeled_heading_line(&section_heading, left.width, theme),
        Line::raw(""),
    ];
    let mut focused_line = 0usize;

    match section {
        OptionalSection::Identity => push_long_form_field(
            &mut lines,
            &mut focused_line,
            section,
            draft,
            state,
            SetupField::DisplayName,
            language,
            theme,
            errors,
            left.width,
        ),
        OptionalSection::Confirmation => {
            push_choice_group(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::ConfirmationProvider,
                t(language, "Provider", "提供方"),
                &[
                    ("default", t(language, "Default", "默认")),
                    ("freedesktop", "freedesktop"),
                    ("ntfy", "ntfy"),
                    ("none", t(language, "None", "无")),
                ],
                theme,
                errors,
                language,
            );
            push_choice_group(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::ConfirmationLanguage,
                t(language, "Language", "语言"),
                &[("zh-CN", "zh-CN"), ("en", "en")],
                theme,
                errors,
                language,
            );
        }
        OptionalSection::Limits => push_limits_form(
            &mut lines,
            &mut focused_line,
            draft,
            state,
            language,
            theme,
            errors,
        ),
        OptionalSection::Sandbox => {
            push_boolean_field(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::SandboxEnabled,
                language,
                theme,
                errors,
            );
            push_long_form_field(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::BubblewrapPath,
                language,
                theme,
                errors,
                left.width,
            );
            push_list_field(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::RequiredRuntimePaths,
                language,
                theme,
                errors,
                left.width,
            );
        }
        OptionalSection::Room => {
            for field in [
                SetupField::RoomTimezone,
                SetupField::DiaryBoundaryHour,
                SetupField::NotebookRoot,
            ] {
                push_long_form_field(
                    &mut lines,
                    &mut focused_line,
                    section,
                    draft,
                    state,
                    field,
                    language,
                    theme,
                    errors,
                    left.width,
                );
            }
        }
        OptionalSection::TunnelClient => {
            for field in [SetupField::TunnelClientVersion, SetupField::TunnelCacheDir] {
                push_long_form_field(
                    &mut lines,
                    &mut focused_line,
                    section,
                    draft,
                    state,
                    field,
                    language,
                    theme,
                    errors,
                    left.width,
                );
            }
            push_boolean_field(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::TunnelAutoDownload,
                language,
                theme,
                errors,
            );
            for field in [
                SetupField::TunnelExecutable,
                SetupField::TunnelDownloadUrl,
                SetupField::TunnelSha256,
            ] {
                push_long_form_field(
                    &mut lines,
                    &mut focused_line,
                    section,
                    draft,
                    state,
                    field,
                    language,
                    theme,
                    errors,
                    left.width,
                );
            }
        }
        OptionalSection::HubReporting => {
            push_boolean_field(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::HubReportingEnabled,
                language,
                theme,
                errors,
            );
            push_choice_group(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::HubReportingDetail,
                t(language, "Reporting detail", "报告详细程度"),
                &[("metadata", "metadata"), ("full", "full")],
                theme,
                errors,
                language,
            );
        }
        OptionalSection::Workspace => unreachable!("workspace has a dedicated renderer"),
    }

    let content_height = left.height.saturating_sub(2);
    let content_area = Rect {
        x: left.x,
        y: left.y,
        width: left.width,
        height: content_height,
    };
    let visible = usize::from(content_height);
    let max_scroll = lines.len().saturating_sub(visible);
    if action_focused {
        focused_line = lines.len().saturating_sub(1);
    }
    let scroll = focused_line
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(max_scroll)
        .min(usize::from(u16::MAX)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content_area);
    render_surface_action_dock(
        frame,
        left,
        optional_action_label(section_dirty, language),
        action_focused,
        theme,
    );

    let field = optional_focus_field(section, draft, state.focus);
    if right.width > 0 {
        render_surface(frame, right, theme);
        let inspector = right.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let title = field
            .map(|field| optional_field_label(field, language))
            .unwrap_or_else(|| section_label(section, language));
        let inspector_body = optional_form_inspector_body(section, field, language);
        render_inspector(frame, inspector, title, inspector_body, theme);
    }
    render_horizontal_rule(frame, bottom_rule, theme);
    render_optional_form_footer(
        frame,
        footer,
        section,
        draft,
        state,
        action_focused,
        language,
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
fn push_long_form_field(
    lines: &mut Vec<Line<'static>>,
    focused_line: &mut usize,
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    field: SetupField,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    width: u16,
) {
    lines.push(subsection_heading_line(
        optional_field_label(field, language),
        theme,
    ));
    let focus = optional_field_index(section, draft, field).unwrap_or(usize::MAX);
    let focused = state.focus == focus;
    if focused {
        *focused_line = lines.len();
    }
    let confirmed = optional_field_value(draft, field);
    let cursor = editing_cursor(state, field);
    let current = current_input_value(state, field, &confirmed);
    if field == SetupField::DiaryBoundaryHour {
        lines.push(numeric_input_value_line(
            current,
            focused,
            cursor.is_some(),
            cursor,
            theme,
        ));
    } else {
        lines.push(long_form_input_value_line(
            current,
            focused,
            cursor.is_some(),
            cursor,
            false,
            false,
            width,
            theme,
        ));
    }
    push_optional_error(lines, errors, field, language, theme);
    lines.push(Line::raw(""));
}

#[allow(clippy::too_many_arguments)]
fn push_boolean_field(
    lines: &mut Vec<Line<'static>>,
    focused_line: &mut usize,
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    field: SetupField,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
) {
    let focus = optional_field_index(section, draft, field).unwrap_or(usize::MAX);
    let focused = state.focus == focus;
    if focused {
        *focused_line = lines.len();
    }
    let value = optional_field_value(draft, field);
    lines.push(boolean_row_line(
        optional_field_label(field, language),
        if value == "true" {
            t(language, "on", "开")
        } else {
            t(language, "off", "关")
        },
        focused,
        24,
        theme,
    ));
    push_optional_error(lines, errors, field, language, theme);
    lines.push(Line::raw(""));
}

#[allow(clippy::too_many_arguments)]
fn push_choice_group(
    lines: &mut Vec<Line<'static>>,
    focused_line: &mut usize,
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    field: SetupField,
    heading: &str,
    choices: &[(&str, &str)],
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    language: UiLanguage,
) {
    lines.push(subsection_heading_line(heading, theme));
    for (value, label) in choices {
        let focus = optional_choice_index(section, draft, field, value).unwrap_or(usize::MAX);
        let focused = state.focus == focus;
        if focused {
            *focused_line = lines.len();
        }
        lines.push(choice_row_line(
            label,
            focused,
            optional_choice_selected(draft, field, value),
            18,
            theme,
        ));
    }
    push_optional_error(lines, errors, field, language, theme);
    lines.push(Line::raw(""));
}

#[allow(clippy::too_many_arguments)]
fn push_list_field(
    lines: &mut Vec<Line<'static>>,
    focused_line: &mut usize,
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    field: SetupField,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    width: u16,
) {
    lines.push(subsection_heading_line(
        optional_field_label(field, language),
        theme,
    ));
    let targets = optional_focus_items(section, draft)
        .iter()
        .enumerate()
        .filter_map(|(focus, item)| match item {
            OptionalFocusItem::List {
                field: candidate,
                index,
            } if *candidate == field => Some((focus, *index)),
            _ => None,
        })
        .collect::<Vec<_>>();
    match optional_list_state(draft, field) {
        Ok(list) if !list.is_empty() => {
            for (focus, index) in targets {
                let Some(index) = index else {
                    continue;
                };
                let focused = state.focus == focus;
                if focused {
                    *focused_line = lines.len();
                }
                let editing = state
                    .list_edit
                    .as_ref()
                    .is_some_and(|target| target.field == field && target.index == index);
                let value = if editing {
                    state
                        .editing
                        .as_ref()
                        .map(|edit| edit.buffer.as_str())
                        .unwrap_or_default()
                } else {
                    list.items()
                        .get(index)
                        .map(String::as_str)
                        .unwrap_or_default()
                };
                lines.push(editable_list_item_line(
                    value,
                    focused,
                    editing,
                    editing
                        .then(|| state.editing.as_ref().map(|edit| edit.cursor))
                        .flatten(),
                    width,
                    theme,
                ));
            }
        }
        _ => {
            let focus = targets
                .first()
                .map(|(focus, _)| *focus)
                .unwrap_or(usize::MAX);
            let focused = state.focus == focus;
            if focused {
                *focused_line = lines.len();
            }
            lines.push(editable_list_item_line(
                "", focused, false, None, width, theme,
            ));
        }
    }
    push_optional_error(lines, errors, field, language, theme);
    lines.push(Line::raw(""));
}

fn push_limits_form(
    lines: &mut Vec<Line<'static>>,
    focused_line: &mut usize,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
) {
    let section = OptionalSection::Limits;
    lines.push(subsection_heading_line(
        t(language, "Task concurrency", "任务并发"),
        theme,
    ));
    let max_focus =
        optional_field_index(section, draft, SetupField::MaxConcurrentTasks).unwrap_or(usize::MAX);
    let max_focused = state.focus == max_focus;
    if max_focused {
        *focused_line = lines.len();
    }
    let max_value = optional_field_value(draft, SetupField::MaxConcurrentTasks);
    let max_cursor = editing_cursor(state, SetupField::MaxConcurrentTasks);
    lines.push(input_row_line(
        optional_field_label(SetupField::MaxConcurrentTasks, language),
        current_input_value(state, SetupField::MaxConcurrentTasks, &max_value),
        max_focused,
        max_cursor.is_some(),
        max_cursor,
        22,
        5,
        theme,
    ));
    push_optional_error(
        lines,
        errors,
        SetupField::MaxConcurrentTasks,
        language,
        theme,
    );
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        format!(
            "  {}",
            optional_field_label(SetupField::MaxActiveJobs, language)
        ),
        theme.muted,
    ));

    let auto_focus = optional_choice_index(section, draft, SetupField::MaxActiveJobs, "auto")
        .unwrap_or(usize::MAX);
    let auto_focused = state.focus == auto_focus;
    if auto_focused {
        *focused_line = lines.len();
    }
    lines.push(choice_row_line(
        t(language, "Auto", "自动"),
        auto_focused,
        optional_choice_selected(draft, SetupField::MaxActiveJobs, "auto"),
        21,
        theme,
    ));

    let custom_focus = optional_choice_index(section, draft, SetupField::MaxActiveJobs, "custom")
        .unwrap_or(usize::MAX);
    let custom_focused = state.focus == custom_focus;
    if custom_focused {
        *focused_line = lines.len();
    }
    let custom_cursor = editing_cursor(state, SetupField::MaxActiveJobs);
    let configured = optional_field_value(draft, SetupField::MaxActiveJobs);
    let custom_value = if custom_cursor.is_some() {
        state
            .editing
            .as_ref()
            .map(|edit| edit.buffer.as_str())
            .unwrap_or(state.max_active_custom.as_str())
    } else if configured != "auto" {
        configured.as_str()
    } else {
        state.max_active_custom.as_str()
    };
    lines.push(choice_input_row_line(
        t(language, "Custom", "自定义"),
        custom_value,
        custom_focused,
        optional_choice_selected(draft, SetupField::MaxActiveJobs, "custom"),
        custom_cursor.is_some(),
        custom_cursor,
        21,
        5,
        theme,
    ));
    push_optional_error(lines, errors, SetupField::MaxActiveJobs, language, theme);
    lines.push(Line::raw(""));

    lines.push(subsection_heading_line(
        t(language, "Search", "搜索"),
        theme,
    ));
    let context_focus = optional_field_index(section, draft, SetupField::MaxFileSearchContextLines)
        .unwrap_or(usize::MAX);
    let context_focused = state.focus == context_focus;
    if context_focused {
        *focused_line = lines.len();
    }
    let context_value = optional_field_value(draft, SetupField::MaxFileSearchContextLines);
    let context_cursor = editing_cursor(state, SetupField::MaxFileSearchContextLines);
    lines.push(input_row_line(
        t(language, "Context lines", "上下文行数"),
        current_input_value(state, SetupField::MaxFileSearchContextLines, &context_value),
        context_focused,
        context_cursor.is_some(),
        context_cursor,
        22,
        5,
        theme,
    ));
    push_optional_error(
        lines,
        errors,
        SetupField::MaxFileSearchContextLines,
        language,
        theme,
    );
    lines.push(Line::raw(""));
}

fn push_optional_error(
    lines: &mut Vec<Line<'static>>,
    errors: &HashMap<SetupField, String>,
    field: SetupField,
    language: UiLanguage,
    theme: &Theme,
) {
    if let Some(error) = errors.get(&field) {
        lines.push(inline_error_line(&localized_error(error, language), theme));
    }
}

fn optional_choice_index(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    field: SetupField,
    value: &str,
) -> Option<usize> {
    optional_focus_items(section, draft)
        .iter()
        .position(|item| {
            matches!(
                item,
                OptionalFocusItem::Choice {
                    field: candidate,
                    value: candidate_value,
                } if *candidate == field && *candidate_value == value
            )
        })
}

fn optional_choice_selected(draft: &OptionalSectionDraft, field: SetupField, choice: &str) -> bool {
    let value = optional_field_value(draft, field);
    match field {
        SetupField::ConfirmationProvider => match choice {
            "default" => matches!(
                value.trim(),
                "default" | "freedesktop-then-hub" | "freedesktopThenHub" | "freedesktop-then-ntfy"
            ),
            "freedesktop" => value.trim() == "freedesktop",
            "ntfy" => matches!(value.trim(), "ntfy" | "hub"),
            "none" => value.trim() == "none",
            _ => false,
        },
        SetupField::ConfirmationLanguage => {
            crate::config::normalize_confirmation_language(&value) == choice
        }
        SetupField::MaxActiveJobs => match choice {
            "auto" => value.trim() == "auto",
            "custom" => value.trim() != "auto",
            _ => false,
        },
        SetupField::HubReportingDetail => value.trim() == choice,
        _ => value.trim() == choice,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_optional_form_footer(
    frame: &mut Frame,
    area: Rect,
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    action_focused: bool,
    language: UiLanguage,
    theme: &Theme,
) {
    if state.editing.is_some() {
        render_contextual_footer(
            frame,
            area,
            &[
                ("Enter", t(language, "confirm", "确认")),
                ("Esc", t(language, "discard", "放弃")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            theme,
        );
        return;
    }
    if action_focused {
        render_contextual_footer(
            frame,
            area,
            &[
                ("Enter/l", t(language, "continue", "继续")),
                ("Esc/h", t(language, "back", "返回")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            theme,
        );
        return;
    }
    if let Some((_, index)) = optional_list_target(section, draft, state.focus) {
        render_contextual_footer(
            frame,
            area,
            &[
                ("↑↓ j/k", t(language, "move", "移动")),
                (
                    "Enter/l",
                    if index.is_some() {
                        t(language, "edit", "编辑")
                    } else {
                        t(language, "add", "新增")
                    },
                ),
                ("a", t(language, "add", "新增")),
                ("d", t(language, "delete", "删除")),
                ("Esc/h", t(language, "back", "返回")),
            ],
            theme,
        );
        return;
    }
    let field = optional_focus_field(section, draft, state.focus);
    let action = if optional_choice_for_focus(section, draft, state.focus).is_some() {
        t(language, "choose", "选择")
    } else if field.is_some_and(optional_field_is_toggle) {
        t(language, "toggle", "切换")
    } else {
        t(language, "edit", "编辑")
    };
    render_contextual_footer(
        frame,
        area,
        &[
            ("↑↓ j/k", t(language, "move", "移动")),
            ("Enter/l", action),
            ("Esc/h", t(language, "back", "返回")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ],
        theme,
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceFocusItem {
    Root,
    Path {
        field: SetupField,
        index: Option<usize>,
    },
}

impl WorkspaceFocusItem {
    fn field(self) -> SetupField {
        match self {
            Self::Root => SetupField::WorkspaceRoot,
            Self::Path { field, .. } => field,
        }
    }
}

fn workspace_list_fields() -> [SetupField; 3] {
    [
        SetupField::WriteRoots,
        SetupField::ReadOnlyRoots,
        SetupField::DenyRoots,
    ]
}

pub(super) fn workspace_list_state(
    draft: &OptionalSectionDraft,
    field: SetupField,
) -> Result<EditableListState, ()> {
    let OptionalSectionDraft::Workspace(workspace) = draft else {
        return Err(());
    };
    let raw = match field {
        SetupField::WriteRoots => &workspace.write_roots,
        SetupField::ReadOnlyRoots => &workspace.read_only_roots,
        SetupField::DenyRoots => &workspace.deny_roots,
        _ => return Err(()),
    };
    serde_json::from_str::<Vec<String>>(raw)
        .map(EditableListState::new)
        .map_err(|_| ())
}

pub(super) fn workspace_focus_items(draft: &OptionalSectionDraft) -> Vec<WorkspaceFocusItem> {
    let mut items = vec![WorkspaceFocusItem::Root];
    for field in workspace_list_fields() {
        match workspace_list_state(draft, field) {
            Ok(state) if !state.is_empty() => {
                items.extend(
                    (0..state.items().len()).map(|index| WorkspaceFocusItem::Path {
                        field,
                        index: Some(index),
                    }),
                );
            }
            _ => items.push(WorkspaceFocusItem::Path { field, index: None }),
        }
    }
    items
}

pub(super) fn workspace_focus_field(
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<SetupField> {
    workspace_focus_items(draft)
        .get(focus)
        .copied()
        .map(WorkspaceFocusItem::field)
}

pub(super) fn workspace_path_target(
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<(SetupField, Option<usize>)> {
    match workspace_focus_items(draft).get(focus).copied() {
        Some(WorkspaceFocusItem::Path { field, index }) => Some((field, index)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OptionalFocusItem {
    Field(SetupField),
    Choice {
        field: SetupField,
        value: &'static str,
    },
    List {
        field: SetupField,
        index: Option<usize>,
    },
}

impl OptionalFocusItem {
    fn field(self) -> SetupField {
        match self {
            Self::Field(field) | Self::Choice { field, .. } | Self::List { field, .. } => field,
        }
    }
}

pub(super) fn optional_list_state(
    draft: &OptionalSectionDraft,
    field: SetupField,
) -> Result<EditableListState, ()> {
    let raw = match (draft, field) {
        (OptionalSectionDraft::Workspace(value), SetupField::WriteRoots) => &value.write_roots,
        (OptionalSectionDraft::Workspace(value), SetupField::ReadOnlyRoots) => {
            &value.read_only_roots
        }
        (OptionalSectionDraft::Workspace(value), SetupField::DenyRoots) => &value.deny_roots,
        (OptionalSectionDraft::Sandbox(value), SetupField::RequiredRuntimePaths) => {
            &value.required_runtime_paths
        }
        _ => return Err(()),
    };
    serde_json::from_str::<Vec<String>>(raw)
        .map(EditableListState::new)
        .map_err(|_| ())
}

pub(super) fn set_optional_list_state(
    draft: &mut OptionalSectionDraft,
    field: SetupField,
    state: &EditableListState,
) -> bool {
    let Ok(serialized) = serde_json::to_string(state.items()) else {
        return false;
    };
    match (draft, field) {
        (OptionalSectionDraft::Workspace(value), SetupField::WriteRoots) => {
            value.write_roots = serialized
        }
        (OptionalSectionDraft::Workspace(value), SetupField::ReadOnlyRoots) => {
            value.read_only_roots = serialized
        }
        (OptionalSectionDraft::Workspace(value), SetupField::DenyRoots) => {
            value.deny_roots = serialized
        }
        (OptionalSectionDraft::Sandbox(value), SetupField::RequiredRuntimePaths) => {
            value.required_runtime_paths = serialized
        }
        _ => return false,
    }
    true
}

fn list_focus_items(field: SetupField, draft: &OptionalSectionDraft) -> Vec<OptionalFocusItem> {
    match optional_list_state(draft, field) {
        Ok(state) if !state.is_empty() => (0..state.items().len())
            .map(|index| OptionalFocusItem::List {
                field,
                index: Some(index),
            })
            .collect(),
        _ => vec![OptionalFocusItem::List { field, index: None }],
    }
}

pub(super) fn optional_focus_items(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
) -> Vec<OptionalFocusItem> {
    match section {
        OptionalSection::Identity => vec![OptionalFocusItem::Field(SetupField::DisplayName)],
        OptionalSection::Workspace => workspace_focus_items(draft)
            .into_iter()
            .map(|item| match item {
                WorkspaceFocusItem::Root => OptionalFocusItem::Field(SetupField::WorkspaceRoot),
                WorkspaceFocusItem::Path { field, index } => {
                    OptionalFocusItem::List { field, index }
                }
            })
            .collect(),
        OptionalSection::Confirmation => ["default", "freedesktop", "ntfy", "none"]
            .into_iter()
            .map(|value| OptionalFocusItem::Choice {
                field: SetupField::ConfirmationProvider,
                value,
            })
            .chain(
                ["zh-CN", "en"]
                    .into_iter()
                    .map(|value| OptionalFocusItem::Choice {
                        field: SetupField::ConfirmationLanguage,
                        value,
                    }),
            )
            .collect(),
        OptionalSection::Limits => vec![
            OptionalFocusItem::Field(SetupField::MaxConcurrentTasks),
            OptionalFocusItem::Choice {
                field: SetupField::MaxActiveJobs,
                value: "auto",
            },
            OptionalFocusItem::Choice {
                field: SetupField::MaxActiveJobs,
                value: "custom",
            },
            OptionalFocusItem::Field(SetupField::MaxFileSearchContextLines),
        ],
        OptionalSection::Sandbox => {
            let mut items = vec![
                OptionalFocusItem::Field(SetupField::SandboxEnabled),
                OptionalFocusItem::Field(SetupField::BubblewrapPath),
            ];
            items.extend(list_focus_items(SetupField::RequiredRuntimePaths, draft));
            items
        }
        OptionalSection::Room => vec![
            OptionalFocusItem::Field(SetupField::RoomTimezone),
            OptionalFocusItem::Field(SetupField::DiaryBoundaryHour),
            OptionalFocusItem::Field(SetupField::NotebookRoot),
        ],
        OptionalSection::TunnelClient => vec![
            OptionalFocusItem::Field(SetupField::TunnelClientVersion),
            OptionalFocusItem::Field(SetupField::TunnelCacheDir),
            OptionalFocusItem::Field(SetupField::TunnelAutoDownload),
            OptionalFocusItem::Field(SetupField::TunnelExecutable),
            OptionalFocusItem::Field(SetupField::TunnelDownloadUrl),
            OptionalFocusItem::Field(SetupField::TunnelSha256),
        ],
        OptionalSection::HubReporting => vec![
            OptionalFocusItem::Field(SetupField::HubReportingEnabled),
            OptionalFocusItem::Choice {
                field: SetupField::HubReportingDetail,
                value: "metadata",
            },
            OptionalFocusItem::Choice {
                field: SetupField::HubReportingDetail,
                value: "full",
            },
        ],
    }
}

pub(super) fn optional_focus_len(section: OptionalSection, draft: &OptionalSectionDraft) -> usize {
    optional_focus_items(section, draft).len() + 1
}

pub(super) fn optional_focus_field(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<SetupField> {
    optional_focus_items(section, draft)
        .get(focus)
        .copied()
        .map(OptionalFocusItem::field)
}

pub(super) fn optional_choice_for_focus(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<(SetupField, &'static str)> {
    match optional_focus_items(section, draft).get(focus).copied() {
        Some(OptionalFocusItem::Choice { field, value }) => Some((field, value)),
        _ => None,
    }
}

pub(super) fn optional_list_target(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<(SetupField, Option<usize>)> {
    match optional_focus_items(section, draft).get(focus).copied() {
        Some(OptionalFocusItem::List { field, index }) => Some((field, index)),
        _ => None,
    }
}

pub(super) fn optional_field_index(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    field: SetupField,
) -> Option<usize> {
    optional_focus_items(section, draft)
        .iter()
        .position(|item| item.field() == field)
}

pub(super) fn optional_item_focus_index(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    field: SetupField,
    item_index: usize,
) -> Option<usize> {
    optional_focus_items(section, draft)
        .iter()
        .position(|item| {
            matches!(
                item,
                OptionalFocusItem::List {
                    field: candidate,
                    index: Some(index)
                } if *candidate == field && *index == item_index
            )
        })
}

#[allow(clippy::too_many_arguments)]
fn render_workspace_form(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    section_draft: Option<&OptionalSectionDraft>,
    section_dirty: bool,
    progress: (usize, usize),
) {
    let [header, top_rule, body, bottom_rule, footer] = surface_shell_areas(frame.area());
    render_surface_header(
        frame,
        header,
        t(language, "AgenticGPT config init", "AgenticGPT 配置初始化"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    render_horizontal_rule(frame, top_rule, theme);
    let [left, _, right] = surface_columns(body);

    let fallback = session.optional_draft(OptionalSection::Workspace);
    let draft = section_draft.unwrap_or(&fallback);
    let focus_items = workspace_focus_items(draft);
    let action_focused = state.focus >= focus_items.len();
    let mut lines = vec![
        labeled_heading_line(
            t(language, "Workspace settings", "工作区配置"),
            left.width,
            theme,
        ),
        Line::raw(""),
    ];
    let mut focused_line = 0usize;

    lines.push(subsection_heading_line(
        optional_field_label(SetupField::WorkspaceRoot, language),
        theme,
    ));
    let root_focused = state.focus == 0;
    if root_focused {
        focused_line = lines.len();
    }
    let root_value = optional_field_value(draft, SetupField::WorkspaceRoot);
    lines.push(long_form_input_value_line(
        current_input_value(state, SetupField::WorkspaceRoot, &root_value),
        root_focused,
        editing_cursor(state, SetupField::WorkspaceRoot).is_some(),
        editing_cursor(state, SetupField::WorkspaceRoot),
        false,
        false,
        left.width,
        theme,
    ));
    if let Some(error) = errors.get(&SetupField::WorkspaceRoot) {
        lines.push(inline_error_line(&localized_error(error, language), theme));
    }
    lines.push(Line::raw(""));

    for field in workspace_list_fields() {
        lines.push(subsection_heading_line(
            optional_field_label(field, language),
            theme,
        ));
        let targets = focus_items
            .iter()
            .enumerate()
            .filter_map(|(focus, item)| match item {
                WorkspaceFocusItem::Path {
                    field: candidate,
                    index,
                } if *candidate == field => Some((focus, *index)),
                _ => None,
            })
            .collect::<Vec<_>>();
        match workspace_list_state(draft, field) {
            Ok(list) if !list.is_empty() => {
                for (focus, index) in targets {
                    let Some(index) = index else {
                        continue;
                    };
                    let focused = state.focus == focus;
                    if focused {
                        focused_line = lines.len();
                    }
                    let list_editing = state
                        .list_edit
                        .as_ref()
                        .is_some_and(|target| target.field == field && target.index == index);
                    let value = if list_editing {
                        state
                            .editing
                            .as_ref()
                            .map(|editing| editing.buffer.as_str())
                            .unwrap_or_default()
                    } else {
                        list.items()
                            .get(index)
                            .map(String::as_str)
                            .unwrap_or_default()
                    };
                    lines.push(editable_list_item_line(
                        value,
                        focused,
                        list_editing,
                        if list_editing {
                            state.editing.as_ref().map(|editing| editing.cursor)
                        } else {
                            None
                        },
                        left.width,
                        theme,
                    ));
                }
            }
            Ok(_) => {
                let focus = targets.first().map(|(focus, _)| *focus).unwrap_or_default();
                let focused = state.focus == focus;
                if focused {
                    focused_line = lines.len();
                }
                lines.push(editable_list_item_line(
                    "", focused, false, None, left.width, theme,
                ));
            }
            Err(()) => {
                let focus = targets.first().map(|(focus, _)| *focus).unwrap_or_default();
                let focused = state.focus == focus;
                if focused {
                    focused_line = lines.len();
                }
                lines.push(editable_list_item_line(
                    "", focused, false, None, left.width, theme,
                ));
                let code = errors
                    .get(&field)
                    .map(String::as_str)
                    .unwrap_or(match field {
                        SetupField::WriteRoots => "config_init_path_policy_write_roots_invalid",
                        SetupField::ReadOnlyRoots => {
                            "config_init_path_policy_read_only_roots_invalid"
                        }
                        SetupField::DenyRoots => "config_init_path_policy_deny_roots_invalid",
                        _ => "config_init_optional_section_invalid",
                    });
                lines.push(inline_error_line(&localized_error(code, language), theme));
            }
        }
        if let Some(error) = errors.get(&field) {
            if workspace_list_state(draft, field).is_ok() {
                lines.push(inline_error_line(&localized_error(error, language), theme));
            }
        }
        lines.push(Line::raw(""));
    }

    let content_height = left.height.saturating_sub(2);
    let content_area = Rect {
        x: left.x,
        y: left.y,
        width: left.width,
        height: content_height,
    };
    let visible = usize::from(content_height);
    let max_scroll = lines.len().saturating_sub(visible);
    if action_focused {
        focused_line = lines.len().saturating_sub(1);
    }
    let scroll = focused_line
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(max_scroll)
        .min(usize::from(u16::MAX)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content_area);
    render_surface_action_dock(
        frame,
        left,
        optional_action_label(section_dirty, language),
        action_focused,
        theme,
    );

    let field = workspace_focus_field(draft, state.focus);
    if right.width > 0 {
        render_surface(frame, right, theme);
        let inspector = right.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let title = field
            .map(|field| optional_field_label(field, language))
            .unwrap_or_else(|| section_label(OptionalSection::Workspace, language));
        let inspector_body =
            optional_form_inspector_body(OptionalSection::Workspace, field, language);
        render_inspector(frame, inspector, title, inspector_body, theme);
    }
    render_horizontal_rule(frame, bottom_rule, theme);
    render_workspace_footer(frame, footer, state, draft, action_focused, language, theme);
}

fn render_workspace_footer(
    frame: &mut Frame,
    area: Rect,
    state: &TuiState,
    draft: &OptionalSectionDraft,
    action_focused: bool,
    language: UiLanguage,
    theme: &Theme,
) {
    if state.editing.is_some() {
        render_contextual_footer(
            frame,
            area,
            &[
                ("Enter", t(language, "confirm", "确认")),
                ("Esc", t(language, "discard", "放弃")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            theme,
        );
        return;
    }
    if action_focused {
        render_contextual_footer(
            frame,
            area,
            &[
                ("Enter/l", t(language, "continue", "继续")),
                ("Esc/h", t(language, "back", "返回")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            theme,
        );
        return;
    }
    if let Some((_, index)) = workspace_path_target(draft, state.focus) {
        render_contextual_footer(
            frame,
            area,
            &[
                ("↑↓ j/k", t(language, "move", "移动")),
                (
                    "Enter/l",
                    if index.is_some() {
                        t(language, "edit", "编辑")
                    } else {
                        t(language, "add", "新增")
                    },
                ),
                ("a", t(language, "add", "新增")),
                ("d", t(language, "delete", "删除")),
                ("Esc/h", t(language, "back", "返回")),
            ],
            theme,
        );
        return;
    }
    render_contextual_footer(
        frame,
        area,
        &[
            ("↑↓ j/k", t(language, "move", "移动")),
            ("Enter/l", t(language, "edit", "编辑")),
            ("Esc/h", t(language, "back", "返回")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ],
        theme,
    );
}

fn optional_center_inspector_body(
    section: Option<OptionalSection>,
    language: UiLanguage,
) -> &'static [&'static str] {
    match section {
        Some(section) => match language {
            UiLanguage::En => match section {
                OptionalSection::Identity => &["Set a stable display name for this agent."],
                OptionalSection::Workspace => &["Keep path policy values explicit and reviewable."],
                OptionalSection::Confirmation => {
                    &["Choose the confirmation provider and language."]
                }
                OptionalSection::Limits => &["Tune concurrency and search limits."],
                OptionalSection::Sandbox => &["Control sandbox execution and runtime paths."],
                OptionalSection::Room => &["Configure long-lived Room context."],
                OptionalSection::TunnelClient => &["Configure the managed Tunnel client."],
                OptionalSection::HubReporting => &["Choose Hub reporting behavior."],
            },
            UiLanguage::ZhCn => match section {
                OptionalSection::Identity => &["为 Agent 设置稳定的显示名称。"],
                OptionalSection::Workspace => &["路径策略保持明确且可复核。"],
                OptionalSection::Confirmation => &["选择确认提供方和语言。"],
                OptionalSection::Limits => &["调整并发和搜索限制。"],
                OptionalSection::Sandbox => &["控制沙箱执行和运行时路径。"],
                OptionalSection::Room => &["配置长期 Room 上下文。"],
                OptionalSection::TunnelClient => &["配置托管的 Tunnel 客户端。"],
                OptionalSection::HubReporting => &["选择 Hub 报告行为。"],
            },
        },
        None => match language {
            UiLanguage::En => &["Select an applicable section to configure."],
            UiLanguage::ZhCn => &["选择一个适用的配置区块。"],
        },
    }
}

fn optional_form_inspector_body(
    section: OptionalSection,
    field: Option<SetupField>,
    language: UiLanguage,
) -> &'static [&'static str] {
    match field {
        Some(field) => match language {
            UiLanguage::En => match field {
                SetupField::DisplayName => &["A human-readable identity for this agent."],
                SetupField::MaxConcurrentTasks | SetupField::MaxActiveJobs => {
                    &["Numeric limits are validated before save."]
                }
                SetupField::MaxFileSearchContextLines => &["Controls search context size."],
                SetupField::SandboxEnabled
                | SetupField::TunnelAutoDownload
                | SetupField::HubReportingEnabled => &["Toggle the staged value with Enter or l."],
                _ => &["Edit the staged value; validation remains authoritative."],
            },
            UiLanguage::ZhCn => match field {
                SetupField::DisplayName => &["用于标识 Agent 的可读名称。"],
                SetupField::MaxConcurrentTasks | SetupField::MaxActiveJobs => {
                    &["保存前会验证数值限制。"]
                }
                SetupField::MaxFileSearchContextLines => &["控制搜索上下文大小。"],
                SetupField::SandboxEnabled
                | SetupField::TunnelAutoDownload
                | SetupField::HubReportingEnabled => &["按 Enter 或 l 切换暂存值。"],
                _ => &["编辑暂存值；验证逻辑保持不变。"],
            },
        },
        None => match language {
            UiLanguage::En => match section {
                OptionalSection::Limits => &["Numeric and Auto/Custom-style values remain staged."],
                _ => &["Values remain staged until this section is saved."],
            },
            UiLanguage::ZhCn => match section {
                OptionalSection::Limits => &["数值和 Auto/Custom 风格的值会暂存。"],
                _ => &["值会暂存到保存此区块为止。"],
            },
        },
    }
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
        SetupField::WriteRoots => t(language, "Write roots", "写入根目录"),
        SetupField::ReadOnlyRoots => t(language, "Read-only roots", "只读根目录"),
        SetupField::DenyRoots => t(language, "Deny roots", "拒绝根目录"),
        SetupField::ConfirmationProvider => t(language, "Provider", "提供方"),
        SetupField::ConfirmationLanguage => t(language, "Language", "语言"),
        SetupField::MaxConcurrentTasks => t(language, "Max concurrent tasks", "最大并发任务"),
        SetupField::MaxActiveJobs => t(language, "Max active jobs", "最大活动作业"),
        SetupField::MaxFileSearchContextLines => {
            t(language, "File-search context lines", "文件搜索上下文行数")
        }
        SetupField::SandboxEnabled => t(language, "Sandbox enabled", "启用沙箱"),
        SetupField::BubblewrapPath => t(language, "Bubblewrap path", "Bubblewrap 路径"),
        SetupField::RequiredRuntimePaths => t(language, "Required runtime paths", "必需运行时路径"),
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
        assert!(rendered.contains("Normal"));
        assert!(rendered.contains("Room"));
        assert!(rendered.contains("Ctrl+C"));
        assert!(rendered.contains("── Runtime mode"));
        assert!(rendered.contains("── Profile"));
        assert!(rendered.contains("❯"));
        assert!(rendered.contains(""));
        assert!(rendered.contains("Suitable for a resident Agent"));
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
    fn workspace_form_renders_paths_as_scrollable_items_without_json() {
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
        assert!(rendered.contains("~/Documents"));
        assert!(rendered.contains("Return"));
        assert!(!rendered.contains("(JSON)"));
        assert!(!rendered.contains("[\"./workspace\""));

        app.focus_field(crate::config_setup::SetupField::DenyRoots);
        let scrolled = content(&app, 100, 20);
        assert!(scrolled.contains("Deny roots"));
        assert!(scrolled.contains("~/.ssh"));
        assert!(scrolled.contains("Return"));
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
    fn standalone_connection_renders_source_as_choices_and_boolean_without_marker() {
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        app.session_mut().standalone_mut().secret_path.clear();
        let rendered = content(&app, 90, 28);
        assert!(rendered.contains("Secret source"));
        assert!(rendered.contains("file"));
        assert!(rendered.contains("env"));
        assert_eq!(rendered.matches('').count(), 1);
        assert!(rendered.contains("Provision secret now"));
        assert!(rendered.contains("off"));
        assert!(!rendered.contains('•'));
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
    fn every_optional_form_keeps_primary_action_in_a_narrow_terminal() {
        for section_index in 0..7 {
            let mut app = ConfigTuiApp::new(SetupSession::new(
                SetupSeed {
                    mode: Some(RuntimeMode::Standalone),
                    profile: Some(WorkerProfile::Normal),
                    ..SetupSeed::default()
                },
                UiLanguage::En,
                "/tmp/config-tui-narrow-optional.json".into(),
            ));
            app.handle_action(TuiAction::Next).unwrap();
            app.handle_action(TuiAction::Next).unwrap();
            assert_eq!(app.page(), ConfigPage::OptionalCenter);
            for _ in 0..section_index {
                app.handle_action(TuiAction::MoveNext).unwrap();
            }
            app.handle_action(TuiAction::Activate).unwrap();
            assert!(matches!(app.page(), ConfigPage::Optional(_)));
            let rendered = content(&app, 48, 14);
            assert!(
                rendered.contains("Return"),
                "optional action missing at center index {section_index}"
            );
        }
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
