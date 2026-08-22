# Rust TUI

`workspace-status` and `ws` from `crates/workspace-status` open a ratatui TUI when stdout is a TTY and you did not pass `--plain`, `--json`, `-v`, `-p`, or `-d`.

A non-TTY run without those flags still prints `--plain`. Agents must pass `--plain` or `--json`.

The TypeScript Ink app stays in this repository. Use it when you need a feature that this TUI does not implement yet.

Screenshots of the daily views live in the [root README](../README.md#screenshots).

## Daily keys

| Key | Action |
| --- | --- |
| `q` | Quit |
| `?` | Help overlay (short list, not a wall of text) |
| `j` / `k` or arrows | Move the tree. On a focused graph, move the graph cursor. On a file list, move the file. On a focused file diff, scroll the diff |
| `z` | Toggle fold |
| `h` / `l` or left / right | Close / open fold. Space does not fold |
| `.` | Show or hide ignored repos |
| `/` | Search prompt. Enter arms the query. Esc cancels |
| `n` / `N` | Next / previous search match (previous is `N`, not `p`) |
| `s` | Stage the focused dirty file |
| `u` | Unstage the focused dirty file |
| `x` | Revert the focused dirty file (confirm `y` / `n`) |
| `e` | Edit the focused file (`$EDITOR` or config `editor`) |
| `f` | Fetch visible targets |
| `p` | Pull visible targets that are behind |
| `d` | Switch visible targets to the default branch |
| `P` | Push the focused visible repo or checkout |
| `S` | Stash menu (`s` create, `a` apply, `p` pop, `d` drop). From a repo or file row this targets the latest stash. From a focused graph stash row it targets that `stash@{n}` |
| `Enter` | Focus the right pane, or drill: graph commit → file list → commit diff |
| `Esc` | Pop one drill level, or return focus to the tree |
| `a` / `p` / `D` | Apply / pop / drop the focused graph stash row. Drop asks `y` / `n` |
| `b` | Branch picker (list, filter, checkout, `C` create) |
| `r` | Reload the workspace snapshot |
| `space` | Mark a dirty file as reviewed. Writes the same viewed-files store as the Ink app |
| `w` / `W` | Remove the focused linked worktree after `y` / `n` |
| `Tab` | Focus the other pane |
| click | Select a tree row, or focus the right pane |
| `g` / `G` | First / last tree row |

`-a` starts with ignored repos shown. `-f` starts a fetch after the first paint. First paint does not wait on a network fetch.

`WS_STATUS_WATCH_MS` polls local git and refreshes the snapshot. Default is `3000`. `0` disables the poll. Fold, focus, and scroll stay put. Only rows whose identity actually changed flash.

`WS_STATUS_FETCH_MS` runs `git fetch` on visible primary checkouts. Default is `300000` (5 minutes). `0` disables it. The watch poll stays a separate timer. Hidden ignored repos stay out. Linked worktrees are not fetched unless you focus that row and press `f`.

## What this TUI does

- Tree of repos, linked worktrees, and dirty files from the same snapshot builder as `--plain` / `--json`
- Right pane: file diff when a dirty file is focused. Graph pane via `workspace-status-graph` when a repo or worktree is focused
- Hidden ignored repos stay out of the tree, search, stage / unstage / revert, and fetch / pull / default unless you show them
- Search matches include folded rows. Focusing a match unfolds its ancestors
- Stage / unstage / revert act on the focused dirty file only. Repo, dir, and workspace rows are a no-op. Revert asks `y` / `n` before it writes. Stage and unstage do not confirm
- `e` uses config `editor`, then `$EDITOR`, then `$VISUAL`, then `vim`. A TTY editor leaves the alternate screen and returns to the same fold, focus, and scroll. GUI editors (`cursor`, `code`) spawn without a remount. Resume drains leftover raw-mode keys
- Fetch / pull / default do not fan out to linked worktrees unless the focused row is that worktree
- `P` pushes the focused visible repo or checkout only. Hidden ignored stay out. Linked worktrees push only when that row is focused
- `S` opens a stash overlay. `s` creates a stash (pathspec when a dirty file is focused). From a repo or file row, `a` / `p` / `d` target the latest stash. From a focused graph stash row they target that `stash@{n}`. Pop and drop ask `y` / `n` first, same as revert
- `Enter` on a graph commit (or stash / uncommitted row) opens that object's file list. `Enter` on a file opens the commit diff. `Esc` pops each level. Hidden ignored repos stay out of the drill unless shown
- Graph stash rows are first-class: `a` apply, `p` pop, and `D` drop the focused `stash@{n}`. Drop asks `y` / `n`
- `b` opens the local branch picker on a checkout or flat repo. Type to filter. Enter checks out. `C` creates a branch at HEAD. When the local branch is out of sync with `origin/*`, checkout asks `y` / `n` then pulls
- `w` / `W` removes the focused linked worktree. Workspace, repo family, file, and hidden ignored rows are a no-op. Confirm with `y` / `n`. The command is `git worktree remove [--force]` from the primary. Bind-mount aliases remap the same way as in the TypeScript app. Ink uses the same keys
- Action / Effect loop: crossterm events become `Action`, dispatch updates state and returns an `Effect`
- Mouse is optional. Keys work without it
- `--plain` / `--json` / `-v` / `-p` / `-d` stay headless

Reviewed marks use `$XDG_STATE_HOME/my-workspace-status/viewed-files.json` (same identity and fingerprint as Ink). A mark drops when the file fingerprint changes. Space toggles dirty file rows only. The viewed glyph is `◉` / `*`, not the clean `✓`. Clean `✓` paints only on the No updates group.

## Still Ink only

These stay in the TypeScript app:

- In-diff drag split and side-by-side resize
- Ink-testing e2e suite
- EasyMotion, theme cycle

See [tui-model.md](./tui-model.md) for the Ink keymap.

## Routing

`should_open_tui` is a pure function. Tests cover TTY vs flag decisions without a real TTY.

| Input | Effect |
| --- | --- |
| TTY, no headless flags | Ratatui TUI |
| `--plain` or `--json` | Snapshot text / JSON |
| `-v`, `-p`, `-d` | Headless `--plain` path (with that flag) |
| Non-TTY | Headless `--plain` unless `--json` |

## Layout

Left pane: workspace tree. Clean default-branch repos sit under a folded `No updates` group.

Right pane: graph for a repo or worktree, a unified diff for a dirty file, or the commit-files drill (file list, then that file's commit diff).

Bottom line: short status. `?` opens a small overlay of the keys above. `/` uses that line as the search prompt.
