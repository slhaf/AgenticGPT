use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
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

pub(crate) fn render_inline_error(frame: &mut Frame, area: Rect, message: &str, theme: &Theme) {
    let line = Line::from(vec![
        Span::styled("! ", theme.error),
        Span::styled(message, theme.error),
    ]);
    frame.render_widget(Paragraph::new(line), area);
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
}
