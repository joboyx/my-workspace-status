# my-workspace-status — agent guide

Read [README.md](./README.md) first for usage, then `docs/` for internals.

Tag `ink-tui` is a historical snapshot of a previous TypeScript TUI. It is not an install path.

## Start here

Cold machine: run `scripts/agent-bootstrap.sh` (`--check` only reports). It checks git and a C linker, installs the toolchain of `rust-toolchain.toml` with clippy and rustfmt, fetches crates, and builds the test binaries.

Build: `cargo build --workspace`. The gate (CI runs the same commands on Linux):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Run one test:

| Test | Command |
| --- | --- |
| Lib unit test | `cargo test -p workspace-status --lib <name>` |
| One PTY test (name = file name under `tests/tui_tty_e2e/human/`) | `cargo test -p workspace-status --test tui_tty_e2e <pty_name>` |
| Graph crate | `cargo test -p workspace-status-graph <name>` |

Optional: with cargo-nextest installed, `cargo nextest run --profile ci` uses `.config/nextest.toml` (retries for the PTY e2e binary, slow-test timeout). CI stays on `cargo test`.

Where code goes: [docs/architecture.md](./docs/architecture.md) → **Where do I add X**.

On macOS, `tests/tag_release.rs` needs GNU `sed` (`scripts/tag-release.sh` uses `sed -i`). CI runs on Linux, where it passes.

## Definition of done

- The gate in **Start here** is green.
- Tests cover the changed behaviour: unit tests, plus a `pty_*` test for keys and screens.
- Each row of **Update the docs with the code** that the change touches is updated.
- A UI change is seen live through the verify skill driver (see **Verify skill**).
- The public-repository rule in **Conventions** holds.
- The pull request text has the test plan: commands and counts (`.github/pull_request_template.md`).

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

PTY and desktop e2e also drop a parent `WS_STATUS_PR_POLL_MS` (the verify driver pins it to `0`), so a test that leaves it unset never polls the fake forge. CI: `tty_spawn_paths_drop_parent_pr_poll_period`.

They also point `XDG_CONFIG_HOME` at a temp dir (PTY and desktop e2e write a fixed baseline config file there, see [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md)), so the operator user config file (`$XDG_CONFIG_HOME/my-workspace-status/config.json`, else `~/.config/...`) cannot change the fixture's settings. CLI integration tests that run the binary do the same. CI: `tty_spawn_paths_isolate_user_config`.

They also point `WS_STATUS_UI_STORE` at a temp file with no last-used theme and the activity sparkline off, so the operator UI-state file (`$XDG_STATE_HOME/my-workspace-status/ui-state.json`) cannot beat the fixture theme, switch the sparkline on, or set the commit-message rows, and a palette theme pick, sparkline toggle, or commit-message rule drag does not write it. The PTY harness uses a new `.e2e-state/ui-state-<n>.json` for each spawn unless the test passes `WS_STATUS_UI_STORE`; the desktop harness and the demo script remove their file before each launch. They also drop a parent `WS_STATUS_THEME`. CI: `tty_spawn_paths_isolate_ui_store`, `tty_spawn_paths_drop_old_theme_store_name`.

They also pin `themeMode` to `dark` and drop a parent `WS_STATUS_THEME_MODE`, so the terminal background cannot swap the fixture theme for its light twin (the desktop xterm is white). PTY and desktop e2e pin it in the baseline config file; the demo script exports `WS_STATUS_THEME_MODE` (`WS_STATUS_STILLS_THEME_MODE`, default `dark`). CI: `tty_spawn_paths_pin_theme_mode`.

The verify skill driver (`scripts/verify-tui.sh`, see **Verify skill**) is a TTY spawn path too. It follows every rule above, with the same guards in `release_watch.rs`.

Env table: [docs/configuration.md](./docs/configuration.md).

## `dist generate`

`.github/workflows/release.yml` is generated. After `dist generate`, restore `workflow_dispatch` and the host-job git-cliff steps. Numbered recipe: [docs/architecture.md](./docs/architecture.md) → **Distribution**. CI: `crates/workspace-status/tests/release_watch.rs`.

## Graph / stash (read this before paint changes)

Scope: graph paint code only (`crates/workspace-status-graph` and the TUI graph paint in `tui/graph_load.rs` / `tui/render.rs`). Other changes can skip this section.

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
| Shared TUI e2e seed, tree hscroll oracle, or fake forge | `crates/workspace-status/tests/common/` + `scripts/fake-forge/` + `docs/tui-tty-e2e.md` |
| Ratatui graph widget model, rows, or paint | `docs/graph.md` + `crates/workspace-status-graph` |
| Diff parsing, layout, or syntax highlighting | `docs/diff-rendering.md` |
| Any git command or operation semantics | `docs/git-operations.md` |
| Environment variables, workspace config, keybindings, themes | `docs/configuration.md` |
| Lint gate, allowed lints, or `src/testutil.rs` fixtures | this file (**Lint gate**) and root `Cargo.toml` |
| The gate, agent setup (`scripts/agent-bootstrap.sh`, `rust-toolchain.toml`, `.claude/hooks/session-start.sh`), `.config/nextest.toml`, or the pull request template | this file (**Start here**, **Definition of done**), `CONTRIBUTING.md` (**Checks**), `.github/pull_request_template.md`, and `crates/workspace-status/tests/agent_bootstrap.rs` |
| Integration test temp dirs or the shared git env | `crates/workspace-status/tests/common/fixture.rs` (test binaries mod it; no per-file copy) + `docs/tui-tty-e2e.md` |
| TUI test layers (PTY / desktop `#[ignore]`) or `WS_STATUS_UPDATE_CHECK_STORE` | this file, `docs/tui-tty-e2e.md` (harness / run / encoding / desktop-session only — a new `pty_*` or desktop test does not edit that file), `docs/configuration.md` |
| cargo-dist `dist generate` / Release git-cliff host steps | `docs/architecture.md` (**Distribution**) and `crates/workspace-status/tests/release_watch.rs` |
| Release tagging (`scripts/tag-release.sh`, `.github/workflows/tag-release.yml`) | `docs/architecture.md` (**Distribution**), `crates/workspace-status/tests/tag_release.rs`, and `crates/workspace-status/tests/release_watch.rs` |
| `test.yml` check jobs, the release gate, or the `tag-release.yml` trigger or push credential | `docs/architecture.md` (**Release gate**, including its required checks list) and `crates/workspace-status/tests/release_gate.rs` |
| Output format of the plain report | `SAMPLE_OUTPUT.md` and `crates/workspace-status/tests/snapshot_contract.rs` |
| Demo workspace seed or screenshot clips | `docs/demo.md` + `scripts/seed-demo-workspace.sh` + `scripts/capture-demo-stills.sh` |
| Verify skill, `wsctl`, its driver, or its scenario sets | `.agents/skills/verify-workspace-status/` (+ `.agents/skills/drive-workspace-status/` for the `wsctl` loop) + `scripts/verify-tui.sh` + `crates/workspace-status/tests/release_watch.rs` and `tests/wsctl.rs` (+ `docs/tui-tty-e2e.md` for the fake forge); keymap, footer, or overlay change → edit `.agents/skills/verify-workspace-status/features/*.md` |
| Desktop Xvfb / Openbox session | `scripts/with-desktop-session.sh` + `scripts/openbox.xml` + `docs/tui-tty-e2e.md` |
| Behaviour of a component on any surface listed in `docs/parity.md` | `docs/parity.md` (every surface of that component) + the test named in the row |

A change is not complete while its documentation is stale.

## Cross-surface parity

A shared TUI component (commit-message footer, file lists, line viewers, scrollbars, search, icon popovers, resize edges, horizontal pan, double-click, row marks, watch-reload flash) must behave the same and have the same features on every surface where it appears. [`docs/parity.md`](./docs/parity.md) lists each component, its surfaces (tab, depth, pane, pane looks, code anchors), and one status per feature.

- A behaviour change to a listed component checks every surface in its table and applies there by default. Each surface that does not take the change gets `n/a: <reason>` in the registry, and the pull request text says why.
- A new surface or a new feature gets its registry row or column in the same change.
- A gap found and not fixed gets a `gap: G<n>` cell and a row in **Open gaps**.
- Boxed and flat panes (`viewDefaults.background` `"terminal"` / `"paint"`) paint and hit-test with different geometry: a behaviour test for a changed feature runs each look its row names.

CI (`cargo test --workspace`, `crates/workspace-status/src/parity_registry.rs`) fails on a malformed cell, an unindexed gap, or a code anchor that is no longer a `fn` under `crates/`. `commit_footer_message_scroll_matches_on_every_registry_surface` runs one test body on every footer surface the registry marks supported, in both looks, and fails when the two surface lists differ. Follow that pattern for other components.

## Lint gate

CI job `lint` runs `cargo fmt --all --check` and
`cargo clippy --workspace --all-targets --locked -- -D warnings`. Run both
before pushing; `cargo clippy --fix` handles most of what it reports.

Two lints are allowed workspace-wide in the root `Cargo.toml`, each with the
remaining sites named in a comment: `too_many_arguments` and
`large_enum_variant`. They are tracked debt, not a standing exemption —
delete the entry once its sites are done, and do not add new ones.

New dead code is a build error under `-D warnings`. An `#[allow(dead_code)]`
needs a one-line comment that says why. The existing sites
(`grep -rn 'allow(dead_code)' crates`) are tracked debt: do not copy them.
One case is accepted: an integration test binary that uses only part of a
shared module under `tests/common/` may put the allow, with that comment,
on its `mod` line.
Test-only helpers belong behind `#[cfg(test)]` so they leave the shipped
binary but keep their coverage; in-crate git fixtures and snapshot /
app-state fixtures live in `src/testutil.rs`, not in a per-module copy.
A unit test gets its temp path from `testutil::unique_dir` (process id,
time, and a counter), never from a timestamp alone: the macOS clock ticks
in microseconds, so two names made back to back can be the same.
A unit test gets a past instant from `testutil::instant_ago`, never from
`Instant::now() - d`: on Windows the clock counts from boot, so that panics
on a fresh host. A fixture path that must be absolute comes from
`testutil::abs_path`: on Windows `/x` is relative.

Rustdoc on exported items (`///` or `//!`) is a review rule, not a lint: no
`missing_docs` lint runs.

## Conventions

- `cargo test --workspace` at the repository root covers `workspace-status` and `workspace-status-graph` (Unix PTY e2e). Desktop `#[ignore]` tests need `--ignored`. See **TUI test layers**.
- Exported Rust items need rustdoc (`///` or `//!`). Reviews check it; no lint does (see **Lint gate**).
- The plain-text report is a user-facing contract — changing it means updating `SAMPLE_OUTPUT.md` and the snapshot e2e suite.
- TTY event loop: do not run git or other blocking I/O on the draw/event thread. The live path is `tui/event_loop.rs` (current-thread Tokio, dedicated input thread, `spawn_blocking` on a `JoinSet`) through `tui/effect.rs`. Fetch / pull / push enqueue on a per-gitdir remote queue (`RemoteQueue`) under the resolved `fetchConcurrency` cap (`WS_STATUS_FETCH_CONCURRENCY` > config > 10). Exclusive writes stay serial. While an exclusive write or default-branch job is in flight, nav / pane switch / cancel / remotes stay live (`BusyAction::Handle`); keys that would start another exclusive write are drained (`Ignore`). An exclusive write on a gitdir that already has a remote dispatches, then `schedule` refuses with breadcrumb `busy`. `E` blob/temp prepare is `UserTag::DiffPrepare` on that pool; `blob_bytes` must not run on the loop thread. Guard: `tty_event_loop_must_not_call_sync_pane_git` in `tui/event_pump.rs`.
- `dist generate` rewrites `release.yml`: see **`dist generate`**. TTY demo clips and e2e must set `WS_STATUS_UPDATE_CHECK_STORE`, `WS_STATUS_UI_STORE`, and `XDG_CONFIG_HOME`, and pin `themeMode` to `dark`. Guards: `tty_spawn_paths_isolate_update_check_store`, `tty_spawn_paths_isolate_ui_store`, `tty_spawn_paths_isolate_user_config`, and `tty_spawn_paths_pin_theme_mode` in `tests/release_watch.rs`.
- This repository is public. Do not commit private workspace paths, personal hostnames, ticket keys of other projects, customer/project names from private work, chat transcripts, screenshots of private work, tokens, or credentials. Task ids of this project (`MYWS-<n>`) are published: use the id in the branch name, as the commit scope (`fix(MYWS-123): …`) and in the pull request title. Use `scripts/seed-demo-workspace.sh` for examples and demo clips.

## Verify skill

`.agents/skills/verify-workspace-status/` is the in-repo skill for proving that a change works. It maps each claim to a test layer (unit, PTY e2e, desktop `#[ignore]`, live forge `#[ignore]`) with the exact commands, and `features/` maps each feature to recipes with the expected screen text and a PASS rule (`features/README.md` is the sweep order). Its controller `scripts/wsctl` lets an agent drive the TUI as a user does from a terminal (`start`, `doctor`, `key`, `type`, `wait`, `snap`, `resize`, `run`, `stop`), with one JSON object per command. `wsctl` is a thin layer over the driver `scripts/verify-tui.sh`, which runs the `target/debug/workspace-status` of this checkout in a private tmux server on a seeded throwaway workspace. It runs offline against the fake `gh` / `glab` scenarios, or live against public repos, and saves screen captures as proof. `.agents/skills/drive-workspace-status/` is a short pointer skill to the `wsctl` loop.

The driver is a **test harness** with the isolation rules of **TUI test layers**. It is not a capture pipeline and it makes no demo clips or stills: those stay in `scripts/capture-demo-stills.sh` (**Demo / screenshots**). "Do not drive the TUI by hand" below is about demo clips. For verification, drive the TUI only through `wsctl` or the driver, never by starting the binary in a bare terminal.

Run the skill:

- After a change to the PR badge, the PR popover, the fake forge, or any TUI behaviour a user sees, before you report it done.
- To reproduce a TUI bug with evidence.
- After a rebase onto work that touches those.

Driver proof does not replace a test: behaviour that must stay gets a `pty_*` test. Keep the skill in step with the code: a new `gh` / `glab` call needs a fake forge answer and scenario sets, a changed badge needs `features/pr-status.md`, a changed PRs tab needs `features/prs-tab.md`, and a keymap, footer, or overlay change needs the `features/*.md` recipes that press or read it. `crates/workspace-status/tests/release_watch.rs` guards the driver's isolation and that `wsctl` spawns only through the driver; `tests/wsctl.rs` checks its syntax, its `--help`, and that it refuses a run that is not this checkout's before any tmux call.

## Demo / screenshots

Refresh the README/demo GIF clips with `./scripts/capture-demo-stills.sh` (ffmpeg records the terminal window; see `docs/demo.md`).
Do not invent a fixture. Do not invent a new capture pipeline. Do not drive the TUI by hand: do not hand-record demo clips.
