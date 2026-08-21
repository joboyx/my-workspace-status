/**
 * Shared types for workspace-status.
 */

export type SyncStatus = 'up-to-date' | 'no-upstream' | 'behind' | 'ahead' | 'diverged';

export interface RepoSnapshot {
  repo: string;
  branch: string;
  syncStatus: SyncStatus;
  syncNote: string;
  hasUnstaged: boolean;
  hasStaged: boolean;
  hasUntracked: boolean;
  unstagedInfo: string;
  stagedFiles: string;
  unstagedFiles: string;
  untrackedFiles: string;
  /**
   * When set from workspace config `defaultBranches`, this is the sole default
   * branch for classification, ordering, branch markers, and default-branch
   * operations (`--default-branch` / TUI `d`). Absent → legacy git/heuristic
   * behaviour.
   */
  defaultBranchOverride?: string;
  /** Primary checkout vs linked `git worktree add` checkout. */
  checkoutKind: 'primary' | 'linked';
  /**
   * Workspace-relative path of the primary checkout when `checkoutKind === 'linked'`.
   * Absent for primaries.
   */
  primaryRepo?: string;
  /**
   * Whether HEAD is an ancestor of the resolved default-branch tip.
   * - `true` / `false` for non-default branches when the default ref is resolvable
   * - `null` when on the default branch, detached without a usable check, or default ref missing
   */
  mergedIntoDefault: boolean | null;
}

/** Single path with optional staged/unstaged/untracked markers (merged across status buckets). */
export interface FileChange {
  path: string;
  stagedStatus?: string;
  unstagedStatus?: string;
  untracked?: boolean;
  oldPath?: string;
}

export interface CliFlags {
  doFetch: boolean;
  verbose: boolean;
  doPull: boolean;
  doDefaultBranch: boolean;
  includeAll: boolean;
  /** Force plain text report (disable TUI). */
  forcePlain: boolean;
  /** Force interactive TUI even when stdout is not a TTY. */
  forceTui: boolean;
  /** Repo paths relative to workspace root; empty means all repos. */
  filterRepos: string[];
}

/** JSON config loaded from .workspace-status-config.json in the workspace root. */
export interface WorkspaceStatusConfig {
  ignoredRepos: string[];
  /**
   * How many path segments below the workspace root to search for git repos.
   * Default is 3 (e.g. `opella/light-modules/opl-lm-spa`).
   */
  maxDepth: number;
  /**
   * Per-repo default branch overrides (workspace-relative path → branch name).
   * When set for a repo, that branch is the sole default for classification and
   * default-branch operations; otherwise defaults are derived as before.
   */
  defaultBranches: Record<string, string>;
  /**
   * Command string for TUI `e` (same shape as `$EDITOR`). Omitted or blank
   * falls through to `$EDITOR` / `$VISUAL` / `vim`.
   */
  editor?: string;
}

export interface VerboseRow {
  repo: string;
  branch: string;
  sync: string;
  /** Dirty/clean working-tree column (display header: Files). */
  files: string;
  note: string;
  branchPriority?: number;
  syncPriority?: number;
}

export interface SummaryState {
  changesUncommitted: Set<string>;
  changesStaged: Set<string>;
  changesBoth: Set<string>;
  changesUntracked: Set<string>;
  syncBehind: Set<string>;
  syncAhead: Set<string>;
  syncDiverged: Set<string>;
  branchFeature: Set<string>;
  branchBugfix: Set<string>;
  branchChore: Set<string>;
  branchRelease: Set<string>;
  branchUnknown: Set<string>;
  /** Workspace-relative paths of linked git worktree checkouts. */
  linkedWorktrees: Set<string>;
}
