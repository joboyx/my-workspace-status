# Rust TUI

`workspace-status` and `ws` from `crates/workspace-status` open a ratatui TUI when stdout is a TTY and you did not pass `--plain`, `--json`, `-v`, `-p`, or `-d`.

A non-TTY run without those flags still prints `--plain`. Agents must pass `--plain` or `--json`.

The TypeScript Ink app stays in this repository. Use it when you need a feature that this TUI does not implement yet.

Screenshots of the daily views live in the [root README](../README.md#screenshots).
To rebuild those frames, seed the workspace in [demo.md](./demo.md).

## Daily keys

| Key | Action |
| --- | --- |
| `q` | Quit |
| `?` | Help overlay (short list, not a wall of text) |
| `j` / `k` or arrows | Move the tree. On a focused graph, move the graph cursor. On a file list, move the file. On a focused file diff, scroll the diff |
| `z` | Toggle fold |
| `t` | Toggle directory tree / flat paths on the workspace tree. Status is `Directory tree` / `Flat paths`. No-op in a commit-files or commit-diff drill |
| `h` / `l` or left / right | Close / open fold. Space does not fold |
| `.` | Show or hide ignored repos |
| `/` | Search prompt. Enter arms the query. Esc cancels |
| `n` / `N` | Next / previous search match (previous is `N`, not `p`) |
| `s` | Stage dirty files in the focused scope (file, repo, dir, checkout, or workspace). Workspace writes every scoped file across repos |
| `u` | Unstage dirty files in the focused scope |
| `x` | Revert dirty files in the focused scope. `y` discards tracked (keeps untracked except a sole untracked file). `Y` also deletes untracked |
| `e` | Edit the focused file (`$EDITOR` or config `editor`) |
| `f` | Fetch visible targets |
| `p` | Pull visible targets that are behind |
| `d` | Switch visible targets that are off the default branch. Already-default is a no-op (does not pull) |
| `P` | Push the focused visible repo or checkout when it is ahead, diverged, or has no upstream. In-sync is a no-op |
| `S` | Stash menu. On a dirty file or repo this is create-only (`s`). On a clean repo it is a no-op. Apply / pop / drop only from a focused graph stash row (`a` / `p` / `D`, or `S` there) |
| `Enter` | Focus the right pane, or drill: graph commit → file list → commit diff |
| `Esc` | Pop one drill level, or return focus to the tree |
| `a` / `p` / `D` | Apply / pop / drop the focused graph stash row. Drop asks `y` / `n` |
| `b` | Tree: local branch picker (list, filter, checkout, `C` create at HEAD). Graph commit: checkout refs on that commit (one name checks out, several open a name picker). Dirty tree refuses with commit-or-stash |
| `c` | Graph commit: create a branch at that commit (`git branch -- name commitId`). No checkout. No-op when a graph commit is not focused |
| `r` | Reload the workspace snapshot |
| `space` | Mark a dirty file as reviewed. Writes the same viewed-files store as the Ink app |
| `w` / `W` | Remove the focused linked worktree after `y` / `n` |
| `Tab` | Focus the other pane |
| click | Select a tree row, or focus the right pane |
| drag | Resize the tree / right pane split, or the in-diff side-by-side RULE |
| `i` | Toggle inline / split on a file diff. Split falls back to inline below 100 columns |
| `g` / `G` | First / last tree row |
| `;` or Ctrl+Space | EasyMotion on the focused list (tree, graph, or commit files). Labels `a`–`z` then `aa`… on the current viewport only. Type a label to jump. Esc cancels. Diff-focused start is a no-op |
| `T` | Cycle the built-in colour theme. Wraps. Seed from `WS_STATUS_THEME`; the cycle stays in this session (same as Ink — there is no theme file) |

`-a` starts with ignored repos shown. `-f` starts a fetch after the first paint. First paint does not wait on a network fetch.

`WS_STATUS_WATCH_MS` polls local git and refreshes the snapshot. Default is `3000`. `0` disables the poll. Fold, focus, and scroll stay put. Only rows whose identity actually changed flash.

`WS_STATUS_FETCH_MS` runs `git fetch` on visible primary checkouts. Default is `300000` (5 minutes). `0` disables it. The watch poll stays a separate timer. Hidden ignored repos stay out. Linked worktrees are not fetched unless you focus that row and press `f`.

## What this TUI does

- Tree of repos, linked worktrees, and dirty files from the same snapshot builder as `--plain` / `--json`
- Files sit in a directory trie by default. `t` toggles tree / flat on the workspace. Status is `Directory tree` / `Flat paths`
- Dir rows fold with `z` / `h` / `l`. A dir `s` / `u` / `x` writes files under that dir
- Tree / flat stays in this session. Rust has no session store. Commit-file lists stay flat
- Right pane: file diff when a dirty file is focused. Graph pane via `workspace-status-graph` when a repo or worktree is focused
- Hidden ignored repos stay out of the tree, search, stage / unstage / revert, and fetch / pull / push / default unless you show them
- Search matches include folded rows. Focusing a match unfolds its ancestors
- Stage / unstage / revert act on every dirty file in the focused scope (file, dir, flat repo, checkout, or workspace), including every scoped file across repos. A file row stays single-file. A dir row writes the dir path and its children. Family containers do not mix linked worktree files. Hidden ignored stay out unless shown. Revert asks `y` / `Y` / `n` on the status line. `y` discards tracked and keeps untracked, except a sole untracked file which still deletes. `Y` also deletes untracked. Stage and unstage do not confirm
- `e` uses config `editor`, then `$EDITOR`, then `$VISUAL`, then `vim`. A TTY editor leaves the alternate screen and returns to the same fold, focus, and scroll. GUI editors (`cursor`, `code`) spawn without a remount. Resume drains leftover raw-mode keys
- Fetch / pull / default do not fan out to linked worktrees unless the focused row is that worktree
- `P` pushes the focused visible repo or checkout only when it is ahead, diverged, or has no upstream. In-sync is a no-op. Workspace never fans out. Hidden ignored stay out. Linked worktrees push only when that row is focused
- `S` on a dirty file or repo is create-only (`s stash`). `S` on a clean repo that has `stash@{0}` is a no-op. Apply / pop / drop only from a focused graph stash row (`a` / `p` / `D`, plus `S` there). Never drop latest from a file or repo row. Pop and drop ask `y` / `n` first
- `d` switches visible targets that are off the default branch. Already-default is a no-op and does not pull. Dirty trees still skip
- `Enter` on a graph commit (or stash / uncommitted row) opens that object's file list. `Enter` on a file opens the commit diff. `Esc` pops each level. Hidden ignored repos stay out of the drill unless shown
- Graph stash rows are first-class: `a` apply, `p` pop, and `D` drop the focused `stash@{n}`. Drop asks `y` / `n`
- `b` on a tree checkout or flat repo opens the local branch picker. Type to filter. Enter checks out. `C` creates a branch at HEAD and checks it out. When the local branch is out of sync with `origin/*`, checkout asks `y` / `n` then pulls
- `b` on a focused graph commit checks out that commit's local and `origin/*` refs. One name checks out. Several names open a picker of those names only. A dirty tree refuses (`Dirty worktree — commit or stash first`). Scope is the checkout that owns the graph. Hidden ignored stay out
- `c` on a focused graph commit opens a name prompt and runs `git branch -- <name> <commitId>`. It does not check the new branch out. `c` is a no-op on a tree, file, or workspace row. Picker `C` still creates and checks out at HEAD
- `w` / `W` removes the focused linked worktree. Workspace, repo family, file, and hidden ignored rows are a no-op. Confirm with `y` / `n`. The command is `git worktree remove [--force]` from the primary. Bind-mount aliases remap the same way as in the TypeScript app. Ink uses the same keys
- Action / Effect loop: crossterm events become `Action`, dispatch updates state and returns an `Effect`
- EasyMotion (`;` / Ctrl+Space) labels the currently visible rows on the focused list (tree, graph, or commit files). Prefix matching is partial / hit / miss. Hit jumps. Esc or a miss cancels and keeps the cursor. A focused diff does not start the overlay
- `T` cycles Tokyo Night → Monokai → Dracula → Gruvbox Dark → Catppuccin Mocha → Tokyo Night. Launch seed is `WS_STATUS_THEME`. The cycle stays in the current session; neither this TUI nor Ink writes a theme file
- Mouse is optional. Keys work without it. Drag the tree / right splitter, or the in-diff RULE in split mode, to resize. `i` toggles inline / split without a mouse
- Tree / right and in-diff split ratios stay in the current session only. They reset on the next launch (Ink does not persist them either)
- `--plain` / `--json` / `-v` / `-p` / `-d` stay headless

Reviewed marks use `$XDG_STATE_HOME/my-workspace-status/viewed-files.json` (same identity and fingerprint as Ink). A mark drops when the file fingerprint changes. Space toggles dirty file rows only. The viewed glyph is `◉` / `*`, not the clean `✓`. Clean `✓` paints only on the No updates group.

## Optional Ink-only

The TypeScript Ink app stays in this repository, including its ink-testing e2e suite. That suite is optional. Daily-path TUI coverage for agents is `cargo test` (`crates/workspace-status/tests/tui_daily_e2e.rs`) on a TestBackend — no TTY.

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

Right pane: graph for a repo or worktree, a unified or side-by-side diff for a dirty file, or the commit-files drill (file list, then that file's commit diff).

The tree / right split defaults to 40% tree. The in-diff RULE defaults to 50/50. Drag either splitter (3-column grab band). Neither pane or column can collapse to zero. Both ratios are session-only.

Bottom line: short status. `?` opens a small overlay of the keys above. `/` uses that line as the search prompt.
