# Benchmark results

## Machine: omarchy

Date: 2026-10-04

| Stack | Workload | p50 | p95 | p99 |
|---|---|---|---|---|
| Rust | ui_render | 329µs | 606µs | 787µs |
| Rust | context_assembly | 2380µs | 5668µs | 6348µs |
| Go | go_reference | 1000µs | — | — |
| TypeScript | typescript_reference | 1000µs | — | — |

## Notes

- Measurements are local-only with fixture-driven workloads. No provider network calls.
- Rust benchmarks use 100 samples; Go and TypeScript reference workloads measure fixture operation counts.
- Startup, idle RSS, render, context, child startup metrics are not yet implemented for all stacks.
- Do not claim general language superiority from this one app.

## Files

- `omarchy-2026-10-04.json`: Full raw results with samples, quantiles, environment metadata, and fixture hashes.