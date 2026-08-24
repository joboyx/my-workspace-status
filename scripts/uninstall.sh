#!/usr/bin/env bash
# Remove a cargo-dist install of workspace-status (bins, updater, receipt).
set -euo pipefail

app=workspace-status
bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
receipt_dir="${config_home}/${app}"
receipt="${receipt_dir}/${app}-receipt.json"

remove_bins() {
  local dir=$1
  [[ -d "$dir" ]] || return 0
  rm -f -- "$dir/ws" "$dir/workspace-status" "$dir/workspace-status-update"
}

remove_bins "$bin_dir"

if [[ -f "$receipt" ]]; then
  prefix=$(sed -n 's/.*"install_prefix":[[:space:]]*"\([^"]*\)".*/\1/p' "$receipt" | head -n 1)
  if [[ -n "$prefix" ]]; then
    remove_bins "$prefix"
    remove_bins "${prefix}/bin"
  fi
  rm -f -- "$receipt"
fi

if [[ -d "$receipt_dir" ]]; then
  rmdir "$receipt_dir" 2>/dev/null || true
fi

echo "Removed ${app} binaries (ws, workspace-status, workspace-status-update) and the cargo-dist install receipt."
