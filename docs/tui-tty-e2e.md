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
- Claims kept because they fail on a no-op: launch paint (wait for `+dirty`, not only the tree — right-pane git is a worker), `?` help, graph drill, graph branch focus `o`/`O`, Shift+letter CSI-u (`O` clear focus, `S` stash menu, `/` capitals), graph-focus unmark-then-Enter, Ctrl+C quit prompt, PTY SGR tree pan, watch apply while keys arrive (no `r`), streamed collect (focused tree + pane before a blocked `git status`), desktop help/search, desktop xfce Shift keys (space toggles `[x]`; unmark-then-Enter before `O`), desktop xterm tree pan. Operator writes: Space reviewed (`*` ASCII), `s`/`u` stage/unstage, `f`/`p`/`P` against a local bare origin (no GitHub), graph `m` merge commit, and full stash (`S` create, graph `a`/`p`/`D`). Fold, `zz` subtree (400ms chord), click-to-select, chevron click, right-pane click, double-click Enter (hit-row Enter matches keyboard drill; chevron folds once), `gg`/`G`, Home/End, `n`/`N` pane next/prev (armed `/`; CSI-u `N`), PgUp/PgDn (CSI `ESC [5~` / `ESC [6~`), Ctrl-u/d (±5 rows; CSI-u Control), graph `c` create-branch, `r`, `.` ignored, `q`, help Enter-arm, revert confirm, worktree `W`, file-diff SGR pan, CSI-u `j` Repeat, picker `C`, `t`/`i`, CSI-u `T` theme (full cycle wrap; toast, surface, pills, heading, graph lanes; not the tree/flat pill), `d`, `Ctrl-o`, `e` editor (stub `$EDITOR` marker, then remount), and the startup update prompt are PTY claims in `operator.rs`.
- Escape is sent as CSI-u (`CSI 27 u`) so it is not swallowed as a CSI prefix. Unmodified printable keys stay single bytes. Shift+letter uses CSI-u (`CSI code ; 2 : 1 u` press, `: 3` release) so `event::read` sees the same encoding the live loop requested with keyboard enhancement — a raw `'O'` byte is a different path. Held nav Repeat is `CSI code ; 1 : 2 u`. Home / End use CSI `1 ; 1 : 1 ~` / `4 ; 1 : 1 ~` (press, then `: 3` release). `CSI 1 u` / `CSI 4 u` are not Home / End. PageUp / PageDown use xterm CSI `ESC [5~` / `ESC [6~`. Ctrl-u / Ctrl-d use CSI-u with modifier 5 (`CSI 117 ; 5 : 1 u` / `CSI 100 ; 5 : 1 u`, then `: 3` release). C0 `\x15` / `\x04` are a different path.

PTY tests are independent processes (own temp workspace + PTY). `cargo test --test tui_tty_e2e` may parallelize them. Desktop stays `--test-threads=1` on one X display. No second screenshot pipeline. Desktop twins exist only where PTY cannot encode the input (XTEST wheel).

## Coverage

Help overlay (`tui/help.rs` MOVE / GIT / VIEW) plus mouse and overlays. **PTY** runs in `cargo test`. **Desktop** is ignored locally and runs on `tui-tty-desktop`. TestBackend-only means `tui_headless_e2e.rs` — not a real-TTY claim.

| Feature | PTY | Desktop | Notes |
| --- | --- | --- | --- |
| Launch paint (tree + `+dirty`) | `pty_launch_paints_tree_diff_and_chrome` | — | Right-pane git is a worker |
| `?` help MOVE/GIT/VIEW | `pty_help_overlay` | `desktop_xfce_keys_help_and_search` | |
| Help `/` search, Enter does not arm | `pty_help_enter_does_not_arm_pane_search` | — | Highlight only |
| `/` pane search | `pty_graph_drill_enter_esc` | `desktop_xfce_keys_help_and_search` | |
| `n` / `N` pane next/prev | `pty_n_and_n_pane_next_prev` | — | Armed `/`. CSI-u `N`. Next match unfolds |
| Shift+letters in `/` (CSI-u) | `pty_shift_letters_csi_u_type_into_search` | `desktop_xfce_shift_keys_search_and_clear_focus` | |
| Graph drill Enter/Esc | `pty_graph_drill_enter_esc` | — | |
| Graph `o` / `O` focus | `pty_graph_branch_focus_overlay` | `desktop_xfce_shift_keys_search_and_clear_focus` | CSI-u `O` |
| Unmark `[x]` then Enter | `pty_graph_focus_unmark_enter_clears` | `desktop_xfce_shift_keys_search_and_clear_focus` | |
| Ctrl+C quit prompt | `pty_ctrl_c_prompts_before_quit` | — | Second Ctrl+C not claimed (sends `q` after) |
| `q` quit | `pty_q_quits_immediately` | — | Process exits |
| Tree SGR hscroll `CSI < 67` | `pty_tree_sgr_hscroll_pans_clipped_path` | `desktop_xterm_xtest_trackpad_hscroll` | Desktop: XTEST `click 7` in xterm |
| Watch while keys (no `r`) | `pty_watch_applies_while_keys_arrive` | — | |
| Streamed collect | `pty_streamed_collect_updates_focused_repo_before_slow` | — | |
| Space reviewed `*` | `pty_space_marks_dirty_file_reviewed` | `desktop_xfce_review_and_stage` | |
| `s` / `u` stage | `pty_stage_and_unstage_dirty_file` | `desktop_xfce_review_and_stage` | |
| `f` fetch | `pty_fetch_local_remote_marks_behind` | `desktop_xfce_fetch_then_pull_local_remote` | Local bare origin |
| `p` pull | `pty_pull_behind_local_remote` | `desktop_xfce_fetch_then_pull_local_remote` | |
| Shift+P push | `pty_shift_p_csi_u_pushes_ahead` | `desktop_xfce_shift_p_pushes_ahead` | |
| Graph `m` merge | `pty_graph_merge_creates_commit` | `desktop_xfce_graph_merge_creates_commit` | |
| Stash `S`/`a`/`D` | `pty_stash_create_apply_and_drop` | `desktop_xfce_stash_create_apply_and_drop` | |
| Graph stash `p` pop | `pty_stash_graph_pop` | `desktop_xfce_stash_graph_pop` | |
| `h`/`l` fold | `pty_fold_h_l_toggles_no_updates_group` | — | `l` on a file must not open the group |
| `z` fold | `pty_z_folds_focused_repo` | — | |
| `zz` subtree | `pty_zz_toggles_subtree_not_only_row` | — | 400ms chord. First `z` folds this row (`z…`). Second `z` folds descendants. Nested leaf stays folded after `l`. Late `z` is row-only. Graph is a no-op |
| `gg` / `G` | `pty_gg_and_g_jump_workspace_tree` | — | `G` via CSI-u |
| Home / End | `pty_home_and_end_jump_workspace_tree` | — | CSI `1~` / `4~` with event types. Cursor bar + fold |
| PgUp / PgDn | `pty_pgup_pgdn_pages_workspace_tree` | — | CSI `ESC [5~` / `ESC [6~`. Tree page jump |
| Ctrl-u / Ctrl-d | `pty_ctrl_u_d_jumps_workspace_tree` | — | CSI-u Control. ±5 rows. Cursor bar + pane body |
| Click-to-select row | `pty_click_selects_tree_row` | — | SGR press+release; must change the right pane |
| Click fold chevron | `pty_click_chevron_toggles_fold` | — | |
| Click right pane | `pty_click_right_pane_focuses` | — | Breadcrumb `[workspace]` |
| Double-click Enter | `pty_double_click_enters_on_hit_row` | — | Hit-row Enter: tree focuses right, graph matches keyboard drill, files open the diff, leaf no-op. Chevron folds once |
| Graph `c` create-branch | `pty_graph_c_creates_branch_at_commit` | — | Ref only, no checkout |
| `c` is not commit | `pty_c_on_tree_file_is_not_commit` | — | No overlay on a dirty file |
| Picker `C` create | `pty_branch_picker_shift_c_creates` | — | |
| `r` refresh | `pty_r_refreshes_new_dirty_file` | — | Watch off; new file appears (chrome toast is not sticky) |
| `.` ignored repos | `pty_dot_toggles_ignored_repos` | — | |
| `x` revert confirm | `pty_revert_confirm_n_cancels` | — | `n` cancels |
| `W` remove worktree | `pty_worktree_w_remove_confirm` | — | Linked row gone after `y`; TUI has no worktree-add key |
| File-diff SGR pan | `pty_left_pane_sgr_hscroll_pans_long_diff` | — | Wheel over the left pane |
| CSI-u `j` Repeat | `pty_key_repeat_j_reaches_no_updates` | — | Kind 2; burst would be drained |
| `t` / `i` view modes | `pty_t_and_i_toggle_view_modes` | — | |
| `T` theme | `pty_shift_t_csi_u_cycles_theme` | — | CSI-u Shift+T. Full cycle wrap. Toast, surface, pill, heading, graph lanes. Not `t` |
| `d` default branch | `pty_d_switches_to_default_branch` | — | |
| `Ctrl-o` full file | `pty_ctrl_o_full_file_context` | — | |
| `e` editor | `pty_e_opens_focused_file_in_editor` | — | Stub `$EDITOR` writes a marker and exits 0. Remount paints `edited README.md` |
| Update prompt | `pty_update_prompt_n_opens_tui` | — | Curl shim; `n` mounts the TUI |

Not claimed on a real TTY (TestBackend and/or no safe operator oracle yet): `m` mouse toggle, vertical wheel, divider/scrollbar drag, second Ctrl+C quit, `Y` revert+delete, graph `b` checkout, `d` skip-when-dirty. Worktree **add** is not a TUI key (`W` is remove only).

Do not add a second screenshot pipeline. Do not replace `tui_headless_e2e.rs`.
