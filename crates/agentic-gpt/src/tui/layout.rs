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

#[cfg(test)]
mod tests {
    use ratatui::layout::{Constraint, Rect};

    use super::{master_detail_layout, MasterDetailSpec, PaneMode, SurfaceCursor};

    const SPEC: MasterDetailSpec =
        MasterDetailSpec::new(Constraint::Min(40), 2, Constraint::Min(24), 66);

    #[test]
    fn master_detail_collapses_to_active_pane() {
        let area = Rect::new(0, 0, 60, 20);
        let master = master_detail_layout(area, SPEC, PaneMode::Master);
        assert_eq!(master.master, area);
        assert_eq!(master.detail, Rect::default());

        let detail = master_detail_layout(area, SPEC, PaneMode::Detail);
        assert_eq!(detail.master, Rect::default());
        assert_eq!(detail.detail, area);
    }

    #[test]
    fn surface_cursor_honors_reserved_bottom_rows() {
        let mut cursor = SurfaceCursor::new(Rect::new(2, 3, 20, 6)).reserve_bottom(2);
        assert_eq!(cursor.rows(3), Some(Rect::new(2, 3, 20, 3)));
        assert_eq!(cursor.rows(1), Some(Rect::new(2, 6, 20, 1)));
        assert_eq!(cursor.rows(1), None);
    }
}
