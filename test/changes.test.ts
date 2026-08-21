import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  badgeForChange,
  fileChangesFromSnapshot,
  fileChangesToSnapshotFields,
} from '../src/changes.js';
import { statusLetterFromChange } from '../src/tui/icons.js';
import type { RepoSnapshot } from '../src/types.js';

function base(partial: Partial<RepoSnapshot>): RepoSnapshot {
  return {
    repo: 'demo',
    branch: 'main',
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

describe('fileChangesFromSnapshot', () => {
  it('merges staged+unstaged into one change and badges MS', () => {
    const changes = fileChangesFromSnapshot(
      base({
        hasStaged: true,
        hasUnstaged: true,
        stagedFiles: 'M\tsrc/a.ts',
        unstagedFiles: 'M\tsrc/a.ts',
      }),
    );
    assert.equal(changes.length, 1);
    assert.equal(badgeForChange(changes[0]!), '🟠MS');
  });

  it('round-trips fields for render compatibility', () => {
    const original = base({
      hasStaged: true,
      hasUntracked: true,
      stagedFiles: 'A\tnew.ts',
      untrackedFiles: 'scratch.md',
    });
    const fields = fileChangesToSnapshotFields(fileChangesFromSnapshot(original));
    assert.equal(fields.hasStaged, true);
    assert.equal(fields.hasUntracked, true);
    assert.match(fields.stagedFiles, /A\tnew\.ts/);
    assert.match(fields.untrackedFiles, /scratch\.md/);
  });

  it('badges unmerged U as conflict (not MS)', () => {
    const changes = fileChangesFromSnapshot(
      base({
        hasUnstaged: true,
        unstagedFiles: 'U\tconflict.txt',
      }),
    );
    assert.equal(changes.length, 1);
    assert.equal(badgeForChange(changes[0]!), '⚠️U');
    assert.equal(statusLetterFromChange(changes[0]!), 'U');
  });

  it('prefers U over MS when both staged and unstaged are set', () => {
    const change = {
      path: 'conflict.txt',
      stagedStatus: 'U',
      unstagedStatus: 'U',
    };
    assert.equal(badgeForChange(change), '⚠️U');
    assert.equal(statusLetterFromChange(change), 'U');
  });
});
