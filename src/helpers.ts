/**
 * Pure helpers for branch/sync classification and string handling.
 */

import type { RepoSnapshot, SyncStatus } from './types.js';

export const SEP = '|||';

export function splitEntries(str: string): string[] {
  if (!str) return [];
  return str
    .split(SEP)
    .map((s) => s.trim())
    .filter(Boolean);
}

export function trimVal(s: string): string {
  return s.replace(/^\s+|\s+$/g, '');
}

export function sanitizePath(p: string): string {
  return p.replace(/[\r\n\x1e\x1f]/g, '').replace(/[\x00-\x1f\x7f]/g, '');
}

export type BranchKind = 'default' | 'feature' | 'bugfix' | 'chore' | 'release' | 'unknown';

/**
 * Whether `branch` counts as the repo default.
 *
 * When `defaultBranchOverride` is set (from workspace config), only that branch
 * is default. Otherwise the legacy set `main` / `master` / `develop` is used.
 */
export function isDefaultBranch(branch: string, defaultBranchOverride?: string): boolean {
  if (defaultBranchOverride !== undefined) return branch === defaultBranchOverride;
  return branch === 'main' || branch === 'master' || branch === 'develop';
}

/** Classify a branch for summary grouping. Default branches are excluded from the Branches section. */
export function getBranchKind(branch: string, defaultBranchOverride?: string): BranchKind {
  if (isDefaultBranch(branch, defaultBranchOverride)) return 'default';
  if (branch.startsWith('feature/')) return 'feature';
  if (branch.startsWith('bugfix/')) return 'bugfix';
  if (branch.startsWith('chore/')) return 'chore';
  if (branch.startsWith('release/')) return 'release';
  return 'unknown';
}

export function getBranchPriority(branch: string): number {
  if (branch === 'main') return 1;
  if (branch === 'master') return 2;
  if (branch === 'develop') return 3;
  return 4;
}

export function getSyncPriority(status: SyncStatus): number {
  const m: Record<SyncStatus, number> = {
    'up-to-date': 0,
    'no-upstream': 1,
    behind: 2,
    ahead: 3,
    diverged: 4,
  };
  return m[status] ?? 5;
}

export function getBranchEmoji(branch: string): string {
  if (branch === 'main' || branch === 'master') return '🔥';
  const kind = getBranchKind(branch);
  if (kind === 'feature') return '🚧';
  if (kind === 'bugfix') return '🐛';
  if (kind === 'chore') return '🔧';
  if (kind === 'release') return '🚀';
  return '🌿';
}

export function getSyncEmoji(status: SyncStatus): string {
  const m: Record<SyncStatus, string> = {
    'up-to-date': '✅',
    'no-upstream': '❓',
    behind: '⬇️',
    ahead: '⬆️',
    diverged: '🔀',
  };
  return m[status] ?? '✅';
}

/**
 * True when syncNote marks a repo that must appear under Attention
 * (unborn / status failure) instead of being treated as quietly clean.
 */
export function isAttentionSyncNote(note: string): boolean {
  return note === 'no commits yet' || note === 'status failed';
}

/**
 * Canonical branch label discovery uses for detached HEAD
 * (`## HEAD (no branch)` → this string).
 */
export const DETACHED_HEAD_BRANCH = 'HEAD (detached)';

/**
 * True when `branch` is detached HEAD (including legacy short forms).
 */
export function isDetachedHeadBranch(branch: string): boolean {
  return (
    !branch ||
    branch === DETACHED_HEAD_BRANCH ||
    branch === 'HEAD' ||
    branch === '(detached)'
  );
}

export function extractTicketId(branch: string): string | null {
  const m = branch.match(/([A-Z]+-\d+)/);
  return m ? m[1] : null;
}

/**
 * Merge-into-default marker for plain report labels.
 * Returns a leading space plus emoji, or empty when unknown / N/A.
 */
export function formatMergeMark(merged: boolean | null): string {
  if (merged === true) return ' ✅';
  if (merged === false) return ' 🌱';
  return '';
}

/**
 * Repo path for plain report: linked checkouts get a `🔗 ` prefix.
 */
export function formatCheckoutRepoLabel(snapshot: RepoSnapshot): string {
  return snapshot.checkoutKind === 'linked' ? `🔗 ${snapshot.repo}` : snapshot.repo;
}

/**
 * Append merge mark to a branch (or summary) label when classification is known.
 */
export function formatBranchWithMerge(
  branchEmojiAndName: string,
  merged: boolean | null,
): string {
  return `${branchEmojiAndName}${formatMergeMark(merged)}`;
}

export function sortedUnique(repos: string[]): string[] {
  return [...new Set(repos)].filter(Boolean).sort();
}

/**
 * Display sort for repo snapshots: primary, then its linked children, by path.
 * Key: `(primaryRepo ?? repo, checkoutKind === 'linked' ? 1 : 0, repo)`.
 */
export function compareRepoPathsForDisplay(a: RepoSnapshot, b: RepoSnapshot): number {
  const aPrimary = a.primaryRepo ?? a.repo;
  const bPrimary = b.primaryRepo ?? b.repo;
  const byPrimary = aPrimary.localeCompare(bPrimary);
  if (byPrimary !== 0) return byPrimary;
  const aLinked = a.checkoutKind === 'linked' ? 1 : 0;
  const bLinked = b.checkoutKind === 'linked' ? 1 : 0;
  if (aLinked !== bLinked) return aLinked - bLinked;
  return a.repo.localeCompare(b.repo);
}

/**
 * Terminal display width for padding tables.
 * Counts emoji / CJK / common symbol presentation as double-width;
 * skips variation selectors and ZWJ.
 */
export function visibleWidth(value: string): number {
  let width = 0;
  for (const char of [...value]) {
    const code = char.codePointAt(0) ?? 0;
    if (code === 0xfe0f || code === 0x200d) continue;
    const wide =
      (code >= 0x1100 && code <= 0x115f) ||
      code === 0x2329 ||
      code === 0x232a ||
      (code >= 0x2190 && code <= 0x21ff) ||
      (code >= 0x2300 && code <= 0x23ff) ||
      (code >= 0x2600 && code <= 0x27bf) ||
      (code >= 0x2b00 && code <= 0x2bff) ||
      (code >= 0x2e80 && code <= 0xa4cf) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0xf900 && code <= 0xfaff) ||
      (code >= 0xfe10 && code <= 0xfe19) ||
      (code >= 0xfe30 && code <= 0xfe6f) ||
      (code >= 0xff00 && code <= 0xff60) ||
      (code >= 0xffe0 && code <= 0xffe6) ||
      (code >= 0x1f000 && code <= 0x1faff);
    width += wide ? 2 : 1;
  }
  return width;
}

/** Normalize a user-supplied repo filter to a workspace-relative path. */
export function normalizeFilterRepo(arg: string): string {
  return arg
    .replace(/^\.\/+/, '')
    .replace(/\\/g, '/')
    .replace(/\/+$/, '');
}
