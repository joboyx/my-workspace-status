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
| `GraphModel` | Commits, stashes, worktrees, HEAD id, sync, `show_ignored`, `uncommitted`, `skip` / `limit` / `has_more` / `window` |
| `Commit` | Id, subject, parents, refs, author name, author date |
| `Stash` | Id, `stash@{n}`, subject, author name, author date, parent id (`stash^1`) |
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
uses the full lane model. `GraphWidget::loading_older` paints
`loading older…` under the list. `GraphWidget::lane_colors` colours each
gutter cell from `GraphCell.color_lane`; an empty slice uses
`DEFAULT_LANE_COLORS` (Ink `laneColors.ts`).

## Visible rows

1. A working-tree row when `uncommitted` is `Some(has_changes)`. A loaded
   graph always sets this (dirty or clean). Fixtures may use `None` to omit it.
2. Stashes whose parent is outside the loaded window.
3. Each commit, newest first. Stashes whose `parent_id` matches sit
   immediately above that commit.
4. Worktrees whose HEAD is a loaded commit attach to that commit row.
5. Other worktrees become their own rows.

Hidden ignored worktrees stay out of this list unless `show_ignored`
is true. That matches the workspace snapshot rule: ignored checkouts
stay out of ops unless shown.

## Paint

`GraphWidget` uses the same chrome budget as Ink `graphChromeBudget`: a
2-line selection footer when height ≥ 3, then a 1-line sync header if
space remains (footer wins when tight; no header when `sync` is unset).
`loading older…` takes one extra row while the next log page loads.

Footer copy matches Ink `graphSelectionDetailLines`:

- uncommitted: `Working tree clean` / `Uncommitted changes`, then
  `worktree · not a commit`
- stash: subject, then `stash@{n} ·` short hash `·` relative date (no
  author)
- commit: subject, then ref chips `·` hash `·` author `·` date

Then one gutter plus label per visible row. Commit and stash rows also
paint a spacer line under the node (densify rails, or the stash spur).

Each gutter cell is a styled span from `color_lane` (Ink
`cellsToSegments`). Adjacent cells with the same lane colour merge.

The commit node line is subject-only. The spacer under it is
`[refs…][pad][hash][ ][date][ ][author]`: branch / tag chips on the
left (local + matching `origin/*` merge into one chip; unmatched remotes
stay as `[origin/…]`), muted short hash / relative date / author on the
right. Narrow panes drop hash, then date, then author, and keep refs.
Relative dates match Ink (`just now` / `Nm` / `Nh` / `Nd` / `Nw` / `Ny`).
The spacer is not a second selectable row; cursor, search, EasyMotion,
and click treat it as the parent commit.

The stash node line is subject-only. The spacer under it is
`[stash@{n}][pad][hash][ ][date][ ][author]` with the same relative-date
buckets and the same hash → date → author drop order (keep `stash@{n}`).
The spacer is not a second selectable row; cursor, search, EasyMotion,
and click treat it as the parent stash.

Lane assignment, parent planning, densify-left, and the
connection-to-glyph map match the Ink graph
(`docs/git-graph-topology.md`). A stash leaf sits on a free spur,
coloured by `stash^1`. It is not a fake DAG lane. The widget paints that
lane colour; it does not flatten the gutter to one unstyled span.

| Role | Unicode | ASCII |
| --- | --- | --- |
| Commit | `●` | `*` |
| HEAD commit | `⊙` | `@` |
| Uncommitted | `○` | `o` |
| Stash | `◇` | `s` |
| Worktree | `` | `L` |
| Ahead | `↑` | `^` |
| Behind | `↓` | `v` |

Junction glyphs use the same map as Ink (`│─╮╭╯╰┤├┬┴┼` /
`|-/\+`). Tests paint with ratatui TestBackend. They do not open a TTY.

## Test

Run `cargo test` at the repository root.
The existing TypeScript suite remains the interactive app check.
