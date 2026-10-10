use std::sync::mpsc::Receiver;

use agentic_gpt_protocol::{ProcessKind, ProcessListItem, ProcessListResponse, ProcessState};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Margin, Rect},
    text::{Line, Span, Text},
    widgets::Paragraph,
    Frame,
};

use crate::cli_i18n::UiLanguage;

use super::{
    master_detail_layout, render_inspector, render_surface, MasterDetailSpec, PaneMode,
    TerminalEvent, Theme,
};

pub(crate) enum ProcessUpdate {
    Processes(ProcessListResponse),
    Error(String),
}

pub(crate) struct ProcessScreen {
    receiver: Receiver<ProcessUpdate>,
    processes: Vec<ProcessListItem>,
    selected: usize,
    next_cursor: Option<String>,
    error: Option<String>,
    pane_mode: PaneMode,
    language: UiLanguage,
}

impl ProcessScreen {
    pub(crate) fn new(receiver: Receiver<ProcessUpdate>, language: UiLanguage) -> Self {
        Self {
            receiver,
            processes: Vec::new(),
            selected: 0,
            next_cursor: None,
            error: None,
            pane_mode: PaneMode::Master,
            language,
        }
    }

    pub(crate) fn render_body(&self, frame: &mut Frame, body: Rect, theme: &Theme) {
        let panes = master_detail_layout(
            body,
            MasterDetailSpec::new(Constraint::Min(34), 2, Constraint::Min(28), 68),
            self.pane_mode,
        );
        self.render_process_list(frame, panes.master, theme);
        if panes.detail.width > 0 {
            self.render_detail(frame, panes.detail, theme);
        }
    }

    fn render_process_list(&self, frame: &mut Frame, body: Rect, theme: &Theme) {
        if body.width == 0 || body.height == 0 {
            return;
        }
        let mut lines = Vec::new();
        if let Some(error) = &self.error {
            lines.push(Line::from(vec![
                Span::styled("! ", theme.error),
                Span::styled(
                    format!(
                        "{}: {error}",
                        t(self.language, "Agent unavailable", "Agent 不可用")
                    ),
                    theme.error,
                ),
            ]));
            lines.push(Line::raw(""));
        }

        if self.processes.is_empty() {
            lines.push(Line::styled(
                t(
                    self.language,
                    "No managed processes yet.",
                    "暂时没有 Managed Process。",
                ),
                theme.muted,
            ));
        } else {
            let visible_rows = usize::from(body.height).saturating_sub(lines.len()).max(1);
            let start = if self.selected >= visible_rows {
                self.selected + 1 - visible_rows
            } else {
                0
            };
            let end = (start + visible_rows).min(self.processes.len());
            for (index, process) in self.processes[start..end].iter().enumerate() {
                let absolute = start + index;
                lines.push(process_line(process, absolute == self.selected, theme));
            }
        }

        frame.render_widget(Paragraph::new(Text::from(lines)), body);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        render_surface(frame, area, theme);
        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let Some(process) = self.processes.get(self.selected) else {
            render_inspector(
                frame,
                inner,
                t(self.language, "Process preview", "Process 预览"),
                &[t(
                    self.language,
                    "Select a managed process to inspect it.",
                    "选择一个 Managed Process 查看详情。",
                )],
                theme,
            );
            return;
        };
        let kind = match process.kind {
            ProcessKind::Command => "command",
            ProcessKind::Skill => "skill",
            ProcessKind::Mcp => "mcp",
        };
        let group = process.group.as_deref().unwrap_or("—");
        let created = process.created_at.to_rfc3339();
        let started = process
            .started_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "—".to_string());
        let finished = process
            .finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "—".to_string());
        let body = [
            format!("processId   {}", process.process_id),
            format!("group       {group}"),
            format!("kind        {kind}"),
            format!("state       {}", process.state.as_str()),
            String::new(),
            format!("created     {created}"),
            format!("started     {started}"),
            format!("finished    {finished}"),
        ];
        let refs = body.iter().map(String::as_str).collect::<Vec<_>>();
        render_inspector(
            frame,
            inner,
            t(self.language, "Process preview", "Process 预览"),
            &refs,
            theme,
        );
    }

    pub(crate) fn status(&self) -> &'static str {
        match &self.error {
            Some(_) => t(self.language, "degraded", "连接异常"),
            None if self.processes.is_empty() => t(self.language, "waiting", "等待数据"),
            None => t(self.language, "live", "实时"),
        }
    }

    pub(crate) fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.pane_mode == PaneMode::Detail {
            vec![(
                "Esc",
                t(self.language, "back to processes", "返回 Process 列表"),
            )]
        } else {
            vec![
                ("↑/↓ j/k", t(self.language, "select", "选择")),
                ("Enter/l", t(self.language, "preview", "预览")),
            ]
        }
    }

    pub(crate) fn detail_active(&self) -> bool {
        self.pane_mode == PaneMode::Detail
    }

    pub(crate) fn refresh(&mut self) {
        self.drain_updates();
    }

    pub(crate) fn handle_event(&mut self, event: TerminalEvent) -> Result<()> {
        self.refresh();
        if let TerminalEvent::Key(key) = event {
            self.handle_key(key);
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.pane_mode == PaneMode::Detail {
            if key.code == KeyCode::Esc {
                self.pane_mode = PaneMode::Master;
            }
            return;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('l') if !self.processes.is_empty() => {
                self.pane_mode = PaneMode::Detail;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.processes.is_empty() {
                    self.selected = (self.selected + 1).min(self.processes.len() - 1);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.processes.len().saturating_sub(1);
            }
            _ => {}
        }
    }

    fn drain_updates(&mut self) {
        while let Ok(update) = self.receiver.try_recv() {
            match update {
                ProcessUpdate::Processes(page) => {
                    let selected_id = self
                        .processes
                        .get(self.selected)
                        .map(|process| process.process_id.clone());
                    self.processes = page.processes;
                    self.next_cursor = page.next_cursor;
                    self.error = None;
                    self.selected = selected_id
                        .and_then(|id| {
                            self.processes
                                .iter()
                                .position(|process| process.process_id == id)
                        })
                        .unwrap_or_else(|| {
                            self.selected.min(self.processes.len().saturating_sub(1))
                        });
                    if self.processes.is_empty() {
                        self.pane_mode = PaneMode::Master;
                    }
                }
                ProcessUpdate::Error(error) => self.error = Some(error),
            }
        }
    }
}

fn process_line(process: &ProcessListItem, selected: bool, theme: &Theme) -> Line<'static> {
    let marker = if selected {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    };
    let group = clip(process.group.as_deref().unwrap_or("—"), 18);
    let kind = match process.kind {
        ProcessKind::Command => "command",
        ProcessKind::Skill => "skill",
        ProcessKind::Mcp => "mcp",
    };
    let state_style = match process.state {
        ProcessState::Completed => theme.success,
        ProcessState::Failed | ProcessState::Rejected | ProcessState::TimedOut => theme.error,
        state if state.is_active() => theme.accent,
        _ => theme.muted,
    };
    let id = short_process_id(&process.process_id);
    let base = if selected {
        theme.emphasis
    } else {
        theme.normal
    };
    Line::from(vec![
        marker,
        Span::styled(format!("{group:<18}"), base),
        Span::raw("  "),
        Span::styled(format!("{kind:<7}"), theme.muted),
        Span::raw("  "),
        Span::styled(format!("{:<21}", process.state.as_str()), state_style),
        Span::raw("  "),
        Span::styled(id, theme.dim),
    ])
}

fn short_process_id(process_id: &str) -> String {
    let suffix = process_id.rsplit('_').next().unwrap_or(process_id);
    clip(suffix, 12)
}

fn clip(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() && max_chars > 1 {
        format!(
            "{}…",
            prefix.chars().take(max_chars - 1).collect::<String>()
        )
    } else {
        prefix
    }
}

fn t<'a>(language: UiLanguage, en: &'a str, zh_cn: &'a str) -> &'a str {
    match language {
        UiLanguage::En => en,
        UiLanguage::ZhCn => zh_cn,
    }
}
