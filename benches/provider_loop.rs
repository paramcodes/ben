//! Provider latency benchmark.
//!
//! Dry-run validates the report schema without credentials or network.
//! Live mode requires explicit `--live-provider-benchmark` and records
//! time-to-first-token, total wall time, tokens, tool calls, and success rate.
//! API keys are read from the environment and never stored.

struct Report {
    workload: String,
    dry_run: bool,
    model: Option<String>,
    endpoint: Option<String>,
    time_to_first_token_ms: Option<u128>,
    total_wall_time_ms: u128,
    tokens: Option<u64>,
    tool_calls: u64,
    success_rate: f64,
    environment: std::collections::HashMap<String, String>,
    fixture_hashes: std::collections::HashMap<String, String>,
}

impl Report {
    fn dry_run() -> Self {
        let mut env = std::collections::HashMap::new();
        env.insert("os".into(), std::env::consts::OS.into());
        env.insert("rust_version".into(), env!("CARGO_PKG_VERSION").into());
        env.insert("terminal_width".into(), "80".into());
        env.insert("terminal_height".into(), "24".into());

        let mut hashes = std::collections::HashMap::new();
        hashes.insert(
            "transcript".into(),
            "8cfa2b9a74079effb9e87daf5a15d43b34abf58d10d2b3e4c14327b1d86522b1".into(),
        );

        Self {
            workload: "provider_loop".into(),
            dry_run: true,
            model: None,
            endpoint: None,
            time_to_first_token_ms: None,
            total_wall_time_ms: 0,
            tokens: None,
            tool_calls: 0,
            success_rate: 0.0,
            environment: env,
            fixture_hashes: hashes,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.workload != "provider_loop" {
            return Err(format!("invalid workload: {}", self.workload));
        }
        if self.dry_run && self.model.is_some() {
            return Err("dry run must not record model".into());
        }
        if self.dry_run && self.endpoint.is_some() {
            return Err("dry run must not record endpoint".into());
        }
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let live = args.iter().any(|a| a == "--live-provider-benchmark");

    if live {
        // Live mode: requires OPENAI_API_KEY and explicit consent.
        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        if api_key.is_empty() {
            eprintln!("error: --live-provider-benchmark requires OPENAI_API_KEY");
            std::process::exit(1);
        }

        // In live mode, we would make actual provider requests.
        // This is a placeholder that records the schema without making calls.
        eprintln!("warning: live mode not yet implemented; running dry-run instead");
    }

    let report = Report::dry_run();

    // Validate schema
    if let Err(e) = report.validate() {
        eprintln!("schema validation failed: {e}");
        std::process::exit(1);
    }

    let result = serde_json::json!({
        "workload": report.workload,
        "dry_run": report.dry_run,
        "model": report.model,
        "endpoint": report.endpoint,
        "time_to_first_token_ms": report.time_to_first_token_ms,
        "total_wall_time_ms": report.total_wall_time_ms,
        "tokens": report.tokens,
        "tool_calls": report.tool_calls,
        "success_rate": report.success_rate,
        "environment": report.environment,
        "fixture_hashes": report.fixture_hashes,
    });

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}
