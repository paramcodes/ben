//! Dry-run test: proves no credential or network is needed to validate the report schema.

#[test]
fn dry_run_produces_valid_schema_without_credentials() {
    let output = std::process::Command::new("cargo")
        .args(["run", "--bin", "provider_loop"])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("failed to run provider_loop");

    assert!(
        output.status.success(),
        "dry run should succeed without credentials"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("output should be valid JSON");

    assert_eq!(report["workload"], "provider_loop");
    assert_eq!(report["dry_run"], true);
    assert!(report["model"].is_null(), "dry run must not record model");
    assert!(
        report["endpoint"].is_null(),
        "dry run must not record endpoint"
    );
    assert!(report["fixture_hashes"].is_object());
}

#[test]
fn dry_run_rejects_live_mode_without_api_key() {
    let output = std::process::Command::new("cargo")
        .args([
            "run",
            "--bin",
            "provider_loop",
            "--",
            "--live-provider-benchmark",
        ])
        .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("failed to run provider_loop");

    assert!(
        !output.status.success(),
        "live mode without API key should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("OPENAI_API_KEY"),
        "error should mention missing API key"
    );
}
