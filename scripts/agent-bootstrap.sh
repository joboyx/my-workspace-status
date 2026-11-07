#!/usr/bin/env bash
# Prepare a cold machine to build and test this repository.
#
# Checks git and a C linker, installs the toolchain that rust-toolchain.toml
# pins (with clippy and rustfmt), fetches crates, and builds the test binaries
# so the first `cargo test` / `cargo clippy` starts warm.
#
# Usage: scripts/agent-bootstrap.sh [--check] [--help]

set -euo pipefail

usage() {
  cat <<'USAGE'
usage: scripts/agent-bootstrap.sh [--check] [--help]

Prepare this checkout for `cargo build`, `cargo clippy` and `cargo test`:
check git and a C linker, install the toolchain pinned in rust-toolchain.toml
with clippy and rustfmt, run `cargo fetch --locked`, then build the test
binaries (`cargo test --workspace --locked --no-run`).

  --check   Report what is present and what is missing. Install nothing.
  --help    Show this help.

Exit status: 0 when the checkout is ready (or, with --check, when nothing
required is missing), 1 when a required tool is missing.
tmux is optional: only the verify skill driver (scripts/verify-tui.sh) needs it.
USAGE
}

CHECK=0
for arg in "$@"; do
  case "$arg" in
    --check) CHECK=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "agent-bootstrap: unknown argument: $arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

MISSING=0
ok() { echo "ok      $*"; }
missing() {
  echo "missing $*" >&2
  MISSING=1
}
warn() { echo "warn    $*" >&2; }

# Pinned channel from rust-toolchain.toml (`channel = "1.85.0"`).
CHANNEL="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)"
[[ -n "$CHANNEL" ]] || {
  echo "agent-bootstrap: no channel in rust-toolchain.toml" >&2
  exit 1
}

if command -v git >/dev/null 2>&1; then ok "git ($(git --version))"; else missing "git"; fi

LINKER=""
for cc in cc gcc clang; do
  if command -v "$cc" >/dev/null 2>&1; then
    LINKER="$cc"
    break
  fi
done
if [[ -n "$LINKER" ]]; then ok "C linker ($LINKER)"; else missing "C linker (cc, gcc or clang)"; fi

if command -v tmux >/dev/null 2>&1; then
  ok "tmux"
else
  warn "tmux not found: the verify skill driver (scripts/verify-tui.sh) needs it"
fi

if ! command -v rustup >/dev/null 2>&1; then
  missing "rustup (install from https://rustup.rs, then re-run)"
  exit 1
fi
ok "rustup ($(rustup --version 2>/dev/null | head -n 1))"

toolchain_installed() {
  rustup toolchain list | grep -q "^$CHANNEL-"
}

component_installed() {
  rustup component list --installed --toolchain "$CHANNEL" 2>/dev/null | grep -q "^$1-"
}

if ((CHECK)); then
  if toolchain_installed; then ok "toolchain $CHANNEL"; else missing "toolchain $CHANNEL"; fi
  for component in clippy rustfmt; do
    if toolchain_installed && component_installed "$component"; then
      ok "component $component"
    else
      missing "component $component"
    fi
  done
  if ((MISSING)); then
    echo "agent-bootstrap: run without --check to install what is missing" >&2
    exit 1
  fi
  echo "agent-bootstrap: ready"
  exit 0
fi

if ((MISSING)); then
  echo "agent-bootstrap: install the missing tools above, then re-run" >&2
  exit 1
fi

# rustup reads rust-toolchain.toml (channel, profile, components). It skips
# components when the toolchain already exists, so add clippy/rustfmt apart.
rustup toolchain install "$CHANNEL" --profile minimal
rustup component add --toolchain "$CHANNEL" clippy rustfmt

cargo fetch --locked
cargo test --workspace --locked --no-run
echo "agent-bootstrap: ready"
