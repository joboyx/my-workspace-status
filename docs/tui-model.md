# TUI model

Pure data under `crates/workspace-status/src/tui/` (tree, fold, flatten, keymap, live refresh). The event loop is `Action` → `AppState::dispatch` → `Effect`.

## Node kinds

`NodeKind` in `tui/tree.rs` is a six-member enum. Five of them are structural; `Group` is the one people forget.

| Kind        | Id                           | Children                    | Notes                                                                                                                                                                                                                                                          |
| ----------- | ---------------------------- | --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Workspace` | `workspace` (literal)        | repos + at most one group   | Single root. Carries `change_count` and `sync_summary`. Root label is rendered as provided (natural case), not uppercased.                                                                                                                                     |
| `Repo`      | `repo:<repoPath>`            | dirs/files **or** checkouts | Flat when the primary has no linked worktrees (branch on the same row). With linked worktrees → **family container**: `ICON_REPO` + path + trailing `N wt` / change count; branch omitted; `merged_into_default: None`; sync is worst-of-family.                |
| `Checkout`  | `checkout:<repoPath>`        | dirs/files                  | Nested under a family container only. Primary uses `ICON_BRANCH` + branch (never `ICON_LINKED_WORKTREE`). Linked extras use `ICON_LINKED_WORKTREE` + branch (or short path when detached) + merge mark. Files live here, not on the container.                 |
| `Group`     | `group:no-updates` (literal) | repos                       | Holds every repo with **no** updates (clean worktree, default branch, up-to-date with upstream, no attention sync note). Only ever one, only present when at least one such repo exists.                                                                       |
| `Dir`       | `dir:<repoPath>:<fullPath>`  | dirs/files                  | Tree mode only. Single-child chains are collapsed (`ai/common/skills` as one node).                                                                                                                                                                            |
| `File`      | `file:<repoPath>:<path>`     | —                           | Leaf. Carries the originating `FileChange`.                                                                                                                                                                                                                    |

`Group` exists so clean repos collapse into one line by default instead of padding the tree. Anything that switches on `kind` — `node_segments`, fold walks, `has_children`, `repo_path_of` — must handle `Checkout`. `repo_path_of` / graph drill: checkout → its path; family container → primary checkout path; file/dir → `repo`. Returns `None` for `Group` and `Workspace`, which is what makes `r` on either of them refresh the whole workspace.

## Ids

Ids are stable path-derived strings, never indices. Three things depend on that:

- After a refresh the cursor is restored by id, so a new file appearing above the cursor does not shift the selection. When the focused id is hidden under a folded ancestor (typical: repo moved into folded `group:no-updates` after pull), ancestors are unfolded so the same entry stays selected. If the id is gone even after unfold (file committed/stashed, ghost expired), ancestor walk uses parent `dir:` prefixes then `repo:` / `checkout:` parsed from the id string.
- The fold set is a set of ids that survives tree rebuilds.
- `watch.rs` attributes file flashes with file node ids and op-row flashes with repo node ids. Chrome flashes walk the tree and skip file nodes — files stay on status+mtime signatures. Chrome identity includes path / branch / sync enum / **sync note** / **HEAD** / change count / fold, so a new local commit on a clean branch or ahead 2→3 is a real update (status, graph, and right pane reload without `r`). Graph and commit-file rows use separate identity maps (`{repo}#{kind}:{id}` and `{repo}#{source}#{row_id}`) so a repo switch is a new list, not a mass flash. Add/update/remove flash the same row only. Removals keep a ghost row for the 800ms fade. The flash is a background fade (four-step ramp); status letter colours stay. The live poll is due from the start of `WS_STATUS_WATCH_MS`, not after collect finishes.

## `build_tree` → flatten → `VisibleRow`

`build_tree` groups snapshots by `primary_repo` when set, otherwise `repo`. A sole primary → flat `Repo` (today). Primary + linked → family container with `Checkout` children (primary first, then linked by path). Family attention if **any** checkout needs attention; containers with linked children start **expanded**. Within each attention bucket, families sort by primary path.

A checkout needs attention when it has file changes, is not on its effective default branch (`main` / `master` / `develop`, or a sole `defaultBranches` override when configured), has a non-idle sync (`behind` / `ahead` / `diverged`), **or** has an attention sync note (`no commits yet` / `status failed`). Only default-branch checkouts that are clean, up-to-date with upstream, and free of those notes land under `group:no-updates`. Workspace `sync_summary` counts those attention notes (`N attention`) so an attention-only workspace is not reported as `all current`. Empty discovery yields `no repos` (not `all current`).

Per checkout (or flat repo), children are either flat file nodes or a directory trie (tree mode).

Flatten walks depth-first, emitting a `VisibleRow` per node: `id`, `depth`, `node`, styled segments / trailing, and a plain `label` (used for pane `/` search). Children are omitted when the node's id is in `folds`.

## Fold state

Default folds: ignored repos with children + the `no-updates` group. Non-ignored repos with changes start expanded. `z` toggles immediately and arms a 400ms `zz` window for subtree toggle.

## ViewStack / Enter / Esc

`DrillView` in `tui/drill.rs` is the depth stack (`Graph` = 0, `Files` = 1, `Diff` = 2). `FocusPane` is left or right. Together they are the ViewStack.

| Depth | Left | Right |
| --- | --- | --- |
| 0, repo/dir focused | Workspace tree | Graph |
| 0, file focused | Workspace tree | Worktree `DiffPane` |
| 1 `Files` | Graph list | Commit detail + file tree |
| 2 `Diff` | Commit-file list | Commit-scoped `DiffPane` |

`list_focus_target` (`tui/state.rs`) is the focused list: depth 0 left → tree; depth 0 right / depth 1 left → graph; depth 1 right / depth 2 left → commit files; depth 2 right (and depth 0 file diffs) → none (diff scroll). `j`/`k`, fold, search, click, and vertical wheel follow that target. Mouse horizontal wheel (and Shift+wheel) pans the pane under the pointer without moving the focused row; the workspace tree matches the right pane. Click still selects. Unshifted `h` / `l` still fold the tree.

| Focus / depth | Enter | Esc |
| --- | --- | --- |
| Left (any depth) | Focus right (same stack) | Pop one depth; stay on left (no-op at depth 0) |
| Right, depth 0 | Push commit files when a graph row can drill; stay on right | Unfocus → left |
| Right, depth 1 | Push commit diff; stay on right | Unfocus → left |
| Right, depth 2 (leaf) | No-op | Unfocus → left |

Enter that deepens the stack keeps **right** focus. Esc that pops a depth keeps **left** focus. Esc never quits.

Left-pane `j`/`k`, click, vertical wheel, and search next/prev load the matching right pane through `Effect::LoadRightPane` (the same `load_right` / `run_work_pumped` path as depth 0). Mouse horizontal wheel over the workspace tree pans without `LoadRightPane`. Depth 1 graph rows load that commit / stash / worktree's files. Depth 2 file rows load that file's commit diff. Directory rows keep the previous diff. Esc/back and the depth stack stay unchanged.

## App state

`AppState` in `tui/state.rs` holds restorable view state: cursor row id, folds, filter, diff mode, full-context ids, tree mode, mouse enabled, theme, nav (focus pane + drill depth), `diff_col_offset`, `left_col_offset`, and `right_col_offset`. Graph window default is 300. There is no on-disk session store — fold, split ratios, pan offsets, and theme cycle reset on the next launch. The GitHub Release startup prompt (`update_check.rs`) runs in the CLI **before** `AppState` exists; it is not session state.

Full-context membership means unlimited unified context (`FULL_DIFF_CONTEXT_LINES`) for that file id — toggled by `Ctrl+O`, cleared by `Esc` or a second `Ctrl+O`. After a full-file toggle, scroll recenters on the prior hunk/change anchor.

## Graph list

Graph rows come from `workspace-status-graph` `GraphModel::visible_rows`. Stashes park immediately above `stash^1`. Pane `/` matches a graph row on subject, author, painted relative date or local `YYYY-MM-DD HH:MM`, UTC `YYYY-MM-DD HH:MM`, branch and tag names, and full or short sha (stash / worktree rows use those fields when present). Graph pane chrome: 2-line selection footer when height ≥ 3 (`graph_chrome_budget` / `selection_detail_lines` / `selection_detail_parts`; footer preferred, header drops first). Uncommitted: `Working tree clean` / `Uncommitted changes` + HEAD commit ref chips, or `worktree · not a commit` when HEAD has none. Spacer: `…` + `connector · not selectable`. Stash: subject + `stash@{n} ·` hash `·` date. Commit: subject + ref chips `·` hash `·` author `·` date, or `(no refs)`. Empty: `no selection`. Footer chips use the same `LabelKind` palette as the row (HEAD / default / feature / remote / tag), not a flat wash. Graph PageUp/PageDown move in painted list space (`visible − 1` painted lines, snap to selectable); `j`/`k` and click stay on selectable `visible_rows`. The 1-column scrollbar thumb is painted only after the list leaves the top; the horizontal bar only after the viewport leaves the left edge. Both are mouse-draggable through the same `hit_split` / `SplitDrag` stack as the pane and in-diff splitters (`SplitDrag::GraphScrollbar` / `GraphHScrollbar`); a track click jumps toward that position without moving the graph cursor. Horizontal mouse wheel (and Shift+wheel) pans the pane under the cursor without changing keyboard focus.

See [git-graph-topology.md](./git-graph-topology.md) and [graph.md](./graph.md).

## Actions and gates

`tui/action.rs` is the single source of truth for which actions exist. `tui/keys.rs` maps crossterm events to `Action`. `tui/gates.rs` hides writes that the focused row / depth / pane must not run. `tui/ops.rs` resolves stage / unstage / revert file lists (`collect_write_files`) and bulk git targets (`op_targets` / `push_targets`).

`stage` / `unstage` / `revert` resolve targets through `collect_write_files` (file / dir / checkout / flat repo). Family containers, workspace, and group yield no files. Revert opens a counted confirm; `y` discards tracked only, `Y` also deletes untracked via per-file `remove_untracked_file`. `r` reloads one checkout unless the focused row is workspace or `group:no-updates`.

Write scope for bulk git (`f` / `p` / `P` / `d`): primaries on workspace/family rows; a linked worktree only when that row is focused; hidden ignored-list paths omitted even if focused. When ignored repos are shown (`.` / `-a`), they follow the same primary / focused-worktree rule. Background fetch uses `background_fetch_targets` and skips hidden ignored repos. Independent checkouts run fetch / pull / push in parallel (`FETCH_CONCURRENCY` = 4). After those ops finish, `status` is a count (`Fetched N repos`, with ` (N failed)` if any) — never a list of names.

Graph pane writes (`graphCheckout` / `graphCreateBranch` / `graphMerge` / stash apply / drop / pop) gate on graph-list focus (depth 0 right or depth 1 left). Graph `m` is `graphMerge` (always confirms). Leftover branch/tag chips on a commit spacer: truncate the next name with `…` (keep brackets) when part of it fits; bold `[+N]` counts only chips fully hidden after that (omit `[+N]` when the last painted chip is merely truncated) so the row does not grow.

## Editor

- **Detached GUI** (`cursor`, `code`, `code-insiders`, `codium`, `gvim`, including `--wait`): spawn detached while the TUI stays mounted. Fold, focus, scroll, search, and `show_ignored` stay in `AppState`. File-status updates come from the live watch poll.
- **TTY editor** (default `vim`): leave the alternate screen, spawn with inherited stdio in the file's repo cwd, restore raw mode, re-enter the alternate screen. Resume drains leftover bytes so the next keypress does not need an extra Enter.

## Keymap

Live keys: [configuration.md](./configuration.md) and [tui-rust.md](./tui-rust.md). `?` paints MOVE / GIT / VIEW from `tui/help.rs`. Held `h`/`j`/`k`/`l` (and arrows) auto-repeat; Repeat of `z`/`g`/writes/quit is ignored so chords stay one-shot.

Quit is a double **Ctrl-C** chord (`tui/ctrl_c_exit.rs`). First press paints `Press Ctrl+C again to exit` for `CTRL_C_EXIT_MS` (2s); second within that window exits. Overlay pickers (stash / branch / create-branch) show the copy inline; otherwise it is a pinned chrome row, never a breadcrumb toast. The chord is handled before overlay modes, so it does not close a picker. `q` also quits immediately (Rust extra). Esc never quits.
