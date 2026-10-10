use ratatui::layout::{Constraint, Layout, Margin, Rect};

pub(crate) fn surface_shell_areas(area: Rect) -> [Rect; 5] {
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

pub(crate) fn centered_overlay(area: Rect, preferred_width: u16, preferred_height: u16) -> Rect {
    let width = preferred_width.min(area.width.saturating_sub(4)).max(1);
    let height = preferred_height.min(area.height.saturating_sub(2)).max(1);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}
