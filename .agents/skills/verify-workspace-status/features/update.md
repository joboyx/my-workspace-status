# Update entry paths

A user starts an update in three ways: `ws --update`, `y` at the TTY startup prompt `workspace-status <current> → <latest> available. Update? [y/N]`, and `ws-dev --update` (a dev install from `scripts/install-dev.sh`). After a new version is installed, the app starts again with the original arguments minus `--update`, in the same cwd. When nothing new is installed (already up to date, a network error), `--update` prints the updater's result and exits with its code, and the startup prompt opens the current TUI.

`scripts/verify-tui.sh update <entry> <outcome>` drives one case as a user does, in a run of its own, and prints one line: `PASS` or `FAIL`, the expected and the actual result, and the evidence dir. `update all` runs every case and exits 1 when one fails. Conventions and the sweep order: [README.md](README.md).

| Entry | User action |
| --- | --- |
| `cli` | `ws --update -a` |
| `prompt` | `ws -a` on a TTY with no update-check store; the driver answers `y` Enter at `Update? [y/N]` |
| `dev` | `ws-dev --update -a`; a dev build never prompts, so there is no dev prompt case |

| Outcome | What the fake updater does |
| --- | --- |
| `install` | Replaces `bin/ws` (or `bin/ws-dev`) through a copy and a rename with a "new version" script. That script appends `args=[<args>] cwd=<pwd>` to `update/new-app.log`, then runs an untouched copy of the same binary, so the real TUI comes up. Prints `FAKE-UPDATER installed`, exit 0. |
| `current` | Prints `already up to date`, exit 0. Changes nothing. |
| `fail` | Prints `network error` to stderr, exit 1. Changes nothing. |

## Fixture and isolation

Each case is a run dir under the root, with the isolation of every driver run (`env.sh`: XDG dirs, stores and config in the run dir, `WS_STATUS_*` dropped; see **Isolation** in [../SKILL.md](../SKILL.md)). On top of that:

- `bin/` holds a **copy** of the checkout's `target/debug/ws` (or the dev `ws` as `ws-dev`), the fake updater (`workspace-status-update`, or `workspace-status-update-dev`) beside it, and a fake `curl`. `next/` holds the second, untouched copy. The checkout's `target/` binaries are never run by the case or replaced.
- `PATH` is `bin/`, `scripts/fake-forge`, then `/usr/bin:/bin:/usr/sbin:/sbin`: no installed `ws`, updater, `~/.local/bin` or `~/.cargo/bin` can answer.
- The fake `curl` answers GitHub `/releases/latest` with `v99.0.0` and the releases list with one note (`- release notes`). Its calls go to `update/curl.log`; the updater's to `update/updater.log`.
- The fixture is the demo seed in `work/sub dir` (a cwd with a space).
- `prompt` runs with no update-check store, so the 6 h check is due. `cli` and `dev` keep the fresh store of every run (`--update` skips the check).
- `dev` builds the dev-marked binary with `WS_STATUS_DEV_BUILD=verify CARGO_TARGET_DIR=target/verify-dev cargo build --locked -p workspace-status` when it is missing or older than the sources. The first build is slow; do not start it beside another cargo build. `cli` and `prompt` build `target/debug/ws` the same way when it is stale.

## U1 Install starts the new version

```bash
V=scripts/verify-tui.sh
$V update cli install
$V update prompt install
$V update dev install
```

Expect, per case: `update/new-app.log` has one line, `args=[-a] cwd=<run>/work/sub dir` (no `--update`); the screen after shows the TUI first paint (`? help`), the process is still running; no `Update? [y/N]` on the screen after the answer.

**PASS:** each line starts with `PASS`. The proof is `evidence/NN-update-<entry>-install-new-app.log.txt` and `NN-update-<entry>-install-after.txt`.

## U2 No install, `--update`: print and exit

```bash
V=scripts/verify-tui.sh
$V update cli current
$V update cli fail
$V update dev current
$V update dev fail
```

Expect: no `update/new-app.log`; the process exits with the updater's code (`current` 0, `fail` 1); the TUI never paints (no sample of the pane shows `? help`); the updater's output (`already up to date` / `network error`) is on the screen or in its scrollback (tmux scrolls one line when it marks the pane dead).

**PASS:** each line starts with `PASS`; `NN-update-<entry>-<outcome>-rc.txt` holds the exit code and `-after.txt` the output.

## U3 No install, startup prompt: open the current TUI

```bash
V=scripts/verify-tui.sh
$V update prompt current
$V update prompt fail
```

Expect: the prompt is on the screen before the answer (`-before.txt`); after `y` Enter, no `update/new-app.log`, and the TUI first paint shows in the same process, still running.

**PASS:** each line starts with `PASS`.

## Evidence

```text
<run>/                                     one run dir per case: <id>-update-<entry>-<outcome>
  bin/ next/ update/                       app copies, fakes, new-app.log, curl.log, updater.log
  work/sub dir/                            the seeded fixture (the cwd)
  evidence/
    NN-update-<entry>-<outcome>-before.txt .ansi   prompt only: the screen before y
    NN-update-<entry>-<outcome>-after.txt .ansi    the screen at the end (paint and prompt checks)
    NN-update-<entry>-<outcome>-history.txt .ansi  the same with the scrollback (updater output check)
    NN-update-<entry>-<outcome>-rc.txt             exit code, or running
    NN-update-<entry>-<outcome>-{new-app,curl,updater}.log.txt
    NN-update-<entry>-<outcome>-result.txt         the PASS / FAIL line
```

The case quits the TUI and ends its tmux server before it prints its line; the run dir stays.
