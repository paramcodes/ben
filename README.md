# ben

`ben` is a local-first coding-agent harness built as a terminal UI and a learning project. The approved design and 46-ticket roadmap are in [`docs/superpowers/`](docs/superpowers/); ticket status and merged-PR evidence live in [`PROGRESS.md`](PROGRESS.md).

## Project status

The repository is in planning and bootstrap. Rust application code starts with Task 1. GitHub issues track each roadmap ticket.

## Prerequisites

- Rust 1.88.0 or newer ([rustup](https://rustup.rs/) recommended)
- An OpenAI API key with Responses API access
- A terminal that supports 256 colors or better (XTerm, iTerm2, Windows Terminal)

## Install and build

```bash
cargo build --release
```

The binary is `target/release/ben` (or `target/debug/ben` for development).

## API key setup

Set your API key as an environment variable before running. The application reads `OPENAI_API_KEY` and never writes it to session files or logs.

```bash
export OPENAI_API_KEY=sk-your-key-here
```

## First run

Start `ben` in the current directory:

```bash
./target/release/ben
```

Or specify a workspace directory:

```bash
./target/release/ben -C /path/to/workspace
```

Use a specific model:

```bash
./target/release/ben --model gpt-4.1
```

Session commands:

```bash
./target/release/ben sessions list         # list saved sessions
./target/release/ben sessions resume <id>  # resume a saved session
./target/release/ben sessions clear <id>   # delete a saved session
```

## Controls

| Key | Action |
|---|---|
| `Esc` | Cancel current operation or return to conversation |
| `Enter` | Submit message or confirm focused approval |
| `↑` / `↓` | Scroll conversation history |
| `←` / `→` | Cycle approval focus (Approve once / Reject / Cancel) |
| `a` | Approve the pending action (when approval dialog is focused) |
| `r` | Reject the pending action |
| `c` | Cancel the pending action |
| `Ctrl+c` | Quit the application |

When the terminal is narrow (< 35 columns), controls simplify to `a/r/c` labels.

## Approval and safety model

The agent proposes side effects before executing them. Every proposed action is shown with its exact command, arguments, and target paths. You must approve, reject, or cancel each action individually.

- **Approve once** — executes this exact action one time. Approvals are bound to an immutable action fingerprint; a changed command or arguments invalidates the approval.
- **Reject** — denies the action; the agent may propose a revised action.
- **Cancel** — cancels the current turn.

Approved edits are shown as a unified diff before application. Commands are bounded by timeout and output limits. The workspace is confined to the selected root; path traversal and symlink escapes are rejected.

API keys are never persisted to session files or logs. Session data is stored locally with restrictive permissions and uses atomic writes to survive interrupted saves.

## Data locations

Sessions are stored in the platform application data directory under `ben/sessions/`:

- Linux: `$XDG_DATA_HOME/ben/sessions/` or `~/.local/share/ben/sessions/`
- macOS: `~/Library/Application Support/ben/sessions/`
- Windows: `%LOCALAPPDATA%\ben\sessions\`

Override with `BEN_DATA_DIR`. Session files are JSON with schema version 1; corrupted files are reported with a recovery path.

## Architecture

```
CLI args → Config → App state → TUI render
                 ↓
          Agent turn → Provider request → SSE stream
                 ↓              ↓
          Tool registry    Policy/approval
                 ↓              ↓
          Session store ←─── Summary
```

The application is organized into typed modules:

| Module | Responsibility |
|---|---|
| `src/cli.rs` | Clap argument parsing |
| `src/config.rs` | Validated configuration and secret resolution |
| `src/app/` | Application state, events, update reducer, terminal lifecycle |
| `src/agent/` | Turn state machine, message handling |
| `src/providers/` | Provider-neutral types, OpenAI adapter, fake provider |
| `src/tools/` | List, read, search, edit, run-command tools |
| `src/policy/` | Path confinement, approval decisions |
| `src/sessions/` | Versioned session model and atomic store |
| `src/ui/` | TUI widgets: conversation, approval, diff, status, layout |

## Learning path

See [`docs/learning-map.md`](docs/learning-map.md) for the 12 milestone learning structure.

## Documentation and design

- [Design spec](docs/superpowers/specs/2026-10-03-terminal-coding-agent-design.md)
- [Implementation plan](docs/superpowers/plans/2026-10-03-terminal-coding-agent.md)
- [Decision records](docs/decisions/)
- [Progress ledger](PROGRESS.md)

## Verification

This is a CLI/TUI project. Verification is CLI-only and does not need a browser or live model credentials.

- Check docs and ticket consistency: `scripts/check-docs.sh`
- Fast offline Rust loop: `scripts/verify-fast.sh [test filter]`
- Full offline local verification: `scripts/verify.sh`
- Record ticket PR state: `scripts/update_progress.py --task N --pr PR_NUMBER`
- Download locked Rust dependencies on a fresh checkout: `cargo fetch --locked`

## Stack direction

The plan recommends Rust, Ratatui/Crossterm, and Tokio for a native terminal app with explicit concurrency boundaries. It compares Go and TypeScript with reproducible workload measurements rather than assumed language-wide performance claims. See the [design spec](docs/superpowers/specs/2026-10-03-terminal-coding-agent-design.md).
