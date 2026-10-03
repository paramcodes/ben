# Rust dependency baseline

## Toolchain floor

The package requires Rust 1.88.0 or newer. Ratatui 0.30.2 declares that minimum, and the project design already calls for the Ratatui 0.30 / Crossterm 0.29 generation. The package uses the 2024 edition.

## Direct dependencies

Cargo.toml records the deliberate feature selections below. Cargo.lock pins the complete resolved graph used by builds and offline verification.

| Crate | Selected version | Enabled features | Purpose |
|---|---:|---|---|
| `tokio` | 1.53.2 | `io-util`, `macros`, `process`, `rt-multi-thread`, `sync`, `time` | Async runtime, channels, timers, and bounded child-process I/O |
| `tokio-util` | 0.7.19 | `rt` | Cancellation tokens shared by provider and tool work |
| `ratatui` | 0.30.2 | `crossterm` | Terminal widgets and Crossterm backend |
| `crossterm` | 0.29.0 | `events` | Terminal input and mode control; matches Ratatui's backend version |
| `reqwest` | 0.13.5 | `json`, `rustls`, `stream` | HTTPS JSON requests and incremental response bodies without native TLS |
| `futures-util` | 0.3.34 | `std` | Consume Reqwest's streamed response body through `StreamExt` |
| `serde` | 1.0.229 | `derive`, `std` | Typed serialization and deserialization |
| `serde_json` | 1.0.151 | `std` | JSON values and provider wire format |
| `clap` | 4.6.7 | `derive`, `error-context`, `help`, `std`, `usage` | CLI parsing, help, and version output |
| `tracing` | 0.1.44 | `attributes`, `std` | Structured application diagnostics and spans |
| `tracing-subscriber` | 0.3.23 | `env-filter`, `fmt`, `std` | `RUST_LOG` filtering and formatted stderr output |
| `thiserror` | 2.0.21 | defaults | Categorized internal errors with standard error sources |
| `tempfile` (dev) | 3.27.0 | `getrandom` | Isolated filesystem fixtures in deterministic tests |

Default features are disabled where practical so TLS, runtime, and terminal choices remain explicit. No `anyhow` dependency is included: the application uses its planned typed, categorized errors instead of collapsing them into one erased error type. Tokio's broad `full` feature is not enabled; filesystem operations can use the standard library outside the UI loop, and Crossterm input is polled by the planned input task.

## Compatibility notes

- Ratatui 0.30.2's `crossterm` backend uses Crossterm 0.29. The direct Crossterm dependency stays on 0.29 so input event types unify instead of introducing another version.
- Reqwest 0.13 uses the `rustls` feature name; `rustls-tls` is from older Reqwest releases. Defaults are disabled to avoid selecting a native TLS backend.
- `futures-util` is direct because provider code will use its `StreamExt` API; it is not imported through Reqwest's transitive dependency.
- Cancellation, `StreamExt`, and temporary filesystem fixtures are already named by the plan's provider, cancellation, and test boundaries, so their supporting crates are declared here rather than added transitively later.

## Sources

- [Ratatui installation and feature flags](https://ratatui.rs/installation/) — current Rust floor and matching Crossterm backend version.
- [Tokio 1.53.2 documentation](https://docs.rs/tokio/1.53.2/tokio/), [Tokio-util 0.7.19 documentation](https://docs.rs/tokio-util/0.7.19/tokio_util/), and [Crossterm 0.29.0 documentation](https://docs.rs/crossterm/0.29.0/crossterm/).
- [Ratatui 0.30.2 documentation](https://docs.rs/ratatui/0.30.2/ratatui/), [Reqwest 0.13.5 documentation](https://docs.rs/reqwest/0.13.5/reqwest/), and [futures-util 0.3.34 documentation](https://docs.rs/futures-util/0.3.34/futures_util/).
- [Serde 1.0.229](https://docs.rs/serde/1.0.229/serde/), [Serde JSON 1.0.151](https://docs.rs/serde_json/1.0.151/serde_json/), and [Clap 4.6.7](https://docs.rs/clap/4.6.7/clap/).
- [Tracing 0.1.44](https://docs.rs/tracing/0.1.44/tracing/), [tracing-subscriber 0.3.23](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/), [thiserror 2.0.21](https://docs.rs/thiserror/2.0.21/thiserror/), and [tempfile 3.27.0](https://docs.rs/tempfile/3.27.0/tempfile/).
- [Crates.io Ratatui package metadata](https://crates.io/api/v1/crates/ratatui) — published versions and declared Rust-version metadata checked on 2026-10-03; the crate documentation links above give the selected releases for the remaining dependencies.
