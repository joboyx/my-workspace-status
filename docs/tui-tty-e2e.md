# Real-TTY TUI e2e

This is the human TUI e2e. Demo GIF clips stay in `scripts/capture-demo-stills.sh`. This harness is not that capture script.

It drives the real `workspace-status` binary the way a person does: a PTY (and, on Linux, a real terminal emulator). Assertions read the painted screen. It does not construct crossterm `Event` values in memory.

Git seeds live in `crates/workspace-status/tests/common/seed.rs`. `seed_two_tall_commit_files` adds a pair of long committed files so a file switch in a commit diff can prove the new view starts at the origin. `seed_two_file_long_message_repo` adds two commits with the same long message (`pair base` with one file, `pair tip` with two) so a depth-2 commit-files footer can scroll a message on one commit and prove another commit starts at the top. The tree hscroll oracle (clipped `very-long` vs `TAIL99`), file-diff tail (`UNIQUE_DIFF_TAIL`), and graph subject prefix (`UNIQUE_GRAP`) live in `tests/common/hscroll.rs`. The PTY harness extracts the left pane itself.

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

Packages (Debian/Ubuntu): `xvfb xfce4-terminal xterm xdotool dbus-x11 openbox`. `script` is util-linux. `scripts/with-desktop-session.sh` starts Xvfb when `DISPLAY` is unset, then dbus and Openbox (`scripts/openbox.xml`) so xdotool can focus the terminal. `scripts/capture-demo-stills.sh` sources the same helper. Do not invent a second screenshot pipeline. The Actions `tui-tty-desktop` job removes the image Chrome apt files (`google-chrome.list` and `google-chrome.sources`) before `apt-get update`. A hash mismatch on that index must not skip xfce or xterm.

## Fake forge

PR badge tests need `gh` and `glab` answers with no network and no login. `scripts/fake-forge/` holds POSIX `sh` stand-ins for both. They answer by flags and by the `glab api` endpoint, not by exact argv, so a longer `--json` field list or query string needs no change. Scenario file layout, failure files (`.exit`, `.stderr`), and the call log: `scripts/fake-forge/README.md`.

`FakeForge` in `crates/workspace-status/tests/common/fake_forge.rs` wires them into a spawn:

- `FakeForge::new(root)` makes a scenario directory and a `$BROWSER` recorder under `root`.
- The setters (`github_pr_list`, `github_pr_view`, `gitlab_mr_list`, `gitlab_mr`, `gitlab_approvals`, `gitlab_pipeline_jobs`) write what the CLI prints (`Reply::Json`) or how it fails (`Reply::Fail`: exit code and stderr). Each write goes to a temporary name and is renamed, so a setter called while the app runs swaps the answer for the next call only.
- `forge.env()` goes to `PtySession::open_with_env`: `PATH` with the scripts first, `WS_FAKE_FORGE_DIR`, and `BROWSER`. The update-check store, UI store, `XDG_CONFIG_HOME`, and theme mode isolation stay as in **Harness notes**.
- `forge.calls()` gives the argv line of each CLI call. `forge.opened()` gives each URL the `$BROWSER` recorder got.
- `set_origin(repo, url)` points a seeded repo's `origin` at a forge URL (`GITHUB_ORIGIN`, `GITLAB_ORIGIN`). Only `git remote` runs.

`pty_fake_forge_shows_pr_badges` proves the harness against the shipped badge. The `fake_forge::tests` unit tests in the helper run the scripts directly.

The PR status suite (`pty_pr_*`) adds two shared helpers:

- `tests/common/pr_scenarios.rs` holds the answers of the six badge states (`Badge`) for a GitHub PR and a GitLab merge request, and of a closed one. `put_github` and `put_gitlab` move a branch to a state, also while the app runs: a transition test swaps the answers, presses no key, and waits for the next poll. `put_github_with` and `put_gitlab_with` also set the title and base branch (`PrText`), for the PRs tab groups and stacks.
- `tests/tui_tty_e2e/pr_support.rs` seeds the checkouts and reads the result off the cell grid: the badge column (the last content cell of the tree, in the boxed and the flat look), the ref left of it, its colours, the toast rows, the row flash, and the call log. `watch_quiet` proves an alert did not show by sampling the screen over a window. For the PRs tab (`pty_prs_tab_*`), `PRS_BOARD` seeds nine PRs on both forges that make every kind of group, and `prs_list` reads the boxed list pane back as headers, PR rows, and the lines above them.

Tests that need polling pass `WS_STATUS_PR_POLL_MS` (300 ms); the poll-off test passes `0`; the others leave it unset, so the default period (60 s or 180 s) never fires inside a test. The Tokyo Night colours of the baseline config are the oracle for glyph and flash colours. A flash lasts 800 ms, so a flash check starts a frame record before the change (`PtySession::record_frames`) and looks for any step of the flash ramp in every whole frame since (`PtySession::wait_recorded_frame`), not in the frame on screen at the next poll.

## Verify driver

To watch the real binary run, against the same fake forge or against a live one, the verify skill drives it in a private tmux server: `.agents/skills/verify/SKILL.md` and `scripts/verify-tui.sh`. It is a test harness with the isolation rules of **Harness notes** (guarded by `crates/workspace-status/tests/release_watch.rs`), not the screenshot pipeline, and it does not replace a `pty_*` test.

## Harness notes

- ASCII glyphs (`WS_STATUS_GLYPHS=ascii`) so CI does not depend on a Nerd Font. This is a test setting, not a product default. The spawn drops parent `NO_COLOR` / `FORCE_COLOR` so colour claims can paint.
- Watch and background fetch off by default (`WS_STATUS_WATCH_MS=0`, `WS_STATUS_FETCH_MS=0`). Per-test env overrides (`PtySession::open_with_env`) re-enable watch for the live-input / streamed-collect cases.
- Isolated `XDG_STATE_HOME` plus a fresh `WS_STATUS_UPDATE_CHECK_STORE` so the GitHub Release prompt does not block mount and the operator XDG file is not written. CI: `crates/workspace-status/tests/release_watch.rs` (`tty_spawn_paths_isolate_update_check_store`). Demo GIF capture (`scripts/capture-demo-stills.sh`) uses the same isolation.
- `open_with_env` overrides for `WS_STATUS_COMMENT_STORE` / `WS_STATUS_VIEWED_STORE` / `WS_STATUS_UI_STORE` win over `.e2e-state` defaults. Tests that omit those keys still use `.e2e-state/comments.json` and `.e2e-state/viewed-files.json`, and a new `.e2e-state/ui-state-<n>.json` for each spawn, so no spawn starts with a last-used theme or the activity sparkline on, and a palette theme pick or sparkline toggle in one spawn reaches a later spawn only when the test passes the same `WS_STATUS_UI_STORE` to both. The desktop harness and `scripts/capture-demo-stills.sh` remove their UI-state store before each launch. CI: `tty_spawn_paths_isolate_ui_store`, `tty_spawn_paths_drop_old_theme_store_name`.
- Parent `WS_STATUS_WORKSPACE` is dropped so the fixture cwd is the workspace root. CI: `tty_spawn_paths_isolate_workspace_env`.
- Parent `WS_STATUS_PR_POLL_MS` is dropped (PTY and desktop harness; the verify driver pins it to `0`), so a test that leaves it unset never polls; a test that wants polling passes it. CI: `tty_spawn_paths_drop_parent_pr_poll_period`.
- `XDG_CONFIG_HOME` points at a `.e2e-config` dir in the fixture, so an operator user config file (`my-workspace-status/config.json`) cannot change the fixture's settings. `open_with_env` may override it. Demo GIF capture uses `tmp/demo-stills-stage/config`. CI: `tty_spawn_paths_isolate_user_config`.
- Before each spawn, the PTY and desktop harnesses write `BASELINE_USER_CONFIG` (`harness.rs`) into that dir: `{"theme":"tokyo-night","themeMode":"dark","viewDefaults":{"background":"terminal"}}`, the v0.1.244 look. A change to the shipped default theme or background does not move the existing tests. `themeMode` `dark` keeps the terminal background out of the colours: the desktop xterm is white, and the default `auto` would pick the light twin there. A parent `WS_STATUS_THEME` or `WS_STATUS_THEME_MODE` is dropped; one the test passes still beats the file (CI: `tty_spawn_paths_pin_theme_mode`). When the config file already exists (the test wrote it), the harness keeps it unchanged: the test owns the whole file. `PtySession::open_size_with_config(..., UserConfig::Own)` writes no file, for tests of the shipped look.
- Parent `VISUAL` is dropped. `e` reads `$VISUAL` before `$EDITOR`, so an operator value would outrank the stub `EDITOR` a test passes.
- Parent `WS_STATUS_TERM_PROGRESS` is dropped, so terminal progress (OSC 9;4) is on, the shipped default. A test that passes the variable still sets it. `PtySession::raw_output()` holds the bytes; `vt100` drops OSC 9;4 from the screen.
- The PTY harness can decode OSC 52 clipboard payloads from the master byte stream (`PtySession::clipboard_payloads` / `last_clipboard`). `vt100` strips OSC 52 from the painted screen, so copy claims must read the raw bytes (`ESC ] 52 ; c ; <base64> BEL` or ST). Adding a `pty_*` test still does not require other edits to this file.
- `PtySession::raw_output` returns every byte the child wrote, escapes included. Claims about sequences that `vt100` does not model (OSC 11, OSC 8 hyperlinks, OSC 9;4 progress) read this tap.
- Every screen and cell read (`screen`, `cell_paint`, `has_rgb`, and the rest) sees a whole frame. The TUI wraps each frame in a synchronized update (`CSI ? 2026 h` … `CSI ? 2026 l`); the harness `FrameParser` feeds every byte to `vt100` at once but shows the screen as it was at the last update end, as a terminal with the mode does. A frame the PTY splits over several reads, or the blank screen between a resize's clear and its next frame, is never visible. Bytes outside an update (startup, mouse-mode changes, exit) show at the end of each read. `PtySession::resize` sets the grid size and then the PTY size under one parser lock, as a terminal resizes its grid before it signals, so the redraw for the new size never lands in the old-size grid.
- `PtySession::open_with_bg_reply(workspace, env, rgb)` answers the startup OSC 11 background query: when the child writes `ESC ] 11 ; ?`, the reader thread writes `ESC ] 11 ; rgb:<rgb> BEL` back on the PTY (`rgb` is, for example, `ffff/ffff/ffff`). It does not answer DA1. Pass `WS_STATUS_THEME_MODE=auto` in `env`, or the baseline `dark` pin skips the query. Plain spawns answer nothing, so an `auto` child waits its 150 ms query timeout and keeps the resolved theme.
- Mouse reports are xterm SGR (`CSI < Cb ; Cx ; Cy M`) with 1-based cells. Motion-bit wheel (`Cb` 99) must not pan (crossterm 0.28 drops it).
- Tree hscroll asserts a clipped `very-long` prefix on the **tree row**, then `TAIL99` after pan, with the prefix gone. A search chip that already contains `TAIL99` does not count. Prefix, tail, and predicates live in `crates/workspace-status/tests/common/hscroll.rs`. Do not `/` search the tail first: that puts `TAIL99` on screen before any wheel. Wait for a clipped tree row on the same frame (dump the screen on timeout). Do not `expect` a row after a later `screen()` call.
- Desktop wheel is a real XTEST pointer event: root-coordinate `mousemove --sync` then `click 7`, **no `--window`** on warp or click. VTE ignores `XSendEvent` (`xdotool --window`). The wheel oracle runs in **xterm** because VTE 0.76 does not report buttons 6/7. xfce stays for help/search keys.
- Openbox is started by `scripts/with-desktop-session.sh` with `scripts/openbox.xml` (no decorations) so cell-to-pixel math matches the cell grid. The test does not `--replace` a running WM. The Rust desktop harness only starts Openbox when none is running (local `DISPLAY` without the helper).
- Desktop xfce `/` arm: wait for SEARCH and the live jump (graph subject) before Return. xfce can drop Enter while right-pane git runs. PTY byte writes do not have that race. Graph stash pop on xfce uses `j` / Tab / `j` / `p`, not SEARCH. Stash create on xfce waits first-paint README, then Shift-S, not `/README`.
- Desktop open waits for idle `? help` chrome (not a space-padded pane title). xdotool key/type uses `--clearmodifiers` so a prior Shift chord cannot turn `/` into help `?`.
- Escape is sent as CSI-u (`CSI 27 u`) so it is not swallowed as a CSI prefix. Unmodified printable keys stay single bytes. Shift-letter uses CSI-u (`CSI code ; 2 : 1 u` press, `: 3` release) so `event::read` sees the same encoding the live loop requested with keyboard enhancement — a raw `'O'` byte is a different path. Held nav Repeat is `CSI code ; 1 : 2 u`. Home / End use CSI `1 ; 1 : 1 ~` / `4 ; 1 : 1 ~` (press, then `: 3` release). `CSI 1 u` / `CSI 4 u` are not Home / End. PageUp / PageDown use xterm CSI `ESC [5~` / `ESC [6~`. Ctrl-u / Ctrl-d use CSI-u with modifier 5 (`CSI 117 ; 5 : 1 u` / `CSI 100 ; 5 : 1 u`, then `: 3` release). First Ctrl-c uses CSI-u Control+c (`CSI 99 ; 5 : 1 u` press, `: 3` release). C0 `\x15` / `\x04` / `\x03` are a different path.
- Typeless CSI-u (VTE that omits event types) is `CSI code ; 1 u`. A same-key report inside the few-millisecond protocol echo window (`G_CHORD_PROTOCOL_ECHO_MS` in `tui/keys.rs`) is the key-up of that tap; a later one is a new tap. `pty_live_g_chords` drives one-report chords. Typeless press+echo `gt` stays in leftover `pty_compare_gt_chords_and_gg`. A raw-byte `gg` stays two `g` bytes and must not inject Release.
- Key gestures are timed by arrival. The TUI stamps each event with the stdin read its bytes came in, and the `gg` / `zz` / `gt` chord window (400 ms), the typeless echo window, the double-click window (400 ms), and the Ctrl-c press-again window (2 s) measure between those stamps. Send every key of one gesture in one PTY write (`support::send_gesture`, `support::sgr_double_click`; the harness `gg`, `zz`, `shift_letter`, `ctrl_letter`, `home`, `end`, and `sgr_click` already do): the bytes of one small write arrive back to back and, in practice, a PTY hands them to one read, so the keys share one stamp. The line discipline does not promise one read; a split read changes the outcome only if the reader also stalls longer than the window between the two reads. Separate writes can arrive any time apart.
- A rule whose point is a gap length (a one-report `gg` 40 ms apart, a watch apply inside the chord window, a chord that expires) is a timing window. A PTY test cannot hold a gap on a loaded machine, so pin it in a `tui::keys` or `tui::state` unit test with explicit instants (`AppState::begin_input`) and keep the PTY test on the path without a gap. A lower bound alone is safe on the PTY: wait for a paint that proves the first key was handled, then sleep (`support::PAST_CHORD_WINDOW`), then send the next key. The same goes for product timers (toast and flash lifetimes, the popover leave grace): a PTY test waits for their paint, and a unit test pins their length.
- Wait for a screen condition, never for time; the harness has no fixed sleep. An absence claim ("Ctrl-o does nothing here", "the confirm holds") needs a later positive anchor, since keys and clicks run in order: `support::prove_keys_handled` (`?` then Esc) in Normal mode, `support::prove_palette_keys_handled` (type `z`, delete it) in an open palette, or the next key that answers an open confirm or picker. Put the whole claim in one `wait_pred` frame (`support::wait_screen`); a second `screen()` read can see a later frame.
- One nav key per wait. The input thread drops a queued copy of a held nav key (`h` `j` `k` `l`, arrows, page, also in text fields), so two copies in one read move once. Step with `support::step_cursor` / `support::step_until_changed` (wait for the cursor, pan, or scroll to change) and stop on the target paint; type text with `support::type_palette_filter`, which waits for the field to show each nav letter. Separate setup clicks from a double-click by cell (one column over), not by time.

PTY tests are independent processes (own temp workspace + PTY). `cargo test --test tui_tty_e2e` may parallelize them. Desktop stays `--test-threads=1` on one X display. No second screenshot pipeline. Desktop twins exist only where PTY cannot encode the input (XTEST wheel).

The tests live under `crates/workspace-status/tests/tui_tty_e2e/human/`. Adding a test does not edit this file.

Do not add a second screenshot pipeline.
