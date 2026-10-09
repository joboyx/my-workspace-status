# Graph widget

`workspace-status-graph` is a ratatui widget for one git graph window.

It paints HEAD, sync, stash, and worktree markers from a `GraphModel`.
The TUI paints this widget in the right pane when a repo or worktree is focused.
The crate itself does not run a terminal app.

Interactive and headless callers share `GraphModel::visible_rows` and
`format_row` / `format_sync`. Display differs. The widget paints a
multi-lane gutter from the same model. The TUI may load a focused
window (`git log <branches>` instead of `--all`) via `o` on the graph
list or a highlighted repo / worktree. The widget still paints
whatever `visible_rows` the model holds.
`GraphModel::visible_row_at` returns one row in the same order and
clones only that row (the focused-row lookups use it).

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
| `IconSpan` | Painted cells of one graph icon (`GraphIconKind`, row index or `None` for the sync header, x, y, width, `part`, chip `target`), from `render_with_icon_spans` |
| `GraphIconKind` | Commit / HEAD / stash node, uncommitted glyph, worktree glyph, open / resolved comment mark, row badge, local / default / remote / tag chip, checkout-and-sync mark run, detached `[HEAD]`, `[+N]`, sync header, footer more-lines hint |
| `LabelPart` | One styled run: text, `LabelKind`, and `target` (`PartTarget`: the chip the run belongs to, or `None`) |
| `PartTarget` | Chip identity: `Chip(refs)` (one ref, or a local branch and its same-name remote), `Marks { branch, checkout, remote }`, `DetachedHead`, `Overflow(hidden chips)` |
| `GraphLabelPalette` | Label colours: subject, meta, chips, overflow, worktree glyph, dirty uncommitted glyph, open and resolved comment marks, sync header ahead and behind counts |
| `graph_scrollbar_thumb` | Thumb offset/length matching a painted bar (TUI hit-test, vertical or horizontal) |
| `graph_col_max` | Max `col_offset` for the longest label in the pane, counting the space and glyph of each `row_badges` entry |
| `graph_vscroll_visible` / `graph_hscroll_visible` | Show the vertical bar whenever the painted lines overflow the list (or it has left the top); the horizontal bar only after leaving the left edge |
| `Action` | `ToggleShowIgnored` and `SetShowIgnored` |
| `Effect` | `None` today. Dispatch stays pure. |

`GraphModel::dispatch` applies an `Action` and returns an `Effect`.
The widget does not bind keys or run an event loop. The TUI hit-tests the
vertical and horizontal scrollbars through `tui/split.rs` (`hit_split` /
`SplitDrag::GraphScrollbar` / `GraphHScrollbar`). The vertical bar is painted
whenever the painted lines overflow the list, at the top too; the horizontal
bar only when `col_offset > 0`, because it paints over the last list row.
`painted_line_count` gives the painted line count without a paint (a commit
or stash row is a node line plus a spacer, other rows one line; width does
not change it). The widget decides the vertical bar from that count, then
paints once at the width the bar leaves. The TUI records the same count for
its hit boxes, so a graph frame paints the model once. The TUI pan clamp
reads the count the last frame recorded (`graph_content_len`).

`GraphWidget::gutter_width` caps painted gutter columns. Topology still
uses the full lane model; every row shares the same left-aligned clip. `GraphWidget::lane_colors` colours each
gutter cell from `GraphCell.color_lane`; an empty slice uses
`DEFAULT_LANE_COLORS`. The TUI passes the active built-in theme's eight
colours (`T` cycles).
`GraphWidget::search_matches` paints the filter/search background on
selectable visible-row indexes and paints their rails, comment mark, and
label in the filter foreground, so lane, chip, and cursor colours that
equal the background stay readable. Spacers stay
unhighlighted. `GraphWidget::flash_rows` paints the fade background on
the same visible-row indexes, including spacers (a flashing commit
keeps its spacer). Flash background wins over cursor and search.
The cursor bar (`▌`) still marks selection when `GraphWidget::cursor_bar`
is on (default). It paints on the selected commit or stash row and on its
spacer row; spacers of other rows get no bar. `cursorBg` paints when the row is not flashing
(`GraphWidget::cursor_style`). When the graph pane is unfocused the TUI
sets `cursor_bar` false and paints the thinner `▏` marker plus
`cursorBgInactive` (`GraphWidget::cursor_inactive_style`).
[`GraphWidget::selected`] still drives the selection footer. `GraphWidget::commented_rows` marks selectable
visible-row indexes that have an object comment or a file-line
comment for that row. Commented rows paint `ICON_COMMENT` (`"` /
nf-fa-comment) after the gutter. `GraphWidget::resolved_comment_rows`
paints `ICON_COMMENT_RESOLVED` (`'` / nf-fa-comment-o) when every
comment on that row is resolved. Open `commented_rows` win when a row
is in both lists. The comment glyph stays
visible on the selected row. Uncommented rows do not reserve a
column. Spacers stay unmarked. The TUI passes `icon_comment` /
`icon_comment_resolved` so the glyphs match tree and diff marks. With a
`label_palette` the mark paints in `comment` (open) or `comment_resolved`
(the TUI passes heading and muted, the tree mark's colours); without one it
paints in the bold cursor colour.
The widget does not use reverse video for the cursor.
`GraphWidget::col_offset` skips label columns (gutter stays put) so long
subjects can pan without growing the row.
`GraphWidget::row_badges` takes `(visible-row index, glyph, colour)`
entries and paints the glyph one space after the label of those selectable
rows. The badge pans and clips with the label (`col_offset`, pane width).
It does not change the gutter, and rows that are not listed reserve no
column. A search-match row paints the badge in the filter foreground.
`GraphWidget::render_with_icon_spans` returns one `GraphIconKind::Badge`
span per badge that is on screen, so the caller can hit-test it (see
**Icon spans**). A badge that is scrolled out, panned past, clipped, or under the
horizontal scrollbar is left out. Pass the same entries to `graph_col_max`
so panning can bring a badge on a long label into view. The TUI badges
`GraphRow::Worktree` rows whose painted branch is the checkout's current
branch and that have a known PR (open, approved, or merged), and records
the spans for Ctrl+click. A graph loaded before a branch switch paints the
old branch and gets no badge. Commit rows get no badge.

### Icon spans

`GraphWidget::render_with_icon_spans` paints like `render` and returns one
`IconSpan` per graph icon on screen, top to bottom: the sync header, then
per list line in paint order (node, comment mark, label glyphs and chips
left to right, badge), then the selection footer's chips and hint.

| `GraphIconKind` | Where |
| --- | --- |
| `CommitNode` / `HeadNode` / `StashNode` | The gutter cell with `CellRole::Node` on the selectable line of a commit (HEAD) or stash row. It does not pan. |
| `Uncommitted` | The `○` / `o` glyph that starts the uncommitted row label |
| `Worktree` | The worktree glyph that starts a worktree row label, and each worktree mark on a commit spacer. `part` is the index into that commit row's `worktrees` (`0` on a worktree row). The span's `row_index` is the commit row. |
| `Comment` / `ResolvedComment` | The comment mark after the gutter |
| `Badge` | A `row_badges` glyph |
| `LocalChip` / `DefaultChip` / `RemoteChip` / `TagChip` | A ref chip on a commit spacer or in the selection footer. `target` is its `PartTarget::Chip` |
| `ChipMarks` | The checkout and sync marks inside a branch chip. One run, so one span: `target` is `PartTarget::Marks` with both flags |
| `DetachedHeadChip` | Detached `[HEAD]` |
| `OverflowChip` | `[+N]`. `target` is `PartTarget::Overflow` with the target of each hidden chip |
| `SyncHeader` | The sync header text (branch and marks). `row_index` is `None` |
| `MoreBelow` | The footer's `<more_below><K>` hint, without its leading blank |

Rails, junctions, spacer gutter cells, and blanks are chrome: they have
no span. A node cut by the gutter cap, a label glyph cut by `col_offset` or
the pane edge, and any icon under the horizontal scrollbar row are left
out.

A chip is one span per run of adjacent parts with the same target, so a
branch chip with marks reads `[`, the marks, then `name]`. A chip cut by
`col_offset`, the footer's `…` truncation, or the pane edge keeps a span
over the cells that show. Spacer chips carry their commit row; footer
chips and the hint carry the selected row (the uncommitted row's footer
paints HEAD's chips). `LabelPart::icon` maps a part to its kind: row
glyphs through `LabelKind::icon`, chips through `PartTarget::icon`.

`LabelPart::target` survives `fit_chip_groups`, chip-name truncation,
`trunc_label_parts`, and `slice_label_parts`, so a pan, a narrow spacer,
and the footer keep each run's chip. Targets do not change text: the
label, the spacer, and `PaintedLine::text` are the same strings.

The label glyphs are their own `LabelPart`s, so the text of a line
(`PaintedLine::text`, the label, the spacer) does not change.
`LabelKind::UncommittedMark { dirty }` paints in `dirty` when the row has
changes and in `meta` when clean. `LabelKind::WorktreeMark { worktree }`
paints in `worktree`. On a worktree row the path and branch are the
subject, `[ignored]` is meta, and `[HEAD]` is the head mark. On a commit
spacer the worktree mark text stays meta. `LabelKind::icon` maps a part to
its `GraphIconKind` and `part`.

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

`GraphWidget` uses a chrome budget (`graph_chrome_budget_for`): a
selection footer when height ≥ 3 (a rule row plus 2 lines collapsed), then a 1-line sync header if
space remains (footer wins when tight; no header when `sync` is unset).
The footer wraps the full subject plus body by default
(`commit_msg_expand`, `selection_footer_parts`); `M` collapses it to the
two clipped lines.

The footer height is fixed. It does not follow the selected message, so
moving between commits never changes the list height:

- The footer's first row is a full-width rule that parts it from the
  list (`FOOTER_RULE_ROWS` = 1). `GraphWidget::footer_rule` sets its glyph
  and colour; the TUI passes `▁` (`_` in ASCII mode) and `borderDim`.
  Without it the widget paints `▁` / `_` in dark grey.
- Under the rule, expanded requests N message rows plus the meta row
  (`graph_footer_request`, N from `GraphWidget::commit_msg_lines`, default
  `COMMIT_MSG_LINES_DEFAULT` = 4, clamped from `COMMIT_MSG_LINES_MIN` (1)
  to `COMMIT_MSG_LINES_MAX` (20)). Collapsed requests 2. The request
  counts the rule (N + 2 expanded, 3 collapsed). The TUI passes
  its session N (`AppState::commit_msg_lines`: `viewDefaults.commitMessageLines`,
  then `-` / `+`) to both the widget and `graph_footer_request`.
- `graph_chrome_budget_for` clamps the rows under the rule to at most
  half the pane (never under 2), adds the rule row, and leaves the list
  at least one row. `GraphChromeBudget::footer_height` counts the rule;
  `footer_body_height` is the rows under it, the height
  `footer_message_scroll_max` takes. A pane under 3 rows drops the footer
  and its rule. The budget depends only on the pane and the
  request. The app calls the same `graph_footer_request`, so its layout
  matches the paint.
- The meta row is always on the footer's bottom row. A short message
  paints from the top and leaves blank rows above the meta row.
- A taller message scrolls: `commit_msg_scroll` sets the first message
  row and a 1-column scrollbar marks the position
  (`footer_message_scroll_max`). While lines are hidden below, the last
  visible message row ends with a muted `<more_below><K>` hint
  (`GlyphSet::more_below`: `` U+F103, `v<K>` with ASCII glyphs; K = hidden
  lines below) left of the scrollbar column. It is not the behind glyph.
  It covers the message text under it. At the end of the message there is
  no hint.

List rows stay one line.
The pane has no `loading older…` row: the status line shows it while the next
log page loads, and the list keeps its height.

Footer copy (`selection_detail_lines` / `selection_detail_parts`; do not invent
other strings):

- no row: `no selection`
- uncommitted: `Working tree clean` / `Uncommitted changes`, then
  HEAD commit ref chips (same chips as the HEAD commit row), or
  `worktree · not a commit` when HEAD has none
- spacer: `…`, then `connector · not selectable`
- stash: subject, then `stash@{n} ·` short hash `·` relative date (no
  author). Expanded: wrap subject plus body (usually empty), then meta
- commit: subject, then ref chips (or `(no refs)` when there are no
  chips) `·` hash `·` parents `·` author `·` date. Parents use short ids
  in git order: `parent <id>` for one, `parents <id> <id>` for a merge
  (first parent first), `root commit` for none. A parent outside the
  loaded window still shows its id. Expanded: wrap subject plus body,
  then the same meta line

Footer paint reuses the commit-spacer chip runs (`LabelKind`: HEAD /
default / local / remote / tag) and their `target`s. A worktree row's
branch run is a local branch chip. `GraphWidget::label_palette` colours
those the same as the row chips. Hash, parents, date, and author stay `meta`.
Do not flatten the footer to one colour.

The sync header is `branch` then the ahead / behind marks
(`format_sync`), or `branch no-upstream`. With a `label_palette` the
branch paints in `branch_default` or `branch_local`, the ahead count in
`ahead`, the behind count in `behind`, and `no-upstream` in `meta`.
Without a palette it keeps the terminal default colour.

Then one gutter plus label per visible row. Commit and stash rows also
paint a spacer line under the node (densify rails, or the stash spur).

Each gutter cell occupies one buffer column from `color_lane`. The widget writes the rail into a fixed-x region, then clips the label in the leftover columns, so wrapping or hiding the subject cannot shift the graph.

The commit node line is subject-only. The spacer under it is
`[refs…][pad][hash][ ][date][ ][author]`: branch / tag chips on the
left (local + matching `origin/*` merge into one chip; unmatched remotes
stay as `[origin/…]`). On a commit with several refs, the checked-out
branch chip is first, then default-branch / other locals / remotes / tags.
Muted short hash / relative date / author sit on the right. Narrow panes drop hash, then date, then author, and keep refs.
Relative dates: `just now` / `Nm` / `Nh` through 3 hours, then local `YYYY-MM-DD HH:MM` (operator timezone). Search still matches that painted clock and a stable UTC `YYYY-MM-DD HH:MM`. Narrow spacers keep painting a leftover **branch or tag** chip when part of its name still fits: truncate that name with `…` and keep the brackets (`[feat…]`). `[+N]` is only the count of chips that are **fully hidden** after the visible (full or truncated) chips — a merely truncated chip does not count toward `N`, and `[+N]` is omitted when nothing else is hidden (not muted `+N`). Overflow colour/bold still apply when `N > 0`. The spacer is capped to the pane width so the row does not grow with extra refs. Long subjects clip to the pane; `h` / `l` (and Shift-Left / Shift-Right) pan the label while the gutter stays put. When the gutter cap is tighter than topology, every row shares the same left-aligned clip (`clip_gutter_shared`) so vertical rails stay in the same columns. The selection footer still lists every full ref.
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
| Ahead (sync header) | `` U+F062 | `^` |
| Behind (sync header) | `` U+F063 | `v` |
| More lines below (footer hint) | `` U+F103 | `v` |

Ahead and behind are the workspace tree's sync glyphs (one catalog entry
each). The footer hint has its own glyph: it counts message lines, not
upstream commits.

Junction glyphs use `│─╮╭╯╰┤├┬┴┼` /
`|-/\+`. Tests paint with ratatui TestBackend. They do not open a TTY.

## Test

Run `cargo test --workspace` at the repository root.
