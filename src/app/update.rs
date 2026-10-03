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
    pub input_cursor: usize,
    pub transcript: Vec<TranscriptEntry>,
    pub status: Status,
    pub pending_approval: Option<PendingApproval>,
    pub transcript_scroll: u16,
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
        AppEvent::ScrollUp => state.transcript_scroll = state.transcript_scroll.saturating_sub(1),
        AppEvent::ScrollDown => state.transcript_scroll = state.transcript_scroll.saturating_add(1),
        AppEvent::Quit => state.should_exit = true,
    }
    state
}

fn update_key(state: &mut AppState, key: KeyEvent) {
    state.input_cursor = state.input_cursor.min(state.input.chars().count());
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_exit = true;
        }
        KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            let byte_index = cursor_byte_index(&state.input, state.input_cursor);
            state.input.insert(byte_index, character);
            state.input_cursor = state.input_cursor.saturating_add(1);
        }
        KeyCode::Backspace => {
            if state.input_cursor > 0 {
                let start = cursor_byte_index(&state.input, state.input_cursor - 1);
                let end = cursor_byte_index(&state.input, state.input_cursor);
                state.input.replace_range(start..end, "");
                state.input_cursor -= 1;
            }
        }
        KeyCode::Left => {
            state.input_cursor = state.input_cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            state.input_cursor = state
                .input_cursor
                .saturating_add(1)
                .min(state.input.chars().count());
        }
        KeyCode::Home => {
            state.input_cursor = 0;
        }
        KeyCode::End => {
            state.input_cursor = state.input.chars().count();
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
    state.input_cursor = 0;
    state.transcript.push(TranscriptEntry {
        speaker: Speaker::User,
        text,
    });
    state.status = Status::Working;
}

fn cursor_byte_index(input: &str, cursor: usize) -> usize {
    input
        .char_indices()
        .nth(cursor)
        .map_or(input.len(), |(byte_index, _)| byte_index)
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
        assert_eq!(state.input_cursor, 0);
        assert_eq!(
            state.transcript,
            [TranscriptEntry {
                speaker: Speaker::User,
                text: "hello".into()
            }]
        );
    }

    #[test]
    fn scroll_keys_move_the_transcript_viewport() {
        let state = AppState {
            transcript_scroll: 2,
            ..AppState::default()
        };

        let state = update(state, AppEvent::ScrollUp);
        assert_eq!(state.transcript_scroll, 1);

        let state = update(state, AppEvent::ScrollDown);
        assert_eq!(state.transcript_scroll, 2);
    }

    #[test]
    fn inserts_text_at_the_cursor_position() {
        let state = AppState {
            input: "ab".into(),
            input_cursor: 1,
            ..AppState::default()
        };

        let state = update(
            state,
            AppEvent::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        );

        assert_eq!(state.input, "axb");
        assert_eq!(state.input_cursor, 2);
    }

    #[test]
    fn backspace_removes_character_before_cursor() {
        let state = AppState {
            input: "axb".into(),
            input_cursor: 2,
            ..AppState::default()
        };

        let state = update(
            state,
            AppEvent::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );

        assert_eq!(state.input, "ab");
        assert_eq!(state.input_cursor, 1);
    }

    #[test]
    fn left_and_right_keys_move_cursor_within_input() {
        let state = AppState {
            input: "abc".into(),
            input_cursor: 1,
            ..AppState::default()
        };

        let state = update(
            state,
            AppEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        );
        assert_eq!(state.input_cursor, 2);

        let state = update(
            state,
            AppEvent::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        );
        assert_eq!(state.input_cursor, 1);
    }
}
