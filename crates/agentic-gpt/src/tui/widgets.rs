use unicode_width::UnicodeWidthStr;

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

const SURFACE_LOCAL_RULE_TRAILING_GAP: u16 = 7;

pub(crate) fn surface_local_rule_width(width: u16) -> u16 {
    width.saturating_sub(SURFACE_LOCAL_RULE_TRAILING_GAP)
}

pub(crate) fn labeled_heading_line(label: &str, width: u16, theme: &Theme) -> Line<'static> {
    let prefix = format!("── {label} ");
    let fill_width = usize::from(surface_local_rule_width(width))
        .saturating_sub(UnicodeWidthStr::width(prefix.as_str()));
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
