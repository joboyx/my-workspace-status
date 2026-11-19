# Tree focus

A focus query narrows the left-pane tree to one feature or repo. `*` focuses the tree on the feature in the branch name of the cursor row (`feature/ABC-123-auth-refresh` → `ABC-123`); `*` again clears it. The palette (`:`) has `Focus: this feature` (chip `*`), `Focus: this repo`, `Focus: type a query…`, and `Focus: clear`. While a focus is set:

- the workspace row shows the chip `(o) <query> · <focused>/<total> repos` (the run uses ASCII glyphs; Nerd glyphs paint `◎`);
- a family keeps only its matching checkouts and shows `k/n wt`;
- the repos outside the focus fold into one dimmed last row `N outside focus · M dirty`, which opens to show them;
- workspace-row `f` / `p` / `d` cover the focused repos only, and the result toast says `Fetched N focused repos · M skipped`.

The focus is saved per workspace root in the run's UI-state file (`treeFocus`) and restored at launch with `focus restored: <query>`. Rules: `docs/tui-model.md` § Focus query, `docs/configuration.md` § Keymap.

Conventions and the sweep order: [README.md](README.md).

**Fixture.** The demo seed has no ticket-like branch names, so TF1, TF2, TF5, TF6 and TF7 rename two branches inside the run's own fixture, then reload the whole workspace with `r` on the workspace row (`r` on a file row reloads one checkout only):

| Repo | Branch after the rename | Under focus `ABC-123` |
| --- | --- | --- |
| `app` (primary + linked `feat-login`) | `feature/ABC-123-auth-refresh` (linked stays `feature/login-page`) | focused, `1/2 wt` |
| `services/api` | `feature/ABC-123-rate-limit` | focused |
| `merger` (primary + linked `recon`) | `feature/reconciliation` | outside, clean |
| `lib` | `main` | outside, clean |
| `notes` | hidden (`hiddenPaths`) | counted nowhere |

So the chip reads `(o) ABC-123 · 2/4 repos` and the outside row `2 outside focus · 0 dirty`. The rename lines are the same in each recipe:

```text
git -C "$RUN/workspace/app" branch -m feature/auth-refresh feature/ABC-123-auth-refresh
git -C "$RUN/workspace/services/api" branch -m feature/rate-limit feature/ABC-123-rate-limit
```

Absence checks read only the tree columns of a capture (`cut -c1-60`; the tree is 64 columns, and `cut` counts bytes, which are never fewer than columns), because the right pane shares the screen lines.

Each run waits for the first diff (`STAGED`, the `session.ts` diff header) before its first key: the app reads keys only after its first load, which can take more than the 10 s default `wait` on a busy host.

## TF1 `*` focuses the feature, `*` again clears it

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS doctor
$WS wait --timeout 60 'STAGED'
git -C "$RUN/workspace/app" branch -m feature/auth-refresh feature/ABC-123-auth-refresh
git -C "$RUN/workspace/services/api" branch -m feature/rate-limit feature/ABC-123-rate-limit
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key r
$WS wait '& feature/ABC-123-auth-refresh'
$WS wait 'services/api +feature/ABC-123-rate-limit'
$WS key j j j j j
$WS wait --cursor 'session.ts'
$WS key '*'
$WS wait 'focus: ABC-123'
$WS wait '\(o\) ABC-123 · 2/4 repos'
$WS wait '> 2 outside focus · 0 dirty'
$WS wait --cursor 'session.ts'
F=$($WS snap --name tf1-focused | jq -r .txt)
grep -E 'v app .*1/2 wt' "$F"
grep -E 'services/api +feature/ABC-123-rate-limit' "$F"
cut -c1-60 "$F" | grep -c -e 'login-page' -e 'merger' | grep -qx 0
$WS key '*'
$WS wait 'focus cleared'
$WS wait --gone 'ABC-123 · 2/4 repos'
$WS wait 'L feature/login-page'
$WS wait 'v merger'
$WS snap --name tf1-cleared --raw | grep -E 'v app .* 2 wt'
```

Expect on `tf1-focused`: the workspace row ends with the chip `(o) ABC-123 · 2/4 repos`; the `app` family row shows `1/2 wt` and keeps only `& feature/ABC-123-auth-refresh` (no `L feature/login-page` row); `services/api` stays; the last tree row is the folded `> 2 outside focus · 0 dirty`, so no `merger` or `lib` row shows. The status says `focus: ABC-123` and the cursor stays on `session.ts`. On `evidence/NN-tf1-focused.ansi` the `ABC-123` part of both branch names paints in the search-match style (bold, filter pill colours). After the second `*`: `focus cleared`, no chip, `app` shows `2 wt` and its linked row again, and `merger` is back.

**PASS:** every `wait` is ok, both `grep -E` lines match `tf1-focused`, the `cut` check counts no `login-page` or `merger` in the tree columns, and the last `grep` matches `tf1-cleared`.

## TF2 Typed query: live preview, then `Esc` restores the focus

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS wait --timeout 60 'STAGED'
git -C "$RUN/workspace/app" branch -m feature/auth-refresh feature/ABC-123-auth-refresh
git -C "$RUN/workspace/services/api" branch -m feature/rate-limit feature/ABC-123-rate-limit
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key r
$WS wait '& feature/ABC-123-auth-refresh'
$WS wait 'services/api +feature/ABC-123-rate-limit'
$WS key j j j j j
$WS wait --cursor 'session.ts'
$WS key '*'
$WS wait 'focus: ABC-123'
$WS key :
$WS type 'type a query'
$WS wait 'Focus: type a query'
$WS key Enter
$WS wait 'Focus tree'
$WS wait 'query: ABC-123'
$WS wait '2/4 repos in focus'
$WS snap --name tf2-open --raw | grep -F 'Enter apply · Esc restore · empty clears'
$WS key BSpace BSpace BSpace BSpace BSpace BSpace BSpace
$WS wait 'query: …'
$WS wait 'no focus · every repo shows'
$WS wait --gone 'outside focus'
$WS type lib
$WS wait 'query: lib'
$WS wait '1/4 repos in focus'
$WS wait '\(o\) lib · 1/4 repos'
$WS wait '> 3 outside focus · 2 dirty'
$WS snap --name tf2-preview --raw | grep -F '1/4 repos in focus'
jq -e '.treeFocus | to_entries | (length == 1 and .[0].value == "ABC-123")' "$RUN/state/ui-state.json"
$WS key Escape
$WS wait 'focus unchanged'
$WS wait --gone 'Focus tree'
$WS wait '\(o\) ABC-123 · 2/4 repos'
$WS wait '> 2 outside focus · 0 dirty'
$WS wait --cursor 'session.ts'
$WS snap --name tf2-restored --raw | grep -F '(o) ABC-123 · 2/4 repos'
```

Expect: `Focus: type a query…` opens the `Focus tree` box with `query: ABC-123`, `2/4 repos in focus`, and the hint `Enter apply · Esc restore · empty clears`; the panes behind it are not dimmed. Seven Backspaces empty the draft (`query: …`, `no focus · every repo shows`) and the tree shows every repo at once. Typing `lib` previews `lib` alone: `1/4 repos in focus`, the chip `(o) lib · 1/4 repos`, and `> 3 outside focus · 2 dirty` (`app` and `services/api` are dirty). The UI-state file still holds `ABC-123`: a preview does not save. `Esc` says `focus unchanged` and brings back the `ABC-123` tree with the cursor on `session.ts`.

**PASS:** every `wait` is ok, the three `grep` lines match, and `jq -e` exits 0.

## TF3 Palette rows: order, disabled reasons, `Focus: clear`

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS wait --timeout 60 'STAGED'
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key :
$WS type 'focus:'
$WS wait 'Focus: clear'
F=$($WS snap --name tf3-workspace-row | jq -r .txt)
[ "$(grep -oE 'Focus: (this feature|this repo|type a query|clear)' "$F" | paste -sd '|' -)" = 'Focus: this feature|Focus: this repo|Focus: type a query|Focus: clear' ]
grep -E 'Focus: this feature .*focus a repo or checkout' "$F"
grep -E 'Focus: this repo .*focus a repo or checkout' "$F"
grep -E 'Focus: clear .*no focus set' "$F"
$WS key Escape
$WS key Escape
$WS wait --gone 'Focus: clear'
$WS key j j j j j
$WS wait --cursor 'session.ts'
$WS key '*'
$WS wait 'focus: auth-refresh'
$WS wait '\(o\) auth-refresh · 1/4 repos'
$WS key :
$WS type 'focus:'
$WS wait 'Focus: clear'
F=$($WS snap --name tf3-file-row | jq -r .txt)
grep -c -e 'focus a repo or checkout' -e 'no focus set' "$F" | grep -qx 0
$WS type ' clear'
$WS wait --gone 'Focus: this repo'
$WS key Enter
$WS wait 'focus cleared'
$WS wait --gone 'auth-refresh · 1/4 repos'
$WS snap --name tf3-cleared --raw | cut -c1-60 | grep -E 'v merger'
```

Expect on `tf3-workspace-row`: the filter `focus:` lists exactly the four rows in the order `Focus: this feature` (chip `*`), `Focus: this repo`, `Focus: type a query…`, `Focus: clear`. On the workspace row the first two are dimmed with `focus a repo or checkout` at the right edge, and `Focus: clear` with `no focus set`; the cursor lands on `Focus: type a query…`, the first enabled row. On `session.ts` (seed branch `feature/auth-refresh`, no ticket) `*` takes the fallback token `auth-refresh` (`1/4 repos`), and on `tf3-file-row` no row has a reason. `Focus: clear` runs and says `focus cleared` (a short status: read it with `wait`, not a later capture), and `tf3-cleared` shows `v merger` again.

**PASS:** every `wait` is ok, the `[ … ]` order test is true, the three reason `grep` lines match `tf3-workspace-row`, the count check finds no reason on `tf3-file-row`, and the last `grep` matches `tf3-cleared`.

## TF4 `Focus: this repo` keeps the whole family

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS wait --timeout 60 'STAGED'
$WS key /
$WS wait 'SEARCH'
$WS type rate-limit
$WS key Enter
$WS wait --cursor 'services/api'
$WS key Escape
$WS wait 'search cleared'
$WS key :
$WS type 'this repo'
$WS wait 'Focus: this repo'
$WS key Enter
$WS wait 'focus: services/api'
$WS wait '\(o\) services/api · 1/4 repos'
$WS wait '> 3 outside focus · 1 dirty'
$WS wait --cursor 'services/api'
$WS snap --name tf4-api --raw | grep -F '(o) services/api · 1/4 repos'
$WS key :
$WS type 'focus: clear'
$WS wait --gone 'Focus: this repo'
$WS key Enter
$WS wait 'focus cleared'
$WS key /
$WS wait 'SEARCH'
$WS type login-page
$WS key Enter
$WS wait --cursor 'L feature/login-page'
$WS key Escape
$WS wait 'search cleared'
$WS key :
$WS type 'this repo'
$WS wait 'Focus: this repo'
$WS key Enter
$WS wait 'focus: app'
$WS wait '\(o\) app · 1/4 repos'
$WS wait '> 3 outside focus · 1 dirty'
$WS wait --cursor 'L feature/login-page'
F=$($WS snap --name tf4-app | jq -r .txt)
grep -E 'v app .* 2 wt' "$F"
grep -F '& feature/auth-refresh' "$F"
cut -c1-60 "$F" | grep -c '1/2 wt' | grep -qx 0
```

Expect: no rename here. On the `services/api` row, `Focus: this repo` sets the query `services/api` (`1/4 repos`; `app` is the one dirty repo outside). On the linked worktree row `L feature/login-page`, it sets the family's primary path `app`, so the whole family stays: `v app` shows `2 wt` (not `1/2 wt`), both checkout rows show, and the cursor stays on the linked row. Outside: `merger`, `lib`, and the dirty `services/api` (`3 outside focus · 1 dirty`).

**PASS:** every `wait` is ok, the `grep` lines match `tf4-api` and `tf4-app`, and the count check finds no `1/2 wt` on `tf4-app`.

## TF5 The outside row opens, its rows paint dimmed, the focus stays

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS wait --timeout 60 'STAGED'
git -C "$RUN/workspace/app" branch -m feature/auth-refresh feature/ABC-123-auth-refresh
git -C "$RUN/workspace/services/api" branch -m feature/rate-limit feature/ABC-123-rate-limit
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key r
$WS wait '& feature/ABC-123-auth-refresh'
$WS wait 'services/api +feature/ABC-123-rate-limit'
$WS key j j j j j
$WS wait --cursor 'session.ts'
$WS key '*'
$WS wait '> 2 outside focus · 0 dirty'
$WS key G
$WS wait --cursor '2 outside focus · 0 dirty'
$WS key z
$WS wait --cursor 'v 2 outside focus · 0 dirty'
$WS wait 'v merger'
$WS wait 'lib +main'
$WS wait '\(o\) ABC-123 · 2/4 repos'
F=$($WS snap --name tf5-open | jq -r .txt)
cut -c1-60 "$F" | awk '/v merger/ { m = NR } / lib +main/ { l = NR } END { exit !(m && l && m < l) }'
$WS key j
$WS wait --cursor 'v merger'
$WS wait 'merge billing into main'
$WS wait '\(o\) ABC-123 · 2/4 repos'
$WS snap --name tf5-merger --raw | grep -F 'merge billing into main'
```

Expect: `G` lands on the folded outside row, `z` opens it (`v 2 outside focus · 0 dirty`). Under it the outside families list in the normal order with no nested `No updates` group: the attention family `merger` (both `feature/reconciliation` checkouts), then the clean `lib  main`, with no clean check. On `NN-tf5-open.ansi` the outside row and every row under it paint in the muted colour, like a hidden repo. The chip stays: opening the row does not clear the focus. The cursor can move into it, and `merger` shows its graph (`merge billing into main`).

**PASS:** every `wait` is ok, the `awk` check finds `v merger` above `lib  main` in the tree columns of `tf5-open`, and the `grep` matches `tf5-merger`.

## TF6 Workspace-row `f` fetches the focused repos only

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS wait --timeout 60 'STAGED'
git -C "$RUN/workspace/app" branch -m feature/auth-refresh feature/ABC-123-auth-refresh
git -C "$RUN/workspace/services/api" branch -m feature/rate-limit feature/ABC-123-rate-limit
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key r
$WS wait '& feature/ABC-123-auth-refresh'
$WS wait 'services/api +feature/ABC-123-rate-limit'
$WS key j
$WS wait --cursor 'v app'
$WS key '*'
$WS wait 'focus: ABC-123'
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key f
$WS wait --timeout 30 'Fetched 2 focused repos · 2 skipped'
$WS snap --name tf6-focused --raw | grep -F 'Fetched 2 focused repos · 2 skipped'
$WS key :
$WS type 'focus: clear'
$WS wait --gone 'Focus: this repo'
$WS key Enter
$WS wait 'focus cleared'
$WS key f
$WS wait --timeout 30 'Fetched 4 repos'
$WS snap --name tf6-all --raw | grep -F 'Fetched 4 repos'
```

Expect: `*` on the `app` family row reads the primary checkout's branch (`ABC-123`). `f` on the workspace row fetches the primaries of the focused families, `app` and `services/api`, over the seed's local remotes; the result toast (bottom right of the panes) says `Fetched 2 focused repos · 2 skipped`. After `Focus: clear`, `f` fetches all four visible primaries (`app`, `lib`, `merger`, `services/api`; `notes` is hidden) and says `Fetched 4 repos`, the text with no focus. The toast fades after a few seconds, so read it right after the `wait`.

**PASS:** every `wait` is ok and both `grep` lines match.

## TF7 A relaunch restores the focus with a one-time notice

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
RUN=$($WS start | jq -r .run)
trap '$WS stop' EXIT
$WS wait --timeout 60 'STAGED'
git -C "$RUN/workspace/app" branch -m feature/auth-refresh feature/ABC-123-auth-refresh
git -C "$RUN/workspace/services/api" branch -m feature/rate-limit feature/ABC-123-rate-limit
$WS key --chord g g
$WS wait --cursor '# workspace'
$WS key r
$WS wait '& feature/ABC-123-auth-refresh'
$WS wait 'services/api +feature/ABC-123-rate-limit'
$WS key j j j j j
$WS wait --cursor 'session.ts'
$WS key '*'
$WS wait 'focus: ABC-123'
jq -e '.treeFocus | to_entries | (length == 1 and .[0].value == "ABC-123")' "$RUN/state/ui-state.json"
$WS stop
# Same workspace, same UI-state file: the saved focus comes back at launch.
$WS start --no-build --workspace "$RUN/workspace" --env WS_STATUS_UI_STORE="$RUN/state/ui-state.json"
$WS wait 'focus restored: ABC-123'
$WS wait '\(o\) ABC-123 · 2/4 repos'
$WS wait '> 2 outside focus · 0 dirty'
$WS wait --cursor 'session.ts'
$WS snap --name tf7-restored --raw | grep -F 'focus restored: ABC-123'
$WS wait --timeout 60 'STAGED'
$WS key '*'
$WS wait 'focus cleared'
$WS snap --name tf7-cleared --raw | cut -c1-60 | grep -F 'L feature/login-page'
jq -e 'has("treeFocus") | not' "$RUN/state/ui-state.json"
```

Expect: the first run saves `"treeFocus": {"<workspace id>": "ABC-123"}` in its UI-state file (the id is the SHA-256 of the workspace root path). The second run, on the same fixture and file, starts focused with no key pressed: the status line says `focus restored: ABC-123`, the chip and the folded outside row are there, and the cursor is on `session.ts`. `*` then clears the focus (`tf7-cleared` shows the linked `L feature/login-page` row again) and removes the key from the file.

**PASS:** every `wait` is ok, the `grep` lines match `tf7-restored` and `tf7-cleared`, and both `jq -e` exit 0.
