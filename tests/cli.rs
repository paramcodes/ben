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
        .current_dir(workspace.path())
        .output()
        .expect("agent should start with default options");

    assert!(output.status.success());
}

#[test]
fn accepts_an_explicit_workspace() {
    let workspace = tempfile::tempdir().expect("temporary workspace should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_ben"))
        .arg(workspace.path())
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
        help.contains("WORKSPACE"),
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
