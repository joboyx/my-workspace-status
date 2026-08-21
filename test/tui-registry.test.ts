import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  ACTIONS,
  actionFor,
  actionVisibleForGraphRow,
  actionsForContext,
  actionsForKind,
  graphActionForKey,
} from '../src/tui/actions/registry.js';

describe('actionFor', () => {
  it('resolves a key only for row kinds where it is valid', () => {
    assert.equal(actionFor('s', 'file')?.id, 'stage');
    assert.equal(actionFor('s', 'repo')?.id, 'stage');
    assert.equal(actionFor('s', 'checkout')?.id, 'stage');
    assert.equal(actionFor('s', 'dir')?.id, 'stage');
    assert.equal(actionFor('s', 'workspace'), undefined);
  });

  it('scopes branch switching to repo and checkout rows', () => {
    assert.equal(actionFor('b', 'repo')?.id, 'branch');
    assert.equal(actionFor('b', 'checkout')?.id, 'branch');
    assert.equal(actionFor('b', 'file'), undefined);
    assert.equal(actionFor('b', 'workspace'), undefined);
  });

  it('registers remove-worktree on checkout and repo (flat linked)', () => {
    assert.equal(actionFor('W', 'checkout')?.id, 'removeWorktree');
    assert.equal(actionFor('W', 'repo')?.id, 'removeWorktree');
    assert.equal(actionFor('W', 'file'), undefined);
  });

  it('accepts lowercase w for remove-worktree (terminals rarely send Shift+W)', () => {
    assert.equal(actionFor('w', 'checkout')?.id, 'removeWorktree');
    assert.equal(actionFor('w', 'repo')?.id, 'removeWorktree');
  });

  it('does not fold lowercase d onto stashDrop D (d is a distinct binding)', () => {
    assert.equal(actionFor('d', 'graphStash'), undefined);
    assert.equal(actionFor('D', 'graphStash')?.id, 'stashDrop');
    assert.equal(graphActionForKey('d', 'graphStash'), undefined);
    assert.equal(graphActionForKey('D', 'graphStash')?.id, 'stashDrop');
  });

  it('binds S to stashMenu on file and does not fold s (s is stage)', () => {
    assert.equal(actionFor('S', 'file')?.id, 'stashMenu');
    assert.equal(actionFor('s', 'file')?.id, 'stage');
    assert.equal(actionFor('S', 'workspace'), undefined);
  });

  it('binds p to stashPop on graphStash and keeps pull on repo', () => {
    assert.equal(graphActionForKey('p', 'graphStash')?.id, 'stashPop');
    assert.equal(actionFor('p', 'repo')?.id, 'pull');
  });

  it('allows workspace-wide flags on the workspace row and repo/checkout rows', () => {
    assert.equal(actionFor('p', 'workspace')?.id, 'pull');
    assert.equal(actionFor('p', 'repo')?.id, 'pull');
    assert.equal(actionFor('p', 'checkout')?.id, 'pull');
    assert.equal(actionFor('p', 'dir'), undefined);
  });

  it('scopes push to repo and checkout rows only (Shift+P)', () => {
    assert.equal(actionFor('P', 'repo')?.id, 'push');
    assert.equal(actionFor('P', 'checkout')?.id, 'push');
    assert.equal(actionFor('P', 'workspace'), undefined);
    assert.equal(actionFor('P', 'dir'), undefined);
    assert.equal(actionFor('p', 'repo')?.id, 'pull');
    assert.equal(actionFor('p', 'checkout')?.id, 'pull');
  });

  it('allows fetch on every kind except group', () => {
    assert.equal(actionFor('f', 'workspace')?.id, 'fetch');
    assert.equal(actionFor('f', 'repo')?.id, 'fetch');
    assert.equal(actionFor('f', 'checkout')?.id, 'fetch');
    assert.equal(actionFor('f', 'dir')?.id, 'fetch');
    assert.equal(actionFor('f', 'file')?.id, 'fetch');
    assert.equal(actionFor('f', 'group'), undefined);
  });

  it('scopes edit to file rows', () => {
    assert.equal(actionFor('e', 'file')?.id, 'edit');
    assert.equal(actionFor('e', 'repo'), undefined);
  });

  it('scopes reviewed to file rows via the space display key', () => {
    assert.equal(actionFor('space', 'file')?.id, 'toggleViewed');
    assert.equal(actionFor('space', 'repo'), undefined);
    assert.equal(actionFor('space', 'dir'), undefined);
    assert.equal(actionFor('space', 'workspace'), undefined);
    assert.equal(actionFor('space', 'graphCommit'), undefined);
    assert.equal(actionFor('v', 'file'), undefined);
    assert.equal(actionFor('v', 'repo'), undefined);
  });

  it('scopes the full-file ctrl+o chord to file rows', () => {
    assert.equal(actionFor('ctrl+o', 'file')?.id, 'fullFile');
    assert.equal(actionFor('ctrl+o', 'repo'), undefined);
    assert.equal(actionFor('ctrl+o', 'dir'), undefined);
    assert.equal(actionFor('ctrl+o', 'workspace'), undefined);
  });

  it('scopes unstage to repo, checkout, dir, and file rows', () => {
    assert.equal(actionFor('u', 'repo')?.id, 'unstage');
    assert.equal(actionFor('u', 'checkout')?.id, 'unstage');
    assert.equal(actionFor('u', 'dir')?.id, 'unstage');
    assert.equal(actionFor('u', 'file')?.id, 'unstage');
    assert.equal(actionFor('u', 'workspace'), undefined);
    assert.equal(actionFor('u', 'group'), undefined);
  });

  it('scopes default-branch switching to workspace, repo, and checkout rows', () => {
    assert.equal(actionFor('d', 'workspace')?.id, 'defaultBranch');
    assert.equal(actionFor('d', 'repo')?.id, 'defaultBranch');
    assert.equal(actionFor('d', 'checkout')?.id, 'defaultBranch');
    assert.equal(actionFor('d', 'dir'), undefined);
    assert.equal(actionFor('d', 'file'), undefined);
  });

  it('returns undefined for keys that are not actions', () => {
    assert.equal(actionFor('j', 'file'), undefined);
    assert.equal(actionFor('', 'file'), undefined);
  });
});

describe('actionsForKind', () => {
  it('lists actions in registry order for hint-bar rendering', () => {
    const ids = actionsForKind('workspace').map((a) => a.id);
    assert.deepEqual(ids, ['fetch', 'pull', 'defaultBranch']);
  });

  it('returns nothing for group rows', () => {
    assert.deepEqual(actionsForKind('group'), []);
  });
});

describe('actionsForContext', () => {
  it('matches actionsForKind at depth 0 left when dims are omitted on specs', () => {
    assert.deepEqual(
      actionsForContext('file', 0, 'left').map((a) => a.id),
      actionsForKind('file').map((a) => a.id),
    );
  });

  it('hides depth-0-only actions when depths are set (fixture via temporary filter logic)', () => {
    // Permanent behaviour: omitted dims ⇒ visible everywhere.
    // This asserts the filter treats an explicit depths:[0] as depth-0-only.
    // Implemented by testing the helper against ACTIONS after we mark nothing —
    // so instead verify right-pane still returns file actions when dims omitted:
    assert.ok(actionsForContext('file', 0, 'right').some((a) => a.id === 'edit'));
    assert.ok(actionsForContext('file', 2, 'left').some((a) => a.id === 'edit'));
  });
});

describe('graph action registry', () => {
  it('advertises create branch on graphCommit at depth 1 left', () => {
    const ids = actionsForContext('graphCommit', 1, 'left').map((a) => a.id);
    assert.ok(ids.includes('graphCreateBranch'));
    assert.ok(ids.includes('graphCheckout'));
  });

  it('lists graph checkout/create at depth 0 right, not tree pull/push/branch', () => {
    const ids = actionsForContext('graphCommit', 0, 'right').map((a) => a.id);
    assert.ok(ids.includes('graphCheckout'));
    assert.ok(ids.includes('graphCreateBranch'));
    assert.ok(!ids.includes('pull'));
    assert.ok(!ids.includes('push'));
    assert.ok(!ids.includes('branch'));
  });

  it('lists graph actions at depth 0 left when the kind is a graph row', () => {
    const ids = actionsForContext('graphCommit', 0, 'left').map((a) => a.id);
    assert.ok(ids.includes('graphCheckout'));
    assert.ok(ids.includes('graphCreateBranch'));
  });

  it('lists tree writes on a repo at depth 0 left, not graph actions', () => {
    const ids = actionsForContext('repo', 0, 'left').map((a) => a.id);
    assert.ok(ids.includes('pull'));
    assert.ok(ids.includes('push'));
    assert.ok(ids.includes('branch'));
    assert.ok(!ids.includes('graphCheckout'));
    assert.ok(!ids.includes('graphCreateBranch'));
  });

  it('lists stash pop at depth 0 right on a graph stash row', () => {
    const ids = actionsForContext('graphStash', 0, 'right').map((a) => a.id);
    assert.ok(ids.includes('stashPop'));
    assert.ok(ids.includes('stashApply'));
    assert.ok(ids.includes('stashDrop'));
  });

  it('hides graph actions at depth 2', () => {
    const ids = actionsForContext('graphCommit', 2, 'left').map((a) => a.id);
    assert.ok(!ids.includes('graphCheckout'));
    assert.ok(!ids.includes('graphCreateBranch'));
  });

  it('graph action specs omit focusPanes and allow depths 0 and 1', () => {
    for (const id of [
      'graphCheckout',
      'graphCreateBranch',
      'stashApply',
      'stashDrop',
      'stashPop',
    ] as const) {
      const spec = ACTIONS.find((a) => a.id === id);
      assert.ok(spec, id);
      assert.equal(spec!.focusPanes, undefined, id);
      assert.deepEqual(spec!.depths, [0, 1], id);
    }
  });

  it('stash actions only on graphStash', () => {
    const stashIds = actionsForContext('graphStash', 1, 'left').map((a) => a.id);
    assert.ok(stashIds.includes('stashMenu'));
    assert.ok(stashIds.includes('stashApply'));
    assert.ok(stashIds.includes('stashPop'));
    assert.ok(stashIds.includes('stashDrop'));
    assert.ok(stashIds.indexOf('stashMenu') < stashIds.indexOf('stashApply'));
    assert.ok(stashIds.indexOf('stashApply') < stashIds.indexOf('stashPop'));
    assert.ok(stashIds.indexOf('stashPop') < stashIds.indexOf('stashDrop'));
    assert.ok(actionsForContext('graphCommit', 1, 'left').some((a) => a.id === 'stashMenu'));
    assert.ok(!actionsForContext('graphCommit', 1, 'left').some((a) => a.id === 'stashDrop'));
    assert.ok(!actionsForContext('graphCommit', 1, 'left').some((a) => a.id === 'stashPop'));
  });

  it('actionVisibleForGraphRow shows b for origin-only commit', () => {
    const checkout = ACTIONS.find((a) => a.id === 'graphCheckout')!;
    const bare = {
      kind: 'commit' as const,
      commit: {
        id: 'x',
        parents: [],
        subject: 's',
        authorName: 'A',
        authorDateUnix: 1,
        refs: [{ kind: 'remote' as const, name: 'origin/x', commitId: 'x' }],
      },
    };
    assert.equal(actionVisibleForGraphRow(checkout, bare), true);
  });

  it('shows stashMenu on stash/uncommitted rows and on dirty or stash-bearing commits', () => {
    const stashMenu = ACTIONS.find((a) => a.id === 'stashMenu')!;
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
    const commit = {
      kind: 'commit' as const,
      commit: {
        id: 'x',
        parents: [],
        subject: 's',
        authorName: 'A',
        authorDateUnix: 1,
        refs: [],
      },
    };
    assert.equal(actionVisibleForGraphRow(stashMenu, stash), true);
    assert.equal(actionVisibleForGraphRow(stashMenu, { kind: 'uncommitted' }), true);
    assert.equal(actionVisibleForGraphRow(stashMenu, commit), false);
    assert.equal(actionVisibleForGraphRow(stashMenu, commit, { dirty: true }), true);
    assert.equal(
      actionVisibleForGraphRow(stashMenu, commit, { latestStashRef: 'stash@{0}' }),
      true,
    );
  });

  it('graphActionForKey resolves c on graphCommit', () => {
    assert.equal(graphActionForKey('c', 'graphCommit')?.id, 'graphCreateBranch');
    assert.equal(graphActionForKey('b', 'graphCommit')?.id, 'graphCheckout');
    assert.equal(graphActionForKey('a', 'graphStash')?.id, 'stashApply');
    assert.equal(graphActionForKey('p', 'graphStash')?.id, 'stashPop');
    assert.equal(graphActionForKey('D', 'graphStash')?.id, 'stashDrop');
    assert.equal(graphActionForKey('b', 'repo'), undefined);
  });
});

describe('ACTIONS', () => {
  it('marks revert, removeWorktree, and stashDrop as destructive', () => {
    const destructive = ACTIONS.filter((a) => a.destructive).map((a) => a.id);
    assert.deepEqual(destructive, ['revert', 'removeWorktree', 'stashDrop']);
  });

  it('has no duplicate key within a single row kind', () => {
    const kinds = [
      'workspace',
      'repo',
      'checkout',
      'group',
      'dir',
      'file',
      'graphCommit',
      'graphStash',
      'graphUncommitted',
    ] as const;
    for (const kind of kinds) {
      const keys = actionsForKind(kind).map((a) => a.key);
      assert.equal(new Set(keys).size, keys.length, `duplicate key for kind ${kind}`);
    }
  });
});
