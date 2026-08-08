#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Standalone,
    Hub,
    Local,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Standalone, Self::Hub, Self::Local];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standalone => "Standalone",
            Self::Hub => "Hub",
            Self::Local => "Local",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Normal,
    Room,
}

impl Profile {
    pub const ALL: [Self; 2] = [Self::Normal, Self::Room];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Room => "Room",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Focus {
    Mode(usize),
    Profile(usize),
    Next,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppState {
    pub mode: Mode,
    pub profile: Profile,
    pub focus: Focus,
    pub border: bool,
    pub advanced: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            mode: Mode::Standalone,
            profile: Profile::Normal,
            focus: Focus::Mode(0),
            border: false,
            advanced: false,
        }
    }
}

impl AppState {
    const FOCUS_ORDER: [Focus; 6] = [
        Focus::Mode(0),
        Focus::Mode(1),
        Focus::Mode(2),
        Focus::Profile(0),
        Focus::Profile(1),
        Focus::Next,
    ];

    pub fn move_down(&mut self) {
        self.focus = self.offset_focus(1);
    }

    pub fn move_up(&mut self) {
        self.focus = self.offset_focus(-1);
    }

    pub fn confirm(&mut self) {
        match self.focus {
            Focus::Mode(index) => self.mode = Mode::ALL[index],
            Focus::Profile(index) => self.profile = Profile::ALL[index],
            Focus::Next => self.advanced = true,
        }
    }

    pub fn toggle_border(&mut self) {
        self.border = !self.border;
    }

    pub fn focus_label(&self) -> &'static str {
        match self.focus {
            Focus::Mode(index) => Mode::ALL[index].label(),
            Focus::Profile(index) => Profile::ALL[index].label(),
            Focus::Next => "下一步",
        }
    }

    fn offset_focus(&self, delta: isize) -> Focus {
        let current = Self::FOCUS_ORDER
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0) as isize;
        let len = Self::FOCUS_ORDER.len() as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        Self::FOCUS_ORDER[next]
    }
}
