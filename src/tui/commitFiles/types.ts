/**
 * Which git object the commit-file list / diff is scoped to.
 */
export type CommitFileSource =
  | { kind: 'commit'; commitId: string }
  | { kind: 'worktree' }
  | { kind: 'stash'; stashRef: string };

/**
 * Sentinel commitId used on the ViewStack for uncommitted depth-2 (P1 WORKTREE).
 */
export const WORKTREE_COMMIT_ID = 'WORKTREE';
