import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  canCreateBranch,
  canGraphCheckout,
  canStashApply,
  canStashDrop,
  canStashMenu,
  canStashPop,
  checkoutableBranchNames,
  localBranchNames,
  planGraphCheckout,
  resolveCheckoutTarget,
  runBusyThenRefresh,
  type GraphActionRow,
} from '../src/tui/graph/actions.js';
import type { GraphCommit } from '../src/tui/graph/types.js';

const commit = (refs: GraphCommit['refs']): GraphActionRow => ({
  kind: 'commit',
  commit: {
    id: 'aaa',
    parents: [],
    subject: 's',
    authorName: 'A',
    authorDateUnix: 1,
    refs,
  },
});

describe('localBranchNames / canGraphCheckout', () => {
  it('true when origin remote plus tag (no local)', () => {
    const row = commit([
      { kind: 'remote', name: 'origin/main', commitId: 'aaa' },
      { kind: 'tag', name: 'v1', commitId: 'aaa' },
    ]);
    assert.deepEqual(localBranchNames(row), []);
    assert.deepEqual(checkoutableBranchNames(row), ['origin/main']);
    assert.equal(canGraphCheckout(row), true);
  });

  it('true when one local branch', () => {
    const row = commit([{ kind: 'local', name: 'main', commitId: 'aaa' }]);
    assert.deepEqual(localBranchNames(row), ['main']);
    assert.equal(canGraphCheckout(row), true);
  });

  it('lists multiple locals without guessing order beyond stable sort', () => {
    const row = commit([
      { kind: 'local', name: 'feature', commitId: 'aaa' },
      { kind: 'local', name: 'main', commitId: 'aaa' },
    ]);
    assert.deepEqual(localBranchNames(row).sort(), ['feature', 'main']);
  });

  it('stash / uncommitted cannot checkout via b', () => {
    assert.equal(canGraphCheckout({ kind: 'uncommitted' }), false);
    assert.equal(
      canGraphCheckout({
        kind: 'stash',
        stash: {
          id: 's',
          stashRef: 'stash@{0}',
          index: 0,
          subject: 'w',
          authorDateUnix: 1,
          parentId: '',
        },
      }),
      false,
    );
  });
});

describe('create / stash gates', () => {
  it('c only on commits', () => {
    assert.equal(canCreateBranch(commit([])), true);
    assert.equal(canCreateBranch({ kind: 'uncommitted' }), false);
  });

  it('a/D only on stash', () => {
    const stash = {
      kind: 'stash' as const,
      stash: {
        id: 's',
        stashRef: 'stash@{0}',
        index: 0,
        subject: 'w',
        authorDateUnix: 1,
        parentId: '',
      },
    };
    assert.equal(canStashApply(stash), true);
    assert.equal(canStashDrop(stash), true);
    assert.equal(canStashPop(stash), true);
    assert.equal(canStashApply(commit([])), false);
    assert.equal(canStashPop(commit([])), false);
    assert.equal(canStashPop({ kind: 'uncommitted' }), false);
  });

  it('stash menu is valid on stash and uncommitted; commit needs dirty or a stash', () => {
    const stash = {
      kind: 'stash' as const,
      stash: {
        id: 's',
        stashRef: 'stash@{0}',
        index: 0,
        subject: 'w',
        authorDateUnix: 1,
        parentId: '',
      },
    };
    assert.equal(canStashMenu(stash), true);
    assert.equal(canStashMenu({ kind: 'uncommitted' }), true);
    assert.equal(canStashMenu(commit([])), false);
    assert.equal(canStashMenu(commit([]), { dirty: true }), true);
    assert.equal(canStashMenu(commit([]), { latestStashRef: 'stash@{0}' }), true);
  });
});

describe('checkoutableBranchNames', () => {
  it('lists locals then origin remotes; omits tags and non-origin remotes', () => {
    const row = commit([
      { kind: 'local', name: 'main', commitId: 'aaa' },
      { kind: 'remote', name: 'origin/main', commitId: 'aaa' },
      { kind: 'remote', name: 'origin/other', commitId: 'aaa' },
      { kind: 'remote', name: 'upstream/x', commitId: 'aaa' },
      { kind: 'tag', name: 'v1', commitId: 'aaa' },
    ]);
    assert.deepEqual(checkoutableBranchNames(row), [
      'main',
      'origin/main',
      'origin/other',
    ]);
  });

  it('dedupes and sorts duplicate unsorted origin remotes', () => {
    const row = commit([
      { kind: 'remote', name: 'origin/zeta', commitId: 'aaa' },
      { kind: 'local', name: 'zeta', commitId: 'aaa' },
      { kind: 'remote', name: 'origin/alpha', commitId: 'aaa' },
      { kind: 'local', name: 'beta', commitId: 'aaa' },
      { kind: 'remote', name: 'origin/alpha', commitId: 'aaa' },
      { kind: 'local', name: 'beta', commitId: 'aaa' },
    ]);
    assert.deepEqual(checkoutableBranchNames(row), [
      'beta',
      'zeta',
      'origin/alpha',
      'origin/zeta',
    ]);
  });
});

describe('planGraphCheckout', () => {
  it('local selection never confirms', () => {
    assert.deepEqual(
      planGraphCheckout({
        selectedName: 'main',
        localExists: true,
        localSha: 'aaa',
        remoteSha: 'bbb',
      }),
      { kind: 'checkout', branch: 'main' },
    );
  });

  it('origin with no local checks out the short name', () => {
    assert.deepEqual(
      planGraphCheckout({
        selectedName: 'origin/feature/x',
        localExists: false,
        localSha: null,
        remoteSha: 'abc',
      }),
      { kind: 'checkout', branch: 'feature/x' },
    );
  });

  it('origin with local same SHA checks out the short name', () => {
    assert.deepEqual(
      planGraphCheckout({
        selectedName: 'origin/main',
        localExists: true,
        localSha: 'aaa',
        remoteSha: 'aaa',
      }),
      { kind: 'checkout', branch: 'main' },
    );
  });

  it('origin with local different SHA confirms then pull', () => {
    assert.deepEqual(
      planGraphCheckout({
        selectedName: 'origin/main',
        localExists: true,
        localSha: 'aaa',
        remoteSha: 'bbb',
      }),
      {
        kind: 'confirmLocalThenPull',
        localBranch: 'main',
        remoteRef: 'origin/main',
      },
    );
  });

  it('origin with local but a SHA is null confirms then pull', () => {
    assert.deepEqual(
      planGraphCheckout({
        selectedName: 'origin/main',
        localExists: true,
        localSha: 'aaa',
        remoteSha: null,
      }),
      {
        kind: 'confirmLocalThenPull',
        localBranch: 'main',
        remoteRef: 'origin/main',
      },
    );
    assert.deepEqual(
      planGraphCheckout({
        selectedName: 'origin/main',
        localExists: true,
        localSha: null,
        remoteSha: 'bbb',
      }),
      {
        kind: 'confirmLocalThenPull',
        localBranch: 'main',
        remoteRef: 'origin/main',
      },
    );
  });
});

describe('resolveCheckoutTarget', () => {
  it('maps name counts to none / single / picker', () => {
    assert.equal(resolveCheckoutTarget([]), 'none');
    assert.equal(resolveCheckoutTarget(['main']), 'single');
    assert.equal(resolveCheckoutTarget(['a', 'b']), 'picker');
  });
});

describe('runBusyThenRefresh', () => {
  it('local checkout does not stick on Busy when refresh needs a free busyRef', async () => {
    const busyRef = { current: false };
    let status = '';
    let checkedOut: string | null = null;
    let refreshed = false;
    const plan = planGraphCheckout({
      selectedName: 'feature/x',
      localExists: true,
      localSha: 'aaa',
      remoteSha: 'bbb',
    });
    assert.equal(plan.kind, 'checkout');

    await runBusyThenRefresh({
      busyRef,
      onBusy: () => {
        status = 'Busy…';
      },
      work: async () => {
        checkedOut = plan.kind === 'checkout' ? plan.branch : null;
        return checkedOut;
      },
      afterRelease: async (branch) => {
        if (!branch) return;
        if (busyRef.current) {
          status = 'Busy…';
          return;
        }
        busyRef.current = true;
        try {
          refreshed = true;
        } finally {
          busyRef.current = false;
        }
      },
    });

    assert.equal(checkedOut, 'feature/x');
    assert.equal(refreshed, true);
    assert.notEqual(status, 'Busy…');
    assert.equal(busyRef.current, false);
  });

  it('origin confirm returns before refresh and leaves busyRef free', async () => {
    const busyRef = { current: false };
    let status = '';
    let confirmed = false;
    let refreshed = false;
    const plan = planGraphCheckout({
      selectedName: 'origin/main',
      localExists: true,
      localSha: 'aaa',
      remoteSha: 'bbb',
    });
    assert.equal(plan.kind, 'confirmLocalThenPull');

    await runBusyThenRefresh({
      busyRef,
      onBusy: () => {
        status = 'Busy…';
      },
      work: async () => {
        confirmed = true;
        return null;
      },
      afterRelease: async (branch) => {
        if (branch == null) return;
        refreshed = true;
      },
    });

    assert.equal(confirmed, true);
    assert.equal(refreshed, false);
    assert.notEqual(status, 'Busy…');
    assert.equal(busyRef.current, false);
  });

  it('already-busy local checkout reports Busy and does not run work', async () => {
    const busyRef = { current: true };
    let status = '';
    let worked = false;
    await runBusyThenRefresh({
      busyRef,
      onBusy: () => {
        status = 'Busy…';
      },
      work: async () => {
        worked = true;
        return 'feature/x';
      },
      afterRelease: async () => {
        worked = true;
      },
    });
    assert.equal(status, 'Busy…');
    assert.equal(worked, false);
    assert.equal(busyRef.current, true);
  });
});
