#!/usr/bin/env bash
# Capture README/demo TUI clips from the official seed workspace.
#
# Usage (repo root):
#   ./scripts/capture-demo-stills.sh [DEST]
#
# DEST is passed to scripts/seed-demo-workspace.sh (default: tmp/demo-workspace).
# Animated GIF outputs land in docs/images/. Each clip records the terminal
# window with ffmpeg x11grab while the hardcoded keys below play, then encodes
# a GIF (palettegen/paletteuse). PNG stills from the same recording land in
# docs/images/stills/: NN-name.png is the last frame, NN-name-mid.png the frame
# a clip marks with mark_mid. Do not drive the TUI by hand and do not invent a
# second pipeline.
#
# WS_STATUS_STILLS_DISPLAY picks the Xvfb display (default 99). Only the
# xfce4-terminal started on that display is stopped, so another capture on a
# different display keeps its terminal.
#
# Self-contained for a Cursor Cloud Agent Linux VM: installs MesloLGS NF,
# xvfb, xfce4-terminal, xdotool, and ffmpeg when missing. Fails loudly instead
# of writing ASCII/gray/static clips over good ones. Xvfb + dbus + Openbox come from
# scripts/with-desktop-session.sh (same session helper as desktop TTY e2e).
#
# Isolates XDG_STATE_HOME, WS_STATUS_UPDATE_CHECK_STORE,
# WS_STATUS_VIEWED_STORE, and WS_STATUS_COMMENT_STORE under
# tmp/demo-stills-stage/state so a TTY launch does not write the operator
# last-check, reviewed-mark, or comment files. Unsets WS_STATUS_WORKSPACE
# so the seeded demo dir is the workspace root.
set -euo pipefail
trap '' HUP

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=with-desktop-session.sh
source "$SCRIPT_DIR/with-desktop-session.sh"
DEST="${1:-"$REPO_ROOT/tmp/demo-workspace"}"
OUT_DIR="$REPO_ROOT/docs/images"
STILLS_DIR="$OUT_DIR/stills"
STAGE_DIR="$REPO_ROOT/tmp/demo-stills-stage"
STATE_DIR="$STAGE_DIR/state"
UPDATE_STORE="$STATE_DIR/update-check.json"
VIEWED_STORE="$STATE_DIR/viewed-files.json"
COMMENT_STORE="$STATE_DIR/comments.json"
FONT_DIR="${HOME}/.local/share/fonts/MesloLGS-NF"
BIN="$REPO_ROOT/target/release/workspace-status"
LAUNCHER="$STAGE_DIR/run-tui.sh"
TERM_PID=""
WID=""
WS_PID=""
REC_PID=""
REC_RAW=""
CLIP_T0=""
CLIP_MID=""

# Clip pacing (seconds) and limits. Text stays at native size: no downscale.
CLIP_FPS=10
START_HOLD=1.2
KEY_GAP=0.45
STEP_HOLD=0.9
END_HOLD=1.8
TYPE_DELAY_MS=140
MAX_GIF_BYTES=$((4 * 1024 * 1024))

# Hardcoded clips. Keys match docs/demo.md. Each clip is a fresh launch.
# 01 j j j j k (walk app rows from session.ts, end on auth.ts)
# 02 / merger Enter, Tab, j j, Enter (graph, then commit files)
# 03 u, s on session.ts (reseed after)
# 04 / auth Enter, n
# 05 j j j unrecorded, then Space, j, Space (clear viewed store after)
# 06 S, Esc, / merger Enter, Tab, j onto stash, D, n
# 07 . .
# 08 ?, Esc
# 09 Ctrl-k, compare, Backspace x7, checkout, Esc
# 10 k k k unrecorded (app checkout row), then b, login, Backspace x5, fix/banner, Esc
# 11 j j j unrecorded (auth.ts), then x, Enter, n
# 12 Tab, s, Tab, / zzz Enter, Esc
# 13 < < < (diff pane reaches split), >
# 14 / auth Enter, n, n

die() {
  echo "capture-demo-stills: $*" >&2
  exit 1
}

have() { command -v "$1" >/dev/null 2>&1; }

apt_install() {
  local missing=()
  local p
  for p in "$@"; do
    if ! dpkg -s "$p" >/dev/null 2>&1; then
      missing+=("$p")
    fi
  done
  if ((${#missing[@]})); then
    have sudo || die "need packages: ${missing[*]} (sudo not available)"
    sudo apt-get update -qq
    sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${missing[@]}"
  fi
}

install_font() {
  mkdir -p "$FONT_DIR"
  local base="https://github.com/romkatv/powerlevel10k-media/raw/master"
  local -A files=(
    ["MesloLGS NF Regular.ttf"]="MesloLGS-NF-Regular.ttf"
    ["MesloLGS NF Bold.ttf"]="MesloLGS-NF-Bold.ttf"
    ["MesloLGS NF Italic.ttf"]="MesloLGS-NF-Italic.ttf"
    ["MesloLGS NF Bold Italic.ttf"]="MesloLGS-NF-BoldItalic.ttf"
  )
  local src dest
  for src in "${!files[@]}"; do
    dest="$FONT_DIR/${files[$src]}"
    if [[ ! -s "$dest" ]]; then
      curl -fsSL "$base/${src// /%20}" -o "$dest"
    fi
  done
  fc-cache -f "$HOME/.local/share/fonts" >/dev/null
  if ! fc-list "MesloLGS NF" | grep -q "MesloLGS NF"; then
    die "MesloLGS NF not in fontconfig after install. Refusing ASCII fallback."
  fi
}

ensure_bin() {
  if [[ ! -x "$BIN" ]]; then
    (cd "$REPO_ROOT" && cargo build --release -p workspace-status)
  fi
  [[ -x "$BIN" ]] || die "missing $BIN"
}

write_helpers() {
  mkdir -p "$STAGE_DIR" "$STATE_DIR"
  # Fresh lastCheckUnix so the 6h GitHub Release prompt is not due.
  printf '{\n  "version": 1,\n  "lastCheckUnix": %s\n}\n' "$(date +%s)" >"$UPDATE_STORE"
  cat >"$LAUNCHER" <<EOF
#!/usr/bin/env bash
# Cloud Agent shells export NO_COLOR=1; a gray first frame means it leaked in.
unset NO_COLOR FORCE_COLOR WS_STATUS_GLYPHS CLICOLOR_FORCE WS_STATUS_WORKSPACE
export WS_STATUS_WATCH_MS=0
export WS_STATUS_FETCH_MS=0
export XDG_STATE_HOME=$(printf '%q' "$STATE_DIR")
export WS_STATUS_UPDATE_CHECK_STORE=$(printf '%q' "$UPDATE_STORE")
export WS_STATUS_VIEWED_STORE=$(printf '%q' "$VIEWED_STORE")
export WS_STATUS_COMMENT_STORE=$(printf '%q' "$COMMENT_STORE")
# Seed timestamps are Asia/Manila; pin TZ so clips match that clock.
export TZ=Asia/Manila
export TERM=xterm-256color
export COLORTERM=truecolor
export NO_AT_BRIDGE=1
export GTK_A11Y=none
exec $(printf '%q' "$BIN")
EOF
  chmod +x "$LAUNCHER"
}

# xfce4-terminal processes launch_tui started on this run's DISPLAY. A bare
# `pkill -x xfce4-terminal` would also stop a capture on another display.
own_terminal_pattern() {
  printf 'xfce4-terminal --disable-server --display=%s ' "${DISPLAY:-none}"
}

own_terminal_running() {
  pgrep -f -- "$(own_terminal_pattern)" >/dev/null 2>&1
}

kill_own_terminals() {
  pkill "${1:--TERM}" -f -- "$(own_terminal_pattern)" >/dev/null 2>&1 || true
}

cleanup() {
  if [[ -n "${REC_PID:-}" ]] && kill -0 "$REC_PID" 2>/dev/null; then
    kill -TERM "$REC_PID" 2>/dev/null || true
    wait "$REC_PID" 2>/dev/null || true
  fi
  if [[ -n "${TERM_PID:-}" ]] && kill -0 "$TERM_PID" 2>/dev/null; then
    kill "$TERM_PID" 2>/dev/null || true
  fi
  kill_own_terminals
  pkill -f "$BIN" >/dev/null 2>&1 || true
  if declare -F ws_desktop_session_stop >/dev/null; then
    ws_desktop_session_stop
  fi
}
trap cleanup EXIT

clear_viewed() {
  rm -f "$VIEWED_STORE" "$COMMENT_STORE"
}

window_id() {
  xdotool search --class xfce4-terminal 2>/dev/null | tail -1 || true
}

window_alive() {
  local wid="${1:-}"
  [[ -n "$wid" ]] || return 1
  xdotool getwindowgeometry "$wid" >/dev/null 2>&1
}

tui_pid() {
  pgrep -f "$BIN" 2>/dev/null | head -1 || true
}

# After workspace-status is up, resolve the visible 140-col terminal.
# Prefer a mapped window named WSDEMO with real geometry — pid search can
# return a tiny VTE child and then every grab retry fails the size check.
window_for_tui() {
  local pid="${1:-}"
  local wid best="" best_a=0 w h area
  [[ -n "$pid" ]] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  WIDTH=0 HEIGHT=0
  for wid in $(xdotool search --class xfce4-terminal 2>/dev/null); do
    window_alive "$wid" || continue
    WIDTH=0 HEIGHT=0
    eval "$(xdotool getwindowgeometry --shell "$wid" 2>/dev/null | grep -E '^(WIDTH|HEIGHT)=' || true)"
    w="${WIDTH:-0}"
    h="${HEIGHT:-0}"
    area=$((w * h))
    if [[ "$area" -gt "$best_a" ]]; then
      best="$wid"
      best_a="$area"
    fi
  done
  # 140x40 cells at 13pt is ~1400x880; reject leftover 1x1 shells.
  [[ "$best_a" -ge 200000 ]] || return 1
  echo "$best"
}

require_tty() {
  local pid="${1:-}"
  local tty
  [[ -n "$pid" ]] || die "workspace-status did not start (no TTY/font). Not writing clips."
  tty="$(readlink -f "/proc/$pid/fd/0" 2>/dev/null || true)"
  if [[ ! "$tty" =~ /dev/pts/ ]]; then
    die "workspace-status stdin is not a pty ($tty). Not writing ASCII/gray clips."
  fi
}

tty_size() {
  local pid="${1:-}"
  local tty
  tty="$(readlink -f "/proc/$pid/fd/0" 2>/dev/null || true)"
  [[ "$tty" =~ /dev/pts/ ]] || return 1
  stty -F "$tty" size 2>/dev/null || true
}

stop_tui() {
  # Do not send `q` here: X reuses window ids, so an in-flight key can quit the next TUI.
  if [[ -n "${TERM_PID:-}" ]] && kill -0 "$TERM_PID" 2>/dev/null; then
    kill "$TERM_PID" 2>/dev/null || true
  fi
  pkill -f "$BIN" >/dev/null 2>&1 || true
  kill_own_terminals
  WID=""
  TERM_PID=""
  WS_PID=""
  local i
  for i in $(seq 1 40); do
    if [[ -z "$(window_id)" ]] \
      && [[ -z "$(tui_pid)" ]] \
      && ! own_terminal_running; then
      sleep 0.35
      return 0
    fi
    sleep 0.1
  done
  kill_own_terminals -KILL
  pkill -9 -f "$BIN" >/dev/null 2>&1 || true
  sleep 0.35
}

launch_tui() {
  echo "capture-demo-stills: stop+launch" >&2
  stop_tui
  sleep 0.35
  unset NO_COLOR FORCE_COLOR WS_STATUS_GLYPHS CLICOLOR_FORCE WS_STATUS_WORKSPACE
  if [[ -n "${WS_STATUS_GLYPHS:-}" ]]; then
    die "WS_STATUS_GLYPHS is set; refusing ASCII clips while MesloLGS NF is installed."
  fi
  setsid xfce4-terminal --disable-server \
    --display="$DISPLAY" \
    --geometry=140x40+24+24 \
    --hide-menubar --hide-toolbar --hide-scrollbar --hide-borders \
    --dynamic-title-mode=none \
    --font='MesloLGS NF 13' \
    --color-bg='#1a1b26' --color-text='#c0caf5' \
    --working-directory="$DEST" \
    -T WSDEMO \
    -e "$LAUNCHER" &
  TERM_PID=$!
  disown "$TERM_PID" 2>/dev/null || true
  echo "capture-demo-stills: terminal pid=$TERM_PID" >&2
  local i
  for i in $(seq 1 80); do
    WS_PID="$(tui_pid)"
    [[ -n "$WS_PID" ]] && break
    sleep 0.1
  done
  require_tty "$WS_PID"
  WID=""
  for i in $(seq 1 80); do
    WID="$(window_for_tui "$WS_PID" || true)"
    window_alive "$WID" && break
    sleep 0.1
  done
  window_alive "$WID" || die "TUI window never appeared for pid=$WS_PID (class=$(xdotool search --class xfce4-terminal 2>/dev/null | tr '\n' ' ')). Not writing clips."
  echo "capture-demo-stills: wid=$WID" >&2
  local rows=0 cols=0
  for i in $(seq 1 30); do
    read -r rows cols <<<"$(tty_size "$WS_PID")"
    if [[ "${cols:-0}" -ge 140 && "${rows:-0}" -ge 40 ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ "${cols:-0}" -lt 140 || "${rows:-0}" -lt 40 ]]; then
    die "TTY is ${cols:-?}x${rows:-?} (need at least 140x40). Not writing clips."
  fi
  echo "capture-demo-stills: tty=${cols}x${rows} pid=$WS_PID" >&2
  sleep 1.1
  if ! window_alive "$WID"; then
    WID="$(window_for_tui "$WS_PID" || true)"
  fi
  window_alive "$WID" || die "window vanished after launch (tui pid=$WS_PID still=$(kill -0 "$WS_PID" 2>/dev/null && echo yes || echo no))"
  xdotool windowfocus --sync "$WID" >/dev/null 2>&1 || true
  xdotool windowactivate --sync "$WID" >/dev/null 2>&1 || true
  sleep 0.2
  echo "capture-demo-stills: focused" >&2
}

refresh_wid() {
  if window_alive "$WID"; then
    return 0
  fi
  WID="$(window_for_tui "$WS_PID" || true)"
  window_alive "$WID" || die "TUI window gone (tui pid=${WS_PID:-empty} still=$(kill -0 "${WS_PID:-0}" 2>/dev/null && echo yes || echo no))"
}

# Args: xdotool key names, or type:TEXT. Paced so a viewer can follow each
# key in the clip; STEP_HOLD after the step lets the frame settle on screen.
send() {
  local tok
  refresh_wid
  xdotool windowfocus --sync "$WID" >/dev/null 2>&1 || true
  for tok in "$@"; do
    if [[ "$tok" == type:* ]]; then
      xdotool type --window "$WID" --delay "$TYPE_DELAY_MS" "${tok#type:}" \
        || die "xdotool type failed (wid=$WID tok=$tok)"
    else
      xdotool key --window "$WID" "$tok" \
        || die "xdotool key failed (wid=$WID tok=$tok)"
    fi
    sleep "$KEY_GAP"
  done
  sleep "$STEP_HOLD"
}

hold() {
  sleep "$1"
}

# Record the terminal window only (absolute geometry from xwininfo), never
# the whole desktop. Lossless mkv first; the GIF is encoded after stop.
clip_start() {
  local name="$1"
  local info ax ay w h
  REC_RAW="$STAGE_DIR/${name}.mkv"
  rm -f "$REC_RAW"
  refresh_wid
  xdotool windowfocus --sync "$WID" >/dev/null 2>&1 || true
  xdotool windowactivate --sync "$WID" >/dev/null 2>&1 || true
  sleep 0.2
  for _ in $(seq 1 12); do
    info="$(xwininfo -id "$WID" 2>/dev/null || true)"
    ax="$(awk -F: '/Absolute upper-left X/ {gsub(/ /,"",$2); print $2}' <<<"$info")"
    ay="$(awk -F: '/Absolute upper-left Y/ {gsub(/ /,"",$2); print $2}' <<<"$info")"
    w="$(awk '/^  Width:/ {print $2}' <<<"$info")"
    h="$(awk '/^  Height:/ {print $2}' <<<"$info")"
    if [[ -n "$ax" && -n "$ay" && -n "$w" && -n "$h" && "$w" -ge 800 && "$h" -ge 400 ]]; then
      break
    fi
    w=""
    sleep 0.25
    refresh_wid
  done
  [[ -n "$w" ]] || die "no terminal geometry for $WID (not recording the desktop)"
  CLIP_T0="$(date +%s.%N)"
  CLIP_MID=""
  ffmpeg -hide_banner -loglevel error -nostdin -y \
    -f x11grab -draw_mouse 0 -framerate "$CLIP_FPS" \
    -video_size "${w}x${h}" -i "${DISPLAY}+${ax},${ay}" \
    -c:v ffv1 "$REC_RAW" &
  REC_PID=$!
  sleep 0.3
  kill -0 "$REC_PID" 2>/dev/null || die "ffmpeg x11grab did not start for $name"
  hold "$START_HOLD"
}

# Mark the frame the last step settled on as this clip's mid still. Call it
# right after a send: the offset lands half a STEP_HOLD before now, inside the
# settled window and clear of ffmpeg's start-up delay.
mark_mid() {
  CLIP_MID="$(awk -v now="$(date +%s.%N)" -v t0="$CLIP_T0" -v hold="$STEP_HOLD" \
    'BEGIN { printf "%.2f", now - t0 - hold / 2 }')"
}

clip_stop() {
  hold "$END_HOLD"
  [[ -n "${REC_PID:-}" ]] || die "clip_stop without a recording"
  kill -0 "$REC_PID" 2>/dev/null || die "ffmpeg exited before $REC_RAW was complete"
  # SIGTERM lets ffmpeg flush and close the mkv.
  kill -TERM "$REC_PID" 2>/dev/null || true
  wait "$REC_PID" 2>/dev/null || true
  REC_PID=""
  [[ -s "$REC_RAW" ]] || die "ffmpeg wrote no frames to $REC_RAW"
}

encode_gif() {
  local raw="$1"
  local gif="$2"
  ffmpeg -hide_banner -loglevel error -nostdin -y -i "$raw" \
    -vf "fps=${CLIP_FPS},split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
    -loop 0 "$gif" \
    || die "GIF encode failed for $raw"
}

# Save the last frame of a GIF as PNG. Prints "FRAMES SECONDS".
# Exit 4 when no frame differs from the first (the keys did nothing). A clip
# may end where it started (u then s), so first == last alone is not a fail.
clip_frames() {
  python3 - "$@" <<'PY'
import sys
from PIL import Image, ImageSequence
gif, last_png = sys.argv[1:3]
im = Image.open(gif)
frames = 0
ms = 0
first_bytes = None
last = None
changed = False
for frame in ImageSequence.Iterator(im):
    frames += 1
    ms += frame.info.get("duration", 0)
    rgb = frame.convert("RGB")
    if first_bytes is None:
        first_bytes = rgb.tobytes()
    elif not changed and rgb.tobytes() != first_bytes:
        changed = True
    last = rgb
last = last.copy()
last.save(last_png, optimize=True)
print(frames, f"{ms / 1000:.1f}")
if not changed:
    sys.exit(4)
PY
}

not_gray() {
  python3 - "$1" <<'PY'
import sys
from PIL import Image
path = sys.argv[1]
im = Image.open(path).convert("RGB")
w, h = im.size
if w < 400 or h < 200:
    sys.stderr.write(f"capture-demo-stills: {path} too small ({w}x{h})\n")
    sys.exit(2)
px = list(im.getdata())
n = len(px)
if n == 0:
    sys.exit(2)
sr = sg = sb = 0
for r, g, b in px:
    sr += r
    sg += g
    sb += b
avg_r, avg_g, avg_b = sr / n, sg / n, sb / n
chan_spread = max(avg_r, avg_g, avg_b) - min(avg_r, avg_g, avg_b)
var = sum((r - avg_r) ** 2 + (g - avg_g) ** 2 + (b - avg_b) ** 2 for r, g, b in px) / n
if chan_spread < 4 and var < 80:
    sys.stderr.write(
        f"capture-demo-stills: {path} looks gray/NO_COLOR "
        f"(avg=({avg_r:.1f},{avg_g:.1f},{avg_b:.1f}) var={var:.1f}). Refusing overwrite.\n"
    )
    sys.exit(3)
PY
}

# Frame at SECONDS of a recording, as PNG (full colour, not the GIF palette).
extract_frame() {
  local raw="$1" seconds="$2" png="$3"
  ffmpeg -hide_banner -loglevel error -nostdin -y -ss "$seconds" -i "$raw" \
    -frames:v 1 "$png" \
    || die "frame extract failed for $raw at ${seconds}s"
  [[ -s "$png" ]] || die "no frame at ${seconds}s in $raw"
}

# Lossless PNG optimize when the host has optipng. Not required.
optimize_png() {
  if have optipng; then
    optipng -quiet -o2 "$1" >/dev/null 2>&1 || true
  fi
}

# Stop the recording, encode the GIF under STAGE_DIR, gate it and its stills,
# then copy the GIF to docs/images and the stills to docs/images/stills. A
# rejected clip leaves the existing GIF and stills in place.
clip_commit() {
  local name="$1"
  local staged="$STAGE_DIR/${name}.gif"
  local final="$OUT_DIR/${name}.gif"
  local last="$STAGE_DIR/${name}-last.png"
  local mid="$STAGE_DIR/${name}-mid.png"
  local info bytes rc
  clip_stop
  encode_gif "$REC_RAW" "$staged"
  rc=0
  info="$(clip_frames "$staged" "$last")" || rc=$?
  if ((rc == 4)); then
    die "rejecting $final: every frame matches the first (keys did nothing). Existing clip left in place."
  elif ((rc != 0)); then
    die "rejecting $final: frame check failed (exit $rc). Existing clip left in place."
  fi
  if ! not_gray "$last"; then
    die "rejecting $final (gray/tiny last frame). Existing clip left in place."
  fi
  if [[ -n "$CLIP_MID" ]]; then
    extract_frame "$REC_RAW" "$CLIP_MID" "$mid"
    if ! not_gray "$mid"; then
      die "rejecting $final (gray/tiny mid frame at ${CLIP_MID}s). Existing clip left in place."
    fi
  fi
  bytes="$(stat -c %s "$staged")"
  if ((bytes > MAX_GIF_BYTES)); then
    die "rejecting $final: $((bytes / 1024)) KiB is over $((MAX_GIF_BYTES / 1024)) KiB. Shorten the clip."
  fi
  mkdir -p "$OUT_DIR" "$STILLS_DIR"
  cp -f "$staged" "$final"
  cp -f "$last" "$STILLS_DIR/${name}.png"
  optimize_png "$STILLS_DIR/${name}.png"
  echo "ok $final ($((bytes / 1024)) KiB, frames/seconds: $info)"
  echo "ok $STILLS_DIR/${name}.png (last frame)"
  if [[ -n "$CLIP_MID" ]]; then
    cp -f "$mid" "$STILLS_DIR/${name}-mid.png"
    optimize_png "$STILLS_DIR/${name}-mid.png"
    echo "ok $STILLS_DIR/${name}-mid.png (${CLIP_MID}s)"
  fi
}

seed() {
  "$REPO_ROOT/scripts/seed-demo-workspace.sh" "$DEST"
  clear_viewed
}

cd "$REPO_ROOT"
# Cloud Agent / CI shells often export these; they paint a gray first frame.
unset NO_COLOR FORCE_COLOR WS_STATUS_GLYPHS CLICOLOR_FORCE WS_STATUS_WORKSPACE
export NO_AT_BRIDGE=1
export GTK_A11Y=none
export TZ=Asia/Manila

apt_install xvfb xfce4-terminal xdotool ffmpeg python3-pil x11-apps x11-utils x11-xserver-utils curl fontconfig dbus-x11 openbox
install_font
ensure_bin
rm -rf "$STAGE_DIR"
write_helpers

ws_desktop_session_start --display "${WS_STATUS_STILLS_DISPLAY:-99}"
seed

# 01 tree + live diff: walk the app rows from session.ts, end on auth.ts.
launch_tui
clip_start 01-tree-diff
send j
send j
send j
send j
send k
clip_commit 01-tree-diff

# 02 git graph: pass the stash, then drill into a commit.
launch_tui
clip_start 02-git-graph
send slash type:merger Return
send Tab
send j
send j
send Return
clip_commit 02-git-graph

# 03 unstage / stage session.ts (real git writes).
launch_tui
clip_start 03-stage-unstage
send u
mark_mid
hold 0.6
send s
clip_commit 03-stage-unstage
seed

# 04 search: type the query, arm it, step to the next match.
launch_tui
clip_start 04-search
send slash type:auth
send Return
send n
clip_commit 04-search

# 05 reviewed marks: move to auth.ts before recording, then mark two files.
launch_tui
send j j j
clip_start 05-reviewed
send space
send j
send space
clip_commit 05-reviewed
clear_viewed

# 06 stash: create-only menu from the tree, then the graph drop confirm.
launch_tui
clip_start 06-stash
send shift+s
send Escape
send slash type:merger Return
send Tab
send j
send shift+d
mark_mid
send n
clip_commit 06-stash

# 07 show / hide ignored repos.
launch_tui
clip_start 07-show-ignored
send period
mark_mid
hold 0.6
send period
clip_commit 07-show-ignored

# 08 help overlay.
launch_tui
clip_start 08-help
send shift+slash
hold 1.2
mark_mid
send Escape
clip_commit 08-help

# 09 command palette: an alias match, then letters that used to move the cursor.
launch_tui
clip_start 09-palette
send ctrl+k
send type:compare
mark_mid
send BackSpace BackSpace BackSpace BackSpace BackSpace BackSpace BackSpace
send type:checkout
send Escape
clip_commit 09-palette

# 10 branch picker on the app checkout: filter, then the explicit create row.
# Esc closes it; nothing is created or checked out.
launch_tui
send k k k
clip_start 10-branch-picker
send b
send type:login
send BackSpace BackSpace BackSpace BackSpace BackSpace
send type:fix/banner
mark_mid
send Escape
clip_commit 10-branch-picker

# 11 revert confirm on auth.ts: Enter does not confirm, n cancels. Never y.
launch_tui
send j j j
clip_start 11-confirm
send x
send Return
mark_mid
send n
clip_commit 11-confirm

# 12 keys that do nothing say why: s from the diff pane, a search with no match.
launch_tui
clip_start 12-blocked-key
send Tab
send s
mark_mid
send Tab
send slash type:zzz
send Return
send Escape
clip_commit 12-blocked-key

# 13 keyboard pane resize: the diff pane reaches 100 columns and turns split.
launch_tui
clip_start 13-split-resize
send "type:<"
send "type:<"
send "type:<"
mark_mid
send "type:>"
clip_commit 13-split-resize

# 14 search position chip: /auth 1/2, 2/2, then wrap to 1/2.
launch_tui
clip_start 14-search-count
send slash type:auth
send Return
send n
mark_mid
send n
clip_commit 14-search-count

stop_tui
echo "capture-demo-stills: wrote clips under $OUT_DIR"
