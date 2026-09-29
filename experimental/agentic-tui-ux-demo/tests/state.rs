use agentic_tui_ux_demo::{AppState, Focus, Mode, Profile};

#[test]
fn moving_focus_does_not_change_selected_mode_until_confirmed() {
    let mut app = AppState::default();
    assert_eq!(app.mode, Mode::Standalone);
    assert_eq!(app.focus, Focus::Mode(0));

    app.move_down();

    assert_eq!(app.focus, Focus::Mode(1));
    assert_eq!(app.mode, Mode::Standalone);

    app.confirm();
    assert_eq!(app.mode, Mode::Hub);
}

#[test]
fn focus_moves_through_modes_profiles_and_next() {
    let mut app = AppState::default();
    for expected in [
        Focus::Mode(1),
        Focus::Mode(2),
        Focus::Profile(0),
        Focus::Profile(1),
        Focus::Next,
        Focus::Mode(0),
    ] {
        app.move_down();
        assert_eq!(app.focus, expected);
    }
}

#[test]
fn confirming_profile_changes_only_profile() {
    let mut app = AppState::default();
    app.focus = Focus::Profile(1);
    app.confirm();
    assert_eq!(app.profile, Profile::Room);
    assert_eq!(app.mode, Mode::Standalone);
}

#[test]
fn border_toggle_is_independent_of_selection() {
    let mut app = AppState::default();
    let before = app.border;
    app.toggle_border();
    assert_ne!(app.border, before);
    assert_eq!(app.mode, Mode::Standalone);
    assert_eq!(app.profile, Profile::Normal);
}
