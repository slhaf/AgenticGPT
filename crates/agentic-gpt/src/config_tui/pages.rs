use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::config_setup::{SetupField, SetupSession};
use crate::config_templates::{RuntimeMode, TunnelSecretSource};
use crate::tui::{
    render_action_button, render_footer, render_header, render_inline_error, render_radio_row,
    render_text_input, Theme,
};

use super::{ConfigPage, TuiState};

pub(super) fn render(
    frame: &mut Frame,
    page: ConfigPage,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    progress: (usize, usize),
) {
    match page {
        ConfigPage::Basic => render_basic(frame, session, state, theme, errors, progress),
        ConfigPage::Connection => render_connection(frame, session, state, theme, errors, progress),
        ConfigPage::OptionalCenter => {
            render_placeholder(frame, "Optional settings", state, theme, progress)
        }
        ConfigPage::Optional(_) => {
            render_placeholder(frame, "Optional section", state, theme, progress)
        }
        ConfigPage::Review => render_placeholder(frame, "Review", state, theme, progress),
        ConfigPage::Completion => render_placeholder(frame, "Done", state, theme, progress),
    }
}

fn render_basic(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    progress: (usize, usize),
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        "AgenticGPT config init",
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    let mode = session.selected_mode();
    let profile = session.selected_profile();
    let [mode_row, profile_row, error_row] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(body);
    render_radio_row(
        frame,
        mode_row,
        &format!("Runtime mode  {mode:?}"),
        true,
        state.focus == 0,
        theme,
    );
    render_radio_row(
        frame,
        profile_row,
        &format!("Profile       {}", format!("{profile:?}").to_lowercase()),
        true,
        state.focus == 1,
        theme,
    );
    if let Some(error) = errors.get(&SetupField::Mode) {
        render_inline_error(frame, error_row, error, theme);
    }
    render_action_button(frame, actions, "Next", state.focus >= 2, theme);
    render_footer(
        frame,
        footer,
        "Enter confirm · Tab move · Ctrl+C cancel",
        theme,
    );
}

fn render_connection(
    frame: &mut Frame,
    session: &SetupSession,
    state: &TuiState,
    theme: &Theme,
    errors: &HashMap<SetupField, String>,
    progress: (usize, usize),
) {
    let [header, body, actions, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(6),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        "Connection settings",
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    let fields = connection_fields_for_session(session);
    let row_height = 1u16;
    let constraints = std::iter::repeat_n(Constraint::Length(row_height), fields.len())
        .chain(std::iter::once(Constraint::Min(1)))
        .collect::<Vec<_>>();
    let rows = Layout::vertical(constraints).split(body);
    for (index, field) in fields.iter().enumerate() {
        let focused = state.focus == index;
        let value = connection_value(session, *field);
        if matches!(
            field,
            SetupField::TunnelSecretSource | SetupField::ProvisionTunnelSecret
        ) {
            render_radio_row(
                frame,
                rows[index],
                &connection_label(*field, value.as_deref()),
                value.as_deref() == Some("true"),
                focused,
                theme,
            );
        } else {
            render_text_input(
                frame,
                rows[index],
                &connection_label(*field, value.as_deref()),
                value.as_deref().unwrap_or_default(),
                focused,
                matches!(
                    field,
                    SetupField::TunnelSecretValue | SetupField::AgentSecret
                ),
                theme,
            );
        }
    }
    if let Some((_, error)) = errors.iter().next() {
        render_inline_error(frame, *rows.last().unwrap_or(&body), error, theme);
    }
    render_action_button(frame, actions, "Next", state.focus >= fields.len(), theme);
    render_footer(
        frame,
        footer,
        "Enter edit · Esc back · Ctrl+C cancel",
        theme,
    );
}

pub(super) fn connection_fields_for_session(session: &SetupSession) -> Vec<SetupField> {
    match session.selected_mode() {
        RuntimeMode::Standalone => {
            let mut fields = vec![SetupField::TunnelId, SetupField::TunnelSecretSource];
            match session.standalone().secret_source {
                TunnelSecretSource::File => {
                    fields.push(SetupField::TunnelSecretPath);
                    fields.push(SetupField::ProvisionTunnelSecret);
                    if session.standalone().provision_secret_now {
                        fields.push(SetupField::TunnelSecretValue);
                    }
                }
                TunnelSecretSource::Environment => {
                    fields.push(SetupField::TunnelSecretEnvironment);
                }
            }
            fields
        }
        RuntimeMode::Hub => vec![
            SetupField::HubUrl,
            SetupField::HubTransport,
            SetupField::AgentId,
            SetupField::AgentSecret,
        ],
        RuntimeMode::Local => Vec::new(),
    }
}

fn connection_label(field: SetupField, value: Option<&str>) -> String {
    match field {
        SetupField::TunnelId => "Tunnel ID".to_string(),
        SetupField::TunnelSecretSource => format!("Secret source: {}", value.unwrap_or("file")),
        SetupField::TunnelSecretPath => "Secret file".to_string(),
        SetupField::TunnelSecretEnvironment => "Secret environment".to_string(),
        SetupField::ProvisionTunnelSecret => "Provision secret now".to_string(),
        SetupField::TunnelSecretValue => "Secret value".to_string(),
        SetupField::HubUrl => "Hub URL".to_string(),
        SetupField::HubTransport => "Transport".to_string(),
        SetupField::AgentId => "Agent ID".to_string(),
        SetupField::AgentSecret => "Agent Secret".to_string(),
        _ => format!("{field:?}"),
    }
}

pub(super) fn connection_value(session: &SetupSession, field: SetupField) -> Option<String> {
    match field {
        SetupField::TunnelId => Some(session.standalone().tunnel_id.clone()),
        SetupField::TunnelSecretSource => Some(match session.standalone().secret_source {
            TunnelSecretSource::File => "file".to_string(),
            TunnelSecretSource::Environment => "env".to_string(),
        }),
        SetupField::TunnelSecretPath => Some(session.standalone().secret_path.clone()),
        SetupField::TunnelSecretEnvironment => {
            Some(session.standalone().secret_environment.clone())
        }
        SetupField::ProvisionTunnelSecret => {
            Some(session.standalone().provision_secret_now.to_string())
        }
        SetupField::TunnelSecretValue => session
            .standalone()
            .secret_value
            .as_ref()
            .map(|secret| secret.expose().to_string()),
        SetupField::HubUrl => Some(session.hub().hub_url.clone()),
        SetupField::HubTransport => Some(session.hub().hub_transport.clone()),
        SetupField::AgentId => Some(session.hub().agent_id.clone()),
        SetupField::AgentSecret => session
            .hub()
            .agent_secret
            .as_ref()
            .map(|secret| secret.expose().to_string()),
        _ => None,
    }
}

fn render_placeholder(
    frame: &mut Frame,
    title: &str,
    state: &TuiState,
    theme: &Theme,
    progress: (usize, usize),
) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(
        frame,
        header,
        title,
        &format!("{} / {}", progress.0, progress.1),
        theme,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Next phase: ", theme.dim),
            Span::styled("staged navigation surface", theme.normal),
        ])),
        body,
    );
    render_footer(frame, footer, "Esc back · Ctrl+C cancel", theme);
    let _ = state;
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::cli_i18n::UiLanguage;
    use crate::config_setup::{SetupSeed, SetupSession};
    use crate::config_templates::RuntimeMode;
    use crate::WorkerProfile;

    use super::super::{ConfigPage, ConfigTuiApp};

    fn content(app: &ConfigTuiApp, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn basic_page_renders_mode_profile_and_footer() {
        let app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                profile: Some(WorkerProfile::Normal),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        let rendered = content(&app, 70, 20);
        assert!(rendered.contains("Runtime mode"));
        assert!(rendered.contains("Standalone"));
        assert!(rendered.contains("normal"));
        assert!(rendered.contains("Ctrl+C"));
    }

    #[test]
    fn progress_header_matches_dynamic_mode_flow() {
        let standalone = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        assert!(content(&standalone, 70, 20).contains("1 / 4"));

        let local = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Local),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        assert!(content(&local, 70, 20).contains("1 / 3"));

        let mut hub = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        hub.handle_action(super::super::TuiAction::Next).unwrap();
        assert!(content(&hub, 70, 20).contains("2 / 4"));
    }

    #[test]
    fn resize_rerender_preserves_page_and_staged_values() {
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                tunnel_id: Some("resize-staged-tunnel".into()),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        let wide = content(&app, 70, 20);
        let narrow = content(&app, 48, 12);
        assert!(wide.contains("resize-staged-tunnel"));
        assert!(narrow.contains("resize-staged-tunnel"));
        assert_eq!(app.page(), ConfigPage::Connection);
        assert_eq!(app.session().standalone().tunnel_id, "resize-staged-tunnel");
    }

    #[test]
    fn connection_pages_are_conditional_and_secret_is_not_rendered() {
        let marker = "connection-secret-marker";
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Hub),
                agent_secret: Some(crate::config_templates::SecretValue::new(marker)),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        let rendered = content(&app, 70, 20);
        assert!(rendered.contains("Hub URL"));
        assert!(rendered.contains("Agent Secret"));
        assert!(!rendered.contains(marker));

        let mut local = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Local),
                ..SetupSeed::default()
            },
            UiLanguage::ZhCn,
            "/tmp/config-tui-render.json".into(),
        ));
        local.handle_action(super::super::TuiAction::Next).unwrap();
        assert_ne!(local.page(), ConfigPage::Connection);
    }

    #[test]
    fn standalone_secret_value_is_only_visible_when_provisioning() {
        let mut app = ConfigTuiApp::new(SetupSession::new(
            SetupSeed {
                mode: Some(RuntimeMode::Standalone),
                ..SetupSeed::default()
            },
            UiLanguage::En,
            "/tmp/config-tui-render.json".into(),
        ));
        app.handle_action(super::super::TuiAction::Next).unwrap();
        assert!(!content(&app, 70, 20).contains("Secret value"));

        app.focus_field(crate::config_setup::SetupField::ProvisionTunnelSecret);
        app.handle_action(super::super::TuiAction::Activate)
            .unwrap();
        assert!(content(&app, 70, 20).contains("Secret value"));
    }
}
