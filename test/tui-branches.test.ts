import assert from 'node:assert';
import { describe, it } from 'node:test';
import type { LocalBranch } from '../src/git.js';
import { branchPickerPath, filterBranches, sortBranchesForPicker } from '../src/tui/branches.js';
import { flatten } from '../src/tui/model/flatten.js';
import { buildTree } from '../src/tui/model/tree.js';
import type { RepoSnapshot } from '../src/types.js';

function branch(
  name: string,
  authordate: number,
  current = false,
): LocalBranch {
  return { name, authordate, current };
}

describe('sortBranchesForPicker', () => {
  it('pins the default branch first, then sorts by newest authordate', () => {
    const branches = [
      branch('feature/old', 100),
      branch('main', 50, true),
      branch('feature/new', 200),
      branch('bugfix/mid', 150),
    ];
    assert.deepEqual(
      sortBranchesForPicker(branches, 'main').map((b) => b.name),
      ['main', 'feature/new', 'bugfix/mid', 'feature/old'],
    );
  });

  it('sorts by newest authordate when defaultBranch is null', () => {
    const branches = [
      branch('a', 10),
      branch('b', 30),
      branch('c', 20),
    ];
    assert.deepEqual(
      sortBranchesForPicker(branches, null).map((b) => b.name),
      ['b', 'c', 'a'],
    );
  });

  it('does not mutate the input array', () => {
    const branches = [branch('z', 1), branch('main', 2)];
    const copy = [...branches];
    sortBranchesForPicker(branches, 'main');
    assert.deepEqual(branches, copy);
  });

  it('keeps default first even when it is not the newest', () => {
    const branches = [branch('develop', 1), branch('feature/x', 999)];
    assert.deepEqual(
      sortBranchesForPicker(branches, 'develop').map((b) => b.name),
      ['develop', 'feature/x'],
    );
  });
});

describe('filterBranches', () => {
  const branches = [
    branch('main', 1),
    branch('feature/JBY-036-picker', 2),
    branch('bugfix/Login', 3),
  ];

  it('returns all branches when query is empty or whitespace', () => {
    assert.deepEqual(filterBranches(branches, ''), branches);
    assert.deepEqual(filterBranches(branches, '   '), branches);
  });

  it('filters by case-insensitive substring on the name', () => {
    assert.deepEqual(
      filterBranches(branches, 'jby').map((b) => b.name),
      ['feature/JBY-036-picker'],
    );
    assert.deepEqual(
      filterBranches(branches, 'LOGIN').map((b) => b.name),
      ['bugfix/Login'],
    );
    assert.deepEqual(
      filterBranches(branches, 'MaIn').map((b) => b.name),
      ['main'],
    );
  });

  it('returns an empty list when nothing matches', () => {
    assert.deepEqual(filterBranches(branches, 'nope'), []);
  });
});

describe('branchPickerPath', () => {
  function snap(over: Partial<RepoSnapshot> & Pick<RepoSnapshot, 'repo'>): RepoSnapshot {
    return {
      branch: 'main',
      syncStatus: 'up-to-date',
      syncNote: '',
      hasStaged: false,
      hasUnstaged: false,
      hasUntracked: false,
      staged: [],
      unstaged: [],
      untracked: [],
      checkoutKind: 'primary',
      mergedIntoDefault: null,
      ...over,
    };
  }

  it('opens on a flat repo and on checkout children, not a family container', () => {
    const primary = snap({ repo: 'app', branch: 'main' });
    const linked = snap({
      repo: 'app/.worktrees/feat',
      branch: 'feature/x',
      checkoutKind: 'linked',
      primaryRepo: 'app',
    });
    const family = flatten(
      buildTree({
        snapshots: [primary, linked],
        ignoredRepos: new Set(),
        treeMode: false,
        workspaceLabel: 'ws',
      }),
      new Set(),
    );
    const container = family.find((r) => r.node.kind === 'repo');
    const primaryRow = family.find(
      (r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'primary',
    );
    const linkedRow = family.find(
      (r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'linked',
    );
    assert.ok(container);
    assert.ok(primaryRow);
    assert.ok(linkedRow);
    assert.equal(branchPickerPath(container), null);
    assert.equal(branchPickerPath(primaryRow), 'app');
    assert.equal(branchPickerPath(linkedRow), 'app/.worktrees/feat');
    assert.equal(branchPickerPath(null), null);

    const flatRepo = flatten(
      buildTree({
        snapshots: [primary],
        ignoredRepos: new Set(),
        treeMode: false,
        workspaceLabel: 'ws',
      }),
      new Set(),
    ).find((r) => r.node.kind === 'repo');
    assert.ok(flatRepo);
    assert.equal(branchPickerPath(flatRepo), 'app');
  });
});
