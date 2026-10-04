# Benchmarks

Reproducible workload measurements for the `ben` terminal coding agent.

## Fixtures

| Fixture | Size | Hash |
|---|---|---|
| Transcript | 500 messages / 100 tool rows | `8cfa2b9a74079effb9e87daf5a15d43b34abf58d10d2b3e4c14327b1d86522b1` |
| Repository | 1,000 files / 100 MiB | `ed15992aaf55dd6cdd18a7b9521138e3024527df81d7df9f48fb7d20040ae473` |

Run `scripts/check-benchmarks.sh` to verify fixture integrity before any benchmark run.

## Measurement protocols

| Workload | Runs | Warmup / sampling |
|---|---|---|
| Cold-start | 20 | none |
| Idle RSS | 10 s warmup / 30 s sampling | steady-state |
| Input-to-frame | 1,000 events | none |
| Render | 10,000 frames | none |
| Context assembly | 100 | none |
| Process launch | 100 | none |
| End-to-end | 30 | none |

## Output schema

Each benchmark emits machine-readable JSON with:

- `workload`: workload name
- `samples`: array of measured values
- `quantiles`: p50, p95, p99 where meaningful
- `environment`: OS, Rust version, toolchain, terminal dimensions
- `fixture_hashes`: sha256 of transcript and repository fixtures

## Notes

- Benchmarks use local fixtures only; no provider network calls in local benchmarks.
- Results are stored in `benchmarks/results/`.
- Compare equivalent algorithms across stacks; do not claim general language superiority from a single application.