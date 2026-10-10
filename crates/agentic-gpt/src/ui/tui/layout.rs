use ratatui::layout::{Constraint, Layout, Rect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneMode {
    Master,
    Detail,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MasterDetailSpec {
    pub(crate) master: Constraint,
    pub(crate) gap: u16,
    pub(crate) detail: Constraint,
    pub(crate) collapse_below: u16,
}

impl MasterDetailSpec {
    pub(crate) const fn new(
        master: Constraint,
        gap: u16,
        detail: Constraint,
        collapse_below: u16,
    ) -> Self {
        Self {
            master,
            gap,
            detail,
            collapse_below,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct MasterDetailAreas {
    pub(crate) master: Rect,
    pub(crate) gap: Rect,
    pub(crate) detail: Rect,
}

pub(crate) fn master_detail_layout(
    area: Rect,
    spec: MasterDetailSpec,
    mode: PaneMode,
) -> MasterDetailAreas {
    if area.width < spec.collapse_below {
        return match mode {
            PaneMode::Master => MasterDetailAreas {
                master: area,
                ..MasterDetailAreas::default()
            },
            PaneMode::Detail => MasterDetailAreas {
                detail: area,
                ..MasterDetailAreas::default()
            },
        };
    }

    let [master, gap, detail] =
        Layout::horizontal([spec.master, Constraint::Length(spec.gap), spec.detail]).areas(area);
    MasterDetailAreas {
        master,
        gap,
        detail,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceCursor {
    area: Rect,
    cursor_y: u16,
    reserve_bottom: u16,
}

impl SurfaceCursor {
    pub(crate) fn new(area: Rect) -> Self {
        Self {
            area,
            cursor_y: area.y,
            reserve_bottom: 0,
        }
    }

    pub(crate) fn reserve_bottom(mut self, rows: u16) -> Self {
        self.reserve_bottom = rows;
        self
    }

    pub(crate) fn rows(&mut self, height: u16) -> Option<Rect> {
        let limit = self
            .area
            .y
            .saturating_add(self.area.height.saturating_sub(self.reserve_bottom));
        if height == 0 || self.cursor_y.saturating_add(height) > limit {
            return None;
        }
        let rows = Rect {
            x: self.area.x,
            y: self.cursor_y,
            width: self.area.width,
            height,
        };
        self.cursor_y = self.cursor_y.saturating_add(height);
        Some(rows)
    }
}
