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

## DF5 Long markdown lines with wrap on: word breaks, one-row hunk header, whitespace marks

A tracked `lib/GUIDE.md` in the `app` checkout has six long-line cases between `HEAD` and the worktree, each line wider than the 96-column diff pane: (1) trailing spaces added (line 3), (2) double spaces inside a line (5), (3) a real word change plus a double space (11), (4) a nested-list indent change (8), (5) a markdown hard break, two trailing spaces (10), (6) a tab changed to 4 spaces (13). Wrap is on (the default). Detail: `docs/diff-rendering.md` § Soft wrap, § Word highlight, § Ignore whitespace.

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS doctor
APP="$RUN/workspace/app"
mkdir -p "$APP/lib"
guide() { # $1: 0 = the HEAD copy, 1 = the worktree copy
  local t3='' t5='words that become double' t8='  -' t10='' t11='one word, also extra' t13=$'\t'
  if [ "$1" = 1 ]; then t3='   ' t5='words  that become  double' t8='    -' t10='  ' t11='two words, also  extra' t13='    '; fi
  printf '%s\n' '# Guide' '' \
    "This first paragraph line is long enough to wrap in the diff pane, and the worktree copy adds trailing spaces.$t3" '' \
    "Here the second paragraph has $t5 spaced in the worktree copy, so every later break moves." '' \
    '- A list item that has a nested item under it, and the line is long enough to wrap in the diff pane of the run.' \
    "$t8 The nested item moves to a deeper indent in the worktree copy, and this line is long enough to wrap too." '' \
    "This line ends with a markdown hard break in the worktree copy, which is two trailing spaces at the line end.$t10" \
    "The real change on this line is $t11 spaces are added, and the line is long enough to wrap." '' \
    "${t13}Tab indented line that becomes four spaces in the worktree copy, and it is long enough to wrap as well."
}
guide 0 >"$APP/lib/GUIDE.md"
git -C "$APP" add lib/GUIDE.md
git -C "$APP" -c user.name=verify -c user.email=verify@example.invalid commit -q -m 'add guide' -- lib/GUIDE.md
guide 1 >"$APP/lib/GUIDE.md"
$WS key r
$WS wait 'GUIDE\.md'
$WS key /
$WS wait 'SEARCH'
$WS type GUIDE
$WS key Enter
$WS wait --cursor 'GUIDE.md'
$WS key Escape
$WS wait 'app/lib/GUIDE\.md +inline \(too narrow\) · wrap'
$WS wait '@@ -1,13 \+1,13 @@'
SHOW=$($WS snap --name df5-show | jq -r .txt)
grep -F -- '-This first paragraph line is long enough to wrap in the diff pane, and the worktree' "$SHOW"
grep -E '│  copy adds trailing spaces\.' "$SHOW"
# Word background (diffDelWordBg / diffAddWordBg, default theme) on the 4 indent columns of the tab → spaces pair.
grep -aF $'\e[48;2;96;59;63m    \e[48;2;59;41;46mTab indented' "${SHOW%.txt}.ansi"
grep -aF $'\e[48;2;56;88;70m    \e[48;2;38;56;50mTab indented' "${SHOW%.txt}.ansi"
$WS key w
$WS wait 'app/lib/GUIDE\.md +inline \(too narrow\) · wrap · -w'
$WS wait '@@ -8,6 \+8,6 @@ Here the second paragraph'
IGN=$($WS snap --name df5-ignore | jq -r .txt)
grep -A1 -F '@@ -8,6 +8,6 @@' "$IGN" | tail -1 | grep -F '8 │'
# Only the changed words get the word background; the double space stays on the row background.
grep -aF $'\e[48;2;56;88;70mwords\e[48;2;38;56;50m, also  extra spaces' "${IGN%.txt}.ansi"
```

Expect on `df5-show` (whitespace shown): every wrapped code row breaks between words, never inside one: `… and the worktree` / `copy adds trailing spaces.`, `… in the worktree copy,` / `so every later break moves.`. The trailing spaces, double spaces, indent, and hard break get the word background, also on wrapped rows. Line 13 (`-` tab, `+` 4 spaces) paints the same text on both rows, and its 4 indent columns get the word background on both. On `df5-ignore` (`w`, ` · -w`): only line 11 stays changed. The hunk header `@@ -8,6 +8,6 @@ Here the second paragraph …` is clipped to one row, and the next row is line 8. On line 11 only `two` and `words` get the word background; the double space in `also  extra` does not.

**PASS:** every `wait` is ok and every `grep` matches its capture.
