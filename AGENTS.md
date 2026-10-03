# Repository guidance

## Project shape

This repository builds a local-first Rust coding agent whose primary interface is a terminal UI. Read `docs/superpowers/specs/2026-10-03-terminal-coding-agent-design.md` for product boundaries and `docs/superpowers/plans/2026-10-03-terminal-coding-agent.md` for ticket interfaces and acceptance checks. `PROGRESS.md` is the durable ticket ledger.

## Ticket workflow

- Work on one GitHub issue per branch and PR. Include `Closes #N` in the PR body.
- Before coding, read the issue, its plan task, the relevant spec sections, and prerequisite tasks' interfaces.
- Keep changes within the ticket. If the plan conflicts with the spec, follow the spec and record the decision in `PROGRESS.md` and the PR description.
- Keep external model calls out of automated checks. Use the deterministic fake provider and local mock HTTP server.
- Update `PROGRESS.md` in the same PR: mark a ticket `In progress` while its PR is open and `Complete` only after merge; record merged PR number and link as the completion evidence.
- A local commit or an open PR is not completion. Merge only after required checks pass and the diff has been reviewed.
- PRs should include What changed, Why, How it works, Verification commands and results, and linked issue.

## Rust conventions

- Use stable Rust and keep dependency versions/features in `Cargo.toml` deliberate and locked in `Cargo.lock`.
- Keep UI rendering, app state transitions, agent orchestration, provider wire format, tools, and policy in their planned module boundaries.
- Parse and validate model-supplied tool arguments before use. Treat paths, commands, provider output, and persisted session data as untrusted input.
- Keep API keys out of logs, session files, snapshots, and error messages. Never add a live credential to fixtures.
- Keep network and child-process work outside the TUI input/render loop. Propagate cancellation and bound time, bytes, and tool-call count.
- Avoid unrelated refactors and new abstractions without a concrete second use.

## Verification

This is a CLI/TUI project. Use local command-line checks only; browser automation is not part of this repository. Automated verification must work without a browser, model API key, or network access after dependencies are cached.

- Fast loop: `scripts/verify-fast.sh [optional cargo test filters]`
- Full local loop: `scripts/verify.sh`
- Documentation/ticket consistency: `scripts/check-docs.sh`
- Run a focused test while iterating, then the full local loop before opening a PR.
- Live provider benchmarks are opt-in and are never part of default verification.
- On a fresh checkout, dependency download is a separate explicit setup action: `cargo fetch --locked`. The verification scripts themselves run Cargo offline.

## PR and merge record

Use `.github/pull_request_template.md`. After a PR merges, update the matching row in `PROGRESS.md` with the merged PR number and URL. Keep issue state synchronized with the progress ledger. Do not mark work complete based only on passing local checks.
