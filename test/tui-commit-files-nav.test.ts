import assert from 'node:assert';
import { describe, it } from 'node:test';
import { commitFilesListKey } from '../src/tui/commitFiles/identity.js';
import { WORKTREE_COMMIT_ID } from '../src/tui/commitFiles/types.js';
import { commitFileSourceFromNav } from '../src/tui/commitFiles/resolveSource.js';

describe('commitFileSourceFromNav', () => {
  it('maps uncommitted graph row to worktree', () => {
    const src = commitFileSourceFromNav(
      { kind: 'repoGraph', repo: 'demo', commitId: null },
      { kind: 'uncommitted' },
    );
    assert.deepEqual(src, { kind: 'worktree' });
  });

  it('maps WORKTREE commitFiles view to worktree', () => {
    const src = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'demo', commitId: WORKTREE_COMMIT_ID, filePath: null },
      null,
    );
    assert.deepEqual(src, { kind: 'worktree' });
  });

  it('maps stash row', () => {
    const src = commitFileSourceFromNav(
      { kind: 'repoGraph', repo: 'demo', commitId: 'abc' },
      { kind: 'stash', stashRef: 'stash@{0}' },
    );
    assert.deepEqual(src, { kind: 'stash', stashRef: 'stash@{0}' });
  });

  it('maps commit id', () => {
    const src = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'demo', commitId: 'abc123', filePath: null },
      { kind: 'commit', commitId: 'abc123' },
    );
    assert.deepEqual(src, { kind: 'commit', commitId: 'abc123' });
  });

  it('source is stable across commitFiles views that differ only by filePath', () => {
    const graphRow = { kind: 'commit' as const, commitId: 'abc123' };
    const before = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'demo', commitId: 'abc123', filePath: null },
      graphRow,
    );
    const after = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'demo', commitId: 'abc123', filePath: 'src/a.ts' },
      graphRow,
    );
    assert.deepEqual(before, after);
    assert.deepEqual(before, { kind: 'commit', commitId: 'abc123' });
  });
});

describe('commitFilesListKey', () => {
  it('returns empty string when repo or source is missing', () => {
    assert.equal(commitFilesListKey(null, { kind: 'worktree' }), '');
    assert.equal(commitFilesListKey('demo', null), '');
    assert.equal(commitFilesListKey(null, null), '');
  });

  it('formats worktree / commit / stash keys', () => {
    assert.equal(commitFilesListKey('demo', { kind: 'worktree' }), 'demo|worktree');
    assert.equal(commitFilesListKey('demo', { kind: 'commit', commitId: 'abc' }), 'demo|commit:abc');
    assert.equal(
      commitFilesListKey('demo', { kind: 'stash', stashRef: 'stash@{0}' }),
      'demo|stash:stash@{0}',
    );
  });

  it('stays stable when only breadcrumb filePath would change (same repo+source)', () => {
    const graphRow = { kind: 'commit' as const, commitId: 'abc' };
    const srcBefore = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'demo', commitId: 'abc', filePath: null },
      graphRow,
    );
    const srcAfter = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'demo', commitId: 'abc', filePath: 'lib/x.ts' },
      graphRow,
    );
    assert.equal(commitFilesListKey('demo', srcBefore), commitFilesListKey('demo', srcAfter));
    assert.equal(commitFilesListKey('demo', srcBefore), 'demo|commit:abc');
  });

  it('distinguishes worktree ≠ commit ≠ stash', () => {
    const worktree = commitFilesListKey('demo', { kind: 'worktree' });
    const commit = commitFilesListKey('demo', { kind: 'commit', commitId: 'abc' });
    const stash = commitFilesListKey('demo', { kind: 'stash', stashRef: 'stash@{0}' });
    assert.notEqual(worktree, commit);
    assert.notEqual(worktree, stash);
    assert.notEqual(commit, stash);
  });

  it('changes when commit id or stash ref changes', () => {
    assert.notEqual(
      commitFilesListKey('demo', { kind: 'commit', commitId: 'abc' }),
      commitFilesListKey('demo', { kind: 'commit', commitId: 'def' }),
    );
    assert.notEqual(
      commitFilesListKey('demo', { kind: 'stash', stashRef: 'stash@{0}' }),
      commitFilesListKey('demo', { kind: 'stash', stashRef: 'stash@{1}' }),
    );
    assert.notEqual(
      commitFilesListKey('demo', { kind: 'worktree' }),
      commitFilesListKey('other', { kind: 'worktree' }),
    );
  });
});
