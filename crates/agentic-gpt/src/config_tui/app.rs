use std::{collections::HashMap, path::Path};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;

use crate::cli_i18n::UiLanguage;
use crate::config_setup::{
    commit_wizard_outcome, OptionalSectionDraft, ReviewEditorKind, ReviewItemTarget, ReviewModel,
    ReviewRowKey, ReviewTarget, SetupField, SetupSession, ValidationErrors, WizardOutcome,
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
    pub(crate) review_subfocus: usize,
    pub(crate) review_mcp_index: Option<usize>,
    pub(crate) review_mcp_create: bool,
    pub(crate) review_search: String,
    pub(crate) review_search_active: bool,
    pub(crate) review_preview_json: Option<String>,
    pub(crate) review_preview_scroll: u16,
    pub(crate) review_preview_search: String,
    pub(crate) review_preview_search_active: bool,
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
    review_focus_anchor: Option<ReviewRowKey>,
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
                review_subfocus: 0,
                review_mcp_index: None,
                review_mcp_create: false,
                review_search: String::new(),
                review_search_active: false,
                review_preview_json: None,
                review_preview_scroll: 0,
                review_preview_search: String::new(),
                review_preview_search_active: false,
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
            review_focus_anchor: None,
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

    pub(crate) fn review_row_count(&self) -> usize {
        self.review
            .as_ref()
            .map(ReviewModel::row_count)
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
                } else if self.state.page == ConfigPage::SystemError {
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
                if self.review_complex_open() {
                    self.close_review_complex();
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
                } else if self.state.page == ConfigPage::SystemError {
                    self.state.finished = true;
                } else if self.navigation.back() {
                    self.sync_page();
                }
            }
            TuiAction::Cancel => {
                self.state.cancelled = true;
                self.state.editing = None;
                self.state.list_edit = None;
                self.state.mcp_edit = None;
                self.state.review_subfocus = 0;
                self.state.review_mcp_index = None;
                self.state.review_mcp_create = false;
                self.state.review_search.clear();
                self.state.review_search_active = false;
                self.leave_review_preview();
                self.section_draft = None;
                self.section_original = None;
                self.review = None;
                self.review_focus_anchor = None;
            }
            TuiAction::Activate => {
                if self.state.editing.is_some() {
                    self.commit_edit();
                } else if self.state.page == ConfigPage::Review {
                    self.activate_review_focus();
                } else if self.state.page == ConfigPage::SystemError {
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
        if self.review_preview_active() {
            if self.state.review_preview_search_active {
                return match key.code {
                    KeyCode::Esc => {
                        self.state.review_preview_search_active = false;
                        Ok(())
                    }
                    KeyCode::Enter => {
                        self.state.review_preview_search_active = false;
                        self.jump_review_preview_match(1);
                        Ok(())
                    }
                    KeyCode::Backspace => {
                        self.state.review_preview_search.pop();
                        Ok(())
                    }
                    KeyCode::Char(character) => {
                        self.state.review_preview_search.push(character);
                        Ok(())
                    }
                    _ => Ok(()),
                };
            }
            return match key.code {
                KeyCode::Esc => {
                    self.leave_review_preview();
                    Ok(())
                }
                KeyCode::Enter => {
                    self.confirm_and_write();
                    Ok(())
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_review_preview_scroll(1);
                    Ok(())
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_review_preview_scroll(-1);
                    Ok(())
                }
                KeyCode::PageDown => {
                    self.move_review_preview_scroll(10);
                    Ok(())
                }
                KeyCode::PageUp => {
                    self.move_review_preview_scroll(-10);
                    Ok(())
                }
                KeyCode::Char('/') => {
                    self.state.review_preview_search.clear();
                    self.state.review_preview_search_active = true;
                    Ok(())
                }
                KeyCode::Char('n') => {
                    self.jump_review_preview_match(1);
                    Ok(())
                }
                KeyCode::Char('N') => {
                    self.jump_review_preview_match(-1);
                    Ok(())
                }
                _ => Ok(()),
            };
        }
        if self.state.review_search_active {
            return match key.code {
                KeyCode::Esc => {
                    self.state.review_search_active = false;
                    Ok(())
                }
                KeyCode::Enter => {
                    self.state.review_search_active = false;
                    self.jump_review_match(1);
                    Ok(())
                }
                KeyCode::Backspace => {
                    self.state.review_search.pop();
                    Ok(())
                }
                KeyCode::Char(character) => {
                    self.state.review_search.push(character);
                    Ok(())
                }
                _ => Ok(()),
            };
        }
        if self.state.editing.is_some() {
            if self.state.page == ConfigPage::Review
                && self.review_current_editor() == Some(ReviewEditorKind::Choice)
            {
                return match key.code {
                    KeyCode::Esc => self.handle_action(TuiAction::Back),
                    KeyCode::Enter => self.handle_action(TuiAction::Activate),
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.cycle_review_choice(1);
                        Ok(())
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.cycle_review_choice(-1);
                        Ok(())
                    }
                    _ => Ok(()),
                };
            }
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
            KeyCode::Char('/')
                if self.state.page == ConfigPage::Review && !self.review_complex_open() =>
            {
                self.state.review_search.clear();
                self.state.review_search_active = true;
                Ok(())
            }
            KeyCode::Char('n') if self.state.page == ConfigPage::Review => {
                self.jump_review_match(1);
                Ok(())
            }
            KeyCode::Char('N') if self.state.page == ConfigPage::Review => {
                self.jump_review_match(-1);
                Ok(())
            }
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
            KeyCode::Char(' ') if self.multi_select_active() => {
                self.toggle_multi_select();
                Ok(())
            }
            KeyCode::Char('J') if self.multi_select_active() => {
                self.move_multi_select_priority(1);
                Ok(())
            }
            KeyCode::Char('K') if self.multi_select_active() => {
                self.move_multi_select_priority(-1);
                Ok(())
            }
            KeyCode::Tab => self.handle_action(TuiAction::MoveNext),
            KeyCode::BackTab => self.handle_action(TuiAction::MovePrevious),
            KeyCode::Down | KeyCode::Right => self.handle_action(TuiAction::MoveNext),
            KeyCode::Up | KeyCode::Left => self.handle_action(TuiAction::MovePrevious),
            KeyCode::Char('a')
                if self.state.page == ConfigPage::Review
                    && self.review_current_editor() == Some(ReviewEditorKind::List)
                    && self.review_complex_open() =>
            {
                self.add_review_list_item();
                Ok(())
            }
            KeyCode::Char('d')
                if self.state.page == ConfigPage::Review
                    && self.review_current_editor() == Some(ReviewEditorKind::List)
                    && self.review_complex_open() =>
            {
                self.remove_review_list_item();
                Ok(())
            }
            KeyCode::Char('d')
                if self.state.page == ConfigPage::Review
                    && self.review_current_editor() == Some(ReviewEditorKind::Compound)
                    && self.review_complex_open() =>
            {
                self.delete_review_mcp_server();
                Ok(())
            }
            KeyCode::Char('a')
                if self.state.page == ConfigPage::Review && !self.review_complex_open() =>
            {
                self.begin_review_mcp_add();
                Ok(())
            }
            KeyCode::Char('a') if matches!(self.state.page, ConfigPage::Optional(_)) => {
                self.handle_action(TuiAction::AddListItem)
            }
            KeyCode::Char('d') if matches!(self.state.page, ConfigPage::Optional(_)) => {
                self.handle_action(TuiAction::RemoveListItem)
            }
            KeyCode::Char('j') => self.handle_action(TuiAction::MoveNext),
            KeyCode::Char('k') => self.handle_action(TuiAction::MovePrevious),
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
            ConfigPage::Review if self.state.focus >= self.review_row_count() => {
                self.activate_review_confirmation();
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
            if self.section_draft.as_ref().is_some_and(|draft| {
                pages::optional_multi_select_for_focus(section, draft, self.state.focus).is_some()
            }) {
                self.mutate_optional_multi_select(section, None);
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
        if self.state.page == ConfigPage::Review {
            self.commit_review_edit(edit);
            return;
        }
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

    fn review_current_editor(&self) -> Option<ReviewEditorKind> {
        if self.state.review_mcp_create {
            return Some(ReviewEditorKind::Compound);
        }
        self.review
            .as_ref()?
            .row(self.state.focus)
            .map(|(_, item)| item.editor)
    }

    fn review_preview_active(&self) -> bool {
        self.state.page == ConfigPage::Review && self.state.review_preview_json.is_some()
    }

    fn review_preview_search_matches(&self) -> Vec<usize> {
        let query = self.state.review_preview_search.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.state
            .review_preview_json
            .as_deref()
            .unwrap_or_default()
            .lines()
            .enumerate()
            .filter_map(|(index, line)| line.to_lowercase().contains(&query).then_some(index))
            .collect()
    }

    fn move_review_preview_scroll(&mut self, delta: isize) {
        let line_count = self
            .state
            .review_preview_json
            .as_deref()
            .map(|json| json.lines().count())
            .unwrap_or(0);
        let max_line = line_count.saturating_sub(1).min(usize::from(u16::MAX));
        let next = (isize::try_from(self.state.review_preview_scroll).unwrap_or_default() + delta)
            .clamp(0, isize::try_from(max_line).unwrap_or(isize::MAX));
        self.state.review_preview_scroll = u16::try_from(next).unwrap_or(u16::MAX);
    }

    fn jump_review_preview_match(&mut self, direction: isize) {
        let matches = self.review_preview_search_matches();
        if matches.is_empty() {
            return;
        }
        let current = usize::from(self.state.review_preview_scroll);
        let target = if direction >= 0 {
            matches
                .iter()
                .copied()
                .find(|line| *line > current)
                .unwrap_or(matches[0])
        } else {
            matches
                .iter()
                .rev()
                .copied()
                .find(|line| *line < current)
                .unwrap_or_else(|| *matches.last().expect("matches is non-empty"))
        };
        self.state.review_preview_scroll = target.min(usize::from(u16::MAX)) as u16;
    }

    fn leave_review_preview(&mut self) {
        self.state.review_preview_json = None;
        self.state.review_preview_scroll = 0;
        self.state.review_preview_search.clear();
        self.state.review_preview_search_active = false;
    }

    fn review_search_matches(&self) -> Vec<usize> {
        let query = self.state.review_search.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let Some(review) = self.review.as_ref() else {
            return Vec::new();
        };
        (0..review.row_count())
            .filter(|index| {
                review.row(*index).is_some_and(|(group, item)| {
                    pages::review_search_text(group, item, self.session(), self.language)
                        .to_lowercase()
                        .contains(&query)
                })
            })
            .collect()
    }

    fn jump_review_match(&mut self, direction: isize) {
        if self.state.page != ConfigPage::Review || self.review_complex_open() {
            return;
        }
        let matches = self.review_search_matches();
        if matches.is_empty() {
            return;
        }
        let current = self.state.focus;
        self.state.focus = if direction >= 0 {
            matches
                .iter()
                .copied()
                .find(|index| *index > current)
                .unwrap_or(matches[0])
        } else {
            matches
                .iter()
                .rev()
                .copied()
                .find(|index| *index < current)
                .unwrap_or_else(|| *matches.last().expect("matches is non-empty"))
        };
    }

    fn multi_select_active(&self) -> bool {
        match self.state.page {
            ConfigPage::Optional(section) => self.section_draft.as_ref().is_some_and(|draft| {
                pages::optional_multi_select_for_focus(section, draft, self.state.focus).is_some()
            }),
            ConfigPage::Review => {
                self.review_complex_open()
                    && self.review_current_editor() == Some(ReviewEditorKind::MultiSelect)
            }
            _ => false,
        }
    }

    fn toggle_multi_select(&mut self) {
        match self.state.page {
            ConfigPage::Optional(section) => self.mutate_optional_multi_select(section, None),
            ConfigPage::Review => self.mutate_review_multi_select(None),
            _ => {}
        }
    }

    fn move_multi_select_priority(&mut self, direction: isize) {
        match self.state.page {
            ConfigPage::Optional(section) => {
                self.mutate_optional_multi_select(section, Some(direction))
            }
            ConfigPage::Review => self.mutate_review_multi_select(Some(direction)),
            _ => {}
        }
    }

    fn mutate_optional_multi_select(
        &mut self,
        section: crate::config_templates::OptionalSection,
        reorder: Option<isize>,
    ) {
        let Some((field, value)) = self.section_draft.as_ref().and_then(|draft| {
            pages::optional_multi_select_for_focus(section, draft, self.state.focus)
        }) else {
            return;
        };
        let Some(draft) = self.section_draft.as_mut() else {
            return;
        };
        let Ok(mut selection) = pages::optional_multi_select_state(draft, field) else {
            return;
        };
        let Some(index) = selection
            .options()
            .iter()
            .position(|option| option == value)
        else {
            return;
        };
        selection.set_focus(index);
        let changed = match reorder {
            Some(direction) => selection.move_focused_selection(direction),
            None => selection.toggle_focused(),
        };
        if changed && pages::set_optional_multi_select_state(draft, field, &selection) {
            self.validate_section_draft();
        }
    }

    fn mutate_review_multi_select(&mut self, reorder: Option<isize>) {
        if self.review_current_editor() != Some(ReviewEditorKind::MultiSelect) {
            return;
        }
        let Some((_, item)) = self
            .review
            .as_ref()
            .and_then(|review| review.row(self.state.focus))
        else {
            return;
        };
        let Some(field) = item.field else {
            return;
        };
        let Some(mut draft) = self.section_draft.as_ref().cloned() else {
            return;
        };
        let Ok(mut selection) = pages::optional_multi_select_state(&draft, field) else {
            return;
        };
        selection.set_focus(self.state.review_subfocus);
        let changed = match reorder {
            Some(direction) => selection.move_focused_selection(direction),
            None => selection.toggle_focused(),
        };
        if !changed || !pages::set_optional_multi_select_state(&mut draft, field, &selection) {
            return;
        }
        match self
            .session_mut()
            .save_optional_section_for_review(draft.clone())
        {
            Ok(()) => {
                self.section_draft = Some(draft);
                self.field_errors.clear();
                let anchor = self.review_focus_anchor;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = anchor
                        .and_then(|key| self.review.as_ref()?.find_row(key))
                        .unwrap_or(self.state.focus);
                }
            }
            Err(errors) => self.record_errors(errors),
        }
    }

    fn review_complex_open(&self) -> bool {
        self.state.page == ConfigPage::Review
            && self.section_draft.is_some()
            && (self.state.review_mcp_create
                || matches!(
                    self.review_current_editor(),
                    Some(
                        ReviewEditorKind::MultiSelect
                            | ReviewEditorKind::List
                            | ReviewEditorKind::Compound
                            | ReviewEditorKind::AutoCustom
                    )
                ))
    }

    fn review_complex_focus_len(&self) -> usize {
        match self.review_current_editor() {
            Some(ReviewEditorKind::MultiSelect) => self
                .review
                .as_ref()
                .and_then(|review| review.row(self.state.focus))
                .and_then(|(_, item)| item.field)
                .map(|field| pages::multi_select_options(field).len())
                .unwrap_or(0),
            Some(ReviewEditorKind::List) => {
                let Some(field) = self.current_review_list_field() else {
                    return 0;
                };
                let Some(draft) = self.section_draft.as_ref() else {
                    return 0;
                };
                pages::optional_list_state(draft, field)
                    .map(|state| state.items().len())
                    .unwrap_or(0)
            }
            Some(ReviewEditorKind::Compound) => {
                if self.state.review_mcp_index.is_some() {
                    4
                } else {
                    0
                }
            }
            Some(ReviewEditorKind::AutoCustom) => 2,
            _ => 0,
        }
    }

    fn current_review_list_field(&self) -> Option<SetupField> {
        let (_, item) = self.review.as_ref()?.row(self.state.focus)?;
        if item.editor == ReviewEditorKind::List {
            item.field
        } else {
            None
        }
    }

    fn activate_review_complex_focus(&mut self) {
        match self.review_current_editor() {
            Some(ReviewEditorKind::MultiSelect) => self.mutate_review_multi_select(None),
            Some(ReviewEditorKind::List) => {
                let Some(field) = self.current_review_list_field() else {
                    return;
                };
                let Some(draft) = self.section_draft.as_ref() else {
                    return;
                };
                let Ok(list) = pages::optional_list_state(draft, field) else {
                    return;
                };
                let item_count = list.items().len();
                if self.state.review_subfocus < item_count {
                    self.begin_review_list_edit(field, self.state.review_subfocus, false);
                }
            }
            Some(ReviewEditorKind::Compound) => {
                let Some(index) = self.state.review_mcp_index else {
                    return;
                };
                match self.state.review_subfocus {
                    0 => self.begin_review_mcp_edit(index, SetupField::McpServerId),
                    1 => {
                        if let Some(draft) = self.section_draft.as_mut() {
                            pages::toggle_mcp_server_enabled(draft, index);
                        }
                        self.stage_review_mcp_draft_if_valid();
                    }
                    2 => {
                        if let Some(draft) = self.section_draft.as_mut() {
                            pages::toggle_mcp_server_transport(draft, index);
                        }
                        self.stage_review_mcp_draft_if_valid();
                    }
                    3 => self.begin_review_mcp_edit(index, SetupField::McpServerEndpoint),
                    _ => {}
                }
            }
            Some(ReviewEditorKind::AutoCustom) => match self.state.review_subfocus {
                0 => self.stage_review_auto_custom("auto".to_string()),
                1 => {
                    let configured = self
                        .section_draft
                        .as_ref()
                        .map(|draft| pages::optional_field_value(draft, SetupField::MaxActiveJobs))
                        .unwrap_or_else(|| "auto".to_string());
                    let value = if configured == "auto" {
                        self.state.max_active_custom.clone()
                    } else {
                        configured
                    };
                    self.field_errors.remove(&SetupField::MaxActiveJobs);
                    self.state.editing = Some(EditState::new(SetupField::MaxActiveJobs, value));
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn begin_review_list_edit(&mut self, field: SetupField, index: usize, created: bool) {
        let value = self
            .section_draft
            .as_ref()
            .and_then(|draft| pages::optional_list_state(draft, field).ok())
            .and_then(|mut list| {
                list.set_focus(index);
                list.focused().map(str::to_string)
            });
        let Some(value) = value else {
            return;
        };
        self.state.list_edit = Some(ListEditTarget {
            field,
            index,
            created,
        });
        self.state.editing = Some(EditState::new(field, value));
        self.field_errors.remove(&field);
    }

    fn add_review_list_item(&mut self) {
        let Some(field) = self.current_review_list_field() else {
            return;
        };
        let Some(draft) = self.section_draft.as_mut() else {
            return;
        };
        let Ok(mut list) = pages::optional_list_state(draft, field) else {
            return;
        };
        if !list.is_empty() {
            let existing = self.state.review_subfocus.min(list.items().len() - 1);
            list.set_focus(existing);
        }
        let index = list.add_after_focused();
        if !pages::set_optional_list_state(draft, field, &list) {
            return;
        }
        self.state.review_subfocus = index;
        self.begin_review_list_edit(field, index, true);
    }

    fn remove_review_list_item(&mut self) {
        let Some(field) = self.current_review_list_field() else {
            return;
        };
        let Some(mut draft) = self.section_draft.as_ref().cloned() else {
            return;
        };
        let Ok(mut list) = pages::optional_list_state(&draft, field) else {
            return;
        };
        if self.state.review_subfocus >= list.items().len() {
            return;
        }
        list.set_focus(self.state.review_subfocus);
        if list.delete_focused().is_none()
            || !pages::set_optional_list_state(&mut draft, field, &list)
        {
            return;
        }
        match self
            .session_mut()
            .save_optional_section_for_review(draft.clone())
        {
            Ok(()) => {
                self.section_draft = Some(draft);
                self.state.review_subfocus = self
                    .state
                    .review_subfocus
                    .min(list.items().len().saturating_sub(1));
                self.field_errors.clear();
                let anchor = self.review_focus_anchor;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = anchor
                        .and_then(|key| self.review.as_ref()?.find_row(key))
                        .unwrap_or(self.state.focus);
                }
            }
            Err(errors) => self.record_errors(errors),
        }
    }

    fn begin_review_mcp_add(&mut self) {
        if self.state.page != ConfigPage::Review || self.review_preview_active() {
            return;
        }
        let section = crate::config_templates::OptionalSection::McpServers;
        let mut draft = self.session().optional_draft(section);
        let Some(index) = pages::add_mcp_server(&mut draft, None) else {
            return;
        };
        self.review_focus_anchor = None;
        self.section_draft = Some(draft);
        self.section_original = None;
        self.state.review_subfocus = 0;
        self.state.review_mcp_index = Some(index);
        self.state.review_mcp_create = true;
        self.field_errors.clear();
    }

    fn begin_review_mcp_edit(&mut self, index: usize, field: SetupField) {
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
            created: false,
        });
        self.state.editing = Some(EditState::new(field, value));
        self.field_errors.remove(&field);
    }

    fn stage_review_auto_custom(&mut self, value: String) {
        let Some(mut draft) = self.section_draft.as_ref().cloned() else {
            return;
        };
        pages::set_optional_field(&mut draft, SetupField::MaxActiveJobs, value.clone());
        match self
            .session_mut()
            .save_optional_section_for_review(draft.clone())
        {
            Ok(()) => {
                if value != "auto" {
                    self.state.max_active_custom = value;
                }
                self.section_draft = Some(draft);
                self.field_errors.clear();
                let anchor = self.review_focus_anchor;
                let old_focus = self.state.focus;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = anchor
                        .and_then(|key| self.review.as_ref()?.find_row(key))
                        .unwrap_or(old_focus.min(self.review_row_count().saturating_sub(1)));
                }
            }
            Err(errors) => self.record_errors(errors),
        }
    }

    fn commit_review_auto_custom(&mut self, edit: EditState) {
        let value = edit.buffer.clone();
        let Some(mut draft) = self.section_draft.as_ref().cloned() else {
            self.state.editing = Some(edit);
            return;
        };
        pages::set_optional_field(&mut draft, SetupField::MaxActiveJobs, value.clone());
        match self
            .session_mut()
            .save_optional_section_for_review(draft.clone())
        {
            Ok(()) => {
                self.state.max_active_custom = value;
                self.section_draft = Some(draft);
                self.state.editing = None;
                self.field_errors.remove(&SetupField::MaxActiveJobs);
                let anchor = self.review_focus_anchor;
                let old_focus = self.state.focus;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = anchor
                        .and_then(|key| self.review.as_ref()?.find_row(key))
                        .unwrap_or(old_focus.min(self.review_row_count().saturating_sub(1)));
                }
            }
            Err(errors) => {
                let inline_code = errors.first().map(|error| error.code);
                self.record_errors(errors);
                if let Some(code) = inline_code {
                    self.field_errors
                        .insert(SetupField::MaxActiveJobs, code.to_string());
                }
                self.state.editing = Some(edit);
            }
        }
    }

    fn stage_review_mcp_draft_if_valid(&mut self) {
        let Some(draft) = self.section_draft.as_ref().cloned() else {
            return;
        };
        match self
            .session_mut()
            .save_optional_section_for_review(draft.clone())
        {
            Ok(()) => {
                let mut anchor = self.review_focus_anchor;
                if self.state.review_mcp_create {
                    if let Some(index) = self.state.review_mcp_index {
                        anchor = Some(ReviewRowKey {
                            group: ReviewTarget::OptionalSection(
                                crate::config_templates::OptionalSection::McpServers,
                            ),
                            field: None,
                            target: ReviewItemTarget::McpServer { index },
                        });
                        self.state.review_mcp_create = false;
                    }
                }
                self.review_focus_anchor = anchor;
                self.section_draft = Some(draft);
                self.field_errors.clear();
                let old_focus = self.state.focus;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = anchor
                        .and_then(|key| self.review.as_ref()?.find_row(key))
                        .unwrap_or(old_focus.min(self.review_row_count().saturating_sub(1)));
                }
            }
            Err(errors) => {
                self.field_errors.clear();
                for error in errors {
                    self.field_errors
                        .insert(error.field, error.code.to_string());
                }
            }
        }
    }

    fn save_review_complex(&mut self) {
        let Some(draft) = self.section_draft.as_ref().cloned() else {
            return;
        };
        match self.session_mut().save_optional_section_for_review(draft) {
            Ok(()) => {
                let anchor = self.review_focus_anchor.take();
                let old_focus = self.state.focus;
                self.section_draft = None;
                self.section_original = None;
                self.state.editing = None;
                self.state.list_edit = None;
                self.state.mcp_edit = None;
                self.state.review_subfocus = 0;
                self.state.review_mcp_index = None;
                self.field_errors.clear();
                self.state.review_mcp_create = false;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = anchor
                        .and_then(|key| self.review.as_ref()?.find_row(key))
                        .unwrap_or_else(|| {
                            old_focus.min(self.review_row_count().saturating_sub(1))
                        });
                }
            }
            Err(errors) => self.record_errors(errors),
        }
    }

    fn delete_review_mcp_server(&mut self) {
        if self.state.review_mcp_create {
            self.close_review_complex();
            return;
        }
        let Some(index) = self.state.review_mcp_index else {
            return;
        };
        let Some(draft) = self.section_draft.as_mut() else {
            return;
        };
        if pages::remove_mcp_server(draft, index) {
            self.save_review_complex();
        }
    }

    fn close_review_complex(&mut self) {
        self.section_draft = None;
        self.section_original = None;
        self.state.editing = None;
        self.state.list_edit = None;
        self.state.mcp_edit = None;
        self.state.review_subfocus = 0;
        self.state.review_mcp_index = None;
        self.state.review_mcp_create = false;
        self.review_focus_anchor = None;
        self.field_errors.clear();
    }

    fn review_edit_value(&self, group: ReviewTarget, field: SetupField) -> Option<String> {
        match group {
            ReviewTarget::Connection => pages::connection_value(self.session(), field),
            ReviewTarget::OptionalSection(section) => {
                let draft = self.session().optional_draft(section);
                Some(pages::optional_field_value(&draft, field))
            }
            ReviewTarget::Basic => match field {
                SetupField::Mode => {
                    Some(format!("{:?}", self.session().selected_mode()).to_lowercase())
                }
                SetupField::Profile => {
                    Some(format!("{:?}", self.session().selected_profile()).to_lowercase())
                }
                _ => None,
            },
            ReviewTarget::OptionalCenter => None,
        }
    }

    fn cycle_review_choice(&mut self, delta: isize) {
        let next_value = {
            let Some(review) = self.review.as_ref() else {
                return;
            };
            let Some((_, item)) = review.row(self.state.focus) else {
                return;
            };
            let choices = item.choice_values();
            if choices.is_empty() {
                return;
            }
            let Some(editing) = self.state.editing.as_ref() else {
                return;
            };
            let current = choices
                .iter()
                .position(|choice| choice.eq_ignore_ascii_case(&editing.buffer))
                .unwrap_or(0);
            let next = (current as isize + delta).rem_euclid(choices.len() as isize) as usize;
            choices[next].to_string()
        };
        if let Some(editing) = self.state.editing.as_mut() {
            editing.buffer = next_value;
            editing.cursor = editing.buffer.chars().count();
        }
    }

    fn commit_review_edit(&mut self, edit: EditState) {
        if let Some(target) = self.state.list_edit.take() {
            self.commit_review_list_item(edit, target);
            return;
        }
        if let Some(target) = self.state.mcp_edit.take() {
            self.commit_review_mcp_field(edit, target);
            return;
        }
        if self.review_current_editor() == Some(ReviewEditorKind::AutoCustom)
            && edit.field == SetupField::MaxActiveJobs
        {
            self.commit_review_auto_custom(edit);
            return;
        }
        let field = edit.field;
        let anchor = self.review_focus_anchor.or_else(|| {
            self.review
                .as_ref()
                .and_then(|review| review.row_key(self.state.focus))
        });
        let Some(anchor) = anchor else {
            self.state.editing = Some(edit);
            return;
        };
        let value = edit.buffer.clone();
        match self.apply_review_value(anchor, field, value) {
            Ok(()) => {
                self.field_errors.clear();
                self.review_focus_anchor = None;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = self
                        .review
                        .as_ref()
                        .and_then(|review| review.find_row(anchor))
                        .unwrap_or_else(|| self.review_row_count().saturating_sub(1));
                }
            }
            Err(errors) => {
                let inline_code = errors.first().map(|error| error.code);
                self.record_errors(errors);
                if let Some(code) = inline_code {
                    self.field_errors.insert(field, code.to_string());
                }
                self.state.editing = Some(edit);
            }
        }
    }

    fn commit_review_list_item(&mut self, edit: EditState, target: ListEditTarget) {
        let Some(mut draft) = self.section_draft.as_ref().cloned() else {
            self.state.editing = Some(edit);
            self.state.list_edit = Some(target);
            return;
        };
        let Ok(mut list) = pages::optional_list_state(&draft, target.field) else {
            self.state.editing = Some(edit);
            self.state.list_edit = Some(target);
            return;
        };
        list.set_focus(target.index);
        if !list.set_focused(edit.buffer.clone())
            || !pages::set_optional_list_state(&mut draft, target.field, &list)
        {
            self.state.editing = Some(edit);
            self.state.list_edit = Some(target);
            return;
        }
        match self
            .session_mut()
            .save_optional_section_for_review(draft.clone())
        {
            Ok(()) => {
                self.section_draft = Some(draft);
                self.state.editing = None;
                self.field_errors.remove(&target.field);
                let anchor = self.review_focus_anchor;
                self.refresh_review();
                if self.state.page == ConfigPage::Review {
                    self.state.focus = anchor
                        .and_then(|key| self.review.as_ref()?.find_row(key))
                        .unwrap_or(self.state.focus);
                }
            }
            Err(errors) => {
                let inline_code = errors.first().map(|error| error.code);
                self.record_errors(errors);
                if let Some(code) = inline_code {
                    self.field_errors.insert(target.field, code.to_string());
                }
                self.state.editing = Some(edit);
                self.state.list_edit = Some(target);
            }
        }
    }

    fn commit_review_mcp_field(&mut self, edit: EditState, target: McpEditTarget) {
        let Some(mut draft) = self.section_draft.as_ref().cloned() else {
            self.state.editing = Some(edit);
            self.state.mcp_edit = Some(target);
            return;
        };
        if !pages::set_mcp_server_value(&mut draft, target.index, target.field, edit.buffer.clone())
        {
            self.state.editing = Some(edit);
            self.state.mcp_edit = Some(target);
            return;
        }
        self.section_draft = Some(draft);
        self.state.editing = None;
        self.field_errors.remove(&target.field);
        self.stage_review_mcp_draft_if_valid();
    }

    fn apply_review_value(
        &mut self,
        anchor: ReviewRowKey,
        field: SetupField,
        value: String,
    ) -> Result<(), ValidationErrors> {
        match anchor.group {
            ReviewTarget::Connection => self.apply_review_connection_value(field, value),
            ReviewTarget::OptionalSection(section) => {
                let mut draft = self.session().optional_draft(section);
                pages::set_optional_field(&mut draft, field, value);
                self.session_mut().save_optional_section_for_review(draft)
            }
            ReviewTarget::Basic => {
                match field {
                    SetupField::Mode => {
                        let mode = match value.as_str() {
                            "standalone" => Some(RuntimeMode::Standalone),
                            "hub" => Some(RuntimeMode::Hub),
                            "local" => Some(RuntimeMode::Local),
                            _ => None,
                        };
                        if let Some(mode) = mode {
                            self.session_mut().set_mode(mode);
                            self.navigation.set_mode(mode);
                        }
                    }
                    SetupField::Profile => match value.as_str() {
                        "normal" => self.session_mut().set_profile(WorkerProfile::Normal),
                        "room" => self.session_mut().set_profile(WorkerProfile::Room),
                        _ => {}
                    },
                    _ => {}
                }
                Ok(())
            }
            ReviewTarget::OptionalCenter => Ok(()),
        }
    }

    fn apply_review_connection_value(
        &mut self,
        field: SetupField,
        value: String,
    ) -> Result<(), ValidationErrors> {
        if field == SetupField::TunnelSecretSource {
            let source = match value.as_str() {
                "file" => TunnelSecretSource::File,
                "env" => TunnelSecretSource::Environment,
                _ => return Ok(()),
            };
            let draft = self.session_mut().standalone_mut();
            if source == TunnelSecretSource::Environment {
                draft.provision_secret_now = false;
                draft.secret_value = None;
            }
            draft.secret_source = source;
            return Ok(());
        }

        if field == SetupField::ProvisionTunnelSecret {
            self.session_mut().standalone_mut().provision_secret_now = value == "true";
            return Ok(());
        }

        if field == SetupField::TunnelSecretValue {
            let previous = self
                .session_mut()
                .standalone_mut()
                .secret_value
                .replace(SecretValue::new(value));
            let result = self.session().validate_field(field);
            if result.is_err() {
                self.session_mut().standalone_mut().secret_value = previous;
            }
            return result;
        }

        if field == SetupField::AgentSecret {
            let previous = self
                .session_mut()
                .hub_mut()
                .agent_secret
                .replace(SecretValue::new(value));
            let result = self.session().validate_field(field);
            if result.is_err() {
                self.session_mut().hub_mut().agent_secret = previous;
            }
            return result;
        }

        let previous = match field {
            SetupField::TunnelId => Some(std::mem::replace(
                &mut self.session_mut().standalone_mut().tunnel_id,
                value,
            )),
            SetupField::TunnelSecretPath => Some(std::mem::replace(
                &mut self.session_mut().standalone_mut().secret_path,
                value,
            )),
            SetupField::TunnelSecretEnvironment => Some(std::mem::replace(
                &mut self.session_mut().standalone_mut().secret_environment,
                value,
            )),
            SetupField::HubUrl => Some(std::mem::replace(
                &mut self.session_mut().hub_mut().hub_url,
                value,
            )),
            SetupField::HubTransport => Some(std::mem::replace(
                &mut self.session_mut().hub_mut().hub_transport,
                value,
            )),
            SetupField::AgentId => Some(std::mem::replace(
                &mut self.session_mut().hub_mut().agent_id,
                value,
            )),
            _ => None,
        };
        let Some(previous) = previous else {
            return Ok(());
        };

        let result = self.session().validate_field(field);
        if result.is_err() {
            match field {
                SetupField::TunnelId => self.session_mut().standalone_mut().tunnel_id = previous,
                SetupField::TunnelSecretPath => {
                    self.session_mut().standalone_mut().secret_path = previous
                }
                SetupField::TunnelSecretEnvironment => {
                    self.session_mut().standalone_mut().secret_environment = previous
                }
                SetupField::HubUrl => self.session_mut().hub_mut().hub_url = previous,
                SetupField::HubTransport => self.session_mut().hub_mut().hub_transport = previous,
                SetupField::AgentId => self.session_mut().hub_mut().agent_id = previous,
                _ => {}
            }
        }
        result
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
        if self.state.page == ConfigPage::Review {
            if self.review_current_editor() == Some(ReviewEditorKind::AutoCustom) {
                self.field_errors.clear();
                self.state.editing = None;
                return;
            }
            if let Some(target) = self.state.list_edit.take() {
                if target.created {
                    if let Some(draft) = self.section_draft.as_mut() {
                        if let Ok(mut list) = pages::optional_list_state(draft, target.field) {
                            list.set_focus(target.index);
                            let _ = list.delete_focused();
                            let _ = pages::set_optional_list_state(draft, target.field, &list);
                            self.state.review_subfocus =
                                self.state.review_subfocus.min(list.items().len());
                        }
                    }
                }
                self.field_errors.clear();
                self.state.editing = None;
                return;
            }
            if self.state.mcp_edit.take().is_some() {
                self.field_errors.clear();
                self.state.editing = None;
                return;
            }
            self.field_errors.clear();
            self.state.editing = None;
            self.review_focus_anchor = None;
            return;
        }
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
        if self.review_complex_open() {
            let length = self.review_complex_focus_len();
            if length > 0 {
                let current = self.state.review_subfocus as isize;
                self.state.review_subfocus =
                    (current + direction).rem_euclid(length as isize) as usize;
            }
            return;
        }
        let length = match self.state.page {
            ConfigPage::Basic => pages::basic_focus_len(),
            ConfigPage::Connection => pages::connection_focus_len(self.session()),
            ConfigPage::OptionalCenter => self.session().available_optional_sections().len() + 1,
            ConfigPage::Optional(section) => self
                .section_draft
                .as_ref()
                .map(|draft| pages::optional_focus_len(section, draft))
                .unwrap_or(1),
            ConfigPage::Review => self.review_row_count() + 1,
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
                SetupField::Mode => Some(match self.session().selected_mode() {
                    RuntimeMode::Standalone => 0,
                    RuntimeMode::Hub => 1,
                    RuntimeMode::Local => 2,
                }),
                SetupField::Profile => Some(match self.session().selected_profile() {
                    WorkerProfile::Normal => 3,
                    WorkerProfile::Room => 4,
                }),
                _ => None,
            },
            ConfigPage::Connection => pages::connection_field_index(self.session(), field),
            ConfigPage::Optional(section) => self
                .section_draft
                .as_ref()
                .and_then(|draft| pages::optional_field_index(section, draft, field)),
            ConfigPage::Review => self.review.as_ref().and_then(|review| {
                (0..review.row_count()).find(|index| {
                    review.row(*index).is_some_and(|(_, item)| {
                        item.field == Some(field)
                            || (matches!(
                                field,
                                SetupField::McpServerId
                                    | SetupField::McpServerEnabled
                                    | SetupField::McpServerTransport
                                    | SetupField::McpServerEndpoint
                            ) && matches!(item.target, ReviewItemTarget::McpServer { .. }))
                    })
                })
            }),
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
        if self.review_complex_open() {
            self.activate_review_complex_focus();
            return;
        }
        let Some(review) = self.review.as_ref() else {
            self.refresh_review();
            return;
        };
        if self.state.focus >= review.row_count() {
            self.activate_review_confirmation();
            return;
        }
        let Some((group, item)) = review.row(self.state.focus) else {
            return;
        };
        let row_key = review.row_key(self.state.focus);
        let group_target = group.target;
        let editor = item.editor;
        let field = item.field;
        let item_target = item.target;

        match editor {
            ReviewEditorKind::Text | ReviewEditorKind::Secret => {
                let Some(field) = field else {
                    return;
                };
                let Some(value) = self.review_edit_value(group_target, field) else {
                    return;
                };
                self.review_focus_anchor = row_key;
                self.field_errors.remove(&field);
                self.state.editing = Some(EditState::new(field, value));
            }
            ReviewEditorKind::Choice => {
                let Some(field) = field else {
                    return;
                };
                let choices = item.choice_values();
                if group_target == ReviewTarget::Basic || choices.len() == 2 {
                    let Some(anchor) = row_key else {
                        return;
                    };
                    let Some(current) = self.review_edit_value(group_target, field) else {
                        return;
                    };
                    let current_index = choices
                        .iter()
                        .position(|choice| choice.eq_ignore_ascii_case(&current))
                        .unwrap_or(0);
                    let next = choices[(current_index + 1) % choices.len()].to_string();
                    match self.apply_review_value(anchor, field, next) {
                        Ok(()) => {
                            self.field_errors.clear();
                            self.refresh_review();
                            if self.state.page == ConfigPage::Review {
                                self.state.focus = self
                                    .review
                                    .as_ref()
                                    .and_then(|review| review.find_row(anchor))
                                    .unwrap_or(0);
                            }
                        }
                        Err(errors) => self.record_errors(errors),
                    }
                } else {
                    let Some(value) = self.review_edit_value(group_target, field) else {
                        return;
                    };
                    self.review_focus_anchor = row_key;
                    self.field_errors.remove(&field);
                    self.state.editing = Some(EditState::new(field, value));
                }
            }
            ReviewEditorKind::MultiSelect | ReviewEditorKind::List => {
                let ReviewTarget::OptionalSection(section) = group_target else {
                    return;
                };
                self.review_focus_anchor = row_key;
                self.section_draft = Some(self.session().optional_draft(section));
                self.section_original = None;
                self.state.review_subfocus = 0;
                self.state.review_mcp_index = None;
                self.state.review_mcp_create = false;
                self.field_errors.clear();
            }
            ReviewEditorKind::Compound => {
                let ReviewTarget::OptionalSection(section) = group_target else {
                    return;
                };
                if section != crate::config_templates::OptionalSection::McpServers {
                    return;
                }
                let draft = self.session().optional_draft(section);
                let index = match item_target {
                    ReviewItemTarget::McpServer { index } => Some(index),
                    ReviewItemTarget::Static => None,
                };
                let Some(index) = index else {
                    return;
                };
                self.review_focus_anchor = row_key;
                self.section_draft = Some(draft);
                self.section_original = None;
                self.state.review_subfocus = 0;
                self.state.review_mcp_index = Some(index);
                self.field_errors.clear();
            }
            ReviewEditorKind::AutoCustom => {
                let ReviewTarget::OptionalSection(section) = group_target else {
                    return;
                };
                let Some(field) = field else {
                    return;
                };
                let draft = self.session().optional_draft(section);
                let configured = pages::optional_field_value(&draft, field);
                if configured != "auto" {
                    self.state.max_active_custom = configured.clone();
                }
                self.review_focus_anchor = row_key;
                self.section_draft = Some(draft);
                self.section_original = None;
                self.state.review_subfocus = usize::from(configured != "auto");
                self.state.review_mcp_index = None;
                self.state.review_mcp_create = false;
                self.field_errors.clear();
            }
            ReviewEditorKind::ReadOnly => {}
        }
    }

    fn return_to_review(&mut self) {
        let restore = self.review_focus_anchor.take();
        self.section_draft = None;
        self.section_original = None;
        self.state.editing = None;
        self.state.list_edit = None;
        self.state.mcp_edit = None;
        self.state.review_subfocus = 0;
        self.state.review_mcp_index = None;
        self.state.review_mcp_create = false;
        self.field_errors.clear();
        self.state.return_target = ReturnTarget::MainFlow;
        self.navigation.go_to(ConfigPage::Review);
        self.state.focus = 0;
        self.refresh_review();
        if self.state.page == ConfigPage::Review {
            self.state.focus = restore
                .and_then(|key| self.review.as_ref()?.find_row(key))
                .unwrap_or(0);
        }
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

    fn activate_review_confirmation(&mut self) {
        if self.review_preview_active() {
            self.confirm_and_write();
        } else {
            self.enter_review_preview();
        }
    }

    fn enter_review_preview(&mut self) {
        if self.state.page != ConfigPage::Review || self.state.finished {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        if let Err(errors) = session.validate_for_review() {
            self.record_errors(errors);
            return;
        }
        match session.redacted_config_json() {
            Ok(json) => {
                self.field_errors.clear();
                self.state.review_search.clear();
                self.state.review_search_active = false;
                self.state.review_preview_json = Some(json);
                self.state.review_preview_scroll = 0;
                self.state.review_preview_search.clear();
                self.state.review_preview_search_active = false;
            }
            Err(error) => self.set_system_error(&error),
        }
    }

    fn confirm_and_write(&mut self) {
        if self.state.page != ConfigPage::Review
            || !self.review_preview_active()
            || self.state.finished
        {
            return;
        }
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let config_path = session.config_path().to_path_buf();
        if let Err(errors) = session.validate_for_review() {
            self.leave_review_preview();
            self.record_errors(errors);
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
                self.state.finished = true;
                self.review = None;
                self.leave_review_preview();
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

fn optional_section_for_field(field: SetupField) -> crate::config_templates::OptionalSection {
    match field {
        SetupField::DisplayName => crate::config_templates::OptionalSection::Identity,
        SetupField::WorkspaceRoot
        | SetupField::WriteRoots
        | SetupField::ReadOnlyRoots
        | SetupField::DenyRoots => crate::config_templates::OptionalSection::Workspace,
        SetupField::ConfirmationChannels | SetupField::ConfirmationLanguage => {
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

    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession, WizardOutcome};
    use crate::config_templates::{InitSummary, RuntimeMode};

    use super::super::{Committer, ConfigPage, ConfigTuiApp, TuiAction};

    struct CountingCommitter {
        calls: Rc<Cell<usize>>,
    }

    impl Committer for CountingCommitter {
        fn commit(
            &mut self,
            config_path: &std::path::Path,
            outcome: WizardOutcome,
        ) -> anyhow::Result<InitSummary> {
            self.calls.set(self.calls.get() + 1);
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
    fn review_final_confirmation_commits_once() {
        let calls = Rc::new(Cell::new(0));
        let mut app = ConfigTuiApp::with_committer(
            SetupSession::new(
                SetupSeed {
                    mode: Some(RuntimeMode::Standalone),
                    tunnel_id: Some("review-test-tunnel".into()),
                    tunnel_api_key: Some("file:/tmp/review-test-secret".into()),
                    ..SetupSeed::default()
                },
                UiLanguage::En,
                PathBuf::from("/tmp/config-tui-review-commit-once.json"),
            ),
            Box::new(CountingCommitter {
                calls: calls.clone(),
            }),
        );
        while app.page() != ConfigPage::Review {
            app.handle_action(TuiAction::Next).unwrap();
        }
        app.state.focus = app.review_row_count();

        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(calls.get(), 0);

        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(calls.get(), 1);

        app.handle_action(TuiAction::Next).unwrap();
        assert_eq!(calls.get(), 1);
    }
}
