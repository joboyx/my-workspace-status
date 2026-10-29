# PR and commit popovers

`gh` pins the icon popover of the focused row. For a checkout row with a PR it has a PR section, the last one: the PR fields, loaded with one more `gh pr view` / `glab api` call, and the actions of the PR. `j` / `k` move between lines, Enter runs the focused action, `y` copies the focused line, `Esc` closes it. A mouse hover peeks and a click pins. The PR recipes use the keys; the commit popovers are mouse only, so PO3 drives them with the driver's `mouse` command.

`gh` pins the popover of every mark on the row, and the PR section comes last, so these recipes launch at `160x70`: on a shorter screen the section is cut off.

Setup (the prelude with `V`, `FORGE`, `RUN`, `fresh`, `relaunch`, `stop_run` and the `to_*` helpers), the `trap` that stops the last run, and how to run it when every command gets a fresh shell: [pr-status.md](pr-status.md). Every recipe starts its own run with `fresh`, which stops the previous one before it resets the scenario files.

## PO1 The PR section, both forges

```bash
fresh context github-failed gitlab-failed -- --size 160x70 --fake-forge "$FORGE"
$V wait --regex "$RUN" '#7 x($|[^[:alnum:]])'
to_app && $V keys "$RUN" g h
$V wait "$RUN" 'Refresh this PR'                        # the lines load after the popover opens
$V capture "$RUN" popover-github
$V keys "$RUN" Escape && $V wait --gone "$RUN" '╭'
to_api && $V keys "$RUN" g h
$V wait "$RUN" 'Open pipeline'
$V capture "$RUN" popover-gitlab
$V keys "$RUN" Escape && $V wait --gone "$RUN" '╭'
```

Expect on `popover-github`: a section titled `x PR · CI failed` with these lines, in this order (each only when it has data):

```text
number  #7 Refresh auth tokens before expiry
state   OPEN · REVIEW_REQUIRED
next    > fix CI
author  octo-user
branch  feature/auth-refresh -> main
checks  1 pass · 1 fail · 0 pending
        fail test (ubuntu-latest)
reviews 0 approved · REVIEW_REQUIRED
updated 2026-10-29 09:00
checked just now
url     https://github.com/acme/app/pull/7
```

On `popover-gitlab`: the title `x PR · CI failed` and the lines `number  !5 Rate limit the public API`, `state   OPEN · NOT_APPROVED`, `next    > fix CI`, `author  octo-user`, `branch  feature/rate-limit -> main`, `checks  pipeline failed`, `fail test` (the failed job), `approvals 0 of 1`, `updated`, `checked`, a clipped `url` line. The `updated` time is in `Asia/Manila` (the driver pins `TZ`): the sets' `01:00:00Z` shows as `09:00`. A value longer than the 64-column box ends in `…`.

**PASS:** every line above is on its capture, in order, and `Esc` closes the popover (`╭` is gone).

## PO2 Actions, `gc` and `r` from the popover

The actions come after a rule: `Open PR  gx`, `Open failed check  gc`, `Copy URL  y`, `Refresh this PR  r` (GitLab: `Open pipeline  gc`). A merged PR keeps `Open PR`, the CI action (by the same rule: a merged PR with a failed check says `Open failed check`) and `Copy URL`, and drops `Refresh this PR`.

```bash
fresh context github-failed gitlab-failed -- --size 160x70 --fake-forge "$FORGE"
$V wait --regex "$RUN" '#7 x($|[^[:alnum:]])'
to_app && $V keys "$RUN" g h
$V wait "$RUN" 'Open failed check'
$V capture "$RUN" popover-actions
$V keys "$RUN" g c                                   # gc works from the popover too
for _ in $(seq 25); do [ "$(tail -n 1 "$RUN/opened.log")" = https://github.com/acme/app/actions/runs/501/job/6002 ] && break; sleep 0.2; done
tail -n 1 "$RUN/opened.log"                          # .../job/6002, the failed check
$V keys "$RUN" Escape && $V wait --gone "$RUN" '╭'
views() { $V calls "$RUN" | grep -c '^gh pr view '; }   # calls of this run only
n=$(views)
$V keys "$RUN" g h && $V wait "$RUN" 'Refresh this PR'
$V keys "$RUN" r                                     # Refresh this PR
for _ in $(seq 25); do [ "$(views)" -gt "$n" ] && break; sleep 0.2; done
[ "$(views)" -gt "$n" ] && echo "PASS refresh"
```

**PASS:** the four actions are on `popover-actions` in the order above; `gc` opens the failed check URL (`.../job/6002`); `r` makes the app call `gh pr view` again (the calls of this run grow, and `PASS refresh` prints).

## PO3 Commit popovers (mouse)

Every painted commit short hash, commit time and commit author opens a commit popover: the pointer resting on it for 400 ms peeks, a click pins. They are mouse only (`gh` does not list them), so this recipe uses `mouse`: `mouse <run> find <text>` prints the 1-based `<col> <row>` where the text starts on screen, `mouse <run> move <col> <row>` rests the pointer there, `mouse <run> click <col> <row>` presses and releases the left button. The reports are SGR (1006) bytes written to the pane, the same bytes a terminal sends.

The cells are the `app` graph row of `HEAD`, read from the run's own fixture. The seed clock is fixed (`2026-08-17`), so the row shows the date as local `YYYY-MM-DD HH:MM`, and every commit is by `Demo User <demo@example.invalid>`.

```bash
fresh github-none gitlab-none context-none -- --fake-forge "$FORGE"
to_app
$V wait "$RUN" 'Tighten session expiry'                   # the graph of app is on the right
SHA=$(git -C "$RUN/workspace/app" rev-parse HEAD)
WHEN=$(TZ=Asia/Manila git -C "$RUN/workspace/app" log -1 --date=format-local:'%Y-%m-%d %H:%M' --format=%ad HEAD)
CELL=$($V mouse "$RUN" find "${SHA:0:7}")                 # the short hash on the HEAD row
$V mouse "$RUN" move $CELL
$V wait "$RUN" "sha       $SHA" && $V wait "$RUN" 'authored'
$V capture "$RUN" commit-hash-peek
grep -q 'Esc close' "$RUN"/evidence/*-commit-hash-peek.txt || echo "PASS peek has no pinned footer"
$V mouse "$RUN" click $CELL
$V wait "$RUN" 'y copy line · Enter run · Esc close'
$V capture "$RUN" commit-hash-pinned
$V keys "$RUN" y                                          # copies the focused line, the full sha
$V wait "$RUN" 'copied'
$V keys "$RUN" Escape && $V wait --gone "$RUN" 'Commit: full id'
read -r COL ROW <<<"$($V mouse "$RUN" find "$WHEN Demo User")"   # date, then author, on the HEAD row
$V mouse "$RUN" move "$COL" "$ROW"
$V wait "$RUN" 'Local times; orig is the UTC offset git recorded' && $V wait "$RUN" 'committer'
$V capture "$RUN" commit-time-peek
$V mouse "$RUN" move $((COL + ${#WHEN} + 1)) "$ROW"       # a peek follows the pointer: no Esc
$V wait "$RUN" 'Author, committer, and Co-authored-by trailers' && $V wait --gone "$RUN" 'Local times;'
$V capture "$RUN" commit-author-peek
$V mouse "$RUN" move 1 1 && $V wait --gone "$RUN" 'Author, committer,'   # off every hit: the peek closes
```

Expect on `commit-hash-peek` a box titled `commit <short7>`, with no pinned footer, and these lines (`<when>` is `YYYY-MM-DD HH:MM:SS local · orig UTC+08:00`):

```text
Commit: full id, message, parents, refs, and dates
sha       <full 40-char HEAD id>
subject   Tighten session expiry
parent    <full 40-char id of HEAD^>
branches  feature/auth-refresh
author    Demo User <demo@example.invalid>
authored  <when>
committed <when>
```

`commit-hash-pinned` shows the same card with the focus marker on `sha` and the footer `y copy line · Enter run · Esc close`. `commit-time-peek` is titled `commit time` with `author`, `authored`, `committer`, `committed`. `commit-author-peek` is titled `commit author` with the same four rows and no `co-author` row (the seed has no `Co-authored-by` trailer).

A peek is not an input mode, so `Esc` does not close it (with the graph focused, `Esc` moves the focus back to the tree): moving the pointer to another hit swaps the peek, and moving it off every hit closes it. The click also moves the graph cursor to the clicked row.

**PASS:** the four captures show those lines; the pinned capture has the footer and the peek does not; `y` sets the `copied` status; `Esc` closes the pinned card; the time peek gives way to the author peek, and the author peek closes when the pointer leaves. The copied text (OSC 52) is not on a tmux capture: `pty_commit_hash_popover_peeks_pins_and_copies_the_full_sha` checks it, and `pty_blame_hash_popover_shows_the_commit_card` covers the card of a commit off the graph.
