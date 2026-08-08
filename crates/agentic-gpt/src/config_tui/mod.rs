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
    let mut terminal = TerminalSession::enter()?;

    loop {
        terminal.terminal_mut().draw(|frame| app.render(frame))?;
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
        let event = terminal.next_event(Duration::from_millis(100))?;
        app.handle_event(event)?;
    }
}
