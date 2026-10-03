use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::update::{AppState, Speaker};

use super::status;

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let area = frame.area();
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

    let transcript = state
        .transcript
        .iter()
        .map(|entry| {
            let speaker = match entry.speaker {
                Speaker::User => "You",
                Speaker::Assistant => "Agent",
                Speaker::Tool => "Tool",
            };
            Line::from(format!("{speaker}: {}", entry.text))
        })
        .collect::<Vec<_>>();
    let transcript = Paragraph::new(transcript)
        .block(Block::default().borders(Borders::ALL).title("Conversation"))
        .wrap(Wrap { trim: false });
    frame.render_widget(transcript, chunks[1]);

    let prompt = Paragraph::new(state.input.as_str())
        .block(Block::default().borders(Borders::ALL).title("Prompt"))
        .wrap(Wrap { trim: false });
    frame.render_widget(prompt, chunks[2]);

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

    fn render_text(width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, &AppState::default()))
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
    fn renders_required_areas_at_standard_size() {
        let rendered = render_text(80, 24);

        assert!(rendered.contains("BEN"));
        assert!(rendered.contains("Ready"));
        assert!(rendered.contains("Prompt"));
        assert!(rendered.contains("Ctrl+C"));
    }

    #[test]
    fn renders_compact_layout_in_narrow_terminal() {
        let rendered = render_text(40, 12);

        assert!(rendered.contains("BEN"));
        assert!(rendered.contains("Ready"));
    }

    #[test]
    fn renders_wide_terminal_without_panicking() {
        let rendered = render_text(160, 50);

        assert!(rendered.contains("BEN"));
        assert!(rendered.contains("Prompt"));
        assert!(rendered.contains("Ctrl+C"));
    }
}
