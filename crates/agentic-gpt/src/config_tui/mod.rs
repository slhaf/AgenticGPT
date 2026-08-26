mod app;
mod input;
mod navigation;
mod pages;

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::cli_i18n::UiLanguage;
use crate::config_setup::{SetupSeed, SetupSession};

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
    let app = ConfigTuiApp::new(session);
    match crate::tui::TuiApp::config(app, language).run()? {
        crate::tui::TuiOutcome::ConfigCommitted(summary) => Ok(summary),
        crate::tui::TuiOutcome::Cancelled | crate::tui::TuiOutcome::Exited => {
            Err(anyhow!("config_init_cancelled"))
        }
    }
}
