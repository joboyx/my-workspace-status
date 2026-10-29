# Tree and graph

The left pane is the workspace tree: one row per checkout, with its PR badge in the trailing run. The right pane follows the cursor: a checkout or folder row shows that checkout's commit graph, a file row shows its diff. `Tab` moves the focus bar between the panes. The graph pane also carries the PR of the focused checkout.

Setup (the prelude with `V`, `FORGE`, `RUN`, `fresh`, `relaunch`, `stop_run` and the `to_*` helpers), the `trap` that stops the last run, how to run it when every command gets a fresh shell, the PS ids, and the **needs phase A** meaning: [pr-status.md](pr-status.md). Every recipe starts its own run with `fresh`, which stops the previous one before it resets the scenario files.

## TG1 Badges in the tree, and the pane follows the cursor

```bash
fresh context github-review gitlab-ready -- --fake-forge "$FORGE"
$V wait --regex "$RUN" 'feature/rate-limit +& A'     # today; final: wait "$RUN" '!5 A'
$V capture "$RUN" tree-wide
to_app
$V wait "$RUN" 'Add token refresh window'            # the graph of app is on the right
$V keys "$RUN" Tab
$V wait --cursor "$RUN" 'uncommitted changes'        # the focus bar is in the graph now
$V capture "$RUN" graph-focus
$V keys "$RUN" Tab
$V wait --cursor "$RUN" '& feature/auth-refresh'     # and back in the tree
```

Expect on `tree-wide`, today (5 rows with a badge, the badge before the `^1 3` counts):

| Row | Text |
| --- | --- |
| `app` checkout | `& feature/auth-refresh` then `P` |
| `app` worktree | `L feature/login-page` then `o P` |
| `merger` | `& feature/reconciliation` then `m` |
| `merger` worktree | `L feature/reconciliation` then `o m` |
| `services/api` | `feature/rate-limit` then `& A` |

Final (**needs phase A**): the badge moves to the end of each trailing run and gets its ref, glyph flush against the tree edge: `#7 P`, `#8 P`, `#12 m`, `#12 m`, `!5 A`. The `lib` row (local remote, no forge) has no badge.

**PASS:** the badges are on the rows above in the captured screen; the graph shows commits of `app` (`Add token refresh window`) when the cursor is on the `app` row; `Tab` moves the focus bar into the graph and the second `Tab` back.

## TG2 Glyph column at wide and narrow widths (needs phase A)

The glyph is the last cell of every badge, so all glyphs of a capture stand in one column. Under 50 tree columns the refs drop on every row and only glyphs stay. A tree pane of 50 columns needs about 125 terminal columns; `100x40` gives about 38.

```bash
fresh context github-review gitlab-ready -- --size 100x40 --fake-forge "$FORGE"   # stops the wide run first
$V wait --regex "$RUN" 'feature/auth-refresh.* P($|[^[:alnum:]])'   # the app badge (in review) on either layout
$V capture "$RUN" tree-narrow
```

**PASS:** on `tree-wide` and on `tree-narrow` the badge glyphs of the five rows are in the same column. On `tree-narrow` no `#7`, `#8`, `#12` or `!5` is in the tree pane. Today's build passes only the part that the badges are on the rows (`tree-narrow` still shows `P` on the `app` and `app` worktree rows).

## TG3 Graph chip and workspace total (needs phase A)

With `github-failed gitlab-failed` and the cursor on the `app` row, the graph pane's branch header line shows, right aligned, `<glyph> <ref> <state label>  ▸ <next action>`. The workspace row shows `<n> PRs · <m> need you`.

```bash
fresh context github-failed gitlab-failed -- --fake-forge "$FORGE"
$V wait --regex "$RUN" 'PRs · 2 need you' 10          # the total on the workspace row
to_app
$V wait --regex "$RUN" 'x #7 CI failed.*fix CI' 10    # the chip, on the graph header line of app
$V capture "$RUN" chip-and-total
$V stop "$RUN"
```

**PASS:** both lines are on the capture. On a `100x40` run (`relaunch --size 100x40 ...`) the chip drops `▸ <next action>` first, then the state label.
