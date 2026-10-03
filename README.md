# ben

`ben` is a local-first coding-agent harness built as a terminal UI and a learning project. The approved design and 46-ticket roadmap are in [`docs/superpowers/`](docs/superpowers/); ticket status and merged-PR evidence live in [`PROGRESS.md`](PROGRESS.md).

## Project status

The repository is in planning and bootstrap. Rust application code starts with Task 1. GitHub issues track each roadmap ticket.

## Development workflow

See [`AGENTS.md`](AGENTS.md) for contributor and agent instructions.

- Check docs and ticket tracking: `scripts/check-docs.sh`
- Fast offline Rust loop: `scripts/verify-fast.sh [test filter]`
- Full offline local verification: `scripts/verify.sh`
- Record ticket PR state: `scripts/update_progress.py --task N --pr PR_NUMBER`
- Download locked Rust dependencies on a fresh checkout: `cargo fetch --locked`

Verification is CLI-only and does not need a browser or live model credentials. The Cargo scripts become usable after the Rust scaffold and dependency lockfile exist. Automated tests use fake providers and local fixtures; live provider measurements are opt-in.

## Stack direction

The plan recommends Rust, Ratatui/Crossterm, and Tokio for a native terminal app with explicit concurrency boundaries. It compares Go and TypeScript with reproducible workload measurements rather than assumed language-wide performance claims. See the [design spec](docs/superpowers/specs/2026-10-03-terminal-coding-agent-design.md).
