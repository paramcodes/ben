use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tools::propose_edit::ProposedEdit;

pub fn render(frame: &mut Frame<'_>, area: ratatui::layout::Rect, edit: &ProposedEdit) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Proposed edit");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let compact = area.width < 50 || area.height < 12;
    let rows = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let header = if compact {
        format!("Edit {}", truncate(&edit.path, 28))
    } else {
        format!("Review changes for {}", edit.path)
    };
    frame.render_widget(Paragraph::new(header), rows[0]);

    let lines: Vec<Line<'_>> = edit
        .unified_diff
        .lines()
        .map(|line| style_diff_line(line, compact))
        .collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[1]);
}

fn style_diff_line(line: &str, compact: bool) -> Line<'static> {
    let display = if compact {
        truncate(line, 40)
    } else {
        line.to_owned()
    };
    let style = if line.starts_with("+++") || line.starts_with("---") {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Magenta)
    } else if line.starts_with('+') {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    Line::from(Span::styled(display, style))
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars.saturating_sub(1)).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::{
        policy::approval::PendingAction,
        tools::propose_edit::{ProposedEdit, unified_diff},
    };

    fn sample_edit() -> ProposedEdit {
        let unified_diff = unified_diff(
            "src/agent/turn.rs",
            "fn ready() {}\nfn go() {}\n",
            "fn ready() {}\nfn run() {}\n",
        );
        ProposedEdit {
            path: "src/agent/turn.rs".into(),
            unified_diff,
            pending_action: PendingAction::new(
                "propose_edit",
                serde_json::json!({
                    "path": "src/agent/turn.rs",
                    "expected_content": "fn ready() {}\nfn go() {}\n",
                    "new_content": "fn ready() {}\nfn run() {}\n",
                }),
            )
            .unwrap(),
        }
    }

    fn rendered(width: u16, height: u16) -> String {
        let edit = sample_edit();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &edit))
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
    fn standard_layout_shows_path_and_hunk_markers() {
        let output = rendered(80, 24);
        for detail in [
            "src/agent/turn.rs",
            "--- a/src/agent/turn.rs",
            "+++ b/src/agent/turn.rs",
            "@@",
            "-fn go() {}",
            "+fn run() {}",
        ] {
            assert!(
                output.contains(detail),
                "missing {detail:?} in rendered diff: {output}"
            );
        }
    }

    #[test]
    fn narrow_layout_keeps_path_identity_visible() {
        let output = rendered(36, 10);
        assert!(output.contains("turn.rs") || output.contains("src/agent"));
        assert!(output.contains("Proposed edit") || output.contains("Edit"));
    }
}
