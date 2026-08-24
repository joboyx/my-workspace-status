/**
 * App state + action dispatch for the workspace-status TUI shell.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import path from 'node:path';
import { collectSnapshotsWithConfig, refreshRepoSnapshot } from '../discovery.js';
import { defaultBranchOverrideFor } from '../config.js';
import { getDefaultBranch } from '../actions.js';
import {
  FULL_DIFF_CONTEXT_LINES,
  checkoutBranch,
  computeRefsFingerprint,
  createBranchAt,
  diffCachedFile,
  diffCommitFile,
  diffFile,
  diffStashFile,
  fastForwardToRemoteRef,
  listCommitNameStatus,
  listLocalBranches,
  listStashNameStatus,
  listWorktreeNameStatus,
  repoHasLocalChanges,
  revParseQuiet,
  stashApply,
  stashDrop,
  stashPop,
  stashPush,
} from '../git.js';
import type { LocalBranch } from '../git.js';
import type { FileChange, RepoSnapshot } from '../types.js';
import type { ActionGateContext } from './actions/gates.js';
import {
  canToggleViewed,
  isTreeWriteBlockedAtDepth,
  TREE_WRITE_BLOCKED_IDS,
} from './actions/gates.js';
import type { ActionId, RowKind } from './actions/registry.js';
import { shouldForwardToUseActions } from './actionRoute.js';
import {
  activeRowKind,
  easyMotionListTarget,
  foldAllowed,
  fullContextToggleId,
  rightPaneLeftListAllowed,
} from './activeContext.js';
import { filterBranches, sortBranchesForPicker } from './branches.js';
import { isValidBranchName } from './createBranchName.js';
import { effectiveDiffMode, type DiffPaneContent } from './DiffPane.js';
import { diffModeToast } from './diffModeLabel.js';
import { fileMtimeKey, readUntrackedAsDiff, repoFileAbs } from './diff/newFile.js';
import { buildDiffRows } from './diff/rows.js';
import { applyPan, maxColOffset } from './diffPan.js';
import {
  anchorRowIndex,
  clampDiffScroll,
  diffScrollForMoveTo,
  scrollToKeepRow,
} from './diffScroll.js';
import { diffCacheKey, toggleFullContext } from './fullContext.js';
import type { Action, KeyFlags, KeyState } from './keys.js';
import { createKeyState, flushPending, handleKey, isLeftListAction } from './keys.js';
import { rightPaneMode } from './RightPaneHost.js';
import { applyPageMove, pageDelta } from './pageNav.js';
import {
  collectSearchMatchIds,
  firstMatchIndex,
  focusTreeSearchMatch,
  matchDiffRowIndices,
  matchIndices,
  nextSearchMatchId,
  stepMatch,
  type SearchLabeledRow,
  type SearchPaneTarget,
  type SearchState,
} from './search.js';
import { resolveEasyMotionJump } from './easyMotion.js';
import { visibleGraphWindow } from './GraphPane.js';
import { visibleTreeWindow } from './TreePane.js';
import { commitDetailHeaderHeight } from './CommitDetailPane.js';
import {
  checkoutableBranchNames,
  isOriginRemoteRef,
  localNameFromOriginRef,
  planGraphCheckout,
  resolveCheckoutTarget,
  runBusyThenRefresh,
  type GraphActionRow,
} from './graph/actions.js';
import { graphActionRowFromSelection, graphListRowKind } from './graph/rowKind.js';
import {
  buildStashOpsContext,
  resolveStashMenuKey,
  stashMenuSubtitle,
  stashOpsForContext,
  stashPushStatus,
  stashRepoRelPath,
  type StashOp,
} from './stashOps.js';
import { startEdit } from './editorLaunch.js';
import { resolveEditor } from './editor.js';
import type { EditRequest, ExitReason, SessionState } from './session.js';
import { focusAncestorIds, resolveFocusAfterRebuild, resolveListFocus } from './session.js';
import { THEMES, cycleThemeId, resolveLaneColors, segmentsText, setActiveTheme } from './theme.js';
import type { ThemeId } from './theme.js';
import { CTRL_C_EXIT_PROMPT } from './ctrlCExit.js';
import { formatTopOpStatus } from './opStatus.js';
import type { PendingConfirm } from './useActions.js';
import { useActions } from './useActions.js';
import { useFetch } from './useFetch.js';
import {
  applyFold,
  collectFoldableIds,
  collectFoldableSubtreeIds,
  createFoldState,
  unfoldForestAncestors,
} from './model/fold.js';
import { flatten } from './model/flatten.js';
import {
  applyViewedMarks,
  collectCurrentFingerprints,
  collectFileNodes,
  fileNodeIdentity,
  fingerprintFileNode,
  loadViewedStore,
  reconcileViewed,
  saveViewedStore,
  toggleViewed,
  viewedRowIds,
} from './viewedFiles.js';
import { collectBackgroundFetchTargets } from './scope.js';
import { buildTree, snapshotsForView } from './model/tree.js';
import type { TreeNode, VisibleRow, WorkspaceNode } from './model/types.js';
import {
  FLASH_MS,
  changeSignatures,
  flashableNodeIds,
  mergeGhostRows,
  mergeSignatures,
  pruneFlashes,
  pruneGhosts,
  removalGhosts,
  removedNodeIds,
  repoNodeId,
  treeChromeSignatures,
  watchIntervalMs,
} from './watch.js';
import type { ChangeSignatures, GhostRow } from './watch.js';
import type { NavState } from './nav/stack.js';
import { applyNavEnter, applyNavEsc, currentView, navDepth } from './nav/stack.js';
import { drillContextFromFocused, drillContextFromGraph } from './nav/drill.js';
import { createGraphCache, shouldAutoload } from './graph/cache.js';
import { isGraphListFocused, listFocusTarget } from './graph/focus.js';
import {
  activeRepoPath,
  applySelectableGraphPageMove,
  buildGraphListRows,
  ensureLaidOut,
  firstSelectableGraphIndex,
  graphCursorAfterRowsReload,
  graphFlashDecision,
  graphFlashMetaFromModel,
  graphRemovalGhosts,
  graphRowFlashIds,
  isNewGraphRowSet,
  shouldResetGraphCursor,
  graphRowSignatures,
  isSelectableGraphRow,
  lastSelectableGraphIndex,
  nearestSelectableGraphIndex,
  selectableGraphIndexFromClick,
  shouldShowGraphDetail,
  stepSelectableGraphCursor,
  type GraphFlashMeta,
  type GraphListRow,
} from './graph/list.js';
import type { GraphModel, LaidOutCommit } from './graph/types.js';
import { buildCommitFileNodes, flattenCommitFiles } from './commitFiles/buildCommitFileTree.js';
import { commitDetailMetaFromRow } from './commitFiles/meta.js';
import { commitFilesListKey } from './commitFiles/identity.js';
import { commitFileSourceFromNav, type GraphRowRef } from './commitFiles/resolveSource.js';
import type { CommitFileSource } from './commitFiles/types.js';

type DiffCacheEntry = {
  staged: string;
  unstaged: string;
  mtimeKey: string;
  isNew: boolean;
};

/** Theme-coloured options for `buildGraphListRows` at a pane width. */
function graphListRowOpts(
  themeId: ThemeId,
  width: number,
  defaultBranchOverride?: string,
  headBranch?: string | null,
) {
  const active = THEMES[themeId];
  return {
    width: Math.max(1, width),
    laneColors: resolveLaneColors(active),
    mutedColor: active.palette.muted,
    subjectColor: active.palette.repo,
    refLocalColor: active.palette.branchFeature,
    refDefaultColor: active.palette.branchDefault,
    refRemoteColor: active.palette.dir,
    refTagColor: active.palette.modified,
    headMarkColor: active.palette.headMark,
    overflowColor: active.palette.heading,
    defaultBranchOverride,
    headBranch: headBranch ?? null,
  };
}

export type DiffMode = 'inline' | 'sideBySide';

function labeledGraphRows(graphRows: GraphListRow[]): SearchLabeledRow[] {
  return graphRows.map((r) => ({
    id: r.id,
    label: segmentsText(r.segments),
    selectable: isSelectableGraphRow(r),
  }));
}

function currentDiffRowsForSearch(
  depth: 0 | 1 | 2,
  commitDiff: DiffPaneContent | null,
  fileDiff: DiffPaneContent | null,
  mode: DiffMode,
  paneWidth: number,
) {
  const content = depth >= 2 ? commitDiff : fileDiff;
  if (!content) return [];
  return buildDiffRows({
    staged: content.staged,
    unstaged: content.unstaged,
    mode: effectiveDiffMode(mode, paneWidth),
    isNew: content.isNew,
  });
}

/**
 * Apply a vertical diff scroll delta; clamp at EOF when row content is known (B8).
 * Row build + viewHeight must match DiffPane paint (`effectiveDiffMode`, bodyHeight).
 */
function applyDiffScrollDelta(
  current: number,
  delta: number,
  content: DiffPaneContent | null,
  mode: DiffMode,
  viewHeight: number,
  paneWidth: number,
): number {
  const next = current + delta;
  if (!content) return Math.max(0, next);
  const rowCount = buildDiffRows({
    staged: content.staged,
    unstaged: content.unstaged,
    mode: effectiveDiffMode(mode, paneWidth),
    isNew: content.isNew,
  }).length;
  return clampDiffScroll(next, rowCount, viewHeight);
}

export type { PendingConfirm };

export interface AppOptions {
  cwd: string;
  snapshots: RepoSnapshot[];
  ignoredRepos: string[];
  /** Discovery depth from workspace config (must match the initial snapshot pass). */
  maxDepth: number;
  /** Per-repo default branch overrides from workspace config. */
  defaultBranches: Record<string, string>;
  filterRepos: string[];
  /**
   * Workspace config `editor` string. GUI names stay mounted on `e`;
   * TTY names still unmount through `onEditRequest`.
   */
  editor?: string;
  /** Restored view state from a previous mount. */
  session: SessionState;
  /** Reports view-state changes so `run.ts` can restore them after a remount. */
  onSessionChange: (session: SessionState) => void;
  /** Called instead of unmounting silently, so `run.ts` knows why the TUI ended. */
  onExit: (reason: ExitReason) => void;
  /**
   * Records an `edit` intent. Paired with a same-keypress `'quit'` from
   * dispatch so the mount ends before Ctrl+C can race a dangling request.
   * Not an `ExitReason` — `run.ts` keys the editor off `pendingEdit`.
   */
  onEditRequest: (request: EditRequest) => void;
}

function hasChildren(node: TreeNode): node is TreeNode & { children: TreeNode[] } {
  return (
    node.kind === 'workspace' ||
    node.kind === 'repo' ||
    node.kind === 'checkout' ||
    node.kind === 'group' ||
    node.kind === 'dir'
  );
}

function collectForestFoldableIds(nodes: TreeNode[]): string[] {
  const ids: string[] = [];
  const walk = (n: TreeNode): void => {
    if (n.kind === 'dir' && n.children.length > 0) {
      ids.push(n.id);
      for (const c of n.children) walk(c);
    }
  };
  for (const n of nodes) walk(n);
  return ids;
}

function collectForestFoldableSubtreeIds(nodes: TreeNode[], focusId: string): string[] {
  const find = (list: TreeNode[]): TreeNode | undefined => {
    for (const n of list) {
      if (n.id === focusId) return n;
      if (n.kind === 'dir') {
        const hit = find(n.children);
        if (hit) return hit;
      }
    }
    return undefined;
  };
  const found = find(nodes);
  if (!found || found.kind !== 'dir') return [];
  return collectForestFoldableIds([found]);
}

function repoPathOf(node: TreeNode): string | null {
  if (node.kind === 'checkout') return node.path;
  if (node.kind === 'repo') {
    const primary = node.children.find(
      (c) => c.kind === 'checkout' && c.checkoutKind === 'primary',
    );
    return primary && primary.kind === 'checkout' ? primary.path : node.path;
  }
  if (node.kind === 'file' || node.kind === 'dir') return node.repoPath;
  return null;
}

function initialCursor(rows: VisibleRow[]): number {
  const fileIdx = rows.findIndex((r) => r.node.kind === 'file');
  if (fileIdx >= 0) return fileIdx;
  const repoIdx = rows.findIndex((r) => r.node.kind === 'repo');
  if (repoIdx >= 0) return repoIdx;
  return 0;
}

function clampCursor(cursor: number, len: number): number {
  if (len <= 0) return 0;
  return Math.max(0, Math.min(cursor, len - 1));
}

function filterRows(rows: VisibleRow[], filter: string): VisibleRow[] {
  const q = filter.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter((r) => r.label.toLowerCase().includes(q));
}

function rebuildTree(
  snapshots: RepoSnapshot[],
  ignoredRepos: string[],
  treeMode: boolean,
  cwd: string,
  showIgnored: boolean,
  namedRepos: string[] = [],
): WorkspaceNode {
  const ignoredSet = new Set(ignoredRepos);
  const namedSet = new Set(namedRepos);
  const visible = snapshotsForView(snapshots, ignoredSet, showIgnored, namedSet);
  return buildTree({
    snapshots: visible,
    // Shown ignored repos match CLI `-a` (no ignored mark / default folds).
    ignoredRepos: showIgnored ? new Set() : ignoredSet,
    treeMode,
    workspaceLabel: path.basename(cwd) || cwd,
  });
}

/** Merge file mtime signatures with chrome from a rebuilt tree. */
function signaturesWithChrome(
  fileSigs: ChangeSignatures,
  snapshots: RepoSnapshot[],
  ignoredRepos: string[],
  treeMode: boolean,
  cwd: string,
  showIgnored: boolean,
  namedRepos: string[] = [],
): ChangeSignatures {
  return mergeSignatures(
    fileSigs,
    treeChromeSignatures(
      rebuildTree(snapshots, ignoredRepos, treeMode, cwd, showIgnored, namedRepos),
    ),
  );
}

/**
 * Open local-branch picker session (App owns filter + cursor).
 */
export type BranchPickerState = {
  repoPath: string;
  branches: LocalBranch[];
  filter: string;
  cursor: number;
  loading: boolean;
};

/** Create-branch name overlay (`c` on graph commit). */
export type CreateBranchOverlayState = {
  commitId: string;
  name: string;
};

/** Checkoutable-branch picker at a graph commit. */
export type GraphBranchPickerState = {
  commitId: string;
  branches: string[];
  cursor: number;
  filter: string;
};

/** Stash drop y/n confirm. */
export type StashDropConfirmState = {
  stashRef: string;
};

/** Origin out-of-sync checkout y/n confirm. */
export type GraphCheckoutConfirmState = {
  localBranch: string;
  remoteRef: string;
};

/** Open stash-menu overlay (`S`) — ops snapshot + git cwd. */
export type StashMenuState = {
  ops: StashOp[];
  subtitle: string;
  repoPath: string;
};

export interface AppStateApi {
  rows: VisibleRow[];
  cursor: number;
  folds: Set<string>;
  treeMode: boolean;
  /** Legacy session filter string — unused by `/` search (kept empty). */
  filter: string;
  searchMode: boolean;
  /** Armed pane search (`null` when idle). Bound to the pane focused at `/`. */
  search: SearchState | null;
  /** Row ids matching the current search query (tree / graph / commit files). */
  searchMatchIds: Set<string>;
  /** Diff row indices matching the current search query. */
  searchMatchDiffIndices: Set<number>;
  /** EasyMotion overlay armed. */
  easyMotion: boolean;
  /** Partial label typed during EasyMotion. */
  easyMotionTyped: string;
  /** App sets left-list viewport height so tree EasyMotion matches TreePane. */
  setListViewportHeight: (height: number) => void;
  /**
   * App sets DiffPane body height (listHeight − path header) so vertical
   * scroll clamp matches DiffPane paint.
   */
  setDiffViewportHeight: (height: number) => void;
  /**
   * App sets DiffPane outer width; stores pane width for narrow side-by-side
   * fallback and code column ≈ pane − 8 for panDiff clamp.
   */
  setDiffPaneWidth: (width: number) => void;
  /**
   * App sets graph list inner width (minus cursor bar) so rows flex with the
   * pane that hosts the graph (depth-0 right / depth-1 left).
   */
  setGraphPaneWidth: (width: number) => void;
  branchMode: boolean;
  /** Create-branch overlay open (graph list focused). */
  createBranchMode: boolean;
  /** Checkoutable-branch graph picker open. */
  graphBranchMode: boolean;
  /** Stash drop confirm open. */
  stashDropMode: boolean;
  /** Origin out-of-sync checkout confirm open. */
  graphCheckoutConfirmMode: boolean;
  /** Stash menu overlay open. */
  stashMenuMode: boolean;
  branchPicker: BranchPickerState | null;
  createBranchOverlay: CreateBranchOverlayState | null;
  graphBranchPicker: GraphBranchPickerState | null;
  stashDropConfirm: StashDropConfirmState | null;
  graphCheckoutConfirm: GraphCheckoutConfirmState | null;
  /** Ops listed in the stash menu overlay, or null when closed. */
  stashMenuOps: StashOp[] | null;
  /** Muted overlay subtitle (repo path or focused stash ref). */
  stashMenuSubtitle: string;
  /** Hint-bar row kind from `activeRowKind` (focused pane, or file feeding a diff). */
  hintRowKind: RowKind;
  /** Payload for graph hint visibility gates. */
  graphActionRow: GraphActionRow | null;
  /** Scope + sync for tree write-action hint / dispatch gates. */
  actionGate: ActionGateContext;
  statusMessage: string;
  diffMode: DiffMode;
  diffScroll: number;
  /** Horizontal pan columns for DiffPane (Track D). */
  diffColOffset: number;
  diffContent: DiffPaneContent | null;
  diffLoading: boolean;
  /** True when the focused file is shown with unlimited unified context. */
  fullContextActive: boolean;
  pendingConfirm: PendingConfirm;
  showHelp: boolean;
  /**
   * Help-overlay `/` query while help is open (`null` = not searching help).
   * Independent of left-pane `search` / `searchMode`.
   */
  helpSearchQuery: string | null;
  zPending: boolean;
  focused: VisibleRow | undefined;
  /** File node id → timestamp of its last change, for the fading row flash. */
  flashes: Map<string, number>;
  /** Wall-clock ms for flash decay; bumped while flashes/ghosts are live. */
  clock: number;
  /** Poll period in ms; 0 when live refresh is disabled. */
  watchMs: number;
  /** Session-facing mouse reporting flag (App owns terminal enable/disable). */
  mouseEnabled: boolean;
  /** Active built-in theme id (cycled with `T`). */
  theme: ThemeId;
  dispatchInput: (input: string, key: KeyFlags) => 'quit' | void;
  /** Move the tree cursor to an absolute visible-row index (mouse click). */
  selectRow: (index: number) => void;
  /** Move the graph cursor to a clickable row index (spacer → parent). */
  selectGraphRow: (index: number) => void;
  /** Move the commit-file cursor to an absolute index (mouse click). */
  selectCommitFileRow: (index: number) => void;
  /** Toggle fold for the workspace-tree row at `index` (mouse chevron click). */
  toggleFoldAt: (index: number) => void;
  /** Toggle fold for the commit-file row at `index`. */
  toggleCommitFileFoldAt: (index: number) => void;
  /** Focus left or right pane (mouse click on that column's list or the diff). */
  focusPaneSide: (side: 'left' | 'right') => void;
  /** Scroll the diff pane by `delta` lines (mouse wheel over diff). */
  scrollDiffBy: (delta: number) => void;
  /** Move the focused list cursor by `delta` rows (legacy; prefer hit-local helpers). */
  moveCursorBy: (delta: number) => void;
  /** Move the workspace-tree cursor by `delta` (wheel over tree). */
  moveTreeCursorBy: (delta: number) => void;
  /** Move the graph cursor by `delta` selectable steps (wheel over graph). */
  moveGraphCursorBy: (delta: number) => void;
  /** Move the commit-file cursor by `delta` (wheel over commit files). */
  moveCommitFileCursorBy: (delta: number) => void;
  setStatusMessage: (msg: string) => void;
  /** Trailing top-chrome op status (fetch/pull + ephemeral toasts). */
  opStatusLine: string;
  /** ViewStack + pane focus (JBY-037). */
  nav: NavState;
  /** Zero-based ViewStack depth. */
  navDepth: 0 | 1 | 2;
  /** Which pane is focused for Enter drill / Esc back. */
  focusPane: 'left' | 'right';
  /** Graph list rows for GraphPane (P3). */
  graphRows: GraphListRow[];
  /** Cursor within graphRows. */
  graphCursor: number;
  /** True while the first graph page is loading. */
  graphLoading: boolean;
  /** True while autoloadNext is fetching older commits. */
  graphLoadingOlder: boolean;
  /** Repo path whose graph is shown, or null. */
  graphRepoPath: string | null;
  /** Loaded graph model (for GraphPane footer / drill meta). */
  graphModel: GraphModel | null;
  /** Sync chrome for the active graph repo (header). */
  graphSync: import('./graph/selectionDetail.js').GraphSyncChrome | null;
  /** Row under graphCursor, or null. */
  selectedGraphRow: GraphListRow | null;
  /** Commit-scoped file rows (depth ≥ 1). */
  commitFileRows: VisibleRow[];
  commitFileCursor: number;
  commitFileFolds: Set<string>;
  /** Independent of workspace treeMode; default true (tree). */
  commitTreeMode: boolean;
  commitDetailTitle: string;
  commitDetailSubtitle?: string;
  commitFilesLoading: boolean;
  /** Diff for the focused commit file at depth 2 (also reused at depth 1 prep). */
  commitDiffContent: DiffPaneContent | null;
  commitDiffLoading: boolean;
}

/**
 * Own TUI navigation / fold / filter / refresh state.
 */
export function useAppState(opts: AppOptions): AppStateApi {
  const ignoredRepos = opts.ignoredRepos;
  const maxDepth = opts.maxDepth;
  const defaultBranches = opts.defaultBranches;
  const filterRepos = opts.filterRepos;
  const cwd = opts.cwd;

  const session = opts.session;
  const onSessionChange = opts.onSessionChange;

  const [boot] = useState(() => {
    const initialTree = rebuildTree(
      opts.snapshots,
      ignoredRepos,
      session.treeMode,
      cwd,
      session.showIgnored,
      filterRepos,
    );
    /**
     * `restored` is carried explicitly rather than inferred from the shape of
     * the session. An empty fold set with a null cursor id is a legitimate
     * restored state (expand-all plus a filter that matches nothing), and
     * treating it as a fresh launch would discard the user's expansion.
     */
    const fresh = !session.restored;
    const initialFolds = fresh ? createFoldState(initialTree) : session.folded;
    // `/` search does not hide rows — always flatten the full tree.
    const initialRows = flatten(initialTree, initialFolds);
    if (fresh) {
      const cursor = initialCursor(initialRows);
      return {
        tree: initialTree,
        folds: initialFolds,
        cursor,
        focusId: initialRows[cursor]?.id ?? null,
      };
    }
    const restored = resolveFocusAfterRebuild(
      initialTree,
      initialFolds,
      session.cursorId,
      0,
      initialRows,
    );
    const painted = flatten(initialTree, restored.folds);
    return {
      tree: initialTree,
      folds: restored.folds,
      cursor: resolveListFocus(painted, restored.focusId, 0).cursor,
      focusId: restored.focusId,
    };
  });

  const [snapshots, setSnapshots] = useState(opts.snapshots);
  const [treeMode, setTreeMode] = useState(session.treeMode);
  const [showIgnored, setShowIgnored] = useState(session.showIgnored);
  const [viewedStore, setViewedStore] = useState(loadViewedStore);
  const viewedStoreRef = useRef(viewedStore);
  viewedStoreRef.current = viewedStore;
  const [tree, setTree] = useState(boot.tree);
  const [folds, setFolds] = useState(boot.folds);
  // B5: cursor-only updates must not call setTree / applySnapshots / new folds.
  // Flattened rows stay identity-stable via useMemo([tree, folds]) across j/k;
  // the painted list also merges ghosts (`[allRows, ghosts, clock]`).
  const [cursor, setCursor] = useState(boot.cursor);
  /** Unused by `/` search; kept empty for session remount compat. */
  const [filter, setFilter] = useState('');
  const [search, setSearch] = useState<SearchState | null>(() => session.search);
  const [easyMotion, setEasyMotion] = useState(session.easyMotion);
  const [easyMotionTyped, setEasyMotionTyped] = useState('');
  const listViewportHeightRef = useRef(20);
  const setListViewportHeight = useCallback((height: number) => {
    listViewportHeightRef.current = Math.max(1, height);
  }, []);
  /** DiffPane body rows (excludes path header) — scroll clamp / keep-in-view. */
  const diffViewportHeightRef = useRef(20);
  const setDiffViewportHeight = useCallback((height: number) => {
    diffViewportHeightRef.current = Math.max(1, height);
  }, []);
  /** DiffPane outer width (narrow SxS fallback) + approx code column for pan. */
  const diffPaneWidthRef = useRef(68);
  const diffCodeWidthRef = useRef(60);
  const setDiffPaneWidth = useCallback((paneWidth: number) => {
    const w = Math.max(1, paneWidth);
    diffPaneWidthRef.current = w;
    diffCodeWidthRef.current = Math.max(1, w - 8);
  }, []);
  /** Graph list segment budget — cursor bar excluded by App. */
  const [graphPaneWidth, setGraphPaneWidthState] = useState(80);
  const setGraphPaneWidth = useCallback((width: number) => {
    setGraphPaneWidthState((prev) => {
      const next = Math.max(1, Math.floor(width));
      return prev === next ? prev : next;
    });
  }, []);
  const [diffMode, setDiffMode] = useState<DiffMode>(session.diffMode);
  const [fullContext, setFullContext] = useState<Set<string>>(() => new Set(session.fullContext));
  const [mouseEnabled, setMouseEnabled] = useState(session.mouseEnabled);
  const [theme, setTheme] = useState<ThemeId>(session.theme);
  const [nav, setNav] = useState<NavState>(() => session.nav);
  const [graphCacheEpoch, setGraphCacheEpoch] = useState(session.graphCacheEpoch);
  const [graphModel, setGraphModel] = useState<GraphModel | null>(null);
  const [graphLaidOut, setGraphLaidOut] = useState<LaidOutCommit[] | undefined>(undefined);
  const [graphLiveRows, setGraphLiveRows] = useState<GraphListRow[]>([]);
  const [graphGhosts, setGraphGhosts] = useState<GhostRow<GraphListRow>[]>([]);
  const [graphCursor, setGraphCursor] = useState(0);
  const [graphLoading, setGraphLoading] = useState(false);
  const [graphLoadingOlder, setGraphLoadingOlder] = useState(false);
  /** Mirrors `graphLoadingOlder` for shouldAutoload without putting it in effect deps. */
  const graphLoadingOlderRef = useRef(false);
  /** Latest layout for autoload merge — not an effect dep (avoids cancel race). */
  const graphLaidOutRef = useRef(graphLaidOut);
  graphLaidOutRef.current = graphLaidOut;
  /** Bumped to ignore stale autoload results / clear loading for the active request. */
  const graphAutoloadGenRef = useRef(0);
  const graphCacheRef = useRef(createGraphCache());
  /** Filled after useActions; graph effects report errors through this. */
  const setStatusMessageRef = useRef<(msg: string) => void>(() => {});
  const [diffScroll, setDiffScroll] = useState(0);
  const [diffColOffset, setDiffColOffset] = useState(session.diffColOffset);
  /** After fullFile toggle, scroll so this pre-toggle anchor row stays in view. */
  const pendingScrollAnchorRef = useRef<number | null>(null);
  const [diffContent, setDiffContent] = useState<DiffPaneContent | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [diffEpoch, setDiffEpoch] = useState(0);
  const [showHelp, setShowHelp] = useState(false);
  /**
   * Help-overlay `/` query. `null` = not searching help; string = active
   * (possibly empty right after `/`). Independent of left-pane `search`.
   */
  const [helpSearchQuery, setHelpSearchQuery] = useState<string | null>(null);
  const [keyState, setKeyState] = useState<KeyState>(() => createKeyState());
  /** File node id → timestamp of its last observed change (drives the flash). */
  const [flashes, setFlashes] = useState<Map<string, number>>(() => new Map());
  /** Removed rows kept visible for one flash window (B3). */
  const [ghosts, setGhosts] = useState<GhostRow[]>([]);
  const ghostsRef = useRef(ghosts);
  ghostsRef.current = ghosts;
  /** Wall-clock ms shared with TreePane as flash `now` (must match flashedAt). */
  const [clock, setClock] = useState(0);

  /** Stamp node ids into the flash map (B9 op completion / watch). */
  const flashNodes = useCallback((ids: string[]) => {
    if (ids.length === 0) return;
    const now = Date.now();
    setClock(now);
    setFlashes((prev) => {
      const merged = pruneFlashes(prev, now);
      for (const id of ids) merged.set(id, now);
      return merged;
    });
  }, []);

  /** Commit / worktree / stash file list (P4). Default tree mode. */
  const [commitTreeMode, setCommitTreeMode] = useState(true);
  const [commitChanges, setCommitChanges] = useState<FileChange[]>([]);
  const [commitFileNodes, setCommitFileNodes] = useState<TreeNode[]>([]);
  const [commitFileFolds, setCommitFileFolds] = useState<Set<string>>(() => new Set());
  const [commitFileCursor, setCommitFileCursor] = useState(0);
  const commitFileFocusIdRef = useRef<string | null>(null);
  const commitFileCursorRef = useRef(commitFileCursor);
  commitFileCursorRef.current = commitFileCursor;
  const commitFileRestoreKeyRef = useRef<string | null>(null);
  const [commitFilesLoading, setCommitFilesLoading] = useState(false);
  const [commitDiffContent, setCommitDiffContent] = useState<DiffPaneContent | null>(null);
  const [commitDiffLoading, setCommitDiffLoading] = useState(false);
  const [commitDiffEpoch, setCommitDiffEpoch] = useState(0);

  /** Shared gate so refresh and write ops cannot overlap / stale-overwrite UI. */
  const busyRef = useRef(false);
  const diffCache = useRef(new Map<string, DiffCacheEntry>());
  /** Last observed change signatures — the poll compares against these. */
  const signaturesRef = useRef<ChangeSignatures>(new Map());
  /** Graph list signatures — separate from tree `signaturesRef` (file/chrome). */
  const graphSignaturesRef = useRef<ChangeSignatures>(new Map());
  const graphFlashMetaRef = useRef<GraphFlashMeta | null>(null);
  /** Row id under the cursor, so a refresh can restore focus by identity. */
  const focusIdRef = useRef<string | null>(boot.focusId);
  /** Latest cursor index for rebuild restore (avoids setState-inside-setState). */
  const cursorRef = useRef(cursor);
  cursorRef.current = cursor;
  /** Latest keymap state for the pending-flush timer (avoids stale closures). */
  const keyStateRef = useRef(keyState);
  keyStateRef.current = keyState;

  const [branchPicker, setBranchPicker] = useState<BranchPickerState | null>(null);
  const [createBranchOverlay, setCreateBranchOverlay] = useState<CreateBranchOverlayState | null>(
    null,
  );
  const [graphBranchPicker, setGraphBranchPicker] = useState<GraphBranchPickerState | null>(null);
  const [stashDropConfirm, setStashDropConfirm] = useState<StashDropConfirmState | null>(null);
  const [graphCheckoutConfirm, setGraphCheckoutConfirm] =
    useState<GraphCheckoutConfirmState | null>(null);
  const [stashMenu, setStashMenu] = useState<StashMenuState | null>(null);
  const branchPickerRef = useRef(branchPicker);
  branchPickerRef.current = branchPicker;
  /** Generation so a late listLocalBranches result cannot reopen a closed picker. */
  const branchOpenGenRef = useRef(0);

  const allRows = useMemo(() => flatten(tree, folds), [tree, folds]);
  const allRowsRef = useRef(allRows);
  allRowsRef.current = allRows;
  const viewedIds = useMemo(
    () => viewedRowIds(collectFileNodes(tree), viewedStore, cwd),
    [tree, viewedStore, cwd],
  );
  const markedRows = useMemo(() => applyViewedMarks(allRows, viewedIds), [allRows, viewedIds]);
  /** Merge ghosts — `/` search never hides rows. */
  const rows = useMemo(
    () => mergeGhostRows(markedRows, ghosts, Date.now()),
    [markedRows, ghosts, clock],
  );
  const rowsRef = useRef(rows);
  rowsRef.current = rows;
  const setTreeCursor = useCallback((nextIndex: number) => {
    const list = rowsRef.current;
    const next = clampCursor(nextIndex, list.length);
    cursorRef.current = next;
    setCursor(next);
    focusIdRef.current = list[next]?.id ?? null;
  }, []);
  /**
   * Apply rebuild folds, then re-index the painted (ghost-merged) list by
   * returned `focusId` — never use the helper's flatten `cursor` after unfold.
   */
  const restoreTreeFocus = useCallback((nextTree: TreeNode, nextFolds: Set<string>) => {
    const now = Date.now();
    const displayed = mergeGhostRows(flatten(nextTree, nextFolds), ghostsRef.current, now);
    const restored = resolveFocusAfterRebuild(
      nextTree,
      nextFolds,
      focusIdRef.current,
      cursorRef.current,
      displayed,
    );
    setFolds(restored.folds);
    const painted = mergeGhostRows(flatten(nextTree, restored.folds), ghostsRef.current, now);
    const listed = resolveListFocus(painted, restored.focusId, cursorRef.current);
    cursorRef.current = listed.cursor;
    setCursor(listed.cursor);
    focusIdRef.current = listed.focusId;
  }, []);
  const graphLiveRowsRef = useRef(graphLiveRows);
  graphLiveRowsRef.current = graphLiveRows;
  const graphGhostsRef = useRef(graphGhosts);
  graphGhostsRef.current = graphGhosts;
  const graphRows = useMemo(
    () => mergeGhostRows(graphLiveRows, graphGhosts, Date.now()),
    [graphLiveRows, graphGhosts, clock],
  );
  const focused = rows[cursor];
  const graphRepoPath = activeRepoPath(nav, focused);
  const graphVisible = shouldShowGraphDetail(nav, focused);
  const selectedGraphRow = graphRows[graphCursor] ?? null;
  const graphSync = useMemo(() => {
    if (!graphRepoPath) return null;
    const snap = snapshots.find((s) => s.repo === graphRepoPath);
    if (!snap) return null;
    return {
      branch: snap.branch,
      syncStatus: snap.syncStatus,
      syncNote: snap.syncNote,
      defaultBranchOverride: snap.defaultBranchOverride,
    };
  }, [graphRepoPath, snapshots]);

  const commitFileRows = useMemo(
    () => flattenCommitFiles(commitFileNodes, commitFileFolds, commitTreeMode),
    [commitFileNodes, commitFileFolds, commitTreeMode],
  );
  const commitFileRowsRef = useRef(commitFileRows);
  commitFileRowsRef.current = commitFileRows;
  const setCommitFileListCursor = useCallback((nextIndex: number) => {
    const list = commitFileRowsRef.current;
    const next = clampCursor(nextIndex, list.length);
    commitFileCursorRef.current = next;
    setCommitFileCursor(next);
    commitFileFocusIdRef.current = list[next]?.id ?? null;
  }, []);
  const commitFileFocused = commitFileRows[commitFileCursor];
  const commitDetailMeta = useMemo(
    () => commitDetailMetaFromRow(selectedGraphRow, graphRepoPath ?? '', graphModel),
    [selectedGraphRow, graphRepoPath, graphModel],
  );

  const graphRowRef: GraphRowRef | null = useMemo(() => {
    const row = selectedGraphRow;
    if (!row || row.kind === 'spacer') return null;
    if (row.kind === 'uncommitted') return { kind: 'uncommitted' };
    if (row.kind === 'stash') {
      return {
        kind: 'stash',
        stashRef: row.stashRef ?? 'stash@{0}',
        commitId: row.commitId ?? undefined,
      };
    }
    if (row.commitId) return { kind: 'commit', commitId: row.commitId };
    return null;
  }, [selectedGraphRow]);

  /**
   * Stable list identity — ignores breadcrumb-only `filePath` nav mutations.
   * Same key ⇒ reuse prior object so loader/rematerialize deps stay referentially stable.
   */
  const commitFileListIdentityRef = useRef<{
    key: string;
    repo: string | null;
    source: CommitFileSource | null;
  }>({ key: '', repo: null, source: null });
  const commitFileListIdentity = useMemo(() => {
    const depth = navDepth(nav);
    let next: {
      key: string;
      repo: string | null;
      source: CommitFileSource | null;
    };
    if (depth < 1) {
      next = { key: '', repo: null, source: null };
    } else {
      const view = currentView(nav);
      const repo =
        graphRepoPath ??
        (view.kind === 'repoGraph' || view.kind === 'commitFiles' ? view.repo : null);
      const source = commitFileSourceFromNav(view, graphRowRef);
      next = { key: commitFilesListKey(repo, source), repo, source };
    }
    const prev = commitFileListIdentityRef.current;
    if (prev.key === next.key && prev.repo === next.repo) {
      return prev;
    }
    commitFileListIdentityRef.current = next;
    return next;
  }, [nav, graphRepoPath, graphRowRef]);
  const commitFileListKey = commitFileListIdentity.key;
  const commitFileListRepo = commitFileListIdentity.repo;
  const commitFileListSource = commitFileListIdentity.source;

  // Keep keymap depth in sync for `t` routing.
  useEffect(() => {
    const d = navDepth(nav);
    setKeyState((s) => {
      if (s.navDepth === d) return s;
      const next = { ...s, navDepth: d };
      keyStateRef.current = next;
      return next;
    });
  }, [nav]);

  /**
   * Push view state upward on every change so a remount can restore it.
   *
   * `onSessionChange` must keep a stable identity across renders, and must not
   * trigger a state update in this component, or this effect re-renders itself.
   */
  useEffect(() => {
    onSessionChange({
      restored: true,
      cursorId: focusIdRef.current,
      folded: folds,
      filter: '',
      diffMode,
      fullContext,
      treeMode,
      showIgnored,
      mouseEnabled,
      theme,
      nav,
      graphWindow: session.graphWindow,
      graphCacheEpoch,
      diffColOffset,
      search,
      easyMotion,
    });
  }, [
    rows,
    cursor,
    folds,
    diffMode,
    fullContext,
    treeMode,
    showIgnored,
    mouseEnabled,
    theme,
    nav,
    session.graphWindow,
    graphCacheEpoch,
    diffColOffset,
    search,
    easyMotion,
    onSessionChange,
  ]);

  /**
   * Load (or reload) the graph when the active repo / visibility / cache epoch changes.
   */
  useEffect(() => {
    if (!graphRepoPath || !graphVisible) {
      setGraphLiveRows([]);
      setGraphGhosts([]);
      setGraphModel(null);
      setGraphLaidOut(undefined);
      setGraphLoading(false);
      setGraphCursor(0);
      return;
    }
    let cancelled = false;
    setGraphLoading(true);
    const limit = session.graphWindow;
    void (async () => {
      try {
        const fp = await computeRefsFingerprint(graphRepoPath);
        if (cancelled) return;
        const model = await graphCacheRef.current.getOrLoad(graphRepoPath, fp, {
          skip: 0,
          limit,
        });
        if (cancelled) return;
        const laid = ensureLaidOut(model);
        const key = {
          repoPath: graphRepoPath,
          refsFingerprint: model.refsFingerprint,
          skip: model.skip,
          limit: model.limit,
        };
        const prev = graphCacheRef.current.get(key);
        graphCacheRef.current.set(key, {
          model,
          laidOut: laid,
          loadedAt: prev?.loadedAt ?? Date.now(),
        });
        setGraphModel(model);
        setGraphLaidOut(laid);
      } catch (err) {
        if (!cancelled) {
          const msg = err instanceof Error ? err.message : String(err);
          setStatusMessageRef.current(`graph: ${msg.slice(0, 60)}`);
          setGraphLiveRows([]);
          setGraphGhosts([]);
          setGraphModel(null);
          setGraphLaidOut(undefined);
        }
      } finally {
        if (!cancelled) setGraphLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graphRepoPath, graphVisible, graphCacheEpoch, session.graphWindow]);

  /**
   * Paint graph list rows from model + layout + pane width / theme.
   * Separate from load so SIGWINCH / depth pane swaps only re-segment.
   * Width / theme must not flash — signatures come from the model, not segments.
   */
  useEffect(() => {
    if (!graphVisible || !graphModel || !graphLaidOut) {
      if (!graphVisible) {
        setGraphLiveRows([]);
        setGraphGhosts([]);
        setGraphCursor(0);
      }
      return;
    }
    const snap = snapshots.find((s) => s.repo === graphRepoPath);
    const nextRows = buildGraphListRows(
      graphModel,
      graphLaidOut,
      graphListRowOpts(theme, graphPaneWidth, snap?.defaultBranchOverride, snap?.branch),
    );
    const before = graphSignaturesRef.current;
    const nextMeta = graphFlashMetaFromModel(graphModel);
    const decision = graphFlashDecision({
      focusedRepo: graphRepoPath,
      beforeSize: before.size,
      prevRowCount: graphLiveRowsRef.current.length,
      nextRowCount: nextRows.length,
      prev: graphFlashMetaRef.current,
      next: nextMeta,
    });
    const now = Date.now();
    let nextGhosts = graphGhostsRef.current;
    if (decision.stale) {
      const resetCursor = shouldResetGraphCursor({
        stale: decision.stale,
        seed: decision.seed,
        prev: graphFlashMetaRef.current,
        next: nextMeta,
      });
      setGraphLiveRows(nextRows);
      setGraphCursor((c) =>
        graphCursorAfterRowsReload(mergeGhostRows(nextRows, nextGhosts, now), c, resetCursor),
      );
      return;
    }
    const after = graphRowSignatures(nextRows, graphModel);
    const seed = decision.seed || isNewGraphRowSet(before, after);
    const resetCursor = shouldResetGraphCursor({
      stale: decision.stale,
      seed,
      prev: graphFlashMetaRef.current,
      next: nextMeta,
    });
    if (seed) {
      nextGhosts = [];
      setGraphGhosts([]);
    } else {
      const flashIds = graphRowFlashIds(before, after, {
        includeAdds: decision.includeAdds,
      });
      if (flashIds.length > 0) {
        flashNodes(flashIds);
        const removed = removedNodeIds(before, after);
        const newGhosts = graphRemovalGhosts(
          graphLiveRowsRef.current,
          removed,
          now,
          graphModel.repoPath,
        );
        if (newGhosts.length > 0) {
          nextGhosts = pruneGhosts([...nextGhosts, ...newGhosts], now);
          setGraphGhosts(nextGhosts);
        }
      }
    }
    graphSignaturesRef.current = after;
    graphFlashMetaRef.current = nextMeta;
    setGraphLiveRows(nextRows);
    setGraphCursor((c) =>
      graphCursorAfterRowsReload(mergeGhostRows(nextRows, nextGhosts, now), c, resetCursor),
    );
  }, [
    flashNodes,
    graphVisible,
    graphModel,
    graphLaidOut,
    graphPaneWidth,
    graphRepoPath,
    snapshots,
    theme,
  ]);

  /**
   * Autoload the next commit window when the cursor hits the last loaded row.
   *
   * Do not put `graphLoadingOlder` / `graphLaidOut` in deps: setting loading true
   * would re-run the effect, cleanup would cancel the only request, shouldAutoload
   * would then see loading=true and return early, and finally would skip clearing
   * → stuck "loading older…". Gate via refs + generation instead.
   */
  useEffect(() => {
    if (!graphRepoPath || !graphModel || !graphVisible) return;
    // Wait until paint has caught up to the model. After autoload merges commits,
    // graphModel updates before graphRows; without this gate we'd re-arm
    // "loading older…" for one frame on already-extended history.
    const expectedSelectable =
      (graphModel.uncommitted ? 1 : 0) + graphModel.stashes.length + graphModel.commits.length;
    const selectableCount = graphRows.reduce((n, r) => n + (isSelectableGraphRow(r) ? 1 : 0), 0);
    if (selectableCount < expectedSelectable) return;
    const commitCount = graphModel.commits.length;
    const lastSel = lastSelectableGraphIndex(graphRows);
    const onLastRow =
      graphRows.length > 0 &&
      isSelectableGraphRow(graphRows[graphCursor]) &&
      graphCursor >= lastSel;
    if (
      !onLastRow ||
      !shouldAutoload({
        cursorIndex: Math.max(0, commitCount - 1),
        loadedCount: commitCount,
        hasMore: graphModel.hasMore,
        loading: graphLoadingOlderRef.current,
      })
    ) {
      return;
    }
    const gen = ++graphAutoloadGenRef.current;
    const repoPath = graphRepoPath;
    const model = graphModel;
    graphLoadingOlderRef.current = true;
    setGraphLoadingOlder(true);
    void (async () => {
      try {
        const merged = await graphCacheRef.current.autoloadNext(repoPath, model);
        if (graphAutoloadGenRef.current !== gen) return;
        const laid = ensureLaidOut(merged, graphLaidOutRef.current);
        const key = {
          repoPath,
          refsFingerprint: merged.refsFingerprint,
          skip: merged.skip,
          limit: merged.limit,
        };
        graphCacheRef.current.set(key, {
          model: merged,
          laidOut: laid,
          loadedAt: Date.now(),
        });
        setGraphModel(merged);
        setGraphLaidOut(laid);
      } catch (err) {
        if (graphAutoloadGenRef.current === gen) {
          const msg = err instanceof Error ? err.message : String(err);
          setStatusMessageRef.current(`graph: ${msg.slice(0, 60)}`);
        }
      } finally {
        if (graphAutoloadGenRef.current === gen) {
          graphLoadingOlderRef.current = false;
          setGraphLoadingOlder(false);
        }
      }
    })();
    return () => {
      // Invalidate this generation so late results are ignored; clear loading
      // when we still own the flag (a newer effect may have taken over).
      if (graphAutoloadGenRef.current === gen) {
        graphAutoloadGenRef.current += 1;
        graphLoadingOlderRef.current = false;
        setGraphLoadingOlder(false);
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- loading/laidOut via refs
  }, [graphCursor, graphModel, graphRepoPath, graphRows.length, graphVisible]);

  /**
   * refreshRepoOnly is defined below (it needs setStatusMessage), so the
   * action layer reaches it through a ref rather than a forward reference.
   *
   * The ref is read *after* the git write awaits, so the refresh always uses
   * the newest `snapshots` / `folds` / `treeMode`. That is deliberate: a write
   * takes long enough for a watch tick to land mid-flight, and rebuilding the
   * tree from the closure captured at dispatch time would replay pre-write
   * snapshots over the newer ones and undo fold changes made while the write
   * ran. Reading the latest closure keeps the refresh a strict follow-up to
   * whatever the UI currently shows.
   */
  const refreshRepoOnlyRef = useRef<(repo: string, message: string) => Promise<void>>(
    async () => {},
  );

  const setConfirmMode = useCallback((on: boolean) => {
    setKeyState((s) => {
      const next = { ...s, confirmMode: on };
      keyStateRef.current = next;
      return next;
    });
  }, []);

  const setBranchMode = useCallback((on: boolean) => {
    setKeyState((s) => {
      const next = { ...s, branchMode: on };
      keyStateRef.current = next;
      return next;
    });
  }, []);

  const setCreateBranchMode = useCallback((on: boolean) => {
    setKeyState((s) => {
      const next = { ...s, createBranchMode: on };
      keyStateRef.current = next;
      return next;
    });
  }, []);

  const setGraphBranchMode = useCallback((on: boolean) => {
    setKeyState((s) => {
      const next = { ...s, graphBranchMode: on };
      keyStateRef.current = next;
      return next;
    });
  }, []);

  const setStashDropMode = useCallback((on: boolean) => {
    setKeyState((s) => {
      const next = { ...s, stashDropMode: on };
      keyStateRef.current = next;
      return next;
    });
  }, []);

  const setGraphCheckoutConfirmMode = useCallback((on: boolean) => {
    setKeyState((s) => {
      const next = { ...s, graphCheckoutConfirmMode: on };
      keyStateRef.current = next;
      return next;
    });
  }, []);

  const setStashMenuMode = useCallback((on: boolean) => {
    setKeyState((s) => {
      const next = { ...s, stashMenuMode: on };
      keyStateRef.current = next;
      return next;
    });
  }, []);

  const callRefreshRepoOnly = useCallback(
    (repo: string, message: string) => refreshRepoOnlyRef.current(repo, message),
    [],
  );

  const ignoredRepoSet = useMemo(() => new Set(ignoredRepos), [ignoredRepos]);

  const repoPaths = useMemo(
    () => collectBackgroundFetchTargets(snapshots, ignoredRepoSet, showIgnored),
    [ignoredRepoSet, showIgnored, snapshots],
  );

  const runFetchRef = useRef<(repos: readonly string[], opts?: { manual?: boolean }) => void>(
    () => {},
  );

  const callRunFetch = useCallback(
    (repos: readonly string[], opts?: { manual?: boolean }) => runFetchRef.current(repos, opts),
    [],
  );

  /**
   * Filled after `refreshReposAfterFetch` exists. Pull / default-branch read
   * through the ref so a mid-op fold change still refreshes against current UI.
   */
  const refreshReposAfterFetchRef = useRef<(repos: readonly string[]) => Promise<void>>(
    async () => {},
  );

  const callRefreshRepos = useCallback(
    (repos: readonly string[]) => refreshReposAfterFetchRef.current(repos),
    [],
  );

  const openBranchPickerRef = useRef<(repoPath: string) => void>(() => {});
  const callOpenBranchPicker = useCallback(
    (repoPath: string) => openBranchPickerRef.current(repoPath),
    [],
  );

  const actions = useActions({
    cwd,
    focused: focused ?? null,
    refreshRepoOnly: callRefreshRepoOnly,
    setConfirmMode,
    busyRef,
    onEditRequest: opts.onEditRequest,
    editor: opts.editor,
    repoPaths,
    snapshots,
    ignoredRepos: ignoredRepoSet,
    showIgnored,
    runFetch: callRunFetch,
    refreshRepos: callRefreshRepos,
    openBranchPicker: callOpenBranchPicker,
    flashNodes,
  });
  const {
    dispatch: dispatchAction,
    pendingConfirm,
    statusMessage,
    actionOp,
    actionOpProgress,
  } = actions;
  const setStatusMessage = actions.setStatusMessage;
  setStatusMessageRef.current = setStatusMessage;

  /**
   * Drop cached graph data and bump epoch so the load effect re-fetches.
   */
  const invalidateGraph = useCallback((repo: string | 'all') => {
    if (repo === 'all') graphCacheRef.current.clear();
    else graphCacheRef.current.invalidateRepo(repo);
    setGraphCacheEpoch((n) => n + 1);
  }, []);

  const closeBranchPicker = useCallback(() => {
    branchOpenGenRef.current += 1;
    setBranchPicker(null);
    setBranchMode(false);
  }, [setBranchMode]);

  const openBranchPicker = useCallback(
    (repoPath: string) => {
      const gen = ++branchOpenGenRef.current;
      setBranchMode(true);
      setBranchPicker({
        repoPath,
        branches: [],
        filter: '',
        cursor: 0,
        loading: true,
      });
      setStatusMessage('');
      void (async () => {
        const repoDir = path.join(cwd, repoPath);
        const [listed, defaultBranch] = await Promise.all([
          listLocalBranches(repoDir),
          getDefaultBranch(repoDir, defaultBranchOverrideFor(repoPath, defaultBranches)),
        ]);
        if (branchOpenGenRef.current !== gen) return;
        const sorted = sortBranchesForPicker(listed, defaultBranch);
        const currentIdx = sorted.findIndex((b) => b.current);
        setBranchPicker({
          repoPath,
          branches: sorted,
          filter: '',
          cursor: currentIdx >= 0 ? currentIdx : 0,
          loading: false,
        });
      })();
    },
    [cwd, defaultBranches, setBranchMode, setStatusMessage],
  );
  openBranchPickerRef.current = openBranchPicker;

  const checkoutFromPicker = useCallback(() => {
    const picker = branchPickerRef.current;
    if (!picker || picker.loading) return;
    const visible = filterBranches(picker.branches, picker.filter);
    const selected = visible[picker.cursor];
    if (!selected) {
      setStatusMessage('No branch selected');
      return;
    }
    if (selected.current) {
      closeBranchPicker();
      setStatusMessage(`Already on ${selected.name}`);
      return;
    }
    const repoDir = path.join(cwd, picker.repoPath);
    if (busyRef.current) {
      setStatusMessage('Busy…');
      return;
    }
    busyRef.current = true;
    void (async () => {
      try {
        if (await repoHasLocalChanges(repoDir)) {
          setStatusMessage('Dirty worktree — commit or stash first');
          return;
        }
        const ok = await checkoutBranch(selected.name, repoDir);
        if (!ok) {
          setStatusMessage(`Checkout failed: ${selected.name}`);
          return;
        }
        closeBranchPicker();
        await refreshRepoOnlyRef.current(picker.repoPath, `Checked out ${selected.name}`);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setStatusMessage(`Checkout failed: ${msg.slice(0, 80)}`);
      } finally {
        busyRef.current = false;
      }
    })();
  }, [closeBranchPicker, cwd, setStatusMessage]);

  const closeCreateBranchOverlay = useCallback(() => {
    setCreateBranchOverlay(null);
    setCreateBranchMode(false);
  }, [setCreateBranchMode]);

  const closeGraphBranchPicker = useCallback(() => {
    setGraphBranchPicker(null);
    setGraphBranchMode(false);
  }, [setGraphBranchMode]);

  const closeStashDropConfirm = useCallback(() => {
    setStashDropConfirm(null);
    setStashDropMode(false);
  }, [setStashDropMode]);

  const closeGraphCheckoutConfirm = useCallback(() => {
    setGraphCheckoutConfirm(null);
    setGraphCheckoutConfirmMode(false);
  }, [setGraphCheckoutConfirmMode]);

  const closeStashMenu = useCallback(() => {
    setStashMenu(null);
    setStashMenuMode(false);
  }, [setStashMenuMode]);

  const activeGraphRepo = useCallback((): string | null => {
    return activeRepoPath(nav, rows[cursor]) ?? graphRepoPath ?? null;
  }, [cursor, graphRepoPath, nav, rows]);

  const runGraphCheckoutPlan = useCallback(
    async (repoPath: string, selectedName: string): Promise<string | null> => {
      const repoDir = path.join(cwd, repoPath);
      const localName = isOriginRemoteRef(selectedName)
        ? localNameFromOriginRef(selectedName)
        : selectedName;
      const localSha = await revParseQuiet(`refs/heads/${localName}`, repoDir);
      const remoteSha = await revParseQuiet(
        isOriginRemoteRef(selectedName)
          ? `refs/remotes/${selectedName}`
          : `refs/remotes/origin/${localName}`,
        repoDir,
      );
      const plan = planGraphCheckout({
        selectedName,
        localExists: localSha !== null,
        localSha,
        remoteSha,
      });
      if (plan.kind === 'confirmLocalThenPull') {
        closeGraphBranchPicker();
        setGraphCheckoutConfirmMode(true);
        setGraphCheckoutConfirm({
          localBranch: plan.localBranch,
          remoteRef: plan.remoteRef,
        });
        setStatusMessage('');
        return null;
      }
      const ok = await checkoutBranch(plan.branch, repoDir);
      if (!ok) {
        setStatusMessage(`Checkout failed: ${plan.branch}`);
        return null;
      }
      closeGraphBranchPicker();
      invalidateGraph(repoPath);
      return plan.branch;
    },
    [closeGraphBranchPicker, cwd, invalidateGraph, setGraphCheckoutConfirmMode, setStatusMessage],
  );

  const checkoutGraphBranch = useCallback(
    (repoPath: string, branchName: string) => {
      const repoDir = path.join(cwd, repoPath);
      void runBusyThenRefresh({
        busyRef,
        onBusy: () => setStatusMessage('Busy…'),
        work: async () => {
          try {
            if (await repoHasLocalChanges(repoDir)) {
              setStatusMessage('Dirty worktree — commit or stash first');
              return null;
            }
            return await runGraphCheckoutPlan(repoPath, branchName);
          } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            setStatusMessage(`Checkout failed: ${msg.slice(0, 80)}`);
            return null;
          }
        },
        afterRelease: async (branch) => {
          if (!branch) return;
          await refreshRepoOnlyRef.current(repoPath, `Checked out ${branch}`);
        },
      });
    },
    [cwd, runGraphCheckoutPlan, setStatusMessage],
  );

  const confirmGraphCheckout = useCallback(() => {
    const confirm = graphCheckoutConfirm;
    const repoPath = activeGraphRepo();
    if (!confirm || !repoPath) return;
    const repoDir = path.join(cwd, repoPath);
    void runBusyThenRefresh({
      busyRef,
      onBusy: () => setStatusMessage('Busy…'),
      work: async () => {
        try {
          const ok = await checkoutBranch(confirm.localBranch, repoDir);
          if (!ok) {
            closeGraphCheckoutConfirm();
            setStatusMessage(`Checkout failed: ${confirm.localBranch}`);
            return null;
          }
          const ff = await fastForwardToRemoteRef(confirm.remoteRef, repoDir);
          closeGraphCheckoutConfirm();
          invalidateGraph(repoPath);
          if (!ff) {
            return `Checked out ${confirm.localBranch}; could not fast-forward to ${confirm.remoteRef}`;
          }
          return `Checked out ${confirm.localBranch} and fast-forwarded to ${confirm.remoteRef}`;
        } catch (err) {
          closeGraphCheckoutConfirm();
          const msg = err instanceof Error ? err.message : String(err);
          setStatusMessage(`Checkout failed: ${msg.slice(0, 80)}`);
          return null;
        }
      },
      afterRelease: async (message) => {
        if (!message) return;
        await refreshRepoOnlyRef.current(repoPath, message);
      },
    });
  }, [
    activeGraphRepo,
    closeGraphCheckoutConfirm,
    cwd,
    graphCheckoutConfirm,
    invalidateGraph,
    setStatusMessage,
  ]);

  const confirmCreateBranch = useCallback(() => {
    const overlay = createBranchOverlay;
    const repoPath = activeGraphRepo();
    if (!overlay || !repoPath) return;
    const name = overlay.name.trim();
    if (!isValidBranchName(name)) {
      setStatusMessage('Invalid branch name');
      return;
    }
    const repoDir = path.join(cwd, repoPath);
    if (busyRef.current) {
      setStatusMessage('Busy…');
      return;
    }
    busyRef.current = true;
    void (async () => {
      try {
        const result = await createBranchAt(repoDir, name, overlay.commitId);
        if (!result.ok) {
          setStatusMessage(result.error ?? 'git branch failed');
          return;
        }
        closeCreateBranchOverlay();
        invalidateGraph(repoPath);
        const short = overlay.commitId.slice(0, 7);
        setStatusMessage(`Created branch ${name} at ${short}`);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setStatusMessage(`Create branch failed: ${msg.slice(0, 80)}`);
      } finally {
        busyRef.current = false;
      }
    })();
  }, [
    activeGraphRepo,
    closeCreateBranchOverlay,
    createBranchOverlay,
    cwd,
    invalidateGraph,
    setStatusMessage,
  ]);

  const confirmStashDrop = useCallback(() => {
    const confirm = stashDropConfirm;
    const repoPath = activeGraphRepo();
    if (!confirm || !repoPath) return;
    const repoDir = path.join(cwd, repoPath);
    if (busyRef.current) {
      setStatusMessage('Busy…');
      return;
    }
    busyRef.current = true;
    void (async () => {
      try {
        const result = await stashDrop(repoDir, confirm.stashRef);
        closeStashDropConfirm();
        if (!result.ok) {
          setStatusMessage(result.error ?? 'git stash drop failed');
          return;
        }
        invalidateGraph(repoPath);
        await refreshRepoOnlyRef.current(repoPath, `Dropped ${confirm.stashRef}`);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setStatusMessage(`Drop stash failed: ${msg.slice(0, 80)}`);
      } finally {
        busyRef.current = false;
      }
    })();
  }, [
    activeGraphRepo,
    closeStashDropConfirm,
    cwd,
    invalidateGraph,
    setStatusMessage,
    stashDropConfirm,
  ]);

  const runStashGit = useCallback(
    (
      kind: 'push' | 'apply' | 'pop',
      repoPath: string,
      opts?: { stashRef?: string; paths?: readonly string[] },
    ) => {
      if (busyRef.current) {
        setStatusMessage('Busy…');
        return;
      }
      busyRef.current = true;
      void (async () => {
        try {
          const repoDir = path.join(cwd, repoPath);
          if (kind === 'push') {
            const result = await stashPush(repoDir, {
              includeUntracked: true,
              paths: opts?.paths,
            });
            if (!result.ok) {
              setStatusMessage(result.error ?? 'git stash push failed');
              return;
            }
            invalidateGraph(repoPath);
            await refreshRepoOnlyRef.current(repoPath, stashPushStatus(opts?.paths));
            return;
          }
          if (kind === 'apply') {
            const ref = opts?.stashRef;
            if (!ref) return;
            const result = await stashApply(repoDir, ref);
            if (!result.ok) {
              setStatusMessage(result.error ?? 'git stash apply failed');
              return;
            }
            invalidateGraph(repoPath);
            await refreshRepoOnlyRef.current(repoPath, `Applied ${ref}`);
            return;
          }
          const ref = opts?.stashRef;
          const result = await stashPop(repoDir, ref);
          if (!result.ok) {
            setStatusMessage(result.error ?? 'git stash pop failed');
            return;
          }
          invalidateGraph(repoPath);
          await refreshRepoOnlyRef.current(repoPath, ref ? `Popped ${ref}` : 'Popped');
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          const verb = kind === 'push' ? 'Stash' : kind === 'apply' ? 'Apply stash' : 'Pop stash';
          setStatusMessage(`${verb} failed: ${msg.slice(0, 80)}`);
        } finally {
          busyRef.current = false;
        }
      })();
    },
    [cwd, invalidateGraph, setStatusMessage],
  );

  const runStashMenuOp = useCallback(
    (op: StashOp) => {
      const menu = stashMenu;
      if (!menu) return;
      if (op.id === 'drop') {
        if (!op.stashRef) return;
        closeStashMenu();
        setStashDropConfirm({ stashRef: op.stashRef });
        setStashDropMode(true);
        setStatusMessage('');
        return;
      }
      if (busyRef.current) {
        setStatusMessage('Busy…');
        return;
      }
      closeStashMenu();
      if (op.id === 'push') {
        runStashGit('push', menu.repoPath, { paths: op.paths });
        return;
      }
      if ((op.id === 'apply' || op.id === 'pop') && op.stashRef) {
        runStashGit(op.id, menu.repoPath, { stashRef: op.stashRef });
      }
    },
    [closeStashMenu, runStashGit, setStashDropMode, setStatusMessage, stashMenu],
  );

  const focusFile = focused?.node.kind === 'file' ? focused.node : null;
  const focusRepo = focusFile?.repoPath ?? null;
  const focusPath = focusFile?.path ?? null;
  const focusUntracked = focusFile?.untracked ?? false;

  useEffect(() => {
    const restored = resolveFocusAfterRebuild(
      tree,
      folds,
      focusIdRef.current,
      cursorRef.current,
      rows,
    );
    if (restored.folds !== folds) {
      setFolds(restored.folds);
    }
    const painted =
      restored.folds === folds
        ? rows
        : mergeGhostRows(flatten(tree, restored.folds), ghostsRef.current, Date.now());
    const listed = resolveListFocus(painted, restored.focusId, cursorRef.current);
    if (listed.cursor !== cursorRef.current) {
      cursorRef.current = listed.cursor;
      setCursor(listed.cursor);
    }
    focusIdRef.current = listed.focusId;
  }, [rows, tree, folds]);

  /**
   * Reset the diff scroll only when focus moves to a different file. A live
   * refresh of the *same* file must leave the reader where they were.
   */
  useEffect(() => {
    setDiffScroll(0);
  }, [focusRepo, focusPath]);

  // Lazy-load staged + unstaged diffs when cursor lands on a file.
  useEffect(() => {
    if (!focusRepo || !focusPath) {
      setDiffContent(null);
      setDiffLoading(false);
      return;
    }

    const focusRowId = focused?.id ?? '';
    const wantFull =
      Boolean(focusRowId) && focused?.node.kind === 'file' && fullContext.has(focusRowId);
    const cacheKey = diffCacheKey(focusRepo, focusPath, wantFull);
    const abs = repoFileAbs(cwd, focusRepo, focusPath);
    const repoDir = path.join(cwd, focusRepo);
    const untracked = focusUntracked;
    let cancelled = false;

    setDiffLoading(true);

    void (async () => {
      const mtimeKey = await fileMtimeKey(abs);
      const hit = diffCache.current.get(cacheKey);
      if (hit && hit.mtimeKey === mtimeKey) {
        if (!cancelled) {
          setDiffContent({
            staged: hit.staged,
            unstaged: hit.unstaged,
            isNew: hit.isNew,
          });
          setDiffLoading(false);
        }
        return;
      }

      try {
        const ctx = wantFull ? FULL_DIFF_CONTEXT_LINES : undefined;
        const [staged, unstagedRaw] = await Promise.all([
          diffCachedFile(repoDir, focusPath, ctx),
          diffFile(repoDir, focusPath, ctx),
        ]);
        let unstaged = unstagedRaw;
        let isNew = false;
        if (!staged.trim() && !unstaged.trim() && untracked) {
          unstaged = await readUntrackedAsDiff(abs, focusPath);
          isNew = Boolean(unstaged.trim());
        }
        const entry: DiffCacheEntry = { staged, unstaged, mtimeKey, isNew };
        diffCache.current.set(cacheKey, entry);
        if (!cancelled) {
          setDiffContent({ staged, unstaged, isNew });
          setDiffLoading(false);
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (!cancelled) {
          setDiffContent(null);
          setDiffLoading(false);
          setStatusMessage(`Diff failed: ${msg.slice(0, 80)}`);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    cwd,
    focusRepo,
    focusPath,
    focusUntracked,
    diffEpoch,
    fullContext,
    focused?.id,
    focused?.node.kind,
  ]);

  // Load commit / worktree / stash file list — keyed by list identity, not raw nav,
  // so depth-2 breadcrumb filePath sync does not re-fetch / replace commitChanges.
  useEffect(() => {
    if (!commitFileListKey || !commitFileListRepo || !commitFileListSource) {
      setCommitChanges([]);
      setCommitFilesLoading(false);
      return;
    }
    const repo = commitFileListRepo;
    const source = commitFileListSource;
    const repoDir = path.join(cwd, repo);
    let cancelled = false;
    setCommitFilesLoading(true);
    void (async () => {
      try {
        const changes =
          source.kind === 'commit'
            ? await listCommitNameStatus(repoDir, source.commitId)
            : source.kind === 'stash'
              ? await listStashNameStatus(repoDir, source.stashRef)
              : await listWorktreeNameStatus(repoDir);
        if (cancelled) return;
        setCommitChanges(changes);
        setCommitFilesLoading(false);
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (!cancelled) {
          setCommitChanges([]);
          setCommitFilesLoading(false);
          setStatusMessage(`Files failed: ${msg.slice(0, 80)}`);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    cwd,
    commitFileListKey,
    commitFileListRepo,
    commitFileListSource,
    graphCacheEpoch,
    setStatusMessage,
  ]);

  // Rebuild forest when changes / tree mode / list identity change.
  useEffect(() => {
    if (!commitFileListKey || !commitFileListRepo) {
      setCommitFileNodes([]);
      return;
    }
    setCommitFileNodes(buildCommitFileNodes(commitFileListRepo, commitChanges, commitTreeMode));
  }, [commitChanges, commitTreeMode, commitFileListKey, commitFileListRepo]);

  // Reset cursor/folds only when list identity or commit tree mode changes —
  // not on every commitChanges refresh with the same identity.
  useEffect(() => {
    setCommitFileFolds(new Set());
    commitFileCursorRef.current = 0;
    commitFileFocusIdRef.current = null;
    setCommitFileCursor(0);
  }, [commitTreeMode, commitFileListKey, commitFileListRepo]);

  useEffect(() => {
    const key = `${commitFileListKey}\0${String(commitTreeMode)}\0${commitFileListRepo ?? ''}`;
    if (commitFileRestoreKeyRef.current !== key) {
      commitFileRestoreKeyRef.current = key;
      return;
    }

    let nextFolds = commitFileFolds;
    const focusId = commitFileFocusIdRef.current;
    if (focusId) {
      nextFolds = unfoldForestAncestors(commitFileNodes, nextFolds, focusId);
      for (const ancestor of focusAncestorIds(focusId)) {
        nextFolds = unfoldForestAncestors(commitFileNodes, nextFolds, ancestor);
      }
    }
    if (nextFolds !== commitFileFolds) {
      setCommitFileFolds(nextFolds);
    }
    const painted =
      nextFolds === commitFileFolds
        ? commitFileRows
        : flattenCommitFiles(commitFileNodes, nextFolds, commitTreeMode);
    const restored = resolveListFocus(
      painted,
      commitFileFocusIdRef.current,
      commitFileCursorRef.current,
    );
    if (restored.cursor !== commitFileCursorRef.current) {
      commitFileCursorRef.current = restored.cursor;
      setCommitFileCursor(restored.cursor);
    }
    commitFileFocusIdRef.current = restored.focusId;
  }, [
    commitFileRows,
    commitFileNodes,
    commitFileFolds,
    commitTreeMode,
    commitFileListKey,
    commitFileListRepo,
  ]);

  // Sync breadcrumb filePath when commit-file cursor lands on a file (depth 2).
  useEffect(() => {
    if (navDepth(nav) < 2) return;
    const filePath = commitFileFocused?.node.kind === 'file' ? commitFileFocused.node.path : null;
    setNav((prev) => {
      const view = currentView(prev);
      if (view.kind !== 'commitFiles') return prev;
      if (view.filePath === filePath) return prev;
      return {
        ...prev,
        stack: [...prev.stack.slice(0, -1), { ...view, filePath }],
      };
    });
  }, [commitFileCursor, commitFileFocused, nav]);

  // Diff for the focused commit file (shown at depth 2).
  useEffect(() => {
    const depth = navDepth(nav);
    if (depth < 1) {
      setCommitDiffContent(null);
      setCommitDiffLoading(false);
      return;
    }
    const view = currentView(nav);
    const source = commitFileSourceFromNav(view, graphRowRef);
    const fileNode = commitFileFocused?.node.kind === 'file' ? commitFileFocused.node : null;
    if (!source || !fileNode) {
      setCommitDiffContent(null);
      setCommitDiffLoading(false);
      return;
    }
    const repo = fileNode.repoPath;
    const repoDir = path.join(cwd, repo);
    const filePath = fileNode.path;
    const wantFull = Boolean(commitFileFocused && fullContext.has(commitFileFocused.id));
    let cancelled = false;
    setCommitDiffLoading(true);
    void (async () => {
      try {
        const ctx = wantFull ? FULL_DIFF_CONTEXT_LINES : undefined;
        let staged = '';
        let unstaged = '';
        let isNew = false;
        if (source.kind === 'worktree') {
          const abs = repoFileAbs(cwd, repo, filePath);
          const [s, u] = await Promise.all([
            diffCachedFile(repoDir, filePath, ctx),
            diffFile(repoDir, filePath, ctx),
          ]);
          staged = s;
          unstaged = u;
          if (!staged.trim() && !unstaged.trim() && fileNode.untracked) {
            unstaged = await readUntrackedAsDiff(abs, filePath);
            isNew = Boolean(unstaged.trim());
          }
        } else if (source.kind === 'commit') {
          unstaged = await diffCommitFile(repoDir, source.commitId, filePath, ctx);
        } else {
          unstaged = await diffStashFile(repoDir, source.stashRef, filePath, ctx);
        }
        if (!cancelled) {
          setCommitDiffContent({ staged, unstaged, isNew });
          setCommitDiffLoading(false);
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        if (!cancelled) {
          setCommitDiffContent(null);
          setCommitDiffLoading(false);
          setStatusMessage(`Diff failed: ${msg.slice(0, 80)}`);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [cwd, nav, graphRowRef, commitFileFocused, commitDiffEpoch, fullContext, setStatusMessage]);

  const onlyRepos = filterRepos.length > 0 ? new Set(filterRepos) : undefined;

  const applySnapshots = useCallback(
    (next: RepoSnapshot[], keepFolds: Set<string>, nextTreeMode: boolean) => {
      const nextTree = rebuildTree(next, ignoredRepos, nextTreeMode, cwd, showIgnored, filterRepos);
      diffCache.current.clear();
      setDiffEpoch((n) => n + 1);
      setSnapshots(next);
      setTree(nextTree);
      restoreTreeFocus(nextTree, keepFolds);
    },
    [cwd, filterRepos, ignoredRepos, restoreTreeFocus, showIgnored],
  );

  /**
   * Stamp add/change/remove flashes and in-place ghosts from a signature diff,
   * then rebuild the tree. Must run against `allRowsRef` *before* applySnapshots
   * — refresh paths used to drop rows first, so the watch tick could no longer
   * capture a ghost and removals vanished with no highlight.
   */
  const applySnapshotsWithChrome = useCallback(
    async (
      next: RepoSnapshot[],
      keepFolds: Set<string>,
      nextTreeMode: boolean,
      precomputedSignatures?: ChangeSignatures,
    ) => {
      const prevRows = allRowsRef.current;
      const before = signaturesRef.current;
      const fileSigs = precomputedSignatures ?? (await changeSignatures(cwd, next));
      const nextSignatures = signaturesWithChrome(
        fileSigs,
        next,
        ignoredRepos,
        nextTreeMode,
        cwd,
        showIgnored,
        filterRepos,
      );
      const now = Date.now();
      const flashable = flashableNodeIds(before, nextSignatures);
      const removed = removedNodeIds(before, nextSignatures);
      const newGhosts = removalGhosts(prevRows, removed, now);
      signaturesRef.current = nextSignatures;
      if (flashable.length > 0 || newGhosts.length > 0) {
        setClock(now);
        setFlashes((prev) => {
          const merged = pruneFlashes(prev, now);
          for (const id of flashable) merged.set(id, now);
          return merged;
        });
        if (newGhosts.length > 0) {
          const merged = pruneGhosts([...ghostsRef.current, ...newGhosts], now);
          ghostsRef.current = merged;
          setGhosts(merged);
        }
      }
      applySnapshots(next, keepFolds, nextTreeMode);
    },
    [applySnapshots, cwd, filterRepos, ignoredRepos, showIgnored],
  );

  const refreshRepoOnly = useCallback(
    async (repo: string, okMessage: string) => {
      invalidateGraph(repo);
      const existing = snapshots.find((s) => s.repo === repo);
      const snap = await refreshRepoSnapshot(
        cwd,
        repo,
        existing?.defaultBranchOverride ?? defaultBranchOverrideFor(repo, defaultBranches),
        existing
          ? {
              checkoutKind: existing.checkoutKind,
              ...(existing.primaryRepo ? { primaryRepo: existing.primaryRepo } : {}),
            }
          : undefined,
      );
      if (!snap) {
        const next = snapshots.filter((s) => s.repo !== repo);
        await applySnapshotsWithChrome(next, folds, treeMode);
        setStatusMessage(`Dropped missing repo ${repo}`);
        return;
      }
      const next = snapshots.map((s) => (s.repo === repo ? snap : s));
      if (!next.some((s) => s.repo === repo)) next.push(snap);
      await applySnapshotsWithChrome(next, folds, treeMode);
      setStatusMessage(okMessage);
    },
    [applySnapshotsWithChrome, cwd, defaultBranches, folds, invalidateGraph, snapshots, treeMode],
  );

  refreshRepoOnlyRef.current = refreshRepoOnly;

  const refreshReposAfterFetch = useCallback(
    async (repos: readonly string[]) => {
      let next = snapshots;
      for (const repo of repos) {
        const existing = next.find((s) => s.repo === repo);
        const snap = await refreshRepoSnapshot(
          cwd,
          repo,
          existing?.defaultBranchOverride ?? defaultBranchOverrideFor(repo, defaultBranches),
          existing
            ? {
                checkoutKind: existing.checkoutKind,
                ...(existing.primaryRepo ? { primaryRepo: existing.primaryRepo } : {}),
              }
            : undefined,
        );
        if (!snap) {
          next = next.filter((s) => s.repo !== repo);
          continue;
        }
        // Update in place only — do not append repos that were not already listed
        // (e.g. primary after removing a named-filter linked-only checkout).
        if (next.some((s) => s.repo === repo)) {
          next = next.map((s) => (s.repo === repo ? snap : s));
        }
      }
      await applySnapshotsWithChrome(next, folds, treeMode);
    },
    [applySnapshotsWithChrome, cwd, defaultBranches, folds, snapshots, treeMode],
  );
  refreshReposAfterFetchRef.current = refreshReposAfterFetch;

  const onFetched = useCallback(
    async (
      repos: readonly string[],
      result: { ok: number; failed: number },
      meta: { manual: boolean },
    ) => {
      for (const repo of repos) {
        graphCacheRef.current.invalidateRepo(repo);
      }
      if (repos.length > 0) setGraphCacheEpoch((n) => n + 1);
      if (result.failed > 0) {
        setStatusMessage(`fetch: ${result.failed} failed`);
      } else if (meta.manual) {
        setStatusMessage('Fetched');
      }
      flashNodes(repos.map((repo) => repoNodeId(repo)));
      await refreshReposAfterFetchRef.current([...repos]);
    },
    [flashNodes, setStatusMessage],
  );

  const { fetchStatusLine, runFetch } = useFetch({
    cwd,
    repoPaths,
    busyRef,
    onFetched,
    onBusy: () => setStatusMessage('Busy…'),
  });
  runFetchRef.current = runFetch;
  // Overlay pickers render statusMessage inline — skip toast duplicate in breadcrumb.
  // Ctrl+C exit prompt never goes in breadcrumb toasts — App's exitPromptPinned owns that UX.
  const overlayActive =
    branchPicker != null || createBranchOverlay != null || graphBranchPicker != null;
  const toastMessage = statusMessage && statusMessage !== CTRL_C_EXIT_PROMPT ? statusMessage : '';
  const opStatusLine = formatTopOpStatus({
    actionOp,
    actionOpProgress,
    fetchStatusLine,
    toasts: overlayActive ? [] : toastMessage ? [toastMessage] : undefined,
  });

  const refreshFocused = useCallback(async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setStatusMessage('Refreshing…');
    try {
      const focus = rows[cursor]?.node;
      if (!focus || focus.kind === 'workspace' || focus.kind === 'group') {
        invalidateGraph('all');
        const config = { ignoredRepos: [] as string[], maxDepth, defaultBranches };
        const next = await collectSnapshotsWithConfig(cwd, false, config, onlyRepos);
        await applySnapshotsWithChrome(next, folds, treeMode);
        setStatusMessage('Refreshed workspace');
        return;
      }
      const repo = repoPathOf(focus);
      if (!repo) {
        setStatusMessage('Nothing to refresh');
        return;
      }
      await refreshRepoOnly(repo, `Refreshed ${repo}`);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setStatusMessage(`Refresh failed: ${msg.slice(0, 80)}`);
    } finally {
      busyRef.current = false;
    }
  }, [
    applySnapshotsWithChrome,
    cwd,
    cursor,
    defaultBranches,
    folds,
    ignoredRepos,
    invalidateGraph,
    maxDepth,
    onlyRepos,
    refreshRepoOnly,
    rows,
    treeMode,
  ]);

  /** Cheap identity of the repo-level metadata shown in the tree. */
  const repoFingerprint = (list: RepoSnapshot[]): string =>
    list
      .map((s) => `${s.repo}|${s.branch}|${s.syncStatus}|${s.syncNote}`)
      .sort()
      .join('\n');

  const watchMs = watchIntervalMs();

  /**
   * Seed the baseline once, so the first poll compares against the state the
   * TUI opened with instead of flashing every file and repo row at startup.
   */
  useEffect(() => {
    let disposed = false;
    void changeSignatures(cwd, opts.snapshots).then((initial) => {
      if (!disposed) {
        signaturesRef.current = mergeSignatures(initial, treeChromeSignatures(boot.tree));
      }
    });
    return () => {
      disposed = true;
    };
    // Baseline is taken from the boot snapshots only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /**
   * Poll every repo on an interval and fold any differences into the tree.
   *
   * Polling rather than `fs.watch`: 30+ repos on a WSL2 mount exhaust inotify
   * watches and miss events. A tick is skipped whenever a write or an earlier
   * tick is still in flight, and state is only replaced when something really
   * moved — otherwise the diff cache would churn on every tick.
   */
  useEffect(() => {
    if (watchMs === 0) return;

    let disposed = false;
    const timer = setInterval(() => {
      if (disposed || busyRef.current) return;
      busyRef.current = true;
      void (async () => {
        try {
          const next = await collectSnapshotsWithConfig(
            cwd,
            false,
            { ignoredRepos: [], maxDepth, defaultBranches },
            onlyRepos,
          );
          if (disposed) return;

          const fileSigs = await changeSignatures(cwd, next);
          if (disposed) return;

          const nextSignatures = signaturesWithChrome(
            fileSigs,
            next,
            ignoredRepos,
            treeMode,
            cwd,
            showIgnored,
            filterRepos,
          );

          const before = signaturesRef.current;
          const flashable = flashableNodeIds(before, nextSignatures);
          const structural =
            repoFingerprint(next) !== repoFingerprint(snapshots) ||
            nextSignatures.size !== before.size;

          if (flashable.length === 0 && !structural) {
            // Keep the baseline fresh even when the tree does not rebuild.
            signaturesRef.current = nextSignatures;
            return;
          }

          invalidateGraph('all');
          await applySnapshotsWithChrome(next, folds, treeMode, fileSigs);
        } catch {
          // A transient git failure must not kill the poll loop.
        } finally {
          busyRef.current = false;
        }
      })();
    }, watchMs);

    return () => {
      disposed = true;
      clearInterval(timer);
    };
  }, [
    applySnapshotsWithChrome,
    cwd,
    folds,
    ignoredRepos,
    defaultBranches,
    filterRepos,
    invalidateGraph,
    maxDepth,
    onlyRepos,
    showIgnored,
    snapshots,
    treeMode,
    watchMs,
  ]);

  /** Repaint while flashes / ghosts decay, then stop so an idle TUI draws nothing. */
  useEffect(() => {
    if (flashes.size === 0 && ghosts.length === 0 && graphGhosts.length === 0) return;
    const timer = setInterval(
      () => {
        const now = Date.now();
        setFlashes((prev) => {
          const pruned = pruneFlashes(prev, now);
          return pruned.size === prev.size ? prev : pruned;
        });
        setGhosts((prev) => {
          const pruned = pruneGhosts(prev, now);
          return pruned.length === prev.length ? prev : pruned;
        });
        setGraphGhosts((prev) => {
          const pruned = pruneGhosts(prev, now);
          return pruned.length === prev.length ? prev : pruned;
        });
        // Wall-clock stamp — TreePane / GraphPane use this as flash `now` (not a tick).
        setClock(now);
      },
      Math.max(120, Math.floor(FLASH_MS / 8)),
    );
    return () => clearInterval(timer);
  }, [flashes, ghosts, graphGhosts]);

  useEffect(() => {
    const files = collectFileNodes(tree);
    const current = collectCurrentFingerprints(
      files,
      cwd,
      new Set(Object.keys(viewedStoreRef.current)),
    );
    const next = reconcileViewed(viewedStoreRef.current, current);
    if (next === viewedStoreRef.current) return;
    viewedStoreRef.current = next;
    setViewedStore(next);
    saveViewedStore(next);
  }, [tree, cwd]);

  const runAction = useCallback(
    (action: Action): 'quit' | void => {
      const depth = navDepth(nav);
      const graphVis = shouldShowGraphDetail(nav, rows[cursor]);
      const target = listFocusTarget({
        depth,
        focusPane: nav.focusPane,
        graphVisible: graphVis,
      });
      const graphListFocused = isGraphListFocused({
        depth,
        focusPane: nav.focusPane,
        graphVisible: graphVis,
      });
      const commitFilesWriteBlocked =
        (target === 'commitFiles' || isTreeWriteBlockedAtDepth(depth)) &&
        TREE_WRITE_BLOCKED_IDS.has(action.type as ActionId);
      if (commitFilesWriteBlocked) return;

      // Right pane: block left-list actions unless the composed allow-list
      // (graph move/write, commit-files nav, diff scroll/file writes) matches.
      if (
        nav.focusPane === 'right' &&
        isLeftListAction(action) &&
        !rightPaneLeftListAllowed(target, action.type)
      ) {
        return;
      }
      switch (action.type) {
        case 'none':
          return;
        case 'quit':
          return 'quit';
        case 'move': {
          if (target === 'graph') {
            setGraphCursor((c) => stepSelectableGraphCursor(graphRows, c, action.delta));
            setStatusMessage('');
            return;
          }
          if (target === 'commitFiles') {
            setCommitFileListCursor(commitFileCursorRef.current + action.delta);
            setDiffScroll(0);
            setStatusMessage('');
            return;
          }
          // Diff focused (depth 2 right, or depth 0 file DiffPane): scroll, don't no-op.
          if (target === 'none' && nav.focusPane === 'right') {
            const paneContent = depth >= 2 ? commitDiffContent : diffContent;
            const viewH = diffViewportHeightRef.current;
            const paneW = diffPaneWidthRef.current;
            setDiffScroll((n) =>
              applyDiffScrollDelta(n, action.delta, paneContent, diffMode, viewH, paneW),
            );
            return;
          }
          setTreeCursor(cursorRef.current + action.delta);
          setDiffScroll(0);
          setStatusMessage('');
          return;
        }
        case 'moveTo': {
          if (target === 'graph') {
            const next =
              action.edge === 'start'
                ? firstSelectableGraphIndex(graphRows)
                : lastSelectableGraphIndex(graphRows);
            setGraphCursor(next);
            setStatusMessage('');
            return;
          }
          if (target === 'commitFiles') {
            const next = action.edge === 'start' ? 0 : Math.max(0, commitFileRows.length - 1);
            setCommitFileListCursor(next);
            setDiffScroll(0);
            setStatusMessage('');
            return;
          }
          if (target === 'none' && nav.focusPane === 'right') {
            const viewH = diffViewportHeightRef.current;
            const paneW = diffPaneWidthRef.current;
            const rowCount = currentDiffRowsForSearch(
              depth,
              commitDiffContent,
              diffContent,
              diffMode,
              paneW,
            ).length;
            setDiffScroll(diffScrollForMoveTo(action.edge, rowCount, viewH));
            return;
          }
          const next = action.edge === 'start' ? 0 : Math.max(0, rows.length - 1);
          setTreeCursor(next);
          setDiffScroll(0);
          setStatusMessage('');
          return;
        }
        case 'toggleMouse': {
          // App owns the terminal enable/disable; here only flip session-facing state.
          setMouseEnabled((v) => {
            const next = !v;
            setStatusMessage(next ? 'Mouse on' : 'Mouse off');
            return next;
          });
          return;
        }
        case 'cycleTheme': {
          const next = cycleThemeId(theme);
          setActiveTheme(THEMES[next]);
          setTheme(next);
          const nextTree = rebuildTree(
            snapshots,
            ignoredRepos,
            treeMode,
            cwd,
            showIgnored,
            filterRepos,
          );
          setTree(nextTree);
          restoreTreeFocus(nextTree, folds);
          setStatusMessage(`theme: ${THEMES[next].label}`);
          return;
        }
        case 'fold': {
          if (!foldAllowed(target)) return;
          if (target === 'commitFiles') {
            const focusId = commitFileRows[commitFileCursor]?.id ?? '';
            const foldableIds =
              action.op === 'closeAll'
                ? collectForestFoldableIds(commitFileNodes)
                : action.op === 'toggleSubtree'
                  ? collectForestFoldableSubtreeIds(commitFileNodes, focusId)
                  : [];
            if (action.op === 'toggleSubtree' && foldableIds.length === 0) return;
            try {
              setCommitFileFolds((prev) => applyFold(prev, action.op, focusId, foldableIds));
            } catch (err) {
              const msg = err instanceof Error ? err.message : String(err);
              setStatusMessage(msg.slice(0, 80));
            }
            return;
          }
          const focusId = rows[cursor]?.id ?? 'workspace';
          const foldableIds =
            action.op === 'closeAll'
              ? collectFoldableIds(tree)
              : action.op === 'toggleSubtree'
                ? collectFoldableSubtreeIds(tree, focusId)
                : [];
          // File rows (and other non-foldable focus) yield [] — no-op, not an error.
          if (action.op === 'toggleSubtree' && foldableIds.length === 0) {
            return;
          }
          try {
            setFolds((prev) => applyFold(prev, action.op, focusId, foldableIds));
          } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            setStatusMessage(msg.slice(0, 80));
          }
          return;
        }
        case 'expand': {
          if (!foldAllowed(target)) return;
          if (target === 'commitFiles') {
            const row = commitFileRows[commitFileCursor];
            if (!row || row.node.kind !== 'dir' || row.node.children.length === 0) return;
            setCommitFileFolds((prev) => applyFold(prev, 'open', row.id));
            return;
          }
          const row = rows[cursor];
          if (!row || !hasChildren(row.node)) return;
          setFolds((prev) => applyFold(prev, 'open', row.id));
          return;
        }
        case 'collapse': {
          if (!foldAllowed(target)) return;
          if (target === 'commitFiles') {
            const row = commitFileRows[commitFileCursor];
            if (!row || row.node.kind !== 'dir' || row.node.children.length === 0) return;
            setCommitFileFolds((prev) => applyFold(prev, 'close', row.id));
            return;
          }
          const row = rows[cursor];
          if (!row || !hasChildren(row.node)) return;
          setFolds((prev) => applyFold(prev, 'close', row.id));
          return;
        }
        case 'toggleCommitTreeMode': {
          setCommitTreeMode((m) => {
            const next = !m;
            setStatusMessage(next ? 'Commit tree' : 'Commit flat');
            return next;
          });
          return;
        }
        case 'toggleTreeMode': {
          const nextMode = !treeMode;
          const nextTree = rebuildTree(
            snapshots,
            ignoredRepos,
            nextMode,
            cwd,
            showIgnored,
            filterRepos,
          );
          const nextFolds = createFoldState(nextTree);
          const fileSigs: ChangeSignatures = new Map();
          for (const [id, signature] of signaturesRef.current) {
            if (id.startsWith('file:')) fileSigs.set(id, signature);
          }
          signaturesRef.current = mergeSignatures(fileSigs, treeChromeSignatures(nextTree));
          setTreeMode(nextMode);
          setTree(nextTree);
          restoreTreeFocus(nextTree, nextFolds);
          setStatusMessage(nextMode ? 'Directory tree' : 'Flat paths');
          return;
        }
        case 'toggleShowIgnored': {
          const nextShow = !showIgnored;
          const nextTree = rebuildTree(
            snapshots,
            ignoredRepos,
            treeMode,
            cwd,
            nextShow,
            filterRepos,
          );
          const nextFolds = createFoldState(nextTree);
          const fileSigs: ChangeSignatures = new Map();
          for (const [id, signature] of signaturesRef.current) {
            if (id.startsWith('file:')) fileSigs.set(id, signature);
          }
          signaturesRef.current = mergeSignatures(fileSigs, treeChromeSignatures(nextTree));
          setShowIgnored(nextShow);
          setTree(nextTree);
          restoreTreeFocus(nextTree, nextFolds);
          setStatusMessage(nextShow ? 'Ignored repos shown' : 'Ignored repos hidden');
          return;
        }
        case 'searchStart':
          setKeyState((s) => {
            const next = { ...s, searchMode: true, searchActive: false, easyMotionMode: false };
            keyStateRef.current = next;
            return next;
          });
          setEasyMotion(false);
          setEasyMotionTyped('');
          setSearch({ query: '', matchIndex: 0, target });
          setFilter('');
          setStatusMessage('');
          return;
        case 'easyMotionStart': {
          if (
            easyMotionListTarget({
              depth,
              focusPane: nav.focusPane,
              graphVisible: graphVis,
            }) === null
          ) {
            return;
          }
          setKeyState((s) => {
            const next = { ...s, easyMotionMode: true, searchMode: false };
            keyStateRef.current = next;
            return next;
          });
          setEasyMotion(true);
          setEasyMotionTyped('');
          setStatusMessage('EasyMotion');
          return;
        }
        case 'searchNext':
        case 'searchPrev': {
          if (!search?.query.trim()) return;
          const bound: SearchPaneTarget = search.target;
          const dir: 1 | -1 = action.type === 'searchNext' ? 1 : -1;
          if (bound === 'graph') {
            const labels = labeledGraphRows(graphRows);
            const indices = matchIndices(labels, search.query).filter(
              (i) => labels[i]!.selectable !== false,
            );
            const nextIdx = stepMatch(indices, graphCursor, dir);
            setGraphCursor(nextIdx);
            setSearch({ query: search.query, matchIndex: nextIdx, target: bound });
            return;
          }
          if (bound === 'commitFiles') {
            const allCf = flattenCommitFiles(commitFileNodes, new Set(), commitTreeMode);
            const nextId = nextSearchMatchId(
              allCf,
              search.query,
              commitFileFocusIdRef.current,
              dir,
            );
            if (nextId) {
              const nextFolds = unfoldForestAncestors(commitFileNodes, commitFileFolds, nextId);
              const visible = flattenCommitFiles(commitFileNodes, nextFolds, commitTreeMode);
              const nextIdx = visible.findIndex((r) => r.id === nextId);
              setCommitFileFolds(nextFolds);
              if (nextIdx >= 0) setCommitFileListCursor(nextIdx);
              setDiffScroll(0);
              setSearch({
                query: search.query,
                matchIndex: nextIdx >= 0 ? nextIdx : 0,
                target: bound,
              });
            }
            return;
          }
          if (bound === 'none') {
            const diffRows = currentDiffRowsForSearch(
              depth,
              commitDiffContent,
              diffContent,
              diffMode,
              diffPaneWidthRef.current,
            );
            const indices = matchDiffRowIndices(diffRows, search.query);
            const nextIdx = stepMatch(indices, search.matchIndex, dir);
            if (indices.length > 0) {
              setDiffScroll(
                scrollToKeepRow({
                  rowIndex: nextIdx,
                  viewHeight: diffViewportHeightRef.current,
                  rowCount: diffRows.length,
                  prefer: 'center',
                }),
              );
            }
            setSearch({ query: search.query, matchIndex: nextIdx, target: bound });
            return;
          }
          {
            const result = focusTreeSearchMatch({
              tree,
              folds,
              query: search.query,
              currentId: focusIdRef.current,
              dir,
            });
            if (result.focusId) {
              const painted = mergeGhostRows(
                flatten(tree, result.folds),
                ghostsRef.current,
                Date.now(),
              );
              const listed = resolveListFocus(painted, result.focusId, cursorRef.current);
              setFolds(result.folds);
              cursorRef.current = listed.cursor;
              setCursor(listed.cursor);
              focusIdRef.current = listed.focusId;
              setDiffScroll(0);
              setSearch({
                query: search.query,
                matchIndex: listed.cursor,
                target: bound,
              });
            }
          }
          return;
        }
        case 'help':
          setShowHelp(true);
          setHelpSearchQuery(null);
          setStatusMessage('');
          return;
        case 'refresh':
          void refreshFocused();
          return;
        case 'toggleDiffMode':
          setDiffMode((m) => {
            const next = m === 'inline' ? 'sideBySide' : 'inline';
            setStatusMessage(diffModeToast(next));
            return next;
          });
          return;
        case 'pageMove':
        case 'scrollDiff': {
          // PageUp/Down use ≈10-row pages; Ctrl-u/d keep ±5. Route by focus target.
          const step =
            action.type === 'pageMove' ? action.deltaPages * pageDelta(11) : action.delta;
          const dir: 1 | -1 = step >= 0 ? 1 : -1;
          const page = Math.max(1, Math.abs(step));
          if (target === 'graph') {
            setGraphCursor((c) => applySelectableGraphPageMove(graphRows, c, page, dir));
            setStatusMessage('');
            return;
          }
          if (target === 'commitFiles') {
            setCommitFileListCursor(
              applyPageMove(commitFileCursorRef.current, commitFileRows.length, page, dir),
            );
            setDiffScroll(0);
            setStatusMessage('');
            return;
          }
          if (target === 'tree') {
            setTreeCursor(applyPageMove(cursorRef.current, rows.length, page, dir));
            setDiffScroll(0);
            setStatusMessage('');
            return;
          }
          // Diff / empty right pane: clamp scroll at EOF (B8); repeated edge is no-op.
          if (nav.focusPane === 'right') {
            const paneContent = depth >= 2 ? commitDiffContent : diffContent;
            const viewH = diffViewportHeightRef.current;
            const paneW = diffPaneWidthRef.current;
            setDiffScroll((n) =>
              applyDiffScrollDelta(n, step, paneContent, diffMode, viewH, paneW),
            );
          }
          return;
        }
        case 'panDiff': {
          const paneContent = depth >= 2 ? commitDiffContent : diffContent;
          let maxKnown = 10_000;
          if (paneContent) {
            const rowsBuilt = buildDiffRows({
              staged: paneContent.staged,
              unstaged: paneContent.unstaged,
              mode: effectiveDiffMode(diffMode, diffPaneWidthRef.current),
              isNew: paneContent.isNew,
            });
            const lengths: number[] = [];
            for (const row of rowsBuilt) {
              if (row.kind !== 'line') continue;
              lengths.push(row.left.text.length);
              if (row.right) lengths.push(row.right.text.length);
            }
            maxKnown = maxColOffset(lengths, diffCodeWidthRef.current);
          }
          setDiffColOffset((o) => applyPan(o, action.delta, maxKnown));
          return;
        }
        case 'stage':
        case 'unstage':
        case 'revert':
        case 'confirmYes':
        case 'confirmYesClean':
        case 'confirmNo':
          // Keep in sync with `USE_ACTIONS_FORWARDED` / `shouldForwardToUseActions`.
          if (!shouldForwardToUseActions(action.type)) return;
          dispatchAction(action);
          return;
        case 'edit': {
          if (!shouldForwardToUseActions(action.type)) return;
          if (target === 'commitFiles' || depth >= 2) {
            const row = commitFileRows[commitFileCursor];
            if (!row || row.node.kind !== 'file') {
              setStatusMessage('Focus a file to edit');
              return;
            }
            return startEdit({
              editor: resolveEditor(process.env, opts.editor),
              request: { repoPath: row.node.repoPath, filePath: row.node.path },
              cwd,
              onEditRequest: opts.onEditRequest,
              onDetachedError: (message) => {
                setStatusMessage(`Failed to launch editor: ${message}`);
              },
            });
          }
          return dispatchAction(action);
        }
        case 'fetch':
        case 'pull':
        case 'push':
        case 'defaultBranch':
        case 'branch':
        case 'removeWorktree':
          if (!shouldForwardToUseActions(action.type)) return;
          dispatchAction(action);
          return;
        case 'graphCheckout': {
          if (!graphListFocused) return;
          const row = graphActionRowFromSelection(selectedGraphRow, graphModel);
          if (!row) return;
          const names = checkoutableBranchNames(row);
          const mode = resolveCheckoutTarget(names);
          if (mode === 'none') return;
          const repoPath = activeGraphRepo();
          if (!repoPath || row.kind !== 'commit') return;
          if (mode === 'single') {
            checkoutGraphBranch(repoPath, names[0]);
            return;
          }
          if (busyRef.current) {
            setStatusMessage('Busy…');
            return;
          }
          busyRef.current = true;
          void (async () => {
            try {
              if (await repoHasLocalChanges(path.join(cwd, repoPath))) {
                setStatusMessage('Dirty worktree — commit or stash first');
                return;
              }
              setGraphBranchMode(true);
              setGraphBranchPicker({
                commitId: row.commit.id,
                branches: names,
                cursor: 0,
                filter: '',
              });
              setStatusMessage('');
            } catch (err) {
              const msg = err instanceof Error ? err.message : String(err);
              setStatusMessage(`Checkout failed: ${msg.slice(0, 80)}`);
            } finally {
              busyRef.current = false;
            }
          })();
          return;
        }
        case 'graphCreateBranch': {
          if (!graphListFocused) return;
          const row = graphActionRowFromSelection(selectedGraphRow, graphModel);
          if (!row || row.kind !== 'commit') return;
          setCreateBranchMode(true);
          setCreateBranchOverlay({ commitId: row.commit.id, name: '' });
          setStatusMessage('');
          return;
        }
        case 'stashApply': {
          if (!graphListFocused) return;
          const row = graphActionRowFromSelection(selectedGraphRow, graphModel);
          const repoPath = activeGraphRepo();
          if (!row || row.kind !== 'stash' || !repoPath) return;
          if (busyRef.current) {
            setStatusMessage('Busy…');
            return;
          }
          busyRef.current = true;
          void (async () => {
            try {
              const result = await stashApply(path.join(cwd, repoPath), row.stash.stashRef);
              if (!result.ok) {
                setStatusMessage(result.error ?? 'git stash apply failed');
                return;
              }
              invalidateGraph(repoPath);
              await refreshRepoOnlyRef.current(repoPath, `Applied ${row.stash.stashRef}`);
            } catch (err) {
              const msg = err instanceof Error ? err.message : String(err);
              setStatusMessage(`Apply stash failed: ${msg.slice(0, 80)}`);
            } finally {
              busyRef.current = false;
            }
          })();
          return;
        }
        case 'stashDrop': {
          if (!graphListFocused) return;
          const row = graphActionRowFromSelection(selectedGraphRow, graphModel);
          if (!row || row.kind !== 'stash') return;
          setStashDropMode(true);
          setStashDropConfirm({ stashRef: row.stash.stashRef });
          setStatusMessage('');
          return;
        }
        case 'stashMenu': {
          const ctx = buildStashOpsContext({
            navDepth: depth,
            focusPane: nav.focusPane,
            focused: rows[cursor] ?? null,
            graphRow: graphActionRowFromSelection(selectedGraphRow, graphModel),
            graphDirty: Boolean(graphModel?.uncommitted?.hasChanges),
            latestStashRef: graphModel?.stashes[0]?.stashRef,
          });
          if (!ctx) return;
          const ops = stashOpsForContext(ctx);
          if (ops.length === 0) return;
          const repoPath = depth === 1 ? activeGraphRepo() : stashRepoRelPath(rows[cursor] ?? null);
          if (!repoPath) return;
          setStashMenuMode(true);
          setStashMenu({
            ops,
            subtitle: stashMenuSubtitle({
              focusedStashRef: ctx.focusedStashRef,
              repoPath,
            }),
            repoPath,
          });
          setStatusMessage('');
          return;
        }
        case 'stashPop': {
          if (!graphListFocused) return;
          const row = graphActionRowFromSelection(selectedGraphRow, graphModel);
          const repoPath = activeGraphRepo();
          if (!row || row.kind !== 'stash' || !repoPath) return;
          runStashGit('pop', repoPath, { stashRef: row.stash.stashRef });
          return;
        }
        case 'toggleViewed': {
          const row = rows[cursor];
          if (!canToggleViewed(row ?? null, depth)) return;
          if (!row || row.node.kind !== 'file') return;
          const identity = fileNodeIdentity(row.node);
          const fingerprint = fingerprintFileNode(cwd, row.node);
          const next = toggleViewed(viewedStoreRef.current, identity, fingerprint);
          viewedStoreRef.current = next;
          setViewedStore(next);
          saveViewedStore(next);
          return;
        }
        case 'fullFile': {
          const paneContent = depth >= 2 ? commitDiffContent : diffContent;
          const viewH = diffViewportHeightRef.current;
          if (paneContent) {
            const built = buildDiffRows({
              staged: paneContent.staged,
              unstaged: paneContent.unstaged,
              mode: effectiveDiffMode(diffMode, diffPaneWidthRef.current),
              isNew: paneContent.isNew,
            });
            pendingScrollAnchorRef.current = anchorRowIndex(built, diffScroll, viewH);
          } else {
            pendingScrollAnchorRef.current = diffScroll;
          }
          const treeRow = rows[cursor];
          const commitRow = commitFileRows[commitFileCursor];
          const treeFileId = treeRow?.node.kind === 'file' ? treeRow.id : null;
          const commitFileId = commitRow?.node.kind === 'file' ? commitRow.id : null;
          const id = fullContextToggleId({
            target,
            depth,
            treeFileId,
            commitFileId,
          });
          if (!id) return;
          setFullContext((prev) => toggleFullContext(id, prev));
          if (id === commitFileId) {
            setCommitDiffEpoch((n) => n + 1);
            return;
          }
          if (!treeRow || treeRow.node.kind !== 'file') return;
          // Drop both cache variants for this path so the effect reloads.
          diffCache.current.delete(diffCacheKey(treeRow.node.repoPath, treeRow.node.path, false));
          diffCache.current.delete(diffCacheKey(treeRow.node.repoPath, treeRow.node.path, true));
          setDiffEpoch((n) => n + 1);
          return;
        }
        case 'navEnter': {
          setNav((prev) => {
            const d = navDepth(prev);
            if (d === 1 && prev.focusPane === 'right') {
              const view = currentView(prev);
              const repo =
                activeRepoPath(prev, rows[cursor]) ??
                (view.kind === 'repoGraph' || view.kind === 'commitFiles' ? view.repo : '');
              return applyNavEnter(prev, drillContextFromGraph(repo, graphRows[graphCursor]));
            }
            return applyNavEnter(prev, drillContextFromFocused(rows[cursor]));
          });
          return;
        }
        case 'navEsc': {
          setNav((prev) => applyNavEsc(prev));
          return;
        }
      }
    },
    [
      commitFileCursor,
      commitFileFolds,
      commitFileNodes,
      commitFileRows,
      commitTreeMode,
      cwd,
      cursor,
      filter,
      folds,
      graphRows.length,
      graphCursor,
      graphRows,
      graphModel,
      ignoredRepos,
      filterRepos,
      dispatchAction,
      nav,
      opts,
      refreshFocused,
      rows,
      search,
      selectedGraphRow,
      snapshots,
      theme,
      tree,
      treeMode,
      showIgnored,
      setStatusMessage,
      activeGraphRepo,
      checkoutGraphBranch,
      invalidateGraph,
      setCreateBranchMode,
      setGraphBranchMode,
      setStashDropMode,
      setStashDropMode,
      restoreTreeFocus,
      setTreeCursor,
      setCommitFileListCursor,
      setStashMenuMode,
      runStashGit,
      commitDiffContent,
      diffContent,
      diffMode,
      diffScroll,
    ],
  );

  const dispatchInput = useCallback(
    (input: string, key: KeyFlags): 'quit' | void => {
      if (showHelp) {
        // Help-local `/` search — does not arm pane searchMode/search.
        if (helpSearchQuery !== null) {
          if (key.escape) {
            setHelpSearchQuery(null);
            return;
          }
          if (key.ctrl && input === 'h') {
            setHelpSearchQuery((q) => (q ?? '').slice(0, -1));
            return;
          }
          if (input === '\x7f' || input === '\b') {
            setHelpSearchQuery((q) => (q ?? '').slice(0, -1));
            return;
          }
          if (input && !key.ctrl && input.length === 1) {
            setHelpSearchQuery((q) => (q ?? '') + input);
            return;
          }
          return;
        }
        if (input === '/') {
          setHelpSearchQuery('');
          return;
        }
        if (key.escape || input === 'q' || input === '?') {
          setShowHelp(false);
          setHelpSearchQuery(null);
          return;
        }
        return;
      }

      if (keyState.easyMotionMode || easyMotion) {
        const clearEasy = () => {
          setEasyMotion(false);
          setEasyMotionTyped('');
          const next = { ...keyStateRef.current, easyMotionMode: false };
          keyStateRef.current = next;
          setKeyState(next);
          setStatusMessage('');
        };
        if (key.escape) {
          clearEasy();
          return;
        }
        if (!input || key.ctrl || input.length !== 1) return;
        const typed = easyMotionTyped + input.toLowerCase();
        const depth = navDepth(nav);
        const graphVis = shouldShowGraphDetail(nav, rows[cursor]);
        const motionTarget = easyMotionListTarget({
          depth,
          focusPane: nav.focusPane,
          graphVisible: graphVis,
        });
        if (motionTarget === null) {
          clearEasy();
          return;
        }
        let start: number;
        let visibleCount: number;
        if (motionTarget === 'graph') {
          const win = visibleGraphWindow(
            graphRows,
            graphCursor,
            listViewportHeightRef.current,
            graphLoadingOlder,
            Boolean(graphSync),
          );
          start = win.start;
          visibleCount = win.visible.length;
        } else {
          const list = motionTarget === 'commitFiles' ? commitFileRows : rows;
          const listCursor = motionTarget === 'commitFiles' ? commitFileCursor : cursor;
          let h = listViewportHeightRef.current;
          if (motionTarget === 'commitFiles' && depth === 1) {
            h = Math.max(
              1,
              h - commitDetailHeaderHeight(h, commitDetailMeta.title, commitDetailMeta.subtitle),
            );
          }
          const win = visibleTreeWindow(list, listCursor, h);
          start = win.start;
          visibleCount = win.visible.length;
        }
        const resolved = resolveEasyMotionJump(visibleCount, start, typed);
        if (resolved.status === 'miss') {
          clearEasy();
          return;
        }
        if (resolved.status === 'partial') {
          setEasyMotionTyped(typed);
          return;
        }
        const abs = resolved.index ?? 0;
        if (motionTarget === 'graph') {
          setGraphCursor(nearestSelectableGraphIndex(graphRows, abs));
        } else if (motionTarget === 'commitFiles') {
          setCommitFileListCursor(abs);
          setDiffScroll(0);
        } else {
          setTreeCursor(abs);
          setDiffScroll(0);
        }
        clearEasy();
        return;
      }

      if (keyState.stashMenuMode) {
        const resolved = resolveStashMenuKey(input, key, stashMenu?.ops ?? []);
        if (resolved.type === 'cancel') {
          closeStashMenu();
          setStatusMessage('');
          return;
        }
        if (resolved.type === 'run') {
          runStashMenuOp(resolved.op);
          return;
        }
        return;
      }

      if (keyState.searchMode) {
        if (key.escape) {
          setSearch(null);
          setFilter('');
          const next = {
            ...keyStateRef.current,
            searchMode: false,
            searchActive: false,
          };
          keyStateRef.current = next;
          setKeyState(next);
          return;
        }
        if (key.return) {
          const q = search?.query ?? '';
          const active = q.trim().length > 0;
          const next = {
            ...keyStateRef.current,
            searchMode: false,
            searchActive: active,
          };
          keyStateRef.current = next;
          setKeyState(next);
          if (!active) setSearch(null);
          return;
        }
        const applyQuery = (query: string) => {
          const depth = navDepth(nav);
          const graphVis = shouldShowGraphDetail(nav, rows[cursor]);
          const bound: SearchPaneTarget =
            search?.target ??
            listFocusTarget({
              depth,
              focusPane: nav.focusPane,
              graphVisible: graphVis,
            });
          if (bound === 'graph') {
            const labels = labeledGraphRows(graphRows);
            const indices = matchIndices(labels, query).filter(
              (i) => labels[i]!.selectable !== false,
            );
            const first = firstMatchIndex(indices);
            if (first !== null) setGraphCursor(first);
            setSearch({ query, matchIndex: first ?? 0, target: bound });
            return;
          }
          if (bound === 'commitFiles') {
            const allCf = flattenCommitFiles(commitFileNodes, new Set(), commitTreeMode);
            const firstId = nextSearchMatchId(allCf, query, null, 0);
            if (firstId) {
              const nextFolds = unfoldForestAncestors(commitFileNodes, commitFileFolds, firstId);
              const visible = flattenCommitFiles(commitFileNodes, nextFolds, commitTreeMode);
              const first = visible.findIndex((r) => r.id === firstId);
              setCommitFileFolds(nextFolds);
              if (first >= 0) {
                setCommitFileListCursor(first);
                setDiffScroll(0);
              }
              setSearch({ query, matchIndex: first >= 0 ? first : 0, target: bound });
            } else {
              setSearch({ query, matchIndex: 0, target: bound });
            }
            return;
          }
          if (bound === 'none') {
            const diffRows = currentDiffRowsForSearch(
              navDepth(nav),
              commitDiffContent,
              diffContent,
              diffMode,
              diffPaneWidthRef.current,
            );
            const indices = matchDiffRowIndices(diffRows, query);
            const first = firstMatchIndex(indices);
            if (first !== null) {
              setDiffScroll(
                scrollToKeepRow({
                  rowIndex: first,
                  viewHeight: diffViewportHeightRef.current,
                  rowCount: diffRows.length,
                  prefer: 'center',
                }),
              );
            }
            setSearch({ query, matchIndex: first ?? 0, target: bound });
            return;
          }
          const result = focusTreeSearchMatch({
            tree,
            folds,
            query,
            currentId: null,
            dir: 0,
          });
          if (result.focusId) {
            const painted = mergeGhostRows(
              flatten(tree, result.folds),
              ghostsRef.current,
              Date.now(),
            );
            const listed = resolveListFocus(painted, result.focusId, cursorRef.current);
            setFolds(result.folds);
            cursorRef.current = listed.cursor;
            setCursor(listed.cursor);
            focusIdRef.current = listed.focusId;
            setDiffScroll(0);
            setSearch({ query, matchIndex: listed.cursor, target: bound });
          } else {
            setSearch({ query, matchIndex: 0, target: bound });
          }
        };
        if (key.ctrl && input === 'h') {
          applyQuery((search?.query ?? '').slice(0, -1));
          return;
        }
        if (input === '\x7f' || input === '\b') {
          applyQuery((search?.query ?? '').slice(0, -1));
          return;
        }
        if (input && !key.ctrl && input.length === 1) {
          applyQuery((search?.query ?? '') + input);
          return;
        }
        return;
      }

      if (keyState.branchMode) {
        if (key.escape) {
          closeBranchPicker();
          return;
        }
        if (key.return) {
          checkoutFromPicker();
          return;
        }
        const move = (delta: number) => {
          setBranchPicker((prev) => {
            if (!prev || prev.loading) return prev;
            const visible = filterBranches(prev.branches, prev.filter);
            if (visible.length === 0) return prev;
            const nextCursor = Math.max(0, Math.min(prev.cursor + delta, visible.length - 1));
            return { ...prev, cursor: nextCursor };
          });
        };
        if (key.downArrow || input === 'j') {
          move(1);
          return;
        }
        if (key.upArrow || input === 'k') {
          move(-1);
          return;
        }
        if (key.ctrl && input === 'h') {
          setBranchPicker((prev) => {
            if (!prev) return prev;
            const filter = prev.filter.slice(0, -1);
            return { ...prev, filter, cursor: 0 };
          });
          return;
        }
        if (input === '\x7f' || input === '\b') {
          setBranchPicker((prev) => {
            if (!prev) return prev;
            const filter = prev.filter.slice(0, -1);
            return { ...prev, filter, cursor: 0 };
          });
          return;
        }
        if (input && !key.ctrl && input.length === 1) {
          setBranchPicker((prev) => {
            if (!prev) return prev;
            const filter = prev.filter + input;
            return { ...prev, filter, cursor: 0 };
          });
          return;
        }
        return;
      }

      if (keyState.createBranchMode) {
        if (key.escape) {
          closeCreateBranchOverlay();
          setStatusMessage('');
          return;
        }
        if (key.return) {
          confirmCreateBranch();
          return;
        }
        if (key.ctrl && input === 'h') {
          setCreateBranchOverlay((prev) =>
            prev ? { ...prev, name: prev.name.slice(0, -1) } : prev,
          );
          return;
        }
        if (input === '\x7f' || input === '\b') {
          setCreateBranchOverlay((prev) =>
            prev ? { ...prev, name: prev.name.slice(0, -1) } : prev,
          );
          return;
        }
        if (input && !key.ctrl && input.length === 1) {
          setCreateBranchOverlay((prev) => (prev ? { ...prev, name: prev.name + input } : prev));
          return;
        }
        return;
      }

      if (keyState.graphBranchMode) {
        if (key.escape) {
          closeGraphBranchPicker();
          setStatusMessage('');
          return;
        }
        if (key.return) {
          const picker = graphBranchPicker;
          const repoPath = activeGraphRepo();
          if (!picker || !repoPath) return;
          const filtered =
            picker.filter.length > 0
              ? picker.branches.filter((b) => b.toLowerCase().includes(picker.filter.toLowerCase()))
              : picker.branches;
          const name = filtered[picker.cursor];
          if (!name) {
            setStatusMessage('No branch selected');
            return;
          }
          checkoutGraphBranch(repoPath, name);
          return;
        }
        const move = (delta: number) => {
          setGraphBranchPicker((prev) => {
            if (!prev) return prev;
            const filtered =
              prev.filter.length > 0
                ? prev.branches.filter((b) => b.toLowerCase().includes(prev.filter.toLowerCase()))
                : prev.branches;
            if (filtered.length === 0) return prev;
            const nextCursor = Math.max(0, Math.min(prev.cursor + delta, filtered.length - 1));
            return { ...prev, cursor: nextCursor };
          });
        };
        if (key.downArrow || input === 'j') {
          move(1);
          return;
        }
        if (key.upArrow || input === 'k') {
          move(-1);
          return;
        }
        if (key.ctrl && input === 'h') {
          setGraphBranchPicker((prev) =>
            prev ? { ...prev, filter: prev.filter.slice(0, -1), cursor: 0 } : prev,
          );
          return;
        }
        if (input === '\x7f' || input === '\b') {
          setGraphBranchPicker((prev) =>
            prev ? { ...prev, filter: prev.filter.slice(0, -1), cursor: 0 } : prev,
          );
          return;
        }
        if (input && !key.ctrl && input.length === 1) {
          setGraphBranchPicker((prev) =>
            prev ? { ...prev, filter: prev.filter + input, cursor: 0 } : prev,
          );
          return;
        }
        return;
      }

      if (keyState.stashDropMode) {
        if (key.escape || input === 'n') {
          closeStashDropConfirm();
          setStatusMessage('');
          return;
        }
        if (input === 'y' || key.return) {
          confirmStashDrop();
          return;
        }
        return;
      }

      if (keyState.graphCheckoutConfirmMode) {
        if (key.escape || input === 'n') {
          closeGraphCheckoutConfirm();
          setStatusMessage('');
          return;
        }
        if (input === 'y' || key.return) {
          confirmGraphCheckout();
          return;
        }
        return;
      }

      const depth = navDepth(nav);
      const graphVis = shouldShowGraphDetail(nav, rows[cursor]);
      const target = listFocusTarget({
        depth,
        focusPane: nav.focusPane,
        graphVisible: graphVis,
      });
      const treeFileId = focused?.node.kind === 'file' ? focused.id : null;
      const commitFileId = commitFileFocused?.node.kind === 'file' ? commitFileFocused.id : null;
      const fullId = fullContextToggleId({
        target,
        depth,
        treeFileId,
        commitFileId,
      });
      if (key.escape && fullId && fullContext.has(fullId)) {
        setFullContext((prev) => {
          const next = new Set(prev);
          next.delete(fullId);
          return next;
        });
        if (target === 'commitFiles' || depth >= 2) {
          setCommitDiffEpoch((n) => n + 1);
        } else if (focused?.node.kind === 'file') {
          const node = focused.node;
          diffCache.current.delete(diffCacheKey(node.repoPath, node.path, false));
          diffCache.current.delete(diffCacheKey(node.repoPath, node.path, true));
          setDiffEpoch((n) => n + 1);
        }
        return;
      }

      // Esc clears armed search highlight before nav back (cursor stays).
      if (key.escape && search !== null) {
        setSearch(null);
        setFilter('');
        const next = {
          ...keyStateRef.current,
          searchMode: false,
          searchActive: false,
        };
        keyStateRef.current = next;
        setKeyState(next);
        return;
      }

      const kind = activeRowKind({
        depth,
        focusPane: nav.focusPane,
        graphVisible: graphVis,
        treeKind: focused?.node.kind ?? null,
        graphKind: graphListRowKind(selectedGraphRow),
        commitFileKind: commitFileFocused?.node.kind ?? null,
      });
      const rightIsDiff = rightPaneMode(nav, focused) === 'diff';
      const {
        state: nextKey,
        action,
        prelude,
      } = handleKey(keyStateRef.current, input, key, kind, Date.now(), {
        focusPane: nav.focusPane,
        rightIsDiff,
      });
      keyStateRef.current = nextKey;
      setKeyState(nextKey);
      // Expired/cancelled chords may flush (e.g. lone space → toggle) before the new key.
      if (prelude && prelude.type !== 'none') {
        const early = runAction(prelude);
        if (early === 'quit') return 'quit';
      }
      return runAction(action);
    },
    [
      allRows,
      checkoutFromPicker,
      closeBranchPicker,
      closeCreateBranchOverlay,
      closeGraphBranchPicker,
      closeStashDropConfirm,
      closeStashDropConfirm,
      closeGraphCheckoutConfirm,
      closeStashMenu,
      confirmCreateBranch,
      confirmStashDrop,
      confirmGraphCheckout,
      checkoutGraphBranch,
      activeGraphRepo,
      commitFileCursor,
      commitFileFolds,
      commitFileNodes,
      commitFileRows,
      commitFileFocused,
      commitTreeMode,
      cursor,
      graphBranchPicker,
      graphCursor,
      graphRows,
      focused,
      fullContext,
      keyState.branchMode,
      keyState.createBranchMode,
      keyState.searchMode,
      keyState.easyMotionMode,
      keyState.graphBranchMode,
      keyState.stashDropMode,
      keyState.stashDropMode,
      keyState.graphCheckoutConfirmMode,
      keyState.stashMenuMode,
      easyMotion,
      easyMotionTyped,
      graphLoadingOlder,
      graphSync,
      commitDetailMeta,
      nav,
      rows,
      runAction,
      runStashMenuOp,
      search,
      selectedGraphRow,
      showHelp,
      tree,
      folds,
      helpSearchQuery,
      setStatusMessage,
      stashMenu,
      commitDiffContent,
      diffContent,
      diffMode,
    ],
  );

  /**
   * Resolve expired double-tap / chord pendings (~50 ms poll).
   * Refs keep the timer off stale `keyState` / `runAction` closures — same
   * pattern as the watch / flash intervals above.
   */
  const runActionRef = useRef(runAction);
  runActionRef.current = runAction;

  useEffect(() => {
    const timer = setInterval(() => {
      const prev = keyStateRef.current;
      if (prev.pendingAt === null) return;
      const { state: next, action } = flushPending(prev, Date.now());
      if (next === prev) return;
      keyStateRef.current = next;
      setKeyState(next);
      if (action.type !== 'none') {
        void runActionRef.current(action);
      }
    }, 50);
    return () => clearInterval(timer);
  }, []);

  const selectRow = useCallback(
    (index: number) => {
      setTreeCursor(index);
      setDiffScroll(0);
      setStatusMessage('');
    },
    [setStatusMessage, setTreeCursor],
  );

  const selectGraphRow = useCallback(
    (index: number) => {
      setGraphCursor(selectableGraphIndexFromClick(graphRows, index));
      setStatusMessage('');
    },
    [graphRows, setStatusMessage],
  );

  const selectCommitFileRow = useCallback(
    (index: number) => {
      setCommitFileListCursor(index);
      setDiffScroll(0);
      setStatusMessage('');
    },
    [setCommitFileListCursor, setStatusMessage],
  );

  const focusPaneSide = useCallback((side: 'left' | 'right') => {
    setNav((prev) => (prev.focusPane === side ? prev : { ...prev, focusPane: side }));
  }, []);

  const toggleFoldAt = useCallback(
    (index: number) => {
      const row = rows[index];
      if (!row) return;
      setTreeCursor(index);
      setDiffScroll(0);
      try {
        setFolds((prev) => applyFold(prev, 'toggle', row.id));
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setStatusMessage(msg.slice(0, 80));
      }
    },
    [rows, setStatusMessage, setTreeCursor],
  );

  const toggleCommitFileFoldAt = useCallback(
    (index: number) => {
      const row = commitFileRows[index];
      if (!row) return;
      setCommitFileListCursor(index);
      setDiffScroll(0);
      try {
        setCommitFileFolds((prev) => applyFold(prev, 'toggle', row.id));
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setStatusMessage(msg.slice(0, 80));
      }
    },
    [commitFileRows, setCommitFileListCursor, setStatusMessage],
  );

  const scrollDiffBy = useCallback(
    (delta: number) => {
      const depth = navDepth(nav);
      const paneContent = depth >= 2 ? commitDiffContent : diffContent;
      const viewH = diffViewportHeightRef.current;
      const paneW = diffPaneWidthRef.current;
      setDiffScroll((n) => applyDiffScrollDelta(n, delta, paneContent, diffMode, viewH, paneW));
    },
    [commitDiffContent, diffContent, diffMode, nav],
  );

  const moveTreeCursorBy = useCallback(
    (delta: number) => {
      setTreeCursor(cursorRef.current + delta);
      setDiffScroll(0);
      setStatusMessage('');
    },
    [setStatusMessage, setTreeCursor],
  );

  const moveGraphCursorBy = useCallback(
    (delta: number) => {
      setGraphCursor((c) => stepSelectableGraphCursor(graphRows, c, delta));
      setStatusMessage('');
    },
    [graphRows, setStatusMessage],
  );

  const moveCommitFileCursorBy = useCallback(
    (delta: number) => {
      setCommitFileListCursor(commitFileCursorRef.current + delta);
      setDiffScroll(0);
      setStatusMessage('');
    },
    [setCommitFileListCursor, setStatusMessage],
  );

  const moveCursorBy = useCallback(
    (delta: number) => {
      // B5: list moves only mutate the relevant cursor (+ diff scroll reset).
      // Do not rebuild tree, folds, or snapshot identity here.
      const depth = navDepth(nav);
      const graphVis = shouldShowGraphDetail(nav, rows[cursor]);
      const target = listFocusTarget({
        depth,
        focusPane: nav.focusPane,
        graphVisible: graphVis,
      });
      if (target === 'graph') {
        moveGraphCursorBy(delta);
        return;
      }
      if (target === 'commitFiles') {
        moveCommitFileCursorBy(delta);
        return;
      }
      // Depth-2 right (or depth-0 file DiffPane): no list — scroll like pageMove.
      if (target === 'none') {
        if (nav.focusPane === 'right') {
          scrollDiffBy(delta);
        }
        return;
      }
      moveTreeCursorBy(delta);
    },
    [cursor, moveCommitFileCursorBy, moveGraphCursorBy, moveTreeCursorBy, nav, rows, scrollDiffBy],
  );

  const depthNow = navDepth(nav);

  // Reset horizontal pan when the focused diff file changes (not on mount —
  // preserve session.diffColOffset restore).
  const focusedDiffId =
    depthNow >= 2
      ? commitFileFocused?.node.kind === 'file'
        ? commitFileFocused.id
        : null
      : focused?.node.kind === 'file'
        ? focused.id
        : null;
  const prevFocusedDiffIdRef = useRef(focusedDiffId);
  useEffect(() => {
    if (prevFocusedDiffIdRef.current === focusedDiffId) return;
    prevFocusedDiffIdRef.current = focusedDiffId;
    setDiffColOffset(0);
  }, [focusedDiffId]);

  // After fullFile toggle, keep the prior hunk/change row in view once rows reload.
  useEffect(() => {
    const anchor = pendingScrollAnchorRef.current;
    if (anchor === null) return;
    const loading = depthNow >= 2 ? commitDiffLoading : diffLoading;
    if (loading) return;
    const content = depthNow >= 2 ? commitDiffContent : diffContent;
    if (!content) return;
    const built = buildDiffRows({
      staged: content.staged,
      unstaged: content.unstaged,
      mode: effectiveDiffMode(diffMode, diffPaneWidthRef.current),
      isNew: content.isNew,
    });
    const rowIndex = Math.min(Math.max(0, anchor), Math.max(0, built.length - 1));
    setDiffScroll(
      scrollToKeepRow({
        rowIndex,
        viewHeight: diffViewportHeightRef.current,
        rowCount: built.length,
        prefer: 'upperThird',
      }),
    );
    pendingScrollAnchorRef.current = null;
  }, [commitDiffContent, commitDiffLoading, depthNow, diffContent, diffLoading, diffMode]);

  const fullContextFileId = fullContextToggleId({
    target: listFocusTarget({
      depth: depthNow,
      focusPane: nav.focusPane,
      graphVisible,
    }),
    depth: depthNow,
    treeFileId: focused?.node.kind === 'file' ? focused.id : null,
    commitFileId: commitFileFocused?.node.kind === 'file' ? commitFileFocused.id : null,
  });
  const fullContextActive = Boolean(fullContextFileId && fullContext.has(fullContextFileId));

  const graphActionRow = useMemo(
    () => graphActionRowFromSelection(selectedGraphRow, graphModel),
    [selectedGraphRow, graphModel],
  );
  const hintRowKind: RowKind = activeRowKind({
    depth: depthNow,
    focusPane: nav.focusPane,
    graphVisible,
    treeKind: focused?.node.kind ?? null,
    graphKind: graphListRowKind(selectedGraphRow),
    commitFileKind: commitFileFocused?.node.kind ?? null,
  });

  /** Tree-focused row + snapshots — never the commit-files synthetic focus. */
  const actionGate: ActionGateContext = useMemo(
    () => ({
      focused: focused ?? null,
      snapshots,
      navDepth: depthNow,
      ignoredRepos: ignoredRepoSet,
      showIgnored,
    }),
    [focused, ignoredRepoSet, showIgnored, snapshots, depthNow],
  );

  const searchMatchIds = useMemo(() => {
    const q = search?.query ?? '';
    const target = search?.target ?? 'tree';
    return collectSearchMatchIds({
      target,
      query: q,
      treeRows: rows,
      graphRows: labeledGraphRows(graphRows),
      commitFileRows,
    });
  }, [commitFileRows, graphRows, rows, search]);

  const searchMatchDiffIndices = useMemo(() => {
    if (!search?.query.trim() || search.target !== 'none') return new Set<number>();
    const depth = navDepth(nav);
    const diffRows = currentDiffRowsForSearch(
      depth,
      commitDiffContent,
      diffContent,
      diffMode,
      diffPaneWidthRef.current,
    );
    return new Set(matchDiffRowIndices(diffRows, search.query));
  }, [commitDiffContent, diffContent, diffMode, nav, search]);

  return {
    rows,
    cursor,
    folds,
    treeMode,
    filter,
    searchMode: keyState.searchMode,
    search,
    searchMatchIds,
    searchMatchDiffIndices,
    easyMotion,
    easyMotionTyped,
    setListViewportHeight,
    setDiffViewportHeight,
    setDiffPaneWidth,
    setGraphPaneWidth,
    branchMode: keyState.branchMode,
    createBranchMode: keyState.createBranchMode,
    graphBranchMode: keyState.graphBranchMode,
    stashDropMode: keyState.stashDropMode,
    graphCheckoutConfirmMode: keyState.graphCheckoutConfirmMode,
    stashMenuMode: keyState.stashMenuMode,
    branchPicker,
    createBranchOverlay,
    graphBranchPicker,
    stashDropConfirm,
    graphCheckoutConfirm,
    stashMenuOps: stashMenu?.ops ?? null,
    stashMenuSubtitle: stashMenu?.subtitle ?? '',
    hintRowKind,
    graphActionRow,
    actionGate,
    statusMessage,
    diffMode,
    diffScroll,
    diffColOffset,
    diffContent: depthNow >= 2 ? commitDiffContent : diffContent,
    diffLoading: depthNow >= 2 ? commitDiffLoading : diffLoading,
    fullContextActive,
    pendingConfirm,
    showHelp,
    helpSearchQuery,
    zPending: keyState.zPending,
    focused: depthNow >= 2 ? commitFileFocused : focused,
    flashes,
    clock,
    watchMs,
    mouseEnabled,
    theme,
    dispatchInput,
    selectRow,
    selectGraphRow,
    selectCommitFileRow,
    toggleFoldAt,
    toggleCommitFileFoldAt,
    focusPaneSide,
    scrollDiffBy,
    moveCursorBy,
    moveTreeCursorBy,
    moveGraphCursorBy,
    moveCommitFileCursorBy,
    setStatusMessage,
    opStatusLine,
    nav,
    navDepth: depthNow,
    focusPane: nav.focusPane,
    graphRows,
    graphCursor,
    graphLoading,
    graphLoadingOlder,
    graphRepoPath,
    graphModel,
    graphSync,
    selectedGraphRow,
    commitFileRows,
    commitFileCursor,
    commitFileFolds,
    commitTreeMode,
    commitDetailTitle: commitDetailMeta.title,
    commitDetailSubtitle: commitDetailMeta.subtitle,
    commitFilesLoading,
    commitDiffContent,
    commitDiffLoading,
  };
}
