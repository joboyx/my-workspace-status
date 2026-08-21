/**
 * Build NavDrillContext from the highlighted workspace-tree row or graph row.
 */

import type { GraphListRow } from '../graph/list.js';
import type { VisibleRow } from '../model/types.js';
import type { NavDrillContext } from './stack.js';

/**
 * Map the focused visible row into drill context for right+Enter pushes.
 * Commit ids stay null until the graph UI selects a commit.
 */
export function drillContextFromFocused(
  focused: VisibleRow | undefined,
): NavDrillContext {
  if (!focused) {
    return { repo: '', commitId: null, filePath: null };
  }
  const node = focused.node;
  if (node.kind === 'checkout') {
    return { repo: node.path, commitId: null, filePath: null };
  }
  if (node.kind === 'repo') {
    // Family container → primary checkout path for graph drill.
    const primary = node.children.find(
      (c) => c.kind === 'checkout' && c.checkoutKind === 'primary',
    );
    return {
      repo: primary && primary.kind === 'checkout' ? primary.path : node.path,
      commitId: null,
      filePath: null,
    };
  }
  if (node.kind === 'file') {
    return { repo: node.repoPath, commitId: null, filePath: node.path };
  }
  if (node.kind === 'dir') {
    return { repo: node.repoPath, commitId: null, filePath: null };
  }
  return { repo: '', commitId: null, filePath: null };
}

/**
 * Map a graph list selection into drill context for depth-1 Enter.
 */
export function drillContextFromGraph(
  repo: string,
  row: GraphListRow | undefined,
): NavDrillContext {
  if (!row) return { repo, commitId: null, filePath: null };
  if (row.kind === 'uncommitted' || row.kind === 'spacer') {
    return { repo, commitId: null, filePath: null };
  }
  return { repo, commitId: row.commitId, filePath: null };
}
