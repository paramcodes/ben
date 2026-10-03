use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::update::{AppState, Speaker};

pub fn render_transcript(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let lines = state
        .transcript
        .iter()
        .flat_map(|entry| {
            let (speaker, color) = match entry.speaker {
                Speaker::User => ("You", Color::Green),
                Speaker::Assistant => ("Agent", Color::Cyan),
                Speaker::Tool => ("Tool", Color::Yellow),
            };
            entry
                .text
                .split('\n')
                .enumerate()
                .map(move |(index, text)| {
                    if index == 0 {
                        Line::from(vec![
                            Span::styled(
                                format!("{speaker}: "),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(text.to_owned()),
                        ])
                    } else {
                        Line::from(vec![Span::raw("  "), Span::raw(text.to_owned())])
                    }
                })
        })
        .collect::<Vec<_>>();

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Conversation"))
        .wrap(Wrap { trim: false })
        .scroll((state.transcript_scroll, 0));
    frame.render_widget(paragraph, area);
}

pub fn render_prompt(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let block = Block::default().borders(Borders::ALL).title("Prompt");
    let inner = block.inner(area);
    frame.render_widget(
        Paragraph::new(state.input.as_str())
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );

    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let cursor = state.input_cursor.min(state.input.chars().count());
    let width = usize::from(inner.width);
    let row = cursor / width;
    let column = cursor % width;
    frame.set_cursor_position(Position::new(
        inner.x + column as u16,
        inner.y + row.min(usize::from(inner.height - 1)) as u16,
    ));
}
