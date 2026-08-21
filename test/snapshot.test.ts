import assert from 'node:assert';
import { describe, it } from 'node:test';

import type { RepoSnapshot } from '../src/types.js';
import {
  buildSummaryState,
  buildVerboseRows,
  buildWorkspaceSnapshot,
  repoSnapshotsFromWorkspace,
  serializeWorkspaceSnapshot,
  visibleWorkspaceSnapshot,
} from '../src/snapshot.js';
import { renderWorkspaceStatus } from '../src/render.js';

function snap(partial: Partial<RepoSnapshot> & Pick<RepoSnapshot, 'repo' | 'branch'>): RepoSnapshot {
  return {
    syncStatus: 'up-to-date',
    syncNote: '',
    hasUnstaged: false,
    hasStaged: false,
    hasUntracked: false,
    unstagedInfo: '',
    stagedFiles: '',
    unstagedFiles: '',
    untrackedFiles: '',
    checkoutKind: 'primary',
    mergedIntoDefault: null,
    ...partial,
  };
}

describe('buildWorkspaceSnapshot', () => {
  it('marks ignored repos and sorts contract fields', () => {
    const workspace = buildWorkspaceSnapshot({
      snapshots: [
        snap({
          repo: 'notes',
          branch: 'main',
          hasUnstaged: true,
          unstagedFiles: 'M\tREADME.md',
        }),
        snap({ repo: 'app', branch: 'main' }),
      ],
      ignoredRepos: ['notes'],
      showIgnored: true,
      filterRepos: [],
    });

    assert.equal(workspace.version, 1);
    assert.equal(workspace.showIgnored, true);
    assert.deepEqual(workspace.ignoredRepos, ['notes']);
    assert.deepEqual(
      workspace.repos.map((r) => r.repo),
      ['app', 'notes'],
    );
    assert.equal(workspace.repos[0]?.ignored, false);
    assert.equal(workspace.repos[1]?.ignored, true);
    assert.deepEqual(workspace.repos[1]?.changes, [{ path: 'README.md', unstagedStatus: 'M' }]);
  });

  it('hides ignored repos from the published snapshot unless shown or named', () => {
    const workspace = buildWorkspaceSnapshot({
      snapshots: [
        snap({ repo: 'app', branch: 'main' }),
        snap({ repo: 'notes', branch: 'main' }),
      ],
      ignoredRepos: ['notes'],
      showIgnored: false,
      filterRepos: [],
    });

    const published = visibleWorkspaceSnapshot(workspace);
    assert.deepEqual(
      published.repos.map((r) => r.repo),
      ['app'],
    );

    const named = visibleWorkspaceSnapshot(
      buildWorkspaceSnapshot({
        snapshots: [snap({ repo: 'notes', branch: 'main' })],
        ignoredRepos: ['notes'],
        showIgnored: false,
        filterRepos: ['notes'],
      }),
    );
    assert.deepEqual(
      named.repos.map((r) => r.repo),
      ['notes'],
    );
    assert.equal(named.repos[0]?.ignored, true);
  });

  it('round-trips to the same plain render as the source snapshots', () => {
    const snapshots = [
      snap({
        repo: 'app',
        branch: 'main',
        hasUnstaged: true,
        unstagedFiles: 'M\tREADME.md',
      }),
    ];
    const workspace = buildWorkspaceSnapshot({
      snapshots,
      ignoredRepos: [],
      showIgnored: false,
      filterRepos: [],
    });
    const restored = repoSnapshotsFromWorkspace(workspace);
    const fromSource = renderWorkspaceStatus({
      snapshots,
      summary: buildSummaryState(snapshots),
      verbose: buildVerboseRows(snapshots),
      showVerbose: false,
    });
    const fromContract = renderWorkspaceStatus({
      snapshots: restored,
      summary: buildSummaryState(restored),
      verbose: buildVerboseRows(restored),
      showVerbose: false,
    });
    assert.deepEqual(fromContract, fromSource);
  });

  it('serializeWorkspaceSnapshot drops hidden ignored repos and keeps stable keys', () => {
    const workspace = buildWorkspaceSnapshot({
      snapshots: [
        snap({
          repo: 'app',
          branch: 'main',
          hasUnstaged: true,
          unstagedFiles: 'M\tREADME.md',
        }),
        snap({ repo: 'notes', branch: 'main' }),
      ],
      ignoredRepos: ['notes'],
      showIgnored: false,
      filterRepos: [],
    });
    const parsed = JSON.parse(serializeWorkspaceSnapshot(workspace));
    assert.equal(parsed.version, 1);
    assert.equal(parsed.showIgnored, false);
    assert.deepEqual(parsed.ignoredRepos, ['notes']);
    assert.deepEqual(
      parsed.repos.map((r: { repo: string }) => r.repo),
      ['app'],
    );
    assert.deepEqual(Object.keys(parsed), [
      'version',
      'showIgnored',
      'filterRepos',
      'ignoredRepos',
      'repos',
    ]);
  });
});
