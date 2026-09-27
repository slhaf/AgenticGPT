use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Theme {
    pub(crate) accent: Style,
    pub(crate) focus: Style,
    pub(crate) pointer: Style,
    pub(crate) normal: Style,
    pub(crate) dim: Style,
    pub(crate) emphasis: Style,
    pub(crate) selected: Style,
    pub(crate) structure: Style,
    pub(crate) muted: Style,
    pub(crate) surface: Style,
    #[allow(dead_code)]
    pub(crate) success: Style,
    pub(crate) error: Style,
    pub(crate) disabled: Style,
}

impl Theme {
    pub(crate) fn from_env() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            Self::no_color()
        } else {
            Self {
                accent: Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                focus: Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
                pointer: Style::default()
                    .fg(Color::Rgb(88, 190, 200))
                    .add_modifier(Modifier::BOLD),
                normal: Style::default().fg(Color::White),
                dim: Style::default().fg(Color::DarkGray),
                emphasis: Style::default()
                    .fg(Color::Rgb(224, 226, 228))
                    .add_modifier(Modifier::BOLD),
                selected: Style::default()
                    .fg(Color::Rgb(224, 226, 228))
                    .add_modifier(Modifier::BOLD),
                structure: Style::default().fg(Color::Rgb(88, 94, 100)),
                muted: Style::default().fg(Color::Rgb(120, 126, 132)),
                surface: Style::default().bg(Color::Rgb(24, 26, 28)),
                success: Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
                error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                disabled: Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            }
        }
    }

    fn no_color() -> Self {
        Self {
            accent: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            focus: Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
            pointer: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            normal: Style::default(),
            dim: Style::default().add_modifier(Modifier::DIM),
            emphasis: Style::default().add_modifier(Modifier::BOLD),
            selected: Style::default().add_modifier(Modifier::BOLD),
            structure: Style::default().add_modifier(Modifier::DIM),
            muted: Style::default().add_modifier(Modifier::DIM),
            surface: Style::default(),
            success: Style::default().add_modifier(Modifier::BOLD),
            error: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            disabled: Style::default().add_modifier(Modifier::DIM | Modifier::CROSSED_OUT),
        }
    }
}
