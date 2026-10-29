# Tree and graph

The left pane is the workspace tree: one row per checkout, with its PR badge as the last item of the trailing run. The right pane follows the cursor: a checkout or folder row shows that checkout's commit graph, a file row shows its diff. `Tab` moves the focus bar between the panes. The graph pane also carries the PR of the focused checkout.

Setup (the prelude with `V`, `FORGE`, `RUN`, `fresh`, `relaunch`, `stop_run` and the `to_*` helpers), the `trap` that stops the last run, how to run it when every command gets a fresh shell, and the PS ids: [pr-status.md](pr-status.md). Every recipe starts its own run with `fresh`, which stops the previous one before it resets the scenario files.

## TG1 Badges in the tree, and the pane follows the cursor

```bash
fresh context github-review gitlab-ready -- --fake-forge "$FORGE"
$V wait --regex "$RUN" '!5 A($|[^[:alnum:]])'
$V capture "$RUN" tree-wide
to_app
$V wait "$RUN" 'Add token refresh window'            # the graph of app is on the right
$V keys "$RUN" Tab
$V wait --cursor "$RUN" 'uncommitted changes'        # the focus bar is in the graph now
$V capture "$RUN" graph-focus
$V keys "$RUN" Tab
$V wait --cursor "$RUN" '& feature/auth-refresh'     # and back in the tree
```

Expect on `tree-wide` (5 rows with a badge, each ending in `<ref> <glyph>` after its counts):

| Row | Text |
| --- | --- |
| `app` checkout | `& feature/auth-refresh` ... `#7 P` |
| `app` worktree | `L feature/login-page` ... `#8 P` |
| `merger` | `& feature/reconciliation` ... `#12 m` |
| `merger` worktree | `L feature/reconciliation` ... `#12 m` |
| `services/api` | `feature/rate-limit` ... `!5 A` |

The `lib` row (local remote, no forge) has no badge.

**PASS:** the badges are on the rows above in the captured screen; the graph shows commits of `app` (`Add token refresh window`) when the cursor is on the `app` row; `Tab` moves the focus bar into the graph and the second `Tab` back.

## TG2 Glyph column at wide and narrow widths

The glyph is the last cell of every badge, so all glyphs of a capture stand in one column. Under 50 tree columns the refs drop on every row and only glyphs stay. The tree is 40% of the terminal width: the default `160x45` gives a 64-column tree, `100x40` gives 40 columns, and a tree of exactly 50 columns inside its border needs 130 terminal columns.

```bash
fresh context github-review gitlab-ready -- --fake-forge "$FORGE"
$V wait --regex "$RUN" '!5 A($|[^[:alnum:]])'
$V capture "$RUN" tree-wide2
WIDE=$(ls "$RUN"/evidence/*-tree-wide2.txt)
fresh context github-review gitlab-ready -- --size 100x40 --fake-forge "$FORGE"   # stops the wide run first
$V wait --regex "$RUN" 'feature/auth-refresh.* P($|[^[:alnum:]])'   # the app badge (in review) on either layout
$V wait --regex "$RUN" 'services/api.* A($|[^[:alnum:]])'            # the api row clips its branch name when narrow
$V capture "$RUN" tree-narrow
NARROW=$(ls "$RUN"/evidence/*-tree-narrow.txt)
```

Look at the tree part of the five rows that carry a badge (the first 64 columns of `tree-wide2`, the first 40 of `tree-narrow`). The `services/api` row names its branch only when there is room, so the rows are picked by branch or by `services/api`:

```bash
badge_rows() { # badge_rows <tree width> <capture>: the tree part of each badge row, trailing blanks cut
  cut -c1-"$1" "$2" | grep -E 'feature/(auth-refresh|login-page|reconciliation)|services/api' | sed 's/ *$//'
}
badge_rows 64 "$WIDE" | awk '{print length($0)}' | sort -u     # wide: one value (the glyph column)
badge_rows 40 "$NARROW" | awk '{print length($0)}' | sort -u   # narrow: one value
badge_rows 40 "$NARROW" | grep -cE '[#!][0-9]+'                # narrow: 0 refs
```

**PASS:** the five rows of `tree-wide2` (tree width 64) have one length, and so do the five rows of `tree-narrow` (tree width 40): that length is the glyph column. The wide rows end in `#7 P`, `#8 P`, `#12 m`, `#12 m`, `!5 A`; on `tree-narrow` no row has a `#7`, `#8`, `#12` or `!5` (the count is 0), and each row ends in its glyph.

## TG3 Graph chip and workspace total

With `github-failed gitlab-failed` and the cursor on the `app` row, the graph pane's branch header line shows, right aligned, `<glyph> <ref> <state label>  ▸ <next action>` (ASCII `>`). The workspace row ends in the PR total: `<n> PRs · <m> need you` when it has room, else `<m> need you` (the seeded `5 changed · 1 ahead, 1 diverged` summary takes the room at the default width).

```bash
fresh context github-failed gitlab-failed -- --fake-forge "$FORGE"
$V wait --regex "$RUN" '2 need you($|[^[:alnum:]])' 10   # the total on the workspace row
to_app
$V wait "$RUN" 'x #7 CI failed  > fix CI' 10            # the chip, on the graph header line of app
$V capture "$RUN" chip-and-total
fresh context github-failed gitlab-failed -- --size 80x40 --fake-forge "$FORGE"
$V wait --regex "$RUN" '#7 x($|[^[:alnum:]])' 10
to_app
$V wait --regex "$RUN" 'x #7' 10
$V capture "$RUN" chip-narrow
```

**PASS:** both lines are on `chip-and-total`. On `chip-narrow` (80x40, a 48-column graph pane) the chip is shorter than the whole `x #7 CI failed  > fix CI`: it has dropped `> fix CI` and keeps `x #7 CI failed` (a narrower pane drops the state label next). At 100x40 the pane still fits the whole chip.
