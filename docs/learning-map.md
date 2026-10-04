# Learning map

`ben` is structured as 12 milestones that build on each other. Each milestone teaches one focused concept and produces a runnable increment.

## Milestones

### 1. Rust and TUI shell
Scaffold the binary, lock dependencies, and render a responsive terminal shell with guaranteed restoration on exit. Learn: Rust toolchain, Ratatui/Crossterm basics, terminal lifecycle.

### 2. App state and events
Define application state, typed events, and the input/event loop. Learn: Immutable state transitions, event routing, separation of input from rendering.

### 3. Configuration and provider boundary
Parse CLI arguments, load validated configuration, and isolate the provider behind a neutral interface. Learn: Secure secret handling, provider abstraction, typed boundaries.

### 4. Streaming and cancellation
Stream provider output into the TUI and cancel in-flight work without losing terminal state. Learn: Async SSE consumption, cancellation tokens, terminal restoration.

### 5. Agent turn state machine
Implement the agent loop: provider request, tool call extraction, and bounded iteration. Learn: State machines, deterministic fake providers, scripted test fixtures.

### 6. Read-only workspace tools
Add list, read, and search tools that are confined to the workspace root. Learn: Path normalization, symlink escape rejection, bounded output.

### 7. Side-effecting tools with approval
Add edit and command tools that require explicit user approval before execution. Learn: Approval UX, action fingerprints, diff review, atomic application.

### 8. Session persistence
Save and load versioned session records with atomic writes and corruption recovery. Learn: Serializable data models, atomic file operations, version migrations.

### 9. Robustness and accessibility
Handle shutdown, terminal capability cases, narrow terminals, and keyboard-only workflows. Learn: Cleanup ordering, capability-aware rendering, accessible controls.

### 10. Documentation and release
Document install, first run, controls, and safety model. Package release artifacts with CI. Learn: User-facing docs, release automation, platform matrices.

### 11. Cross-stack benchmarks
Build comparable Rust, Go, and TypeScript reference workloads with identical fixtures. Learn: Reproducible measurement, fair comparison, environment metadata.

### 12. Provider latency isolation
Measure end-to-end provider latency separately from local runtime costs. Learn: Latency decomposition, opt-in measurement, credential safety.

## How to use this map

- **New builders**: Start at milestone 1 and follow the tickets in order.
- **Contributors**: Pick the milestone that matches your contribution area.
- **Reviewers**: Check that each ticket's learning goal matches its milestone.

## See also

- [Design spec](superpowers/specs/2026-10-03-terminal-coding-agent-design.md)
- [Implementation plan](superpowers/plans/2026-10-03-terminal-coding-agent.md)
- [Progress ledger](../PROGRESS.md)