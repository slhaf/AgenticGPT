use std::collections::HashMap;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use crate::config_setup::{SetupField, SetupSession, ValidationErrors};
use crate::config_templates::{InitSummary, RuntimeMode, SecretValue};
use crate::tui::{TerminalEvent, Theme};
use crate::WorkerProfile;

use super::input::{apply_text_key, EditState};
use super::navigation::{ConfigPage, Navigation, ReturnTarget};
use super::pages;

pub(crate) struct TuiState {
    pub(crate) page: ConfigPage,
    pub(crate) return_target: ReturnTarget,
    pub(crate) focus: usize,
    pub(crate) editing: Option<EditState>,
    pub(crate) scroll: u16,
    pub(crate) modal: Option<String>,
    pub(crate) cancelled: bool,
    pub(crate) committed_summary: Option<InitSummary>,
}

pub(crate) enum TuiAction {
    Next,
    Back,
    Cancel,
    Activate,
    Text(char),
    Backspace,
    Delete,
    CursorLeft,
    CursorRight,
    MoveNext,
    MovePrevious,
    SetMode(RuntimeMode),
    SetProfile(WorkerProfile),
}

pub(crate) struct ConfigTuiApp {
    session: SetupSession,
    navigation: Navigation,
    state: TuiState,
    theme: Theme,
    field_errors: HashMap<SetupField, String>,
}

impl ConfigTuiApp {
    pub(crate) fn new(session: SetupSession) -> Self {
        let navigation = Navigation::new(session.selected_mode());
        let page = navigation.current();
        Self {
            session,
            navigation,
            state: TuiState {
                page,
                return_target: ReturnTarget::MainFlow,
                focus: 0,
                editing: None,
                scroll: 0,
                modal: None,
                cancelled: false,
                committed_summary: None,
            },
            theme: Theme::from_env(),
            field_errors: HashMap::new(),
        }
    }

    pub(crate) fn page(&self) -> ConfigPage {
        self.state.page
    }

    pub(crate) fn state(&self) -> &TuiState {
        &self.state
    }

    pub(crate) fn session(&self) -> &SetupSession {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut SetupSession {
        &mut self.session
    }

    pub(crate) fn editing(&self) -> Option<&EditState> {
        self.state.editing.as_ref()
    }

    pub(crate) fn focus_field(&mut self, field: SetupField) {
        self.state.focus = self.field_index(field).unwrap_or(self.state.focus);
    }

    pub(crate) fn focused_field(&self) -> Option<SetupField> {
        match self.state.page {
            ConfigPage::Basic => [SetupField::Mode, SetupField::Profile]
                .get(self.state.focus)
                .copied(),
            ConfigPage::Connection => pages::connection_fields_for_session(&self.session)
                .get(self.state.focus)
                .copied(),
            _ => None,
        }
    }

    pub(crate) fn handle_event(&mut self, event: TerminalEvent) -> Result<()> {
        if let TerminalEvent::Key(key) = event {
            self.handle_key(key)?;
        }
        Ok(())
    }

    pub(crate) fn handle_action(&mut self, action: TuiAction) -> Result<()> {
        match action {
            TuiAction::Next => {
                if self.state.editing.is_some() {
                    self.commit_edit();
                } else {
                    self.next_page();
                }
            }
            TuiAction::Back => {
                if self.state.editing.take().is_some() {
                    return Ok(());
                }
                if self.navigation.back() {
                    self.sync_page();
                }
            }
            TuiAction::Cancel => {
                self.state.cancelled = true;
                self.state.editing = None;
            }
            TuiAction::Activate => {
                if self.state.editing.is_some() {
                    self.commit_edit();
                } else {
                    self.activate_focus();
                }
            }
            TuiAction::Text(character) => {
                self.with_edit(|edit| apply_text_key(edit, KeyCode::Char(character)));
            }
            TuiAction::Backspace => {
                self.with_edit(|edit| apply_text_key(edit, KeyCode::Backspace));
            }
            TuiAction::Delete => {
                self.with_edit(|edit| apply_text_key(edit, KeyCode::Delete));
            }
            TuiAction::CursorLeft => {
                self.with_edit(|edit| apply_text_key(edit, KeyCode::Left));
            }
            TuiAction::CursorRight => {
                self.with_edit(|edit| apply_text_key(edit, KeyCode::Right));
            }
            TuiAction::MoveNext => self.move_focus(1),
            TuiAction::MovePrevious => self.move_focus(-1),
            TuiAction::SetMode(mode) => {
                self.session.set_mode(mode);
                self.navigation.set_mode(mode);
                self.state.focus = 0;
                self.sync_page();
            }
            TuiAction::SetProfile(profile) => self.session.set_profile(profile),
        }
        Ok(())
    }

    pub(crate) fn render(&self, frame: &mut Frame) {
        pages::render(
            frame,
            self.state.page,
            &self.session,
            &self.state,
            &self.theme,
            &self.field_errors,
            self.navigation.progress(),
        );
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return self.handle_action(TuiAction::Cancel);
        }
        if self.state.editing.is_some() {
            return match key.code {
                KeyCode::Esc => self.handle_action(TuiAction::Back),
                KeyCode::Enter => self.handle_action(TuiAction::Activate),
                KeyCode::Backspace => self.handle_action(TuiAction::Backspace),
                KeyCode::Delete => self.handle_action(TuiAction::Delete),
                KeyCode::Left => self.handle_action(TuiAction::CursorLeft),
                KeyCode::Right => self.handle_action(TuiAction::CursorRight),
                KeyCode::Char(character) => self.handle_action(TuiAction::Text(character)),
                _ => Ok(()),
            };
        }
        match key.code {
            KeyCode::Esc => self.handle_action(TuiAction::Back),
            KeyCode::Enter => {
                if self.focused_field().is_some() {
                    self.handle_action(TuiAction::Activate)
                } else {
                    self.handle_action(TuiAction::Next)
                }
            }
            KeyCode::Tab => self.handle_action(TuiAction::MoveNext),
            KeyCode::BackTab => self.handle_action(TuiAction::MovePrevious),
            KeyCode::Down | KeyCode::Right => self.handle_action(TuiAction::MoveNext),
            KeyCode::Up | KeyCode::Left => self.handle_action(TuiAction::MovePrevious),
            _ => Ok(()),
        }
    }

    fn next_page(&mut self) {
        let validation = match self.state.page {
            ConfigPage::Basic => self.session.validate_basic(),
            ConfigPage::Connection => self.session.validate_connection(),
            _ => Ok(()),
        };
        if let Err(errors) = validation {
            self.record_errors(errors);
            return;
        }
        self.field_errors.clear();
        if self.navigation.advance() {
            self.state.focus = 0;
            self.sync_page();
        }
    }

    fn activate_focus(&mut self) {
        let Some(field) = self.focused_field() else {
            return;
        };
        match field {
            SetupField::Mode => {
                let next = match self.session.selected_mode() {
                    RuntimeMode::Standalone => RuntimeMode::Hub,
                    RuntimeMode::Hub => RuntimeMode::Local,
                    RuntimeMode::Local => RuntimeMode::Standalone,
                };
                self.session.set_mode(next);
                self.navigation.set_mode(next);
                self.state.focus = 0;
                self.sync_page();
            }
            SetupField::Profile => {
                let next = match self.session.selected_profile() {
                    WorkerProfile::Normal => WorkerProfile::Room,
                    WorkerProfile::Room => WorkerProfile::Normal,
                };
                self.session.set_profile(next);
            }
            SetupField::TunnelSecretSource => {
                let draft = self.session.standalone_mut();
                draft.secret_source = match draft.secret_source {
                    crate::config_templates::TunnelSecretSource::File => {
                        crate::config_templates::TunnelSecretSource::Environment
                    }
                    crate::config_templates::TunnelSecretSource::Environment => {
                        crate::config_templates::TunnelSecretSource::File
                    }
                };
            }
            SetupField::ProvisionTunnelSecret => {
                let draft = self.session.standalone_mut();
                draft.provision_secret_now = !draft.provision_secret_now;
            }
            _ => {
                let value = pages::connection_value(&self.session, field).unwrap_or_default();
                self.state.editing = Some(EditState::new(field, value));
            }
        }
    }

    fn commit_edit(&mut self) {
        let Some(edit) = self.state.editing.take() else {
            return;
        };
        let field = edit.field;
        let value = edit.buffer;
        match field {
            SetupField::TunnelId => self.session.standalone_mut().tunnel_id = value,
            SetupField::TunnelSecretPath => self.session.standalone_mut().secret_path = value,
            SetupField::TunnelSecretEnvironment => {
                self.session.standalone_mut().secret_environment = value
            }
            SetupField::TunnelSecretValue => {
                self.session.standalone_mut().secret_value = Some(SecretValue::new(value))
            }
            SetupField::HubUrl => self.session.hub_mut().hub_url = value,
            SetupField::HubTransport => self.session.hub_mut().hub_transport = value,
            SetupField::AgentId => self.session.hub_mut().agent_id = value,
            SetupField::AgentSecret => {
                self.session.hub_mut().agent_secret = Some(SecretValue::new(value))
            }
            _ => {}
        }
        match self.session.validate_field(field) {
            Ok(()) => {
                self.field_errors.remove(&field);
            }
            Err(errors) => self.record_errors(errors),
        }
    }

    fn with_edit(&mut self, update: impl FnOnce(EditState) -> EditState) {
        if let Some(edit) = self.state.editing.take() {
            self.state.editing = Some(update(edit));
        }
    }

    fn move_focus(&mut self, direction: isize) {
        let length = match self.state.page {
            ConfigPage::Basic => 3,
            ConfigPage::Connection => pages::connection_fields_for_session(&self.session).len() + 1,
            _ => 1,
        };
        if length == 0 {
            return;
        }
        let current = self.state.focus as isize;
        self.state.focus = (current + direction).rem_euclid(length as isize) as usize;
    }

    fn field_index(&self, field: SetupField) -> Option<usize> {
        match self.state.page {
            ConfigPage::Basic => [SetupField::Mode, SetupField::Profile]
                .iter()
                .position(|candidate| *candidate == field),
            ConfigPage::Connection => pages::connection_fields_for_session(&self.session)
                .iter()
                .position(|candidate| *candidate == field),
            _ => None,
        }
    }

    fn record_errors(&mut self, errors: ValidationErrors) {
        self.field_errors.clear();
        if let Some(first) = errors.first() {
            self.focus_field(first.field);
        }
        for error in errors {
            self.field_errors
                .insert(error.field, error.code.to_string());
        }
    }

    fn sync_page(&mut self) {
        self.state.page = self.navigation.current();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession};
    use crate::config_templates::{RuntimeMode, SecretValue};
    use crate::tui::TerminalEvent;
    use crate::WorkerProfile;

    use super::super::{ConfigPage, ConfigTuiApp, TuiAction};

    fn app(mode: RuntimeMode) -> ConfigTuiApp {
        ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(mode),
                profile: Some(WorkerProfile::Normal),
                tunnel_id: Some("staged-tunnel".to_string()),
                tunnel_api_key: Some("file:/tmp/staged-secret".to_string()),
                agent_secret: Some(SecretValue::new("hub-secret-marker")),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            PathBuf::from("/tmp/config-tui-test.json"),
        ))
    }

    #[test]
    fn standalone_and_local_flow_have_expected_pages() {
        let mut standalone_app = app(RuntimeMode::Standalone);
        assert_eq!(standalone_app.page(), ConfigPage::Basic);
        standalone_app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(standalone_app.page(), ConfigPage::Connection);
        standalone_app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(standalone_app.page(), ConfigPage::OptionalCenter);
        standalone_app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(standalone_app.page(), ConfigPage::Review);

        let mut hub = app(RuntimeMode::Hub);
        hub.handle_action(TuiAction::Next).unwrap();
        assert_eq!(hub.page(), ConfigPage::Connection);
        hub.handle_action(TuiAction::Next).unwrap();
        assert_eq!(hub.page(), ConfigPage::OptionalCenter);
        hub.handle_action(TuiAction::Next).unwrap();
        assert_eq!(hub.page(), ConfigPage::Review);

        let mut local = app(RuntimeMode::Local);
        local.handle_action(TuiAction::Next).unwrap();
        assert_eq!(local.page(), ConfigPage::OptionalCenter);
        local.handle_action(TuiAction::Next).unwrap();
        assert_eq!(local.page(), ConfigPage::Review);
    }

    #[test]
    fn mode_changes_rebuild_future_flow_without_losing_drafts() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::SetMode(RuntimeMode::Hub))
            .unwrap();
        assert_eq!(app.page(), ConfigPage::Basic);
        assert_eq!(app.session().standalone().tunnel_id, "staged-tunnel");
        assert_eq!(app.session().hub().hub_url, "http://localhost:8787");

        app.handle_action(TuiAction::SetMode(RuntimeMode::Standalone))
            .unwrap();
        assert_eq!(app.page(), ConfigPage::Basic);
        assert_eq!(app.session().standalone().tunnel_id, "staged-tunnel");
    }

    #[test]
    fn child_back_returns_to_basic_and_root_back_is_a_noop() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.page(), ConfigPage::Connection);
        app.handle_action(TuiAction::Back).unwrap();
        assert_eq!(app.page(), ConfigPage::Basic);
        app.handle_action(TuiAction::Back).unwrap();
        assert_eq!(app.page(), ConfigPage::Basic);
    }

    #[test]
    fn focus_wraps_across_connection_fields_and_action() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.state().focus, 0);
        app.handle_action(TuiAction::MovePrevious).unwrap();
        assert_eq!(app.state().focus, 4);
        app.handle_action(TuiAction::MoveNext).unwrap();
        assert_eq!(app.state().focus, 0);
    }

    #[test]
    fn enter_event_activates_fields_and_advances_from_action() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        app.focus_field(crate::config_setup::SetupField::TunnelId);
        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert!(app.editing().is_some());
        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert!(app.editing().is_none());

        app.handle_action(TuiAction::MovePrevious).unwrap();
        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.page(), ConfigPage::OptionalCenter);
    }

    #[test]
    fn editing_enter_seeds_confirmed_value_and_escape_discards_buffer() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        app.focus_field(crate::config_setup::SetupField::TunnelId);
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(app.editing().unwrap().buffer, "staged-tunnel");
        app.handle_action(TuiAction::Text('x')).unwrap();
        app.handle_action(TuiAction::Back).unwrap();
        assert!(app.editing().is_none());
        assert_eq!(app.session().standalone().tunnel_id, "staged-tunnel");
    }

    #[test]
    fn ctrl_c_cancels_from_navigation_and_editing() {
        let mut navigation_app = app(RuntimeMode::Standalone);
        navigation_app.handle_action(TuiAction::Cancel).unwrap();
        assert!(navigation_app.state().cancelled);
        let mut editing_app = app(RuntimeMode::Standalone);
        editing_app.handle_action(TuiAction::Next).unwrap();
        editing_app.focus_field(crate::config_setup::SetupField::TunnelId);
        editing_app.handle_action(TuiAction::Activate).unwrap();
        editing_app.handle_action(TuiAction::Cancel).unwrap();
        assert!(editing_app.state().cancelled);
    }
}
