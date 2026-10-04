use crate::app::update::TranscriptEntry;
use crate::policy::approval::{ActionFingerprint, ApprovalDecision, PendingAction};
use crate::providers::types::{ProviderError, ProviderEvent, Usage};
use crate::sessions::model::ToolCallSummary;
use crate::tools::propose_edit::ProposedEdit;
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
    ApprovalRequested(PendingAction),
    ApprovalDecision {
        fingerprint: ActionFingerprint,
        decision: ApprovalDecision,
    },
    EditProposed(ProposedEdit),
    /// The user asked to clear a stored session; the terminal must confirm it
    /// before the file is removed.
    ClearRequested {
        id: String,
    },
    /// The user confirmed or rejected clearing a stored session.
    ClearResolved {
        id: String,
        confirmed: bool,
    },
    /// A turn completed with summary information for the UI.
    TurnComplete {
        model: String,
        usage: Option<Usage>,
        elapsed_ms: u64,
        tool_outcomes: Vec<ToolCallSummary>,
        changed_files: Vec<String>,
    },
    /// A turn failed; `next_step` suggests what the user can do.
    TurnFailed {
        error: String,
        next_step: String,
    },
    /// The stored conversation was loaded before the first frame.
    SessionRestored(Vec<TranscriptEntry>),
    Cancel,
    Interrupt,
    Quit,
}

pub fn map_key(key: KeyEvent) -> Option<AppEvent> {
    use crossterm::event::{KeyCode, KeyModifiers};

    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(AppEvent::Interrupt)
        }
        KeyCode::Esc => Some(AppEvent::Cancel),
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
    fn maps_ctrl_c_to_interrupt() {
        let event = map_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert_eq!(event, Some(AppEvent::Interrupt));
    }

    #[test]
    fn maps_escape_to_cancel() {
        let event = map_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert_eq!(event, Some(AppEvent::Cancel));
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
