use std::{io, sync::OnceLock, time::Duration};

use agentic_tui_ux_demo::{AppState, Focus, Mode, Profile};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

static FIXED_COLORS: OnceLock<bool> = OnceLock::new();

#[derive(Clone, Copy, Eq, PartialEq)]
enum DemoPage {
    Basic,
    Optional,
    Identity,
    Limits,
    Review,
}

#[derive(Clone, Copy)]
enum ReviewTarget {
    Basic,
    Identity(usize),
    Limits(usize),
    Optional(usize),
    PreviewOnly,
}

struct ReviewRow {
    group: &'static str,
    label: &'static str,
    value: String,
    target: ReviewTarget,
}

enum ReviewEditor {
    Choice {
        row: usize,
        cursor: usize,
        options: &'static [&'static str],
    },
    Text {
        row: usize,
        buffer: String,
        numeric: bool,
    },
    AutoCustom {
        cursor: usize,
        buffer: String,
        editing_number: bool,
    },
}

struct ReviewState {
    focus: usize,
    editor: Option<ReviewEditor>,
    overrides: Vec<Option<String>>,
    search_input: Option<String>,
    search_origin: usize,
    last_search: Option<String>,
}

impl Default for ReviewState {
    fn default() -> Self {
        Self {
            focus: 0,
            editor: None,
            overrides: vec![None; 24],
            search_input: None,
            search_origin: 0,
            last_search: None,
        }
    }
}

impl ReviewState {
    fn move_focus(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.focus = 0;
            return;
        }
        self.focus = (self.focus as isize + delta).rem_euclid(len as isize) as usize;
    }

    fn override_value(&self, row: usize, fallback: &str) -> String {
        self.overrides
            .get(row)
            .and_then(|value| value.clone())
            .unwrap_or_else(|| fallback.to_string())
    }

    fn set_override(&mut self, row: usize, value: String) {
        if let Some(slot) = self.overrides.get_mut(row) {
            *slot = Some(value);
        }
    }

    fn begin_search(&mut self) {
        self.search_origin = self.focus;
        self.search_input = Some(String::new());
    }

    fn cancel_search(&mut self) {
        self.focus = self.search_origin;
        self.search_input = None;
    }

    fn commit_search(&mut self) {
        if let Some(query) = self.search_input.take() {
            if !query.is_empty() {
                self.last_search = Some(query);
            }
        }
    }

    fn update_incremental_search(&mut self, rows: &[ReviewRow]) {
        let Some(query) = self.search_input.as_deref() else {
            return;
        };
        if query.is_empty() {
            self.focus = self.search_origin;
            return;
        }
        if let Some(index) = find_review_match(rows, self.search_origin, query, 1, true) {
            self.focus = index;
        }
    }

    fn repeat_search(&mut self, rows: &[ReviewRow], direction: isize) {
        let Some(query) = self.last_search.as_deref() else {
            return;
        };
        if let Some(index) = find_review_match(rows, self.focus, query, direction, false) {
            self.focus = index;
        }
    }
}

fn review_row_matches(row: &ReviewRow, query: &str) -> bool {
    let query = query.to_lowercase();
    row.group.to_lowercase().contains(&query)
        || row.label.to_lowercase().contains(&query)
        || row.value.to_lowercase().contains(&query)
}

fn find_review_match(
    rows: &[ReviewRow],
    start: usize,
    query: &str,
    direction: isize,
    include_start: bool,
) -> Option<usize> {
    if rows.is_empty() || query.is_empty() {
        return None;
    }
    for step in 0..rows.len() {
        let offset = if include_start { step } else { step + 1 } as isize;
        let index = (start as isize + direction * offset).rem_euclid(rows.len() as isize) as usize;
        if review_row_matches(&rows[index], query) {
            return Some(index);
        }
    }
    None
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum EditField {
    MaxTasks,
    CustomJobs,
    ContextLines,
}

struct IdentityState {
    focus: usize,
    display_name: String,
    agent_id: String,
    partner_home: String,
    editing: Option<usize>,
    edit_buffer: String,
}

impl Default for IdentityState {
    fn default() -> Self {
        Self {
            focus: 0,
            display_name: String::new(),
            agent_id: "laptop".to_string(),
            partner_home: "~/Projects/Partner".to_string(),
            editing: None,
            edit_buffer: String::new(),
        }
    }
}

impl IdentityState {
    fn value(&self, field: usize) -> &str {
        match field {
            0 => &self.display_name,
            1 => &self.agent_id,
            2 => &self.partner_home,
            _ => "",
        }
    }

    fn is_default_field(&self, field: usize) -> bool {
        match field {
            0 => self.display_name.is_empty(),
            1 => self.agent_id == "laptop",
            2 => self.partner_home == "~/Projects/Partner",
            _ => false,
        }
    }

    fn start_edit(&mut self, field: usize) {
        self.focus = field;
        self.editing = Some(field);
        self.edit_buffer = self.value(field).to_string();
    }

    fn cancel_edit(&mut self) {
        self.editing = None;
        self.edit_buffer.clear();
    }

    fn commit_edit(&mut self) {
        let Some(field) = self.editing else {
            return;
        };
        let value = self.edit_buffer.trim().to_string();
        match field {
            0 => self.display_name = value,
            1 => self.agent_id = value,
            2 => self.partner_home = value,
            _ => {}
        }
        self.editing = None;
        self.edit_buffer.clear();
        self.focus = if field < 2 { field + 1 } else { 3 };
    }

    fn move_focus(&mut self, delta: isize) {
        self.focus = (self.focus as isize + delta).rem_euclid(4) as usize;
    }

    fn is_modified(&self) -> bool {
        !self.display_name.is_empty()
            || self.agent_id != "laptop"
            || self.partner_home != "~/Projects/Partner"
    }
}

struct LimitsState {
    focus: usize,
    max_tasks: u16,
    jobs_auto: bool,
    custom_jobs: u16,
    context_lines: u16,
    editing: Option<EditField>,
    edit_buffer: String,
    error: Option<&'static str>,
}

impl Default for LimitsState {
    fn default() -> Self {
        Self {
            focus: 0,
            max_tasks: 2,
            jobs_auto: true,
            custom_jobs: 12,
            context_lines: 5,
            editing: None,
            edit_buffer: String::new(),
            error: None,
        }
    }
}

impl LimitsState {
    fn move_focus(&mut self, delta: isize) {
        self.focus = (self.focus as isize + delta).rem_euclid(5) as usize;
        self.error = None;
    }

    fn start_edit(&mut self, field: EditField) {
        self.editing = Some(field);
        self.edit_buffer = match field {
            EditField::MaxTasks => self.max_tasks.to_string(),
            EditField::CustomJobs => self.custom_jobs.to_string(),
            EditField::ContextLines => self.context_lines.to_string(),
        };
        self.error = None;
    }

    fn cancel_edit(&mut self) {
        self.editing = None;
        self.edit_buffer.clear();
        self.error = None;
    }

    fn commit_edit(&mut self) {
        let Some(field) = self.editing else {
            return;
        };
        let Ok(value) = self.edit_buffer.parse::<u16>() else {
            self.error = Some("请输入有效数字");
            return;
        };
        if field != EditField::ContextLines && value == 0 {
            self.error = Some("数值必须大于 0");
            return;
        }
        match field {
            EditField::MaxTasks => self.max_tasks = value,
            EditField::CustomJobs => self.custom_jobs = value,
            EditField::ContextLines => self.context_lines = value,
        }
        self.editing = None;
        self.edit_buffer.clear();
        self.error = None;
    }

    fn is_modified(&self) -> bool {
        self.max_tasks != 2 || !self.jobs_auto || self.custom_jobs != 12 || self.context_lines != 5
    }
}

#[derive(Clone, Copy)]
enum VisualVariant {
    Minimal,
    FocusFrame,
    Surface,
    Labeled,
}

const OPTIONAL_LABELS: [&str; 8] = [
    "身份",
    "工作区",
    "确认",
    "限制",
    "沙箱",
    "Room",
    "隧道客户端",
    "Hub 报告",
];

impl VisualVariant {
    fn next(self) -> Self {
        match self {
            Self::Minimal => Self::FocusFrame,
            Self::FocusFrame => Self::Surface,
            Self::Surface => Self::Labeled,
            Self::Labeled => Self::Minimal,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Minimal => "A Minimal",
            Self::FocusFrame => "B Frame",
            Self::Surface => "C Surface",
            Self::Labeled => "D Labeled",
        }
    }
}

fn fixed_colors() -> bool {
    *FIXED_COLORS.get_or_init(|| std::env::args().any(|arg| arg == "--fixed-colors"))
}

fn focus_color() -> Color {
    if fixed_colors() {
        Color::Rgb(88, 190, 200)
    } else {
        Color::Cyan
    }
}

fn selected_color() -> Color {
    emphasis_color()
}

fn emphasis_color() -> Color {
    if fixed_colors() {
        Color::Rgb(224, 226, 228)
    } else {
        Color::White
    }
}

fn structure_color() -> Color {
    if fixed_colors() {
        Color::Rgb(88, 94, 100)
    } else {
        Color::DarkGray
    }
}

fn muted_color() -> Color {
    if fixed_colors() {
        Color::Rgb(120, 126, 132)
    } else {
        Color::DarkGray
    }
}

fn surface_color() -> Color {
    if fixed_colors() {
        Color::Rgb(24, 26, 28)
    } else {
        Color::Black
    }
}

fn error_color() -> Color {
    if fixed_colors() {
        Color::Rgb(220, 105, 105)
    } else {
        Color::Red
    }
}

fn optional_available(index: usize, app: &AppState) -> bool {
    match index {
        5 => app.profile == Profile::Room,
        6 | 7 => app.mode == Mode::Standalone,
        8 => true,
        _ => index < OPTIONAL_LABELS.len(),
    }
}

fn optional_status(
    index: usize,
    app: &AppState,
    identity: &IdentityState,
    limits: &LimitsState,
) -> &'static str {
    match index {
        0 if identity.is_modified() => "已修改",
        0 => "未设置",
        3 if limits.is_modified() => "已修改",
        5..=7 if !optional_available(index, app) => "不适用",
        0..=7 => "默认",
        _ => "",
    }
}

fn move_optional_focus(current: usize, delta: isize, app: &AppState) -> usize {
    let len = OPTIONAL_LABELS.len() + 1;
    for step in 1..=len {
        let next = (current as isize + delta * step as isize).rem_euclid(len as isize) as usize;
        if optional_available(next, app) {
            return next;
        }
    }
    current
}

fn review_rows(
    app: &AppState,
    identity: &IdentityState,
    limits: &LimitsState,
    review: &ReviewState,
) -> Vec<ReviewRow> {
    let mode = match app.mode {
        Mode::Standalone => "Standalone",
        Mode::Hub => "Hub",
        Mode::Local => "Local",
    };
    let profile = match app.profile {
        Profile::Normal => "Normal",
        Profile::Room => "Room",
    };
    let jobs = if limits.jobs_auto {
        "auto".to_string()
    } else {
        limits.custom_jobs.to_string()
    };

    let mut rows = vec![
        ReviewRow { group: "基础", label: "运行模式", value: mode.to_string(), target: ReviewTarget::Basic },
        ReviewRow { group: "基础", label: "能力配置", value: profile.to_string(), target: ReviewTarget::Basic },
        ReviewRow { group: "身份", label: "显示名称", value: if identity.display_name.is_empty() { "未设置".to_string() } else { identity.display_name.clone() }, target: ReviewTarget::Identity(0) },
        ReviewRow { group: "身份", label: "Agent ID", value: identity.agent_id.clone(), target: ReviewTarget::Identity(1) },
        ReviewRow { group: "身份", label: "Partner Home", value: identity.partner_home.clone(), target: ReviewTarget::Identity(2) },
        ReviewRow { group: "工作区", label: "根目录", value: "~/.agentic_gpt/workspace".to_string(), target: ReviewTarget::Optional(1) },
        ReviewRow { group: "工作区", label: "写入路径", value: "~/.agentic_gpt/workspace".to_string(), target: ReviewTarget::Optional(1) },
        ReviewRow { group: "工作区", label: "写入路径", value: "~/Projects/AgenticGPT".to_string(), target: ReviewTarget::Optional(1) },
        ReviewRow { group: "工作区", label: "只读路径", value: "/etc/os-release".to_string(), target: ReviewTarget::Optional(1) },
        ReviewRow { group: "工作区", label: "拒绝路径", value: "~/.ssh".to_string(), target: ReviewTarget::Optional(1) },
        ReviewRow { group: "确认", label: "确认通道", value: "桌面通知 → ntfy".to_string(), target: ReviewTarget::Optional(2) },
        ReviewRow { group: "确认", label: "语言", value: "zh-CN".to_string(), target: ReviewTarget::Optional(2) },
        ReviewRow { group: "限制", label: "最大并发任务", value: limits.max_tasks.to_string(), target: ReviewTarget::Limits(0) },
        ReviewRow { group: "限制", label: "最大活动作业", value: jobs, target: ReviewTarget::Limits(1) },
        ReviewRow { group: "限制", label: "搜索上下文行数", value: limits.context_lines.to_string(), target: ReviewTarget::Limits(3) },
        ReviewRow { group: "沙箱", label: "运行后端", value: "bubblewrap".to_string(), target: ReviewTarget::Optional(4) },
        ReviewRow { group: "沙箱", label: "Runtime Path", value: "/usr/bin:/usr/local/bin:/home/slhaf/.local/bin".to_string(), target: ReviewTarget::Optional(4) },
        ReviewRow { group: "模式相关", label: "Tunnel", value: "cloudflared tunnel --url http://127.0.0.1:8787 --no-autoupdate".to_string(), target: ReviewTarget::Optional(6) },
        ReviewRow { group: "模式相关", label: "Hub Report", value: "https://hub.example.internal/api/v1/agents/laptop/report".to_string(), target: ReviewTarget::Optional(7) },
        ReviewRow { group: "Process · 运行中", label: "command", value: "cargo test -p agentic-gpt --all-targets --all-features -- --nocapture".to_string(), target: ReviewTarget::PreviewOnly },
        ReviewRow { group: "Process · 运行中", label: "working dir", value: "/home/slhaf/Projects/AgenticGPT/crates/agentic-gpt".to_string(), target: ReviewTarget::PreviewOnly },
        ReviewRow { group: "Process · 等待中", label: "command", value: "RUST_LOG=agentic_gpt=debug cargo run -p agentic-gpt -- config init --language zh-CN --non-interactive".to_string(), target: ReviewTarget::PreviewOnly },
        ReviewRow { group: "Process · 失败", label: "stderr", value: "warning: local socket did not become ready before bounded runtime verification deadline".to_string(), target: ReviewTarget::PreviewOnly },
        ReviewRow { group: "Process · 已完成", label: "result", value: "exit 0 · 318 tests passed · elapsed 12.84s".to_string(), target: ReviewTarget::PreviewOnly },
    ];

    for index in [5usize, 6, 7, 8, 9, 10, 11, 15, 16, 17, 18] {
        if let Some(value) = review.overrides.get(index).and_then(|value| value.clone()) {
            if let Some(row) = rows.get_mut(index) {
                row.value = value;
            }
        }
    }
    rows
}

const CONFIRM_OPTIONS: &[&str] = &["桌面通知 → ntfy", "仅桌面通知", "仅 ntfy", "无"];
const LANGUAGE_OPTIONS: &[&str] = &["zh-CN", "en-US"];
const SANDBOX_OPTIONS: &[&str] = &["bubblewrap", "none"];

fn begin_review_edit(
    row: usize,
    value: &str,
    app: &AppState,
    limits: &LimitsState,
    review: &mut ReviewState,
) {
    review.editor = match row {
        0 => Some(ReviewEditor::Choice {
            row,
            cursor: Mode::ALL
                .iter()
                .position(|mode| *mode == app.mode)
                .unwrap_or(0),
            options: &["Standalone", "Hub", "Local"],
        }),
        1 => Some(ReviewEditor::Choice {
            row,
            cursor: Profile::ALL
                .iter()
                .position(|profile| *profile == app.profile)
                .unwrap_or(0),
            options: &["Normal", "Room"],
        }),
        10 => Some(ReviewEditor::Choice {
            row,
            cursor: CONFIRM_OPTIONS
                .iter()
                .position(|option| *option == value)
                .unwrap_or(0),
            options: CONFIRM_OPTIONS,
        }),
        11 => Some(ReviewEditor::Choice {
            row,
            cursor: LANGUAGE_OPTIONS
                .iter()
                .position(|option| *option == value)
                .unwrap_or(0),
            options: LANGUAGE_OPTIONS,
        }),
        13 => Some(ReviewEditor::AutoCustom {
            cursor: usize::from(!limits.jobs_auto),
            buffer: limits.custom_jobs.to_string(),
            editing_number: false,
        }),
        15 => Some(ReviewEditor::Choice {
            row,
            cursor: SANDBOX_OPTIONS
                .iter()
                .position(|option| *option == value)
                .unwrap_or(0),
            options: SANDBOX_OPTIONS,
        }),
        2..=9 | 12 | 14 | 16..=18 => Some(ReviewEditor::Text {
            row,
            buffer: if row == 2 && value == "未设置" {
                String::new()
            } else {
                value.to_string()
            },
            numeric: matches!(row, 12 | 14),
        }),
        _ => None,
    };
}

fn commit_review_text(
    row: usize,
    buffer: String,
    identity: &mut IdentityState,
    limits: &mut LimitsState,
    review: &mut ReviewState,
) -> bool {
    let value = buffer.trim().to_string();
    match row {
        2 => identity.display_name = value,
        3 => identity.agent_id = value,
        4 => identity.partner_home = value,
        12 => {
            let Ok(parsed) = value.parse::<u16>() else {
                return false;
            };
            if parsed == 0 {
                return false;
            }
            limits.max_tasks = parsed;
        }
        14 => {
            let Ok(parsed) = value.parse::<u16>() else {
                return false;
            };
            limits.context_lines = parsed;
        }
        5..=9 | 16..=18 => review.set_override(row, value),
        _ => return false,
    }
    true
}

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let result = run(&mut terminal);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = AppState::default();
    let mut variant = VisualVariant::Labeled;
    let mut page = DemoPage::Basic;
    let mut optional_focus = 0usize;
    let mut identity = IdentityState::default();
    let mut limits = LimitsState::default();
    let mut review = ReviewState::default();
    loop {
        terminal.draw(|frame| {
            render(
                frame,
                &app,
                variant,
                page,
                optional_focus,
                &identity,
                &limits,
                &review,
            )
        })?;
        if !event::poll(Duration::from_millis(150))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            break;
        }

        match page {
            DemoPage::Basic => match key.code {
                KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => app.move_up(),
                KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => app.move_down(),
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l')
                    if app.focus == Focus::Next =>
                {
                    page = DemoPage::Optional;
                    optional_focus = 0;
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') => app.confirm(),
                KeyCode::Char('b') | KeyCode::Char('B') => app.toggle_border(),
                KeyCode::Char('v') | KeyCode::Char('V') => variant = variant.next(),
                KeyCode::Esc | KeyCode::Char('q') => break,
                _ => {}
            },
            DemoPage::Optional => match key.code {
                KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
                    optional_focus = move_optional_focus(optional_focus, -1, &app)
                }
                KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
                    optional_focus = move_optional_focus(optional_focus, 1, &app)
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') if optional_focus == 0 => {
                    page = DemoPage::Identity;
                    identity.start_edit(0);
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') if optional_focus == 3 => {
                    page = DemoPage::Limits;
                    limits.focus = 0;
                    limits.cancel_edit();
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') if optional_focus == 8 => {
                    review.focus = 0;
                    page = DemoPage::Review;
                }
                KeyCode::Esc | KeyCode::Char('h') => page = DemoPage::Basic,
                KeyCode::Char('q') => break,
                _ => {}
            },
            DemoPage::Identity if identity.editing.is_some() => match key.code {
                KeyCode::Char(c) if !c.is_control() => {
                    if identity.edit_buffer.chars().count() < 128 {
                        identity.edit_buffer.push(c);
                    }
                }
                KeyCode::Backspace => {
                    identity.edit_buffer.pop();
                }
                KeyCode::Enter => identity.commit_edit(),
                KeyCode::Esc => identity.cancel_edit(),
                _ => {}
            },
            DemoPage::Identity => match key.code {
                KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => identity.move_focus(-1),
                KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => identity.move_focus(1),
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') if identity.focus < 3 => {
                    identity.start_edit(identity.focus);
                }
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') if identity.focus == 3 => {
                    page = DemoPage::Optional;
                    optional_focus = 0;
                }
                KeyCode::Esc | KeyCode::Char('h') => {
                    page = DemoPage::Optional;
                    optional_focus = 0;
                }
                KeyCode::Char('q') => break,
                _ => {}
            },
            DemoPage::Limits if limits.editing.is_some() => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    if limits.edit_buffer.len() < 5 {
                        limits.edit_buffer.push(c);
                        limits.error = None;
                    }
                }
                KeyCode::Backspace => {
                    limits.edit_buffer.pop();
                    limits.error = None;
                }
                KeyCode::Enter => limits.commit_edit(),
                KeyCode::Esc => limits.cancel_edit(),
                _ => {}
            },
            DemoPage::Limits => match key.code {
                KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => limits.move_focus(-1),
                KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => limits.move_focus(1),
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') => match limits.focus {
                    0 => limits.start_edit(EditField::MaxTasks),
                    1 => {
                        limits.jobs_auto = true;
                        limits.error = None;
                    }
                    2 => {
                        limits.jobs_auto = false;
                        limits.start_edit(EditField::CustomJobs);
                    }
                    3 => limits.start_edit(EditField::ContextLines),
                    4 => {
                        page = DemoPage::Optional;
                        optional_focus = 3;
                    }
                    _ => {}
                },
                KeyCode::Esc | KeyCode::Char('h') => {
                    page = DemoPage::Optional;
                    optional_focus = 3;
                }
                KeyCode::Char('q') => break,
                _ => {}
            },
            DemoPage::Review => {
                if review.search_input.is_some() {
                    let rows = review_rows(&app, &identity, &limits, &review);
                    match key.code {
                        KeyCode::Char(c) if !c.is_control() => {
                            if let Some(query) = review.search_input.as_mut() {
                                if query.chars().count() < 80 {
                                    query.push(c);
                                }
                            }
                            review.update_incremental_search(&rows);
                        }
                        KeyCode::Backspace => {
                            if let Some(query) = review.search_input.as_mut() {
                                query.pop();
                            }
                            review.update_incremental_search(&rows);
                        }
                        KeyCode::Enter => review.commit_search(),
                        KeyCode::Esc => review.cancel_search(),
                        _ => {}
                    }
                } else if let Some(editor) = review.editor.take() {
                    match editor {
                        ReviewEditor::Choice {
                            row,
                            mut cursor,
                            options,
                        } => match key.code {
                            KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
                                cursor = (cursor as isize - 1).rem_euclid(options.len() as isize)
                                    as usize;
                                review.editor = Some(ReviewEditor::Choice {
                                    row,
                                    cursor,
                                    options,
                                });
                            }
                            KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
                                cursor = (cursor + 1) % options.len();
                                review.editor = Some(ReviewEditor::Choice {
                                    row,
                                    cursor,
                                    options,
                                });
                            }
                            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') => match row {
                                0 => app.mode = Mode::ALL[cursor],
                                1 => app.profile = Profile::ALL[cursor],
                                10 | 11 | 15 => {
                                    review.set_override(row, options[cursor].to_string())
                                }
                                _ => {}
                            },
                            KeyCode::Esc | KeyCode::Char('h') => {}
                            _ => {
                                review.editor = Some(ReviewEditor::Choice {
                                    row,
                                    cursor,
                                    options,
                                });
                            }
                        },
                        ReviewEditor::Text {
                            row,
                            mut buffer,
                            numeric,
                        } => match key.code {
                            KeyCode::Char(c)
                                if (!numeric || c.is_ascii_digit()) && !c.is_control() =>
                            {
                                if buffer.chars().count() < 160 {
                                    buffer.push(c);
                                }
                                review.editor = Some(ReviewEditor::Text {
                                    row,
                                    buffer,
                                    numeric,
                                });
                            }
                            KeyCode::Backspace => {
                                buffer.pop();
                                review.editor = Some(ReviewEditor::Text {
                                    row,
                                    buffer,
                                    numeric,
                                });
                            }
                            KeyCode::Enter => {
                                if !commit_review_text(
                                    row,
                                    buffer.clone(),
                                    &mut identity,
                                    &mut limits,
                                    &mut review,
                                ) {
                                    review.editor = Some(ReviewEditor::Text {
                                        row,
                                        buffer,
                                        numeric,
                                    });
                                }
                            }
                            KeyCode::Esc => {}
                            _ => {
                                review.editor = Some(ReviewEditor::Text {
                                    row,
                                    buffer,
                                    numeric,
                                });
                            }
                        },
                        ReviewEditor::AutoCustom {
                            mut cursor,
                            mut buffer,
                            mut editing_number,
                        } => match key.code {
                            KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k')
                                if !editing_number =>
                            {
                                cursor = (cursor + 1) % 2;
                                review.editor = Some(ReviewEditor::AutoCustom {
                                    cursor,
                                    buffer,
                                    editing_number,
                                });
                            }
                            KeyCode::Down | KeyCode::Tab | KeyCode::Char('j')
                                if !editing_number =>
                            {
                                cursor = (cursor + 1) % 2;
                                review.editor = Some(ReviewEditor::AutoCustom {
                                    cursor,
                                    buffer,
                                    editing_number,
                                });
                            }
                            KeyCode::Char(c) if editing_number && c.is_ascii_digit() => {
                                if buffer.len() < 5 {
                                    buffer.push(c);
                                }
                                review.editor = Some(ReviewEditor::AutoCustom {
                                    cursor,
                                    buffer,
                                    editing_number,
                                });
                            }
                            KeyCode::Backspace if editing_number => {
                                buffer.pop();
                                review.editor = Some(ReviewEditor::AutoCustom {
                                    cursor,
                                    buffer,
                                    editing_number,
                                });
                            }
                            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l')
                                if cursor == 0 && !editing_number =>
                            {
                                limits.jobs_auto = true;
                            }
                            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l')
                                if !editing_number =>
                            {
                                editing_number = true;
                                review.editor = Some(ReviewEditor::AutoCustom {
                                    cursor,
                                    buffer,
                                    editing_number,
                                });
                            }
                            KeyCode::Enter if editing_number => {
                                if let Ok(value) = buffer.parse::<u16>() {
                                    if value > 0 {
                                        limits.jobs_auto = false;
                                        limits.custom_jobs = value;
                                    } else {
                                        review.editor = Some(ReviewEditor::AutoCustom {
                                            cursor,
                                            buffer,
                                            editing_number,
                                        });
                                    }
                                } else {
                                    review.editor = Some(ReviewEditor::AutoCustom {
                                        cursor,
                                        buffer,
                                        editing_number,
                                    });
                                }
                            }
                            KeyCode::Esc if editing_number => {
                                editing_number = false;
                                review.editor = Some(ReviewEditor::AutoCustom {
                                    cursor,
                                    buffer,
                                    editing_number,
                                });
                            }
                            KeyCode::Esc => {}
                            KeyCode::Char('h') if !editing_number => {}
                            _ => {
                                review.editor = Some(ReviewEditor::AutoCustom {
                                    cursor,
                                    buffer,
                                    editing_number,
                                });
                            }
                        },
                    }
                } else {
                    let rows = review_rows(&app, &identity, &limits, &review);
                    match key.code {
                        KeyCode::Up | KeyCode::BackTab | KeyCode::Char('k') => {
                            review.move_focus(-1, rows.len())
                        }
                        KeyCode::Down | KeyCode::Tab | KeyCode::Char('j') => {
                            review.move_focus(1, rows.len())
                        }
                        KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('l') => {
                            if let Some(row) = rows.get(review.focus) {
                                if !matches!(row.target, ReviewTarget::PreviewOnly) {
                                    begin_review_edit(
                                        review.focus,
                                        &row.value,
                                        &app,
                                        &limits,
                                        &mut review,
                                    );
                                }
                            }
                        }
                        KeyCode::Char('/') => review.begin_search(),
                        KeyCode::Char('n') => review.repeat_search(&rows, 1),
                        KeyCode::Char('N') => review.repeat_search(&rows, -1),
                        KeyCode::Esc | KeyCode::Char('h') => {
                            page = DemoPage::Optional;
                            optional_focus = 8;
                        }
                        KeyCode::Char('q') => break,
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn render(
    frame: &mut Frame<'_>,
    app: &AppState,
    variant: VisualVariant,
    page: DemoPage,
    optional_focus: usize,
    identity: &IdentityState,
    limits: &LimitsState,
    review: &ReviewState,
) {
    let full = frame.area();
    let content = if app.border && full.width > 6 && full.height > 6 {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(structure_color()));
        let inner = block.inner(full);
        frame.render_widget(block, full);
        inner
    } else {
        full
    };

    let content = content.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(content);

    render_header(frame, rows[0], page);
    render_horizontal_separator(frame, rows[1]);
    match page {
        DemoPage::Basic => render_body(frame, rows[3], app, variant),
        DemoPage::Optional => render_optional_body(
            frame,
            rows[3],
            app,
            optional_focus,
            identity,
            limits,
            variant,
        ),
        DemoPage::Identity => render_identity_body(frame, rows[3], identity, variant),
        DemoPage::Limits => render_limits_body(frame, rows[3], limits, variant),
        DemoPage::Review => {
            render_review_body(frame, rows[3], app, identity, limits, review, variant)
        }
    }
    render_horizontal_separator(frame, rows[5]);
    render_footer(frame, rows[6], app, variant, page, identity, limits, review);
}

fn render_header(frame: &mut Frame<'_>, area: Rect, page: DemoPage) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(8)])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("AgenticGPT", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(" 配置初始化"),
        ])),
        cols[0],
    );
    let progress = match page {
        DemoPage::Basic => "1 / 4",
        DemoPage::Optional | DemoPage::Identity | DemoPage::Limits => "2 / 4",
        DemoPage::Review => "4 / 4",
    };
    frame.render_widget(
        Paragraph::new(progress)
            .alignment(Alignment::Right)
            .style(Style::default().fg(muted_color())),
        cols[1],
    );
}

fn render_horizontal_separator(frame: &mut Frame<'_>, area: Rect) {
    if area.width == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize))
            .style(Style::default().fg(structure_color())),
        area,
    );
}

fn render_review_body(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    identity: &IdentityState,
    limits: &LimitsState,
    review: &ReviewState,
    variant: VisualVariant,
) {
    let rows = review_rows(app, identity, limits, review);
    if area.width < 72 {
        render_review_list(frame, area, &rows, review);
        return;
    }

    let surface = matches!(variant, VisualVariant::Surface);
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(61),
            Constraint::Length(if surface { 2 } else { 1 }),
            Constraint::Min(28),
        ])
        .split(area);

    render_review_list(frame, split[0], &rows, review);
    if surface {
        frame.render_widget(
            Block::default().style(Style::default().bg(surface_color())),
            split[2],
        );
    } else {
        let divider = "│\n".repeat(split[1].height as usize);
        frame.render_widget(
            Paragraph::new(divider).style(Style::default().fg(structure_color())),
            split[1],
        );
    }
    render_review_inspector(
        frame,
        split[2].inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
        rows.get(review.focus),
    );
}

fn render_review_list(frame: &mut Frame<'_>, area: Rect, rows: &[ReviewRow], review: &ReviewState) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut previous_group = "";
    let mut focus_visual_line = 0usize;
    let editor_row = match &review.editor {
        Some(ReviewEditor::Choice { row, .. }) | Some(ReviewEditor::Text { row, .. }) => Some(*row),
        Some(ReviewEditor::AutoCustom { .. }) => Some(13),
        None => None,
    };

    for (index, row) in rows.iter().enumerate() {
        if row.group != previous_group {
            if !previous_group.is_empty() {
                lines.push(Line::raw(""));
            }
            previous_group = row.group;
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("◆ ", Style::default().fg(emphasis_color())),
                Span::styled(
                    row.group.to_string(),
                    Style::default()
                        .fg(emphasis_color())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }

        if index == review.focus {
            focus_visual_line = lines.len();
        }

        if editor_row == Some(index) {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    row.label.to_string(),
                    Style::default()
                        .fg(emphasis_color())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            match &review.editor {
                Some(ReviewEditor::Choice {
                    cursor, options, ..
                }) => {
                    for (option_index, option) in options.iter().enumerate() {
                        let selected = row.value == *option;
                        lines.push(Line::from(vec![
                            Span::raw("    "),
                            focus_span(*cursor == option_index),
                            Span::raw((*option).to_string()),
                            Span::raw("  "),
                            if selected {
                                Span::styled(
                                    "",
                                    Style::default()
                                        .fg(selected_color())
                                        .add_modifier(Modifier::BOLD),
                                )
                            } else {
                                Span::raw(" ")
                            },
                        ]));
                    }
                }
                Some(ReviewEditor::Text { buffer, .. }) => {
                    let max_inner = area.width.saturating_sub(12).min(36) as usize;
                    let min_inner = max_inner.min(8);
                    let inner_width = UnicodeWidthStr::width(buffer.as_str())
                        .clamp(min_inner, max_inner.max(min_inner));
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        focus_span(true),
                        dynamic_input_value_line(buffer, true, inner_width, false),
                    ]));
                }
                Some(ReviewEditor::AutoCustom {
                    cursor,
                    buffer,
                    editing_number,
                }) => {
                    let committed_auto = row.value == "auto";
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        focus_span(*cursor == 0 && !editing_number),
                        Span::raw("自动"),
                        Span::raw("  "),
                        if committed_auto {
                            Span::styled("", Style::default().fg(selected_color()))
                        } else {
                            Span::raw(" ")
                        },
                    ]));
                    let input_width = UnicodeWidthStr::width(buffer.as_str()).max(4).min(8);
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        focus_span(*cursor == 1),
                        Span::raw("自定义  "),
                        dynamic_input_value_line(buffer, *editing_number, input_width, false),
                        Span::raw("  "),
                        if !committed_auto {
                            Span::styled("", Style::default().fg(selected_color()))
                        } else {
                            Span::raw(" ")
                        },
                    ]));
                }
                None => {}
            }
            continue;
        }

        let label_width = 18usize;
        let fixed_width = 2usize + 2 + label_width + 2;
        let value_width = (area.width as usize).saturating_sub(fixed_width + 2).max(4);
        let label = pad_or_truncate(row.label, label_width);
        let value = leading_text(&row.value, value_width);
        let active_search = review
            .search_input
            .as_deref()
            .or(review.last_search.as_deref())
            .filter(|query| !query.is_empty());
        let search_match = active_search
            .map(|query| review_row_matches(row, query))
            .unwrap_or(false);
        let label_style = if search_match {
            Style::default()
                .fg(selected_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(emphasis_color())
        };
        let value_style = if search_match {
            Style::default()
                .fg(selected_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(muted_color())
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            focus_span(index == review.focus),
            Span::styled(label, label_style),
            Span::raw("  "),
            Span::styled(format!("[{value}]"), value_style),
        ]));
    }

    let visible = area.height as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = focus_visual_line
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(max_scroll);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((scroll as u16, 0)),
        area,
    );
}

fn render_review_inspector(frame: &mut Frame<'_>, area: Rect, row: Option<&ReviewRow>) {
    let Some(row) = row else {
        return;
    };
    let editable = !matches!(row.target, ReviewTarget::PreviewOnly);
    let inspector_rule_width = area.width.saturating_sub(2).min(18) as usize;
    let mut lines = vec![
        Line::styled(
            row.label.to_string(),
            Style::default()
                .fg(emphasis_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "─".repeat(inspector_rule_width),
            Style::default().fg(structure_color()),
        ),
        Line::raw(""),
        Line::styled(
            format!("区域：{}", row.group),
            Style::default().fg(muted_color()),
        ),
        Line::raw(""),
        Line::raw(row.value.clone()),
        Line::raw(""),
    ];
    if editable {
        lines.push(Line::styled(
            "Enter 修改此项",
            Style::default().fg(muted_color()),
        ));
    } else {
        lines.push(Line::styled(
            "预览项 · 不可修改",
            Style::default().fg(muted_color()),
        ));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn pad_or_truncate(value: &str, width: usize) -> String {
    let value = leading_text(value, width);
    let used = UnicodeWidthStr::width(value.as_str());
    format!("{value}{}", " ".repeat(width.saturating_sub(used)))
}

fn render_body(frame: &mut Frame<'_>, area: Rect, app: &AppState, variant: VisualVariant) {
    if area.width < 58 {
        render_controls(frame, area, app);
        return;
    }

    match variant {
        VisualVariant::Minimal => render_minimal_body(frame, area, app),
        VisualVariant::FocusFrame => render_focus_frame_body(frame, area, app),
        VisualVariant::Surface => render_surface_body(frame, area, app),
        VisualVariant::Labeled => render_labeled_body(frame, area, app),
    }
}

fn render_minimal_body(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(43),
            Constraint::Length(1),
            Constraint::Min(24),
        ])
        .split(area);

    render_controls(frame, split[0], app);
    let divider = "│\n".repeat(split[1].height as usize);
    frame.render_widget(
        Paragraph::new(divider).style(Style::default().fg(structure_color())),
        split[1],
    );
    render_inspector(
        frame,
        split[2].inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
        app,
    );
}

fn render_focus_frame_body(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(43),
            Constraint::Length(2),
            Constraint::Min(24),
        ])
        .split(area);

    let left_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::THICK)
        .border_style(Style::default().fg(focus_color()))
        .title(Span::styled(
            " 配置 ",
            Style::default()
                .fg(emphasis_color())
                .add_modifier(Modifier::BOLD),
        ));
    let left_inner = left_block.inner(split[0]).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    frame.render_widget(left_block, split[0]);
    render_controls(frame, left_inner, app);

    let right_block = Block::default()
        .borders(Borders::ALL)
        .border_set(border::PLAIN)
        .border_style(Style::default().fg(structure_color()))
        .title(Span::styled(" 说明 ", Style::default().fg(muted_color())));
    let right_inner = right_block.inner(split[2]).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    frame.render_widget(right_block, split[2]);
    render_inspector(frame, right_inner, app);
}

fn render_surface_body(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(43),
            Constraint::Length(2),
            Constraint::Min(24),
        ])
        .split(area);

    render_controls_labeled(frame, split[0], app);
    frame.render_widget(
        Block::default().style(Style::default().bg(surface_color())),
        split[2],
    );
    render_inspector(
        frame,
        split[2].inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
        app,
    );
}

fn render_labeled_body(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(43),
            Constraint::Length(1),
            Constraint::Min(24),
        ])
        .split(area);

    render_controls_labeled(frame, split[0], app);
    let divider = "│\n".repeat(split[1].height as usize);
    frame.render_widget(
        Paragraph::new(divider).style(Style::default().fg(structure_color())),
        split[1],
    );
    render_inspector(
        frame,
        split[2].inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
        app,
    );
}

fn render_identity_body(
    frame: &mut Frame<'_>,
    area: Rect,
    identity: &IdentityState,
    variant: VisualVariant,
) {
    if area.width < 58 {
        render_identity_controls(frame, area, identity);
        return;
    }

    let surface = matches!(variant, VisualVariant::Surface);
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(43),
            Constraint::Length(if surface { 2 } else { 1 }),
            Constraint::Min(24),
        ])
        .split(area);

    render_identity_controls(frame, split[0], identity);
    if surface {
        frame.render_widget(
            Block::default().style(Style::default().bg(surface_color())),
            split[2],
        );
    } else {
        let divider = "│\n".repeat(split[1].height as usize);
        frame.render_widget(
            Paragraph::new(divider).style(Style::default().fg(structure_color())),
            split[1],
        );
    }
    render_identity_inspector(
        frame,
        split[2].inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
        identity,
    );
}

fn render_identity_controls(frame: &mut Frame<'_>, area: Rect, identity: &IdentityState) {
    render_optional_heading(frame, area, area.y, "身份");

    for (field, label, y_offset) in [
        (0usize, "显示名称", 2u16),
        (1usize, "Agent ID", 5u16),
        (2usize, "Partner Home", 8u16),
    ] {
        let editing = identity.editing == Some(field);
        let value = if editing {
            identity.edit_buffer.as_str()
        } else {
            identity.value(field)
        };
        render_long_form_field(
            frame,
            area,
            area.y.saturating_add(y_offset),
            label,
            value,
            identity.focus == field,
            editing,
            identity.is_default_field(field),
        );
    }

    let action_y = area.y.saturating_add(area.height.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(action_line(identity.focus == 3, "保存并返回")),
        Rect {
            x: area.x,
            y: action_y,
            width: area.width,
            height: 1,
        },
    );
}

fn render_long_form_field(
    frame: &mut Frame<'_>,
    area: Rect,
    label_y: u16,
    label: &'static str,
    value: &str,
    focused: bool,
    editing: bool,
    is_default: bool,
) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("◆ ", Style::default().fg(emphasis_color())),
            Span::styled(label, Style::default().fg(emphasis_color())),
        ])),
        Rect {
            x: area.x,
            y: label_y,
            width: area.width,
            height: 1,
        },
    );

    let max_inner = area.width.saturating_sub(10).min(36) as usize;
    let min_inner = max_inner.min(8);
    let content_width = UnicodeWidthStr::width(value);
    let inner_width = content_width.clamp(min_inner, max_inner.max(min_inner));
    let input_width = (inner_width + 6) as u16;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            focus_span(focused),
            dynamic_input_value_line(value, editing, inner_width, is_default),
        ])),
        Rect {
            x: area.x,
            y: label_y.saturating_add(1),
            width: input_width.min(area.width),
            height: 1,
        },
    );
}

fn render_identity_inspector(frame: &mut Frame<'_>, area: Rect, identity: &IdentityState) {
    let (title, body): (&str, Vec<&str>) = match identity.focus {
        0 => (
            "显示名称",
            vec!["Agent 对外展示的名称。", "默认留空；不会自动写入占位名称。"],
        ),
        1 => (
            "Agent ID",
            vec![
                "用于观察多个长文本输入的间距与默认值样式。",
                "修改后可直接看颜色变化。",
            ],
        ),
        2 => (
            "Partner Home",
            vec!["用于观察较长输入的动态宽度。", "修改后可直接看颜色变化。"],
        ),
        _ => (
            "保存并返回",
            vec![
                "保留当前输入并返回可选配置中心。",
                "Demo 不会写入真实配置文件。",
            ],
        ),
    };

    let inspector_rule_width = area.width.saturating_sub(2).min(18) as usize;
    let mut lines = vec![
        Line::styled(
            title,
            Style::default()
                .fg(emphasis_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "─".repeat(inspector_rule_width),
            Style::default().fg(structure_color()),
        ),
        Line::raw(""),
    ];
    if identity.editing.is_some() {
        lines.push(Line::styled(
            "正在编辑",
            Style::default()
                .fg(focus_color())
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::styled(
            "Enter 确认 · Esc 放弃",
            Style::default().fg(muted_color()),
        ));
        lines.push(Line::raw(""));
    }
    lines.extend(body.into_iter().map(Line::raw));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_limits_body(
    frame: &mut Frame<'_>,
    area: Rect,
    limits: &LimitsState,
    variant: VisualVariant,
) {
    if area.width < 58 {
        render_limits_controls(frame, area, limits);
        return;
    }

    let surface = matches!(variant, VisualVariant::Surface);
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(43),
            Constraint::Length(if surface { 2 } else { 1 }),
            Constraint::Min(24),
        ])
        .split(area);

    render_limits_controls(frame, split[0], limits);
    if surface {
        frame.render_widget(
            Block::default().style(Style::default().bg(surface_color())),
            split[2],
        );
    } else {
        let divider = "│\n".repeat(split[1].height as usize);
        frame.render_widget(
            Paragraph::new(divider).style(Style::default().fg(structure_color())),
            split[1],
        );
    }
    render_limits_inspector(
        frame,
        split[2].inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
        limits,
    );
}

fn render_limits_controls(frame: &mut Frame<'_>, area: Rect, limits: &LimitsState) {
    let mut y = area.y;
    render_optional_heading(frame, area, y, "任务并发");
    y = y.saturating_add(2);

    let max_tasks = if limits.editing == Some(EditField::MaxTasks) {
        limits.edit_buffer.as_str()
    } else {
        ""
    };
    let max_tasks_value = if limits.editing == Some(EditField::MaxTasks) {
        max_tasks.to_string()
    } else {
        limits.max_tasks.to_string()
    };
    render_limit_input_row(
        frame,
        area,
        y,
        limits.focus == 0,
        "最大并发任务",
        &max_tasks_value,
        limits.editing == Some(EditField::MaxTasks),
    );

    y = y.saturating_add(2);
    frame.render_widget(
        Paragraph::new("  最大活动作业").style(Style::default().fg(muted_color())),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        },
    );
    y = y.saturating_add(1);
    render_limit_select_row(
        frame,
        area,
        y,
        limits.focus == 1,
        limits.jobs_auto,
        "自动",
        None,
        false,
    );
    y = y.saturating_add(1);
    let custom_value = if limits.editing == Some(EditField::CustomJobs) {
        limits.edit_buffer.clone()
    } else {
        limits.custom_jobs.to_string()
    };
    render_limit_select_row(
        frame,
        area,
        y,
        limits.focus == 2,
        !limits.jobs_auto,
        "自定义",
        Some(custom_value.as_str()),
        limits.editing == Some(EditField::CustomJobs),
    );

    y = y.saturating_add(2);
    render_optional_heading(frame, area, y, "搜索");
    y = y.saturating_add(2);
    let context_value = if limits.editing == Some(EditField::ContextLines) {
        limits.edit_buffer.clone()
    } else {
        limits.context_lines.to_string()
    };
    render_limit_input_row(
        frame,
        area,
        y,
        limits.focus == 3,
        "上下文行数",
        &context_value,
        limits.editing == Some(EditField::ContextLines),
    );

    let action_y = area.y.saturating_add(area.height.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(action_line(limits.focus == 4, "保存并返回")),
        Rect {
            x: area.x,
            y: action_y,
            width: area.width,
            height: 1,
        },
    );
}

fn render_limit_input_row(
    frame: &mut Frame<'_>,
    area: Rect,
    y: u16,
    focused: bool,
    label: &'static str,
    value: &str,
    editing: bool,
) {
    let row = Rect {
        x: area.x,
        y,
        width: area.width.saturating_sub(7),
        height: 1,
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(18), Constraint::Length(10)])
        .split(row);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            focus_span(focused),
            Span::raw(label),
        ])),
        cols[0],
    );
    frame.render_widget(Paragraph::new(input_value_line(value, editing)), cols[1]);
}

fn render_limit_select_row(
    frame: &mut Frame<'_>,
    area: Rect,
    y: u16,
    focused: bool,
    selected: bool,
    label: &'static str,
    value: Option<&str>,
    editing: bool,
) {
    let row = Rect {
        x: area.x,
        y,
        width: area.width.saturating_sub(7),
        height: 1,
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(14),
            Constraint::Length(3),
            Constraint::Length(10),
        ])
        .split(row);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            focus_span(focused),
            Span::raw(label),
        ])),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(if selected {
            Line::from(Span::styled(
                "",
                Style::default()
                    .fg(selected_color())
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::raw("")
        }),
        cols[1],
    );
    if let Some(value) = value {
        frame.render_widget(Paragraph::new(input_value_line(value, editing)), cols[2]);
    }
}

fn focus_span(focused: bool) -> Span<'static> {
    if focused {
        Span::styled(
            "❯ ",
            Style::default()
                .fg(focus_color())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    }
}

fn dynamic_input_value_line(
    value: &str,
    editing: bool,
    inner_width: usize,
    is_default: bool,
) -> Span<'static> {
    let content = if editing {
        let visible = trailing_text(value, inner_width);
        let used = UnicodeWidthStr::width(visible.as_str());
        format!(
            "[{visible}{}]",
            " ".repeat(inner_width.saturating_sub(used))
        )
    } else if value.is_empty() {
        format!("[{}]", " ".repeat(inner_width))
    } else {
        let visible = leading_text(value, inner_width);
        let used = UnicodeWidthStr::width(visible.as_str());
        format!(
            "[{visible}{}]",
            " ".repeat(inner_width.saturating_sub(used))
        )
    };

    let style = if is_default && !editing {
        Style::default()
            .fg(muted_color())
            .add_modifier(Modifier::DIM)
    } else if editing {
        Style::default()
            .fg(emphasis_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(emphasis_color())
    };

    Span::styled(content, style)
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
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > target {
            break;
        }
        out.push(ch);
        used += ch_width;
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
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > target {
            break;
        }
        chars.push(ch);
        used += ch_width;
    }
    chars.reverse();
    let mut out = String::from("…");
    out.extend(chars);
    out
}

fn input_value_line(value: &str, editing: bool) -> Line<'static> {
    if editing {
        Line::from(vec![
            Span::styled("[", Style::default().fg(focus_color())),
            Span::styled(
                format!("{value}▏"),
                Style::default()
                    .fg(emphasis_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("]", Style::default().fg(focus_color())),
        ])
    } else {
        Line::from(vec![
            Span::styled("[", Style::default().fg(muted_color())),
            Span::raw(value.to_string()),
            Span::styled("]", Style::default().fg(muted_color())),
        ])
    }
}

fn render_limits_inspector(frame: &mut Frame<'_>, area: Rect, limits: &LimitsState) {
    let (title, body): (&str, Vec<&str>) = match limits.focus {
        0 => (
            "最大并发任务",
            vec!["同时执行的顶层任务数量。", "Enter 进入数字编辑。"],
        ),
        1 => (
            "自动",
            vec![
                "由运行时根据系统资源决定活动作业上限。",
                "Enter 选择自动模式。",
            ],
        ),
        2 => (
            "自定义活动作业",
            vec!["显式指定最大活动作业数量。", "Enter 选择并直接编辑数值。"],
        ),
        3 => (
            "文件搜索上下文",
            vec!["控制文件搜索结果附带的上下文行数。", "允许设置为 0。"],
        ),
        4 => (
            "保存并返回",
            vec![
                "保留本页修改并返回可选配置中心。",
                "Demo 不会写入真实配置文件。",
            ],
        ),
        _ => ("", vec![]),
    };

    let inspector_rule_width = area.width.saturating_sub(2).min(18) as usize;
    let mut lines = vec![
        Line::styled(
            title,
            Style::default()
                .fg(emphasis_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "─".repeat(inspector_rule_width),
            Style::default().fg(structure_color()),
        ),
        Line::raw(""),
    ];
    if limits.editing.is_some() {
        lines.push(Line::styled(
            "正在编辑",
            Style::default()
                .fg(focus_color())
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::styled(
            "Enter 确认 · Esc 放弃",
            Style::default().fg(muted_color()),
        ));
        lines.push(Line::raw(""));
    }
    lines.extend(body.into_iter().map(Line::raw));
    if let Some(error) = limits.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(error, Style::default().fg(error_color())));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_optional_body(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    focus: usize,
    identity: &IdentityState,
    limits: &LimitsState,
    variant: VisualVariant,
) {
    if area.width < 58 {
        render_optional_list(frame, area, app, focus, identity, limits);
        return;
    }

    let surface = matches!(variant, VisualVariant::Surface);
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(43),
            Constraint::Length(if surface { 2 } else { 1 }),
            Constraint::Min(24),
        ])
        .split(area);

    render_optional_list(frame, split[0], app, focus, identity, limits);
    if surface {
        frame.render_widget(
            Block::default().style(Style::default().bg(surface_color())),
            split[2],
        );
    } else {
        let divider = "│\n".repeat(split[1].height as usize);
        frame.render_widget(
            Paragraph::new(divider).style(Style::default().fg(structure_color())),
            split[1],
        );
    }
    render_optional_inspector(
        frame,
        split[2].inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
        app,
        focus,
        identity,
        limits,
    );
}

fn render_optional_list(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    focus: usize,
    identity: &IdentityState,
    limits: &LimitsState,
) {
    let mut y = area.y;
    render_optional_heading(frame, area, y, "通用配置");
    y = y.saturating_add(2);
    for index in 0..5 {
        render_optional_row(frame, area, y, index, app, focus, identity, limits);
        y = y.saturating_add(1);
    }

    y = y.saturating_add(1);
    render_optional_heading(frame, area, y, "模式相关");
    y = y.saturating_add(2);
    for index in 5..8 {
        render_optional_row(frame, area, y, index, app, focus, identity, limits);
        y = y.saturating_add(1);
    }

    let next_y = area.y.saturating_add(area.height.saturating_sub(2));
    if next_y < area.y.saturating_add(area.height) {
        frame.render_widget(
            Paragraph::new(action_line(focus == 8, "完成并继续")),
            Rect {
                x: area.x,
                y: next_y,
                width: area.width,
                height: 1,
            },
        );
    }
}

fn render_optional_heading(frame: &mut Frame<'_>, area: Rect, y: u16, label: &'static str) {
    if y >= area.y.saturating_add(area.height) {
        return;
    }
    frame.render_widget(
        Paragraph::new(labeled_heading(label, area.width)),
        Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        },
    );
}

fn render_optional_row(
    frame: &mut Frame<'_>,
    area: Rect,
    y: u16,
    index: usize,
    app: &AppState,
    focus: usize,
    identity: &IdentityState,
    limits: &LimitsState,
) {
    if y >= area.y.saturating_add(area.height) {
        return;
    }

    let available = optional_available(index, app);
    let row = Rect {
        x: area.x,
        y,
        width: area.width.saturating_sub(10),
        height: 1,
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(2),
        ])
        .split(row);

    let pointer = if focus == index && available {
        Span::styled(
            "❯ ",
            Style::default()
                .fg(focus_color())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let label_style = if available {
        Style::default()
    } else {
        Style::default()
            .fg(muted_color())
            .add_modifier(Modifier::DIM)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            pointer,
            Span::styled(OPTIONAL_LABELS[index], label_style),
        ])),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{:<8}",
            optional_status(index, app, identity, limits)
        ))
        .style(if available {
            Style::default().fg(muted_color())
        } else {
            Style::default()
                .fg(muted_color())
                .add_modifier(Modifier::DIM)
        }),
        cols[1],
    );
}

fn render_optional_inspector(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    focus: usize,
    identity: &IdentityState,
    limits: &LimitsState,
) {
    let (title, body): (&str, Vec<&str>) = match focus {
        0 => (
            "身份",
            vec![
                "设置 Agent 对外展示的名称。",
                "默认留空，不替用户猜测身份。",
            ],
        ),
        1 => (
            "工作区",
            vec![
                "根目录以及写入、只读、拒绝路径。",
                "路径集合使用逐项列表编辑。",
            ],
        ),
        2 => (
            "确认",
            vec!["配置确认通道与回退顺序。", "推荐：桌面通知 → ntfy。"],
        ),
        3 => (
            "限制",
            vec![
                "并发任务、活动作业与搜索上下文。",
                "联合类型使用结构化选择，不暴露字符串编码。",
            ],
        ),
        4 => (
            "沙箱",
            vec![
                "配置沙箱以及运行时所需路径。",
                "Required Runtime Paths 也按列表管理。",
            ],
        ),
        5 => (
            "Room",
            vec![
                "Room 专属的时区、Diary 与 Notebook。",
                "仅能力配置为 Room 时可用。",
            ],
        ),
        6 => (
            "隧道客户端",
            vec![
                "Standalone 专属的远程访问配置。",
                "Hub / Local 模式不会写入这些字段。",
            ],
        ),
        7 => (
            "Hub 报告",
            vec![
                "Standalone 向 Hub 报告状态所需配置。",
                "其他运行模式不参与落盘。",
            ],
        ),
        8 => (
            "完成并继续",
            vec![
                "保留当前可选配置并进入确认页。",
                "这里只验证 Optional Center 的布局与导航。",
            ],
        ),
        _ => ("", vec![]),
    };

    let inspector_rule_width = area.width.saturating_sub(2).min(18) as usize;
    let mut lines = vec![
        Line::styled(
            title,
            Style::default()
                .fg(emphasis_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "─".repeat(inspector_rule_width),
            Style::default().fg(structure_color()),
        ),
        Line::raw(""),
    ];
    if focus < OPTIONAL_LABELS.len() {
        lines.push(Line::styled(
            format!("状态：{}", optional_status(focus, app, identity, limits)),
            Style::default().fg(muted_color()),
        ));
        lines.push(Line::raw(""));
    }
    lines.extend(body.into_iter().map(Line::raw));
    if focus == 8 && app.advanced {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " Demo 已触发继续",
            Style::default().fg(emphasis_color()),
        ));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_controls(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let heading_style = Style::default()
        .fg(emphasis_color())
        .add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::styled("运行模式", heading_style), Line::raw("")];
    for (index, mode) in Mode::ALL.iter().copied().enumerate() {
        lines.push(choice_line(
            app.focus == Focus::Mode(index),
            app.mode == mode,
            mode.label(),
            area.width,
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("能力配置", heading_style));
    lines.push(Line::raw(""));
    for (index, profile) in Profile::ALL.iter().copied().enumerate() {
        lines.push(choice_line(
            app.focus == Focus::Profile(index),
            app.profile == profile,
            profile.label(),
            area.width,
        ));
    }

    let base_height = lines.len() as u16;
    let target_next_y = area.height.saturating_sub(2);
    let gap = target_next_y.saturating_sub(base_height);
    for _ in 0..gap {
        lines.push(Line::raw(""));
    }
    lines.push(action_line(app.focus == Focus::Next, "下一步"));

    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn render_controls_labeled(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let mut lines = vec![labeled_heading("运行模式", area.width), Line::raw("")];
    for (index, mode) in Mode::ALL.iter().copied().enumerate() {
        lines.push(choice_line(
            app.focus == Focus::Mode(index),
            app.mode == mode,
            mode.label(),
            area.width,
        ));
    }
    lines.push(Line::raw(""));
    lines.push(labeled_heading("能力配置", area.width));
    lines.push(Line::raw(""));
    for (index, profile) in Profile::ALL.iter().copied().enumerate() {
        lines.push(choice_line(
            app.focus == Focus::Profile(index),
            app.profile == profile,
            profile.label(),
            area.width,
        ));
    }

    let base_height = lines.len() as u16;
    let target_next_y = area.height.saturating_sub(2);
    let gap = target_next_y.saturating_sub(base_height);
    for _ in 0..gap {
        lines.push(Line::raw(""));
    }
    lines.push(action_line(app.focus == Focus::Next, "下一步"));
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn labeled_heading(label: &'static str, width: u16) -> Line<'static> {
    let prefix = format!("── {label} ");
    let fill_width = width.saturating_sub(prefix.chars().count() as u16 + 7) as usize;
    Line::from(vec![
        Span::styled(
            prefix,
            Style::default()
                .fg(emphasis_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "─".repeat(fill_width),
            Style::default().fg(structure_color()),
        ),
    ])
}

fn choice_line(focused: bool, selected: bool, label: &'static str, _width: u16) -> Line<'static> {
    let focus = if focused {
        Span::styled(
            "❯ ",
            Style::default()
                .fg(focus_color())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    let label_width = 12usize;
    let padding = " ".repeat(label_width.saturating_sub(label.chars().count()));
    let selected = if selected {
        Span::styled(
            "",
            Style::default()
                .fg(selected_color())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(" ")
    };
    Line::from(vec![
        Span::raw("  "),
        focus,
        Span::raw(label),
        Span::raw(padding),
        selected,
    ])
}

fn action_line(focused: bool, label: &'static str) -> Line<'static> {
    let focus = if focused {
        Span::styled(
            "❯ ",
            Style::default()
                .fg(focus_color())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };
    Line::from(vec![Span::raw("  "), focus, Span::raw(label)])
}

fn render_inspector(frame: &mut Frame<'_>, area: Rect, app: &AppState) {
    let (title, body): (&str, Vec<&str>) = match app.focus {
        Focus::Mode(0) => (
            "Standalone",
            vec![
                "Agent 独立运行，不依赖外部 Hub。",
                "通过 Tunnel 提供远程访问。",
                "",
                "适合：",
                "• 单机常驻 Agent",
                "• 需要远程接入",
            ],
        ),
        Focus::Mode(1) => (
            "Hub",
            vec![
                "连接远程 Agentic Hub，",
                "由 Hub 管理连接与调度。",
                "",
                "适合：",
                "• 集中管理多个 Agent",
                "• 需要远程调度",
            ],
        ),
        Focus::Mode(2) => (
            "Local",
            vec![
                "仅在本机提供 MCP 能力。",
                "不连接 Hub，也不启用 Tunnel。",
                "",
                "适合：",
                "• 本地开发与个人使用",
                "• 最少连接面",
            ],
        ),
        Focus::Profile(0) => (
            "Normal",
            vec!["启用通用 Agent 能力集。", "保持配置和运行面最精简。"],
        ),
        Focus::Profile(1) => (
            "Room",
            vec![
                "在 Normal 基础上启用 Room 能力。",
                "包括 Diary、Notebook 等长期上下文功能。",
            ],
        ),
        Focus::Next => (
            "下一步",
            vec!["确认本页选择并进入连接配置。", "所有值仍只保存在内存中。"],
        ),
        _ => ("", vec![]),
    };

    let inspector_rule_width = area.width.saturating_sub(2).min(18) as usize;
    let mut lines = vec![
        Line::styled(
            title,
            Style::default()
                .fg(emphasis_color())
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            "─".repeat(inspector_rule_width),
            Style::default().fg(structure_color()),
        ),
        Line::raw(""),
    ];
    lines.extend(body.into_iter().map(Line::raw));
    if app.advanced {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            " Demo 已触发下一步",
            Style::default().fg(selected_color()),
        ));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &AppState,
    variant: VisualVariant,
    page: DemoPage,
    identity: &IdentityState,
    limits: &LimitsState,
    review: &ReviewState,
) {
    let key_style = Style::default().fg(emphasis_color());
    let line = match page {
        DemoPage::Basic => {
            let border_state = if app.border { "开" } else { "关" };
            Line::from(vec![
                Span::styled("↑/↓", key_style),
                Span::styled(" 移动    ", Style::default().fg(muted_color())),
                Span::styled("Enter", key_style),
                Span::styled(" 选择    ", Style::default().fg(muted_color())),
                Span::styled("B", key_style),
                Span::styled(
                    format!(" 边框:{border_state}    "),
                    Style::default().fg(muted_color()),
                ),
                Span::styled("Ctrl+C", key_style),
                Span::styled(" 取消    ", Style::default().fg(muted_color())),
                Span::styled("V ", Style::default().fg(emphasis_color())),
                Span::styled(variant.label(), Style::default().fg(muted_color())),
            ])
        }
        DemoPage::Optional => Line::from(vec![
            Span::styled("↑/↓", key_style),
            Span::styled(" 移动    ", Style::default().fg(muted_color())),
            Span::styled("Enter", key_style),
            Span::styled(" 打开    ", Style::default().fg(muted_color())),
            Span::styled("Esc", key_style),
            Span::styled(" 返回    ", Style::default().fg(muted_color())),
            Span::styled("Ctrl+C", key_style),
            Span::styled(" 取消", Style::default().fg(muted_color())),
        ]),
        DemoPage::Identity if identity.editing.is_some() => Line::from(vec![
            Span::styled("文字", key_style),
            Span::styled(" 输入    ", Style::default().fg(muted_color())),
            Span::styled("Enter", key_style),
            Span::styled(" 确认    ", Style::default().fg(muted_color())),
            Span::styled("Esc", key_style),
            Span::styled(" 放弃", Style::default().fg(muted_color())),
        ]),
        DemoPage::Identity => Line::from(vec![
            Span::styled("↑/↓", key_style),
            Span::styled(" 移动    ", Style::default().fg(muted_color())),
            Span::styled("Enter", key_style),
            Span::styled(" 编辑/保存    ", Style::default().fg(muted_color())),
            Span::styled("Esc", key_style),
            Span::styled(" 返回", Style::default().fg(muted_color())),
        ]),
        DemoPage::Limits if limits.editing.is_some() => Line::from(vec![
            Span::styled("0-9", key_style),
            Span::styled(" 输入    ", Style::default().fg(muted_color())),
            Span::styled("Enter", key_style),
            Span::styled(" 确认    ", Style::default().fg(muted_color())),
            Span::styled("Esc", key_style),
            Span::styled(" 放弃    ", Style::default().fg(muted_color())),
            Span::styled("Ctrl+C", key_style),
            Span::styled(" 取消", Style::default().fg(muted_color())),
        ]),
        DemoPage::Limits => Line::from(vec![
            Span::styled("↑/↓", key_style),
            Span::styled(" 移动    ", Style::default().fg(muted_color())),
            Span::styled("Enter", key_style),
            Span::styled(" 编辑/选择    ", Style::default().fg(muted_color())),
            Span::styled("Esc", key_style),
            Span::styled(" 返回    ", Style::default().fg(muted_color())),
            Span::styled("Ctrl+C", key_style),
            Span::styled(" 取消", Style::default().fg(muted_color())),
        ]),
        DemoPage::Review if review.search_input.is_some() => {
            let query = review.search_input.as_deref().unwrap_or_default();
            Line::from(vec![
                Span::styled("/", key_style),
                Span::raw(query.to_string()),
                Span::styled("    Enter", key_style),
                Span::styled(" 确认    ", Style::default().fg(muted_color())),
                Span::styled("Esc", key_style),
                Span::styled(" 取消搜索", Style::default().fg(muted_color())),
            ])
        }
        DemoPage::Review => Line::from(vec![
            Span::styled("↑/↓ j/k", key_style),
            Span::styled(" 移动    ", Style::default().fg(muted_color())),
            Span::styled("Enter/l", key_style),
            Span::styled(" 修改    ", Style::default().fg(muted_color())),
            Span::styled("/", key_style),
            Span::styled(" 搜索    ", Style::default().fg(muted_color())),
            Span::styled("n/N", key_style),
            Span::styled(" 匹配    ", Style::default().fg(muted_color())),
            Span::styled("Esc/h", key_style),
            Span::styled(" 返回", Style::default().fg(muted_color())),
        ]),
    };
    frame.render_widget(Paragraph::new(line), area);
}
