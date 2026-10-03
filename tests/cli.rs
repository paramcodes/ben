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
