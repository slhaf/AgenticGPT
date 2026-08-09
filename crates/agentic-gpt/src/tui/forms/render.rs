use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
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

pub(crate) fn ordered_multi_select_row_line(
    label: &str,
    focused: bool,
    selection_rank: Option<usize>,
    label_width: usize,
    theme: &Theme,
) -> Line<'static> {
    let padding = " ".repeat(label_width.saturating_sub(UnicodeWidthStr::width(label)));
    let selected = selection_rank.map_or_else(
        || Span::raw("   "),
        |rank| Span::styled(format!(" {}", rank + 1), theme.selected),
    );
    Line::from(vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::raw(label.to_string()),
        Span::raw(padding),
        selected,
    ])
}

pub(crate) fn value_row_line(
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
        Span::raw("  "),
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

pub(crate) fn inline_input_spans(
    value: &str,
    editing: bool,
    cursor: Option<usize>,
    width: usize,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let width = width.max(1);
    let window = input_window(
        value,
        editing,
        cursor.unwrap_or(value.chars().count()),
        width,
    );
    let used = UnicodeWidthStr::width(window.text.as_str());
    let remaining = width.saturating_sub(used);
    let left_padding = " ".repeat((remaining + 1) / 2);
    let right_padding = " ".repeat(remaining / 2);
    let style = if editing {
        theme.emphasis.add_modifier(Modifier::BOLD)
    } else {
        theme.emphasis
    };
    let mut spans = vec![
        Span::styled("[", theme.structure),
        Span::styled(left_padding, style),
    ];
    spans.extend(input_window_spans(&window, style, theme));
    spans.push(Span::styled(right_padding, style));
    spans.push(Span::styled("]", theme.structure));
    spans
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
        let inner_width = usize::from(area_width).saturating_sub(4).max(1);
        let window = input_window(value, false, value.chars().count(), inner_width);
        let style = if focused {
            theme.emphasis
        } else {
            theme.normal
        };
        let mut spans = vec![Span::raw("  "), focus_span(focused, theme)];
        spans.extend(input_window_spans(&window, style, theme));
        return Line::from(spans);
    }
    let inner_width = input_inner_width(value, area_width, true);
    let window = input_window(
        value,
        true,
        cursor.unwrap_or(value.chars().count()),
        inner_width,
    );
    let content_width = UnicodeWidthStr::width(window.text.as_str());
    let padding = " ".repeat(inner_width.saturating_sub(content_width));
    let style = theme.emphasis.add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::styled("[", theme.structure),
    ];
    spans.extend(input_window_spans(&window, style, theme));
    spans.push(Span::styled(padding, style));
    spans.push(Span::styled("]", theme.structure));
    Line::from(spans)
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
    let window = input_window(
        &display_value,
        editing,
        cursor.unwrap_or(display_value.chars().count()),
        inner_width,
    );
    let content_width = UnicodeWidthStr::width(window.text.as_str());
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
    let mut spans = vec![
        Span::raw("  "),
        focus_span(focused, theme),
        Span::styled("[", theme.structure),
        Span::styled(left_padding, value_style),
    ];
    spans.extend(input_window_spans(&window, value_style, theme));
    spans.push(Span::styled(right_padding, value_style));
    spans.push(Span::styled("]", theme.structure));
    Line::from(spans)
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct InputWindow {
    text: String,
    left_clipped: bool,
    right_clipped: bool,
}

fn input_window(value: &str, editing: bool, cursor: usize, width: usize) -> InputWindow {
    if width == 0 {
        return InputWindow {
            text: String::new(),
            left_clipped: false,
            right_clipped: false,
        };
    }
    if !editing {
        let value_width = UnicodeWidthStr::width(value);
        return InputWindow {
            text: leading_plain_text(value, width),
            left_clipped: false,
            right_clipped: value_width > width,
        };
    }

    let chars = value.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());
    let before = chars[..cursor].iter().collect::<String>();
    let after = chars[cursor..].iter().collect::<String>();
    let before_width = UnicodeWidthStr::width(before.as_str());
    let after_width = UnicodeWidthStr::width(after.as_str());
    if before_width + 1 + after_width <= width {
        return InputWindow {
            text: format!("{before}█{after}"),
            left_clipped: false,
            right_clipped: false,
        };
    }

    let available = width.saturating_sub(1);
    let preferred_left = available.saturating_mul(2) / 3;
    let mut left_budget = preferred_left.min(before_width);
    let mut right_budget = available.saturating_sub(left_budget).min(after_width);
    let mut spare = available.saturating_sub(left_budget + right_budget);
    if spare > 0 {
        let add_left = spare.min(before_width.saturating_sub(left_budget));
        left_budget += add_left;
        spare -= add_left;
        right_budget += spare.min(after_width.saturating_sub(right_budget));
    }

    let mut left = trailing_plain_text(&before, left_budget);
    let mut right = leading_plain_text(&after, right_budget);
    let mut used = UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(right.as_str());
    let mut cell_spare = available.saturating_sub(used);
    if cell_spare > 0 && UnicodeWidthStr::width(left.as_str()) < before_width {
        left = trailing_plain_text(&before, left_budget + cell_spare);
        used = UnicodeWidthStr::width(left.as_str()) + UnicodeWidthStr::width(right.as_str());
        cell_spare = available.saturating_sub(used);
    }
    if cell_spare > 0 && UnicodeWidthStr::width(right.as_str()) < after_width {
        right = leading_plain_text(&after, right_budget + cell_spare);
    }

    let left_clipped = UnicodeWidthStr::width(left.as_str()) < before_width;
    let right_clipped = UnicodeWidthStr::width(right.as_str()) < after_width;
    InputWindow {
        text: format!("{left}█{right}"),
        left_clipped,
        right_clipped,
    }
}

fn input_window_spans(
    window: &InputWindow,
    base_style: Style,
    theme: &Theme,
) -> Vec<Span<'static>> {
    const FADE_CHARS: usize = 4;

    let chars = window.text.chars().collect::<Vec<_>>();
    let char_count = chars.len();
    let cursor_index = chars.iter().position(|ch| *ch == '█');
    let left_visible = cursor_index.unwrap_or(char_count);
    let right_visible = cursor_index
        .map(|index| char_count.saturating_sub(index + 1))
        .unwrap_or(char_count);
    let left_fade = if window.left_clipped {
        left_visible.min(FADE_CHARS)
    } else {
        0
    };
    let right_fade = if window.right_clipped {
        right_visible.min(FADE_CHARS)
    } else {
        0
    };

    chars
        .into_iter()
        .enumerate()
        .map(|(index, ch)| {
            let style = if left_fade > 0 && index < left_fade {
                fade_style(base_style, theme, index + 1, left_fade + 1)
            } else if right_fade > 0 && index >= char_count.saturating_sub(right_fade) {
                let distance_from_right = char_count.saturating_sub(index);
                fade_style(base_style, theme, distance_from_right, right_fade + 1)
            } else {
                base_style
            };
            Span::styled(ch.to_string(), style)
        })
        .collect()
}

fn fade_style(base_style: Style, theme: &Theme, numerator: usize, denominator: usize) -> Style {
    match (base_style.fg, theme.surface.bg) {
        (Some(Color::Rgb(fr, fg, fb)), Some(Color::Rgb(br, bg, bb))) => {
            let numerator = numerator.min(denominator) as u16;
            let denominator = denominator.max(1) as u16;
            let blend = |background: u8, foreground: u8| -> u8 {
                let background = u16::from(background);
                let foreground = u16::from(foreground);
                ((background * (denominator - numerator) + foreground * numerator) / denominator)
                    as u8
            };
            base_style.fg(Color::Rgb(blend(br, fr), blend(bg, fg), blend(bb, fb)))
        }
        _ if numerator.saturating_mul(2) <= denominator => base_style.add_modifier(Modifier::DIM),
        _ => base_style,
    }
}

fn leading_plain_text(value: &str, width: usize) -> String {
    let mut used = 0usize;
    let mut out = String::new();
    for ch in value.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + char_width > width {
            break;
        }
        out.push(ch);
        used += char_width;
    }
    out
}

fn trailing_plain_text(value: &str, width: usize) -> String {
    let mut used = 0usize;
    let mut chars = Vec::new();
    for ch in value.chars().rev() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + char_width > width {
            break;
        }
        chars.push(ch);
        used += char_width;
    }
    chars.reverse();
    chars.into_iter().collect()
}
