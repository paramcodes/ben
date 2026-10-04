use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    context::ignore::IgnoredWalk,
    tools::{Tool, ToolExecutionError, ToolMetadata, ToolRequest, ToolResult},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListFilesOutput {
    pub paths: Vec<String>,
    pub truncated: bool,
    pub bytes: usize,
}

pub fn list_files(
    root: impl AsRef<Path>,
    max_entries: usize,
    max_bytes: usize,
) -> Result<ListFilesOutput, crate::context::ignore::WorkspaceWalkError> {
    let files = IgnoredWalk::new(root)?.files()?;
    let mut output = ListFilesOutput {
        paths: Vec::new(),
        truncated: false,
        bytes: 0,
    };
    for path in files {
        let path = path.to_string_lossy().replace('\\', "/");
        let added = path.len() + usize::from(!output.paths.is_empty());
        if output.paths.len() >= max_entries || output.bytes.saturating_add(added) > max_bytes {
            output.truncated = true;
            break;
        }
        output.bytes += added;
        output.paths.push(path);
    }
    Ok(output)
}

pub struct ListFilesTool {
    root: std::path::PathBuf,
    max_entries: usize,
    max_bytes: usize,
}
impl ListFilesTool {
    pub fn new(root: impl Into<std::path::PathBuf>, max_entries: usize, max_bytes: usize) -> Self {
        Self {
            root: root.into(),
            max_entries,
            max_bytes,
        }
    }
}
impl Tool for ListFilesTool {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "list_files".into(),
            description: "List visible files in the workspace".into(),
            input_schema: serde_json::json!({"type":"object","additionalProperties":false}),
        }
    }
    fn execute<'a>(
        &'a self,
        request: ToolRequest,
    ) -> futures_util::future::BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
        Box::pin(async move {
            if request.name != "list_files"
                || !request
                    .arguments
                    .as_object()
                    .is_some_and(serde_json::Map::is_empty)
            {
                return Err(ToolExecutionError::InvalidArguments);
            }
            let output = list_files(&self.root, self.max_entries, self.max_bytes)
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
    use super::list_files;
    use std::path::PathBuf;
    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
    }
    #[test]
    fn output_is_sorted_and_obeys_count_and_bytes() {
        let full = list_files(fixture(), 100, 100_000).unwrap();
        assert!(full.paths.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(!full.paths.iter().any(|p| p.contains("ignored")));
        let one = list_files(fixture(), 1, 100_000).unwrap();
        assert_eq!(one.paths.len(), 1);
        assert!(one.truncated);
        let tiny = list_files(fixture(), 100, 1).unwrap();
        assert!(tiny.paths.is_empty());
        assert!(tiny.truncated);
    }
}
