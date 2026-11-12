---
name: verify-workspace-status
description: Prove a workspace-status change works. Picks the test layer that proves a claim, then drives the real TUI of this checkout in an isolated tmux run with the wsctl controller (JSON output), offline with fake gh and glab answers or live against public repos, and saves screen captures as proof. Use when verifying a TUI, PR badge, PRs tab or forge change, reproducing a bug with evidence, or before you report a UI change done.
---

# Verify workspace-status

`workspace-status` (`ws`) is a ratatui TUI over every git checkout under a workspace root. This skill does three things:

1. It tells you which test layer proves which claim, with the exact commands.
2. It gives a controller, [`scripts/wsctl`](scripts/wsctl), that an agent uses as a user uses a terminal: start an isolated run of the binary **of this checkout**, send keys, wait for text, read the screen, stop. Every command prints one JSON object.
3. It maps each feature to recipes with the screen text to expect and a PASS rule: [features/README.md](features/README.md).

`wsctl` is a thin layer over the driver `scripts/verify-tui.sh`, which runs the binary in a private tmux server on a seeded throwaway workspace. Every spawn goes through the driver, so its isolation covers `wsctl` too; `crates/workspace-status/tests/release_watch.rs` guards both. The driver is a **test harness**, like the PTY e2e suite, not a capture pipeline: README and demo clips come only from `scripts/capture-demo-stills.sh` (`AGENTS.md`, **Demo / screenshots**).

## Pick the layer

Commands run from the repository root (or a worktree root).

| Layer | Proves | Command |
| --- | --- | --- |
| Unit | A rule or a parser: state ladders, JSON to state, layout math, glyph widths | `cargo test -p workspace-status --lib`, `cargo test -p workspace-status-graph` |
| PTY e2e | The real binary on a PTY with live `event::read`: keys, painted text, fake `gh` / `glab`. Unix only. A `slow-tests` suite: CI runs it all, run the tests you work on | `cargo test -p workspace-status --features slow-tests --test tui_tty_e2e <pty_name>` |
| Desktop `#[ignore]` | Real terminal emulator under `DISPLAY`: xfce keys, xterm wheel | `./scripts/with-desktop-session.sh --auto -- cargo test -p workspace-status --features slow-tests --test tui_tty_e2e -- --ignored --nocapture --test-threads=1` |
| Live forge `#[ignore]` | Real `gh` / `glab` against public PRs on github.com and gitlab.com (`cli/cli`, `gitlab-org/cli`), read-only | `cargo test -p workspace-status --lib pull_request -- --ignored --nocapture` |
| wsctl / driver | What a user sees, in states a test does not cover yet; live forge in the TUI | `wsctl` (below); `scripts/verify-tui.sh` for scenarios, mouse and forge calls |
| Local gate | The fast suites; CI adds the `slow-tests` suites (`--all-features`) on the pull request | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --locked` |

Rule of thumb: a rule goes to a unit test. A key, a screen, or a forge answer that must keep working goes to a `pty_*` test. Drive the TUI to see a change with your own eyes, to reproduce a bug, to try a live forge, and to recheck after a rebase. Driver proof does not replace a test: behaviour that must stay needs a `pty_*` test too.

## Capabilities first

Before a repro or a sweep, report the five capabilities and stop (fail closed) when one is missing. Never report a check as PASS through a path you could not drive.

| Capability | How | Missing means |
| --- | --- | --- |
| Bring up | `wsctl start` prints `"ok":true` with the run dir | No TUI checks. Build errors are in `stderr` of the failure JSON |
| Drive | `wsctl key` / `wsctl type` on a live session (`doctor`: `session`) | No TUI checks |
| Inspect | `wsctl wait` / `wsctl snap` read the pane (`doctor`: `size`, `session`) | No TUI checks |
| Snapshot | `wsctl run -- --json` (`jq` to check it) | No headless checks; TUI checks can still run |
| Clean up | `wsctl stop` prints `"stopped":true` | Do not start another run; report the live socket |

`wsctl doctor` checks tmux, bash, a fresh binary, the run's private tmux session, the run's isolation and the pane size, and exits 1 when one fails. Without tmux, prove the behaviour with the PTY e2e instead: `cargo test -p workspace-status --features slow-tests --test tui_tty_e2e <pty_name>`.

## wsctl loop

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start                         # build, seed docs/demo.md § Workspace, launch 160x45; prints the run
$WS doctor                        # every check ok, or stop and report
$WS key j                         # one key per write
$WS wait --cursor 'Changes'       # wait for the state the key should produce
$WS snap --name after-j           # evidence/NN-after-j.txt/.ansi; "screen" holds the text
$WS run -- --json | jq '.json.repos[0].repo'
$WS stop                          # quit, end the tmux server, clear the current run
```

1. **Launch.** `start` builds (`scripts/verify-tui.sh build`), seeds and launches. Options: `--size`, `--fake-forge DIR`, `--workspace DIR`, `--env K=V`, `--no-build`. It refuses while this checkout's current run is live.
2. **Doctor.** `doctor` after every `start`, and again when a step behaves oddly.
3. **Drive.** `key`, `type`, `resize`; follow each with `wait` (or `wait-ready` after a resize or an overlay).
4. **Evidence.** `snap` before and after the key that should change the screen; `run` for the headless snapshot. All files land in `<run>/evidence/`.
5. **Clean up.** `stop`, also after a failure. It is safe to repeat.

`wsctl --help` is the canonical command list; `wsctl <command> --help` gives the options and the JSON members. `--run DIR` (or `WSCTL_RUN`) acts on another run of this checkout; `start` with one of them set leaves the current run alone. The current run is a file under the root, one per checkout (`<root>/wsctl-current-<checksum of the checkout path>`), so it carries over between shells (each tool call may be a new shell) and worktrees that share the root never see or replace each other's run. Every command refuses a run that is not this checkout's: outside the root, an incomplete `run.env`, a socket other than `ws-verify-<run id>`, or another checkout's path or binary. The root is `$WS_VERIFY_ROOT`, else `${TMPDIR:-/tmp}/ws-verify`. A system `TMPDIR` can be cleared on reboot: set `WS_VERIFY_ROOT` to a durable directory when you must keep the proof.

For the fake forge, mouse and forge calls, use the driver with the run dir that `start` prints: `scripts/verify-tui.sh scenario "$FORGE" github-ready` (before `start --fake-forge "$FORGE"`, or later to swap an answer), `scripts/verify-tui.sh mouse "$RUN" find merger`, `scripts/verify-tui.sh calls "$RUN"`.

### Gotchas

- **One key per call.** `wsctl key j j j` makes three writes 50 ms apart: a repeat that reaches the app in one read counts as one press (the held-navigation guard). A chord is two keys in one write: `wsctl key --chord g x` (`g g`, `g p`, `z z`); two presses more than 0.4 s apart are not a chord.
- **Wait, never sleep.** Panes load off the event thread, so a screen read right after a key can be one frame old. `wait` on the text the step should produce.
- **`wait --cursor` is plain text** on the focus-bar line (`▌`). The tree and the right pane share screen lines, so pick text that only the target row has (`'& feature/auth-refresh'`, not `'app'`).
- **Overlays hide the footer.** Search, help (`?`) and the command palette hide `? help`. `wait-ready` waits for the tab strip ` Workspace` on row 1, which stays above every overlay.
- **Timers are off.** `WS_STATUS_WATCH_MS=0`, `WS_STATUS_FETCH_MS=0` and `WS_STATUS_PR_POLL_MS=0`: the screen never changes on its own. Press `r` to refresh, or turn a timer on with `--env`.
- **Rebuild after a code change.** A run keeps the binary it started with. `wsctl stop`, then `wsctl start` (it rebuilds); `doctor` fails on a stale build or a run older than the build.
- **A failed `wait` leaves the TUI running.** The failure JSON holds the saved screen (`screen`, `screen_file`). Stop the run before the next one.

## Proof

- **Drive the user path.** Press the keys a user presses on the screen under test. A `--plain` or `--json` result does not prove the TUI, and another path passing does not prove this one.
- **Record the trigger and the end state.** Capture the screen before and after the key that should change it, and keep the key list (`evidence/actions.log`).
- **PASS only on captured text.** A claim is PASS only when the expected text is on a captured screen (or in a recorded URL or call), never because a command exited 0.

```text
<run>/
  run.env env.sh launch.sh opened.log      run facts, isolation, URLs the app opened
  evidence/
    run.txt env.sh.txt seed.log actions.log
    NN-<name>.txt .ansi                     snap / capture
    NN-wait-timeout.txt                     the screen when a wait failed
    NN-cli-<args>.{cmd,out,err,rc}          wsctl run / cli
    forge-calls.log opened.log              copied by stop (the calls of this run only)
  workspace/ config/ state/                 fixture and app state (disposable)
```

Report format:

```text
Verify: <what was checked>   build <short sha> <version>   run <run dir>   mode fake forge | live
| # | Check | Result | Proof |
| V1 | github-ready shows the ready badge on app | PASS | evidence/04-ready.txt |
| V2 | gc opens the failed check | FAIL | expected .../job/6002, got .../job/6001 (opened.log) |
| V3 | toast on transition | SKIP | needs polling and toasts (this build has neither) |
Not verified: <what could not run and why>
```

PASS, FAIL (quote what you saw), or SKIP (name the missing precondition). Never report a SKIP as PASS.

**Full sweep:** run the feature files in the order of [features/README.md](features/README.md), ending with [multi-surface-journeys](features/multi-surface-journeys.md).

## Driver reference

`wsctl` calls these; the fake forge, scenario and mouse recipes call them directly.

```bash
V=scripts/verify-tui.sh
$V build                                  # target/debug/workspace-status of THIS checkout; prints the path
FORGE=$(mktemp -d "${TMPDIR:-/tmp}/ws-forge.XXXXXX")
$V scenario "$FORGE" context github-review gitlab-ready
RUN=$($V launch --fake-forge "$FORGE")    # prints the run dir
$V wait --regex "$RUN" 'feature/auth-refresh +P'       # the badge has landed
for row in '/ src' '# Staged' '& feature/auth-refresh'; do  # cursor from session.ts up to the app checkout row
  $V keys "$RUN" k && $V wait --cursor "$RUN" "$row"
done
$V keys "$RUN" g x                        # gx opens the PR; the recorder gets the URL
$V wait "$RUN" 'opened PR #7'
for _ in $(seq 50); do grep -q . "$RUN/opened.log" 2>/dev/null && break; sleep 0.1; done   # the recorder can lag the status line
cat "$RUN/opened.log"
$V capture "$RUN" after-gx                # prints the screen, saves evidence/NN-after-gx.txt/.ansi
$V stop "$RUN"
```

| Command | Does |
| --- | --- |
| `build` | `cargo build -p workspace-status --bin workspace-status`; prints the binary path. |
| `launch [--size COLSxROWS] [--env K=V]... [--fake-forge DIR] [--workspace DIR]` | Seeds `scripts/seed-demo-workspace.sh` into the run dir, starts the TUI in a private tmux server (default `160x45`), waits for the `? help` chrome, prints the run dir. `--env` overrides a default (repeatable). `--fake-forge DIR` turns on the fake forge (below). `--workspace DIR` drives a fixture you built, which must be inside the root. |
| `keys [-l] <run> <key>...` | `tmux send-keys` with key names: `j`, `Enter`, `Escape`, `Tab`, `C-c`, `'?'`. `g x` sends the two keys of a chord. `-l` sends one argument as literal text (a search term), one character at a time. |
| `mouse <run> move\|click <col> <row>`, `mouse <run> find <text>` | Writes an xterm SGR (1006) mouse report to the pane, as a terminal does. `move` rests the pointer on a cell (motion, no button: hover); `click` is a left press and release. Cells are 1-based; capture line 1 is row 1. `find` prints `<col> <row>` of the first cell where `<text>` starts on screen (one cell per character), or exits 1, so a recipe hovers by text, not by a fixed cell. |
| `wait [--gone] [--cursor\|--regex] <run> <text> [timeout]` | Waits until `<text>` is on the screen (default 10 s). `--cursor`: until a line with the focus bar `▌` holds it. `--regex`: `<text>` is an extended regex that matches inside one line. `--gone` inverts any of them. It fails at once, in both modes, when the tmux session is missing or the TUI has exited (a gone TUI must not satisfy `--gone`). A timeout or such a failure saves the screen to `evidence/NN-wait-timeout.txt` or `NN-wait-failed.txt` and exits 1. |
| `capture <run> [name]` | Prints the screen and saves `evidence/NN-<name>.txt` plus `.ansi` (with colour). Works on a dead pane: it shows the panic text. |
| `stop <run>` | Sends `q`, ends the tmux server of this run, copies the calls of this run and `opened.log` into `evidence/`. The run dir stays. Safe to repeat. |
| `cli <run> -- <args>` | Runs the same binary with the same isolation, headless, in the fixture. Needs `--plain`, `--json`, `--help` or `--version`; refuses `--update`. Saves `NN-cli-*.{cmd,out,err,rc}`. |
| `scenario [--reset] <forge-dir> <set>...` | Copies scenario sets into a fake forge directory. Without `--reset` the sets are **overlays** on what is there: a set replaces the files it ships and leaves the others. `--reset` first clears the scenario files (`gh/`, `glab/`; `calls.log` stays), so use it to start a run from a known state, before `launch`. Each file goes in through a temporary name and a rename, so an overlay is also the mid-run swap (a reset is not: a call in the gap finds no scenario). |
| `calls <run>` | Prints the `gh` / `glab` calls this run made (fake forge only): the lines of `calls.log` after the ones that were there at launch. |

`<run>` is the run dir that `launch` (or `wsctl start`) prints, or its id under the root. The driver runs only `target/debug/workspace-status` of the checkout it lives in, never a `ws` or `workspace-status` on `PATH` (an installed release is often older than the code under test).

### Isolation

Same rules as the PTY suite (`AGENTS.md`, **TUI test layers**); `crates/workspace-status/tests/release_watch.rs` guards them, and `wsctl doctor` checks the main ones on a live run. `launch` and `cli` read one file, `<run>/env.sh` (a copy is `evidence/env.sh.txt`):

- `WS_STATUS_UPDATE_CHECK_STORE` is a temp file with a fresh `lastCheckUnix`, so the `Update? [y/N]` prompt cannot block mount or touch the operator file.
- `XDG_CONFIG_HOME` and `XDG_STATE_HOME` are inside the run dir, with `WS_STATUS_UI_STORE`, `WS_STATUS_COMMENT_STORE` and `WS_STATUS_VIEWED_STORE`. The operator config, theme, comments and reviewed marks do not leak in, and nothing writes them.
- `WS_STATUS_THEME_MODE=dark`. `WS_STATUS_WORKSPACE` and `WS_STATUS_THEME` are unset, and so is every other inherited `WS_STATUS_*` and `WORKSPACE_STATUS_*`.
- `NO_COLOR`, `FORCE_COLOR`, `CLICOLOR_FORCE`, `VISUAL`, `DISPLAY` and `WAYLAND_DISPLAY` are unset (no host clipboard tool, no editor).
- Git: `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`, `GIT_ALLOW_PROTOCOL=file`, `GIT_TERMINAL_PROMPT=0`. A fixture can never fetch or push over a network, whatever key you press.
- Stable frames: `WS_STATUS_GLYPHS=ascii`, `WS_STATUS_WATCH_MS=0`, `WS_STATUS_FETCH_MS=0`, `WS_STATUS_PR_POLL_MS=0`, `TERM=xterm-256color`, `COLORTERM=truecolor`, a UTF-8 locale, `TZ=Asia/Manila` (the seed clock). Override with `--env` (for example `--env WS_STATUS_GLYPHS=nerd`).
- `BROWSER` is a recorder that appends each opened URL to `<run>/opened.log`. A run never starts a real browser, in fake or live mode.
- With `--fake-forge`: `GH_TOKEN`, `GITHUB_TOKEN`, `GITLAB_TOKEN` (and the enterprise and access-token variants) and the `gh` / `glab` config dirs are unset. `scripts/fake-forge` is first on `PATH`, and `WS_FAKE_FORGE_DIR` names the scenario directory.
- Live (no `--fake-forge`): token variables stay as they are. The `gh` and `glab` config dirs are resolved from the operator environment (`glab config path` names the glab one) before `XDG_CONFIG_HOME` moves, and exported as `GH_CONFIG_DIR` and `GLAB_CONFIG_DIR`, so a stored login keeps working.

The seeded workspace is the demo fixture: `docs/demo.md` § Workspace. The first cursor is `session.ts` (`S`) under `app` → Staged.

## Fake forge

`--fake-forge DIR` swaps the real `gh` and `glab` for the scripts in `scripts/fake-forge/`. They answer from JSON files under `DIR`; the format is in `scripts/fake-forge/README.md`. The app has no test hook: it runs the same code path as in production.

The driver points the seeded repos at made-up remotes, so the app finds a forge:

| Checkout | Origin | Branch |
| --- | --- | --- |
| `app` (and its worktree rows) | `https://github.com/acme/app.git` | `feature/auth-refresh`; worktree `feature/login-page` |
| `merger` (and its worktree row) | `https://github.com/acme/merger.git` | `feature/reconciliation` |
| `services/api` | `https://gitlab.com/acme/services/api.git` | `feature/rate-limit` |
| `lib` | local path, no forge | `main` |

Scenario sets live in `.agents/skills/verify-workspace-status/scenarios/`. Apply sets with `scenario` before `launch` (the first lookup runs at launch) or later to swap an answer; then press `r` on the checkout (or on the workspace row, which reloads every checkout), or wait for a poll.

| Set | Answer |
| --- | --- |
| `context` | `app` worktree `feature/login-page`: open PR #8, in review. `merger` `feature/reconciliation`: merged PR #12. Fixed rows for a second and third badge. |
| `github-failed`, `-ready`, `-running`, `-draft`, `-review`, `-merged` | PR #7 of `acme/app` in that state (list answer and `gh pr view` answer). Two checks: `lint` (listed first) and `test (ubuntu-latest)`. |
| `gitlab-failed`, `-ready`, `-running`, `-draft`, `-review`, `-merged` | MR !5 of `acme/services/api` in that state (list, MR, approvals, and for failed or running the pipeline jobs). |
| `github-behind`, `gitlab-behind` | PR #7 / MR !5 approved, green CI, branch behind its base: GitHub `mergeStateStatus: BEHIND`, GitLab `detailed_merge_status: need_rebase` (approvals left 0). The badge stays in review (`P`); the next action is `update branch`. |
| `github-none`, `gitlab-none` | The forge answers with an empty list: no PR. |
| `context-none` | The `context` branches (`feature/login-page`, `feature/reconciliation`) answer an empty list. With `github-none` and `gitlab-none`, no seeded branch has a PR. |
| `github-cli-fails`, `gitlab-cli-fails` | The CLI exits 1 with one stderr line. |
| `prs-tab` | Every seeded branch gets a PR whose title carries a ticket key, for the PRs tab groups: #7 CI failed and #8 stacked on it (`ABC-123`), !5 CI running (`ABC-125`), #12 merged (`ABC-120`). List and detail answers. |

A set directory has the layout of a forge directory, so `scenario` is a copy that renames each file into place and removes a stale failure file. Add a set by adding a directory.

`$FORGE/calls.log` lists every `gh` and `glab` call the app made (argv line, program first), across runs. `calls <run>` prints only the lines of one run. Use it to prove the app asked, and to find a call the fake does not know (`fake forge: no scenario for ...`). Use one run at a time per forge directory; give runs that overlap a forge directory each.

## Live mode

Start without `--fake-forge`, on a fixture whose `origin` is a public repo. The app then calls the real `gh` and `glab`: read-only lookups of public PRs. `gh auth status` and `glab auth status` must pass first. Pick the branches at run time; the recipe is in [features/pr-status.md](features/pr-status.md), **Live**. No key you press can fetch or push (`GIT_ALLOW_PROTOCOL=file`).

## Rules for driving

- **Mouse by text, then wait.** Get the cell with `mouse find`, send `mouse move` or `mouse click`, then `wait` for the text the pointer should produce (a hover peek needs a 400 ms rest). A click is a real click: it selects the row under it, and two clicks on one cell close together are a double click.
- **Select by row text.** Use `wait --cursor` on text unique to the row, not a key count. After `Tab` the focus bar is in the right pane; `wait --cursor` follows it.
- **Keys under tmux are legacy bytes.** There is no kitty keyboard protocol: Shift letters arrive as plain letters and `Escape` is a lone ESC byte. The PTY suite sends CSI-u (`docs/tui-tty-e2e.md`), a different input path.
- **OSC 8 links and OSC 9;4 progress are not on a tmux capture.** Claims about them belong to PTY e2e tests, which read the raw byte stream.
- **Mutating keys** (`s` `u` `x` `S` `a` `p` `D` `P` `f` `d` `b` `m` `c` `W`, Space) write git state or a store in the fixture. Use them only against the run's own fixture, answer confirms with `n` unless the feature needs `y`, and start a new run after any write. `e` and `E` start an editor or diff tool in the pane, and `y` and `'` copy through OSC 52, which tmux can pass on to the host clipboard: press them only when the feature needs them.
- **One TUI per run.** Never drive, signal or attach to an instance this run did not start. A TTY run waits for keys forever: never start the binary outside `wsctl` or the driver without `--plain` or `--json`.
- **Stop every run.** A run keeps its tmux server, and with `WS_STATUS_PR_POLL_MS` set it keeps polling, so it can answer into the next run. The driver recipes in the forge feature files stop with `fresh` / `relaunch` and an exit `trap`. **If every command gets a fresh shell** (one tool call = one shell), that `trap` fires when the call that set it ends, and `V`, `RUN` and the helpers do not carry over: run Setup and a whole recipe in one call or one script file, or set `KEEP_RUN=1`, keep the printed run dir, and `stop <run dir>` by path in the last call. See **Setup**, **One shell or many** in [features/pr-status.md](features/pr-status.md). `wsctl` keeps its current run in a file, so its calls carry over between shells.

## Features

[features/README.md](features/README.md) lists every feature file in sweep order. Each file lists steps, the screen text to expect, and what counts as PASS. The forge files need the PR status visuals (six states, ref beside the glyph, `gc`, polling, toasts), the `prs-tab` checks need the PRs tab, PO3 needs the commit popovers, and PS7, PS9 and PT7 need the outage toast and the header failure mark; on an older build they are SKIP.
