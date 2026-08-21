#!/usr/bin/env -S npx tsx
/**
 * Workspace git status aggregation.
 * Displays repo status across all git repositories in the current directory.
 */

import { loadWorkspaceStatusConfig } from './config.js';
import { collectSnapshotsWithConfig, validateFilterRepos } from './discovery.js';
import { buildSummaryState, buildVerboseRows, nonDefaultBranchRepos } from './snapshot.js';
import { renderWorkspaceStatus } from './render.js';
import { pullBehindRepos, switchReposToDefaultBranch } from './actions.js';
import { sortedUnique } from './helpers.js';
import { parseArgs } from './cli.js';
import type { RepoSnapshot } from './types.js';

// --- Main ---

async function main() {
  const flags = parseArgs(process.argv.slice(2));
  const cwd = process.cwd();
  const onlyRepos = flags.filterRepos.length > 0 ? new Set(flags.filterRepos) : undefined;

  if (onlyRepos) {
    await validateFilterRepos(cwd, flags.filterRepos);
  }

  const loaded = await loadWorkspaceStatusConfig(cwd);

  const wantTui =
    (flags.forceTui || (process.stdout.isTTY && !flags.forcePlain)) &&
    !flags.verbose &&
    !flags.doPull &&
    !flags.doDefaultBranch;

  // TUI always discovers ignored repos so `.` can show them without `-a`.
  const config = flags.includeAll || wantTui ? { ...loaded, ignoredRepos: [] } : loaded;

  if (flags.doFetch) {
    console.log('🔄 Fetching from remotes (this may take a moment)...');
    console.log('');
  }

  let snapshots: RepoSnapshot[] = await collectSnapshotsWithConfig(
    cwd,
    flags.doFetch,
    config,
    onlyRepos,
  );
  const snapshotMap = new Map(snapshots.map((s) => [s.repo, s]));

  let summary = buildSummaryState(snapshots);

  if (flags.doPull && summary.syncBehind.size > 0) {
    console.log('⬇️ Pulling repos that are behind...');
    console.log('');
    await pullBehindRepos(cwd, sortedUnique([...summary.syncBehind]));
    console.log('');
    console.log('🔄 Re-checking status after pull...');
    console.log('');
    snapshots = await collectSnapshotsWithConfig(cwd, false, config, onlyRepos);
    for (const s of snapshots) snapshotMap.set(s.repo, s);
    summary = buildSummaryState(snapshots);
  }

  if (flags.doDefaultBranch) {
    const toSwitch = nonDefaultBranchRepos(summary);
    if (toSwitch.length === 0) {
      console.log('  ℹ️ No non-default branches found to switch');
    } else {
      console.log('🔄 Switching to default branch and pulling...');
      console.log('');
      const switchTasks = toSwitch.flatMap((repo) => {
        const snapshot = snapshotMap.get(repo);
        return snapshot
          ? [
              {
                repoPath: repo,
                currentBranch: snapshot.branch,
                defaultBranchOverride: snapshot.defaultBranchOverride,
              },
            ]
          : [];
      });
      const switched = await switchReposToDefaultBranch(cwd, switchTasks);
      if (switched > 0) {
        console.log('');
        console.log('🔄 Re-checking status after switch...');
        console.log('');
        snapshots = await collectSnapshotsWithConfig(cwd, false, config, onlyRepos);
        for (const s of snapshots) snapshotMap.set(s.repo, s);
        summary = buildSummaryState(snapshots);
      }
    }
  }

  if (wantTui) {
    const { runTui } = await import('./tui/run.js');
    await runTui({
      cwd,
      snapshots,
      ignoredRepos: loaded.ignoredRepos,
      showIgnored: flags.includeAll,
      maxDepth: config.maxDepth,
      defaultBranches: config.defaultBranches,
      filterRepos: flags.filterRepos,
      editor: config.editor,
    });
    return;
  }

  const verbose = buildVerboseRows(snapshots);
  for (const line of renderWorkspaceStatus({
    snapshots,
    summary,
    verbose,
    showVerbose: flags.verbose,
  })) {
    console.log(line);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
