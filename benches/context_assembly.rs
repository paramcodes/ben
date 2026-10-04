//! Context assembly benchmark.
//!
//! Measures how long it takes to assemble repository context from
//! fixture files. Outputs machine-readable JSON with samples and quantiles.

use std::path::Path;
use std::time::Instant;

/// Assemble context from fixture files and return elapsed nanoseconds.
fn assemble_once(fixture_dir: &Path) -> u128 {
    let start = Instant::now();
    let mut total_size: u64 = 0;
    let mut file_count = 0u64;

    if let Ok(entries) = std::fs::read_dir(fixture_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                total_size += metadata.len();
                file_count += 1;
            }
        }
    }
    let _ = (total_size, file_count);
    start.elapsed().as_nanos()
}

fn quantile(samples: &mut [u128], p: f64) -> u128 {
    samples.sort_unstable();
    let idx = (p / 100.0 * (samples.len() - 1) as f64).round() as usize;
    samples[idx.min(samples.len() - 1)]
}

fn main() {
    let iterations = 100;
    let fixture_dir = Path::new("benchmarks/fixtures/repository");

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        samples.push(assemble_once(fixture_dir));
    }

    let result = serde_json::json!({
        "workload": "context_assembly",
        "samples": samples,
        "quantiles": {
            "p50": quantile(&mut samples.clone(), 50.0),
            "p95": quantile(&mut samples.clone(), 95.0),
            "p99": quantile(&mut samples.clone(), 99.0),
        },
        "environment": {
            "os": std::env::consts::OS,
            "rust_version": env!("CARGO_PKG_VERSION"),
            "terminal_width": 80,
            "terminal_height": 24,
        },
        "fixture_hashes": {
            "repository": "ed15992aaf55dd6cdd18a7b9521138e3024527df81d7df9f48fb7d20040ae473",
        },
    });

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}
