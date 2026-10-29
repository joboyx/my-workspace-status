# PR status

A checkout row in the tree shows the pull request of its branch: a GitHub PR through `gh`, a GitLab merge request through `glab`. User text says "PR" for both. The badge is the PR state; the final design has six states and the ref (`#7` on GitHub, `!5` on GitLab) left of the glyph, flush with the tree edge.

| State | Set suffix | ASCII | Nerd | Ref on GitHub / GitLab |
| --- | --- | --- | --- | --- |
| CI failed | `failed` | `x` | `✖` | `#7 x` / `!5 x` |
| ready to merge | `ready` | `A` | `◉` | `#7 A` / `!5 A` |
| CI running | `running` | `r` | `◐` | `#7 r` / `!5 r` |
| draft | `draft` | `d` | `◌` | `#7 d` / `!5 d` |
| in review | `review` | `P` | `○` | `#7 P` / `!5 P` |
| merged | `merged` | `m` | `✔` | `#7 m` / `!5 m` |

**Needs phase A** marks a check that needs the final PR status visuals. A build from before that (the badge has three states, `P` / `A` / `m`, and no ref) cannot pass it: report SKIP and name the missing piece. Today's build maps `failed`, `draft` and `review` to `P`, and `ready` and `running` to `A` (an approved PR whose CI still runs is not ready to merge in the final design, so `running` is the check that tells the two builds apart).

## Setup

Repository root, or a worktree root. All steps use these names:

```bash
V=scripts/verify-tui.sh
$V build
FORGE=$(mktemp -d "${TMPDIR:-/tmp}/ws-forge.XXXXXX")

# Move the cursor: to the workspace row, to the app checkout row, to any row by its text.
to_top() { $V keys "$RUN" g g && $V wait --cursor "$RUN" '# workspace'; }
to_app() { to_top && $V keys "$RUN" j && $V wait --cursor "$RUN" 'v app' && $V keys "$RUN" j && $V wait --cursor "$RUN" '& feature/auth-refresh'; }
to_row() { $V keys "$RUN" / && $V keys -l "$RUN" "$1" && $V keys "$RUN" Enter && $V wait --cursor "$RUN" "$1" && $V keys "$RUN" Escape && $V wait "$RUN" 'search cleared'; }
to_api() { to_row 'rate-limit'; }
```

The fake forge points `app` at `acme/app` (GitHub, PR #7, branch `feature/auth-refresh`) and `services/api` at `acme/services/api` (GitLab, MR !5, branch `feature/rate-limit`). Set names and answers: `SKILL.md`, **Fake forge**.

## PS1 First paint, both forges

```bash
$V scenario "$FORGE" context github-review gitlab-review
RUN=$($V launch --fake-forge "$FORGE")
$V wait --regex "$RUN" 'feature/auth-refresh +P'      # today; final: wait "$RUN" '#7 P'
$V wait --regex "$RUN" 'feature/rate-limit +& P'      # today; final: wait "$RUN" '!5 P'
$V capture "$RUN" first-paint
```

Expect, today: `app` row `feature/auth-refresh` has `P`; `services/api` row has `P`; the `app` worktree row `L feature/login-page` has `P`; both `merger` rows (`& feature/reconciliation`, `L feature/reconciliation`) have `m`. Final: the same rows end in `#7 P`, `!5 P`, `#8 P`, `#12 m`. `$FORGE/calls.log` has a `gh pr list` line and a `glab api` line.

**PASS:** all five rows show the expected badge on the captured screen and `calls.log` shows both CLIs were called.

## PS2 Every state, both forges

Each `sweep` swaps both forges to one state, reloads every checkout (`r` on the workspace row), and waits for both badges. The order matters: next to each other, two states never show the same glyph, so a wait cannot pass on the badge of the step before.

```bash
sweep() { # sweep <state> <github-regex> <gitlab-regex>
  $V scenario "$FORGE" "github-$1" "gitlab-$1" && to_top && $V keys "$RUN" r &&
    $V wait --regex "$RUN" "$2" && $V wait --regex "$RUN" "$3" &&
    $V capture "$RUN" "state-$1" >/dev/null && echo "PASS $1" || echo "FAIL $1"
}
# Today's build:
sweep ready   'feature/auth-refresh +A' 'feature/rate-limit +& A'
sweep draft   'feature/auth-refresh +P' 'feature/rate-limit +& P'   # final: d
sweep running 'feature/auth-refresh +A' 'feature/rate-limit +& A'   # final: r
sweep failed  'feature/auth-refresh +P' 'feature/rate-limit +& P'   # final: x
sweep merged  'feature/auth-refresh +m' 'feature/rate-limit +& m'
sweep review  'feature/auth-refresh +P' 'feature/rate-limit +& P'
# Final build (needs phase A): the same order with refs.
sweep ready   '#7 A' '!5 A'
sweep draft   '#7 d' '!5 d'
sweep running '#7 r' '!5 r'
sweep failed  '#7 x' '!5 x'
sweep merged  '#7 m' '!5 m'
sweep review  '#7 P' '!5 P'
```

**PASS:** every sweep prints PASS for its column, and each saved capture shows no other state glyph on those two rows. The states `draft`, `running` and `failed` are **needs phase A** in their final form (`d`, `r`, `x`); on an older build they must show the `P` / `A` of the table at the top.

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

## PS4 `gc` opens the CI page (needs phase A)

`gc` on a checkout row opens the CI page of its PR. GitHub: the first failed check, else the first running check, else the first check with a link, else `<PR URL>/checks`. GitLab: the head pipeline, else `<MR URL>/pipelines`. The sets list `lint` before `test (ubuntu-latest)` on purpose, so "first failed" differs from "first listed".

| Set | Row | `tail -n 1 "$RUN/opened.log"` after `g c` |
| --- | --- | --- |
| `github-failed` | `app` | `https://github.com/acme/app/actions/runs/501/job/6002` |
| `github-running` | `app` | `https://github.com/acme/app/actions/runs/501/job/6002` |
| `github-ready` | `app` | `https://github.com/acme/app/actions/runs/501/job/6001` |
| `gitlab-failed` | `services/api` | `https://gitlab.com/acme/services/api/-/pipelines/9001` |
| `gitlab-running` | `services/api` | `https://gitlab.com/acme/services/api/-/pipelines/9002` |
| `gitlab-ready` | `services/api` | `https://gitlab.com/acme/services/api/-/pipelines/9003` |

Steps per row: `$V scenario "$FORGE" <set>`, `to_top`, `r`, `wait` for the badge of that set, `to_app` or `to_api`, `$V keys "$RUN" g c`, then read `opened.log`. With `github-none`: `g c` says `no PR for feature/auth-refresh` and `opened.log` does not grow. A build before phase A leaves `opened.log` unchanged on `g c`: SKIP.

**PASS:** the last opened URL equals the table row, and the capture shows the status the app printed.

## PS5 No PR

```bash
$V scenario "$FORGE" github-none gitlab-none
to_top && $V keys "$RUN" r
$V wait --gone --regex "$RUN" 'feature/auth-refresh +[xArdPm] '   # today; final: --gone --regex "$RUN" '#7 [xArdPm] '
$V wait --gone --regex "$RUN" 'feature/rate-limit +& [xArdPm] '   # today; final: --gone --regex "$RUN" '!5 [xArdPm] '
to_app && $V keys "$RUN" g x
$V wait "$RUN" 'no PR for feature/auth-refresh'
```

**PASS:** the badge leaves both rows, and `gx` says `no PR for feature/auth-refresh`. A closed PR that was not merged counts as no PR; the sets have no closed PR, so only the empty list is proved here.

## PS6 Scripted transition, toast and total (needs phase A)

Polling needs a short interval: `WS_STATUS_PR_POLL_MS=1000`. The driver pins it to `0` (off) otherwise. An alert is a row flash plus a PR toast that lives 8 s; there is no alert on first load, on a branch change, or on the first lookup of a checkout.

```bash
$V scenario "$FORGE" context github-running gitlab-running
RUN=$($V launch --env WS_STATUS_PR_POLL_MS=1000 --fake-forge "$FORGE")
$V wait "$RUN" '#7 r' && $V wait "$RUN" '!5 r'
$V capture "$RUN" t0-first-load                      # no toast here
$V scenario "$FORGE" github-failed gitlab-failed     # no key: the poll finds it
$V wait "$RUN" 'x #7 CI failed · test (ubuntu-latest)' 10
$V wait "$RUN" 'x !5 CI failed' 10
$V wait --regex "$RUN" 'PRs · 2 need you' 10
$V capture "$RUN" t1-ci-failed
$V wait --gone "$RUN" 'CI failed' 12                 # the toast is gone after 8 s
```

Then: `github-review gitlab-review` (silent: badge `#7 P` / `!5 P`, no toast), `github-ready gitlab-ready` (toasts `A #7 ready to merge · feature/auth-refresh` and `A !5 ready to merge · feature/rate-limit`), `github-merged gitlab-merged` (toasts `m #7 merged · feature/auth-refresh` and `m !5 merged · feature/rate-limit`). On GitLab the text after `CI failed ·` is the first failed job or the branch.

**PASS:** t0 holds no toast text (`CI failed`, `ready to merge`, `merged`); each toast text appears after its swap with no key pressed; the `review` step prints no toast; the total shows `2 need you` while both PRs are failed. The row flash is colour only: compare the `.ansi` captures taken just before and just after a swap.

## PS7 A failing CLI keeps the last badge (needs phase A)

```bash
$V scenario "$FORGE" context github-review gitlab-review
RUN=$($V launch --env WS_STATUS_PR_POLL_MS=1000 --fake-forge "$FORGE")
$V wait "$RUN" '#7 P' && $V wait "$RUN" '!5 P'
$V scenario "$FORGE" github-cli-fails gitlab-cli-fails
sleep 4                                              # a few polls; the one sleep in this file
$V capture "$RUN" cli-fails
to_app && $V keys "$RUN" g h                         # pin the popover
$V wait "$RUN" 'last lookup failed: error connecting to api.github.com'
```

**PASS:** after the failing polls `#7 P` and `!5 P` are still on the screen (dimmed after the data is stale: compare `.ansi`), no toast appears, and the popover `checked` line carries the first stderr line of the fake CLI. Today's build hides the badge on a failed lookup and `gx` says `could not look up PR for feature/auth-refresh`: that is the behaviour the final design replaces.

## PS8 Nerd glyph shapes (needs phase A)

`launch --env WS_STATUS_GLYPHS=nerd`, then run the PS2 sweeps with `'#7 ✖' '!5 ✖'`, `'#7 ◉' '!5 ◉'`, `'#7 ◐' '!5 ◐'`, `'#7 ◌' '!5 ◌'`, `'#7 ○' '!5 ○'`, `'#7 ✔' '!5 ✔'` in the order of PS2.

**PASS:** each shape is on both rows, and the glyph is the last cell of the badge on a captured tree row.

## Live

Real `gh` and `glab` against public repos, read-only. Needs `gh auth status` and `glab auth status` to pass, and network. Pick the branches at run time, so the recipe does not rot when a PR closes:

```bash
V=scripts/verify-tui.sh
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
RUN=$($V launch --workspace "$WS")                  # no --fake-forge: the real gh and glab
```

Expect: the open PR rows get an open-PR badge (today `P` or `A`; final one of `x A r d P` with `#<n>` or `!<n>`), the merged rows get `m` (`#<n> m`). Then check `gx` on each row against the forge CLI itself:

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
```

**PASS:** every row shows a badge of the right kind (open or merged), and each `gx` URL equals the URL the forge CLI gives for that branch. SKIP, with the reason, when a CLI is missing or not logged in, when no same-repository PR of that kind exists, or when the network is down. Do not report a fake-forge run as the live result. **Needs phase A** for the state glyph and the ref; the live `#[ignore]` unit tests (`SKILL.md`, **Pick the layer**) check the same parse in code.

## Gotchas

- The first lookup runs at launch. Apply scenario sets **before** `launch` if the first paint matters; a later `scenario` needs `r` (or a poll) to show.
- `r` on a checkout row reloads that checkout; on the workspace row it reloads every checkout.
- `wait --regex` matches inside one screen line, and the tree row also carries the right pane text. Anchor on the branch name (`feature/auth-refresh +P`), not on `P` alone.
- `fake forge: no scenario for ...` in `calls.log` means the app made a call no set answers (a new `gh` / `glab` call). Teach the fake in `scripts/fake-forge/` and add the file to the sets.
- OSC 8 links on the badge are not in a tmux capture. The PTY e2e suite covers them.
