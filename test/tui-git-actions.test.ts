import assert from 'node:assert';
import { describe, it } from 'node:test';
import { formatPullStatus, formatPushStatus, formatSwitchStatus } from '../src/tui/gitActions.js';

describe('formatPullStatus', () => {
  it('reports nothing when no repos were attempted', () => {
    assert.equal(formatPullStatus(0, 0, 0), 'Nothing to pull');
  });

  it('uses a short singular success line', () => {
    assert.equal(formatPullStatus(1, 0, 1), 'Pulled');
  });

  it('counts multi-repo success and mixed failures', () => {
    assert.equal(formatPullStatus(3, 0, 3), 'Pulled 3');
    assert.equal(formatPullStatus(2, 1, 3), 'Pulled 2 · 1 failed');
    assert.equal(formatPullStatus(0, 2, 2), 'pull: 2 failed');
  });
});

describe('formatPushStatus', () => {
  it('reports nothing when no repos were attempted', () => {
    assert.equal(formatPushStatus(0, 0, 0), 'Nothing to push');
  });

  it('uses a short singular success line', () => {
    assert.equal(formatPushStatus(1, 0, 1), 'Pushed');
  });

  it('counts multi-repo success and mixed failures', () => {
    assert.equal(formatPushStatus(3, 0, 3), 'Pushed 3');
    assert.equal(formatPushStatus(2, 1, 3), 'Pushed 2 · 1 failed');
    assert.equal(formatPushStatus(0, 2, 2), 'push: 2 failed');
  });
});

describe('formatSwitchStatus', () => {
  it('reports nothing for an empty batch', () => {
    assert.equal(formatSwitchStatus([]), 'Nothing to switch');
  });

  it('uses short single-repo phrases', () => {
    assert.equal(formatSwitchStatus(['switched']), 'Switched');
    assert.equal(formatSwitchStatus(['already']), 'Already on default');
    assert.equal(formatSwitchStatus(['skipped-dirty']), 'Skipped (dirty)');
    assert.equal(formatSwitchStatus(['no-default']), 'No default branch');
    assert.equal(formatSwitchStatus(['failed']), 'Switch failed');
  });

  it('summarises mixed multi-repo outcomes', () => {
    assert.equal(
      formatSwitchStatus(['switched', 'switched', 'skipped-dirty', 'already']),
      'Switched 2 · skipped 1 dirty',
    );
    assert.equal(
      formatSwitchStatus(['already', 'already', 'skipped-dirty']),
      'skipped 1 dirty · 2 already',
    );
  });
});

describe('tui batch onProgress', () => {
  it('reports settled counts after each pull (nonexistent repos)', async () => {
    const { tuiPullRepos } = await import('../src/tui/gitActions.js');
    const progress: Array<[number, number]> = [];
    const result = await tuiPullRepos('/tmp', ['a', 'b', 'c'], {
      onProgress: (done, total) => progress.push([done, total]),
    });
    assert.equal(result.ok, 0);
    assert.equal(result.failed, 3);
    assert.equal(progress.length, 3);
    assert.deepEqual(
      progress.map(([, total]) => total),
      [3, 3, 3],
    );
    assert.deepEqual(
      progress.map(([done]) => done).sort((a, b) => a - b),
      [1, 2, 3],
    );
  });

  it('reports settled counts after each push (nonexistent repos)', async () => {
    const { tuiPushRepos } = await import('../src/tui/gitActions.js');
    const progress: Array<[number, number]> = [];
    const result = await tuiPushRepos('/tmp', ['a', 'b'], {
      onProgress: (done, total) => progress.push([done, total]),
    });
    assert.equal(result.ok, 0);
    assert.equal(result.failed, 2);
    assert.deepEqual(
      progress.map(([done]) => done).sort((a, b) => a - b),
      [1, 2],
    );
  });

  it('reports settled counts after each default-branch switch', async () => {
    const { tuiSwitchReposToDefault } = await import('../src/tui/gitActions.js');
    const progress: Array<[number, number]> = [];
    const outcomes = await tuiSwitchReposToDefault(
      '/tmp',
      [
        { repoPath: 'a', currentBranch: 'feat' },
        { repoPath: 'b', currentBranch: 'feat' },
      ],
      { onProgress: (done, total) => progress.push([done, total]) },
    );
    assert.equal(outcomes.length, 2);
    assert.deepEqual(
      progress.map(([done]) => done).sort((a, b) => a - b),
      [1, 2],
    );
  });
});
