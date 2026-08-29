# Real-TTY TUI e2e

Headless TestBackend coverage stays in `crates/workspace-status/tests/tui_headless_e2e.rs`. Screenshot stills stay in `scripts/capture-demo-stills.sh`. This harness is neither.

It drives the real `workspace-status` binary the way a person does: a PTY (and, on Linux, a real terminal emulator). Assertions read the painted screen. It does not construct crossterm `Event` values in memory.

Git seeds shared with the TestBackend suite live in `crates/workspace-status/tests/common/seed.rs`. The tree hscroll oracle (clipped `very-long` vs `TAIL99`) lives in `tests/common/hscroll.rs`. Each harness still extracts the left pane itself.

## What runs where

| Path | How | Where it executes |
| --- | --- | --- |
| PTY | `portable-pty` spawn + byte writes (keys, xterm SGR mouse) + `vt100` screen | `cargo test --workspace` on Unix. GitHub Actions `cargo` job. |
| Desktop | xfce4-terminal keys; xterm + XTEST `click 7` for wheel; `script(1)` typescript | GitHub Actions `tui-tty-desktop` job (`scripts/with-desktop-session.sh`). Local Linux with `DISPLAY`. |

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
# Existing DISPLAY (local X):
cargo test --test tui_tty_e2e -- --ignored --nocapture --test-threads=1

# No DISPLAY (same helper as GitHub Actions tui-tty-desktop):
./scripts/with-desktop-session.sh --auto -- \
  cargo test --test tui_tty_e2e -- --ignored --nocapture --test-threads=1
```

Packages (Debian/Ubuntu): `xvfb xfce4-terminal xterm xdotool dbus-x11 openbox`. `script` is util-linux. `scripts/with-desktop-session.sh` starts Xvfb when `DISPLAY` is unset, then dbus and Openbox (`scripts/openbox.xml`) so xdotool can focus the terminal. `scripts/capture-demo-stills.sh` sources the same helper. Do not invent a second screenshot pipeline.

## Harness notes

- ASCII glyphs (`WS_STATUS_GLYPHS=ascii`) so CI does not depend on a Nerd Font. This is a test setting, not a product default. The spawn drops parent `NO_COLOR` / `FORCE_COLOR` so colour claims can paint.
- Watch and background fetch off by default (`WS_STATUS_WATCH_MS=0`, `WS_STATUS_FETCH_MS=0`). Per-test env overrides (`PtySession::open_with_env`) re-enable watch for the live-input / streamed-collect cases.
- Isolated `XDG_STATE_HOME` plus a fresh `WS_STATUS_UPDATE_CHECK_STORE` so the GitHub Release prompt does not block mount and the operator XDG file is not written. CI: `crates/workspace-status/tests/release_watch.rs` (`tty_spawn_paths_isolate_update_check_store`). Screenshot stills (`scripts/capture-demo-stills.sh`) use the same isolation.
- Mouse reports are xterm SGR (`CSI < Cb ; Cx ; Cy M`) with 1-based cells. Motion-bit wheel (`Cb` 99) must not pan (crossterm 0.28 drops it).
- Tree hscroll asserts a clipped `very-long` prefix on the **tree row**, then `TAIL99` after pan, with the prefix gone. A search chip that already contains `TAIL99` does not count. Prefix, tail, and predicates live in `crates/workspace-status/tests/common/hscroll.rs` (shared with `tui_headless_e2e`). Do not `/` search the tail first: that puts `TAIL99` on screen before any wheel. Wait for a clipped tree row on the same frame (dump the screen on timeout). Do not `expect` a row after a later `screen()` call.
- Desktop wheel is a real XTEST pointer event: root-coordinate `mousemove --sync` then `click 7`, **no `--window`** on warp or click. VTE ignores `XSendEvent` (`xdotool --window`). The wheel oracle runs in **xterm** because VTE 0.76 does not report buttons 6/7. xfce stays for help/search keys.
- Openbox is started by `scripts/with-desktop-session.sh` with `scripts/openbox.xml` (no decorations) so cell-to-pixel math matches the cell grid. The test does not `--replace` a running WM. The Rust desktop harness only starts Openbox when none is running (local `DISPLAY` without the helper).
- Desktop xfce `/` arm: wait for SEARCH and the live jump (graph subject) before Return. xfce can drop Enter while right-pane git runs. PTY byte writes do not have that race. `desktop_xfce_stash_graph_pop` is the graph-jump case.
- Escape is sent as CSI-u (`CSI 27 u`) so it is not swallowed as a CSI prefix. Unmodified printable keys stay single bytes. Shift+letter uses CSI-u (`CSI code ; 2 : 1 u` press, `: 3` release) so `event::read` sees the same encoding the live loop requested with keyboard enhancement — a raw `'O'` byte is a different path. Held nav Repeat is `CSI code ; 1 : 2 u`. Home / End use CSI `1 ; 1 : 1 ~` / `4 ; 1 : 1 ~` (press, then `: 3` release). `CSI 1 u` / `CSI 4 u` are not Home / End. PageUp / PageDown use xterm CSI `ESC [5~` / `ESC [6~`. Ctrl-u / Ctrl-d use CSI-u with modifier 5 (`CSI 117 ; 5 : 1 u` / `CSI 100 ; 5 : 1 u`, then `: 3` release). First Ctrl+C uses CSI-u Control+c (`CSI 99 ; 5 : 1 u` press, `: 3` release). C0 `\x15` / `\x04` / `\x03` are a different path.

PTY tests are independent processes (own temp workspace + PTY). `cargo test --test tui_tty_e2e` may parallelize them. Desktop stays `--test-threads=1` on one X display. No second screenshot pipeline. Desktop twins exist only where PTY cannot encode the input (XTEST wheel).

The tests in `crates/workspace-status/tests/tui_tty_e2e/` are the list. Adding a test does not edit this file.

Do not add a second screenshot pipeline. Do not replace `tui_headless_e2e.rs`.
