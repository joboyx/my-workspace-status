# PR popover

`gh` pins the icon popover of the focused row. For a checkout row with a PR it has a PR section, the last one: the PR fields, loaded with one more `gh pr view` / `glab api` call, and the actions of the PR. `j` / `k` move between lines, Enter runs the focused action, `y` copies the focused line, `Esc` closes it. A mouse hover peeks and a click pins; a tmux run sends no mouse, so the keys are the way in.

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
