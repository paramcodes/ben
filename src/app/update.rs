use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::event::AppEvent;
use crate::policy::approval::{ApprovalDecision, ApprovalState};
use crate::providers::types::ProviderEvent;
use crate::sessions::model::SessionMessage;
use crate::sessions::model::SessionRecord;
use crate::tools::propose_edit::ProposedEdit;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Conversation,
    Approval,
    ConfirmClear,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ApprovalFocus {
    ApproveOnce,
    Reject,
    #[default]
    Cancel,
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

/// A stored session the user is being asked to delete. The id travels with the
/// decision so a stale confirmation cannot delete a different session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearPrompt {
    pub id: String,
    pub focused: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Ready,
    Working,
    Tool(String),
    Connecting,
    Streaming,
    Busy,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    pub screen: Screen,
    pub input: String,
    pub input_cursor: usize,
    pub transcript: Vec<TranscriptEntry>,
    pub status: Status,
    pub approval: ApprovalState,
    pub approval_focus: ApprovalFocus,
    pub pending_edit: Option<ProposedEdit>,
    pub transcript_scroll: u16,
    pub should_exit: bool,
    pub cancel_requested: bool,
    /// The session whose deletion is awaiting confirmation, if any.
    pub clear_prompt: Option<ClearPrompt>,
    /// A session the user chose to restore, kept so it can be saved again.
    pub resumed_session_id: Option<String>,
    /// One-shot outcome of a clear confirmation, consumed by the caller.
    pub cleared_session_id: Option<String>,
}

/// Apply one event to state. This function performs no terminal I/O.
pub fn update(mut state: AppState, event: AppEvent) -> AppState {
    match event {
        AppEvent::Key(key) => {
            if state.screen == Screen::Approval {
                update_approval_key(&mut state, key);
            } else if state.screen == Screen::ConfirmClear {
                update_clear_key(&mut state, key);
            } else {
                update_key(&mut state, key);
            }
        }
        AppEvent::Submit => {
            if state.screen == Screen::Approval {
                decide_focused_approval(&mut state);
            } else if state.screen == Screen::ConfirmClear {
                let focused = state.clear_prompt.as_ref().is_some_and(|p| p.focused);
                resolve_clear(&mut state, focused);
            } else if is_busy(&state.status) {
                state.status = Status::Busy;
            } else {
                submit(&mut state);
            }
        }
        AppEvent::AppendOutput(text) => state.transcript.push(TranscriptEntry {
            speaker: Speaker::Assistant,
            text,
        }),
        AppEvent::ToolStatus(status) => state.status = Status::Tool(status),
        AppEvent::ProviderStarted => {
            state.status = Status::Connecting;
            state.cancel_requested = false;
        }
        AppEvent::ProviderEvent(event) => match event {
            ProviderEvent::TextDelta(text) => {
                state.status = Status::Streaming;
                if let Some(entry) = state
                    .transcript
                    .last_mut()
                    .filter(|entry| entry.speaker == Speaker::Assistant)
                {
                    entry.text.push_str(&text);
                } else {
                    state.transcript.push(TranscriptEntry {
                        speaker: Speaker::Assistant,
                        text,
                    });
                }
            }
            ProviderEvent::Completed(_) => {
                state.status = Status::Completed;
                state.cancel_requested = false;
            }
            ProviderEvent::ToolCallStarted { .. }
            | ProviderEvent::ToolCallArgumentsDelta { .. }
            | ProviderEvent::ToolCallCompleted(_)
            | ProviderEvent::Usage(_) => state.status = Status::Streaming,
        },
        AppEvent::ProviderFailed(_) => {
            state.status = Status::Failed;
            state.cancel_requested = false;
        }
        AppEvent::ProviderCancelled => {
            state.status = Status::Cancelled;
            state.cancel_requested = false;
        }
        AppEvent::ApprovalRequested(action) => {
            state.approval.request(action);
            state.screen = Screen::Approval;
            state.approval_focus = ApprovalFocus::Cancel;
        }
        AppEvent::ApprovalDecision {
            fingerprint,
            decision,
        } => {
            if state.approval.decide(fingerprint, decision).is_ok() {
                state.screen = Screen::Conversation;
                state.pending_edit = None;
            }
        }
        AppEvent::EditProposed(edit) => {
            state.pending_edit = Some(edit);
        }
        AppEvent::SessionRestored(entries) => {
            state.transcript.extend(entries);
            state.status = Status::Ready;
        }
        AppEvent::ClearRequested { id } => {
            state.clear_prompt = Some(ClearPrompt { id, focused: false });
            state.screen = Screen::ConfirmClear;
        }
        AppEvent::ClearResolved { id, confirmed } => {
            if state
                .clear_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.id == id)
            {
                state.clear_prompt = None;
                state.screen = Screen::Conversation;
                state.status = Status::Ready;
                if confirmed {
                    state.cleared_session_id = Some(id);
                } else {
                    state.transcript.push(TranscriptEntry {
                        speaker: Speaker::Assistant,
                        text: format!("Kept session {id}."),
                    });
                }
            }
        }
        AppEvent::Cancel => {
            if state.screen == Screen::ConfirmClear {
                cancel_clear(&mut state);
            } else if state.screen == Screen::Approval {
                resolve_approval(&mut state, ApprovalDecision::Cancel);
            } else if is_busy(&state.status) {
                state.cancel_requested = true;
            }
        }
        AppEvent::Interrupt => {
            if is_busy(&state.status) {
                state.cancel_requested = true;
            } else {
                state.should_exit = true;
            }
        }
        AppEvent::ScrollUp => state.transcript_scroll = state.transcript_scroll.saturating_sub(1),
        AppEvent::ScrollDown => state.transcript_scroll = state.transcript_scroll.saturating_add(1),
        AppEvent::Quit => state.should_exit = true,
    }
    state
}

fn is_busy(status: &Status) -> bool {
    matches!(
        status,
        Status::Working | Status::Connecting | Status::Streaming | Status::Busy
    )
}

fn resolve_approval(state: &mut AppState, decision: ApprovalDecision) {
    let Some(fingerprint) = state
        .approval
        .pending()
        .map(|pending| pending.fingerprint())
    else {
        return;
    };
    if state.approval.decide(fingerprint, decision).is_ok() {
        state.screen = Screen::Conversation;
        state.pending_edit = None;
    }
}

fn decide_focused_approval(state: &mut AppState) {
    let decision = match state.approval_focus {
        ApprovalFocus::ApproveOnce => ApprovalDecision::ApproveOnce,
        ApprovalFocus::Reject => ApprovalDecision::Reject,
        ApprovalFocus::Cancel => ApprovalDecision::Cancel,
    };
    resolve_approval(state, decision);
}

/// Keys for the clear confirmation. Deletion is destructive, so the safe
/// option is focused by default and Enter alone cannot delete a session.
fn update_clear_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') => resolve_clear(state, true),
        KeyCode::Char('n') => resolve_clear(state, false),
        KeyCode::Char('c') | KeyCode::Esc => cancel_clear(state),
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
            if let Some(prompt) = state.clear_prompt.as_mut() {
                prompt.focused = !prompt.focused;
            }
        }
        KeyCode::Enter => resolve_clear(
            state,
            state
                .clear_prompt
                .as_ref()
                .is_some_and(|prompt| prompt.focused),
        ),
        _ => {}
    }
}

fn resolve_clear(state: &mut AppState, confirmed: bool) {
    let Some(id) = state.clear_prompt.as_ref().map(|prompt| prompt.id.clone()) else {
        return;
    };
    *state = update(state.clone(), AppEvent::ClearResolved { id, confirmed });
}

fn cancel_clear(state: &mut AppState) {
    resolve_clear(state, false);
}

/// Turns a stored conversation into transcript entries. Tool calls and results
/// become tool lines so a resumed session reads in the original order.
pub fn transcript_from_session(record: &SessionRecord) -> Vec<TranscriptEntry> {
    record
        .messages
        .iter()
        .map(|message| match message {
            SessionMessage::User { text } => TranscriptEntry {
                speaker: Speaker::User,
                text: text.clone(),
            },
            SessionMessage::Assistant { text } => TranscriptEntry {
                speaker: Speaker::Assistant,
                text: text.clone(),
            },
            SessionMessage::ToolCall {
                tool, arguments, ..
            } => TranscriptEntry {
                speaker: Speaker::Tool,
                text: format!("{tool}({arguments})"),
            },
            SessionMessage::ToolResult { content, .. } => TranscriptEntry {
                speaker: Speaker::Tool,
                text: content.clone(),
            },
        })
        .collect()
}

fn update_approval_key(state: &mut AppState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('a') => resolve_approval(state, ApprovalDecision::ApproveOnce),
        KeyCode::Char('r') => resolve_approval(state, ApprovalDecision::Reject),
        KeyCode::Char('c') | KeyCode::Esc => resolve_approval(state, ApprovalDecision::Cancel),
        KeyCode::Left | KeyCode::Up => {
            state.approval_focus = match state.approval_focus {
                ApprovalFocus::ApproveOnce => ApprovalFocus::Cancel,
                ApprovalFocus::Reject => ApprovalFocus::ApproveOnce,
                ApprovalFocus::Cancel => ApprovalFocus::Reject,
            };
        }
        KeyCode::Right | KeyCode::Down => {
            state.approval_focus = match state.approval_focus {
                ApprovalFocus::ApproveOnce => ApprovalFocus::Reject,
                ApprovalFocus::Reject => ApprovalFocus::Cancel,
                ApprovalFocus::Cancel => ApprovalFocus::ApproveOnce,
            };
        }
        KeyCode::Home => state.approval_focus = ApprovalFocus::ApproveOnce,
        KeyCode::End => state.approval_focus = ApprovalFocus::Cancel,
        KeyCode::Enter => decide_focused_approval(state),
        _ => {}
    }
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
    use crate::{
        app::{
            event::AppEvent,
            update::{
                AppState, ApprovalFocus, Screen, Speaker, Status, TranscriptEntry,
                transcript_from_session,
            },
        },
        policy::approval::{ApprovalDecision, PendingAction},
        providers::types::{ProviderError, ProviderEvent},
        sessions::model::{SessionMessage, SessionRecord},
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn record() -> SessionRecord {
        SessionRecord::new(
            "session-1",
            "test-model",
            1_700_000_000_000,
            vec![
                SessionMessage::User {
                    text: "update the notes".into(),
                },
                SessionMessage::Assistant {
                    text: "updated notes.txt".into(),
                },
            ],
            Vec::new(),
        )
        .unwrap()
    }

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
    fn stale_approval_decisions_do_not_apply_to_changed_actions() {
        let first =
            PendingAction::new("run_command", serde_json::json!({"command":"cargo test"})).unwrap();
        let changed =
            PendingAction::new("run_command", serde_json::json!({"command":"cargo check"}))
                .unwrap();
        let mut state = update(AppState::default(), AppEvent::ApprovalRequested(first));
        let stale = state.approval.pending().unwrap().fingerprint();
        state = update(state, AppEvent::ApprovalRequested(changed.clone()));
        let current = state.approval.pending().unwrap().fingerprint();
        assert_ne!(stale, current);

        state = update(
            state,
            AppEvent::ApprovalDecision {
                fingerprint: stale,
                decision: ApprovalDecision::ApproveOnce,
            },
        );

        assert_eq!(state.screen, super::Screen::Approval);
        assert_eq!(state.approval.pending().unwrap().action(), &changed);
        assert!(state.approval.resolution().is_none());
    }

    #[test]
    fn approval_decision_is_consumed_once_by_the_reducer() {
        let action =
            PendingAction::new("run_command", serde_json::json!({"command":"cargo test"})).unwrap();
        let mut state = update(AppState::default(), AppEvent::ApprovalRequested(action));
        let fingerprint = state.approval.pending().unwrap().fingerprint();
        let decision = AppEvent::ApprovalDecision {
            fingerprint,
            decision: ApprovalDecision::ApproveOnce,
        };
        state = update(state, decision.clone());
        assert_eq!(state.screen, super::Screen::Conversation);
        assert_eq!(
            state.approval.resolution().unwrap().decision,
            ApprovalDecision::ApproveOnce
        );

        state = update(state, decision);
        assert_eq!(
            state.approval.resolution().unwrap().decision,
            ApprovalDecision::ApproveOnce
        );
    }

    #[test]
    fn approval_shortcuts_choose_each_one_shot_decision() {
        for (key, expected) in [
            ('a', ApprovalDecision::ApproveOnce),
            ('r', ApprovalDecision::Reject),
            ('c', ApprovalDecision::Cancel),
        ] {
            let action =
                PendingAction::new("run_command", serde_json::json!({"command":"cargo test"}))
                    .unwrap();
            let state = update(AppState::default(), AppEvent::ApprovalRequested(action));
            let state = update(
                state,
                AppEvent::Key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE)),
            );
            assert_eq!(state.approval.resolution().unwrap().decision, expected);
            assert_eq!(state.screen, super::Screen::Conversation);
        }
    }

    #[test]
    fn approval_enter_defaults_to_cancel_and_focus_can_move_to_reject() {
        let action =
            PendingAction::new("run_command", serde_json::json!({"command":"cargo test"})).unwrap();
        let state = update(AppState::default(), AppEvent::ApprovalRequested(action));
        assert_eq!(state.approval_focus, ApprovalFocus::Cancel);
        let state = update(state, AppEvent::Submit);
        assert_eq!(
            state.approval.resolution().unwrap().decision,
            ApprovalDecision::Cancel
        );

        let action =
            PendingAction::new("run_command", serde_json::json!({"command":"cargo test"})).unwrap();
        let state = update(AppState::default(), AppEvent::ApprovalRequested(action));
        let state = update(
            state,
            AppEvent::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        );
        assert_eq!(state.approval_focus, ApprovalFocus::Reject);
        let state = update(state, AppEvent::Submit);
        assert_eq!(
            state.approval.resolution().unwrap().decision,
            ApprovalDecision::Reject
        );
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

    #[test]
    fn provider_text_deltas_append_in_order_to_one_assistant_entry() {
        let state = update(AppState::default(), AppEvent::ProviderStarted);
        assert_eq!(state.status, Status::Connecting);

        let state = update(
            state,
            AppEvent::ProviderEvent(ProviderEvent::TextDelta("hello".into())),
        );
        assert_eq!(state.status, Status::Streaming);
        let state = update(
            state,
            AppEvent::ProviderEvent(ProviderEvent::TextDelta(" world".into())),
        );
        let state = update(
            state,
            AppEvent::ProviderEvent(ProviderEvent::Completed(
                crate::providers::types::CompletionReason::EndTurn,
            )),
        );

        assert_eq!(state.status, Status::Completed);
        assert_eq!(
            state.transcript,
            [TranscriptEntry {
                speaker: Speaker::Assistant,
                text: "hello world".into()
            }]
        );
    }

    #[test]
    fn provider_error_sets_failed_status_and_retains_prior_output() {
        let state = AppState {
            transcript: vec![TranscriptEntry {
                speaker: Speaker::Assistant,
                text: "partial answer".into(),
            }],
            ..AppState::default()
        };

        let state = update(state, AppEvent::ProviderFailed(ProviderError::Transport));

        assert_eq!(state.status, Status::Failed);
        assert_eq!(state.transcript[0].text, "partial answer");
    }

    #[test]
    fn provider_cancellation_sets_cancelled_status() {
        let state = update(AppState::default(), AppEvent::ProviderCancelled);

        assert_eq!(state.status, Status::Cancelled);
    }

    #[test]
    fn submit_while_busy_keeps_prompt_and_sets_visible_busy_status() {
        let state = AppState {
            input: "another question".into(),
            status: Status::Streaming,
            ..AppState::default()
        };

        let state = update(state, AppEvent::Submit);

        assert_eq!(state.status, Status::Busy);
        assert_eq!(state.input, "another question");
        assert_eq!(state.transcript.len(), 0);
    }

    #[test]
    fn cancel_requests_stop_for_active_turn_and_interrupt_quits_when_idle() {
        let state = AppState {
            status: Status::Streaming,
            ..AppState::default()
        };
        let state = update(state, AppEvent::Cancel);
        assert!(state.cancel_requested);
        assert!(!state.should_exit);

        let state = update(AppState::default(), AppEvent::Interrupt);
        assert!(state.should_exit);
    }

    #[test]
    fn a_stored_session_becomes_a_transcript_before_the_first_frame() {
        let state = update(
            AppState::default(),
            AppEvent::SessionRestored(transcript_from_session(&record())),
        );

        assert_eq!(
            state.transcript,
            [
                TranscriptEntry {
                    speaker: Speaker::User,
                    text: "update the notes".into()
                },
                TranscriptEntry {
                    speaker: Speaker::Assistant,
                    text: "updated notes.txt".into()
                }
            ]
        );
        assert_eq!(state.status, Status::Ready);
        assert!(state.clear_prompt.is_none());
    }

    #[test]
    fn tool_activity_is_restored_as_tool_lines_in_order() {
        let record = SessionRecord::new(
            "session-1",
            "test-model",
            1_700_000_000_000,
            vec![
                SessionMessage::ToolCall {
                    call_id: "call-1".into(),
                    tool: "read_file".into(),
                    arguments: r#"{"path":"notes.txt"}"#.into(),
                },
                SessionMessage::ToolResult {
                    call_id: "call-1".into(),
                    content: "contents".into(),
                },
            ],
            Vec::new(),
        )
        .unwrap();

        let entries = transcript_from_session(&record);

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.speaker == Speaker::Tool));
        assert!(
            entries[0].text.starts_with("read_file("),
            "{}",
            entries[0].text
        );
        assert_eq!(entries[1].text, "contents");
    }

    #[test]
    fn clearing_a_session_requires_a_confirmation_and_default_is_the_safe_choice() {
        let state = update(
            AppState::default(),
            AppEvent::ClearRequested {
                id: "session-1".into(),
            },
        );

        assert_eq!(state.screen, Screen::ConfirmClear);
        let prompt = state.clear_prompt.as_ref().expect("a prompt is pending");
        assert_eq!(prompt.id, "session-1");
        assert!(!prompt.focused, "deletion must not be the focused default");
        assert!(state.cleared_session_id.is_none());

        let state = update(state, AppEvent::Submit);
        assert_eq!(
            state.cleared_session_id, None,
            "Enter alone must not delete a session"
        );
        assert_eq!(state.screen, Screen::Conversation);
    }

    #[test]
    fn confirming_a_clear_reports_the_session_once() {
        let state = update(
            AppState::default(),
            AppEvent::ClearRequested {
                id: "session-1".into(),
            },
        );

        let state = update(state, key(KeyCode::Char('y')));

        assert_eq!(state.cleared_session_id.as_deref(), Some("session-1"));
        assert!(state.clear_prompt.is_none());
        assert_eq!(state.screen, Screen::Conversation);
    }

    #[test]
    fn rejecting_a_clear_keeps_the_session_and_says_so() {
        let state = update(
            AppState::default(),
            AppEvent::ClearRequested {
                id: "session-1".into(),
            },
        );

        let state = update(state, key(KeyCode::Char('n')));

        assert_eq!(state.cleared_session_id, None);
        assert!(state.clear_prompt.is_none());
        assert!(
            state
                .transcript
                .iter()
                .any(|entry| entry.text.contains("Kept session session-1")),
            "{:?}",
            state.transcript
        );
    }

    #[test]
    fn escape_cancels_a_clear_without_deleting() {
        let state = update(
            AppState::default(),
            AppEvent::ClearRequested {
                id: "session-1".into(),
            },
        );

        let state = update(state, AppEvent::Cancel);

        assert_eq!(state.cleared_session_id, None);
        assert!(state.clear_prompt.is_none());
    }

    #[test]
    fn a_stale_confirmation_cannot_delete_a_different_session() {
        let state = update(
            AppState::default(),
            AppEvent::ClearRequested {
                id: "session-1".into(),
            },
        );

        let state = update(
            state,
            AppEvent::ClearResolved {
                id: "session-2".into(),
                confirmed: true,
            },
        );

        assert_eq!(state.cleared_session_id, None);
        assert!(
            state.clear_prompt.is_some(),
            "a mismatched id must not clear the pending prompt"
        );
    }

    #[test]
    fn the_clear_prompt_does_not_swallow_conversation_keystrokes_after_resolving() {
        let state = update(
            AppState {
                input: "ab".into(),
                input_cursor: 2,
                ..AppState::default()
            },
            AppEvent::ClearRequested {
                id: "session-1".into(),
            },
        );

        let state = update(state, key(KeyCode::Char('n')));
        let state = update(state, key(KeyCode::Char('c')));

        assert_eq!(state.input, "abc");
        assert_eq!(state.input_cursor, 3);
    }
}
