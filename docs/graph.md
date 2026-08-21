# Graph widget

`workspace-status-graph` is a ratatui widget for one git graph window.

It paints HEAD, sync, stash, and worktree markers from a `GraphModel`.
The Rust TUI paints this widget in the right pane when a repo or worktree is focused.
The crate itself does not run a terminal app. The TypeScript Ink TUI still has its own graph paint.

Interactive and headless callers share `GraphModel::visible_rows` and
`format_row` / `format_sync`. Display differs.

## Types

| Type | Role |
| --- | --- |
| `GraphModel` | Commits, stashes, worktrees, HEAD id, sync, `show_ignored`, uncommitted |
| `Commit` | Id, subject, parents, refs |
| `Stash` | `stash@{n}`, subject, parent id (`stash^1`) |
| `Worktree` | Path, HEAD id, branch, `ignored`, `is_current` |
| `SyncState` | Branch, status, ahead, behind |
| `GraphRow` | One visible row: uncommitted, stash, commit, or worktree |
| `GraphWidget` | Ratatui `Widget` over a `GraphModel` |
| `Action` | `ToggleShowIgnored` and `SetShowIgnored` |
| `Effect` | `None` today. Dispatch stays pure. |

`GraphModel::dispatch` applies an `Action` and returns an `Effect`.
This crate does not bind keys or run an event loop.

## Visible rows

1. An uncommitted row when `uncommitted` is true.
2. Stashes whose parent is outside the loaded window.
3. Each commit, newest first. Stashes whose `parent_id` matches sit
   immediately above that commit.
4. Worktrees whose HEAD is a loaded commit attach to that commit row.
5. Other worktrees become their own rows.

Hidden ignored worktrees stay out of this list unless `show_ignored`
is true. That matches the workspace snapshot rule: ignored checkouts
stay out of ops unless shown.

## Paint

`GraphWidget` writes one line per visible row, plus a sync header when
`sync` is set.

| Role | Unicode | ASCII |
| --- | --- | --- |
| Commit | `●` | `*` |
| HEAD commit | `⊙` | `@` |
| Uncommitted | `○` | `o` |
| Stash | `◇` | `s` |
| Worktree | `🔗` | `wt` |
| Ahead | `↑` | `^` |
| Behind | `↓` | `v` |

Unicode matches `docs/git-graph-topology.md` for node glyphs.
This widget uses a single lane. Multi-lane gutter topology stays in
the TypeScript graph under `src/tui/graph/`.

Tests paint with ratatui TestBackend. They do not open a TTY.

## Test

Run `cargo test` at the repository root.
The existing TypeScript suite remains the interactive app check.
