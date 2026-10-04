use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum WorkspaceWalkError {
    #[error("workspace root is invalid")]
    InvalidRoot,
    #[error("workspace could not be read")]
    Read,
}

#[derive(Debug)]
pub struct IgnoredWalk {
    root: PathBuf,
}

impl IgnoredWalk {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, WorkspaceWalkError> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|_| WorkspaceWalkError::InvalidRoot)?;
        if !root.is_dir() {
            return Err(WorkspaceWalkError::InvalidRoot);
        }
        Ok(Self { root })
    }

    pub fn files(&self) -> Result<Vec<PathBuf>, WorkspaceWalkError> {
        let mut builder = ignore::WalkBuilder::new(&self.root);
        builder
            .hidden(true)
            .parents(false)
            .git_global(false)
            .git_exclude(false)
            .follow_links(false)
            .filter_entry(|entry| {
                let name = entry.file_name().to_string_lossy();
                !is_excluded_name(&name)
            });
        let mut files = Vec::new();
        for result in builder.build() {
            let entry = result.map_err(|_| WorkspaceWalkError::Read)?;
            if entry.depth() == 0 {
                continue;
            }
            let ty = entry.file_type().ok_or(WorkspaceWalkError::Read)?;
            if !ty.is_file() || ty.is_symlink() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&self.root)
                .map_err(|_| WorkspaceWalkError::Read)?;
            files.push(relative.to_path_buf());
        }
        files.sort();
        Ok(files)
    }
}

fn is_excluded_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | "target"
            | "node_modules"
            | "dist"
            | "build"
            | ".venv"
            | "venv"
            | ".next"
            | ".ssh"
    ) || name == ".env"
        || name.starts_with(".env.")
        || matches!(
            name,
            "id_rsa" | "id_ed25519" | "credentials.json" | "secrets.json"
        )
        || name.ends_with(".pem")
        || name.ends_with(".key")
}

#[cfg(test)]
mod tests {
    use super::{IgnoredWalk, WorkspaceWalkError};
    use std::path::PathBuf;
    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace")
    }
    #[test]
    fn returns_sorted_visible_files_and_honors_ignore_rules() {
        let files = IgnoredWalk::new(fixture()).unwrap().files().unwrap();
        assert!(files.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(files.contains(&PathBuf::from("visible.txt")));
        assert!(files.contains(&PathBuf::from("src/main.rs")));
        assert!(
            !files
                .iter()
                .any(|p| p.to_string_lossy().contains("ignored"))
        );
        assert!(!files.iter().any(|p| p.starts_with("target")));
        assert!(!files.iter().any(|p| p.to_string_lossy().starts_with(".")));
    }
    #[test]
    fn rejects_file_as_root() {
        assert_eq!(
            IgnoredWalk::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
                .unwrap_err(),
            WorkspaceWalkError::InvalidRoot
        );
    }
    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks_outside_workspace() {
        use std::{fs, os::unix::fs::symlink};
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        assert!(
            IgnoredWalk::new(root.path())
                .unwrap()
                .files()
                .unwrap()
                .is_empty()
        );
    }
}
