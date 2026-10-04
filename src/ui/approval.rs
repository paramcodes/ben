use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::{
    app::update::{AppState, ApprovalFocus},
    policy::approval::PendingApproval,
};

pub fn render(frame: &mut Frame<'_>, area: ratatui::layout::Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Approval required");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let compact = area.width < 50 || area.height < 14;
    let rows = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(if compact { 2 } else { 3 }),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(if compact {
            "Review this exact action before continuing."
        } else {
            "The action will wait until you approve it."
        }),
        rows[0],
    );

    if let Some(pending) = state.approval.pending() {
        let details = action_details(pending, compact);
        frame.render_widget(Paragraph::new(details).wrap(Wrap { trim: false }), rows[1]);
    } else {
        frame.render_widget(Paragraph::new("No action is pending."), rows[1]);
    }

    let controls = if area.width < 35 {
        Line::from(vec![Span::raw("a/r/c")])
    } else if compact {
        Line::from(vec![
            control(
                "a Approve",
                ApprovalFocus::ApproveOnce,
                state.approval_focus,
            ),
            Span::raw(" "),
            control("r Reject", ApprovalFocus::Reject, state.approval_focus),
            Span::raw(" "),
            control("c Cancel", ApprovalFocus::Cancel, state.approval_focus),
        ])
    } else {
        Line::from(vec![
            control(
                "[a] Approve once",
                ApprovalFocus::ApproveOnce,
                state.approval_focus,
            ),
            Span::raw("   "),
            control("[r] Reject", ApprovalFocus::Reject, state.approval_focus),
            Span::raw("   "),
            control("[c] Cancel", ApprovalFocus::Cancel, state.approval_focus),
        ])
    };
    frame.render_widget(Paragraph::new(controls), rows[2]);
    frame.render_widget(
        Paragraph::new(if compact {
            "Arrows move · Enter · Esc"
        } else {
            "←/→ choose · Enter confirm · Esc cancel"
        }),
        rows[3],
    );
}

fn control(label: &'static str, focus: ApprovalFocus, selected: ApprovalFocus) -> Span<'static> {
    if focus == selected {
        Span::styled(
            format!("> {label}"),
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(format!("  {label}"))
    }
}

fn action_details(pending: &PendingApproval, compact: bool) -> String {
    let action = pending.action();
    if compact {
        let arguments = action.arguments().as_object();
        let command = arguments
            .and_then(|args| args.get("command"))
            .map(compact_value);
        let target = arguments
            .and_then(|args| {
                [
                    "path",
                    "target",
                    "cwd",
                    "working_dir",
                    "working_directory",
                    "directory",
                ]
                .iter()
                .find_map(|key| args.get(*key))
            })
            .map(compact_value);
        let mut details = vec![format!("Tool: {}", action.tool())];
        if let Some(command) = command {
            details.push(format!("Command: {}", narrow_value(&command)));
        }
        if let Some(target) = target {
            details.push(format!("Target: {}", narrow_value(&target)));
        }
        if let Some(arguments) = arguments {
            let other: serde_json::Map<String, serde_json::Value> = arguments
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "command"
                            | "path"
                            | "target"
                            | "cwd"
                            | "working_dir"
                            | "working_directory"
                            | "directory"
                    )
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            if !other.is_empty() {
                let serialized = serde_json::to_string(&other).unwrap_or_else(|_| "{}".into());
                details.push(format!("Args: {}", narrow_value(&serialized)));
            }
        }
        details.push("Resize for full arguments.".into());
        return details.join("\n");
    }
    let mut details = vec![format!("Tool: {}", action.tool())];
    if let Some(arguments) = action.arguments().as_object() {
        let mut other = serde_json::Map::new();
        for (key, value) in arguments {
            match key.as_str() {
                "command" => details.push(format!("Command: {}", compact_value(value))),
                "path" | "target" | "cwd" | "working_dir" | "working_directory" | "directory" => {
                    details.push(format!("Target ({key}): {}", compact_value(value)))
                }
                _ => {
                    other.insert(key.clone(), value.clone());
                }
            }
        }
        if !other.is_empty() {
            let arguments = serde_json::to_string(&other).unwrap_or_else(|_| "{}".into());
            details.push(format!("Arguments: {arguments}"));
        }
    }
    details.join("\n")
}

fn narrow_value(value: &str) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(21).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn compact_value(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_else(|_| "<invalid>".into()))
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::{
        app::{
            event::AppEvent,
            update::{AppState, ApprovalFocus, Screen, update},
        },
        policy::approval::{ApprovalDecision, PendingAction},
    };

    fn key(code: KeyCode) -> AppEvent {
        AppEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn pending_state() -> AppState {
        let action = PendingAction::new(
            "run_command",
            serde_json::json!({
                "command": "cargo test --lib",
                "path": "src/agent/turn.rs",
                "args": ["--offline", "--locked"]
            }),
        )
        .unwrap();
        update(AppState::default(), AppEvent::ApprovalRequested(action))
    }

    fn rendered(width: u16, height: u16) -> String {
        let state = pending_state();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn standard_layout_shows_action_details_and_all_decisions() {
        let output = rendered(80, 24);
        for detail in [
            "run_command",
            "cargo test --lib",
            "src/agent/turn.rs",
            "--offline",
            "Approve once",
            "Reject",
            "Cancel",
        ] {
            assert!(
                output.contains(detail),
                "missing {detail:?} in rendered approval prompt: {output}"
            );
        }
    }

    #[test]
    fn narrow_layout_keeps_action_identity_and_safe_controls_visible() {
        let output = rendered(36, 12);
        assert!(output.contains("run_command"));
        assert!(output.contains("Cancel"));
        assert!(output.contains("Approve"));
    }

    #[test]
    fn focus_marker_visible_without_color() {
        let output = rendered(80, 24);
        assert!(
            output.contains("> [c] Cancel"),
            "focused choice must have a text marker: {output}"
        );
    }

    #[test]
    fn right_arrow_cycles_focus_through_all_choices() {
        let mut state = pending_state();
        assert_eq!(state.approval_focus, ApprovalFocus::Cancel);

        state = update(state, key(KeyCode::Right));
        assert_eq!(state.approval_focus, ApprovalFocus::ApproveOnce);

        state = update(state, key(KeyCode::Right));
        assert_eq!(state.approval_focus, ApprovalFocus::Reject);

        state = update(state, key(KeyCode::Right));
        assert_eq!(state.approval_focus, ApprovalFocus::Cancel);
    }

    #[test]
    fn left_arrow_cycles_focus_through_all_choices() {
        let mut state = pending_state();
        assert_eq!(state.approval_focus, ApprovalFocus::Cancel);

        state = update(state, key(KeyCode::Left));
        assert_eq!(state.approval_focus, ApprovalFocus::Reject);

        state = update(state, key(KeyCode::Left));
        assert_eq!(state.approval_focus, ApprovalFocus::ApproveOnce);
    }

    #[test]
    fn enter_confirms_focused_approval() {
        let mut state = pending_state();
        state.approval_focus = ApprovalFocus::ApproveOnce;

        let state = update(state, AppEvent::Submit);
        assert!(
            state
                .approval
                .resolution()
                .as_ref()
                .is_some_and(|r| r.decision == ApprovalDecision::ApproveOnce),
            "focused approval must be recorded"
        );
    }

    #[test]
    fn escape_records_cancel_and_returns_to_conversation() {
        let state = pending_state();
        let state = update(state, AppEvent::Cancel);

        assert_eq!(state.screen, Screen::Conversation);
        assert!(
            state
                .approval
                .resolution()
                .as_ref()
                .is_some_and(|r| r.decision == ApprovalDecision::Cancel),
            "Esc must record a Cancel resolution"
        );
    }
}
