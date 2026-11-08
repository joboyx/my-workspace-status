#!/bin/bash
# SessionStart hook for Claude Code cloud sessions: runs
# scripts/agent-bootstrap.sh, which installs the pinned Rust toolchain
# (rust-toolchain.toml) with the CI lint components, fetches crates and
# pre-builds test binaries so `cargo test` / `cargo clippy` start warm.
# Local sessions skip it; run the script by hand on a cold machine.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(dirname "$0")/../..}"

exec scripts/agent-bootstrap.sh
