# Feature map

One file per feature. Each file lists recipes with steps, the screen text to expect, and a **PASS** rule. A full sweep runs the files in the order below, so the basic surfaces pass before the journeys that cross them.

| # | File | Proves | Driven with |
| --- | --- | --- | --- |
| 1 | [tree-nav](tree-nav.md) | Tree cursor `j` / `k`, `Enter` focuses the right pane, `Esc` comes back, folds `h` / `l` / `z` | `wsctl` |
| 2 | [diff](diff.md) | A dirty file row shows its diff: staged and unstaged hunks, inline under 100 diff columns, split at 200 terminal columns | `wsctl` |
| 3 | [search](search.md) | `/` search: `/merger` Enter (demo clip 02), `/auth` Enter `n` wraps (clips 04, 14), no match and `Esc` clears (clip 12) | `wsctl` |
| 4 | [help-overlay](help-overlay.md) | `?` opens the key overlay over the panes, `Esc` closes it | `wsctl` |
| 5 | [json-snapshot](json-snapshot.md) | `--json` and `--plain` of the fixture, run headless with the run's isolation; the refused flags | `wsctl run` |
| 6 | [tree-and-graph](tree-and-graph.md) | PR badges in the tree and the graph pane, the glyph column at wide and narrow widths, the graph chip | `scripts/verify-tui.sh` (fake forge) |
| 7 | [pr-status](pr-status.md) | PR badge states on both forges, `gx` and `gc`, a scripted transition, a failing CLI, the lookup outage, live recipe | `scripts/verify-tui.sh` (fake forge, live) |
| 8 | [popovers](popovers.md) | The PR popover and the commit popovers by mouse hover and click | `scripts/verify-tui.sh` (fake forge, mouse) |
| 9 | [prs-tab](prs-tab.md) | The PRs tab (`gp`): groups, detail, folds, actions, narrow layout, outage, empty state | `scripts/verify-tui.sh` (fake forge) |
| 10 | [multi-surface-journeys](multi-surface-journeys.md) | One user path across tree → diff → graph (`Tab`) → commit files → back | `wsctl` |

Files 6 to 9 use the driver directly: they need `scenario`, `calls` and `mouse`, which `wsctl` leaves to the driver. Their shared prelude is **Setup** in [pr-status.md](pr-status.md).

## Recipe conventions (wsctl files)

- Run from the repository root. Each block is one script: run it with `bash -e` (or paste it into one shell). `WS=.agents/skills/verify-workspace-status/scripts/wsctl`.
- Each block starts its own run with `$WS start` and stops it with an `EXIT` trap, so a failed step still ends the run. `start` refuses while another run is live.
- Put every `wsctl` call on its own line. Under `bash -e`, a failed call that is not the last one of an `a && b` list does not stop the script.
- Every move is one `key` call followed by a `wait` on the row or text it should produce. `wait --cursor` takes plain text that must be on the focus-bar line; pick text that only the target row has (the tree and the right pane share screen lines).
- A recipe proves a check with `snap --name <id> --raw | grep ...`: the text is read from the capture that is kept in `evidence/`.
- The fixture is the demo seed (`docs/demo.md` § Workspace). The first cursor is `session.ts` (`S`) under `app` → Staged → `src`. The default size is `160x45`: tree 64 columns, diff pane 96 columns (inline, `too narrow` for split).

Report every check as PASS, FAIL (quote what the screen showed) or SKIP (name the missing precondition), with the evidence file: see **Proof** in [../SKILL.md](../SKILL.md).
