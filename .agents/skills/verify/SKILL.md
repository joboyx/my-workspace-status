---
name: verify
description: Prove a workspace-status change works. Picks the test layer that proves a claim, then drives the real TUI of this checkout in an isolated tmux run, offline with fake gh and glab answers or live against public repos, and saves screen captures as proof. Use when verifying a TUI, PR badge, PRs tab or forge change, reproducing a bug with evidence, or before you report a UI change done.
---

# Verify workspace-status

`workspace-status` (`ws`) is a ratatui TUI over every git checkout under a workspace root. This skill does two things:

1. It tells you which test layer proves which claim, with the exact commands.
2. It gives a driver, `scripts/verify-tui.sh`, that runs the real binary **of this checkout** in a private tmux server on a seeded throwaway workspace. You send real key and mouse bytes, wait for text, and save the screen as proof.

The driver is a **test harness**, like the PTY e2e suite. It is not a capture pipeline. README and demo clips come only from `scripts/capture-demo-stills.sh` (`AGENTS.md`, **Demo / screenshots**). Do not use the driver to make clips or stills, and do not use the demo script to check behaviour.

## Pick the layer

Commands run from the repository root (or a worktree root).

| Layer | Proves | Command |
| --- | --- | --- |
| Unit | A rule or a parser: state ladders, JSON to state, layout math, glyph widths | `cargo test -p workspace-status --lib`, `cargo test -p workspace-status-graph` |
| PTY e2e | The real binary on a PTY with live `event::read`: keys, painted text, fake `gh` / `glab`. Unix only | `cargo test -p workspace-status --test tui_tty_e2e` (add a test name to filter) |
| Desktop `#[ignore]` | Real terminal emulator under `DISPLAY`: xfce keys, xterm wheel | `./scripts/with-desktop-session.sh --auto -- cargo test --test tui_tty_e2e -- --ignored --nocapture --test-threads=1` |
| Live forge `#[ignore]` | Real `gh` / `glab` against public PRs on github.com and gitlab.com (`cli/cli`, `gitlab-org/cli`), read-only | `cargo test -p workspace-status --lib pull_request -- --ignored --nocapture` |
| Verify driver | What a user sees, in states a test does not cover yet; live forge in the TUI | `scripts/verify-tui.sh` (below) |
| CI gate | Everything CI runs | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` |

Rule of thumb: a rule goes to a unit test. A key, a screen, or a forge answer that must keep working goes to a `pty_*` test. Use the driver to see a change with your own eyes, to reproduce a bug, to try a live forge, and to recheck after a rebase. Driver proof does not replace a test: behaviour that must stay needs a `pty_*` test too.

## Driver

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
| `wait [--gone] [--cursor\|--regex] <run> <text> [timeout]` | Waits until `<text>` is on the screen (default 10 s). `--cursor`: until a line with the focus bar `▌` holds it. `--regex`: `<text>` is an extended regex that matches inside one line. `--gone` inverts any of them: wait until it is not there. It fails at once, in both modes, when the tmux session is missing or the TUI has exited (a gone TUI must not satisfy `--gone`). A timeout or such a failure saves the screen to `evidence/NN-wait-timeout.txt` or `NN-wait-failed.txt` and exits 1. |
| `capture <run> [name]` | Prints the screen and saves `evidence/NN-<name>.txt` plus `.ansi` (with colour). Works on a dead pane: it shows the panic text. |
| `stop <run>` | Sends `q`, ends the tmux server of this run, copies the calls of this run and `opened.log` into `evidence/`. The run dir stays. Safe to repeat. |
| `cli <run> -- <args>` | Runs the same binary with the same isolation, headless, in the fixture. Needs `--plain`, `--json`, `--help` or `--version`; refuses `--update`. Saves `NN-cli-*.{cmd,out,err,rc}`. |
| `scenario [--reset] <forge-dir> <set>...` | Copies scenario sets into a fake forge directory. Without `--reset` the sets are **overlays** on what is there: a set replaces the files it ships and leaves the others. `--reset` first clears the scenario files (`gh/`, `glab/`; `calls.log` stays), so use it to start a run from a known state, before `launch`. Each file goes in through a temporary name and a rename, so an overlay is also the mid-run swap (a reset is not: a call in the gap finds no scenario). |
| `calls <run>` | Prints the `gh` / `glab` calls this run made (fake forge only): the lines of `calls.log` after the ones that were there at launch. |

`<run>` is the run dir that `launch` prints (or its id under the root). The root is `$WS_VERIFY_ROOT`, else `${TMPDIR:-/tmp}/ws-verify`. A system `TMPDIR` can be cleared on reboot: set `WS_VERIFY_ROOT` to a durable directory when you must keep the proof.

The driver runs only `target/debug/workspace-status` of the checkout it lives in, never a `ws` or `workspace-status` on `PATH` (an installed release is often older than the code under test). Rebuild after a code change, then launch a new run: a run keeps the binary it started with.

### Isolation

Same rules as the PTY suite (`AGENTS.md`, **TUI test layers**); `crates/workspace-status/tests/release_watch.rs` guards them. `launch` and `cli` read one file, `<run>/env.sh` (a copy is `evidence/env.sh.txt`):

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

Scenario sets live in `.agents/skills/verify/scenarios/`. Apply sets with `scenario` before `launch` (the first lookup runs at launch) or later to swap an answer; then press `r` on the checkout (or on the workspace row, which reloads every checkout), or wait for a poll.

| Set | Answer |
| --- | --- |
| `context` | `app` worktree `feature/login-page`: open PR #8, in review. `merger` `feature/reconciliation`: merged PR #12. Fixed rows for a second and third badge. |
| `github-failed`, `-ready`, `-running`, `-draft`, `-review`, `-merged` | PR #7 of `acme/app` in that state (list answer and `gh pr view` answer). Two checks: `lint` (listed first) and `test (ubuntu-latest)`. |
| `gitlab-failed`, `-ready`, `-running`, `-draft`, `-review`, `-merged` | MR !5 of `acme/services/api` in that state (list, MR, approvals, and for failed or running the pipeline jobs). |
| `github-none`, `gitlab-none` | The forge answers with an empty list: no PR. |
| `context-none` | The `context` branches (`feature/login-page`, `feature/reconciliation`) answer an empty list. With `github-none` and `gitlab-none`, no seeded branch has a PR. |
| `github-cli-fails`, `gitlab-cli-fails` | The CLI exits 1 with one stderr line. |
| `prs-tab` | Every seeded branch gets a PR whose title carries a ticket key, for the PRs tab groups: #7 CI failed and #8 stacked on it (`ABC-123`), !5 CI running (`ABC-125`), #12 merged (`ABC-120`). List and detail answers. |

`.agents/skills/verify/scenarios/<set>/` has the layout of a forge directory, so `scenario` is a copy that renames each file into place and removes a stale failure file. Add a set by adding a directory.

`$FORGE/calls.log` lists every `gh` and `glab` call the app made (argv line, program first), across runs. `calls <run>` prints only the lines of one run. Use it to prove the app asked, and to find a call the fake does not know (`fake forge: no scenario for ...`). Use one run at a time per forge directory; give runs that overlap a forge directory each.

## Live mode

Run `launch` without `--fake-forge`, on a fixture whose `origin` is a public repo. The app then calls the real `gh` and `glab`: read-only lookups of public PRs. `gh auth status` and `glab auth status` must pass first. Pick the branches at run time; the recipe is in [features/pr-status.md](features/pr-status.md), **Live**. No key you press can fetch or push (`GIT_ALLOW_PROTOCOL=file`).

## Rules for driving

- **Wait for text, never sleep.** Panes load off the event thread, so a screen read right after a key can be one frame old. Follow each key with `wait` on the text the step should produce.
- **Send one key per write; check the cursor after each move.** Give movement keys one per `keys` call and follow each with a `wait` (use `--cursor` on the row you expect). `keys -l` types text one character at a time. A chord is the one exception: two keys in one call.
- **Mouse by text, then wait.** Get the cell with `mouse find`, send `mouse move` or `mouse click`, then `wait` for the text the pointer should produce (a hover peek needs a 400 ms rest). A click is a real click: it selects the row under it, and two clicks on one cell close together are a double click.
- **Select by row text.** Use `wait --cursor` on text unique to the row (`'& feature/auth-refresh'`), not a key count. After `Tab` the focus bar is in the graph; `wait --cursor` follows it.
- **Chords are two keys in one call** (`g x`, `g c`, `g g`). Two separate presses of `z` or `g` more than 0.4 s apart are not a chord.
- **Keys under tmux are legacy bytes.** There is no kitty keyboard protocol: Shift letters arrive as plain letters and `Escape` is a lone ESC byte. The PTY suite sends CSI-u (`docs/tui-tty-e2e.md`), a different input path.
- **OSC 8 links and OSC 9;4 progress are not on a tmux capture.** Claims about them belong to PTY e2e tests, which read the raw byte stream.
- **Mutating keys** (`s` `u` `x` `S` `a` `p` `D` `P` `f` `d` `b` `m` `c` `W`, Space) write git state or a store in the fixture. Use them only against the run's own fixture, answer confirms with `n` unless the feature needs `y`, and start a new run after any write. `e` and `E` start an editor or diff tool in the pane, and `y` and `'` copy through OSC 52, which tmux can pass on to the host clipboard: press them only when the feature needs them.
- **One TUI per run.** Never drive, signal or attach to an instance this run did not start.
- A TTY run waits for keys forever. Never start the binary outside the driver without `--plain` or `--json`.
- **A failed `wait` leaves the TUI running.** So does any step that stops early. Always `stop` the run, and `stop` before you launch the next one: a run keeps its tmux server, and with `WS_STATUS_PR_POLL_MS` set it keeps polling, so it can answer into the next run. The feature files do this with `fresh` / `relaunch` and an exit `trap`. **If every command gets a fresh shell** (one tool call = one shell), that `trap` fires when the call that set it ends and stops the run that call started, and `V`, `RUN` and the helpers do not carry over: run Setup and a whole recipe in one call or one script file, or set `KEEP_RUN=1`, keep the printed run dir, and `stop <run dir>` by path in the last call. See **Setup**, **One shell or many** in [features/pr-status.md](features/pr-status.md).

## Proof

```text
<run>/
  run.env env.sh launch.sh opened.log      run facts, isolation, URLs the app opened
  evidence/
    run.txt env.sh.txt seed.log actions.log
    NN-<name>.txt .ansi                     capture
    NN-wait-timeout.txt                     the screen when a wait failed
    NN-cli-<args>.{cmd,out,err,rc}          cli
    forge-calls.log opened.log              copied by stop (the calls of this run only)
  workspace/ config/ state/                 fixture and app state (disposable)
```

Capture the screen **before and after** a key that should change it. A claim is PASS only when the expected text is on a captured screen (or in a recorded URL or call), never because a command exited 0.

Report format:

```text
Verify: <what was checked>   build <short sha> <version>   run <run dir>   mode fake forge | live
| # | Check | Result | Proof |
| V1 | github-ready shows the ready badge on app | PASS | evidence/04-ready.txt |
| V2 | gc opens the failed check | FAIL | expected .../job/6002, got .../job/6001 (opened.log) |
| V3 | toast on transition | SKIP | needs polling and toasts (this build has neither) |
Not verified: <what could not run and why>
```

PASS, FAIL (quote what you saw), or SKIP (name the missing precondition). Never report a SKIP as PASS, and never report a path as verified because another path passed (a `--plain` result does not prove the TUI).

## Features

| Feature file | Proves |
| --- | --- |
| [pr-status](features/pr-status.md) | PR badge states on both forges, `gx` and `gc`, a scripted transition, a failing CLI, live recipe |
| [tree-and-graph](features/tree-and-graph.md) | Badges in the tree and the graph pane, glyph column at wide and narrow widths |
| [popovers](features/popovers.md) | The PR popover: lines, actions, `gc` and `r` from it; the commit popovers (hash card, time, author) by mouse hover and click |
| [prs-tab](features/prs-tab.md) | The PRs tab (`gp`): group and row order, detail, `z` / `Z` folds, Enter / `gc` / `o` / `Ctrl-w`, narrow layout, a failing CLI, the empty state, the title command, Nerd glyphs |

Each feature file lists steps, the screen text to expect, and what counts as PASS. Every check needs the PR status visuals (six states, ref beside the glyph, `gc`, polling, toasts), the `prs-tab` checks need the PRs tab, and PO3 needs the commit popovers; on an older build they are SKIP.
