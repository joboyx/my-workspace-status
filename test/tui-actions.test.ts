import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  buildPendingConfirm,
  buildRemoveWorktreeConfirm,
  isRevertible,
  opPaths,
  refreshPathsAfterRemoveWorktree,
  shouldDeleteUntracked,
  toRevertTarget,
} from '../src/tui/useActions.js';
import type { CheckoutNode, FileNode, VisibleRow } from '../src/tui/model/types.js';

function fileNode(over: Partial<FileNode> = {}): FileNode {
  const filePath = over.path ?? 'src/a.ts';
  return {
    kind: 'file',
    id: `file:repo:${filePath}`,
    path: filePath,
    repoPath: 'repo',
    status: 'M',
    staged: false,
    unstaged: true,
    untracked: false,
    change: { path: filePath, status: 'M' } as FileNode['change'],
    ...over,
  };
}

function fileRow(file: FileNode): VisibleRow {
  return {
    id: file.id,
    depth: 1,
    node: file,
    label: file.path,
    segments: [],
    trailing: [],
  };
}

describe('opPaths', () => {
  it('returns both endpoints of a rename so git records delete + add', () => {
    const file = fileNode({
      path: 'src/new.ts',
      status: 'R',
      renameFrom: 'src/old.ts',
    });
    assert.deepEqual(opPaths(file), ['src/old.ts', 'src/new.ts']);
  });

  it('falls back to the old path recorded on the change', () => {
    const file = fileNode({
      path: 'src/new.ts',
      status: 'R',
      change: {
        path: 'src/new.ts',
        status: 'R',
        oldPath: 'src/old.ts',
      } as FileNode['change'],
    });
    assert.deepEqual(opPaths(file), ['src/old.ts', 'src/new.ts']);
  });

  it('returns a single path for a plain modification', () => {
    assert.deepEqual(opPaths(fileNode()), ['src/a.ts']);
  });

  it('returns a single path when the rename source equals the target', () => {
    const file = fileNode({ path: 'src/a.ts', renameFrom: 'src/a.ts' });
    assert.deepEqual(opPaths(file), ['src/a.ts']);
  });
});

describe('isRevertible', () => {
  it('keeps unstaged and untracked; skips staged-only', () => {
    assert.equal(isRevertible(fileNode({ unstaged: true })), true);
    assert.equal(
      isRevertible(fileNode({ unstaged: false, untracked: true, status: '?' })),
      true,
    );
    assert.equal(
      isRevertible(
        fileNode({ staged: true, unstaged: false, untracked: false }),
      ),
      false,
    );
  });
});

describe('toRevertTarget / buildPendingConfirm', () => {
  it('serialises renameFrom from change.oldPath when needed', () => {
    const file = fileNode({
      path: 'src/new.ts',
      change: {
        path: 'src/new.ts',
        status: 'R',
        oldPath: 'src/old.ts',
      } as FileNode['change'],
    });
    assert.deepEqual(toRevertTarget(file), {
      path: 'src/new.ts',
      untracked: false,
      renameFrom: 'src/old.ts',
    });
  });

  it('counts tracked vs untracked under the focused label', () => {
    const tracked = fileNode({ path: 'a.ts' });
    const untracked = fileNode({
      path: 'b.tmp',
      unstaged: false,
      untracked: true,
      status: '?',
    });
    const pending = buildPendingConfirm(fileRow(tracked), [tracked, untracked]);
    assert.equal(pending.kind, 'revert');
    assert.equal(pending.repo, 'repo');
    assert.equal(pending.label, 'a.ts');
    assert.equal(pending.trackedCount, 1);
    assert.equal(pending.untrackedCount, 1);
    assert.equal(pending.targets.length, 2);
  });
});

describe('buildRemoveWorktreeConfirm', () => {
  it('marks force when the checkout has changes', () => {
    const node: CheckoutNode = {
      kind: 'checkout',
      id: 'checkout:app/.worktrees/feat',
      path: 'app/.worktrees/feat',
      branch: 'feature/x',
      checkoutKind: 'linked',
      primaryRepo: 'app',
      mergedIntoDefault: false,
      sync: '=',
      syncStatus: 'up-to-date',
      changeCount: 2,
      children: [],
    };
    const pending = buildRemoveWorktreeConfirm(node);
    assert.equal(pending.kind, 'removeWorktree');
    assert.equal(pending.force, true);
    assert.equal(pending.dirty, true);
    assert.equal(pending.mergedIntoDefault, false);
    assert.equal(pending.primaryRepo, 'app');
  });
});

describe('refreshPathsAfterRemoveWorktree', () => {
  it('refreshes primary when it is already in the snapshot list', () => {
    assert.deepEqual(
      refreshPathsAfterRemoveWorktree('app/.worktrees/feat', 'app', [
        'app',
        'app/.worktrees/feat',
      ]),
      ['app/.worktrees/feat', 'app'],
    );
  });

  it('drops linked only when primary was not listed (named-filter linked-only)', () => {
    assert.deepEqual(
      refreshPathsAfterRemoveWorktree('app/.worktrees/feat', 'app', [
        'app/.worktrees/feat',
      ]),
      ['app/.worktrees/feat'],
    );
  });
});

describe('shouldDeleteUntracked', () => {
  it('deletes on confirmYesClean, or when the only target is untracked', () => {
    assert.equal(shouldDeleteUntracked([{ path: 'a', untracked: false }], true), true);
    assert.equal(
      shouldDeleteUntracked([{ path: 'a', untracked: true }], false),
      true,
    );
    assert.equal(
      shouldDeleteUntracked(
        [
          { path: 'a', untracked: true },
          { path: 'b', untracked: true },
        ],
        false,
      ),
      false,
    );
  });
});
