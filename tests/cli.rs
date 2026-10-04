use std::process::Command;

#[test]
fn version_flag_prints_package_name_and_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_ben"))
        .arg("--version")
        .output()
        .expect("version command should start");

    assert!(
        output.status.success(),
        "--version should exit successfully"
    );

    let stdout = String::from_utf8(output.stdout).expect("version output should be UTF-8");
    assert!(
        stdout.contains(env!("CARGO_PKG_NAME")),
        "output should contain package name: {stdout}"
    );
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "output should contain package version: {stdout}"
    );
}

#[test]
fn defaults_to_the_current_workspace() {
    let workspace = tempfile::tempdir().expect("temporary workspace should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_ben"))
        .env("OPENAI_API_KEY", "test-api-key")
        .current_dir(workspace.path())
        .output()
        .expect("agent should start with default options");

    assert!(output.status.success());
}

#[test]
fn accepts_an_explicit_workspace() {
    let workspace = tempfile::tempdir().expect("temporary workspace should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_ben"))
        .env("OPENAI_API_KEY", "test-api-key")
        .args([
            "-C",
            workspace.path().to_str().expect("path should be UTF-8"),
        ])
        .output()
        .expect("agent should accept an explicit workspace");

    assert!(output.status.success());
}

#[test]
fn help_lists_workspace_and_model_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_ben"))
        .arg("--help")
        .output()
        .expect("help command should start");

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help output should be UTF-8");
    assert!(help.contains("Usage:"), "help should include usage: {help}");
    assert!(
        help.contains("-C") || help.contains("--directory"),
        "help should document workspace: {help}"
    );
    assert!(
        help.contains("--model"),
        "help should document model: {help}"
    );
}

#[test]
fn rejects_an_empty_model_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_ben"))
        .args(["--model", ""])
        .output()
        .expect("agent command should start");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(
        stderr.contains("invalid value"),
        "expected a parse error: {stderr}"
    );
}

#[test]
fn missing_api_key_reports_setup_instructions() {
    let output = Command::new(env!("CARGO_BIN_EXE_ben"))
        .env_remove("OPENAI_API_KEY")
        .env_remove("BEN_TEST_STARTUP_ERROR")
        .output()
        .expect("agent should report missing configuration");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("error output should be UTF-8");
    assert!(
        stderr.contains("OPENAI_API_KEY is required"),
        "expected credential setup instructions: {stderr}"
    );
}

/// Session flows run against an isolated data directory so the checks never
/// read or write a developer's real sessions.
mod sessions {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    const SESSION_JSON: &str = r#"{
        "version": 1,
        "id": "session-1",
        "model": "test-model",
        "created_at_ms": 1791090232000,
        "updated_at_ms": 1791090232000,
        "messages": [
            {"role": "user", "text": "update the notes"},
            {"role": "assistant", "text": "updated notes.txt"}
        ],
        "tool_calls": []
    }"#;

    fn sessions_root(data_dir: &Path) -> PathBuf {
        data_dir.join("sessions")
    }

    fn write_session(data_dir: &Path, id: &str) -> PathBuf {
        fs::create_dir_all(sessions_root(data_dir)).expect("session directory should be created");
        let path = sessions_root(data_dir).join(format!("{id}.json"));
        fs::write(&path, SESSION_JSON.replace("session-1", id))
            .expect("session file should be written");
        path
    }

    /// Runs the binary with a private data directory and no controlling
    /// terminal, so session commands never enter the alternate screen.
    fn ben(data_dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_ben"))
            .env("OPENAI_API_KEY", "test-api-key")
            .env("BEN_DATA_DIR", data_dir)
            .env_remove("BEN_TEST_STARTUP_ERROR")
            .args(args)
            .output()
            .expect("agent should start")
    }

    fn stdout_of(output: &std::process::Output) -> String {
        String::from_utf8(output.stdout.clone()).expect("stdout should be UTF-8")
    }

    fn stderr_of(output: &std::process::Output) -> String {
        String::from_utf8(output.stderr.clone()).expect("stderr should be UTF-8")
    }

    #[test]
    fn lists_saved_session_identifiers() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");
        write_session(data_dir.path(), "alpha");
        write_session(data_dir.path(), "beta");

        let output = ben(data_dir.path(), &["sessions", "list"]);

        assert!(
            output.status.success(),
            "listing should succeed: {}",
            stderr_of(&output)
        );
        let stdout = stdout_of(&output);
        assert!(
            stdout.contains("alpha"),
            "listing should name alpha: {stdout}"
        );
        assert!(
            stdout.contains("beta"),
            "listing should name beta: {stdout}"
        );
        assert!(
            !stdout.contains("No saved sessions."),
            "a populated store should not report itself empty: {stdout}"
        );
    }

    #[test]
    fn reports_an_empty_store_without_failing() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");

        let output = ben(data_dir.path(), &["sessions", "list"]);

        assert!(
            output.status.success(),
            "listing an unused store should succeed: {}",
            stderr_of(&output)
        );
        assert!(
            stdout_of(&output).contains("No saved sessions."),
            "an empty store should say so: {}",
            stdout_of(&output)
        );
    }

    #[test]
    fn resuming_a_missing_session_reports_the_identifier_and_next_step() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");

        let output = ben(data_dir.path(), &["sessions", "resume", "absent"]);

        assert!(
            !output.status.success(),
            "resuming an unknown session should fail"
        );
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("absent"),
            "the error should name the session: {stderr}"
        );
        assert!(
            stderr.contains("ben sessions list"),
            "the error should suggest listing sessions: {stderr}"
        );
        assert!(
            !stderr.contains("panicked at"),
            "a missing session must not panic: {stderr}"
        );
    }

    #[test]
    fn resuming_a_damaged_session_reports_a_recovery_path() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");
        write_session(data_dir.path(), "damaged");
        fs::write(
            sessions_root(data_dir.path()).join("damaged.json"),
            "{\"version\":1",
        )
        .expect("truncated session should be written");

        let output = ben(data_dir.path(), &["sessions", "resume", "damaged"]);

        assert!(
            !output.status.success(),
            "resuming a damaged session should fail"
        );
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("ben sessions list"),
            "the error should suggest a recovery step: {stderr}"
        );
        assert!(
            !stderr.contains("panicked at"),
            "a damaged session must not panic: {stderr}"
        );
    }

    #[test]
    fn resuming_a_saved_session_starts_without_error() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");
        write_session(data_dir.path(), "resumable");

        let output = ben(data_dir.path(), &["sessions", "resume", "resumable"]);

        assert!(
            output.status.success(),
            "resuming a readable session should succeed: {}",
            stderr_of(&output)
        );
        assert!(
            !stderr_of(&output).contains("panicked at"),
            "resuming must not panic: {}",
            stderr_of(&output)
        );
    }

    #[test]
    fn clearing_a_missing_session_reports_the_identifier() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");

        let output = ben(data_dir.path(), &["sessions", "clear", "absent"]);

        assert!(
            !output.status.success(),
            "clearing an unknown session should fail"
        );
        assert!(
            stderr_of(&output).contains("absent"),
            "the error should name the session: {}",
            stderr_of(&output)
        );
    }

    #[test]
    fn clearing_refuses_an_unsafe_identifier() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");

        let output = ben(data_dir.path(), &["sessions", "clear", "../escape"]);

        assert!(
            !output.status.success(),
            "a traversing identifier must be refused"
        );
        let stderr = stderr_of(&output);
        assert!(
            stderr.contains("not a safe file name"),
            "the error should explain the refusal: {stderr}"
        );
    }

    #[test]
    fn session_commands_are_documented_in_help() {
        let data_dir = tempfile::tempdir().expect("temporary data directory should be created");

        let output = ben(data_dir.path(), &["--help"]);

        assert!(output.status.success());
        let help = stdout_of(&output);
        assert!(
            help.contains("sessions"),
            "help should document the session commands: {help}"
        );
        assert!(
            help.contains("--directory") || help.contains("-C"),
            "help should document how to set the workspace: {help}"
        );
    }
}
