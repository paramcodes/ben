use std::process::Command;

fn ben() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ben"))
}

#[test]
fn version_command_starts_successfully() {
    let output = ben()
        .env_remove("BEN_TEST_STARTUP_ERROR")
        .env("RUST_LOG", "ben=debug")
        .arg("--version")
        .output()
        .expect("ben should start");

    assert!(output.status.success(), "--version failed: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains(env!("CARGO_PKG_NAME")));
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.contains("startup initialized"),
        "RUST_LOG should enable startup diagnostics: {stderr}"
    );
}

#[test]
fn startup_error_is_reported_without_a_panic_dump() {
    let output = ben()
        .env("BEN_TEST_STARTUP_ERROR", "1")
        .env("RUST_LOG", "error")
        .env("OPENAI_API_KEY", "startup-test-secret")
        .output()
        .expect("ben should start");

    assert!(
        !output.status.success(),
        "simulated startup error succeeded"
    );
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.contains("error:"),
        "stderr lacks an error prefix: {stderr}"
    );
    assert!(
        stderr.contains("application startup failed"),
        "tracing should report the error category: {stderr}"
    );
    assert!(
        !stderr.contains("startup-test-secret"),
        "stderr must not expose configuration secrets: {stderr}"
    );
    assert!(
        stderr.lines().any(|line| {
            let Some(message) = line.split_once("error:").map(|(_, rest)| rest.trim()) else {
                return false;
            };
            !message.is_empty() && message.chars().any(char::is_alphabetic)
        }),
        "stderr should include an actionable error message: {stderr}"
    );
    assert!(
        !stderr.contains("panicked at"),
        "stderr contains panic output: {stderr}"
    );
    assert!(
        !stderr.contains("stack backtrace"),
        "stderr contains a backtrace: {stderr}"
    );
}
