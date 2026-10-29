# PR popover

`gh` pins the icon popover of the focused row. For a checkout row with a PR it has a PR section: the PR fields, loaded with one more `gh pr view` / `glab api` call, and the actions of the PR. `j` / `k` move between lines, Enter runs the focused action, `y` copies the focused line, `Esc` closes it. A mouse hover peeks and a click pins; a tmux run sends no mouse, so the keys are the way in.

Setup, the `V`, `FORGE` and `to_*` helpers, and the **needs phase A** meaning: [pr-status.md](pr-status.md).

## PO1 The PR section, both forges (all builds)

```bash
$V scenario "$FORGE" context github-failed gitlab-failed
RUN=$($V launch --fake-forge "$FORGE")
$V wait --regex "$RUN" 'feature/rate-limit +& P'
to_app && $V keys "$RUN" g h
$V wait "$RUN" 'Refresh auth tokens before expiry'      # the lines load after the popover opens
$V capture "$RUN" popover-github
$V keys "$RUN" Escape && $V wait --gone "$RUN" '╭'
to_api && $V keys "$RUN" g h
$V wait "$RUN" 'Rate limit the public API'
$V capture "$RUN" popover-gitlab
$V keys "$RUN" Escape && $V wait --gone "$RUN" '╭'
```

Expect on `popover-github` (today): a section titled `P PR open` with these lines, in this order:

```text
number  #7 Refresh auth tokens before expiry
state   OPEN · REVIEW_REQUIRED
author  octo-user
branch  feature/auth-refresh -> main
checks  1 pass · 1 fail · 0 pending
updated 2026-10-29 09:00
url     https://github.com/acme/app/pull/7
Open PR                                    gx
```

On `popover-gitlab` (today): `number  #5 Rate limit the public API`, `state   OPEN · NOT_APPROVED`, `author  octo-user`, `branch  feature/rate-limit -> main`, `checks  0 pass · 1 fail · 0 pending`, a clipped `url` line, `Open PR  gx`. The `updated` time is in `Asia/Manila` (the driver pins `TZ`): the sets' `01:00:00Z` shows as `09:00`.

**PASS:** every line above is on the capture, in order, and `Esc` closes the popover (`╭` is gone).

## PO2 Final lines and actions (needs phase A)

With `github-failed` and the cursor on the `app` row, `g h` shows the section header `x PR · CI failed` and these lines, each only when it has data: `number`, `state`, `next` (`▸ fix CI`), `author`, `branch`, `checks`, up to two lines naming checks (failed first, here `test (ubuntu-latest)`), `reviews`, `updated`, `checked`, `url`. Actions: `Open failed check  gc`, `Open PR  gx`, `Copy URL  y`, `Refresh this PR  r`. On `gitlab-failed`: `approvals 0 of 1`, `checks` as `pipeline failed`, a line for the failed job `test`, and the action `Open pipeline  gc`.

```bash
$V scenario "$FORGE" github-failed
to_app && $V keys "$RUN" g h
$V wait "$RUN" 'Open failed check'
$V capture "$RUN" popover-final
$V keys "$RUN" g c                                   # gc works from the popover too
for _ in $(seq 25); do [ "$(tail -n 1 "$RUN/opened.log")" = https://github.com/acme/app/actions/runs/501/job/6002 ] && break; sleep 0.2; done
tail -n 1 "$RUN/opened.log"                          # .../job/6002, the failed check
$V keys "$RUN" Escape && $V wait --gone "$RUN" '╭'
n=$(grep -c '^gh pr view ' "$FORGE/calls.log")
$V keys "$RUN" g h && $V wait "$RUN" 'Refresh this PR'
$V keys "$RUN" r                                     # Refresh this PR
for _ in $(seq 25); do [ "$(grep -c '^gh pr view ' "$FORGE/calls.log")" -gt "$n" ] && break; sleep 0.2; done
[ "$(grep -c '^gh pr view ' "$FORGE/calls.log")" -gt "$n" ] && echo "PASS refresh"
```

**PASS:** the final lines and actions are on `popover-final` in the order above; `gc` opens the failed check URL; `r` makes the app call `gh pr view` again (`calls.log` grows). A merged PR keeps `Open PR`, the CI action and `Copy URL`, and drops `Refresh this PR`.
