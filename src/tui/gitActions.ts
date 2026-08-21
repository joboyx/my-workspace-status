/**
 * Quiet git write helpers for the TUI.
 *
 * CLI counterparts in `actions.ts` log to stdout; Ink must not receive that
 * traffic, so these mirror the same skip / checkout / pull rules and return
 * short status fragments for the status bar instead.
 */

import * as path from 'node:path';
import { getDefaultBranch } from '../actions.js';
import { mapWithConcurrency } from '../concurrency.js';
import {
  checkoutBranch,
  execGit,
  pullQuiet,
  pullQuietDetailed,
  pushQuiet,
  repoHasLocalChanges,
} from '../git.js';

const DEFAULT_BRANCH_CONCURRENCY = 8;

/**
 * Outcome of a quiet pull batch.
 */
export type TuiPullResult = {
  ok: number;
  failed: number;
};

/**
 * Outcome of a quiet push batch.
 */
export type TuiPushResult = {
  ok: number;
  failed: number;
};

/**
 * Optional per-repo progress for a TUI batch (same contract as fetch).
 */
export type TuiBatchProgressOptions = {
  /** Called after each repo settles with settled count and batch size. */
  onProgress?: (done: number, total: number) => void;
};

/**
 * Pull each repo under `cwd` (auto-stash when dirty). Failures are counted, not thrown.
 */
export async function tuiPullRepos(
  cwd: string,
  repos: readonly string[],
  opts: TuiBatchProgressOptions = {},
): Promise<TuiPullResult> {
  if (repos.length === 0) return { ok: 0, failed: 0 };
  const total = repos.length;
  let ok = 0;
  let failed = 0;
  let done = 0;
  await Promise.all(
    repos.map(async (repo) => {
      try {
        const result = await pullQuietDetailed(path.join(cwd, repo));
        if (result.ok) ok += 1;
        else failed += 1;
      } catch {
        failed += 1;
      }
      done += 1;
      opts.onProgress?.(done, total);
    }),
  );
  return { ok, failed };
}

/**
 * Compact status-bar line after a pull batch.
 */
export function formatPullStatus(ok: number, failed: number, attempted: number): string {
  if (attempted === 0) return 'Nothing to pull';
  if (failed > 0 && ok === 0) return `pull: ${failed} failed`;
  if (failed > 0) return `Pulled ${ok} · ${failed} failed`;
  if (ok === 1) return 'Pulled';
  return `Pulled ${ok}`;
}

/**
 * Push each repo under `cwd` (`git push --quiet`). Failures are counted, not thrown.
 */
export async function tuiPushRepos(
  cwd: string,
  repos: readonly string[],
  opts: TuiBatchProgressOptions = {},
): Promise<TuiPushResult> {
  if (repos.length === 0) return { ok: 0, failed: 0 };
  const total = repos.length;
  let ok = 0;
  let failed = 0;
  let done = 0;
  await Promise.all(
    repos.map(async (repo) => {
      try {
        if (await pushQuiet(path.join(cwd, repo))) ok += 1;
        else failed += 1;
      } catch {
        failed += 1;
      }
      done += 1;
      opts.onProgress?.(done, total);
    }),
  );
  return { ok, failed };
}

/**
 * Compact status-bar line after a push batch.
 */
export function formatPushStatus(ok: number, failed: number, attempted: number): string {
  if (attempted === 0) return 'Nothing to push';
  if (failed > 0 && ok === 0) return `push: ${failed} failed`;
  if (failed > 0) return `Pushed ${ok} · ${failed} failed`;
  if (ok === 1) return 'Pushed';
  return `Pushed ${ok}`;
}

/**
 * Quiet default-branch switch outcome for one repo.
 */
export type TuiSwitchOutcome = 'switched' | 'already' | 'skipped-dirty' | 'no-default' | 'failed';

/**
 * Quiet default-branch switch for one repo (no stdout logging).
 *
 * Mirrors `switchRepoToDefaultBranch`: skip dirty worktrees when leaving the
 * current branch; when already on the default, pull (auto-stash if dirty).
 */
export async function tuiSwitchRepoToDefault(
  cwd: string,
  repoPath: string,
  currentBranch: string,
  defaultBranchOverride?: string,
): Promise<TuiSwitchOutcome> {
  const repoDir = path.join(cwd, repoPath);
  const defaultBranch = await getDefaultBranch(repoDir, defaultBranchOverride);
  if (!defaultBranch) return 'no-default';

  if (currentBranch === defaultBranch) {
    await pullQuiet(repoDir);
    return 'already';
  }

  if (await repoHasLocalChanges(repoDir)) {
    return 'skipped-dirty';
  }

  await execGit(['fetch', '--quiet', 'origin', defaultBranch], repoDir);
  if (!(await checkoutBranch(defaultBranch, repoDir))) {
    return 'failed';
  }

  const localCommit = await execGit(['rev-parse', 'HEAD'], repoDir);
  const remoteCommit = await execGit(['rev-parse', `origin/${defaultBranch}`], repoDir);
  if (localCommit !== remoteCommit) {
    await pullQuiet(repoDir);
  }
  return 'switched';
}

/**
 * One repo to switch, with the branch name from the current snapshot.
 */
export type TuiSwitchTask = {
  repoPath: string;
  currentBranch: string;
  defaultBranchOverride?: string;
};

/**
 * Quiet default-branch switch for many repos (concurrency 8).
 */
export async function tuiSwitchReposToDefault(
  cwd: string,
  tasks: readonly TuiSwitchTask[],
  opts: TuiBatchProgressOptions = {},
): Promise<TuiSwitchOutcome[]> {
  if (tasks.length === 0) return [];
  const total = tasks.length;
  let done = 0;
  return mapWithConcurrency(tasks, DEFAULT_BRANCH_CONCURRENCY, async (task) => {
    let outcome: TuiSwitchOutcome;
    try {
      outcome = await tuiSwitchRepoToDefault(
        cwd,
        task.repoPath,
        task.currentBranch,
        task.defaultBranchOverride,
      );
    } catch {
      outcome = 'failed';
    }
    done += 1;
    opts.onProgress?.(done, total);
    return outcome;
  });
}

/**
 * Compact status-bar line after default-branch switches.
 */
export function formatSwitchStatus(outcomes: readonly TuiSwitchOutcome[]): string {
  if (outcomes.length === 0) return 'Nothing to switch';
  if (outcomes.length === 1) {
    switch (outcomes[0]) {
      case 'switched':
        return 'Switched';
      case 'already':
        return 'Already on default';
      case 'skipped-dirty':
        return 'Skipped (dirty)';
      case 'no-default':
        return 'No default branch';
      case 'failed':
        return 'Switch failed';
    }
  }

  let switched = 0;
  let dirty = 0;
  let already = 0;
  let failed = 0;
  for (const o of outcomes) {
    if (o === 'switched') switched++;
    else if (o === 'skipped-dirty') dirty++;
    else if (o === 'already') already++;
    else failed++;
  }

  const parts: string[] = [];
  if (switched > 0) parts.push(`Switched ${switched}`);
  if (dirty > 0) parts.push(`skipped ${dirty} dirty`);
  if (already > 0 && switched === 0) parts.push(`${already} already`);
  if (failed > 0) parts.push(`${failed} failed`);
  return parts.join(' · ') || 'Nothing to switch';
}
