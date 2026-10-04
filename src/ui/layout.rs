use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::update::{AppState, Screen};

use super::{approval, clear, conversation, status};

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
    if state.screen == Screen::Approval {
        approval::render(frame, area, state);
        return;
    }
    if state.screen == Screen::ConfirmClear {
        clear::render(frame, area, state.clear_prompt.as_ref());
        return;
    }
    let compact = area.width < 60 || area.height < 16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(if compact { 3 } else { 4 }),
            Constraint::Length(1),
        ])
        .split(area);

    let header = Paragraph::new(Line::from(vec![
        Span::styled("BEN", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(
            status::label(&state.status),
            Style::default().fg(Color::Yellow),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Terminal Agent"),
    );
    frame.render_widget(header, chunks[0]);

    conversation::render_transcript(frame, chunks[1], state);
    conversation::render_prompt(frame, chunks[2], state);

    let help = if compact {
        "Enter send · Ctrl+C quit"
    } else {
        "Enter: send    Ctrl+C: quit"
    };
    frame.render_widget(Paragraph::new(help), chunks[3]);
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::app::update::AppState;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_state(width: u16, height: u16, state: &AppState) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn renders_required_areas_at_standard_size() {
        let rendered = render_state(80, 24, &AppState::default());

        assert!(rendered.contains("BEN"));
        assert!(rendered.contains("Ready"));
        assert!(rendered.contains("Prompt"));
        assert!(rendered.contains("Ctrl+C"));
    }

    #[test]
    fn renders_compact_layout_in_narrow_terminal() {
        let rendered = render_state(40, 12, &AppState::default());

        assert!(rendered.contains("BEN"));
        assert!(rendered.contains("Ready"));
    }

    #[test]
    fn renders_wide_terminal_without_panicking() {
        let rendered = render_state(160, 50, &AppState::default());

        assert!(rendered.contains("BEN"));
        assert!(rendered.contains("Prompt"));
        assert!(rendered.contains("Ctrl+C"));
    }

    #[test]
    fn renders_multiline_messages_on_separate_lines() {
        use crate::app::update::{Speaker, TranscriptEntry};

        let state = AppState {
            transcript: vec![TranscriptEntry {
                speaker: Speaker::Assistant,
                text: "first line\nsecond line".into(),
            }],
            ..AppState::default()
        };

        let rendered = render_state(80, 24, &state);

        assert!(rendered.contains("Agent: first line"));
        assert!(rendered.contains("second line"));
    }

    #[test]
    fn shows_prompt_cursor_at_the_input_position() {
        let state = AppState {
            input: "abc".into(),
            input_cursor: 2,
            ..AppState::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        terminal.draw(|frame| render(frame, &state)).unwrap();

        assert!(terminal.backend().cursor_visible());
        assert_eq!(terminal.backend().cursor_position().x, 3);
    }
}
