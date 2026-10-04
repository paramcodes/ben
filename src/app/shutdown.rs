//! Idempotent shutdown coordination.
//!
//! The shutdown sequence is a single, linear, repeatable path. It signals
//! cancellation, stops the input poller, awaits tracked child processes and
//! spawned tasks, settles the session (save on quit, delete on clear), and
//! restores the terminal. Every stage still runs when an earlier stage fails,
//! so repeated shutdown is harmless and the terminal is never left in raw mode.
//!
//! The coordinator owns no terminal I/O of its own: restoration is done through
//! the `TerminalGuard` the TUI loop holds. Tests substitute a recording
//! terminal, a fake provider, and a fake child process to assert ordering and
//! error recovery.

use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        {Arc, Mutex},
    },
    time::{Duration, SystemTime},
};

use tokio::{
    task::JoinHandle,
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

use crate::{
    app::terminal::{TerminalControl, TerminalGuard},
    app::update::{AppState, Speaker},
    sessions::{
        SessionStore, SessionStoreError,
        model::{SessionMessage, SessionRecord, now_ms},
    },
};

/// How long shutdown waits for in-progress work before continuing cleanup.
pub const WORK_AWAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What to do with the session once the agent has stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShutdownAction {
    /// Persist the conversation so it can be resumed later.
    Quit,
    /// Remove the stored session; it must not be saved again.
    Clear { id: String },
}

/// A failure observed during shutdown cleanup. All failures are reported; no
/// single failure prevents the remaining stages from running.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShutdownError {
    #[error("in-progress work did not stop within {timeout:?}: {detail}")]
    Work {
        timeout: std::time::Duration,
        detail: String,
    },
    #[error("session state was not persisted: {0}")]
    Session(String),
    #[error("the terminal could not be restored: {0}")]
    Terminal(String),
}

/// Coordinates cancellation, child cleanup, session settlement, and terminal
/// restoration for exactly one application run.
pub struct Shutdown {
    store: SessionStore,
    cancellation: CancellationToken,
    stop_input: Arc<AtomicBool>,
    input: Option<JoinHandle<()>>,
    model: String,
    work: Vec<JoinHandle<()>>,
    work_timeout: std::time::Duration,
    finished: bool,
}

impl Shutdown {
    /// Build a coordinator around the handles the TUI loop owns: the
    /// cancellation token shared with provider/agent tasks, the input poller
    /// stop flag, the session store, and the configured model used when saving
    /// the conversation.
    pub fn new(
        store: SessionStore,
        model: impl Into<String>,
        cancellation: CancellationToken,
        stop_input: Arc<AtomicBool>,
    ) -> Self {
        Self {
            store,
            cancellation,
            stop_input,
            input: None,
            model: model.into(),
            work: Vec::new(),
            work_timeout: WORK_AWAIT_TIMEOUT,
            finished: false,
        }
    }

    /// Hands the input poller's join handle to the coordinator; it is stopped
    /// and awaited like any other owned task.
    pub fn set_input(&mut self, input: JoinHandle<()>) {
        self.input = Some(input);
    }

    /// Tracks a spawned task (provider stream, agent turn, or a task awaiting a
    /// child process) that must terminate before the session is saved.
    pub fn track(&mut self, work: JoinHandle<()>) {
        self.work.push(work);
    }

    /// Bounds how long shutdown waits for in-progress work.
    pub fn with_work_timeout(mut self, work_timeout: std::time::Duration) -> Self {
        self.work_timeout = work_timeout;
        self
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Runs the full shutdown sequence. Idempotent: a second call returns
    /// immediately with no new failures. Each failure is aggregated, but no
    /// failure skips the remaining stages.
    ///
    /// Stages, in order: (1) cancel in-flight work, (2) stop the input poller
    /// and await tracked tasks, (3) save or delete the session, (4) restore
    /// the terminal through `terminal`.
    pub async fn run<T: TerminalControl + 'static>(
        &mut self,
        action: ShutdownAction,
        state: &AppState,
        terminal: &mut TerminalGuard<T>,
    ) -> Vec<ShutdownError> {
        if self.finished {
            return Vec::new();
        }
        self.finished = true;
        let mut failures = Vec::new();

        // 1. Cancel in-flight work. This is a signal only; the await below
        //    verifies that the work actually stopped.
        self.cancellation.cancel();

        // 2. Stop the input poller and wait, with a bounded deadline, for every
        //    tracked task and child process to terminate.
        self.stop_input.store(true, Ordering::Relaxed);
        let deadline = Instant::now() + self.work_timeout;
        if let Some(input) = self.input.take()
            && let Some(detail) = wait_for_task(input, "the input task", deadline).await
        {
            failures.push(ShutdownError::Work {
                timeout: self.work_timeout,
                detail,
            });
        }
        for handle in self.work.drain(..) {
            if let Some(detail) = wait_for_task(handle, "a work task", deadline).await {
                failures.push(ShutdownError::Work {
                    timeout: self.work_timeout,
                    detail,
                });
            }
        }

        // 3. Persist or clear the session state.
        if let Err(error) = self.settle_session(&action, state) {
            failures.push(error);
        }

        // 4. Restore the terminal last, so the user never sees a half-cleaned
        //    screen while the application is still writing to it.
        if let Err(error) = terminal.restore() {
            failures.push(ShutdownError::Terminal(error.to_string()));
        }

        failures
    }

    fn settle_session(
        &mut self,
        action: &ShutdownAction,
        state: &AppState,
    ) -> Result<(), ShutdownError> {
        match action {
            ShutdownAction::Clear { id } => self.store.delete(id).map_err(|error| {
                ShutdownError::Session(format!("session {id} could not be cleared: {error}"))
            }),
            ShutdownAction::Quit => save_session(&self.store, &self.model, state)
                .map_err(|error| ShutdownError::Session(error.to_string())),
        }
    }
}

/// Awaits one task, surfacing any failure and reporting a timeout. Returns
/// `Some(message)` when the task did not stop cleanly.
async fn wait_for_task(
    mut handle: JoinHandle<()>,
    name: &'static str,
    deadline: Instant,
) -> Option<String> {
    if Instant::now() >= deadline {
        return last_look(&mut handle, name).await;
    }
    match timeout(deadline - Instant::now(), &mut handle).await {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("{name} failed: {error}")),
        Err(_) => last_look(&mut handle, name).await,
    }
}

/// Polls a task handle once without advancing the clock. Used to collect the
/// outcome of a task that finished right at the deadline, and to distinguish a
/// genuinely stuck task (`None` -> timeout) from a panicked one (`Err`).
async fn last_look(handle: &mut JoinHandle<()>, name: &'static str) -> Option<String> {
    match timeout(Duration::ZERO, handle).await {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("{name} failed: {error}")),
        Err(_) => Some(format!("{name} did not stop before the shutdown deadline")),
    }
}

/// Saves the visible conversation so it can be resumed later. System notices
/// and empty conversations leave no file behind.
fn save_session(
    store: &SessionStore,
    model: &str,
    state: &AppState,
) -> Result<(), SessionStoreError> {
    let messages = state
        .transcript
        .iter()
        .filter(|entry| !matches!(entry.speaker, Speaker::System))
        .map(|entry| match entry.speaker {
            Speaker::User => SessionMessage::User {
                text: entry.text.clone(),
            },
            Speaker::Assistant => SessionMessage::Assistant {
                text: entry.text.clone(),
            },
            Speaker::Tool => SessionMessage::ToolResult {
                call_id: String::new(),
                content: entry.text.clone(),
            },
            Speaker::System => unreachable!("system entries are filtered above"),
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return Ok(());
    }

    let (id, created_at_ms, used_model) = match &state.resumed_session_id {
        Some(existing) => {
            let loaded = store.load(existing).ok();
            let created_at_ms = loaded
                .as_ref()
                .map(|record| record.created_at_ms)
                .unwrap_or_else(|| now_ms().unwrap_or(0));
            let used_model = loaded
                .as_ref()
                .map(|record| record.model.clone())
                .unwrap_or_else(|| model.to_string());
            (existing.clone(), created_at_ms, used_model)
        }
        None => (store.new_id(), now_ms().unwrap_or(0), model.to_string()),
    };

    let mut record = SessionRecord::new(id, used_model, created_at_ms, messages, Vec::new())?;
    record.updated_at_ms = now_ms().unwrap_or(record.created_at_ms);
    store.save(&record).map(|_| ())
}

/// A `TerminalControl` that records enter/restore calls for tests. Restoration
/// also records the wall-clock moment it ran, so tests can order the save stage
/// (which writes a file) against the restore stage.
#[derive(Debug, Clone)]
pub struct RecordingControl {
    events: Arc<Mutex<Vec<&'static str>>>,
    restored_at: Arc<Mutex<Option<SystemTime>>>,
    restore_fails: bool,
}

impl Default for RecordingControl {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingControl {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            restored_at: Arc::new(Mutex::new(None)),
            restore_fails: false,
        }
    }

    pub fn failing_restore() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            restored_at: Arc::new(Mutex::new(None)),
            restore_fails: true,
        }
    }

    pub fn events(&self) -> Vec<&'static str> {
        self.events.lock().unwrap().clone()
    }

    pub fn restored_at(&self) -> Option<SystemTime> {
        *self.restored_at.lock().unwrap()
    }
}

impl TerminalControl for RecordingControl {
    fn enter(&mut self) -> io::Result<()> {
        self.events.lock().unwrap().push("enter");
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        self.events.lock().unwrap().push("restore");
        *self.restored_at.lock().unwrap() = Some(SystemTime::now());
        if self.restore_fails {
            Err(io::Error::other("restore failed"))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use tempfile::tempdir;
    use tokio::{sync::mpsc, task::JoinHandle};
    use tokio_util::sync::CancellationToken;

    use super::{RecordingControl, Shutdown, ShutdownAction, ShutdownError};
    use crate::{
        agent::message::Message,
        app::terminal::TerminalGuard,
        app::update::{AppState, Speaker, TranscriptEntry},
        app::{event::AppEvent, spawn_provider_stream},
        providers::types::{
            Provider, ProviderCancellation, ProviderError, ProviderEvent, ProviderRequest,
            ProviderStream,
        },
        sessions::{
            SessionStore,
            model::{SessionMessage, SessionRecord},
        },
    };

    fn state_with_conversation() -> AppState {
        AppState {
            transcript: vec![
                TranscriptEntry {
                    speaker: Speaker::User,
                    text: "update the notes".into(),
                },
                TranscriptEntry {
                    speaker: Speaker::Assistant,
                    text: "updated notes.txt".into(),
                },
            ],
            ..AppState::default()
        }
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "test-model".into(),
            messages: vec![Message::user("hello")],
            tools: Vec::new(),
            max_output_tokens: None,
        }
    }

    fn record(id: &str) -> SessionRecord {
        SessionRecord::new(
            id,
            "test-model",
            1_700_000_000_000,
            vec![SessionMessage::User {
                text: "hello".into(),
            }],
            Vec::new(),
        )
        .unwrap()
    }

    /// Returns the sorted `*.json` stems in `dir` for deterministic assertions.
    fn saved_ids(dir: &Path) -> Vec<String> {
        let mut ids: Vec<String> = dir
            .read_dir()
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_owned();
                name.strip_suffix(".json").map(|stem| stem.to_owned())
            })
            .collect();
        ids.sort();
        ids
    }

    /// A provider whose stream never yields, so it can only complete once the
    /// shutdown cancellation reaches it.
    struct StuckProvider;

    impl Provider for StuckProvider {
        fn stream(
            &self,
            _request: ProviderRequest,
            _cancellation: ProviderCancellation,
        ) -> ProviderStream {
            Box::pin(futures_util::stream::pending::<
                Result<ProviderEvent, ProviderError>,
            >())
        }
    }

    #[tokio::test]
    async fn cancel_before_await_before_save_before_restore() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));

        // Work #1: a provider stream that can only stop after cancellation fires.
        let (stream_sender, _stream_receiver) = mpsc::channel::<io::Result<AppEvent>>(32);
        let provider_task = spawn_provider_stream(
            Arc::new(StuckProvider),
            request(),
            cancellation.clone(),
            stream_sender,
        );

        // Work #2: a fake child process the shutdown must wait for.
        let child_finished = Arc::new(AtomicBool::new(false));
        let child_task: JoinHandle<()> = tokio::spawn({
            let finished = Arc::clone(&child_finished);
            async move {
                let mut child = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg("sleep 0.2")
                    .spawn()
                    .expect("spawn a fake child process");
                let result = child.wait().await;
                finished.store(true, Ordering::SeqCst);
                assert!(result.is_ok(), "the fake child must terminate");
            }
        });

        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation.clone(),
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_secs(5));
        shutdown.track(provider_task);
        shutdown.track(child_task);

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;
        assert!(
            failures.is_empty(),
            "a clean shutdown must not report errors: {failures:?}"
        );

        // Each stage's effect is observable. On the single-threaded test runtime
        // the stages run in program order, so these effects demonstrate the
        // sequence cancel -> await -> save -> restore.
        // Stage 1 (cancel).
        assert!(cancellation.is_cancelled());
        // Stage 2 (await): the child process stopped before continuing.
        assert!(child_finished.load(Ordering::SeqCst));
        // Stage 3 (save): exactly one session file was written.
        let mut ids = saved_ids(dir.path());
        assert_eq!(ids.len(), 1, "the conversation must be saved once: {ids:?}");
        let saved_path = dir.path().join(format!("{}.json", ids.remove(0)));
        let file_mtime = fs::metadata(&saved_path)
            .expect("saved session file")
            .modified()
            .unwrap();
        // Stage 4 (restore): last touch on the terminal, and only once.
        let restored_at = control.restored_at().expect("restore ran");
        assert_eq!(control.events(), ["enter", "restore"]);
        // The save runs in stage 3, restore in stage 4: the saved file must
        // predate the restore instant (robust to filesystem mtime granularity,
        // which only ever rounds the save timestamp downward).
        assert!(file_mtime <= restored_at, "save must precede restore");
    }

    #[tokio::test]
    async fn repeated_shutdown_runs_cleanup_only_once() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation.clone(),
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_secs(5));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let first = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;
        let second = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;

        assert!(first.is_empty(), "{first:?}");
        assert!(
            second.is_empty(),
            "a second shutdown must not report errors: {second:?}"
        );
        assert_eq!(control.events(), ["enter", "restore"]);
        assert_eq!(
            saved_ids(dir.path()).len(),
            1,
            "the session must be saved exactly once"
        );
        assert!(shutdown.is_finished());
    }

    #[tokio::test]
    async fn a_failing_work_task_does_not_stop_the_rest_of_cleanup() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation,
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_secs(5));
        shutdown.track(tokio::spawn(async {
            panic!("simulated work failure");
        }));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;

        let work_failure = failures
            .iter()
            .find(|failure| matches!(failure, ShutdownError::Work { .. }))
            .expect("a panicking task must be reported as a work failure");
        assert!(work_failure.to_string().contains("failed"));
        assert_eq!(
            saved_ids(dir.path()).len(),
            1,
            "cleanup must still save the session"
        );
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn stuck_work_does_not_hold_shutdown_hostage() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation,
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_millis(120));
        shutdown.track(tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let start = std::time::Instant::now();
        let failures = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;
        let elapsed = start.elapsed();

        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(matches!(failures[0], ShutdownError::Work { .. }));
        assert!(failures[0].to_string().contains("within 120ms"));
        assert!(elapsed < Duration::from_secs(5));
        assert_eq!(saved_ids(dir.path()).len(), 1);
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn a_failed_session_save_still_restores_the_terminal() {
        let dir = tempdir().unwrap();
        let blocking = dir.path().join("store-is-a-file");
        fs::write(&blocking, "x").unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(blocking),
            "test-model",
            cancellation,
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_millis(100));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;

        let session_failure = failures
            .iter()
            .find(|failure| matches!(failure, ShutdownError::Session(_)))
            .expect("a save failure must be reported as a session error");
        assert!(session_failure.to_string().contains("was not persisted"));
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn a_terminal_restore_failure_is_reported_and_save_ran_first() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::failing_restore();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation,
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_secs(5));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;

        let terminal_failure = failures
            .iter()
            .find(|failure| matches!(failure, ShutdownError::Terminal(_)))
            .expect("a failed restore must be reported as a terminal error");
        assert!(
            terminal_failure
                .to_string()
                .contains("could not be restored")
        );
        assert_eq!(saved_ids(dir.path()).len(), 1, "save runs before restore");
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn quitting_an_empty_conversation_leaves_no_session_behind() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation,
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_secs(5));

        let state = AppState::default();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;
        assert!(failures.is_empty(), "{failures:?}");
        assert!(
            saved_ids(dir.path()).is_empty(),
            "no session must be saved for an empty conversation"
        );
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn quitting_a_resumed_session_keeps_its_id_and_creation_time() {
        let dir = tempdir().unwrap();
        let seeded = SessionStore::new(dir.path());
        seeded.save(&record("session-1")).unwrap();

        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation.clone(),
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_secs(5));

        let state = AppState {
            resumed_session_id: Some("session-1".into()),
            transcript: vec![
                TranscriptEntry {
                    speaker: Speaker::User,
                    text: "hi".into(),
                },
                TranscriptEntry {
                    speaker: Speaker::Assistant,
                    text: "updated".into(),
                },
            ],
            ..AppState::default()
        };
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown.run(ShutdownAction::Quit, &state, &mut guard).await;
        assert!(failures.is_empty(), "{failures:?}");

        let loaded = SessionStore::new(dir.path()).load("session-1").unwrap();
        assert_eq!(loaded.id, "session-1");
        assert_eq!(loaded.model, "test-model");
        assert_eq!(loaded.created_at_ms, 1_700_000_000_000);
        assert!(loaded.updated_at_ms >= loaded.created_at_ms);
        assert_eq!(
            loaded.messages.len(),
            2,
            "the resumed transcript must persist"
        );
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn a_confirmed_clear_deletes_the_session_without_resaving() {
        let dir = tempdir().unwrap();
        let seeded = SessionStore::new(dir.path());
        seeded.save(&record("session-1")).unwrap();
        assert_eq!(saved_ids(dir.path()).len(), 1);

        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation.clone(),
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_secs(5));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown
            .run(
                ShutdownAction::Clear {
                    id: "session-1".into(),
                },
                &state,
                &mut guard,
            )
            .await;
        assert!(failures.is_empty(), "{failures:?}");
        assert!(
            SessionStore::new(dir.path()).load("session-1").is_err(),
            "the cleared session must be gone"
        );
        assert!(
            saved_ids(dir.path()).is_empty(),
            "a cleared session must not be written back"
        );
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn clearing_a_missing_session_is_reported_and_the_terminal_is_restored() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation,
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_millis(100));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown
            .run(
                ShutdownAction::Clear {
                    id: "absent".into(),
                },
                &state,
                &mut guard,
            )
            .await;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(matches!(failures[0], ShutdownError::Session(_)));
        assert_eq!(control.events(), ["enter", "restore"]);
    }

    #[tokio::test]
    async fn clearing_refuses_an_unsafe_identifier_without_touching_the_store() {
        let dir = tempdir().unwrap();
        let control = RecordingControl::new();
        let cancellation = CancellationToken::new();
        let stop_input = Arc::new(AtomicBool::new(false));
        let mut shutdown = Shutdown::new(
            SessionStore::new(dir.path()),
            "test-model",
            cancellation,
            Arc::clone(&stop_input),
        )
        .with_work_timeout(Duration::from_millis(100));

        let state = state_with_conversation();
        let mut guard = TerminalGuard::new(control.clone());
        guard.enter().unwrap();

        let failures = shutdown
            .run(
                ShutdownAction::Clear {
                    id: "../escape".into(),
                },
                &state,
                &mut guard,
            )
            .await;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(matches!(failures[0], ShutdownError::Session(_)));
        assert!(failures[0].to_string().contains("could not be cleared"));
        assert!(
            saved_ids(dir.path()).is_empty(),
            "an unsafe id must not write a file outside the session directory"
        );
        assert_eq!(control.events(), ["enter", "restore"]);
    }
}
