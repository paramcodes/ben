use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathIntent {
    /// The target must already exist.
    Existing,
    /// The target may be missing, but its nearest existing parent must be inside the root.
    Create,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathResolutionError {
    #[error("workspace root is unavailable")]
    WorkspaceUnavailable,
    #[error("absolute paths are not allowed")]
    AbsolutePath,
    #[error("parent traversal is not allowed")]
    Traversal,
    #[error("target does not exist")]
    NotFound,
    #[error("target resolves outside the workspace")]
    OutsideWorkspace,
    #[error("an existing parent is not a directory")]
    NotDirectory,
    #[error("path could not be resolved")]
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspacePath {
    root: PathBuf,
    path: PathBuf,
}

impl WorkspacePath {
    pub fn resolve(
        root: impl AsRef<Path>,
        input: impl AsRef<Path>,
        intent: PathIntent,
    ) -> Result<Self, PathResolutionError> {
        let root = fs::canonicalize(root).map_err(|_| PathResolutionError::WorkspaceUnavailable)?;
        if !root.is_dir() {
            return Err(PathResolutionError::WorkspaceUnavailable);
        }
        let relative = normalize_relative(input.as_ref())?;
        let candidate = root.join(relative);
        let path = match intent {
            PathIntent::Existing => {
                let canonical = fs::canonicalize(&candidate).map_err(map_io_error)?;
                ensure_within(&root, &canonical)?;
                canonical
            }
            PathIntent::Create => canonicalize_for_creation(&root, &candidate)?,
        };
        Ok(Self { root, path })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn normalize_relative(input: &Path) -> Result<PathBuf, PathResolutionError> {
    if input.is_absolute() {
        return Err(PathResolutionError::AbsolutePath);
    }
    let mut relative = PathBuf::new();
    for component in input.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err(PathResolutionError::Traversal),
            Component::RootDir | Component::Prefix(_) => {
                return Err(PathResolutionError::AbsolutePath);
            }
        }
    }
    Ok(relative)
}

fn canonicalize_for_creation(
    root: &Path,
    candidate: &Path,
) -> Result<PathBuf, PathResolutionError> {
    let mut ancestor = candidate;
    let mut missing = Vec::<OsString>::new();
    loop {
        match fs::canonicalize(ancestor) {
            Ok(canonical) => {
                ensure_within(root, &canonical)?;
                if !missing.is_empty() && !canonical.is_dir() {
                    return Err(PathResolutionError::NotDirectory);
                }
                let mut resolved = canonical;
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                ensure_within(root, &resolved)?;
                return Ok(resolved);
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if fs::symlink_metadata(ancestor)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
                {
                    return Err(PathResolutionError::OutsideWorkspace);
                }
                let name = ancestor.file_name().ok_or(PathResolutionError::NotFound)?;
                missing.push(name.to_os_string());
                ancestor = ancestor.parent().ok_or(PathResolutionError::NotFound)?;
            }
            Err(_) => return Err(PathResolutionError::Io),
        }
    }
}

fn ensure_within(root: &Path, path: &Path) -> Result<(), PathResolutionError> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(PathResolutionError::OutsideWorkspace)
    }
}

fn map_io_error(error: std::io::Error) -> PathResolutionError {
    if error.kind() == ErrorKind::NotFound {
        PathResolutionError::NotFound
    } else {
        PathResolutionError::Io
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{PathIntent, PathResolutionError, WorkspacePath};

    #[test]
    fn resolves_normal_child_paths_relative_to_canonical_root() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::write(root.path().join("src/main.rs"), "fn main() {}").unwrap();

        let resolved =
            WorkspacePath::resolve(root.path(), "src/main.rs", PathIntent::Existing).unwrap();

        assert_eq!(
            resolved.path(),
            fs::canonicalize(root.path().join("src/main.rs")).unwrap()
        );
    }

    #[test]
    fn rejects_parent_traversal_and_absolute_paths() {
        let root = tempdir().unwrap();
        let absolute = root.path().join("file.txt");

        assert_eq!(
            WorkspacePath::resolve(root.path(), "../outside", PathIntent::Create).unwrap_err(),
            PathResolutionError::Traversal
        );
        assert_eq!(
            WorkspacePath::resolve(root.path(), absolute, PathIntent::Create).unwrap_err(),
            PathResolutionError::AbsolutePath
        );
    }

    #[test]
    fn allows_new_files_when_existing_parent_is_inside_root() {
        let root = tempdir().unwrap();
        let root_canonical = fs::canonicalize(root.path()).unwrap();

        let resolved =
            WorkspacePath::resolve(root.path(), "new/deep/file.txt", PathIntent::Create).unwrap();

        assert_eq!(resolved.path(), root_canonical.join("new/deep/file.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn resolves_symlink_that_stays_inside_workspace() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::write(root.path().join("real/file.txt"), "safe").unwrap();
        symlink("real", root.path().join("alias")).unwrap();

        let resolved =
            WorkspacePath::resolve(root.path(), "alias/file.txt", PathIntent::Existing).unwrap();

        assert_eq!(
            resolved.path(),
            fs::canonicalize(root.path().join("real/file.txt")).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_that_escapes_workspace_for_existing_and_new_targets() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "outside").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();

        assert_eq!(
            WorkspacePath::resolve(root.path(), "escape/secret.txt", PathIntent::Existing)
                .unwrap_err(),
            PathResolutionError::OutsideWorkspace
        );
        assert_eq!(
            WorkspacePath::resolve(root.path(), "escape/new.txt", PathIntent::Create).unwrap_err(),
            PathResolutionError::OutsideWorkspace
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_dangling_symlink_ancestor_for_new_targets() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let outside = tempdir().unwrap();
        symlink(
            outside.path().join("not-created-yet"),
            root.path().join("escape"),
        )
        .unwrap();

        assert_eq!(
            WorkspacePath::resolve(root.path(), "escape/new.txt", PathIntent::Create).unwrap_err(),
            PathResolutionError::OutsideWorkspace
        );
    }

    #[test]
    fn existing_intent_rejects_a_missing_path() {
        let root = tempdir().unwrap();
        let error = WorkspacePath::resolve(root.path(), Path::new("missing"), PathIntent::Existing)
            .unwrap_err();
        assert_eq!(error, PathResolutionError::NotFound);
    }
}
