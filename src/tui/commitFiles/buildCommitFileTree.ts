/**
 * Build and flatten commit-scoped file forests (no workspace/repo wrapper).
 */

import type { FileChange } from '../../types.js';
import { segmentsText } from '../theme.js';
import { materializeChangeForest, nodeSegments } from '../model/tree.js';
import type { TreeNode, VisibleRow } from '../model/types.js';

/**
 * Dir/file forest for a commit / worktree / stash file list.
 */
export function buildCommitFileNodes(
  repoPath: string,
  changes: FileChange[],
  treeMode: boolean,
): TreeNode[] {
  return materializeChangeForest(repoPath, changes, treeMode);
}

function walk(
  nodes: TreeNode[],
  folds: Set<string>,
  treeMode: boolean,
  depth: number,
  out: VisibleRow[],
): void {
  for (const node of nodes) {
    const { segments, trailing } = nodeSegments(node, treeMode);
    const left = segmentsText(segments).trimEnd();
    const right = segmentsText(trailing).trim();
    out.push({
      id: node.id,
      depth,
      node,
      label: right ? `${left}  ${right}` : left,
      segments,
      trailing,
    });
    if (node.kind === 'dir' && node.children.length > 0 && !folds.has(node.id)) {
      walk(node.children, folds, treeMode, depth + 1, out);
    }
  }
}

/**
 * Flatten a commit-file forest with an explicit treeMode (B10-ready VisibleRows).
 */
export function flattenCommitFiles(
  nodes: TreeNode[],
  folds: Set<string>,
  treeMode: boolean,
): VisibleRow[] {
  const out: VisibleRow[] = [];
  walk(nodes, folds, treeMode, 0, out);
  return out;
}
