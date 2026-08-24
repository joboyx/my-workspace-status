# TUI model

Pure data under `src/tui/model/` (no React, no Ink), plus the keymap and live-refresh machinery.

## Node kinds

`TreeNode` in `model/types.ts` is a six-member union. Five of them are structural; `group` is the one people forget.

| Kind        | Id                           | Children                    | Notes                                                                                                                                                                                                                                                          |
| ----------- | ---------------------------- | --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `workspace` | `workspace` (literal)        | repos + at most one group   | Single root. Carries `changeCount` and `syncSummary`. Root label is rendered as provided (natural case), not uppercased.                                                                                                                                       |
| `repo`      | `repo:<repoPath>`            | dirs/files **or** checkouts | Flat when the primary has no linked worktrees (branch on the same row, today’s chrome). With linked worktrees → **family container**: `ICON_REPO` + path + trailing `N wt` / change count; branch omitted; `mergedIntoDefault: null`; sync is worst-of-family. |
| `checkout`  | `checkout:<repoPath>`        | dirs/files                  | Nested under a family container only. Primary uses `ICON_BRANCH` + branch; linked uses `ICON_LINKED_WORKTREE` + branch (or short path when detached) + merge mark. Files live here, not on the container.                                                      |
| `group`     | `group:no-updates` (literal) | repos                       | Holds every repo with **no** updates (clean worktree, default branch, up-to-date with upstream, no attention sync note). Only ever one, only present when at least one such repo exists.                                                                       |
| `dir`       | `dir:<repoPath>:<fullPath>`  | dirs/files                  | Tree mode only. Single-child chains are collapsed (`ai/common/skills` as one node).                                                                                                                                                                            |
| `file`      | `file:<repoPath>:<path>`     | —                           | Leaf. Carries the originating `FileChange`.                                                                                                                                                                                                                    |

`group` exists so clean repos collapse into one line by default instead of padding the tree. Anything that switches on `kind` — `nodeSegments`, `walkFoldable`, `createFoldState`, `hasChildren` (duplicated in `flatten.ts` and `useAppState.ts`), `repoPathOf` — must handle `checkout`. `repoPathOf` / graph drill: checkout → its path; family container → primary checkout path; file/dir → `repoPath`. Returns `null` for `group` and `workspace`, which is what makes `r` on either of them refresh the whole workspace.

## Ids

Ids are stable path-derived strings, never indices. Three things depend on that:

- After a refresh the cursor is restored by id (`focusIdRef` + `resolveFocusAfterRebuild` in `session.ts`, applied from `useAppState.ts`), so a new file appearing above the cursor does not shift the selection. `focusIdRef` is the intended row id: it is written only in cursor mutations (`setTreeCursor`) and after restore — never from `rows[cursor]` during render, which would stamp a stale index onto the wrong row. When `displayedRows` is passed (flatten plus `mergeGhostRows` ghosts), restore uses that painted list so a ghost spliced above the cursor does not steal the highlight. After an unfold, callers apply returned `folds`, rebuild the painted list with the same clock as the first merge, then set the cursor via `resolveListFocus` on that painted list (id, then ancestors, then clamp) rather than the helper's flatten `cursor` or `cursorIndexFor` (which jumps to row 0 on a miss). When the focused id is hidden under a folded ancestor (typical: repo moved into folded `group:no-updates` after pull), ancestors are unfolded so the same entry stays selected. If the id is gone even after unfold (file committed/stashed, ghost expired), `focusAncestorIds` walks parent `dir:` prefixes then `repo:` / `checkout:` parsed from the id string. Returned `focusId` is the row actually selected. Session remount after `$EDITOR` uses `resolveFocusAfterRebuild` (workspace tree only; commit-file cursor is not in session state) so a vanished edited file does not jump to row 0.
- The fold set is a `Set<string>` of ids that survives tree rebuilds.
- `watch.ts` attributes file flashes with `fileNodeId` (must match `makeFileNode`) and op-row flashes with `repoNodeId` (must match `makeRepoNode`). Chrome flashes walk the tree (`treeChromeSignatures`) and skip file nodes — files stay on status+mtime `changeSignatures`.

## buildTree → flatten → VisibleRow

`buildTree(BuildTreeInput)` groups snapshots by `primaryRepo ?? repo`. A sole primary → flat `RepoNode` (today). Primary + linked → family container with `CheckoutNode` children (primary first, then linked by path). Family attention if **any** checkout needs attention; containers with linked children start **expanded**. Within each attention bucket, families sort by primary path.

`buildTree` classifies a checkout as needing attention when it has file changes, is not on its effective default branch (`main` / `master` / `develop`, or a sole `defaultBranches` override when configured), has a non-idle sync (`syncStatus === 'behind' | 'ahead' | 'diverged'`), **or** has an attention sync note (`no commits yet` / `status failed`, via `isAttentionSyncNote`). Only default-branch checkouts that are clean, up-to-date with upstream, and free of those notes land under `group:no-updates`; a spotless feature/bugfix/chore branch, a behind/ahead/diverged default-branch checkout, or an unborn/status-failed checkout stays top-level so its branch/sync badge remains visible. Workspace `syncSummary` counts those attention notes (`N attention`) so an attention-only workspace is not reported as `all current`. Empty discovery yields `no repos` (not `all current`).

Per checkout (or flat repo), children are either flat file nodes or a directory trie via `collapseDir` chain-compaction (tree mode).

`flatten(tree, folds)` walks depth-first, emitting a `VisibleRow` per node: `id`, `depth`, `node`, styled `segments` / `trailing`, and a plain `label` (used for pane `/` search and tests). Children are omitted when the node's id is in `folds`.

`flatten` **infers** tree-vs-flat mode by scanning for any `dir` node (`detectTreeMode`) rather than taking a parameter. It works because only tree mode produces `dir` nodes, but it misfires for a workspace whose every changed file sits at a repo root — that tree has no `dir` node, so file rows render in flat style (dimmed containing directory appended) even though tree mode is on. The correct source of truth is `treeMode` in `useAppState`, which is not threaded through.

## Fold state

`model/fold.ts`:

| Function                                          | Behaviour                                                                                                           |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `createFoldState(tree)`                           | Default folds: ignored repos with children + the `no-updates` group. Non-ignored repos with changes start expanded. |
| `collectFoldableIds(tree)`                        | Every non-`file` id.                                                                                                |
| `collectFoldableSubtreeIds(tree, focusId)`        | `focusId` plus every foldable descendant under it; `[]` when missing or a file.                                     |
| `ancestorIdsTo(tree, id)` / `unfoldAncestors`     | Ancestor chain / open folded parents so a relocated id can appear in `flatten`.                                     |
| `unfoldForestAncestors(nodes, folds, id)`         | Same unfold across a forest of roots (commit-file lists have no workspace wrapper).                                 |
| `applyFold(folds, action, focusId, foldableIds?)` | Returns a new `Set`; never mutates.                                                                                 |

`FoldAction` vocabulary — `toggle` / `open` / `close` / `openAll` / `closeAll` / `toggleSubtree`. `openAll` returns an empty set. `closeAll` **throws** when `foldableIds` is empty rather than silently returning an empty set, because a silent no-op there is indistinguishable from "nothing was foldable" and would have hidden a real wiring bug. `toggleSubtree` opens or closes `focusId` and all foldable descendants together (same empty-ids throw spirit as `closeAll`); pass ids from `collectFoldableSubtreeIds`.

## Keymap state machine

`keys.ts` maps one keypress plus a `KeyState` to an `Action`. It owns no application state.

`KeyState` gates, checked in this order:

| Gate / field               | Effect                                                                                                                                                                       |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `confirmMode`              | Only `y`/`Enter` → `confirmYes`, `Y` → `confirmYesClean`, `n`/`Esc` → `confirmNo`. Everything else is swallowed.                                                             |
| `searchMode`               | Returns `none` for everything — `App`/`useAppState` consume `/` query characters.                                                                                            |
| `easyMotionMode`           | Returns `none` — `App` consumes EasyMotion label keys; Esc cancels.                                                                                                          |
| `branchMode`               | Returns `none` for everything — `App` owns picker filter / cursor / Esc / Enter; Esc closes and never quits.                                                                 |
| `createBranchMode`         | Returns `none` — `App` owns create-branch name input; Esc cancels, never quits.                                                                                              |
| `graphBranchMode`          | Returns `none` — checkoutable-branch picker at a graph commit.                                                                                                               |
| `stashDropMode`            | Returns `none` — stash drop `y`/`n` confirm.                                                                                                                                 |
| `graphCheckoutConfirmMode` | Returns `none` — origin out-of-sync checkout `y`/`n` confirm.                                                                                                                |
| `stashMenuMode`            | Returns `none` — stash overlay (`s`/`a`/`p`/`d`/Enter/Esc). Overlay keys are not registry. Drop closes the menu then opens `stashDropMode`.                                  |
| `zPending`                 | First `z` already emitted `fold.toggle`. Second `z` within the window → `toggleSubtree`. Any other key clears pending and redispatches (no `za`/`zo`/`zc`/`zR`/`zM` chords). |
| `gPending`                 | Second `g` within the window → `moveTo start`. Lone `g` expires to `none`.                                                                                                   |
| `searchActive`             | When true (armed search query), `n` → `searchNext`, `N` → `searchPrev`. Idle `p` stays pull; graph stash `p` stays stashPop.                                                 |
| `pendingAt`                | Timestamp when a pending double-tap was armed; `flushPending(state, now)` resolves expired pendings.                                                                         |

`handleKey` sets the pending flags itself but only _reads_ `confirmMode` / `searchMode` / `easyMotionMode` / `searchActive` / `branchMode` / `createBranchMode` / `graphBranchMode` / `stashDropMode` / `graphCheckoutConfirmMode` / `stashMenuMode`; `useAppState` owns those flags. The split means chords are testable in isolation while the modal gates stay with the state that raises them. The `kind` passed into `handleKey` is **`activeRowKind`** (below) — the same value the hint bar uses.

`flushPending` runs from a ~50 ms timer in `useAppState`: after `DOUBLE_TAP_MS` (400), expired `z` pending clears with **`none`** (toggle already fired on the first key); lone `g` also clears without moving. Space is not a fold chord — on a file row it emits `toggleViewed`, otherwise `none`.

Also handled outside double-taps: `G` → `moveTo end`, `m` → `toggleMouse`, `T` → `cycleTheme`, arrows/`j`/`k`/`h`/`l` (with optional `HandleKeyCtx`: when `focusPane === 'right'` and `rightIsDiff`, `j`/`k`/↑↓ → `scrollDiff` ±1 and `h`/`l`/←→ → `panDiff` instead of list-move / fold), `PageUp`/`PageDown` → `pageMove` (±1 page on the focused pane), Ctrl-u/d → `scrollDiff` ±5 (same focus routing + clamp), `Ctrl+Space` / `;` → `easyMotionStart`, `/` → `searchStart`, registry action keys, `Enter` → `navEnter`, `Esc` → `navEsc` (outside overlays; armed search clears first). `KeyState.navDepth` mirrors the ViewStack so `t` routes to `toggleTreeMode` (depth 0) or `toggleCommitTreeMode` (depth ≥ 1). `t` is a **view-mode** toggle like `i` — it is not a left-list action, so it still fires when the right pane is focused. `.` emits `toggleShowIgnored` the same way: it flips session `showIgnored`, refilters the tree (`snapshotsForView`), and restores focus. Launch with `-a` seeds `showIgnored: true`. `gg`/`G` (`moveTo`) on a focused diff scroll to start/end via `diffScrollForMoveTo` / `clampDiffScroll`.

### Active row kind

`activeRowKind` in `activeContext.ts` is the single row kind for hints, keymap `kind`, and `runAction` gates. It calls `listFocusTarget({ depth, focusPane, graphVisible })` then picks the row on that pane (or the file feeding a focused diff). Fallback `'workspace'` when that source is null.

| `listFocusTarget` | Active row kind                                                            |
| ----------------- | -------------------------------------------------------------------------- |
| `tree`            | workspace-tree focused node                                                |
| `graph`           | graph list row (`graphCommit` / `graphStash` / `graphUncommitted`)         |
| `commitFiles`     | commit-file focused node                                                   |
| `none` (diff)     | file feeding the diff (commit-file at depth ≥ 2, else workspace-tree file) |

Graph pane writes (`graphCheckout` / `graphCreateBranch` / `graphMerge` / `stashApply` / `stashDrop` / `stashPop`) gate on graph-list focus (depth 0 right, depth 1 left, or any later depth where the focused pane is the graph list).

### Track C — fold, search, EasyMotion, page

| Feature                 | Behaviour                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Instant fold (C1)       | `z` toggles immediately; double-tap within 400 ms applies `toggleSubtree`. No `z*` open/close/all chords. `fold` / `expand` / `collapse` (`z`/`h`/`l` fold) no-op when the graph or a diff is focused — they must not mutate a hidden workspace tree.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| Mouse double-click (C2) | Same cell within `DOUBLE_CLICK_MS` on a tree, graph, or commit-file row (not the chevron) → `navEnter` (same as Enter). Chevron click still toggles fold. Graph spacers click to the paired parent.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Page + clamp (C3 / B8)  | `pageMove` / Ctrl-u/d / right-diff `j`/`k` move the focused list cursor or scroll the diff; indices clamp at ends. Diff scroll uses `clampDiffScroll` (EOF = `rowCount - viewHeight`) so repeated PageDown is a no-op — state matches DiffPane paint. `gg`/`G` on a focused diff are the same clamp at start/end.                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Vim search (C5)         | `/` searches the focused pane (workspace tree, graph, commit files, or diff). Matches jump without hiding unmatched rows. Folded hits stay in the match list. `n`/`N` (and the first match while typing) unfold ancestors of the row about to receive focus; other matches stay folded until visited. Match rows use B10 search background. `n`/`N` wrap on the pane that was focused when `/` opened. Esc clears highlight; the cursor stays. While `?` help is open, `/` is help-local (`helpSearch.ts` / `helpSearchQuery`) and does not arm pane search.                                                                                                                                                                                      |
| EasyMotion (C4)         | `Ctrl+Space` or `;` labels and jumps the **focused** list (workspace tree, graph, or commit files; `a`–`z`, then `aa`…). Ink does not pass Ctrl+Space as `' '` with `ctrl`. NUL (`\x00`) becomes `` ` `` with `ctrl`. A named space with `ctrl` is `input === 'space'`. `isEasyMotionStart` accepts those encodings. macOS often does not send Ctrl+Space (input-source switch or a terminal binding). `;` is the fallback that still arrives. Diff-focused start is a no-op (mode does not stick). Glyphs paint only on that list (`easyMotionPaintSlot`) — never on an unfocused tree. Jump resolves against the same painted window (`visibleTreeWindow` / `visibleGraphWindow` + `resolveEasyMotionJump`). Typing a label jumps; Esc cancels. |

### Track D — diff pan + full-file scroll

| Feature               | Behaviour                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Horizontal pan (D1)   | No word-wrap. `diffColOffset` + `DiffPane` `colOffset` slice code cells. `h`/`l`/arrows pan only when right-focused on a file diff; otherwise fold. Reset on focused-file change.                                                                                                                                                                                                                                                                                                         |
| Vertical scroll (D1b) | When right-focused on a file diff, `j`/`k`/↑↓ emit `scrollDiff` ±1 (same clamp as PageUp/Ctrl-u). Depth 2 right and depth 0 file DiffPane both use this path (`listFocusTarget` → `'none'`).                                                                                                                                                                                                                                                                                              |
| Full-file scroll (D2) | `ctrl+o` still toggles `fullContext` only (never opens `$EDITOR`). After reload, `anchorRowIndex` + `scrollToKeepRow` keep the prior change/hunk near the upper third of the viewport. `e` / `ctrl+o` fire on commit-file rows (depth 1 right and depth 2) **and** on a focused file diff at every depth. Tree writes (`s`/`u`/`x`/`f`/`p`/`P`/`d`/`b`/`W`) stay blocked at depth ≥ 1 and are not advertised when the right pane is focused (`e` / `ctrl+o` stay on a focused file diff). |

Pure helpers: `search.ts` (`matchIndices` / `stepMatch` / `collectMatchIds` / `focusTreeSearchMatch` / `matchDiffRowIndices` / `collectSearchMatchIds`), `helpSearch.ts` (`helpEntryMatches` for `?` overlay `/`), `helpLayout.ts` (column width + description wrap + overlay height), `easyMotion.ts` (`easyMotionLabels` / `resolveEasyMotionLabel` / `resolveEasyMotionJump`), `pageNav.ts` (`applyPageMove` / `pageDelta`), `diffPan.ts` / `diffScroll.ts` (`clampDiffScroll`, `scrollToKeepRow`, `anchorRowIndex`).

The `Action` union covers move / `moveTo`, fold/expand/collapse, toggleTreeMode / toggleCommitTreeMode / toggleShowIgnored, cycleTheme, stage/unstage/revert, toggleDiffMode, refresh, edit, `toggleViewed`, `fullFile`, `searchStart` / `searchNext` / `searchPrev`, `easyMotionStart`, `pageMove`, help, quit, confirmYes / confirmYesClean / confirmNo, scrollDiff, `panDiff`, toggleMouse, `navEnter` / `navEsc`, `graphCheckout` / `graphCreateBranch` / `graphMerge` / `stashApply` / `stashDrop` / `stashMenu` / `stashPop`, none.

`stage` / `unstage` / `revert` resolve targets through `collectFiles` in `scope.ts` (file / dir / checkout / flat repo). Family containers, workspace, and group yield no files — write on a checkout child. Revert opens a counted confirm (`PendingConfirm` with `kind: 'revert'`); `y` discards tracked only, `Y` also deletes untracked via per-file `removeUntrackedFile`. The ratatui TUI uses the same scope in `crates/workspace-status/src/tui/ops.rs`. `r` reloads one checkout unless the focused row is workspace or `group:no-updates`.

`branch` opens the local-only picker on a checkout or flat repo (`BranchPicker` + `listLocalBranches` / `sortBranchesForPicker` / `filterBranches` / `branchPickerPath`). Family containers hide `b`. Checkout refuses a dirty worktree; success refreshes that path and closes the picker. `useAppState.runAction` must forward `branch` to `useActions` (`USE_ACTIONS_FORWARDED` / `case 'branch'`) — otherwise the key is a silent no-op.

`removeWorktree` (`W` / `w`) is registry-scoped to linked `checkout` rows and flat linked `repo` rows (named-filter linked-only). Opens `PendingConfirm` with `kind: 'removeWorktree'` (path, branch, merge state, dirty/`force`). Confirm copy states `merged into default` / `NOT merged into default` and whether `--force` will be used. `y` runs `git worktree remove [--force]` via `resolveWorktreeRemoveTarget` (bind-mount aliases remap to git’s registered path + primary prefix), then refreshes (drops missing linked + refreshes primary). Hint label may read `remove worktree (merged)` / `remove worktree (open)`. `useAppState.runAction` must forward `removeWorktree` to `useActions` (`USE_ACTIONS_FORWARDED` / `case 'removeWorktree'`) — otherwise the key is a silent no-op.

### Graph write actions (A8 / P5)

Whenever the graph **list** is focused (`isGraphListFocused` / `listFocusTarget === 'graph'`): depth 0 right **or** depth 1 left. Registry kinds: `graphCommit` / `graphStash` / `graphUncommitted`. Specs use `depths: [0, 1]` and omit `focusPanes`. Hints use `actionsForContext` plus `actionVisibleForGraphRow` **only when `rowKind` is a graph kind**, so `b` is omitted when the commit has no local branch or `origin/*` ref. Tree hint lists (`file` / `repo` / …) stay on `actionVisibleForScope` even if a graph selection is still passed in. `stashMenu` (`S`) is `focusPanes: ['left']` on tree and graph kinds.

| ActionId            | Key | Behaviour                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| ------------------- | --- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `graphCheckout`     | `b` | `checkoutableBranchNames` → none / single / `GraphBranchPicker` (locals + `origin/*`). Dirty refuse before picker, checkout, or confirm. `planGraphCheckout` then `checkoutBranch`, or `GraphCheckoutConfirm` when a local exists and tips differ (or a SHA cannot be read). Confirm Yes: checkout local then `fastForwardToRemoteRef` of the selected `origin/<name>` (no fetch; `merge --ff-only`). Ahead/diverged: stay on local tip, StatusBar error. No / Esc: cancel (no reset). No detached HEAD. |
| `graphCreateBranch` | `c` | `CreateBranchOverlay` → `createBranchAt` (ref only).                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `graphMerge`        | `m` | Boxed confirm, then merge the focused commit into the checkout's current HEAD. Local / `origin/*` names when present; tags and unlabeled commits use the commit id. `git merge --ff-only`, else `git merge --no-ff --no-edit` (no rebase). Dirty tracked worktree refuses (`Dirty worktree — commit or stash first`) before the overlay. Conflicts stay uncommitted. Linked worktrees only when that checkout row is focused. Tree / stash / uncommitted `m` stays mouse-capture toggle.                                                                                                                                 |
| `stashMenu`         | `S` | Opens stash overlay (`stashMenuMode`) with ops from `stashOpsForContext`. Overlay keys `s`/`a`/`p`/`d`/Enter/Esc are not registry. Push uses `stashPush` (`-u`; whole-tree). File/dir pathspecs are depth 0 only. Apply/pop/drop then invalidate + refresh. Overlay paints `statusMessage` (Busy… / Ctrl+C).                                                                                                                                                                                             |
| `stashApply`        | `a` | Graph stash row accelerator: `stashApply` + invalidate + refresh.                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `stashPop`          | `p` | Graph stash row accelerator: `stashPop` + invalidate + refresh.                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `stashDrop`         | `D` | Graph stash row accelerator: `StashDropConfirm` then `stashDrop` + invalidate + refresh. Overlay `d` shares this confirm.                                                                                                                                                                                                                                                                                                                                                                                |

`stashMenu` is also valid at depth 0 on dirty `repo` / `checkout` / `dir` / `file` (left pane). Hidden on workspace, group, family, depth 2, and the right pane. `actionVisibleForScope` shows it at depth 0 only when the focused scope is dirty; at depth ≥ 1 the graph-row filter (`actionVisibleForGraphRow`) decides. Graph: uncommitted → push; stash row → apply/pop/drop (and push if dirty); commit → push if dirty, apply/pop latest if stashes exist. Drop never targets “latest” from a non-stash row.

Depth 0 **tree** `b` stays ActionId `branch`; depth 0 **graph** `b` is `graphCheckout`. Graph `m` is `graphMerge` (always confirms).

## Mouse reporting and hit-testing

`mouse.ts` owns SGR enable/disable sequences (`MOUSE_ENABLE` / `MOUSE_DISABLE`), `parseMouseChunk` for incremental CSI `<…M/m` decoding, `mouseListPressAction` for left-press fold / select / navEnter / ignore mapping, and `mouseClickFocus` for column focus (`diff` → right, `tree` → left). Mode 1002 reports left-button drag motion as SGR button `32` (`0+32`) with final `M` → `{ button: 'left', action: 'drag', x, y }`.

`SessionState.mouseEnabled` (default `true`) is flipped by `m` / `toggleMouse`. `App` writes the enable sequence when the flag is on, disables on toggle-off and unmount, and attaches a raw `stdin` `'data'` listener only while enabled. Incomplete CSI tails stay in a buffer ref; keyboard bytes that are not an escape prefix are dropped from that buffer so they do not accumulate.

`hitTest.ts` maps 1-based `(x, y)` onto a `HitTarget` (`tree` / `graph` / `commitFiles` with optional `rowIndex`, `diff`, `divider`, `diffSplit`, `status`, or `none`) using the same frozen layout widths `App` derives from `paneWidths(termCols, treeFraction)` (stored in a layout ref; updated on terminal resize, divider drag, or fraction change — B1). Each column passes a `HitListLayout` (`kind`, viewport cursor/rows, graph/commit-detail `headerLines` + `footerLines` chrome, `listHeight`) so clicks land on the list that is actually painted (depth-0 tree, depth-1 graph, depth-2 commit files, right-pane graph / commit files / diff). List hits keep the pane kind on title/header/footer/empty slots (`rowIndex: null`) so the wheel still moves that list; only a concrete `rowIndex` selects. The divider band is columns `treeWidth` and `treeWidth ± 1` (clamped to valid cols), checked **before** left/right mapping. Label length never feeds hit columns. `treeViewportStart` / `visibleTreeWindow` are shared with `TreePane` / `GraphPane` so click rows match the painted viewport window (B2). Fold-chevron column (tree + commit files only): after left pad, content col `1 + depth*2` (cursor bar + indent).

Dispatch helpers on `useAppState` (preferred over fake keypresses): `selectRow` / `selectGraphRow` / `selectCommitFileRow`, `toggleFoldAt` / `toggleCommitFileFoldAt`, `focusPaneSide`, `scrollDiffBy`, `moveTreeCursorBy` / `moveGraphCursorBy` / `moveCommitFileCursorBy` (plus `moveCursorBy` for focus-routed keyboard parity). Wheel follows the pane under the pointer: list → ±1 cursor; diff → ±3 scroll. Left press selects (graph spacers snap to the paired parent via `selectableGraphIndexFromClick`) or toggles fold on the chevron. **Clicking the diff** focuses the right pane (`mouseClickFocus('diff')` → `'right'`). Lists still focus their side. **Double-click** a concrete list row (tree, graph, or commit files, not the chevron) selects that row, focuses the pane, then dispatches `navEnter` (same Enter ladder). Diff double-click does not drill. Graph rows record `ClickMemory` so the second press can be a double-click. `mouseListPressAction` in `mouse.ts` maps `{ pane, rowIndex, foldChevron, doubleClick }` to `fold` / `select` / `navEnter` / `ignore`. Left press on the divider starts a session-only resize drag; subsequent `drag` (and press) events follow `x` via `treeFractionFromWidth` until left **release** (list click/fold suppressed while dragging). When the right pane is a side-by-side diff, the in-diff RULE uses the same ±1 grab band and press/drag/release loop (`diffSplit` hit, `diffSplitFractionFromTerminalX`). Modal gates clear an in-progress drag. Neither split is persisted — next launch resets the pane split to 40% and the in-diff split to 50%.

If a mouse CSI also reaches Ink `useInput`, `App` ignores inputs containing `\x1b[<` / `\x1b[M` so they do not become garbage keystrokes.

## Layout freeze and tree virtualization (P9)

| Concern                    | Mechanism                                                                                                                                                                     |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Divider jump (B1)          | `layoutWidths.paneWidths(termCols, sessionFraction)` → App `widths` state; `flexShrink={0}` on the tree box; never widen from row content; drag updates session fraction only |
| Large-tree flicker (B2/B5) | `TreePane` renders only `visibleTreeWindow` rows; `TreeRowView` is `React.memo` with `key={row.id}`; no `key={cursor}` on the pane root                                       |
| Cursor hygiene (B5)        | `j`/`k` / `moveCursorBy` update cursor (+ diff scroll) only — flatten memoized on `[tree, folds]`; painted `rows` also merge ghosts (`[allRows, ghosts, clock]`)              |

## Session state and the render loop

Reviewed marks (`space` / `toggleViewed` in `viewedFiles.ts`) are not session state. They persist in `$XDG_STATE_HOME/my-workspace-status/viewed-files.json` (override `WS_STATUS_VIEWED_STORE`). Identity is the normalized repo path + file path. Fingerprint is SHA-256 of staged/unstaged/untracked status plus worktree bytes (`missing` when deleted; size-only above 1 MB). A mark stays after TUI restart when that pair still matches. Content or status change (including stage/unstage that changes the status token) drops it. Space again unmarks while contents are unchanged. Only dirty workspace-tree file rows (depth 0) accept the key or the trailing eye (`ICON_VIEWED` `` / `*`, cyan/blue). Space on any other row is a no-op and does not fold. The clean check (`ICON_CLEAN` `` / `.`) paints only on repo / checkout rows inside `group:no-updates`. Rows outside that section omit the up-to-date check even when sync is current — including folders under a dirty repo.

`SessionState` (`session.ts`) holds restorable view state: cursor row id, folds, filter, diff mode, full-context ids, tree mode, `mouseEnabled`, `theme`, `nav` (ViewStack + `focusPane`), and `diffColOffset` (horizontal pan; Track D). Forward-compat stubs also seed `graphWindow` (300), `graphCacheEpoch` (0), `search` (`null`), and `easyMotion` (`false`). It lives in `run.ts` outside any single Ink mount. Phase 4 wires `fullContext`: membership means unlimited unified context (`FULL_DIFF_CONTEXT_LINES`) for that file id — toggled by `ctrl+o`, cleared by `Esc` or a second `ctrl+o`. The id is `fullContextToggleId` (commit-file when that list is focused or depth ≥ 2, else the workspace-tree file) so Esc at depth 2 clears the same file `ctrl+o` toggled. Full-context Esc clear runs in `dispatchInput` **before** `handleKey`, so the first Esc clears full-file view; only a subsequent Esc becomes `navEsc`. After a full-file toggle, Track D recenters scroll on the prior hunk/change anchor.

## Navigation shell (JBY-037 P1)

Track A nav shell (A2–A5): a pure ViewStack + `focusPane` model, Enter/Esc ladder, display-only breadcrumb, registry hint dims, and a right-pane host. The graph engine (`src/tui/graph/**`) is wired in P3 (see below).

### `NavState` / `ViewDepth`

Defined in `src/tui/nav/stack.ts`:

```typescript
type ViewDepth =
  | { kind: 'workspace' }
  | { kind: 'repoGraph'; repo: string; commitId: string | null }
  | { kind: 'commitFiles'; repo: string; commitId: string; filePath: string | null };

type FocusPane = 'left' | 'right';

interface NavState {
  stack: ViewDepth[]; // length always 1..3
  focusPane: FocusPane;
}
```

`navDepth(nav)` is `stack.length - 1` (0 | 1 | 2). `createNavState()` starts at workspace + left focus. `useAppState` holds `nav` in React state, applies transitions via `applyNavEnter` / `applyNavEsc`, and persists `nav` through `onSessionChange` so editor remounts restore the stack.

### Enter / Esc ladder

| Focus / depth         | Enter                                                                                   | Esc                                            |
| --------------------- | --------------------------------------------------------------------------------------- | ---------------------------------------------- |
| Left (any depth)      | Focus right (same stack)                                                                | Pop one depth; stay on left (no-op at depth 0) |
| Right, depth 0        | Push `repoGraph` when drill `repo` is set; stay on right; else no-op                    | Unfocus → left                                 |
| Right, depth 1        | Push `commitFiles` (drill/graph `commitId`, else stack, else `WORKTREE`); stay on right | Unfocus → left                                 |
| Right, depth 2 (leaf) | No-op                                                                                   | Unfocus → left                                 |

Enter that deepens the stack keeps **right** focus; Esc that pops a depth keeps **left** focus.

Only double **Ctrl+C** quits (first press prompts `Press Ctrl+C again to exit`; second within `CTRL_C_EXIT_MS` / 2s exits — same pattern as Claude / Cursor). `q` is intentionally unbound. Esc never quits.

**Overlay precedence** (unchanged, runs before nav): filter / branch / create-branch / graph-branch / stash-menu / stash-drop / confirm / help own Esc and Enter. Stash-menu keys run in `useAppState` while `stashMenuMode` is set (`handleKey` returns `none`). Drop confirm paints above the menu (the menu is already closed). Help may nest a `/` search (`helpSearchQuery`); Esc clears that query before closing help. Full-context Esc clear in `dispatchInput` also precedes `navEsc`. Chord Esc cancels pending prefixes.

Drill context comes from `drillContextFromFocused` / `drillContextFromGraph` (`src/tui/nav/drill.ts`) — tree rows at depth 0, graph selection at depth 1 right+Enter.

### Breadcrumb

`Breadcrumb.tsx` is display-only chrome above Confirm / BranchPicker / StatusBar (hidden during help). It mirrors `breadcrumbSegments(nav, workspaceLabel)` with `›` separators and styles the current segment by `focusPane`. Not focusable — Esc remains the sole back key. `statusLines` adds +1 for the breadcrumb row whenever it is shown.

**Op-status (right):** `formatTopOpStatus` fills a trailing fragment on the same row — primary slot is in-progress pull/push/default-branch **or** fetch age/progress, then up to three ephemeral toasts from `statusMessage` (`setStatusMessage('')` clears; non-empty replaces with `[msg]`). In-progress multi-repo ops use the same `Verb done/total…` shape as fetch (`Fetching 2/18…`, `Pulling 2/18…`, `Pushing 1/3…`, `Switching 0/5…`) via `actionOpProgress` / `onProgress`. `CTRL_C_EXIT_PROMPT` is never a breadcrumb toast — `exitPromptPinned` owns that UX (except overlay pickers, which render `statusMessage` inline). Fragments join with `·`. StatusBar keeps mode pills + key hints only (never swaps the hint slot for status text). Overlay pickers still render `statusMessage` inline.

### Registry dims + status hints

`ActionSpec` may optionally declare `depths` and/or `focusPanes`. Omitted ⇒ valid at all depths / both panes. Keymap gating stays `actionFor(key, kind)` (row-kind only); `kind` is `activeRowKind`. StatusBar uses `actionsForContext(kind, depth, focusPane)` plus `navChromeHintSegments` (Enter/Esc chrome labels prepended to action hints). Tree-write ids are then dropped when `treeWritesHiddenForContext` is true (depth ≥ 1, or `focusPane === 'right'`) so the bar does not advertise keys `runAction` will no-op. Graph writes on the right are different specs; `e` / `ctrl+o` stay on a focused file diff.

The bottom bar stays **row/pane-scoped** actions + nav chrome + `? help`. Do not dump every global key (`j`/`k`, EasyMotion, `/`, `t`/`i`, `gg`/`G`, …) onto the hint bar — those live in the `?` overlay (`HELP_GROUPS` in `StatusBar.tsx`).

**Legend polish (B7):** hint-bar and `?` help keep key chips visually distinct from labels — inverse/pill chip (`key` on cursor or danger background) then ≥2 columns of gap, then a muted description. Destructive actions keep danger colour on chip and label. `HintSegment` is `{ key, label, destructive }`; fit width budgets pill pad (`key.length + 2`) + `HINT_CHIP_GAP` (2) + label. Help overlay columns share the panel inner width (three groups, not a fixed 40). Descriptions word-wrap under the chip pad; `helpStatusLines(termCols)` counts wrapped body + footer so panes shrink instead of overlapping. Help `/` highlights every visual line of a matching entry with the filter/search pill background without hiding non-matches.

### Graph in UI (JBY-037 P3)

| Depth               | Left             | Right                            |
| ------------------- | ---------------- | -------------------------------- |
| 0, repo/dir focused | Workspace tree   | Multi-branch graph (`GraphPane`) |
| 0, file focused     | Workspace tree   | `DiffPane` (unchanged)           |
| 1 `repoGraph`       | Graph list       | Commit detail + file tree (P4)   |
| 2 `commitFiles`     | Commit file tree | Commit-scoped `DiffPane` (P4)    |

- Load: `createGraphCache` + `loadGraphModel`; window = `session.graphWindow` (300).
- Autoload: when the graph cursor reaches the last loaded row and `hasMore`, `autoloadNext` appends the next window; status shows `loading older…`. In-flight gating uses refs + a generation counter (not `graphLoadingOlder` in effect deps) so setting the loading flag cannot cancel-and-stick the only request.
- Lane colours: `resolveLaneColors(activeTheme)` applied **per graph cell** (pipe/node), not one colour for the whole gutter string.
- Gutter paint: `layoutCommits` builds a cell matrix via an internal directional connection model (`docs/git-graph-topology.md`) — commit/merge `●`, HEAD `⊙`, uncommitted `○`, junctions `│─╭╮╰╯├┤┬┴┼` (ASCII via `WS_STATUS_GLYPHS`). Same-row horizontals connect merge/fork swim lanes; rows padded to a stable window width. First parent always keeps a waiter on the commit lane (duplicate parent ids across lanes are intentional until the parent row joins them). Incoming duplicate waiters close with `╰─` / `─╯` on the parent row. After each row, **active lanes densify left** and secondary parents open to the **right** of the commit lane so joins do not leave a permanent blank sibling column. **HEAD** (from `loadGraphModel` → `headId` + checkout branch from sync chrome) paints `⊙` / ASCII `@` on the node (including merge-at-HEAD). On a named branch the checkout chip is prefixed with Nerd Font crosshairs / ASCII `+` (no separate `[HEAD]`); detached HEAD keeps a bold `[HEAD]` chip. Synced local+remote chips put Nerd Font exchange / ASCII `=` before the branch name (PUA icons — misc unicode `⌖`/`⇄` bleed in MesloLGM). Stash rows are **side leaf tips** off `stash^1` — same topology as a **one-node side-branch tip**, glyph `◇` / `s` (normative grammar + S0–S7 in `docs/git-graph-topology.md`). List order **parks** each stash immediately above its `parentId` (not chrono-interleaved by stash date); orphans (parent outside window) sit after uncommitted. Never land `◇` on a live DAG lane; join at `stash^1`. Subject has no leading diamond. Commit spacers densify to the next laid-out commit even when a parked stash sits between them. Adjacent commits after a densify remap get elbows on the commit spacer via `stashRailCells` (helper name is historical — densify is commit↔commit only).
- Row text (layout A): commit row 1 = gutter + subject; stash row 1 = leaf-tip gutter + subject (no diamond in subject text); spacer row 2 = rails (commits) or live-gap rails + **short leaf spur** toward `stash^1` when parked (orphans: through-rails only) + chips/`stash@{n}` left + right-anchored hash / date / author. Pane width drives the flex budget (not a hardcoded 80); leftover branch/tag chips collapse to a bold `[+N]` overflow chip so the row does not grow. Gutter uses a hybrid cap (`≤30%` of list width and leave ≥24 cols for subject) with commit-lane-anchored clipping when topology exceeds the cap (`gutterBudget.ts`). Focus paints the full 2-row pair.
- GraphPane chrome: 2-line selection footer when height ≥ 3 (`graphChromeBudget` / `graphSelectionDetailLines`; footer preferred, header drops first / omitted when no sync snap). Uncommitted: `Working tree clean` / `Uncommitted changes` + HEAD commit ref chips (Ink `graphRefChipSegments` on the HEAD commit), or `worktree · not a commit` when HEAD has none. Spacer: `…` + `connector · not selectable`. Stash: subject + `stash@{n} ·` hash `·` date. Commit: subject + ref chips `·` hash `·` author `·` date, or `(no refs)`. Empty: `no selection`. 1-line sync header (`branch` + `tuiSyncMark`) when remaining space allows. Graph PageUp/PageDown move in painted `GraphListRow` space (`applySelectableGraphPageMove`: `visible − 1` painted lines, snap to selectable); `j`/`k`, EasyMotion, and click stay on selectable `visible_rows`.
- Invalidate: `r` refresh, fetch completion, watch changes, checkout / create-branch / stash push·apply·pop·drop — `invalidateRepo` / `clear` + bump `graphCacheEpoch`.
- List focus: depth 0 right (graph) and depth 1 left drive `graphCursor`; depth 1 right and depth 2 left drive `commitFileCursor`; tree cursor unchanged underneath at depth 0. Depth 2 right (and depth 0 file DiffPane) resolve to `'none'` — `j`/`k` scroll the diff via `scrollDiff`, not a list cursor. Graph cursor is a numeric index: a new right-pane dataset (repo switch, first paint, or commit-id identity change that is not an autoload suffix) resets to the first selectable row. Same-view rebuilds (width/theme, autoload older, identical window) keep the nearest index. Commit-file cursor already resets on new `commitFileListKey` / repo / tree-mode. Diff scroll resets when `focusRepo`/`focusPath` changes (Ctrl-o keeps `pendingScrollAnchorRef`). Left-tree restore stays id-based (`focusIdRef`).
- Graph write actions `b`/`c`/`a`/`p`/`D` fire whenever the graph list is focused (depth 0 right or depth 1 left). `S` is left-pane only (depth 0 dirty tree + depth 1 left graph).

### Commit file trees (JBY-037 P4)

Depth 1 right shows `CommitDetailPane`: meta header (repo + commit/stash/uncommitted subtitle) plus an embedded file `TreePane`. Depth 2 left is that file list; depth 2 right is `DiffPane` for the selected file. Leaf Enter at depth 2 remains a no-op; `e` / `ctrl+o` work on the focused commit-file row **and** on that file's DiffPane when the diff is focused.

**Source mapping** (`commitFileSourceFromNav` in `src/tui/commitFiles/resolveSource.ts`):

| Graph / view                                | `CommitFileSource`             |
| ------------------------------------------- | ------------------------------ |
| Uncommitted row, or `commitId === WORKTREE` | `{ kind: 'worktree' }`         |
| Stash row                                   | `{ kind: 'stash', stashRef }`  |
| Commit row / `commitFiles` hash             | `{ kind: 'commit', commitId }` |

**List identity / cursor:** the commit-file **loader** and forest rematerialize key off `commitFilesListKey(repo, source)` (`src/tui/commitFiles/identity.ts`) — not the raw `nav` object. Depth-2 breadcrumb sync that only mutates `filePath` therefore does **not** re-fetch or reset `commitFileCursor`. Forest nodes rebuild when `commitChanges`, `commitTreeMode`, or the list key changes; cursor/folds reset **only** when the list key or `commitTreeMode` changes (not on a same-identity `commitChanges` refresh). On that same-identity refresh, `resolveListFocus` keeps the highlighted file by id (unfolding folded parent dirs when the file is hidden). If the file is gone, focus falls back along `focusAncestorIds`.

**Right-pane reset (new view/rows/data → top):** the right column does not keep a previous scroll or active row when its dataset identity changes — repo, pane kind (`rightPaneMode`: graph / commitMeta / diff / empty), graph contents identity (`GraphFlashMeta` commit-id window), or commit-file list key. `shouldResetGraphCursor` / `graphCursorAfterRowsReload` in `graph/list.ts` snap `graphCursor` to the first selectable row on a new graph dataset (first paint, repo switch, or non-autoload commit-id change) and keep `nearestSelectableGraphIndex` on same-view rebuilds (width/theme, autoload older, identical window, stale paint). Commit-file reset on new list key is unchanged. Diff scroll still resets on `focusRepo`/`focusPath` change; `Ctrl-o` still uses `pendingScrollAnchorRef`. Left-tree restore stays id-based (`focusIdRef` / `resolveFocusAfterRebuild`).

Loaders: `listCommitNameStatus` / `listWorktreeNameStatus` / `listStashNameStatus` → `FileChange[]` → `buildCommitFileNodes` / `flattenCommitFiles`. Diffs: worktree reuses `diffFile` + `diffCachedFile` (+ untracked synthesis); commits use `diffCommitFile`; stashes use `diffStashFile`. Unified text is placed in the **unstaged** DiffPane slot for commit/stash.

**`commitTreeMode`:** dedicated boolean, default **`true` (tree)**. Independent of workspace `treeMode`. At depth ≥ 1, `t` emits `toggleCommitTreeMode` (not workspace `toggleTreeMode`). `t` is a view-mode like `i` — it works when the right pane is focused. Status bar mode label follows `commitTreeMode` when `navDepth >= 1`.

**B10 emphasis** on commit (and workspace) file lists: selected row = left-edge `CURSOR_BAR` + `cursorBg` only (`listRowBackground` for commit-file lists; workspace tree uses `treeRowEmphasis`). Status letter colours on file segments stay sacred — never wash the whole label with cursor foreground.

Parsers live in `src/nameStatus.ts` (shared so `git.ts` does not import `tui/`); `src/tui/commitFiles/parseNameStatus.ts` re-exports for callers.

## Themes

Built-in colour presets live in `theme.ts` (`tokyo-night`, `monokai`, `dracula`, `gruvbox-dark`, `catppuccin-mocha`). React panes read via `ThemeProvider` / `useTheme()`; non-React builders (`tree.ts`, `icons.ts`) call `getTheme()` at colour-use time so a mid-session cycle refreshes segment colours. `T` advances `SessionState.theme`, calls `setActiveTheme`, and rebuilds the tree (cursor restored by id). There is no user theme file — only the five presets and `WS_STATUS_THEME` for the launch seed.

`runTui` wraps the mount loop in `withAlternateScreen` (DEC 1049 via `alternateScreen.ts`): enter on TTY before the first mount, leave in `finally` on every exit path (including thrown render/wait errors), and leave/re-enter around a blocking TTY `$EDITOR` so vim sees the primary buffer. GUI editors stay mounted. Leave writes DECRST 1049 plus show-cursor, and `beforeExit`/`exit` listeners call leave once (idempotent; listeners removed on leave) so a hard process exit still restores the primary buffer like Vim/less.

PageUp/PageDown: Ink's `key.pageUp`/`pageDown` are preferred; `pageKeyFlagsFromInput` in `keys.ts` also maps raw CSI (`\x1b[5~` / `\x1b[6~` and legacy `\x1b[[5~` / `\x1b[[6~`) when those flags are missing, and `App.tsx` ORs them into the dispatch flags.

`e` has two launch paths (`isDetachedEditor` in `editor.ts`, `startEdit` in `editorLaunch.ts`):

- **Detached GUI** (`cursor`, `code`, `code-insiders`, `codium`, `gvim`, including `--wait`): spawn with `stdio: 'ignore'` + `detached` while Ink stays mounted. Fold, focus, scroll, search, and `showIgnored` stay in React state. File-status updates come from the live watch poll. This avoids the remount that used to fire as soon as the Cursor CLI returned.
- **Blocking TTY** (vim, nvim, nano, and unknown names): `onEditRequest` + unmount-as-quit, then the `runTui` remount loop below.

`runTui` loops (TTY editors only):

1. Mount `App` with the current `session` (already on the alternate screen when stdout is a TTY).
2. Await `waitUntilExit()`.
3. If there is no `pendingEdit`, return (plain double Ctrl+C quit, hang-up) — `finally` leaves the alternate screen and restores prior primary contents.
4. Otherwise leave the alternate screen, resolve config `editor` then `$EDITOR` / `$VISUAL` (fallback `vim`), parse into argv (command + fixed args; see `parseEditorArgv`), call `prepareTerminalForEditor` (cooked stdin, pause Node's reader, disable mouse, drain leftover bytes), spawn with `stdio: 'inherit'` in the file's repo cwd, call `restoreTerminalAfterEditor` so the remounted Ink instance receives the next keypress without an extra Enter, re-enter the alternate screen, then remount. Spawn failures log to stderr and the loop remounts.

Why this shape: handing the terminal to an interactive TTY editor requires unmounting Ink. List focus and folds cannot live only in React state, so `onSessionChange` pushes them upward each render and the next mount restores the workspace tree by stable id (`resolveFocusAfterRebuild`, including ancestor fallback). Commit-file cursor is not in `SessionState`; `resolveListFocus` applies to live commit-file list refreshes, not editor remount. After `$EDITOR` exits, `run.ts` calls `restoreTerminalAfterEditor` (raw mode + stdin resume) before the next `render` — without that, many terminals leave stdin cooked/paused and swallow the first keystroke. The alternate-screen leave/re-enter around the editor keeps `$EDITOR` on the primary buffer while still restoring scrollback when the TUI finally exits.

If Node keeps stdin in raw/flowing mode while vim inherits the fd, parent and child both read keypresses and vim can drop characters. `prepareTerminalForEditor` closes that gap before spawn. Remaining swallows after that hand-off are outside this process (IME, tmux, the editor itself).

`edit` uses `onEditRequest` + unmount-as-quit for TTY editors only, not `onExit({ type: 'edit', ... })`. Writing an edit exit reason from the action left a window where Ctrl+C after `e` still launched the editor once a loop landed. Recording `pendingEdit` and unmounting immediately closes that window; the loop keys off `pendingEdit` after exit, and App still reports `{ type: 'quit' }` for the unmount itself.

Backspace is not modelled in `keys.ts`; `App.tsx` translates Ink's `key.backspace` into a literal `\x7f` before dispatching when pane search, branch/create/graph-branch overlays, or help-local `/` search (`helpSearchQuery !== null`) is active.

## Live refresh

`watch.ts` polls; there is no `fs.watch`. The rationale is in the module header: 30+ repos on a WSL2 mount exhaust inotify watches and drop events.

Per tick (`useAppState.ts`):

1. Skip entirely if `busyRef` is set — a git write or an earlier tick is in flight.
2. Re-run `collectSnapshotsWithConfig` (the same call the `r` key uses).
3. File signatures: `changeSignatures(cwd, snapshots)` → `Map<fileNodeId, "<status>:<size>:<mtimeMs>">`. Combining the status letter with the worktree mtime catches both `M → MS` (staged from another terminal) and an in-place edit of an already-modified file. Files missing from disk get `:gone`.
4. Chrome signatures: `treeChromeSignatures(tree)` walks the rebuilt tree and **skips file nodes**. Workspace is `changeCount|syncSummary`; repo/checkout is `branch|sync|syncStatus|changeCount|mergedIntoDefault|checkoutKind|childKindCount` (`childKindCount` = `children.length`, so family worktree count lives there); dir/group is the sorted child-id set. `mergeSignatures(fileMap, chromeMap)` unions those maps without dropping mtime-only file entries.
5. `flashableNodeIds(before, after)` unions new/altered ids (`changedNodeIds`) with disappeared ids (`removedNodeIds`). Removals stamp the flash map and keep a **ghost row** (last visible `VisibleRow` + its list index) re-inserted in place for `FLASH_MS` so the row flashes where it was — not off-screen at the list end. Repo / dir / checkout removals use the same `removalGhosts` / `mergeGhostRows` path once those ids are in the signature map. The same chrome runs on stage/discard/`r` refresh (`applySnapshotsWithChrome`) so action-driven removals are not wiped before a ghost can be captured.
6. State is only replaced when something flashable changed, or when the repo fingerprint (`repo|branch|syncStatus|syncNote`) or signature-map size moved. Otherwise the diff cache would be cleared on every tick.

The baseline is seeded once from the boot snapshots **and** the boot tree's chrome signatures, so the first tick does not flash every file or repo in the workspace. Toggling tree/flat mode reseeds chrome (keeping file mtime entries) so directory rows are not treated as newly added data.

Flash decay: `flashStrength(flashedAt, now)` eases 1 → 0 over `FLASH_MS` (800). `flashBackground(strength)` maps that onto the active theme's `flashRamp` in `theme.ts` — four progressively dimmer greens — so a changed row fades out instead of blinking off. Ghosts are pruned on the same timer. A second interval (`FLASH_MS / 8`, floor 120 ms) bumps a wall-clock `clock` stamp (same domain as `flashedAt`) so TreePane and GraphPane can recompute strength without calling `Date.now()` on every cursor move; the timer stops once the flash map and ghost lists are empty so an idle TUI draws nothing. `pruneFlashes` / `pruneGhosts` keep those structures bounded.

**Graph list flash:** `graphRowSignatures(rows, model)` in `graph/list.ts` signs selectable rows from the **model** (commit `subject|sortedRefNames|isHead`, stash `subject|parentId`, uncommitted `hasChanges`) — never painted segments, so pane width / theme / gutter ripples do not flash. Signature keys are `graphRowIdentity` (`<repoPath>#` + pair id from `graphRowFlashId`) so the same sha / `stash@{n}` / uncommitted row in another repo is a different row. Signatures live in `graphSignaturesRef`, not the tree `signaturesRef` (graph ids would confuse tree ghosts). `GraphPane` shares the same `flashes` map + `clock`; spacers follow their paired commit/stash via `graphRowIdentity`. `graphRowFlashIds` flashes nothing when the two maps share no identity (repo switch, first paint, or a wholly different commit set) — that is a new list, not added/removed rows. Seed also on empty signatures, `graphFlashDecision` repo switch, or first non-empty paint. Skip signature/flash updates while the painted model is for a different repo than focus (depth-0 `j`/`k`). Autoload (same skip/limit, next commit ids are prev plus an older suffix) flashes same-identity changes + removes only (`includeAdds: false`); new tips prepend ids and still flash adds. Watch / invalidate reloads flash same-identity adds + changes + removes. Disappeared selectable graph rows (and paired spacers) stay visible as `graphGhosts` for `FLASH_MS`. Diff hunks never flash. Commit-file `TreePane` (left depth-2 and `CommitDetailPane`) also receives the shared flash map; file ids already match `file:repo:path`.

**B9 op-row flash:** after a successful fetch / pull / push / default-branch batch, `flashNodes` stamps `repo:<path>` ids (via `repoNodeId`, paired with `makeRepoNode`) so the repo row gets the same background flash as file changes. Watch ticks now also flash those rows when chrome signatures move (branch / sync / merge mark / changeCount / checkoutKind / family wt count), not only after an explicit op.

**B10 tree chrome:** `treeRowEmphasis` paints selection as left `CURSOR_BAR` + `cursorBg` only, and flash as `flashBackground` only — status / M·A·D foregrounds stay sacred (no select bold wash).

`WS_STATUS_WATCH_MS=0` disables the poll loop entirely.

## Action registry

`src/tui/actions/registry.ts` is the single source of truth for which actions exist and which row kinds accept them. The keymap resolves keys through it and the status bar renders hints from it, so what the user is told and what actually fires cannot drift apart.

Each entry is an `ActionSpec`:

| Field         | Meaning                                                                           |
| ------------- | --------------------------------------------------------------------------------- |
| `id`          | Stable `ActionId` the dispatcher switches on                                      |
| `key`         | Single input character, or `ctrl+o` for the control chord                         |
| `label`       | Short text shown in the hint bar                                                  |
| `kinds`       | Row kinds (`workspace`, `repo`, `group`, `dir`, `file`) where the action is valid |
| `destructive` | Routes through the confirmation flow (only `revert` today)                        |
| `depths?`     | When set, hint-valid only at these ViewStack depths; omitted ⇒ all depths         |
| `focusPanes?` | When set, hint-valid only for these panes; omitted ⇒ both                         |

Registry order is hint-bar order — most-used actions first. `actionFor(key, kind)` returns the spec bound to a key for the highlighted row, or `undefined` when the key is not an action or is not valid there. When the terminal sends a lowercase letter and that letter is **not** itself a registered action key, `actionFor` / `graphActionForKey` also try the uppercase form (so `w` fires `W` / remove-worktree; `d` does **not** steal `D` / stashDrop because `d` is already bound). `actionsForKind(kind)` returns every valid action in registry order (kind-only). `actionsForContext(kind, depth, focusPane)` additionally filters by optional `depths` / `focusPanes` for the status hint bar. Graph kinds (`graphCommit` / `graphStash` / `graphUncommitted`) plus `actionVisibleForGraphRow` hide `b` when the selected commit has no local branch and no `origin/*` ref; `actionHintSegments` applies that graph-row filter only for those kinds. `graphActionForKey` resolves graph-list keys at depths 0 and 1 (no `focusPanes` requirement).

State-dependent tree gates live in `src/tui/actions/gates.ts` (`actionVisibleForScope`): hide `s`/`u`/`x` when the focused scope has nothing to stage/unstage/discard; hide `S` (`stashMenu`) at depth 0 unless the focused scope is dirty (depth ≥ 1 defers to `actionVisibleForGraphRow`); hide `p` unless relevant repo(s) are `behind`; hide `P` unless a bulk-git target (primary on a family row, or the focused checkout) is `ahead`/`diverged`/`no-upstream` (never on workspace); hide `d` when no bulk-git target is off the default; hide the same write set as `commitFilesWriteBlocked` at depth ≥ 1 (`stashMenu` is not in that blocked set). Hints and `useActions` dispatch share these predicates.

`group` rows (the "no file changes" bucket) accept no actions at all, so `actionsForKind('group')` is always empty.

Adding an action means two edits: one entry here, plus a dispatch case in the action dispatcher.

Write scope for stage/unstage/revert is `src/tui/scope.ts` (`collectFiles`). Bulk git writes (`f` / `p` / `P` / `d`) use `collectBulkGitTargets` in the same module: primaries on workspace/family rows; a linked worktree only when that row is focused; hidden ignored-list paths omitted even if focused. When ignored repos are shown (`.` / `-a`), they follow the same primary / focused-worktree rule. Registry `kinds` already allow `repo` / `dir` / `file` for those ids; the dispatcher filters collected files by staged/unstaged/untracked before calling git. Background fetch uses `collectBackgroundFetchTargets` and skips hidden ignored repos.
