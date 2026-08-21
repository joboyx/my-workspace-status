/**
 * Build workspace → repo → (checkout) → (dir) → file tree from snapshots.
 */

import { fileChangesFromSnapshot } from '../../changes.js';
import {
  compareRepoPathsForDisplay,
  getSyncEmoji,
  isAttentionSyncNote,
  isDefaultBranch,
  isDetachedHeadBranch,
} from '../../helpers.js';
import type { FileChange, RepoSnapshot, SyncStatus } from '../../types.js';
import {
  ICON_BRANCH,
  ICON_CLEAN,
  ICON_FOLDER,
  ICON_IGNORED,
  ICON_LINKED_WORKTREE,
  ICON_REPO,
  ICON_WORKSPACE,
  fileIcon,
  statusColor,
  statusLetterFromChange,
  syncColor as syncColorOf,
  tuiFileBadge,
  tuiMergeMark,
  tuiSyncMark,
} from '../icons.js';
import { getTheme, segmentsText } from '../theme.js';
import type { Segment } from '../theme.js';
import type {
  BuildTreeInput,
  CheckoutNode,
  DirNode,
  FileNode,
  GroupNode,
  RepoNode,
  TreeNode,
  WorkspaceNode,
} from './types.js';

/** Re-export for TUI callers; canonical definition lives in `helpers.ts`. */
export { compareRepoPathsForDisplay };

function hasFileChanges(snapshot: RepoSnapshot): boolean {
  return snapshot.hasStaged || snapshot.hasUnstaged || snapshot.hasUntracked;
}

/**
 * True when the TUI should treat the repo as needing attention: file changes,
 * a non-default branch (even with a spotless worktree), a non-idle sync
 * (`behind` / `ahead` / `diverged`), or an attention sync note
 * (`no commits yet` / `status failed`).
 */
export function repoNeedsAttention(snapshot: RepoSnapshot): boolean {
  return (
    hasFileChanges(snapshot) ||
    !isDefaultBranch(snapshot.branch, snapshot.defaultBranchOverride) ||
    snapshot.syncStatus === 'behind' ||
    snapshot.syncStatus === 'ahead' ||
    snapshot.syncStatus === 'diverged' ||
    isAttentionSyncNote(snapshot.syncNote)
  );
}

/**
 * True when a repo row is a family container (children are checkouts).
 */
export function isCheckoutFamily(node: RepoNode): boolean {
  return node.children.some((c) => c.kind === 'checkout');
}

/**
 * Primary checkout path under a family container, else the repo path.
 */
export function primaryCheckoutPath(node: RepoNode): string {
  const primary = node.children.find(
    (c): c is CheckoutNode => c.kind === 'checkout' && c.checkoutKind === 'primary',
  );
  return primary?.path ?? node.path;
}

/**
 * Checkout paths under a family container (empty when flat).
 */
export function familyCheckoutPaths(node: RepoNode): string[] {
  return node.children.filter((c): c is CheckoutNode => c.kind === 'checkout').map((c) => c.path);
}

/** Worst-of-family ranking: diverged > behind > ahead > no-upstream > up-to-date. */
function syncWorstRank(status: SyncStatus): number {
  switch (status) {
    case 'diverged':
      return 4;
    case 'behind':
      return 3;
    case 'ahead':
      return 2;
    case 'no-upstream':
      return 1;
    default:
      return 0;
  }
}

/**
 * Pick the worst sync status in a family (brief ranking).
 */
export function worstSyncStatus(statuses: readonly SyncStatus[]): SyncStatus {
  let worst: SyncStatus = 'up-to-date';
  let rank = -1;
  for (const s of statuses) {
    const r = syncWorstRank(s);
    if (r > rank) {
      rank = r;
      worst = s;
    }
  }
  return worst;
}

function makeFileNode(repoPath: string, change: FileChange): FileNode {
  return {
    kind: 'file',
    id: `file:${repoPath}:${change.path}`,
    path: change.path,
    repoPath,
    status: statusLetterFromChange(change),
    staged: Boolean(change.stagedStatus),
    unstaged: Boolean(change.unstagedStatus),
    untracked: Boolean(change.untracked),
    renameFrom: change.oldPath,
    change,
  };
}

interface MutableDir {
  dirs: Map<string, MutableDir>;
  files: FileChange[];
}

function addChange(root: MutableDir, change: FileChange): void {
  const parts = change.path.split('/').filter(Boolean);
  const fileName = parts.pop();
  if (!fileName) return;
  let node = root;
  for (const dir of parts) {
    let child = node.dirs.get(dir);
    if (!child) {
      child = { dirs: new Map(), files: [] };
      node.dirs.set(dir, child);
    }
    node = child;
  }
  node.files.push(change);
}

function collapseDir(name: string, node: MutableDir): { name: string; node: MutableDir } {
  let collapsedName = name;
  let collapsedNode = node;
  while (collapsedNode.files.length === 0 && collapsedNode.dirs.size === 1) {
    const [[childName, childNode]] = [...collapsedNode.dirs.entries()];
    collapsedName += `/${childName}`;
    collapsedNode = childNode!;
  }
  return { name: collapsedName, node: collapsedNode };
}

function materializeDir(repoPath: string, dirPath: string, node: MutableDir): TreeNode[] {
  const dirEntries = [...node.dirs.entries()]
    .map(([name, child]) => collapseDir(name, child))
    .sort((a, b) => a.name.localeCompare(b.name));
  const fileEntries = [...node.files].sort((a, b) => a.path.localeCompare(b.path));

  const children: TreeNode[] = [];
  for (const entry of dirEntries) {
    const fullPath = dirPath ? `${dirPath}/${entry.name}` : entry.name;
    const dirNode: DirNode = {
      kind: 'dir',
      id: `dir:${repoPath}:${fullPath}`,
      path: fullPath,
      name: entry.name,
      repoPath,
      children: materializeDir(repoPath, fullPath, entry.node),
    };
    children.push(dirNode);
  }
  for (const change of fileEntries) {
    children.push(makeFileNode(repoPath, change));
  }
  return children;
}

/**
 * Materialize a file/dir forest from FileChange rows (no workspace/repo wrapper).
 * Shared by workspace repo children and commit-scoped file trees.
 */
export function materializeChangeForest(
  repoPath: string,
  changes: FileChange[],
  treeMode: boolean,
): TreeNode[] {
  if (!treeMode) {
    return changes.map((change) => makeFileNode(repoPath, change));
  }
  const root: MutableDir = { dirs: new Map(), files: [] };
  for (const change of changes) addChange(root, change);
  return materializeDir(repoPath, '', root);
}

function buildRepoChildren(snapshot: RepoSnapshot, treeMode: boolean): TreeNode[] {
  return materializeChangeForest(snapshot.repo, fileChangesFromSnapshot(snapshot), treeMode);
}

function makeRepoNode(snapshot: RepoSnapshot, ignored: boolean, treeMode: boolean): RepoNode {
  return {
    kind: 'repo',
    id: `repo:${snapshot.repo}`,
    path: snapshot.repo,
    branch: snapshot.branch,
    defaultBranchOverride: snapshot.defaultBranchOverride,
    checkoutKind: snapshot.checkoutKind,
    ...(snapshot.primaryRepo ? { primaryRepo: snapshot.primaryRepo } : {}),
    mergedIntoDefault: snapshot.mergedIntoDefault,
    sync: tuiSyncMark(snapshot.syncStatus, snapshot.syncNote),
    syncStatus: snapshot.syncStatus,
    ignored,
    changeCount: fileChangesFromSnapshot(snapshot).length,
    children: buildRepoChildren(snapshot, treeMode),
  };
}

/**
 * Stable id for a checkout row — must stay in sync with watch/flash helpers.
 */
export function checkoutNodeId(path: string): string {
  return `checkout:${path}`;
}

function makeCheckoutNode(snapshot: RepoSnapshot, treeMode: boolean): CheckoutNode {
  return {
    kind: 'checkout',
    id: checkoutNodeId(snapshot.repo),
    path: snapshot.repo,
    branch: snapshot.branch,
    defaultBranchOverride: snapshot.defaultBranchOverride,
    checkoutKind: snapshot.checkoutKind,
    ...(snapshot.primaryRepo ? { primaryRepo: snapshot.primaryRepo } : {}),
    mergedIntoDefault: snapshot.mergedIntoDefault,
    sync: tuiSyncMark(snapshot.syncStatus, snapshot.syncNote),
    syncStatus: snapshot.syncStatus,
    changeCount: fileChangesFromSnapshot(snapshot).length,
    children: buildRepoChildren(snapshot, treeMode),
  };
}

function makeFamilyRepoNode(
  primaryPath: string,
  snapshots: RepoSnapshot[],
  ignoredRepos: Set<string>,
  treeMode: boolean,
): RepoNode {
  const primarySnap = snapshots.find((s) => s.checkoutKind === 'primary');
  const sorted = [...snapshots].sort(compareRepoPathsForDisplay);
  const checkouts = sorted.map((s) => makeCheckoutNode(s, treeMode));
  const changeCount = checkouts.reduce((n, c) => n + c.changeCount, 0);
  const worst = worstSyncStatus(checkouts.map((c) => c.syncStatus));
  const noteForWorst = sorted.find((s) => s.syncStatus === worst)?.syncNote ?? '';
  return {
    kind: 'repo',
    id: `repo:${primaryPath}`,
    path: primaryPath,
    branch: '',
    defaultBranchOverride: primarySnap?.defaultBranchOverride,
    checkoutKind: 'primary',
    mergedIntoDefault: null,
    sync: tuiSyncMark(worst, noteForWorst),
    syncStatus: worst,
    ignored: ignoredRepos.has(primaryPath),
    changeCount,
    children: checkouts,
  };
}

/** Short path under the primary for linked checkouts (fallback: basename). */
function linkedShortName(path: string, primaryRepo?: string): string {
  if (primaryRepo && path.startsWith(`${primaryRepo}/`)) {
    return path.slice(primaryRepo.length + 1);
  }
  const slash = path.lastIndexOf('/');
  return slash >= 0 ? path.slice(slash + 1) : path;
}

function syncSummary(snapshots: RepoSnapshot[]): string {
  if (snapshots.length === 0) return 'no repos';
  let behind = 0;
  let ahead = 0;
  let diverged = 0;
  let attention = 0;
  for (const s of snapshots) {
    if (isAttentionSyncNote(s.syncNote)) attention++;
    else if (s.syncStatus === 'behind') behind++;
    else if (s.syncStatus === 'ahead') ahead++;
    else if (s.syncStatus === 'diverged') diverged++;
  }
  const parts: string[] = [];
  if (behind) parts.push(`${behind} behind`);
  if (ahead) parts.push(`${ahead} ahead`);
  if (diverged) parts.push(`${diverged} diverged`);
  if (attention) parts.push(`${attention} attention`);
  return parts.length > 0 ? parts.join(', ') : 'all current';
}

/** A tree row split into a left run and a right-aligned run. */
export interface NodeSegments {
  segments: Segment[];
  trailing: Segment[];
}

function icon(text: string, color: string): Segment {
  return { text: `${text} `, color };
}

/**
 * True when the clean / no-updates check (`ICON_CLEAN`) should paint on a
 * repo, checkout, or folder row. Only descendants of `group:no-updates`.
 */
export function showCleanCheck(inNoUpdates: boolean): boolean {
  return inNoUpdates;
}

/**
 * Trailing sync run. The up-to-date `` (ICON_CLEAN) is gated to No updates;
 * behind / ahead / diverged / no-upstream marks always paint.
 */
function syncTrailing(
  node: { sync: string; syncStatus: SyncStatus },
  inNoUpdates: boolean,
): Segment[] {
  if (node.syncStatus === 'up-to-date') {
    if (!showCleanCheck(inNoUpdates)) return [];
    return [{ text: ICON_CLEAN, color: syncColorOf(node.syncStatus) }];
  }
  return [{ text: node.sync, color: syncColorOf(node.syncStatus) }];
}

function fileSegments(node: FileNode, treeMode: boolean): NodeSegments {
  const p = getTheme().palette;
  const status = statusLetterFromChange(node.change);
  const color = statusColor(status);
  const { glyph, color: iconColor } = fileIcon(node.path);

  const name = node.path.split('/').pop() ?? node.path;
  const dir = node.path.slice(0, node.path.length - name.length).replace(/\/$/, '');

  const segments: Segment[] = [icon(glyph, iconColor), { text: name, color }];

  if (node.renameFrom) {
    const oldName = treeMode
      ? (node.renameFrom.split('/').pop() ?? node.renameFrom)
      : node.renameFrom;
    segments.splice(1, 0, { text: `${oldName} → `, color: p.muted });
  }
  // Flat mode keeps the containing directory visible, dimmed after the name.
  if (!treeMode && dir) {
    segments.push({ text: `  ${dir}`, color: p.muted, dim: true });
  }

  return {
    segments,
    trailing: [{ text: tuiFileBadge(status), color, bold: true }],
  };
}

function repoSegments(node: RepoNode, inNoUpdates: boolean): NodeSegments {
  const p = getTheme().palette;
  const nameColor = node.ignored ? p.muted : p.repo;

  // Family container: path + wt count; branch lives on checkout children.
  if (isCheckoutFamily(node)) {
    const wtCount = familyCheckoutPaths(node).length;
    const segments: Segment[] = [
      icon(ICON_REPO, node.ignored ? p.muted : p.heading),
      { text: node.path, color: nameColor, bold: true },
    ];
    if (node.ignored) {
      segments.push(icon(` ${ICON_IGNORED}`, p.muted));
    }
    const syncSegs = syncTrailing(node, inNoUpdates);
    const wtPrefix = syncSegs.length > 0 ? '  ' : '';
    const trailing: Segment[] = [
      ...syncSegs,
      { text: `${wtPrefix}${wtCount} wt`, color: p.muted },
    ];
    if (node.changeCount > 0) {
      trailing.push({ text: `  ${node.changeCount}`, color: p.muted });
    }
    return { segments, trailing };
  }

  // Flat repo (no linked worktrees): today's chrome.
  const offDefault = !isDefaultBranch(node.branch, node.defaultBranchOverride);
  const merge = tuiMergeMark(node.mergedIntoDefault);
  const branchColor = offDefault ? p.branchFeature : p.branchDefault;
  const linked = node.checkoutKind === 'linked';
  const repoIcon = linked ? ICON_LINKED_WORKTREE : ICON_REPO;

  const segments: Segment[] = [icon(repoIcon, node.ignored ? p.muted : p.heading)];
  if (linked) {
    segments.push({
      text: linkedShortName(node.path, node.primaryRepo),
      color: nameColor,
      bold: true,
    });
    if (node.primaryRepo) {
      segments.push({ text: ` · ${node.primaryRepo}`, color: p.muted, dim: true });
    }
  } else {
    segments.push({ text: node.path, color: nameColor, bold: true });
  }
  if (node.ignored) {
    segments.push(icon(` ${ICON_IGNORED}`, p.muted));
  }
  const branchText = merge ? `${node.branch} ${merge}` : node.branch;
  segments.push({ text: '  ', dim: true }, icon(ICON_BRANCH, branchColor), {
    text: branchText,
    color: branchColor,
  });

  const trailing: Segment[] = [...syncTrailing(node, inNoUpdates)];
  if (node.changeCount > 0) {
    trailing.push({ text: `  ${node.changeCount}`, color: p.muted });
  }
  return { segments, trailing };
}

function checkoutSegments(node: CheckoutNode, inNoUpdates: boolean): NodeSegments {
  const p = getTheme().palette;
  const offDefault = !isDefaultBranch(node.branch, node.defaultBranchOverride);
  const merge = tuiMergeMark(node.mergedIntoDefault);
  const branchColor = offDefault ? p.branchFeature : p.branchDefault;
  const linked = node.checkoutKind === 'linked';
  const rowIcon = linked ? ICON_LINKED_WORKTREE : ICON_BRANCH;

  const mainLabel =
    linked && isDetachedHeadBranch(node.branch)
      ? linkedShortName(node.path, node.primaryRepo)
      : node.branch;
  const branchText = merge ? `${mainLabel} ${merge}` : mainLabel;

  const segments: Segment[] = [
    icon(rowIcon, p.heading),
    { text: branchText, color: branchColor, bold: true },
  ];

  const trailing: Segment[] = [...syncTrailing(node, inNoUpdates)];
  if (node.changeCount > 0) {
    trailing.push({ text: `  ${node.changeCount}`, color: p.muted });
  }
  return { segments, trailing };
}

function workspaceSegments(node: WorkspaceNode): NodeSegments {
  const p = getTheme().palette;
  return {
    segments: [icon(ICON_WORKSPACE, p.heading), { text: node.label, color: p.heading, bold: true }],
    trailing: [
      {
        text: `${node.changeCount} changed · ${node.syncSummary}`,
        color: p.muted,
      },
    ],
  };
}

/**
 * Styled segments for a tree node — the TUI's only label source.
 * `trailing` is right-aligned by the pane; `segments` flow from the left.
 */
export function nodeSegments(
  node: TreeNode,
  treeMode: boolean,
  inNoUpdates = false,
): NodeSegments {
  switch (node.kind) {
    case 'workspace':
      return workspaceSegments(node);
    case 'repo':
      return repoSegments(node, inNoUpdates);
    case 'checkout':
      return checkoutSegments(node, inNoUpdates);
    case 'group': {
      const p = getTheme().palette;
      return {
        segments: [icon(ICON_CLEAN, p.muted), { text: 'No updates', color: p.muted }],
        trailing: [{ text: `${node.children.length}`, color: p.muted }],
      };
    }
    case 'dir': {
      const p = getTheme().palette;
      return {
        segments: [icon(ICON_FOLDER, p.dir), { text: node.name, color: p.dir }],
        trailing: [],
      };
    }
    case 'file':
      return fileSegments(node, treeMode);
  }
}

/** Plain-text label for a node — filtering, truncation probes and tests. */
export function nodeLabel(node: TreeNode, treeMode: boolean, inNoUpdates = false): string {
  const { segments, trailing } = nodeSegments(node, treeMode, inNoUpdates);
  const left = segmentsText(segments).trimEnd();
  const right = segmentsText(trailing).trim();
  return right ? `${left}  ${right}` : left;
}

/** Plain-report sync emoji (not used for TUI labels). */
export function repoSyncEmoji(status: SyncStatus): string {
  return getSyncEmoji(status);
}

function groupKey(snapshot: RepoSnapshot): string {
  return snapshot.primaryRepo ?? snapshot.repo;
}

/**
 * True when this checkout is on the ignore list and is not a named filter.
 * Linked checkouts follow the primary path when `primaryRepo` is set.
 */
export function isHiddenIgnoredRepo(
  snapshot: RepoSnapshot,
  ignoredRepos: ReadonlySet<string>,
  namedRepos: ReadonlySet<string> = new Set(),
): boolean {
  const keys = [snapshot.repo, snapshot.primaryRepo].filter((key): key is string => Boolean(key));
  if (keys.some((key) => namedRepos.has(key))) return false;
  return keys.some((key) => ignoredRepos.has(key));
}

/**
 * Snapshots that belong in the workspace tree for the current ignored-repo view.
 * Hidden ignored repos drop out. Named filter repos stay, even when ignored.
 * When `showIgnored` is on, every snapshot stays (same as CLI `-a`).
 */
export function snapshotsForView(
  snapshots: RepoSnapshot[],
  ignoredRepos: ReadonlySet<string>,
  showIgnored: boolean,
  namedRepos: ReadonlySet<string> = new Set(),
): RepoSnapshot[] {
  if (showIgnored || ignoredRepos.size === 0) return snapshots;
  return snapshots.filter((s) => !isHiddenIgnoredRepo(s, ignoredRepos, namedRepos));
}

/**
 * Build a single workspace root tree from snapshots.
 * Repos that need attention (file changes, non-default branch, behind, or an
 * attention sync note) are top-level; clean up-to-date default-branch repos go
 * under `no-updates`. Off-default / behind / attention-note repos stay top-level
 * even when the worktree is spotless.
 *
 * When a primary has linked worktrees, they nest under a family `RepoNode`
 * container; otherwise the primary stays a flat `RepoNode` (branch on the same row).
 * Linked-only families (no primary snapshot, e.g. named filter on a linked path)
 * flat-render as `RepoNode` rows — never invent a phantom primary container.
 */
export function buildTree(input: BuildTreeInput): WorkspaceNode {
  const { snapshots, ignoredRepos, treeMode, workspaceLabel } = input;
  const withChanges: RepoNode[] = [];
  const withoutChanges: RepoNode[] = [];
  let changeCount = 0;

  for (const snapshot of snapshots) {
    changeCount += fileChangesFromSnapshot(snapshot).length;
  }

  const byFamily = new Map<string, RepoSnapshot[]>();
  for (const snapshot of snapshots) {
    const key = groupKey(snapshot);
    const list = byFamily.get(key) ?? [];
    list.push(snapshot);
    byFamily.set(key, list);
  }

  const familyKeys = [...byFamily.keys()].sort((a, b) => a.localeCompare(b));
  for (const key of familyKeys) {
    const family = byFamily.get(key)!;
    const hasLinked = family.some((s) => s.checkoutKind === 'linked');
    const hasPrimary = family.some((s) => s.checkoutKind === 'primary');

    // Nest only when a primary snapshot is present. Linked-only (e.g. named
    // filter on a linked path) stays flat — never invent a phantom container.
    if (hasLinked && hasPrimary) {
      const repoNode = makeFamilyRepoNode(key, family, ignoredRepos, treeMode);
      if (family.some(repoNeedsAttention)) withChanges.push(repoNode);
      else withoutChanges.push(repoNode);
      continue;
    }

    for (const snapshot of [...family].sort(compareRepoPathsForDisplay)) {
      const repoNode = makeRepoNode(snapshot, ignoredRepos.has(snapshot.repo), treeMode);
      if (repoNeedsAttention(snapshot)) withChanges.push(repoNode);
      else withoutChanges.push(repoNode);
    }
  }

  const children: TreeNode[] = [...withChanges];
  if (withoutChanges.length > 0) {
    const group: GroupNode = {
      kind: 'group',
      id: 'group:no-updates',
      children: withoutChanges,
    };
    children.push(group);
  }

  return {
    kind: 'workspace',
    id: 'workspace',
    label: workspaceLabel,
    changeCount,
    syncSummary: syncSummary(snapshots),
    children,
  };
}
