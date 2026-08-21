#!/bin/bash
# Launcher for workspace-status.
# Resolves to the script directory so it works when invoked from any cwd.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ENTRYPOINT="$SCRIPT_DIR/dist/index.js"
NODE_MODULES="$SCRIPT_DIR/node_modules"
LOCKFILE="$SCRIPT_DIR/package-lock.json"

needs_install() {
  [[ ! -d "$NODE_MODULES" ]] && return 0
  [[ ! -f "$NODE_MODULES/.package-lock.json" ]] && return 0
  [[ -f "$LOCKFILE" && "$LOCKFILE" -nt "$NODE_MODULES/.package-lock.json" ]] && return 0
  return 1
}

if needs_install; then
  npm --prefix "$SCRIPT_DIR" ci >/dev/null
fi

needs_build() {
  [[ ! -f "$ENTRYPOINT" ]] && return 0
  [[ "$SCRIPT_DIR/tsconfig.json" -nt "$ENTRYPOINT" ]] && return 0
  find "$SCRIPT_DIR/src" -type f \( -name '*.ts' -o -name '*.tsx' \) -newer "$ENTRYPOINT" -print -quit | grep -q .
}

if needs_build; then
  npm --prefix "$SCRIPT_DIR" run build >/dev/null
fi

exec node "$ENTRYPOINT" "$@"
