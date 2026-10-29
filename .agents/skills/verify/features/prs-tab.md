# PRs tab

`gp` (or the palette row `PRs tab`, or a click on the workspace PR total) opens the PRs tab: every PR of the checked-out branches in one list, grouped by ticket key, beside a detail pane. A second `gp` focuses it; `Ctrl-w` closes it. On the tab: `j` / `k` / `gg` / `G` move, Enter opens the PR, `gc` opens its CI page, `o` shows its checkout in the Workspace tab, `z` folds the group under the cursor, `Z` folds every finished group, `r` looks every PR up again, `y` copies the URL, and `Tab` switches list and detail in a frame under 100 columns (there the move keys scroll the shown detail). The list and the detail each paint a vertical scrollbar in their last column while their lines overflow; the wheel over the detail scrolls it, and another PR starts its detail at the top. Reference: `docs/tui-rust.md`, **PRs tab**; the title command is `WS_STATUS_PR_TITLE_CMD` in `docs/configuration.md`.

A build without the PRs tab (`gp` opens nothing, no `PRs <n>` tab label) cannot pass: report SKIP and name the missing piece.

## Setup

Use the prelude of [pr-status.md](pr-status.md), **Setup**, and its **One shell or many** rules. Add these helpers to the prelude file:

```bash
# <screen> | in_order <text>...: each text sits on a later screen line than the text before it.
in_order() {
  local scr prev=0 n t
  scr=$(cat)
  for t in "$@"; do
    n=$(grep -nF -- "$t" <<<"$scr" | head -n 1 | cut -d: -f1)
    if [ -z "$n" ] || [ "$n" -le "$prev" ]; then echo "FAIL order at '$t'"; return 1; fi
    prev=$n
  done
  echo "PASS order"
}
# PRs tab cursor: top goes to the first line; walk presses j once per row text, waiting for each.
top() { $V keys "$RUN" g g && $V wait --cursor "$RUN" 'v ABC-123'; }
walk() { local row; for row in "$@"; do $V keys "$RUN" j && $V wait --cursor "$RUN" "$row" || return 1; done; }
```

The `prs-tab` set gives each seeded branch a PR whose title carries a ticket key, so the tab makes groups:

| Checkout | PR | State | Title | Group |
| --- | --- | --- | --- | --- |
| `app`, `feature/auth-refresh` | `#7` | CI failed | `fix(ABC-123): Refresh auth tokens before expiry` | `ABC-123` (commit scope) |
| `app` worktree, `feature/login-page` | `#8` | in review, base `feature/auth-refresh` | `feat(ABC-123): Add the login page` | `ABC-123`, stacked on `#7` |
| `services/api`, `feature/rate-limit` | `!5` | CI running | `[ABC-125] Rate limit the public API` | `ABC-125` (`[KEY]` prefix) |
| `merger` and its worktree, `feature/reconciliation` | `#12` | merged | `ABC-120: Reconcile billing runs` | `ABC-120` (`KEY:` prefix) |

`lib` has no forge remote and no PR. The tab lists 4 PRs: `#12` shows from two checkouts but is one PR. The `wait` texts below are ASCII mode (the driver default) except PT10.

## PT1 `gp` opens the tab, groups in order

```bash
fresh prs-tab -- --fake-forge "$FORGE"
$V wait --regex "$RUN" '!5 r($|[^[:alnum:]])' && $V wait --regex "$RUN" '#12 m($|[^[:alnum:]])'   # every lookup landed
$V keys "$RUN" g p
$V wait "$RUN" 'PRs 4' && $V wait "$RUN" 'v ABC-120 · m merged'
$V capture "$RUN" groups | in_order 'v ABC-123 · > fix CI #7' 'x #7   acme/app' 'P #8   acme/app' \
  'v ABC-125 · > wait for CI !5' 'r !5   acme/servic…' 'v ABC-120 · m merged' 'm #12  acme/merger'
$V wait --cursor "$RUN" 'v ABC-123'
```

Expect: the tab strip ` Workspace │ PRs 4 [✗]`; the list pane `prs` beside the detail pane `PR`. The headers read `ABC-123 · > fix CI #7`, `ABC-125 · > wait for CI !5`, `ABC-120 · m merged`; only the first header ends in `checked just now`. Rows show the title without its key and commit type (`Refresh auth tokens before expiry`, `Add the login page`, `Rate limit the public API`, `Reconcile billing runs`), a CI cell (`CI x`, `CI ok`, `CI ..`, `CI -`), and the state label; the `#8` row ends in `in review   after #7`. The detail starts `x #7 fix(ABC-123): Refresh auth tokens` (the full title, cut at the pane edge): a header shows the PR its next action names.

**PASS:** `in_order` prints PASS, the capture shows `PRs 4`, `after #7` on the `#8` row, and `checked ` on the first header only, and the cursor starts on the `ABC-123` header. The group order follows what the author must do (CI failed, then waiting, then finished), not the key: key order would put `ABC-120` first.

## PT2 The detail follows the cursor

```bash
walk '#7   acme' '#8   acme' 'v ABC-125' '!5   acme' 'v ABC-120' '#12  acme'
$V wait "$RUN" 'merger/.worktrees/recon · worktree'
$V capture "$RUN" detail-12
```

Expect on the capture: the detail title `m #12 ABC-120: Reconcile billing runs`, the popover lines (`state    MERGED · APPROVED`, `url      https://github.com/acme/merger/pull/12`), `checkout merger · primary` with `merger/.worktrees/recon · worktree` under it, `repo     acme/merger`, then the actions `Enter  Open PR`, `gc     Open checks`, `o      Show in tree`, `y      Copy URL`.

**PASS:** every `wait --cursor` of the walk passes and the capture holds the detail lines above: one PR seen from two checkouts lists both.

## PT3 Fold with `z` and `Z`

With 12 PRs or fewer, every group starts unfolded (above 12, finished groups start folded; the PTY test `pty_prs_tab_z_folds_a_group_and_finished_groups_start_folded` covers that side). The cursor is on `#12` after PT2.

```bash
$V keys "$RUN" z && $V wait --cursor "$RUN" '> ABC-120 · m merged' && $V wait --gone "$RUN" '#12  acme'
$V capture "$RUN" z-folded
$V keys "$RUN" z && $V wait "$RUN" '#12  acme'
top
$V keys "$RUN" Z && $V wait "$RUN" '> ABC-120 · m merged' && $V wait --gone "$RUN" '#12  acme'
$V capture "$RUN" Z-folded
$V keys "$RUN" Z && $V wait "$RUN" 'v ABC-120 · m merged' && $V wait "$RUN" '#12  acme'
```

**PASS:** `z` folds the group of the row under the cursor, the cursor moves to its header (`> ABC-120`), and the `#12` row goes; `z` again unfolds it. `Z` with the cursor on `ABC-123` folds only the finished group: the `Z-folded` capture shows `> ABC-120` with `v ABC-123` and `v ABC-125` unchanged. `Z` again unfolds it.

## PT4 Enter and `gc` open the PR and its CI page, on both forges

```bash
top && walk '#7   acme'
$V keys "$RUN" Enter && $V wait "$RUN" 'opened PR #7'
$V keys "$RUN" g c && $V wait "$RUN" 'opened CI for #7'
walk '#8   acme' 'v ABC-125' '!5   acme'
$V keys "$RUN" Enter && $V wait "$RUN" 'opened PR #5'
$V keys "$RUN" g c && $V wait "$RUN" 'opened CI for !5'
cat "$RUN/opened.log"
```

Expect `opened.log` to hold exactly these four lines, in this order:

```text
https://github.com/acme/app/pull/7
https://github.com/acme/app/actions/runs/501/job/6002
https://gitlab.com/acme/services/api/-/merge_requests/5
https://gitlab.com/acme/services/api/-/pipelines/9002
```

**PASS:** each status text shows after its key, and `opened.log` is the four URLs above: the PR page, the failed check (`job/6002`, not the first check `lint`), the merge request, and the head pipeline. No browser starts.

## PT5 `o` shows the checkout in the tree; `gp` comes back; `Ctrl-w` closes

```bash
$V keys "$RUN" o                                   # the cursor is on !5 after PT4
$V wait --cursor "$RUN" 'services/api  feature/rate-limit' && $V wait --gone "$RUN" 'v ABC-123'
$V capture "$RUN" after-o
$V keys "$RUN" g p && $V wait --cursor "$RUN" '!5   acme'
$V keys "$RUN" C-w && $V wait --gone "$RUN" 'PRs 4'
$V capture "$RUN" closed
```

**PASS:** after `o` the Workspace tab shows the tree with the cursor bar on the `services/api` row and the breadcrumb `workspace › api`, while the tab strip still holds `PRs 4`; `gp` brings the same PRs tab back with the cursor still on `!5`; `Ctrl-w` closes it, and the `closed` capture shows the tree and no `PRs` label.

## PT6 Narrow frame: one pane, `Tab` switches

```bash
fresh prs-tab -- --size 90x45 --fake-forge "$FORGE"
$V keys "$RUN" g p                                 # at this width the tree drops its refs: wait on the tab instead
$V wait "$RUN" 'r !5   acme' && $V wait "$RUN" 'm #12  acme' && $V wait --gone "$RUN" 'looking up PRs'
top && $V wait --gone "$RUN" 'Enter  Open PR'
$V capture "$RUN" narrow-list
$V keys "$RUN" Tab && $V wait "$RUN" 'Enter  Open PR' && $V wait --gone "$RUN" 'v ABC-120'
$V capture "$RUN" narrow-detail
$V keys "$RUN" Escape && $V wait "$RUN" 'v ABC-120 · m merged' && $V wait --gone "$RUN" 'Enter  Open PR'
```

**PASS:** at 90 columns the tab shows the list alone (no detail actions), `Tab` shows the detail alone (the detail of `#7`, the PR the first header names), and Escape shows the list again.

## PT7 A failing `glab`: its error line, and the GitHub PRs still list

```bash
fresh prs-tab gitlab-cli-fails -- --fake-forge "$FORGE"
$V wait --regex "$RUN" '#12 m($|[^[:alnum:]])'
$V keys "$RUN" g p
$V wait "$RUN" '! glab (gitlab.com): glab: HTTP 502' && $V wait "$RUN" 'lookups failing' && $V wait "$RUN" 'PRs 3'
$V capture "$RUN" glab-fails | grep -c '!5   acme'       # 0
$V scenario "$FORGE" prs-tab                              # glab answers again (overlay)
$V keys "$RUN" r
$V wait "$RUN" '!5   acme' && $V wait --gone "$RUN" '! glab (gitlab.com)' && $V wait --gone "$RUN" 'lookups failing'
$V capture "$RUN" glab-back
```

**PASS:** the `glab-fails` capture shows the line `! glab (gitlab.com): glab: HTTP 502` above the groups, the label `PRs 3`, the rows `#7`, `#8` and `#12`, no `!5` row (the count prints `0`), and the first header ends in `checked just now · lookups failing`. After the swap, `r` on the tab brings `!5` back and the error line and `lookups failing` go.

## PT8 Empty state

```bash
fresh github-none gitlab-none context-none -- --fake-forge "$FORGE"
$V keys "$RUN" g p
$V wait "$RUN" 'No PR for any checked-out branch.' && $V wait "$RUN" 'PRs 0'
$V capture "$RUN" empty
```

**PASS:** the capture shows the label `PRs 0` and the three lines `No PR for any checked-out branch.`, `Open and merged PRs of each checkout's branch show here.`, `Push a branch and open a PR, then press r.`, with no group header. While lookups run, the list may first say `looking up PRs · <done> of <total> checkouts`; the empty lines show only once every lookup has landed.

## PT9 Title command

The program in `WS_STATUS_PR_TITLE_CMD` runs as `<cmd> <KEY>` for each ticket key, with no shell and a 2 s limit. Exit 0 with a non-empty first line gives the group title; anything else gives none.

```bash
ROOT=${WS_VERIFY_ROOT:-${TMPDIR:-/tmp}/ws-verify}; mkdir -p "$ROOT"
TITLES=$ROOT/pr-titles; : >"$TITLES.log"
cat >"$TITLES" <<EOF
#!/bin/sh
printf '%s\n' "\$1" >>'$TITLES.log'
case "\$1" in
  ABC-123) echo 'Login refresh' ;;
  ABC-125) echo 'API limits' ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$TITLES"
fresh prs-tab -- --env WS_STATUS_PR_TITLE_CMD="$TITLES" --fake-forge "$FORGE"
$V wait --regex "$RUN" '!5 r($|[^[:alnum:]])' && $V wait --regex "$RUN" '#12 m($|[^[:alnum:]])'
$V keys "$RUN" g p
$V wait "$RUN" 'v ABC-123 · Login refresh · > fix CI #7' && $V wait "$RUN" 'v ABC-125 · API limits · > wait for CI !5'
$V capture "$RUN" titles
sort -u "$TITLES.log"                                     # ABC-120, ABC-123, ABC-125
```

**PASS:** the capture shows both titled headers and `v ABC-120 · m merged` with no title (the script exits 1 for it), and the log names exactly `ABC-120`, `ABC-123` and `ABC-125`.

## PT10 Nerd glyphs

```bash
fresh prs-tab -- --env WS_STATUS_GLYPHS=nerd --fake-forge "$FORGE"
$V wait "$RUN" '!5 ◐' && $V wait "$RUN" '#12 ✓'
$V keys "$RUN" g p && $V wait "$RUN" 'ABC-120 · ✓ merged'
$V capture "$RUN" nerd
```

**PASS:** the capture shows the headers `▾ ABC-123 · ▸ fix CI #7` and `▾ ABC-120 · ✓ merged`, the rows `✕ #7`, `○ #8`, `◐ !5`, `✓ #12`, the CI cells `CI ✕`, `CI ✓`, `CI …`, `CI —`, and the stack mark `⧗ after #7`.

Stop the last run when you are done (`stop_run`, or `$V stop <run dir>`).

## Gotchas

- The tab opens with the cursor on its first line at that moment, and the cursor then follows that line. Wait for every badge before `gp` (as above), or the first line can be a group that later moves down.
- The tree row and the detail pane share screen lines with the list. `x #7 ` alone also matches the detail title on the first line; use the row text with its repository (`x #7   acme/app`) or `wait --cursor`.
- `Z` and `C-w` reach the app as legacy key bytes under tmux; the PTY suite sends Shift and Ctrl keys as CSI-u. Both paths work, and only the PTY suite proves the CSI-u one.
- The title command runs once per key per fill. A fill starts when the tab opens, when the listed keys change, and after `r` on the tab; with polling off (the driver default) nothing else starts one.
- Mouse clicks, OSC 8 links on the refs, and the row flash are not proved here: the PTY tests and the render tests cover them.
