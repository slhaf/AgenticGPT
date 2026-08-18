use std::{collections::HashMap, path::PathBuf};

use unicode_width::UnicodeWidthStr;

use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::cli_i18n::UiLanguage;
use crate::config_setup::{
    default_optional_draft, McpServerDraft, OptionalSectionDraft, ReviewEditorKind, ReviewGroup,
    ReviewItem, ReviewModel, ReviewTarget, SectionStatus, SetupField, SetupSession,
};
use crate::config_templates::{OptionalSection, RuntimeMode, TunnelSecretSource};
use crate::tui::forms::{
    boolean_row_line, choice_input_row_line, choice_row_line, editable_list_item_line,
    inline_input_spans, input_row_line, long_form_input_value_line, numeric_input_value_line,
    ordered_multi_select_row_line, subsection_heading_line, value_row_line, EditableListState,
    OrderedMultiSelectState,
};
use crate::tui::{
    action_line, inline_error_line, labeled_heading_line, render_action_button,
    render_contextual_footer, render_footer, render_header, render_horizontal_rule,
    render_inspector, render_surface, render_surface_header, surface_choice_line,
    surface_local_rule_width, surface_status_line, Theme,
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
        "config_init_confirmation_channels_invalid" => (
            "Confirmation channels must be an ordered list of supported channels without duplicates.",
            "确认通道必须是受支持且不重复的有序列表。",
        ),
        "config_init_reporting_detail_invalid" => {
            ("Reporting detail is invalid.", "报告详细程度无效。")
        }
        "config_init_mcp_server_id_duplicate" => {
            ("MCP server IDs must be unique.", "MCP 服务 ID 不能重复。")
        }
        "config_init_mcp_server_id_invalid" => ("MCP server ID is invalid.", "MCP 服务 ID 无效。"),
        "config_init_mcp_transport_invalid" => (
            "Transport must be streamable-http or stdio.",
            "传输方式必须是 streamable-http 或 stdio。",
        ),
        "config_init_mcp_endpoint_invalid" => (
            "Enter a valid HTTP URL or stdio command.",
            "请输入有效的 HTTP URL 或 stdio 命令。",
        ),
        "config_init_mcp_bearer_token_invalid" => (
            "Bearer token must be non-empty and contain no whitespace or control characters.",
            "Bearer 令牌不能为空，且不能包含空白或控制字符。",
        ),
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
                render_review(
                    frame,
                    review,
                    session,
                    state,
                    language,
                    theme,
                    errors,
                    section_draft,
                    progress,
                )
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
    let mut lines = vec![
        labeled_heading_line(
            t(language, "Global settings", "全局设定"),
            area.width,
            theme,
        ),
        Line::raw(""),
        subsection_heading_line(t(language, "Runtime mode", "运行模式"), theme),
    ];
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
    lines.push(subsection_heading_line(
        t(language, "Profile", "能力配置"),
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

    let content_height = area.height.saturating_sub(2);
    let content_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: content_height,
    };
    lines.truncate(usize::from(content_height));
    frame.render_widget(Paragraph::new(lines), content_area);
    render_surface_action_dock(
        frame,
        area,
        t(language, "Next", "下一步"),
        state.focus == BASIC_NEXT_FOCUS,
        theme,
    );
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
                    "Expose a remote Tunnel while keeping local MCP access.",
                    "This mode is intended for ChatGPT Developer mode with Secure MCP Tunnel.",
                ],
                UiLanguage::ZhCn => &[
                    "通过 Tunnel 提供远程入口，同时保留本机 MCP。",
                    "此模式仅适用于 ChatGPT Developer mode + Secure MCP Tunnel。",
                ],
            },
        ),
        1 => (
            t(language, "Hub", "Hub"),
            match language {
                UiLanguage::En => &[
                    "Connect directly to an AgenticGPT Hub for routing and dispatch.",
                    "Suitable for regular Streamable HTTP MCP access and centrally managed agents.",
                ],
                UiLanguage::ZhCn => &[
                    "直接连接 AgenticGPT Hub，由 Hub 负责路由与调度。",
                    "适用于常规 Streamable HTTP MCP 接入和集中管理多个 Agent。",
                ],
            },
        ),
        2 => (
            t(language, "Local", "Local"),
            match language {
                UiLanguage::En => &[
                    "Serve MCP only on this machine without connecting to Tunnel or Hub.",
                    "Use it when no remote entry point is needed.",
                ],
                UiLanguage::ZhCn => &[
                    "不连接 Tunnel 或 Hub，只在本机提供 MCP。",
                    "适合不需要远程入口的本地使用。",
                ],
            },
        ),
        3 => (
            t(language, "Normal", "Normal"),
            match language {
                UiLanguage::En => &[
                    "Enable the general agent capability set; this is the default profile.",
                    "Profiles are not stored in config, so launch the agent with Normal as well.",
                ],
                UiLanguage::ZhCn => &[
                    "仅启用通用 Agent 能力；这是默认 profile。",
                    "Profile 不写入配置，启动时仍需使用 Normal。",
                ],
            },
        ),
        4 => (
            t(language, "Room", "Room"),
            match language {
                UiLanguage::En => &[
                    "Add Diary, Notebook, and other long-lived context capabilities to Normal.",
                    "Profiles are not stored in config, so launch the agent with Room as well.",
                ],
                UiLanguage::ZhCn => &[
                    "在 Normal 基础上增加 Diary、Notebook 等长期上下文能力。",
                    "Profile 不写入配置，启动时仍需使用 Room。",
                ],
            },
        ),
        _ => (
            t(language, "Next", "下一步"),
            match language {
                UiLanguage::En => &[
                    "Continue to connection settings with the selected mode and profile.",
                    "Nothing is written to disk until final confirmation.",
                ],
                UiLanguage::ZhCn => &[
                    "按当前运行模式和 profile 进入连接设置。",
                    "最终确认前不会写入磁盘。",
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

    const CONNECTION_LABEL_WIDTH: usize = 12;
    const CONNECTION_MIN_INPUT_WIDTH: usize = 14;
    let max_input_width = usize::from(left.width)
        .saturating_sub(CONNECTION_LABEL_WIDTH + 8)
        .clamp(8, 36);

    for (index, item) in items.iter().copied().enumerate() {
        match item {
            ConnectionFocusItem::SecretSource => {
                let focused = state.focus == index;
                let value = match session.standalone().secret_source {
                    TunnelSecretSource::File => "file",
                    TunnelSecretSource::Environment => "env",
                };
                if let Some(row) = next_surface_row(left, &mut cursor) {
                    frame.render_widget(
                        Paragraph::new(value_row_line(
                            &connection_label(SetupField::TunnelSecretSource, None, language),
                            value,
                            focused,
                            CONNECTION_LABEL_WIDTH,
                            theme,
                        )),
                        row,
                    );
                }
                if focused {
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
                }
            }
            ConnectionFocusItem::Field(field) => {
                let focused = state.focus == index;
                let confirmed_value = connection_value(session, field);
                let value = confirmed_value.as_deref().unwrap_or_default();
                let label = connection_label(field, None, language);
                if field == SetupField::ProvisionTunnelSecret {
                    if let Some(row) = next_surface_row(left, &mut cursor) {
                        frame.render_widget(
                            Paragraph::new(value_row_line(
                                &label,
                                if value == "true" {
                                    t(language, "on", "开")
                                } else {
                                    t(language, "off", "关")
                                },
                                focused,
                                CONNECTION_LABEL_WIDTH,
                                theme,
                            )),
                            row,
                        );
                    }
                } else if let Some(row) = next_surface_row(left, &mut cursor) {
                    let edit_cursor = editing_cursor(state, field);
                    let editing = edit_cursor.is_some();
                    let current_value = current_input_value(state, field, value);
                    let display_value = if matches!(
                        field,
                        SetupField::TunnelSecretValue | SetupField::AgentSecret
                    ) {
                        "•".repeat(current_value.chars().count())
                    } else {
                        current_value.to_string()
                    };
                    let input_width = (UnicodeWidthStr::width(display_value.as_str())
                        + usize::from(editing))
                    .clamp(
                        CONNECTION_MIN_INPUT_WIDTH.min(max_input_width),
                        max_input_width,
                    );
                    frame.render_widget(
                        Paragraph::new(input_row_line(
                            &label,
                            &display_value,
                            focused,
                            editing,
                            edit_cursor,
                            CONNECTION_LABEL_WIDTH,
                            input_width,
                            theme,
                        )),
                        row,
                    );
                }
                if focused {
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
        }
    }

    render_surface_action_dock(
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
    } else if connection_secret_source_for_focus(session, state.focus).is_some()
        || field == Some(SetupField::ProvisionTunnelSecret)
    {
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
    let action_y = area.y + area.height.saturating_sub(2);
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
        width: surface_local_rule_width(area.width),
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
                "The Tunnel client uses this ID to connect to its assigned Tunnel.",
                "",
                "Get it from:",
                "• OpenAI Platform → Organization → Tunnels",
                "• https://platform.openai.com/settings/organization/tunnels",
            ],
            UiLanguage::ZhCn => &[
                "Tunnel client 用它连接指定 Tunnel；此项不能为空。",
                "",
                "获取位置：",
                "• OpenAI Platform → Organization → Tunnels",
                "• https://platform.openai.com/settings/organization/tunnels",
            ],
        },
        Some(SetupField::TunnelSecretSource) => match language {
            UiLanguage::En => &[
                "Choose whether the Tunnel API key is read from a file or environment variable.",
                "Config stores only the reference, not the plaintext key.",
                "",
                "Create the key at:",
                "• OpenAI Platform → Organization → API keys",
                "• https://platform.openai.com/settings/organization/api-keys",
            ],
            UiLanguage::ZhCn => &[
                "选择从文件或环境变量读取 Tunnel API key。",
                "配置只保存引用，不保存明文密钥。",
                "",
                "密钥获取位置：",
                "• OpenAI Platform → Organization → API keys",
                "• https://platform.openai.com/settings/organization/api-keys",
            ],
        },
        Some(SetupField::TunnelSecretPath) => match language {
            UiLanguage::En => &[
                "The Tunnel API key is read from this file at startup.",
                "Default: ~/.agentic_gpt/secrets/tunnel-api-key; naming the path does not create it.",
            ],
            UiLanguage::ZhCn => &[
                "启动时从此文件读取 Tunnel API key。",
                "默认 ~/.agentic_gpt/secrets/tunnel-api-key；填写路径本身不会创建文件。",
            ],
        },
        Some(SetupField::TunnelSecretEnvironment) => match language {
            UiLanguage::En => &[
                "The Tunnel API key is read from this environment variable at startup.",
                "Enter a valid variable name and ensure the launching process receives a non-empty value.",
            ],
            UiLanguage::ZhCn => &[
                "启动时从这个环境变量读取 Tunnel API key。",
                "切到 env 后必须填写有效变量名，并确保启动进程能读取到非空值。",
            ],
        },
        Some(SetupField::ProvisionTunnelSecret) => match language {
            UiLanguage::En => &[
                "When enabled, final commit writes the secret to the selected file.",
                "It is off by default and is available only for a file source.",
            ],
            UiLanguage::ZhCn => &[
                "开启后，最终提交时把密钥写入所选文件。",
                "默认关闭，并且只支持 file 来源。",
            ],
        },
        Some(SetupField::TunnelSecretValue) => match language {
            UiLanguage::En => &[
                "This is the secret content written to the Secret file for this transaction.",
                "It is never stored in config JSON and must be non-empty when provisioning is enabled.",
            ],
            UiLanguage::ZhCn => &[
                "这是本次要写入 Secret file 的密钥内容。",
                "它不会进入配置 JSON；启用立即写入后不能为空。",
            ],
        },
        Some(SetupField::HubUrl) => match language {
            UiLanguage::En => &[
                "Base URL of the Hub this agent connects to.",
                "Default: http://localhost:8787; change it when the Hub is not running locally.",
            ],
            UiLanguage::ZhCn => &[
                "Agent 连接的 Hub 基地址。",
                "默认 http://localhost:8787；Hub 不在本机时改为它的 HTTP(S) 地址。",
            ],
        },
        Some(SetupField::HubTransport) => match language {
            UiLanguage::En => &[
                "Choose websocket or sse for the Hub connection; websocket is the default.",
                "Switch to sse mainly when the network or proxy does not support WebSockets well.",
            ],
            UiLanguage::ZhCn => &[
                "选择连接 Hub 的 websocket 或 sse 传输；默认 websocket。",
                "网络或代理不适合 WebSocket 时通常才改为 sse。",
            ],
        },
        Some(SetupField::AgentId) => match language {
            UiLanguage::En => &[
                "The Hub uses this ID to identify and route to the agent; default: laptop.",
                "It also names local socket/runtime paths, so use a stable ASCII identifier.",
            ],
            UiLanguage::ZhCn => &[
                "Hub 用它识别并路由到此 Agent；默认 laptop。",
                "它也用于本机 socket/运行目录，建议使用稳定的 ASCII 标识。",
            ],
        },
        Some(SetupField::AgentSecret) => match language {
            UiLanguage::En => &[
                "Sent as x-agent-secret when connecting to the Hub.",
                "There is no usable default; it must match the Hub and is stored as a string in config.",
            ],
            UiLanguage::ZhCn => &[
                "连接 Hub 时作为 x-agent-secret 发送。",
                "没有可用默认值，必须与 Hub 注册一致；它会以字符串保存在配置文件中。",
            ],
        },
        _ => match language {
            UiLanguage::En => &[
                "Continue with the current connection settings.",
                "Nothing is written to disk until final confirmation.",
            ],
            UiLanguage::ZhCn => &["按当前连接设置继续。", "最终确认前不会写入磁盘。"],
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConnectionFocusItem {
    Field(SetupField),
    SecretSource,
}

impl ConnectionFocusItem {
    fn field(self) -> SetupField {
        match self {
            Self::Field(field) => field,
            Self::SecretSource => SetupField::TunnelSecretSource,
        }
    }
}

pub(super) fn connection_focus_items(session: &SetupSession) -> Vec<ConnectionFocusItem> {
    match session.selected_mode() {
        RuntimeMode::Standalone => {
            let mut items = vec![
                ConnectionFocusItem::Field(SetupField::TunnelId),
                ConnectionFocusItem::SecretSource,
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
        Some(ConnectionFocusItem::SecretSource) => Some(match session.standalone().secret_source {
            TunnelSecretSource::File => TunnelSecretSource::Environment,
            TunnelSecretSource::Environment => TunnelSecretSource::File,
        }),
        _ => None,
    }
}

pub(super) fn connection_field_index(session: &SetupSession, field: SetupField) -> Option<usize> {
    connection_focus_items(session)
        .iter()
        .position(|item| item.field() == field)
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
        SetupField::TunnelSecretValue => Some(
            session
                .standalone()
                .secret_value
                .as_ref()
                .map(|secret| secret.expose().to_string())
                .unwrap_or_default(),
        ),
        SetupField::HubUrl => Some(session.hub().hub_url.clone()),
        SetupField::HubTransport => Some(session.hub().hub_transport.clone()),
        SetupField::AgentId => Some(session.hub().agent_id.clone()),
        SetupField::AgentSecret => Some(
            session
                .hub()
                .agent_secret
                .as_ref()
                .map(|secret| secret.expose().to_string())
                .unwrap_or_default(),
        ),
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
    let status_label_width = sections
        .iter()
        .map(|section| UnicodeWidthStr::width(section_label(*section, language)))
        .max()
        .unwrap_or(0);
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
        let label = section_label(section, language);
        let padded_label = format!(
            "{label}{}",
            " ".repeat(status_label_width.saturating_sub(UnicodeWidthStr::width(label)))
        );
        if let Some(row) = next_surface_row(left, &mut cursor) {
            frame.render_widget(
                Paragraph::new(surface_status_line(
                    &padded_label,
                    section_status_label(status, language),
                    focused,
                    status == SectionStatus::NotApplicable,
                    theme,
                )),
                row,
            );
        }
    }
    render_surface_action_dock(
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
            session,
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
            push_ordered_multi_select_group(
                &mut lines,
                &mut focused_line,
                section,
                draft,
                state,
                SetupField::ConfirmationChannels,
                t(language, "Fallback channels", "降级通道"),
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
                session,
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
        OptionalSection::McpServers => push_mcp_servers_form(
            &mut lines,
            &mut focused_line,
            draft,
            state,
            language,
            theme,
            errors,
            left.width,
        ),
        OptionalSection::Room => {
            for field in [
                SetupField::RoomTimezone,
                SetupField::DiaryBoundaryHour,
                SetupField::NotebookRoot,
            ] {
                push_long_form_field(
                    &mut lines,
                    &mut focused_line,
                    session,
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
                    session,
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
                    session,
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
    session: &SetupSession,
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
    let default_draft = default_optional_draft(language, section);
    let default_value = optional_field_value(&default_draft, field);
    let is_default = confirmed == default_value;
    let cursor = editing_cursor(state, field);
    let current = current_input_value(state, field, &confirmed);
    let display_value = if cursor.is_none() && is_default && confirmed.is_empty() {
        match field {
            SetupField::NotebookRoot => {
                let workspace = session.optional_draft(OptionalSection::Workspace);
                let workspace_root = optional_field_value(&workspace, SetupField::WorkspaceRoot);
                PathBuf::from(workspace_root)
                    .join("notebook")
                    .to_string_lossy()
                    .into_owned()
            }
            SetupField::TunnelClientVersion => {
                crate::tunnel_distribution::PINNED_VERSION.to_string()
            }
            _ => current.to_string(),
        }
    } else {
        current.to_string()
    };
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
            &display_value,
            focused,
            cursor.is_some(),
            cursor,
            false,
            is_default,
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
fn push_ordered_multi_select_group(
    lines: &mut Vec<Line<'static>>,
    focused_line: &mut usize,
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    field: SetupField,
    heading: &str,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    language: UiLanguage,
) {
    lines.push(subsection_heading_line(heading, theme));
    let Ok(selection) = optional_multi_select_state(draft, field) else {
        push_optional_error(lines, errors, field, language, theme);
        lines.push(Line::raw(""));
        return;
    };
    let focus_items = optional_focus_items(section, draft);
    for option in selection.options() {
        let focus = focus_items.iter().position(|item| {
            matches!(item, OptionalFocusItem::MultiSelect { field: candidate, value } if *candidate == field && *value == option.as_str())
        });
        let focused = focus == Some(state.focus);
        if focused {
            *focused_line = lines.len();
        }
        lines.push(ordered_multi_select_row_line(
            option,
            focused,
            selection.selection_rank(option),
            18,
            theme,
        ));
    }
    push_optional_error(lines, errors, field, language, theme);
    lines.push(Line::raw(""));
}

#[allow(clippy::too_many_arguments)]
fn push_mcp_servers_form(
    lines: &mut Vec<Line<'static>>,
    focused_line: &mut usize,
    draft: &OptionalSectionDraft,
    state: &TuiState,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    width: u16,
) {
    const LABEL_WIDTH: usize = 10;
    const ID_INPUT_WIDTH: usize = 14;

    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return;
    };
    if mcp.servers.is_empty() {
        if state.focus == 0 {
            *focused_line = lines.len();
        }
        lines.push(action_line(
            t(language, "Add MCP server", "新增 MCP 服务"),
            state.focus == 0,
            theme,
        ));
        lines.push(Line::raw(""));
        return;
    }

    let focus_items = optional_focus_items(OptionalSection::McpServers, draft);
    let max_endpoint_input_width = usize::from(width).saturating_sub(20).clamp(8, 36);
    for (index, server) in mcp.servers.iter().enumerate() {
        let id_focus = focus_items.iter().position(|item| {
            matches!(item, OptionalFocusItem::McpField { field: SetupField::McpServerId, index: candidate } if *candidate == index)
        });
        let id_focused = id_focus == Some(state.focus);
        let id_editing = state
            .mcp_edit
            .as_ref()
            .is_some_and(|target| target.index == index && target.field == SetupField::McpServerId);
        let id_value = if id_editing {
            state
                .editing
                .as_ref()
                .map(|edit| edit.buffer.as_str())
                .unwrap_or(server.id.as_str())
        } else {
            server.id.as_str()
        };
        let heading = if id_value.trim().is_empty() {
            t(language, "New MCP server", "新 MCP 服务")
        } else {
            id_value
        };
        lines.push(subsection_heading_line(heading, theme));

        if id_focused {
            *focused_line = lines.len();
        }
        lines.push(input_row_line(
            "ID",
            id_value,
            id_focused,
            id_editing,
            id_editing
                .then(|| state.editing.as_ref().map(|edit| edit.cursor))
                .flatten(),
            LABEL_WIDTH,
            ID_INPUT_WIDTH,
            theme,
        ));
        if id_focused {
            if let Some(error) = errors.get(&SetupField::McpServerId) {
                lines.push(inline_error_line(&localized_error(error, language), theme));
            }
        }

        let enabled_focus = focus_items.iter().position(|item| {
            matches!(item, OptionalFocusItem::McpField { field: SetupField::McpServerEnabled, index: candidate } if *candidate == index)
        });
        let enabled_focused = enabled_focus == Some(state.focus);
        if enabled_focused {
            *focused_line = lines.len();
        }
        lines.push(value_row_line(
            t(language, "Enabled", "启用"),
            if server.enabled {
                t(language, "on", "开")
            } else {
                t(language, "off", "关")
            },
            enabled_focused,
            LABEL_WIDTH,
            theme,
        ));

        let transport_focus = focus_items.iter().position(|item| {
            matches!(item, OptionalFocusItem::McpField { field: SetupField::McpServerTransport, index: candidate } if *candidate == index)
        });
        let transport_focused = transport_focus == Some(state.focus);
        if transport_focused {
            *focused_line = lines.len();
        }
        lines.push(value_row_line(
            t(language, "Transport", "传输方式"),
            &server.transport,
            transport_focused,
            LABEL_WIDTH,
            theme,
        ));
        if transport_focused {
            if let Some(error) = errors.get(&SetupField::McpServerTransport) {
                lines.push(inline_error_line(&localized_error(error, language), theme));
            }
        }

        let endpoint_focus = focus_items.iter().position(|item| {
            matches!(item, OptionalFocusItem::McpField { field: SetupField::McpServerEndpoint, index: candidate } if *candidate == index)
        });
        let endpoint_focused = endpoint_focus == Some(state.focus);
        let endpoint_editing = state.mcp_edit.as_ref().is_some_and(|target| {
            target.index == index && target.field == SetupField::McpServerEndpoint
        });
        if endpoint_focused {
            *focused_line = lines.len();
        }
        let endpoint_value = if endpoint_editing {
            state
                .editing
                .as_ref()
                .map(|edit| edit.buffer.as_str())
                .unwrap_or(server.endpoint.as_str())
        } else {
            server.endpoint.as_str()
        };
        let endpoint_input_width =
            (UnicodeWidthStr::width(endpoint_value) + usize::from(endpoint_editing)).clamp(
                ID_INPUT_WIDTH.min(max_endpoint_input_width),
                max_endpoint_input_width,
            );
        lines.push(input_row_line(
            if server.transport == "stdio" {
                t(language, "Command", "命令")
            } else {
                "URL"
            },
            endpoint_value,
            endpoint_focused,
            endpoint_editing,
            endpoint_editing
                .then(|| state.editing.as_ref().map(|edit| edit.cursor))
                .flatten(),
            LABEL_WIDTH,
            endpoint_input_width,
            theme,
        ));
        if endpoint_focused {
            if let Some(error) = errors.get(&SetupField::McpServerEndpoint) {
                lines.push(inline_error_line(&localized_error(error, language), theme));
            }
        }

        if server.transport == "streamable-http" {
            let auth_focus = focus_items.iter().position(|item| {
                matches!(item, OptionalFocusItem::McpField { field: SetupField::McpServerBearerAuth, index: candidate } if *candidate == index)
            });
            let auth_focused = auth_focus == Some(state.focus);
            if auth_focused {
                *focused_line = lines.len();
            }
            lines.push(value_row_line(
                t(language, "Auth", "认证"),
                if server.bearer_auth { "bearer" } else { "none" },
                auth_focused,
                LABEL_WIDTH,
                theme,
            ));

            if server.bearer_auth {
                let token_focus = focus_items.iter().position(|item| {
                    matches!(item, OptionalFocusItem::McpField { field: SetupField::McpServerBearerToken, index: candidate } if *candidate == index)
                });
                let token_focused = token_focus == Some(state.focus);
                let token_editing = state.mcp_edit.as_ref().is_some_and(|target| {
                    target.index == index && target.field == SetupField::McpServerBearerToken
                });
                if token_focused {
                    *focused_line = lines.len();
                }
                let token_value = if token_editing {
                    state
                        .editing
                        .as_ref()
                        .map(|edit| edit.buffer.as_str())
                        .unwrap_or_default()
                } else {
                    server
                        .bearer_token
                        .as_ref()
                        .map(|value| value.expose())
                        .unwrap_or_default()
                };
                let masked = "•".repeat(token_value.chars().count());
                lines.push(input_row_line(
                    t(language, "Token", "令牌"),
                    &masked,
                    token_focused,
                    token_editing,
                    token_editing
                        .then(|| state.editing.as_ref().map(|edit| edit.cursor))
                        .flatten(),
                    LABEL_WIDTH,
                    (masked.chars().count() + usize::from(token_editing)).clamp(8, 36),
                    theme,
                ));
                if token_focused {
                    if let Some(error) = errors.get(&SetupField::McpServerBearerToken) {
                        lines.push(inline_error_line(&localized_error(error, language), theme));
                    }
                }
            }
        }
        lines.push(Line::raw(""));
    }
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
    lines.push(subsection_heading_line(
        optional_field_label(SetupField::MaxActiveJobs, language),
        theme,
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
        t(language, "File search", "文件搜索"),
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
    if section == OptionalSection::McpServers {
        if let Some(target) = optional_mcp_target(section, draft, state.focus) {
            let action = match target {
                McpFocusTarget::Add => t(language, "add", "新增"),
                McpFocusTarget::Field {
                    field:
                        SetupField::McpServerEnabled
                        | SetupField::McpServerTransport
                        | SetupField::McpServerBearerAuth,
                    ..
                } => t(language, "toggle", "切换"),
                McpFocusTarget::Field { .. } => t(language, "edit", "编辑"),
            };
            let mut bindings = vec![
                ("↑↓ j/k", t(language, "move", "移动")),
                ("Enter/l", action),
                ("a", t(language, "add", "新增")),
            ];
            if !matches!(target, McpFocusTarget::Add) {
                bindings.push(("d", t(language, "delete", "删除")));
            }
            bindings.push(("Esc/h", t(language, "back", "返回")));
            render_contextual_footer(frame, area, &bindings, theme);
            return;
        }
    }
    if optional_multi_select_for_focus(section, draft, state.focus).is_some() {
        render_contextual_footer(
            frame,
            area,
            &[
                ("↑↓ j/k", t(language, "move", "移动")),
                ("Enter/Space", t(language, "select / clear", "选择 / 取消")),
                ("J/K", t(language, "change priority", "调整优先级")),
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
    MultiSelect {
        field: SetupField,
        value: &'static str,
    },
    List {
        field: SetupField,
        index: Option<usize>,
    },
    McpField {
        field: SetupField,
        index: usize,
    },
    McpAdd,
}

impl OptionalFocusItem {
    fn field(self) -> SetupField {
        match self {
            Self::Field(field)
            | Self::Choice { field, .. }
            | Self::MultiSelect { field, .. }
            | Self::List { field, .. }
            | Self::McpField { field, .. } => field,
            Self::McpAdd => SetupField::McpServerId,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum McpFocusTarget {
    Add,
    Field { index: usize, field: SetupField },
}

pub(super) fn multi_select_options(field: SetupField) -> &'static [&'static str] {
    match field {
        SetupField::ConfirmationChannels => &["freedesktop", "ntfy"],
        _ => &[],
    }
}

pub(super) fn optional_multi_select_state(
    draft: &OptionalSectionDraft,
    field: SetupField,
) -> Result<OrderedMultiSelectState, ()> {
    let raw = match (draft, field) {
        (OptionalSectionDraft::Confirmation(value), SetupField::ConfirmationChannels) => {
            &value.channels
        }
        _ => return Err(()),
    };
    let selected = serde_json::from_str::<Vec<String>>(raw).map_err(|_| ())?;
    Ok(OrderedMultiSelectState::new(
        multi_select_options(field)
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        selected,
    ))
}

pub(super) fn set_optional_multi_select_state(
    draft: &mut OptionalSectionDraft,
    field: SetupField,
    state: &OrderedMultiSelectState,
) -> bool {
    let Ok(serialized) = serde_json::to_string(state.selected()) else {
        return false;
    };
    match (draft, field) {
        (OptionalSectionDraft::Confirmation(value), SetupField::ConfirmationChannels) => {
            value.channels = serialized;
            true
        }
        _ => false,
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
        OptionalSection::Confirmation => multi_select_options(SetupField::ConfirmationChannels)
            .iter()
            .copied()
            .map(|value| OptionalFocusItem::MultiSelect {
                field: SetupField::ConfirmationChannels,
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
        OptionalSection::McpServers => match draft {
            OptionalSectionDraft::McpServers(value) if value.servers.is_empty() => {
                vec![OptionalFocusItem::McpAdd]
            }
            OptionalSectionDraft::McpServers(value) => value
                .servers
                .iter()
                .enumerate()
                .flat_map(|(index, server)| {
                    let mut items = vec![
                        OptionalFocusItem::McpField {
                            field: SetupField::McpServerId,
                            index,
                        },
                        OptionalFocusItem::McpField {
                            field: SetupField::McpServerEnabled,
                            index,
                        },
                        OptionalFocusItem::McpField {
                            field: SetupField::McpServerTransport,
                            index,
                        },
                        OptionalFocusItem::McpField {
                            field: SetupField::McpServerEndpoint,
                            index,
                        },
                    ];
                    if server.transport == "streamable-http" {
                        items.push(OptionalFocusItem::McpField {
                            field: SetupField::McpServerBearerAuth,
                            index,
                        });
                        if server.bearer_auth {
                            items.push(OptionalFocusItem::McpField {
                                field: SetupField::McpServerBearerToken,
                                index,
                            });
                        }
                    }
                    items
                })
                .collect(),
            _ => Vec::new(),
        },
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

pub(super) fn optional_mcp_target(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<McpFocusTarget> {
    if section != OptionalSection::McpServers {
        return None;
    }
    match optional_focus_items(section, draft).get(focus).copied()? {
        OptionalFocusItem::McpAdd => Some(McpFocusTarget::Add),
        OptionalFocusItem::McpField { field, index } => {
            Some(McpFocusTarget::Field { index, field })
        }
        _ => None,
    }
}

pub(super) fn mcp_server_value(
    draft: &OptionalSectionDraft,
    index: usize,
    field: SetupField,
) -> Option<String> {
    let OptionalSectionDraft::McpServers(value) = draft else {
        return None;
    };
    let server = value.servers.get(index)?;
    match field {
        SetupField::McpServerId => Some(server.id.clone()),
        SetupField::McpServerEnabled => Some(server.enabled.to_string()),
        SetupField::McpServerTransport => Some(server.transport.clone()),
        SetupField::McpServerEndpoint => Some(server.endpoint.clone()),
        SetupField::McpServerBearerAuth => Some(server.bearer_auth.to_string()),
        SetupField::McpServerBearerToken => Some(
            server
                .bearer_token
                .as_ref()
                .map(|value| value.expose().to_string())
                .unwrap_or_default(),
        ),
        _ => None,
    }
}

pub(super) fn set_mcp_server_value(
    draft: &mut OptionalSectionDraft,
    index: usize,
    field: SetupField,
    value: String,
) -> bool {
    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return false;
    };
    let Some(server) = mcp.servers.get_mut(index) else {
        return false;
    };
    match field {
        SetupField::McpServerId => server.id = value,
        SetupField::McpServerEndpoint => server.endpoint = value,
        SetupField::McpServerTransport => server.transport = value,
        SetupField::McpServerEnabled => server.enabled = value == "true",
        SetupField::McpServerBearerAuth => server.bearer_auth = value == "true",
        SetupField::McpServerBearerToken => {
            server.bearer_token = if value.is_empty() {
                None
            } else {
                Some(crate::config_templates::SecretValue::new(value))
            }
        }
        _ => return false,
    }
    true
}

pub(super) fn toggle_mcp_server_enabled(draft: &mut OptionalSectionDraft, index: usize) -> bool {
    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return false;
    };
    let Some(server) = mcp.servers.get_mut(index) else {
        return false;
    };
    server.enabled = !server.enabled;
    true
}

pub(super) fn toggle_mcp_server_transport(draft: &mut OptionalSectionDraft, index: usize) -> bool {
    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return false;
    };
    let Some(server) = mcp.servers.get_mut(index) else {
        return false;
    };
    server.transport = if server.transport == "stdio" {
        "streamable-http".to_string()
    } else {
        server.bearer_auth = false;
        server.bearer_token = None;
        "stdio".to_string()
    };
    true
}

pub(super) fn toggle_mcp_server_bearer_auth(
    draft: &mut OptionalSectionDraft,
    index: usize,
) -> bool {
    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return false;
    };
    let Some(server) = mcp.servers.get_mut(index) else {
        return false;
    };
    if server.transport != "streamable-http" {
        return false;
    }
    server.bearer_auth = !server.bearer_auth;
    if !server.bearer_auth {
        server.bearer_token = None;
    }
    true
}

pub(super) fn mcp_server_review_fields(
    draft: &OptionalSectionDraft,
    index: usize,
) -> Vec<SetupField> {
    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return Vec::new();
    };
    let Some(server) = mcp.servers.get(index) else {
        return Vec::new();
    };
    let mut fields = vec![
        SetupField::McpServerId,
        SetupField::McpServerEnabled,
        SetupField::McpServerTransport,
        SetupField::McpServerEndpoint,
    ];
    if server.transport == "streamable-http" {
        fields.push(SetupField::McpServerBearerAuth);
        if server.bearer_auth {
            fields.push(SetupField::McpServerBearerToken);
        }
    }
    fields
}

pub(super) fn add_mcp_server(
    draft: &mut OptionalSectionDraft,
    after: Option<usize>,
) -> Option<usize> {
    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return None;
    };
    let index = after
        .map(|index| index.saturating_add(1).min(mcp.servers.len()))
        .unwrap_or(mcp.servers.len());
    mcp.servers.insert(
        index,
        McpServerDraft {
            id: String::new(),
            enabled: true,
            transport: "streamable-http".to_string(),
            endpoint: String::new(),
            bearer_auth: false,
            bearer_token: None,
        },
    );
    Some(index)
}

pub(super) fn remove_mcp_server(draft: &mut OptionalSectionDraft, index: usize) -> bool {
    let OptionalSectionDraft::McpServers(mcp) = draft else {
        return false;
    };
    if index >= mcp.servers.len() {
        return false;
    }
    mcp.servers.remove(index);
    true
}

pub(super) fn mcp_server_focus_index(
    draft: &OptionalSectionDraft,
    index: usize,
    field: SetupField,
) -> Option<usize> {
    optional_focus_items(OptionalSection::McpServers, draft)
        .iter()
        .position(|item| matches!(item, OptionalFocusItem::McpField { field: candidate, index: candidate_index } if *candidate == field && *candidate_index == index))
}

pub(super) fn mcp_server_index_for_focus(
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<usize> {
    match optional_mcp_target(OptionalSection::McpServers, draft, focus)? {
        McpFocusTarget::Field { index, .. } => Some(index),
        McpFocusTarget::Add => None,
    }
}

pub(super) fn mcp_server_count(draft: &OptionalSectionDraft) -> usize {
    match draft {
        OptionalSectionDraft::McpServers(value) => value.servers.len(),
        _ => 0,
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

pub(super) fn optional_multi_select_for_focus(
    section: OptionalSection,
    draft: &OptionalSectionDraft,
    focus: usize,
) -> Option<(SetupField, &'static str)> {
    match optional_focus_items(section, draft).get(focus).copied() {
        Some(OptionalFocusItem::MultiSelect { field, value }) => Some((field, value)),
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
    let default_workspace = default_optional_draft(language, OptionalSection::Workspace);
    let default_root = optional_field_value(&default_workspace, SetupField::WorkspaceRoot);
    lines.push(long_form_input_value_line(
        current_input_value(state, SetupField::WorkspaceRoot, &root_value),
        root_focused,
        editing_cursor(state, SetupField::WorkspaceRoot).is_some(),
        editing_cursor(state, SetupField::WorkspaceRoot),
        false,
        root_value == default_root,
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
                OptionalSection::Identity => &[
                    "Set the human-readable name shown for this agent.",
                    "The display name is separate from the Agent ID used for routing.",
                ],
                OptionalSection::Workspace => &[
                    "Set the default working directory and the paths file tools may write, read only, or not access.",
                    "",
                    "Access priority:",
                    "• deny > write > read-only",
                    "• workspace root is always writable",
                ],
                OptionalSection::Confirmation => &[
                    "Choose how confirmation requests are delivered and which language they use.",
                    "",
                    "Default channel order:",
                    "• freedesktop → ntfy",
                ],
                OptionalSection::Limits => &[
                    "Control Process batch concurrency, total active Job capacity, and file-search context.",
                    "",
                    "Defaults:",
                    "• Process batch concurrency: 2",
                    "• Active Jobs: auto",
                    "• File-search context: 5 lines",
                ],
                OptionalSection::Sandbox => &[
                    "Add bubblewrap isolation to process execution.",
                    "Path policy and confirmation checks still apply whether the sandbox is on or off.",
                    "",
                    "Default: off",
                ],
                OptionalSection::McpServers => &[
                    "Configure downstream MCP servers used by mcp.listTools, mcp.callTool, and mcp.batch.",
                    "",
                    "Supported transports:",
                    "• streamable-http",
                    "• stdio",
                ],
                OptionalSection::Room => &[
                    "Set Room timezone, diary day boundary, and Notebook storage location.",
                    "Only available when the Room profile is selected.",
                ],
                OptionalSection::TunnelClient => &[
                    "Configure how Standalone locates or downloads the Tunnel client.",
                    "Default: managed version 0.0.10 with auto-download enabled.",
                ],
                OptionalSection::HubReporting => &[
                    "Control optional run and Job reporting from Standalone to the Hub.",
                    "Default: off; metadata hides tool arguments, results, and command/output details.",
                ],
            },
            UiLanguage::ZhCn => match section {
                OptionalSection::Identity => &[
                    "设置此 Agent 的可读显示名称。",
                    "显示名称与用于路由的 Agent ID 相互独立。",
                ],
                OptionalSection::Workspace => &[
                    "设置默认工作目录，以及文件工具允许写入、只读和禁止访问的路径范围。",
                    "",
                    "权限优先级：",
                    "• deny > write > read-only",
                    "• workspace root 始终可写",
                ],
                OptionalSection::Confirmation => &[
                    "设置确认请求的投递通道和语言。",
                    "",
                    "默认通道顺序：",
                    "• freedesktop → ntfy",
                ],
                OptionalSection::Limits => &[
                    "控制 Process 批处理并发、活动 Job 总容量和文件搜索上下文。",
                    "",
                    "默认值：",
                    "• Process 批处理并发：2",
                    "• 活动 Job：auto",
                    "• 文件搜索上下文：5 行",
                ],
                OptionalSection::Sandbox => &[
                    "为进程执行增加 bubblewrap 隔离。",
                    "无论是否启用沙箱，路径策略和确认检查仍然生效。",
                    "",
                    "默认：关闭",
                ],
                OptionalSection::McpServers => &[
                    "配置 mcp.listTools、mcp.callTool 和 mcp.batch 使用的下游 MCP 服务。",
                    "",
                    "支持的传输：",
                    "• streamable-http",
                    "• stdio",
                ],
                OptionalSection::Room => &[
                    "设置 Room 的时区、日记日界线和 Notebook 存储位置。",
                    "仅在选择 Room profile 时可用。",
                ],
                OptionalSection::TunnelClient => &[
                    "设置 Standalone 如何查找或下载 Tunnel client。",
                    "默认使用托管版本 0.0.10，并开启自动下载。",
                ],
                OptionalSection::HubReporting => &[
                    "控制 Standalone 向 Hub 上报运行和 Job 信息。",
                    "默认关闭；metadata 会隐藏工具参数、结果以及命令和输出细节。",
                ],
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
                SetupField::DisplayName => &[
                    "Human-readable name shown in status and notifications; it does not change the Agent ID.",
                    "",
                    "Default behavior:",
                    "• untouched: use the host name when available",
                    "• saved default draft: write AgenticGPT agent",
                ],
                SetupField::WorkspaceRoot => &[
                    "Default working directory for relative paths and workspace-scoped data.",
                    "Default: ~/.agentic_gpt/workspace.",
                    "",
                    "Access rule:",
                    "• workspace root is always writable",
                ],
                SetupField::WriteRoots => &[
                    "Additional roots file tools may modify.",
                    "Default: workspace, ~/Documents, ~/Downloads, /tmp.",
                    "",
                    "Workspace root remains writable even if it is removed from this list.",
                ],
                SetupField::ReadOnlyRoots => &[
                    "Roots file tools may read but not modify.",
                    "",
                    "Access priority:",
                    "• write overrides read-only",
                    "• deny overrides both",
                ],
                SetupField::DenyRoots => &[
                    "Roots file tools cannot access.",
                    "Default includes common credential, browser, cloud, and package-manager locations.",
                    "",
                    "Access priority:",
                    "• deny overrides write and read-only",
                ],
                SetupField::ConfirmationChannels => &[
                    "Choose confirmation fallback channels.",
                    "Selected order is the fallback order: the first available channel handles the request.",
                    "",
                    "An empty selection means required confirmations cannot be delivered.",
                ],
                SetupField::ConfirmationLanguage => &[
                    "Language used when confirmation requests are presented to the user.",
                    "If this section stays Default, final config follows the current Config TUI interface language.",
                ],
                SetupField::MaxConcurrentTasks => &[
                    "Maximum number of child Process Jobs from one batch that may run at once.",
                    "Default: 2. A configured 0 still runs one child at a time.",
                    "",
                    "This does not raise the total active Job capacity controlled by Max active jobs.",
                ],
                SetupField::MaxActiveJobs => &[
                    "Total number of managed Jobs that may be active at once.",
                    "Default: auto.",
                    "",
                    "Auto behavior:",
                    "• ceil(available CPU core count × 1.5)",
                    "• clamped to 6–24",
                    "• explicit 0 rejects new Jobs",
                ],
                SetupField::MaxFileSearchContextLines => &[
                    "Number of surrounding lines included for each file-search hit.",
                    "Default: 5; valid range: 0–100.",
                ],
                SetupField::SandboxEnabled => &[
                    "Wrap process execution with bubblewrap isolation when enabled.",
                    "Default: off. Path policy and confirmation checks still apply either way.",
                ],
                SetupField::BubblewrapPath => &[
                    "Command or path used to launch bubblewrap.",
                    "Default: bwrap. This wizard checks only that the value is non-empty.",
                ],
                SetupField::RequiredRuntimePaths => &[
                    "Host paths mounted read-only into the sandbox so programs can run.",
                    "Default: /usr, /bin, /lib, /lib64, /etc/ssl.",
                    "",
                    "Paths that do not exist on the host are skipped.",
                ],
                SetupField::McpServerId => &[
                    "Stable server ID used in MCP requests, confirmations, Jobs, and audit records.",
                    "",
                    "Rules:",
                    "• 1–64 bytes and unique",
                    "• ASCII letters, digits, dot, underscore, or dash",
                    "• no leading or trailing whitespace",
                ],
                SetupField::McpServerEnabled => &[
                    "Controls whether this configured MCP server may be called.",
                    "Disabled servers remain saved and listed, but calls to them fail as disabled.",
                ],
                SetupField::McpServerTransport => &[
                    "Select how AgenticGPT connects to this MCP server.",
                    "",
                    "Supported transports:",
                    "• streamable-http: Endpoint is an HTTP(S) URL",
                    "• stdio: Endpoint is a shell command",
                ],
                SetupField::McpServerEndpoint => &[
                    "Connection target interpreted according to the selected transport.",
                    "",
                    "Requirements:",
                    "• streamable-http: absolute HTTP(S) URL with a host",
                    "• stdio: full shell command executed with sh -lc",
                    "• stdio shell quoting, expansion, and composition apply",
                ],
                SetupField::McpServerBearerAuth => &[
                    "Optional Bearer authentication for streamable-http MCP servers.",
                    "When enabled, AgenticGPT sends Authorization: Bearer <token> on each MCP HTTP request.",
                    "Switching the server to stdio clears this setting.",
                ],
                SetupField::McpServerBearerToken => &[
                    "Bearer token for this streamable-http MCP server.",
                    "Enter the token without the Bearer prefix.",
                    "The TUI masks this value and redacts it from the final JSON preview.",
                ],
                SetupField::RoomTimezone => &[
                    "Timezone used to assign Room diary and Notebook dates.",
                    "Default: Asia/Shanghai. Use an IANA timezone such as Europe/Berlin.",
                    "",
                    "The wizard checks only that it is non-empty; an invalid timezone fails at runtime.",
                ],
                SetupField::DiaryBoundaryHour => &[
                    "Hour at which the Room diary date rolls over.",
                    "Default: 5; valid range: 0–23.",
                    "",
                    "Times before the boundary belong to the previous diary day.",
                ],
                SetupField::NotebookRoot => &[
                    "Directory used by Room Notebook storage.",
                    "Leave blank to use <workspace>/notebook.",
                ],
                SetupField::TunnelClientVersion => &[
                    "Managed Tunnel client version.",
                    "Leave blank to use the built-in pinned version 0.0.10.",
                ],
                SetupField::TunnelCacheDir => &[
                    "Cache directory for managed Tunnel client artifacts.",
                    "Default: ~/.agentic_gpt/cache/tunnel-client.",
                ],
                SetupField::TunnelAutoDownload => &[
                    "Allow AgenticGPT to download the managed Tunnel client when the cache is missing or invalid.",
                    "Default: on.",
                    "",
                    "When off, a valid cached client or executable override must already be available.",
                ],
                SetupField::TunnelExecutable => &[
                    "Optional custom Tunnel client executable.",
                    "When set, it takes precedence over the managed cache and download path.",
                    "",
                    "Leave blank to use the managed client.",
                ],
                SetupField::TunnelDownloadUrl => &[
                    "Override the managed Tunnel client download source.",
                    "Usually leave this blank.",
                    "",
                    "A custom source must use HTTPS and requires a SHA-256 value.",
                ],
                SetupField::TunnelSha256 => &[
                    "Expected SHA-256 for a custom download or executable.",
                    "It must contain exactly 64 hexadecimal characters.",
                    "",
                    "A custom download URL requires this value.",
                ],
                SetupField::HubReportingEnabled => &[
                    "Enable reporting of Standalone Tunnel runs and Jobs to the Hub.",
                    "Default: off.",
                ],
                SetupField::HubReportingDetail => &[
                    "Choose how much data Hub reporting includes.",
                    "",
                    "Levels:",
                    "• metadata: hide tool arguments/results and command, cwd, stdout/stderr details",
                    "• full: include bounded arguments/results and full Job details",
                ],
                _ => &["Edit the staged value; validation remains authoritative."],
            },
            UiLanguage::ZhCn => match field {
                SetupField::DisplayName => &[
                    "显示在状态信息和通知里的可读名称；不会修改 Agent ID。",
                    "",
                    "默认行为：",
                    "• 未配置：优先使用主机名",
                    "• 保存默认草稿：写入 AgenticGPT agent",
                ],
                SetupField::WorkspaceRoot => &[
                    "相对路径和工作区数据使用的默认目录。",
                    "默认 ~/.agentic_gpt/workspace。",
                    "",
                    "访问规则：",
                    "• workspace root 始终可写",
                ],
                SetupField::WriteRoots => &[
                    "文件工具允许修改的额外根目录。",
                    "默认包含 workspace、~/Documents、~/Downloads 和 /tmp。",
                    "",
                    "即使从此列表移除，workspace root 仍保持可写。",
                ],
                SetupField::ReadOnlyRoots => &[
                    "文件工具可以读取、但不能修改的根目录。",
                    "",
                    "权限优先级：",
                    "• write 覆盖 read-only",
                    "• deny 覆盖两者",
                ],
                SetupField::DenyRoots => &[
                    "文件工具禁止访问的根目录。",
                    "默认包含常见的凭据、浏览器、云服务和包管理器配置位置。",
                    "",
                    "权限优先级：",
                    "• deny 覆盖 write 和 read-only",
                ],
                SetupField::ConfirmationChannels => &[
                    "选择确认请求的降级通道。",
                    "选中顺序就是降级顺序：按顺序尝试，首个可用通道处理请求。",
                    "",
                    "全部取消表示需要确认的操作无法投递确认。",
                ],
                SetupField::ConfirmationLanguage => &[
                    "向用户展示确认请求时使用的语言。",
                    "如果此区块保持 Default，最终配置会跟随当前 Config TUI 的界面语言。",
                ],
                SetupField::MaxConcurrentTasks => &[
                    "单个 Process 批处理中，同时运行的子 Job 数上限。",
                    "默认 2；即使配置为 0，运行时仍会按 1 个并发处理。",
                    "",
                    "它不会提高 Max active jobs 控制的活动 Job 总容量。",
                ],
                SetupField::MaxActiveJobs => &[
                    "允许同时处于活动状态的托管 Job 总数。",
                    "默认 auto。",
                    "",
                    "Auto 行为：",
                    "• ceil(可用 CPU 核心数量 × 1.5)",
                    "• 结果限制在 6–24",
                    "• 显式设为 0 会拒绝新的 Job",
                ],
                SetupField::MaxFileSearchContextLines => &[
                    "每个文件搜索命中项附带的上下文行数。",
                    "默认 5；有效范围 0–100。",
                ],
                SetupField::SandboxEnabled => &[
                    "开启后使用 bubblewrap 隔离进程执行。",
                    "默认关闭；无论是否开启，路径策略和确认检查仍然生效。",
                ],
                SetupField::BubblewrapPath => &[
                    "启动 bubblewrap 使用的命令名或路径。",
                    "默认 bwrap；此向导只检查它是否非空，不检查可执行文件是否存在。",
                ],
                SetupField::RequiredRuntimePaths => &[
                    "以只读方式挂载进沙箱、供程序运行所需的宿主机路径。",
                    "默认 /usr、/bin、/lib、/lib64、/etc/ssl。",
                    "",
                    "宿主机上不存在的路径会跳过。",
                ],
                SetupField::McpServerId => &[
                    "MCP 请求、确认、Job 和审计记录中使用的稳定服务 ID。",
                    "",
                    "规则：",
                    "• 1–64 字节且不能重复",
                    "• 仅 ASCII 字母、数字、点、下划线和短横线",
                    "• 首尾不能有空白",
                ],
                SetupField::McpServerEnabled => &[
                    "控制此 MCP 服务是否允许被调用。",
                    "关闭后配置仍会保留并显示，但调用会以 disabled 失败。",
                ],
                SetupField::McpServerTransport => &[
                    "选择 AgenticGPT 连接此 MCP 服务的方式。",
                    "",
                    "支持的传输：",
                    "• streamable-http：Endpoint 是 HTTP(S) URL",
                    "• stdio：Endpoint 是 shell 命令",
                ],
                SetupField::McpServerEndpoint => &[
                    "连接目标，其含义取决于当前 transport。",
                    "",
                    "要求：",
                    "• streamable-http：带 host 的绝对 HTTP(S) URL",
                    "• stdio：通过 sh -lc 执行的完整 shell 命令",
                    "• stdio 会应用 shell 的引用、展开和命令组合语义",
                ],
                SetupField::McpServerBearerAuth => &[
                    "streamable-http MCP 服务可选的 Bearer 认证。",
                    "启用后，AgenticGPT 会在每个 MCP HTTP 请求中发送 Authorization: Bearer <token>。",
                    "切换到 stdio 时会清除此设置。",
                ],
                SetupField::McpServerBearerToken => &[
                    "此 streamable-http MCP 服务使用的 Bearer token。",
                    "只填写 token 本身，不要带 Bearer 前缀。",
                    "TUI 会掩码该值，并在最终 JSON 预览中脱敏。",
                ],
                SetupField::RoomTimezone => &[
                    "Room 日记和 Notebook 日期归属使用的时区。",
                    "默认 Asia/Shanghai；建议使用 Europe/Berlin 这类 IANA 时区。",
                    "",
                    "向导只检查非空；无效时区会在运行时失败。",
                ],
                SetupField::DiaryBoundaryHour => &[
                    "Room 日记切换日期的小时。",
                    "默认 5；有效范围 0–23。",
                    "",
                    "边界时间之前的内容归到前一个日记日。",
                ],
                SetupField::NotebookRoot => &[
                    "Room Notebook 的存储目录。",
                    "留空时使用 <workspace>/notebook。",
                ],
                SetupField::TunnelClientVersion => &[
                    "Tunnel client 的托管版本。",
                    "留空时使用内置固定版本 0.0.10。",
                ],
                SetupField::TunnelCacheDir => &[
                    "托管 Tunnel client 文件的缓存目录。",
                    "默认 ~/.agentic_gpt/cache/tunnel-client。",
                ],
                SetupField::TunnelAutoDownload => &[
                    "缓存缺失或无效时，是否允许 AgenticGPT 自动下载托管 Tunnel client。",
                    "默认开启。",
                    "",
                    "关闭后，必须已有有效缓存或提供 executable override。",
                ],
                SetupField::TunnelExecutable => &[
                    "可选的自定义 Tunnel client 可执行文件。",
                    "设置后优先于托管缓存和下载流程。",
                    "",
                    "留空时使用托管 client。",
                ],
                SetupField::TunnelDownloadUrl => &[
                    "覆盖 Tunnel client 的默认下载来源。",
                    "通常无需设置。",
                    "",
                    "自定义来源必须使用 HTTPS，并同时提供 SHA-256。",
                ],
                SetupField::TunnelSha256 => &[
                    "自定义下载包或可执行文件的预期 SHA-256。",
                    "必须正好是 64 个十六进制字符。",
                    "",
                    "设置自定义下载 URL 时此项必填。",
                ],
                SetupField::HubReportingEnabled => &[
                    "是否把 Standalone Tunnel 的运行和 Job 信息上报到 Hub。",
                    "默认关闭。",
                ],
                SetupField::HubReportingDetail => &[
                    "选择 Hub 上报包含的信息量。",
                    "",
                    "级别：",
                    "• metadata：隐藏工具参数/结果，以及命令、cwd、stdout/stderr 细节",
                    "• full：包含受大小限制的参数/结果和完整 Job 细节",
                ],
                _ => &["编辑暂存值；验证逻辑保持不变。"],
            },
        },
        None => optional_center_inspector_body(Some(section), language),
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

fn all_optional_sections() -> [OptionalSection; 9] {
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
}

fn section_label(section: OptionalSection, language: UiLanguage) -> &'static str {
    match section {
        OptionalSection::Identity => t(language, "Identity", "身份"),
        OptionalSection::Workspace => t(language, "Workspace", "工作区"),
        OptionalSection::Confirmation => t(language, "Confirmation", "确认"),
        OptionalSection::Limits => t(language, "Limits", "限制"),
        OptionalSection::Sandbox => t(language, "Sandbox", "沙箱"),
        OptionalSection::McpServers => t(language, "MCP servers", "MCP 服务"),
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
        SetupField::ConfirmationChannels => t(language, "Fallback channels", "降级通道"),
        SetupField::ConfirmationLanguage => t(language, "Language", "语言"),
        SetupField::MaxConcurrentTasks => t(language, "Max concurrent tasks", "最大并发任务"),
        SetupField::MaxActiveJobs => t(language, "Max active jobs", "最大活动作业"),
        SetupField::MaxFileSearchContextLines => {
            t(language, "File-search context lines", "文件搜索上下文行数")
        }
        SetupField::SandboxEnabled => t(language, "Sandbox enabled", "启用沙箱"),
        SetupField::BubblewrapPath => t(language, "Bubblewrap path", "Bubblewrap 路径"),
        SetupField::RequiredRuntimePaths => t(language, "Required runtime paths", "必需运行时路径"),
        SetupField::McpServerId => t(language, "MCP server ID", "MCP 服务 ID"),
        SetupField::McpServerEnabled => t(language, "Enabled", "启用"),
        SetupField::McpServerTransport => t(language, "Transport", "传输方式"),
        SetupField::McpServerEndpoint => t(language, "URL / command", "URL / 命令"),
        SetupField::McpServerBearerAuth => t(language, "Bearer auth", "Bearer 认证"),
        SetupField::McpServerBearerToken => t(language, "Bearer token", "Bearer 令牌"),
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
            SetupField::ConfirmationChannels => value.channels.clone(),
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
        OptionalSectionDraft::McpServers(_) => String::new(),
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
            SetupField::ConfirmationChannels => draft.channels = value,
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

#[allow(clippy::too_many_arguments)]
fn render_review(
    frame: &mut Frame,
    review: &ReviewModel,
    _session: &SetupSession,
    state: &TuiState,
    language: UiLanguage,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    section_draft: Option<&OptionalSectionDraft>,
    progress: (usize, usize),
) {
    let [header, top_rule, body, bottom_rule, footer] = surface_shell_areas(frame.area());
    render_surface_header(
        frame,
        header,
        t(language, "Review and write", "检查并写入"),
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    render_horizontal_rule(frame, top_rule, theme);
    let [left, _, right] = surface_columns(body);

    let row_count = review.row_count();
    let action_focused = state.focus >= row_count;
    let groups = review.groups();
    let label_width = groups
        .iter()
        .flat_map(|group| group.items.iter())
        .map(|item| UnicodeWidthStr::width(review_item_label(item.label_key, language)))
        .max()
        .unwrap_or(0)
        .min(24);

    let mut lines = vec![Line::from(vec![
        Span::raw("  "),
        Span::styled(t(language, "Config path: ", "配置路径："), theme.dim),
        Span::styled(review.config_path.display().to_string(), theme.normal),
    ])];
    if review.will_backup_existing_config {
        let backup_path = review
            .config_path
            .parent()
            .map(|parent| parent.join("backups").join("config.<timestamp>.json"))
            .unwrap_or_else(|| PathBuf::from("backups/config.<timestamp>.json"));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(t(language, "Backup path: ", "备份路径："), theme.dim),
            Span::styled(backup_path.display().to_string(), theme.normal),
        ]));
    }
    if state.review_search_active || !state.review_search.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(t(language, "Search: ", "搜索："), theme.dim),
            Span::styled(format!("/{}", state.review_search), theme.emphasis),
        ]));
    }
    lines.push(Line::raw(""));

    const REVIEW_MIN_VALUE_WIDTH: usize = 14;
    let max_value_width = usize::from(left.width)
        .saturating_sub(label_width + 8)
        .clamp(8, 36);
    let min_value_width = REVIEW_MIN_VALUE_WIDTH.min(max_value_width);

    let mut row_line_indices = Vec::with_capacity(row_count);
    let mut mcp_create_line = None;
    let mut row_index = 0usize;
    for group in groups {
        let visible_status = match (group.target, section_draft) {
            (ReviewTarget::OptionalSection(section), Some(draft)) if draft.section() == section => {
                if *draft == default_optional_draft(language, section) {
                    SectionStatus::Default
                } else {
                    SectionStatus::Configured
                }
            }
            _ => group.status,
        };
        lines.push(Line::from(vec![
            Span::styled("◆ ", theme.emphasis),
            Span::styled(review_target_label(group.target, language), theme.emphasis),
            Span::styled(
                format!("  [{}]", section_status_label(visible_status, language)),
                theme.muted,
            ),
        ]));
        if state.review_mcp_create
            && group.target == ReviewTarget::OptionalSection(OptionalSection::McpServers)
            && section_draft.is_some()
        {
            mcp_create_line = Some(lines.len());
            let synthetic = ReviewItem {
                field: None,
                label_key: "mcp_server",
                value: String::new(),
                editor: ReviewEditorKind::Compound,
                target: crate::config_setup::ReviewItemTarget::Static,
            };
            lines.extend(review_complex_lines(
                &synthetic,
                state,
                section_draft,
                errors,
                language,
                left.width,
                theme,
            ));
        }
        for item in &group.items {
            let focused = state.focus == row_index;
            let nested_active = focused
                && section_draft.is_some()
                && matches!(
                    item.editor,
                    ReviewEditorKind::MultiSelect
                        | ReviewEditorKind::List
                        | ReviewEditorKind::Compound
                        | ReviewEditorKind::AutoCustom
                );
            row_line_indices.push(lines.len());
            let label = review_item_label(item.label_key, language);
            let label_padding =
                " ".repeat(label_width.saturating_sub(UnicodeWidthStr::width(label)));
            let editing = focused
                .then_some(())
                .and(state.editing.as_ref())
                .filter(|editing| item.field == Some(editing.field));
            let display_value = if let Some(editing) = editing {
                if item.editor == ReviewEditorKind::Secret {
                    "•".repeat(editing.buffer.chars().count())
                } else {
                    editing.buffer.clone()
                }
            } else {
                review_display_value(_session, group, item)
            };
            let inline_editing = editing.is_some()
                && matches!(
                    item.editor,
                    ReviewEditorKind::Text | ReviewEditorKind::Secret
                );
            let value_width = UnicodeWidthStr::width(display_value.as_str())
                .saturating_add(usize::from(inline_editing))
                .clamp(min_value_width, max_value_width);
            let mut row_spans = vec![
                if focused {
                    Span::styled(
                        "❯ ",
                        if nested_active {
                            theme.emphasis
                        } else {
                            theme.pointer
                        },
                    )
                } else {
                    Span::raw("  ")
                },
                Span::styled(
                    format!("{label}{label_padding}"),
                    if nested_active {
                        theme.emphasis
                    } else if focused {
                        theme.selected
                    } else {
                        theme.dim
                    },
                ),
                Span::raw("  "),
            ];
            row_spans.extend(inline_input_spans(
                &display_value,
                inline_editing,
                editing.map(|editing| editing.cursor),
                value_width,
                theme,
            ));
            lines.push(Line::from(row_spans));

            if focused {
                if let Some(editing) = editing {
                    if item.editor == ReviewEditorKind::Choice && item.choice_values().len() > 2 {
                        for choice in item.choice_values() {
                            lines.push(choice_row_line(
                                choice,
                                editing.buffer == *choice,
                                item.value.eq_ignore_ascii_case(choice),
                                0,
                                theme,
                            ));
                        }
                    }
                }
                if !matches!(
                    item.editor,
                    ReviewEditorKind::MultiSelect
                        | ReviewEditorKind::List
                        | ReviewEditorKind::Compound
                        | ReviewEditorKind::AutoCustom
                ) {
                    if let Some(field) = item.field {
                        if let Some(error) = errors.get(&field) {
                            lines.push(inline_error_line(&localized_error(error, language), theme));
                        }
                    }
                }
            }
            if focused
                && section_draft.is_some()
                && matches!(
                    item.editor,
                    ReviewEditorKind::MultiSelect
                        | ReviewEditorKind::List
                        | ReviewEditorKind::Compound
                        | ReviewEditorKind::AutoCustom
                )
            {
                lines.extend(review_complex_lines(
                    item,
                    state,
                    section_draft,
                    errors,
                    language,
                    left.width,
                    theme,
                ));
            }
            row_index += 1;
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
    let focused_line = if let Some(line) = mcp_create_line {
        line
    } else if action_focused {
        lines.len().saturating_sub(1)
    } else {
        row_line_indices
            .get(state.focus)
            .copied()
            .unwrap_or_default()
    };
    let scroll = focused_line
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(max_scroll)
        .min(usize::from(u16::MAX)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), content_area);
    render_surface_action_dock(
        frame,
        left,
        if state.review_preview_json.is_some() {
            t(language, "Confirm and write", "确认并写入")
        } else {
            t(language, "Preview final JSON", "预览配置")
        },
        action_focused,
        theme,
    );

    if right.width > 0 {
        render_surface(frame, right, theme);
        let inspector = right.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        if state.review_preview_json.is_some() {
            render_review_json_preview(frame, inspector, state, language, theme);
        } else if action_focused {
            render_inspector(
                frame,
                inspector,
                t(language, "Preview final JSON", "预览配置"),
                review_confirm_inspector_body(language),
                theme,
            );
        } else if let Some((group, item)) = review.row(state.focus) {
            let title = review_item_label(item.label_key, language);
            let body = review_item_inspector_body(group, item, state, language);
            render_inspector(frame, inspector, title, body, theme);
        }
    }

    render_horizontal_rule(frame, bottom_rule, theme);
    let hints = if state.review_preview_json.is_some() {
        if state.review_preview_search_active {
            vec![
                ("text", t(language, "query", "输入查询")),
                ("Backspace", t(language, "delete", "删除")),
                ("Enter", t(language, "find", "查找")),
                ("Esc", t(language, "close search", "关闭搜索")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ]
        } else {
            vec![
                ("↑↓ j/k", t(language, "scroll JSON", "滚动 JSON")),
                ("/", t(language, "search JSON", "搜索 JSON")),
                ("n/N", t(language, "matches", "匹配项")),
                ("Esc", t(language, "modify", "返回修改")),
                ("Enter", t(language, "write", "写入")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ]
        }
    } else if state.review_search_active {
        vec![
            ("text", t(language, "query", "输入查询")),
            ("Backspace", t(language, "delete", "删除")),
            ("Enter", t(language, "find", "查找")),
            ("Esc", t(language, "close", "关闭")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ]
    } else if state.editing.is_some() {
        if review
            .row(state.focus)
            .is_some_and(|(_, item)| item.editor == ReviewEditorKind::Choice)
        {
            vec![
                ("↑↓ j/k", t(language, "choose", "选择")),
                ("Enter", t(language, "save", "保存")),
                ("Esc", t(language, "discard", "放弃")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ]
        } else {
            vec![
                ("←→", t(language, "cursor", "光标")),
                ("Enter", t(language, "save", "保存")),
                ("Esc", t(language, "discard", "放弃")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ]
        }
    } else if section_draft.is_some() {
        match review.row(state.focus).map(|(_, item)| item.editor) {
            Some(ReviewEditorKind::MultiSelect) => vec![
                ("↑↓ j/k", t(language, "move", "移动")),
                ("Enter/Space", t(language, "select / clear", "选择 / 取消")),
                ("J/K", t(language, "change priority", "调整优先级")),
                ("Esc", t(language, "back", "返回")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            Some(ReviewEditorKind::List) => vec![
                ("↑↓ j/k", t(language, "move", "移动")),
                ("Enter", t(language, "edit", "编辑")),
                ("a/d", t(language, "add / remove", "添加 / 删除")),
                ("Esc", t(language, "back", "返回")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            Some(ReviewEditorKind::Compound) => vec![
                ("↑↓ j/k", t(language, "move", "移动")),
                ("Enter", t(language, "edit / toggle", "编辑 / 切换")),
                ("d", t(language, "delete server", "删除服务")),
                ("Esc", t(language, "back", "返回")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            Some(ReviewEditorKind::AutoCustom) => vec![
                ("↑↓ j/k", t(language, "move", "移动")),
                ("Enter", t(language, "select / edit", "选择 / 编辑")),
                ("Esc", t(language, "back", "返回")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
            _ => vec![
                ("↑↓ j/k", t(language, "move", "移动")),
                ("Enter", t(language, "edit", "编辑")),
                ("Ctrl+C", t(language, "cancel", "取消")),
            ],
        }
    } else {
        vec![
            ("↑↓ j/k", t(language, "move", "移动")),
            ("Enter", t(language, "edit / toggle", "编辑 / 切换")),
            ("a", t(language, "add MCP", "添加 MCP")),
            ("/", t(language, "search", "搜索")),
            ("n/N", t(language, "matches", "匹配项")),
            ("Ctrl+C", t(language, "cancel", "取消")),
        ]
    };
    render_contextual_footer(frame, footer, &hints, theme);
}

fn render_review_json_preview(
    frame: &mut Frame,
    area: Rect,
    state: &TuiState,
    language: UiLanguage,
    theme: &Theme,
) {
    let search_visible =
        state.review_preview_search_active || !state.review_preview_search.is_empty();
    let [title_area, rule_area, search_area, body_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(if search_visible { 2 } else { 0 }),
        Constraint::Min(1),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::styled(
            t(language, "Final JSON preview", "最终 JSON 预览"),
            theme.emphasis,
        )),
        title_area,
    );
    let rule_width = rule_area.width.saturating_sub(2).min(18) as usize;
    frame.render_widget(
        Paragraph::new(Line::styled("─".repeat(rule_width), theme.structure)),
        rule_area,
    );
    if search_visible {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(t(language, "Search: ", "搜索："), theme.dim),
                Span::styled(format!("/{}", state.review_preview_search), theme.emphasis),
            ])),
            search_area,
        );
    }

    let query = state.review_preview_search.trim().to_lowercase();
    let json_lines: Vec<Line<'static>> = state
        .review_preview_json
        .as_deref()
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let style = if !query.is_empty() && line.to_lowercase().contains(&query) {
                theme.emphasis
            } else {
                theme.normal
            };
            Line::styled(line.to_string(), style)
        })
        .collect();
    let visible = usize::from(body_area.height);
    let max_scroll = json_lines.len().saturating_sub(visible);
    let scroll = usize::from(state.review_preview_scroll)
        .min(max_scroll)
        .min(usize::from(u16::MAX)) as u16;
    frame.render_widget(Paragraph::new(json_lines).scroll((scroll, 0)), body_area);
}

fn review_display_value(session: &SetupSession, group: &ReviewGroup, item: &ReviewItem) -> String {
    if !item.value.is_empty() {
        return item.value.clone();
    }
    match (group.target, item.field) {
        (ReviewTarget::OptionalSection(OptionalSection::Room), Some(SetupField::NotebookRoot)) => {
            let workspace = session.optional_draft(OptionalSection::Workspace);
            let workspace_root = optional_field_value(&workspace, SetupField::WorkspaceRoot);
            PathBuf::from(workspace_root)
                .join("notebook")
                .to_string_lossy()
                .into_owned()
        }
        (
            ReviewTarget::OptionalSection(OptionalSection::TunnelClient),
            Some(SetupField::TunnelClientVersion),
        ) => crate::tunnel_distribution::PINNED_VERSION.to_string(),
        _ => item.value.clone(),
    }
}

fn review_complex_lines(
    item: &ReviewItem,
    state: &TuiState,
    section_draft: Option<&OptionalSectionDraft>,
    errors: &HashMap<SetupField, String>,
    language: UiLanguage,
    area_width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let Some(draft) = section_draft else {
        return Vec::new();
    };
    match item.editor {
        ReviewEditorKind::MultiSelect => {
            let Some(field) = item.field else {
                return Vec::new();
            };
            let Ok(selection) = optional_multi_select_state(draft, field) else {
                return Vec::new();
            };
            let mut lines = Vec::new();
            for (index, option) in selection.options().iter().enumerate() {
                lines.push(ordered_multi_select_row_line(
                    option,
                    state.review_subfocus == index,
                    selection.selection_rank(option),
                    18,
                    theme,
                ));
            }
            if let Some(error) = errors.get(&field) {
                lines.push(inline_error_line(&localized_error(error, language), theme));
            }
            lines
        }
        ReviewEditorKind::List => {
            let Some(field) = item.field else {
                return Vec::new();
            };
            let Ok(list) = optional_list_state(draft, field) else {
                return Vec::new();
            };
            let mut lines = Vec::new();
            for (index, value) in list.items().iter().enumerate() {
                let editing = state.list_edit.as_ref().is_some_and(|target| {
                    target.field == field && target.index == index && state.editing.is_some()
                });
                let display_value = state
                    .editing
                    .as_ref()
                    .filter(|_| editing)
                    .map(|edit| edit.buffer.as_str())
                    .unwrap_or(value);
                let cursor = state
                    .editing
                    .as_ref()
                    .filter(|_| editing)
                    .map(|edit| edit.cursor);
                lines.push(editable_list_item_line(
                    display_value,
                    state.review_subfocus == index,
                    editing,
                    cursor,
                    area_width.saturating_sub(2),
                    theme,
                ));
            }
            if let Some(error) = errors.get(&field) {
                lines.push(inline_error_line(&localized_error(error, language), theme));
            }
            lines
        }
        ReviewEditorKind::Compound => {
            let Some(index) = state.review_mcp_index else {
                return Vec::new();
            };
            let mut lines = Vec::new();
            const LABEL_WIDTH: usize = 12;
            for (subfocus, field) in mcp_server_review_fields(draft, index)
                .into_iter()
                .enumerate()
            {
                let value = mcp_server_value(draft, index, field).unwrap_or_default();
                let label = optional_field_label(field, language);
                let focused = state.review_subfocus == subfocus;
                let editing = state.mcp_edit.as_ref().is_some_and(|target| {
                    target.index == index && target.field == field && state.editing.is_some()
                });
                if matches!(
                    field,
                    SetupField::McpServerId
                        | SetupField::McpServerEndpoint
                        | SetupField::McpServerBearerToken
                ) {
                    let raw_display_value = state
                        .editing
                        .as_ref()
                        .filter(|_| editing)
                        .map(|edit| edit.buffer.as_str())
                        .unwrap_or(value.as_str());
                    let display_value = if field == SetupField::McpServerBearerToken {
                        "•".repeat(raw_display_value.chars().count())
                    } else {
                        raw_display_value.to_string()
                    };
                    let cursor = state
                        .editing
                        .as_ref()
                        .filter(|_| editing)
                        .map(|edit| edit.cursor);
                    let input_width = UnicodeWidthStr::width(display_value.as_str())
                        .saturating_add(usize::from(editing))
                        .clamp(8, 36);
                    lines.push(input_row_line(
                        label,
                        &display_value,
                        focused,
                        editing,
                        cursor,
                        LABEL_WIDTH,
                        input_width,
                        theme,
                    ));
                } else {
                    let display_value = if field == SetupField::McpServerBearerAuth {
                        if value == "true" {
                            "bearer"
                        } else {
                            "none"
                        }
                    } else {
                        value.as_str()
                    };
                    lines.push(value_row_line(
                        label,
                        display_value,
                        focused,
                        LABEL_WIDTH,
                        theme,
                    ));
                }
                if focused {
                    if let Some(error) = errors.get(&field) {
                        lines.push(inline_error_line(&localized_error(error, language), theme));
                    }
                }
            }
            lines
        }
        ReviewEditorKind::AutoCustom => {
            let configured = optional_field_value(draft, SetupField::MaxActiveJobs);
            let editing = state
                .editing
                .as_ref()
                .filter(|edit| edit.field == SetupField::MaxActiveJobs);
            let custom_value = if let Some(edit) = editing {
                edit.buffer.as_str()
            } else if configured != "auto" {
                configured.as_str()
            } else {
                state.max_active_custom.as_str()
            };
            let mut lines = vec![choice_row_line(
                t(language, "Auto", "自动"),
                state.review_subfocus == 0,
                configured == "auto",
                12,
                theme,
            )];
            lines.push(choice_input_row_line(
                t(language, "Custom", "自定义"),
                custom_value,
                state.review_subfocus == 1,
                configured != "auto",
                editing.is_some(),
                editing.map(|edit| edit.cursor),
                12,
                5,
                theme,
            ));
            if let Some(error) = errors.get(&SetupField::MaxActiveJobs) {
                lines.push(inline_error_line(&localized_error(error, language), theme));
            }
            lines
        }
        _ => Vec::new(),
    }
}

fn review_item_inspector_body(
    group: &ReviewGroup,
    item: &ReviewItem,
    state: &TuiState,
    language: UiLanguage,
) -> &'static [&'static str] {
    if item.editor == ReviewEditorKind::Compound && state.review_mcp_index.is_some() {
        if let ReviewTarget::OptionalSection(section) = group.target {
            let field = match state.review_subfocus {
                0 => Some(SetupField::McpServerId),
                1 => Some(SetupField::McpServerEnabled),
                2 => Some(SetupField::McpServerTransport),
                3 => Some(SetupField::McpServerEndpoint),
                4 => Some(SetupField::McpServerBearerAuth),
                5 => Some(SetupField::McpServerBearerToken),
                _ => None,
            };
            return optional_form_inspector_body(section, field, language);
        }
    }
    match item.field {
        Some(SetupField::Mode) => match item.value.as_str() {
            "Standalone" | "standalone" => basic_inspector_copy(0, language).1,
            "Hub" | "hub" => basic_inspector_copy(1, language).1,
            "Local" | "local" => basic_inspector_copy(2, language).1,
            _ => basic_inspector_copy(0, language).1,
        },
        Some(SetupField::Profile) => match item.value.as_str() {
            "Room" | "room" => basic_inspector_copy(4, language).1,
            _ => basic_inspector_copy(3, language).1,
        },
        field @ Some(
            SetupField::TunnelId
            | SetupField::TunnelSecretSource
            | SetupField::TunnelSecretPath
            | SetupField::TunnelSecretEnvironment
            | SetupField::ProvisionTunnelSecret
            | SetupField::TunnelSecretValue
            | SetupField::HubUrl
            | SetupField::HubTransport
            | SetupField::AgentId
            | SetupField::AgentSecret,
        ) => connection_inspector_body(field, language),
        field @ Some(_) => match group.target {
            ReviewTarget::OptionalSection(section) => {
                optional_form_inspector_body(section, field, language)
            }
            _ => &[],
        },
        None => match group.target {
            ReviewTarget::OptionalSection(section) => {
                optional_form_inspector_body(section, None, language)
            }
            _ => &[],
        },
    }
}

fn review_confirm_inspector_body(language: UiLanguage) -> &'static [&'static str] {
    match language {
        UiLanguage::En => &[
            "Write the staged configuration and any approved secret file changes.",
            "",
            "This is the only Review action that writes to disk.",
        ],
        UiLanguage::ZhCn => &[
            "写入当前暂存配置，以及已确认的密钥文件变更。",
            "",
            "这是 Review 中唯一会写入磁盘的操作。",
        ],
    }
}

fn review_target_label(target: ReviewTarget, language: UiLanguage) -> &'static str {
    match target {
        ReviewTarget::Basic => t(language, "Global settings", "全局设定"),
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

pub(super) fn review_search_text(
    group: &ReviewGroup,
    item: &ReviewItem,
    session: &SetupSession,
    language: UiLanguage,
) -> String {
    let value = if item.editor == ReviewEditorKind::Secret {
        String::new()
    } else {
        review_display_value(session, group, item)
    };
    format!(
        "{} {} {}",
        review_target_label(group.target, language),
        review_item_label(item.label_key, language),
        value
    )
}

fn review_item_label(label_key: &str, language: UiLanguage) -> &'static str {
    match label_key {
        "mode" => t(language, "Mode", "模式"),
        "profile" => t(language, "Profile", "能力配置"),
        "tunnel_id" => t(language, "Tunnel ID", "隧道 ID"),
        "tunnel_secret_source" => t(language, "Secret source", "密钥来源"),
        "tunnel_secret_reference" => t(language, "Secret reference", "密钥引用"),
        "provision_tunnel_secret" => t(language, "Provision secret now", "立即写入密钥"),
        "tunnel_secret_value" => t(language, "Secret value", "密钥值"),
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
        "confirmation_channels" => t(language, "Fallback channels", "降级通道"),
        "confirmation_language" => t(language, "Language", "语言"),
        "max_concurrent_tasks" => t(language, "Max concurrent tasks", "最大并发任务"),
        "max_active_jobs" => t(language, "Max active jobs", "最大活动作业"),
        "max_file_search_context_lines" => {
            t(language, "File-search context lines", "文件搜索上下文行数")
        }
        "sandbox_enabled" => t(language, "Sandbox enabled", "启用沙箱"),
        "bubblewrap_path" => t(language, "Bubblewrap path", "Bubblewrap 路径"),
        "required_runtime_paths" => t(language, "Required runtime paths", "必需运行时路径"),
        "mcp_server" => t(language, "MCP server", "MCP 服务"),
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
