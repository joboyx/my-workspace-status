# my-workspace-status — agent guide

Read [README.md](./README.md) first for usage, then `docs/` for internals.

Tag `ink-tui` is a historical snapshot of a previous TypeScript TUI. It is not an install path.

## TUI test layers

Two harnesses. Do not mix them. Detail: [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md).

| Layer | Path | What it drives | Command |
| --- | --- | --- | --- |
| Real-input PTY | `crates/workspace-status/tests/tui_tty_e2e/` (tests without `#[ignore]`) | Spawns the `workspace-status` binary on a PTY. Live `event::read`. Unix only. Windows compiles this crate with no tests. | `cargo test --workspace` |
| Desktop `#[ignore]` | same crate, `#[ignore]` tests (xfce keys, xterm XTEST wheel) | Real terminal emulator under `DISPLAY`. GitHub Actions job `tui-tty-desktop` (`xvfb-run`). | `cargo test --test tui_tty_e2e -- --ignored` |

`cargo test --workspace` is the default suite. It covers both crates, the snapshot contract, CLI, discovery, and Unix PTY e2e. It does not run `#[ignore]` tests.

Desktop local run (Linux, `DISPLAY`, packages in [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md)):

```bash
cargo test --test tui_tty_e2e -- --ignored --nocapture --test-threads=1
```

Demo GIF clips stay in `scripts/capture-demo-stills.sh`. That script is not a TUI e2e harness.

Harness, how-to-run, encoding, and desktop-session notes live in [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md). Update that file when those change. Adding a `pty_*` or desktop test does not.

### `WS_STATUS_UPDATE_CHECK_STORE`

A TTY `ws` / `workspace-status` launch may ask `workspace-status <current> → <latest> available. Update? [y/N]` before the TUI mounts. `--plain`, `--json`, `--update`, and `WS_STATUS_UPDATE_CHECK=0` skip that check. The check runs at most every 6 hours. Last-check time lives in `$XDG_STATE_HOME/my-workspace-status/update-check.json`. `WS_STATUS_UPDATE_CHECK_STORE` overrides that path.

PTY e2e, desktop e2e, and `scripts/capture-demo-stills.sh` spawn a real TTY binary. Point `WS_STATUS_UPDATE_CHECK_STORE` at a temp file with a fresh `lastCheckUnix`. That keeps the prompt from blocking mount and avoids writing the operator XDG file. Tests that drive `--plain` / `--json` / `--update` should also point the store at a temp path. Those modes must not create the file.

CI fails if a TTY spawn path drops that assignment: `crates/workspace-status/tests/release_watch.rs`.

PTY e2e, desktop e2e, and `scripts/capture-demo-stills.sh` also unset `WS_STATUS_WORKSPACE` so a parent pin cannot retarget the fixture. CI: `tty_spawn_paths_isolate_workspace_env`.

They also point `XDG_CONFIG_HOME` at a temp dir (PTY and desktop e2e write a fixed baseline config file there, see [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md)), so the operator user config file (`$XDG_CONFIG_HOME/my-workspace-status/config.json`, else `~/.config/...`) cannot change the fixture's settings. CLI integration tests that run the binary do the same. CI: `tty_spawn_paths_isolate_user_config`.

They also point `WS_STATUS_UI_STORE` at a temp file with no last-used theme and the activity sparkline off, so the operator UI-state file (`$XDG_STATE_HOME/my-workspace-status/ui-state.json`) cannot beat the fixture theme, switch the sparkline on, or set the commit-message rows, and a palette theme pick, sparkline toggle, or commit-message rule drag does not write it. The PTY harness uses a new `.e2e-state/ui-state-<n>.json` for each spawn unless the test passes `WS_STATUS_UI_STORE`; the desktop harness and the demo script remove their file before each launch. They also drop a parent `WS_STATUS_THEME`. CI: `tty_spawn_paths_isolate_ui_store`, `tty_spawn_paths_drop_old_theme_store_name`.

They also pin `themeMode` to `dark` and drop a parent `WS_STATUS_THEME_MODE`, so the terminal background cannot swap the fixture theme for its light twin (the desktop xterm is white). PTY and desktop e2e pin it in the baseline config file; the demo script exports `WS_STATUS_THEME_MODE` (`WS_STATUS_STILLS_THEME_MODE`, default `dark`). CI: `tty_spawn_paths_pin_theme_mode`.

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
| Lint gate, allowed lints, or `src/testutil.rs` fixtures | this file (**Lint gate**) and root `Cargo.toml` |
| TUI test layers (PTY / desktop `#[ignore]`) or `WS_STATUS_UPDATE_CHECK_STORE` | this file, `docs/tui-tty-e2e.md` (harness / run / encoding / desktop-session only — a new `pty_*` or desktop test does not edit that file), `docs/configuration.md` |
| cargo-dist `dist generate` / Release git-cliff host steps | `docs/architecture.md` (**Distribution**) and `crates/workspace-status/tests/release_watch.rs` |
| Output format of the plain report | `SAMPLE_OUTPUT.md` and `crates/workspace-status/tests/snapshot_contract.rs` |
| Demo workspace seed or screenshot clips | `docs/demo.md` + `scripts/seed-demo-workspace.sh` + `scripts/capture-demo-stills.sh` |
| Desktop Xvfb / Openbox session | `scripts/with-desktop-session.sh` + `scripts/openbox.xml` + `docs/tui-tty-e2e.md` |
| Behaviour of a component on any surface listed in `docs/parity.md` | `docs/parity.md` (every surface of that component) + the test named in the row |

A change is not complete while its documentation is stale.

## Cross-surface parity

A shared TUI component (commit-message footer, file lists, line viewers, scrollbars, search, icon popovers, resize edges, horizontal pan, double-click, row marks, watch-reload flash) must behave the same and have the same features on every surface where it appears. [`docs/parity.md`](./docs/parity.md) lists each component, its surfaces (tab, depth, pane, pane looks, code anchors), and one status per feature.

- A behaviour change to a listed component checks every surface in its table and applies there by default. Each surface that does not take the change gets `n/a: <reason>` in the registry, and the pull request text says why.
- A new surface or a new feature gets its registry row or column in the same change.
- A gap found and not fixed gets a `gap: G<n>` cell and a row in **Open gaps**.
- Boxed and flat panes (`viewDefaults.background` `"terminal"` / `"paint"`) paint and hit-test with different geometry: a behaviour test covers each look its row names.

CI (`cargo test --workspace`, `crates/workspace-status/src/parity_registry.rs`) fails on a malformed cell, an unindexed gap, or a code anchor that is no longer a `fn` under `crates/`. `commit_footer_message_scroll_matches_on_every_registry_surface` runs one test body on every footer surface the registry marks supported, in both looks, and fails when the two surface lists differ. Follow that pattern for other components.

## Lint gate

CI job `lint` runs `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets --locked -- -D warnings`. Run both
before pushing; `cargo clippy --fix` handles most of what it reports.

Two lints are allowed workspace-wide in the root `Cargo.toml`, each with the
remaining sites named in a comment: `too_many_arguments` and
`large_enum_variant`. They are tracked debt, not a standing exemption —
delete the entry once its sites are done, and do not add new ones.

Dead code is a build error under `-D warnings`. Test-only helpers belong
behind `#[cfg(test)]` so they leave the shipped binary but keep their
coverage; in-crate git fixtures live in `src/testutil.rs`, not in a
per-module copy.

## Conventions

- `cargo test --workspace` at the repository root covers `workspace-status` and `workspace-status-graph` (Unix PTY e2e). Desktop `#[ignore]` tests need `--ignored`. See **TUI test layers**.
- Exported Rust items need rustdoc (`///` or `//!`).
- The plain-text report is a user-facing contract — changing it means updating `SAMPLE_OUTPUT.md` and the snapshot e2e suite.
- TTY event loop: do not run git or other blocking I/O on the draw/event thread. The live path is `tui/event_loop.rs` (current-thread Tokio, dedicated input thread, `spawn_blocking` on a `JoinSet`) through `tui/effect.rs`. Fetch / pull / push enqueue on a per-gitdir remote queue (`RemoteQueue`) under the resolved `fetchConcurrency` cap (`WS_STATUS_FETCH_CONCURRENCY` > config > 10). Exclusive writes stay serial. While an exclusive write or default-branch job is in flight, nav / pane switch / cancel / remotes stay live (`BusyAction::Handle`); keys that would start another exclusive write are drained (`Ignore`). An exclusive write on a gitdir that already has a remote dispatches, then `schedule` refuses with breadcrumb `busy`. `E` blob/temp prepare is `UserTag::DiffPrepare` on that pool; `blob_bytes` must not run on the loop thread. Guard: `tty_event_loop_must_not_call_sync_pane_git` in `tui/event_pump.rs`.
- After `dist generate`, restore `workflow_dispatch` and host git-cliff on `release.yml`. TTY demo clips and e2e must set `WS_STATUS_UPDATE_CHECK_STORE`, `WS_STATUS_UI_STORE`, and `XDG_CONFIG_HOME`, and pin `themeMode` to `dark`. Guards: `tty_spawn_paths_isolate_update_check_store`, `tty_spawn_paths_isolate_ui_store`, `tty_spawn_paths_isolate_user_config`, and `tty_spawn_paths_pin_theme_mode` in `tests/release_watch.rs`.
- This repository is public. Do not commit private workspace paths, personal hostnames, unpublished ticket keys, customer/project names from private work, chat transcripts, screenshots of private work, tokens, or credentials. Use `scripts/seed-demo-workspace.sh` for examples and demo clips.

## Demo / screenshots

Refresh the README/demo GIF clips with `./scripts/capture-demo-stills.sh` (ffmpeg records the terminal window; see `docs/demo.md`).
Do not invent a fixture. Do not invent a new capture pipeline. Do not drive the TUI by hand.
