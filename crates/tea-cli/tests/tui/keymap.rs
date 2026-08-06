use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tea_cli::tui::{BindingAction, KeyMap};
use tea_coding::config::TuiSettings;

#[test]
fn defaults_cover_idle_and_running_actions() {
    let map = KeyMap::from_settings(&TuiSettings::default()).unwrap();
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), false),
        Some(BindingAction::Submit)
    );
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), false),
        Some(BindingAction::Newline)
    );
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), true),
        Some(BindingAction::Steer)
    );
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), true),
        Some(BindingAction::FollowUp)
    );
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), true),
        Some(BindingAction::Abort)
    );
}

#[test]
fn validated_configuration_overrides_bindings_and_rejects_ambiguity() {
    let mut settings = TuiSettings {
        submit_key: "ctrl+enter".to_owned(),
        ..TuiSettings::default()
    };
    let map = KeyMap::from_settings(&settings).unwrap();
    assert_eq!(
        map.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), false),
        Some(BindingAction::Submit)
    );

    settings.newline_key = "ctrl+enter".to_owned();
    assert!(KeyMap::from_settings(&settings).is_err());
    settings.newline_key = "not+a+key".to_owned();
    assert!(KeyMap::from_settings(&settings).is_err());

    let settings = TuiSettings {
        abort_key: "enter".to_owned(),
        ..TuiSettings::default()
    };
    assert!(KeyMap::from_settings(&settings).is_err());
}
