import type { ViewDepth } from '../nav/stack.js';
import { WORKTREE_COMMIT_ID, type CommitFileSource } from './types.js';

/**
 * Minimal graph-row identity needed to pick a commit-file source.
 */
export type GraphRowRef =
  | { kind: 'commit'; commitId: string }
  | { kind: 'stash'; stashRef: string; commitId?: string }
  | { kind: 'uncommitted' };

/**
 * Map nav view + optional graph cursor to a commit-file load source.
 */
export function commitFileSourceFromNav(
  view: ViewDepth,
  graphRow: GraphRowRef | null,
): CommitFileSource | null {
  if (view.kind === 'workspace') return null;

  if (graphRow?.kind === 'uncommitted') {
    return { kind: 'worktree' };
  }
  if (graphRow?.kind === 'stash') {
    return { kind: 'stash', stashRef: graphRow.stashRef };
  }
  if (graphRow?.kind === 'commit') {
    return { kind: 'commit', commitId: graphRow.commitId };
  }

  if (view.kind === 'commitFiles') {
    if (view.commitId === WORKTREE_COMMIT_ID) return { kind: 'worktree' };
    if (view.commitId) return { kind: 'commit', commitId: view.commitId };
  }

  if (view.kind === 'repoGraph') {
    if (view.commitId === WORKTREE_COMMIT_ID || view.commitId === null) {
      // null alone is ambiguous — need a graph row; without it refuse
      if (view.commitId === WORKTREE_COMMIT_ID) return { kind: 'worktree' };
      return null;
    }
    return { kind: 'commit', commitId: view.commitId };
  }

  return null;
}
