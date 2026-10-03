#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ ! -f Cargo.toml ]]; then
  echo "Rust scaffold is not present yet. Complete Task 1 before running Cargo checks." >&2
  exit 2
fi

cargo fmt --all -- --check
cargo test --offline --locked --lib "$@"
