/**
 * Fold state: set of collapsed node ids.
 */

import type { FoldAction, TreeNode } from './types.js';

function walkFoldable(node: TreeNode, out: string[]): void {
  switch (node.kind) {
    case 'workspace':
    case 'repo':
    case 'checkout':
    case 'group':
    case 'dir':
      out.push(node.id);
      for (const child of node.children) walkFoldable(child, out);
      break;
    case 'file':
      break;
  }
}

/** All node ids that can be collapsed (everything except files). */
export function collectFoldableIds(tree: TreeNode): string[] {
  const ids: string[] = [];
  walkFoldable(tree, ids);
  return ids;
}

/**
 * Foldable ids for `focusId` and every foldable descendant under it.
 * Empty when the id is missing or names a file.
 */
export function collectFoldableSubtreeIds(tree: TreeNode, focusId: string): string[] {
  const found = findNode(tree, focusId);
  if (!found || found.kind === 'file') return [];
  const ids: string[] = [];
  walkFoldable(found, ids);
  return ids;
}

function findNode(node: TreeNode, id: string): TreeNode | undefined {
  if (node.id === id) return node;
  if (node.kind === 'file') return undefined;
  for (const child of node.children) {
    const hit = findNode(child, id);
    if (hit) return hit;
  }
  return undefined;
}

/**
 * Foldable ancestor ids from root to the parent of `targetId`.
 * `null` when the id is missing from the tree.
 */
export function ancestorIdsTo(tree: TreeNode, targetId: string): string[] | null {
  const walk = (node: TreeNode, chain: string[]): string[] | null => {
    if (node.id === targetId) return chain;
    if (node.kind === 'file') return null;
    const next = [...chain, node.id];
    for (const child of node.children) {
      const hit = walk(child, next);
      if (hit) return hit;
    }
    return null;
  };
  return walk(tree, []);
}

/**
 * Open every folded ancestor so `targetId` can appear in `flatten`.
 * Returns the same Set when the id is missing or already visible.
 */
export function unfoldAncestors(
  tree: TreeNode,
  folds: Set<string>,
  targetId: string,
): Set<string> {
  const ancestors = ancestorIdsTo(tree, targetId);
  if (!ancestors) return folds;
  let changed = false;
  for (const id of ancestors) {
    if (folds.has(id)) {
      changed = true;
      break;
    }
  }
  if (!changed) return folds;
  const next = new Set(folds);
  for (const id of ancestors) next.delete(id);
  return next;
}

/**
 * Open folded ancestors of `targetId` across a forest of roots (commit-file
 * lists have no workspace wrapper). Same Set when the id is missing.
 */
export function unfoldForestAncestors(
  nodes: readonly TreeNode[],
  folds: Set<string>,
  targetId: string,
): Set<string> {
  let next = folds;
  for (const node of nodes) {
    next = unfoldAncestors(node, next, targetId);
  }
  return next;
}

/**
 * Default folds: ignored repos with children + the `no-updates` group.
 * Non-ignored repos with changes start expanded.
 */
export function createFoldState(tree: TreeNode): Set<string> {
  const folds = new Set<string>();

  const visit = (node: TreeNode): void => {
    if (node.kind === 'repo' && node.ignored) {
      folds.add(node.id);
    }
    if (node.kind === 'group' && node.id === 'group:no-updates') {
      folds.add(node.id);
    }
    if (
      node.kind === 'workspace' ||
      node.kind === 'repo' ||
      node.kind === 'checkout' ||
      node.kind === 'group' ||
      node.kind === 'dir'
    ) {
      for (const child of node.children) visit(child);
    }
  };

  visit(tree);
  return folds;
}

/**
 * Apply a fold action.
 * `closeAll` requires `foldableIds` from `collectFoldableIds(tree)` — missing/empty throws.
 * `toggleSubtree` requires subtree ids from `collectFoldableSubtreeIds(tree, focusId)`.
 */
export function applyFold(
  folds: Set<string>,
  action: FoldAction,
  focusId: string,
  foldableIds: Iterable<string> = [],
): Set<string> {
  const next = new Set(folds);
  switch (action) {
    case 'toggle':
      if (next.has(focusId)) next.delete(focusId);
      else next.add(focusId);
      return next;
    case 'open':
      next.delete(focusId);
      return next;
    case 'close':
      next.add(focusId);
      return next;
    case 'openAll':
      return new Set();
    case 'closeAll': {
      const ids = [...foldableIds];
      if (ids.length === 0) {
        throw new Error(
          "applyFold('closeAll') requires foldableIds; pass collectFoldableIds(tree)",
        );
      }
      return new Set(ids);
    }
    case 'toggleSubtree': {
      const ids = [...foldableIds];
      if (ids.length === 0) {
        throw new Error(
          "applyFold('toggleSubtree') requires subtree ids; pass collectFoldableSubtreeIds(tree, focusId)",
        );
      }
      const opening = next.has(focusId);
      for (const id of ids) {
        if (opening) next.delete(id);
        else next.add(id);
      }
      return next;
    }
  }
}
