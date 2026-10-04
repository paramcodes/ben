use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::update::ClearPrompt;

/// Renders the confirmation shown before a stored session is deleted. The
/// session identity and both outcomes stay visible in a narrow terminal so the
/// choice is never ambiguous.
pub fn render(frame: &mut Frame<'_>, area: ratatui::layout::Rect, prompt: Option<&ClearPrompt>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Clear saved session");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let compact = area.width < 50 || area.height < 14;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(if compact { 2 } else { 3 }),
            Constraint::Length(1),
        ])
        .split(inner);

    let explanation = match prompt {
        Some(prompt) => format!(
            "Delete the stored session {}? This cannot be undone.",
            prompt.id
        ),
        None => "No session is waiting to be cleared.".to_owned(),
    };
    frame.render_widget(
        Paragraph::new(explanation).wrap(Wrap { trim: false }),
        rows[0],
    );

    let focused = prompt.is_some_and(|prompt| prompt.focused);
    let controls = if compact {
        Line::from(vec![
            control("[y] Delete", focused),
            Span::raw("  "),
            control("[n] Keep", !focused),
        ])
    } else {
        Line::from(vec![
            control("[y] Delete session", focused),
            Span::raw("    "),
            control("[n] Keep session", !focused),
        ])
    };
    frame.render_widget(Paragraph::new(controls).wrap(Wrap { trim: false }), rows[1]);
    frame.render_widget(
        Paragraph::new(if compact {
            "y delete · n keep"
        } else {
            "y deletes the stored session · n or Esc keeps it"
        }),
        rows[2],
    );
}

fn control(label: &'static str, selected: bool) -> Span<'static> {
    if selected {
        Span::styled(
            label,
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(label)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::app::{
        event::AppEvent,
        update::{AppState, update},
    };

    fn prompting(focused: bool) -> AppState {
        let mut state = update(
            AppState::default(),
            AppEvent::ClearRequested {
                id: "2026-10-04-notes".into(),
            },
        );
        if let Some(prompt) = state.clear_prompt.as_mut() {
            prompt.focused = focused;
        }
        state
    }

    fn rendered(width: u16, height: u16, state: &AppState) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state.clear_prompt.as_ref()))
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
    fn names_the_session_and_offers_both_outcomes() {
        let output = rendered(80, 24, &prompting(false));

        for detail in ["2026-10-04-notes", "Delete", "Keep", "cannot be undone"] {
            assert!(
                output.contains(detail),
                "missing {detail:?} in clear prompt: {output}"
            );
        }
    }

    #[test]
    fn keeps_both_choices_visible_in_a_narrow_terminal() {
        let output = rendered(36, 12, &prompting(true));

        assert!(output.contains("2026-10-04"));
        assert!(output.contains("Delete"));
        assert!(output.contains("Keep"));
    }
}
