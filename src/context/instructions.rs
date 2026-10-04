use std::{collections::HashSet, fs::File, io::Read, path::Path};

use crate::{
    context::ignore::IgnoredWalk,
    policy::paths::{PathIntent, WorkspacePath},
};

const MAX_INSTRUCTION_FILE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionFile {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InstructionContext {
    /// Applicable files ordered from workspace root toward the current directory.
    pub files: Vec<InstructionFile>,
    /// Sources omitted while loading. Budget omissions are added by the budget assembler.
    pub omitted: Vec<String>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InstructionError {
    #[error("workspace path is invalid")]
    InvalidPath,
    #[error("instructions could not be read")]
    Read,
}

/// Load applicable `AGENTS.md` files from the workspace root through `current_path`.
/// Hidden, ignored, generated, and out-of-workspace paths follow the common walker policy.
pub fn load_instructions(
    root: impl AsRef<Path>,
    current_path: impl AsRef<Path>,
) -> Result<InstructionContext, InstructionError> {
    let resolved = WorkspacePath::resolve(root, current_path, PathIntent::Existing)
        .map_err(|_| InstructionError::InvalidPath)?;
    if !resolved.path().is_dir() {
        return Err(InstructionError::InvalidPath);
    }
    let workspace_files: HashSet<_> = IgnoredWalk::new(resolved.root())
        .map_err(|_| InstructionError::Read)?
        .files()
        .map_err(|_| InstructionError::Read)?
        .into_iter()
        .collect();

    let relative = resolved
        .path()
        .strip_prefix(resolved.root())
        .map_err(|_| InstructionError::InvalidPath)?;
    let mut directories = vec![std::path::PathBuf::new()];
    let mut cursor = std::path::PathBuf::new();
    for component in relative.components() {
        cursor.push(component);
        directories.push(cursor.clone());
    }

    let mut context = InstructionContext::default();
    for directory in directories {
        let file_path = directory.join("AGENTS.md");
        if !workspace_files.contains(&file_path) {
            continue;
        }
        let absolute = resolved.root().join(&file_path);
        let mut bytes = Vec::new();
        File::open(&absolute)
            .and_then(|file| {
                file.take(MAX_INSTRUCTION_FILE_BYTES.saturating_add(1) as u64)
                    .read_to_end(&mut bytes)
            })
            .map_err(|_| InstructionError::Read)?;
        let truncated = bytes.len() > MAX_INSTRUCTION_FILE_BYTES;
        if truncated {
            bytes.truncate(MAX_INSTRUCTION_FILE_BYTES);
        }
        let mut content = match std::str::from_utf8(&bytes) {
            Ok(content) => content.to_owned(),
            Err(error) if truncated && error.error_len().is_none() => {
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .map_err(|_| InstructionError::Read)?
                    .to_owned()
            }
            Err(_) => return Err(InstructionError::Read),
        };
        if truncated {
            content.push_str(&format!(
                "\n[AGENTS.md content truncated at {MAX_INSTRUCTION_FILE_BYTES} bytes.]\n"
            ));
        }
        context.files.push(InstructionFile {
            path: file_path.to_string_lossy().replace('\\', "/"),
            content,
            truncated,
        });
    }
    Ok(context)
}

#[cfg(test)]
mod tests {
    use super::load_instructions;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn loads_root_to_nested_instructions_and_skips_ignored_files() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("src/nested")).unwrap();
        fs::write(root.path().join("AGENTS.md"), "root guidance").unwrap();
        fs::write(root.path().join("src/AGENTS.md"), "nested guidance").unwrap();
        fs::write(root.path().join("src/nested/AGENTS.md"), "deep guidance").unwrap();
        fs::write(root.path().join(".ignore"), "src/nested/AGENTS.md\n").unwrap();
        let loaded = load_instructions(root.path(), "src/nested").unwrap();
        let contents: Vec<_> = loaded
            .files
            .iter()
            .map(|file| file.content.as_str())
            .collect();
        assert_eq!(contents, ["root guidance", "nested guidance"]);
        assert_eq!(loaded.files[0].path, "AGENTS.md");
        assert_eq!(loaded.files[1].path, "src/AGENTS.md");
    }

    #[test]
    fn missing_instruction_files_produce_empty_context() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("subdir")).unwrap();
        let loaded = load_instructions(root.path(), "subdir").unwrap();
        assert!(loaded.files.is_empty());
        assert!(loaded.omitted.is_empty());
    }

    #[test]
    fn bounds_large_instruction_files_and_marks_the_truncation() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("AGENTS.md"), "x".repeat(100_000)).unwrap();
        let loaded = load_instructions(root.path(), ".").unwrap();
        assert!(loaded.files[0].truncated);
        assert!(loaded.files[0].content.contains("content truncated"));
        assert!(loaded.files[0].content.len() < 66 * 1024);
    }
}
