use std::{collections::HashMap, path::Path};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use crate::cli_i18n::UiLanguage;
use crate::config_setup::{
    commit_wizard_outcome, OptionalSectionDraft, ReviewModel, SetupField, SetupSession,
    ValidationErrors, WizardOutcome,
};
use crate::config_templates::{InitSummary, RuntimeMode, SecretValue, TunnelSecretSource};
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
    pub(crate) list_edit: Option<ListEditTarget>,
    pub(crate) mcp_edit: Option<McpEditTarget>,
    pub(crate) max_active_custom: String,
    pub(crate) optional_center_focus: usize,
    #[allow(dead_code)]
    pub(crate) scroll: u16,
    #[allow(dead_code)]
    pub(crate) modal: Option<String>,
    pub(crate) cancelled: bool,
    pub(crate) committed_summary: Option<InitSummary>,
    pub(crate) finished: bool,
    pub(crate) system_error: Option<SystemError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ListEditTarget {
    pub(crate) field: SetupField,
    pub(crate) index: usize,
    pub(crate) created: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct McpEditTarget {
    pub(crate) field: SetupField,
    pub(crate) index: usize,
    pub(crate) created: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SystemError {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

pub(crate) trait Committer {
    fn commit(&mut self, config_path: &Path, outcome: WizardOutcome) -> Result<InitSummary>;
}

struct ProductionCommitter;

impl Committer for ProductionCommitter {
    fn commit(&mut self, config_path: &Path, outcome: WizardOutcome) -> Result<InitSummary> {
        commit_wizard_outcome(config_path, outcome)
    }
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
    AddListItem,
    RemoveListItem,
    #[allow(dead_code)]
    SetMode(RuntimeMode),
    #[allow(dead_code)]
    SetProfile(WorkerProfile),
}

pub(crate) struct ConfigTuiApp {
    session: Option<SetupSession>,
    navigation: Navigation,
    state: TuiState,
    theme: Theme,
    field_errors: HashMap<SetupField, String>,
    section_draft: Option<OptionalSectionDraft>,
    section_original: Option<OptionalSectionDraft>,
    review: Option<ReviewModel>,
    language: UiLanguage,
    committer: Box<dyn Committer>,
}

impl ConfigTuiApp {
    pub(crate) fn new(session: SetupSession) -> Self {
        Self::with_committer(session, Box::new(ProductionCommitter))
    }

    pub(crate) fn with_committer(session: SetupSession, committer: Box<dyn Committer>) -> Self {
        let navigation = Navigation::new(session.selected_mode());
        let page = navigation.current();
        let language = session.language();
        Self {
            session: Some(session),
            navigation,
            state: TuiState {
                page,
                return_target: ReturnTarget::MainFlow,
                focus: 0,
                editing: None,
                list_edit: None,
                mcp_edit: None,
                max_active_custom: "12".to_string(),
                optional_center_focus: 0,
                scroll: 0,
                modal: None,
                cancelled: false,
                committed_summary: None,
                finished: false,
                system_error: None,
            },
            theme: Theme::from_env(),
            field_errors: HashMap::new(),
            section_draft: None,
            section_original: None,
            review: None,
            language,
            committer,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn page(&self) -> ConfigPage {
        self.state.page
    }

    pub(crate) fn state(&self) -> &TuiState {
        &self.state
    }

    pub(crate) fn session(&self) -> &SetupSession {
        self.session
            .as_ref()
            .expect("setup session is unavailable after commit")
    }

    pub(crate) fn session_mut(&mut self) -> &mut SetupSession {
        self.session
            .as_mut()
            .expect("setup session is unavailable after commit")
    }

    #[allow(dead_code)]
    pub(crate) fn editing(&self) -> Option<&EditState> {
        self.state.editing.as_ref()
    }

    #[allow(dead_code)]
    pub(crate) fn review_model(&self) -> Option<&ReviewModel> {
        self.review.as_ref()
    }

    pub(crate) fn review_group_count(&self) -> usize {
        self.review
            .as_ref()
            .map(review_group_count)
            .unwrap_or_default()
    }

    pub(crate) fn take_committed_summary(&mut self) -> Option<InitSummary> {
        self.state.committed_summary.take()
    }

    pub(crate) fn focus_field(&mut self, field: SetupField) {
        self.state.focus = self.field_index(field).unwrap_or(self.state.focus);
    }

    pub(crate) fn focused_field(&self) -> Option<SetupField> {
        match self.state.page {
            ConfigPage::Basic => pages::basic_focus_field(self.state.focus),
            ConfigPage::Connection => {
                pages::connection_focus_field(self.session(), self.state.focus)
            }
            ConfigPage::Optional(section) => self
                .section_draft
                .as_ref()
                .and_then(|draft| pages::optional_focus_field(section, draft, self.state.focus)),
            _ => None,
        }
    }

    pub(crate) fn handle_event(&mut self, event: TerminalEvent) -> Result<()> {
        match event {
            TerminalEvent::Key(key) => self.handle_key(key)?,
            TerminalEvent::Resize | TerminalEvent::Tick => {}
        }
        Ok(())
    }

    pub(crate) fn handle_action(&mut self, action: TuiAction) -> Result<()> {
        match action {
            TuiAction::Next => {
                if self.state.editing.is_some() {
                    self.commit_edit();
                } else if matches!(self.state.page, ConfigPage::Optional(_)) {
                    self.save_optional_section();
                } else if matches!(
                    self.state.page,
                    ConfigPage::Completion | ConfigPage::SystemError
                ) {
                    self.state.finished = true;
                } else {
                    self.next_page();
                }
            }
            TuiAction::Back => {
                if self.state.editing.is_some() {
                    self.cancel_edit();
                    return Ok(());
                }
                if matches!(self.state.page, ConfigPage::Optional(_)) {
                    self.leave_optional_section();
                } else if self.state.return_target == ReturnTarget::Review
                    && matches!(self.state.page, ConfigPage::Basic | ConfigPage::Connection)
                {
                    self.return_to_review();
                } else if self.state.page == ConfigPage::Review {
                    // Review is the wizard root after the staged flow; Esc is a no-op.
                } else if matches!(
                    self.state.page,
                    ConfigPage::Completion | ConfigPage::SystemError
                ) {
                    self.state.finished = true;
                } else if self.navigation.back() {
                    self.sync_page();
                }
            }
            TuiAction::Cancel => {
                if self.state.page == ConfigPage::Completion {
                    self.state.finished = true;
                } else {
                    self.state.cancelled = true;
                    self.state.editing = None;
                    self.state.list_edit = None;
                    self.state.mcp_edit = None;
                    self.section_draft = None;
                    self.section_original = None;
                    self.review = None;
                }
            }
            TuiAction::Activate => {
                if self.state.editing.is_some() {
                    self.commit_edit();
                } else if self.state.page == ConfigPage::Review {
                    self.activate_review_focus();
                } else if matches!(
                    self.state.page,
                    ConfigPage::Completion | ConfigPage::SystemError
                ) {
                    self.state.finished = true;
                } else {
                    self.activate_focus();
                }
            }
            TuiAction::Text(character) => {
                let accepts_character =
                    self.state.editing.as_ref().is_none_or(|edit| {
                        !numeric_field(edit.field) || character.is_ascii_digit()
                    });
                if accepts_character {
                    self.with_edit(|edit| apply_text_key(edit, KeyCode::Char(character)));
                }
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
            TuiAction::AddListItem => self.add_optional_list_item(),
            TuiAction::RemoveListItem => self.remove_optional_list_item(),
            TuiAction::SetMode(mode) => {
                self.session_mut().set_mode(mode);
                self.navigation.set_mode(mode);
                self.state.focus = 0;
                self.sync_page();
            }
            TuiAction::SetProfile(profile) => self.session_mut().set_profile(profile),
        }
        Ok(())
    }

    pub(crate) fn render(&self, frame: &mut Frame) {
        match self.state.page {
            ConfigPage::Completion => pages::render_completion(
                frame,
                self.state.committed_summary.as_ref(),
                self.state.finished,
                self.language,
                &self.theme,
            ),
            ConfigPage::SystemError => pages::render_system_error(
                frame,
                self.state.system_error.as_ref(),
                self.language,
                &self.theme,
            ),
            _ => {
                let Some(session) = self.session.as_ref() else {
                    return;
                };
                pages::render(
                    frame,
                    self.state.page,
                    session,
                    &self.state,
                    self.language,
                    &self.theme,
                    &self.field_errors,
                    self.section_draft.as_ref(),
                    self.section_is_dirty(),
                    self.review.as_ref(),
                    self.navigation.progress(),
                );
            }
        }
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
                if matches!(
                    self.state.page,
                    ConfigPage::OptionalCenter | ConfigPage::Review
                ) || self.focused_field().is_some()
                {
                    self.handle_action(TuiAction::Activate)
                } else {
                    self.handle_action(TuiAction::Next)
                }
            }
            KeyCode::Tab => self.handle_action(TuiAction::MoveNext),
            KeyCode::BackTab => self.handle_action(TuiAction::MovePrevious),
            KeyCode::Down | KeyCode::Right => self.handle_action(TuiAction::MoveNext),
            KeyCode::Up | KeyCode::Left => self.handle_action(TuiAction::MovePrevious),
            KeyCode::Char('a') if matches!(self.state.page, ConfigPage::Optional(_)) => {
                self.handle_action(TuiAction::AddListItem)
            }
            KeyCode::Char('d') if matches!(self.state.page, ConfigPage::Optional(_)) => {
                self.handle_action(TuiAction::RemoveListItem)
            }
            KeyCode::Char('j') if self.state.page != ConfigPage::Review => {
                self.handle_action(TuiAction::MoveNext)
            }
            KeyCode::Char('k') if self.state.page != ConfigPage::Review => {
                self.handle_action(TuiAction::MovePrevious)
            }
            KeyCode::Char('l') if self.state.page != ConfigPage::Review => {
                if matches!(self.state.page, ConfigPage::OptionalCenter)
                    || self.focused_field().is_some()
                {
                    self.handle_action(TuiAction::Activate)
                } else {
                    self.handle_action(TuiAction::Next)
                }
            }
            KeyCode::Char('h') if self.state.page != ConfigPage::Review => {
                self.handle_action(TuiAction::Back)
            }
            _ => Ok(()),
        }
    }

    fn next_page(&mut self) {
        match self.state.page {
            ConfigPage::Basic => {
                if let Err(errors) = self.session().validate_basic() {
                    self.record_errors(errors);
                    return;
                }
                self.field_errors.clear();
                if self.state.return_target == ReturnTarget::Review {
                    self.return_to_review();
                } else if self.navigation.advance() {
                    self.state.focus = 0;
                    self.sync_page();
                }
            }
            ConfigPage::Connection => {
                if let Err(errors) = self.session().validate_connection() {
                    self.record_errors(errors);
                    return;
                }
                self.field_errors.clear();
                if self.state.return_target == ReturnTarget::Review {
                    self.return_to_review();
                } else if self.navigation.advance() {
                    self.state.focus = 0;
                    self.sync_page();
                }
            }
            ConfigPage::OptionalCenter => match self.session().review_model() {
                Ok(review) => {
                    self.review = Some(review);
                    self.field_errors.clear();
                    if self.navigation.advance() {
                        self.state.focus = 0;
                        self.sync_page();
                    }
                }
                Err(errors) => self.route_validation_errors(errors),
            },
            ConfigPage::Review if self.state.focus >= self.review_group_count() => {
                self.confirm_and_write();
            }
            ConfigPage::Review => {}
            _ => {}
        }
    }

    fn activate_focus(&mut self) {
        if self.state.page == ConfigPage::OptionalCenter {
            self.activate_optional_center();
            return;
        }
        if self.state.page == ConfigPage::Connection {
            if let Some(source) =
                pages::connection_secret_source_for_focus(self.session(), self.state.focus)
            {
                let draft = self.session_mut().standalone_mut();
                if source == TunnelSecretSource::Environment {
                    draft.provision_secret_now = false;
                    draft.secret_value = None;
                }
                draft.secret_source = source;
                self.field_errors.remove(&SetupField::TunnelSecretSource);
                return;
            }
        }
        if let ConfigPage::Optional(section) = self.state.page {
            if section == crate::config_templates::OptionalSection::McpServers {
                let target = self
                    .section_draft
                    .as_ref()
                    .and_then(|draft| pages::optional_mcp_target(section, draft, self.state.focus));
                match target {
                    Some(pages::McpFocusTarget::Add) => self.add_mcp_server(),
                    Some(pages::McpFocusTarget::Field { index, field })
                        if field == SetupField::McpServerEnabled =>
                    {
                        if let Some(draft) = self.section_draft.as_mut() {
                            pages::toggle_mcp_server_enabled(draft, index);
                        }
                        self.field_errors.remove(&field);
                    }
                    Some(pages::McpFocusTarget::Field { index, field })
                        if field == SetupField::McpServerTransport =>
                    {
                        if let Some(draft) = self.section_draft.as_mut() {
                            pages::toggle_mcp_server_transport(draft, index);
                        }
                        self.field_errors.remove(&field);
                    }
                    Some(pages::McpFocusTarget::Field { index, field }) => {
                        self.begin_mcp_edit(index, field, false);
                    }
                    None => {}
                }
                return;
            }
            if let Some((field, index)) = self.optional_list_target() {
                if let Some(index) = index {
                    self.begin_optional_list_edit(field, index);
                } else {
                    self.add_optional_list_item();
                }
                return;
            }
            let choice = self.section_draft.as_ref().and_then(|draft| {
                pages::optional_choice_for_focus(section, draft, self.state.focus)
            });
            if let Some((field, choice)) = choice {
                if field == SetupField::MaxActiveJobs && choice == "custom" {
                    let custom = self.state.max_active_custom.clone();
                    if let Some(draft) = self.section_draft.as_mut() {
                        pages::set_optional_field(draft, field, custom.clone());
                    }
                    self.state.editing = Some(EditState::new(field, custom));
                } else {
                    if let Some(draft) = self.section_draft.as_mut() {
                        pages::set_optional_field(draft, field, choice.to_string());
                    }
                    self.validate_section_draft();
                }
                return;
            }

            let Some(field) = self.focused_field() else {
                return;
            };
            if pages::optional_field_is_toggle(field) {
                if let Some(draft) = self.section_draft.as_mut() {
                    pages::toggle_optional_field(draft, field);
                    self.validate_section_draft();
                }
            } else if let Some(draft) = self.section_draft.as_ref() {
                let value = pages::optional_field_value(draft, field);
                self.state.editing = Some(EditState::new(field, value));
            }
            return;
        }
        let Some(field) = self.focused_field() else {
            return;
        };
        match field {
            SetupField::Mode => {
                if let Some(mode) = pages::basic_mode_for_focus(self.state.focus) {
                    self.session_mut().set_mode(mode);
                    self.navigation.set_mode(mode);
                    self.sync_page();
                }
            }
            SetupField::Profile => {
                if let Some(profile) = pages::basic_profile_for_focus(self.state.focus) {
                    self.session_mut().set_profile(profile);
                }
            }
            SetupField::ProvisionTunnelSecret => {
                let draft = self.session_mut().standalone_mut();
                draft.provision_secret_now = !draft.provision_secret_now;
            }
            _ => {
                let value = pages::connection_value(self.session(), field).unwrap_or_default();
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
        if let Some(target) = self.state.mcp_edit.take() {
            let updated = self.section_draft.as_mut().is_some_and(|draft| {
                pages::set_mcp_server_value(draft, target.index, target.field, value)
            });
            if updated {
                if target.created && target.field == SetupField::McpServerId {
                    self.field_errors.remove(&SetupField::McpServerId);
                } else {
                    self.validate_section_draft();
                }
            }
            return;
        }
        if let Some(target) = self.state.list_edit.take() {
            let updated = self.section_draft.as_mut().is_some_and(|draft| {
                let Ok(mut list) = pages::optional_list_state(draft, target.field) else {
                    return false;
                };
                list.set_focus(target.index);
                if !list.set_focused(value) {
                    return false;
                }
                pages::set_optional_list_state(draft, target.field, &list)
            });
            if updated {
                self.validate_section_draft();
            }
            return;
        }
        if matches!(self.state.page, ConfigPage::Optional(_)) {
            if field == SetupField::MaxActiveJobs && value.parse::<usize>().is_ok() {
                self.state.max_active_custom = value.clone();
            }
            if let Some(draft) = self.section_draft.as_mut() {
                pages::set_optional_field(draft, field, value);
            }
            self.validate_section_draft();
            return;
        }
        match field {
            SetupField::TunnelId => self.session_mut().standalone_mut().tunnel_id = value,
            SetupField::TunnelSecretPath => self.session_mut().standalone_mut().secret_path = value,
            SetupField::TunnelSecretEnvironment => {
                self.session_mut().standalone_mut().secret_environment = value
            }
            SetupField::TunnelSecretValue => {
                self.session_mut().standalone_mut().secret_value = Some(SecretValue::new(value))
            }
            SetupField::HubUrl => self.session_mut().hub_mut().hub_url = value,
            SetupField::HubTransport => self.session_mut().hub_mut().hub_transport = value,
            SetupField::AgentId => self.session_mut().hub_mut().agent_id = value,
            SetupField::AgentSecret => {
                self.session_mut().hub_mut().agent_secret = Some(SecretValue::new(value))
            }
            _ => {}
        }
        match self.session().validate_field(field) {
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

    fn current_optional_section(&self) -> Option<crate::config_templates::OptionalSection> {
        match self.state.page {
            ConfigPage::Optional(section) => Some(section),
            _ => None,
        }
    }

    fn optional_list_target(&self) -> Option<(SetupField, Option<usize>)> {
        let section = self.current_optional_section()?;
        self.section_draft
            .as_ref()
            .and_then(|draft| pages::optional_list_target(section, draft, self.state.focus))
    }

    fn begin_mcp_edit(&mut self, index: usize, field: SetupField, created: bool) {
        let value = self
            .section_draft
            .as_ref()
            .and_then(|draft| pages::mcp_server_value(draft, index, field));
        let Some(value) = value else {
            return;
        };
        self.state.mcp_edit = Some(McpEditTarget {
            field,
            index,
            created,
        });
        self.state.editing = Some(EditState::new(field, value));
    }

    fn add_mcp_server(&mut self) {
        let after = self
            .section_draft
            .as_ref()
            .and_then(|draft| pages::mcp_server_index_for_focus(draft, self.state.focus));
        let new_index = self
            .section_draft
            .as_mut()
            .and_then(|draft| pages::add_mcp_server(draft, after));
        let Some(new_index) = new_index else {
            return;
        };
        if let Some(draft) = self.section_draft.as_ref() {
            self.state.focus =
                pages::mcp_server_focus_index(draft, new_index, SetupField::McpServerId)
                    .unwrap_or(self.state.focus);
        }
        self.begin_mcp_edit(new_index, SetupField::McpServerId, true);
    }

    fn remove_mcp_server(&mut self) {
        let Some(index) = self
            .section_draft
            .as_ref()
            .and_then(|draft| pages::mcp_server_index_for_focus(draft, self.state.focus))
        else {
            return;
        };
        let removed = self
            .section_draft
            .as_mut()
            .is_some_and(|draft| pages::remove_mcp_server(draft, index));
        if !removed {
            return;
        }
        if let Some(draft) = self.section_draft.as_ref() {
            let count = pages::mcp_server_count(draft);
            self.state.focus = if count == 0 {
                0
            } else {
                pages::mcp_server_focus_index(
                    draft,
                    index.min(count.saturating_sub(1)),
                    SetupField::McpServerId,
                )
                .unwrap_or(0)
            };
        }
        self.field_errors.clear();
    }

    fn begin_optional_list_edit(&mut self, field: SetupField, index: usize) {
        let value = self.section_draft.as_ref().and_then(|draft| {
            let mut list = pages::optional_list_state(draft, field).ok()?;
            list.set_focus(index);
            list.focused().map(str::to_string)
        });
        let Some(value) = value else {
            self.validate_section_draft();
            return;
        };
        self.state.list_edit = Some(ListEditTarget {
            field,
            index,
            created: false,
        });
        self.state.editing = Some(EditState::new(field, value));
    }

    fn add_optional_list_item(&mut self) {
        let Some(section) = self.current_optional_section() else {
            return;
        };
        if section == crate::config_templates::OptionalSection::McpServers {
            self.add_mcp_server();
            return;
        }
        let Some((field, current_index)) = self.optional_list_target() else {
            return;
        };
        let new_index = self.section_draft.as_mut().and_then(|draft| {
            let mut list = pages::optional_list_state(draft, field).ok()?;
            if let Some(index) = current_index {
                list.set_focus(index);
            }
            let new_index = list.add_after_focused();
            pages::set_optional_list_state(draft, field, &list).then_some(new_index)
        });
        let Some(new_index) = new_index else {
            self.validate_section_draft();
            return;
        };
        if let Some(draft) = self.section_draft.as_ref() {
            self.state.focus = pages::optional_item_focus_index(section, draft, field, new_index)
                .unwrap_or(self.state.focus);
        }
        self.state.list_edit = Some(ListEditTarget {
            field,
            index: new_index,
            created: true,
        });
        self.state.editing = Some(EditState::new(field, ""));
    }

    fn remove_optional_list_item(&mut self) {
        let Some(section) = self.current_optional_section() else {
            return;
        };
        if section == crate::config_templates::OptionalSection::McpServers {
            self.remove_mcp_server();
            return;
        }
        let Some((field, Some(index))) = self.optional_list_target() else {
            return;
        };
        let next_item = self.section_draft.as_mut().and_then(|draft| {
            let mut list = pages::optional_list_state(draft, field).ok()?;
            list.set_focus(index);
            list.delete_focused()?;
            let next_item = (!list.is_empty()).then_some(list.focus());
            pages::set_optional_list_state(draft, field, &list).then_some(next_item)
        });
        let Some(next_item) = next_item else {
            self.validate_section_draft();
            return;
        };
        if let Some(draft) = self.section_draft.as_ref() {
            self.state.focus = next_item
                .and_then(|index| pages::optional_item_focus_index(section, draft, field, index))
                .or_else(|| pages::optional_field_index(section, draft, field))
                .unwrap_or(self.state.focus);
        }
        self.validate_section_draft();
    }

    fn cancel_edit(&mut self) {
        self.state.editing = None;
        if let Some(target) = self.state.mcp_edit.take() {
            if target.created {
                if let Some(draft) = self.section_draft.as_mut() {
                    pages::remove_mcp_server(draft, target.index);
                }
                if let Some(draft) = self.section_draft.as_ref() {
                    let count = pages::mcp_server_count(draft);
                    self.state.focus = if count == 0 {
                        0
                    } else {
                        pages::mcp_server_focus_index(
                            draft,
                            target.index.min(count.saturating_sub(1)),
                            SetupField::McpServerId,
                        )
                        .unwrap_or(0)
                    };
                }
                self.field_errors.clear();
            }
            return;
        }
        let Some(target) = self.state.list_edit.take() else {
            return;
        };
        if !target.created {
            return;
        }
        let Some(section) = self.current_optional_section() else {
            return;
        };
        let next_item = self.section_draft.as_mut().and_then(|draft| {
            let mut list = pages::optional_list_state(draft, target.field).ok()?;
            list.set_focus(target.index);
            list.delete_focused()?;
            let next_item = (!list.is_empty()).then_some(list.focus());
            pages::set_optional_list_state(draft, target.field, &list).then_some(next_item)
        });
        if let (Some(next_item), Some(draft)) = (next_item, self.section_draft.as_ref()) {
            self.state.focus = next_item
                .and_then(|index| {
                    pages::optional_item_focus_index(section, draft, target.field, index)
                })
                .or_else(|| pages::optional_field_index(section, draft, target.field))
                .unwrap_or(self.state.focus);
        }
        self.validate_section_draft();
    }

    fn move_focus(&mut self, direction: isize) {
        let length = match self.state.page {
            ConfigPage::Basic => pages::basic_focus_len(),
            ConfigPage::Connection => pages::connection_focus_len(self.session()),
            ConfigPage::OptionalCenter => self.session().available_optional_sections().len() + 1,
            ConfigPage::Optional(section) => self
                .section_draft
                .as_ref()
                .map(|draft| pages::optional_focus_len(section, draft))
                .unwrap_or(1),
            ConfigPage::Review => self.review_group_count() + 1,
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
            ConfigPage::Basic => match field {
                SetupField::Mode => Some(0),
                SetupField::Profile => Some(3),
                _ => None,
            },
            ConfigPage::Connection => pages::connection_field_index(self.session(), field),
            ConfigPage::Optional(section) => self
                .section_draft
                .as_ref()
                .and_then(|draft| pages::optional_field_index(section, draft, field)),
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

    fn activate_optional_center(&mut self) {
        let sections = self.session().available_optional_sections();
        if let Some(section) = sections.get(self.state.focus).copied() {
            self.state.optional_center_focus = self.state.focus;
            let draft = self.session().optional_draft(section);
            self.sync_optional_ui_state(&draft);
            self.section_original = Some(draft.clone());
            self.section_draft = Some(draft);
            self.field_errors.clear();
            self.state.editing = None;
            self.state.list_edit = None;
            self.state.mcp_edit = None;
            self.state.page = ConfigPage::Optional(section);
            self.state.return_target = ReturnTarget::MainFlow;
            self.state.focus = 0;
        } else {
            self.next_page();
        }
    }

    fn sync_optional_ui_state(&mut self, draft: &OptionalSectionDraft) {
        if let OptionalSectionDraft::Limits(limits) = draft {
            let custom = limits.max_active_jobs.trim();
            if custom != "auto" && custom.parse::<usize>().is_ok() {
                self.state.max_active_custom = custom.to_string();
            }
        }
    }

    fn leave_optional_section(&mut self) {
        let return_target = self.state.return_target;
        let optional_center_focus = self.state.optional_center_focus;
        self.section_draft = None;
        self.section_original = None;
        self.field_errors.clear();
        self.state.editing = None;
        self.state.list_edit = None;
        self.state.mcp_edit = None;
        self.state.focus = 0;
        if return_target == ReturnTarget::Review {
            self.return_to_review();
        } else {
            self.state.return_target = ReturnTarget::MainFlow;
            self.sync_page();
            if self.state.page == ConfigPage::OptionalCenter {
                let max_focus = self.session().available_optional_sections().len();
                self.state.focus = optional_center_focus.min(max_focus);
            }
        }
    }

    fn section_is_dirty(&self) -> bool {
        self.section_draft != self.section_original
    }

    fn save_optional_section(&mut self) {
        let Some(draft) = self.section_draft.as_ref().cloned() else {
            self.leave_optional_section();
            return;
        };
        if !self.section_is_dirty() {
            self.leave_optional_section();
            return;
        }
        if let Err(errors) = self.session().validate_optional_draft(&draft) {
            self.record_errors(errors);
            return;
        }
        if let Err(errors) = self.session_mut().save_optional_section(draft) {
            self.record_errors(errors);
            return;
        }
        self.leave_optional_section();
    }

    fn validate_section_draft(&mut self) {
        let Some(draft) = self.section_draft.as_ref() else {
            return;
        };
        match self.session().validate_optional_draft(draft) {
            Ok(()) => self.field_errors.clear(),
            Err(errors) => self.record_errors(errors),
        }
    }

    fn activate_review_focus(&mut self) {
        let Some(review) = self.review.as_ref() else {
            self.refresh_review();
            return;
        };
        let targets = review_targets(review);
        if let Some(target) = targets.get(self.state.focus).copied() {
            self.open_review_target(target);
        } else {
            self.confirm_and_write();
        }
    }

    fn open_review_target(&mut self, target: ReturnTargetKind) {
        self.state.return_target = ReturnTarget::Review;
        self.state.editing = None;
        self.field_errors.clear();
        match target {
            ReturnTargetKind::Basic => {
                self.navigation.go_to(ConfigPage::Basic);
                self.state.page = ConfigPage::Basic;
                self.state.focus = 0;
            }
            ReturnTargetKind::Connection => {
                if self.navigation.go_to(ConfigPage::Connection) {
                    self.state.page = ConfigPage::Connection;
                    self.state.focus = 0;
                }
            }
            ReturnTargetKind::Optional(section) => {
                let draft = self.session().optional_draft(section);
                self.sync_optional_ui_state(&draft);
                self.section_original = Some(draft.clone());
                self.section_draft = Some(draft);
                self.state.list_edit = None;
                self.state.mcp_edit = None;
                self.state.page = ConfigPage::Optional(section);
                self.state.focus = 0;
            }
        }
    }

    fn return_to_review(&mut self) {
        self.section_draft = None;
        self.section_original = None;
        self.state.editing = None;
        self.state.list_edit = None;
        self.state.mcp_edit = None;
        self.field_errors.clear();
        self.state.return_target = ReturnTarget::MainFlow;
        self.navigation.go_to(ConfigPage::Review);
        self.state.focus = 0;
        self.refresh_review();
    }

    fn refresh_review(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        match session.review_model() {
            Ok(review) => {
                self.review = Some(review);
                self.state.page = ConfigPage::Review;
            }
            Err(errors) => self.route_validation_errors(errors),
        }
    }

    fn route_validation_errors(&mut self, errors: ValidationErrors) {
        let field = errors.first().map(|error| error.field);
        self.record_errors(errors);
        let Some(field) = field else {
            return;
        };
        match field {
            SetupField::Mode | SetupField::Profile => {
                self.navigation.go_to(ConfigPage::Basic);
                self.state.page = ConfigPage::Basic;
                self.state.focus = self.field_index(field).unwrap_or(0);
            }
            SetupField::TunnelId
            | SetupField::TunnelSecretSource
            | SetupField::TunnelSecretPath
            | SetupField::TunnelSecretEnvironment
            | SetupField::ProvisionTunnelSecret
            | SetupField::TunnelSecretValue
            | SetupField::HubUrl
            | SetupField::HubTransport
            | SetupField::AgentId
            | SetupField::AgentSecret => {
                if self.navigation.go_to(ConfigPage::Connection) {
                    self.state.page = ConfigPage::Connection;
                    self.state.focus = self.field_index(field).unwrap_or(0);
                }
            }
            _ => {
                let section = optional_section_for_field(field);
                let draft = self.session().optional_draft(section);
                self.section_original = Some(draft.clone());
                self.section_draft = Some(draft);
                self.state.page = ConfigPage::Optional(section);
                self.state.return_target = ReturnTarget::MainFlow;
                self.state.focus = self.field_index(field).unwrap_or(0);
            }
        }
    }

    fn confirm_and_write(&mut self) {
        if self.state.page != ConfigPage::Review || self.state.finished {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let config_path = session.config_path().to_path_buf();
        if let Err(errors) = session.validate_for_review() {
            self.route_validation_errors(errors);
            return;
        }
        let session = self.session.take().expect("session was checked above");
        let outcome = match session.into_wizard_outcome() {
            Ok(outcome) => outcome,
            Err(errors) => {
                self.route_validation_errors_after_session_loss(errors);
                return;
            }
        };
        match self.committer.commit(&config_path, outcome) {
            Ok(summary) => {
                self.state.committed_summary = Some(summary);
                self.state.page = ConfigPage::Completion;
                self.state.focus = 0;
                self.state.return_target = ReturnTarget::MainFlow;
                self.review = None;
            }
            Err(error) => self.set_system_error(&error),
        }
    }

    fn route_validation_errors_after_session_loss(&mut self, errors: ValidationErrors) {
        self.field_errors.clear();
        if let Some(error) = errors.first() {
            self.field_errors
                .insert(error.field, error.code.to_string());
        }
        self.set_system_error_code("config_init_build_invalid");
    }

    fn set_system_error(&mut self, error: &anyhow::Error) {
        let error_text = error.to_string();
        let code = if error_text.contains("config_init_secret_rollback_failed") {
            "config_init_secret_rollback_failed"
        } else {
            error_text.split(':').next().unwrap_or_default()
        };
        let code = match code {
            "config_init_secret_path_invalid" => "config_init_secret_path_invalid",
            "config_init_secret_parent_invalid" => "config_init_secret_parent_invalid",
            "config_init_secret_write_failed" => "config_init_secret_write_failed",
            "config_init_config_write_failed" => "config_init_config_write_failed",
            "config_init_secret_rollback_failed" => "config_init_secret_rollback_failed",
            _ => "config_init_system_error",
        };
        self.set_system_error_code(code);
    }

    pub(crate) fn set_runtime_error(&mut self) {
        self.set_system_error_code("config_init_terminal_error");
    }

    pub(crate) fn set_system_error_code(&mut self, code: &'static str) {
        self.state.system_error = Some(SystemError {
            code,
            message: system_error_message(code, self.language),
        });
        self.state.page = ConfigPage::SystemError;
        self.state.focus = 0;
        self.state.editing = None;
        self.state.list_edit = None;
        self.state.mcp_edit = None;
        self.section_draft = None;
        self.section_original = None;
    }
}

fn numeric_field(field: SetupField) -> bool {
    matches!(
        field,
        SetupField::MaxConcurrentTasks
            | SetupField::MaxActiveJobs
            | SetupField::MaxFileSearchContextLines
            | SetupField::DiaryBoundaryHour
    )
}

#[derive(Clone, Copy)]
enum ReturnTargetKind {
    Basic,
    Connection,
    Optional(crate::config_templates::OptionalSection),
}

fn review_group_count(review: &ReviewModel) -> usize {
    review_targets(review).len()
}

fn review_targets(review: &ReviewModel) -> Vec<ReturnTargetKind> {
    let mut targets = vec![ReturnTargetKind::Basic];
    if review.mode != RuntimeMode::Local {
        targets.push(ReturnTargetKind::Connection);
    }
    targets.extend(
        review
            .optional_sections
            .iter()
            .filter_map(|group| match group.target {
                crate::config_setup::ReviewTarget::OptionalSection(section)
                    if group.status != crate::config_setup::SectionStatus::NotApplicable =>
                {
                    Some(ReturnTargetKind::Optional(section))
                }
                _ => None,
            }),
    );
    targets
}

fn optional_section_for_field(field: SetupField) -> crate::config_templates::OptionalSection {
    match field {
        SetupField::DisplayName => crate::config_templates::OptionalSection::Identity,
        SetupField::WorkspaceRoot
        | SetupField::WriteRoots
        | SetupField::ReadOnlyRoots
        | SetupField::DenyRoots => crate::config_templates::OptionalSection::Workspace,
        SetupField::ConfirmationProvider | SetupField::ConfirmationLanguage => {
            crate::config_templates::OptionalSection::Confirmation
        }
        SetupField::MaxConcurrentTasks
        | SetupField::MaxActiveJobs
        | SetupField::MaxFileSearchContextLines => crate::config_templates::OptionalSection::Limits,
        SetupField::SandboxEnabled
        | SetupField::BubblewrapPath
        | SetupField::RequiredRuntimePaths => crate::config_templates::OptionalSection::Sandbox,
        SetupField::RoomTimezone | SetupField::DiaryBoundaryHour | SetupField::NotebookRoot => {
            crate::config_templates::OptionalSection::Room
        }
        SetupField::TunnelClientVersion
        | SetupField::TunnelCacheDir
        | SetupField::TunnelAutoDownload
        | SetupField::TunnelExecutable
        | SetupField::TunnelDownloadUrl
        | SetupField::TunnelSha256 => crate::config_templates::OptionalSection::TunnelClient,
        SetupField::HubReportingEnabled | SetupField::HubReportingDetail => {
            crate::config_templates::OptionalSection::HubReporting
        }
        _ => crate::config_templates::OptionalSection::Identity,
    }
}

fn system_error_message(code: &'static str, language: UiLanguage) -> &'static str {
    match (code, language) {
        ("config_init_secret_path_invalid", UiLanguage::ZhCn) => "密钥路径无效，未写入配置。",
        ("config_init_secret_parent_invalid", UiLanguage::ZhCn) => "密钥目录不可用，未写入配置。",
        ("config_init_secret_write_failed", UiLanguage::ZhCn) => "密钥写入失败，未完成初始化。",
        ("config_init_config_write_failed", UiLanguage::ZhCn) => "配置写入失败，未完成初始化。",
        ("config_init_secret_rollback_failed", UiLanguage::ZhCn) => {
            "回滚失败，请检查配置与密钥文件。"
        }
        ("config_init_terminal_error", UiLanguage::ZhCn) => {
            "终端初始化或刷新失败，请重试配置初始化。"
        }
        ("config_init_secret_path_invalid", UiLanguage::En) => {
            "The secret path is invalid; configuration was not written."
        }
        ("config_init_secret_parent_invalid", UiLanguage::En) => {
            "The secret directory is unavailable; configuration was not written."
        }
        ("config_init_secret_write_failed", UiLanguage::En) => {
            "The secret write failed; initialization did not complete."
        }
        ("config_init_config_write_failed", UiLanguage::En) => {
            "The configuration write failed; initialization did not complete."
        }
        ("config_init_secret_rollback_failed", UiLanguage::En) => {
            "Rollback failed; inspect the configuration and secret files."
        }
        ("config_init_terminal_error", UiLanguage::En) => {
            "Terminal setup or refresh failed; please retry configuration initialization."
        }
        (_, UiLanguage::ZhCn) => "初始化失败，未写入配置。",
        (_, UiLanguage::En) => "Initialization failed; configuration was not written.",
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use anyhow::anyhow;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};

    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession, WizardOutcome};
    use crate::config_templates::{InitSummary, OptionalSection, RuntimeMode, SecretValue};
    use crate::tui::TerminalEvent;
    use crate::WorkerProfile;

    use super::super::{Committer, ConfigPage, ConfigTuiApp, TuiAction};

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

    fn optional_center_app() -> ConfigTuiApp {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.page(), ConfigPage::OptionalCenter);
        app
    }

    fn focus_optional_section(app: &mut ConfigTuiApp, section: OptionalSection) {
        app.state.focus = app
            .session()
            .available_optional_sections()
            .iter()
            .position(|candidate| *candidate == section)
            .expect("optional section is available");
    }

    fn review_app(mode: RuntimeMode) -> ConfigTuiApp {
        let mut app = app(mode);
        while app.page() != ConfigPage::Review {
            app.handle_action(TuiAction::Next).unwrap();
        }
        app
    }

    fn rendered(app: &ConfigTuiApp) -> String {
        let mut terminal = Terminal::new(TestBackend::new(90, 28)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    struct CountingCommitter {
        calls: Rc<Cell<usize>>,
        failure: Option<String>,
    }

    impl Committer for CountingCommitter {
        fn commit(
            &mut self,
            config_path: &std::path::Path,
            outcome: WizardOutcome,
        ) -> anyhow::Result<InitSummary> {
            self.calls.set(self.calls.get() + 1);
            if let Some(message) = &self.failure {
                return Err(anyhow!(message.clone()));
            }
            let build = outcome.build;
            Ok(InitSummary {
                mode: build.mode,
                profile: build.profile,
                config_path: config_path.to_path_buf(),
                pending: build.pending,
            })
        }
    }

    #[test]
    fn cancelling_new_mcp_server_id_discards_the_created_server() {
        let mut app = optional_center_app();
        for _ in 0..5 {
            app.handle_action(TuiAction::MoveNext).unwrap();
        }
        app.handle_action(TuiAction::Activate).unwrap();
        app.handle_action(TuiAction::AddListItem).unwrap();
        assert!(app.editing().is_some());

        app.handle_action(TuiAction::Back).unwrap();

        assert!(app.editing().is_none());
        let draft = app.section_draft.as_ref().unwrap();
        assert!(matches!(
            draft,
            crate::config_setup::OptionalSectionDraft::McpServers(value) if value.servers.is_empty()
        ));
    }

    #[test]
    fn mcp_servers_edit_as_compound_items_and_flow_into_config() {
        let mut app = optional_center_app();
        for _ in 0..5 {
            app.handle_action(TuiAction::MoveNext).unwrap();
        }
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::McpServers)
        );

        app.handle_action(TuiAction::AddListItem).unwrap();
        assert_eq!(
            app.editing().unwrap().field,
            crate::config_setup::SetupField::McpServerId
        );
        for character in "local_tools".chars() {
            app.handle_action(TuiAction::Text(character)).unwrap();
        }
        app.handle_action(TuiAction::Activate).unwrap();

        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        for character in "node ./server.mjs".chars() {
            app.handle_action(TuiAction::Text(character)).unwrap();
        }
        app.handle_action(TuiAction::Activate).unwrap();

        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.page(), ConfigPage::OptionalCenter);

        let input = app.session().build_active_input().unwrap();
        let servers = input.mcp_servers.unwrap();
        let server = servers.get("local_tools").unwrap();
        assert!(!server.enabled);
        assert_eq!(server.transport, "stdio");
        assert_eq!(server.url.as_deref(), Some("node ./server.mjs"));
    }

    #[test]
    fn optional_center_skips_not_applicable_rows_and_reenters_saved_sections() {
        let mut app = optional_center_app();
        focus_optional_section(&mut app, OptionalSection::TunnelClient);
        let tunnel_focus = app.state().focus;
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::TunnelClient)
        );
        assert!(rendered(&app).contains("Return"));
        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.page(), ConfigPage::OptionalCenter);
        assert_eq!(app.state().focus, tunnel_focus);
        assert!(app.session().optional_drafts().tunnel_client.is_none());
        assert_eq!(
            app.session()
                .section_status(crate::config_templates::OptionalSection::TunnelClient),
            crate::config_setup::SectionStatus::Default
        );

        app.state.focus = 0;
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::Identity)
        );
        app.focus_field(crate::config_setup::SetupField::DisplayName);
        app.handle_action(TuiAction::Activate).unwrap();
        app.handle_action(TuiAction::Text('!')).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        assert!(rendered(&app).contains("Save and return"));
        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.page(), ConfigPage::OptionalCenter);
        assert_eq!(
            app.session()
                .optional_drafts()
                .identity
                .as_ref()
                .unwrap()
                .display_name,
            "AgenticGPT agent!"
        );

        app.state.focus = 0;
        app.handle_action(TuiAction::Activate).unwrap();
        app.focus_field(crate::config_setup::SetupField::DisplayName);
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(app.editing().unwrap().buffer, "AgenticGPT agent!");
    }

    #[test]
    fn confirmation_choices_move_focus_without_committing_until_activation() {
        let mut app = optional_center_app();
        app.state.focus = 2;
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::Confirmation)
        );
        let provider = |app: &ConfigTuiApp| match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::Confirmation(draft) => {
                draft.provider.clone()
            }
            _ => unreachable!(),
        };
        assert_eq!(provider(&app), "default");
        app.handle_action(TuiAction::MoveNext).unwrap();
        assert_eq!(provider(&app), "default");
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(provider(&app), "freedesktop");
        let rendered = rendered(&app);
        assert!(rendered.contains("Provider"));
        assert!(rendered.contains("freedesktop"));
        assert!(rendered.contains("ntfy"));
    }

    #[test]
    fn limits_auto_custom_matches_demo_and_numeric_edit_rejects_letters() {
        let mut app = optional_center_app();
        app.state.focus = 3;
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::Limits)
        );
        let max_active = |app: &ConfigTuiApp| match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::Limits(draft) => {
                draft.max_active_jobs.clone()
            }
            _ => unreachable!(),
        };
        assert_eq!(max_active(&app), "auto");

        app.focus_field(crate::config_setup::SetupField::MaxActiveJobs);
        app.handle_action(TuiAction::MoveNext).unwrap();
        assert_eq!(max_active(&app), "auto");
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(max_active(&app), "12");
        assert_eq!(app.editing().unwrap().buffer, "12");
        app.handle_action(TuiAction::Backspace).unwrap();
        app.handle_action(TuiAction::Backspace).unwrap();
        app.handle_action(TuiAction::Text('x')).unwrap();
        assert_eq!(app.editing().unwrap().buffer, "");
        app.handle_action(TuiAction::Text('7')).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(max_active(&app), "7");
        assert_eq!(app.state.max_active_custom, "7");

        app.handle_action(TuiAction::MovePrevious).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(max_active(&app), "auto");
        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(app.editing().unwrap().buffer, "7");
    }

    #[test]
    fn sandbox_boolean_and_runtime_paths_use_form_semantics_without_json_ui() {
        let mut app = optional_center_app();
        app.state.focus = 4;
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::Sandbox)
        );
        app.focus_field(crate::config_setup::SetupField::SandboxEnabled);
        app.handle_action(TuiAction::Activate).unwrap();
        let enabled = match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::Sandbox(draft) => draft.enabled,
            _ => unreachable!(),
        };
        assert!(enabled);

        app.focus_field(crate::config_setup::SetupField::RequiredRuntimePaths);
        app.handle_action(TuiAction::AddListItem).unwrap();
        for character in "/custom/runtime".chars() {
            app.handle_action(TuiAction::Text(character)).unwrap();
        }
        app.handle_action(TuiAction::Activate).unwrap();
        let paths = match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::Sandbox(draft) => {
                serde_json::from_str::<Vec<String>>(&draft.required_runtime_paths).unwrap()
            }
            _ => unreachable!(),
        };
        assert!(paths.iter().any(|path| path == "/custom/runtime"));
        let rendered = rendered(&app);
        assert!(rendered.contains("Required runtime paths"));
        assert!(!rendered.contains("(JSON)"));
        assert!(!rendered.contains(''));
    }

    #[test]
    fn tunnel_client_uses_real_defaults_and_empty_inputs_without_placeholders() {
        let mut app = optional_center_app();
        focus_optional_section(&mut app, OptionalSection::TunnelClient);
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::TunnelClient)
        );
        let rendered = rendered(&app);
        assert!(rendered.contains("Client version"));
        assert!(rendered.contains("~/.agentic_gpt/cache/tunnel-client"));
        assert!(rendered.contains("Auto-download"));
        assert!(rendered.contains("on"));
        assert!(rendered.contains("[        ]"));
        assert!(!rendered.contains('•'));
        assert!(!rendered.contains(''));
    }

    #[test]
    fn hub_reporting_detail_is_a_committed_choice_not_a_cycle_row() {
        let mut app = optional_center_app();
        focus_optional_section(&mut app, OptionalSection::HubReporting);
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::HubReporting)
        );
        let detail = |app: &ConfigTuiApp| match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::HubReporting(draft) => draft.detail.clone(),
            _ => unreachable!(),
        };
        assert_eq!(detail(&app), "metadata");
        app.focus_field(crate::config_setup::SetupField::HubReportingDetail);
        app.handle_action(TuiAction::MoveNext).unwrap();
        assert_eq!(detail(&app), "metadata");
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(detail(&app), "full");
        let rendered = rendered(&app);
        assert!(rendered.contains("metadata"));
        assert!(rendered.contains("full"));
        assert_eq!(rendered.matches('').count(), 1);
    }

    #[test]
    fn workspace_list_edits_stay_staged_until_section_save() {
        let mut app = optional_center_app();
        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::Workspace)
        );
        app.focus_field(crate::config_setup::SetupField::WriteRoots);
        app.handle_action(TuiAction::Activate).unwrap();
        let original = app.editing().unwrap().buffer.clone();
        app.handle_action(TuiAction::Text('x')).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        assert!(app.session().optional_drafts().workspace.is_none());
        let staged = match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::Workspace(draft) => {
                serde_json::from_str::<Vec<String>>(&draft.write_roots).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!(staged[0], format!("{original}x"));

        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.page(), ConfigPage::OptionalCenter);
        let saved = app.session().optional_drafts().workspace.as_ref().unwrap();
        let saved_roots = serde_json::from_str::<Vec<String>>(&saved.write_roots).unwrap();
        assert_eq!(saved_roots[0], format!("{original}x"));
    }

    #[test]
    fn workspace_list_add_delete_and_cancel_are_item_scoped() {
        let mut app = optional_center_app();
        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        app.focus_field(crate::config_setup::SetupField::WriteRoots);

        let list_len = |app: &ConfigTuiApp| match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::Workspace(draft) => {
                serde_json::from_str::<Vec<String>>(&draft.write_roots)
                    .unwrap()
                    .len()
            }
            _ => unreachable!(),
        };
        let original_len = list_len(&app);

        app.handle_action(TuiAction::AddListItem).unwrap();
        assert_eq!(list_len(&app), original_len + 1);
        assert!(app.editing().is_some());
        for character in "added/path".chars() {
            app.handle_action(TuiAction::Text(character)).unwrap();
        }
        app.handle_action(TuiAction::Activate).unwrap();
        assert!(app.editing().is_none());
        assert_eq!(list_len(&app), original_len + 1);

        app.handle_action(TuiAction::RemoveListItem).unwrap();
        assert_eq!(list_len(&app), original_len);

        app.handle_action(TuiAction::AddListItem).unwrap();
        assert_eq!(list_len(&app), original_len + 1);
        app.handle_action(TuiAction::Back).unwrap();
        assert!(app.editing().is_none());
        assert_eq!(list_len(&app), original_len);
    }

    #[test]
    fn malformed_workspace_list_surfaces_validation_without_overwrite() {
        let mut app = optional_center_app();
        app.handle_action(TuiAction::MoveNext).unwrap();
        app.handle_action(TuiAction::Activate).unwrap();
        if let Some(crate::config_setup::OptionalSectionDraft::Workspace(draft)) =
            app.section_draft.as_mut()
        {
            draft.write_roots = "not-json".to_string();
        }
        app.focus_field(crate::config_setup::SetupField::WriteRoots);
        app.handle_action(TuiAction::AddListItem).unwrap();
        assert!(app.editing().is_none());
        assert_eq!(
            app.field_errors
                .get(&crate::config_setup::SetupField::WriteRoots)
                .map(String::as_str),
            Some("config_init_path_policy_write_roots_invalid")
        );
        let raw = match app.section_draft.as_ref().unwrap() {
            crate::config_setup::OptionalSectionDraft::Workspace(draft) => &draft.write_roots,
            _ => unreachable!(),
        };
        assert_eq!(raw, "not-json");
    }

    #[test]
    fn review_return_targets_rebuild_the_review_model() {
        let mut basic = review_app(RuntimeMode::Standalone);
        basic.state.focus = 0;
        basic.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(basic.page(), ConfigPage::Basic);
        assert_eq!(
            basic.state.return_target,
            super::super::navigation::ReturnTarget::Review
        );
        basic
            .handle_action(TuiAction::SetMode(RuntimeMode::Hub))
            .unwrap();
        basic.handle_action(TuiAction::Next).unwrap();
        assert_eq!(basic.page(), ConfigPage::Review);
        assert_eq!(basic.review_model().unwrap().mode, RuntimeMode::Hub);
        assert!(basic
            .review_model()
            .unwrap()
            .connection
            .items
            .iter()
            .any(|item| item.label_key == "hub_url"));

        let mut connection = review_app(RuntimeMode::Standalone);
        connection.state.focus = 1;
        connection.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(connection.page(), ConfigPage::Connection);
        connection.handle_action(TuiAction::Next).unwrap();
        assert_eq!(connection.page(), ConfigPage::Review);

        let mut optional = review_app(RuntimeMode::Standalone);
        optional.state.focus = 2;
        optional.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            optional.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::Identity)
        );
        optional.handle_action(TuiAction::Next).unwrap();
        assert_eq!(optional.page(), ConfigPage::Review);

        let mut escape = review_app(RuntimeMode::Standalone);
        escape.state.focus = 0;
        escape.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(escape.page(), ConfigPage::Basic);
        escape.handle_action(TuiAction::Back).unwrap();
        assert_eq!(escape.page(), ConfigPage::Review);
    }

    #[test]
    fn review_cancel_has_no_side_effect_and_confirm_commits_once() {
        let root =
            std::env::temp_dir().join(format!("agentic-gpt-review-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.json");
        let secret_path = root.join("secret");

        let calls = Rc::new(Cell::new(0));
        let mut cancelled = ConfigTuiApp::with_committer(
            SetupSession::new(
                SetupSeed {
                    mode: Some(RuntimeMode::Standalone),
                    tunnel_id: Some("review-test-tunnel".into()),
                    tunnel_api_key: Some(format!("file:{}", secret_path.display())),
                    ..SetupSeed::default()
                },
                UiLanguage::En,
                config_path.clone(),
            ),
            Box::new(CountingCommitter {
                calls: calls.clone(),
                failure: None,
            }),
        );
        while cancelled.page() != ConfigPage::Review {
            cancelled.handle_action(TuiAction::Next).unwrap();
        }
        cancelled.handle_action(TuiAction::Cancel).unwrap();
        assert_eq!(calls.get(), 0);
        assert!(!config_path.exists());
        assert!(!secret_path.exists());

        let calls = Rc::new(Cell::new(0));
        let mut committed = ConfigTuiApp::with_committer(
            SetupSession::new(
                SetupSeed {
                    mode: Some(RuntimeMode::Standalone),
                    tunnel_id: Some("review-test-tunnel".into()),
                    tunnel_api_key: Some(format!("file:{}", secret_path.display())),
                    ..SetupSeed::default()
                },
                UiLanguage::En,
                config_path.clone(),
            ),
            Box::new(CountingCommitter {
                calls: calls.clone(),
                failure: None,
            }),
        );
        while committed.page() != ConfigPage::Review {
            committed.handle_action(TuiAction::Next).unwrap();
        }
        committed.state.focus = committed.review_group_count();
        committed.handle_action(TuiAction::Next).unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(committed.page(), ConfigPage::Completion);
        committed.handle_action(TuiAction::Next).unwrap();
        assert_eq!(calls.get(), 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn review_completion_and_system_error_render_without_secret_markers() {
        let marker = "phase-seven-secret-marker";
        let calls = Rc::new(Cell::new(0));
        let mut committed = ConfigTuiApp::with_committer(
            SetupSession::new(
                SetupSeed {
                    mode: Some(RuntimeMode::Standalone),
                    tunnel_id: Some("phase-seven-tunnel".into()),
                    tunnel_api_key: Some("file:/tmp/phase-seven-secret".into()),
                    ..SetupSeed::default()
                },
                UiLanguage::En,
                "/tmp/phase-seven-review.json".into(),
            ),
            Box::new(CountingCommitter {
                calls: calls.clone(),
                failure: None,
            }),
        );
        committed
            .session_mut()
            .standalone_mut()
            .provision_secret_now = true;
        committed.session_mut().standalone_mut().secret_value = Some(SecretValue::new(marker));
        while committed.page() != ConfigPage::Review {
            committed.handle_action(TuiAction::Next).unwrap();
        }
        assert!(rendered(&committed).contains("Review and write"));
        assert!(rendered(&committed).contains("Config path"));
        assert!(!rendered(&committed).contains(marker));
        committed.state.focus = committed.review_group_count();
        committed.handle_action(TuiAction::Next).unwrap();
        let completion = rendered(&committed);
        assert!(completion.contains("AgenticGPT initialization complete"));
        assert!(completion.contains("phase-seven-review.json"));
        assert!(completion.contains("Done"));
        assert!(!rendered(&committed).contains(marker));

        let calls = Rc::new(Cell::new(0));
        let mut failed = ConfigTuiApp::with_committer(
            SetupSession::new(
                SetupSeed {
                    mode: Some(RuntimeMode::Standalone),
                    tunnel_id: Some("phase-seven-tunnel".into()),
                    tunnel_api_key: Some("file:/tmp/phase-seven-secret".into()),
                    ..SetupSeed::default()
                },
                UiLanguage::En,
                "/tmp/phase-seven-error.json".into(),
            ),
            Box::new(CountingCommitter {
                calls,
                failure: Some(format!("write failed: {marker}")),
            }),
        );
        while failed.page() != ConfigPage::Review {
            failed.handle_action(TuiAction::Next).unwrap();
        }
        failed.state.focus = failed.review_group_count();
        failed.handle_action(TuiAction::Next).unwrap();
        assert_eq!(failed.page(), ConfigPage::SystemError);
        let error = rendered(&failed);
        assert!(error.contains("Initialization error"));
        assert!(error.contains("Exit"));
        assert!(!error.contains(marker));
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
    fn basic_selection_commits_only_on_activation_and_vim_aliases_are_mode_aware() {
        let mut app = app(RuntimeMode::Standalone);
        assert_eq!(app.session().selected_mode(), RuntimeMode::Standalone);

        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.state().focus, 1);
        assert_eq!(app.session().selected_mode(), RuntimeMode::Standalone);

        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.session().selected_mode(), RuntimeMode::Hub);
        assert_eq!(app.page(), ConfigPage::Basic);

        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.state().focus, 0);
        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.page(), ConfigPage::Basic);

        assert_eq!(app.session().selected_profile(), WorkerProfile::Normal);
        app.state.focus = 4;
        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.session().selected_profile(), WorkerProfile::Room);

        app.state.focus = 5;
        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Char('l'),
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.page(), ConfigPage::Connection);

        app.focus_field(crate::config_setup::SetupField::TunnelId);
        app.handle_action(TuiAction::Activate).unwrap();
        for character in ['h', 'j', 'k', 'l'] {
            app.handle_event(TerminalEvent::Key(KeyEvent::new(
                KeyCode::Char(character),
                KeyModifiers::NONE,
            )))
            .unwrap();
        }
        assert!(app.editing().unwrap().buffer.ends_with("hjkl"));
        assert_eq!(app.page(), ConfigPage::Connection);
    }

    #[test]
    fn enter_event_opens_optional_sections_and_activates_finish_action() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(app.page(), ConfigPage::OptionalCenter);

        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(
            app.page(),
            ConfigPage::Optional(crate::config_templates::OptionalSection::Identity)
        );

        app.handle_action(TuiAction::Back).unwrap();
        app.state.focus = app.session().available_optional_sections().len();
        app.handle_event(TerminalEvent::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
        assert_eq!(app.page(), ConfigPage::Review);
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
    fn editing_render_shows_live_buffer_and_cursor() {
        let mut standalone = app(RuntimeMode::Standalone);
        standalone.handle_action(TuiAction::Next).unwrap();
        standalone.focus_field(crate::config_setup::SetupField::TunnelId);
        standalone.handle_action(TuiAction::Activate).unwrap();
        standalone.handle_action(TuiAction::Text('X')).unwrap();
        let standalone_rendered = rendered(&standalone);
        assert!(standalone_rendered.contains("staged-tunnelX"));
        assert!(standalone_rendered.contains('█'));

        let mut hub = app(RuntimeMode::Hub);
        hub.handle_action(TuiAction::Next).unwrap();
        hub.focus_field(crate::config_setup::SetupField::AgentSecret);
        hub.handle_action(TuiAction::Activate).unwrap();
        hub.handle_action(TuiAction::Text('x')).unwrap();
        let hub_rendered = rendered(&hub);
        assert!(hub_rendered.contains('█'));
        assert!(!hub_rendered.contains("hub-secret-marker"));
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

    #[test]
    fn standalone_source_toggle_clears_file_only_provision() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        app.focus_field(crate::config_setup::SetupField::ProvisionTunnelSecret);
        app.handle_action(TuiAction::Activate).unwrap();
        assert!(app.session().standalone().provision_secret_now);

        app.focus_field(crate::config_setup::SetupField::TunnelSecretSource);
        assert_eq!(
            app.session().standalone().secret_source,
            crate::config_templates::TunnelSecretSource::File
        );
        app.handle_action(TuiAction::Activate).unwrap();
        assert_eq!(
            app.session().standalone().secret_source,
            crate::config_templates::TunnelSecretSource::Environment
        );
        assert!(!app.session().standalone().provision_secret_now);
        let rendered = rendered(&app);
        assert!(rendered.contains("Secret source"));
        assert!(rendered.contains("env"));
    }

    #[test]
    fn rollback_failure_keeps_the_specific_safe_system_error_code() {
        let mut app = app(RuntimeMode::Standalone);
        app.set_system_error(&anyhow!(
            "config_init_config_write_failed: config_init_secret_rollback_failed"
        ));
        assert_eq!(
            app.state().system_error.as_ref().unwrap().code,
            "config_init_secret_rollback_failed"
        );
    }

    #[test]
    fn validation_error_is_rendered_near_the_first_focused_field() {
        let mut app = app(RuntimeMode::Standalone);
        app.handle_action(TuiAction::Next).unwrap();
        app.session_mut().standalone_mut().tunnel_id.clear();
        app.session_mut().standalone_mut().secret_path.clear();
        app.handle_action(TuiAction::Next).unwrap();
        let rendered = rendered(&app);
        let tunnel_id = rendered.find("Tunnel ID").unwrap();
        let required = rendered.find("Required").unwrap();
        assert!(tunnel_id < required);
        assert_eq!(
            app.focused_field(),
            Some(crate::config_setup::SetupField::TunnelId)
        );
    }

    #[test]
    fn review_focus_scrolls_lower_groups_into_view() {
        let mut app = review_app(RuntimeMode::Standalone);
        app.state.focus = app.review_group_count() - 1;
        let rendered = rendered(&app);
        assert!(rendered.contains("Hub reporting"));
        assert!(rendered.contains("› Hub reporting"));
    }

    #[test]
    fn chinese_language_renders_the_setup_surface_in_chinese() {
        let app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::ZhCn,
            "/tmp/config-tui-zh.json".into(),
        ));
        let rendered = rendered(&app);
        assert!(rendered.contains("AgenticGPT 配"));
        assert!(rendered.contains("运 行"));
        assert!(!rendered.contains("AgenticGPT config init"));
    }
}
