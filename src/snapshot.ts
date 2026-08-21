/**
 * Snapshot aggregation: build verbose rows and summary state from repo snapshots.
 */

import type { RepoSnapshot, SummaryState, SyncStatus, VerboseRow } from './types.js';
import {
  compareRepoPathsForDisplay,
  formatBranchWithMerge,
  formatCheckoutRepoLabel,
  getBranchEmoji,
  getBranchKind,
  getBranchPriority,
  getSyncPriority,
  isAttentionSyncNote,
  isDefaultBranch,
  sortedUnique,
  visibleWidth,
} from './helpers.js';

function syncDisplay(status: SyncStatus, note: string): string {
  if (status === 'no-upstream') {
    if (note === 'no commits yet') return '❓ no commits yet';
    if (note === 'status failed') return '⚠️ status failed';
    return '❓ no upstream';
  }
  if (status === 'behind') {
    const count = note.match(/behind by (\d+)/)?.[1] ?? '';
    return `⬇️ behind${count ? ` ${count}` : ''}`;
  }
  if (status === 'ahead') {
    const count = note.match(/ahead by (\d+)/)?.[1] ?? '';
    return `⬆️ ahead${count ? ` ${count}` : ''}`;
  }
  if (status === 'diverged') {
    const match = note.match(/ahead (\d+), behind (\d+)/);
    return match ? `🔀 ${match[1]}/${match[2]}` : '🔀 diverged';
  }
  return '✅ current';
}

function filesDisplay(snapshot: RepoSnapshot): string {
  const changedFiles = new Set<string>();
  for (const fileList of [snapshot.stagedFiles, snapshot.unstagedFiles, snapshot.untrackedFiles]) {
    for (const entry of fileList
      .split('|||')
      .map((s) => s.trim())
      .filter(Boolean)) {
      const parts = entry.split('\t');
      changedFiles.add(parts[parts.length - 1] ?? entry);
    }
  }

  if (snapshot.hasStaged && snapshot.hasUnstaged) return '⚠️ staged+dirty';
  if (snapshot.hasStaged) return '✨ staged';
  if (snapshot.hasUnstaged || snapshot.hasUntracked) return `📝 ${changedFiles.size} files`;
  return '💾 clean';
}

function toVerboseRow(s: RepoSnapshot): VerboseRow {
  const noteParts: string[] = [];

  if (s.hasStaged && s.hasUnstaged) {
    if (s.unstagedInfo) noteParts.push(s.unstagedInfo);
  } else if (s.hasUnstaged || s.hasUntracked) {
    if (s.unstagedInfo) noteParts.push(s.unstagedInfo);
  }

  return {
    repo: formatCheckoutRepoLabel(s),
    branch: formatBranchWithMerge(`${getBranchEmoji(s.branch)} ${s.branch}`, s.mergedIntoDefault),
    sync: syncDisplay(s.syncStatus, s.syncNote),
    files: filesDisplay(s),
    note: noteParts.length > 0 ? `(${noteParts.join(', ')})` : '',
  };
}

export function buildVerboseRows(snapshots: RepoSnapshot[]): {
  cleanDefault: VerboseRow[];
  cleanNonDefault: VerboseRow[];
  changeRepos: VerboseRow[];
  repoWidth: number;
  branchWidth: number;
} {
  const cleanDefaultSnaps: RepoSnapshot[] = [];
  const cleanNonDefaultSnaps: RepoSnapshot[] = [];
  const changeSnaps: RepoSnapshot[] = [];

  for (const s of snapshots) {
    const hasChanges = s.hasUnstaged || s.hasStaged || s.hasUntracked;
    if (hasChanges) {
      changeSnaps.push(s);
    } else if (isDefaultBranch(s.branch, s.defaultBranchOverride)) {
      cleanDefaultSnaps.push(s);
    } else {
      cleanNonDefaultSnaps.push(s);
    }
  }

  cleanDefaultSnaps.sort((a, b) => {
    const sp = getSyncPriority(a.syncStatus) - getSyncPriority(b.syncStatus);
    if (sp !== 0) return sp;
    const bp = getBranchPriority(a.branch) - getBranchPriority(b.branch);
    if (bp !== 0) return bp;
    return compareRepoPathsForDisplay(a, b);
  });
  cleanNonDefaultSnaps.sort(compareRepoPathsForDisplay);
  changeSnaps.sort(compareRepoPathsForDisplay);

  const cleanDefault = cleanDefaultSnaps.map(toVerboseRow);
  const cleanNonDefault = cleanNonDefaultSnaps.map(toVerboseRow);
  const changeRepos = changeSnaps.map(toVerboseRow);

  const all = [...cleanDefault, ...cleanNonDefault, ...changeRepos];
  let repoWidth = 20;
  let branchWidth = 25;
  for (const r of all) {
    repoWidth = Math.max(repoWidth, visibleWidth(r.repo));
    branchWidth = Math.max(branchWidth, visibleWidth(r.branch));
  }

  return { cleanDefault, cleanNonDefault, changeRepos, repoWidth, branchWidth };
}

export function buildSummaryState(snapshots: RepoSnapshot[]): SummaryState {
  const state: SummaryState = {
    changesUncommitted: new Set(),
    changesStaged: new Set(),
    changesBoth: new Set(),
    changesUntracked: new Set(),
    syncBehind: new Set(),
    syncAhead: new Set(),
    syncDiverged: new Set(),
    branchFeature: new Set(),
    branchBugfix: new Set(),
    branchChore: new Set(),
    branchRelease: new Set(),
    branchUnknown: new Set(),
    linkedWorktrees: new Set(),
  };

  for (const s of snapshots) {
    if (s.checkoutKind === 'linked') state.linkedWorktrees.add(s.repo);

    if (s.hasUnstaged && s.hasStaged) state.changesBoth.add(s.repo);
    else if (s.hasUnstaged) state.changesUncommitted.add(s.repo);
    else if (s.hasStaged) state.changesStaged.add(s.repo);
    if (s.hasUntracked) state.changesUntracked.add(s.repo);

    // Unborn / status-failed repos are listed under Attention, not Sync/Branches.
    if (isAttentionSyncNote(s.syncNote)) continue;

    if (s.syncStatus === 'behind') state.syncBehind.add(s.repo);
    else if (s.syncStatus === 'ahead') state.syncAhead.add(s.repo);
    else if (s.syncStatus === 'diverged') state.syncDiverged.add(s.repo);

    const kind = getBranchKind(s.branch, s.defaultBranchOverride);
    if (kind === 'feature') state.branchFeature.add(s.repo);
    else if (kind === 'bugfix') state.branchBugfix.add(s.repo);
    else if (kind === 'chore') state.branchChore.add(s.repo);
    else if (kind === 'release') state.branchRelease.add(s.repo);
    else if (kind === 'unknown') state.branchUnknown.add(s.repo);
  }

  return state;
}

/** All repos currently on a non-default branch (shown in Branches summary / eligible for -d). */
export function nonDefaultBranchRepos(summary: SummaryState): string[] {
  return sortedUnique([
    ...summary.branchFeature,
    ...summary.branchBugfix,
    ...summary.branchChore,
    ...summary.branchRelease,
    ...summary.branchUnknown,
  ]);
}
