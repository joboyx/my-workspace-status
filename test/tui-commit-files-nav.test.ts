import assert from 'node:assert';
import { describe, it } from 'node:test';
import { commitFilesListKey } from '../src/tui/commitFiles/identity.js';
import { WORKTREE_COMMIT_ID } from '../src/tui/commitFiles/types.js';
import { commitFileSourceFromNav } from '../src/tui/commitFiles/resolveSource.js';

describe('commitFileSourceFromNav', () => {
  it('maps uncommitted graph row to worktree', () => {
    const src = commitFileSourceFromNav(
      { kind: 'repoGraph', repo: 'rs', commitId: null },
      { kind: 'uncommitted' },
    );
    assert.deepEqual(src, { kind: 'worktree' });
  });

  it('maps WORKTREE commitFiles view to worktree', () => {
    const src = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'rs', commitId: WORKTREE_COMMIT_ID, filePath: null },
      null,
    );
    assert.deepEqual(src, { kind: 'worktree' });
  });

  it('maps stash row', () => {
    const src = commitFileSourceFromNav(
      { kind: 'repoGraph', repo: 'rs', commitId: 'abc' },
      { kind: 'stash', stashRef: 'stash@{0}' },
    );
    assert.deepEqual(src, { kind: 'stash', stashRef: 'stash@{0}' });
  });

  it('maps commit id', () => {
    const src = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'rs', commitId: 'abc123', filePath: null },
      { kind: 'commit', commitId: 'abc123' },
    );
    assert.deepEqual(src, { kind: 'commit', commitId: 'abc123' });
  });

  it('source is stable across commitFiles views that differ only by filePath', () => {
    const graphRow = { kind: 'commit' as const, commitId: 'abc123' };
    const before = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'rs', commitId: 'abc123', filePath: null },
      graphRow,
    );
    const after = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'rs', commitId: 'abc123', filePath: 'src/a.ts' },
      graphRow,
    );
    assert.deepEqual(before, after);
    assert.deepEqual(before, { kind: 'commit', commitId: 'abc123' });
  });
});

describe('commitFilesListKey', () => {
  it('returns empty string when repo or source is missing', () => {
    assert.equal(commitFilesListKey(null, { kind: 'worktree' }), '');
    assert.equal(commitFilesListKey('rs', null), '');
    assert.equal(commitFilesListKey(null, null), '');
  });

  it('formats worktree / commit / stash keys', () => {
    assert.equal(commitFilesListKey('rs', { kind: 'worktree' }), 'rs|worktree');
    assert.equal(commitFilesListKey('rs', { kind: 'commit', commitId: 'abc' }), 'rs|commit:abc');
    assert.equal(
      commitFilesListKey('rs', { kind: 'stash', stashRef: 'stash@{0}' }),
      'rs|stash:stash@{0}',
    );
  });

  it('stays stable when only breadcrumb filePath would change (same repo+source)', () => {
    const graphRow = { kind: 'commit' as const, commitId: 'abc' };
    const srcBefore = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'rs', commitId: 'abc', filePath: null },
      graphRow,
    );
    const srcAfter = commitFileSourceFromNav(
      { kind: 'commitFiles', repo: 'rs', commitId: 'abc', filePath: 'lib/x.ts' },
      graphRow,
    );
    assert.equal(commitFilesListKey('rs', srcBefore), commitFilesListKey('rs', srcAfter));
    assert.equal(commitFilesListKey('rs', srcBefore), 'rs|commit:abc');
  });

  it('distinguishes worktree ≠ commit ≠ stash', () => {
    const worktree = commitFilesListKey('rs', { kind: 'worktree' });
    const commit = commitFilesListKey('rs', { kind: 'commit', commitId: 'abc' });
    const stash = commitFilesListKey('rs', { kind: 'stash', stashRef: 'stash@{0}' });
    assert.notEqual(worktree, commit);
    assert.notEqual(worktree, stash);
    assert.notEqual(commit, stash);
  });

  it('changes when commit id or stash ref changes', () => {
    assert.notEqual(
      commitFilesListKey('rs', { kind: 'commit', commitId: 'abc' }),
      commitFilesListKey('rs', { kind: 'commit', commitId: 'def' }),
    );
    assert.notEqual(
      commitFilesListKey('rs', { kind: 'stash', stashRef: 'stash@{0}' }),
      commitFilesListKey('rs', { kind: 'stash', stashRef: 'stash@{1}' }),
    );
    assert.notEqual(
      commitFilesListKey('rs', { kind: 'worktree' }),
      commitFilesListKey('other', { kind: 'worktree' }),
    );
  });
});
