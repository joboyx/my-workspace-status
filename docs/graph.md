# Graph widget

`workspace-status-graph` is a ratatui widget for one git graph window.

It paints HEAD, sync, stash, and worktree markers from a `GraphModel`.
The TUI paints this widget in the right pane when a repo or worktree is focused.
The crate itself does not run a terminal app.

Interactive and headless callers share `GraphModel::visible_rows` and
`format_row` / `format_sync`. Display differs. The widget paints a
multi-lane gutter from the same model. The TUI may load a focused
window (`git log <branches>` instead of `--all`) via graph `o`; the
widget still paints whatever `visible_rows` the model holds.

## Types

| Type | Role |
| --- | --- |
| `GraphModel` | Commits, stashes, worktrees, HEAD id, sync, `show_ignored`, `uncommitted`, `skip` / `limit` / `has_more` / `window` |
| `Commit` | Id, subject, parents, refs, author name, author date |
| `Stash` | Id, `stash@{n}`, subject, author name, author date, parent id (`stash^1`) |
| `Worktree` | Path, HEAD id, branch, `ignored`, `is_current`. Linked extras only (`git worktree list` / `.git` gitfile). The main checkout (`.git` directory) is not marked. |
| `SyncState` | Branch, status, ahead, behind |
| `GraphRow` | One visible row: uncommitted, stash, commit, or worktree |
| `GraphCell` | One gutter column: glyph, colour lane, role |
| `LaidOutCommit` | Lane assignment plus stem metadata for one commit |
| `GraphWidget` | Ratatui `Widget` over a `GraphModel` |
| `graph_scrollbar_thumb` | Thumb offset/length matching a painted bar (TUI hit-test, vertical or horizontal) |
| `graph_col_max` | Max `col_offset` for the longest label in the pane |
| `graph_vscroll_visible` / `graph_hscroll_visible` | Show the vertical bar only after leaving the top; the horizontal bar only after leaving the left edge |
| `Action` | `ToggleShowIgnored` and `SetShowIgnored` |
| `Effect` | `None` today. Dispatch stays pure. |

`GraphModel::dispatch` applies an `Action` and returns an `Effect`.
The widget does not bind keys or run an event loop. The TUI hit-tests the
vertical and horizontal scrollbars through `tui/split.rs` (`hit_split` /
`SplitDrag::GraphScrollbar` / `GraphHScrollbar`). The vertical bar is painted
only when `scroll > 0`; the horizontal bar only when `col_offset > 0`.

`GraphWidget::gutter_width` caps painted gutter columns. Topology still
uses the full lane model; every row shares the same left-aligned clip. `GraphWidget::loading_older` paints
`loading older…` under the list. `GraphWidget::lane_colors` colours each
gutter cell from `GraphCell.color_lane`; an empty slice uses
`DEFAULT_LANE_COLORS`. The TUI passes the active built-in theme's eight
colours (`T` cycles).
`GraphWidget::search_matches` paints the filter/search background on
selectable visible-row indexes. Spacers stay
unhighlighted. `GraphWidget::flash_rows` paints the fade background on
the same visible-row indexes, including spacers (a flashing commit
keeps its spacer). `GraphWidget::commented_rows` marks selectable
visible-row indexes that have an object comment or a file-line
comment for that row. Commented rows paint `ICON_COMMENT` (`"` /
nf-fa-comment) after the gutter. `GraphWidget::resolved_comment_rows`
paints `ICON_COMMENT_RESOLVED` (`'` / nf-fa-comment-o) when every
comment on that row is resolved. Open `commented_rows` win when a row
is in both lists. The selected cursor still wins: `▌`
plus `cursorBg` (`GraphWidget::cursor_style`). The comment glyph stays
visible on the selected row. Uncommented rows do not reserve a
column. Spacers stay unmarked. The TUI passes `icon_comment` /
`icon_comment_resolved` so the glyphs match tree and diff marks.
The widget does not use reverse video for the cursor.
`GraphWidget::col_offset` skips label columns (gutter stays put) so long
subjects can pan without growing the row.

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

`GraphWidget` uses a chrome budget (`graph_chrome_budget`): a
2-line selection footer when height ≥ 3, then a 1-line sync header if
space remains (footer wins when tight; no header when `sync` is unset).
`loading older…` takes one extra row while the next log page loads.

Footer copy (`selection_detail_lines` / `selection_detail_parts`; do not invent
other strings):

- no row: `no selection`
- uncommitted: `Working tree clean` / `Uncommitted changes`, then
  HEAD commit ref chips (same chips as the HEAD commit row), or
  `worktree · not a commit` when HEAD has none
- spacer: `…`, then `connector · not selectable`
- stash: subject, then `stash@{n} ·` short hash `·` relative date (no
  author)
- commit: subject, then ref chips `·` hash `·` author `·` date, or
  `(no refs)` when there are no chips

Footer paint reuses the commit-spacer chip runs (`LabelKind`: HEAD /
default / local / remote / tag). `GraphWidget::label_palette` colours
those the same as the row chips. Hash, date, and author stay `meta`.
Do not flatten the footer to one colour.

Then one gutter plus label per visible row. Commit and stash rows also
paint a spacer line under the node (densify rails, or the stash spur).

Each gutter cell occupies one buffer column from `color_lane`. The widget writes the rail into a fixed-x region, then clips the label in the leftover columns, so wrapping or hiding the subject cannot shift the graph.

The commit node line is subject-only. The spacer under it is
`[refs…][pad][hash][ ][date][ ][author]`: branch / tag chips on the
left (local + matching `origin/*` merge into one chip; unmatched remotes
stay as `[origin/…]`). On a commit with several refs, the checked-out
branch chip is first, then default-branch / other locals / remotes / tags.
Muted short hash / relative date / author sit on the right. Narrow panes drop hash, then date, then author, and keep refs.
Relative dates: `just now` / `Nm` / `Nh` through 3 hours, then local `YYYY-MM-DD HH:MM` (operator timezone). Search still matches that painted clock and a stable UTC `YYYY-MM-DD HH:MM`. Narrow spacers keep painting a leftover **branch or tag** chip when part of its name still fits: truncate that name with `…` and keep the brackets (`[feat…]`). `[+N]` is only the count of chips that are **fully hidden** after the visible (full or truncated) chips — a merely truncated chip does not count toward `N`, and `[+N]` is omitted when nothing else is hidden (not muted `+N`). Overflow colour/bold still apply when `N > 0`. The spacer is capped to the pane width so the row does not grow with extra refs. Long subjects clip to the pane; `h` / `l` (and Shift+Left / Shift+Right) pan the label while the gutter stays put. When the gutter cap is tighter than topology, every row shares the same left-aligned clip (`clip_gutter_shared`) so vertical rails stay in the same columns. The selection footer still lists every full ref.
The spacer is not a second selectable row; cursor, search, `j`/`k`,
and click treat it as the parent commit.

The stash node line is subject-only. The spacer under it is
`[stash@{n}][pad][hash][ ][date][ ][author]` with the same relative-date
buckets and the same hash → date → author drop order (keep `stash@{n}`).
The spacer is not a second selectable row; cursor, search, `j`/`k`,
and click treat it as the parent stash.

Lane assignment, parent planning, densify-left, and the
connection-to-glyph map match [git-graph-topology.md](./git-graph-topology.md). A stash leaf sits on a free spur,
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

Junction glyphs use `│─╮╭╯╰┤├┬┴┼` /
`|-/\+`. Tests paint with ratatui TestBackend. They do not open a TTY.

## Test

Run `cargo test --workspace` at the repository root.
