# Sample Output - workspace-status.sh

This document shows the main output states and combinations from `workspace-status.sh`.
`workspace-status.sh` is the source of truth. This document is supplemental and mirrors current script behavior.
All scenarios in this document are covered by `test/workspace-status.e2e.ts`.

## Workspace Config

`workspace-status.sh` reads `.workspace-status-config.json` from the current workspace root. Repos listed in `ignoredRepos` are skipped from discovery, status checks, fetch, pull, and default-branch switching unless `-a` or `--all` is passed. `maxDepth` (default `3`) is how many path segments below cwd to search for git repos. `defaultBranches` optionally maps a workspace-relative repo path to a sole default branch name; when set, classification, ordering, branch markers, and `--default-branch` / TUI `d` use that branch. Repos without an entry keep the previous behaviour (treat `main`/`master`/`develop` as default for classification; resolve switch target from git).

```json
{
  "ignoredRepos": ["notes"],
  "maxDepth": 3,
  "defaultBranches": {
    "opella/opella-main": "develop",
    "opella/opl-frontend": "develop"
  }
}
```

### Default branch override (classification)

With `"override-app": "develop"` and the repo clean on `main` (git `origin/HEAD` may still be `main`), `main` is **not** treated as default — it appears under Branches:

```text
Repo                  Branch       Sync       Files
override-app         🔥 main      ✅ current  💾 clean

🌿 Branches (1):
    - override-app [main]
```

`--default-branch` / TUI `d` switches to the configured branch (e.g. `develop`), not `origin/HEAD`.

## Main Table Output

`--verbose` prints a headered table before the summary. Columns are padded by visible display width.

The table is sorted by:

1. Clean repos on default branches (`main`, `master`, `develop`, or a configured `defaultBranches` override) by sync priority, then default-branch priority, then repo name
2. Clean repos on non-default branches alphabetically
3. Repos with changes alphabetically

```text
Repo                  Branch                         Sync           Files
portal-app           🔥 main                        ✅ current      💾 clean
orders-api           🔥 main                        ⬇️ behind 1     💾 clean
dotfiles             🚧 feature/JBY-019-status-ui   ✅ current      📝 2 files
test-repo            🐛 bugfix/ABCD-9999-fix-bug   🔀 1/1         ⚠️ staged+dirty
local-tool           🔥 main                        ❓ no upstream  💾 clean
```

### Branch Labels

-                    🔥 = main or master branch
-                    🚧 = feature branch
-                    🐛 = bugfix branch
-                    🔧 = chore branch
-                    🚀 = release branch
-                    🌿 = develop, unknown non-default branch, or detached HEAD

Non-default branches may append a merge-into-default mark after the name:

- `✅` = tip is already merged into the default branch
- `🌱` = tip is still open (not an ancestor of default)
- (no mark) = on default, detached, or default tip not resolvable

Example verbose cell: `🚧 feature/foo 🌱`

### Sync Labels

- ✅ current
- ⬇️ behind `<count>`
- ⬆️ ahead `<count>`
- 🔀 `<ahead>/<behind>`
- ❓ no upstream

### Files Labels

The **Files** column is dirty/clean working-tree state only (not “linked git worktree”).

- 💾 clean
- 📝 `<count>` files
- ✨ staged
- ⚠️ staged+dirty

### Linked checkouts

Linked `git worktree` paths under the workspace cwd appear like any other repo, with a `🔗 ` prefix on the repo path in the verbose table and summary labels:

```text
Repo                                       Branch                          Sync         Files
app                                       🔥 main                         ✅ current   💾 clean
🔗 app/.worktrees/NDRMD-1422-asr-retry    🚧 feature/NDRMD-1422-asr-retry 🌱  ✅ current   💾 clean

🌿 Branches (1):
  🚧 feature:
    - 🔗 app/.worktrees/NDRMD-1422-asr-retry (NDRMD-1422) 🌱

🔗 Linked worktrees (1):
    - 🔗 app/.worktrees/NDRMD-1422-asr-retry (NDRMD-1422) 🌱
```

## Clean Workspace

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ✅ current  💾 clean
orders-api           🔥 main      ✅ current  💾 clean

✅ All repos clean and up-to-date
```

## Empty Workspace

When discovery finds no git repos (or every candidate was filtered out):

```text
ℹ️ No git repos found
```

## Attention (Unborn / Status Failed)

Unborn repos (`## No commits yet on …`) and repos whose `git status` output is unusable are kept in the report instead of being dropped. They never produce a false all-clean:

```text
⚠️ Attention (2):
    - broken [(unknown)] - status failed
    - unborn [main] - no commits yet
```

## File Changes

File changes are rendered as one VS Code SCM-style tree per repo. Files are not split into staged, unstaged, and untracked sections.
Each repo is a lightweight section header prefixed with `📦`, and repos are separated by one blank line.

```text
File changes
  📦 dotfiles (JBY-019)
     └─ ai/common/skills/my-workspace-status
        ├─ 🟡M render.ts
        ├─ 🔵S SAMPLE_OUTPUT.md
        ├─ 🟢A render-preview.md
        └─ 🟠MS snapshot.ts

  📦 media-service (ABCD-1234)
     └─ src
        ├─ components
        │  └─ 🔵S StatusPanel.ts
        ├─ lib
        │  ├─ 🔴D deprecated.ts
        │  └─ 🟣R old-name.ts -> new-name.ts
        └─ 🟡M index.ts
```

Multi-repo file changes keep repo sections visually separated:

```text
File changes
  📦 billing-service
     └─ 🟡M serverless-config.yml

  📦 dotfiles
     └─ ai
        ├─ codex
        │  └─ 🟡M config.toml
        └─ cursor
           └─ 🟡M hooks.json

  📦 notes
     └─ logs
        └─ 🟢A monitor-order-vbrqjnfplq.log
```

### File Badges

- 🟢A = added or untracked file
- 🟡M = unstaged modified file
- 🔵S = staged-only file
- 🟠MS = staged file modified again in the worktree
- 🔴D = deleted file
- 🟣R = renamed file
- ⚠️U = unmerged / merge conflict

---

## Example 1: All Clean and Up-to-Date

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ✅ current  💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   ✅ current  💾 clean
checkout-service     🌿 develop   ✅ current  💾 clean

✅ All repos clean and up-to-date
```

---

## Example 2: Mix of Clean Repos

Default-branch repos stay quiet in the summary. Any other branch is listed under Branches
(known prefixes get their own group; everything else is `unknown`).

```text
Repo                  Branch                    Sync           Files
portal-app           🔥 main                   ✅ current      💾 clean
orders-api           🔥 main                   ✅ current      💾 clean
workflow-api         🌿 develop                ✅ current      💾 clean
checkout-service     🌿 develop                ✅ current      💾 clean
dotfiles             🌿 some-other-branch      ✅ current      💾 clean
edge-proxy           🌿 custom-branch          ✅ current      💾 clean
local-tool           🔥 main                   ❓ no upstream  💾 clean

🌿 Branches (2):
  🌿 unknown:
    - dotfiles [some-other-branch]
    - edge-proxy [custom-branch]
```

---

## Example 3: Unstaged Changes Only

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ✅ current  💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   ✅ current  💾 clean
dotfiles             🔥 main      ✅ current  📝 2 files
my-project           🌿 develop   ✅ current  📝 1 files

File changes
  📦 dotfiles
     ├─ 🟡M README.md
     └─ ai/common/skills/my-workspace-status
        └─ 🟡M workspace-status.sh

  📦 my-project
     └─ src
        └─ 🟡M existing-file.ts
```

---

## Example 3a: Untracked Files

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ✅ current  💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   ✅ current  💾 clean
dotfiles             🔥 main      ✅ current  📝 2 files

File changes
  📦 dotfiles
     ├─ ai/common/skills/my-workspace-status
     │  ├─ 🟢A SAMPLE_OUTPUT.md
     │  └─ 🟡M workspace-status.sh
     └─ notes/tasks
        └─ 🟢A JBY-019-workspace-status-output-ui.md
```

---

## Example 4: Staged Changes Only

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ✅ current  💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   ✅ current  💾 clean
dotfiles             🔥 main      ✅ current  ✨ staged
my-project           🌿 develop   ✅ current  ✨ staged

File changes
  📦 dotfiles
     └─ ai/common/skills/my-workspace-status
        ├─ 🔵S workspace-status.sh
        └─ 🟢A new-feature.sh

  📦 my-project
     └─ src
        ├─ 🔵S helpers.ts
        └─ 🔵S utils.ts
```

---

## Example 5: Staged and Unstaged Changes

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ✅ current  💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   ✅ current  💾 clean
dotfiles             🔥 main      ✅ current  ⚠️ staged+dirty
my-project           🌿 develop   ✅ current  ⚠️ staged+dirty

File changes
  📦 dotfiles
     ├─ 🟡M README.md
     ├─ 🟡M config.json
     └─ ai/common/skills/my-workspace-status
        └─ 🔵S workspace-status.sh

  📦 my-project
     └─ src
        ├─ 🔴D deprecated.ts
        ├─ 🟡M old-file.ts
        └─ 🟢A new-component.ts
```

---

## Example 5a: Staged, Unstaged, and Untracked Files

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ✅ current  💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   ✅ current  💾 clean
dotfiles             🔥 main      ✅ current  ⚠️ staged+dirty

File changes
  📦 dotfiles
     ├─ 🟡M README.md
     └─ ai/common/skills/my-workspace-status
        ├─ 🟢A SAMPLE_OUTPUT.md
        └─ 🟠MS workspace-status.sh
```

---

## Example 5b: Nested Directory Tree and Renames

```text
File changes
  📦 media-service (ABCD-1234)
     └─ src
        ├─ components
        │  └─ 🔵S StatusPanel.ts
        ├─ lib
        │  ├─ 🔴D deprecated.ts
        │  └─ 🟣R old-name.ts -> new-name.ts
        └─ 🟡M index.ts

🌿 Branches (1):
  🚧 feature:
    - media-service (ABCD-1234)
```

---

## Example 6: Sync Status - Behind Remote

```text
Repo                  Branch       Sync        Files
portal-app           🔥 main      ⬇️ behind 1  💾 clean
orders-api           🔥 main      ✅ current   💾 clean
workflow-api         🌿 develop   ⬇️ behind 1  💾 clean
checkout-service     🌿 develop   ✅ current   💾 clean

🔄 Sync status (2):
  ⬇️ behind:
    - portal-app [main] - behind by 1 commits
    - workflow-api [develop] - behind by 1 commits
```

---

## Example 7: Sync Status - Ahead of Remote

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      ⬆️ ahead 1  💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   ⬆️ ahead 1  💾 clean
checkout-service     🌿 develop   ✅ current  💾 clean

🔄 Sync status (2):
  ⬆️ ahead:
    - portal-app [main] - ahead by 1 commits
    - workflow-api [develop] - ahead by 1 commits
```

---

## Example 8: Sync Status - Diverged

```text
Repo                  Branch       Sync       Files
portal-app           🔥 main      🔀 1/1      💾 clean
orders-api           🔥 main      ✅ current  💾 clean
workflow-api         🌿 develop   🔀 1/1      💾 clean
checkout-service     🌿 develop   ✅ current  💾 clean

🔄 Sync status (2):
  🔀 diverged:
    - portal-app [main] - diverged (ahead 1, behind 1)
    - workflow-api [develop] - diverged (ahead 1, behind 1)
```

---

## Example 9: Non-Default Branches (feature, bugfix, chore, release, unknown)

```text
Repo                  Branch                                  Sync       Files
portal-app           🔥 main                                 ✅ current  💾 clean
workflow-api         🌿 develop                              ✅ current  💾 clean
checkout-service     🚧 feature/ABCD-1234-add-new-feature   ✅ current  💾 clean
dotfiles             🚧 feature/ABCD-5678-improve-script    ✅ current  💾 clean
my-project           🐛 bugfix/ABCD-9999-fix-critical-bug   ✅ current  💾 clean
orders-api           🔧 chore/container-tags-script          ✅ current  💾 clean
billing-service      🚀 release/2026-07-20_CR2026053300      ✅ current  💾 clean
edge-proxy           🌿 hotfix/urgent                        ✅ current  💾 clean

🌿 Branches (5):
  🚧 feature:
    - dotfiles (ABCD-5678)
    - checkout-service (ABCD-1234)
  🐛 bugfix:
    - my-project (ABCD-9999)
  🔧 chore:
    - orders-api [chore/container-tags-script]
  🚀 release:
    - billing-service [release/2026-07-20_CR2026053300]
  🌿 unknown:
    - edge-proxy [hotfix/urgent]
```

---

## Example 10: Complete Example - All States Combined

```text
Repo                    Branch                            Sync        Files
alpha-clean-main     🔥 main                           ✅ current   💾 clean
gamma-clean-develop  🌿 develop                        ✅ current   💾 clean
beta-behind-main     🔥 main                           ⬇️ behind 1  💾 clean
delta-ahead-develop  🌿 develop                        ⬆️ ahead 1   💾 clean
zeta-diverged-main   🔥 main                           🔀 1/1       💾 clean
eta-chore-clean      🔧 chore/release-hygiene          ✅ current   💾 clean
epsilon-default-dirty🔥 main                           ✅ current   📝 1 files
iota-bugfix-both     🐛 bugfix/ABCD-7002-bug          ✅ current   ⚠️ staged+dirty
theta-feature-staged 🚧 feature/ABCD-7001-feature     ✅ current   ✨ staged

File changes
  📦 epsilon-default-dirty
     └─ 🟡M README.md

  📦 iota-bugfix-both (ABCD-7002)
     ├─ 🔴D delete-me.txt
     └─ 🟠MS README.md

  📦 theta-feature-staged (ABCD-7001)
     └─ 🟢A feature.txt

🔄 Sync status (3):
  ⬇️ behind:
    - beta-behind-main [main] - behind by 1 commits
  ⬆️ ahead:
    - delta-ahead-develop [develop] - ahead by 1 commits
  🔀 diverged:
    - zeta-diverged-main [main] - diverged (ahead 1, behind 1)

🌿 Branches (3):
  🚧 feature:
    - theta-feature-staged (ABCD-7001)
  🐛 bugfix:
    - iota-bugfix-both (ABCD-7002)
  🔧 chore:
    - eta-chore-clean [chore/release-hygiene]
```

---

## Example 11: With --fetch Flag

When `--fetch` discovers a remote change, fetch progress appears before the refreshed table.

```text
🔄 Fetching from remotes (this may take a moment)...
Repo                  Branch       Sync        Files
fetch-repo           🔥 main      ⬇️ behind 1  💾 clean

🔄 Sync status (1):
  ⬇️ behind:
    - fetch-repo [main] - behind by 1 commits
```

---

## Example 12: Pull Behind Repos

```text
⬇️ Pulling repos that are behind...
  Pulling pull-repo...
    ✅ Success
🔄 Re-checking status after pull...
✅ All repos clean and up-to-date
```

Dirty behind repos are auto-stashed around pull (local edits reapplied after):

```text
⬇️ Pulling repos that are behind...
  Pulling dirty-pull-repo...
    ✅ Success (stashed local changes, reapplied)
🔄 Re-checking status after pull...
File changes
  📦 dirty-pull-repo
     └─ 🟡M local.txt
```

---

## Example 13: Switch Clean Task Branches to Default Branch

```text
🔄 Switching to default branch and pulling...
  🔄 switch-feature: Switching from feature/ABCD-5001-switch to main
  🔄 switch-bugfix: Switching from bugfix/ABCD-5002-switch to develop
  ⚠️ stay-dirty (chore/ABCD-5003-dirty): Has uncommitted changes, skipping
🔄 Re-checking status after switch...
File changes
  📦 stay-dirty (ABCD-5003)
     └─ 🟡M README.md

🌿 Branches (1):
  🔧 chore:
    - stay-dirty (ABCD-5003)
```

## Example 14: Release and Unknown Branches

```text
Repo                  Branch                            Sync       Files
billing-service      🚀 release/2026-07-20_CR2026053300 ✅ current  💾 clean
edge-proxy           🌿 hotfix/urgent                   ✅ current  💾 clean

🌿 Branches (2):
  🚀 release:
    - billing-service [release/2026-07-20_CR2026053300]
  🌿 unknown:
    - edge-proxy [hotfix/urgent]
```

## Notes

- Repos are sorted as described in Main Table Output.
- Ticket IDs such as `ABCD-1234` are extracted from branch names and shown in repo labels.
- Summary sections are omitted when empty.
- Detached HEAD repos are shown in the verbose table as `HEAD (detached)`.
- File trees collapse single-child directory chains, such as `ai/common/skills/my-workspace-status`.
- Empty discovery prints `ℹ️ No git repos found` (not all-clean).
- Unborn and status-failed repos appear under `⚠️ Attention` and block the all-clean message.
