# Terminal Coding Agent Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a learning-oriented local coding agent with a responsive TUI, streamed model output, reviewed tool execution, persistent sessions, and reproducible performance comparisons.

**Architecture:** One Rust binary, divided into small modules for the TUI, agent state machine, provider adapter, workspace context, tools, policy, sessions, and diagnostics. UI and orchestration communicate through typed events; tool requests pass through workspace policy and approval before execution. The first provider is OpenAI's Responses API over HTTPS/SSE behind a provider-neutral interface.

**Tech Stack:** Stable Rust; Tokio; Ratatui + Crossterm; Reqwest with rustls; Serde/Serde JSON; Clap; tracing; tempfile for isolated filesystem fixtures. Exact compatible dependency versions are selected and locked during Task 2. Cross-stack benchmark reference programs use Go and TypeScript/Node.js only for the specified comparable workloads.

**Spec:** [2026-10-03-terminal-coding-agent-design.md](../specs/2026-10-03-terminal-coding-agent-design.md)

## Global Constraints

- Local-first single-process CLI; TUI is the main interface.
- Initial workspace access is confined to the selected root; reject traversal and symlink escapes.
- Model-generated edits and commands require approval before side effects.
- Never persist API keys in session files or logs; bound tool output and model tool-call cycles.
- Provider events, agent requests, tool requests, policy decisions, and UI state use explicit typed boundaries.
- Run slow provider and tool work outside the input/render loop; cancellation must restore terminal state.
- Benchmark Rust, Go, and TypeScript against identical fixtures and semantics; publish measured results with environment metadata, never assumed language numbers.
- Each ticket ends with a focused automated check and a commit; each milestone leaves a runnable increment.

## Review Focus

- A workspace symlink can escape the selected root: Task 24 asserts read rejection and Task 30 asserts write rejection.
- A truncated, malformed, or reordered SSE stream can create an incomplete tool call: Task 14 tests chunk boundaries, malformed frames, EOF, and incomplete calls.
- Approval could be replayed after a tool action changes: Task 27 asserts approval is tied to an immutable action fingerprint.
- Cancellation and panic paths can leave the terminal in raw/alternate-screen mode or a child process running: Tasks 4 and 26 exercise cleanup paths.
- A retry after a partial response could execute the same tool twice: Task 20 verifies retry policy does not re-dispatch completed side effects.

---

## File map

| Path | Responsibility |
|---|---|
| `Cargo.toml`, `Cargo.lock` | Binary package, feature selection, and pinned dependency graph |
| `src/main.rs` | Process entry point and top-level error reporting |
| `src/cli.rs` | Clap arguments and command modes |
| `src/config.rs` | Validated environment/config loading and secret resolution |
| `src/app/{mod.rs,terminal.rs}` | Application state, startup/shutdown, event routing, terminal lifecycle |
| `src/app/event.rs` | Typed user, provider, tool, and timer events |
| `src/app/update.rs` | State transition reducer |
| `src/ui/mod.rs`, `src/ui/layout.rs` | TUI rendering and responsive layout |
| `src/ui/conversation.rs`, `src/ui/approval.rs`, `src/ui/status.rs` | Focused widgets |
| `src/ui/diff.rs` | Reviewable unified diff presentation |
| `src/agent/mod.rs`, `src/agent/turn.rs` | Agent orchestration and turn lifecycle |
| `src/agent/message.rs`, `src/agent/limits.rs` | Provider-neutral messages and loop/output bounds |
| `src/providers/mod.rs`, `src/providers/types.rs` | Provider trait and shared typed event/request model |
| `src/providers/openai/{mod.rs,request.rs,http.rs,sse.rs}` | Responses request mapping, HTTP transport, and SSE parsing |
| `src/providers/{fake.rs,retry.rs}` | Deterministic agent testing and safe retry policy |
| `src/tools/{mod.rs,registry.rs,types.rs}` | Tool contract, lookup, typed input/output |
| `src/tools/{list_files,read_file,search,propose_edit,run_command}.rs` | Individual tools |
| `src/policy/{mod.rs,paths.rs,approval.rs,commands.rs}` | Workspace confinement, action approval, and command decisions |
| `src/context/{mod.rs,ignore.rs,budget.rs,instructions.rs}` | File selection and bounded prompt context |
| `src/sessions/{mod.rs,model.rs,store.rs}` | Versioned local session persistence |
| `src/telemetry.rs` | Secret-safe structured diagnostics and timing points |
| `tests/fixtures/workspace/` | Stable repository fixture for context/tool tests |
| `benches/` | Rust local workload benchmarks |
| `benchmarks/{go,typescript}/` | Minimal reference workloads, not alternate full products |
| `benchmarks/README.md`, `benchmarks/results/` | Run protocol, environment record, raw and summarized measurements |
| `README.md`, `docs/decisions/` | Setup guide, learning map, and short architecture decisions |

## Ticket conventions

Each ticket is intentionally narrow: one concept, one observable deliverable, and one focused test boundary. Ticket descriptions can be copied into an issue tracker. Run only the listed focused check while implementing a ticket, then run its milestone check before moving on. Keep commits small and use the suggested message. In ticket test steps, “test” means write or update the described automated check; do not use a live API key or network in the test suite.

## Milestone 0 — Rust project and terminal lifecycle

### Task 1: Scaffold the Rust executable

**Learning goal:** Learn Cargo package structure and a minimal executable.

**Files:** Create `Cargo.toml`, `src/main.rs`, `.gitignore`.

**Produces:** `cargo run -- --version` prints a stable name/version and exits successfully.

- [ ] Add a CLI smoke test that invokes the binary with `--version` and checks the output contains package name and version.
- [ ] Run `cargo test --test cli version_flag` and confirm the new assertion fails before implementation.
- [ ] Create the package and minimal entry point; add only the dependency needed for the CLI in a later ticket.
- [ ] Run `cargo test --test cli version_flag`; expect pass.
- [ ] Commit as `chore: scaffold terminal agent`.

### Task 2: Select and lock compatible dependencies

**Learning goal:** Understand direct dependencies, feature flags, and reproducible builds.

**Files:** Modify `Cargo.toml`, create/update `Cargo.lock`.

**Produces:** Stable-compatible dependency set for Tokio, Ratatui/Crossterm, Reqwest/rustls, Serde, Clap, tracing, and error types.

- [ ] Record the selected toolchain floor and dependency versions/features in `docs/decisions/0001-rust-dependencies.md`.
- [ ] Run `cargo check --locked`; expect failure until the lockfile is generated and committed.
- [ ] Add dependencies with default features disabled where practical; enable only required runtime, TLS, JSON, and terminal features.
- [ ] Run `cargo check --locked`; expect pass without duplicate incompatible Crossterm versions.
- [ ] Commit as `chore: pin initial Rust dependencies`.

### Task 3: Add structured top-level errors and logs

**Learning goal:** Separate user-facing errors from diagnostic context.

**Files:** Modify `src/main.rs`, create `src/telemetry.rs`.

**Produces:** Startup initializes stderr logging; fatal startup errors print a short action-oriented message and exit nonzero.

- [ ] Add tests for successful startup and a simulated startup error producing a nonzero exit without a panic dump.
- [ ] Run `cargo test --test startup_errors`; expect the new error-format assertion to fail.
- [ ] Initialize `tracing_subscriber` from `RUST_LOG`; use typed errors for internal context and avoid logging configuration secrets.
- [ ] Run `cargo test --test startup_errors`; expect pass.
- [ ] Commit as `feat: add structured diagnostics`.

### Task 4: Add terminal setup and guaranteed restoration

**Learning goal:** Manage terminal modes using a cleanup guard.

**Files:** Create `src/app/terminal.rs`, modify `src/main.rs`.

**Produces:** Enter raw/alternate-screen modes once; restore them on normal return, error, and unwind.

- [ ] Add a testable terminal lifecycle abstraction and tests asserting restore is called once on success and error.
- [ ] Run the lifecycle unit tests; expect failure before guard implementation.
- [ ] Implement a guard around Crossterm setup/restore; keep terminal I/O behind an injectable interface for tests.
- [ ] Run `cargo test terminal::`; expect pass.
- [ ] Commit as `feat: manage terminal lifecycle safely`.

## Milestone 1 — TUI shell and state

### Task 5: Define application state and event types

**Learning goal:** Model a UI as state transitions rather than logic inside drawing code.

**Files:** Create `src/app/mod.rs`, `src/app/event.rs`, `src/app/update.rs`.

**Produces:** `AppState` tracks screen, input, transcript, status, pending approval, and exit state; `AppEvent` covers key input and internal messages.

- [ ] Add reducer tests for submit, append output, tool status, and quit events.
- [ ] Run `cargo test app::update`; expect missing transition assertions to fail.
- [ ] Implement pure `update(state: AppState, event: AppEvent) -> AppState` transitions without terminal I/O.
- [ ] Run `cargo test app::update`; expect pass.
- [ ] Commit as `feat: model application state and events`.

### Task 6: Render a static responsive layout

**Learning goal:** Compose Ratatui layouts and small widgets.

**Files:** Create `src/ui/mod.rs`, `src/ui/layout.rs`, `src/ui/status.rs`.

**Produces:** Header/status, transcript, input, and help areas adapt to terminal dimensions.

- [ ] Add `TestBackend` rendering checks at 80x24, 40x12, and 160x50; assert no panic and required labels are visible where space permits.
- [ ] Run `cargo test ui::layout`; expect failures before layout exists.
- [ ] Implement rendering from immutable `&AppState`; show a compact mode at 40x12.
- [ ] Run `cargo test ui::layout`; expect pass.
- [ ] Commit as `feat: render responsive TUI shell`.

### Task 7: Implement keyboard input and event loop

**Learning goal:** Keep input polling, state updates, and rendering in a predictable loop.

**Files:** Modify `src/app/event.rs`, `src/app/mod.rs`, `src/ui/mod.rs`.

**Produces:** Keyboard-only input supports text entry, submit, scroll, and quit; UI remains responsive while background tasks are pending.

- [ ] Add tests mapping key events to application events, including Ctrl-C and Enter behavior.
- [ ] Run `cargo test app::event`; expect unmapped event tests to fail.
- [ ] Implement Crossterm input task plus Tokio event channel; render on state changes and a bounded redraw tick.
- [ ] Run `cargo test app::event` and manually launch/exit the TUI; expect tests pass and shell terminal state restored.
- [ ] Commit as `feat: handle terminal input events`.

### Task 8: Build the conversation and input widgets

**Learning goal:** Render scrollable content and an editable prompt.

**Files:** Create `src/ui/conversation.rs`, modify `src/ui/mod.rs`, `src/app/update.rs`.

**Produces:** User messages and assistant messages display distinctly; input supports insertion, backspace, cursor movement, submit, and clear-on-submit.

- [ ] Add reducer tests for editing and submitting input; add `TestBackend` rendering coverage for multiline messages.
- [ ] Run the focused app and UI tests; expect new behavior assertions to fail.
- [ ] Implement input editing in state and transcript rendering in the widget.
- [ ] Run `cargo test app:: ui::`; expect pass.
- [ ] Commit as `feat: add conversation and prompt input`.

## Milestone 2 — Configuration and provider boundary

### Task 9: Parse CLI arguments and workspace selection

**Learning goal:** Keep startup options distinct from app state.

**Files:** Create `src/cli.rs`, modify `src/main.rs`.

**Produces:** `agent [workspace]`, `--model`, `--version`, and `--help`; current directory is the default workspace.

- [ ] Add CLI integration cases for defaults, explicit workspace, help, and invalid model argument.
- [ ] Run `cargo test --test cli`; verify cases fail where parsing is absent.
- [ ] Define `Cli` and validated `StartupOptions`; resolve workspace to an absolute path.
- [ ] Run `cargo test --test cli`; expect pass.
- [ ] Commit as `feat: parse CLI startup options`.

### Task 10: Load configuration and protect credentials

**Learning goal:** Validate configuration at the edge and keep secrets out of serializable state.

**Files:** Create `src/config.rs`; modify `src/main.rs`.

**Produces:** Read `OPENAI_API_KEY`, model, and configurable context/tool limits; missing key gives setup instructions before a network request.

- [ ] Add tests for missing key, empty key, valid configuration, and ensuring `Debug` output omits secret bytes.
- [ ] Run `cargo test config::`; expect missing validation tests to fail.
- [ ] Implement `Config::load` returning validated non-serializable secret wrapper and public non-secret settings.
- [ ] Run `cargo test config::`; expect pass.
- [ ] Commit as `feat: load validated configuration securely`.

### Task 11: Define provider-neutral request and event types

**Learning goal:** Hide API-specific wire details behind an application interface.

**Files:** Create `src/providers/mod.rs`, `src/providers/types.rs`, `src/agent/message.rs`.

**Produces:** Typed conversation messages, tool specifications, streamed text/tool events, usage, completion, error, and cancellation types.

- [ ] Add serialization-independent unit tests for message roles, tool-call identity, and event ordering.
- [ ] Run `cargo test providers::types`; expect type/behavior tests to be incomplete.
- [ ] Define `Provider::stream(request, cancellation) -> Stream<Item = Result<ProviderEvent, ProviderError>>` and the shared types.
- [ ] Run `cargo test providers::types`; expect pass.
- [ ] Commit as `feat: define provider-neutral model interface`.

### Task 12: Implement OpenAI Responses request mapping

**Learning goal:** Translate internal messages to an external API schema.

**Files:** Create `src/providers/openai/mod.rs`, `src/providers/openai/request.rs`.

**Produces:** JSON request to `POST /v1/responses`, stream enabled, configured model, messages/instructions, and function tools mapped without leaking OpenAI types into `agent`.

- [ ] Add golden JSON tests for a text-only request and a request containing two function tools.
- [ ] Run `cargo test providers::openai::request`; expect golden comparisons to fail before mapping exists.
- [ ] Implement request structs with Serde; keep model ID configuration-driven and use a configurable API base only for tests.
- [ ] Run `cargo test providers::openai::request`; expect pass.
- [ ] Commit as `feat: map agent requests to Responses API`.

### Task 13: Add HTTP transport and typed provider errors

**Learning goal:** Make network code cancellable and testable without a live account.

**Files:** Modify `src/providers/openai/mod.rs`, create `src/providers/openai/http.rs`.

**Produces:** Reqwest client sends auth and JSON; transport, HTTP status, API error body, and timeout map to distinct `ProviderError` variants.

- [ ] Use a local mock HTTP server in tests to assert method/path/headers/body and simulate 401, 429, 500, and timeout.
- [ ] Run `cargo test providers::openai::http`; expect request/error mapping assertions to fail.
- [ ] Implement a bounded-timeout client; redact authorization headers and response bodies from ordinary logs.
- [ ] Run `cargo test providers::openai::http`; expect pass without external network.
- [ ] Commit as `feat: send provider requests over HTTPS`.

## Milestone 3 — Streaming and agent loop

### Task 14: Parse SSE frames incrementally

**Learning goal:** Handle network streams whose chunks do not align to message boundaries.

**Files:** Create `src/providers/openai/sse.rs`.

**Produces:** Decoder handles partial UTF-8/network chunks, comments, event/data fields, CRLF, multiple frames per chunk, malformed JSON, and EOF with incomplete frame.

- [ ] Add table-driven parser tests for split boundaries, blank data, unknown event, malformed JSON, and truncated frame.
- [ ] Run `cargo test providers::openai::sse`; expect parser assertions to fail.
- [ ] Implement incremental SSE decoder with bounded frame size and explicit malformed/incomplete errors.
- [ ] Run `cargo test providers::openai::sse`; expect pass.
- [ ] Commit as `feat: decode streamed server events`.

### Task 15: Map Responses SSE events into shared provider events

**Learning goal:** Convert a vendor event protocol into stable internal events.

**Files:** Modify `src/providers/openai/sse.rs`, `src/providers/openai/mod.rs`.

**Produces:** Handles output text deltas, function-call argument deltas and completion, response completion/failure, and API error events; ignores documented irrelevant event variants safely.

- [ ] Add fixture tests using captured-shaped event JSON for each supported event and unknown event types.
- [ ] Verify incomplete function arguments at EOF never produce a completed tool call.
- [ ] Implement event conversion and per-call argument accumulation keyed by call/item identity.
- [ ] Run `cargo test providers::openai`; expect pass.
- [ ] Commit as `feat: map Responses stream events`.

### Task 16: Stream provider events into app state

**Learning goal:** Bridge async work into a responsive UI without rendering in the network task.

**Files:** Modify `src/app/event.rs`, `src/app/update.rs`, `src/app/mod.rs`.

**Produces:** Provider text deltas append incrementally; status reflects connecting, streaming, completed, failed, and cancelled states.

- [ ] Add reducer tests for ordered deltas and terminal provider error while retaining prior output.
- [ ] Run `cargo test app::update`; verify stream-state assertions fail before wiring.
- [ ] Spawn provider work under Tokio and forward typed events through the app channel.
- [ ] Run `cargo test app::`; expect pass; manually verify input remains responsive during delayed mock streaming.
- [ ] Commit as `feat: stream assistant output into the TUI`.

### Task 17: Add a deterministic fake provider

**Learning goal:** Develop agent behavior independently of network services.

**Files:** Create `src/providers/fake.rs`, modify `src/providers/mod.rs`.

**Produces:** Scripted provider emits configured event sequences and records requests for assertions.

- [ ] Add tests for text response, tool call response, error, and cancellation sequences.
- [ ] Run `cargo test providers::fake`; confirm cancellation expectation initially fails.
- [ ] Implement fake provider with deterministic delays controlled by test configuration.
- [ ] Run `cargo test providers::fake`; expect pass.
- [ ] Commit as `test: add deterministic fake provider`.

### Task 18: Implement the agent turn state machine

**Learning goal:** Orchestrate model turns explicitly and make stop conditions visible.

**Files:** Create `src/agent/mod.rs`, `src/agent/turn.rs`, `src/agent/limits.rs`.

**Produces:** Accept user message, request provider stream, emit app events, finish on final response, and stop at configured turn/tool-call bounds.

- [ ] Test with fake provider for final answer, provider error, empty completion, and max-turn termination.
- [ ] Run `cargo test agent::`; verify state-machine behavior tests fail before implementation.
- [ ] Implement `Agent::run_turn` over provider trait, cancellation token, bounded history, and app event sender.
- [ ] Run `cargo test agent::`; expect pass.
- [ ] Commit as `feat: orchestrate agent turns`.

### Task 19: Add cancellation and busy-state behavior

**Learning goal:** Propagate cancellation across async task boundaries.

**Files:** Modify `src/app/event.rs`, `src/app/update.rs`, `src/app/mod.rs`, `src/agent/turn.rs`.

**Produces:** Ctrl-C/escape cancellation stops an active provider stream; second submit while busy is handled visibly; the TUI remains interactive.

- [ ] Add fake-provider tests that cancellation stops polling and returns a cancelled stop reason.
- [ ] Add app reducer tests for submit-while-busy and cancellation state.
- [ ] Implement one cancellation token per active turn and ensure terminal guard drops after cancellation.
- [ ] Run `cargo test agent:: app::`; expect pass.
- [ ] Commit as `feat: cancel active agent turns`.

### Task 20: Define retry and duplicate-action rules

**Learning goal:** Understand why retrying a request is different from retrying a side effect.

**Files:** Create `src/providers/retry.rs`, modify `src/agent/turn.rs`.

**Produces:** Retry only classified transient provider connection failures before any tool execution; never blindly replay after an action result exists.

- [ ] Add deterministic fake transport tests for transient-then-success, permanent error, and failure after a completed tool result.
- [ ] Run `cargo test providers::retry`; expect replay prevention assertion to fail.
- [ ] Implement bounded retry count with backoff and an explicit side-effect checkpoint.
- [ ] Run `cargo test providers::retry agent::`; expect pass.
- [ ] Commit as `feat: retry safe provider failures`.

## Milestone 4 — Workspace context and read-only tools

### Task 21: Establish workspace root and path normalization

**Learning goal:** Treat model-supplied paths as untrusted input.

**Files:** Create `src/policy/mod.rs`, `src/policy/paths.rs`.

**Produces:** Normalize relative paths under a canonical workspace root and reject absolute paths, traversal, and symlink escape.

- [ ] Use temporary directory tests for normal child paths, `..`, absolute paths, symlinks inside, and symlinks outside.
- [ ] Run `cargo test policy::paths`; expect escape cases to fail before implementation.
- [ ] Implement `WorkspacePath::resolve(root, input, intent)` with canonicalization and explicit errors.
- [ ] Run `cargo test policy::paths`; expect all pass on supported OS.
- [ ] Commit as `feat: confine paths to workspace root`.

### Task 22: Define tool contract and registry

**Learning goal:** Keep tools independently describable and invokable.

**Files:** Create `src/tools/mod.rs`, `src/tools/types.rs`, `src/tools/registry.rs`.

**Produces:** Typed `ToolRequest`, `ToolResult`, metadata/schema, and registry lookup; unknown tool names return structured errors.

- [ ] Add tests for registration, duplicate-name rejection, schema listing, and unknown tool handling.
- [ ] Run `cargo test tools::registry`; expect missing registry behavior to fail.
- [ ] Implement registry independent of provider-specific wire structs.
- [ ] Run `cargo test tools::registry`; expect pass.
- [ ] Commit as `feat: define typed tool registry`.

### Task 23: Add ignore-aware file listing and search

**Learning goal:** Traverse a repository while honoring exclusions and output bounds.

**Files:** Create `src/context/mod.rs`, `src/context/ignore.rs`, `src/tools/list_files.rs`, `src/tools/search.rs`, create `tests/fixtures/workspace/`.

**Produces:** List/search files under root, skip common VCS/build/secret directories and configured ignore rules, cap result count/bytes.

- [ ] Build fixture cases for ignored files, binary content, hidden files, and result-limit truncation.
- [ ] Add focused tests for deterministic sorted output and no traversal outside the workspace.
- [ ] Implement ignore matching and bounded listing/search.
- [ ] Run `cargo test context:: tools::list_files tools::search`; expect pass.
- [ ] Commit as `feat: add bounded workspace listing and search`.

### Task 24: Add safe bounded file reading

**Learning goal:** Read text files with explicit size and encoding behavior.

**Files:** Create `src/tools/read_file.rs`, modify `src/context/mod.rs`.

**Produces:** Read a normalized workspace path, reject binary/oversized files, support requested line ranges, truncate at configured byte limit.

- [ ] Test UTF-8 text, invalid UTF-8, oversized file, missing file, line range, traversal, and symlink escape.
- [ ] Run `cargo test tools::read_file`; verify failures for limits and path escape before code.
- [ ] Implement `ReadFileInput` validation and structured output with truncation metadata.
- [ ] Run `cargo test tools::read_file`; expect pass.
- [ ] Commit as `feat: read bounded workspace files`.

### Task 25: Assemble repository instructions and bounded context

**Learning goal:** Select useful prompt context within an explicit budget.

**Files:** Create `src/context/instructions.rs`, `src/context/budget.rs`, modify `src/agent/turn.rs`.

**Produces:** Load applicable `AGENTS.md` instructions from root toward current path, include task-relevant context within configured byte/token estimate budget, disclose omitted/truncated content.

- [ ] Test precedence of nested instruction files, ignored instruction files, missing files, and deterministic budget truncation.
- [ ] Run `cargo test context::`; verify budget and precedence tests fail before implementation.
- [ ] Implement instruction collection and an injectable tokenizer-estimator interface with a documented conservative byte fallback.
- [ ] Run `cargo test context:: agent::`; expect pass.
- [ ] Commit as `feat: assemble bounded repository context`.

### Task 26: Expose read-only tools to the agent

**Learning goal:** Complete a safe first model-to-tool cycle.

**Files:** Modify `src/tools/registry.rs`, `src/agent/turn.rs`, `src/providers/openai/request.rs`.

**Produces:** Agent receives registered list/read/search tools, validates complete arguments, executes read-only tool, and sends bounded result back to provider with matching call ID.

- [ ] Script fake-provider tool request and assert registry input, returned result, and follow-up provider request history.
- [ ] Test malformed JSON, unknown tool, oversized result, and mismatched call ID handling.
- [ ] Implement loop for multiple read-only calls up to configured limit; keep incomplete/invalid requests unexecuted.
- [ ] Run `cargo test agent:: tools::`; expect pass.
- [ ] Commit as `feat: execute read-only model tools`.

## Milestone 5 — Approval and side-effecting tools

### Task 27: Model approval decisions and action fingerprints

**Learning goal:** Bind user intent to an exact pending operation.

**Files:** Create `src/policy/approval.rs`, modify `src/app/event.rs`, `src/app/update.rs`.

**Produces:** Pending approval stores immutable normalized action plus fingerprint; decisions are approve-once, reject, or cancel.

- [ ] Test that changing command/arguments/path invalidates an earlier approval and that a decision applies once only.
- [ ] Run `cargo test policy::approval`; expect replay test to fail.
- [ ] Implement fingerprinted decision state with no broad “always allow” mode in the initial release.
- [ ] Run `cargo test policy::approval app::update`; expect pass.
- [ ] Commit as `feat: bind approvals to exact actions`.

### Task 28: Build the approval prompt UI

**Learning goal:** Make risky actions understandable at the moment of decision.

**Files:** Create `src/ui/approval.rs`, modify `src/ui/mod.rs`, `src/app/update.rs`.

**Produces:** Show tool name, normalized target/command, relevant arguments, and approve/reject/cancel controls; no action runs before approval.

- [ ] Add reducer tests for each key and `TestBackend` checks that displayed action details are not clipped at standard terminal size.
- [ ] Run focused UI tests; expect approval interaction assertions to fail.
- [ ] Implement pending approval view with narrow-terminal fallback and explicit default focus on reject/cancel.
- [ ] Run `cargo test ui::approval app::update`; expect pass.
- [ ] Commit as `feat: review tool actions in the TUI`.

### Task 29: Implement proposed edit and diff rendering

**Learning goal:** Separate a model's proposed change from the act of writing it.

**Files:** Create `src/tools/propose_edit.rs`, `src/ui/diff.rs`, modify `src/app/event.rs`.

**Produces:** Validate a structured file replacement request, generate a unified diff, and display old/new context without touching the file.

- [ ] Test diff output for add, delete, replace, multiple hunks, missing target, and stale expected content.
- [ ] Run `cargo test tools::propose_edit ui::diff`; verify stale-content check fails before implementation.
- [ ] Implement proposal as immutable pending action; reject binary files and oversized diffs.
- [ ] Run `cargo test tools::propose_edit ui::diff`; expect pass and assert no fixture file changed.
- [ ] Commit as `feat: preview proposed file edits`.

### Task 30: Apply approved edit atomically

**Learning goal:** Reduce partial writes and detect concurrent modifications.

**Files:** Modify `src/tools/propose_edit.rs`, `src/policy/approval.rs`.

**Produces:** After approval revalidate path and expected original content, write a sibling temporary file, sync/rename where supported, and report changed path.

- [ ] Test successful apply, rejected proposal, changed-file conflict, symlink swap, and cleanup after simulated write error.
- [ ] Run `cargo test tools::propose_edit`; expect conflict/symlink cases to fail before implementation.
- [ ] Implement revalidation and atomic replacement; approval fingerprint includes target and content digest.
- [ ] Run `cargo test tools::propose_edit policy::`; expect pass.
- [ ] Commit as `feat: apply approved edits atomically`.

### Task 31: Implement bounded command execution

**Learning goal:** Manage child processes with explicit environment, timeout, output, and cancellation.

**Files:** Create `src/policy/commands.rs`, `src/tools/run_command.rs`.

**Produces:** Command request has argv, working directory, timeout, and output limits; no shell string interpolation by default; policy requires per-action approval.

- [ ] Use a fake executable in temp fixtures to test stdout, stderr, nonzero exit, timeout, cancellation, output overflow, and cwd.
- [ ] Run `cargo test tools::run_command policy::commands`; expect lifecycle tests to fail.
- [ ] Implement `tokio::process::Command` with kill-on-drop/explicit terminate, bounded capture, and normalized approval display.
- [ ] Run `cargo test tools::run_command policy::commands`; expect all pass and no child survives cancellation.
- [ ] Commit as `feat: run approved workspace commands`.

### Task 32: Connect approvals to the tool execution path

**Learning goal:** Enforce policy at the last responsible moment.

**Files:** Modify `src/agent/turn.rs`, `src/tools/registry.rs`, `src/app/mod.rs`.

**Produces:** Side-effecting tool call pauses agent for approval; reject/cancel returns a structured result to the model; approve revalidates and executes once.

- [ ] Script fake-provider scenarios for approve, reject, cancel, modified action, and duplicate event delivery.
- [ ] Assert rejected/cancelled actions cause no filesystem/process side effect.
- [ ] Implement pending-action channel and resume agent after decision; send outputs using original tool call ID.
- [ ] Run `cargo test agent:: tools:: policy::`; expect pass.
- [ ] Commit as `feat: gate side effects on user approval`.

## Milestone 6 — Sessions and robust recovery

### Task 33: Define versioned session data

**Learning goal:** Separate durable conversation data from runtime-only secrets and handles.

**Files:** Create `src/sessions/mod.rs`, `src/sessions/model.rs`.

**Produces:** Serializable session schema stores messages, tool summaries, timestamps, and schema version; excludes API keys, live streams, and raw terminal state.

- [ ] Add JSON round-trip tests and a test that serialized session text contains no configured secret.
- [ ] Run `cargo test sessions::model`; expect serialization/secret assertions to fail.
- [ ] Implement explicit serializable types and version field.
- [ ] Run `cargo test sessions::model`; expect pass.
- [ ] Commit as `feat: define versioned session format`.

### Task 34: Save and load sessions safely

**Learning goal:** Use atomic persistence and recover from damaged local data.

**Files:** Create `src/sessions/store.rs`, modify `src/config.rs`.

**Produces:** Store sessions in app data directory using restrictive permissions where supported, atomic replace, load validation, and clear corruption errors/recovery path.

- [ ] Test round-trip, truncated JSON, unknown schema version, permission behavior, and interrupted write cleanup in temp directories.
- [ ] Run `cargo test sessions::store`; expect recovery behavior to fail.
- [ ] Implement `SessionStore` with injectable root path and version migrations limited to known versions.
- [ ] Run `cargo test sessions::store`; expect pass.
- [ ] Commit as `feat: persist and recover sessions`.

### Task 35: Add resume/list/clear CLI flows

**Learning goal:** Connect persistent data to a usable command interface.

**Files:** Modify `src/cli.rs`, `src/main.rs`, `src/app/mod.rs`.

**Produces:** List local session IDs, resume a selected session, and clear one session with a confirmation prompt in TUI.

- [ ] Add CLI tests for list, missing session, resume, and clear confirmation/rejection.
- [ ] Run `cargo test --test cli`; expect session commands to fail before wiring.
- [ ] Implement commands and load saved conversation before rendering.
- [ ] Run `cargo test --test cli sessions::`; expect pass.
- [ ] Commit as `feat: resume and manage local sessions`.

### Task 36: Add shutdown consistency and terminal restoration coverage

**Learning goal:** Coordinate cancellation, child cleanup, session save, and terminal restoration.

**Files:** Modify `src/app/mod.rs`, `src/app/terminal.rs`, `src/agent/turn.rs`, `src/sessions/store.rs`.

**Produces:** Shutdown sequence cancels work, awaits child termination, saves consistent session state, then restores terminal; repeated shutdown is harmless.

- [ ] Add integration tests with fake provider, fake child process, and in-memory terminal guard that assert shutdown ordering.
- [ ] Simulate errors at each shutdown stage and verify remaining cleanup stages still run.
- [ ] Implement idempotent shutdown coordinator and error aggregation.
- [ ] Run `cargo test app::shutdown`; expect pass.
- [ ] Commit as `fix: make shutdown cleanup reliable`.

## Milestone 7 — Product usability and release

### Task 37: Add usage, error, and changed-file summaries

**Learning goal:** Surface enough outcome information for users to verify work.

**Files:** Modify `src/app/event.rs`, `src/app/update.rs`, `src/ui/status.rs`, `src/ui/conversation.rs`.

**Produces:** Show provider/model, token usage when reported, elapsed turn time, tool outcomes, and changed-file list; errors suggest a next step.

- [ ] Add reducer/render tests for absent usage metadata, provider errors, tool failures, and successful file changes.
- [ ] Run `cargo test ui:: app::`; expect missing summary states to fail.
- [ ] Implement summaries using provider-neutral optional usage values and categorized errors.
- [ ] Run `cargo test ui:: app::`; expect pass.
- [ ] Commit as `feat: summarize agent work and errors`.

### Task 38: Handle terminal capability and accessibility cases

**Learning goal:** Test assumptions around color, width, and keyboard-only workflows.

**Files:** Modify `src/ui/layout.rs`, `src/ui/approval.rs`, `src/ui/conversation.rs`.

**Produces:** Respect no-color terminal, compact mode at narrow width, visible focus, and keyboard-only access to every action.

- [ ] Add TestBackend snapshots/assertions for 40x12, 80x24, and no-color styles; test every approval choice through keys.
- [ ] Run `cargo test ui::`; verify small-terminal and focus assertions fail where missing.
- [ ] Implement capability-aware styles and minimum-width fallback/help text.
- [ ] Run `cargo test ui::`; expect pass.
- [ ] Commit as `feat: improve terminal accessibility`.

### Task 39: Document install, first run, controls, and safety model

**Learning goal:** Explain the system in the order a new builder/user encounters it.

**Files:** Create/update `README.md`, `docs/learning-map.md`, `docs/decisions/`.

**Produces:** Prerequisites, install/run, API key setup, keyboard map, approval semantics, data locations, architecture, and milestone learning path.

- [ ] Check documentation commands against CLI help in a lightweight docs consistency script or manual checklist.
- [ ] Add examples with placeholder credentials only; scan docs and fixtures for key-like strings.
- [ ] Write the guide and link design/plan and architecture diagrams.
- [ ] Run the docs consistency check and `git diff --check`; expect clean output.
- [ ] Commit as `docs: explain setup and learning path`.

### Task 40: Package release artifacts for initial platforms

**Learning goal:** Make a native CLI reproducible to build and install.

**Files:** Create `.github/workflows/ci.yml`, `release.toml` or equivalent packaging config, modify `README.md`.

**Produces:** CI checks formatting, lint, and tests on Linux/macOS; release build produces archives and checksums for supported targets.

- [ ] Add a CI workflow validation check and document target matrix.
- [ ] Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --locked` locally.
- [ ] Configure workflows with pinned actions and locked Rust dependencies; upload checksums alongside archives.
- [ ] Run workflow lint/check and reproduce one release build locally where target is available.
- [ ] Commit as `build: package initial CLI releases`.

## Milestone 8 — Performance evidence and stack comparison

### Task 41: Freeze benchmark fixtures and command protocol

**Learning goal:** Make measurements repeatable before comparing languages.

**Files:** Create `benchmarks/README.md`, `benchmarks/fixtures/`, `benchmarks/results/.gitkeep`.

**Produces:** Deterministic transcript fixture (500 messages/100 tool rows) and repository fixture (1,000 files/100 MiB), fixed terminal dimensions, warmup/sample counts, environment metadata schema.

- [ ] Add a fixture integrity check for file count, byte count, and stable hashes.
- [ ] Run benchmark fixture integrity command; expect the declared sizes/hashes to match.
- [ ] Document cold-start (20 runs), idle RSS (10s warmup/30s sampling), input-to-frame (1,000 events), render (10,000 frames), context assembly, process launch (100 runs), and end-to-end (30 runs) protocols.
- [ ] Re-run integrity command from a clean checkout; expect identical hashes.
- [ ] Commit as `bench: define repeatable workload fixtures`.

### Task 42: Add Rust benchmark instrumentation

**Learning goal:** Distinguish UI render cost, context cost, and external latency.

**Files:** Create `benches/ui_render.rs`, `benches/context_assembly.rs`, `src/telemetry.rs`.

**Produces:** Rust measurements use same fixture and report p50/p95/p99 where meaningful, with no provider network calls in local benchmarks.

- [ ] Add a smoke check that benchmark binary enumerates each required workload and outputs machine-readable JSON.
- [ ] Run the benchmark smoke check; expect missing workload names to fail.
- [ ] Instrument time boundaries with monotonic clock and emit workload/environment metadata.
- [ ] Run one short local benchmark; confirm JSON includes samples and quantiles, without making performance claims.
- [ ] Commit as `bench: measure Rust local workloads`.

### Task 43: Build comparable Go reference workloads

**Learning goal:** Compare equivalent algorithms and know where runtime setup differs.

**Files:** Create `benchmarks/go/go.mod`, `benchmarks/go/main.go`.

**Produces:** Go executable uses Bubble Tea for the fixed transcript view and implements fixture context assembly, child launch, RSS sampling interface, and the shared output schema.

- [ ] Add fixture-driven tests for deterministic context output and transcript operation counts.
- [ ] Run `go test ./...` in `benchmarks/go`; confirm tests fail before workload implementation.
- [ ] Implement only shared workload semantics; record Go version and build flags; exclude a full TUI/provider implementation.
- [ ] Run `go test ./...`; expect pass and verify output schema matches Rust.
- [ ] Commit as `bench: add Go reference workloads`.

### Task 44: Build comparable TypeScript reference workloads

**Learning goal:** Compare Node runtime and distribution costs without benchmarking unrelated framework behavior.

**Files:** Create `benchmarks/typescript/package.json`, `benchmarks/typescript/tsconfig.json`, `benchmarks/typescript/src/main.ts`.

**Produces:** Node program uses Ink for the fixed transcript view and implements fixture context assembly, child launch, startup/RSS measurement hooks, and shared output schema.

- [ ] Add fixture-driven tests for deterministic context output and transcript operation counts.
- [ ] Run `npm test`; confirm workload assertions fail before implementation.
- [ ] Implement shared semantics and record Node/TypeScript versions and build/runtime flags.
- [ ] Run `npm test`; expect pass and verify schema matches Rust/Go.
- [ ] Commit as `bench: add TypeScript reference workloads`.

### Task 45: Run the three-stack benchmark comparison

**Learning goal:** Collect results with enough context to make honest conclusions.

**Files:** Create `benchmarks/results/<machine-id>-<date>.json`, `benchmarks/results/README.md`.

**Produces:** One complete run of Rust, Go, and TypeScript workload suite on the same machine, with raw samples, medians, tail latencies, spread, artifact size, and environment metadata.

- [ ] Validate all three outputs against a shared JSON schema and fixture hashes.
- [ ] Run each workload in randomized stack order with stated warmups and repetitions; retain raw samples.
- [ ] Generate a comparison table and simple static plot for startup, idle RSS, render, context, child startup, and artifact size; mark unsupported metrics explicitly.
- [ ] Review conclusions for workload-specific wording; do not claim general language superiority from this one app.
- [ ] Commit as `bench: record local stack comparison`.

### Task 46: Measure end-to-end provider latency separately

**Learning goal:** Separate runtime responsiveness from remote model/network time.

**Files:** Create `benchmarks/provider-loop/`, modify `benchmarks/README.md`.

**Produces:** Optional opt-in run records model identifier, region/endpoint, time-to-first-token, total wall time, tokens, tool calls, and success rate; API keys are read from environment and never stored.

- [ ] Add a dry-run test proving no credential or network is needed to validate the report schema.
- [ ] Implement opt-in command requiring explicit `--live-provider-benchmark`; store only non-secret metadata and aggregate output.
- [ ] Run schema dry run without credentials; expect pass and no HTTP attempt.
- [ ] Perform 30-run measurement only when a user supplies credentials during actual execution; record failed/aborted runs as well as successes.
- [ ] Commit as `bench: separate provider latency report`.

## Ticket dependency map

Tasks are sequential within each milestone. Cross-milestone dependencies: Tasks 11–16 build on Tasks 5–8; Task 18 requires Tasks 11, 16, and 17; Tasks 21–26 require Task 18; Tasks 27–32 require Tasks 22 and 26; Tasks 33–36 require Tasks 18 and 32; Tasks 37–40 require the integrated app; Tasks 41–45 can start once fixture semantics are agreed and can proceed alongside UI work, while Task 46 requires a working provider loop and secure configuration.

## Stack comparison interpretation

The plan does not assert numeric performance wins in advance. Rust is recommended for native deployment, explicit concurrency boundaries, and low-overhead potential, while Go offers the gentlest systems-language learning curve and TypeScript the quickest path for API-oriented prototyping. Actual comparisons come from Tasks 41–45 using the same machine and workload fixtures. Provider latency is isolated in Task 46. Ratatui's documented buffer diff is relevant to render design, and official Go performance guidance recommends comparison against a baseline; neither supplies this project's Rust/Go/TypeScript numbers. [Ratatui rendering internals](https://ratatui.rs/concepts/rendering/under-the-hood/), [Go performance monitoring](https://go.dev/wiki/PerformanceMonitoring)

The first provider adapter uses OpenAI Responses streaming over SSE and function calls, mapped to internal provider-neutral types. The official OpenAI documentation describes typed semantic streaming events and a multi-step function-call/tool-output loop. Keep API model IDs configured rather than hard-coded in code so model availability changes do not require an architecture change. [OpenAI streaming guide](https://developers.openai.com/api/docs/guides/streaming-responses), [OpenAI function-calling guide](https://developers.openai.com/api/docs/guides/function-calling)
