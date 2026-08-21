/**
 * Output formatting and rendering for workspace status.
 */

import type { FileChange, RepoSnapshot, SummaryState, VerboseRow } from './types.js';
import { badgeForChange, fileChangesFromSnapshot } from './changes.js';
import {
  extractTicketId,
  formatBranchWithMerge,
  formatCheckoutRepoLabel,
  isAttentionSyncNote,
  sortedUnique,
  splitEntries,
  sanitizePath,
  trimVal,
  visibleWidth,
} from './helpers.js';

export interface WorkspaceStatusRenderInput {
  snapshots: RepoSnapshot[];
  summary: SummaryState;
  verbose: {
    cleanDefault: VerboseRow[];
    cleanNonDefault: VerboseRow[];
    changeRepos: VerboseRow[];
    repoWidth: number;
    branchWidth: number;
  };
  showVerbose: boolean;
}

export function formatFileChanges(filesStr: string): string {
  const entries = splitEntries(filesStr);
  let added = 0,
    modified = 0,
    deleted = 0;
  for (const e of entries) {
    const status = e[0];
    if (status === 'A') added++;
    else if (status === 'M') modified++;
    else if (status === 'D') deleted++;
    else if (status === 'R') modified++;
  }
  const parts: string[] = [];
  if (added > 0) parts.push(`➕ ${added} added`);
  if (modified > 0) parts.push(`📝 ${modified} modified`);
  if (deleted > 0) parts.push(`➖ ${deleted} deleted`);
  return parts.length > 0 ? `(${parts.join(', ')})` : '';
}

export function formatUntrackedChanges(filesStr: string): string {
  const entries = splitEntries(filesStr);
  if (entries.length === 0) return '';
  return `(➕ ${entries.length} added)`;
}

interface FileTreeNode {
  dirs: Map<string, FileTreeNode>;
  files: FileChange[];
}

function padVisible(value: string, width: number): string {
  return value + ' '.repeat(Math.max(0, width - visibleWidth(value)));
}

function fileStatusEmoji(status: string): string {
  const m: Record<string, string> = { A: '➕', M: '📝', D: '➖', R: '🔄' };
  return m[status] ?? '';
}

function extractFilePath(entry: string): string {
  const tab = entry.indexOf('\t');
  if (tab < 0) return '';
  const status = entry[0];
  const rest = entry.slice(tab + 1);
  if (status === 'R') {
    const secondTab = rest.indexOf('\t');
    return secondTab >= 0 ? trimVal(rest.slice(secondTab + 1)) : trimVal(rest);
  }
  return trimVal(rest);
}

export function renderVerbose(
  rows: VerboseRow[],
  repoWidth: number,
  branchWidth: number,
): string[] {
  const syncWidth = Math.max('Sync'.length, ...rows.map((r) => visibleWidth(r.sync)));
  const filesWidth = Math.max('Files'.length, ...rows.map((r) => visibleWidth(r.files)));
  const lines = [
    `${padVisible('Repo', repoWidth)}  ${padVisible('Branch', branchWidth)}  ${padVisible('Sync', syncWidth)}  ${padVisible('Files', filesWidth)}`,
  ];
  for (const r of rows) {
    lines.push(
      `${padVisible(r.repo, repoWidth)}  ${padVisible(r.branch, branchWidth)}  ${padVisible(r.sync, syncWidth)}  ${padVisible(r.files, filesWidth)}${r.note ? `  ${r.note}` : ''}`,
    );
  }
  return lines;
}

export function renderTrackedFileEntries(filesStr: string): string[] {
  const entries = splitEntries(filesStr);
  const lines: string[] = [];
  for (const e of entries) {
    const filepath = sanitizePath(extractFilePath(e));
    if (!filepath) continue;
    const emoji = fileStatusEmoji(e[0]);
    if (emoji) lines.push(`      ${emoji} ${filepath}`);
  }
  return lines;
}

export function renderUntrackedFileEntries(filesStr: string): string[] {
  const entries = splitEntries(filesStr);
  return entries
    .map((e) => sanitizePath(trimVal(e)))
    .filter(Boolean)
    .map((p) => `      ➕ ${p}`);
}

function repoLabel(snapshot: RepoSnapshot): string {
  const repo = formatCheckoutRepoLabel(snapshot);
  const ticketId = extractTicketId(snapshot.branch);
  return ticketId ? `${repo} (${ticketId})` : repo;
}

function fileDisplay(change: FileChange): string {
  const name = change.path.split('/').pop() ?? change.path;
  if (change.oldPath) {
    const oldName = change.oldPath.split('/').pop() ?? change.oldPath;
    return `${badgeForChange(change)} ${oldName} -> ${name}`;
  }
  return `${badgeForChange(change)} ${name}`;
}

function addTreeChange(root: FileTreeNode, change: FileChange): void {
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

function collapseNode(name: string, node: FileTreeNode): { name: string; node: FileTreeNode } {
  let collapsedName = name;
  let collapsedNode = node;
  while (collapsedNode.files.length === 0 && collapsedNode.dirs.size === 1) {
    const [[childName, childNode]] = collapsedNode.dirs;
    collapsedName += `/${childName}`;
    collapsedNode = childNode;
  }
  return { name: collapsedName, node: collapsedNode };
}

function renderTreeNode(node: FileTreeNode, prefix: string): string[] {
  const dirEntries = [...node.dirs.entries()]
    .map(([name, child]) => collapseNode(name, child))
    .sort((a, b) => a.name.localeCompare(b.name));
  const fileEntries = [...node.files].sort((a, b) => a.path.localeCompare(b.path));
  const items: Array<{ label: string; child?: FileTreeNode }> = [
    ...dirEntries.map((entry) => ({ label: entry.name, child: entry.node })),
    ...fileEntries.map((change) => ({ label: fileDisplay(change) })),
  ];

  const lines: string[] = [];
  items.forEach((item, index) => {
    const last = index === items.length - 1;
    lines.push(`${prefix}${last ? '└─' : '├─'} ${item.label}`);
    if (item.child) lines.push(...renderTreeNode(item.child, `${prefix}${last ? '   ' : '│  '}`));
  });
  return lines;
}

function syncLabel(snapshot: RepoSnapshot): string {
  const repo = formatCheckoutRepoLabel(snapshot);
  const ticketId = extractTicketId(snapshot.branch);
  let label = `${repo} [${snapshot.branch}]`;
  if (ticketId) label += ` (${ticketId})`;
  if (snapshot.syncNote) label += ` - ${snapshot.syncNote}`;
  return label;
}

function renderRepoChangeLines(snapshot: RepoSnapshot): string[] {
  const root: FileTreeNode = { dirs: new Map(), files: [] };
  for (const change of fileChangesFromSnapshot(snapshot)) addTreeChange(root, change);
  return [`  📦 ${repoLabel(snapshot)}`, ...renderTreeNode(root, '     ')];
}

function appendSectionGap(lines: string[]): void {
  if (lines.length > 0) lines.push('');
}

function appendSyncGroupLines(
  lines: string[],
  label: string,
  repos: string[],
  snapshotMap: Map<string, RepoSnapshot>,
): void {
  if (repos.length === 0) return;
  lines.push(label);
  for (const repo of repos) {
    const snapshot = snapshotMap.get(repo);
    if (snapshot) lines.push(`    - ${syncLabel(snapshot)}`);
  }
}

function branchSummaryLabel(snapshot: RepoSnapshot): string {
  const repo = formatCheckoutRepoLabel(snapshot);
  const ticketId = extractTicketId(snapshot.branch);
  const base = ticketId ? `${repo} (${ticketId})` : `${repo} [${snapshot.branch}]`;
  return formatBranchWithMerge(base, snapshot.mergedIntoDefault);
}

function appendBranchGroupLines(
  lines: string[],
  label: string,
  repos: string[],
  snapshotMap: Map<string, RepoSnapshot>,
): void {
  if (repos.length === 0) return;
  lines.push(label);
  for (const repo of repos) {
    const snapshot = snapshotMap.get(repo);
    if (snapshot) lines.push(`    - ${branchSummaryLabel(snapshot)}`);
  }
}

function linkedWorktreeSummaryLabel(snapshot: RepoSnapshot): string {
  return branchSummaryLabel(snapshot);
}

function appendLinkedWorktreesSection(
  lines: string[],
  linkedPaths: string[],
  snapshotMap: Map<string, RepoSnapshot>,
): void {
  if (linkedPaths.length === 0) return;
  appendSectionGap(lines);
  lines.push(`🔗 Linked worktrees (${linkedPaths.length}):`);
  for (const repo of linkedPaths) {
    const snapshot = snapshotMap.get(repo);
    if (snapshot) lines.push(`    - ${linkedWorktreeSummaryLabel(snapshot)}`);
  }
}

export function renderWorkspaceStatus(input: WorkspaceStatusRenderInput): string[] {
  const { snapshots, summary, verbose, showVerbose } = input;
  const lines: string[] = [];
  const snapshotMap = new Map(snapshots.map((s) => [s.repo, s]));
  const linkedPaths = sortedUnique([...summary.linkedWorktrees]);

  if (showVerbose) {
    const rows = [...verbose.cleanDefault, ...verbose.cleanNonDefault, ...verbose.changeRepos];
    lines.push(...renderVerbose(rows, verbose.repoWidth, verbose.branchWidth));
  }

  if (snapshots.length === 0) {
    appendSectionGap(lines);
    lines.push('ℹ️ No git repos found');
    return lines;
  }

  const totalChanges =
    summary.changesUncommitted.size +
    summary.changesStaged.size +
    summary.changesBoth.size +
    summary.changesUntracked.size;
  const totalSync = summary.syncBehind.size + summary.syncAhead.size + summary.syncDiverged.size;
  const totalBranches =
    summary.branchFeature.size +
    summary.branchBugfix.size +
    summary.branchChore.size +
    summary.branchRelease.size +
    summary.branchUnknown.size;
  const attentionRepos = snapshots
    .filter((s) => isAttentionSyncNote(s.syncNote))
    .sort((a, b) => a.repo.localeCompare(b.repo));

  if (
    totalChanges === 0 &&
    totalSync === 0 &&
    totalBranches === 0 &&
    attentionRepos.length === 0
  ) {
    appendSectionGap(lines);
    lines.push('✅ All repos clean and up-to-date');
    appendLinkedWorktreesSection(lines, linkedPaths, snapshotMap);
    return lines;
  }

  if (totalChanges > 0) {
    const reposWithChanges = sortedUnique([
      ...summary.changesUncommitted,
      ...summary.changesStaged,
      ...summary.changesBoth,
      ...summary.changesUntracked,
    ]);
    appendSectionGap(lines);
    lines.push('File changes');
    reposWithChanges.forEach((repo, index) => {
      const snapshot = snapshotMap.get(repo);
      if (snapshot && index > 0) lines.push('');
      if (snapshot) lines.push(...renderRepoChangeLines(snapshot));
    });
  }

  if (totalSync > 0) {
    appendSectionGap(lines);
    lines.push(`🔄 Sync status (${totalSync}):`);
    appendSyncGroupLines(lines, '  ⬇️ behind:', sortedUnique([...summary.syncBehind]), snapshotMap);
    appendSyncGroupLines(lines, '  ⬆️ ahead:', sortedUnique([...summary.syncAhead]), snapshotMap);
    appendSyncGroupLines(
      lines,
      '  🔀 diverged:',
      sortedUnique([...summary.syncDiverged]),
      snapshotMap,
    );
  }

  if (totalBranches > 0) {
    appendSectionGap(lines);
    lines.push(`🌿 Branches (${totalBranches}):`);
    appendBranchGroupLines(
      lines,
      '  🚧 feature:',
      sortedUnique([...summary.branchFeature]),
      snapshotMap,
    );
    appendBranchGroupLines(
      lines,
      '  🐛 bugfix:',
      sortedUnique([...summary.branchBugfix]),
      snapshotMap,
    );
    appendBranchGroupLines(
      lines,
      '  🔧 chore:',
      sortedUnique([...summary.branchChore]),
      snapshotMap,
    );
    appendBranchGroupLines(
      lines,
      '  🚀 release:',
      sortedUnique([...summary.branchRelease]),
      snapshotMap,
    );
    appendBranchGroupLines(
      lines,
      '  🌿 unknown:',
      sortedUnique([...summary.branchUnknown]),
      snapshotMap,
    );
  }

  if (attentionRepos.length > 0) {
    appendSectionGap(lines);
    lines.push(`⚠️ Attention (${attentionRepos.length}):`);
    for (const snapshot of attentionRepos) {
      lines.push(`    - ${syncLabel(snapshot)}`);
    }
  }

  appendLinkedWorktreesSection(lines, linkedPaths, snapshotMap);

  return lines;
}
