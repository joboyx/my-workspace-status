# Help overlay

`?` opens the key overlay over the tree and the right pane: three columns `MOVE`, `GIT`, `VIEW`, an `ICONS` legend, and a footer line `j/k scroll · ... · / search help · Esc closes` with the version on the right. The overlay hides the `? help` footer of the app. `Esc` closes it and the panes come back unchanged (demo clip 08).

Conventions and the sweep order: [README.md](README.md).

## HO1 `?` opens the overlay, `Esc` closes it

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS doctor
$WS wait '\? help'
$WS snap --name ho1-before --raw | grep -E '▌ +· session\.ts'
$WS key '?'
$WS wait 'Esc closes'
$WS wait --gone '\? help'
$WS wait-ready
F=$($WS snap --name ho1-open | jq -r .txt)
grep -E 'MOVE .* GIT .* VIEW' "$F"
grep -E 'Esc closes +v[0-9]+\.[0-9]+\.[0-9]+' "$F"
$WS key Escape
$WS wait --gone 'Esc closes'
$WS wait '\? help'
$WS snap --name ho1-closed --raw | grep -E '▌ +· session\.ts'
```

Expect on `ho1-open`: the boxed overlay with the `MOVE`, `GIT` and `VIEW` headings on one line, the footer `... Esc closes` with `v<version>` at the right, and no `? help` in the app footer. `wait-ready` passes with the overlay open: the tab strip ` Workspace` on row 1 stays above it. On `ho1-closed` the footer `? help` and the cursor on `session.ts` are back.

**PASS:** every `wait` is ok and all four `grep` lines match their captures.
