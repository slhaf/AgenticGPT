use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::Modifier,
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

use super::Theme;

pub(crate) fn render_header(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    progress: &str,
    theme: &Theme,
) {
    let line = Line::from(vec![
        Span::styled(title, theme.accent),
        Span::raw("  "),
        Span::styled(progress, theme.dim),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

pub(crate) fn render_surface_header(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    progress: &str,
    theme: &Theme,
) {
    let [title_area, progress_area] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(8)]).areas(area);
    frame.render_widget(
        Paragraph::new(Span::styled(title, theme.emphasis)),
        title_area,
    );
    frame.render_widget(
        Paragraph::new(progress)
            .alignment(Alignment::Right)
            .style(theme.muted),
        progress_area,
    );
}

pub(crate) fn render_horizontal_rule(frame: &mut Frame, area: Rect, theme: &Theme) {
    if area.width == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize)).style(theme.structure),
        area,
    );
}

pub(crate) fn labeled_heading_line(label: &str, width: u16, theme: &Theme) -> Line<'static> {
    let prefix = format!("── {label} ");
    let fill_width = width.saturating_sub(prefix.chars().count() as u16 + 7) as usize;
    Line::from(vec![
        Span::styled(prefix, theme.emphasis),
        Span::styled("─".repeat(fill_width), theme.structure),
    ])
}

pub(crate) fn surface_choice_line(
    label: &str,
    selected: bool,
    focused: bool,
    theme: &Theme,
) -> Line<'static> {
    let pointer = if focused {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    };
    let label_width = 12usize;
    let padding = " ".repeat(label_width.saturating_sub(label.chars().count()));
    let selected = if selected {
        Span::styled("", theme.selected)
    } else {
        Span::raw(" ")
    };
    Line::from(vec![
        Span::raw("  "),
        pointer,
        Span::raw(label.to_string()),
        Span::raw(padding),
        selected,
    ])
}

pub(crate) fn surface_choice_value_line(
    label: &str,
    value: &str,
    selected: bool,
    focused: bool,
    theme: &Theme,
) -> Line<'static> {
    let pointer = if focused {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    };
    let label_width = 24usize;
    let padding = " ".repeat(label_width.saturating_sub(label.chars().count()));
    let marker = if selected {
        Span::styled("", theme.selected)
    } else {
        Span::raw(" ")
    };
    Line::from(vec![
        Span::raw("  "),
        pointer,
        Span::raw(label.to_string()),
        Span::raw(padding),
        marker,
        Span::styled(
            format!("  {value}"),
            if focused {
                theme.emphasis
            } else {
                theme.normal
            },
        ),
    ])
}

pub(crate) fn surface_status_line(
    label: &str,
    status: &str,
    focused: bool,
    disabled: bool,
    theme: &Theme,
) -> Line<'static> {
    let pointer = if focused {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    };
    let style = if disabled {
        theme.disabled
    } else if focused {
        theme.emphasis
    } else {
        theme.normal
    };
    Line::from(vec![
        Span::raw("  "),
        pointer,
        Span::styled(label.to_string(), style),
        Span::styled(
            format!("  [{status}]"),
            if disabled {
                theme.disabled
            } else {
                theme.muted
            },
        ),
    ])
}

pub(crate) fn action_line(label: &str, focused: bool, theme: &Theme) -> Line<'static> {
    let pointer = if focused {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    };
    Line::from(vec![Span::raw("  "), pointer, Span::raw(label.to_string())])
}

pub(crate) fn inline_error_line(message: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("! ", theme.error),
        Span::styled(message.to_string(), theme.error),
    ])
}

pub(crate) fn render_surface(frame: &mut Frame, area: Rect, theme: &Theme) {
    frame.render_widget(Block::default().style(theme.surface), area);
}

pub(crate) fn render_inspector(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    body: &[&str],
    theme: &Theme,
) {
    let rule_width = area.width.saturating_sub(2).min(18) as usize;
    let mut lines = vec![
        Line::styled(title.to_string(), theme.emphasis),
        Line::styled("─".repeat(rule_width), theme.structure),
        Line::raw(""),
    ];
    lines.extend(body.iter().map(|line| Line::raw((*line).to_string())));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

pub(crate) fn render_contextual_footer(
    frame: &mut Frame,
    area: Rect,
    hints: &[(&str, &str)],
    theme: &Theme,
) {
    let mut spans = Vec::with_capacity(hints.len() * 2);
    for (index, (key, action)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("    "));
        }
        spans.push(Span::styled((*key).to_string(), theme.emphasis));
        spans.push(Span::styled(format!(" {action}"), theme.muted));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[allow(dead_code)]
pub(crate) fn render_radio_row(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    selected: bool,
    focused: bool,
    theme: &Theme,
) {
    let marker = if selected { "●" } else { "○" };
    let prefix = if focused { "› " } else { "  " };
    let style = if focused { theme.focus } else { theme.normal };
    let line = Line::from(vec![
        Span::styled(prefix, style),
        Span::styled(marker, style.add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::styled(label, style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

#[allow(dead_code)]
pub(crate) fn render_text_input(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    secret: bool,
    theme: &Theme,
) {
    render_text_input_with_cursor(frame, area, label, value, focused, secret, None, theme);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_text_input_with_cursor(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    secret: bool,
    cursor: Option<usize>,
    theme: &Theme,
) {
    let prefix = if focused { "› " } else { "  " };
    let style = if focused { theme.focus } else { theme.normal };
    let masked = if secret {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let mut spans = vec![
        Span::styled(prefix, style),
        Span::styled(format!("{label}: "), theme.dim),
    ];
    if let Some(cursor) = cursor {
        let cursor = cursor.min(masked.chars().count());
        let before = masked.chars().take(cursor).collect::<String>();
        let after = masked.chars().skip(cursor).collect::<String>();
        if !before.is_empty() {
            spans.push(Span::styled(before, style));
        }
        spans.push(Span::styled("█", style));
        if !after.is_empty() {
            spans.push(Span::styled(after, style));
        }
    } else {
        let displayed = if masked.is_empty() {
            "•".to_string()
        } else {
            masked
        };
        spans.push(Span::styled(displayed, style));
    }
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_surface_text_input_with_cursor(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    secret: bool,
    cursor: Option<usize>,
    theme: &Theme,
) {
    let pointer = if focused {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    };
    let style = if focused {
        theme.emphasis
    } else {
        theme.normal
    };
    let masked = if secret {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let mut spans = vec![
        Span::raw("  "),
        pointer,
        Span::styled(format!("{label}: "), theme.muted),
        Span::styled("[", theme.structure),
    ];
    if let Some(cursor) = cursor {
        let cursor = cursor.min(masked.chars().count());
        let before = masked.chars().take(cursor).collect::<String>();
        let after = masked.chars().skip(cursor).collect::<String>();
        if !before.is_empty() {
            spans.push(Span::styled(before, style));
        }
        spans.push(Span::styled(
            "█",
            if focused { theme.pointer } else { style },
        ));
        if !after.is_empty() {
            spans.push(Span::styled(after, style));
        }
    } else {
        spans.push(Span::styled(
            if masked.is_empty() {
                "•".to_string()
            } else {
                masked
            },
            style,
        ));
    }
    spans.push(Span::styled("]", theme.structure));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[allow(dead_code)]
pub(crate) fn render_inline_error(frame: &mut Frame, area: Rect, message: &str, theme: &Theme) {
    frame.render_widget(Paragraph::new(inline_error_line(message, theme)), area);
}

pub(crate) fn render_action_button(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    focused: bool,
    theme: &Theme,
) {
    let style = if focused { theme.focus } else { theme.normal };
    let text = format!("[ {label} ]");
    frame.render_widget(Paragraph::new(text).style(style), area);
}

pub(crate) fn render_footer(frame: &mut Frame, area: Rect, text: &str, theme: &Theme) {
    frame.render_widget(Paragraph::new(text).style(theme.dim), area);
}

#[cfg(test)]
mod tests {
    use ratatui::{
        backend::TestBackend,
        layout::{Constraint, Layout},
        widgets::Paragraph,
        Terminal,
    };

    use super::super::Theme;

    #[test]
    fn widgets_render_the_full_common_surface_in_a_wide_frame() {
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let areas = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(1),
                ])
                .split(area);
                super::render_header(frame, areas[0], "Config Init", "1 / 4", &theme);
                super::render_radio_row(frame, areas[1], "Standalone", true, true, &theme);
                super::render_radio_row(frame, areas[2], "Hub", false, false, &theme);
                super::render_text_input(
                    frame,
                    areas[3],
                    "Tunnel ID",
                    "tunnel_1",
                    true,
                    false,
                    &theme,
                );
                super::render_text_input(frame, areas[4], "Secret", "hidden", false, true, &theme);
                super::render_inline_error(frame, areas[5], "value is required", &theme);
                super::render_action_button(frame, areas[6], "Next", true, &theme);
                super::render_footer(frame, areas[7], "Enter confirm · Ctrl+C cancel", &theme);
            })
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("Config Init"));
        assert!(content.contains("1 / 4"));
        assert!(content.contains("›"));
        assert!(content.contains("●"));
        assert!(content.contains("○"));
        assert!(content.contains("value is required"));
        assert!(content.contains("Next"));
        assert!(content.contains("Ctrl+C cancel"));
    }

    #[test]
    fn widgets_render_without_panicking_in_a_narrow_frame() {
        let backend = TestBackend::new(36, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let areas = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                ])
                .split(area);
                super::render_header(frame, areas[0], "Config Init", "4 / 4", &theme);
                super::render_action_button(frame, areas[1], "Write", true, &theme);
                super::render_footer(frame, areas[2], "Ctrl+C cancel", &theme);
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("Write"));
        assert!(content.contains("Ctrl+C"));
    }

    #[test]
    fn text_input_renders_live_cursor_without_revealing_secret_text() {
        let backend = TestBackend::new(50, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                super::render_text_input_with_cursor(
                    frame,
                    frame.area(),
                    "Tunnel ID",
                    "abc",
                    true,
                    false,
                    Some(1),
                    &theme,
                );
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("a█bc"));

        let backend = TestBackend::new(50, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                super::render_text_input_with_cursor(
                    frame,
                    frame.area(),
                    "Secret",
                    "hidden-secret",
                    true,
                    true,
                    Some(3),
                    &theme,
                );
            })
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("•••█•••••••••"));
        assert!(!content.contains("hidden-secret"));
    }

    #[test]
    fn surface_primitives_render_heading_pointer_selection_inspector_and_footer() {
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                let area = frame.area();
                let areas = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(4),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(area);
                frame.render_widget(
                    Paragraph::new(vec![
                        super::labeled_heading_line("Runtime mode", areas[0].width, &theme),
                        super::surface_choice_line("Hub", true, true, &theme),
                        super::action_line("Next", false, &theme),
                    ]),
                    areas[2],
                );
                super::render_horizontal_rule(frame, areas[1], &theme);
                super::render_surface(frame, areas[3], &theme);
                super::render_inspector(frame, areas[3], "Hub", &["remote dispatch"], &theme);
                super::render_contextual_footer(
                    frame,
                    areas[5],
                    &[("j/k", "move"), ("l", "choose")],
                    &theme,
                );
            })
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("Runtime mode"));
        assert!(content.contains("❯"));
        assert!(content.contains(""));
        assert!(content.contains("Hub"));
        assert!(content.contains("remote dispatch"));
        assert!(content.contains("j/k move"));
    }
}
