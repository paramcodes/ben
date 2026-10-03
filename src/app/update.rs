use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::event::AppEvent;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Conversation,
    Approval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Speaker {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub speaker: Speaker,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Ready,
    Working,
    Tool(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingApproval {
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    pub screen: Screen,
    pub input: String,
    pub transcript: Vec<TranscriptEntry>,
    pub status: Status,
    pub pending_approval: Option<PendingApproval>,
    pub should_exit: bool,
}

/// Apply one event to state. This function performs no terminal I/O.
pub fn update(mut state: AppState, event: AppEvent) -> AppState {
    match event {
        AppEvent::Key(key) => update_key(&mut state, key),
        AppEvent::Submit => submit(&mut state),
        AppEvent::AppendOutput(text) => state.transcript.push(TranscriptEntry {
            speaker: Speaker::Assistant,
            text,
        }),
        AppEvent::ToolStatus(status) => state.status = Status::Tool(status),
        AppEvent::Quit => state.should_exit = true,
    }
    state
}

fn update_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
        }
        KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.input.push(character);
        }
        KeyCode::Backspace => {
            state.input.pop();
        }
        KeyCode::Enter => submit(state),
        _ => {}
    }
}

fn submit(state: &mut AppState) {
    if state.input.trim().is_empty() {
        return;
    }

    let text = std::mem::take(&mut state.input);
    state.transcript.push(TranscriptEntry {
        speaker: Speaker::User,
        text,
    });
    state.status = Status::Working;
}

#[cfg(test)]
mod tests {
    use super::update;
    use crate::app::{
        event::AppEvent,
        update::{AppState, Speaker, Status, TranscriptEntry},
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn submit_moves_input_to_transcript_and_sets_working_status() {
        let state = AppState {
            input: "fix the bug".into(),
            ..AppState::default()
        };

        let state = update(state, AppEvent::Submit);

        assert_eq!(state.input, "");
        assert_eq!(state.transcript.len(), 1);
        assert_eq!(state.transcript[0].speaker, Speaker::User);
        assert_eq!(state.transcript[0].text, "fix the bug");
        assert_eq!(state.status, Status::Working);
    }

    #[test]
    fn append_output_adds_assistant_transcript_entry() {
        let state = update(AppState::default(), AppEvent::AppendOutput("Done.".into()));

        assert_eq!(state.transcript.len(), 1);
        assert_eq!(state.transcript[0].speaker, Speaker::Assistant);
        assert_eq!(state.transcript[0].text, "Done.");
    }

    #[test]
    fn tool_status_updates_status_without_terminal_io() {
        let state = update(
            AppState::default(),
            AppEvent::ToolStatus("Reading files".into()),
        );

        assert_eq!(state.status, Status::Tool("Reading files".into()));
    }

    #[test]
    fn quit_event_marks_state_for_exit() {
        let state = update(AppState::default(), AppEvent::Quit);

        assert!(state.should_exit);
    }

    #[test]
    fn key_input_appends_to_prompt() {
        let state = update(
            AppState::default(),
            AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        );

        assert_eq!(state.input, "x");
    }

    #[test]
    fn enter_key_submits_prompt() {
        let state = AppState {
            input: "hello".into(),
            ..AppState::default()
        };

        let state = update(
            state,
            AppEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(state.input, "");
        assert_eq!(
            state.transcript,
            [TranscriptEntry {
                speaker: Speaker::User,
                text: "hello".into()
            }]
        );
    }
}
