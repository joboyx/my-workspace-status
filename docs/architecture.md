# Architecture

State of the code as it stands today.

## One snapshot, three renderers

```
src/index.ts ── collectSnapshotsWithConfig
             ├── buildWorkspaceSnapshot
             │      ├─► src/render.ts                 --plain
             │      └─► serializeWorkspaceSnapshot    --json
             └── src/tui/run.ts                       Ink TUI (same collected snapshots)
```

`--plain` and `--json` print the same workspace snapshot. Display differs. See [snapshot.md](./snapshot.md).

The TypeScript app still paints Ink from `src/tui/run.ts`.
The Rust binary paints a ratatui TUI from `crates/workspace-status/src/tui` on a TTY.
Both read the same snapshot builder. See [tui-rust.md](./tui-rust.md).

`main()` in `src/index.ts` picks with:

```ts
const wantTui =
  (flags.forceTui || (process.stdout.isTTY && !flags.forcePlain && !flags.forceJson)) &&
  !flags.verbose &&
  !flags.doPull &&
  !flags.doDefaultBranch;
```

| Input            | Effect                                                                                                  |
| ---------------- | ------------------------------------------------------------------------------------------------------- |
| stdout is a TTY  | TUI (the default) — **agents must pass `--plain` or `--json`** (TTY without one hangs on keyboard input) |
| `-i` / `--tui`   | TUI even without a TTY (humans only)                                                                    |
| `--plain`        | plain report (required for agent runs unless `--json`)                                                  |
| `--json`         | workspace snapshot JSON on stdout (progress from `-f`/`-p`/`-d` goes to stderr)                         |
| `-v`, `-p`, `-d` | plain report — these flags print progress logs mid-run, which cannot coexist with Ink owning the screen |

`src/tui/run.ts` is loaded with a dynamic `import()` so the plain path never pays for React/Ink/`cli-highlight`. On a TTY it wraps the mount loop with `withAlternateScreen` (`tui/alternateScreen.ts`, DEC 1049) so Ink frames stay off the primary scrollback; leave/re-enter brackets a blocking TTY `$EDITOR` (vim). GUI editors such as Cursor spawn detached and stay on the mounted TUI. Leave also shows the cursor and registers `beforeExit`/`exit` hooks so abrupt process exit still restores the primary buffer.

## Data pipeline

| Stage          | Module                                                                              | Produces                                                                                                                                                                 |
| -------------- | ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Config         | `config.ts`                                                                         | `WorkspaceStatusConfig` (`ignoredRepos`, `maxDepth`, `defaultBranches`)                                                                                                  |
| Discovery      | `discovery.ts` — `findReposWithConfig` + linked via `git worktree list --porcelain` | primary paths up to `maxDepth`, then linked worktrees under cwd (dot dirs still skipped by walk)                                                                         |
| Snapshot       | `discovery.ts` — `processRepo`                                                      | `RepoSnapshot` per path (`checkoutKind`, `primaryRepo?`, `mergedIntoDefault`) from status + merge-base probe                                                             |
| Workspace      | `snapshot.ts` — `buildWorkspaceSnapshot`                                            | Workspace snapshot (`docs/snapshot.md`): repos, ignore, branch/sync/checkout, file changes. `--json` prints it. Hidden ignored repos stay out of `repos` unless shown         |
| Aggregation    | `snapshot.ts`                                                                       | `SummaryState` (incl. `linkedWorktrees`), `VerboseRow[]` with `files` column + `🔗` / merge marks; bucket sort uses `compareRepoPathsForDisplay` (same adjacency as TUI) |
| Per-file parse | `changes.ts` — `fileChangesFromSnapshot`                                            | `FileChange[]`, one row per path, merged across staged/unstaged/untracked                                                                                                |
| Output         | `render.ts` or `tui/model/tree.ts` — `buildTree`                                    | plain report (`Files` header, Linked summary) or a `WorkspaceNode` tree                                                                                                  |

`RepoSnapshot` carries its file lists as `|||`-joined `STATUS\tpath` strings (`R\told\tnew` for renames), not arrays. That shape is the discovery↔render contract and is reconstructable in both directions (`fileChangesToSnapshotFields`). It exists because the original implementation was a shell script; it is a wire format, not a good in-memory model.

Porcelain v1 is pinned deliberately (comment in `processRepo`): v2 would change rename and XY handling and every parser downstream.

## Git subprocesses

All of them live in `src/git.ts` and run through zx's `$` **called from Node**, not the zx CLI shebang. Keeping the entry point an ordinary Node program means `tsc` output in `dist/` stays directly runnable and `workspace-status.sh` does not need zx on `PATH`.

`GIT_BINARY` is resolved rather than assumed:

```ts
process.env.WORKSPACE_STATUS_GIT ?? (fs.existsSync('/usr/bin/git') ? '/usr/bin/git' : 'git');
```

On WSL2 a Windows `git.exe` frequently shadows the Linux git on `PATH`; it produces Windows-style paths and is an order of magnitude slower. Preferring `/usr/bin/git` avoids that, and the env var gives tests a seam.

`execGit` swallows failures and returns `''`; `execGitStatus` returns the exit code. `execGitAsync` is the only one that throws — used for fetch/pull where the caller decides what a failure means.

## Concurrency

`concurrency.ts` exports one function, `mapWithConcurrency`. Repo work is bounded at 8 (`STATUS_CONCURRENCY`, `DEFAULT_BRANCH_CONCURRENCY`) and file stats at 16 (`STAT_CONCURRENCY` in `tui/watch.ts`). Unbounded `Promise.all` over 30+ repos spawns 30+ git processes at once and, on a WSL2 filesystem mount, is slower than the bounded version as well as being able to exhaust file descriptors.

`pullBehindRepos` is the exception — it uses unbounded `Promise.all` over `pullQuietDetailed` (network-bound, not CPU-bound).

## Live refresh and background fetch

`tui/watch.ts` polls file signatures (status + mtime) and tree chrome signatures (repo / checkout / dir / workspace / group) so those rows flash on semantic updates, including remove ghosts for `FLASH_MS`≈800, without a full remount. Graph list rows use a separate `graphSignaturesRef` (`graphRowSignatures` from the model, not segments) and stamp the same `flashes` map; `GraphPane` paints `flashBg` (spacers follow the paired commit/stash). Fetch / pull / push / default-branch completion also flashes `repo:<path>` rows. `tui/fetch.ts` plus `tui/useFetch.ts` schedule bounded `git fetch --quiet` batches (`WS_STATUS_FETCH_MS`, default 5 minutes; `0` disables) and power the manual `f` action; age and in-flight `Fetching done/total…` land on the trailing op-status. Pull / push / default-branch use the same `Verb done/total…` slot via `useActions` `actionOpProgress`.

The Rust TUI matches those two contracts in `crates/workspace-status/src/tui/watch.rs` and `fetch.rs`: file `row_signature` is status letter + `size:mtimeMs` (or `gone`), and `background_fetch_targets` is every snapshot except hidden ignored (linked worktrees included). Manual `f` stays on focus-scoped `op_targets`.

## Layout stability (JBY-037 P9 / B1 B2 B5)

Pane widths come from `layoutWidths.paneWidths(termCols, fraction?)` — never from tree label lengths. Default fraction is `TREE_WIDTH_FRACTION` (0.4). `App` keeps a session-only `treeFraction` (resets on next launch; not persisted) and freezes `{ treeWidth, treeInnerWidth, diffWidth }` in state, recomputing when terminal columns change (`SIGWINCH` / stdout resize) **or** when the user drags the divider — so the left/right divider does not jitter on `j`/`k`. Hit-testing consumes the same frozen `treeWidth`. Clamp helpers (`clampTreeFraction` / `treeFractionFromWidth`) keep both panes ≥ 20 cols (accounting for padding) when the terminal is wide enough. The in-diff side-by-side RULE uses the same session-only drag model via `diffSplit.ts` (`diffSplitFraction`, not persisted). Hit-testing consumes `diffSplitRuleX` (RULE ± 1) only while split mode is actually painted (`≥ NARROW_SXS`). The `?` help overlay height comes from `helpStatusLines(termCols)` (wrap math in `helpLayout.ts`) so wrapped descriptions shrink the panes instead of overlapping them.

`TreePane` paints a viewport window (`visibleTreeWindow` / `treeViewportStart`) with `React.memo` row views keyed by `row.id`. Cursor-only moves in `useAppState` call `setCursor` (and reset diff scroll) — they do not rebuild `tree`, `folds`, or snapshot identity; `rows` stays memoized on `[tree, folds]`.

## Repo graph engine (JBY-037 P2b) + UI (P3)

Pure library under `src/tui/graph/`, **wired in P3** via `graph/list.ts`, `GraphPane`, `RightPaneHost`, `useAppState`.

| Module                                                                                                                             | Role                                                                                                                                      |
| ---------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `graph/types.ts`                                                                                                                   | `GraphCommit` / `GraphRef` / `GraphStash` / `GraphUncommitted` / `GraphModel` / `LaidOutCommit`; `DEFAULT_GRAPH_WINDOW = 300`             |
| `graph/load.ts`                                                                                                                    | `loadGraphModel` — joins log window + refs + stashes + uncommitted + fingerprint                                                          |
| `graph/layout.ts`                                                                                                                  | `layoutCommits` — lane assignment + topology paint (`●`/`⊙` + connectors)                                                                 |
| `graph/topology.ts`                                                                                                                | Internal `up/down/left/right` connection model → junction glyphs                                                                          |
| `graph/glyphs.ts`                                                                                                                  | Unicode / ASCII glyph maps (`WS_STATUS_GLYPHS`); see `docs/git-graph-topology.md`                                                         |
| `graph/rows.ts`                                                                                                                    | Per-cell gutter colour + right-anchored hash/date/author columns                                                                          |
| `graph/rows.ts`                                                                                                                    | `graphCommitSegments` (+ stash/uncommitted); drop hash → date → author                                                                    |
| `graph/cache.ts`                                                                                                                   | `createGraphCache` keyed by `repoPath + refsFingerprint + skip + limit`; `shouldAutoload` / `autoloadNext`                                |
| `graph/laneColors.ts`                                                                                                              | Default tokyo-night lane palette; themes override via `resolveLaneColors`                                                                 |
| `graph/list.ts`                                                                                                                    | Flatten model → `GraphListRow[]`; visibility helpers (`shouldShowGraphDetail`, …); `graphRowIdentity` / `graphRowFlashIds`                |
| `graph/focus.ts`                                                                                                                   | `listFocusTarget` / `isGraphListFocused` — which list (or diff) the focused pane is                                                       |
| `activeContext.ts`                                                                                                                 | `activeRowKind` + write/fold/EasyMotion helpers shared by hints, keymap, and `runAction`                                                  |
| `graph/actions.ts`                                                                                                                 | Pure A8 gates: `checkoutableBranchNames` / `planGraphCheckout` / `runBusyThenRefresh` / create·stash predicates / `resolveCheckoutTarget` |
| `graph/rowKind.ts`                                                                                                                 | Map graph list selection → registry `RowKind` + `GraphActionRow`                                                                          |
| `GraphPane.tsx`                                                                                                                    | Ink list for graph rows (cursor bar + Segments + shared flash map)                                                                        |
| `CreateBranchOverlay.tsx` / `GraphBranchPicker.tsx` / `GraphCheckoutConfirm.tsx` / `StashMenuOverlay.tsx` / `StashDropConfirm.tsx` | A8 overlays (replace StatusBar while open)                                                                                                |
| `CommitDetailPane.tsx`                                                                                                             | Depth-1 right: meta header + commit file tree (P4)                                                                                        |
| `commitFiles/**`                                                                                                                   | Name-status parse re-exports, tree build/flatten, source resolve, meta                                                                    |

Git I/O for the engine lives in `src/git.ts` (`gitLogGraphWindow`, `listRefs`, `listStashes`, `computeRefsFingerprint`, `repoHasPorcelainChanges`, P4 `list*NameStatus` / `diffCommitFile` / `diffStashFile`, P5 `createBranchAt` / `stashPush` / `stashApply` / `stashPop` / `stashDrop`, `revParseQuiet` for graph checkout SHA compare).

Graph checkout confirm (and several other graph UX choices) is inspired by [Git Graph](https://github.com/mhutchie/vscode-git-graph) (mhutchie, VS Code).

## Where do I add X

| Change                | Touch                                                                                                                                                                     |
| --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| New git operation     | `src/git.ts` (add the wrapper), caller in `src/actions.ts` or `src/tui/useAppState.ts`                                                                                    |
| Graph engine / UI     | `src/tui/graph/**` + `GraphPane.tsx` / `CommitDetailPane.tsx` + `commitFiles/**` + helpers in `src/git.ts`; wired via `useAppState` / `RightPaneHost` / `App`             |
| New tree node kind    | `src/tui/model/types.ts` (`TreeNode` union) + `model/tree.ts` (`nodeSegments`) + `model/flatten.ts` (`hasChildren`) + `model/fold.ts` (`walkFoldable`, `createFoldState`) |
| New pane / overlay    | `src/tui/` component + `bottomChromeRows` (`src/tui/bottomChrome.ts`) + `App.tsx` layout arithmetic                                                                       |
| Nav shell / ViewStack | `src/tui/nav/` (`stack.ts` transitions, `drill.ts` row→context) + `Breadcrumb.tsx` + `RightPaneHost.tsx`                                                                  |
| New CLI flag          | `src/cli.ts` (`parseArgs`, `HELP`) + `src/types.ts` (`CliFlags`)                                                                                                          |
| Snapshot contract     | `src/snapshot.ts` (`buildWorkspaceSnapshot`) + `docs/snapshot.md` + `test/snapshot-contract.e2e.ts`                                                                       |
| New key binding       | `src/tui/keys.ts` (`Action`, `handleKey`) + `useAppState.ts` (`runAction`) + `StatusBar.tsx` (`HELP_GROUPS`) + `activeContext.ts` / registry if the key is row-scoped     |

Adding a node kind means editing four files that each `switch` on `kind`. TypeScript catches three of them exhaustively; `flatten.ts`'s `hasChildren` is a boolean predicate and will silently treat the new kind as a leaf.


## Rust CLI crate

`crates/workspace-status` is the Rust CLI (`workspace-status` and `ws`).
It implements discovery, `--plain`, `--json`, `-a`/`--all`, repo filters,
ignored-repo visibility from snapshot.md, and the ratatui TUI on a TTY.

Git calls use a subprocess. The binary prefers `/usr/bin/git` so WSL does not pick a Windows `git.exe`. Set `WORKSPACE_STATUS_GIT` to override.

`--fetch`, `--pull`, and `--default-branch` write progress to stderr when `--json` is set. `--json` wins when both `--json` and `--plain` are set. `-v` applies to `--plain` only.

On a TTY (or `-i` / `--tui`) the binary opens the ratatui TUI. Tree chrome (status letters, Nerd glyphs, workspace wording, linked-checkout labels, sync marks) is ported from Ink: `tui/icons.rs` (glyph registry), `tui/tree.rs` (`node_segments`), `tui/render.rs` (right-aligned trailing + cursor bar). Bottom chrome (mode pills, hint chips, breadcrumb) lives in `tui/chrome.rs`. Commit-file lists reuse the same file chrome. See [tui-rust.md](./tui-rust.md).

The TypeScript Ink app stays in this repository for features the Rust TUI does not cover yet.

## Rust graph crate

`crates/workspace-status-graph` is a ratatui widget for one git graph window.
The TypeScript Ink app still has its own graph paint. The ratatui TUI
loads a `GraphModel` in `graph_load.rs` (`log --exclude=refs/stash --all
--topo-order --date-order`, window 300, autoload, extra `stash^1`) and
paints `GraphWidget`. Hidden ignored worktrees stay out of `visible_rows`
unless `show_ignored` is true. Gutter cells use `GraphCell.color_lane`
and `DEFAULT_LANE_COLORS`. Chrome is a 2-line selection footer
(`selection_detail_lines`) plus optional sync header
(`graph_chrome_budget`). A loaded graph always emits the working-tree
row. Commit and stash spacers reuse the same hash / date / author drop
order.

See [graph.md](./graph.md).

## Decisions

**Ink over hand-rolled ANSI.** The TUI is stateful — fold sets, cursor identity tracking, an async diff cache, a poll loop, decaying flash timers. React's re-render-from-state model handles that; a hand-rolled ANSI renderer would mean owning diffing and cursor placement by hand for no gain. The cost is real: Ink does not re-render on `SIGWINCH`, so `App.tsx` tracks terminal size itself.

**Black-box tests against real temporary git repositories.** The plain report's output format is the user-facing contract (`SAMPLE_OUTPUT.md`). Mocked git would let porcelain parsing bugs through — rename arrows, `??` handling, `## branch...upstream [ahead N, behind M]` — which is exactly the class of bug that matters here. `test/workspace-status.e2e.ts` builds real repos, bare remotes, and collaborator clones per scenario.

**zx via Node, not the zx CLI.** See above: the built `dist/index.js` must run as a plain Node program.
