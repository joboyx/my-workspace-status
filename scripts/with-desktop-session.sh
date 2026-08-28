#!/usr/bin/env bash
# Shared Xvfb + dbus + Openbox session for desktop TUI work.
#
# Callers:
#   GitHub Actions tui-tty-desktop and local ignored TTY e2e
#   scripts/capture-demo-stills.sh
#
# This script starts a display session. It does not launch xfce4-terminal or
# xterm, and it does not grab screenshots. Terminal spawn stays in the e2e
# harness. Demo stills stay in capture-demo-stills.sh.
#
# Usage (exec):
#   ./scripts/with-desktop-session.sh [--display N | --auto] -- command [args...]
#
# Usage (source from another script):
#   source scripts/with-desktop-session.sh
#   ws_desktop_session_start [--display N | --auto]
#   ws_desktop_session_stop
#
# --display N  Start Xvfb on :N (replace a leftover server on that display).
# --auto       Pick a free display from :99 upward (same idea as xvfb-run -a).
# Neither flag, DISPLAY already set: reuse that X. Start dbus and Openbox only.
# Neither flag, DISPLAY unset: same as --auto.
#
# Screen geometry is 1600x1000x24. Openbox uses scripts/openbox.xml (no
# decorations) so cell-to-pixel math matches the cell grid.

_WS_DESKTOP_SESSION_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_WS_OPENBOX_RC="$_WS_DESKTOP_SESSION_DIR/openbox.xml"
_WS_XVFB_LOG="${TMPDIR:-/tmp}/ws-desktop-session-xvfb.log"
_WS_OPENBOX_LOG="${TMPDIR:-/tmp}/ws-desktop-session-openbox.log"
_WS_XVFB_PID=""
_WS_OPENBOX_PID=""
_WS_DBUS_PID=""
_WS_STARTED_XVFB=0
_WS_STARTED_OPENBOX=0
_WS_STARTED_DBUS=0
_WS_PREV_DISPLAY=""
_WS_SESSION_ACTIVE=0

_ws_desktop_die() {
  echo "with-desktop-session: $*" >&2
  return 1
}

_ws_desktop_have() {
  command -v "$1" >/dev/null 2>&1
}

_ws_desktop_usage() {
  cat <<'USAGE'
usage: with-desktop-session.sh [--display N | --auto] -- command [args...]

Start Xvfb (when needed), dbus, and Openbox, then run the command.
DISPLAY already set and no --display/--auto: reuse that X.

source scripts/with-desktop-session.sh
ws_desktop_session_start [--display N | --auto]
ws_desktop_session_stop
USAGE
}

_ws_desktop_kill_pid() {
  local pid="${1:-}"
  [[ -n "$pid" ]] || return 0
  kill -0 "$pid" 2>/dev/null || return 0
  kill "$pid" 2>/dev/null || true
  local i
  for i in $(seq 1 20); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -9 "$pid" 2>/dev/null || true
  return 0
}

_ws_desktop_display_busy() {
  local n="$1"
  [[ -S "/tmp/.X11-unix/X$n" || -f "/tmp/.X$n-lock" ]]
}

_ws_desktop_lock_pid() {
  local lock="/tmp/.X$1-lock"
  local pid=""
  [[ -f "$lock" ]] || return 0
  pid="$(tr -d '[:space:]' <"$lock" 2>/dev/null || true)"
  if [[ "$pid" =~ ^[0-9]+$ ]]; then
    printf '%s' "$pid"
  fi
}

_ws_desktop_release_display() {
  local n="$1"
  local sock="/tmp/.X11-unix/X$n"
  local pid
  pid="$(_ws_desktop_lock_pid "$n")"
  if [[ -n "$pid" ]]; then
    _ws_desktop_kill_pid "$pid"
  elif _ws_desktop_display_busy "$n"; then
    _ws_desktop_die "display :$n is in use (no PID in /tmp/.X$n-lock)"
    return 1
  else
    return 0
  fi
  local i
  for i in $(seq 1 30); do
    if ! _ws_desktop_display_busy "$n"; then
      return 0
    fi
    sleep 0.1
  done
  _ws_desktop_die "display :$n did not release"
}

_ws_desktop_pick_display() {
  local n
  for n in $(seq 99 119); do
    if ! _ws_desktop_display_busy "$n"; then
      printf '%s' "$n"
      return 0
    fi
  done
  _ws_desktop_die "no free X display in :99-:119"
}

_ws_desktop_parse_args() {
  _WS_ARG_DISPLAY=""
  _WS_ARG_AUTO=0
  _WS_ARG_CMD=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      -h | --help)
        _ws_desktop_usage
        return 2
        ;;
      --display)
        [[ -n "${2:-}" ]] || {
          _ws_desktop_die "missing value for --display"
          return 1
        }
        _WS_ARG_DISPLAY="$2"
        shift 2
        ;;
      --display=*)
        _WS_ARG_DISPLAY="${1#--display=}"
        shift
        ;;
      --auto)
        _WS_ARG_AUTO=1
        shift
        ;;
      --)
        shift
        _WS_ARG_CMD=("$@")
        return 0
        ;;
      -*)
        _ws_desktop_die "unknown option: $1"
        return 1
        ;;
      *)
        _WS_ARG_CMD=("$@")
        return 0
        ;;
    esac
  done
  return 0
}

_ws_desktop_start_xvfb() {
  local n="$1"
  _ws_desktop_have Xvfb || {
    _ws_desktop_die "Xvfb missing. Install xvfb (Debian/Ubuntu: xvfb)."
    return 1
  }
  _WS_PREV_DISPLAY="${DISPLAY-}"
  _ws_desktop_release_display "$n" || return 1
  Xvfb ":$n" -screen 0 1600x1000x24 -nolisten tcp >"$_WS_XVFB_LOG" 2>&1 &
  _WS_XVFB_PID=$!
  _WS_STARTED_XVFB=1
  export DISPLAY=":$n"
  local i
  for i in $(seq 1 50); do
    if [[ -S "/tmp/.X11-unix/X$n" ]]; then
      return 0
    fi
    sleep 0.1
  done
  _ws_desktop_die "Xvfb :$n did not start (see $_WS_XVFB_LOG)"
}

_ws_desktop_start_dbus() {
  if [[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
    return 0
  fi
  _ws_desktop_have dbus-launch || return 0
  eval "$(dbus-launch --sh-syntax)"
  _WS_DBUS_PID="${DBUS_SESSION_BUS_PID:-}"
  if [[ -n "$_WS_DBUS_PID" ]]; then
    _WS_STARTED_DBUS=1
  fi
}

_ws_desktop_start_openbox() {
  _ws_desktop_have openbox || return 0
  if pgrep -x openbox >/dev/null 2>&1; then
    # Session Openbox should already use scripts/openbox.xml. Do not
    # --replace: that would steal a shared DISPLAY.
    return 0
  fi
  [[ -f "$_WS_OPENBOX_RC" ]] || {
    _ws_desktop_die "missing Openbox rc: $_WS_OPENBOX_RC"
    return 1
  }
  setsid openbox --config-file "$_WS_OPENBOX_RC" >"$_WS_OPENBOX_LOG" 2>&1 &
  _WS_OPENBOX_PID=$!
  _WS_STARTED_OPENBOX=1
  sleep 0.4
}

# Start Xvfb when needed, then dbus and Openbox. Safe to call once per process.
ws_desktop_session_start() {
  if [[ "$_WS_SESSION_ACTIVE" -eq 1 ]]; then
    return 0
  fi
  _ws_desktop_parse_args "$@" || {
    local st=$?
    [[ $st -eq 2 ]] && return 0
    return 1
  }
  if [[ $_WS_ARG_AUTO -eq 1 && -n "$_WS_ARG_DISPLAY" ]]; then
    _ws_desktop_die "use either --display or --auto, not both"
    return 1
  fi
  local display_num=""
  if [[ -n "$_WS_ARG_DISPLAY" ]]; then
    [[ "$_WS_ARG_DISPLAY" =~ ^[0-9]+$ ]] || {
      _ws_desktop_die "--display needs an integer, got: $_WS_ARG_DISPLAY"
      return 1
    }
    display_num="$_WS_ARG_DISPLAY"
  elif [[ $_WS_ARG_AUTO -eq 1 || -z "${DISPLAY:-}" ]]; then
    display_num="$(_ws_desktop_pick_display)" || return 1
  fi
  if [[ -n "$display_num" ]]; then
    _ws_desktop_start_xvfb "$display_num" || return 1
  fi
  export NO_AT_BRIDGE=1
  export GTK_A11Y=none
  _ws_desktop_start_dbus || return 1
  _ws_desktop_start_openbox || return 1
  [[ -n "${DISPLAY:-}" ]] || {
    _ws_desktop_die "DISPLAY is unset after session start"
    return 1
  }
  _WS_SESSION_ACTIVE=1
}

# Stop processes this script started. Leaves a pre-existing X or Openbox.
ws_desktop_session_stop() {
  if [[ "$_WS_STARTED_OPENBOX" -eq 1 ]]; then
    _ws_desktop_kill_pid "$_WS_OPENBOX_PID"
    _WS_OPENBOX_PID=""
    _WS_STARTED_OPENBOX=0
  fi
  if [[ "$_WS_STARTED_DBUS" -eq 1 ]]; then
    _ws_desktop_kill_pid "$_WS_DBUS_PID"
    _WS_DBUS_PID=""
    _WS_STARTED_DBUS=0
    unset DBUS_SESSION_BUS_ADDRESS DBUS_SESSION_BUS_PID
  fi
  if [[ "$_WS_STARTED_XVFB" -eq 1 ]]; then
    _ws_desktop_kill_pid "$_WS_XVFB_PID"
    _WS_XVFB_PID=""
    _WS_STARTED_XVFB=0
    if [[ -n "$_WS_PREV_DISPLAY" ]]; then
      export DISPLAY="$_WS_PREV_DISPLAY"
    else
      unset DISPLAY
    fi
  fi
  _WS_SESSION_ACTIVE=0
}

_ws_desktop_session_main() {
  _ws_desktop_parse_args "$@" || {
    local st=$?
    [[ $st -eq 2 ]] && return 0
    return "$st"
  }
  if [[ ${#_WS_ARG_CMD[@]} -eq 0 ]]; then
    _ws_desktop_usage >&2
    return 2
  fi
  local start_flags=()
  local cmd=("${_WS_ARG_CMD[@]}")
  if [[ -n "$_WS_ARG_DISPLAY" ]]; then
    start_flags+=(--display "$_WS_ARG_DISPLAY")
  elif [[ $_WS_ARG_AUTO -eq 1 ]]; then
    start_flags+=(--auto)
  fi
  ws_desktop_session_start "${start_flags[@]}" || return 1
  trap 'ws_desktop_session_stop' EXIT
  "${cmd[@]}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  trap '' HUP
  _ws_desktop_session_main "$@"
fi
