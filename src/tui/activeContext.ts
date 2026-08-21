/**
 * Focused-pane row kind shared by hints, keymap dispatch, and action gates.
 */

import type { NavDepthIndex, RowKind } from './actions/registry.js';
import { listFocusTarget, type GraphFocusTarget } from './graph/focus.js';
import type { FocusPane } from './nav/stack.js';

/**
 * Inputs for resolving the row kind of the focused pane.
 */
export type ActiveRowKindArgs = {
  depth: NavDepthIndex;
  focusPane: FocusPane;
  graphVisible: boolean;
  treeKind: RowKind | null;
  graphKind: RowKind | null;
  commitFileKind: RowKind | null;
};

/**
 * Row kind for hints and key dispatch: the focused pane's row, or the file
 * feeding a focused diff. Falls back to `'workspace'` when that source is null.
 */
export function activeRowKind(args: ActiveRowKindArgs): RowKind {
  const target = listFocusTarget({
    depth: args.depth,
    focusPane: args.focusPane,
    graphVisible: args.graphVisible,
  });
  switch (target) {
    case 'tree':
      return args.treeKind ?? 'workspace';
    case 'graph':
      return args.graphKind ?? 'workspace';
    case 'commitFiles':
      return args.commitFileKind ?? 'workspace';
    case 'none':
      return (args.depth >= 2 ? args.commitFileKind : args.treeKind) ?? 'workspace';
  }
}

/**
 * True when graph checkout / create-branch / stash writes may run.
 */
export function graphWritesAllowed(target: GraphFocusTarget): boolean {
  return target === 'graph';
}

/**
 * True when `edit` / `fullFile` / `toggleViewed` may pass the right-pane
 * left-list gate on a focused diff (`listFocusTarget === 'none'`).
 */
export function fileWritesOnDiffAllowed(target: GraphFocusTarget, actionType: string): boolean {
  return (
    target === 'none' &&
    (actionType === 'edit' || actionType === 'fullFile' || actionType === 'toggleViewed')
  );
}

const GRAPH_WRITE_ACTION_TYPES = new Set([
  'graphCheckout',
  'graphCreateBranch',
  'stashApply',
  'stashDrop',
  'stashPop',
]);

const COMMIT_NAV_ACTION_TYPES = new Set([
  'move',
  'moveTo',
  'fold',
  'expand',
  'collapse',
  'edit',
  'fullFile',
]);

/**
 * True when a left-list action may still run with `focusPane === 'right'`.
 * Graph writes must be in this allow-list or depth-0-right `b`/`c` no-op
 * even when `graphWritesAllowed` is true inside `runAction`.
 */
export function rightPaneLeftListAllowed(target: GraphFocusTarget, actionType: string): boolean {
  const graphMove = target === 'graph' && (actionType === 'move' || actionType === 'moveTo');
  const graphWrite = graphWritesAllowed(target) && GRAPH_WRITE_ACTION_TYPES.has(actionType);
  const commitNav = target === 'commitFiles' && COMMIT_NAV_ACTION_TYPES.has(actionType);
  const diffMove = target === 'none' && (actionType === 'move' || actionType === 'moveTo');
  const diffFileWrite = fileWritesOnDiffAllowed(target, actionType);
  return graphMove || graphWrite || commitNav || diffMove || diffFileWrite;
}

/**
 * Fold/expand/collapse may mutate the focused list. Graph and diff are no-ops
 * so they cannot change hidden workspace-tree folds.
 */
export function foldAllowed(target: GraphFocusTarget): boolean {
  return target === 'tree' || target === 'commitFiles';
}

/**
 * Which list EasyMotion labels. `null` means the focused pane is a diff —
 * start is a no-op and the mode must not stick.
 */
export function easyMotionListTarget(args: {
  depth: NavDepthIndex;
  focusPane: FocusPane;
  graphVisible: boolean;
}): Exclude<GraphFocusTarget, 'none'> | null {
  const target = listFocusTarget(args);
  return target === 'none' ? null : target;
}

/**
 * Physical widget that paints EasyMotion glyphs. Matches `easyMotionListTarget`
 * so an unfocused workspace tree never shows labels while jumps hit the graph.
 */
export type EasyMotionPaintSlot =
  'leftTree' | 'leftGraph' | 'rightGraph' | 'leftCommitFiles' | 'rightCommitFiles';

/**
 * Which on-screen list paints EasyMotion labels. `null` when the focused pane
 * is a diff (no glyphs).
 */
export function easyMotionPaintSlot(args: {
  depth: NavDepthIndex;
  focusPane: FocusPane;
  graphVisible: boolean;
}): EasyMotionPaintSlot | null {
  const target = easyMotionListTarget(args);
  if (target === 'tree') return 'leftTree';
  if (target === 'graph') return args.depth === 1 ? 'leftGraph' : 'rightGraph';
  if (target === 'commitFiles') {
    return args.depth === 2 ? 'leftCommitFiles' : 'rightCommitFiles';
  }
  return null;
}

/**
 * File ids `fullFile` toggles (same id Esc must clear).
 */
export type FullContextToggleIdArgs = {
  target: GraphFocusTarget;
  depth: NavDepthIndex;
  treeFileId: string | null;
  commitFileId: string | null;
};

/**
 * File row id `fullFile` toggles. Commit-files when that list is focused or
 * nav depth is ≥ 2; otherwise the workspace-tree file.
 */
export function fullContextToggleId(args: FullContextToggleIdArgs): string | null {
  if (args.target === 'commitFiles' || args.depth >= 2) {
    return args.commitFileId;
  }
  return args.treeFileId;
}
