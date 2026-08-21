/**
 * State-dependent visibility for tree write actions.
 *
 * Shared by the hint bar and the dispatcher so advertised keys and what
 * actually fires cannot drift (same contract as `actionVisibleForGraphRow`).
 */

import { DETACHED_HEAD_BRANCH, isDefaultBranch } from '../../helpers.js';
import type { RepoSnapshot } from '../../types.js';
import type { CheckoutNode, FileNode, RepoNode, VisibleRow } from '../model/types.js';
import { isCheckoutFamily } from '../model/tree.js';
import { collectBulkGitTargets, collectFiles } from '../scope.js';
import type { FocusPane } from '../nav/stack.js';
import type { ActionId, ActionSpec, NavDepthIndex } from './registry.js';

/**
 * Context needed to decide which depth-0 tree actions are currently valid.
 */
export interface ActionGateContext {
  /** Focused workspace-tree row (not a commit-files synthetic focus). */
  readonly focused: VisibleRow | null;
  readonly snapshots: readonly RepoSnapshot[];
  readonly navDepth: NavDepthIndex;
  /** Workspace `ignoredRepos`. Hidden ignored paths are skipped on bulk ops. */
  readonly ignoredRepos?: ReadonlySet<string>;
  /** Session `.` / `-a` flag. When true, ignored repos are treated as visible. */
  readonly showIgnored?: boolean;
}

/**
 * Tree write actions that `useAppState` already refuses at depth ≥ 1 /
 * commit-files focus. Hints must hide the same set.
 */
export const TREE_WRITE_BLOCKED_IDS: ReadonlySet<ActionId> = new Set([
  'stage',
  'unstage',
  'revert',
  'fetch',
  'pull',
  'push',
  'defaultBranch',
  'branch',
  'removeWorktree',
]);

/**
 * True when tree write actions are blocked for the current ViewStack depth
 * (matches `commitFilesWriteBlocked` in `useAppState`: depth ≥ 1 covers
 * commit-files focus at depth 1 right and the whole depth-2 leaf).
 */
export function isTreeWriteBlockedAtDepth(depth: NavDepthIndex): boolean {
  return depth >= 1;
}

/**
 * True when tree-write hints must hide because those actions cannot fire:
 * ViewStack depth ≥ 1, or the right pane is focused. Tree writes are
 * left-list only (`rightPaneLeftListAllowed` never lets them through).
 * Graph writes are different specs and are not in `TREE_WRITE_BLOCKED_IDS`.
 * File-diff `e` / `ctrl+o` stay visible because they are not tree writes.
 */
export function treeWritesHiddenForContext(depth: NavDepthIndex, focusPane: FocusPane): boolean {
  return isTreeWriteBlockedAtDepth(depth) || focusPane === 'right';
}

/**
 * True when `x` should touch this file: unstaged worktree changes or untracked.
 * Staged-only files are skipped (worktree already matches the index).
 */
export function isRevertible(file: FileNode): boolean {
  return file.unstaged || file.untracked;
}

const STAGE_KINDS = new Set(['file', 'dir', 'repo', 'checkout']);

/** Scope has at least one file with unstaged changes or untracked. */
export function canStage(focused: VisibleRow | null): boolean {
  if (!focused) return false;
  if (!STAGE_KINDS.has(focused.node.kind)) return false;
  return collectFiles(focused).some((f) => f.unstaged || f.untracked);
}

/** Scope has at least one staged file. */
export function canUnstage(focused: VisibleRow | null): boolean {
  if (!focused) return false;
  if (!STAGE_KINDS.has(focused.node.kind)) return false;
  return collectFiles(focused).some((f) => f.staged);
}

/** Scope has staged, unstaged, or untracked files (stash push). */
export function canStashPush(focused: VisibleRow | null): boolean {
  if (!focused) return false;
  if (!STAGE_KINDS.has(focused.node.kind)) return false;
  return collectFiles(focused).some((f) => f.staged || f.unstaged || f.untracked);
}

/**
 * True when a workspace-tree file currently has uncommitted or unstaged diffs.
 */
export function isViewableFile(file: FileNode): boolean {
  return file.staged || file.unstaged || file.untracked;
}

/**
 * Space is valid on a dirty workspace-tree file at depth 0 only.
 * Repo / dir / workspace / graph / commit-file rows never get the action.
 */
export function canToggleViewed(focused: VisibleRow | null, navDepth: NavDepthIndex): boolean {
  if (!focused || focused.node.kind !== 'file') return false;
  if (navDepth >= 1) return false;
  return isViewableFile(focused.node);
}

/** Scope has discardable work (unstaged and/or untracked). */
export function canRevert(focused: VisibleRow | null): boolean {
  if (!focused) return false;
  if (!STAGE_KINDS.has(focused.node.kind)) return false;
  return collectFiles(focused).some(isRevertible);
}

function snapshotBehind(s: RepoSnapshot): boolean {
  return s.syncStatus === 'behind';
}

function branchPushable(branch: string): boolean {
  return branch !== DETACHED_HEAD_BRANCH && branch !== '(unknown)';
}

function snapshotPushable(s: RepoSnapshot): boolean {
  if (!branchPushable(s.branch)) return false;
  return s.syncStatus === 'ahead' || s.syncStatus === 'diverged' || s.syncStatus === 'no-upstream';
}

function syncStatusPushable(status: RepoSnapshot['syncStatus'], branch: string): boolean {
  if (!branchPushable(branch)) return false;
  return status === 'ahead' || status === 'diverged' || status === 'no-upstream';
}

function snapshotNonDefault(s: RepoSnapshot): boolean {
  return !isDefaultBranch(s.branch, s.defaultBranchOverride);
}

/**
 * Pull is valid when a bulk-git target is behind. Workspace and family rows
 * consider primary checkouts only; a focused linked worktree uses that path.
 */
export function canPull(
  focused: VisibleRow | null,
  snapshots: readonly RepoSnapshot[],
  ignoredRepos: ReadonlySet<string> = new Set(),
  showIgnored = false,
): boolean {
  if (!focused) return false;
  const node = focused.node;
  if (node.kind !== 'workspace' && node.kind !== 'repo' && node.kind !== 'checkout') {
    return false;
  }
  return collectBulkGitTargets(focused, snapshots, ignoredRepos, showIgnored).some((path) => {
    const snap = snapshots.find((s) => s.repo === path);
    if (snap) return snapshotBehind(snap);
    if ((node.kind === 'checkout' || node.kind === 'repo') && node.path === path) {
      return node.syncStatus === 'behind';
    }
    return false;
  });
}

/**
 * Push is valid when a bulk-git target is ahead, diverged, or has no upstream
 * (first publish). Workspace rows never push. Family rows use the primary only.
 * Detached / unknown tips never push.
 */
export function canPush(
  focused: VisibleRow | null,
  snapshots: readonly RepoSnapshot[],
  ignoredRepos: ReadonlySet<string> = new Set(),
  showIgnored = false,
): boolean {
  if (!focused) return false;
  const node = focused.node;
  if (node.kind !== 'repo' && node.kind !== 'checkout') return false;
  return collectBulkGitTargets(focused, snapshots, ignoredRepos, showIgnored).some((path) => {
    const snap = snapshots.find((s) => s.repo === path);
    if (snap) return snapshotPushable(snap);
    if ((node.kind === 'checkout' || node.kind === 'repo') && node.path === path) {
      return syncStatusPushable(node.syncStatus, node.branch);
    }
    return false;
  });
}

/**
 * Default-branch switch is valid when a bulk-git target is off the default.
 * Workspace and family rows consider primary checkouts only.
 */
export function canDefaultBranch(
  focused: VisibleRow | null,
  snapshots: readonly RepoSnapshot[],
  ignoredRepos: ReadonlySet<string> = new Set(),
  showIgnored = false,
): boolean {
  if (!focused) return false;
  const node = focused.node;
  if (node.kind !== 'workspace' && node.kind !== 'repo' && node.kind !== 'checkout') {
    return false;
  }
  return collectBulkGitTargets(focused, snapshots, ignoredRepos, showIgnored).some((path) => {
    const snap = snapshots.find((s) => s.repo === path);
    if (snap) return snapshotNonDefault(snap);
    if ((node.kind === 'checkout' || node.kind === 'repo') && node.path === path) {
      return !isDefaultBranch(node.branch, node.defaultBranchOverride);
    }
    return false;
  });
}

/**
 * Branch picker is valid on a checkout row, or on a flat repo (no nested
 * checkouts). Family containers hide `b` — pick a checkout child instead.
 */
export function canBranch(focused: VisibleRow | null): boolean {
  if (!focused) return false;
  const node = focused.node;
  if (node.kind === 'checkout') return true;
  if (node.kind === 'repo') return !isCheckoutFamily(node);
  return false;
}

/**
 * Remove worktree is valid on a linked checkout row, or a flat linked `repo`
 * row (named-filter linked-only — no nested checkout children).
 * Family containers and primary checkouts are never removable.
 */
export function canRemoveWorktree(focused: VisibleRow | null): boolean {
  if (!focused) return false;
  const node = focused.node;
  if (node.kind === 'checkout') return node.checkoutKind === 'linked';
  if (node.kind === 'repo') {
    return node.checkoutKind === 'linked' && !isCheckoutFamily(node);
  }
  return false;
}

/**
 * Hint label for remove-worktree: includes merge state when known.
 */
export function removeWorktreeHintLabel(node: CheckoutNode | RepoNode): string {
  if (node.checkoutKind === 'linked') {
    if (node.mergedIntoDefault === true) return 'remove worktree (merged)';
    if (node.mergedIntoDefault === false) return 'remove worktree (open)';
  }
  return 'remove worktree';
}

/**
 * Extra predicate for hints / dispatch that depend on focused scope + sync.
 * Graph payload gates stay in `actionVisibleForGraphRow`.
 */
export function actionVisibleForScope(action: ActionSpec, ctx: ActionGateContext): boolean {
  if (TREE_WRITE_BLOCKED_IDS.has(action.id) && isTreeWriteBlockedAtDepth(ctx.navDepth)) {
    return false;
  }
  switch (action.id) {
    case 'stage':
      return canStage(ctx.focused);
    case 'unstage':
      return canUnstage(ctx.focused);
    case 'revert':
      return canRevert(ctx.focused);
    case 'pull':
      return canPull(ctx.focused, ctx.snapshots, ctx.ignoredRepos, ctx.showIgnored);
    case 'push':
      return canPush(ctx.focused, ctx.snapshots, ctx.ignoredRepos, ctx.showIgnored);
    case 'defaultBranch':
      return canDefaultBranch(ctx.focused, ctx.snapshots, ctx.ignoredRepos, ctx.showIgnored);
    case 'branch':
      return canBranch(ctx.focused);
    case 'removeWorktree':
      return canRemoveWorktree(ctx.focused);
    case 'stashMenu':
      if (ctx.navDepth >= 2) return false;
      if (ctx.navDepth >= 1) return true;
      return canStashPush(ctx.focused);
    case 'toggleViewed':
      return canToggleViewed(ctx.focused, ctx.navDepth);
    default:
      return true;
  }
}
