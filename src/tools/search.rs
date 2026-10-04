use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    context::ignore::IgnoredWalk,
    policy::paths::{PathIntent, WorkspacePath},
    tools::{Tool, ToolExecutionError, ToolMetadata, ToolRequest, ToolResult},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchOutput {
    pub matches: Vec<SearchMatch>,
    pub truncated: bool,
    pub bytes: usize,
}

pub fn search_files(
    root: impl AsRef<Path>,
    query: &str,
    max_matches: usize,
    max_bytes: usize,
    max_file_bytes: usize,
) -> Result<SearchOutput, crate::context::ignore::WorkspaceWalkError> {
    let root = root
        .as_ref()
        .canonicalize()
        .map_err(|_| crate::context::ignore::WorkspaceWalkError::InvalidRoot)?;
    let files = IgnoredWalk::new(&root)?.files()?;
    let mut output = SearchOutput {
        matches: Vec::new(),
        truncated: false,
        bytes: 0,
    };
    'files: for relative in files {
        let path = WorkspacePath::resolve(&root, &relative, PathIntent::Existing)
            .map_err(|_| crate::context::ignore::WorkspaceWalkError::Read)?;
        let mut bytes = Vec::new();
        File::open(path.path())
            .and_then(|file| {
                file.take(max_file_bytes.saturating_add(1) as u64)
                    .read_to_end(&mut bytes)
            })
            .map_err(|_| crate::context::ignore::WorkspaceWalkError::Read)?;
        if bytes.len() > max_file_bytes {
            output.truncated = true;
            continue;
        }
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let display = relative.to_string_lossy().replace('\\', "/");
        for (index, line) in text.lines().enumerate() {
            if !line.contains(query) {
                continue;
            }
            let extra = display.len() + line.len() + 32;
            if output.matches.len() >= max_matches || output.bytes.saturating_add(extra) > max_bytes
            {
                output.truncated = true;
                break 'files;
            }
            output.bytes += extra;
            output.matches.push(SearchMatch {
                path: display.clone(),
                line_number: index + 1,
                line: line.to_owned(),
            });
        }
    }
    Ok(output)
}

pub struct SearchTool {
    root: PathBuf,
    max_matches: usize,
    max_bytes: usize,
    max_file_bytes: usize,
}
impl SearchTool {
    pub fn new(
        root: impl Into<PathBuf>,
        max_matches: usize,
        max_bytes: usize,
        max_file_bytes: usize,
    ) -> Self {
        Self {
            root: root.into(),
            max_matches,
            max_bytes,
            max_file_bytes,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
}
impl Tool for SearchTool {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "search".into(),
            description: "Search visible text files in the workspace".into(),
            input_schema: serde_json::json!({"type":"object","properties":{"query":{"type":"string","minLength":1}},"required":["query"],"additionalProperties":false}),
        }
    }
    fn execute<'a>(
        &'a self,
        request: ToolRequest,
    ) -> futures_util::future::BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
        Box::pin(async move {
            if request.name != "search" {
                return Err(ToolExecutionError::InvalidArguments);
            }
            let args: SearchArgs = serde_json::from_value(request.arguments)
                .map_err(|_| ToolExecutionError::InvalidArguments)?;
            if args.query.is_empty() || args.query.len() > 4096 {
                return Err(ToolExecutionError::InvalidArguments);
            }
            let output = search_files(
                &self.root,
                &args.query,
                self.max_matches,
                self.max_bytes,
                self.max_file_bytes,
            )
            .map_err(|_| ToolExecutionError::Failed)?;
            Ok(ToolResult {
                call_id: request.call_id,
                content: serde_json::to_string(&output).map_err(|_| ToolExecutionError::Failed)?,
                is_error: false,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::search_files;
    use std::path::PathBuf;
    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
    }
    #[test]
    fn searches_visible_utf8_text_with_bounded_output() {
        let full = search_files(fixture(), "needle", 100, 100_000, 16_384).unwrap();
        assert!(
            full.matches
                .iter()
                .any(|m| m.path == "visible.txt" && m.line_number == 1)
        );
        assert!(full.matches.iter().any(|m| m.path == "src/main.rs"));
        assert!(
            !full
                .matches
                .iter()
                .any(|m| m.path.contains("ignored") || m.path.starts_with('.'))
        );
        assert!(!full.matches.iter().any(|m| m.path == "src/binary.dat"));
        let one = search_files(fixture(), "needle", 1, 100_000, 16_384).unwrap();
        assert_eq!(one.matches.len(), 1);
        assert!(one.truncated);
        let tiny = search_files(fixture(), "needle", 100, 1, 16_384).unwrap();
        assert!(tiny.matches.is_empty());
        assert!(tiny.truncated);
    }
}
