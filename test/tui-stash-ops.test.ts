import assert from 'node:assert';
import { describe, it } from 'node:test';

import { stashOpsForContext } from '../src/tui/stashOps.js';

describe('stashOpsForContext', () => {
  it('dirty file yields only push with paths', () => {
    assert.deepEqual(
      stashOpsForContext({
        kind: 'file',
        dirty: true,
        dirtyPaths: ['src/a.ts'],
      }),
      [{ id: 'push', key: 's', label: 'stash', paths: ['src/a.ts'] }],
    );
  });

  it('stash row yields apply, pop, and drop', () => {
    assert.deepEqual(
      stashOpsForContext({
        kind: 'graphStash',
        dirty: false,
        focusedStashRef: 'stash@{0}',
      }),
      [
        { id: 'apply', key: 'a', label: 'apply stash', stashRef: 'stash@{0}' },
        { id: 'pop', key: 'p', label: 'pop stash', stashRef: 'stash@{0}' },
        { id: 'drop', key: 'd', label: 'drop stash', stashRef: 'stash@{0}' },
      ],
    );
  });

  it('commit + latest stash + dirty yields push, apply, pop (no drop)', () => {
    assert.deepEqual(
      stashOpsForContext({
        kind: 'graphCommit',
        dirty: true,
        latestStashRef: 'stash@{0}',
      }),
      [
        { id: 'push', key: 's', label: 'stash' },
        { id: 'apply', key: 'a', label: 'apply stash', stashRef: 'stash@{0}' },
        { id: 'pop', key: 'p', label: 'pop stash', stashRef: 'stash@{0}' },
      ],
    );
  });

  it('clean commit with no stashes yields an empty list', () => {
    assert.deepEqual(
      stashOpsForContext({
        kind: 'graphCommit',
        dirty: false,
      }),
      [],
    );
  });

  it('omits paths when dirtyPaths is empty and drop when only latestStashRef is set', () => {
    const pushOnly = stashOpsForContext({
      kind: 'dir',
      dirty: true,
      dirtyPaths: [],
    });
    assert.deepEqual(pushOnly, [{ id: 'push', key: 's', label: 'stash' }]);
    assert.equal('paths' in pushOnly[0]!, false);

    const fromLatest = stashOpsForContext({
      kind: 'graphCommit',
      dirty: false,
      latestStashRef: 'stash@{2}',
    });
    assert.deepEqual(
      fromLatest.map((op) => op.id),
      ['apply', 'pop'],
    );
    assert.equal(
      fromLatest.every((op) => op.stashRef === 'stash@{2}'),
      true,
    );
  });
});
