# my-workspace-status — agent guide

Read [README.md](./README.md) first for usage, then `docs/` for internals.

Tag `ink-tui` is a historical snapshot of a previous TypeScript TUI. It is not an install path.

## TUI test layers

Three harnesses. Do not mix them. Detail: [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md).

| Layer | Path | What it drives | Command |
| --- | --- | --- | --- |
| Headless TestBackend | `crates/workspace-status/tests/tui_headless_e2e.rs` | In-process `HeadlessTui` on ratatui `TestBackend`. No TTY. No binary spawn. Does not run the GitHub Release startup check. | `cargo test --workspace` |
| Real-input PTY | `crates/workspace-status/tests/tui_tty_e2e/` (tests without `#[ignore]`) | Spawns the `workspace-status` binary on a PTY. Live `event::read`. Unix only. Windows compiles this crate with no tests. | `cargo test --workspace` |
| Desktop `#[ignore]` | same crate, `#[ignore]` tests (xfce keys, xterm XTEST wheel) | Real terminal emulator under `DISPLAY`. GitHub Actions job `tui-tty-desktop` (`xvfb-run`). | `cargo test --test tui_tty_e2e -- --ignored` |

`cargo test --workspace` is the default suite. It covers both crates, the snapshot contract, CLI, discovery, headless TestBackend, and Unix PTY e2e. It does not run `#[ignore]` tests.

Desktop local run (Linux, `DISPLAY`, packages in [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md)):

```bash
cargo test --test tui_tty_e2e -- --ignored --nocapture --test-threads=1
```

Screenshot stills stay in `scripts/capture-demo-stills.sh`. That script is not a TUI e2e harness.

### `WS_STATUS_UPDATE_CHECK_STORE`

A TTY `ws` / `workspace-status` launch may ask `new version available, update? [y/n]` before the TUI mounts. `--plain`, `--json`, and `--update` skip that check. The check runs at most every 6 hours. Last-check time lives in `$XDG_STATE_HOME/my-workspace-status/update-check.json`. `WS_STATUS_UPDATE_CHECK_STORE` overrides that path.

`HeadlessTui` does not run the check.

PTY e2e, desktop e2e, and `scripts/capture-demo-stills.sh` spawn a real TTY binary. Point `WS_STATUS_UPDATE_CHECK_STORE` at a temp file with a fresh `lastCheckUnix`. That keeps the prompt from blocking mount and avoids writing the operator XDG file. Tests that drive `--plain` / `--json` / `--update` should also point the store at a temp path. Those modes must not create the file.

CI fails if a TTY spawn path drops that assignment: `crates/workspace-status/tests/release_watch.rs`.

Env table: [docs/configuration.md](./docs/configuration.md).

## `dist generate`

`.github/workflows/release.yml` is generated. After `dist generate`, restore `workflow_dispatch` and the host-job git-cliff steps. Numbered recipe: [docs/architecture.md](./docs/architecture.md) → **Distribution**. CI: `crates/workspace-status/tests/release_watch.rs`.

## Graph / stash (read this before paint changes)

Stash gutter tips are **not** a special chrome problem. Normative rule:

> **Stash ≡ one-node side-branch tip** (same topology as a single-commit side leaf), glyph `◇` instead of `●`. Join at `stash^1`. Nothing under `◇`. Never land `◇` on a live DAG lane.

**List placement:** park each stash **immediately above** its `stash^1` (`parent_id`) — do **not** chrono-interleave by stash author-date (that creates tip↔join gaps). Orphans (parent outside the loaded window) sit after uncommitted.

Full grammar, S0–S7, code map, anti-patterns, and known gaps:
[`docs/git-graph-topology.md`](./docs/git-graph-topology.md) → **Stash rows — visual grammar**.

Do **not** invent spur heuristics (`parent.lane + 1`, spine `◇─╯`, mid-rail `├─◇`, etc.) until those pictures match. Densify stays on commit spacers only.

**Before claiming a stash paint fix is done:** dump live gutters (`load_graph_model` → `GraphModel::visible_rows` → gutter text) on a multi-stash repo. The demo seed (`scripts/seed-demo-workspace.sh`) includes `merger` with a stash. Unit fixtures that already place tip adjacent to `stash^1` will not catch placement bugs.

## Update the docs with the code

| When you change | Update |
| --- | --- |
| Module boundaries, data flow, new top-level module | `docs/architecture.md` |
| Workspace snapshot fields or `--json` / `--plain` contract | `docs/snapshot.md` and `crates/workspace-status/tests/snapshot_contract.rs` |
| Tree nodes, row kinds, fold behaviour, keymap, session state, action registry | `docs/tui-model.md` |
| Ratatui TUI | `docs/tui-rust.md` |
| Graph gutter topology, glyphs, junctions, densify rails, stash leaf tips | `docs/git-graph-topology.md` (+ architecture / tui-model pointers) |
| Shared TUI e2e seed or tree hscroll oracle | `crates/workspace-status/tests/common/` + `docs/tui-tty-e2e.md` |
| Ratatui graph widget model, rows, or paint | `docs/graph.md` + `crates/workspace-status-graph` |
| Diff parsing, layout, or syntax highlighting | `docs/diff-rendering.md` |
| Any git command or operation semantics | `docs/git-operations.md` |
| Environment variables, workspace config, keybindings, themes | `docs/configuration.md` |
| TUI test layers (headless / PTY / desktop `#[ignore]`) or `WS_STATUS_UPDATE_CHECK_STORE` | this file, `docs/tui-tty-e2e.md`, `docs/configuration.md` |
| cargo-dist `dist generate` / Release git-cliff host steps | `docs/architecture.md` (**Distribution**) and `crates/workspace-status/tests/release_watch.rs` |
| Output format of the plain report | `SAMPLE_OUTPUT.md` and `crates/workspace-status/tests/snapshot_contract.rs` |
| Demo workspace seed or screenshot frames | `docs/demo.md` + `scripts/seed-demo-workspace.sh` + `scripts/capture-demo-stills.sh` |
| Desktop Xvfb / Openbox session | `scripts/with-desktop-session.sh` + `scripts/openbox.xml` + `docs/tui-tty-e2e.md` |

A change is not complete while its documentation is stale.

## Conventions

- `cargo test --workspace` at the repository root covers `workspace-status` and `workspace-status-graph` (headless TestBackend + Unix PTY e2e). Desktop `#[ignore]` tests need `--ignored`. See **TUI test layers**.
- Exported Rust items need rustdoc (`///` or `//!`).
- The plain-text report is a user-facing contract — changing it means updating `SAMPLE_OUTPUT.md` and the snapshot e2e suite.
- TTY event loop: do not run git or other blocking I/O on the draw/event thread. The live path is `tui/event_loop.rs` (current-thread Tokio, dedicated input thread, `spawn_blocking` on a `JoinSet`) through `tui/effect.rs`. While exclusive writes or a remote batch run, nav / pane switch / cancel stay live (`BusyAction::Handle`); only actions that start another git write are drained. Headless e2e pumps the same interpreter on the test thread (`Interpreter::interpret_sync`) and does not run `Effect::EditFile`. Guard: `tty_event_loop_must_not_call_sync_pane_git` in `tui/event_pump.rs`.
- After `dist generate`, restore `workflow_dispatch` and host git-cliff on `release.yml`. TTY stills and e2e must set `WS_STATUS_UPDATE_CHECK_STORE`. Guard: `tty_spawn_paths_isolate_update_check_store` in `tests/release_watch.rs`.
- This repository is public. Do not commit private workspace paths, personal hostnames, unpublished ticket keys, customer/project names from private work, chat transcripts, screenshots of private work, tokens, or credentials. Use `scripts/seed-demo-workspace.sh` for examples and stills.

## Demo / screenshots

Refresh README/demo stills with `./scripts/capture-demo-stills.sh`.
Do not invent a fixture. Do not invent a new capture pipeline. Do not drive the TUI by hand.
