use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::Tab;

/// Actions the app can perform in response to key events.
#[derive(Debug)]
pub enum AppAction {
    Quit,
    SwitchTab(Tab),
    TabKey(KeyEvent),
}

/// Map a crossterm KeyEvent to an AppAction, considering the active tab and whether
/// a text input field has focus (suppresses global shortcuts when typing).
pub fn map_key(key: KeyEvent, _active_tab: Tab, input_focused: bool) -> Option<AppAction> {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(AppAction::Quit);
    }

    // When a text input has focus, pass everything through to the tab handler
    if input_focused {
        return Some(AppAction::TabKey(key));
    }

    match key.code {
        KeyCode::Char('q') => Some(AppAction::Quit),
        KeyCode::Char('1') => Some(AppAction::SwitchTab(Tab::Dashboard)),
        KeyCode::Char('2') => Some(AppAction::SwitchTab(Tab::Settings)),
        KeyCode::Char('3') => Some(AppAction::SwitchTab(Tab::Dtc)),
        KeyCode::Char('4') => Some(AppAction::SwitchTab(Tab::Scans)),
        KeyCode::Char('5') => Some(AppAction::SwitchTab(Tab::Backup)),
        KeyCode::Char('6') => Some(AppAction::SwitchTab(Tab::Raw)),
        _ => Some(AppAction::TabKey(key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_quit_key() {
        let action = map_key(key(KeyCode::Char('q')), Tab::Dashboard, false);
        assert!(matches!(action, Some(AppAction::Quit)));
    }

    #[test]
    fn test_tab_switch() {
        let action = map_key(key(KeyCode::Char('3')), Tab::Dashboard, false);
        assert!(matches!(action, Some(AppAction::SwitchTab(Tab::Dtc))));
    }

    #[test]
    fn test_input_focused_passes_through() {
        let action = map_key(key(KeyCode::Char('q')), Tab::Raw, true);
        assert!(matches!(action, Some(AppAction::TabKey(_))));
    }

    #[test]
    fn test_ctrl_c_always_quits() {
        let k = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let action = map_key(k, Tab::Raw, true);
        assert!(matches!(action, Some(AppAction::Quit)));
    }
}
