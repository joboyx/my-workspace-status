# JSON and plain snapshot

`wsctl run -- <args>` runs the binary of the run headless, in the run's fixture, with the run's `env.sh` (through `scripts/verify-tui.sh cli`). It needs `--plain`, `--json`, `--help` or `--version` and refuses `--update` and `--tui`. The output, stderr and exit code go to `evidence/NN-cli-*.{cmd,out,err,rc}`. For `--json`, the JSON of `wsctl` embeds the parsed snapshot as `.json` when `jq` is installed. The snapshot contract is `docs/snapshot.md`; the plain report is `SAMPLE_OUTPUT.md`.

Conventions and the sweep order: [README.md](README.md). These recipes need `jq`. A headless result proves the snapshot, not the TUI: it never stands in for a TUI check.

## JS1 `--json` lists the fixture checkouts

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
$WS doctor
trap '$WS stop' EXIT
OUT=$($WS run -- --json)
echo "$OUT" | jq -e '.ok and .rc == 0'
echo "$OUT" | jq -e '[.json.repos[].repo] == ["app","app/.worktrees/feat-login","lib","merger","merger/.worktrees/recon","services/api"]'
echo "$OUT" | jq -e '.json.showHidden == false and .json.hiddenPaths == ["notes"]'
echo "$OUT" | jq -e '.json.repos[] | select(.repo == "app") | .branch == "feature/auth-refresh" and .syncStatus == "ahead" and ([.changes[].path] == ["src/auth.ts","src/login.ts","src/session.ts"])'
echo "$OUT" | jq -e '.json.repos[] | select(.repo == "services/api") | .syncStatus == "diverged"'
echo "$OUT" | jq -r .files.out
```

Expect: six checkouts in this order, `notes` hidden (`hiddenPaths: ["notes"]`, `showHidden: false`), `app` on `feature/auth-refresh` and ahead with three changes (`auth.ts` unstaged `M`, `login.ts` untracked, `session.ts` staged), `services/api` diverged.

**PASS:** every `jq -e` prints `true`, and the snapshot file named by `.files.out` holds the JSON.

## JS2 `--json -a` shows the hidden checkout

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
$WS run -- --json -a | jq -e '.json.repos[] | select(.repo == "notes") | .hidden == true'
```

**PASS:** `jq -e` prints `true`: `notes` is in the snapshot with `hidden: true`.

## JS3 `--plain` and `--version`

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
PLAIN=$($WS run -- --plain | jq -r .stdout)
grep -F '📦 app' <<<"$PLAIN"
grep -F '🟡M auth.ts' <<<"$PLAIN"
grep -F 'services/api [feature/rate-limit] - diverged (ahead 1, behind 1)' <<<"$PLAIN"
$WS run -- --version | jq -e '.stdout | test("^workspace-status [0-9]+\\.[0-9]+\\.[0-9]+")'
```

Expect: the plain report starts with `File changes` and lists `📦 app` with `🟡M auth.ts`, `🟢A login.ts`, `🔵S session.ts`, then `🔄 Sync status (2)` with `services/api [feature/rate-limit] - diverged (ahead 1, behind 1)`. `--version` prints `workspace-status <version>`.

**PASS:** the three `grep` lines match and `jq -e` prints `true`.

## JS4 The refused flags

```bash
WS=.agents/skills/verify-workspace-status/scripts/wsctl
$WS start
trap '$WS stop' EXIT
if R=$($WS run -- --update); then echo "FAIL: --update ran"; exit 1; fi
echo "$R" | jq -e '.ok == false and (.error | test("--update is refused"))'
if R=$($WS run -- --verbose); then echo "FAIL: a bare --verbose ran"; exit 1; fi
echo "$R" | jq -e '.ok == false and (.error | test("pass --plain, --json, --help or --version"))'
```

Expect: `--update` fails with `--update is refused (updater and TUI need the tmux run)`; `--verbose` alone fails with `pass --plain, --json, --help or --version`, because a bare run opens the TUI and waits for keys. In both cases wsctl exits 1 and prints `"ok":false`.

**PASS:** neither `if` line prints `FAIL` and both `jq -e` lines print `true`.
