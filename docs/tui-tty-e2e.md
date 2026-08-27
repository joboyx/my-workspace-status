# Real-TTY TUI e2e

Headless TestBackend coverage stays in `crates/workspace-status/tests/tui_daily_e2e.rs`. Screenshot stills stay in `scripts/capture-demo-stills.sh`. This harness is neither.

It drives the real `workspace-status` binary the way a person does: a PTY (and, on Linux, a real terminal emulator). Assertions read the painted screen. It does not construct crossterm `Event` values in memory.

## What runs where

| Path | How | Where it executes |
| --- | --- | --- |
| PTY | `portable-pty` spawn + byte writes (keys, xterm SGR mouse) + `vt100` screen | `cargo test --workspace` on Unix. GitHub Actions `cargo` job. |
| Desktop | xfce4-terminal keys; xterm + XTEST `click 7` for wheel; `script(1)` typescript | GitHub Actions `tui-tty-desktop` job (`xvfb-run`). Local Linux with `DISPLAY`. |

GitHub-hosted runners have no physical trackpad. The PTY path writes the same SGR bytes a terminal sends for trackpad hscroll (`CSI < 67` wheel right) into a live `event::read` loop. Motion-bit `CSI < 99` must not pan. The desktop wheel path uses xterm: VTE 0.76 does not report X11 buttons 6/7, so xfce never encodes SGR 67. xterm does, from a real XTEST `click 7`.

Windows `cargo test --workspace` skips this crate (no PTY harness). The Actions `cargo` job is Ubuntu.

## Commands

From the repository root:

```bash
cargo test --workspace
cargo test --test tui_tty_e2e
```

Desktop (Linux, needs `DISPLAY`, `xfce4-terminal`, `xterm`, `xdotool`, `script`):

```bash
# GitHub Actions uses xvfb-run. Local X or Xvfb:
cargo test --test tui_tty_e2e -- --ignored --nocapture
```

Under Xvfb without a session already:

```bash
xvfb-run -a -s "-screen 0 1600x1000x24" bash -c '
  export NO_AT_BRIDGE=1 GTK_A11Y=none
  openbox --config-file crates/workspace-status/tests/tui_tty_e2e/openbox.xml \
    >/tmp/ws-tty-e2e-openbox.log 2>&1 &
  sleep 0.4
  cargo test --test tui_tty_e2e -- --ignored --nocapture --test-threads=1
'
```

Packages (Debian/Ubuntu): `xvfb xfce4-terminal xterm xdotool dbus-x11 openbox`. `script` is util-linux. The desktop job starts Openbox under Xvfb so xdotool can focus the terminal.

## Harness notes

- ASCII glyphs (`WS_STATUS_GLYPHS=ascii`) so CI does not depend on a Nerd Font. This is a test setting, not a product default.
- Watch and background fetch off (`WS_STATUS_WATCH_MS=0`, `WS_STATUS_FETCH_MS=0`).
- Isolated `XDG_STATE_HOME` plus a fresh `WS_STATUS_UPDATE_CHECK_STORE` so the GitHub Release prompt does not block mount.
- Mouse reports are xterm SGR (`CSI < Cb ; Cx ; Cy M`) with 1-based cells. Motion-bit wheel (`Cb` 99) must not pan (crossterm 0.28 drops it).
- Tree hscroll asserts a clipped `very-long` prefix on the **tree row**, then `TAIL99` after pan, with the prefix gone. A search chip that already contains `TAIL99` does not count. Same oracle as `tui_daily_e2e` `tree_trackpad_sgr_hscroll_pans_without_stealing_focus`. Do not `/` search the tail first: that puts `TAIL99` on screen before any wheel. Wait for a clipped tree row on the same frame (dump the screen on timeout). Do not `expect` a row after a later `screen()` call.
- Desktop wheel is a real XTEST pointer event: root-coordinate `mousemove --sync` then `click 7`, **no `--window`** on warp or click. VTE ignores `XSendEvent` (`xdotool --window`). The wheel oracle runs in **xterm** because VTE 0.76 does not report buttons 6/7. xfce stays for help/search keys.
- Openbox is started with `--config-file crates/workspace-status/tests/tui_tty_e2e/openbox.xml` (no decorations) so cell-to-pixel math matches the cell grid. The test does not `--replace` a running WM.
- Claims kept because they fail on a no-op: launch paint, `?` help, graph drill, graph branch focus `o`/`O`, Ctrl+C quit prompt, PTY SGR tree pan, desktop help/search, desktop xterm tree pan. Stash / stage / reviewed / fold / diff-pan / click are not claimed here.
- Escape is sent as CSI-u (`CSI 27 u`) so it is not swallowed as a CSI prefix. Printable keys stay single bytes.

Do not add a second screenshot pipeline. Do not replace `tui_daily_e2e.rs`.
