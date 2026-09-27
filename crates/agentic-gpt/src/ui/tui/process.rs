use std::sync::mpsc::Receiver;

use agentic_gpt_protocol::{JobKind, JobListItem, JobListResponse, JobState};
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
    Jobs(JobListResponse),
    Error(String),
}

pub(crate) struct ProcessScreen {
    receiver: Receiver<ProcessUpdate>,
    jobs: Vec<JobListItem>,
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
            jobs: Vec::new(),
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
        self.render_job_list(frame, panes.master, theme);
        if panes.detail.width > 0 {
            self.render_detail(frame, panes.detail, theme);
        }
    }

    fn render_job_list(&self, frame: &mut Frame, body: Rect, theme: &Theme) {
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

        if self.jobs.is_empty() {
            lines.push(Line::styled(
                t(
                    self.language,
                    "No managed jobs yet.",
                    "暂时没有 Managed Job。",
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
            let end = (start + visible_rows).min(self.jobs.len());
            for (index, job) in self.jobs[start..end].iter().enumerate() {
                let absolute = start + index;
                lines.push(job_line(job, absolute == self.selected, theme));
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
        let Some(job) = self.jobs.get(self.selected) else {
            render_inspector(
                frame,
                inner,
                t(self.language, "Job preview", "Job 预览"),
                &[t(
                    self.language,
                    "Select a managed Job to inspect it.",
                    "选择一个 Managed Job 查看详情。",
                )],
                theme,
            );
            return;
        };
        let kind = match job.kind {
            JobKind::Process => "process",
            JobKind::Skill => "skill",
            JobKind::Mcp => "mcp",
        };
        let group = job.group.as_deref().unwrap_or("—");
        let created = job.created_at.to_rfc3339();
        let started = job
            .started_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "—".to_string());
        let finished = job
            .finished_at
            .map(|value| value.to_rfc3339())
            .unwrap_or_else(|| "—".to_string());
        let body = [
            format!("jobId   {}", job.job_id),
            format!("group   {group}"),
            format!("kind    {kind}"),
            format!("state   {}", job.state.as_str()),
            String::new(),
            format!("created {created}"),
            format!("started {started}"),
            format!("finished {finished}"),
        ];
        let refs = body.iter().map(String::as_str).collect::<Vec<_>>();
        render_inspector(
            frame,
            inner,
            t(self.language, "Job preview", "Job 预览"),
            &refs,
            theme,
        );
    }

    pub(crate) fn status(&self) -> &'static str {
        match &self.error {
            Some(_) => t(self.language, "degraded", "连接异常"),
            None if self.jobs.is_empty() => t(self.language, "waiting", "等待数据"),
            None => t(self.language, "live", "实时"),
        }
    }

    pub(crate) fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.pane_mode == PaneMode::Detail {
            vec![("Esc", t(self.language, "back to jobs", "返回 Job 列表"))]
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
            KeyCode::Enter | KeyCode::Char('l') if !self.jobs.is_empty() => {
                self.pane_mode = PaneMode::Detail;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.jobs.is_empty() {
                    self.selected = (self.selected + 1).min(self.jobs.len() - 1);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => self.selected = 0,
            KeyCode::End | KeyCode::Char('G') => {
                self.selected = self.jobs.len().saturating_sub(1);
            }
            _ => {}
        }
    }

    fn drain_updates(&mut self) {
        while let Ok(update) = self.receiver.try_recv() {
            match update {
                ProcessUpdate::Jobs(page) => {
                    let selected_id = self.jobs.get(self.selected).map(|job| job.job_id.clone());
                    self.jobs = page.jobs;
                    self.next_cursor = page.next_cursor;
                    self.error = None;
                    self.selected = selected_id
                        .and_then(|id| self.jobs.iter().position(|job| job.job_id == id))
                        .unwrap_or_else(|| self.selected.min(self.jobs.len().saturating_sub(1)));
                    if self.jobs.is_empty() {
                        self.pane_mode = PaneMode::Master;
                    }
                }
                ProcessUpdate::Error(error) => self.error = Some(error),
            }
        }
    }
}

fn job_line(job: &JobListItem, selected: bool, theme: &Theme) -> Line<'static> {
    let marker = if selected {
        Span::styled("❯ ", theme.pointer)
    } else {
        Span::raw("  ")
    };
    let group = clip(job.group.as_deref().unwrap_or("—"), 18);
    let kind = match job.kind {
        JobKind::Process => "process",
        JobKind::Skill => "skill",
        JobKind::Mcp => "mcp",
    };
    let state_style = match job.state {
        JobState::Completed => theme.success,
        JobState::Failed | JobState::Rejected | JobState::TimedOut => theme.error,
        state if state.is_active() => theme.accent,
        _ => theme.muted,
    };
    let id = short_job_id(&job.job_id);
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
        Span::styled(format!("{:<21}", job.state.as_str()), state_style),
        Span::raw("  "),
        Span::styled(id, theme.dim),
    ])
}

fn short_job_id(job_id: &str) -> String {
    let suffix = job_id.rsplit('_').next().unwrap_or(job_id);
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

#[cfg(test)]
mod tests {
    use super::{clip, short_job_id};

    #[test]
    fn clipping_and_short_id_are_bounded() {
        assert_eq!(clip("abcdefghijkl", 6), "abcde…");
        assert_eq!(clip("abc", 6), "abc");
        assert_eq!(short_job_id("job_boot_1234567890abcdef"), "1234567890a…");
    }
}
