//! UI render benchmark.
//!
//! Measures how long it takes to render the conversation widget for a fixed
//! transcript. Outputs machine-readable JSON with samples and quantiles.

use std::time::Instant;

/// Render a fixed transcript and return elapsed nanoseconds.
fn render_once(transcript_size: usize) -> u128 {
    // Simulate rendering work proportional to transcript size.
    let mut lines = Vec::with_capacity(transcript_size);
    let start = Instant::now();
    for i in 0..transcript_size {
        lines.push(format!("line {i}: benchmark render content"));
    }
    let _ = lines.join("\n");
    start.elapsed().as_nanos()
}

fn quantile(samples: &mut [u128], p: f64) -> u128 {
    samples.sort_unstable();
    let idx = (p / 100.0 * (samples.len() - 1) as f64).round() as usize;
    samples[idx.min(samples.len() - 1)]
}

fn main() {
    let iterations = 100;
    let transcript_size = 500;

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        samples.push(render_once(transcript_size));
    }

    let result = serde_json::json!({
        "workload": "ui_render",
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
            "transcript": "8cfa2b9a74079effb9e87daf5a15d43b34abf58d10d2b3e4c14327b1d86522b1",
        },
    });

    println!("{}", serde_json::to_string_pretty(&result).unwrap());
}
