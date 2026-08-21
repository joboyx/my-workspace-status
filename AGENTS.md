# my-workspace-status — agent guide

Read [README.md](./README.md) first for usage, then `docs/` for internals.

## Graph / stash (read this before paint changes)

Stash gutter tips are **not** a special chrome problem. Normative rule:

> **Stash ≡ one-node side-branch tip** (same topology as a single-commit side leaf), glyph `◇` instead of `●`. Join at `stash^1`. Nothing under `◇`. Never land `◇` on a live DAG lane.

**List placement:** park each stash **immediately above** its `stash^1` (`parentId`) — do **not** chrono-interleave by stash author-date (that creates tip↔join gaps). Orphans (parent outside the loaded window) sit after uncommitted.

Full grammar, S0–S7, code map, anti-patterns, and known gaps:
[`docs/git-graph-topology.md`](./docs/git-graph-topology.md) → **Stash rows — visual grammar**.

Do **not** invent spur heuristics (`parent.lane + 1`, spine `◇─╯`, mid-rail `├─◇`, etc.) until those pictures match. Densify stays on commit spacers only.

**Before claiming a stash paint fix is done:** dump live gutters (`loadGraphModel` → `buildGraphListRows` → gutter text) on a multi-stash repo (e.g. `notes`). Unit fixtures that already place tip adjacent to `stash^1` will not catch placement bugs.

## Update the docs with the code

| When you change | Update |
| --- | --- |
| Module boundaries, data flow, new top-level module | `docs/architecture.md` |
| Tree nodes, row kinds, fold behaviour, keymap, session state, action registry | `docs/tui-model.md` |
| Graph gutter topology, glyphs, junctions, densify rails, stash leaf tips | `docs/git-graph-topology.md` (+ architecture / tui-model pointers) |
| Diff parsing, layout, or syntax highlighting | `docs/diff-rendering.md` |
| Any git command or operation semantics | `docs/git-operations.md` |
| Environment variables, workspace config, keybindings, themes | `docs/configuration.md` |
| Output format of the plain report | `SAMPLE_OUTPUT.md` and `test/workspace-status.e2e.ts` |

A change is not complete while its documentation is stale.

## Conventions

- ESM: import with `.js` extensions from TypeScript sources.
- Tests use `node:test` + `tsx`. New test files must be listed in the `test` script in `package.json`.
- Exported symbols need multi-line JSDoc.
- The plain-text report is a user-facing contract — changing it means updating `SAMPLE_OUTPUT.md` and the E2E suite.
