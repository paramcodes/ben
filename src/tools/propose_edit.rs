use std::{fs::File, io::Read, path::Path};

use serde::{Deserialize, Serialize};

use crate::policy::{
    approval::PendingAction,
    paths::{PathIntent, PathResolutionError, WorkspacePath},
};

const DEFAULT_CONTEXT_LINES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeEditInput {
    pub path: String,
    pub expected_content: String,
    pub new_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedEdit {
    pub path: String,
    pub unified_diff: String,
    pub pending_action: PendingAction,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProposeEditError {
    #[error("edit input is invalid")]
    InvalidInput,
    #[error("target file is missing")]
    MissingTarget,
    #[error("expected content does not match the file")]
    StaleContent,
    #[error("file is too large")]
    TooLarge,
    #[error("diff is too large")]
    DiffTooLarge,
    #[error("file is not valid UTF-8 text")]
    InvalidText,
    #[error("file could not be read within the workspace")]
    Read,
}

pub fn propose_edit(
    root: impl AsRef<Path>,
    input: ProposeEditInput,
    max_file_bytes: usize,
    max_diff_bytes: usize,
) -> Result<ProposedEdit, ProposeEditError> {
    if input.path.trim().is_empty() {
        return Err(ProposeEditError::InvalidInput);
    }
    if input.expected_content.len() > max_file_bytes || input.new_content.len() > max_file_bytes {
        return Err(ProposeEditError::TooLarge);
    }
    if input.new_content.contains('\0') || input.expected_content.contains('\0') {
        return Err(ProposeEditError::InvalidText);
    }

    let (relative_path, old_content) = load_current_content(root.as_ref(), &input, max_file_bytes)?;
    if old_content == input.new_content {
        return Err(ProposeEditError::InvalidInput);
    }

    let unified_diff = unified_diff(&relative_path, &old_content, &input.new_content);
    if unified_diff.len() > max_diff_bytes {
        return Err(ProposeEditError::DiffTooLarge);
    }

    let pending_action = PendingAction::new(
        "propose_edit",
        serde_json::json!({
            "path": relative_path,
            "expected_content": input.expected_content,
            "new_content": input.new_content,
        }),
    )
    .map_err(|_| ProposeEditError::InvalidInput)?;

    Ok(ProposedEdit {
        path: relative_path,
        unified_diff,
        pending_action,
    })
}

fn load_current_content(
    root: &Path,
    input: &ProposeEditInput,
    max_file_bytes: usize,
) -> Result<(String, String), ProposeEditError> {
    match WorkspacePath::resolve(root, &input.path, PathIntent::Existing) {
        Ok(resolved) => {
            let content = read_workspace_text(resolved.path(), max_file_bytes)?;
            if content != input.expected_content {
                return Err(ProposeEditError::StaleContent);
            }
            Ok((relative_path(&resolved)?, content))
        }
        Err(PathResolutionError::NotFound) => {
            if !input.expected_content.is_empty() {
                return Err(ProposeEditError::MissingTarget);
            }
            let resolved = WorkspacePath::resolve(root, &input.path, PathIntent::Create)
                .map_err(map_path_error)?;
            if resolved.path().exists() {
                return Err(ProposeEditError::StaleContent);
            }
            Ok((relative_path(&resolved)?, String::new()))
        }
        Err(error) => Err(map_path_error(error)),
    }
}

fn read_workspace_text(path: &Path, max_file_bytes: usize) -> Result<String, ProposeEditError> {
    let metadata = std::fs::metadata(path).map_err(|_| ProposeEditError::Read)?;
    if !metadata.is_file() || metadata.len() > max_file_bytes as u64 {
        return Err(ProposeEditError::TooLarge);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| {
            file.take(max_file_bytes.saturating_add(1) as u64)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| ProposeEditError::Read)?;
    if bytes.len() > max_file_bytes {
        return Err(ProposeEditError::TooLarge);
    }
    if bytes.contains(&0) {
        return Err(ProposeEditError::InvalidText);
    }
    String::from_utf8(bytes).map_err(|_| ProposeEditError::InvalidText)
}

fn relative_path(resolved: &WorkspacePath) -> Result<String, ProposeEditError> {
    resolved
        .path()
        .strip_prefix(resolved.root())
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .map_err(|_| ProposeEditError::Read)
}

fn map_path_error(error: PathResolutionError) -> ProposeEditError {
    match error {
        PathResolutionError::NotFound => ProposeEditError::MissingTarget,
        PathResolutionError::AbsolutePath
        | PathResolutionError::Traversal
        | PathResolutionError::OutsideWorkspace
        | PathResolutionError::NotDirectory
        | PathResolutionError::WorkspaceUnavailable
        | PathResolutionError::Io => ProposeEditError::Read,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditOp {
    Equal(usize),
    Delete(usize),
    Insert(usize),
}

pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let ops = diff_ops(&old_lines, &new_lines);
    let hunks = group_hunks(&ops, DEFAULT_CONTEXT_LINES);
    let mut output = format!("--- a/{path}\n+++ b/{path}\n");
    for hunk in hunks {
        output.push_str(&format_hunk(&old_lines, &new_lines, &hunk));
    }
    output
}

fn diff_ops(old_lines: &[&str], new_lines: &[&str]) -> Vec<EditOp> {
    let old_len = old_lines.len();
    let new_len = new_lines.len();
    let mut lengths = vec![vec![0usize; new_len + 1]; old_len + 1];
    for i in (0..old_len).rev() {
        for j in (0..new_len).rev() {
            lengths[i][j] = if old_lines[i] == new_lines[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let mut i = 0;
    let mut j = 0;
    while i < old_len || j < new_len {
        if i < old_len && j < new_len && old_lines[i] == new_lines[j] {
            ops.push(EditOp::Equal(i));
            i += 1;
            j += 1;
        } else if j < new_len && (i == old_len || lengths[i][j + 1] >= lengths[i + 1][j]) {
            ops.push(EditOp::Insert(j));
            j += 1;
        } else if i < old_len {
            ops.push(EditOp::Delete(i));
            i += 1;
        }
    }
    ops
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    old_start: usize,
    new_start: usize,
    ops: Vec<EditOp>,
}

fn group_hunks(ops: &[EditOp], context: usize) -> Vec<Hunk> {
    let change_indexes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter_map(|(index, op)| match op {
            EditOp::Equal(_) => None,
            EditOp::Delete(_) | EditOp::Insert(_) => Some(index),
        })
        .collect();
    if change_indexes.is_empty() {
        return Vec::new();
    }

    let mut hunks = Vec::new();
    let mut start = change_indexes[0].saturating_sub(context);
    let mut end = (change_indexes[0] + 1)
        .saturating_add(context)
        .min(ops.len());
    for &change in change_indexes.iter().skip(1) {
        let next_start = change.saturating_sub(context);
        if next_start <= end {
            end = (change + 1).saturating_add(context).min(ops.len());
        } else {
            hunks.push(make_hunk(&ops[start..end]));
            start = next_start;
            end = (change + 1).saturating_add(context).min(ops.len());
        }
    }
    hunks.push(make_hunk(&ops[start..end]));
    hunks
}

fn make_hunk(ops: &[EditOp]) -> Hunk {
    let mut old_start = 0;
    let mut new_start = 0;
    let mut set = false;
    for op in ops {
        match op {
            EditOp::Equal(index) | EditOp::Delete(index) => {
                old_start = *index;
                set = true;
                break;
            }
            EditOp::Insert(_) => {}
        }
    }
    for op in ops {
        match op {
            EditOp::Equal(index) | EditOp::Insert(index) => {
                new_start = *index;
                break;
            }
            EditOp::Delete(_) => {}
        }
    }
    if !set {
        old_start = 0;
    }
    Hunk {
        old_start,
        new_start,
        ops: ops.to_vec(),
    }
}

fn format_hunk(old_lines: &[&str], new_lines: &[&str], hunk: &Hunk) -> String {
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    let mut body = String::new();
    for op in &hunk.ops {
        match *op {
            EditOp::Equal(index) => {
                old_count += 1;
                new_count += 1;
                body.push(' ');
                body.push_str(old_lines[index]);
                body.push('\n');
            }
            EditOp::Delete(index) => {
                old_count += 1;
                body.push('-');
                body.push_str(old_lines[index]);
                body.push('\n');
            }
            EditOp::Insert(index) => {
                new_count += 1;
                body.push('+');
                body.push_str(new_lines[index]);
                body.push('\n');
            }
        }
    }
    let old_start = if old_count == 0 {
        hunk.old_start
    } else {
        hunk.old_start + 1
    };
    let new_start = if new_count == 0 {
        hunk.new_start
    } else {
        hunk.new_start + 1
    };
    format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n{body}")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{ProposeEditError, ProposeEditInput, propose_edit, unified_diff};

    fn workspace_with(content: &str) -> (tempfile::TempDir, String) {
        let root = tempdir().unwrap();
        let path = "src/demo.rs";
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::write(root.path().join(path), content).unwrap();
        (root, path.into())
    }

    fn snapshot(root: &tempfile::TempDir, path: &str) -> Vec<u8> {
        fs::read(root.path().join(path)).unwrap()
    }

    #[test]
    fn replace_produces_unified_diff_without_writing() {
        let (root, path) = workspace_with("alpha\nbeta\ngamma\n");
        let before = snapshot(&root, &path);
        let proposal = propose_edit(
            root.path(),
            ProposeEditInput {
                path: path.clone(),
                expected_content: "alpha\nbeta\ngamma\n".into(),
                new_content: "alpha\nBETA\ngamma\n".into(),
            },
            4_096,
            4_096,
        )
        .unwrap();
        assert_eq!(proposal.path, path);
        assert!(proposal.unified_diff.contains("--- a/src/demo.rs"));
        assert!(proposal.unified_diff.contains("+++ b/src/demo.rs"));
        assert!(proposal.unified_diff.contains("-beta"));
        assert!(proposal.unified_diff.contains("+BETA"));
        assert!(proposal.unified_diff.contains("@@"));
        assert_eq!(proposal.pending_action.tool(), "propose_edit");
        assert_eq!(snapshot(&root, &path), before);
    }

    #[test]
    fn add_create_and_delete_cover_diff_shapes() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();

        let created = propose_edit(
            root.path(),
            ProposeEditInput {
                path: "src/new.rs".into(),
                expected_content: String::new(),
                new_content: "fn main() {}\n".into(),
            },
            4_096,
            4_096,
        )
        .unwrap();
        assert!(created.unified_diff.contains("+fn main() {}"));
        assert!(!root.path().join("src/new.rs").exists());

        fs::write(root.path().join("src/gone.rs"), "remove me\n").unwrap();
        let before = fs::read(root.path().join("src/gone.rs")).unwrap();
        let deleted = propose_edit(
            root.path(),
            ProposeEditInput {
                path: "src/gone.rs".into(),
                expected_content: "remove me\n".into(),
                new_content: String::new(),
            },
            4_096,
            4_096,
        )
        .unwrap();
        assert!(deleted.unified_diff.contains("-remove me"));
        assert_eq!(fs::read(root.path().join("src/gone.rs")).unwrap(), before);
    }

    #[test]
    fn multiple_hunks_are_emitted_for_distant_changes() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\n";
        let new = "A\nb\nc\nd\ne\nf\ng\nh\nI\n";
        let diff = unified_diff("file.txt", old, new);
        assert_eq!(diff.matches("@@ ").count(), 2);
        assert!(diff.contains("-a"));
        assert!(diff.contains("+A"));
        assert!(diff.contains("-i"));
        assert!(diff.contains("+I"));
    }

    #[test]
    fn missing_target_and_stale_content_are_rejected() {
        let (root, path) = workspace_with("alpha\n");
        let before = snapshot(&root, &path);

        let missing = propose_edit(
            root.path(),
            ProposeEditInput {
                path: "src/missing.rs".into(),
                expected_content: "nope\n".into(),
                new_content: "fresh\n".into(),
            },
            4_096,
            4_096,
        );
        assert_eq!(missing, Err(ProposeEditError::MissingTarget));

        let stale = propose_edit(
            root.path(),
            ProposeEditInput {
                path,
                expected_content: "beta\n".into(),
                new_content: "gamma\n".into(),
            },
            4_096,
            4_096,
        );
        assert_eq!(stale, Err(ProposeEditError::StaleContent));
        assert_eq!(snapshot(&root, "src/demo.rs"), before);
    }

    #[test]
    fn rejects_binary_and_oversized_inputs() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("bin.dat"), b"ok\0no").unwrap();
        assert_eq!(
            propose_edit(
                root.path(),
                ProposeEditInput {
                    path: "bin.dat".into(),
                    expected_content: "ignored".into(),
                    new_content: "clean".into(),
                },
                4_096,
                4_096,
            ),
            Err(ProposeEditError::InvalidText)
        );
        assert_eq!(
            propose_edit(
                root.path(),
                ProposeEditInput {
                    path: "fresh.txt".into(),
                    expected_content: String::new(),
                    new_content: "has\0null".into(),
                },
                4_096,
                4_096,
            ),
            Err(ProposeEditError::InvalidText)
        );

        let (root, path) = workspace_with("small\n");
        assert_eq!(
            propose_edit(
                root.path(),
                ProposeEditInput {
                    path,
                    expected_content: "small\n".into(),
                    new_content: "x".repeat(64),
                },
                8,
                4_096,
            ),
            Err(ProposeEditError::TooLarge)
        );
    }
}
