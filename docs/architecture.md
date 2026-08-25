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
| `--update`       | exec `workspace-status-update` and exit — never opens the TUI or applies repo filters                      |
| TUI startup (TTY) | before `run_tui`: at most every 6 hours, fetch the latest published GitHub Release. Newer → `new version available, update? [y/n]`. `y` runs the sidecar. `n` / fail / current → open the TUI. `--plain` / `--json` / `--update` skip this |

On a TTY the mount loop uses the alternate screen (DEC 1049) so frames stay off the primary scrollback. Leave/re-enter brackets a blocking TTY `$EDITOR` (vim). GUI editors such as Cursor spawn detached and stay on the mounted TUI.

## Data pipeline

| Stage          | Module                                                                              | Produces                                                                                                                                                                 |
| -------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Config         | `config.rs`                                                                         | `WorkspaceStatusConfig` (`ignoredRepos`, `maxDepth`, `defaultBranches`, `editor`)                                                                                        |
| Startup update | `update_check.rs` (TUI launch only)                                                 | Last-check timestamp under `$XDG_STATE_HOME/my-workspace-status/update-check.json`. `curl` GET of GitHub Releases `latest` (`tag_name` vs `CARGO_PKG_VERSION`). Prompt, never silent install. Missing/failed `curl` stays quiet.     |
| Discovery      | `discovery.rs` — `find_repos_with_config` + linked via `git worktree list --porcelain` | primary paths up to `maxDepth`, then linked worktrees under cwd (dot dirs still skipped by walk)                                                                       |
| Snapshot       | `discovery.rs` — `process_repo`                                                     | `WorkspaceRepoSnapshot` per path (`checkoutKind`, `primaryRepo?`, `mergedIntoDefault`) from status + merge-base probe                                                     |
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

`tui/watch.rs` polls file signatures (status + mtime) and tree chrome signatures (repo / checkout / dir / workspace / group) so those rows flash on semantic updates, including remove ghosts for ~800ms. Graph list rows and commit-file rows use separate signature maps keyed by repo (and source). A disjoint identity set seeds and does not flash, so switching repos does not light up the whole graph. Fetch / pull / push / default-branch completion also flashes `repo:<path>` / `checkout:<path>` rows. The flash is a four-step background fade; status letter colours stay. Watch snapshot collect and the follow-up graph/diff reload run on a worker; identical signatures skip that pane git.

`tui/fetch.rs` schedules bounded `git fetch --quiet` batches (`WS_STATUS_FETCH_MS`, default 5 minutes; `0` disables) and powers the manual `f` action. File `row_signature` is status letter + `size:mtimeMs` (or `gone`), and `background_fetch_targets` is every snapshot except hidden ignored (linked worktrees included). Manual `f` stays on focus-scoped `op_targets`.

## Layout stability

Pane widths come from `tui/split.rs` `pane_widths(term_cols, fraction)` — never from tree label lengths. Long tree paths, graph subjects, and diff lines clip to that frozen width; `left_col_offset` / `right_col_offset` / `diff_col_offset` pan inside the clip. Default fraction is `TREE_WIDTH_FRACTION` (0.4). The session keeps a `tree_fraction` (resets on next launch; not persisted) and freezes `{ tree_width, tree_inner_width, diff_width }`, recomputing when terminal columns change or when the user drags the divider. Clamp helpers keep both panes ≥ 20 cols (accounting for padding) when the terminal is wide enough. The in-diff side-by-side RULE uses the same session-only drag model. Hit-testing consumes `diff_split_rule_x` only while split mode is actually painted (`≥ NARROW_SXS`). The `?` help overlay shrinks the panes instead of overlapping them.

The ratatui TUI applies crossterm `Resize` to `Terminal::resize` (the event size, not a later ioctl) and recomputes pane widths, graph gutter cap, help overlay rows, and list viewports from the new area. The event loop drains queued input with `poll(0)` + `read`, skips watch/fetch ticks while an overlay is open, and autoloads graph history only after list movement. Long git work (fetch / pull / push / default-branch / watch / full reload **and** the follow-up graph `log` / file `diff` in `load_right`) runs on a worker; the loop still paints and accepts resize and quit. An unchanged watch poll skips that pane reload so idle ticks do not re-run `git log`.

## Graph widget

`crates/workspace-status-graph` is a ratatui widget for one git graph window. The TUI loads a `GraphModel` in `graph_load.rs` (`log --exclude=refs/stash --all --topo-order --date-order`, window 300, autoload, extra `stash^1`) and paints `GraphWidget`. Hidden ignored worktrees stay out of `visible_rows` unless `show_ignored` is true. Worktree marks are linked extras from `git worktree list` (`.git` gitfile); the main checkout is not marked. Gutter cells use `GraphCell.color_lane` and `DEFAULT_LANE_COLORS`. Chrome is a 2-line selection footer (`selection_detail_lines` / `selection_detail_parts`) plus optional sync header (`graph_chrome_budget`). Footer ref chips reuse the same `LabelKind` runs as the commit spacer so `GraphWidget::label_palette` paints HEAD / default / feature / remote / tag with the row-chip colours (not a single footer wash). A loaded graph always emits the working-tree row. Commit and stash spacers reuse the same hash / date / author drop order. Dates are relative through 3 hours, then UTC `YYYY-MM-DD HH:MM`. The gutter is capped (`gutter.rs`: at most 30% of the pane, keep ≥24 columns for refs+subject) and windowed around the focused commit. Overflow branch/tag chips collapse to a bold `[+N]` chip on the row itself (heading accent, not muted meta); the footer lists every ref (HEAD commit refs on the working-tree row).

See [graph.md](./graph.md) and [git-graph-topology.md](./git-graph-topology.md).

Graph checkout confirm (and several other graph UX choices) is inspired by [Git Graph](https://github.com/mhutchie/vscode-git-graph) (mhutchie, VS Code).

## Where do I add X

| Change                | Touch                                                                                                                                                                     |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| New git operation     | `crates/workspace-status/src/git.rs` (add the wrapper), caller in `actions.rs` or `tui/app.rs` / `tui/state.rs`                                                           |
| Graph engine / UI     | `crates/workspace-status-graph` + `tui/graph_load.rs` + `tui/render.rs` + helpers in `git.rs`                                                                              |
| New tree node kind    | `tui/tree.rs` (`NodeKind`, `node_segments`, flatten / fold walks)                                                                                                         |
| New pane / overlay    | `tui/` module + `chrome.rs` row budget + `render.rs` layout                                                                                                               |
| Nav shell / drill     | `tui/drill.rs` + `tui/state.rs` + `chrome.rs` breadcrumb                                                                                                                  |
| New CLI flag          | `cli.rs` + `tui::HeadlessFlags` / `should_open_tui` if it affects TUI vs headless                                                                                          |
| TUI startup update check | `update_check.rs` + `cli.rs` (before `run_tui`). Sidecar exec stays in `update.rs`                                                                                      |
| Snapshot contract     | `snapshot.rs` (`build_workspace_snapshot`) + `docs/snapshot.md` + `tests/snapshot_contract.rs`                                                                            |
| New key binding       | `tui/keys.rs` (`Action`) + `tui/state.rs` dispatch + `tui/help.rs` (`HELP_GROUPS`) + `tui/gates.rs` if the key is row-scoped                                               |
| Event-loop freeze / overlay ticks / graph autoload gates | `tui/event_pump.rs` + `tui/app.rs` (`run_loop`, `run_work_pumped`, `load_right_pumped`)                                                                                    |

## CLI crate

`crates/workspace-status` is the CLI (`workspace-status` and `ws`).
It implements discovery, `--plain`, `--json`, `-a`/`--all`, `--update`, the TUI-startup GitHub Release prompt, repo filters,
ignored-repo visibility from snapshot.md, and the ratatui TUI on a TTY.
CLI flags live in `cli.rs`.

`--fetch`, `--pull`, and `--default-branch` write progress to stderr when `--json` is set. `--json` wins when both `--json` and `--plain` are set. `-v` applies to `--plain` only.

On a TTY (or `-i` / `--tui`) the binary opens the ratatui TUI. Tree chrome (status letters, Nerd glyphs, workspace wording, linked-checkout labels, sync marks) lives in `tui/icons.rs` (glyph registry), `tui/tree.rs` (`node_segments`), and `tui/render.rs` (right-aligned trailing + cursor bar). Bottom chrome (mode pills, hint chips, breadcrumb) lives in `tui/chrome.rs`. In-flight fetch / pull / push / default-branch paint `Verb n/N…` on the breadcrumb trailing slot (`tui/ops.rs` `format_running_op`) and redraw after each repo. Completion uses `format_completed_op` (`Fetched N repos`, with ` (N failed)` if any) so the slot never lists repo names. Commit-file lists reuse the same file chrome. See [tui-rust.md](./tui-rust.md).

## Graph crate

`crates/workspace-status-graph` is a ratatui widget for one git graph window.
The crate itself does not run a terminal app. The TUI paints `GraphWidget`.
`GraphWidget` colours subject vs meta vs HEAD / default / feature / remote / tag chips
(including the 2-line selection footer, which reuses the row-chip `LabelKind` runs)
and paints a 1-column position scrollbar.

See [graph.md](./graph.md).

## Distribution

The CLI is published with [cargo-dist](https://axodotdev.github.io/cargo-dist/) 0.32 (`dist-workspace.toml`).
`.github/workflows/release.yml` is generated (`dist generate`) and builds GitHub Release archives plus `workspace-status-installer.sh` / `.ps1` on a version tag (linux, macOS, and Windows `x86_64-pc-windows-msvc`; linux/mac aarch64 and x86_64).
`.github/workflows/tag-release.yml` writes an annotated `vX.Y.Z` on each push to `main` and dispatches Release (`GITHUB_TOKEN` tag pushes do not start other workflows). `allow-dirty = ["ci"]` keeps the extra `workflow_dispatch` on the generated Release workflow.
Installers place `workspace-status`, `ws`, and `workspace-status-update` in `~/.local/bin`.
`--update` (`ws --update` / `workspace-status --update`) execs `workspace-status-update` from the same directory as the current executable, then PATH. The sidecar's exit status is the process exit status. That run does not open the TUI or apply repo filters. `install-updater = true` keeps the sidecar in the installer.
A TTY TUI launch (`ws` / `workspace-status` without `--plain` / `--json` / `--update`) may ask to run that same sidecar **before** the alternate screen mounts: at most every 6 hours it `curl`s the latest published GitHub Release. Newer → `new version available, update? [y/n]`. `y` execs the sidecar. `n` opens the TUI. Offline / current / parse failure / missing `curl` stay quiet. The last-check time is stored in `$XDG_STATE_HOME/my-workspace-status/update-check.json` (`WS_STATUS_UPDATE_CHECK_STORE` overrides). HeadlessTui tests do not run this check.
`workspace-status-graph` is a path library, not a separate dist app. There is no crates.io or Homebrew publish job.

## Decisions

**Black-box tests against real temporary git repositories.** The plain report's output format is the user-facing contract (`SAMPLE_OUTPUT.md`). Mocked git would let porcelain parsing bugs through — rename arrows, `??` handling, `## branch...upstream [ahead N, behind M]` — which is exactly the class of bug that matters here. `crates/workspace-status/tests/snapshot_contract.rs` and the crate unit tests build real repos per scenario.
