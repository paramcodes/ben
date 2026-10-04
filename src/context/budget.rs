use super::instructions::InstructionContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextItem {
    pub source: String,
    pub content: String,
}

/// Token estimators should be deterministic and monotonic as text is extended.
pub trait TokenEstimator {
    fn estimate_tokens(&self, text: &str) -> usize;
}

/// Conservative fallback: charge one estimated token per UTF-8 byte.
/// This may substantially underfill a budget, but avoids assuming language-specific token ratios.
#[derive(Debug, Clone, Copy, Default)]
pub struct ByteFallbackEstimator;
impl TokenEstimator for ByteFallbackEstimator {
    fn estimate_tokens(&self, text: &str) -> usize {
        text.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoundedContext {
    pub content: String,
    pub included_sources: Vec<String>,
    pub truncated_sources: Vec<String>,
    pub omitted_sources: Vec<String>,
    pub estimated_tokens: usize,
}

/// Assemble applicable repository instructions before caller-selected task context.
pub fn assemble_workspace_context(
    instructions: &InstructionContext,
    relevant_context: &[ContextItem],
    token_budget: usize,
    estimator: &dyn TokenEstimator,
) -> BoundedContext {
    let mut items: Vec<_> = instructions
        .files
        .iter()
        .map(|file| ContextItem {
            source: format!("AGENTS.md ({})", file.path),
            content: file.content.clone(),
        })
        .collect();
    items.extend_from_slice(relevant_context);
    let mut bounded = assemble_context(&items, token_budget, estimator);
    for file in &instructions.files {
        if file.truncated {
            let source = format!("AGENTS.md ({})", file.path);
            if bounded.included_sources.contains(&source)
                && !bounded.truncated_sources.contains(&source)
            {
                bounded.truncated_sources.push(source);
            }
        }
    }
    bounded
}

const DISCLOSURE: &str = "\n[Context omitted or truncated by budget.]";

pub fn assemble_context(
    items: &[ContextItem],
    token_budget: usize,
    estimator: &dyn TokenEstimator,
) -> BoundedContext {
    let full = render_all(items);
    if estimator.estimate_tokens(&full) <= token_budget {
        return BoundedContext {
            content: full,
            included_sources: items.iter().map(|item| item.source.clone()).collect(),
            estimated_tokens: estimator.estimate_tokens(&render_all(items)),
            ..BoundedContext::default()
        };
    }

    let mut result = BoundedContext::default();
    for (index, item) in items.iter().enumerate() {
        let section = render_item(item, &item.content);
        let mut candidate = result.content.clone();
        candidate.push_str(&section);
        candidate.push_str(DISCLOSURE);
        if estimator.estimate_tokens(&candidate) <= token_budget {
            result.content.push_str(&section);
            result.included_sources.push(item.source.clone());
            continue;
        }

        let later_sources: Vec<_> = items[index + 1..]
            .iter()
            .map(|later| later.source.clone())
            .collect();
        let disclosure = fitting_disclosure(&result.content, token_budget, estimator);
        let prefix =
            largest_fitting_prefix(&result.content, item, &disclosure, token_budget, estimator);
        if let Some(prefix) = prefix {
            result.content.push_str(&render_item(item, &prefix));
            result.truncated_sources.push(item.source.clone());
        } else {
            result.omitted_sources.push(item.source.clone());
        }
        result.omitted_sources.extend(later_sources);
        result.content.push_str(&disclosure);
        result.estimated_tokens = estimator.estimate_tokens(&result.content);
        return result;
    }

    result
}

fn render_all(items: &[ContextItem]) -> String {
    items
        .iter()
        .map(|item| render_item(item, &item.content))
        .collect()
}

fn render_item(item: &ContextItem, content: &str) -> String {
    format!("### {}\n{}\n", item.source, content)
}

fn largest_fitting_prefix(
    current: &str,
    item: &ContextItem,
    disclosure: &str,
    budget: usize,
    estimator: &dyn TokenEstimator,
) -> Option<String> {
    let mut boundaries: Vec<usize> = item
        .content
        .char_indices()
        .map(|(index, _)| index)
        .collect();
    boundaries.push(item.content.len());
    let mut low = 0usize;
    let mut high = boundaries.len();
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        let prefix = &item.content[..boundaries[middle - 1]];
        let mut candidate = current.to_owned();
        candidate.push_str(&render_item(item, prefix));
        candidate.push_str(disclosure);
        if estimator.estimate_tokens(&candidate) <= budget {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    if low == 0 {
        return None;
    }
    let prefix = item.content[..boundaries[low - 1]].to_owned();
    (!prefix.is_empty()).then_some(prefix)
}

fn fitting_disclosure(current: &str, budget: usize, estimator: &dyn TokenEstimator) -> String {
    let mut boundaries: Vec<usize> = DISCLOSURE.char_indices().map(|(index, _)| index).collect();
    boundaries.push(DISCLOSURE.len());
    for boundary in boundaries.into_iter().rev() {
        let note = &DISCLOSURE[..boundary];
        let candidate = format!("{current}{note}");
        if estimator.estimate_tokens(&candidate) <= budget {
            return note.to_owned();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::{
        ByteFallbackEstimator, ContextItem, InstructionContext, TokenEstimator, assemble_context,
        assemble_workspace_context,
    };

    struct CharacterEstimator;
    impl TokenEstimator for CharacterEstimator {
        fn estimate_tokens(&self, text: &str) -> usize {
            text.chars().count()
        }
    }

    #[test]
    fn budget_truncation_is_deterministic_and_discloses_omissions() {
        let items = vec![
            ContextItem {
                source: "instructions".into(),
                content: "Keep changes focused.".into(),
            },
            ContextItem {
                source: "task".into(),
                content: "Implement the requested feature.".into(),
            },
        ];
        let a = assemble_context(&items, 64, &CharacterEstimator);
        let b = assemble_context(&items, 64, &CharacterEstimator);
        assert_eq!(a, b);
        assert!(a.estimated_tokens <= 64);
        assert!(!a.omitted_sources.is_empty() || !a.truncated_sources.is_empty());
        assert!(a.content.contains("omitted") || a.content.contains("truncated"));
    }

    #[test]
    fn fallback_estimator_is_conservative_for_utf8_bytes() {
        let estimator = ByteFallbackEstimator;
        assert!(estimator.estimate_tokens("é") >= 2);
    }

    #[test]
    fn workspace_assembly_prioritizes_instructions_then_task_context() {
        let instructions = InstructionContext {
            files: vec![super::super::instructions::InstructionFile {
                path: "AGENTS.md".into(),
                content: "Keep paths inside the workspace.".into(),
                truncated: false,
            }],
            omitted: Vec::new(),
        };
        let task = [ContextItem {
            source: "Current task".into(),
            content: "Review src/main.rs".into(),
        }];
        let assembled =
            assemble_workspace_context(&instructions, &task, 10_000, &CharacterEstimator);
        assert!(
            assembled.content.find("AGENTS.md").unwrap()
                < assembled.content.find("Current task").unwrap()
        );
        assert_eq!(assembled.included_sources.len(), 2);
    }
}
