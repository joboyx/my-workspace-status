# Architecture

State of the code as it stands today.

## One snapshot, two renderers

```
crates/workspace-status ── collect_snapshots
                         ├── build_workspace_snapshot
                         │      ├─► render.rs                         --plain
                         │      └─► serialize_workspace_snapshot      --json
                         └── tui::run_tui                             ratatui (same snapshot)
```

`--plain` and `--json` print the same workspace snapshot. Display differs. See [snapshot.md](./snapshot.md).

The binary paints a ratatui TUI from `crates/workspace-status/src/tui` on a TTY (or `-i` / `--tui`). See [tui-rust.md](./tui-rust.md).

`should_open_tui` in `tui/mod.rs` picks with:

```rust
if flags.plain || flags.json || flags.verbose || flags.pull || flags.default_branch {
    return false;
}
stdout_is_tty || flags.force_tui
```

| Input            | Effect                                                                                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------- |
| stdout is a TTY  | TUI (the default) — **agents must pass `--plain` or `--json`** (TTY without one hangs on keyboard input) |
| `-i` / `--tui`   | TUI even without a TTY (humans only)                                                                    |
| `--plain`        | plain report (required for agent runs unless `--json`)                                                  |
| `--json`         | workspace snapshot JSON on stdout (progress from `-f`/`-p`/`-d` goes to stderr)                         |
| `-v`, `-p`, `-d` | plain report — these flags print progress logs mid-run, which cannot coexist with ratatui owning the screen |
| `--update`       | print GitHub Release notes newer than this install, then exec `workspace-status-update` and exit — never opens the TUI or applies repo filters |
| TUI startup (TTY) | before `run_tui`: at most every 6 hours, fetch the latest published GitHub Release. Newer → `new version available, update? [y/n]`. `y` runs `--update` (notes then sidecar). `n` / fail / current → open the TUI. `--plain` / `--json` / `--update` skip this |

On a TTY the mount loop uses the alternate screen (DEC 1049) so frames stay off the primary scrollback. Leave/re-enter brackets a blocking TTY `$EDITOR` (vim). GUI editors such as Cursor spawn detached and stay on the mounted TUI.

## Data pipeline

| Stage          | Module                                                                              | Produces                                                                                                                                                                 |
| -------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Config         | `config.rs`                                                                         | `WorkspaceStatusConfig` (`ignoredRepos`, `maxDepth`, `defaultBranches`, `editor`)                                                                                        |
| Startup update | `update_check.rs` (TUI launch only)                                                 | Last-check timestamp under `$XDG_STATE_HOME/my-workspace-status/update-check.json`. `curl` GET of GitHub Releases `latest` (`tag_name` vs `CARGO_PKG_VERSION`). Prompt, never silent install. Missing/failed `curl` stays quiet.     |
| `--update` notes | `update.rs` + `update_check.rs` `github_get`                                        | `curl` GET of GitHub Releases list. Print git-cliff notes for published tags newer than `CARGO_PKG_VERSION` (strip cargo-dist `## Install `), then exec the sidecar. Failed fetch stays quiet. |
| Discovery      | `discovery.rs` — `find_repos_with_config` + linked via `git worktree list --porcelain` | primary paths up to `maxDepth`, then linked worktrees under cwd (dot dirs still skipped by walk)                                                                       |
| Snapshot       | `discovery.rs` — `process_repo` via `parallel.rs` `map_with_concurrency` | `WorkspaceRepoSnapshot` per path (`checkoutKind`, `primaryRepo?`, `mergedIntoDefault`) from status + merge-base probe. Same-commit as the default tip is not merged. Cap `FETCH_CONCURRENCY` (4). |
| Workspace      | `snapshot.rs` — `build_workspace_snapshot`                                          | Workspace snapshot (`docs/snapshot.md`): repos, ignore, branch/sync/checkout, file changes. `--json` prints it. Hidden ignored repos stay out of `repos` unless shown    |
| Aggregation    | `snapshot.rs` / `render.rs`                                                         | `SummaryState` (incl. linked worktrees), verbose rows with `Files` column + `🔗` / merge marks; bucket sort uses the same adjacency as the TUI                            |
| Per-file parse | `snapshot.rs` / porcelain parse in `discovery.rs`                                   | `FileChange[]`, one row per path, merged across staged/unstaged/untracked                                                                                                |
| Output         | `render.rs` or `tui/tree.rs` — `build_tree`                                         | plain report (`Files` header, Linked summary) or a `TreeNode` tree                                                                                                       |

Porcelain v1 is pinned deliberately: v2 would change rename and XY handling and every parser downstream.

## Git subprocesses

All of them live in `crates/workspace-status/src/git.rs` and run as a subprocess. The binary prefers `/usr/bin/git` so WSL does not pick a Windows `git.exe`. Set `WORKSPACE_STATUS_GIT` to override.

Every wrapper starts git with stdin `/dev/null` and `GIT_TERMINAL_PROMPT=0`. A credential or SSH prompt therefore fails fast instead of blocking the ratatui event loop (the parent would be stuck in `output()` while the child waited on the same TTY). Merge also sets `GIT_EDITOR=true` and `GIT_MERGE_AUTOEDIT=no`.

`exec_git` swallows failures and returns `""`; `exec_git_status` returns the exit code. `exec_git_checked` is the wrapper that surfaces failure to the caller.

See [git-operations.md](./git-operations.md).

## Live refresh and background fetch

`tui/watch.rs` polls file signatures (status + mtime) and tree chrome signatures (repo / checkout / dir / workspace / group, including `HEAD` and `sync_note`) so those rows flash on semantic updates, including remove ghosts for ~800ms. Graph list rows and commit-file rows use separate signature maps keyed by repo (and source). A disjoint identity set seeds and does not flash, so switching repos does not light up the whole graph. Fetch / pull / push / default-branch completion also flashes `repo:<path>` / `checkout:<path>` rows. The flash is a four-step background fade; status letter colours stay. Watch discover + per-checkout `process_repo` jobs share the same cap as fetch; each `RepoSnapshot` replaces that path immediately and the focused pane reloads when that checkout's identity moves. Identical signatures **and** unchanged checkout identities (`HEAD` / sync note / dirty set) skip that pane git. The next poll is scheduled from the start of the interval, not after collect finishes. Keys cannot starve the tick: the loop is a fair `select!` with a bounded input batch.

`tui/fetch.rs` schedules bounded `git fetch --quiet` batches (`WS_STATUS_FETCH_MS`, default 5 minutes; `0` disables) and powers the manual `f` action. Independent checkouts run with a cap of `FETCH_CONCURRENCY` (4; `WS_STATUS_FETCH_CONCURRENCY`) so a workspace of many remotes is not one-repo-at-a-time. Progress is `Fetching n/N…` as completions land. File `row_signature` is status letter + `size:mtimeMs` (or `gone`), and `background_fetch_targets` is every snapshot except hidden ignored (linked worktrees included). Manual `f` stays on focus-scoped `op_targets`.

## Layout stability

Pane widths come from `tui/split.rs` `pane_widths(term_cols, fraction)` — never from tree label lengths. Long tree paths, graph subjects, and diff lines clip to that frozen width; `left_col_offset` / `right_col_offset` / `diff_col_offset` pan inside the clip. When a focused list or file-diff row moves, the viewport keeps it near the vertical middle (`list_viewport_start`). Graph scrollbar drag is the exception: it scrolls the viewport without moving the cursor. Mouse horizontal wheel (and Shift+wheel) pans the pane under the pointer without moving the focused row. The workspace tree matches the right pane. Trackpad hscroll is SGR `66`/`67`. The live loop enables mouse through `tui/tty.rs` (click + button-event tracking and SGR encoding; never DECSET 1003, because xterm protocol modes 1000/1002/1003 are exclusive and crossterm 0.28 drops motion-bit wheel). When a file diff has long lines, that report over the left pane pans the diff; short tree paths still pan the tree when the painted diff fits. Default fraction is `TREE_WIDTH_FRACTION` (0.4). The session keeps a `tree_fraction` (resets on next launch; not persisted) and freezes `{ tree_width, tree_inner_width, diff_width }`, recomputing when terminal columns change or when the user drags the divider. Clamp helpers keep both panes ≥ 20 cols (accounting for padding) when the terminal is wide enough. The in-diff side-by-side RULE uses the same session-only drag model. Hit-testing (`hit_split`) consumes `diff_split_rule_x` only while split mode is actually painted (`≥ NARROW_SXS`), and the graph scrollbar column / horizontal track (`graph_scrollbar_x` / `graph_hscrollbar_y`) from the last paint. Graph thumb drag and track jump reuse `SplitDrag` with the pane and in-diff splitters (one mouse stack). The vertical graph bar is recorded only after `graph_scroll` leaves 0; the horizontal bar only after `col_offset` leaves 0. File-diff bars use the same origin-hidden rule (`diff_scroll` / `diff_col_offset`). The `?` help overlay shrinks the panes instead of overlapping them. It keeps its wrapped row budget. Panes take the leftover rows (this can be fewer than 3).

The ratatui TUI applies crossterm `Resize` to `Terminal::resize` (the event size, not a later ioctl) and recomputes pane widths, graph gutter cap, help overlay rows, and list viewports from the new area. `run_tui` stays synchronous via a current-thread Tokio runtime. One async loop (`tui/event_loop.rs`) fairly `select!`s terminal input (dedicated thread, `tty::poll_event` / `read_event`, bounded batch), watch/fetch deadlines, `JoinSet` completions, flash / Ctrl-C, and a small presenter (dirty flag, ~16–33ms draw cadence, 120ms flash). Overlay modes skip watch/fetch ticks. Graph autoload runs only after list movement. Held nav (`h`/`j`/`k`/`l`) accepts Repeat; the input thread drops queued copies of that key after each move. Every git/process effect is `spawn_blocking`. Fetch / pull / push of independent checkouts share `env_fetch_concurrency` (default 4) and paint progress as each repo finishes. Watch/status streams `discover_checkouts` (focused checkout first) then `process_repo`; a slow checkout does not block applying the others or the focused pane. Paths the new discovery omitted (removed worktree, deleted repo) drop immediately — streamed collect never emits `None` for a vanished checkout. Generation ids drop superseded collect / pane results. A watch tick during a collect latches one rerun. First paint is the tree; the initial pane load (and optional startup fetch) enqueue after that. The loop still accepts nav, pane switch, cancel, resize, and quit. Actions that would start another git write are drained (`BusyAction::Ignore`). An unchanged watch poll (tree signatures **and** checkout `HEAD` / sync note / dirty set) skips that pane reload. The next watch tick is due from the start of the interval. Left-pane movement at every drill depth uses the same `RightPaneRequest` through `tui/effect.rs` (TTY `spawn_blocking` / Headless `interpret_sync`): depth 0 tree → graph or worktree diff; depth 1 graph row → commit files; depth 2 commit-file row → that file's diff. Headless does not run `Effect::EditFile` (TTY `$EDITOR`). `tui/event_pump.rs` `tty_event_loop_must_not_call_sync_pane_git` fails CI if pumps or loop-thread git return.

## Graph widget

`crates/workspace-status-graph` is a ratatui widget for one git graph window. The TUI loads a `GraphModel` in `graph_load.rs` (`log --exclude=refs/stash --all --topo-order --date-order`, window 300, autoload, extra `stash^1`) and paints `GraphWidget`. Graph `o` can pass selected local tips instead of `--all` so the window is only those ancestors; `O` restores `--all`. Hidden ignored worktrees stay out of `visible_rows` unless `show_ignored` is true. Worktree marks are linked extras from `git worktree list` (`.git` gitfile); the main checkout is not marked. Gutter cells use `GraphCell.color_lane` and the active theme's eight lane colours (`T` cycles; `GraphWidget::lane_colors`. An empty slice still falls back to `DEFAULT_LANE_COLORS`). Chrome is a 2-line selection footer (`selection_detail_lines` / `selection_detail_parts`) plus optional sync header (`graph_chrome_budget`). Footer ref chips reuse the same `LabelKind` runs as the commit spacer so `GraphWidget::label_palette` paints HEAD / default / feature / remote / tag with the row-chip colours (not a single footer wash). A loaded graph always emits the working-tree row. Commit and stash spacers reuse the same hash / date / author drop order. Dates are relative through 3 hours, then local `YYYY-MM-DD HH:MM`. The gutter is capped (`gutter.rs`: at most 30% of the pane, keep ≥24 columns for refs+subject) with one left-aligned clip for every row so rails stay column-aligned when subjects clip. Overflow branch/tag chips: truncate the next name with `…` (keep brackets) when part of it fits; a bold `[+N]` on the row itself counts only fully hidden chips (heading accent, not muted meta; omit `[+N]` when the last painted chip is merely truncated). The footer lists every full ref (HEAD commit refs on the working-tree row).

See [graph.md](./graph.md) and [git-graph-topology.md](./git-graph-topology.md).

Graph checkout confirm (and several other graph UX choices) is inspired by [Git Graph](https://github.com/mhutchie/vscode-git-graph) (mhutchie, VS Code).

## Where do I add X

| Change                | Touch                                                                                                                                                                     |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| New git operation     | `crates/workspace-status/src/git.rs` (add the wrapper), caller in `actions.rs` or `tui/app.rs` / `tui/state/`                                                           |
| Graph engine / UI     | `crates/workspace-status-graph` + `tui/graph_load.rs` + `tui/render.rs` + helpers in `git.rs`                                                                              |
| New tree node kind    | `tui/tree.rs` (`NodeKind`, `node_segments`, flatten / fold walks)                                                                                                         |
| New pane / overlay    | `tui/` module + `chrome.rs` row budget + `render.rs` layout                                                                                                               |
| TUI comments / markdown export | `tui/comments/` (`store`, `target` / GC, overlay, markdown export) + `;` / `V` / `y` / overlay `Ctrl-R` in `tui/keys.rs` + `help.rs` + `chrome.rs` overlay budget + `render.rs`. The comment overlay is a multiline caret editor (`CommentPrompt` wraps `tui-textarea`; Shift+Enter newline; Ctrl-A/E line; Ctrl-Left/Right word). It replaces the idle status row. Overlay Ctrl-R toggles `CommentPrompt.resolved`. Enter persists body and resolve state (`CommentEntry.resolved` in `comments.json`). `V` is visual-line highlight on a focused file diff. Without highlight, `;` opens the covering stored span. Highlight drops when the painted row list changes. `y` copies the focused tree / graph / commit-file / diff row and descendants under that row (after GC). Resolved comments stay in that copy and carry `[resolved]`. A line comment with a newline stays one markdown bullet. Continuation lines are quoted. GC uses snapshot `local_branches` (snapshot worker), not git on apply. Status-failed snapshots keep last-good branch names only when every checkout of that identity has an empty counted list. |
| Nav shell / drill     | `tui/drill.rs` + `tui/state/` + `chrome.rs` breadcrumb                                                                                                                  |
| New CLI flag          | `cli.rs` + `tui::HeadlessFlags` / `should_open_tui` if it affects TUI vs headless                                                                                          |
| TUI startup update check | `update_check.rs` + `cli.rs` (before `run_tui`). Sidecar exec and `--update` notes stay in `update.rs`. TTY spawn must set `WS_STATUS_UPDATE_CHECK_STORE` (CI: `tests/release_watch.rs`) |
| cargo-dist CI / git-cliff host steps | `.github/workflows/release.yml` + `dist-workspace.toml`. After `dist generate`, restore `workflow_dispatch` and host git-cliff. Guard: `tests/release_watch.rs`. Recipe under **Distribution** |
| Snapshot contract     | `snapshot.rs` (`build_workspace_snapshot`) + `docs/snapshot.md` + `tests/snapshot_contract.rs`                                                                            |
| New key binding       | `tui/keys.rs` (`Action`) + `tui/state/dispatch.rs` + `dispatch_keymap.rs` / `pan.rs` / `dispatch_drill.rs` / `dispatch_write.rs` + `tui/help.rs` (`HELP_GROUPS`) + `tui/gates.rs` if the key is row-scoped |
| Event-loop freeze / overlay ticks / graph autoload / key-repeat | `tui/event_loop.rs` + `tui/effect.rs` + `tui/scheduler.rs` + `tui/event_pump.rs` + `tui/keys.rs` (CI: `tty_event_loop_must_not_call_sync_pane_git`) |
| Mouse enable / SGR decode | `tui/tty.rs` (live `poll_event` / `read_event` / `enable_mouse`; Headless SGR through `decode_sgr_mouse`) + the input thread in `tui/event_loop.rs` |
| Shared TUI e2e seed / hscroll oracle | `crates/workspace-status/tests/common/` — both `tui_headless_e2e` and `tui_tty_e2e`. Not a third harness. See [tui-tty-e2e.md](./tui-tty-e2e.md) |
| Real-TTY TUI e2e | `crates/workspace-status/tests/tui_tty_e2e/` (PTY in `cargo test`; xfce keys + xterm XTEST wheel in Actions `tui-tty-desktop`). Not TestBackend. See [tui-tty-e2e.md](./tui-tty-e2e.md) |
| Desktop Xvfb / Openbox session | `scripts/with-desktop-session.sh` + `scripts/openbox.xml`. Callers: Actions `tui-tty-desktop`, `scripts/capture-demo-stills.sh`. Do not add a second screenshot pipeline. |

## CLI crate

`crates/workspace-status` is the CLI (`workspace-status` and `ws`).
It implements discovery, `--plain`, `--json`, `-a`/`--all`, `--update`, the TUI-startup GitHub Release prompt, repo filters,
ignored-repo visibility from snapshot.md, and the ratatui TUI on a TTY.
CLI flags live in `cli.rs`.

`--fetch`, `--pull`, and `--default-branch` write progress to stderr when `--json` is set. `--json` wins when both `--json` and `--plain` are set. `-v` applies to `--plain` only.

On a TTY (or `-i` / `--tui`) the binary opens the ratatui TUI. Tree chrome (status letters, Nerd glyphs, workspace wording, linked-checkout labels, sync marks) lives in `tui/icons.rs` (glyph registry), `tui/tree.rs` (`node_segments`), and `tui/render.rs` (right-aligned trailing + cursor bar). Bottom chrome (mode pills, hint chips, breadcrumb) lives in `tui/chrome.rs`. In-flight fetch / pull / push / default-branch paint `Verb n/N…` on the breadcrumb trailing slot (`tui/ops.rs` `format_running_op`) and redraw as each repo **completes** (fetch / pull / push overlap under `FETCH_CONCURRENCY`). Completion uses `format_completed_op` (`Fetched N repos`, with ` (N failed)` if any) so the slot never lists repo names. Commit-file lists reuse the same file chrome. See [tui-rust.md](./tui-rust.md).

## Graph crate

`crates/workspace-status-graph` is a ratatui widget for one git graph window.
The crate itself does not run a terminal app. The TUI paints `GraphWidget`.
`GraphWidget` colours subject vs meta vs HEAD / default / feature / remote / tag chips
(including the 2-line selection footer, which reuses the row-chip `LabelKind` runs)
and paints a 1-column position scrollbar after the list leaves the top, plus a 1-row horizontal bar after the viewport leaves the left edge. The TUI hit-tests those thumbs through
the same `tui/split.rs` `hit_split` / `SplitDrag` stack as the pane divider
and in-diff RULE (`SplitDrag::GraphScrollbar` / `GraphHScrollbar`). Track clicks jump toward that
position. Keyboard `j` / `k`, `h` / `l`, and PageUp/PageDown are unchanged.

See [graph.md](./graph.md).

## Distribution

The CLI is published with [cargo-dist](https://axodotdev.github.io/cargo-dist/) 0.32 (`dist-workspace.toml`).
`.github/workflows/release.yml` is generated (`dist generate`) and builds GitHub Release archives plus `workspace-status-installer.sh` / `.ps1` on a version tag (linux, macOS, and Windows `x86_64-pc-windows-msvc`; linux/mac aarch64 and x86_64).
`.github/workflows/tag-release.yml` writes an annotated `vX.Y.Z` on each push to `main` and dispatches Release (`GITHUB_TOKEN` tag pushes do not start other workflows). `allow-dirty = ["ci"]` lets generate succeed when `release.yml` has local edits. Generate still rewrites that file.
Do not set `dispatch-releases = true`. That switch drops tag-push. This repo needs tag-push plus `workflow_dispatch`.
The host job runs [git-cliff](https://git-cliff.org/) (`cliff.toml`, conventional commits) for the current tag and prepends that changelog to cargo-dist's installer/download body. That is what GitHub Release notes and `ws --update` show.
Installers place `workspace-status`, `ws`, and `workspace-status-update` in `~/.local/bin`.
`--update` (`ws --update` / `workspace-status --update`) prints GitHub Release notes for published versions newer than this binary (`CARGO_PKG_VERSION`), then execs `workspace-status-update` from the same directory as the current executable, then PATH. Installer-only historical bodies contribute no notes. A failed notes fetch stays quiet and still runs the sidecar. The sidecar's exit status is the process exit status. That run does not open the TUI or apply repo filters. `install-updater = true` keeps the sidecar in the installer.
A TTY TUI launch (`ws` / `workspace-status` without `--plain` / `--json` / `--update`) may ask to run that same sidecar **before** the alternate screen mounts: at most every 6 hours it `curl`s the latest published GitHub Release. Newer → `new version available, update? [y/n]`. `y` runs `--update` (notes then sidecar). `n` opens the TUI. Offline / current / parse failure / missing `curl` stay quiet. The last-check time is stored in `$XDG_STATE_HOME/my-workspace-status/update-check.json` (`WS_STATUS_UPDATE_CHECK_STORE` overrides). HeadlessTui tests do not run this check.
A TTY spawn that can write that file (PTY e2e, desktop e2e, `scripts/capture-demo-stills.sh`) must point `WS_STATUS_UPDATE_CHECK_STORE` at a temp path with a fresh `lastCheckUnix`. Otherwise the default XDG file is overwritten and the prompt can block mount. CI: `crates/workspace-status/tests/release_watch.rs`.
`workspace-status-graph` is a path library, not a separate dist app. There is no crates.io or Homebrew publish job.

### `dist generate`

1. Install cargo-dist 0.32.0. Match `cargo-dist-version` in `dist-workspace.toml`.
2. From the repository root, run `dist generate`.
3. Restore these host-job edits in `.github/workflows/release.yml`:
   - Keep `on.workflow_dispatch` (`tag-release.yml` dispatches it).
   - On the host checkout, set `fetch-depth: 0` and `fetch-tags: true`.
   - Run git-cliff (`orhun/git-cliff-action`, `--current --strip header`).
   - Prepend that changelog to the cargo-dist announcement before `gh release create`.
4. Run `cargo test --test release_watch`. That suite fails if generate dropped those steps, or if a TTY spawn path no longer assigns `WS_STATUS_UPDATE_CHECK_STORE`.

## Decisions

**Black-box tests against real temporary git repositories.** The plain report's output format is the user-facing contract (`SAMPLE_OUTPUT.md`). Mocked git would let porcelain parsing bugs through — rename arrows, `??` handling, `## branch...upstream [ahead N, behind M]` — which is exactly the class of bug that matters here. `crates/workspace-status/tests/snapshot_contract.rs` and the crate unit tests build real repos per scenario.

**Real-TTY TUI e2e** spawns the `workspace-status` binary on a PTY and writes keys / xterm SGR mouse bytes so `event::read` is the live loop. Headless TestBackend stays in `tui_headless_e2e.rs`. Desktop (xfce4-terminal keys; xterm XTEST wheel) runs in GitHub Actions `tui-tty-desktop` because hosted runners have no trackpad. That job, and `scripts/capture-demo-stills.sh`, start Xvfb / dbus / Openbox through `scripts/with-desktop-session.sh`. See [tui-tty-e2e.md](./tui-tty-e2e.md).
