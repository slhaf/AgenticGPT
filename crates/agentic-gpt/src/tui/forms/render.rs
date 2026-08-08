use ratatui::{
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::super::Theme;

const MIN_INPUT_INNER_WIDTH: usize = 8;
const MAX_INPUT_INNER_WIDTH: usize = 36;

pub(crate) fn subsection_heading_line(label: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled("◆ ", theme.emphasis),
        Span::styled(label.to_string(), theme.emphasis),
    ])
}

pub(crate) fn choice_row_line(
    label: &str,
    focused: bool,
    selected: bool,
    label_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let padding = " ".repeat(label_width.saturating_sub(UnicodeWidthStr::width(label)));
    Line::from(vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::raw(label.to_string()),
        Span::raw(padding),
        if selected {
            Span::styled("", theme.selected)
        } else {
            Span::raw(" ")
        },
    ])
}

pub(crate) fn boolean_row_line(
    label: &str,
    value: &str,
    focused: bool,
    label_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let padding = " ".repeat(label_width.saturating_sub(UnicodeWidthStr::width(label)));
    Line::from(vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::raw(label.to_string()),
        Span::raw(padding),
        Span::styled(
            value.to_string(),
            if focused {
                theme.emphasis
            } else {
                theme.normal
            },
        ),
    ])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn input_row_line(
    label: &str,
    value: &str,
    focused: bool,
    editing: bool,
    cursor: Option<usize>,
    label_width: usize,
    input_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let padding = " ".repeat(label_width.saturating_sub(UnicodeWidthStr::width(label)));
    let mut spans = vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::raw(label.to_string()),
        Span::raw(padding),
        Span::raw("  "),
    ];
    spans.extend(inline_input_spans(
        value,
        editing,
        cursor,
        input_width,
        theme,
    ));
    Line::from(spans)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn choice_input_row_line(
    label: &str,
    value: &str,
    focused: bool,
    selected: bool,
    editing: bool,
    cursor: Option<usize>,
    label_width: usize,
    input_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let padding = " ".repeat(label_width.saturating_sub(UnicodeWidthStr::width(label)));
    let mut spans = vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::raw(label.to_string()),
        Span::raw(padding),
        if selected {
            Span::styled("", theme.selected)
        } else {
            Span::raw(" ")
        },
        Span::raw("  "),
    ];
    spans.extend(inline_input_spans(
        value,
        editing,
        cursor,
        input_width,
        theme,
    ));
    Line::from(spans)
}

fn inline_input_spans(
    value: &str,
    editing: bool,
    cursor: Option<usize>,
    width: usize,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let width = width.max(1);
    let content = input_content(
        value,
        editing,
        cursor.unwrap_or(value.chars().count()),
        width,
    );
    let used = UnicodeWidthStr::width(content.as_str());
    let remaining = width.saturating_sub(used);
    let left_padding = " ".repeat((remaining + 1) / 2);
    let right_padding = " ".repeat(remaining / 2);
    let style = if editing {
        theme.emphasis.add_modifier(Modifier::BOLD)
    } else {
        theme.emphasis
    };
    vec![
        Span::styled("[", theme.structure),
        Span::styled(left_padding, style),
        Span::styled(content, style),
        Span::styled(right_padding, style),
        Span::styled("]", theme.structure),
    ]
}

pub(crate) fn numeric_input_value_line(
    value: &str,
    focused: bool,
    editing: bool,
    cursor: Option<usize>,
    theme: &Theme,
) -> Line<'static> {
    let mut spans = vec![Span::raw("  "), focus_span(focused, theme)];
    spans.extend(inline_input_spans(value, editing, cursor, 5, theme));
    Line::from(spans)
}

pub(crate) fn list_item_line(value: &str, focused: bool, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::raw(value.to_string()),
    ])
}

pub(crate) fn editable_list_item_line(
    value: &str,
    focused: bool,
    editing: bool,
    cursor: Option<usize>,
    area_width: u16,
    theme: &Theme,
) -> Line<'static> {
    if !editing {
        return list_item_line(value, focused, theme);
    }
    let inner_width = input_inner_width(value, area_width, true);
    let content = input_content(
        value,
        true,
        cursor.unwrap_or(value.chars().count()),
        inner_width,
    );
    let content_width = UnicodeWidthStr::width(content.as_str());
    let padding = " ".repeat(inner_width.saturating_sub(content_width));
    Line::from(vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::styled("[", theme.structure),
        Span::styled(content, theme.emphasis.add_modifier(Modifier::BOLD)),
        Span::styled(padding, theme.emphasis.add_modifier(Modifier::BOLD)),
        Span::styled("]", theme.structure),
    ])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn long_form_input_value_line(
    value: &str,
    focused: bool,
    editing: bool,
    cursor: Option<usize>,
    secret: bool,
    is_default: bool,
    area_width: u16,
    theme: &Theme,
) -> Line<'static> {
    let display_value = if secret {
        "•".repeat(value.chars().count())
    } else {
        value.to_string()
    };
    let inner_width = input_inner_width(&display_value, area_width, editing);
    let content = input_content(
        &display_value,
        editing,
        cursor.unwrap_or(display_value.chars().count()),
        inner_width,
    );
    let content_width = UnicodeWidthStr::width(content.as_str());
    let remaining = inner_width.saturating_sub(content_width);
    let left_padding = " ".repeat((remaining + 1) / 2);
    let right_padding = " ".repeat(remaining / 2);
    let value_style = if editing {
        theme.emphasis.add_modifier(Modifier::BOLD)
    } else if is_default {
        theme.muted.add_modifier(Modifier::DIM)
    } else {
        theme.emphasis
    };
    Line::from(vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::styled("[", theme.structure),
        Span::styled(left_padding, value_style),
        Span::styled(content, value_style),
        Span::styled(right_padding, value_style),
        Span::styled("]", theme.structure),
    ])
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_long_form_input(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    focused: bool,
    editing: bool,
    cursor: Option<usize>,
    secret: bool,
    is_default: bool,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(
        Paragraph::new(subsection_heading_line(label, theme)),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );
    if area.height < 2 {
        return;
    }

    frame.render_widget(
        Paragraph::new(long_form_input_value_line(
            value, focused, editing, cursor, secret, is_default, area.width, theme,
        )),
        Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: 1,
        },
    );
}

fn focus_span(focused: bool, theme: &Theme) -> Span<'static> {
    if focused {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    }
}

fn input_inner_width(value: &str, area_width: u16, editing: bool) -> usize {
    let max_inner = area_width
        .saturating_sub(6)
        .min(MAX_INPUT_INNER_WIDTH as u16) as usize;
    if max_inner == 0 {
        return 0;
    }
    let min_inner = max_inner.min(MIN_INPUT_INNER_WIDTH);
    let desired = UnicodeWidthStr::width(value) + usize::from(editing);
    desired.clamp(min_inner, max_inner)
}

fn input_content(value: &str, editing: bool, cursor: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if !editing {
        return leading_text(value, width);
    }

    let chars = value.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());
    let before = chars[..cursor].iter().collect::<String>();
    let after = chars[cursor..].iter().collect::<String>();
    if UnicodeWidthStr::width(before.as_str()) + 1 + UnicodeWidthStr::width(after.as_str()) <= width
    {
        return format!("{before}█{after}");
    }

    let available = width.saturating_sub(1);
    let left_budget = available.saturating_mul(2) / 3;
    let right_budget = available.saturating_sub(left_budget);
    let left = trailing_text(&before, left_budget);
    let right = leading_text(&after, right_budget);
    format!("{left}█{right}")
}

fn leading_text(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let target = width.saturating_sub(1);
    let mut used = 0usize;
    let mut out = String::new();
    for ch in value.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + char_width > target {
            break;
        }
        out.push(ch);
        used += char_width;
    }
    out.push('…');
    out
}

fn trailing_text(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let target = width.saturating_sub(1);
    let mut used = 0usize;
    let mut chars = Vec::new();
    for ch in value.chars().rev() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + char_width > target {
            break;
        }
        chars.push(ch);
        used += char_width;
    }
    chars.reverse();
    let mut out = String::from("…");
    out.extend(chars);
    out
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, layout::Rect, widgets::Paragraph, Terminal};

    use super::*;

    fn row_text(terminal: &Terminal<TestBackend>, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| {
                terminal
                    .backend()
                    .buffer()
                    .cell((x, y))
                    .map(|cell| cell.symbol())
                    .unwrap_or(" ")
            })
            .collect::<String>()
    }

    #[test]
    fn subsection_heading_uses_diamond_without_nested_rule() {
        let backend = TestBackend::new(30, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(subsection_heading_line("任务并发", &theme)),
                    frame.area(),
                );
            })
            .unwrap();
        let row = row_text(&terminal, 0, 30);
        assert!(row.contains('◆'));
        for character in "任务并发".chars() {
            assert!(row.contains(character));
        }
        assert!(!row.contains('─'));
    }

    #[test]
    fn empty_input_is_blank_and_cursor_only_exists_while_editing() {
        let backend = TestBackend::new(42, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                render_long_form_input(
                    frame,
                    Rect::new(0, 0, 42, 2),
                    "显示名称",
                    "",
                    true,
                    false,
                    None,
                    false,
                    false,
                    &theme,
                );
            })
            .unwrap();
        let idle = row_text(&terminal, 1, 42);
        assert!(idle.contains("[        ]"));
        assert!(!idle.contains('•'));
        assert!(!idle.contains('█'));

        terminal
            .draw(|frame| {
                render_long_form_input(
                    frame,
                    Rect::new(0, 0, 42, 2),
                    "显示名称",
                    "abc",
                    true,
                    true,
                    Some(1),
                    false,
                    false,
                    &theme,
                );
            })
            .unwrap();
        let editing = row_text(&terminal, 1, 42);
        assert!(editing.contains("a█bc"));
    }

    #[test]
    fn short_long_form_values_are_centered_inside_the_input() {
        let backend = TestBackend::new(42, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                render_long_form_input(
                    frame,
                    Rect::new(0, 0, 42, 2),
                    "Bubblewrap path",
                    "bwrap",
                    true,
                    false,
                    None,
                    false,
                    false,
                    &theme,
                );
            })
            .unwrap();
        let value = row_text(&terminal, 1, 42);
        assert!(value.contains("[  bwrap ]"));
    }

    #[test]
    fn choice_marker_and_boolean_value_have_distinct_semantics() {
        let backend = TestBackend::new(42, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(choice_row_line("file", false, true, 12, &theme)),
                    Rect::new(0, 0, 42, 1),
                );
                frame.render_widget(
                    Paragraph::new(choice_row_line("env", true, false, 12, &theme)),
                    Rect::new(0, 1, 42, 1),
                );
                frame.render_widget(
                    Paragraph::new(boolean_row_line("启用沙箱", "关", true, 18, &theme)),
                    Rect::new(0, 2, 42, 1),
                );
            })
            .unwrap();
        let file = row_text(&terminal, 0, 42);
        let env = row_text(&terminal, 1, 42);
        let boolean = row_text(&terminal, 2, 42);
        assert!(file.contains(''));
        assert!(!file.contains('❯'));
        assert!(env.contains('❯'));
        assert!(!env.contains(''));
        assert!(boolean.contains('❯'));
        for character in "启用沙箱".chars() {
            assert!(boolean.contains(character));
        }
        assert!(boolean.contains('关'));
        assert!(!boolean.contains(''));
    }

    #[test]
    fn inline_choice_input_keeps_focus_selection_and_edit_cursor_distinct() {
        let backend = TestBackend::new(50, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(choice_input_row_line(
                        "Custom",
                        "12",
                        true,
                        false,
                        true,
                        Some(1),
                        21,
                        5,
                        &theme,
                    )),
                    Rect::new(0, 0, 50, 1),
                );
                frame.render_widget(
                    Paragraph::new(choice_input_row_line(
                        "Custom", "12", false, true, false, None, 21, 5, &theme,
                    )),
                    Rect::new(0, 1, 50, 1),
                );
                frame.render_widget(
                    Paragraph::new(input_row_line(
                        "Max concurrent",
                        "2",
                        true,
                        false,
                        None,
                        22,
                        5,
                        &theme,
                    )),
                    Rect::new(0, 2, 50, 1),
                );
            })
            .unwrap();

        let editing = row_text(&terminal, 0, 50);
        assert!(editing.contains('❯'));
        assert!(!editing.contains(''));
        assert!(editing.contains("[ 1█2 ]"));

        let selected = row_text(&terminal, 1, 50);
        assert!(!selected.contains('❯'));
        assert!(selected.contains(''));
        assert!(selected.contains("[  12 ]"));

        let numeric = row_text(&terminal, 2, 50);
        assert!(numeric.contains('❯'));
        assert!(!numeric.contains(''));
        assert!(numeric.contains("[  2  ]"));
    }

    #[test]
    fn long_form_width_uses_terminal_cells_for_cjk() {
        let backend = TestBackend::new(28, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::from_env();
        terminal
            .draw(|frame| {
                render_long_form_input(
                    frame,
                    Rect::new(0, 0, 28, 2),
                    "路径",
                    "中文路径/abcdef",
                    true,
                    false,
                    None,
                    false,
                    false,
                    &theme,
                );
            })
            .unwrap();
        let row = row_text(&terminal, 1, 28);
        for character in "中文路径".chars() {
            assert!(row.contains(character));
        }
        assert!(row.contains("/abcdef"));
        assert!(row.contains(']'));
    }
}
