# my-workspace-status — agent guide

Read [README.md](./README.md) first for usage, then `docs/` for internals.

Tag `ink-tui` is a historical snapshot of a previous TypeScript TUI. It is not an install path.

## Graph / stash (read this before paint changes)

Stash gutter tips are **not** a special chrome problem. Normative rule:

> **Stash ≡ one-node side-branch tip** (same topology as a single-commit side leaf), glyph `◇` instead of `●`. Join at `stash^1`. Nothing under `◇`. Never land `◇` on a live DAG lane.

**List placement:** park each stash **immediately above** its `stash^1` (`parent_id`) — do **not** chrono-interleave by stash author-date (that creates tip↔join gaps). Orphans (parent outside the loaded window) sit after uncommitted.

Full grammar, S0–S7, code map, anti-patterns, and known gaps:
[`docs/git-graph-topology.md`](./docs/git-graph-topology.md) → **Stash rows — visual grammar**.

Do **not** invent spur heuristics (`parent.lane + 1`, spine `◇─╯`, mid-rail `├─◇`, etc.) until those pictures match. Densify stays on commit spacers only.

**Before claiming a stash paint fix is done:** dump live gutters (`load_graph_model` → `GraphModel::visible_rows` → gutter text) on a multi-stash repo (e.g. `notes`). Unit fixtures that already place tip adjacent to `stash^1` will not catch placement bugs.

## Update the docs with the code

| When you change | Update |
| --- | --- |
| Module boundaries, data flow, new top-level module | `docs/architecture.md` |
| Workspace snapshot fields or `--json` / `--plain` contract | `docs/snapshot.md` and `crates/workspace-status/tests/snapshot_contract.rs` |
| Tree nodes, row kinds, fold behaviour, keymap, session state, action registry | `docs/tui-model.md` |
| Ratatui TUI | `docs/tui-rust.md` |
| Graph gutter topology, glyphs, junctions, densify rails, stash leaf tips | `docs/git-graph-topology.md` (+ architecture / tui-model pointers) |
| Ratatui graph widget model, rows, or paint | `docs/graph.md` + `crates/workspace-status-graph` |
| Diff parsing, layout, or syntax highlighting | `docs/diff-rendering.md` |
| Any git command or operation semantics | `docs/git-operations.md` |
| Environment variables, workspace config, keybindings, themes | `docs/configuration.md` |
| Output format of the plain report | `SAMPLE_OUTPUT.md` and `crates/workspace-status/tests/snapshot_contract.rs` |
| Demo workspace seed or screenshot frames | `docs/demo.md` + `scripts/seed-demo-workspace.sh` + `scripts/capture-demo-stills.sh` |

A change is not complete while its documentation is stale.

## Conventions

- `cargo test --workspace` at the repository root covers `workspace-status` and `workspace-status-graph`.
- Exported Rust items need rustdoc (`///` or `//!`).
- The plain-text report is a user-facing contract — changing it means updating `SAMPLE_OUTPUT.md` and the snapshot e2e suite.
- TTY event loop: do not run git or other blocking I/O on the draw/event thread. Use `run_work_pumped` (and `run_capped_pumped` for independent per-repo fetch / pull / push). While a worker runs, nav / pane switch / cancel stay live (`BusyAction::Handle`); only actions that start another git write are drained. Headless e2e may stay sync. Guard: `tty_event_loop_must_not_call_sync_pane_git` in `tui/event_pump.rs`.
- This repository is public. Do not commit private workspace paths, personal hostnames, unpublished ticket keys, customer/project names from private work, chat transcripts, screenshots of private work, tokens, or credentials. Use `scripts/seed-demo-workspace.sh` for examples and stills.

## Demo / screenshots

Refresh README/demo stills with `./scripts/capture-demo-stills.sh`.
Do not invent a fixture. Do not invent a new capture pipeline. Do not drive the TUI by hand.
