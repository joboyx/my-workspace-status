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

MISSING=0
ok() { echo "ok      $*"; }
missing() {
  echo "missing $*" >&2
  MISSING=1
}
warn() { echo "warn    $*" >&2; }

# Pinned channel from rust-toolchain.toml (`channel = "1.85.0"`).
CHANNEL="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$REPO_ROOT/rust-toolchain.toml")"
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
# rustup_probe ARGS...: read-only rustup query that cannot install anything.
# Inside the repository, rust-toolchain.toml makes `rustup --version` and
# `rustup toolchain list` resolve (and install) the pinned toolchain. Run
# from / with RUSTUP_AUTO_INSTALL=0 (rustup 1.28+) and no RUSTUP_TOOLCHAIN,
# so older rustup has no toolchain file or override to act on either.
rustup_probe() {
  (cd / && env -u RUSTUP_TOOLCHAIN RUSTUP_AUTO_INSTALL=0 rustup "$@")
}

# has_line_prefix TEXT PREFIX: some line of TEXT starts with PREFIX.
# Plain string match: no grep pipe (SIGPIPE under pipefail) and no regex
# meaning for the dots of a version.
has_line_prefix() {
  local line
  while IFS= read -r line; do
    [[ "$line" == "$2"* ]] && return 0
  done <<<"$1"
  return 1
}

RUSTUP_VERSION="$(rustup_probe --version 2>/dev/null)" || RUSTUP_VERSION=""
ok "rustup (${RUSTUP_VERSION%%$'\n'*})"

if ((CHECK)); then
  # `rustup toolchain list` lines: `1.85.0-<host> (default)`.
  TOOLCHAINS="$(rustup_probe toolchain list 2>/dev/null)" || TOOLCHAINS=""
  if has_line_prefix "$TOOLCHAINS" "$CHANNEL-"; then
    ok "toolchain $CHANNEL"
    # Installed component lines: `clippy-<host>`.
    COMPONENTS="$(rustup_probe component list --installed --toolchain "$CHANNEL" 2>/dev/null)" ||
      COMPONENTS=""
  else
    missing "toolchain $CHANNEL"
    COMPONENTS=""
  fi
  for component in clippy rustfmt; do
    if has_line_prefix "$COMPONENTS" "$component-"; then
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

cd "$REPO_ROOT"
# rustup reads rust-toolchain.toml (channel, profile, components). It skips
# components when the toolchain already exists, so add clippy/rustfmt apart.
rustup toolchain install "$CHANNEL" --profile minimal
rustup component add --toolchain "$CHANNEL" clippy rustfmt

cargo fetch --locked
cargo test --workspace --locked --no-run
echo "agent-bootstrap: ready"
