# Tree navigation

The left pane is the workspace tree. `j` / `k` move the cursor one row; the focus bar `▌` marks the cursor row of the focused pane, and the other pane shows a thin `▏`. The right pane follows the cursor: a file row shows its diff, a folder or checkout row shows the commit graph. `Enter` on a file focuses the diff, `Esc` goes back. `h` folds the folder under the cursor and `l` opens it; `z` toggles the fold of any row. Key map: `crates/workspace-status/src/tui/keys.rs`, `docs/tui-model.md`.

Conventions and the sweep order: [README.md](README.md). Rows of the demo fixture from the first cursor down:

```text
    v app                                           @ ^1 2 wt 4
      v & feature/auth-refresh                             ^1 3
        v # Staged
          v / src
▌             · session.ts                                   S      first cursor
        v ~ Changes
          v / src
              · auth.ts                                      M
              · login.ts                                     A
```

## TN1 `j` / `k` move one row per press

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
$WS doctor
trap '$WS stop' EXIT
$WS wait --cursor 'session.ts'
$WS key j
$WS wait --cursor '~ Changes'
$WS key j
$WS wait --cursor '/ src'
$WS key j
$WS wait --cursor 'auth.ts'
$WS key j
$WS wait --cursor 'login.ts'
$WS key k
$WS wait --cursor 'auth.ts'
$WS snap --name tn1-auth --raw | grep -E '▌ +· auth\.ts +M'
```

Expect: each `wait` is ok; on `tn1-auth` the focus bar is on the `auth.ts` row (`▌             · auth.ts   M`) and the right pane shows the diff of `app/src/auth.ts`.

**PASS:** every `wait` is ok and the `grep` finds the focus bar on `auth.ts` in `evidence/NN-tn1-auth.txt`. Four `j` and one `k` moved exactly five rows: a held-key repeat that moved two rows would put the bar on another row and fail a `wait`.

## TN2 `Enter` focuses the diff, `Esc` comes back

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS key j j j
$WS wait --cursor 'auth.ts'
$WS wait 'app/src/auth\.ts'
$WS key Enter
$WS wait --cursor 'UNSTAGED'
$WS snap --name tn2-diff-focus --raw | grep -E '▏ +· auth\.ts'
$WS key Escape
$WS wait --cursor 'auth.ts'
$WS snap --name tn2-back --raw | grep -E '▌ +· auth\.ts'
```

Expect on `tn2-diff-focus`: the focus bar `▌` is on the first diff line (`UNSTAGED`) and the tree row `auth.ts` has the thin `▏`. On `tn2-back` the bar is on `auth.ts` again.

**PASS:** both `grep` lines match their captures.

## TN3 Folds: `h` / `l` on a folder, `z` on a checkout

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS key j
$WS wait --cursor '~ Changes'
$WS key j
$WS wait --cursor 'v / src'
$WS key h
$WS wait --cursor '> / src'
$WS wait --gone 'login\.ts'
$WS snap --name tn3-folder-folded --raw | grep -E '▌ +> / src'
$WS key l
$WS wait --cursor 'v / src'
$WS wait 'login\.ts'
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key j
$WS wait --cursor 'v app'
$WS key z
$WS wait --cursor '> app'
$WS wait --gone 'session\.ts'
$WS snap --name tn3-app-folded --raw | grep -E '▌ +> app'
$WS key z
$WS wait --cursor 'v app'
$WS wait 'session\.ts'
```

Expect: `h` on the `Changes` → `src` folder turns `v / src` into `> / src` and hides `auth.ts` and `login.ts`; `l` shows them again. `gg` (a chord: two keys in one write) jumps to the `# workspace` row. `z` on `app` folds the whole checkout (`> app`, no `session.ts` on screen), and the next `z` opens it.

**PASS:** every `wait` is ok and both `grep` lines match: the folded rows are on captured screens.
