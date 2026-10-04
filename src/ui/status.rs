use crate::app::update::{AppState, Status};

pub fn label(status: &Status) -> String {
    match status {
        Status::Ready => "Ready".to_owned(),
        Status::Working => "Working".to_owned(),
        Status::Tool(description) => format!("Tool: {description}"),
        Status::Connecting => "Connecting".to_owned(),
        Status::Streaming => "Streaming".to_owned(),
        Status::Busy => "Busy — finish or cancel the current turn".to_owned(),
        Status::Completed => "Completed".to_owned(),
        Status::Failed => "Failed".to_owned(),
        Status::Cancelled => "Cancelled".to_owned(),
    }
}

/// Returns a one-line summary of the last turn: model, usage, elapsed time,
/// and the suggested next step on failure. Returns `None` when no summary
/// data is available.
pub fn summary(state: &AppState) -> Option<String> {
    let label = label(&state.status);
    if state.status == Status::Completed {
        let model = state
            .model
            .as_deref()
            .map(|m| format!("model:{m}"))
            .unwrap_or_default();
        let usage = state
            .usage
            .map(|u| format!("in:{} out:{}", u.input_tokens, u.output_tokens))
            .unwrap_or_else(|| "no usage".to_owned());
        let elapsed = state
            .elapsed_ms
            .map(|ms| format!("{:.1}s", ms as f64 / 1000.0))
            .unwrap_or_else(|| "—".to_owned());
        let parts: Vec<&str> = [label.as_str(), &model, &usage, &elapsed]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect();
        Some(parts.join(" | "))
    } else if state.status == Status::Failed {
        let next = state
            .error_next_step
            .as_deref()
            .map(|s| format!("Try: {s}"))
            .unwrap_or_default();
        let parts: Vec<&str> = [label.as_str(), next.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect();
        Some(parts.join(" | "))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Status, summary};
    use crate::app::update::AppState;

    #[test]
    fn absent_usage_is_shown_when_no_metadata_reported() {
        let state = AppState {
            status: Status::Completed,
            model: Some("test-model".into()),
            usage: None,
            elapsed_ms: Some(500),
            ..AppState::default()
        };
        let s = summary(&state).expect("a completed turn has a summary");
        assert!(
            s.contains("no usage"),
            "missing usage must be explicit: {s}"
        );
        assert!(s.contains("test-model"));
        assert!(s.contains("0.5s"));
    }

    #[test]
    fn provider_error_shows_next_step() {
        let state = AppState {
            status: Status::Failed,
            error_next_step: Some("check your network and retry".into()),
            ..AppState::default()
        };
        let s = summary(&state).expect("a failed turn has a summary");
        assert!(s.contains("Failed"), "failure must show the status: {s}");
        assert!(
            s.contains("check your network and retry"),
            "next step must appear: {s}"
        );
    }

    #[test]
    fn tool_failures_are_visible_in_summary() {
        let state = AppState {
            status: Status::Failed,
            error_next_step: Some("review the tool output".into()),
            ..AppState::default()
        };
        let s = summary(&state).unwrap();
        assert!(s.contains("review the tool output"));
    }

    #[test]
    fn successful_changes_include_model_and_usage() {
        let state = AppState {
            status: Status::Completed,
            model: Some("gpt-4".into()),
            usage: Some(crate::providers::types::Usage {
                input_tokens: 10,
                output_tokens: 5,
            }),
            elapsed_ms: Some(1200),
            changed_files: vec!["src/app/update.rs".into()],
            ..AppState::default()
        };
        let s = summary(&state).unwrap();
        assert!(s.contains("gpt-4"), "model must appear: {s}");
        assert!(s.contains("in:10 out:5"), "usage must appear: {s}");
        assert!(s.contains("1.2s"), "elapsed must appear: {s}");
    }

    #[test]
    fn absent_summary_is_clean() {
        let state = AppState::default();
        assert!(summary(&state).is_none(), "ready state has no summary");
    }
}
