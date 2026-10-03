use crate::providers::types::{ProviderError, ProviderEvent};
use crossterm::event::KeyEvent;

/// Input and internal messages that can change application state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    Key(KeyEvent),
    Submit,
    ScrollUp,
    ScrollDown,
    AppendOutput(String),
    ToolStatus(String),
    ProviderStarted,
    ProviderEvent(ProviderEvent),
    ProviderFailed(ProviderError),
    ProviderCancelled,
    Quit,
}

pub fn map_key(key: KeyEvent) -> Option<AppEvent> {
    use crossterm::event::{KeyCode, KeyModifiers};

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(AppEvent::Quit),
        KeyCode::Enter => Some(AppEvent::Submit),
        KeyCode::Up => Some(AppEvent::ScrollUp),
        KeyCode::Down => Some(AppEvent::ScrollDown),
        KeyCode::Char(_)
        | KeyCode::Backspace
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Home
        | KeyCode::End => Some(AppEvent::Key(key)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{AppEvent, map_key};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn maps_enter_to_submit() {
        let event = map_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        assert_eq!(event, Some(AppEvent::Submit));
    }

    #[test]
    fn maps_ctrl_c_to_quit() {
        let event = map_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert_eq!(event, Some(AppEvent::Quit));
    }

    #[test]
    fn maps_text_keys_and_scroll_keys() {
        assert!(matches!(
            map_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(AppEvent::Key(_))
        ));
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(AppEvent::ScrollUp)
        );
        assert_eq!(
            map_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Some(AppEvent::ScrollDown)
        );
    }
}
