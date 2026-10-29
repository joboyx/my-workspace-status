#!/usr/bin/env bash
# Drive this checkout's own workspace-status in a private tmux server, on a
# disposable seeded workspace, and keep screen captures as proof.
#
# This is a test harness for the verify skill (.agents/skills/verify/SKILL.md),
# not the demo capture pipeline: scripts/capture-demo-stills.sh owns the README
# clips. The isolation below follows the TTY spawn rules in AGENTS.md; CI guards
# it in crates/workspace-status/tests/release_watch.rs.
#
# Run `scripts/verify-tui.sh help` for the commands. Needs bash and tmux
# (Linux and macOS; bash 3.2 is enough).
set -euo pipefail
unset CDPATH

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
SCENARIOS_DIR="$REPO_ROOT/.agents/skills/verify/scenarios"
ROOT="${WS_VERIFY_ROOT:-${TMPDIR:-/tmp}/ws-verify}"
ROOT="${ROOT%/}"
SESSION=ws

# GitHub / GitLab origins the seeded repos get under --fake-forge. The scenario
# sets in .agents/skills/verify/scenarios/ are keyed to these paths.
APP_ORIGIN=https://github.com/acme/app.git
API_ORIGIN=https://gitlab.com/acme/services/api.git
MERGER_ORIGIN=https://github.com/acme/merger.git

die() {
  echo "verify-tui: $*" >&2
  exit 1
}

usage() {
  cat <<'USAGE'
usage: scripts/verify-tui.sh <command> ...

  build                          build target/debug/workspace-status; print its path
  launch [--size COLSxROWS] [--env K=V]... [--fake-forge DIR] [--workspace DIR]
                                 seed a workspace, start the TUI in a private tmux
                                 server, wait for the chrome; print the run dir
  keys [-l] <run> <key>...       send tmux key names (j Enter Escape Tab C-c '?');
                                 with -l send one argument as literal text
  mouse <run> move|click <col> <row>
                                 send an SGR mouse report at a 1-based screen cell
                                 (row 1 is the first capture line): move = pointer
                                 motion, no button (hover); click = left press and
                                 release
  mouse <run> find <text>        print "<col> <row>" (1-based) of the first cell
                                 where <text> starts on screen; exit 1 if absent
  wait [--gone] [--cursor|--regex] <run> <text> [timeout-s]
                                 wait until <text> is on screen; --gone: until it is
                                 not; --cursor: a line with the focus bar holds it;
                                 --regex: <text> is an extended regex for one line;
                                 default 10 s; a timeout, a dead TUI or a missing
                                 session saves the screen and exits 1
  capture <run> [name]           print the screen; save evidence/NN-<name>.txt/.ansi
  stop <run>                     quit the TUI, end the tmux server, keep the run dir
  cli <run> -- <args>            run the binary headless (--plain/--json/--help/
                                 --version) with the same isolation in the fixture
  scenario [--reset] <forge-dir> <set>...
                                 copy scenario sets into a fake forge directory;
                                 --reset clears the old ones first (before launch)
                                 (without --reset the sets overlay what is there, and
                                 each file is renamed in, so it is the mid-run swap)
  calls <run>                    print the gh / glab calls this run made (fake forge)

<run> is the run dir that launch prints, or its id under the root.
Root: $WS_VERIFY_ROOT, else ${TMPDIR:-/tmp}/ws-verify.
USAGE
}

# ---------- paths and run state ----------

real_dir() { (cd "$1" 2>/dev/null && pwd -P); }

under() { # under CHILD PARENT: CHILD is PARENT or inside it (real paths)
  [[ "$1" == "$2" || "$1" == "$2"/* ]]
}

binary_path() {
  local target="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
  [[ "$target" == /* ]] || target="$REPO_ROOT/$target"
  echo "$target/debug/workspace-status"
}

load_run() {
  local arg="${1:-}"
  [[ -n "$arg" ]] || die "missing <run> (the dir that launch printed)"
  if [[ -f "$arg/run.env" ]]; then
    RUN_DIR="$(real_dir "$arg")"
  elif [[ -f "$ROOT/$arg/run.env" ]]; then
    RUN_DIR="$(real_dir "$ROOT/$arg")"
  else
    die "no run '$arg' (no run.env in it or under $ROOT)"
  fi
  # shellcheck source=/dev/null
  source "$RUN_DIR/run.env"
  EVIDENCE="$RUN_DIR/evidence"
  SOCK="$RUN_SOCK"
}

record() { printf '%s=%q\n' "$1" "$2" >>"$RUN_DIR/run.env"; }

next_seq() {
  local f="$RUN_DIR/.seq" n=0
  if [[ -f "$f" ]]; then n="$(cat "$f")"; fi
  n=$((n + 1))
  echo "$n" >"$f"
  printf '%02d' "$n"
}

log_action() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >>"$EVIDENCE/actions.log" 2>/dev/null || true; }

tm() { tmux -L "$SOCK" "$@"; }
pane_field() { tm display-message -p -t "$SESSION" "#{$1}" 2>/dev/null; }
screen_text() { tm capture-pane -p -t "$SESSION" 2>/dev/null; }

# End this run's private tmux server. tmux leaves the socket file behind and every
# run has its own socket name, so remove the file once no server answers on it.
end_server() {
  tm kill-server 2>/dev/null || true
  if [[ -n "${RUN_SOCKET_PATH:-}" && -S "$RUN_SOCKET_PATH" ]] && ! tm list-sessions >/dev/null 2>&1; then
    rm -f -- "$RUN_SOCKET_PATH"
  fi
}

require_live() {
  tm has-session -t "$SESSION" 2>/dev/null || die "no tmux session for this run: it was stopped or never started"
  [[ "$(pane_field pane_dead)" == 0 ]] || die "the TUI exited (status $(pane_field pane_dead_status)); capture shows its last screen"
}

# ---------- build ----------

cmd_build() {
  (cd "$REPO_ROOT" && cargo build -p workspace-status --bin workspace-status) >&2
  local bin
  bin="$(binary_path)"
  [[ -x "$bin" ]] || die "build finished but $bin is missing"
  echo "$bin"
}

# ---------- isolation ----------

# Write the env file that launch and cli both source: drop what would retarget
# or restyle the run, then pin the run to its own dirs. $1 is the file, $2 the
# fake forge dir (empty for a live run), then the --env overrides.
write_env_file() {
  local file="$1" forge="$2" locale=C.UTF-8
  shift 2
  if [[ "$(uname)" == Darwin ]]; then locale=en_US.UTF-8; fi
  {
    cat <<'EOF'
# Isolation for one verify run, written by scripts/verify-tui.sh.
# Drop parent state that would retarget or restyle the run.
unset NO_COLOR FORCE_COLOR CLICOLOR_FORCE VISUAL DISPLAY WAYLAND_DISPLAY
unset WS_FAKE_FORGE_DIR
unset WS_STATUS_WORKSPACE WS_STATUS_THEME WS_STATUS_THEME_MODE
for v in $(compgen -e); do
  case "$v" in WS_STATUS_* | WORKSPACE_STATUS_*) unset "$v" ;; esac
done
unset v
# TUI git never reaches a network or reads operator hooks and signing config.
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_NOSYSTEM=1
export GIT_ALLOW_PROTOCOL=file
export GIT_TERMINAL_PROMPT=0
EOF
    printf 'export XDG_CONFIG_HOME=%q\n' "$RUN_DIR/config"
    printf 'export XDG_STATE_HOME=%q\n' "$RUN_DIR/state"
    printf 'export WS_STATUS_UPDATE_CHECK_STORE=%q\n' "$RUN_DIR/state/update-check.json"
    printf 'export WS_STATUS_UI_STORE=%q\n' "$RUN_DIR/state/ui-state.json"
    printf 'export WS_STATUS_COMMENT_STORE=%q\n' "$RUN_DIR/state/comments.json"
    printf 'export WS_STATUS_VIEWED_STORE=%q\n' "$RUN_DIR/state/viewed-files.json"
    cat <<'EOF'
export WS_STATUS_THEME_MODE=dark
export WS_STATUS_GLYPHS=ascii
export WS_STATUS_WATCH_MS=0
export WS_STATUS_FETCH_MS=0
export WS_STATUS_PR_POLL_MS=0
export TERM=xterm-256color
export COLORTERM=truecolor
export TZ=Asia/Manila
EOF
    printf 'export LANG=%s LC_ALL=%s\n' "$locale" "$locale"
    # The recorder gets every URL gx / gc opens, so a run never starts a browser.
    # Keep this line before the fake / live split below: both modes need it, and
    # release_watch.rs checks the order.
    printf 'export BROWSER=%q\n' "$RUN_DIR/browser/open %s"
    if [[ -n "$forge" ]]; then
      # Fake forge: the stand-ins come first on PATH, and no forge login of the
      # operator can reach the run.
      cat <<'EOF'
unset GH_TOKEN GITHUB_TOKEN GITLAB_TOKEN GH_ENTERPRISE_TOKEN GITHUB_ENTERPRISE_TOKEN GITLAB_ACCESS_TOKEN
unset GH_CONFIG_DIR GLAB_CONFIG_DIR
EOF
      printf 'export WS_FAKE_FORGE_DIR=%q\n' "$forge"
      printf 'export PATH=%q\n' "$SCRIPT_DIR/fake-forge:$PATH"
    else
      write_live_cli_config
    fi
    local kv
    for kv in "$@"; do
      printf 'export %s=%q\n' "${kv%%=*}" "${kv#*=}"
    done
  } >"$file"
}

# Live run: gh and glab keep the operator login. Token variables stay as they
# are. The config dirs move with XDG_CONFIG_HOME, so resolve them here, while
# the operator value is still in place, and export them explicitly.
write_live_cli_config() {
  local gh_dir="${GH_CONFIG_DIR:-}" glab_dir="${GLAB_CONFIG_DIR:-}" path
  if [[ -z "$gh_dir" ]]; then gh_dir="${XDG_CONFIG_HOME:-$HOME/.config}/gh"; fi
  if [[ -z "$glab_dir" ]] && command -v glab >/dev/null 2>&1; then
    # `glab config path` names the config file on every platform.
    path="$(glab config path 2>/dev/null | head -n 1 || true)"
    if [[ "$path" == /* ]]; then glab_dir="$(dirname "$path")"; fi
  fi
  if [[ -z "$glab_dir" ]]; then glab_dir="${XDG_CONFIG_HOME:-$HOME/.config}/glab-cli"; fi
  printf 'export GH_CONFIG_DIR=%q\n' "$gh_dir"
  printf 'export GLAB_CONFIG_DIR=%q\n' "$glab_dir"
}

# ---------- launch ----------

cmd_launch() {
  local size=160x45 forge="" workspace="" extra=()
  while (($#)); do
    case "$1" in
      --size) size="${2:?--size needs COLSxROWS}"; shift 2 ;;
      --fake-forge) forge="${2:?--fake-forge needs DIR}"; shift 2 ;;
      --workspace) workspace="${2:?--workspace needs DIR}"; shift 2 ;;
      --env)
        [[ "${2:-}" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] || die "--env needs K=V"
        extra+=("$2")
        shift 2
        ;;
      *) die "launch: unknown argument $1" ;;
    esac
  done
  [[ "$size" =~ ^([0-9]+)x([0-9]+)$ ]] || die "--size must be COLSxROWS, for example 160x45"
  local cols="${BASH_REMATCH[1]}" rows="${BASH_REMATCH[2]}"
  command -v tmux >/dev/null 2>&1 || die "tmux is not installed"

  local bin
  bin="$(binary_path)"
  [[ -x "$bin" ]] || die "no binary at $bin: run scripts/verify-tui.sh build first"

  mkdir -p "$ROOT"
  local root_real fixture="" seeded=0
  root_real="$(real_dir "$ROOT")"
  if [[ -n "$workspace" ]]; then
    [[ -d "$workspace" ]] || die "--workspace $workspace is not a directory"
    fixture="$(real_dir "$workspace")"
    if [[ "$fixture" == "$root_real" ]] || ! under "$fixture" "$root_real"; then
      die "--workspace must be a fixture inside $root_real (got $fixture)"
    fi
  fi

  local run_id
  run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
  RUN_DIR="$root_real/$run_id"
  # $BROWSER splits on spaces and colons, so the recorder path may hold neither.
  case "$RUN_DIR" in *[[:space:]:]*) die "run dir '$RUN_DIR' holds a space or colon; set WS_VERIFY_ROOT to a plain path" ;; esac
  EVIDENCE="$RUN_DIR/evidence"
  SOCK="ws-verify-$run_id"
  mkdir -p "$EVIDENCE" "$RUN_DIR/config" "$RUN_DIR/state" "$RUN_DIR/browser"
  : >"$RUN_DIR/run.env"

  if [[ -z "$fixture" ]]; then
    fixture="$RUN_DIR/workspace"
    "$SCRIPT_DIR/seed-demo-workspace.sh" "$fixture" >"$EVIDENCE/seed.log" 2>&1 ||
      die "seed failed, see $EVIDENCE/seed.log"
    seeded=1
  fi

  if [[ -n "$forge" ]]; then
    mkdir -p "$forge"
    forge="$(real_dir "$forge")"
    if ((seeded)); then
      git -C "$fixture/app" remote set-url origin "$APP_ORIGIN"
      git -C "$fixture/services/api" remote set-url origin "$API_ORIGIN"
      git -C "$fixture/merger" remote set-url origin "$MERGER_ORIGIN"
    fi
  fi

  # The recorder appends each URL the TUI opens to opened.log, one per line.
  cat >"$RUN_DIR/browser/open" <<EOF
#!/bin/sh
printf '%s\n' "\$1" >> $(printf '%q' "$RUN_DIR/opened.log")
EOF
  chmod +x "$RUN_DIR/browser/open"

  # A fresh lastCheckUnix keeps the 6 h update prompt from blocking mount.
  printf '{\n  "version": 1,\n  "lastCheckUnix": %s\n}\n' "$(date +%s)" >"$RUN_DIR/state/update-check.json"

  write_env_file "$RUN_DIR/env.sh" "$forge" ${extra[@]+"${extra[@]}"}
  cp "$RUN_DIR/env.sh" "$EVIDENCE/env.sh.txt"
  local launch="$RUN_DIR/launch.sh"
  {
    echo '#!/usr/bin/env bash'
    printf 'source %q\n' "$RUN_DIR/env.sh"
    printf 'cd %q\n' "$fixture"
    printf 'exec %q\n' "$bin"
  } >"$launch"
  chmod +x "$launch"

  record RUN_ID "$run_id"
  record RUN_SOCK "$SOCK"
  record RUN_BIN "$bin"
  record RUN_FIXTURE "$fixture"
  record RUN_FORGE "$forge"
  # Runs share one forge dir and its calls.log. Remember where this run starts.
  local calls_from=0
  if [[ -n "$forge" && -f "$forge/calls.log" ]]; then calls_from="$(wc -l <"$forge/calls.log" | tr -d ' ')"; fi
  record RUN_CALLS_FROM "$calls_from"
  record RUN_SIZE "$size"
  record RUN_HEAD "$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  record RUN_VERSION "$("$bin" --version </dev/null 2>&1 || echo unknown)"

  tmux -L "$SOCK" -f /dev/null start-server \; set-option -g status off \; \
    set-option -g remain-on-exit on \; \
    new-session -d -s "$SESSION" -x "$cols" -y "$rows" "$(printf '%q' "$launch")"
  RUN_SOCKET_PATH="$(tm display-message -p '#{socket_path}')"
  record RUN_SOCKET_PATH "$RUN_SOCKET_PATH"

  local i scr=""
  for ((i = 0; i < 80; i++)); do
    scr="$(screen_text || true)"
    if [[ "$scr" == *"Update? [y/N]"* ]]; then
      printf '%s\n' "$scr" >"$EVIDENCE/launch-failure.txt"
      end_server
      die "update prompt on screen (isolation failed); see $EVIDENCE/launch-failure.txt"
    fi
    if [[ "$(pane_field pane_dead)" == 1 ]]; then
      printf '%s\n' "$scr" >"$EVIDENCE/launch-failure.txt"
      end_server
      die "the TUI exited at launch; see $EVIDENCE/launch-failure.txt"
    fi
    if [[ "$scr" == *"? help"* ]]; then break; fi
    sleep 0.25
  done
  if [[ "$scr" != *"? help"* ]]; then
    printf '%s\n' "$scr" >"$EVIDENCE/launch-failure.txt"
    end_server
    die "no '? help' chrome after 20 s; see $EVIDENCE/launch-failure.txt"
  fi
  cp "$RUN_DIR/run.env" "$EVIDENCE/run.txt"
  log_action "launch $bin in $fixture (${size}${forge:+, fake forge $forge})"
  echo "ready: $("$bin" --version </dev/null 2>&1), ${forge:+fake forge $forge, }run $RUN_DIR" >&2
  echo "$RUN_DIR"
}

# ---------- drive ----------

cmd_keys() {
  local literal=0
  if [[ "${1:-}" == -l ]]; then
    literal=1
    shift
  fi
  load_run "${1:-}"
  shift
  (($#)) || die "keys: give key names (or one text argument with -l)"
  require_live
  if ((literal)); then
    (($# == 1)) || die "keys -l: give one text argument"
    # Send one key per write, so every key reaches the app as its own event.
    local text="$1" i
    for ((i = 0; i < ${#text}; i++)); do
      tm send-keys -t "$SESSION" -l -- "${text:i:1}"
      sleep 0.02
    done
  else
    tm send-keys -t "$SESSION" -- "$@"
  fi
  if ((literal)); then log_action "keys -l $*"; else log_action "keys $*"; fi
}

# The TUI turns on xterm SGR (1006) mouse reports, so a report written to the
# pane reaches it as a real pointer event. Cells are 1-based, as the reports
# count them; with the status line off, capture line N is screen row N.
cmd_mouse() {
  load_run "${1:-}"
  local action="${2:-}"
  require_live
  case "$action" in
    move | click)
      local col="${3:-}" row="${4:-}"
      [[ "$col" =~ ^[1-9][0-9]*$ && "$row" =~ ^[1-9][0-9]*$ ]] ||
        die "mouse $action: give <col> <row> as positive integers (1-based cells), got '$col' '$row'"
      if [[ "$action" == move ]]; then
        # Button 3 + 32: motion with no button held (any-event tracking).
        tm send-keys -t "$SESSION" -l -- $'\e'"[<35;$col;${row}M"
      else
        tm send-keys -t "$SESSION" -l -- $'\e'"[<0;$col;${row}M"
        tm send-keys -t "$SESSION" -l -- $'\e'"[<0;$col;${row}m"
      fi
      log_action "mouse $action $col $row"
      ;;
    find)
      local text="${3:-}"
      [[ -n "$text" ]] || die "mouse find: give the text to find"
      local cell
      cell="$(screen_cell "$text" "$(screen_text || true)")" ||
        die "mouse find: '$text' is not on screen"
      log_action "mouse find '$text' -> $cell"
      echo "$cell"
      ;;
    *) die "mouse: give move|click <col> <row>, or find <text>" ;;
  esac
}

# screen_cell TEXT SCREEN: "<col> <row>" (1-based) of the first cell where TEXT
# starts. One cell per character, so wide glyphs (CJK, some emoji) put later
# columns off by one each; the driver's default ASCII glyphs have none.
screen_cell() {
  local text="$1" line prefix row=0 LC_ALL=C.UTF-8
  if [[ "$(uname)" == Darwin ]]; then LC_ALL=en_US.UTF-8; fi
  while IFS= read -r line; do
    row=$((row + 1))
    if [[ "$line" == *"$text"* ]]; then
      prefix="${line%%"$text"*}"
      echo "$((${#prefix} + 1)) $row"
      return 0
    fi
  done <<<"$2"
  return 1
}

# screen_matches KIND TEXT SCREEN. KIND is text (a substring), regex (an
# extended regular expression that matches inside one screen line), or cursor
# (a screen line that holds both the focus bar and the substring TEXT).
screen_matches() {
  local bars
  case "$1" in
    text) [[ "$3" == *"$2"* ]] ;;
    regex) grep -qE -- "$2" <<<"$3" ;;
    cursor)
      bars="$(grep -F '▌' <<<"$3" || true)"
      grep -qF -- "$2" <<<"$bars"
      ;;
  esac
}

cmd_wait() {
  local kind=text negate=0
  while [[ "${1:-}" == --* ]]; do
    case "$1" in
      --gone) negate=1 ;;
      --cursor) kind=cursor ;;
      --regex) kind=regex ;;
      *) die "wait: unknown option $1" ;;
    esac
    shift
  done
  load_run "${1:-}"
  local text="${2:-}" timeout="${3:-10}" what found end scr f
  [[ -n "$text" ]] || die "wait: give the text to wait for"
  if ((negate)); then what="$kind gone"; else what="$kind"; fi
  end=$((SECONDS + timeout))
  while :; do
    scr="$(screen_text || true)"
    # A dead pane or a missing server proves nothing, so this check comes before
    # the match, in both modes: a gone TUI must never satisfy --gone.
    if ! tm has-session -t "$SESSION" 2>/dev/null; then
      f="$EVIDENCE/$(next_seq)-wait-failed.txt"
      printf '%s\n' "$scr" >"$f"
      log_action "wait $what '$text' FAILED: no tmux session -> $(basename "$f")"
      die "wait $what '$text' failed: no tmux session for this run (stopped, or the server is gone); see $f"
    fi
    if [[ "$(pane_field pane_dead)" != 0 ]]; then
      f="$EVIDENCE/$(next_seq)-wait-failed.txt"
      printf '%s\n' "$scr" >"$f"
      log_action "wait $what '$text' FAILED: the TUI exited -> $(basename "$f")"
      die "wait $what '$text' failed: the TUI exited (status $(pane_field pane_dead_status)); its last screen is in $f"
    fi
    found=0
    if screen_matches "$kind" "$text" "$scr"; then found=1; fi
    if ((found + negate == 1)); then
      log_action "wait $what '$text' ok"
      return 0
    fi
    if ((SECONDS >= end)); then
      f="$EVIDENCE/$(next_seq)-wait-timeout.txt"
      printf '%s\n' "$scr" >"$f"
      log_action "wait $what '$text' FAILED after ${timeout}s -> $(basename "$f")"
      die "wait $what '$text' failed after ${timeout}s; screen saved to $f"
    fi
    sleep 0.2
  done
}

cmd_capture() {
  load_run "${1:-}"
  tm has-session -t "$SESSION" 2>/dev/null || die "no tmux session for this run: nothing to capture"
  local name base
  name="$(printf '%s' "${2:-screen}" | tr -c 'A-Za-z0-9._-' '-')"
  base="$EVIDENCE/$(next_seq)-$name"
  tm capture-pane -p -t "$SESSION" >"$base.txt"
  tm capture-pane -p -e -t "$SESSION" >"$base.ansi"
  log_action "capture $(basename "$base")"
  cat "$base.txt"
  echo "saved: $base.txt, $base.ansi" >&2
}

# The fake forge calls of this run: the lines of calls.log after the ones that
# were there at launch.
run_calls() {
  if [[ -f "$RUN_FORGE/calls.log" ]]; then tail -n "+$((${RUN_CALLS_FROM:-0} + 1))" "$RUN_FORGE/calls.log"; fi
}

cmd_calls() {
  load_run "${1:-}"
  [[ -n "$RUN_FORGE" ]] || die "calls: this run has no fake forge"
  run_calls
}

cmd_stop() {
  load_run "${1:-}"
  local i
  if tm has-session -t "$SESSION" 2>/dev/null; then
    if [[ "$(pane_field pane_dead)" == 0 ]]; then
      tm send-keys -t "$SESSION" q
      for ((i = 0; i < 15; i++)); do
        if [[ "$(pane_field pane_dead)" == 1 ]]; then break; fi
        sleep 0.2
      done
    fi
  fi
  end_server
  if [[ -n "$RUN_FORGE" ]]; then run_calls >"$EVIDENCE/forge-calls.log"; fi
  if [[ -f "$RUN_DIR/opened.log" ]]; then cp "$RUN_DIR/opened.log" "$EVIDENCE/opened.log"; fi
  log_action "stop"
  echo "evidence: $EVIDENCE" >&2
  ls -1 "$EVIDENCE"
}

# ---------- cli ----------

cmd_cli() {
  load_run "${1:-}"
  shift
  if [[ "${1:-}" == -- ]]; then shift; fi
  (($#)) || die "cli: give arguments after --"
  local a headless=0
  for a in "$@"; do
    case "$a" in
      --update | --update=* | --tui) die "cli: $a is refused (updater and TUI need the tmux run)" ;;
      --plain | --json | --help | -h | --version | -V) headless=1 ;;
    esac
  done
  ((headless)) || die "cli: pass --plain, --json, --help or --version (a bare run opens the TUI and waits for keys)"
  local slug base rc=0
  slug="$(printf '%s' "$*" | tr -c 'A-Za-z0-9' '-' | tr -s '-' | cut -c1-40)"
  base="$EVIDENCE/$(next_seq)-cli${slug%-}"
  {
    printf 'cd %q && source %q && %q' "$RUN_FIXTURE" "$RUN_DIR/env.sh" "$RUN_BIN"
    printf ' %q' "$@"
    echo
  } >"$base.cmd"
  (
    # shellcheck source=/dev/null
    source "$RUN_DIR/env.sh"
    cd "$RUN_FIXTURE"
    if command -v timeout >/dev/null 2>&1; then
      timeout 60 "$RUN_BIN" "$@"
    else
      "$RUN_BIN" "$@"
    fi
  ) </dev/null >"$base.out" 2>"$base.err" || rc=$?
  echo "$rc" >"$base.rc"
  log_action "cli $* -> rc $rc ($(basename "$base").*)"
  cat "$base.out"
  cat "$base.err" >&2
  echo "verify-tui: rc $rc, evidence $base.{cmd,out,err,rc}" >&2
  return "$rc"
}

# ---------- scenario ----------

# Copy $1 to $2 through a temporary name in the same directory, then rename, so
# the fake gh / glab never read half a file.
put_file() {
  mkdir -p "$(dirname "$2")"
  cp "$1" "$2.tmp.$$"
  mv -f "$2.tmp.$$" "$2"
}

# scenario [--reset] <forge-dir> <set>...: without --reset the sets are overlays
# on what is in the directory. --reset first clears the scenario files (gh/ and
# glab/) and keeps calls.log. Use it before launch, never as a mid-run swap: a
# call that lands in the gap would find no scenario.
cmd_scenario() {
  local reset=0
  if [[ "${1:-}" == --reset ]]; then
    reset=1
    shift
  fi
  local forge="${1:-}"
  if [[ -z "$forge" ]] || (($# < 2)); then
    die "scenario: give [--reset] <forge-dir> and at least one set (ls $SCENARIOS_DIR)"
  fi
  shift
  local set src f base s
  for set in "$@"; do
    [[ -d "$SCENARIOS_DIR/$set" ]] || die "unknown scenario set '$set' (ls $SCENARIOS_DIR)"
  done
  if ((reset)); then
    # Remove only scenario directories. In the directory of the fake gh and glab
    # scripts (scripts/fake-forge) these names are the executables: refuse there.
    local d
    for d in gh glab; do
      if [[ -e "$forge/$d" && ! -d "$forge/$d" ]]; then
        die "scenario --reset: $forge/$d is a file, not a scenario directory (is $forge the fake gh / glab scripts directory?); nothing was removed"
      fi
    done
    for d in gh glab; do
      if [[ -d "$forge/$d" ]]; then rm -rf "${forge:?}/$d"; fi
    done
  fi
  mkdir -p "$forge"
  for set in "$@"; do
    src="$SCENARIOS_DIR/$set"
    while IFS= read -r f; do
      f="${f#./}"
      case "$f" in
        *.json)
          # A set that does not ship the failure files clears the old ones.
          for s in exit stderr; do
            if [[ ! -e "$src/$f.$s" ]]; then rm -f "$forge/$f.$s"; fi
          done
          put_file "$src/$f" "$forge/$f"
          ;;
        *.json.exit | *.json.stderr)
          put_file "$src/$f" "$forge/$f"
          base="${f%.*}"
          if [[ ! -e "$src/$base" ]]; then rm -f "$forge/$base"; fi
          ;;
        *) put_file "$src/$f" "$forge/$f" ;;
      esac
    done < <(cd "$src" && find . -type f | sort)
  done
  echo "scenario: $* -> $forge" >&2
}

# ---------- main ----------

cmd="${1:-help}"
if (($#)); then shift; fi
case "$cmd" in
  build) cmd_build "$@" ;;
  launch) cmd_launch "$@" ;;
  keys) cmd_keys "$@" ;;
  mouse) cmd_mouse "$@" ;;
  wait) cmd_wait "$@" ;;
  capture) cmd_capture "$@" ;;
  stop) cmd_stop "$@" ;;
  cli) cmd_cli "$@" ;;
  scenario) cmd_scenario "$@" ;;
  calls) cmd_calls "$@" ;;
  help | -h | --help) usage ;;
  *)
    usage >&2
    die "unknown command '$cmd'"
    ;;
esac
