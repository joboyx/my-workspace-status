import assert from 'node:assert';
import { describe, it } from 'node:test';
import { buildTree } from '../src/tui/model/tree.js';
import { flatten } from '../src/tui/model/flatten.js';
import {
  collectBackgroundFetchTargets,
  collectBulkGitTargets,
  collectFiles,
} from '../src/tui/scope.js';
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

describe('collectFiles', () => {
  it('collects all files under a repo row', () => {
    const snap = base({
      repo: 'app',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts|||M\tsrc/b.ts',
    });
    const tree = buildTree({
      snapshots: [snap],
      ignoredRepos: new Set(),
      treeMode: true,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const repoRow = rows.find((r) => r.node.kind === 'repo');
    assert.ok(repoRow);
    const files = collectFiles(repoRow);
    assert.deepEqual(files.map((f) => f.path).sort(), ['src/a.ts', 'src/b.ts']);
  });

  it('returns a single file for a file row', () => {
    const snap = base({
      repo: 'app',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts',
    });
    const tree = buildTree({
      snapshots: [snap],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const fileRow = rows.find((r) => r.node.kind === 'file');
    assert.ok(fileRow);
    assert.equal(collectFiles(fileRow).length, 1);
  });

  it('collects all files under a dir row', () => {
    const snap = base({
      repo: 'app',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts|||M\tsrc/b.ts|||M\tother/c.ts',
    });
    const tree = buildTree({
      snapshots: [snap],
      ignoredRepos: new Set(),
      treeMode: true,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const dirRow = rows.find((r) => r.node.kind === 'dir' && r.node.path === 'src');
    assert.ok(dirRow);
    const files = collectFiles(dirRow);
    assert.deepEqual(files.map((f) => f.path).sort(), ['src/a.ts', 'src/b.ts']);
  });

  it('returns empty for a family container; files only under checkout', () => {
    const primary = base({
      repo: 'app',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts',
    });
    const linked = base({
      repo: 'app/.worktrees/feat',
      branch: 'feature/x',
      checkoutKind: 'linked',
      primaryRepo: 'app',
      mergedIntoDefault: false,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/b.ts',
    });
    const tree = buildTree({
      snapshots: [primary, linked],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const container = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'app');
    const primaryCheckout = rows.find((r) => r.node.kind === 'checkout' && r.node.path === 'app');
    const linkedCheckout = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.path === 'app/.worktrees/feat',
    );
    assert.ok(container);
    assert.ok(primaryCheckout);
    assert.ok(linkedCheckout);
    assert.deepEqual(collectFiles(container), []);
    assert.deepEqual(
      collectFiles(primaryCheckout).map((f) => f.path),
      ['src/a.ts'],
    );
    assert.deepEqual(
      collectFiles(linkedCheckout).map((f) => f.path),
      ['src/b.ts'],
    );
  });

  it('returns empty for workspace and group rows', () => {
    const tree = buildTree({
      snapshots: [
        base({ repo: 'clean-a' }),
        base({ repo: 'clean-b' }),
        base({
          repo: 'dirty',
          hasUnstaged: true,
          unstagedFiles: 'M\ta.ts',
        }),
      ],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const workspaceRow = rows.find((r) => r.node.kind === 'workspace');
    const groupRow = rows.find((r) => r.node.kind === 'group');
    assert.ok(workspaceRow);
    assert.ok(groupRow);
    assert.deepEqual(collectFiles(workspaceRow), []);
    assert.deepEqual(collectFiles(groupRow), []);
  });
});

describe('collectBulkGitTargets', () => {
  function familySnaps(): RepoSnapshot[] {
    return [
      base({
        repo: 'app',
        branch: 'main',
        syncStatus: 'behind',
      }),
      base({
        repo: 'app/.worktrees/feat',
        branch: 'feature/x',
        syncStatus: 'behind',
        checkoutKind: 'linked',
        primaryRepo: 'app',
        mergedIntoDefault: false,
      }),
      base({
        repo: 'notes',
        branch: 'main',
        syncStatus: 'behind',
      }),
    ];
  }

  function familyRows() {
    const snapshots = familySnaps();
    const tree = buildTree({
      snapshots,
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const workspace = rows.find((r) => r.node.kind === 'workspace');
    const family = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'app');
    const notes = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'notes');
    const primary = rows.find((r) => r.node.kind === 'checkout' && r.node.path === 'app');
    const linked = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.path === 'app/.worktrees/feat',
    );
    assert.ok(workspace);
    assert.ok(family);
    assert.ok(notes);
    assert.ok(primary);
    assert.ok(linked);
    return { snapshots, workspace, family, notes, primary, linked };
  }

  it('skips linked worktrees on workspace and family rows', () => {
    const { snapshots, workspace, family, notes } = familyRows();
    assert.deepEqual(collectBulkGitTargets(workspace, snapshots).sort(), ['app', 'notes']);
    assert.deepEqual(collectBulkGitTargets(family, snapshots), ['app']);
    assert.deepEqual(collectBulkGitTargets(notes, snapshots), ['notes']);
  });

  it('includes a worktree only when that checkout is focused', () => {
    const { snapshots, primary, linked } = familyRows();
    assert.deepEqual(collectBulkGitTargets(primary, snapshots), ['app']);
    assert.deepEqual(collectBulkGitTargets(linked, snapshots), ['app/.worktrees/feat']);
  });

  it('includes a worktree when a file under it is focused', () => {
    const snapshots = [
      base({
        repo: 'app',
        hasUnstaged: true,
        unstagedFiles: 'M\tsrc/a.ts',
      }),
      base({
        repo: 'app/.worktrees/feat',
        branch: 'feature/x',
        checkoutKind: 'linked',
        primaryRepo: 'app',
        mergedIntoDefault: false,
        hasUnstaged: true,
        unstagedFiles: 'M\tsrc/b.ts',
      }),
    ];
    const tree = buildTree({
      snapshots,
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const file = rows.find(
      (r) => r.node.kind === 'file' && r.node.repoPath === 'app/.worktrees/feat',
    );
    assert.ok(file);
    assert.deepEqual(collectBulkGitTargets(file, snapshots), ['app/.worktrees/feat']);
  });

  it('includes a flat linked repo because that row is the worktree', () => {
    const snapshots = [
      base({
        repo: 'app/.worktrees/feat',
        branch: 'feature/x',
        checkoutKind: 'linked',
        primaryRepo: 'app',
        mergedIntoDefault: false,
      }),
    ];
    const tree = buildTree({
      snapshots,
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const flat = rows.find((r) => r.node.kind === 'repo');
    assert.ok(flat);
    assert.deepEqual(collectBulkGitTargets(flat, snapshots), ['app/.worktrees/feat']);
  });

  it('skips ignored primaries on a workspace row', () => {
    const { snapshots, workspace } = familyRows();
    const ignored = new Set(['notes']);
    assert.deepEqual(collectBulkGitTargets(workspace, snapshots, ignored), ['app']);
  });

  it('skips a focused ignored repo while ignored repos are hidden', () => {
    const { snapshots, notes, workspace } = familyRows();
    const ignored = new Set(['notes']);
    assert.deepEqual(collectBulkGitTargets(notes, snapshots, ignored, false), []);
    assert.deepEqual(collectBulkGitTargets(workspace, snapshots, ignored, false), ['app']);
  });

  it('includes ignored repos like other visible rows when they are shown', () => {
    const { snapshots, notes, workspace, family, linked } = familyRows();
    const ignored = new Set(['notes']);
    assert.deepEqual(collectBulkGitTargets(workspace, snapshots, ignored, true).sort(), [
      'app',
      'notes',
    ]);
    assert.deepEqual(collectBulkGitTargets(notes, snapshots, ignored, true), ['notes']);
    assert.deepEqual(collectBulkGitTargets(family, snapshots, ignored, true), ['app']);
    assert.deepEqual(collectBulkGitTargets(linked, snapshots, ignored, true), [
      'app/.worktrees/feat',
    ]);
  });

  it('skips a focused checkout inside a hidden ignored family', () => {
    const snapshots = [
      base({
        repo: 'notes',
        branch: 'main',
        syncStatus: 'behind',
      }),
      base({
        repo: 'notes/.worktrees/feat',
        branch: 'feature/x',
        syncStatus: 'behind',
        checkoutKind: 'linked',
        primaryRepo: 'notes',
        mergedIntoDefault: false,
      }),
      base({ repo: 'app', branch: 'main', syncStatus: 'behind' }),
    ];
    const ignored = new Set(['notes']);
    const tree = buildTree({
      snapshots,
      ignoredRepos: ignored,
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const workspace = rows.find((r) => r.node.kind === 'workspace');
    const family = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'notes');
    const linked = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.path === 'notes/.worktrees/feat',
    );
    const fileTree = buildTree({
      snapshots: [
        base({
          repo: 'notes',
          hasUnstaged: true,
          unstagedFiles: 'M\tREADME.md',
        }),
        base({ repo: 'app' }),
      ],
      ignoredRepos: ignored,
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const fileRows = flatten(fileTree, new Set());
    const file = fileRows.find((r) => r.node.kind === 'file' && r.node.repoPath === 'notes');
    assert.ok(workspace);
    assert.ok(family);
    assert.ok(linked);
    assert.ok(file);
    assert.deepEqual(collectBulkGitTargets(workspace, snapshots, ignored, false).sort(), ['app']);
    assert.deepEqual(collectBulkGitTargets(family, snapshots, ignored, false), []);
    assert.deepEqual(collectBulkGitTargets(linked, snapshots, ignored, false), []);
    assert.deepEqual(
      collectBulkGitTargets(file, [snapshots[0]!, snapshots[2]!], ignored, false),
      [],
    );
  });

  it('treats a shown ignored family like other visible rows', () => {
    const snapshots = [
      base({
        repo: 'notes',
        branch: 'main',
        syncStatus: 'behind',
      }),
      base({
        repo: 'notes/.worktrees/feat',
        branch: 'feature/x',
        syncStatus: 'behind',
        checkoutKind: 'linked',
        primaryRepo: 'notes',
        mergedIntoDefault: false,
      }),
      base({ repo: 'app', branch: 'main', syncStatus: 'behind' }),
    ];
    const ignored = new Set(['notes']);
    const tree = buildTree({
      snapshots,
      ignoredRepos: ignored,
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const workspace = rows.find((r) => r.node.kind === 'workspace');
    const family = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'notes');
    const linked = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.path === 'notes/.worktrees/feat',
    );
    const fileTree = buildTree({
      snapshots: [
        base({
          repo: 'notes',
          hasUnstaged: true,
          unstagedFiles: 'M\tREADME.md',
        }),
        base({ repo: 'app' }),
      ],
      ignoredRepos: ignored,
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const fileRows = flatten(fileTree, new Set());
    const file = fileRows.find((r) => r.node.kind === 'file' && r.node.repoPath === 'notes');
    assert.ok(workspace);
    assert.ok(family);
    assert.ok(linked);
    assert.ok(file);
    assert.deepEqual(collectBulkGitTargets(workspace, snapshots, ignored, true).sort(), [
      'app',
      'notes',
    ]);
    assert.deepEqual(collectBulkGitTargets(family, snapshots, ignored, true), ['notes']);
    assert.deepEqual(collectBulkGitTargets(linked, snapshots, ignored, true), [
      'notes/.worktrees/feat',
    ]);
    assert.deepEqual(collectBulkGitTargets(file, [snapshots[0]!, snapshots[2]!], ignored, true), [
      'notes',
    ]);
  });

  it('keeps the worktree rule when an ignore list is present', () => {
    const { snapshots, workspace, family, linked } = familyRows();
    const ignored = new Set(['notes']);
    assert.deepEqual(collectBulkGitTargets(workspace, snapshots, ignored), ['app']);
    assert.deepEqual(collectBulkGitTargets(family, snapshots, ignored), ['app']);
    assert.deepEqual(collectBulkGitTargets(linked, snapshots, ignored), ['app/.worktrees/feat']);
  });

  it('skips hidden ignored repos on background fetch and includes them when shown', () => {
    const snapshots = [
      base({ repo: 'app' }),
      base({ repo: 'notes' }),
      base({
        repo: 'notes/.worktrees/feat',
        checkoutKind: 'linked',
        primaryRepo: 'notes',
      }),
    ];
    const ignored = new Set(['notes']);
    assert.deepEqual(collectBackgroundFetchTargets(snapshots, ignored, false), ['app']);
    assert.deepEqual(collectBackgroundFetchTargets(snapshots, ignored, true), [
      'app',
      'notes',
      'notes/.worktrees/feat',
    ]);
  });
});
