# Pane split

The tree pane keeps a remembered width in columns for the session (`AppState::tree_cols`). A launch sets it to 40% of the terminal width, at most 80 columns (`TREE_LAUNCH_MAX_COLS`). A wider terminal gives its extra columns to the right pane. A narrower terminal clamps only the painted tree, so the right pane keeps 20 columns (`MIN_PANE_COLS`) plus its 2 pad columns; the tree gets its remembered columns back when the terminal grows. `<` / `>` and a divider drag set the remembered width; each key press moves it by 5% of the current terminal width, rounded. Detail: `docs/tui-rust.md` (split rule), `crates/workspace-status/src/tui/split.rs`.

Conventions and the sweep order: [README.md](README.md).

## Measure the tree width

The pane title row (line 2 of a capture, flat look, the default) starts with ` tree ` and an `_` rule, and the right pane title ` diff ` starts on the first column of the right pane. For a tree `W` columns wide in a terminal `C` columns wide, the whole line is:

```text
 tree <W − 6 × _> diff <C − W − 6 × _>
```

The recipes `wait` for that whole line with an extended regex, for example `'^ tree _{58} diff _{90}$'` for a 64-column tree at 160 columns. The rule length after ` diff ` names the terminal width, so an old frame left from before a resize does not match. To print the tree width of a saved capture:

```bash
awk 'NR == 2 { print index($0, " diff ") - 1 }' "$F"   # $F: evidence/NN-<name>.txt
```

| Terminal | Tree | Title line regex |
| --- | --- | --- |
| 160 | 64 (launch) | `^ tree _{58} diff _{90}$` |
| 220 | 64 | `^ tree _{58} diff _{150}$` |
| 220 | 75 (`>` at 220) | `^ tree _{69} diff _{139}$` |
| 80 | 58 (clamped) | `^ tree _{52} diff _{16}$` |
| 300 | 80 (launch cap) | `^ tree _{74} diff _{214}$` |

## SP1 A wider terminal keeps the tree columns

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS doctor
$WS wait '^ tree _{58} diff _{90}$'
$WS wait '@@ -2,7 \+2,7 @@'
A=$($WS snap --name sp1-160 | jq -r .txt)
$WS resize 220x45
$WS wait-ready
$WS wait '^ tree _{58} diff _{150}$'
$WS snap --name sp1-220 --raw | grep -E '^ tree _{58} diff _{150}$'
$WS resize 160x45
$WS wait '^ tree _{58} diff _{90}$'
$WS wait '@@ -2,7 \+2,7 @@'
B=$($WS snap --name sp1-back | jq -r .txt)
cmp "$A" "$B"
$WS doctor
```

Expect: at `160x45` the tree has 64 columns and the right pane 96. On `sp1-220` the tree still has 64 columns and the right pane 156: the extra 60 columns go to the right pane. Back at `160x45` the screen is the same as before the resize.

**PASS:** every `wait` is ok, the `grep` matches `sp1-220`, `cmp` exits 0, and `doctor` is ok.

## SP2 Shrink, then grow: the tree gets its remembered width back

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS doctor
$WS resize 220x45
$WS wait '^ tree _{58} diff _{150}$'
$WS key '>'
$WS wait '^ tree _{69} diff _{139}$'
$WS resize 80x45
$WS wait '^ tree _{52} diff _{16}$'
$WS snap --name sp2-80 --raw | grep -E '^ tree _{52} diff _{16}$'
$WS resize 220x45
$WS wait '^ tree _{69} diff _{139}$'
$WS snap --name sp2-220 --raw | grep -E '^ tree _{69} diff _{139}$'
$WS doctor
```

Expect: the 160-column launch gives a 64-column tree, and the resize to `220x45` keeps it. `>` at 220 columns adds round(5% of 220) = 11 columns: 75. On `sp2-80` the tree is clamped to 58 columns (80 − 20 − 2), so the right pane keeps its minimum. On `sp2-220` the tree has its 75 columns again, not the clamped 58.

**PASS:** every `wait` is ok, both `grep` lines match, and `doctor` is ok.

## SP3 `<` / `>` set the remembered width

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS doctor
$WS wait '^ tree _{58} diff _{90}$'
$WS key '>'
$WS wait '^ tree _{66} diff _{82}$'
$WS resize 200x45
$WS wait '^ tree _{66} diff _{122}$'
$WS key '<'
$WS wait '^ tree _{56} diff _{132}$'
$WS resize 160x45
$WS wait '^ tree _{56} diff _{92}$'
$WS snap --name sp3-160 --raw | grep -E '^ tree _{56} diff _{92}$'
$WS doctor
```

Expect: at `160x45`, `>` adds round(5% of 160) = 8 columns: 72. The resize to `200x45` keeps 72. `<` at 200 columns takes round(5% of 200) = 10 columns: 62. Back at `160x45` the tree keeps 62 columns, not the launch 64.

**PASS:** every `wait` is ok, the `grep` matches `sp3-160`, and `doctor` is ok.

The driver has no mouse drag (`scripts/verify-tui.sh mouse` sends `move`, `click`, and `wheel` only), so no recipe drags the divider. The drag sets the same remembered width: unit tests `flat_divider_pad_press_and_drag_resizes` and `stepwise_terminal_resize_keeps_the_launch_tree_columns_on_every_surface`, PTY test `pty_divider_scrollbar_drag`.

## SP4 A wide launch stops the tree at 80 columns

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start --size 300x45
trap '$WS stop' EXIT
$WS doctor
$WS wait '^ tree _{74} diff _{214}$'
$WS snap --name sp4-300 --raw | grep -E '^ tree _{74} diff _{214}$'
```

Expect: on `sp4-300` the tree has 80 columns (`TREE_LAUNCH_MAX_COLS`), not 40% of 300 (120), and the right pane 220.

**PASS:** `doctor` and the `wait` are ok, and the `grep` matches `sp4-300`.
