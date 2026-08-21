/**
 * Discover git repos and collect snapshots.
 */

import * as fs from 'fs';
import * as path from 'path';
import {
  workspaceStatusConfig,
  defaultBranchOverrideFor,
  loadWorkspaceStatusConfig,
} from './config.js';
import { mapWithConcurrency } from './concurrency.js';
import {
  execGitAsync,
  execGitOutputAsync,
  isAncestor,
  listWorktreesPorcelain,
  resolveDefaultBranchName,
  resolveDefaultBranchTipRef,
} from './git.js';
import { DETACHED_HEAD_BRANCH, isDefaultBranch } from './helpers.js';
import {
  classifyMergedIntoDefault,
  linkedWorktreesUnderCwd,
  parseWorktreeListPorcelain,
} from './worktrees.js';
import type { RepoSnapshot, SyncStatus, WorkspaceStatusConfig } from './types.js';

const STATUS_CONCURRENCY = 8;

/** Metadata for how a path was discovered (primary walk vs linked worktree). */
export type RepoCheckoutMeta = {
  checkoutKind: 'primary' | 'linked';
  primaryRepo?: string;
};

function pathDepth(repoPath: string): number {
  return repoPath.split(/[/\\]/).filter(Boolean).length;
}

/** True when repoPath is an exact onlyRepos match or an ancestor of one. */
function canReachOnlyRepo(repoPath: string, onlyRepos: Set<string>): boolean {
  for (const only of onlyRepos) {
    if (only === repoPath) return true;
    if (only.startsWith(`${repoPath}/`) || only.startsWith(`${repoPath}${path.sep}`)) {
      return true;
    }
  }
  return false;
}

/** True when dir has a `.git` directory or gitfile (linked worktrees / some submodules). */
function hasGitDir(dir: string): boolean {
  try {
    const st = fs.statSync(path.join(dir, '.git'));
    return st.isDirectory() || st.isFile();
  } catch {
    // Missing path or broken symlink.
    return false;
  }
}

/** True when `.git` is a directory (main checkout), not a gitfile (linked worktree). */
function isMainCheckout(repoDir: string): boolean {
  try {
    return fs.statSync(path.join(repoDir, '.git')).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Walk up from `relPath` to find a main checkout (`.git` directory) under `cwd`.
 * Returns workspace-relative path with `/` separators, or null.
 */
function findMainCheckoutRel(cwd: string, relPath: string): string | null {
  const cwdAbs = path.resolve(cwd);
  let abs = path.resolve(cwd, relPath);
  while (abs === cwdAbs || abs.startsWith(cwdAbs + path.sep)) {
    try {
      if (fs.statSync(path.join(abs, '.git')).isDirectory()) {
        const rel = path.relative(cwdAbs, abs).split(path.sep).join('/');
        return rel;
      }
    } catch {
      // continue upward
    }
    const parent = path.dirname(abs);
    if (parent === abs) break;
    abs = parent;
  }
  return null;
}

function isEffectiveDirectory(cwd: string, e: fs.Dirent): boolean {
  if (e.name.startsWith('.')) return false;
  if (e.isDirectory()) return true;
  if (e.isSymbolicLink()) {
    try {
      return fs.statSync(path.join(cwd, e.name)).isDirectory();
    } catch {
      return false;
    }
  }
  return false;
}

function isIgnoredRepo(repoPath: string, ignoredRepos: Set<string>): boolean {
  return ignoredRepos.has(repoPath);
}

function parseBranchLine(line: string): {
  branch: string;
  syncStatus: SyncStatus;
  syncNote: string;
} | null {
  if (!line.startsWith('## ')) return null;
  const value = line.slice(3);
  if (value.startsWith('No commits yet on ')) {
    const branch = value.slice('No commits yet on '.length).trim();
    if (!branch) return null;
    return { branch, syncStatus: 'no-upstream', syncNote: 'no commits yet' };
  }
  if (value === 'HEAD (no branch)') {
    return { branch: DETACHED_HEAD_BRANCH, syncStatus: 'no-upstream', syncNote: '' };
  }

  const match = value.match(/^(.+?)(?:\.\.\.(\S+)(?: \[(.+)\])?)?$/);
  if (!match) return null;

  const branch = match[1] ?? '';
  const upstream = match[2];
  const tracking = match[3] ?? '';
  if (!upstream) return { branch, syncStatus: 'no-upstream', syncNote: '' };

  const ahead = Number(tracking.match(/ahead (\d+)/)?.[1] ?? 0);
  const behind = Number(tracking.match(/behind (\d+)/)?.[1] ?? 0);
  if (ahead > 0 && behind > 0) {
    return {
      branch,
      syncStatus: 'diverged',
      syncNote: `diverged (ahead ${ahead}, behind ${behind})`,
    };
  }
  if (behind > 0) return { branch, syncStatus: 'behind', syncNote: `behind by ${behind} commits` };
  if (ahead > 0) return { branch, syncStatus: 'ahead', syncNote: `ahead by ${ahead} commits` };
  return { branch, syncStatus: 'up-to-date', syncNote: '' };
}

/** Snapshot used when `.git` exists but `git status` output is unusable. */
function failedRepoSnapshot(
  repoPath: string,
  defaultBranchOverride?: string,
  meta: RepoCheckoutMeta = { checkoutKind: 'primary' },
): RepoSnapshot {
  return {
    repo: repoPath,
    branch: '(unknown)',
    syncStatus: 'no-upstream',
    syncNote: 'status failed',
    hasUnstaged: false,
    hasStaged: false,
    hasUntracked: false,
    unstagedInfo: '',
    stagedFiles: '',
    unstagedFiles: '',
    untrackedFiles: '',
    checkoutKind: meta.checkoutKind,
    mergedIntoDefault: null,
    ...(meta.primaryRepo ? { primaryRepo: meta.primaryRepo } : {}),
    ...(defaultBranchOverride ? { defaultBranchOverride } : {}),
  };
}

function normalizePorcelainStatus(status: string): string {
  if (status === 'R' || status === 'C') return 'R';
  if (status === 'A' || status === 'M' || status === 'D' || status === 'U') return status;
  if (status === 'T') return 'M';
  return '';
}

/**
 * True when porcelain XY is an unmerged/conflict pair.
 * Covers UU/AA/DD/AU/UA/DU/UD and any XY that contains `U`.
 */
function isUnmergedXy(xy: string): boolean {
  if (xy.includes('U')) return true;
  return xy === 'AA' || xy === 'DD';
}

function formatTrackedEntry(status: string, filePart: string): string | null {
  const normalized = normalizePorcelainStatus(status);
  if (!normalized) return null;
  if (normalized === 'R') {
    const renameParts = filePart.split(' -> ');
    if (renameParts.length >= 2)
      return `R\t${renameParts[0]}\t${renameParts.slice(1).join(' -> ')}`;
  }
  return `${normalized}\t${filePart}`;
}

/**
 * Parse porcelain=v1 file lines (no `##` branch header) into staged/unstaged/untracked entries.
 * Unmerged XY pairs become a single unstaged `U` entry.
 */
export function parsePorcelainChangeLines(lines: string[]): {
  stagedEntries: string[];
  unstagedEntries: string[];
  untrackedEntries: string[];
} {
  const stagedEntries: string[] = [];
  const unstagedEntries: string[] = [];
  const untrackedEntries: string[] = [];

  for (const line of lines) {
    const xy = line.slice(0, 2);
    const filePart = line.slice(3);
    if (xy === '??') {
      untrackedEntries.push(filePart);
      continue;
    }

    // Emit a single unstaged U so badges stay conflicted (not MS).
    if (isUnmergedXy(xy)) {
      const conflictEntry = formatTrackedEntry('U', filePart);
      if (conflictEntry) unstagedEntries.push(conflictEntry);
      continue;
    }

    const stagedEntry = formatTrackedEntry(xy[0] ?? '', filePart);
    if (stagedEntry) stagedEntries.push(stagedEntry);

    const unstagedEntry = formatTrackedEntry(xy[1] ?? '', filePart);
    if (unstagedEntry) unstagedEntries.push(unstagedEntry);
  }

  return { stagedEntries, unstagedEntries, untrackedEntries };
}

/** Discover git repos below cwd without applying workspace config. */
export async function findRepos(cwd: string): Promise<string[]> {
  return findReposWithConfig(cwd, workspaceStatusConfig());
}

function shouldIncludeRepo(
  repoPath: string,
  ignoredRepos: Set<string>,
  onlyRepos?: Set<string>,
): boolean {
  if (onlyRepos) {
    return onlyRepos.has(repoPath);
  }
  return !isIgnoredRepo(repoPath, ignoredRepos);
}

/**
 * Discover git repos below cwd, excluding repos listed in workspace config.
 * Walks up to `config.maxDepth` path segments (default 3).
 */
export async function findReposWithConfig(
  cwd: string,
  config: WorkspaceStatusConfig,
  onlyRepos?: Set<string>,
): Promise<string[]> {
  const dirs: string[] = [];
  const ignoredRepos = new Set(config.ignoredRepos);
  const maxDepth = config.maxDepth;

  async function walk(relParent: string): Promise<void> {
    const parentDepth = relParent === '' ? 0 : pathDepth(relParent);
    if (parentDepth >= maxDepth) return;

    const absParent = relParent === '' ? cwd : path.join(cwd, relParent);
    const entries = await fs.promises.readdir(absParent, { withFileTypes: true });
    for (const e of entries) {
      if (!isEffectiveDirectory(absParent, e)) continue;

      const repoPath = relParent === '' ? e.name : path.join(relParent, e.name);
      if (onlyRepos) {
        if (!canReachOnlyRepo(repoPath, onlyRepos)) continue;
      } else if (isIgnoredRepo(repoPath, ignoredRepos)) {
        continue;
      }

      const full = path.join(absParent, e.name);
      if (hasGitDir(full) && shouldIncludeRepo(repoPath, ignoredRepos, onlyRepos)) {
        dirs.push(repoPath);
      }
      await walk(repoPath);
    }
  }

  await walk('');
  return dirs.sort();
}

/** Exit with an error when any filter repo is not discovered under cwd (incl. linked worktrees). */
export async function validateFilterRepos(cwd: string, filterRepos: string[]): Promise<void> {
  if (filterRepos.length === 0) return;
  // Empty ignore so named filters for ignored repos still validate (collect bypasses ignore).
  // Honor workspace maxDepth so deep named filters still resolve.
  const loaded = await loadWorkspaceStatusConfig(cwd);
  const config = workspaceStatusConfig({
    ignoredRepos: [],
    maxDepth: loaded.maxDepth,
    defaultBranches: loaded.defaultBranches,
  });
  const walkPrimaries = await findReposWithConfig(cwd, config);
  const entries = await expandReposWithLinkedWorktrees(cwd, walkPrimaries, config);
  const known = new Set(entries.map((e) => e.repoPath));
  for (const repo of filterRepos) {
    if (!known.has(repo)) {
      console.error(`Unknown repo: ${repo}`);
      process.exit(1);
    }
  }
}

/**
 * Whether a linked worktree path should be included given ignore / named-filter rules.
 */
export function shouldIncludeLinkedWorktree(
  linkedRelPath: string,
  primaryRelPath: string,
  ignoredRepos: Set<string>,
  onlyRepos?: Set<string>,
): boolean {
  if (onlyRepos) {
    return onlyRepos.has(linkedRelPath) || onlyRepos.has(primaryRelPath);
  }
  if (ignoredRepos.has(linkedRelPath)) return false;
  return true;
}

/**
 * Expand walk primaries with linked worktrees under cwd (ignore/filter applied).
 * Linked metadata wins when a path appears as both walk primary and linked.
 */
export async function expandReposWithLinkedWorktrees(
  cwd: string,
  walkPrimaries: string[],
  config: WorkspaceStatusConfig,
  onlyRepos?: Set<string>,
): Promise<Array<RepoCheckoutMeta & { repoPath: string }>> {
  const ignoredRepos = new Set(config.ignoredRepos);
  const cwdAbs = path.resolve(cwd);

  const listingRoots = new Set(walkPrimaries);
  if (onlyRepos) {
    for (const filter of onlyRepos) {
      if (walkPrimaries.includes(filter)) continue;
      const mainRel = findMainCheckoutRel(cwd, filter);
      if (mainRel) listingRoots.add(mainRel);
    }
  }

  const byPath = new Map<string, RepoCheckoutMeta & { repoPath: string }>();
  for (const primary of walkPrimaries) {
    byPath.set(primary, { repoPath: primary, checkoutKind: 'primary' });
  }

  for (const primary of listingRoots) {
    const primaryAbs = path.join(cwd, primary);
    if (!isMainCheckout(primaryAbs)) continue;

    const porcelain = await listWorktreesPorcelain(primaryAbs);
    if (!porcelain) continue;
    const linked = linkedWorktreesUnderCwd(
      parseWorktreeListPorcelain(porcelain),
      cwdAbs,
      primaryAbs,
    );
    for (const { relPath } of linked) {
      if (!shouldIncludeLinkedWorktree(relPath, primary, ignoredRepos, onlyRepos)) continue;
      byPath.set(relPath, {
        repoPath: relPath,
        checkoutKind: 'linked',
        primaryRepo: primary,
      });
    }
  }

  return [...byPath.values()].sort((a, b) => a.repoPath.localeCompare(b.repoPath));
}

function defaultBranchOverrideForPath(
  repoPath: string,
  primaryRepo: string | undefined,
  defaultBranches: Record<string, string>,
): string | undefined {
  return (
    defaultBranchOverrideFor(repoPath, defaultBranches) ??
    (primaryRepo ? defaultBranchOverrideFor(primaryRepo, defaultBranches) : undefined)
  );
}

async function computeMergedIntoDefault(
  repoDir: string,
  branch: string,
  defaultBranchOverride?: string,
): Promise<boolean | null> {
  // Legacy defaults (main/master/develop) must not get merge marks even when
  // origin/HEAD names a different branch (e.g. develop + origin/HEAD → main).
  if (isDefaultBranch(branch, defaultBranchOverride)) return null;
  const defaultBranch = await resolveDefaultBranchName(repoDir, defaultBranchOverride);
  if (branch === defaultBranch) return null;
  const tipRef = await resolveDefaultBranchTipRef(repoDir, defaultBranch);
  if (!tipRef) {
    return classifyMergedIntoDefault({
      branch,
      defaultBranch,
      isAncestorOfDefault: null,
    });
  }
  const ancestor = await isAncestor(repoDir, 'HEAD', tipRef);
  return classifyMergedIntoDefault({
    branch,
    defaultBranch,
    isAncestorOfDefault: ancestor,
  });
}

/** Collect a single-repo snapshot; optional fetch when doFetch is true. */
export async function processRepo(
  repoPath: string,
  cwd: string,
  doFetch: boolean,
  defaultBranchOverride?: string,
  meta: RepoCheckoutMeta = { checkoutKind: 'primary' },
): Promise<RepoSnapshot | null> {
  const repoDir = path.join(cwd, repoPath);
  if (!fs.existsSync(path.join(repoDir, '.git'))) return null;

  if (doFetch) {
    try {
      await execGitAsync(['fetch', '--quiet'], repoDir);
    } catch {
      // Fetch failed (network, auth, etc). Continue with existing refs.
    }
  }

  // Intentional: keep porcelain=v1 (XY + renames) — v2 only if e2e/specs later require it.
  const porcelain = await execGitOutputAsync(
    ['status', '--porcelain=v1', '--branch', '--ahead-behind', '--untracked-files=all'],
    repoDir,
  );
  const lines = porcelain.split('\n').filter(Boolean);
  const branchState = parseBranchLine(lines[0] ?? '');
  if (!branchState) return failedRepoSnapshot(repoPath, defaultBranchOverride, meta);

  const { stagedEntries, unstagedEntries, untrackedEntries } = parsePorcelainChangeLines(
    lines.slice(1),
  );

  const hasUnstaged = unstagedEntries.length > 0;
  const hasStaged = stagedEntries.length > 0;
  const hasUntracked = untrackedEntries.length > 0;
  const mergedIntoDefault = await computeMergedIntoDefault(
    repoDir,
    branchState.branch,
    defaultBranchOverride,
  );

  return {
    repo: repoPath,
    branch: branchState.branch,
    syncStatus: branchState.syncStatus,
    syncNote: branchState.syncNote,
    hasUnstaged,
    hasStaged,
    hasUntracked,
    unstagedInfo: '',
    stagedFiles: stagedEntries.join('|||'),
    unstagedFiles: unstagedEntries.join('|||'),
    untrackedFiles: untrackedEntries.join('|||'),
    checkoutKind: meta.checkoutKind,
    mergedIntoDefault,
    ...(meta.primaryRepo ? { primaryRepo: meta.primaryRepo } : {}),
    ...(defaultBranchOverride ? { defaultBranchOverride } : {}),
  };
}

/** Refresh one repo snapshot without fetch (TUI post-action refresh). */
export async function refreshRepoSnapshot(
  cwd: string,
  repoPath: string,
  defaultBranchOverride?: string,
  meta?: RepoCheckoutMeta,
): Promise<RepoSnapshot | null> {
  return processRepo(repoPath, cwd, false, defaultBranchOverride, meta);
}

export async function collectSnapshots(cwd: string, doFetch: boolean): Promise<RepoSnapshot[]> {
  return collectSnapshotsWithConfig(cwd, doFetch, workspaceStatusConfig());
}

/** Collect repo snapshots while excluding repos listed in workspace config. */
export async function collectSnapshotsWithConfig(
  cwd: string,
  doFetch: boolean,
  config: WorkspaceStatusConfig,
  onlyRepos?: Set<string>,
): Promise<RepoSnapshot[]> {
  const walkPrimaries = await findReposWithConfig(cwd, config, onlyRepos);
  const entries = await expandReposWithLinkedWorktrees(cwd, walkPrimaries, config, onlyRepos);
  const results = await mapWithConcurrency(entries, STATUS_CONCURRENCY, (entry) => {
    const override = defaultBranchOverrideForPath(
      entry.repoPath,
      entry.primaryRepo,
      config.defaultBranches,
    );
    return processRepo(entry.repoPath, cwd, doFetch, override, {
      checkoutKind: entry.checkoutKind,
      ...(entry.primaryRepo ? { primaryRepo: entry.primaryRepo } : {}),
    });
  });
  return results.filter((s): s is RepoSnapshot => s !== null);
}
