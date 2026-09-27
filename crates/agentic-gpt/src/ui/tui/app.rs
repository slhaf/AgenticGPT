use std::time::Duration;

use anyhow::{anyhow, Result};
use ratatui::Frame;

use crate::cli_i18n::UiLanguage;
use crate::config_templates::InitSummary;
use crate::config_tui::ConfigTuiApp;

use super::{ProcessScreen, TerminalEvent, TerminalSession, Theme, WorkspaceState};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) enum TuiOutcome {
    ConfigCommitted(InitSummary),
    Cancelled,
    Exited,
}

enum TuiScreen {
    Config(Box<ConfigTuiApp>),
    Workspace(WorkspaceState),
}

pub(crate) struct TuiApp {
    screen: TuiScreen,
    theme: Theme,
    language: UiLanguage,
}

impl TuiApp {
    pub(crate) fn config(app: ConfigTuiApp, language: UiLanguage) -> Self {
        Self {
            screen: TuiScreen::Config(Box::new(app)),
            theme: Theme::from_env(),
            language,
        }
    }

    pub(crate) fn process(app: ProcessScreen, language: UiLanguage) -> Self {
        Self {
            screen: TuiScreen::Workspace(WorkspaceState::process(app, language)),
            theme: Theme::from_env(),
            language,
        }
    }

    pub(crate) fn run(mut self) -> Result<TuiOutcome> {
        let mut terminal = match TerminalSession::enter() {
            Ok(terminal) => terminal,
            Err(_) => return Err(anyhow!(terminal_error_message(self.language))),
        };

        loop {
            if terminal
                .terminal_mut()
                .draw(|frame| self.render(frame))
                .is_err()
            {
                self.set_runtime_error();
                return Err(anyhow!(terminal_error_message(self.language)));
            }

            if let Some(outcome) = self.take_outcome()? {
                return Ok(outcome);
            }

            let event = match terminal.next_event(EVENT_POLL_INTERVAL) {
                Ok(event) => event,
                Err(_) => {
                    self.set_runtime_error();
                    let _ = terminal.terminal_mut().draw(|frame| self.render(frame));
                    return Err(anyhow!(terminal_error_message(self.language)));
                }
            };

            if self.handle_event(event).is_err() {
                self.set_runtime_error();
                let _ = terminal.terminal_mut().draw(|frame| self.render(frame));
                return Err(anyhow!(terminal_error_message(self.language)));
            }

            if let Some(outcome) = self.take_outcome()? {
                return Ok(outcome);
            }
        }
    }

    fn render(&self, frame: &mut Frame) {
        match &self.screen {
            TuiScreen::Config(app) => app.render(frame, &self.theme),
            TuiScreen::Workspace(workspace) => workspace.render(frame, &self.theme),
        }
    }

    fn handle_event(&mut self, event: TerminalEvent) -> Result<()> {
        match &mut self.screen {
            TuiScreen::Config(app) => app.handle_event(event),
            TuiScreen::Workspace(workspace) => workspace.handle_event(event),
        }
    }

    fn set_runtime_error(&mut self) {
        match &mut self.screen {
            TuiScreen::Config(app) => app.set_runtime_error(),
            TuiScreen::Workspace(_) => {}
        }
    }

    fn take_outcome(&mut self) -> Result<Option<TuiOutcome>> {
        match &mut self.screen {
            TuiScreen::Config(app) => {
                if app.state().cancelled {
                    return Ok(Some(TuiOutcome::Cancelled));
                }
                if !app.state().finished {
                    return Ok(None);
                }
                if let Some(error) = app.state().system_error.as_ref() {
                    return Err(anyhow!(error.code));
                }
                Ok(Some(
                    app.take_committed_summary()
                        .map(TuiOutcome::ConfigCommitted)
                        .unwrap_or(TuiOutcome::Cancelled),
                ))
            }
            TuiScreen::Workspace(workspace) => Ok(workspace.exited().then_some(TuiOutcome::Exited)),
        }
    }
}

fn terminal_error_message(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::ZhCn => "终端初始化或刷新失败，请重试。",
        UiLanguage::En => "Terminal setup or refresh failed; please retry.",
    }
}
