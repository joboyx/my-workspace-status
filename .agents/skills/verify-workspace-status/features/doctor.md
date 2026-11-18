# Doctor setup checkup

The `:` palette row `Doctor: setup checkup` opens a dialog that checks the setup the TUI needs (`docs/configuration.md` § Doctor setup checkup). Each row has a chip (`ok`, `warn`, `fail`, or `?` for unknown), a name, and what was found; the head counts the rows as `<n> ok · <n> warn · <n> fail · <n> unknown`, or shows `checking…` while the probe runs and the terminal's answers come in (at most 200 ms). The TERMINAL group has six capability rows that ask the terminal (XTVERSION, DECRQM, kitty flags, OSC 11, DA1) and the live row `Mouse wheel (live test)`. `ws doctor` prints the same rows as plain text.

Conventions and the sweep order: [README.md](README.md). The recipes launch at `160x70` so that the whole TERMINAL group is on screen. D3 and D4 also need the driver's `mouse` command (the run dir comes from `start`).

Two facts of the driver's private tmux server, so a reader is not confused:

- **tmux answers the capability queries.** The TUI runs inside tmux, so tmux, not an outer terminal, answers (every capability row has the hint `Inside tmux, tmux answers for the outer terminal.`). What tmux answers depends on its version: a row is `ok` when tmux answered and `?` when it did not, never `fail`.
- **tmux `mouse` is off there.** The `Mouse` row shows `warn` (`On, but tmux mouse is off: tmux does not forward clicks and the wheel.`). That is expected: `scripts/verify-tui.sh mouse` writes the wheel report into the pane with `send-keys`, so the app still gets it, as it gets a report from a terminal.

On a build without the capability rows (a head with three counts), D1 to D4 are SKIP; on a build without `ws doctor`, D5 is SKIP (the driver refuses `doctor`, and the binary reads it as a repo filter: `Unknown repo: doctor`, exit 1).

## D1 The palette opens the Doctor, the head has four counts

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start --size 160x70
trap '$WS stop' EXIT
$WS doctor
$WS wait '\? help'
$WS key :
$WS type doctor
$WS wait 'Doctor: setup checkup'
$WS key Enter
$WS wait 'Doctor · setup checkup'
$WS wait '[0-9]+ ok · [0-9]+ warn · [0-9]+ fail · [0-9]+ unknown'
F=$($WS snap --name d1-open | jq -r .txt)
grep -F 'Doctor · setup checkup' "$F"
grep -E '[0-9]+ ok · [0-9]+ warn · 0 fail · [0-9]+ unknown' "$F"
grep -F '> ok   Terminal size' "$F"
grep -F 'j/k move · y copy fix · r re-run · Esc close' "$F"
```

Expect on `d1-open`: the boxed dialog with the title `Doctor · setup checkup`, the head with four counts and `0 fail` (example from a run: `14 ok · 3 warn · 0 fail · 5 unknown`), the group title `TERMINAL`, the cursor `>` on the first row `ok   Terminal size` (`160×70`), and the footer `j/k move · y copy fix · r re-run · Esc close`.

**PASS:** every `wait` is ok and all four `grep` lines match `d1-open`.

## D2 The capability rows show tmux's answers or unknown, never fail

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start --size 160x70
trap '$WS stop' EXIT
$WS doctor
$WS wait '\? help'
$WS key :
$WS type doctor
$WS wait 'Doctor: setup checkup'
$WS key Enter
$WS wait '[0-9]+ ok · [0-9]+ warn · [0-9]+ fail · [0-9]+ unknown'
F=$($WS snap --name d2-caps | jq -r .txt)
for name in 'Terminal (XTVERSION)' 'SGR mouse (1006)' 'Bracketed paste (2004)' \
  'Synchronized output (2026)' 'Kitty keyboard (CSI u)' 'Background (OSC 11)'; do
  line=$(grep -F "$name" "$F")
  case "$line" in
    *"ok   $name"* | *"?    $name"*) echo "ok or unknown: $name" ;;
    *) echo "FAIL: $line"; exit 1 ;;
  esac
done
[ "$(grep -cF 'Inside tmux, tmux answers for the outer terminal.' "$F")" -eq 6 ]
```

Expect on `d2-caps`: the six rows after `Mouse wheel (live test)`, in this order, each with the chip `ok` or `?` and the tmux hint under it. A `?` row's text ends with `(no answer)` when the DA1 reply ended the wait, or `(no answer within 200 ms)` when the time ran out. Example, labelled as such, from tmux 3.4, which answered XTVERSION and DA1 only:

```text
  ok   Terminal (XTVERSION)
       tmux 3.4
  ?    SGR mouse (1006)
       The terminal did not report mode 1006. (no answer)
  ?    Bracketed paste (2004)
       The terminal did not report mode 2004. (no answer)
  ?    Synchronized output (2026)
       The terminal did not report mode 2026. (no answer)
  ok   Kitty keyboard (CSI u)
       Not supported (optional): keys use the legacy encoding.
  ?    Background (OSC 11)
       The terminal did not report its background: themeMode auto keeps the configured
       theme. (no answer)
```

(Each row also has the line `Inside tmux, tmux answers for the outer terminal.`) Another tmux version can answer more (an `ok` row with `Supported.`); that is a PASS too.

**PASS:** every `wait` is ok, the loop prints `ok or unknown` for all six names and no `FAIL`, and the tmux hint is on the screen six times.

## D3 A wheel report over the dialog turns the live wheel row ok

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
V=scripts/verify-tui.sh
RUN=$($WS start --size 160x70 | jq -r .run)
trap '$WS stop' EXIT
$WS doctor
$WS wait '\? help'
$WS key :
$WS type doctor
$WS wait 'Doctor: setup checkup'
$WS key Enter
$WS wait '[0-9]+ ok · [0-9]+ warn · [0-9]+ fail · [0-9]+ unknown'
$WS snap --name d3-before --raw | grep -F '?    Mouse wheel (live test)'
CELL=$($V mouse "$RUN" find 'Doctor · setup checkup')
$V mouse "$RUN" wheel down $CELL
$WS wait 'ok   Mouse wheel \(live test\)'
F=$($WS snap --name d3-after | jq -r .txt)
grep -F 'A wheel event arrived: the terminal sends mouse reports.' "$F"
grep -F '> ok   UTF-8 locale' "$F"
grep -F 'warn Mouse' "$F"
```

Expect on `d3-before`: `?    Mouse wheel (live test)` with `Scroll the mouse wheel once over this dialog.` On `d3-after`: `ok   Mouse wheel (live test)` with `A wheel event arrived: the terminal sends mouse reports.`, and the cursor one row down on `> ok   UTF-8 locale` (wheel down moves the cursor one row). The `Mouse` row stays `warn` (tmux `mouse` is off in the run, see above); the wheel report still reached the app.

**PASS:** every `wait` is ok and the four `grep` lines match their captures.

## D4 A fresh open resets the test, and Down arrow turns it warn

The wheel result stays while the Doctor is open (later arrow keys do not downgrade it) and resets when the Doctor opens again. So this recipe scrolls first, closes, opens the Doctor again, and presses Down.

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
V=scripts/verify-tui.sh
RUN=$($WS start --size 160x70 | jq -r .run)
trap '$WS stop' EXIT
$WS doctor
$WS wait '\? help'
$WS key :
$WS type doctor
$WS wait 'Doctor: setup checkup'
$WS key Enter
$WS wait '[0-9]+ ok · [0-9]+ warn · [0-9]+ fail · [0-9]+ unknown'
CELL=$($V mouse "$RUN" find 'Doctor · setup checkup')
$V mouse "$RUN" wheel up $CELL
$WS wait 'ok   Mouse wheel \(live test\)'
$WS key Escape
$WS wait --gone 'Doctor · setup checkup'
$WS wait '\? help'
$WS key :
$WS type doctor
$WS wait 'Doctor: setup checkup'
$WS key Enter
$WS wait '[0-9]+ ok · [0-9]+ warn · [0-9]+ fail · [0-9]+ unknown'
$WS snap --name d4-reopen --raw | grep -F '?    Mouse wheel (live test)'
$WS key Down
$WS wait 'warn Mouse wheel \(live test\)'
F=$($WS snap --name d4-down | jq -r .txt)
grep -F 'Got Up/Down arrow keys and no wheel event.' "$F"
grep -F 'macOS Terminal.app: View → Allow Mouse Reporting (Cmd+R).' "$F"
grep -F 'In tmux: set -g mouse on.' "$F"
grep -F '> ok   UTF-8 locale' "$F"
```

Expect on `d4-reopen`: the row is back to `?    Mouse wheel (live test)` (`Scroll the mouse wheel once over this dialog.`). On `d4-down` (the box wraps the long lines):

```text
  warn Mouse wheel (live test)
       Got Up/Down arrow keys and no wheel event. If you scrolled, the terminal turns the
       wheel into arrow keys: mouse reporting is off.
       macOS Terminal.app: View → Allow Mouse Reporting (Cmd+R). In tmux: set -g mouse on.
       If you pressed the arrow keys, scroll once to test again.
       set -g mouse on
```

The last line is the fix to copy with `y`: the run is inside tmux, so it is there. The Down arrow still moves the cursor (`> ok   UTF-8 locale`).

**PASS:** every `wait` is ok and the five `grep` lines match their captures.

## D5 `ws doctor` prints the text report and exits 0

`wsctl run -- doctor` runs the binary headless through the driver: stdin is `/dev/null` and stdout a file, so `ws doctor` asks the terminal nothing and cannot read the terminal size.

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS doctor
R=$($WS run -- doctor)
echo "$R" | jq -e '.ok == true and .rc == 0'
OUT=$(echo "$R" | jq -r .files.out)
head -n 2 "$OUT"
head -n 1 "$OUT" | grep -E '^Doctor · setup checkup · workspace-status v[0-9]+\.[0-9]+\.[0-9]+'
sed -n 2p "$OUT" | grep -E '^[0-9]+ ok · [0-9]+ warn · 0 fail · [0-9]+ unknown$'
grep -A1 -F 'unknown Mouse wheel (live test)' "$OUT" | grep -F 'TUI only: open Doctor: setup checkup in the TUI and scroll once.'
grep -A1 -F 'unknown Terminal size' "$OUT" | grep -F 'stdout is not a terminal.'
[ "$(grep -cF 'Not asked: stdin or stdout is not a terminal.' "$OUT")" -eq 6 ]
if grep -q $'\e' "$OUT"; then echo "FAIL: escape codes in the report"; exit 1; fi
```

Expect in `evidence/NN-clidoctor.out`: the line `Doctor · setup checkup · workspace-status v<version>`, the head `<n> ok · <n> warn · 0 fail · <n> unknown`, then the groups `TERMINAL`, `TOOLS`, `FILES`, each row as `  <status> <name>` with the why line under it. The rows that differ from the TUI:

```text
  unknown Terminal size
          stdout is not a terminal.
  unknown Mouse wheel (live test)
          TUI only: open Doctor: setup checkup in the TUI and scroll once.
  unknown Terminal (XTVERSION)
          Not asked: stdin or stdout is not a terminal.
```

and the same `Not asked` line for the other five capability rows. `evidence/NN-clidoctor.rc` holds `0` (no row is `fail`). The CLI runs in the caller's shell, not in the run's tmux pane, so the rows that read tmux (`Hyperlinks (OSC 8)`, `Clipboard`, `Mouse`) follow the caller: outside tmux they differ from the TUI rows of D1 (for example `ok      Mouse` with `On.`).

**PASS:** the `jq -e` line prints `true`, every `grep` line matches, the count is 6, and no `FAIL` line prints.
