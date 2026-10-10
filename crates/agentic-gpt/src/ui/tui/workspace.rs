use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::cli_i18n::UiLanguage;

use super::{
    centered_overlay, render_contextual_footer, render_horizontal_rule, render_surface_header,
    surface_shell_areas, ProcessScreen, TerminalEvent, Theme,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceRoute {
    Process,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceCommand {
    Route(WorkspaceRoute),
    Quit,
}

#[derive(Clone, Copy)]
struct CommandEntry {
    name: &'static str,
    description_en: &'static str,
    description_zh: &'static str,
    command: WorkspaceCommand,
}

const COMMANDS: &[CommandEntry] = &[
    CommandEntry {
        name: "process",
        description_en: "Managed processes",
        description_zh: "受管进程",
        command: WorkspaceCommand::Route(WorkspaceRoute::Process),
    },
    CommandEntry {
        name: "quit",
        description_en: "Exit console",
        description_zh: "退出控制台",
        command: WorkspaceCommand::Quit,
    },
];

#[derive(Default)]
struct CommandPalette {
    query: String,
    selected: usize,
}

pub(crate) struct WorkspaceState {
    route: WorkspaceRoute,
    process: ProcessScreen,
    palette: Option<CommandPalette>,
    exited: bool,
    language: UiLanguage,
}

impl WorkspaceState {
    pub(crate) fn process(process: ProcessScreen, language: UiLanguage) -> Self {
        Self {
            route: WorkspaceRoute::Process,
            process,
            palette: None,
            exited: false,
            language,
        }
    }

    pub(crate) fn render(&self, frame: &mut Frame, theme: &Theme) {
        let [header, top_rule, body, bottom_rule, footer] = surface_shell_areas(frame.area());
        render_surface_header(frame, header, self.title(), self.status(), theme);
        render_horizontal_rule(frame, top_rule, theme);
        match self.route {
            WorkspaceRoute::Process => self.process.render_body(frame, body, theme),
        }
        render_horizontal_rule(frame, bottom_rule, theme);
        let mut hints = match self.route {
            WorkspaceRoute::Process => self.process.footer_hints(),
        };
        hints.push((":", t(self.language, "views", "视图")));
        hints.push(("q", t(self.language, "exit", "退出")));
        render_contextual_footer(frame, footer, &hints, theme);
        if let Some(palette) = &self.palette {
            render_palette(frame, palette, self.language, theme);
        }
    }

    pub(crate) fn handle_event(&mut self, event: TerminalEvent) -> Result<()> {
        if self.palette.is_some() {
            self.process.refresh();
            if let TerminalEvent::Key(key) = event {
                self.handle_palette_key(key);
            }
            return Ok(());
        }

        if let TerminalEvent::Key(key) = &event {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                self.exited = true;
                return Ok(());
            }
            match key.code {
                KeyCode::Char(':') => {
                    self.process.refresh();
                    self.palette = Some(CommandPalette::default());
                    return Ok(());
                }
                KeyCode::Char('q') => {
                    self.exited = true;
                    return Ok(());
                }
                KeyCode::Esc if !self.process.detail_active() => {
                    self.exited = true;
                    return Ok(());
                }
                _ => {}
            }
        }

        self.process.handle_event(event)
    }

    pub(crate) fn exited(&self) -> bool {
        self.exited
    }

    fn title(&self) -> &'static str {
        match (self.route, self.language) {
            (WorkspaceRoute::Process, UiLanguage::En) => "AgenticGPT / Process",
            (WorkspaceRoute::Process, UiLanguage::ZhCn) => "AgenticGPT / 进程",
        }
    }

    fn status(&self) -> &'static str {
        match self.route {
            WorkspaceRoute::Process => self.process.status(),
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.palette = None;
            return;
        }
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.palette = None,
            KeyCode::Backspace => {
                palette.query.pop();
                palette.selected = 0;
            }
            KeyCode::Up => {
                let count = filtered_commands(&palette.query).len();
                if count > 0 {
                    palette.selected = palette.selected.saturating_sub(1);
                }
            }
            KeyCode::Down => {
                let count = filtered_commands(&palette.query).len();
                if count > 0 {
                    palette.selected = (palette.selected + 1).min(count - 1);
                }
            }
            KeyCode::Enter => {
                let command = filtered_commands(&palette.query)
                    .get(palette.selected)
                    .map(|entry| entry.command);
                if let Some(command) = command {
                    self.execute_command(command);
                }
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                palette.query.push(character);
                palette.selected = 0;
            }
            _ => {}
        }
    }

    fn execute_command(&mut self, command: WorkspaceCommand) {
        match command {
            WorkspaceCommand::Route(route) => self.route = route,
            WorkspaceCommand::Quit => self.exited = true,
        }
        self.palette = None;
    }
}

fn filtered_commands(query: &str) -> Vec<&'static CommandEntry> {
    let query = query.trim().to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|entry| query.is_empty() || entry.name.contains(&query))
        .collect()
}

fn render_palette(
    frame: &mut Frame,
    palette: &CommandPalette,
    language: UiLanguage,
    theme: &Theme,
) {
    let entries = filtered_commands(&palette.query);
    let height = 4 + entries.len().max(1) as u16;
    let area = centered_overlay(frame.area(), 54, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(t(language, " Views ", " 视图 "))
            .style(theme.surface),
        area,
    );
    let inner = area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let mut lines = vec![Line::from(vec![
        Span::styled(":", theme.accent),
        Span::styled(palette.query.clone(), theme.normal),
        Span::styled("█", theme.pointer),
    ])];
    lines.push(Line::raw(""));
    if entries.is_empty() {
        lines.push(Line::styled(
            t(language, "No matching command", "没有匹配的命令"),
            theme.muted,
        ));
    } else {
        for (index, entry) in entries.iter().enumerate() {
            let focused = index == palette.selected;
            let pointer = if focused {
                Span::styled("❯ ", theme.pointer)
            } else {
                Span::raw("  ")
            };
            let style = if focused {
                theme.emphasis
            } else {
                theme.normal
            };
            let description = match language {
                UiLanguage::En => entry.description_en,
                UiLanguage::ZhCn => entry.description_zh,
            };
            lines.push(Line::from(vec![
                pointer,
                Span::styled(format!(":{:<12}", entry.name), style),
                Span::styled(description, theme.muted),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn t<'a>(language: UiLanguage, en: &'a str, zh_cn: &'a str) -> &'a str {
    match language {
        UiLanguage::En => en,
        UiLanguage::ZhCn => zh_cn,
    }
}
