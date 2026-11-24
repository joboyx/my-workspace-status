#!/usr/bin/env bash
# Drive this checkout's own workspace-status in a private tmux server, on a
# disposable seeded workspace, and keep screen captures as proof.
#
# This is a test harness for the verify skill
# (.agents/skills/verify-workspace-status/SKILL.md; its controller wsctl calls
# this script for every spawn),
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
SCENARIOS_DIR="$REPO_ROOT/.agents/skills/verify-workspace-status/scenarios"
ROOT="${WS_VERIFY_ROOT:-${TMPDIR:-/tmp}/ws-verify}"
ROOT="${ROOT%/}"
SESSION=ws

# GitHub / GitLab origins the seeded repos get under --fake-forge. The scenario
# sets in .agents/skills/verify-workspace-status/scenarios/ are keyed to these
# paths.
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
  bin                            print the binary path that build makes (no build)
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
  mouse <run> wheel up|down <col> <row>
                                 send one SGR wheel report (button 64 up, 65 down)
                                 at a 1-based screen cell
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
                                 --version, or doctor as the first word) with the
                                 same isolation in the fixture
  scenario [--reset] <forge-dir> <set>...
                                 copy scenario sets into a fake forge directory;
                                 --reset clears the old ones first (before launch)
                                 (without --reset the sets overlay what is there, and
                                 each file is renamed in, so it is the mid-run swap)
  calls <run>                    print the gh / glab calls this run made (fake forge)
  update <entry> <outcome> | update all
                                 start the app through an update entry path in a
                                 run of its own, with a fake updater and a fake
                                 curl beside a copy of the binary; print one
                                 PASS / FAIL line per case; exit 1 on a FAIL.
                                 entry: cli (ws --update -a), prompt (ws -a,
                                 answer y), dev (ws-dev --update -a, dev build in
                                 target/verify-dev); outcome: install, current,
                                 fail

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

# The text of the startup update prompt, and the chrome of the TUI's first
# paint. launch fails on the prompt (isolation broke); update answers it.
UPDATE_PROMPT='Update? [y/N]'
first_paint() { [[ "$1" == *"? help"* ]]; }

# new_run [SUFFIX]: make a run dir under the root (id = UTC time, pid, SUFFIX)
# and set RUN_DIR, EVIDENCE and SOCK.
new_run() {
  local root_real run_id
  mkdir -p "$ROOT"
  root_real="$(real_dir "$ROOT")"
  run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$${1:+-$1}"
  RUN_DIR="$root_real/$run_id"
  RUN_ID="$run_id"
  # $BROWSER splits on spaces and colons, so the recorder path may hold neither.
  case "$RUN_DIR" in *[[:space:]:]*) die "run dir '$RUN_DIR' holds a space or colon; set WS_VERIFY_ROOT to a plain path" ;; esac
  EVIDENCE="$RUN_DIR/evidence"
  SOCK="ws-verify-$run_id"
  mkdir -p "$EVIDENCE" "$RUN_DIR/config" "$RUN_DIR/state" "$RUN_DIR/browser"
  : >"$RUN_DIR/run.env"
  # The recorder appends each URL the TUI opens to opened.log, one per line.
  cat >"$RUN_DIR/browser/open" <<EOF
#!/bin/sh
printf '%s\n' "\$1" >> $(printf '%q' "$RUN_DIR/opened.log")
EOF
  chmod +x "$RUN_DIR/browser/open"
}

# A fresh lastCheckUnix keeps the 6 h update prompt from blocking mount.
stamp_update_check() {
  printf '{\n  "version": 1,\n  "lastCheckUnix": %s\n}\n' "$(date +%s)" >"$RUN_DIR/state/update-check.json"
}

# write_launch FILE CWD RCFILE BIN [ARG]...: the pane's script: source the
# run's env.sh, cd to CWD, exec BIN with the ARGs. With an RCFILE, run BIN as
# a child instead, as a user's shell does, and write its exit code there:
# tmux can leave an exited pane process unreaped for seconds, so
# pane_dead_status is not there to read in time.
write_launch() {
  local file="$1" cwd="$2" rcfile="$3" bin="$4"
  shift 4
  {
    echo '#!/usr/bin/env bash'
    printf 'source %q\n' "$RUN_DIR/env.sh"
    printf 'cd %q\n' "$cwd"
    if [[ -n "$rcfile" ]]; then
      printf 'rcfile=%q\n' "$rcfile"
      printf '%q' "$bin"
      printf ' %q' "$@"
      echo
      cat <<'EOF'
rc=$?
echo "$rc" >"$rcfile"
exit "$rc"
EOF
    elif (($#)); then
      printf 'exec %q' "$bin"
      printf ' %q' "$@"
      echo
    else
      printf 'exec %q\n' "$bin"
    fi
  } >"$file"
  chmod +x "$file"
}

# record_run BIN FIXTURE FORGE SIZE: the run facts that load_run, stop and
# wsctl read back.
record_run() {
  record RUN_ID "$RUN_ID"
  record RUN_SOCK "$SOCK"
  record RUN_BIN "$1"
  record RUN_FIXTURE "$2"
  record RUN_FORGE "$3"
  # The checkout that made the run: wsctl refuses a run of another checkout.
  record RUN_REPO "$REPO_ROOT"
  # Runs share one forge dir and its calls.log. Remember where this run starts.
  local calls_from=0
  if [[ -n "$3" && -f "$3/calls.log" ]]; then calls_from="$(wc -l <"$3/calls.log" | tr -d ' ')"; fi
  record RUN_CALLS_FROM "$calls_from"
  record RUN_SIZE "$4"
  record RUN_HEAD "$(git -C "$REPO_ROOT" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  record RUN_VERSION "$("$1" --version </dev/null 2>&1 || echo unknown)"
}

# start_session COLS ROWS: start the run's private tmux server with launch.sh
# in its one pane; the pane stays after the process exits (its status stays).
start_session() {
  tmux -L "$SOCK" -f /dev/null start-server \; set-option -g status off \; \
    set-option -g remain-on-exit on \; \
    new-session -d -s "$SESSION" -x "$1" -y "$2" "$(printf '%q' "$RUN_DIR/launch.sh")"
  RUN_SOCKET_PATH="$(tm display-message -p '#{socket_path}')"
  record RUN_SOCKET_PATH "$RUN_SOCKET_PATH"
}

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

  new_run

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

  stamp_update_check
  write_env_file "$RUN_DIR/env.sh" "$forge" ${extra[@]+"${extra[@]}"}
  cp "$RUN_DIR/env.sh" "$EVIDENCE/env.sh.txt"
  write_launch "$RUN_DIR/launch.sh" "$fixture" "" "$bin"
  record_run "$bin" "$fixture" "$forge" "$size"
  start_session "$cols" "$rows"

  local i scr=""
  for ((i = 0; i < 80; i++)); do
    scr="$(screen_text || true)"
    if [[ "$scr" == *"$UPDATE_PROMPT"* ]]; then
      printf '%s\n' "$scr" >"$EVIDENCE/launch-failure.txt"
      end_server
      die "update prompt on screen (isolation failed); see $EVIDENCE/launch-failure.txt"
    fi
    if [[ "$(pane_field pane_dead)" == 1 ]]; then
      printf '%s\n' "$scr" >"$EVIDENCE/launch-failure.txt"
      end_server
      die "the TUI exited at launch; see $EVIDENCE/launch-failure.txt"
    fi
    if first_paint "$scr"; then break; fi
    sleep 0.25
  done
  if ! first_paint "$scr"; then
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
# count them; with the status line off, capture line N is screen row N. The
# run's tmux server keeps its mouse option off: send-keys writes the report
# into the pane, so it still reaches the app, as a terminal's report would.
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
    wheel)
      local dir="${3:-}" col="${4:-}" row="${5:-}" button
      case "$dir" in
        up) button=64 ;;
        down) button=65 ;;
        *) die "mouse wheel: give up|down <col> <row>, got '$dir'" ;;
      esac
      [[ "$col" =~ ^[1-9][0-9]*$ && "$row" =~ ^[1-9][0-9]*$ ]] ||
        die "mouse wheel: give <col> <row> as positive integers (1-based cells), got '$col' '$row'"
      # A wheel notch is one press report, with no release (64 up, 65 down).
      tm send-keys -t "$SESSION" -l -- $'\e'"[<$button;$col;${row}M"
      log_action "mouse wheel $dir $col $row"
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
    *) die "mouse: give move|click <col> <row>, wheel up|down <col> <row>, or find <text>" ;;
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

# save_capture NAME [OPTION]...: save the screen to evidence/NN-NAME.txt and
# .ansi (OPTIONs go to capture-pane); sets CAPTURE_BASE to the path without
# the extension.
save_capture() {
  local name
  name="$(printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '-')"
  shift
  CAPTURE_BASE="$EVIDENCE/$(next_seq)-$name"
  tm capture-pane -p "$@" -t "$SESSION" >"$CAPTURE_BASE.txt"
  tm capture-pane -p -e "$@" -t "$SESSION" >"$CAPTURE_BASE.ansi"
  log_action "capture $(basename "$CAPTURE_BASE")"
}

cmd_capture() {
  load_run "${1:-}"
  tm has-session -t "$SESSION" 2>/dev/null || die "no tmux session for this run: nothing to capture"
  save_capture "${2:-screen}"
  cat "$CAPTURE_BASE.txt"
  echo "saved: $CAPTURE_BASE.txt, $CAPTURE_BASE.ansi" >&2
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

# Quit a live TUI with q (up to 3 s), then end the run's tmux server.
quit_session() {
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
}

cmd_stop() {
  load_run "${1:-}"
  quit_session
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
  local a headless=0 skip=0 first=""
  for a in "$@"; do
    case "$a" in
      --update | --update=* | --tui) die "cli: $a is refused (updater and TUI need the tmux run)" ;;
      --plain | --json | --help | -h | --version | -V) headless=1 ;;
    esac
    # The first word that is not an option or the value of -C / --workspace:
    # `doctor` there is the subcommand, which prints and exits. A word before
    # it makes `doctor` a repo filter, and that run opens the TUI.
    if ((skip)); then
      skip=0
    elif [[ -z "$first" ]]; then
      case "$a" in
        -C | --workspace) skip=1 ;;
        --) first="$a" ;; # every word after -- is a repo filter
        -*) ;;
        *) first="$a" ;;
      esac
    fi
  done
  if [[ "$first" == doctor ]]; then headless=1; fi
  ((headless)) || die "cli: pass --plain, --json, --help or --version, or doctor as the first word (a bare run opens the TUI and waits for keys)"
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

# ---------- update ----------

# update <entry> <outcome> (or update all) starts the app through one of the
# three update entry paths, as a user does, in a run of its own, and prints one
# PASS / FAIL line. It is the one command that runs an updater, and only a fake
# one beside a copy of the binary (cli refuses --update):
#
#   entry    cli      ws --update -a
#            prompt   ws -a on a TTY with no update-check store; answers y Enter
#            dev      ws-dev --update -a, a dev-marked build (it never prompts)
#   outcome  install  the updater swaps the app for a "new version" that logs
#                     its args and cwd to update/new-app.log, then runs an
#                     untouched copy of the same binary
#            current  the updater prints "already up to date" and exits 0
#            fail     the updater prints "network error" to stderr and exits 1
UPDATE_ENTRIES=(cli prompt dev)
UPDATE_OUTCOMES=(install current fail)
# PATH after the run's bin/: system dirs only, so no installed ws, updater,
# ~/.local/bin or ~/.cargo/bin can answer.
UPDATE_SYSTEM_PATH=/usr/bin:/bin:/usr/sbin:/sbin
# The fixture cwd holds a space, so a relaunch must keep the cwd intact.
UPDATE_CWD="work/sub dir"
# Where the dev-marked build goes; never the checkout's target/debug.
DEV_TARGET="$REPO_ROOT/target/verify-dev"

# needs_build BIN: true when BIN is missing or a build input (sources,
# manifests, the lock file; not the integration tests) is newer than it.
needs_build() {
  [[ -x "$1" ]] || return 0
  local newer
  newer="$(find "$REPO_ROOT/Cargo.toml" "$REPO_ROOT/Cargo.lock" "$REPO_ROOT/crates" \
    -path "$REPO_ROOT/crates/*/tests" -prune -o -type f -newer "$1" -print 2>/dev/null | head -n 1 || true)"
  [[ -n "$newer" ]]
}

# update_source ENTRY: print the binary a case copies, built first when stale.
# dev: the dev-marked ws in target/verify-dev; else the checkout's ws. The
# caller runs it in $(...), where errexit is off, so each build has its own
# `|| die`: a failed build must not leave an older binary to test.
update_source() {
  local bin
  if [[ "$1" == dev ]]; then
    bin="$DEV_TARGET/debug/ws"
    if needs_build "$bin"; then
      echo "verify-tui: building the dev-marked ws into $DEV_TARGET (slow the first time)" >&2
      (cd "$REPO_ROOT" && WS_STATUS_DEV_BUILD=verify CARGO_TARGET_DIR="$DEV_TARGET" cargo build --locked -p workspace-status) >&2 ||
        die "cargo build failed for $bin"
    fi
  else
    bin="$(dirname "$(binary_path)")/ws"
    if needs_build "$bin"; then
      (cd "$REPO_ROOT" && cargo build -p workspace-status --bin ws) >&2 ||
        die "cargo build failed for $bin"
    fi
  fi
  [[ -x "$bin" ]] || die "build finished but $bin is missing"
  echo "$bin"
}

# update_fakes ENTRY OUTCOME SRC: bin/ gets a copy of SRC as the app, the fake
# updater for OUTCOME and a fake curl; next/ gets the untouched copy that the
# "new version" runs. Sets UPDATE_APP (ws, or ws-dev for dev).
update_fakes() {
  local outcome="$2" src="$3" updater="workspace-status-update"
  UPDATE_APP=ws
  if [[ "$1" == dev ]]; then
    UPDATE_APP=ws-dev
    updater="workspace-status-update-dev"
  fi
  mkdir -p "$RUN_DIR/bin" "$RUN_DIR/next" "$RUN_DIR/update"
  cp "$src" "$RUN_DIR/bin/$UPDATE_APP"
  cp "$src" "$RUN_DIR/next/$UPDATE_APP"
  {
    echo '#!/usr/bin/env bash'
    echo '# The "new version" of a verify update run: log the start, run the app.'
    printf 'log=%q\nnext=%q\n' "$RUN_DIR/update/new-app.log" "$RUN_DIR/next/$UPDATE_APP"
    cat <<'EOF'
printf 'args=[%s] cwd=%s\n' "$*" "$(pwd -P)" >>"$log"
exec "$next" "$@"
EOF
  } >"$RUN_DIR/update/new-version"
  {
    echo '#!/usr/bin/env bash'
    echo "# Fake updater of a verify update run ($outcome)."
    printf 'log=%q\n' "$RUN_DIR/update/updater.log"
    cat <<'EOF'
printf '%s %s\n' "$(basename "$0")" "$*" >>"$log"
EOF
    case "$outcome" in
      install)
        printf 'app=%q\nnew=%q\n' "$RUN_DIR/bin/$UPDATE_APP" "$RUN_DIR/update/new-version"
        cat <<'EOF'
# Replace the app as an installer does: a copy beside it, then a rename.
cp "$new" "$app.new.$$"
chmod +x "$app.new.$$"
mv -f "$app.new.$$" "$app"
echo 'FAKE-UPDATER installed'
EOF
        ;;
      current) echo "echo 'already up to date'" ;;
      fail) printf '%s\n' "echo 'network error' >&2" 'exit 1' ;;
    esac
  } >"$RUN_DIR/bin/$updater"
  {
    echo '#!/usr/bin/env bash'
    echo '# Fake curl of a verify update run: release v99.0.0 with one note.'
    printf 'log=%q\n' "$RUN_DIR/update/curl.log"
    cat <<'EOF'
printf '%s\n' "$*" >>"$log"
for a in "$@"; do
  case "$a" in
    */releases/latest*) printf '%s\n' '{"tag_name":"v99.0.0"}'; exit 0 ;;
    */releases\?*) printf '%s\n' '[{"tag_name":"v99.0.0","draft":false,"prerelease":false,"body":"## [99.0.0]\n\n### Features\n\n- release notes\n\n## Install workspace-status 99.0.0\n"}]'; exit 0 ;;
  esac
done
exit 22
EOF
  } >"$RUN_DIR/bin/curl"
  chmod +x "$RUN_DIR/update/new-version" "$RUN_DIR/bin/$updater" "$RUN_DIR/bin/curl"
}

# update_watch SECONDS UNTIL: poll the pane until it is dead, until the prompt
# is up (UNTIL=prompt) or until the first paint (UNTIL=paint); 1 on timeout.
# Sets UPDATE_PAINTED=1 once a sample shows the first paint.
update_watch() {
  local end=$((SECONDS + $1)) scr
  while :; do
    scr="$(screen_text || true)"
    if first_paint "$scr"; then UPDATE_PAINTED=1; fi
    if [[ "$2" == prompt && "$scr" == *"$UPDATE_PROMPT"* ]]; then return 0; fi
    if [[ "$2" == paint ]] && first_paint "$scr"; then return 0; fi
    if [[ "$(pane_field pane_dead)" == 1 ]]; then return 0; fi
    if ((SECONDS >= end)); then return 1; fi
    sleep 0.2
  done
}

# update_case ENTRY OUTCOME: one case; prints its line, returns 1 on FAIL.
update_case() {
  local entry="$1" outcome="$2" src name="update-$1-$2"
  src="$(update_source "$entry")"
  new_run "$name"
  local fixture="$RUN_DIR/$UPDATE_CWD"
  "$SCRIPT_DIR/seed-demo-workspace.sh" "$fixture" >"$EVIDENCE/seed.log" 2>&1 ||
    die "seed failed, see $EVIDENCE/seed.log"
  fixture="$(real_dir "$fixture")"
  update_fakes "$entry" "$outcome" "$src"
  # The prompt needs a due check, so its store stays absent. --update skips
  # the check; those entries keep the fresh store of every run.
  if [[ "$entry" != prompt ]]; then stamp_update_check; fi
  mkdir -p "$RUN_DIR/forge"
  write_env_file "$RUN_DIR/env.sh" "$RUN_DIR/forge" \
    "PATH=$RUN_DIR/bin:$SCRIPT_DIR/fake-forge:$UPDATE_SYSTEM_PATH"
  cp "$RUN_DIR/env.sh" "$EVIDENCE/env.sh.txt"
  local args=(--update -a)
  if [[ "$entry" == prompt ]]; then args=(-a); fi
  write_launch "$RUN_DIR/launch.sh" "$fixture" "$RUN_DIR/update/rc" "$RUN_DIR/bin/$UPDATE_APP" "${args[@]}"
  record_run "$RUN_DIR/bin/$UPDATE_APP" "$fixture" "$RUN_DIR/forge" 160x45
  record RUN_UPDATE_CASE "$entry $outcome"
  log_action "update $entry $outcome: $UPDATE_APP ${args[*]} in $fixture"
  start_session 160 45

  UPDATE_PAINTED=0
  local answered=n/a
  if [[ "$entry" == prompt ]]; then
    answered=no
    update_watch 20 prompt || true
    save_capture "$name-before"
    if grep -qF -- "$UPDATE_PROMPT" "$CAPTURE_BASE.txt" && [[ "$(pane_field pane_dead)" == 0 ]]; then
      tm send-keys -t "$SESSION" y Enter
      log_action "keys y Enter"
      answered=yes
    fi
  fi
  update_watch 30 paint || true
  local after scrollback
  save_capture "$name-after"
  after="$(cat "$CAPTURE_BASE.txt")"
  # The updater output is read with the history too: tmux writes its dead-pane
  # line at the bottom and scrolls, so a short output leaves the screen.
  save_capture "$name-history" -S -
  scrollback="$(cat "$CAPTURE_BASE.txt")"

  local rc=running log="$RUN_DIR/update/new-app.log" lines=0 first="" newapp=absent f
  if [[ "$(pane_field pane_dead)" == 1 ]]; then
    rc="$(cat "$RUN_DIR/update/rc" 2>/dev/null || true)"
    rc="${rc:-unknown}"
  fi
  echo "$rc" >"$EVIDENCE/$(next_seq)-$name-rc.txt"
  if [[ -f "$log" ]]; then
    lines="$(wc -l <"$log" | tr -d ' ')"
    first="$(head -n 1 "$log")"
    newapp="$lines line(s), first '$first'"
  fi
  for f in new-app curl updater; do
    if [[ -f "$RUN_DIR/update/$f.log" ]]; then cp "$RUN_DIR/update/$f.log" "$EVIDENCE/$(next_seq)-$name-$f.log.txt"; fi
  done

  local marker painted=no prompt_on=no out=no
  case "$outcome" in
    install) marker='FAKE-UPDATER installed' ;;
    current) marker='already up to date' ;;
    fail) marker='network error' ;;
  esac
  if first_paint "$after"; then painted=yes; fi
  if [[ "$after" == *"$UPDATE_PROMPT"* ]]; then prompt_on=yes; fi
  if [[ "$scrollback" == *"$marker"* ]]; then out=yes; fi

  local verdict=FAIL expected want="args=[-a] cwd=$fixture"
  case "$entry/$outcome" in
    */install)
      expected="new-app.log 1 line '$want'; first paint, running; no prompt on screen"
      if [[ "$lines" == 1 && "$first" == "$want" && "$painted" == yes && "$rc" == running &&
        "$prompt_on" == no && "$answered" != no ]]; then verdict=PASS; fi
      ;;
    prompt/*)
      expected="no new-app.log; prompt answered; first paint, running (same process)"
      if [[ ! -f "$log" && "$answered" == yes && "$painted" == yes && "$rc" == running ]]; then verdict=PASS; fi
      ;;
    *)
      local want_rc=0
      if [[ "$outcome" == fail ]]; then want_rc=1; fi
      expected="no new-app.log; exit $want_rc; never painted; '$marker' on screen"
      if [[ ! -f "$log" && "$rc" == "$want_rc" && "$UPDATE_PAINTED" == 0 && "$out" == yes ]]; then verdict=PASS; fi
      ;;
  esac
  local actual="new-app.log $newapp; rc $rc; painted ever $UPDATE_PAINTED, last screen $painted; prompt on last screen $prompt_on; '$marker' on screen $out"
  if [[ "$entry" == prompt ]]; then actual="prompt answered $answered; $actual"; fi

  quit_session
  local line="$verdict update $entry $outcome | expected: $expected | actual: $actual | evidence $EVIDENCE"
  echo "$line" >"$EVIDENCE/$(next_seq)-$name-result.txt"
  log_action "$line"
  echo "$line"
  [[ "$verdict" == PASS ]]
}

cmd_update() {
  command -v tmux >/dev/null 2>&1 || die "tmux is not installed"
  if [[ "${1:-}" == all && $# == 1 ]]; then
    # One driver process per case, so each case has its own run id and shell.
    local e o fails=0 total=0
    for e in "${UPDATE_ENTRIES[@]}"; do
      for o in "${UPDATE_OUTCOMES[@]}"; do
        total=$((total + 1))
        "$SCRIPT_DIR/verify-tui.sh" update "$e" "$o" || fails=$((fails + 1))
      done
    done
    echo "verify-tui: update all: $fails of $total cases FAIL" >&2
    ((fails == 0)) || exit 1
    return 0
  fi
  (($# == 2)) || die "update: give <entry> <outcome> (entry ${UPDATE_ENTRIES[*]}; outcome ${UPDATE_OUTCOMES[*]}) or all"
  case " ${UPDATE_ENTRIES[*]} " in *" $1 "*) ;; *) die "update: unknown entry '$1' (${UPDATE_ENTRIES[*]})" ;; esac
  case " ${UPDATE_OUTCOMES[*]} " in *" $2 "*) ;; *) die "update: unknown outcome '$2' (${UPDATE_OUTCOMES[*]})" ;; esac
  # Not `|| exit 1`: that list would turn errexit off inside the case.
  update_case "$1" "$2"
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
  bin) binary_path ;;
  launch) cmd_launch "$@" ;;
  keys) cmd_keys "$@" ;;
  mouse) cmd_mouse "$@" ;;
  wait) cmd_wait "$@" ;;
  capture) cmd_capture "$@" ;;
  stop) cmd_stop "$@" ;;
  cli) cmd_cli "$@" ;;
  scenario) cmd_scenario "$@" ;;
  calls) cmd_calls "$@" ;;
  update) cmd_update "$@" ;;
  help | -h | --help) usage ;;
  *)
    usage >&2
    die "unknown command '$cmd'"
    ;;
esac
