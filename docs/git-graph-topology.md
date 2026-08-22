# Git graph topology

Gutter painting for the workspace-status TUI (`src/tui/graph/`).

Lane assignment, parent planning, densify-left, fixed gutter width, viewport,
and navigation are unchanged aside from spacer skipping (below). This doc
covers **how cells are painted**.

**Read order for stash work:** lock the [stash visual grammar](#stash-rows--visual-grammar)
(pictures / isomorphism) before changing densify, spur-lane math, or
`stashLeafRailCells`. Wrong paint almost always comes from skipping that step.

## Connection model

Paint builds an internal directional model per cell (`up` / `down` / `left` /
`right`) before choosing a glyph. Public `GraphCell` stays `{ ch, colorLane,
role }`.

Colour ownership:

| Content | `colorLane` |
| --- | --- |
| Node | commit lane |
| Horizontal parent edge / open / join | target (parent) lane |
| Through-rail crossed by a horizontal | the through-rail’s own lane |
| Stash leaf tip / its stub·bridge | `stash^1`’s lane colour (not a fake spur-lane palette) |

## Node glyphs

| Role | Unicode | ASCII |
| --- | --- | --- |
| Commit and merge | `●` | `*` |
| HEAD (incl. merge-at-HEAD) | `⊙` | `@` |
| Uncommitted | `○` | `o` |
| Stash side leaf tip | `◇` | `s` |

Merges are identified by topology (extra elbows / tees), not a distinct node
glyph.

## Connection → glyph

| Connections | Unicode | ASCII |
| --- | --- | --- |
| left + up + down | `┤` | `+` |
| right + up + down | `├` | `+` |
| left + right + down | `┬` | `+` |
| left + right + up | `┴` | `+` |
| all four | `┼` | `+` |
| left + down | `╮` | `\` |
| right + down | `╭` | `/` |
| left + up | `╯` | `/` |
| right + up | `╰` | `\` |
| up + down | `│` | `\|` |
| left + right | `─` | `-` |

ASCII uses the **same** topology; only the glyph map changes
(`WS_STATUS_GLYPHS=ascii` or `layoutCommits(commits, { ascii: true })`).
Note: ASCII `/` and `\` each map two Unicode corners (lossy if inferred from
glyphs alone — densify rails therefore use topology stem metadata, not glyphs).

## Leaf vs join (commits)

A **leaf** is a tip: a node with no children in the DAG.

A **join / close** (`●─╯`, `╰─`, …) is often painted on an older commit where a
side lane dies into the spine. That commit is **not** “the leaf” of the side
branch — the leaf was the tip on that side lane.

Example — one-node side tip (commit grammar):

```text
●        LEAF — tip A (e.g. main)
│
│ ●      LEAF — tip B (1 node only; nothing under it)
│ │
●─╯      join / diverge point (NOT tip B)
│
●
```

## Spacer rows

Every **commit** and **stash** is followed by a non-selectable `spacer`
(second line of the block).

- **Commit row:** node + subject (full flex).
- **Commit spacer:** stem rails + that commit’s branch / tag chips (left) +
  right-anchored hash / date / author (layout A). Checkout on a named branch
  prefixes nf-fa-crosshairs (``, ASCII `+`) on that chip — no separate
  `[HEAD]`. Detached HEAD keeps a bold `[HEAD]` chip. Synced local+remote
  pairs put nf-fa-exchange (``, ASCII `=`) **before** the branch name.
  Misc unicode (`⌖` / `⇄`) is avoided — those glyphs bleed in MesloLGM.
- **Stash row / spacer:** see [stash visual grammar](#stash-rows--visual-grammar).

Between consecutive commits the spacer gutter densifies
(`prev.stemDown` → `next.stemUp`), including when a parked stash leaf sits
between those commits (stash is always immediately above `stash^1`). Stash
rows **never** own densify.

### Navigation

Cursor / search / page / EasyMotion land only on selectable rows
(`uncommitted` | `stash` | `commit`). Spacers remain in the painted list for
display and viewport scrolling, but `j`/`k` and match stepping skip them.
Focusing a commit or stash highlights the full 2-row pair (selectable + its
spacer); the cursor bar stays on the selectable row.

## Stash rows — visual grammar

### Isomorphism (normative)

**A stash is the same topology as a one-node side-branch tip**, with glyph
`◇` instead of `●`.

Take the commit “1-node side tip” schematic above and swap tip B’s `●` → `◇`.
That is the stash leaf. Do not invent a separate “stash chrome” metaphor
(spine diamond, mid-rail bead, orphan floating `◇` without the leaf grammar).

```text
●        LEAF — tip A (e.g. main)
│
│ ◇      LEAF — stash (1 node only)
│ │      short spur on spacer toward join (no second node)
●─╯      stash^1 (join)
│
●
```

### Hard rules

1. Stash is **always a leaf** — never the parent of any commit or stash.
2. Attachment **identity** is always `stash^1` (`parentId` = first `%P` parent /
   HEAD at stash time). List **placement** parks each stash **immediately above**
   that parent (1 node away) — stash author-date does not interleave it into the
   commit chrono stream. Out-of-window `stash^1` is fetched into the model at
   load so it can park. True orphans (parent object missing) sit after
   uncommitted, newest-first. Extra parents are appended after the log-window
   prefix (`windowCount`); autoload skip uses that prefix, not `commits.length`.
   Layout inserts extras into the git window by author date without reordering
   window rows; extra `%P` ids not in the layout set are dropped (no ghost waiter).
3. The tip is **one node**: no second node under `◇`, and no **dangling** side rail.
   The stash spacer carries a **short** spur toward the join when tip-above-parent
   (always true for parked stashes; orphans have no join).
4. Join lands on `stash^1` (elbow on that commit’s row), same family as commit
   side-tip joins — not a `◇─╯` stealing the spine node.
5. `◇` must **never** sit on a live commit-DAG lane column (no mid-rail “parent bead”).
6. Subject text has **no** leading diamond — diamond is gutter-only.
7. Densify stays on **commit** spacers only; stash rows do not bridge the DAG.

### Placement vs topology

Commits stay newest-first among themselves. Stashes follow **parent attachment**,
not stash author-date interleave — so tip and join stay adjacent (see S2). That
does **not** authorize painting `◇` onto an already-live merge lane.

### Representative stash configurations

Newest at top. `◇` = stash leaf tip.

| ID | Name | Schematic (gutter only) |
| --- | --- | --- |
| **S0** | Canonical | `●` / `│ ◇` / `│ │` / `●─╯` (`stash^1`) / `│` / `●` |
| **S1** | Stash is the only tip | `│ ◇` / `│ │` / `●─╯` / `│` / `●` |
| **S2** | Mid-spine (commits above only) | `⊙` / `●`… / `│ ◇` / `│ │` / `●─╯` — **no** spine commit between `◇` and join while a side `│` continues |
| **S3** | Above unrelated merge ancestry | S0, then later `●─╮` … `●─╯` further down the spine |
| **S4** | Inside live merge rails | Stash on its **own** leaf lane; never steals the live side mid-rail |
| **S5** | Parent on a side lane | `stash^1` is a side-lane commit; stash is still a 1-node leaf off that parent |
| **S6** | Two stash leaves | Two independent `◇` tips; each joins on its own `stash^1`; neither parents the other |
| **S6b** | Same-parent siblings | Distinct free-lane `◇` stacked above one `stash^1`; newer spur continues through the older tip; join `●─┴─╯` |
| **S7** | `stash^1` is HEAD tip | `│ ◇` / `│ │` / `⊙─╯` (join on the tip row) |

### Anti-patterns (rejected)

| Bad | Why |
| --- | --- |
| Blind `parent.lane + 1` spur into a live lane | Lands `◇` on a live merge mid-rail → parent-bead reading |
| Inventing parent-lane `│` before that lane is open | Phantom stem above where the parent lane exists |
| `◇─╯` / diamond **on** the spine / parent node | Steals the join commit; not a side tip |
| Dangling side `│` under `◇` (tip not adjacent to join) | Looks like a multi-node branch / “gap below stash” |
| Stash row dropping sibling live rails | Wrong rails look terminated (blink) |
| Stash owning commit↔commit densify | Stash must not bridge the DAG |
| Mid-rail `├─◇` tee as the tip metaphor | Encodes continuation through the tip; rejected 3b grammar |

### Paint intent (implementation must match)

Target behaviour for list + `stashLeaf*` wiring (docs are normative; if code
disagrees, fix the code):

1. **Placement** (`buildGraphListRows`): commits newest-first; each stash tip +
   spacer inserted **immediately above** its `parentId` commit. Multiple stashes
   on the same parent stack newest-first above that parent, each on its **own**
   free leaf lane. `loadGraphModel` fetches missing `stash^1` commits into the
   model (does not change `skip`/`limit`/`hasMore`) so park-on-parent can join.
   True orphans (parent object missing) still sit after uncommitted, newest-first.
   Extra parents insert into the layout walk by date; the git window order is
   unchanged. Extra `%P` targets not in the layout set are dropped.
2. **Base layer** on stash row + stash spacer: through-verticals for rails that
   are genuinely live in that gap (post-densify: prefer `next.stemUp`, else
   `prev.stemDown` at the window tail).
3. **Leaf overlay** on the stash selectable row: 1-node tip `◇` on a **free**
   leaf lane (allocator — never steal a live DAG lane / parent lane). Live
   through-rails stay; **no** mid-rail `├─◇` tee; **no** `down` on the tip column.
4. **Spacer:** live through-rails plus a **short spur** toward the join when
   tip-above-parent (parked stashes always tip-above-parent). Orphans: no spur /
   no join.
5. **Join:** close elbow on the `stash^1` commit row (`●─╯` / `⊙─╯` family) when
   tip-above-parent; parent outside window → lone free-lane `◇` (no fake spine tee).
6. Subject: no leading `◇`.

`stashRailCells` remains **commit↔commit densify** on commit spacers (historical
name; not stash leaf paint).

### Code map (where to change what)

| Concern | Primary files |
| --- | --- |
| List order / park-on-parent / join overlay wiring | `src/tui/graph/list.ts` (`buildGraphListRows`) |
| Free leaf lane, tip `◇`, spacer spur, live through-rails | `src/tui/graph/rows.ts` (`buildStashRailContext`, `graphStashSegments`, `graphStashSpacerSegments`, join helpers) |
| Commit DAG layout + densify | `src/tui/graph/layout.ts` (+ densify helpers used from rows) |
| Stash model (`parentId` = `stash^1`); fetch missing parents | `src/git.ts` (`listStashes`, `gitLogCommitsByIds`) / `src/tui/graph/load.ts` |
| Gutter clip window (node + stash join cols) | `src/tui/graph/gutterBudget.ts` (`sliceCellsAroundLane`) |
| Regression tests (S0–S7 + park order + gutter dump) | `test/tui-graph-list.test.ts` (also rows / load / git-graph tests when changing cell paint or window load) |
| Rust ratatui gutter (same lane model) | `crates/workspace-status-graph` (`layout.rs`, `topology.rs`, `stash.rs`, `paint.rs`) |

Canonical skill tree: `ai/common/skills/my-workspace-status/` only.

### Verify before shipping stash paint

1. `npm test` in the skill directory.
2. **Live gutter dump** on a multi-stash repo (e.g. workspace `notes`):
   `loadGraphModel` → `layoutCommits` → `buildGraphListRows` → print gutter
   slices around each `stash@{n}`. Confirm tip row is immediately above
   `stash^1` and the spacer carries a short spur into a close elbow — no gap
   rows between `◇` and join while a side `│` continues.
3. Unit fixtures that already place tip adjacent to parent will **not** catch
   chrono-placement regressions; keep an explicit park / distant-parent test.

### Known gaps / future work (operator still sees failures)

Park-on-parent + 3b→◇ leaf paint landed (PR #54 era). Residual issues — do
**not** “fix” by reintroducing chrono interleave or `parent.lane + 1`:

| Gap | Notes |
| --- | --- |
| Orphan pile | **Addressed:** `loadGraphModel` lazy-loads missing `stash^1` via `gitLogCommitsByIds` so those tips park and join. Extra parents insert into the layout walk by date without reshuffling the git window; `%P` targets not in the layout set are dropped so they cannot plant a waiter through the window. Remaining true orphans are parent objects git cannot resolve — still a lone `◇` after uncommitted (no fake spine tee). |
| Crowded / multi-lane gutters | **Improved:** sibling tips reserve distinct leaf lanes; clip window prefers covering node + join/leaf columns; join overlay keeps through-rails (`┼`) instead of eating them. Extreme density (more live lanes than the gutter cap) can still clip. |
| Operator-reported edge cases | Capture failing screenshots + short gutter dump next to S0–S7 before changing paint; extend the config table rather than one-off glyph hacks. |
| Agent skill copies | Edit **common** only; redeploy/symlink per `ai/AGENTS.md` — do not fork paint logic under `agents/*/skills/`. |

## Non-goals

- Pane-edge ceiling/floor rails
- Extra navigation rows (spacers are display-only; cursor skips them)
- Crossing minimization
- Full lazygit pipe-set rewrite
- Chrono-interleaving stashes by author-date (rejected — creates tip↔join gaps)
- Continuous spur stem across intervening **commit** rows (forbidden — park so tip and join stay adjacent; see S2)

## Representative scenarios (commits + stash)

| Case | Expected |
| --- | --- |
| Linear | single `●` column; spacer `│` between consecutive commits |
| New secondary open | corner (`╮` / `╭`), not a through-tee |
| Horizontal crosses a live rail | `┼` (or `+`) |
| Link into an already-live secondary parent | `├` / `┤` (or `+`) |
| Join close | `╯` / `╰` / `┴` on the **join** commit — not “the leaf” |
| One-node side tip (commit) | side `●` with nothing under it; join on older row |
| HEAD | node becomes `⊙` / `@`; checkout chip gets nf-fa-crosshairs before the name (or standalone `[HEAD]` when detached) |
| Stash side leaf | same as one-node side tip with `◇`; see S0–S7 |
| Adjacent commits after densify remap (no stash) | densify elbows on the **spacer** between them; full overlay then `sliceCellsAroundLane`; layout stem cols unchanged |

Schematic mockups are visual guidance — **DAG connectivity wins** when a
drawing conflicts with the commit graph. For stash tips, the isomorphism to a
one-node side-branch tip wins over spur heuristics that violate it.
