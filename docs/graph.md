# Graph widget

`workspace-status-graph` is a ratatui widget for one git graph window.

It paints HEAD, sync, stash, and worktree markers from a `GraphModel`.
The Rust TUI paints this widget in the right pane when a repo or worktree is focused.
The crate itself does not run a terminal app. The TypeScript Ink TUI still has its own graph paint.

Interactive and headless callers share `GraphModel::visible_rows` and
`format_row` / `format_sync`. Display differs. The widget paints a
multi-lane gutter from the same model.

## Types

| Type | Role |
| --- | --- |
| `GraphModel` | Commits, stashes, worktrees, HEAD id, sync, `show_ignored`, uncommitted |
| `Commit` | Id, subject, parents, refs |
| `Stash` | `stash@{n}`, subject, parent id (`stash^1`) |
| `Worktree` | Path, HEAD id, branch, `ignored`, `is_current` |
| `SyncState` | Branch, status, ahead, behind |
| `GraphRow` | One visible row: uncommitted, stash, commit, or worktree |
| `GraphCell` | One gutter column: glyph, colour lane, role |
| `LaidOutCommit` | Lane assignment plus stem metadata for one commit |
| `GraphWidget` | Ratatui `Widget` over a `GraphModel` |
| `Action` | `ToggleShowIgnored` and `SetShowIgnored` |
| `Effect` | `None` today. Dispatch stays pure. |

`GraphModel::dispatch` applies an `Action` and returns an `Effect`.
This crate does not bind keys or run an event loop.

`GraphWidget::gutter_width` caps painted gutter columns. Topology still
uses the full lane model.

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

`GraphWidget` writes a sync header when `sync` is set, then one gutter
plus label per visible row. Commit and stash rows also paint a spacer
line under the node (densify rails, or the stash spur).

Lane assignment, parent planning, densify-left, and the
connection-to-glyph map match the Ink graph
(`docs/git-graph-topology.md`). A stash leaf sits on a free spur,
coloured by `stash^1`. It is not a fake DAG lane.

| Role | Unicode | ASCII |
| --- | --- | --- |
| Commit | `●` | `*` |
| HEAD commit | `⊙` | `@` |
| Uncommitted | `○` | `o` |
| Stash | `◇` | `s` |
| Worktree | `🔗` | `wt` |
| Ahead | `↑` | `^` |
| Behind | `↓` | `v` |

Junction glyphs use the same map as Ink (`│─╮╭╯╰┤├┬┴┼` /
`|-/\+`). Tests paint with ratatui TestBackend. They do not open a TTY.

## Test

Run `cargo test` at the repository root.
The existing TypeScript suite remains the interactive app check.
