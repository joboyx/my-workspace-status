# PR status

A checkout row in the tree shows the pull request of its branch: a GitHub PR through `gh`, a GitLab merge request through `glab`. User text says "PR" for both. The badge is the PR state: six states, and the ref (`#7` on GitHub, `!5` on GitLab) left of the glyph, flush with the tree edge.

| State | Set suffix | ASCII | Nerd | Ref on GitHub / GitLab |
| --- | --- | --- | --- | --- |
| CI failed | `failed` | `x` | `✕` | `#7 x` / `!5 x` |
| ready to merge | `ready` | `A` | `◉` | `#7 A` / `!5 A` |
| CI running | `running` | `r` | `◐` | `#7 r` / `!5 r` |
| draft | `draft` | `d` | `◌` | `#7 d` / `!5 d` |
| in review | `review` | `P` | `○` | `#7 P` / `!5 P` |
| merged | `merged` | `m` | `✓` | `#7 m` / `!5 m` |

Every check in the feature files needs the PR status visuals (six states, the ref beside the glyph, `gc`, polling, toasts). A build from before them (a three-state badge `P` / `A` / `m`, no ref) cannot pass: report SKIP and name the missing piece.

## Setup

Every recipe in these files uses the names of this block (the prelude). Save it as a file, for example `${TMPDIR:-/tmp}/ws-prelude.sh`, and source it at the top of a recipe. The shell must be in the repository root (or a worktree root).

```bash
V=$(pwd)/scripts/verify-tui.sh
$V build >/dev/null                      # a no-op when the binary is up to date
FORGE=${FORGE:-$(mktemp -d "${TMPDIR:-/tmp}/ws-forge.XXXXXX")}
RUN=${RUN:-}

# One run at a time per forge dir. A run that is not stopped keeps its tmux server and,
# with polling on, keeps polling; a failed wait leaves the TUI running too.
stop_run() { [ -z "$RUN" ] || $V stop "$RUN" >/dev/null 2>&1; }
[ -n "${KEEP_RUN:-}" ] || trap stop_run EXIT       # see "One shell or many" below
relaunch() { stop_run; RUN=$($V launch "$@"); }
# fresh <set>... -- <launch args>: stop the old run, THEN reset the scenario files to
# these sets, THEN launch. The order matters: the old run may still poll the forge.
fresh() {
  stop_run
  local sets=()
  while [ $# -gt 0 ] && [ "$1" != -- ]; do sets+=("$1"); shift; done
  shift
  $V scenario --reset "$FORGE" "${sets[@]}" && RUN=$($V launch "$@")
}

# Move the cursor: to the workspace row, to the app checkout row, to any row by its text.
# Only to_top works with --env WS_STATUS_GLYPHS=nerd: the others wait on ASCII row text.
to_top() { $V keys "$RUN" g g && $V wait --cursor "$RUN" 'workspace  '; }   # also in Nerd mode: the icon before the name differs
to_app() { to_top && $V keys "$RUN" j && $V wait --cursor "$RUN" 'v app' && $V keys "$RUN" j && $V wait --cursor "$RUN" '& feature/auth-refresh'; }
to_row() { $V keys "$RUN" / && $V keys -l "$RUN" "$1" && $V keys "$RUN" Enter && $V wait --cursor "$RUN" "$1" && $V keys "$RUN" Escape && $V wait "$RUN" 'search cleared'; }
to_api() { to_row 'rate-limit'; }
```

`$V calls "$RUN"` lists the `gh` / `glab` calls of this run only, so an earlier run on the same forge dir does not count.

### One shell or many

The `trap` stops the last run when the shell that set it exits. What that means depends on how you run commands:

- **One shell for the whole recipe** (an interactive terminal, a script file, or one tool call): the names stay, and the run is stopped when the shell exits. In an interactive terminal that is when you close it, so call `stop_run` yourself when a recipe is done or has failed.
- **A fresh shell for every command** (one tool call = one shell): `V`, `RUN`, `fresh` and the `to_*` helpers do not carry over to the next call, and the `trap` fires when the call that set it ends, so it stops the run that call just started. Pick one of these:
  - **(a) One call per recipe.** Put the prelude and the whole recipe in one call or one script file (example below). The trap stops the run at the end of the call. The evidence stays in the run dir, which `stop` keeps.
  - **(b) A recipe split over calls.** Skip the trap with `export KEEP_RUN=1` before the prelude, print `FORGE` and `RUN` in the first call, pass them back in each later call, and `stop` the run **by path** in the last call, also when an earlier call failed:

    ```bash
    # call 1
    cd <repo root> && export KEEP_RUN=1 && . "${TMPDIR:-/tmp}/ws-prelude.sh"
    fresh context github-review gitlab-review -- --fake-forge "$FORGE"
    echo "FORGE=$FORGE RUN=$RUN"                  # keep both lines
    # call 2 (and more)
    cd <repo root> && export KEEP_RUN=1 FORGE=<printed forge dir> RUN=<printed run dir> && . "${TMPDIR:-/tmp}/ws-prelude.sh"
    to_app && $V keys "$RUN" g x && $V wait "$RUN" 'opened PR #7'
    # last call
    <repo root>/scripts/verify-tui.sh stop <printed run dir>
    ```

  - **(c) One call per recipe, the example (PS3 in short):**

    ```bash
    cd <repo root> && . "${TMPDIR:-/tmp}/ws-prelude.sh"
    fresh context github-review gitlab-review -- --fake-forge "$FORGE"
    $V wait --regex "$RUN" 'feature/auth-refresh +P'
    to_app && $V keys "$RUN" g x
    $V wait "$RUN" 'opened PR #7'
    tail -n 1 "$RUN/opened.log"                   # https://github.com/acme/app/pull/7
    $V capture "$RUN" after-gx
    echo "run dir: $RUN"                          # the trap stops the run when this call ends
    ```

The fake forge points `app` at `acme/app` (GitHub, PR #7, branch `feature/auth-refresh`) and `services/api` at `acme/services/api` (GitLab, MR !5, branch `feature/rate-limit`). Set names and answers: `SKILL.md`, **Fake forge**.

## PS1 First paint, both forges

```bash
fresh context github-review gitlab-review -- --fake-forge "$FORGE"
$V wait "$RUN" '#7 P'
$V wait "$RUN" '!5 P'
$V capture "$RUN" first-paint
CALLS=$($V calls "$RUN")
grep -q '^gh pr list ' <<<"$CALLS" && grep -q '^glab api ' <<<"$CALLS" && echo "PASS both CLIs were called"
```

Expect: the `app` row `feature/auth-refresh` ends in `#7 P`; the `services/api` row ends in `!5 P`; the `app` worktree row `L feature/login-page` ends in `#8 P`; both `merger` rows (`& feature/reconciliation`, `L feature/reconciliation`) end in `#12 m`.

**PASS:** all five rows show the expected badge on the captured screen and the calls of this run include a `gh pr list` line and a `glab api` line.

## PS2 Every state, both forges

Each `sweep` swaps both forges to one state, reloads every checkout (`r` on the workspace row), and waits for both badges. The order matters: next to each other, two states never show the same glyph, so a wait cannot pass on the badge of the step before. The swap is an overlay (no `--reset`), so no call finds a gap.

Every pattern ends in "not a letter or digit". A change into ready to merge, merged or CI failed raises a toast that lives 8 s (`A !5 ready to merge · feature/rate-limit`), and `!5 r` is a prefix of `!5 ready`: without the boundary the next sweep would pass on the toast of the one before.

```bash
sweep() { # sweep <state> <github-regex> <gitlab-regex>
  $V scenario "$FORGE" "github-$1" "gitlab-$1" && to_top && $V keys "$RUN" r &&
    $V wait --regex "$RUN" "$2" && $V wait --regex "$RUN" "$3" &&
    $V capture "$RUN" "state-$1" >/dev/null && echo "PASS $1" || echo "FAIL $1"
}
END='($|[^[:alnum:]])'
sweep ready   "#7 A$END" "!5 A$END"
sweep draft   "#7 d$END" "!5 d$END"
sweep running "#7 r$END" "!5 r$END"
sweep failed  "#7 x$END" "!5 x$END"
sweep merged  "#7 m$END" "!5 m$END"
sweep review  "#7 P$END" "!5 P$END"
```

**PASS:** every sweep prints PASS for its column, and each saved capture shows no other state glyph on those two rows (the `app` row `feature/auth-refresh` and the `services/api` row `feature/rate-limit`).

## PS3 `gx` opens the PR

```bash
to_app && $V keys "$RUN" g x
$V wait "$RUN" 'opened PR #7'
tail -n 1 "$RUN/opened.log"        # https://github.com/acme/app/pull/7
to_api && $V keys "$RUN" g x
$V wait "$RUN" 'opened PR #5'
tail -n 1 "$RUN/opened.log"        # https://gitlab.com/acme/services/api/-/merge_requests/5
```

**PASS:** the status line says `opened PR #7` / `opened PR #5`, and the last line of `opened.log` is the PR URL of that forge. No browser starts: the recorder only appends the URL.

## PS4 `gc` opens the CI page

`gc` on a checkout row opens the CI page of its PR. GitHub: the first failed check, else the first running check, else the first check with a link, else `<PR URL>/checks`. GitLab: the head pipeline, else `<MR URL>/pipelines`. The sets list `lint` before `test (ubuntu-latest)` on purpose, so "first failed" differs from "first listed".

| Set | Row | `tail -n 1 "$RUN/opened.log"` after `g c` | Status line |
| --- | --- | --- | --- |
| `github-failed` | `app` | `https://github.com/acme/app/actions/runs/501/job/6002` | `opened CI for #7` |
| `github-running` | `app` | `https://github.com/acme/app/actions/runs/501/job/6002` | `opened CI for #7` |
| `github-ready` | `app` | `https://github.com/acme/app/actions/runs/501/job/6001` | `opened CI for #7` |
| `gitlab-failed` | `services/api` | `https://gitlab.com/acme/services/api/-/pipelines/9001` | `opened CI for !5` |
| `gitlab-running` | `services/api` | `https://gitlab.com/acme/services/api/-/pipelines/9002` | `opened CI for !5` |
| `gitlab-ready` | `services/api` | `https://gitlab.com/acme/services/api/-/pipelines/9003` | `opened CI for !5` |

Steps per row: `$V scenario "$FORGE" <set>`, `to_top`, `r`, `wait` for the badge of that set, `to_app` or `to_api`, note the line count of `opened.log`, `$V keys "$RUN" g c`, `wait` for the status text of the row, then read `opened.log`. With `github-none`: `g c` says `no PR for feature/auth-refresh` and `opened.log` does not grow.

**PASS:** the status line shows the exact text of the row (`opened CI for #7` or `opened CI for !5`), `opened.log` grew by one line, and its last line equals the URL of the row.

## PS5 No PR

The badge must be on the screen first, then go. The patterns end in "not a letter or digit", not in a space: the glyph is flush with the tree edge, so a border can follow it, and a toast can start with the same letter (`#7 ready`).

```bash
fresh context github-review gitlab-review -- --fake-forge "$FORGE"
BADGE_GH='#7 [xArdPm]($|[^[:alnum:]])'
BADGE_GL='!5 [xArdPm]($|[^[:alnum:]])'
$V wait --regex "$RUN" "$BADGE_GH" && $V wait --regex "$RUN" "$BADGE_GL"     # present before the swap
$V scenario "$FORGE" github-none gitlab-none
to_top && $V keys "$RUN" r
$V wait --gone --regex "$RUN" "$BADGE_GH" && $V wait --gone --regex "$RUN" "$BADGE_GL"
$V capture "$RUN" no-pr
to_app && $V keys "$RUN" g x
$V wait "$RUN" 'no PR for feature/auth-refresh'
```

**PASS:** both badges were present, both are gone on the capture, and `gx` says `no PR for feature/auth-refresh`. A closed PR that was not merged counts as no PR; the sets have no closed PR, so only the empty list is proved here.

## PS6 Scripted transition, toast and total

Polling needs a short interval: `WS_STATUS_PR_POLL_MS=1000`. The driver pins it to `0` (off) otherwise. An alert is a row flash plus a PR toast that lives 8 s; there is no alert on first load, on a branch change, or on the first lookup of a checkout. The workspace total reads `<n> PRs · <m> need you` when its row has room for both parts. With the seeded `5 changed · 1 ahead, 1 diverged` summary the default width has room for `2 need you` only, so the recipe waits for that part (it is on the row in both cases).

```bash
fresh context github-running gitlab-running -- --env WS_STATUS_PR_POLL_MS=1000 --fake-forge "$FORGE"
$V wait --regex "$RUN" '#7 r($|[^[:alnum:]])' && $V wait --regex "$RUN" '!5 r($|[^[:alnum:]])'
$V capture "$RUN" t0-first-load                      # no toast here
$V scenario "$FORGE" github-failed gitlab-failed     # no key: the poll finds it
$V wait "$RUN" 'x #7 CI failed · test (ubuntu-latest)' 10     # GitHub names the failed check
$V wait "$RUN" 'x !5 CI failed · feature/rate-limit' 10      # GitLab names the branch (the badge path reads no job)
$V wait --regex "$RUN" '2 need you($|[^[:alnum:]])' 10
$V capture "$RUN" t1-ci-failed
$V wait --gone "$RUN" 'CI failed ·' 12              # the toasts are gone after 8 s
$V scenario "$FORGE" github-review gitlab-review    # silent: no toast for review
$V wait --regex "$RUN" '#7 P($|[^[:alnum:]])' && $V wait --regex "$RUN" '!5 P($|[^[:alnum:]])'
$V capture "$RUN" t2-review
$V scenario "$FORGE" github-ready gitlab-ready
$V wait "$RUN" 'A #7 ready to merge · feature/auth-refresh' 10
$V wait "$RUN" 'A !5 ready to merge · feature/rate-limit' 10
$V capture "$RUN" t3-ready
$V scenario "$FORGE" github-merged gitlab-merged
$V wait "$RUN" 'm #7 merged · feature/auth-refresh' 10
$V wait "$RUN" 'm !5 merged · feature/rate-limit' 10
$V capture "$RUN" t4-merged
```

**PASS:** t0 holds no toast text (`CI failed`, `ready to merge`, `merged ·`); each toast text appears after its swap with no key pressed; the `review` step (t2) prints no toast; the total shows `2 need you` while both PRs are failed. The row flash is colour only: the `.ansi` captures t0 and t1 differ.

## PS7 A failing CLI keeps the last badge

The fake CLIs exit 1 with a one-line stderr (`gh: HTTP 502`, `glab: HTTP 502`). The popover is 64 columns wide and ends a longer value in `…`, so the sets keep the line short. `gh` pins the popover of every mark on the row, and the PR section is the last: the recipe launches at `160x70` so the `services/api` row (branch, repo, diverged and change count sections before it) shows the whole PR section.

```bash
fresh context github-review gitlab-review -- --size 160x70 --env WS_STATUS_PR_POLL_MS=1000 --fake-forge "$FORGE"
$V wait "$RUN" '#7 P' && $V wait "$RUN" '!5 P'
$V capture "$RUN" before-fail
$V scenario "$FORGE" github-cli-fails gitlab-cli-fails
sleep 4                                              # a few polls; the one sleep in this file
$V capture "$RUN" cli-fails
to_app && $V keys "$RUN" g h                         # pin the popover
$V wait "$RUN" 'last lookup failed: gh: HTTP 502'
$V capture "$RUN" popover-github
$V keys "$RUN" Escape && $V wait --gone "$RUN" 'last lookup failed'
to_api && $V keys "$RUN" g h
$V wait "$RUN" 'last lookup failed: glab: HTTP 502'
$V capture "$RUN" popover-gitlab
```

**PASS:** after the failing polls `#7 P` and `!5 P` are still on the screen, no toast appears, the glyphs are dimmed (the `.ansi` captures `before-fail` and `cli-fails` differ in the colour of the glyph: the data is stale after twice the poll period), and each popover `checked` line carries the first stderr line of its fake CLI.

## PS8 Nerd glyph shapes

```bash
fresh context github-review gitlab-review -- --env WS_STATUS_GLYPHS=nerd --fake-forge "$FORGE"
# sweep is the function of PS2
sweep ready   '#7 ◉' '!5 ◉'
sweep draft   '#7 ◌' '!5 ◌'
sweep running '#7 ◐' '!5 ◐'
sweep failed  '#7 ✕' '!5 ✕'
sweep merged  '#7 ✓' '!5 ✓'
sweep review  '#7 ○' '!5 ○'
```

**PASS:** each sweep prints PASS, each shape is on both rows, and the glyph is the last cell of the badge on a captured tree row (`#7 ◉` then the end of the tree column).

## Live

Real `gh` and `glab` against public repos, read-only. Needs `gh`, `glab`, `jq` and `git` on `PATH`, `gh auth status` and `glab auth status` passing, and network. Pick the branches at run time, so the recipe does not rot when a PR closes. The Setup helpers above are used here too.

```bash
stop_run                                            # first: an old run may still be polling or use this fixture
GH_OPEN=$(gh pr list -R cli/cli --state open --limit 100 --json headRefName,isCrossRepository --jq '[.[] | select(.isCrossRepository | not)][0].headRefName // empty')
GH_MERGED=$(gh pr list -R cli/cli --state merged --limit 100 --json headRefName,isCrossRepository --jq '[.[] | select(.isCrossRepository | not)][0].headRefName // empty')
GL_OPEN=$(glab api 'projects/gitlab-org%2Fcli/merge_requests?state=opened&per_page=100' | jq -r '[.[] | select(.source_project_id == .target_project_id)][0].source_branch // empty')
GL_MERGED=$(glab api 'projects/gitlab-org%2Fcli/merge_requests?state=merged&per_page=100' | jq -r '[.[] | select(.source_project_id == .target_project_id)][0].source_branch // empty')

ROOT=${WS_VERIFY_ROOT:-${TMPDIR:-/tmp}/ws-verify}; WS=$ROOT/live-fixture
rm -rf "$WS"; mkdir -p "$WS"
mk() { # mk <dir> <origin> <branch>: a clean checkout on that branch; no fetch happens
  git init -q -b main "$WS/$1" &&
    git -C "$WS/$1" -c user.name=verify -c user.email=verify@example.invalid commit -q --allow-empty -m init &&
    git -C "$WS/$1" remote add origin "$2" && git -C "$WS/$1" checkout -q -b "$3"
}
mk gh-open   https://github.com/cli/cli.git        "$GH_OPEN"
mk gh-merged https://github.com/cli/cli.git        "$GH_MERGED"
mk gl-open   https://gitlab.com/gitlab-org/cli.git "$GL_OPEN"
mk gl-merged https://gitlab.com/gitlab-org/cli.git "$GL_MERGED"
relaunch --workspace "$WS"                          # no --fake-forge: the real gh and glab

# Wait for every badge to land, then capture: the PASS rule judges this screen.
$V wait --regex "$RUN" 'gh-open .* [xArdP]($|[^[:alnum:]])' 20
$V wait --regex "$RUN" 'gh-merged .* m($|[^[:alnum:]])' 20
$V wait --regex "$RUN" 'gl-open .* [xArdP]($|[^[:alnum:]])' 20
$V wait --regex "$RUN" 'gl-merged .* m($|[^[:alnum:]])' 20
$V capture "$RUN" live-badges
```

Expect: the open PR rows get an open-PR badge (one of `x A r d P`, with `#<n>` or `!<n>` before it), the merged rows get `m` (`#<n> m`). Then check `gx` on each row against the forge CLI itself, and stop the run:

```bash
live_check() { # live_check <branch> <url the forge CLI gives for that branch>
  to_row "$1" && $V keys "$RUN" g x || return 1
  for _ in $(seq 50); do [ "$(tail -n 1 "$RUN/opened.log" 2>/dev/null)" = "$2" ] && break; sleep 0.2; done
  [ "$(tail -n 1 "$RUN/opened.log")" = "$2" ] && echo "PASS $1" || echo "FAIL $1"
}
gh_url() { gh pr list -R cli/cli --head "$1" --state "$2" --json url --jq '.[0].url'; }
gl_url() { glab api "projects/gitlab-org%2Fcli/merge_requests?state=$2&per_page=100" | jq -r --arg b "$1" '[.[] | select(.source_branch == $b)][0].web_url'; }
live_check "$GH_OPEN"   "$(gh_url "$GH_OPEN" open)"
live_check "$GH_MERGED" "$(gh_url "$GH_MERGED" merged)"
live_check "$GL_OPEN"   "$(gl_url "$GL_OPEN" opened)"
live_check "$GL_MERGED" "$(gl_url "$GL_MERGED" merged)"
$V stop "$RUN"
```

**PASS:** the `live-badges` capture shows a badge of the right kind on every row (open or merged), and each `gx` URL equals the URL the forge CLI gives for that branch. SKIP, with the reason, when a CLI is missing or not logged in, when no same-repository PR of that kind exists, or when the network is down. Do not report a fake-forge run as the live result. The live `#[ignore]` unit tests (`SKILL.md`, **Pick the layer**) check the same parse in code.

## Gotchas

- The first lookup runs at launch. Apply scenario sets **before** `launch` if the first paint matters; a later `scenario` needs `r` (or a poll) to show.
- `r` on a checkout row reloads that checkout; on the workspace row it reloads every checkout.
- `wait --regex` matches inside one screen line, and the tree row also carries the right pane text. Anchor on the ref (`#7 P`) and end a one-letter glyph pattern with `($|[^[:alnum:]])`: a toast that lingers for 8 s can start with the same letters (`#7 ready`, `!5 merged`).
- `fake forge: no scenario for ...` in `$V calls "$RUN"` means the app made a call no set answers (a new `gh` / `glab` call). Teach the fake in `scripts/fake-forge/` and add the file to the sets.
- OSC 8 links on the badge are not in a tmux capture. The PTY e2e suite covers them.
