# Git operations

Every git subprocess the tool runs. `<git>` is `git_binary()` (`WORKSPACE_STATUS_GIT`, else `/usr/bin/git` when it exists, else `git`).

All wrappers in this file attach stdin to `/dev/null` and set `GIT_TERMINAL_PROMPT=0`. That keeps git from inheriting the TUI's raw-mode TTY (a credential prompt would otherwise deadlock: the parent waits on `output()`, the child waits on stdin). `merge_into_head` also sets `GIT_EDITOR=true` and `GIT_MERGE_AUTOEDIT=no`.

## `crates/workspace-status/src/git.rs`

| Function | Command | Returns | Purpose |
| --- | --- | --- | --- |
| `exec_git(args, cwd)` | `<git> <args>` | trimmed stdout, `""` on any failure | Generic read. Swallows errors by design — callers treat empty as "unknown". |
| `exec_git_status(args, cwd)` | `<git> <args>` | exit code, `-1` on throw | Generic write / predicate. |
| `exec_git_checked(args, cwd)` | `<git> <args>` | `Result<(), String>` | Surfaces failure to the caller. |
| `repo_has_local_changes(cwd)` | `diff --quiet`, then `diff --cached --quiet` | boolean | True when either exits non-zero. Untracked files are **not** counted. |
| `rev_parse_quiet(ref, cwd)` | `rev-parse --verify --quiet <ref>` | SHA string, or `None` when missing | Graph checkout SHA compare (`refs/heads/<local>` vs `refs/remotes/origin/<local>`) |
| `checkout_branch(branch, cwd)` | `checkout <branch> --quiet`, falling back to `checkout -b <branch> origin/<branch> --quiet` | boolean | Second form creates a local tracking branch when the branch only exists on the remote. |
| `fast_forward_to_remote_ref(remote_ref, cwd)` | `merge --ff-only --quiet` of `origin/foo` or `refs/remotes/origin/foo` (no fetch) | boolean | Graph confirm Yes: advance HEAD to the **selected** remote-tracking tip. Ahead/diverged/missing → false; HEAD unchanged. No reset. |
| `list_local_branches(cwd)` | `for-each-ref` on `refs/heads/` | `LocalBranch[]` | Local branches only (no remotes). |
| `pull_quiet_detailed(cwd)` | when dirty: `stash push -m …` → `pull --quiet` → `stash pop`; else `pull --quiet` | `PullQuietResult` | Auto-stash tracked local changes around pull; pop always runs after pull |
| `pull_quiet(cwd)` | delegates to `pull_quiet_detailed` | boolean (`result.ok`) | |
| `push_quiet(cwd)` | `push --quiet`, or `push -u <remote> HEAD --quiet` when no/wrong upstream | `Result` | TUI `P`. No force, no auto-stash; first publish uses `-u`; diverged remotes may fail |
| `FULL_DIFF_CONTEXT_LINES` | — | `999_999` | Large enough `-U` value to keep a typical source file in one hunk. |
| `git_diff_args(base, path, context)` | inserts `-U<n>` and `-- <path>` | argv | Shared builder for worktree / cached / commit / stash diffs. |
| `stage_file` / `unstage_file` | `add -- <path>` / `restore --staged -- <path>` | `Result` | TUI `s` / `u` |
| `revert_tracked_file` / `remove_untracked_file` | `restore -- <path>` / `clean -f -- <path>` | `Result` | TUI `x`. **Destructive.** |
| `list_worktrees_porcelain(cwd)` | `worktree list --porcelain` | stdout (or `""`) | Enumerate checkouts for linked-worktree discovery |
| `is_ancestor(cwd, maybe_ancestor, tip)` | `merge-base --is-ancestor` | `Some(true/false)` / `None` | Merge-into-default probe |
| `resolve_default_branch_tip_ref` / `resolve_default_branch_name` / `get_default_branch` | `rev-parse` / `symbolic-ref` / `show-ref` | branch / tip | Default branch name and tip for classification and `-d` |
| `create_branch_at(cwd, name, commit_id)` | `branch -- <name> <commitId>` | `Result` | Create a local ref **without** checking it out (graph `c`) |
| `create_branch_checkout(cwd, name)` | `checkout -b <name> --quiet` | `Result` | Picker `C` |
| `stash_push` / `stash_apply` / `stash_pop` / `stash_drop` | `stash push -u` / `apply` / `pop` / `drop` | `Result` | Stash menu and graph stash rows. Unchanged stash list after push is failure |
| `list_stash_refs` / `latest_stash_ref` | `stash list --format=%gd` | refs | Latest stash for graph `S` apply / pop on a non-stash row |
| `remove_worktree(primary, path, force)` | `worktree remove [--force] <path>` from primary | `Result` | Remove a linked worktree after TUI confirm (`W`) |
| `list_commit_name_status` | `diff-tree --name-status -r <commit>^ <commit>`; empty → `--root` | `NameStatus[]` | First-parent file list (merges); `--root` for root commits |
| `list_worktree_name_status` | `diff HEAD --name-status` + untracked | `NameStatus[]` | Worktree + index + untracked |
| `list_stash_name_status` | `stash show --name-status <ref>` | `NameStatus[]` | Files in a stash entry |
| `diff_commit_file` / `_ctx` | `diff <commit>^ <commit> -- <path>`; empty → `show --first-parent` | unified diff lines | First-parent per-file diff |
| `diff_stash_file` / `_ctx` | `diff <stash>^1 <stash> -- <path>` | unified diff lines | Per-file stash diff |
| `origin_out_of_sync` | compare `rev-parse` of local vs `origin/<branch>` | `Option<origin/…>` | Helper for graph checkout confirm |
| `merge_into_head` | `merge --ff-only --quiet -- <rev>`, else `merge --no-ff --no-edit --quiet -- <rev>` | `MergeIntoHeadResult` | Graph `m` confirm Yes: fast-forward HEAD when possible, otherwise a merge commit. No rebase. Conflicts leave `MERGE_HEAD` (no abort, no continue). Tags are passed as the commit id |

Every wrapper that takes a path puts `--` before it, so a file named `-f` or `HEAD` cannot be read as an option or a revision.

## `crates/workspace-status/src/discovery.rs`

| Call | Command |
| --- | --- |
| `expand_repos_with_linked_worktrees` | `worktree list --porcelain` per main checkout |
| `process_repo` (when `do_fetch`) | `fetch --quiet` — failure is caught and ignored; stale refs are better than no output |
| `process_repo` | `status --porcelain=v1 --branch --ahead-behind --untracked-files=all` |
| `process_repo` (merge probe) | `resolve_default_branch_name` + `resolve_default_branch_tip_ref` + `merge-base --is-ancestor HEAD <tip>` |

After `find_repos_with_config` (primaries; still skips dot-dirs), discovery lists linked worktrees under the workspace cwd, applies the same ignore / named-filter rules (filter on a primary includes its linked children; filter on a linked path includes only that path), dedupes by path (linked metadata wins), and runs `process_repo` with `checkout_kind` / `primary_repo`.

One status call per repo produces branch, upstream, ahead/behind counts, and all three file buckets. `--untracked-files=all` lists files inside untracked directories rather than collapsing to `dir/`, which the tree view needs.

Unborn repos (`## No commits yet on <branch>`) become a normal snapshot with `sync_note: no commits yet`. When status stdout is empty or the branch header cannot be parsed, `process_repo` returns a failure snapshot (`sync_note: status failed`, `merged_into_default: None`) instead of dropping the repo — so the plain report cannot claim all-clean by omission.

## `crates/workspace-status/src/actions.rs`

CLI `-p` / `-d` (progress strings go to the caller; `--json` sends them to stderr).

| Function | Purpose |
| --- | --- |
| `pull_behind_repos` | `pull_quiet_detailed` per behind repo. Logs success / stash-pop conflict / failure. |
| `switch_repo_to_default_branch` | Fetch, checkout default, pull when the remote tip differs. Skips dirty repos. |

## TUI writes (`tui/ops.rs`, `tui/fetch.rs`, `tui/app.rs`)

| Function | Purpose |
| --- | --- |
| `collect_write_files` | File nodes under the focused row: `[file]` / dir subtree / checkout files / flat-repo files; empty for family containers, workspace, and group. |
| `op_targets` | Checkout paths for `f` / `p` / `d`. Workspace and family rows yield primary checkouts only. Group is empty. A linked worktree is included only when that row is focused. Hidden ignored repos are omitted. |
| `push_targets` | Same primary / focused-worktree rule for `P`. Never on workspace. |
| `background_fetch_targets` | Snapshot paths for the TUI background fetch timer. Hidden ignored checkouts are omitted. When ignored repos are shown, every snapshot path is included, including linked worktrees. Manual `f` stays on `op_targets`. |
| `refresh_target` | Workspace / No-updates → whole snapshot; otherwise the focused checkout path. |

After `p` / `P` / `d` / `f`, the TUI refreshes the affected repos and stamps those `repo:<path>` and `checkout:<path>` ids into the flash map.

## Graph load (`tui/graph_load.rs`)

Default window is 300 (`DEFAULT_GRAPH_WINDOW`). `--exclude=refs/stash` precedes `--all`.

| Function | Command | Purpose |
| --- | --- | --- |
| `load_graph_model_window` | `log --exclude=refs/stash --all --topo-order --date-order --skip --max-count --pretty=%H%x00%P%x00%s%x00%an%x00%at` | One history page. Always sets the working-tree row (`Some(has_changes)`). |
| extra `stash^1` | `log --no-walk --ignore-missing --pretty=…` | Missing stash parents appended after the log prefix so autoload skip uses `window`, not `commits.len()` |
| `should_autoload` / `merge_autoload` | next page at `skip + window_count` | Cursor on last loaded row; skip stays at the original window start; `window` grows |

Hidden ignored checkouts stay out of `P` / `S` / `b` unless shown. Linked worktrees are included on `f` / `p` / `P` / `d` only when that row is focused. The background fetch timer (`background_fetch_targets` in `tui/fetch.rs`) includes every snapshot except hidden ignored — linked worktrees and shown ignored repos included. See [tui-rust.md](./tui-rust.md).

Manual `f` / `p` / `P` / `d` and the background fetch tick paint a trailing breadcrumb counter (`Fetching n/N…`, `Pulling n/N…`, `Pushing n/N…`, `Switching n/N…`) and redraw after each repo settles. When the op finishes, that slot is a count (`Fetched N repos`, `Pulled N repos`, `Pushed N repos`, `Switched N repos`), with ` (N failed)` if any failed — never a list of names. The hint row stays pills + keys. Graph autoload still uses `loading older…`. Those git children (and watch / full-snapshot reload) run on a worker thread so resize and quit still reach the event loop; overlay modes do not start the watch or fetch timers.

## Non-obvious semantics

**Renames need both paths.** Staging only the new path leaves the deletion of the old path unstaged, and git then reports the pair as `D` + `A` rather than `R`. Writes apply to each path in order and stop at the first failure.

**Bulk stage / unstage.** `s` / `u` use `collect_write_files`: a file row is itself; a dir or checkout (or flat repo) walks file/dir descendants — never mixes sibling checkouts under a family container. Workspace, group, and family-container rows yield an empty list. Stage keeps files with unstaged or untracked; unstage keeps staged. Empty after filtering: `Nothing to stage` / `Nothing to unstage`. Wrong focus: `Focus a file, dir, checkout, or repo to stage|unstage`.

**Focused refresh (`r`).** Reloads the whole workspace on the workspace row or No-updates group, and otherwise one checkout (`refresh_target` → `ReloadSnapshot` vs `ReloadRepo { repo }`).

**Bulk revert with counted confirm.** `x` uses the same `collect_write_files` scope, keeping unstaged or untracked (staged-only skipped). Confirm shows counts; `y`/`Enter` runs `git restore` on tracked targets and **keeps** untracked; `Y` also deletes each untracked via `remove_untracked_file` (per-file `clean -f`, not `clean -fd`). Exception: a single untracked target still deletes on both `y` and `Y`. Empty after filter: `Nothing to discard` (or `Nothing to discard (staged only)` on a staged-only file).

**Remove linked worktree (`W`).** Linked `Checkout` rows only. Confirm shows branch, `merged into default` / `NOT merged into default`, and `--force` when dirty. On Unix, bind-mount aliases remap via inode so gitdir back-pointers match. On Windows, worktree identity is canonical path plus size and mtime (no inode / bind-mount remap).

**Reverting an untracked file deletes it.** There is no git object to restore to, so untracked “revert” means remove from disk — irrecoverable. Bulk `y` leaves untracked alone; opt in with `Y`, or press `y` when the only target is one untracked file.

**Staged-only files refuse revert.** When a file is staged with no unstaged component, the worktree already matches the index for that path, so `git restore` would be a no-op. Discarding a staged change is a two-step operation (`u` then `x`) by design.

**`repo_has_local_changes` ignores untracked files.** `-d` will therefore switch a branch in a repo that has untracked files. That is usually right (untracked files survive a checkout) but it is not what "has local changes" implies.

**`stash_push` treats a no-op as failure.** Apple Git 2.50 prints `No local changes to save` but exits 0. The wrapper compares `stash list` before and after and returns `Err` when the list is unchanged.

**Local branch picker (`b`).** Opens on a checkout or flat repo row (hidden on family containers), lists `refs/heads/` only (no remotes). Esc closes without quitting. Typing filters; `j`/`k` move; Enter checks out. Dirty worktrees refuse checkout with `Dirty worktree — commit or stash first`. Selecting the current branch closes with `Already on …` and skips the dirty check.

**Graph actions** (graph list focused — depth 0 right or depth 1 left):

| Key | When visible | Behaviour |
| --- | --- | --- |
| `b` | Commit row with ≥1 local branch or `origin/*` ref | Dirty check first. One name → checkout (creates tracking from origin when local is missing). Several names → picker (locals then `origin/*`). Selecting `origin/<name>` when a local exists and tips differ opens confirm: Yes checks out the local then `fast_forward_to_remote_ref` of that selected `origin/<name>` (no fetch; `merge --ff-only`). Tags and non-`origin` remotes are not targets. |
| `c` | Any commit row | Name prompt → `create_branch_at` (ref only, HEAD unchanged). |
| `m` | Any commit row | Boxed confirm, then merge that ref into the checkout's current HEAD. Local / `origin/*` names when present; tags and unlabeled commits use the commit id. `git merge --ff-only`, else `git merge --no-ff --no-edit` (no rebase). Dirty tracked worktree refuses (`Dirty worktree — commit or stash first`) before the overlay. Conflicts stay uncommitted (no abort, no continue). Linked worktrees only when that checkout row is focused. |
| `S` | Uncommitted, stash, or commit with stash/dirty ops | Stash overlay (`stash_push -u` / apply / pop / drop as listed). |
| `a` / `p` / `D` | Stash row | Apply / pop / drop (drop confirms with `y`/`n`/Esc). |

## Destructive operations

| Operation | Confirmation | Recoverable |
| --- | --- | --- |
| `s` stage / `u` unstage | none | yes — trivially reversible |
| `x` revert, tracked (`y`) | `y`/`Y`/`n` prompt | only via git's object store if the change was ever committed or stashed |
| `x` revert + delete untracked (`Y`) | same prompt | **no** for deleted untracked |
| `x` single untracked (`y` or `Y`) | same prompt | **no** — the file is deleted |
| `-p` / `--pull` | none | yes — but can fail on conflicts |
| `-d` / `--default-branch` | none | yes — dirty repos are skipped, so no work is lost |
| `b` checkout (local / origin) | none when in sync; `y`/`n` when local exists and origin tips differ | yes — dirty worktrees refuse before checkout; confirm Yes is checkout then `fast_forward_to_remote_ref` of the selected `origin/*` (no reset) |
| `m` graph merge into HEAD | `y`/`n` boxed confirm | yes — dirty tracked worktrees refuse before confirm; conflicts stay uncommitted (no abort) |

Revert, stash drop, origin-out-of-sync graph checkout, and graph merge use modal overlays, so no other key can act while one is up.

## Write serialisation

A single busy flag is shared by refresh, every write (including TUI `p` / `d`), background/manual fetch, and the watch poll. Concurrent git writes against the same index would race on `.git/index.lock`; the gate makes the second one report `Busy…` instead. A poll tick that lands while a write or fetch is in flight is dropped, not queued.
