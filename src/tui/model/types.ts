/**
 * Pure TUI tree domain types (no Ink / React).
 */

import type { FileChange } from '../../types.js';

/** Letter code aligned with badgeForChange vocabulary. */
export type FileStatusLetter = 'A' | 'M' | 'D' | 'R' | 'C' | 'U' | 'MS' | 'S';

export type TreeNode =
  | WorkspaceNode
  | RepoNode
  | CheckoutNode
  | GroupNode
  | DirNode
  | FileNode;

export interface WorkspaceNode {
  kind: 'workspace';
  id: 'workspace';
  label: string;
  changeCount: number;
  syncSummary: string;
  children: TreeNode[];
}

export interface RepoNode {
  kind: 'repo';
  id: string;
  path: string;
  branch: string;
  /** Configured default-branch override when present. */
  defaultBranchOverride?: string;
  /** Primary checkout vs linked `git worktree add` checkout. */
  checkoutKind: 'primary' | 'linked';
  /**
   * Workspace-relative path of the primary checkout when `checkoutKind === 'linked'`.
   * Absent for primaries.
   */
  primaryRepo?: string;
  /**
   * Whether HEAD is merged into the default-branch tip (`null` = N/A / unknown).
   * Drives the TUI merge mark next to the branch name.
   * Family containers (nested checkouts) always use `null`.
   */
  mergedIntoDefault: boolean | null;
  /** Rendered sync mark (glyph + count). */
  sync: string;
  /** Raw sync state — drives the sync mark colour. */
  syncStatus: import('../../types.js').SyncStatus;
  ignored: boolean;
  /** Number of changed files in this repo (shown as a trailing count). */
  changeCount: number;
  /**
   * File/dir forest for a flat repo, or `CheckoutNode` children when this row
   * is a family container (primary has linked worktrees).
   */
  children: TreeNode[];
}

/**
 * One git checkout row under a family container (or unused when the primary
 * has no linked worktrees — those stay a flat `RepoNode`).
 */
export interface CheckoutNode {
  kind: 'checkout';
  id: string;
  /** Workspace-relative git cwd. */
  path: string;
  branch: string;
  /** Configured default-branch override when present. */
  defaultBranchOverride?: string;
  checkoutKind: 'primary' | 'linked';
  /** Set for linked checkouts — workspace-relative primary path. */
  primaryRepo?: string;
  mergedIntoDefault: boolean | null;
  sync: string;
  syncStatus: import('../../types.js').SyncStatus;
  changeCount: number;
  /** File/dir forest only. */
  children: TreeNode[];
}

export interface GroupNode {
  kind: 'group';
  id: 'group:no-updates';
  children: TreeNode[];
}

export interface DirNode {
  kind: 'dir';
  id: string;
  /** Repo-relative directory path (may be compacted, e.g. `ai/common/skills`). */
  path: string;
  /** Display name under parent (collapsed segment, e.g. `ai/common/skills`). */
  name: string;
  /** Owning repo path (for id stability). */
  repoPath: string;
  children: TreeNode[];
}

export interface FileNode {
  kind: 'file';
  id: string;
  /** Repo-relative file path. */
  path: string;
  /** Owning repo path. */
  repoPath: string;
  status: FileStatusLetter;
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
  renameFrom?: string;
  /** Original FileChange for badge/label helpers. */
  change: FileChange;
}

export interface VisibleRow {
  id: string;
  depth: number;
  node: TreeNode;
  /** Plain text of `segments` + `trailing` — used for filtering and tests. */
  label: string;
  /** Left-aligned styled run (icon, name, metadata). */
  segments: import('../theme.js').Segment[];
  /** Right-aligned styled run (status badge, counts). */
  trailing: import('../theme.js').Segment[];
}

export type FoldAction = 'toggle' | 'open' | 'close' | 'openAll' | 'closeAll' | 'toggleSubtree';

export interface BuildTreeInput {
  snapshots: import('../../types.js').RepoSnapshot[];
  ignoredRepos: Set<string>;
  treeMode: boolean;
  workspaceLabel: string;
}
