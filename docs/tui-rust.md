# Rust TUI

`workspace-status` and `ws` from `crates/workspace-status` open a ratatui TUI when stdout is a TTY and you did not pass `--plain`, `--json`, `-v`, `-p`, or `-d`.

A non-TTY run without those flags still prints `--plain`. Agents must pass `--plain` or `--json`.

The TypeScript Ink app stays in this repository. Use it when you need a feature that this TUI does not implement yet.

## Daily keys

| Key | Action |
| --- | --- |
| `q` | Quit |
| `?` | Help overlay (short list, not a wall of text) |
| `j` / `k` or arrows | Move the tree. On a focused file diff, scroll the diff |
| `z` | Toggle fold |
| `h` / `l` or left / right | Close / open fold. Space does not fold |
| `.` | Show or hide ignored repos |
| `f` | Fetch visible targets |
| `p` | Pull visible targets that are behind |
| `d` | Switch visible targets to the default branch |
| `r` | Reload the workspace snapshot |
| `space` | Mark a dirty file as reviewed (in memory only) |
| `Tab` | Focus the other pane |
| click | Select a tree row, or focus the right pane |
| `g` / `G` | First / last tree row |

`-a` starts with ignored repos shown. `-f` starts a fetch after the first paint. First paint does not wait on a network fetch.

## What this TUI does

- Tree of repos, linked worktrees, and dirty files from the same snapshot builder as `--plain` / `--json`
- Right pane: file diff when a dirty file is focused. Graph pane via `workspace-status-graph` when a repo or worktree is focused
- Hidden ignored repos stay out of the tree and out of fetch / pull / default unless you show them
- Fetch / pull / default do not fan out to linked worktrees unless the focused row is that worktree
- Action / Effect loop: crossterm events become `Action`, dispatch updates state and returns an `Effect`
- Mouse is optional. Keys work without it
- `--plain` / `--json` / `-v` / `-p` / `-d` stay headless

Reviewed marks last for this process only. They do not write the Ink viewed-files store.

## Still Ink only

These stay in the TypeScript app:

- Stash pop / drop / stash menu
- Branch picker and graph checkout / create branch
- Worktree remove
- Edit in editor and remount
- In-diff drag split and side-by-side resize
- Commit-files drill (depth 1 / depth 2)
- Persisted reviewed store
- Live watch poll and background fetch timer
- Ink-testing e2e suite
- Multi-lane graph gutter (the crate still paints a single lane)
- Search (`/` `n` `N`), EasyMotion, theme cycle, stage / unstage / revert, push (`P`)

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

Right pane: graph for a repo or worktree, or a unified diff for a dirty file.

Bottom line: short status. `?` opens a small overlay of the keys above.
