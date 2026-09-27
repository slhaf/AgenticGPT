mod app;
pub(crate) mod forms;
mod layout;
mod process;
mod runtime;
mod shell;
mod theme;
mod widgets;
mod workspace;

pub(crate) use app::{TuiApp, TuiOutcome};
pub(crate) use layout::{master_detail_layout, MasterDetailSpec, PaneMode, SurfaceCursor};
pub(crate) use process::{ProcessScreen, ProcessUpdate};
pub(crate) use runtime::{TerminalEvent, TerminalSession};
pub(crate) use shell::{centered_overlay, surface_shell_areas};
pub(crate) use theme::Theme;
pub(crate) use widgets::{
    action_line, inline_error_line, labeled_heading_line, render_action_button,
    render_contextual_footer, render_footer, render_header, render_horizontal_rule,
    render_inspector, render_surface, render_surface_action_dock, render_surface_header,
    surface_choice_line, surface_status_line,
};
pub(crate) use workspace::WorkspaceState;
