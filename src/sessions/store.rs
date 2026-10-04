//! Local session persistence: atomic writes, restrictive permissions, and an
//! explicit recovery path for damaged files.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    config::Config,
    sessions::model::{SessionError, SessionRecord},
};

const SESSIONS_DIR: &str = "sessions";
const SESSION_EXTENSION: &str = "json";
const MAX_ID_LENGTH: usize = 128;
const MAX_TEMP_ATTEMPTS: u64 = 100;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum SessionStoreError {
    #[error("session id is not a safe file name")]
    InvalidId,
    #[error("session store directory {path} could not be prepared: {source}")]
    Root {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("session {id} could not be written: {source}")]
    Write { id: String, source: std::io::Error },
    #[error("session {id} could not be read: {source}")]
    Read { id: String, source: std::io::Error },
    #[error("session {id} could not be cleared: {source}")]
    Delete { id: String, source: std::io::Error },
    #[error("session {id} was not found in {path}")]
    NotFound { id: String, path: PathBuf },
    #[error("session {id} is damaged and cannot be read; remove {path} to start a new session")]
    Corrupt {
        id: String,
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Schema(#[from] SessionError),
}

/// Session files under one directory, replaced atomically on every save.
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Stores sessions under the configured application data directory.
    pub fn for_config(config: &Config) -> Self {
        Self::new(config.data_dir.join(SESSIONS_DIR))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolves a session file, refusing ids that could escape the directory.
    pub fn path_for(&self, id: &str) -> Result<PathBuf, SessionStoreError> {
        if !is_safe_id(id) {
            return Err(SessionStoreError::InvalidId);
        }
        Ok(self.root.join(format!("{id}.{SESSION_EXTENSION}")))
    }

    /// Writes the record through a temporary sibling so an interrupted save
    /// cannot truncate the previous session.
    pub fn save(&self, record: &SessionRecord) -> Result<PathBuf, SessionStoreError> {
        let json = record
            .to_json()
            .map_err(|source| SessionStoreError::Write {
                id: record.id.clone(),
                source: std::io::Error::other(source),
            })?;
        self.write_record(&record.id, |file| {
            file.write_all(json.as_bytes())?;
            file.sync_all()
        })
    }

    /// Reads a session, refusing a damaged file or an unknown schema version.
    pub fn load(&self, id: &str) -> Result<SessionRecord, SessionStoreError> {
        let path = self.path_for(id)?;
        let text = fs::read_to_string(&path).map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => SessionStoreError::NotFound {
                id: id.to_owned(),
                path: path.clone(),
            },
            _ => SessionStoreError::Read {
                id: id.to_owned(),
                source,
            },
        })?;
        let record =
            SessionRecord::from_json(&text).map_err(|source| SessionStoreError::Corrupt {
                id: id.to_owned(),
                path: path.clone(),
                source,
            })?;
        record.validate()?;
        Ok(record)
    }

    /// Sorted session ids in the store. A directory that does not exist yet
    /// lists as empty rather than failing, so a first run is not an error.
    /// Damaged files are still listed: their ids are what a user needs in order
    /// to clear them.
    pub fn list_ids(&self) -> Result<Vec<String>, SessionStoreError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(SessionStoreError::Root {
                    path: self.root.clone(),
                    source,
                });
            }
        };

        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| SessionStoreError::Read {
                id: self.root.to_string_lossy().into_owned(),
                source,
            })?;
            let path = entry.path();
            let is_session_file = path
                .extension()
                .is_some_and(|extension| extension == SESSION_EXTENSION);
            let is_session_id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(is_safe_id);
            if is_session_file && is_session_id {
                let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                ids.push(id.to_owned());
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    /// Whether a session is stored under this id. A damaged file still counts
    /// as present, because clearing it is the documented recovery path.
    pub fn exists(&self, id: &str) -> Result<bool, SessionStoreError> {
        Ok(self.path_for(id)?.is_file())
    }

    /// Removes one stored session. A missing id reports its path so the user
    /// can see whether the session was already cleared.
    pub fn delete(&self, id: &str) -> Result<(), SessionStoreError> {
        let path = self.path_for(id)?;
        fs::remove_file(&path).map_err(|source| match source.kind() {
            std::io::ErrorKind::NotFound => SessionStoreError::NotFound {
                id: id.to_owned(),
                path,
            },
            _ => SessionStoreError::Delete {
                id: id.to_owned(),
                source,
            },
        })
    }

    fn write_record<F>(&self, id: &str, write: F) -> Result<PathBuf, SessionStoreError>
    where
        F: FnOnce(&mut File) -> std::io::Result<()>,
    {
        let target = self.path_for(id)?;
        self.ensure_root()?;
        let (temp_path, mut temp_file) = self.create_temp_file(id)?;
        let mut guard = TempFile(temp_path.clone(), true);

        write(&mut temp_file).map_err(|source| SessionStoreError::Write {
            id: id.to_owned(),
            source,
        })?;
        drop(temp_file);

        fs::rename(&temp_path, &target).map_err(|source| SessionStoreError::Write {
            id: id.to_owned(),
            source,
        })?;
        guard.1 = false;

        if let Ok(directory) = File::open(&self.root) {
            let _ = directory.sync_all();
        }
        Ok(target)
    }

    fn ensure_root(&self) -> Result<(), SessionStoreError> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&self.root)
            .map_err(|source| SessionStoreError::Root {
                path: self.root.clone(),
                source,
            })
    }

    /// Creates a private sibling file, retrying past collisions with other writers.
    fn create_temp_file(&self, id: &str) -> Result<(PathBuf, File), SessionStoreError> {
        let name = self
            .path_for(id)?
            .file_name()
            .map_or_else(|| id.to_owned(), |name| name.to_string_lossy().into_owned());
        let pid = std::process::id();
        for _ in 0..MAX_TEMP_ATTEMPTS {
            let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let temp_path = self.root.join(format!(".{name}.{pid}.{count}.tmp"));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&temp_path) {
                Ok(file) => return Ok((temp_path, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(SessionStoreError::Write {
                        id: id.to_owned(),
                        source,
                    });
                }
            }
        }
        Err(SessionStoreError::Write {
            id: id.to_owned(),
            source: std::io::Error::other("could not create a temporary session file"),
        })
    }

    /// Test hook: a save that fails mid-write, as an interrupted process would.
    #[cfg(test)]
    fn save_with_write_failure(
        &self,
        record: &SessionRecord,
    ) -> Result<PathBuf, SessionStoreError> {
        self.write_record(&record.id, |_| {
            Err(std::io::Error::other("simulated write error"))
        })
    }
}

/// Removes a temporary file unless the save replaced its target.
struct TempFile(PathBuf, bool);

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.1 {
            let _ = fs::remove_file(&self.0);
        }
    }
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LENGTH
        && id != "."
        && id != ".."
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::Path};

    use tempfile::tempdir;

    use super::{SessionStore, SessionStoreError};
    use crate::{
        config::Config,
        sessions::model::{SCHEMA_VERSION, SessionError, SessionMessage, SessionRecord},
    };

    fn record(id: &str) -> SessionRecord {
        SessionRecord::new(
            id,
            "test-model",
            1_700_000_000_000,
            vec![SessionMessage::User {
                text: "update the notes".into(),
            }],
            Vec::new(),
        )
        .unwrap()
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn config_with_data_dir(data_dir: &Path) -> Config {
        let mut values = HashMap::new();
        values.insert("OPENAI_API_KEY".to_owned(), "test-secret-value".to_owned());
        values.insert(
            "BEN_DATA_DIR".to_owned(),
            data_dir.to_string_lossy().into_owned(),
        );
        Config::from_values(&values, None).unwrap()
    }

    #[test]
    fn saved_sessions_round_trip_through_the_store() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let saved = record("session-1");

        let path = store.save(&saved).unwrap();
        let loaded = store.load("session-1").unwrap();

        assert_eq!(loaded, saved);
        assert_eq!(path.file_name().unwrap(), "session-1.json");
        assert_eq!(entries(dir.path()), ["session-1.json"]);
    }

    #[test]
    fn a_saved_session_can_be_replaced_without_leaving_debris() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store.save(&record("session-1")).unwrap();
        let updated = SessionRecord::new(
            "session-1",
            "test-model",
            1_700_000_000_001,
            vec![SessionMessage::Assistant {
                text: "done".into(),
            }],
            Vec::new(),
        )
        .unwrap();

        store.save(&updated).unwrap();

        assert_eq!(store.load("session-1").unwrap(), updated);
        assert_eq!(entries(dir.path()), ["session-1.json"]);
    }

    #[test]
    fn store_files_are_not_world_readable() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));

        let path = store.save(&record("session-1")).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(store.root()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        #[cfg(not(unix))]
        assert!(path.exists());
    }

    #[test]
    fn a_missing_session_reports_its_id() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        assert!(matches!(
            store.load("absent"),
            Err(SessionStoreError::NotFound { .. })
        ));
    }

    #[test]
    fn a_truncated_session_file_reports_a_recovery_path() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store.save(&record("session-1")).unwrap();
        fs::write(
            dir.path().join("session-1.json"),
            r#"{"version":1,"id":"sess"#,
        )
        .unwrap();

        let error = store.load("session-1").unwrap_err();

        assert!(
            matches!(error, SessionStoreError::Corrupt { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("session-1"), "{message}");
        assert!(message.contains("remove"), "{message}");
    }

    #[test]
    fn an_unknown_schema_version_is_refused_instead_of_guessed() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let future = record("session-1");
        fs::write(
            dir.path().join("session-1.json"),
            future
                .to_json()
                .unwrap()
                .replace(&format!("\"version\":{SCHEMA_VERSION}"), "\"version\":99"),
        )
        .unwrap();

        assert!(matches!(
            store.load("session-1"),
            Err(SessionStoreError::Schema(
                SessionError::UnsupportedVersion { found: 99, .. }
            ))
        ));
    }

    #[test]
    fn an_interrupted_write_leaves_the_previous_session_intact() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        let saved = record("session-1");
        store.save(&saved).unwrap();
        let replacement = SessionRecord::new(
            "session-1",
            "test-model",
            1_700_000_000_002,
            vec![SessionMessage::Assistant {
                text: "interrupted".into(),
            }],
            Vec::new(),
        )
        .unwrap();

        let error = store.save_with_write_failure(&replacement).unwrap_err();

        assert!(
            matches!(error, SessionStoreError::Write { .. }),
            "{error:?}"
        );
        assert_eq!(entries(dir.path()), ["session-1.json"]);
        assert_eq!(store.load("session-1").unwrap(), saved);
    }

    #[test]
    fn ids_cannot_escape_the_store_directory() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        for id in ["../escape", "nested/id", "", "  ", ".", ".."] {
            assert!(
                matches!(store.path_for(id), Err(SessionStoreError::InvalidId)),
                "id {id:?} must be refused"
            );
        }
    }

    #[test]
    fn the_store_root_follows_the_configured_data_directory() {
        let dir = tempdir().unwrap();
        let config = config_with_data_dir(dir.path());

        let store = SessionStore::for_config(&config);
        store.save(&record("session-1")).unwrap();

        assert_eq!(store.root(), dir.path().join("sessions"));
        assert!(dir.path().join("sessions").join("session-1.json").exists());
    }

    #[test]
    fn listing_reports_sorted_ids_and_ignores_unrelated_files() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store.save(&record("beta")).unwrap();
        store.save(&record("alpha")).unwrap();
        fs::write(dir.path().join("notes.txt"), "unrelated").unwrap();
        fs::write(dir.path().join(".alpha.json.1.tmp"), "debris").unwrap();

        assert_eq!(store.list_ids().unwrap(), ["alpha", "beta"]);
    }

    #[test]
    fn listing_an_absent_directory_is_empty_rather_than_an_error() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("never-created"));

        assert_eq!(store.list_ids().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn listing_includes_a_damaged_session_so_it_can_be_cleared() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store.save(&record("broken")).unwrap();
        fs::write(dir.path().join("broken.json"), "{\"version\":1").unwrap();

        assert_eq!(store.list_ids().unwrap(), ["broken"]);
        assert!(
            matches!(store.load("broken"), Err(SessionStoreError::Corrupt { .. })),
            "the damaged session is listed but not loadable"
        );
    }

    #[test]
    fn clearing_removes_the_file_and_a_second_clear_reports_it_missing() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store.save(&record("session-1")).unwrap();

        store.delete("session-1").unwrap();

        assert!(!dir.path().join("session-1.json").exists());
        assert_eq!(store.list_ids().unwrap(), Vec::<String>::new());
        assert!(matches!(
            store.delete("session-1"),
            Err(SessionStoreError::NotFound { .. })
        ));
    }

    #[test]
    fn a_damaged_session_can_still_be_cleared() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());
        store.save(&record("broken")).unwrap();
        fs::write(dir.path().join("broken.json"), "{\"version\":1").unwrap();

        assert!(store.exists("broken").unwrap());
        store.delete("broken").unwrap();
        assert!(!store.exists("broken").unwrap());
    }

    #[test]
    fn existence_and_clearing_refuse_ids_that_are_not_safe_file_names() {
        let dir = tempdir().unwrap();
        let store = SessionStore::new(dir.path());

        for id in ["../escape", "nested/id", "", "  "] {
            assert!(
                matches!(store.exists(id), Err(SessionStoreError::InvalidId)),
                "id {id:?} must be refused by exists"
            );
            assert!(
                matches!(store.delete(id), Err(SessionStoreError::InvalidId)),
                "id {id:?} must be refused by delete"
            );
        }
        assert!(!dir.path().join("escape.json").exists());
    }
}
