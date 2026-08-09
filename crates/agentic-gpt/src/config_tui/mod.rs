mod app;
mod input;
mod navigation;
mod pages;

use std::{path::Path, time::Duration};

use anyhow::{anyhow, Result};

use crate::cli_i18n::UiLanguage;
use crate::config_setup::{SetupSeed, SetupSession};
use crate::tui::TerminalSession;

#[cfg(test)]
pub(crate) use app::{Committer, TuiAction};
pub(crate) use app::{ConfigTuiApp, SystemError, TuiState};
pub(crate) use navigation::ConfigPage;

pub(crate) fn run_config_tui(
    config_path: &Path,
    seed: SetupSeed,
    language: UiLanguage,
) -> Result<crate::config_templates::InitSummary> {
    let session = SetupSession::new(seed, language, config_path.to_path_buf());
    let mut app = ConfigTuiApp::new(session);
    let mut terminal = match TerminalSession::enter() {
        Ok(terminal) => terminal,
        Err(_) => return Err(anyhow!(terminal_error_message(language))),
    };

    loop {
        if terminal
            .terminal_mut()
            .draw(|frame| app.render(frame))
            .is_err()
        {
            app.set_runtime_error();
            return Err(anyhow!(terminal_error_message(language)));
        }
        if app.state().cancelled {
            return Err(anyhow!("config_init_cancelled"));
        }
        if app.state().finished {
            if let Some(error) = app.state().system_error.as_ref() {
                return Err(anyhow!(error.code));
            }
            return app
                .take_committed_summary()
                .ok_or_else(|| anyhow!("config_init_cancelled"));
        }
        let event = match terminal.next_event(Duration::from_millis(100)) {
            Ok(event) => event,
            Err(_) => {
                app.set_runtime_error();
                let _ = terminal.terminal_mut().draw(|frame| app.render(frame));
                return Err(anyhow!(terminal_error_message(language)));
            }
        };
        if app.handle_event(event).is_err() {
            app.set_runtime_error();
            let _ = terminal.terminal_mut().draw(|frame| app.render(frame));
            return Err(anyhow!(terminal_error_message(language)));
        }
        if app.state().finished {
            if let Some(error) = app.state().system_error.as_ref() {
                return Err(anyhow!(error.code));
            }
            return app
                .take_committed_summary()
                .ok_or_else(|| anyhow!("config_init_cancelled"));
        }
    }
}

fn terminal_error_message(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::ZhCn => "终端初始化或刷新失败，请重试配置初始化。",
        UiLanguage::En => {
            "Terminal setup or refresh failed; please retry configuration initialization."
        }
    }
}
