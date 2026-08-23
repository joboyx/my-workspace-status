#!/usr/bin/env bash
# Capture README/demo TUI stills from the official seed workspace.
#
# Usage (repo root):
#   ./scripts/capture-demo-stills.sh [DEST]
#
# DEST is passed to scripts/seed-demo-workspace.sh (default: tmp/demo-workspace).
# PNG outputs land in docs/images/. Key sequences are hardcoded below — do not
# drive the TUI by hand and do not invent a second pipeline.
#
# Self-contained for a Cursor Cloud Agent Linux VM: installs MesloLGS NF,
# xvfb, xfce4-terminal, and grab tools when missing. Fails loudly instead of
# writing ASCII/gray frames over good stills.
set -euo pipefail
trap '' HUP

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEST="${1:-"$REPO_ROOT/tmp/demo-workspace"}"
OUT_DIR="$REPO_ROOT/docs/images"
STAGE_DIR="$REPO_ROOT/tmp/demo-stills-stage"
FONT_DIR="${HOME}/.local/share/fonts/MesloLGS-NF"
BIN="$REPO_ROOT/target/release/workspace-status"
LAUNCHER="$STAGE_DIR/run-tui.sh"
OPENBOX_RC="$STAGE_DIR/openbox.xml"
DISPLAY_NUM="${WS_STATUS_STILLS_DISPLAY:-99}"
XVFB_PID=""
TERM_PID=""
WID=""
WS_PID=""
declare -A STILL_HASHES=()

# Hardcoded stills. Keys match docs/demo.md. Do not add extra k/n.
# 01 fresh launch (cursor on auth.ts)
# 02 / merger Enter
# 03 ?
# 04 / auth Enter
# 05 / merger Enter, Tab, j onto stash, D
# 06 S on dirty app
# 07 Space on auth.ts (reseed after)
# 08 .
# 09 / merger Enter, Tab, j to a commit, Enter
# 10 ; on tree

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
  mkdir -p "$STAGE_DIR"
  cat >"$LAUNCHER" <<EOF
#!/usr/bin/env bash
# Cloud Agent shells export NO_COLOR=1; a gray first frame means it leaked in.
unset NO_COLOR FORCE_COLOR WS_STATUS_GLYPHS CLICOLOR_FORCE
export WS_STATUS_WATCH_MS=0
export WS_STATUS_FETCH_MS=0
export TERM=xterm-256color
export COLORTERM=truecolor
exec $(printf '%q' "$BIN")
EOF
  chmod +x "$LAUNCHER"

  cat >"$OPENBOX_RC" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<openbox_config xmlns="http://openbox.org/3.4/rc">
  <theme>
    <titleLayout></titleLayout>
    <keepBorder>no</keepBorder>
  </theme>
  <applications>
    <application class="*">
      <decor>no</decor>
    </application>
  </applications>
</openbox_config>
EOF
}

start_xvfb() {
  have Xvfb || die "Xvfb missing after apt install"
  if [[ -S "/tmp/.X11-unix/X$DISPLAY_NUM" ]]; then
    pkill -f "Xvfb :$DISPLAY_NUM" >/dev/null 2>&1 || true
    sleep 0.2
  fi
  Xvfb ":$DISPLAY_NUM" -screen 0 1600x1000x24 -nolisten tcp >/tmp/ws-stills-xvfb.log 2>&1 &
  XVFB_PID=$!
  export DISPLAY=":$DISPLAY_NUM"
  local i
  for i in $(seq 1 50); do
    [[ -S "/tmp/.X11-unix/X$DISPLAY_NUM" ]] && return 0
    sleep 0.1
  done
  die "Xvfb :$DISPLAY_NUM did not start (see /tmp/ws-stills-xvfb.log)"
}

start_wm() {
  if have openbox; then
    setsid openbox --config-file "$OPENBOX_RC" >/tmp/ws-stills-openbox.log 2>&1 &
    sleep 0.4
  fi
}

cleanup() {
  if [[ -n "${TERM_PID:-}" ]] && kill -0 "$TERM_PID" 2>/dev/null; then
    kill "$TERM_PID" 2>/dev/null || true
  fi
  pkill -x xfce4-terminal >/dev/null 2>&1 || true
  pkill -f '/target/release/workspace-status' >/dev/null 2>&1 || true
  if [[ -n "${XVFB_PID:-}" ]] && kill -0 "$XVFB_PID" 2>/dev/null; then
    kill "$XVFB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

clear_viewed() {
  rm -f \
    "${XDG_STATE_HOME:-$HOME/.local/state}/my-workspace-status/viewed-files.json" \
    "$HOME/.local/state/my-workspace-status/viewed-files.json"
}

window_id() {
  xdotool search --name WSDEMO 2>/dev/null | tail -1 || true
}

window_alive() {
  local wid="${1:-}"
  [[ -n "$wid" ]] || return 1
  xdotool getwindowgeometry "$wid" >/dev/null 2>&1
}

wait_window() {
  local i wid
  for i in $(seq 1 80); do
    wid="$(window_id)"
    if window_alive "$wid"; then
      echo "$wid"
      return 0
    fi
    sleep 0.1
  done
  die "TUI window never appeared (no TTY/X). Not writing stills."
}

tui_pid() {
  pgrep -f '/target/release/workspace-status' 2>/dev/null | head -1 || true
}

require_tty() {
  local pid="${1:-}"
  local tty
  [[ -n "$pid" ]] || die "workspace-status did not start (no TTY/font). Not writing stills."
  tty="$(readlink -f "/proc/$pid/fd/0" 2>/dev/null || true)"
  if [[ ! "$tty" =~ /dev/pts/ ]]; then
    die "workspace-status stdin is not a pty ($tty). Not writing ASCII/gray stills."
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
  local wid
  wid="${WID:-$(window_id)}"
  if window_alive "$wid"; then
    xdotool windowfocus --sync "$wid" >/dev/null 2>&1 || true
    xdotool key --window "$wid" --delay 40 q >/dev/null 2>&1 || true
    sleep 0.2
  fi
  pkill -f '/target/release/workspace-status' >/dev/null 2>&1 || true
  pkill -x xfce4-terminal >/dev/null 2>&1 || true
  WID=""
  TERM_PID=""
  WS_PID=""
  local i
  for i in $(seq 1 40); do
    if [[ -z "$(window_id)" ]] \
      && [[ -z "$(tui_pid)" ]] \
      && ! pgrep -x xfce4-terminal >/dev/null 2>&1; then
      sleep 0.15
      return 0
    fi
    sleep 0.1
  done
  pkill -9 -x xfce4-terminal >/dev/null 2>&1 || true
  pkill -9 -f '/target/release/workspace-status' >/dev/null 2>&1 || true
  sleep 0.2
}

launch_tui() {
  echo "capture-demo-stills: stop+launch" >&2
  stop_tui
  unset NO_COLOR FORCE_COLOR WS_STATUS_GLYPHS CLICOLOR_FORCE
  if [[ -n "${WS_STATUS_GLYPHS:-}" ]]; then
    die "WS_STATUS_GLYPHS is set; refusing ASCII stills while MesloLGS NF is installed."
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
  WID="$(wait_window)"
  echo "capture-demo-stills: wid=$WID" >&2
  local i
  for i in $(seq 1 50); do
    WS_PID="$(tui_pid)"
    [[ -n "$WS_PID" ]] && break
    sleep 0.1
  done
  require_tty "$WS_PID"
  local rows=0 cols=0
  for i in $(seq 1 30); do
    read -r rows cols <<<"$(tty_size "$WS_PID")"
    if [[ "${cols:-0}" -ge 140 && "${rows:-0}" -ge 40 ]]; then
      break
    fi
    sleep 0.1
  done
  if [[ "${cols:-0}" -lt 140 || "${rows:-0}" -lt 40 ]]; then
    die "TTY is ${cols:-?}x${rows:-?} (need at least 140x40). Not writing stills."
  fi
  echo "capture-demo-stills: tty=${cols}x${rows} pid=$WS_PID" >&2
  sleep 1.1
  window_alive "$WID" || die "window vanished after launch (wid=$WID)"
  xdotool windowfocus --sync "$WID" >/dev/null 2>&1 || true
  xdotool windowactivate --sync "$WID" >/dev/null 2>&1 || true
  sleep 0.2
  echo "capture-demo-stills: focused" >&2
}

# Args: xdotool key names, or type:TEXT
send() {
  local tok
  window_alive "$WID" || die "window gone while sending keys (wid=${WID:-empty})"
  xdotool windowfocus --sync "$WID" >/dev/null 2>&1 || true
  for tok in "$@"; do
    if [[ "$tok" == type:* ]]; then
      sleep 0.12
      xdotool type --window "$WID" --delay 50 "${tok#type:}" \
        || die "xdotool type failed (wid=$WID tok=$tok)"
    else
      xdotool key --window "$WID" --delay 80 "$tok" \
        || die "xdotool key failed (wid=$WID tok=$tok)"
    fi
    sleep 0.18
  done
  sleep 0.45
}

grab() {
  local dest="$1"
  window_alive "$WID" || die "window gone while grabbing $dest (wid=${WID:-empty})"
  xdotool windowfocus --sync "$WID" >/dev/null 2>&1 || true
  rm -f "$dest"
  if have import; then
    import -window "$WID" "$dest"
  elif have xwd && have convert; then
    xwd -silent -id "$WID" | convert xwd:- "$dest"
  else
    die "need ImageMagick import or xwd+convert to grab the terminal window"
  fi
  [[ -s "$dest" ]] || die "empty grab: $dest"
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

commit_still() {
  local staged="$1"
  local final="$2"
  local name digest other
  name="$(basename "$final")"
  if ! not_gray "$staged"; then
    die "rejecting $final (gray/tiny). Existing still left in place if any."
  fi
  digest="$(md5sum "$staged" | awk '{print $1}')"
  for other in "${!STILL_HASHES[@]}"; do
    if [[ "${STILL_HASHES[$other]}" == "$digest" ]]; then
      die "rejecting $final: identical pixmap to $other (keys/window grab failed). Existing still left in place."
    fi
  done
  STILL_HASHES["$name"]="$digest"
  mkdir -p "$(dirname "$final")"
  cp -f "$staged" "$final"
  echo "ok $final ($digest)"
}

seed() {
  "$REPO_ROOT/scripts/seed-demo-workspace.sh" "$DEST"
  clear_viewed
}

cd "$REPO_ROOT"
# Cloud Agent / CI shells often export these; they paint a gray first frame.
unset NO_COLOR FORCE_COLOR WS_STATUS_GLYPHS CLICOLOR_FORCE

apt_install xvfb xfce4-terminal xdotool imagemagick python3-pil x11-apps x11-xserver-utils curl fontconfig dbus-x11 openbox
install_font
ensure_bin
rm -rf "$STAGE_DIR"
write_helpers

if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]] && have dbus-launch; then
  eval "$(dbus-launch --sh-syntax)"
fi

start_xvfb
start_wm
seed

launch_tui
grab "$STAGE_DIR/01-file-diff.png"
commit_still "$STAGE_DIR/01-file-diff.png" "$OUT_DIR/01-file-diff.png"

launch_tui
send slash type:merger Return
grab "$STAGE_DIR/02-git-graph.png"
commit_still "$STAGE_DIR/02-git-graph.png" "$OUT_DIR/02-git-graph.png"

launch_tui
send shift+slash
grab "$STAGE_DIR/03-help.png"
commit_still "$STAGE_DIR/03-help.png" "$OUT_DIR/03-help.png"

launch_tui
send slash type:auth Return
grab "$STAGE_DIR/04-search.png"
commit_still "$STAGE_DIR/04-search.png" "$OUT_DIR/04-search.png"

launch_tui
send slash type:merger Return Tab j shift+d
grab "$STAGE_DIR/05-confirm.png"
commit_still "$STAGE_DIR/05-confirm.png" "$OUT_DIR/05-confirm.png"

launch_tui
send shift+s
grab "$STAGE_DIR/06-stash-menu.png"
commit_still "$STAGE_DIR/06-stash-menu.png" "$OUT_DIR/06-stash-menu.png"

launch_tui
send space
grab "$STAGE_DIR/07-reviewed.png"
commit_still "$STAGE_DIR/07-reviewed.png" "$OUT_DIR/07-reviewed.png"

seed

launch_tui
send period
grab "$STAGE_DIR/08-show-ignored.png"
commit_still "$STAGE_DIR/08-show-ignored.png" "$OUT_DIR/08-show-ignored.png"

launch_tui
send slash type:merger Return Tab j j Return
grab "$STAGE_DIR/09-commit-files.png"
commit_still "$STAGE_DIR/09-commit-files.png" "$OUT_DIR/09-commit-files.png"

launch_tui
send semicolon
grab "$STAGE_DIR/10-easymotion.png"
commit_still "$STAGE_DIR/10-easymotion.png" "$OUT_DIR/10-easymotion.png"

stop_tui
echo "capture-demo-stills: wrote stills under $OUT_DIR"
