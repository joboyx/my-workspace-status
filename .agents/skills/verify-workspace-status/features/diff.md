# Diff pane

A dirty file row in the tree shows its diff in the right pane: a header line `<checkout>/<path>  <mode> · wrap` (plus ` · -w` while whitespace is ignored, DF4), a section line (`STAGED` or `UNSTAGED`), then the hunks (`@@ -a,b +c,d @@`) with old and new line numbers. The default mode is split, but split needs a diff pane of 100 columns or more: at the default `160x45` the pane has 96 columns and the header says `inline (too narrow)`. Detail: `docs/diff-rendering.md`.

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

## DF4 `w` hides a whitespace-only change, the palette row flips it, a restart keeps it

`w` (or `:` `whitespace` Enter) loads every diff with `git diff -w`: the header adds ` · -w`, and a diff whose every change is whitespace shows `(no diff)`. The value is saved in the run's UI-state file, so a second run on the same workspace and the same file starts with it. While it is on, a `V` stage refuses.

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
# A whitespace-only change to the tracked, clean app/src/config.ts.
printf '%s\n' 'export  const API_BASE = "/api";   ' 'export const SESSION_TTL_MS = 30 * 60 * 1000;' >"$RUN/workspace/app/src/config.ts"
$WS key r
$WS wait 'config\.ts'
$WS key j j j j
$WS wait --cursor 'config.ts'
$WS wait 'app/src/config\.ts +inline \(too narrow\) · wrap'
$WS wait '@@ -1,2 \+1,2 @@'
$WS wait --gone '· -w'
$WS key w
$WS wait 'app/src/config\.ts +inline \(too narrow\) · wrap · -w'
$WS wait 'whitespace hidden'
$WS wait '\(no diff\)'
$WS snap --name df4-hidden --raw | grep -F '(no diff)'
$WS key w
$WS wait 'whitespace shown'
$WS wait '@@ -1,2 \+1,2 @@'
$WS key :
$WS type whitespace
$WS wait 'Ignore / show whitespace'
$WS key Enter
$WS wait 'app/src/config\.ts +inline \(too narrow\) · wrap · -w'
jq -e '.ignoreWhitespace == true' "$RUN/state/ui-state.json"
$WS stop
# Same workspace, same UI-state file: the saved value wins at launch.
$WS start --no-build --workspace "$RUN/workspace" --env WS_STATUS_UI_STORE="$RUN/state/ui-state.json"
$WS key j j j j
$WS wait --cursor 'config.ts'
$WS wait 'app/src/config\.ts +inline \(too narrow\) · wrap · -w'
$WS wait '\(no diff\)'
$WS key k
$WS wait --cursor 'auth.ts'
$WS wait 'app/src/auth\.ts +inline \(too narrow\) · wrap · -w'
$WS key Enter
$WS key V
$WS wait 'VISUAL'
$WS key s
$WS wait 'turn whitespace on to stage hunks'
$WS snap --name df4-stage-refused --raw | grep -F 'turn whitespace on to stage hunks'
git -C "$RUN/workspace/app" diff --cached --quiet -- src/auth.ts
```

Expect: before `w` the `config.ts` diff shows its one hunk (`-export const API_BASE = "/api";`, `+export  const API_BASE = "/api";   `). On `df4-hidden` the header ends `· wrap · -w`, the body is `(no diff)`, the status says `whitespace hidden`, and `config.ts` stays in the tree. The second `w` brings the hunk back (`whitespace shown`). The palette row turns it on again and the UI-state file holds `"ignoreWhitespace": true`. After the restart the `config.ts` header has `· -w` with no key pressed. On `auth.ts` (a real change, so its hunks still show), `V` `s` refuses with `turn whitespace on to stage hunks` and nothing is staged.

**PASS:** every `wait` is ok, both `grep` lines match their captures, `jq -e` exits 0, and the last `git diff --cached --quiet` exits 0.
