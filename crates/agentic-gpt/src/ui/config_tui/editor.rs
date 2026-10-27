use std::fmt;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Margin, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::{
    cli_i18n::UiLanguage,
    config_setup::SetupField,
    tui::{centered_overlay, forms, Theme},
};

use super::input::{apply_text_key, EditState};

pub(super) enum EditorAction {
    Continue,
    Cancel,
    Submit(String),
}

enum Content {
    SecretReference {
        field: SetupField,
        source: SecretSource,
        path: String,
        environment: String,
        focus: usize,
        editing: Option<EditState>,
    },
    AllowHosts {
        mode: AllowHostsMode,
        items: Vec<String>,
        focus: usize,
        editing: Option<EditState>,
    },
}

#[derive(Clone, Copy)]
enum SecretSource {
    Unset,
    File,
    Environment,
}

#[derive(Clone, Copy)]
enum AllowHostsMode {
    Automatic,
    All,
    Custom,
}

fn secret_source_index(source: SecretSource) -> usize {
    match source {
        SecretSource::Unset => 0,
        SecretSource::File => 1,
        SecretSource::Environment => 2,
    }
}

fn next_secret_source(source: SecretSource) -> SecretSource {
    match source {
        SecretSource::Unset => SecretSource::File,
        SecretSource::File => SecretSource::Environment,
        SecretSource::Environment => SecretSource::Unset,
    }
}

fn previous_secret_source(source: SecretSource) -> SecretSource {
    match source {
        SecretSource::Unset => SecretSource::Environment,
        SecretSource::File => SecretSource::Unset,
        SecretSource::Environment => SecretSource::File,
    }
}

fn mode_index(mode: AllowHostsMode) -> usize {
    match mode {
        AllowHostsMode::Automatic => 0,
        AllowHostsMode::All => 1,
        AllowHostsMode::Custom => 2,
    }
}

fn next_mode(mode: AllowHostsMode) -> AllowHostsMode {
    match mode {
        AllowHostsMode::Automatic => AllowHostsMode::All,
        AllowHostsMode::All => AllowHostsMode::Custom,
        AllowHostsMode::Custom => AllowHostsMode::Automatic,
    }
}

fn previous_mode(mode: AllowHostsMode) -> AllowHostsMode {
    match mode {
        AllowHostsMode::Automatic => AllowHostsMode::Custom,
        AllowHostsMode::All => AllowHostsMode::Automatic,
        AllowHostsMode::Custom => AllowHostsMode::All,
    }
}

pub(super) struct StructuredEditor {
    field: SetupField,
    content: Content,
}

impl fmt::Debug for StructuredEditor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StructuredEditor")
            .field("field", &self.field)
            .field("content", &"[REDACTED]")
            .finish()
    }
}

impl StructuredEditor {
    pub(super) fn new(field: SetupField, value: &str) -> Option<Self> {
        let content = if field.is_secret_reference() {
            let (source, path, environment) = if value.is_empty() {
                (SecretSource::Unset, String::new(), String::new())
            } else if let Some(path) = value.strip_prefix("file:") {
                (SecretSource::File, path.to_string(), String::new())
            } else if let Some(environment) = value.strip_prefix("env:") {
                (
                    SecretSource::Environment,
                    String::new(),
                    environment.to_string(),
                )
            } else {
                // Keep malformed references visible and editable so opening the
                // editor never discards the user's current value.
                (SecretSource::File, value.to_string(), String::new())
            };
            Content::SecretReference {
                field,
                source,
                path,
                environment,
                focus: 0,
                editing: None,
            }
        } else if field == SetupField::HttpMcpAllowHosts {
            let (mode, items) = match serde_json::from_str::<Option<Vec<String>>>(value) {
                Ok(None) => (AllowHostsMode::Automatic, Vec::new()),
                Ok(Some(items)) if items.as_slice() == ["*"] => (AllowHostsMode::All, Vec::new()),
                Ok(Some(items)) => (AllowHostsMode::Custom, items),
                Err(_) => (AllowHostsMode::Custom, vec![value.to_string()]),
            };
            Content::AllowHosts {
                mode,
                items,
                focus: 0,
                editing: None,
            }
        } else {
            return None;
        };
        Some(Self { field, content })
    }

    pub(super) fn field(&self) -> SetupField {
        self.field
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> EditorAction {
        if key.code == KeyCode::Esc {
            return EditorAction::Cancel;
        }
        match &mut self.content {
            Content::SecretReference {
                source,
                path,
                environment,
                focus,
                editing,
                ..
            } => {
                if let Some(edit) = editing.take() {
                    if key.code == KeyCode::Enter {
                        match edit.field {
                            SetupField::TunnelSecretPath => *path = edit.buffer,
                            _ => *environment = edit.buffer,
                        }
                    } else {
                        let updated = apply_text_key(edit, key.code);
                        match updated.field {
                            SetupField::TunnelSecretPath => *path = updated.buffer.clone(),
                            _ => *environment = updated.buffer.clone(),
                        }
                        *editing = Some(updated);
                    }
                    return EditorAction::Continue;
                }
                let focus_count = if matches!(source, SecretSource::Unset) {
                    2
                } else {
                    3
                };
                match key.code {
                    KeyCode::Tab | KeyCode::Down => *focus = (*focus + 1) % focus_count,
                    KeyCode::BackTab | KeyCode::Up => {
                        *focus = (*focus + focus_count - 1) % focus_count
                    }
                    KeyCode::Left if *focus == 0 => *source = previous_secret_source(*source),
                    KeyCode::Right if *focus == 0 => *source = next_secret_source(*source),
                    KeyCode::Enter if *focus == 0 => {
                        *source = next_secret_source(*source);
                    }
                    KeyCode::Enter if *focus == 1 && !matches!(source, SecretSource::Unset) => {
                        let (edit_field, value) = match source {
                            SecretSource::Unset => unreachable!(),
                            SecretSource::File => (SetupField::TunnelSecretPath, path.clone()),
                            SecretSource::Environment => (self.field, environment.clone()),
                        };
                        *editing = Some(EditState::new(edit_field, value));
                    }
                    KeyCode::Enter | KeyCode::Char('a' | 'A') if *focus == focus_count - 1 => {
                        let value = match source {
                            SecretSource::Unset => String::new(),
                            SecretSource::File => format!("file:{path}"),
                            SecretSource::Environment => format!("env:{environment}"),
                        };
                        return EditorAction::Submit(value);
                    }
                    KeyCode::Char('c' | 'C') => return EditorAction::Submit(String::new()),
                    KeyCode::Char('a' | 'A') => {
                        let value = match source {
                            SecretSource::Unset => String::new(),
                            SecretSource::File => format!("file:{path}"),
                            SecretSource::Environment => format!("env:{environment}"),
                        };
                        return EditorAction::Submit(value);
                    }
                    _ => {}
                }
            }
            Content::AllowHosts {
                mode,
                items,
                focus,
                editing,
            } => {
                if let Some(edit) = editing.take() {
                    if key.code == KeyCode::Enter {
                        if let Some(item) =
                            focus.checked_sub(3).and_then(|index| items.get_mut(index))
                        {
                            *item = edit.buffer;
                        }
                    } else {
                        let updated = apply_text_key(edit, key.code);
                        if let Some(item) =
                            focus.checked_sub(3).and_then(|index| items.get_mut(index))
                        {
                            *item = updated.buffer.clone();
                        }
                        *editing = Some(updated);
                    }
                    return EditorAction::Continue;
                }
                let focus_count = 4 + if matches!(mode, AllowHostsMode::Custom) {
                    items.len()
                } else {
                    0
                };
                match key.code {
                    KeyCode::Tab | KeyCode::Down => *focus = (*focus + 1) % focus_count,
                    KeyCode::BackTab | KeyCode::Up => {
                        *focus = (*focus + focus_count - 1) % focus_count
                    }
                    KeyCode::Left if *focus < 3 => {
                        *mode = previous_mode(*mode);
                        *focus = mode_index(*mode);
                    }
                    KeyCode::Right if *focus < 3 => {
                        *mode = next_mode(*mode);
                        *focus = mode_index(*mode);
                    }
                    KeyCode::Enter if *focus < 3 => {
                        *mode = match *focus {
                            0 => AllowHostsMode::Automatic,
                            1 => AllowHostsMode::All,
                            _ => AllowHostsMode::Custom,
                        };
                    }
                    KeyCode::Insert | KeyCode::Char('n' | 'N')
                        if matches!(mode, AllowHostsMode::Custom) =>
                    {
                        let insert_at = if *focus >= 3 && *focus < items.len() + 3 {
                            *focus - 2
                        } else {
                            items.len()
                        };
                        items.insert(insert_at, String::new());
                        *focus = 3 + insert_at;
                        *editing = Some(EditState::new(self.field, String::new()));
                    }
                    KeyCode::Delete | KeyCode::Char('d' | 'D')
                        if matches!(mode, AllowHostsMode::Custom)
                            && *focus >= 3
                            && *focus < items.len() + 3
                            && !items.is_empty() =>
                    {
                        let index = *focus - 3;
                        items.remove(index);
                        *focus = (3 + index).min(3 + items.len());
                    }
                    KeyCode::Enter
                        if matches!(mode, AllowHostsMode::Custom)
                            && *focus >= 3
                            && *focus < items.len() + 3 =>
                    {
                        *editing = Some(EditState::new(self.field, items[*focus - 3].clone()));
                    }
                    KeyCode::Enter | KeyCode::Char('a' | 'A') if *focus == focus_count - 1 => {
                        return EditorAction::Submit(match mode {
                            AllowHostsMode::Automatic => "null".to_string(),
                            AllowHostsMode::All => "[\"*\"]".to_string(),
                            AllowHostsMode::Custom => serde_json::to_string(&Some(items.clone()))
                                .expect("serializing strings cannot fail"),
                        });
                    }
                    KeyCode::Char('a' | 'A') => {
                        return EditorAction::Submit(match mode {
                            AllowHostsMode::Automatic => "null".to_string(),
                            AllowHostsMode::All => "[\"*\"]".to_string(),
                            AllowHostsMode::Custom => serde_json::to_string(&Some(items.clone()))
                                .expect("serializing strings cannot fail"),
                        });
                    }
                    _ => {}
                }
            }
        }
        EditorAction::Continue
    }

    pub(super) fn render(
        &self,
        frame: &mut Frame,
        theme: &Theme,
        language: UiLanguage,
        error: Option<&str>,
    ) {
        let (width, height) = match &self.content {
            Content::SecretReference { .. } => (70, 13),
            Content::AllowHosts { .. } => (70, 18),
        };
        let area = centered_overlay(frame.area(), width, height);
        if area.width == 0 || area.height == 0 {
            return;
        }
        frame.render_widget(Clear, area);
        let title = match &self.content {
            Content::SecretReference { .. } => tr(language, " Secret reference ", " 密钥引用 "),
            Content::AllowHosts { .. } => tr(
                language,
                " HTTP MCP allowed hosts ",
                " HTTP MCP 允许的主机 ",
            ),
        };
        frame.render_widget(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .style(theme.surface),
            area,
        );
        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let (lines, focused_line, hint) = match &self.content {
            Content::SecretReference {
                field,
                source,
                path,
                environment,
                focus,
                editing,
            } => (
                secret_lines(
                    *field,
                    *source,
                    path,
                    environment,
                    *focus,
                    editing.as_ref(),
                    language,
                    theme,
                    inner.width,
                ),
                if *focus == 0 {
                    1
                } else if matches!(source, SecretSource::Unset) {
                    0
                } else {
                    2
                },
                tr(
                    language,
                    "Tab/↑↓ move · ←→ source · A apply · C clear",
                    "Tab/↑↓ 移动 · ←→ 来源 · A 应用 · C 清空",
                ),
            ),
            Content::AllowHosts {
                mode,
                items,
                focus,
                editing,
            } => {
                let focused_line = match (*mode, *focus) {
                    (_, 0..=2) => 1 + *focus,
                    (AllowHostsMode::Custom, list_focus) if list_focus < 3 + items.len() => {
                        5 + list_focus - 3
                    }
                    _ => 0,
                };
                (
                    allow_hosts_lines(
                        *mode,
                        items,
                        *focus,
                        editing.as_ref(),
                        language,
                        theme,
                        inner.width,
                    ),
                    focused_line,
                    tr(
                        language,
                        "Tab/↑↓ move · Insert add · Delete remove · A apply",
                        "Tab/↑↓ 移动 · Insert 添加 · Delete 删除 · A 应用",
                    ),
                )
            }
        };
        let footer_height = (if error.is_some() { 2 } else { 1 }).min(inner.height);
        let body_height = inner.height.saturating_sub(footer_height);
        if body_height > 0 {
            let body = Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: body_height,
            };
            let scroll = focused_line.saturating_sub(usize::from(body_height).saturating_sub(1));
            frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), body);
        }
        let footer = Rect {
            x: inner.x,
            y: inner.y + body_height,
            width: inner.width,
            height: footer_height,
        };
        let mut footer_lines = Vec::new();
        if let Some(error) = error {
            if footer_height > 1 {
                footer_lines.push(Line::styled(error.to_string(), theme.error));
            }
        }
        let apply_focused = match &self.content {
            Content::SecretReference { source, focus, .. } => {
                *focus
                    == if matches!(source, SecretSource::Unset) {
                        1
                    } else {
                        2
                    }
            }
            Content::AllowHosts {
                mode, items, focus, ..
            } => {
                *focus
                    == 3 + if matches!(mode, AllowHostsMode::Custom) {
                        items.len()
                    } else {
                        0
                    }
            }
        };
        footer_lines.push(Line::from(vec![
            Span::styled(
                if apply_focused { "❯ " } else { "  " },
                if apply_focused {
                    theme.pointer
                } else {
                    theme.normal
                },
            ),
            Span::styled(
                tr(language, "[A] Apply", "[A] 应用"),
                if apply_focused {
                    theme.focus
                } else {
                    theme.emphasis
                },
            ),
            Span::raw(" · "),
            Span::styled("Esc", theme.accent),
            Span::raw(tr(language, " cancel · ", " 取消 · ")),
            Span::styled(hint.to_string(), theme.muted),
        ]));
        frame.render_widget(Paragraph::new(footer_lines), footer);
    }
}

#[allow(clippy::too_many_arguments)]
fn secret_lines(
    field: SetupField,
    source: SecretSource,
    path: &str,
    environment: &str,
    focus: usize,
    editing: Option<&EditState>,
    language: UiLanguage,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    let source_index = secret_source_index(source);
    let value = match source {
        SecretSource::Unset => "",
        SecretSource::File => path,
        SecretSource::Environment => environment,
    };
    let editing_value = editing.is_some();
    let cursor = editing.map(|edit| edit.cursor);
    let label = match source {
        SecretSource::Unset => "",
        SecretSource::File => tr(language, "File path", "文件路径"),
        SecretSource::Environment => tr(language, "Variable name", "变量名"),
    };
    let mut lines = vec![
        Line::styled(
            tr(language, "Choose a reference source", "选择引用方式"),
            theme.muted,
        ),
        Line::from(vec![
            Span::styled(
                if focus == 0 { "❯ " } else { "  " },
                if focus == 0 {
                    theme.pointer
                } else {
                    theme.normal
                },
            ),
            Span::styled(
                format!("[{}] unset", if source_index == 0 { "x" } else { " " }),
                if source_index == 0 {
                    theme.selected
                } else {
                    theme.normal
                },
            ),
            Span::raw("  "),
            Span::styled(
                format!("[{}] file", if source_index == 1 { "x" } else { " " }),
                if source_index == 1 {
                    theme.selected
                } else {
                    theme.normal
                },
            ),
            Span::raw("  "),
            Span::styled(
                format!("[{}] env", if source_index == 2 { "x" } else { " " }),
                if source_index == 2 {
                    theme.selected
                } else {
                    theme.normal
                },
            ),
        ]),
    ];
    if !matches!(source, SecretSource::Unset) {
        let prefix = format!("  {label}: ");
        let input_width = usize::from(width)
            .saturating_sub(prefix.chars().count() + 3)
            .max(1);
        lines.push(Line::from(
            vec![
                Span::styled(
                    if focus == 1 { "❯ " } else { "  " },
                    if focus == 1 {
                        theme.pointer
                    } else {
                        theme.normal
                    },
                ),
                Span::styled(format!("{label}: "), theme.normal),
            ]
            .into_iter()
            .chain(forms::inline_input_spans(
                value,
                editing_value,
                cursor,
                input_width,
                theme,
            ))
            .collect::<Vec<_>>(),
        ));
        let hint = match (field, source) {
            (SetupField::HttpMcpBearerToken, SecretSource::File) => tr(
                language,
                "HTTP MCP file references need an absolute path.",
                "HTTP MCP 文件引用需要绝对路径。",
            ),
            (_, SecretSource::File) => tr(
                language,
                "File references may use ~. Environment input is a NAME without env:.",
                "文件引用可使用 ~。环境变量只填 NAME，不含 env:。",
            ),
            _ => tr(
                language,
                "Environment input is a NAME without env:.",
                "环境变量只填 NAME，不含 env:。",
            ),
        };
        lines.push(Line::styled(hint, theme.muted));
    }
    lines
}

fn allow_hosts_lines(
    mode: AllowHostsMode,
    items: &[String],
    focus: usize,
    editing: Option<&EditState>,
    language: UiLanguage,
    theme: &Theme,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        tr(
            language,
            "Choose how HTTP MCP hosts are allowed",
            "选择 HTTP MCP 主机允许方式",
        ),
        theme.muted,
    )];
    let labels = [
        tr(language, "Automatic (null)", "自动（null）"),
        tr(language, "Allow all [*]", "允许全部 [*]"),
        tr(language, "Custom list", "指定列表"),
    ];
    for (index, label) in labels.iter().enumerate() {
        let selected_mode = matches!(
            (index, mode),
            (0, AllowHostsMode::Automatic) | (1, AllowHostsMode::All) | (2, AllowHostsMode::Custom)
        );
        lines.push(Line::from(vec![
            Span::styled(
                if focus == index { "❯ " } else { "  " },
                if focus == index {
                    theme.pointer
                } else {
                    theme.normal
                },
            ),
            Span::styled(
                format!("[{}] {label}", if selected_mode { "x" } else { " " }),
                if selected_mode {
                    theme.selected
                } else {
                    theme.normal
                },
            ),
        ]));
    }
    if matches!(mode, AllowHostsMode::Custom) {
        lines.push(Line::styled(
            tr(
                language,
                "Hosts (Insert add · Delete remove · Enter edit)",
                "主机列表（Insert 添加 · Delete 删除 · Enter 编辑）",
            ),
            theme.muted,
        ));
        for (index, item) in items.iter().enumerate() {
            let focused = focus == 3 + index;
            let cursor = editing
                .filter(|edit| edit.field == SetupField::HttpMcpAllowHosts && focused)
                .map(|edit| edit.cursor);
            let value = editing
                .filter(|edit| edit.field == SetupField::HttpMcpAllowHosts && focused)
                .map_or(item.as_str(), |edit| edit.buffer.as_str());
            let input_width = usize::from(width).saturating_sub(12).max(1);
            let mut row = vec![
                Span::styled(
                    if focused { "❯ " } else { "  " },
                    if focused { theme.pointer } else { theme.normal },
                ),
                Span::raw(format!("{}: ", index + 1)),
            ];
            if focused {
                row.extend(forms::inline_input_spans(
                    value,
                    editing.is_some(),
                    cursor,
                    input_width,
                    theme,
                ));
            } else {
                row.push(Span::styled(value.to_string(), theme.normal));
            }
            lines.push(Line::from(row));
        }
        if items.is_empty() {
            lines.push(Line::styled(
                tr(language, "(empty list)", "（空列表）"),
                theme.dim,
            ));
        }
    }
    lines
}

fn tr<'a>(language: UiLanguage, en: &'a str, zh: &'a str) -> &'a str {
    match language {
        UiLanguage::En => en,
        UiLanguage::ZhCn => zh,
    }
}
