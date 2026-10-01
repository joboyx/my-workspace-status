# Git operations

Every git subprocess the tool runs. `<git>` is `git_binary()` (`WORKSPACE_STATUS_GIT`, else `/usr/bin/git` when it exists, else `git`).

Most wrappers in this file attach stdin to `/dev/null` and set `GIT_TERMINAL_PROMPT=0`. That keeps git from inheriting the TUI's raw-mode TTY (a credential prompt would otherwise deadlock: the parent waits on `output()`, the child waits on stdin). `apply_cached_patch` and `apply_worktree_patch_reverse` are the exceptions: they write the unified patch to git's stdin (`git apply --cached` / `git apply --reverse --cached` / `git apply --reverse`). `merge_into_head` also sets `GIT_EDITOR=true` and `GIT_MERGE_AUTOEDIT=no`.

Every git subprocess also runs with `GIT_OPTIONAL_LOCKS=0` (set by `git::git_process`, the shared constructor `git_command` and `run_with_stdin` build on). `ws` polls every repo's status on a timer, and plain `git status` can take `.git/index.lock` to write a refreshed index; with several `ws` instances running, or a user/agent running `git add` / `commit` / `checkout` at the same time, that collides and fails with `Unable to create '.git/index.lock': File exists`. `GIT_OPTIONAL_LOCKS=0` tells git to skip that optional lock (see git-status(1), "BACKGROUND REFRESH"). It only affects optional locks — write commands still take the locks they need — so it is safe on every spawn, read or write.

## `crates/workspace-status/src/git.rs`

| Function | Command | Returns | Purpose |
| --- | --- | --- | --- |
| `exec_git(args, cwd)` | `<git> <args>` | trimmed stdout, `""` on any failure | Generic read. Swallows errors by design — callers treat empty as "unknown". |
| `exec_git_stdout(args, cwd)` | `<git> <args>` | `Result<String, String>` | Compare reads, and the worktree file diff (`diff [--cached]` / `diff HEAD -- <path>`, `tui/diff.rs` `git_diff_text`). Empty stdout is success. Failure is `Err`; the diff pane paints `git diff failed: <reason>`. |
| `rev_parse_commit` | `rev-parse --verify --quiet <ref>^{commit}` | `Result<Option<SHA>>` | Missing ref is `Ok(None)`. Other failures are `Err`. |
| `merge_base` | `merge-base <a> <b>` | `Result<Option<SHA>>` | Unrelated histories are `Ok(None)`. |
| `list_compare_name_status` | `diff --name-status --find-renames <base>...<head> --` | `Result<NameStatus[]>` | Committed three-dot file list. |
| `diff_compare_file_ctx` | `diff <base>...<head> -- <path>` | `Result<lines>` | One compare path. Empty stdout is `(no diff)`. |
| `list_compare_picker_branches` | `for-each-ref` on `refs/heads/` + `refs/remotes/origin/` | `Result<LocalBranch[]>` | Drops `origin/HEAD` and the current local. No checkout. |
| `exec_git_status(args, cwd)` | `<git> <args>` | exit code, `-1` on throw | Generic write / predicate. |
| `exec_git_checked(args, cwd)` | `<git> <args>` | `Result<(), String>` | Surfaces failure to the caller. `Err` is git's reason line (`git_reason_line`: a push `! [rejected]` line, else the first `fatal:` / `error:` stderr line without its tag, else the first non-`hint:` stderr line, else a stdout `CONFLICT` line), or `git <sub> exited with code N` when git printed none. TUI writes show it as `<op> failed: <reason>` (`stash pop failed: …`). `exec_git_stdout` and the patch-on-stdin wrappers return the same one line. |
| `repo_has_local_changes(cwd)` | `diff --quiet`, then `diff --cached --quiet` | boolean | True when either exits non-zero. Untracked files are **not** counted. |
| `rev_parse_quiet(ref, cwd)` | `rev-parse --verify --quiet <ref>` | SHA string, or `None` when missing | Graph checkout SHA compare (`refs/heads/<local>` vs `refs/remotes/origin/<local>`) |
| `checkout_branch(branch, cwd)` | `checkout <branch> --quiet`, falling back to `checkout -b <branch> origin/<branch> --quiet` | boolean | Second form creates a local tracking branch when the branch only exists on the remote. |
| `fast_forward_to_remote_ref(remote_ref, cwd)` | `merge --ff-only --quiet` of `origin/foo` or `refs/remotes/origin/foo` (no fetch) | boolean | Graph confirm Yes: advance HEAD to the **selected** remote-tracking tip. Ahead/diverged/missing → false; HEAD unchanged. No reset. |
| `list_local_branches(cwd)` | `for-each-ref` on `refs/heads/` | `LocalBranch[]` | Local branches only (no remotes). |
| `pull_quiet_detailed(cwd)` | when dirty: `stash push -m …` → `pull --quiet` → `stash pop`; else `pull --quiet` | `PullQuietResult` | Auto-stash tracked local changes around pull; pop always runs after pull. `stash_pop_failed` when that pop conflicted (stash kept); `error` holds git's reason when the stash push or pull failed |
| `pull_quiet(cwd)` | delegates to `pull_quiet_detailed` | boolean (`result.ok`) | |
| `push_quiet(cwd)` | `push --quiet`, or `push -u <remote> HEAD --quiet` when no/wrong upstream | `Result` | TUI `P`. No force, no auto-stash; first publish uses `-u`; diverged remotes may fail |
| `FULL_DIFF_CONTEXT_LINES` | — | `999_999` | Large enough `-U` value to keep a typical source file in one hunk. |
| `git_diff_args(base, path, context)` | inserts `-U<n>` and `-- <path>` | argv | Shared builder for worktree / cached / commit / stash diffs. |
| `stage_file` / `unstage_file` | `add -- <path>` / `restore --staged -- <path>` | `Result` | TUI `s` / `u` whole file |
| `apply_cached_patch` | `apply --cached --unidiff-zero --whitespace=nowarn [-R] -` with the patch on stdin | `Result` | DiffVisual `s` / `u` range. Empty patch is `Err`. |
| `apply_worktree_patch_reverse` | `apply --reverse --unidiff-zero --whitespace=nowarn -` with the patch on stdin (no `--cached`) | `Result` | DiffVisual `x` range. Changes the worktree only; the index stays. Empty patch is `Err`. **Destructive.** |
| `revert_tracked_file` / `remove_untracked_file` | `restore -- <path>` / `clean -f -- <path>` | `Result` | TUI `x`. **Destructive.** |
| `ensure_compare_head_clean(cwd, head, paths)` | `rev-parse --verify --quiet HEAD^{commit}`, then `status --porcelain=v1 --untracked-files=all --ignored -- <paths>` | `Result` | Compare `x` guard, run on the write worker right before the write. `Err` (`revert aborted: HEAD moved` / `revert aborted: <path> has uncommitted changes`) unless HEAD is still the compare head and `git status` lists none of `paths`. Every refusal starts with `COMPARE_REVERT_ABORTED`; the TUI shows it as a warn, without the `revert failed:` prefix. |
| `restore_worktree_from(cwd, rev, paths)` | `restore --source=<rev> --worktree -- <paths>` | `Result` | Compare `x` whole file. Worktree only; the index stays. A tracked path that `<rev>` lacks is removed. **Destructive.** |
| `revert_compare_file` / `revert_compare_patch` | `ensure_compare_head_clean`, then `restore_worktree_from(<merge-base>)` / `apply_worktree_patch_reverse` | `Result` | Compare `x` on a file / on highlighted lines. **Destructive.** |
| `list_worktrees_porcelain(cwd)` | `worktree list --porcelain` | stdout (or `""`) | Enumerate checkouts for linked-worktree discovery |
| `is_ancestor(cwd, maybe_ancestor, tip)` | `merge-base --is-ancestor` | `Some(true/false)` / `None` | Merge-into-default probe |
| `head_equals_ref(cwd, git_ref)` | `rev-parse` of `HEAD` and `git_ref` | boolean | Same-commit as default tip is open, not merged |
| `resolve_default_branch_tip_ref` / `resolve_default_branch_name` / `get_default_branch` | `rev-parse` / `symbolic-ref` / `show-ref` | branch / tip | Default branch name and tip for classification and `-d` |
| `create_branch_at(cwd, name, commit_id)` | `branch -- <name> <commitId>` | `Result` | Create a local ref **without** checking it out (graph `c`, and the create row of the graph `b` picker, which hides for any existing local branch from the snapshot) |
| `create_branch_checkout(cwd, name)` | `checkout -b <name> --quiet` | `Result` | The `+ create branch <name>` row of the tree `b` picker. Both create paths check the name first with `branches::branch_name_error` (git `check-ref-format` rules); a bad name shows the reason and git does not run |
| `stash_push` / `stash_apply` / `stash_pop` / `stash_drop` | `stash push -u` / `apply` / `pop` / `drop` | `Result` | Stash menu and graph stash rows. Unchanged stash list after push is failure |
| `list_stash_refs` / `latest_stash_ref` | `stash list --format=%gd` | refs | Latest stash for graph `S` apply / pop on a non-stash row |
| `remove_worktree(primary, path, force)` | `worktree remove [--force] <path>` from primary | `Result` | Remove a linked worktree after TUI confirm (`W`) |
| `list_commit_name_status` | `diff-tree --name-status -r <commit>^ <commit>`; empty → `--root` | `NameStatus[]` | First-parent file list (merges); `--root` for root commits |
| `list_worktree_name_status` | `diff HEAD --name-status` + untracked | `NameStatus[]` | Worktree + index + untracked |
| `list_stash_name_status` | `stash show --name-status <ref>` | `NameStatus[]` | Files in a stash entry |
| `diff_commit_file` / `_ctx` | `diff <commit>^ <commit> -- <path>`; empty → `show --first-parent` | unified diff lines | First-parent per-file diff |
| `diff_stash_file` / `_ctx` | `diff <stash>^1 <stash> -- <path>` | unified diff lines | Per-file stash diff |
| `origin_out_of_sync` | compare `rev-parse` of local vs `origin/<branch>` | `Option<origin/…>` | Helper for graph checkout confirm |
| `ahead_behind(left, right, cwd)` | `rev-list --left-right --count <left>...<right>` | `Option<(ahead, behind)>` | Counts for the out-of-sync checkout confirm and the reason a fast-forward failed |
| `merge_into_head` | `merge --ff-only --quiet -- <rev>`, else `merge --no-ff --no-edit --quiet -- <rev>` | `MergeIntoHeadResult` | Graph `m` confirm Yes: fast-forward HEAD when possible, otherwise a merge commit. No rebase. Conflicts leave `MERGE_HEAD` (no abort, no continue). Tags are passed as the commit id |

Every wrapper that takes a path puts `--` before it, so a file named `-f` or `HEAD` cannot be read as an option or a revision.

## `crates/workspace-status/src/discovery.rs`

| Call | Command |
| --- | --- |
| `expand_repos_with_linked_worktrees` | `worktree list --porcelain` per main checkout |
| `process_repo` (when `do_fetch`) | `fetch --quiet` — failure is caught and ignored; stale refs are better than no output |
| `process_repo` | `status --porcelain=v1 --branch --ahead-behind --untracked-files=all` |
| `process_repo` (merge probe) | `resolve_default_branch_name` + `resolve_default_branch_tip_ref` + `merge-base --is-ancestor HEAD <tip>` + same-commit SHA compare. Same-commit as the default tip is open, not merged. |

After `find_repos_with_config` (primaries; still skips dot-dirs), discovery lists linked worktrees under the workspace cwd, applies the same ignore / named-filter rules (filter on a primary includes its linked children; filter on a linked path includes only that path), dedupes by path (linked metadata wins), and runs `process_repo` with `checkout_kind` / `primary_repo`. Independent checkouts run with a cap of `FETCH_CONCURRENCY` (10; `WS_STATUS_FETCH_CONCURRENCY`) so a live watch tick is not one-repo-at-a-time. There is no inotify.

One status call per repo produces branch, upstream, ahead/behind counts, and all three file buckets. `--untracked-files=all` lists files inside untracked directories rather than collapsing to `dir/`, which the tree view needs.

Unborn repos (`## No commits yet on <branch>`) become a normal snapshot with `sync_note: no commits yet`. When status stdout is empty or the branch header cannot be parsed, `process_repo` returns a failure snapshot (`sync_note: status failed`, `merged_into_default: None`) instead of dropping the repo — so the plain report cannot claim all-clean by omission.

## `crates/workspace-status/src/parallel.rs`

Bounded map (`map_with_concurrency` / `CappedBatch`) used by CLI `collect_snapshots` and tests. Cap is `FETCH_CONCURRENCY` (10; `WS_STATUS_FETCH_CONCURRENCY`). Completions are counted as jobs **finish**. The live TTY path is Scheduler JoinSet `spawn_blocking` in `tui/effect.rs`. Exclusive checkout writes stay serial there.

## `crates/workspace-status/src/actions.rs`

CLI `-p` / `-d` (progress strings go to the caller; `--json` sends them to stderr).

| Function | Purpose |
| --- | --- |
| `pull_behind_repos` | `pull_quiet_detailed` per behind repo. Logs success / stash-pop conflict / failure. |
| `switch_repo_to_default_branch` | Fetch, checkout default, pull when the remote tip differs. Skips dirty repos. |

## Compare reads (`tui/app.rs` `compute_compare_range` / `compute_compare_diff`)

Default tip is `resolve_default_branch_name` then `resolve_default_branch_tip_ref` (`origin/<default>` before local). Then:

```
git rev-parse --verify --quiet HEAD^{commit}
git rev-parse --verify --quiet <base-ref>^{commit}
git merge-base <base-sha> <head-sha>
git diff --name-status --find-renames <base-sha>...<head-sha> --
git diff <base-sha>...<head-sha> -- <path>
```

`E` on a compare file uses LEFT `<merge-base>:<old-path-or-path>` and RIGHT `<head>:<path>`. Compare never changes HEAD or the index. The only compare write is `x` (below), which changes the worktree. The picker never checkouts, creates, or fetches. A watch probe reloads only after both SHAs are recorded and HEAD or the base-tip SHA then changes.

## TUI writes (`tui/ops.rs`, `tui/fetch.rs`, `tui/effect.rs`)

| Function | Purpose |
| --- | --- |
| `collect_write_files` | File nodes under the focused row: `[file]` / dir subtree (Changes dirs use `#unstaged`; Staged dirs are unsuffixed; no section chrome keeps every dirty file under the dir) / section header (checkout files on that side) / checkout files / flat-repo files; empty for family containers, workspace, and group. |
| `op_targets` | Checkout paths for `f` / `p` / `d`. Workspace and family rows yield primary checkouts only. Group is empty (`op_is_kind_noop`). A linked worktree is included only when that row is focused. Hidden ignored repos are omitted. |
| `push_targets` | Focused visible repo or checkout for `P`. Never on workspace, group, file, dir, or section. |
| `background_fetch_targets` | Snapshot paths for the TUI background fetch timer. Hidden ignored checkouts are omitted. When ignored repos are shown, every snapshot path is included, including linked worktrees. Manual `f` stays on `op_targets`. |
| `refresh_target` | Workspace / No-updates → whole snapshot; otherwise the focused checkout path. |

After `p` / `P` / `d` / `f`, the TUI refreshes the affected repos and stamps those `repo:<path>` and `checkout:<path>` ids into the flash map. TTY local writes (`s` / `u` / `x` / stash / checkout / create-branch / merge / remove-worktree, including DiffVisual range `git apply --cached` / `git apply --reverse`) run on `spawn_blocking` in `tui/effect.rs`. Error paths still enqueue snapshot + pane so leftover keys cannot flush. Independent per-repo `f` / `p` / `P` (and `FetchTick`) share the per-gitdir remote queue. Cap is `FETCH_CONCURRENCY` (10).

## Graph load (`tui/graph_load.rs`)

Default window is 300 (`DEFAULT_GRAPH_WINDOW`). `--exclude=refs/stash` precedes `--all`. Graph `o` replaces `--all` with the selected local tips (`refs/heads/<name>`).

| Function | Command | Purpose |
| --- | --- | --- |
| `load_graph_model_window` | `log --exclude=refs/stash --all --topo-order --date-order --skip --max-count --pretty=%H%x00%P%x00%s%x00%an%x00%at` | One history page. Always sets the working-tree row (`Some(has_changes)`). Empty `focus_branches` uses `--all`; otherwise `log` the named tips (no `--all`) so unrelated history drops. |
| extra `stash^1` | `log --no-walk --ignore-missing --pretty=…` | Missing stash parents appended after the log prefix so autoload skip uses `window`, not `commits.len()`. Skipped while branch focus is on. |
| `should_autoload` / `merge_autoload` | next page at `skip + window_count` | Cursor on last loaded row; skip stays at the original window start; `window` grows |

Hidden ignored checkouts stay out of `P` / `S` / `b` unless shown. Linked worktrees are included on `f` / `p` / `P` / `d` only when that row is focused. The background fetch timer (`background_fetch_targets` in `tui/fetch.rs`) includes every snapshot except hidden ignored — linked worktrees and shown ignored repos included. See [tui-rust.md](./tui-rust.md).

Manual `f` / `p` / `P` / `d` and the background fetch tick paint a trailing breadcrumb counter (`Fetching n/N…`, `Pulling n/N…`, `Pushing n/N…`, `Switching n/N…`) and redraw as each repo completes (not as it starts). Fetch / pull / push of independent checkouts overlap under `FETCH_CONCURRENCY` (10) on the per-gitdir remote queue. Mixed kinds paint `Fetching 1 · Pulling 1…` or `Fetching 1/2 · queued 1`. When the op finishes, that slot is a count (`Fetched N repos`, `Pulled N repos`, `Pushed N repos`, `Switched N repos`), with ` (N failed: <first repo> — <git reason>)` (`, +N more` when several failed) and ` (N skipped: dirty)` for a `d` that left a dirty repo alone. A pull whose auto-stash pop conflicted reads `<repo>: pulled, but restoring local changes conflicted — resolve, then check git stash list` instead (error; the stash entry is kept). Repos that worked are not listed. Any failure is an error status; skips alone are a warning. The hint row stays pills + keys. Graph autoload still uses `loading older…`. Those git children (and watch / full-snapshot reload) run on `spawn_blocking` so resize and quit still reach the event loop; overlay modes do not start the watch or fetch timers. Watch collect applies each checkout as it finishes. The follow-up right-pane reload (`git log` / file `diff` / commit files after fetch / pull / push / watch / left-pane movement at every depth) is another worker job. An unchanged watch snapshot (tree signatures **and** checkout `HEAD` / sync note / dirty set) skips it. The next watch tick is scheduled from the start of the interval.

## Non-obvious semantics

**Renames need both paths.** Staging only the new path leaves the deletion of the old path unstaged, and git then reports the pair as `D` + `A` rather than `R`. Writes apply to each path in order and stop at the first failure.

**Bulk stage / unstage.** `s` / `u` use `collect_write_files`: a file row is itself; a dir walks descendants on that section side (every Changes dir id ends with `#unstaged`, including when collapse names differ from Staged); a Staged / Changes header walks every dirty file on that side of the checkout; a checkout (or flat repo) walks every dirty file — never mixes sibling checkouts under a family container. Workspace, group, and family-container rows yield an empty list. Stage keeps files with unstaged or untracked; unstage keeps staged. Empty after filtering: `Nothing to stage` / `Nothing to unstage`. Wrong focus: `Focus a file, dir, checkout, or repo to stage|unstage`.

**Visual-line range stage / unstage.** While `V` highlight is on a focused worktree file diff, `s` / `u` do not whole-file stage. They build a unified patch from the highlighted add/del lines (a hunk header in the range selects that whole hunk) and run `apply_cached_patch`. Stage reads the UNSTAGED / NEW section (`git apply --cached`). Unstage reads STAGED (`git apply --reverse --cached`). For Stage, unselected additions drop and unselected deletions become context. For Unstage, the patch applies in reverse, so the post-image must match the index: unselected additions become context and unselected deletions drop. Inside a run of deletions and the additions after it, the patch interleaves them by pair (first `-` then first `+`, and so on), so a selected pair lands where its deletion was and the other lines keep their order. Each rewritten hunk keeps the start line of the side that matches the target (the index for Stage, the index or worktree for Unstage / Revert). The other side's start is moved by the line-count change of the hunks emitted before it: git applies hunks in order and uses that side as the position hint, and with `diff.context=0` a pure insertion has no context to search, so a stale hint would place it on the wrong line. A `\ No newline at end of file` marker stays only after the last line of its side: a `-` / `+` line that is no longer last loses it (it gains a newline), and a context line that ends one side but not the other splits into a `-` / `+` pair. Otherwise git would join that line with the next one. Fail closed (breadcrumb, no write) when the range is context-only, spans staged and unstaged, is a committed diff, is binary, or cannot become a valid patch. Success clears the highlight. Normal-mode `s` / `u` stay whole-file.

**Visual-line range revert.** While `V` highlight is on a focused worktree file diff, `x` discards only the highlighted add/del lines from the worktree. It reads the UNSTAGED section only, builds the same reverse patch as Unstage, and runs `apply_worktree_patch_reverse` (`git apply --reverse`, no `--cached`). The index does not change. The patch is built before the confirm opens (`Discard highlighted lines in <path>?`, `y` / `n`), which clears the highlight. Fail closed when the range is context-only or only in STAGED, spans staged and unstaged, is a committed diff, is binary, is a new file (untracked, or intent-to-add with a `new file mode` header), or has a rename, copy, mode change, or deletion header. Commit / stash / worktree drill diffs refuse. A compare diff reverts to the merge base instead (see **Compare revert to the merge base**). Normal-mode `x` stays whole-file.

**Compare revert to the merge base.** On a compare tab, `x` puts the **merge base** version back in the worktree: the left side of the `<base>...HEAD` diff, not the base branch tip. If the base branch moved after the fork, the revert still restores the fork-point version. The worktree is the only thing that changes; HEAD and the index stay. A compare tab writes only while its head **is** the checked-out working tree for the file. The tab and the open diff are loaded for the current range and path (`compare diff still loading`), the checkout's HEAD equals the compare head (`compare is stale: HEAD moved`), and the file and a rename's old path have no staged, unstaged, or untracked change (`<path> has uncommitted changes`). The TUI checks this against the snapshot before the confirm opens and on the palette rows. The write worker checks it again with git (`ensure_compare_head_clean`) right before it writes, because the snapshot can be a watch tick old, and aborts with a status message when either check fails. Every compare write goes through a confirm (`y` confirms; Enter only names that key; `n` / Esc cancels; `Y` is not offered).

- **Highlighted lines.** `V` highlight + `x` builds a reverse patch from the highlighted rows of the COMMITTED section (`PartialPatchKind::RevertCommitted`), with the same hunk rewrite rules and the same binary / rename / copy / new-file / mode-change / deletion refusals as the worktree range revert. The confirm is `Revert highlighted lines in <path> to the <base-ref> merge base?`. `y` runs `revert_compare_patch` (`git apply --reverse`, no `--cached`); the post-image is the head blob, which the guard proves is the worktree file.
- **Whole file.** `x` on a compare file row or on the open compare diff (no highlight) opens `Revert <path> to the <base-ref> merge base?`. `y` runs `revert_compare_file` (`git restore --source=<merge-base> --worktree`). Status `M` restores the content. `A` (added on HEAD) deletes the file, and the confirm says so. `D` writes the file back. `R` writes the old path back and deletes the new path. Directories refuse (`focus a file to revert`).

The compare diff covers commits only, so it does not change after a revert. The result shows on the Workspace tab (`… (see Workspace tab)` status). Stage and unstage stay refused on a compare tab.

**Focused refresh (`r`).** Reloads the whole workspace on the workspace row or No-updates group, and otherwise one checkout (`refresh_target` → `ReloadSnapshot` vs `ReloadRepo { repo }`).

**Bulk revert with counted confirm.** `x` uses the same `collect_write_files` scope (section headers and dirs stay side-filtered), keeping unstaged or untracked (staged-only skipped). Confirm shows counts and only the keys that apply. `y` runs `git restore` on tracked targets and **keeps** untracked; with untracked targets present, `Y` also deletes each untracked via `remove_untracked_file` (per-file `clean -f`, not `clean -fd`). Tracked only: no `Y`. One untracked target and nothing tracked: `y` deletes it, no `Y`. Several untracked and nothing tracked: only `Y` (deletes them). A key the box does not show does nothing. Empty after filter: `Nothing to discard` (or `Nothing to discard (staged only)` on a staged-only file).

**Remove linked worktree (`W`).** Linked `Checkout` rows only. Confirm shows branch, `merged into default` / `NOT merged into default`, and what is lost: `N changed files will be deleted permanently · branch <b> is kept` when dirty (`--force`; N is the snapshot change count), else `clean worktree · branch <b> is kept`. A detached worktree shows neither branch part. Same-commit as the default tip is `NOT merged into default` (just created). On Unix, bind-mount aliases remap via inode so gitdir back-pointers match. On Windows, worktree identity is canonical path plus size and mtime (no inode / bind-mount remap).

**Reverting an untracked file deletes it.** There is no git object to restore to, so untracked “revert” means remove from disk — irrecoverable. Bulk `y` leaves untracked alone; opt in with `Y`, or press `y` when the only target is one untracked file.

**Staged-only files refuse revert.** When a file is staged with no unstaged component, the worktree already matches the index for that path, so `git restore` would be a no-op. Discarding a staged change is a two-step operation (`u` then `x`) by design.

**`repo_has_local_changes` ignores untracked files.** `-d` will therefore switch a branch in a repo that has untracked files. That is usually right (untracked files survive a checkout) but it is not what "has local changes" implies.

**`stash_push` treats a no-op as failure.** Apple Git 2.50 prints `No local changes to save` but exits 0. The wrapper compares `stash list` before and after and returns `Err` when the list is unchanged.

**Local branch picker (`b`).** Opens on a checkout or flat repo row (hidden on family containers), lists `refs/heads/` only (no remotes). Esc closes without quitting. Every printable key types into the filter; ↑/↓, Ctrl-n / Ctrl-p, and Ctrl-j / Ctrl-k move; Enter checks out. A filter that is a valid new name and not an exact local branch adds a last row `+ create branch <name>`; Enter on it runs `create_branch_checkout`. Enter with no row warns `no branch matches <name>` (plus the name rule it breaks). Dirty worktrees refuse checkout with `Dirty worktree — commit or stash first`. Selecting the current branch closes with `Already on …` and skips the dirty check.

**Graph actions** (graph list focused — depth 0 right or depth 1 left):

| Key | When visible | Behaviour |
| --- | --- | --- |
| `b` | Commit row with ≥1 local branch or `origin/*` ref | Dirty check first. One name → checkout (creates tracking from origin when local is missing). Several names → picker (locals then `origin/*`); its `+ create branch <name> at <short>` row runs `create_branch_at` on that commit (no checkout). Selecting `origin/<name>` when a local exists and tips differ opens confirm: Yes checks out the local then `fast_forward_to_remote_ref` of that selected `origin/<name>` (no fetch; `merge --ff-only`). Tags and non-`origin` remotes are not targets. |
| `c` | Any commit row | Name prompt → `create_branch_at` (ref only, HEAD unchanged). |
| `m` | Any commit row | Boxed confirm, then merge that ref into the checkout's current HEAD. Local / `origin/*` names when present; tags and unlabeled commits use the commit id. `git merge --ff-only`, else `git merge --no-ff --no-edit` (no rebase). Dirty tracked worktree refuses (`Dirty worktree — commit or stash first`) before the overlay. Conflicts stay uncommitted (no abort, no continue). Linked worktrees only when that checkout row is focused. |
| `S` | Uncommitted, stash, or commit with stash/dirty ops | Stash overlay (`stash_push -u` / apply / pop / drop as listed). |
| `o` | Graph list, or highlighted repo / worktree | Local-branch overlay. Space marks a set; Enter applies visible marks or the cursor row. Hidden marks do not leak through a filter. Reloads the graph as ancestors of those tips. Overlay Esc cancels. File, dir, workspace, and commit-file rows are a no-op. |
| `O` | Graph list, or highlighted repo / worktree with an active focus | Restore `--all`. In the focus overlay Ctrl-o clears (`O` types there). |
| `a` / `p` / `D` | Stash row | Apply / pop / drop (drop confirms with `y`/`n`/Esc). |

## Destructive operations

| Operation | Confirmation | Recoverable |
| --- | --- | --- |
| `s` stage / `u` unstage | none | yes — trivially reversible |
| `x` revert, tracked (`y`) | `y`/`n` prompt, plus `Y` when untracked is in scope | only via git's object store if the change was ever committed or stashed |
| `x` revert + delete untracked (`Y`) | same prompt | **no** for deleted untracked |
| `x` single untracked (`y`) | `y`/`n` prompt | **no** — the file is deleted |
| `-p` / `--pull` | none | yes — but can fail on conflicts |
| `-d` / `--default-branch` | none | yes — dirty repos are skipped, so no work is lost |
| TUI `d` | none for one repo; `y`/`n` boxed confirm (`Switch N repos to their default branch?`) when the scope has more than one repo off its default | yes — dirty repos are skipped |
| `b` checkout (local / origin) | none when in sync; `y`/`n` when local exists and origin tips differ (`Check out <b> and fast-forward to origin/<b> (no fetch)?`, plus `ahead_behind` counts) | yes — dirty worktrees refuse before checkout; confirm Yes is checkout then `fast_forward_to_remote_ref` of the already-fetched `origin/*` (no fetch, no reset). When local has commits the remote lacks, the checkout stays and the warn says `could not fast-forward to origin/<b>: local has commits origin/<b> lacks` |
| `m` graph merge into HEAD | `y`/`n` boxed confirm | yes — dirty tracked worktrees refuse before confirm; conflicts stay uncommitted (no abort) |
| `W` remove linked worktree, clean | `y`/`n` boxed confirm (`clean worktree · branch <b> is kept`) | yes — the branch and its commits stay; `git worktree add` brings the checkout back |
| `W` remove linked worktree, dirty (`--force`) | same confirm, which says `N changed files will be deleted permanently` | **no** for uncommitted and untracked files in that worktree; the branch is kept |
| Stash drop (`D` in the stash menu or on a graph stash row) | `y`/`n` boxed confirm | only from git's object store (`git fsck --unreachable`) until it is pruned |
| Stash pop (`p`) | none | yes — on a conflict git keeps the stash, so nothing is lost |
| Compare `x` (whole file or highlighted lines) | `y`/`n` boxed confirm (`Y` not offered) | yes — the guard requires the file to match HEAD, so `git restore` brings it back |
| `P` push | none | yes — never forces; a diverged remote makes the push fail |

Revert, stash drop, origin-out-of-sync graph checkout, graph merge, worktree remove, and multi-repo `d` use modal overlays, so no other key can act while one is up. Only the key the box shows (`y`, or `Y` where offered) accepts. Enter never confirms; it says which key does. `n` / Esc cancel.

## Write serialisation

Independent per-repo `git fetch`, `pull`, and `push` share one remote queue per gitdir. Manual `f` / `p` / `P` and the background fetch tick use that queue in `tui/effect.rs` (`RemoteQueue`). Occupy key is snapshot `primary_repo` when set. Otherwise the key is the checkout path. Linked worktrees of one repo share that key.

The Scheduler JoinSet starts a job when that gitdir is free. Cap is `env_fetch_concurrency()` (default 10). Override with `WS_STATUS_FETCH_CONCURRENCY`. `CappedBatch` is CLI `collect_snapshots` and tests. The live TTY path is Scheduler JoinSet `spawn_blocking`.

Pending jobs coalesce on that checkout only. A second `f` / `p` / `P` enqueues. Duplicate Fetch stays one pending job. A pending Fetch becomes Pull, and Pull drops a later Fetch. Push stays in FIFO order with Fetch and Pull. An inflight same-kind job does not queue a duplicate.

`busy_for_writes` is true only for an exclusive write or default-branch switch (`scheduler.rs`). Remotes are not a workspace mutex. `Fetch` / `Pull` / `Push` / `FetchTick` are `BusyAction::Handle` (`event_pump.rs`). Exclusive writes (stage, unstage, revert, stash, checkout, create-branch, merge into HEAD, remove-worktree, confirm-yes) are Ignore only while exclusive write or default-branch is busy. Default-branch `d` uses the same Ignore rule. Exclusive writes stay serial.

If that gitdir already has a remote, an exclusive write still dispatches. Then `schedule` refuses with breadcrumb `busy`. Confirm Yes, stash create/apply/pop, and create-branch submit keep the overlay in that case. Branch-picker create from a new name keeps the picker. Status is `busy`. A free gitdir may write while other repos fetch.

If `p` lands during an inflight fetch on that gitdir, dispatch may set `nothing behind to pull` (or `<repo> has diverged — …`). Unfetched tracking still looks in-sync. The queue then starts Pull after occupy release for those `op_targets` that have inflight or pending Fetch and are not diverged. Right-pane, compare, and drill `Effect::None` Pull do not follow.

Progress is `Fetching n/N…` as each checkout finishes. Mixed kinds paint `Fetching 1 · Pulling 1…` or `Fetching 1/2 · queued 1`. After a kind finishes, the slot is `Fetched N repos`, with ` (N failed: <first repo> — <git reason>)` (`, +N more` when several failed) and ` (N skipped: dirty)` for a `d` that left a dirty repo alone. A pull whose auto-stash pop conflicted reads `<repo>: pulled, but restoring local changes conflicted — resolve, then check git stash list` instead (error; the stash entry is kept). Repos that worked are not listed.

Quit drops remotes that have not started. In-flight git is unchanged.

Watch/status collect (`discover_checkouts` / `process_repo`) uses the same JoinSet cap. A live tick is not one-repo-at-a-time. There is no inotify.
