#!/bin/bash
# SessionStart hook for Claude Code cloud sessions: installs the pinned Rust
# toolchain (rust-toolchain.toml) with the CI lint components, fetches crates
# and pre-builds test binaries so `cargo test` / `cargo clippy` start warm.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}"

# rustup reads rust-toolchain.toml. `toolchain install` skips components when
# the toolchain already exists, so add clippy/rustfmt (CI `lint` job) apart.
rustup toolchain install --profile minimal
rustup component add clippy rustfmt

cargo fetch --locked
cargo test --workspace --locked --no-run
