use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    policy::paths::{PathIntent, WorkspacePath},
    tools::{Tool, ToolExecutionError, ToolMetadata, ToolRequest, ToolResult},
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadFileInput {
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadFileOutput {
    pub path: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ReadFileError {
    #[error("file input is invalid")]
    InvalidInput,
    #[error("file is too large")]
    TooLarge,
    #[error("file is not valid UTF-8 text")]
    InvalidText,
    #[error("file could not be read within the workspace")]
    Read,
}

pub fn read_file(
    root: impl AsRef<Path>,
    input: ReadFileInput,
    max_output_bytes: usize,
    max_file_bytes: usize,
) -> Result<ReadFileOutput, ReadFileError> {
    if input.path.is_empty()
        || input.start_line == Some(0)
        || input.end_line == Some(0)
        || matches!((input.start_line, input.end_line), (Some(start), Some(end)) if start > end)
    {
        return Err(ReadFileError::InvalidInput);
    }
    let resolved = WorkspacePath::resolve(root, &input.path, PathIntent::Existing)
        .map_err(|_| ReadFileError::Read)?;
    let metadata = std::fs::metadata(resolved.path()).map_err(|_| ReadFileError::Read)?;
    if !metadata.is_file() || metadata.len() > max_file_bytes as u64 {
        return Err(ReadFileError::TooLarge);
    }
    let mut bytes = Vec::new();
    File::open(resolved.path())
        .and_then(|file| {
            file.take(max_file_bytes.saturating_add(1) as u64)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| ReadFileError::Read)?;
    if bytes.len() > max_file_bytes {
        return Err(ReadFileError::TooLarge);
    }
    if bytes.contains(&0) {
        return Err(ReadFileError::InvalidText);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| ReadFileError::InvalidText)?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (start, end, selected) = if lines.is_empty() {
        if input.start_line.is_some() || input.end_line.is_some() {
            return Err(ReadFileError::InvalidInput);
        }
        (0, 0, String::new())
    } else {
        let start = input.start_line.unwrap_or(1);
        let end = input.end_line.unwrap_or(lines.len());
        if start > lines.len() || end > lines.len() {
            return Err(ReadFileError::InvalidInput);
        }
        (start, end, lines[start - 1..end].concat())
    };
    let mut boundary = selected.len().min(max_output_bytes);
    while !selected.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let content = selected[..boundary].to_owned();
    let truncated = boundary < selected.len();
    let relative = resolved
        .path()
        .strip_prefix(resolved.root())
        .map_err(|_| ReadFileError::Read)?;
    Ok(ReadFileOutput {
        path: relative.to_string_lossy().replace('\\', "/"),
        bytes: content.len(),
        content,
        start_line: start,
        end_line: end,
        truncated,
    })
}

pub struct ReadFileTool {
    root: PathBuf,
    max_output_bytes: usize,
    max_file_bytes: usize,
}
impl ReadFileTool {
    pub fn new(root: impl Into<PathBuf>, max_output_bytes: usize, max_file_bytes: usize) -> Self {
        Self {
            root: root.into(),
            max_output_bytes,
            max_file_bytes,
        }
    }
}

impl Tool for ReadFileTool {
    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            name: "read_file".into(),
            description: "Read bounded UTF-8 text from a workspace file".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "start_line": {"type": "integer", "minimum": 1},
                    "end_line": {"type": "integer", "minimum": 1}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        }
    }

    fn execute<'a>(
        &'a self,
        request: ToolRequest,
    ) -> futures_util::future::BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
        Box::pin(async move {
            if request.name != "read_file" {
                return Err(ToolExecutionError::InvalidArguments);
            }
            let input: ReadFileInput = serde_json::from_value(request.arguments)
                .map_err(|_| ToolExecutionError::InvalidArguments)?;
            let output = read_file(
                &self.root,
                input,
                self.max_output_bytes,
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
    use std::fs;

    use tempfile::tempdir;

    use super::{ReadFileInput, read_file};

    fn workspace() -> tempfile::TempDir {
        let root = tempdir().unwrap();
        fs::write(root.path().join("notes.txt"), "alpha\nbeta\ngamma\n").unwrap();
        root
    }

    #[test]
    fn reads_utf8_text_and_requested_one_based_line_range() {
        let root = workspace();
        let output = read_file(
            root.path(),
            ReadFileInput {
                path: "notes.txt".into(),
                start_line: Some(2),
                end_line: Some(2),
            },
            100,
            100,
        )
        .unwrap();
        assert_eq!(output.content, "beta\n");
        assert_eq!(output.start_line, 2);
        assert_eq!(output.end_line, 2);
        assert!(!output.truncated);
    }

    #[test]
    fn truncates_at_byte_limit_and_rejects_oversized_or_invalid_utf8_files() {
        let root = workspace();
        let truncated = read_file(
            root.path(),
            ReadFileInput {
                path: "notes.txt".into(),
                start_line: None,
                end_line: None,
            },
            6,
            100,
        )
        .unwrap();
        assert_eq!(truncated.content, "alpha\n");
        assert!(truncated.truncated);

        fs::write(root.path().join("large.txt"), "x".repeat(101)).unwrap();
        assert!(
            read_file(
                root.path(),
                ReadFileInput {
                    path: "large.txt".into(),
                    start_line: None,
                    end_line: None
                },
                100,
                100
            )
            .is_err()
        );

        fs::write(root.path().join("invalid.txt"), [0xff, 0xfe]).unwrap();
        assert!(
            read_file(
                root.path(),
                ReadFileInput {
                    path: "invalid.txt".into(),
                    start_line: None,
                    end_line: None
                },
                100,
                100
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_missing_traversal_and_invalid_ranges() {
        let root = workspace();
        for input in [
            ReadFileInput {
                path: "missing.txt".into(),
                start_line: None,
                end_line: None,
            },
            ReadFileInput {
                path: "../outside.txt".into(),
                start_line: None,
                end_line: None,
            },
            ReadFileInput {
                path: "notes.txt".into(),
                start_line: Some(0),
                end_line: None,
            },
            ReadFileInput {
                path: "notes.txt".into(),
                start_line: Some(3),
                end_line: Some(2),
            },
        ] {
            assert!(read_file(root.path(), input, 100, 100).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let root = workspace();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(
            outside.path().join("secret.txt"),
            root.path().join("escape.txt"),
        )
        .unwrap();
        assert!(
            read_file(
                root.path(),
                ReadFileInput {
                    path: "escape.txt".into(),
                    start_line: None,
                    end_line: None
                },
                100,
                100
            )
            .is_err()
        );
    }
}
