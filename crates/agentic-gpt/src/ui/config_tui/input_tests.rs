use super::*;
use crate::config_setup::SetupSeed;
use crate::config_templates::OptionalSection;

fn app() -> ConfigTuiApp {
    ConfigTuiApp::new(SetupSession::new(
        SetupSeed {
            mode: Some(RuntimeMode::Standalone),
            profile: Some(WorkerProfile::Room),
            tunnel_id: Some("input-editor-test".into()),
            tunnel_api_key: Some("env:INPUT_TEST_TUNNEL_KEY".into()),
            http_mcp_bearer_token: Some(SecretValue::new("env:INPUT_TEST_HTTP_TOKEN")),
            http_mcp_allow_hosts: Some("null".into()),
            ..SetupSeed::default()
        },
        UiLanguage::En,
        std::env::temp_dir().join(format!("config-input-{}.json", uuid::Uuid::new_v4())),
    ))
}

fn key(app: &mut ConfigTuiApp, code: KeyCode) {
    app.handle_event(TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap();
}

fn text(app: &mut ConfigTuiApp, value: &str) {
    for character in value.chars() {
        key(app, KeyCode::Char(character));
    }
}

fn review(app: &mut ConfigTuiApp) {
    for _ in 0..3 {
        app.handle_action(TuiAction::Next).unwrap();
    }
    assert_eq!(app.page(), ConfigPage::Review);
}

#[test]
fn typed_mail_form_selections_and_numbers_survive_review() {
    let mut app = app();
    app.handle_action(TuiAction::Next).unwrap();
    app.handle_action(TuiAction::Next).unwrap();
    app.state.focus = app
        .session()
        .available_optional_sections()
        .iter()
        .position(|section| *section == OptionalSection::Email)
        .unwrap();
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.page(), ConfigPage::Optional(OptionalSection::Email));

    app.focus_field(SetupField::EmailSendPolicy);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.editing().is_none());
    app.focus_field(SetupField::EmailImapTls);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert!(app.editing().is_none());

    app.focus_field(SetupField::EmailSmtpPort);
    key(&mut app, KeyCode::Enter);
    for _ in 0..3 {
        key(&mut app, KeyCode::Backspace);
    }
    text(&mut app, "70000x");
    assert_eq!(app.editing().unwrap().buffer, "70000");
    key(&mut app, KeyCode::Enter);
    assert!(app.field_errors.contains_key(&SetupField::EmailSmtpPort));
    assert!(!app.field_errors.contains_key(&SetupField::EmailEnabled));
    key(&mut app, KeyCode::Enter);
    for _ in 0..5 {
        key(&mut app, KeyCode::Backspace);
    }
    text(&mut app, "465");
    key(&mut app, KeyCode::Enter);

    app.focus_field(SetupField::EmailSyncInterval);
    key(&mut app, KeyCode::Enter);
    for _ in 0..2 {
        key(&mut app, KeyCode::Backspace);
    }
    text(&mut app, "600x");
    key(&mut app, KeyCode::Enter);
    app.handle_action(TuiAction::Next).unwrap();
    app.handle_action(TuiAction::Next).unwrap();
    assert_eq!(app.page(), ConfigPage::Review);
    let draft = app.session().optional_draft(OptionalSection::Email);
    assert_eq!(
        pages::optional_field_value(&draft, SetupField::EmailSendPolicy),
        "confirm"
    );
    assert_eq!(
        pages::optional_field_value(&draft, SetupField::EmailImapTls),
        "starttls"
    );
    assert_eq!(
        pages::optional_field_value(&draft, SetupField::EmailSyncInterval),
        "600"
    );

    app.focus_field(SetupField::EmailSendPolicy);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Char('x'));
    key(&mut app, KeyCode::Enter);
    let OptionalSectionDraft::Email(draft) = app.session().optional_draft(OptionalSection::Email)
    else {
        panic!()
    };
    assert_eq!(
        draft.to_config().unwrap().send_policy,
        crate::config::EmailPolicy::Deny
    );
}

#[test]
fn host_modes_and_list_changes_apply_through_both_editing_surfaces() {
    let mut app = app();
    app.handle_action(TuiAction::Next).unwrap();
    app.focus_field(SetupField::HttpMcpAllowHosts);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.session().standalone().http_mcp_allow_hosts, "[\"*\"]");
    app.handle_action(TuiAction::Next).unwrap();
    app.handle_action(TuiAction::Next).unwrap();
    assert_eq!(app.page(), ConfigPage::Review);

    app.focus_field(SetupField::HttpMcpAllowHosts);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Insert);
    text(&mut app, "first.example");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Insert);
    text(&mut app, "second.example:443");
    key(&mut app, KeyCode::Enter);
    // The selected list value must remain visible while editing a populated modal.
    for (width, height) in [(80, 24), (40, 10), (12, 4)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| app.render(frame, &Theme::from_env()))
            .unwrap();
        if width == 80 {
            let screen = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(screen.contains("second.example:443"));
        }
    }
    // Tab to Apply must not edit the previous row or trap keyboard focus.
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    assert!(app.structured_editor.is_none());
    let hosts: Vec<String> =
        serde_json::from_str(&app.session().standalone().http_mcp_allow_hosts).unwrap();
    assert_eq!(hosts, ["first.example", "second.example:443"]);

    app.focus_field(SetupField::HttpMcpAllowHosts);
    key(&mut app, KeyCode::Enter);
    // Select the second row using Tab, delete it, then cancel the whole edit.
    for _ in 0..4 {
        key(&mut app, KeyCode::Tab);
    }
    key(&mut app, KeyCode::Delete);
    key(&mut app, KeyCode::Esc);
    let retained: Vec<String> =
        serde_json::from_str(&app.session().standalone().http_mcp_allow_hosts).unwrap();
    assert_eq!(retained, hosts);

    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('a'));
    assert_eq!(app.session().standalone().http_mcp_allow_hosts, "null");
}

#[test]
fn invalid_structured_input_can_be_cancelled_without_changing_the_draft() {
    let mut app = app();
    app.handle_action(TuiAction::Next).unwrap();
    app.focus_field(SetupField::HttpMcpAllowHosts);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Insert);
    text(&mut app, "http://invalid.example/path");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('a'));
    assert!(app.structured_editor.is_some());
    assert!(app
        .field_errors
        .contains_key(&SetupField::HttpMcpAllowHosts));
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.session().standalone().http_mcp_allow_hosts, "null");
    assert!(app.structured_editor.is_none());
}

#[test]
fn reference_editor_switches_sources_clears_and_keeps_secrets_out_of_preview() {
    let mut app = app();
    review(&mut app);
    app.focus_field(SetupField::HttpMcpBearerToken);
    key(&mut app, KeyCode::Enter);
    // Existing env reference -> file source, then enter a path.
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Tab);
    key(&mut app, KeyCode::Enter);
    text(&mut app, "/tmp/input-test-token");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('a'));
    assert!(app.structured_editor.is_none());
    assert_eq!(
        app.session()
            .standalone()
            .http_mcp_bearer_token
            .as_ref()
            .unwrap()
            .expose(),
        "file:/tmp/input-test-token"
    );
    let preview = app.session().redacted_config_json().unwrap();
    assert!(!preview.contains("/tmp/input-test-token"));

    app.focus_field(SetupField::HttpMcpBearerToken);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Char('c'));
    assert!(app.session().standalone().http_mcp_bearer_token.is_none());
    assert!(app.structured_editor.is_none());
    let final_config: serde_json::Value =
        serde_json::from_str(&app.session().redacted_config_json().unwrap()).unwrap();
    assert!(final_config["httpMcp"]["bearerToken"]
        .as_str()
        .is_none_or(str::is_empty));
}
