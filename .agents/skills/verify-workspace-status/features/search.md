# Search

`/` opens the search prompt in the footer (`SEARCH  <query>▏   Enter arms query · Esc clears · n/N after Enter`). Matches highlight while you type. `Enter` jumps to the first match after the cursor and arms the query: the footer chip `/<query> <pos>/<count> · <pane>`. `n` / `N` go to the next / previous match and wrap (`search wrapped to top`). `Esc` clears an armed query (`search cleared`). The search prompt hides the `? help` footer.

Conventions and the sweep order: [README.md](README.md). The keys follow the demo clips (`docs/demo.md`): 02 (`/merger`), 04 and 14 (`/auth`, `n`), 12 (`/zzz`).

## SR1 `/merger` Enter jumps to the checkout (clip 02)

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
$WS doctor
trap '$WS stop' EXIT
$WS key /
$WS wait 'SEARCH'
$WS type merger
$WS wait 'SEARCH +merger'
$WS snap --name sr1-typed --raw | grep -F 'Enter arms query'
$WS key Enter
$WS wait --cursor 'v merger'
$WS wait '/merger 1/1 · tree'
$WS wait 'merge billing into main'
$WS snap --name sr1-armed --raw | grep -E '▌ +v merger'
```

Expect: while typing, the footer shows `SEARCH  merger▏   Enter arms query · Esc clears · n/N after Enter`. After `Enter` the cursor is on the `merger` checkout row, the footer chip says `/merger 1/1 · tree`, and the right pane shows the `merger` graph with `merge billing into main`.

**PASS:** every `wait` is ok and both `grep` lines match.

## SR2 `/auth` Enter, then `n` wraps (clips 04 and 14)

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS key /
$WS wait 'SEARCH'
$WS type auth
$WS key Enter
$WS wait --cursor 'auth.ts'
$WS wait '/auth 2/2 · tree'
$WS snap --name sr2-first --raw | grep -F '/auth 2/2 · tree'
$WS key n
$WS wait --cursor '& feature/auth-refresh'
$WS wait 'search wrapped to top'
$WS wait '/auth 1/2 · tree'
$WS snap --name sr2-wrapped --raw | grep -F 'search wrapped to top'
$WS key n
$WS wait --cursor 'auth.ts'
$WS wait '/auth 2/2 · tree'
```

Expect: `Enter` moves from the `session.ts` cursor to the first match after it, `auth.ts` (`/auth 2/2 · tree`). `n` wraps to the `feature/auth-refresh` row (`search wrapped to top`, `1/2`), and the next `n` returns to `auth.ts` (`2/2`).

**PASS:** every `wait` is ok and both `grep` lines match.

## SR3 No match, then `Esc` clears (clip 12)

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS key /
$WS wait 'SEARCH'
$WS type zzz
$WS key Enter
$WS wait 'no match'
$WS snap --name sr3-no-match --raw | grep -F '/zzz -/0 · tree'
$WS key Escape
$WS wait 'search cleared'
$WS wait --cursor 'session.ts'
```

Expect: `/zzz` Enter says `no match` with the chip `/zzz -/0 · tree`, and the cursor stays on `session.ts`. `Esc` says `search cleared`.

**PASS:** every `wait` is ok and the `grep` finds the chip.
