# Diff pane

A dirty file row in the tree shows its diff in the right pane: a header line `<checkout>/<path>  <mode> · wrap`, a section line (`STAGED` or `UNSTAGED`), then the hunks (`@@ -a,b +c,d @@`) with old and new line numbers. The default mode is split, but split needs a diff pane of 100 columns or more: at the default `160x45` the pane has 96 columns and the header says `inline (too narrow)`. Detail: `docs/diff-rendering.md`.

Conventions and the sweep order: [README.md](README.md). Demo fixture changes (`docs/demo.md` § Workspace): `session.ts` staged (`session expired` → `session expired — sign in again`), `auth.ts` unstaged (refresh window `5m` → `2m`, plus `withRefreshedExpiry`), `login.ts` untracked.

## DF1 The staged diff of the first cursor

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS doctor
$WS wait --cursor 'session.ts'
$WS wait 'app/src/session\.ts +inline \(too narrow\)'
$WS wait 'STAGED'
$WS wait '@@ -2,7 \+2,7 @@'
$WS snap --name df1-session --raw | grep -F '+    throw new Error("session expired — sign in again");'
```

Expect on `df1-session`, right pane:

```text
app/src/session.ts  inline (too narrow) · wrap
 STAGED
@@ -2,7 +2,7 @@ import { type Session, isExpired, needsRefresh } from "./auth.ts";
  5 │ -    throw new Error("session expired");
  5 │ +    throw new Error("session expired — sign in again");
```

**PASS:** every `wait` is ok and the `grep` finds the added line in the capture.

## DF2 Select a dirty file: the unstaged hunks of `auth.ts`

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS key j j j
$WS wait --cursor 'auth.ts'
$WS wait 'app/src/auth\.ts'
$WS wait 'UNSTAGED'
$WS wait '@@ -4,7 \+4,7 @@'
$WS wait '@@ -13,3 \+13,11 @@'
F=$($WS snap --name df2-auth | jq -r .txt)      # the capture file in evidence/
grep -F -- '-const REFRESH_MS = 5 * 60 * 1000;' "$F"
grep -F -- '+const REFRESH_MS = 2 * 60 * 1000;' "$F"
grep -F -- '+export function withRefreshedExpiry(' "$F"
```

Expect on `df2-auth`: the header `app/src/auth.ts`, `UNSTAGED`, two hunks (`@@ -4,7 +4,7 @@` and `@@ -13,3 +13,11 @@`), the removed `-const REFRESH_MS = 5 * 60 * 1000;`, the added `+const REFRESH_MS = 2 * 60 * 1000;` and `+export function withRefreshedExpiry(`.

**PASS:** every `wait` is ok and the three `grep` lines match `evidence/NN-df2-auth.txt`.

## DF3 Split at 200 columns, inline again at 160

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS key j j j
$WS wait --cursor 'auth.ts'
$WS wait 'app/src/auth\.ts +inline \(too narrow\)'
$WS resize 200x45
$WS wait 'app/src/auth\.ts +split · wrap'
$WS snap --name df3-split --raw | grep -E '7 │ -const REFRESH_MS = 5 \* 60 \* 1000; +│ +7 │ \+const REFRESH_MS = 2'
$WS resize 160x45
$WS wait 'app/src/auth\.ts +inline \(too narrow\)'
$WS doctor
```

Expect: at `200x45` the tree has 80 columns and the diff pane 120, so the header says `split · wrap` and one screen line holds the old line 7 on the left and the new line 7 on the right of the `│` gutter. Back at `160x45` the header says `inline (too narrow)` again, and `doctor` checks the pane size against the last resize.

**PASS:** both `wait` lines on the header are ok, the `grep` finds the side-by-side line on `df3-split`, and `doctor` is ok.
