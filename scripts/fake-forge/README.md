# Fake forge

POSIX `sh` stand-ins for `gh` and `glab`, for e2e tests. Put this directory first on `PATH`, set `WS_FAKE_FORGE_DIR` to a scenario directory, and the app finds the PR of a branch with no network and no login. The Rust helper `tests/common/fake_forge.rs` does both and writes the scenario files.

The scripts read flags and the `glab api` endpoint, not whole argv strings, so a longer `--json` field list or a longer query string needs no change here. They ignore `--hostname`, `--state`, `--limit` and `--json`.

## Calls answered

| Call | Scenario file |
| --- | --- |
| `gh pr list -R <host>/<owner>/<repo> --head <branch> …` | `gh/<owner>__<repo>/pr-list/<branch>.json` |
| `gh pr view <n> -R <host>/<owner>/<repo> …` | `gh/<owner>__<repo>/pr-view/<n>.json` |
| `glab api … projects/<path>/merge_requests?source_branch=<branch>&…` | `glab/<path>/mr-list/<branch>.json` |
| `glab api … projects/<path>/merge_requests/<iid>` | `glab/<path>/mr/<iid>.json` |
| `glab api … projects/<path>/merge_requests/<iid>/approvals` | `glab/<path>/mr/<iid>-approvals.json` |
| `glab api … projects/<path>/pipelines/<id>/jobs?…` | `glab/<path>/pipeline-jobs/<id>.json` |

- In a file name, `/` becomes `__`: branch `feature/x` is `feature__x`, project `acme/services/api` is `acme__services__api`.
- `-R host/owner/repo` and `-R owner/repo` both work. `glab` endpoints arrive URL-encoded (`projects/acme%2Fservices%2Fapi/…?source_branch=feature%2Fx`); the script decodes them.
- The file holds the exact stdout the real CLI would print.

## Failures

A file next to a scenario file sets how the call ends. Names add to the full file name:

- `feature__x.json.exit`: the exit code (default `0`).
- `feature__x.json.stderr`: text for stderr.

A scenario may have only the `.exit` file (a failing call with no output). A call with no scenario file prints `fake forge: no scenario for <argv>` on stderr and exits `1`.

## Log

Each call appends its argv, program first, to `$WS_FAKE_FORGE_DIR/calls.log`, one line per call, including calls with no scenario.

## Changing an answer mid-run

Write the new file under a temporary name, then `mv` it over the old one, so a call never reads half a file. The helper does this on every write.
