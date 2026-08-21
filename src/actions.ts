/**
 * Git actions: pull, switch to default branch.
 */

import * as path from 'path';
import { mapWithConcurrency } from './concurrency.js';
import {
  checkoutBranch,
  execGit,
  execGitStatus,
  pullQuiet,
  pullQuietDetailed,
  repoHasLocalChanges,
} from './git.js';

const DEFAULT_BRANCH_CONCURRENCY = 8;

/**
 * Resolve the default branch for a repo.
 *
 * When `defaultBranchOverride` is set (workspace config), that name wins.
 * Otherwise: `origin/HEAD`, then probe `develop` / `main` / `master` on remote,
 * then the same names locally.
 */
export async function getDefaultBranch(
  repoDir: string,
  defaultBranchOverride?: string,
): Promise<string | null> {
  if (defaultBranchOverride) return defaultBranchOverride;

  const remoteHead = await execGit(
    ['symbolic-ref', '--quiet', '--short', 'refs/remotes/origin/HEAD'],
    repoDir,
  );
  if (remoteHead) {
    const m = remoteHead.match(/^origin\/(.+)$/);
    return m ? m[1] : remoteHead;
  }
  for (const b of ['develop', 'main', 'master']) {
    if ((await execGitStatus(['show-ref', '--verify', 'refs/remotes/origin/' + b], repoDir)) === 0)
      return b;
  }
  for (const b of ['develop', 'main', 'master']) {
    if ((await execGitStatus(['show-ref', '--verify', 'refs/heads/' + b], repoDir)) === 0) return b;
  }
  return null;
}

export async function switchRepoToDefaultBranch(
  repoPath: string,
  currentBranch: string,
  cwd: string,
  defaultBranchOverride?: string,
): Promise<boolean> {
  const repoDir = path.join(cwd, repoPath);
  const defaultBranch = await getDefaultBranch(repoDir, defaultBranchOverride);
  if (!defaultBranch) {
    console.error(`  ⚠️ ${repoPath}: No default branch found (develop/main/master)`);
    return false;
  }

  if (currentBranch === defaultBranch) {
    console.log(`  ✅ ${repoPath}: Already on ${defaultBranch}`);
    console.log('    Pulling latest...');
    const result = await pullQuietDetailed(repoDir);
    if (result.ok) {
      console.log(
        result.stashed
          ? '    ✅ Pulled successfully (stashed local changes, reapplied)'
          : '    ✅ Pulled successfully',
      );
    } else if (result.stashPopFailed) {
      console.log('    ⚠️ Pulled but stash pop conflicted — resolve conflicts / check stash list');
    } else {
      console.log('    ⚠️ Pull failed or no updates');
    }
    return false;
  }

  if (await repoHasLocalChanges(repoDir)) {
    console.log(`  ⚠️ ${repoPath} (${currentBranch}): Has uncommitted changes, skipping`);
    return false;
  }

  console.log(`  🔄 ${repoPath}: Switching from ${currentBranch} to ${defaultBranch}`);
  await execGit(['fetch', '--quiet', 'origin', defaultBranch], repoDir);

  if (!(await checkoutBranch(defaultBranch, repoDir))) {
    console.log('    ⚠️ Failed to switch (branch may not exist)');
    return false;
  }

  console.log('    ✅ Switched successfully');
  console.log('    Pulling latest...');
  const localCommit = await execGit(['rev-parse', 'HEAD'], repoDir);
  const remoteCommit = await execGit(['rev-parse', `origin/${defaultBranch}`], repoDir);
  if (localCommit !== remoteCommit) {
    if (await pullQuiet(repoDir)) console.log('    ✅ Pulled successfully');
    else console.log('    ⚠️ Pull failed');
  } else {
    console.log('    ✅ Already up to date');
  }
  return true;
}

export type DefaultBranchSwitchTask = {
  repoPath: string;
  currentBranch: string;
  defaultBranchOverride?: string;
};

/** Switch multiple repos to their default branch concurrently (per-repo steps stay sequential). */
export async function switchReposToDefaultBranch(
  cwd: string,
  tasks: DefaultBranchSwitchTask[],
): Promise<number> {
  const results = await mapWithConcurrency(tasks, DEFAULT_BRANCH_CONCURRENCY, (task) =>
    switchRepoToDefaultBranch(
      task.repoPath,
      task.currentBranch,
      cwd,
      task.defaultBranchOverride,
    ),
  );
  return results.filter(Boolean).length;
}

/** Pull multiple repos in parallel (auto-stash dirty worktrees around each pull). */
export async function pullBehindRepos(cwd: string, repos: string[]): Promise<void> {
  const results = await Promise.all(
    repos.map(async (repo) => ({
      repo,
      result: await pullQuietDetailed(path.join(cwd, repo)),
    })),
  );
  for (const { repo, result } of results) {
    console.log(`  Pulling ${repo}...`);
    if (result.ok) {
      console.log(
        result.stashed ? '    ✅ Success (stashed local changes, reapplied)' : '    ✅ Success',
      );
    } else if (result.stashPopFailed) {
      console.log('    ⚠️ Pulled but stash pop conflicted — resolve conflicts / check stash list');
    } else {
      console.log('    ⚠️ Failed (may have conflicts)');
    }
  }
}
