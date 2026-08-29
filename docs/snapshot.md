# Workspace snapshot

`--plain` and `--json` read one workspace snapshot.
The TUI uses the same discovery path. Display differs.

`--json` prints this object on stdout. `--plain` renders the same object as text.

Agents must pass `--plain` or `--json`. A TTY without one of those flags opens the TUI and waits for keys.

## Object

```json
{
  "version": 1,
  "showIgnored": false,
  "filterRepos": [],
  "ignoredRepos": ["notes"],
  "repos": []
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | `1` | Contract version. Bump only for a breaking change. |
| `showIgnored` | boolean | True when this run includes ignored repos (`-a` / `--all`). |
| `filterRepos` | string[] | Positional repo filters, sorted. Empty means all discovered repos. |
| `ignoredRepos` | string[] | Paths from `.workspace-status-config.json`, sorted. |
| `repos` | object[] | Ops-visible repos only. Hidden ignored repos are omitted. |

Paths are workspace-relative. The snapshot does not include the workspace root, machine paths, or secrets.

## Repo object

| Field | Type | Meaning |
| --- | --- | --- |
| `repo` | string | Workspace-relative checkout path. |
| `ignored` | boolean | True when `repo` is listed in `ignoredRepos`. |
| `branch` | string | Current branch, or a detached / unknown label from git status. |
| `syncStatus` | string | `up-to-date`, `no-upstream`, `behind`, `ahead`, or `diverged`. |
| `syncNote` | string | Extra sync text (`ahead by 2`, `status failed`, empty). |
| `checkoutKind` | string | `primary` or `linked`. |
| `primaryRepo` | string | Present on linked checkouts. Path of the primary checkout. |
| `mergedIntoDefault` | boolean or null | Whether HEAD is merged into the default branch. `null` when not checked. |
| `defaultBranchOverride` | string | Present when workspace config sets a default branch for this repo. |
| `hasUnstaged` | boolean | Unstaged tracked changes. |
| `hasStaged` | boolean | Staged changes. |
| `hasUntracked` | boolean | Untracked files. |
| `changes` | object[] | Per-path file changes, sorted by `path`. |

Omitted optional keys: `primaryRepo`, `defaultBranchOverride`, and empty change flags on each file.

These fields cover branch, sync, checkout kind, merge-into-default, and file changes. They are not a full commit graph. `HEAD` is collected for TUI live-watch identity and is omitted from `--json`. Local `refs/heads` names are collected on the snapshot worker for TUI comment GC and are omitted from `--json`. When status fails, the TUI keeps the last good branch list for GC only if every checkout of that identity has an empty counted list. A successful sibling's live names win.

## File change object

| Field | Type | Meaning |
| --- | --- | --- |
| `path` | string | Path relative to that repo. |
| `stagedStatus` | string | Porcelain status letter when staged (`A`, `M`, `D`, `R`, `U`, …). |
| `unstagedStatus` | string | Porcelain status letter when unstaged. |
| `untracked` | boolean | True for untracked paths. |
| `oldPath` | string | Present on renames. |

## Visibility and ops

Hidden ignored repos stay out of `repos` and out of fetch, pull, and default-branch work unless they are shown.

Shown means:

1. `-a` / `--all` (`showIgnored` is true), or
2. the repo is named in `filterRepos`.

Named filters still bypass `ignoredRepos`. The TUI can show hidden ignored repos with `.`. That toggle does not change `--plain` or `--json` unless you pass `-a`.

## Flags

| Flag | Role |
| --- | --- |
| `--plain` | Human text of this snapshot. Required for agents if you do not use `--json`. |
| `--json` | Pretty-printed JSON of this snapshot on stdout. Progress from `--fetch`, `--pull`, or `--default-branch` goes to stderr. |
| `-v` | Verbose table in the `--plain` renderer. Ignored by `--json`. |
| `-a` | Include ignored repos in `repos`. |
| `[REPO...]` | Limit `repos` to those paths. |

`--json` and `--plain` both disable the TUI. If both are set, `--json` wins.

## Example

A workspace with `app` dirty and `notes` ignored:

```json
{
  "version": 1,
  "showIgnored": false,
  "filterRepos": [],
  "ignoredRepos": ["notes"],
  "repos": [
    {
      "repo": "app",
      "ignored": false,
      "branch": "main",
      "syncStatus": "up-to-date",
      "syncNote": "",
      "checkoutKind": "primary",
      "mergedIntoDefault": null,
      "hasUnstaged": true,
      "hasStaged": false,
      "hasUntracked": false,
      "changes": [
        {
          "path": "README.md",
          "unstagedStatus": "M"
        }
      ]
    }
  ]
}
```

`--plain` on the same snapshot prints the File changes section for `app` and does not mention `notes`.
