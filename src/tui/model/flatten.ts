/**
 * Flatten tree + fold set into visible rows for list navigation.
 */

import { segmentsText } from '../theme.js';
import { nodeSegments } from './tree.js';
import type { TreeNode, VisibleRow } from './types.js';

function hasChildren(node: TreeNode): node is TreeNode & { children: TreeNode[] } {
  return (
    node.kind === 'workspace' ||
    node.kind === 'repo' ||
    node.kind === 'checkout' ||
    node.kind === 'group' ||
    node.kind === 'dir'
  );
}

/**
 * Depth-first visible rows. Ids present in `folds` are collapsed (children omitted).
 * `treeMode` is inferred from whether any `dir` node exists in the tree.
 */
export function flatten(tree: TreeNode, folds: Set<string>): VisibleRow[] {
  const treeMode = detectTreeMode(tree);
  const rows: VisibleRow[] = [];

  const walk = (node: TreeNode, depth: number, inNoUpdates: boolean): void => {
    const { segments, trailing } = nodeSegments(node, treeMode, inNoUpdates);
    const left = segmentsText(segments).trimEnd();
    const right = segmentsText(trailing).trim();
    rows.push({
      id: node.id,
      depth,
      node,
      label: right ? `${left}  ${right}` : left,
      segments,
      trailing,
    });
    if (!hasChildren(node)) return;
    if (folds.has(node.id)) return;
    const childInNoUpdates = inNoUpdates || node.kind === 'group';
    for (const child of node.children) walk(child, depth + 1, childInNoUpdates);
  };

  walk(tree, 0, false);
  return rows;
}

function detectTreeMode(node: TreeNode): boolean {
  if (node.kind === 'dir') return true;
  if (hasChildren(node)) {
    for (const child of node.children) {
      if (detectTreeMode(child)) return true;
    }
  }
  return false;
}
