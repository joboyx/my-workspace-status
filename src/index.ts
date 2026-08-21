#!/usr/bin/env -S npx tsx
/**
 * Workspace git status aggregation.
 * Displays repo status across all git repositories in the current directory.
 */

import { loadWorkspaceStatusConfig } from './config.js';
import { collectSnapshotsWithConfig, validateFilterRepos } from './discovery.js';
import {
  buildSummaryState,
  buildVerboseRows,
  buildWorkspaceSnapshot,
  nonDefaultBranchRepos,
  repoSnapshotsFromWorkspace,
  serializeWorkspaceSnapshot,
  visibleWorkspaceSnapshot,
} from './snapshot.js';
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
    (flags.forceTui || (process.stdout.isTTY && !flags.forcePlain && !flags.forceJson)) &&
    !flags.verbose &&
    !flags.doPull &&
    !flags.doDefaultBranch;

  // TUI always discovers ignored repos so `.` can show them without `-a`.
  const config = flags.includeAll || wantTui ? { ...loaded, ignoredRepos: [] } : loaded;

  const say = (line: string): void => {
    if (flags.forceJson) console.error(line);
    else console.log(line);
  };

  if (flags.doFetch) {
    say('🔄 Fetching from remotes (this may take a moment)...');
    say('');
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
    say('⬇️ Pulling repos that are behind...');
    say('');
    await pullBehindRepos(cwd, sortedUnique([...summary.syncBehind]));
    say('');
    say('🔄 Re-checking status after pull...');
    say('');
    snapshots = await collectSnapshotsWithConfig(cwd, false, config, onlyRepos);
    for (const s of snapshots) snapshotMap.set(s.repo, s);
    summary = buildSummaryState(snapshots);
  }

  if (flags.doDefaultBranch) {
    const toSwitch = nonDefaultBranchRepos(summary);
    if (toSwitch.length === 0) {
      say('  ℹ️ No non-default branches found to switch');
    } else {
      say('🔄 Switching to default branch and pulling...');
      say('');
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
        say('');
        say('🔄 Re-checking status after switch...');
        say('');
        snapshots = await collectSnapshotsWithConfig(cwd, false, config, onlyRepos);
        for (const s of snapshots) snapshotMap.set(s.repo, s);
        summary = buildSummaryState(snapshots);
      }
    }
  }

  const workspace = buildWorkspaceSnapshot({
    snapshots,
    ignoredRepos: loaded.ignoredRepos,
    showIgnored: flags.includeAll,
    filterRepos: flags.filterRepos,
  });

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

  const published = visibleWorkspaceSnapshot(workspace);

  if (flags.forceJson) {
    process.stdout.write(serializeWorkspaceSnapshot(published));
    return;
  }

  const visibleSnapshots = repoSnapshotsFromWorkspace(published);
  const visibleSummary = buildSummaryState(visibleSnapshots);
  const verbose = buildVerboseRows(visibleSnapshots);
  for (const line of renderWorkspaceStatus({
    snapshots: visibleSnapshots,
    summary: visibleSummary,
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
