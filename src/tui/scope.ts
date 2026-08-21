/**
 * Resolve which files or checkouts a write action should touch for the focused row.
 */

import type { RepoSnapshot } from '../types.js';
import { isCheckoutFamily, isHiddenIgnoredRepo, primaryCheckoutPath } from './model/tree.js';
import type { FileNode, TreeNode, VisibleRow } from './model/types.js';

function walkFiles(node: TreeNode, out: FileNode[]): void {
  if (node.kind === 'file') {
    out.push(node);
    return;
  }
  // Never recurse into sibling checkouts / nested repos as files of a parent.
  if (node.kind === 'repo' || node.kind === 'checkout') {
    for (const child of node.children) {
      if (child.kind === 'file' || child.kind === 'dir') walkFiles(child, out);
    }
    return;
  }
  if ('children' in node) {
    for (const child of node.children) walkFiles(child, out);
  }
}

/**
 * Files under the focused row: one file, a directory subtree, a checkout, or a
 * flat repo. Family containers (repo with checkout children) yield no files —
 * stage/unstage/revert belong on the checkout rows. Workspace/group → empty.
 */
export function collectFiles(focused: VisibleRow): FileNode[] {
  const node = focused.node;
  if (node.kind === 'file') return [node];
  if (node.kind === 'dir' || node.kind === 'checkout') {
    const out: FileNode[] = [];
    walkFiles(node, out);
    return out;
  }
  if (node.kind === 'repo') {
    // Family container: do not mix linked checkout files.
    if (node.children.some((c) => c.kind === 'checkout')) return [];
    const out: FileNode[] = [];
    walkFiles(node, out);
    return out;
  }
  return [];
}

function primarySnapshotPaths(snapshots: readonly RepoSnapshot[]): string[] {
  return snapshots.filter((s) => s.checkoutKind === 'primary').map((s) => s.repo);
}

function checkoutIsIgnored(
  path: string,
  snapshots: readonly RepoSnapshot[],
  ignoredRepos: ReadonlySet<string>,
): boolean {
  const snap = snapshots.find((s) => s.repo === path);
  if (snap) return isHiddenIgnoredRepo(snap, ignoredRepos);
  return ignoredRepos.has(path);
}

/**
 * Drop ignored checkouts while they are hidden. When ignored repos are shown
 * (`.` / `-a`), they stay in the path list like any other visible row.
 */
function excludeHiddenIgnored(
  paths: string[],
  snapshots: readonly RepoSnapshot[],
  ignoredRepos: ReadonlySet<string>,
  showIgnored: boolean,
): string[] {
  if (showIgnored || ignoredRepos.size === 0) return paths;
  return paths.filter((path) => !checkoutIsIgnored(path, snapshots, ignoredRepos));
}

/**
 * Checkout paths a bulk git write (fetch / pull / push / default-branch)
 * should touch for the focused row.
 *
 * Workspace and family rows resolve to primary checkouts only. A linked
 * worktree is included only when the focused row is that checkout, a file or
 * dir under it, or a flat linked repo (named-filter linked-only). Sibling
 * worktrees are never added just because they share a repo.
 *
 * While ignored repos are hidden, paths on `ignoredRepos` are omitted even if
 * that checkout is focused. While they are shown, they follow the same
 * primary / focused-worktree rule as other visible rows.
 */
export function collectBulkGitTargets(
  focused: VisibleRow | null,
  snapshots: readonly RepoSnapshot[],
  ignoredRepos: ReadonlySet<string> = new Set(),
  showIgnored = false,
): string[] {
  if (!focused) return [];
  const node = focused.node;
  let paths: string[];
  switch (node.kind) {
    case 'workspace':
      paths = primarySnapshotPaths(snapshots);
      break;
    case 'group':
      return [];
    case 'checkout':
      paths = [node.path];
      break;
    case 'file':
    case 'dir':
      paths = [node.repoPath];
      break;
    case 'repo':
      paths = isCheckoutFamily(node) ? [primaryCheckoutPath(node)] : [node.path];
      break;
  }
  return excludeHiddenIgnored(paths, snapshots, ignoredRepos, showIgnored);
}

/**
 * Snapshot paths background fetch may touch.
 * Hidden ignored checkouts are omitted. When ignored repos are shown, every
 * snapshot path is included (same as other visible rows).
 */
export function collectBackgroundFetchTargets(
  snapshots: readonly RepoSnapshot[],
  ignoredRepos: ReadonlySet<string> = new Set(),
  showIgnored = false,
): string[] {
  return snapshots
    .map((s) => s.repo)
    .filter((path) => {
      if (showIgnored || ignoredRepos.size === 0) return true;
      return !checkoutIsIgnored(path, snapshots, ignoredRepos);
    });
}
