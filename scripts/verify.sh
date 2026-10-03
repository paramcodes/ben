#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ ! -f Cargo.toml ]]; then
  echo "Rust scaffold is not present yet. Complete Task 1 before running Cargo checks." >&2
  exit 2
fi

scripts/check-docs.sh
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
cargo test --offline --locked --all-targets
cargo build --offline --locked --release
