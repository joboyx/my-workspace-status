/**
 * UI state that outlives a single Ink mount.
 *
 * The edit action unmounts the TUI to hand the terminal to `$EDITOR`, so the
 * cursor, folds, and view modes cannot live only in component state. `run.ts`
 * owns this object and passes it back in when it re-renders.
 */

import type { TreeNode, VisibleRow } from './model/types.js';
import { unfoldAncestors } from './model/fold.js';
import { flatten } from './model/flatten.js';
import { clampIndex } from './pageNav.js';
import type { DiffMode } from './useAppState.js';
import type { SearchState } from './search.js';
import type { ThemeId } from './theme.js';
import { THEMES, resolveThemeId, setActiveTheme } from './theme.js';
import type { NavState } from './nav/stack.js';
import { createNavState } from './nav/stack.js';

/**
 * A request to open one file in `$EDITOR`.
 *
 * Kept separate from `ExitReason` on purpose: the `edit` action records an
 * intent while the TUI is still mounted, and only the code that actually
 * unmounts for the editor may turn that intent into an exit reason.
 */
export interface EditRequest {
  repoPath: string;
  filePath: string;
  line?: number;
}

/**
 * Why the TUI unmounted, so the caller can act on it (open `$EDITOR`, stop).
 */
export type ExitReason = { type: 'quit' } | ({ type: 'edit' } & EditRequest);

/** Restorable view state for one TUI session. */
export interface SessionState {
  /**
   * False only for a session that has never been rendered. The fold restore
   * needs this stated outright: an empty `folded` set with a null `cursorId`
   * is also reachable after "expand all" while a filter matches no rows, and
   * inferring "fresh launch" from that would silently re-apply default folds.
   */
  restored: boolean;
  /** Stable row id, not an index — the row set changes between mounts. */
  cursorId: string | null;
  folded: Set<string>;
  filter: string;
  diffMode: DiffMode;
  /** Ids of files currently shown with unlimited diff context. */
  fullContext: Set<string>;
  treeMode: boolean;
  /**
   * When true, repos listed in workspace `ignoredRepos` appear in the tree.
   * Seeded from CLI `-a` / `--all`; `.` toggles it at runtime.
   */
  showIgnored: boolean;
  /**
   * Whether mouse reporting is enabled for this session. App owns the terminal
   * enable/disable; this flag is the remount-safe session-facing mirror.
   */
  mouseEnabled: boolean;
  /** Active built-in theme id for this session. */
  theme: ThemeId;
  /** ViewStack + pane focus (JBY-037). */
  nav: NavState;
  /** Graph history window size (P2b+); default 300. */
  graphWindow: number;
  /** Bumped to invalidate graph cache (P2b+). */
  graphCacheEpoch: number;
  /** Horizontal diff pan offset (Track D); default 0. */
  diffColOffset: number;
  /** Pane vim search (Track C); null when idle. Bound to the pane focused at `/`. */
  search: SearchState | null;
  /** EasyMotion overlay armed (Track C). */
  easyMotion: boolean;
}

/**
 * Session state for a fresh TUI launch.
 *
 * Seeds `theme` from `WS_STATUS_THEME` and activates it via `setActiveTheme`.
 * `seed.showIgnored` is the launch visibility (`-a` starts true).
 */
export function createSessionState(
  env: NodeJS.ProcessEnv = process.env,
  seed: { showIgnored?: boolean } = {},
): SessionState {
  const theme = resolveThemeId(env.WS_STATUS_THEME);
  setActiveTheme(THEMES[theme]);
  return {
    restored: false,
    cursorId: null,
    folded: new Set(),
    filter: '',
    diffMode: 'sideBySide',
    fullContext: new Set(),
    treeMode: true,
    showIgnored: seed.showIgnored ?? false,
    mouseEnabled: true,
    theme,
    nav: createNavState(),
    graphWindow: 300,
    graphCacheEpoch: 0,
    diffColOffset: 0,
    search: null,
    easyMotion: false,
  };
}

/**
 * Identity-stable fallback ids for a focused row, longest / most-specific
 * first. Parsed from the id string — no tree walk.
 *
 * `file:` / `dir:` → parent-prefix `dir:` ids, then `repo:` and `checkout:`.
 * `checkout:` → `repo:` with the same path. `repo:` and other kinds → none.
 */
export function focusAncestorIds(id: string): string[] {
  const colon = id.indexOf(':');
  if (colon <= 0) return [];
  const kind = id.slice(0, colon);
  const rest = id.slice(colon + 1);
  if (!rest) return [];
  if (kind === 'checkout') return [`repo:${rest}`];
  if (kind !== 'file' && kind !== 'dir') return [];

  const repoColon = rest.indexOf(':');
  if (repoColon <= 0) return [];
  const repo = rest.slice(0, repoColon);
  const segments = rest
    .slice(repoColon + 1)
    .split('/')
    .filter(Boolean);
  const parentSegs = segments.slice(0, -1);
  const ids: string[] = [];
  for (let len = parentSegs.length; len >= 1; len--) {
    ids.push(`dir:${repo}:${parentSegs.slice(0, len).join('/')}`);
  }
  ids.push(`repo:${repo}`, `checkout:${repo}`);
  return ids;
}

/**
 * Index of the saved cursor row in the current row list, falling back to the
 * first row when the saved row no longer exists (deleted, staged away, or
 * filtered out).
 */
export function cursorIndexFor(rows: VisibleRow[], cursorId: string | null): number {
  if (!cursorId) return 0;
  const index = rows.findIndex((r) => r.id === cursorId);
  return index >= 0 ? index : 0;
}

/**
 * After a tree rebuild, keep selection on `focusId` when the user did not
 * navigate away. Prefers `displayedRows` (live + ghosts) when given, without
 * unfolding. Otherwise searches `flatten`, unfolds folded ancestors (e.g.
 * `group:no-updates` after a pull), then walks `focusAncestorIds`. Clamps
 * when nothing matches. Returned `focusId` is the id actually selected.
 */
export function resolveFocusAfterRebuild(
  tree: TreeNode,
  folds: Set<string>,
  focusId: string | null,
  previousCursor: number,
  displayedRows?: VisibleRow[],
): { folds: Set<string>; cursor: number; focusId: string | null } {
  let nextFolds = folds;

  const locateInFlatten = (id: string): number => {
    let rows = flatten(tree, nextFolds);
    let index = rows.findIndex((r) => r.id === id);
    if (index < 0) {
      nextFolds = unfoldAncestors(tree, nextFolds, id);
      rows = flatten(tree, nextFolds);
      index = rows.findIndex((r) => r.id === id);
    }
    return index;
  };

  const tryResolve = (id: string): number | null => {
    if (displayedRows) {
      const displayedIndex = displayedRows.findIndex((r) => r.id === id);
      if (displayedIndex >= 0) return displayedIndex;
    }
    const index = locateInFlatten(id);
    return index >= 0 ? index : null;
  };

  if (focusId) {
    const direct = tryResolve(focusId);
    if (direct !== null) return { folds: nextFolds, cursor: direct, focusId };
    for (const ancestor of focusAncestorIds(focusId)) {
      const hit = tryResolve(ancestor);
      if (hit !== null) return { folds: nextFolds, cursor: hit, focusId: ancestor };
    }
  }

  const list = displayedRows ?? flatten(tree, nextFolds);
  const cursor = clampIndex(previousCursor, list.length);
  return {
    folds: nextFolds,
    cursor,
    focusId: list.length === 0 ? null : (list[cursor]?.id ?? null),
  };
}

/**
 * Restore list focus by row id on a flat `VisibleRow[]` (no tree walk).
 * Tries `focusId`, then `focusAncestorIds`, then clamps `previousCursor`.
 */
export function resolveListFocus(
  rows: VisibleRow[],
  focusId: string | null,
  previousCursor: number,
): { cursor: number; focusId: string | null } {
  const indexOf = (id: string): number => rows.findIndex((r) => r.id === id);

  if (focusId) {
    const direct = indexOf(focusId);
    if (direct >= 0) return { cursor: direct, focusId };
    for (const ancestor of focusAncestorIds(focusId)) {
      const hit = indexOf(ancestor);
      if (hit >= 0) return { cursor: hit, focusId: ancestor };
    }
  }

  const cursor = clampIndex(previousCursor, rows.length);
  return {
    cursor,
    focusId: rows.length === 0 ? null : (rows[cursor]?.id ?? null),
  };
}
