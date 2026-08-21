import assert from 'node:assert';
import { describe, it } from 'node:test';

import {
  actionVisibleForScope,
  canBranch,
  canDefaultBranch,
  canPull,
  canPush,
  canRemoveWorktree,
  canRevert,
  canStage,
  canStashPush,
  canToggleViewed,
  canUnstage,
  isRevertible,
  isTreeWriteBlockedAtDepth,
  TREE_WRITE_BLOCKED_IDS,
  treeWritesHiddenForContext,
  type ActionGateContext,
} from '../src/tui/actions/gates.js';
import { ACTIONS } from '../src/tui/actions/registry.js';
import { actionHintSegments } from '../src/tui/StatusBar.js';
import { buildTree } from '../src/tui/model/tree.js';
import { flatten } from '../src/tui/model/flatten.js';
import type { FileNode, VisibleRow } from '../src/tui/model/types.js';
import type { RepoSnapshot } from '../src/types.js';

function base(partial: Partial<RepoSnapshot> = {}): RepoSnapshot {
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
    change: { path: filePath, unstagedStatus: 'M' },
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

function spec(id: string) {
  const found = ACTIONS.find((a) => a.id === id);
  assert.ok(found, `missing action ${id}`);
  return found!;
}

function rowFor(snapshots: RepoSnapshot[], kind: 'workspace' | 'repo' | 'file'): VisibleRow {
  const tree = buildTree({
    snapshots,
    ignoredRepos: new Set(),
    treeMode: false,
    workspaceLabel: 'ws',
  });
  const rows = flatten(tree, new Set());
  const row = rows.find((r) => r.node.kind === kind);
  assert.ok(row, `missing ${kind} row`);
  return row!;
}

describe('isRevertible / stage-unstage-revert gates', () => {
  it('matches staged/unstaged/untracked eligibility', () => {
    assert.equal(isRevertible(fileNode({ unstaged: true })), true);
    assert.equal(isRevertible(fileNode({ unstaged: false, untracked: true, status: '?' })), true);
    assert.equal(
      isRevertible(fileNode({ staged: true, unstaged: false, untracked: false })),
      false,
    );

    const unstaged = fileRow(fileNode({ unstaged: true, staged: false }));
    assert.equal(canStage(unstaged), true);
    assert.equal(canUnstage(unstaged), false);
    assert.equal(canRevert(unstaged), true);

    const stagedOnly = fileRow(fileNode({ staged: true, unstaged: false, untracked: false }));
    assert.equal(canStage(stagedOnly), false);
    assert.equal(canUnstage(stagedOnly), true);
    assert.equal(canRevert(stagedOnly), false);
  });
});

describe('canStashPush / stashMenu scope gate', () => {
  it('is true when the scope has staged, unstaged, or untracked files', () => {
    assert.equal(canStashPush(fileRow(fileNode({ unstaged: true, staged: false }))), true);
    assert.equal(
      canStashPush(fileRow(fileNode({ staged: true, unstaged: false, untracked: false }))),
      true,
    );
    assert.equal(
      canStashPush(
        fileRow(
          fileNode({
            staged: false,
            unstaged: false,
            untracked: true,
            status: '?',
          }),
        ),
      ),
      true,
    );
    assert.equal(canStashPush(null), false);
  });

  it('hides stashMenu at depth 0 unless canStashPush, and does not hide it at depth 1', () => {
    const snaps = [
      base({
        repo: 'a',
        hasUnstaged: true,
        unstagedFiles: 'M\ta.ts',
      }),
    ];
    const focused = rowFor(snaps, 'file');
    assert.equal(
      actionVisibleForScope(spec('stashMenu'), {
        focused,
        snapshots: snaps,
        navDepth: 0,
      }),
      true,
    );
    assert.equal(
      actionVisibleForScope(spec('stashMenu'), {
        focused: null,
        snapshots: snaps,
        navDepth: 0,
      }),
      false,
    );
    assert.equal(
      actionVisibleForScope(spec('stashMenu'), {
        focused,
        snapshots: snaps,
        navDepth: 1,
      }),
      true,
    );
    assert.equal(
      actionVisibleForScope(spec('stashMenu'), {
        focused,
        snapshots: snaps,
        navDepth: 2,
      }),
      false,
    );
  });

  it('does not put stashMenu in TREE_WRITE_BLOCKED_IDS', () => {
    assert.equal(TREE_WRITE_BLOCKED_IDS.has('stashMenu'), false);
  });
});

describe('canPull / canPush / canDefaultBranch', () => {
  it('gates pull on behind sync status', () => {
    const behind = [base({ repo: 'a', syncStatus: 'behind' })];
    const current = [base({ repo: 'a', syncStatus: 'up-to-date' })];
    assert.equal(canPull(rowFor(behind, 'workspace'), behind), true);
    assert.equal(canPull(rowFor(current, 'workspace'), current), false);
    assert.equal(canPull(rowFor(behind, 'repo'), behind), true);
    assert.equal(canPull(rowFor(current, 'repo'), current), false);
  });

  it('gates push on ahead/diverged/no-upstream; never on workspace', () => {
    const ahead = [base({ repo: 'a', syncStatus: 'ahead' })];
    const diverged = [base({ repo: 'a', syncStatus: 'diverged' })];
    const noUpstream = [base({ repo: 'a', branch: 'feature/new', syncStatus: 'no-upstream' })];
    const behind = [base({ repo: 'a', syncStatus: 'behind' })];
    const current = [base({ repo: 'a', syncStatus: 'up-to-date' })];
    const detached = [base({ repo: 'a', branch: 'HEAD (detached)', syncStatus: 'no-upstream' })];
    assert.equal(canPush(rowFor(ahead, 'repo'), ahead), true);
    assert.equal(canPush(rowFor(diverged, 'repo'), diverged), true);
    assert.equal(canPush(rowFor(noUpstream, 'repo'), noUpstream), true);
    assert.equal(canPush(rowFor(behind, 'repo'), behind), false);
    assert.equal(canPush(rowFor(current, 'repo'), current), false);
    assert.equal(canPush(rowFor(detached, 'repo'), detached), false);
    assert.equal(canPush(rowFor(ahead, 'workspace'), ahead), false);

    const checkoutAhead: VisibleRow = {
      id: 'checkout:a',
      depth: 1,
      node: {
        kind: 'checkout',
        id: 'checkout:a',
        path: 'a',
        branch: 'main',
        checkoutKind: 'primary',
        mergedIntoDefault: null,
        sync: '',
        syncStatus: 'ahead',
        changeCount: 0,
        children: [],
      },
      label: 'a',
      segments: [],
      trailing: [],
    };
    const checkoutCurrent: VisibleRow = {
      id: 'checkout:a',
      depth: 1,
      node: {
        kind: 'checkout',
        id: 'checkout:a',
        path: 'a',
        branch: 'main',
        checkoutKind: 'primary',
        mergedIntoDefault: null,
        sync: '',
        syncStatus: 'up-to-date',
        changeCount: 0,
        children: [],
      },
      label: 'a',
      segments: [],
      trailing: [],
    };
    assert.equal(canPush(checkoutAhead, ahead), true);
    assert.equal(canPush(checkoutCurrent, current), false);
  });

  it('does not gate family/workspace push/pull/default-branch on sibling worktrees', () => {
    const snaps = [
      base({ repo: 'app', branch: 'main', syncStatus: 'up-to-date' }),
      base({
        repo: 'app/.worktrees/feat',
        branch: 'feature/x',
        syncStatus: 'ahead',
        checkoutKind: 'linked',
        primaryRepo: 'app',
      }),
    ];
    const tree = buildTree({
      snapshots: snaps,
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const container = rows.find((r) => r.node.kind === 'repo');
    const workspace = rows.find((r) => r.node.kind === 'workspace');
    const linked = rows.find((r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'linked');
    assert.ok(container);
    assert.ok(workspace);
    assert.ok(linked);
    assert.equal(canPush(container, snaps), false);
    assert.equal(canPush(linked, snaps), true);
    assert.equal(canPull(workspace, snaps), false);
    assert.equal(canDefaultBranch(workspace, snaps), false);
    assert.equal(canDefaultBranch(linked, snaps), true);

    const behindLinked = [
      base({ repo: 'app', branch: 'main', syncStatus: 'up-to-date' }),
      base({
        repo: 'app/.worktrees/feat',
        branch: 'feature/x',
        syncStatus: 'behind',
        checkoutKind: 'linked',
        primaryRepo: 'app',
      }),
    ];
    const behindRows = flatten(
      buildTree({
        snapshots: behindLinked,
        ignoredRepos: new Set(),
        treeMode: false,
        workspaceLabel: 'ws',
      }),
      new Set(),
    );
    const behindWorkspace = behindRows.find((r) => r.node.kind === 'workspace');
    const behindFamily = behindRows.find((r) => r.node.kind === 'repo');
    const behindLinkedRow = behindRows.find(
      (r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'linked',
    );
    assert.ok(behindWorkspace);
    assert.ok(behindFamily);
    assert.ok(behindLinkedRow);
    assert.equal(canPull(behindWorkspace, behindLinked), false);
    assert.equal(canPull(behindFamily, behindLinked), false);
    assert.equal(canPull(behindLinkedRow, behindLinked), true);

    const primaryAhead = [
      base({ repo: 'app', branch: 'main', syncStatus: 'ahead' }),
      base({
        repo: 'app/.worktrees/feat',
        branch: 'feature/x',
        syncStatus: 'up-to-date',
        checkoutKind: 'linked',
        primaryRepo: 'app',
      }),
    ];
    const aheadRows = flatten(
      buildTree({
        snapshots: primaryAhead,
        ignoredRepos: new Set(),
        treeMode: false,
        workspaceLabel: 'ws',
      }),
      new Set(),
    );
    const aheadFamily = aheadRows.find((r) => r.node.kind === 'repo');
    assert.ok(aheadFamily);
    assert.equal(canPush(aheadFamily, primaryAhead), true);
  });

  it('gates default-branch when already on default', () => {
    const onDefault = [base({ repo: 'a', branch: 'main' })];
    const onFeature = [base({ repo: 'a', branch: 'feature/x' })];
    assert.equal(canDefaultBranch(rowFor(onDefault, 'workspace'), onDefault), false);
    assert.equal(canDefaultBranch(rowFor(onFeature, 'workspace'), onFeature), true);
    assert.equal(canDefaultBranch(rowFor(onDefault, 'repo'), onDefault), false);
    assert.equal(canDefaultBranch(rowFor(onFeature, 'repo'), onFeature), true);
  });

  it('gates default-branch using defaultBranchOverride as the sole default', () => {
    const onMainWithDevelopOverride = [
      base({ repo: 'a', branch: 'main', defaultBranchOverride: 'develop' }),
    ];
    const onDevelopWithOverride = [
      base({ repo: 'a', branch: 'develop', defaultBranchOverride: 'develop' }),
    ];
    assert.equal(
      canDefaultBranch(rowFor(onMainWithDevelopOverride, 'repo'), onMainWithDevelopOverride),
      true,
    );
    assert.equal(
      canDefaultBranch(rowFor(onDevelopWithOverride, 'repo'), onDevelopWithOverride),
      false,
    );
    assert.equal(
      canDefaultBranch(rowFor(onMainWithDevelopOverride, 'workspace'), onMainWithDevelopOverride),
      true,
    );
  });
});

describe('canRemoveWorktree / canBranch on nested checkouts', () => {
  it('allows remove on linked checkout and flat linked repo; not family/primary', () => {
    const snaps = [
      base({
        repo: 'app',
        branch: 'main',
        hasUnstaged: true,
        unstagedFiles: 'M\ta.ts',
      }),
      base({
        repo: 'app/.worktrees/feat',
        branch: 'feature/x',
        checkoutKind: 'linked',
        primaryRepo: 'app',
        mergedIntoDefault: false,
        hasUnstaged: true,
        unstagedFiles: 'M\tb.ts',
      }),
    ];
    const tree = buildTree({
      snapshots: snaps,
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const container = rows.find((r) => r.node.kind === 'repo');
    const primary = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'primary',
    );
    const linked = rows.find((r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'linked');
    assert.ok(container);
    assert.ok(primary);
    assert.ok(linked);
    assert.equal(canRemoveWorktree(linked), true);
    assert.equal(canRemoveWorktree(primary), false);
    assert.equal(canRemoveWorktree(container), false);
    assert.equal(
      actionVisibleForScope(spec('removeWorktree'), {
        focused: linked,
        snapshots: snaps,
        navDepth: 0,
      }),
      true,
    );
    assert.equal(
      actionVisibleForScope(spec('removeWorktree'), {
        focused: primary,
        snapshots: snaps,
        navDepth: 0,
      }),
      false,
    );
    assert.equal(canBranch(linked), true);
    assert.equal(canBranch(primary), true);
    assert.equal(canBranch(container), false);

    const flatTree = buildTree({
      snapshots: [snaps[1]!],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const flatLinked = flatten(flatTree, new Set()).find(
      (r) => r.node.kind === 'repo' && r.node.checkoutKind === 'linked',
    );
    assert.ok(flatLinked);
    assert.equal(canRemoveWorktree(flatLinked), true);
  });
});

describe('depth write-block', () => {
  it('blocks tree write actions at depth ≥ 1', () => {
    assert.equal(isTreeWriteBlockedAtDepth(0), false);
    assert.equal(isTreeWriteBlockedAtDepth(1), true);
    assert.equal(isTreeWriteBlockedAtDepth(2), true);

    const snaps = [
      base({
        repo: 'a',
        branch: 'feature/x',
        syncStatus: 'behind',
        hasUnstaged: true,
        unstagedFiles: 'M\ta.ts',
      }),
    ];
    const focused = rowFor(snaps, 'file');
    const ctxDepth2: ActionGateContext = {
      focused,
      snapshots: snaps,
      navDepth: 2,
    };
    for (const id of [
      'stage',
      'unstage',
      'revert',
      'fetch',
      'pull',
      'push',
      'defaultBranch',
      'branch',
      'removeWorktree',
    ] as const) {
      assert.equal(
        actionVisibleForScope(spec(id), ctxDepth2),
        false,
        `${id} should hide at depth 2`,
      );
    }
    assert.equal(actionVisibleForScope(spec('edit'), ctxDepth2), true);
    assert.equal(actionVisibleForScope(spec('fullFile'), ctxDepth2), true);
  });

  it('treeWritesHiddenForContext covers depth ≥ 1 and any right-pane focus', () => {
    assert.equal(treeWritesHiddenForContext(0, 'left'), false);
    assert.equal(treeWritesHiddenForContext(0, 'right'), true);
    assert.equal(treeWritesHiddenForContext(1, 'left'), true);
    assert.equal(treeWritesHiddenForContext(1, 'right'), true);
    assert.equal(treeWritesHiddenForContext(2, 'right'), true);
  });
});

describe('actionHintSegments + scope gates', () => {
  it('omits stage/unstage/revert when scope has nothing for them', () => {
    const snaps = [
      base({
        repo: 'a',
        hasStaged: true,
        stagedFiles: 'M\ta.ts',
      }),
    ];
    const focused = rowFor(snaps, 'file');
    const scope: ActionGateContext = {
      focused,
      snapshots: snaps,
      navDepth: 0,
    };
    const keys = actionHintSegments('file', 0, 'left', null, scope).map((s) => s.key);
    const text = keys.join(' ');
    assert.ok(!keys.includes('s'), `unexpected stage: ${text}`);
    assert.ok(keys.includes('u'), `expected unstage: ${text}`);
    assert.ok(!keys.includes('x'), `unexpected revert: ${text}`);
    assert.ok(keys.includes('f'), `expected fetch: ${text}`);
    assert.ok(keys.includes('e'), `expected edit: ${text}`);
    assert.ok(keys.includes('space'), `expected space reviewed: ${text}`);
  });

  it('omits pull/default-branch on a clean default-branch workspace', () => {
    const snaps = [base({ repo: 'a', branch: 'main', syncStatus: 'up-to-date' })];
    const focused = rowFor(snaps, 'workspace');
    const scope: ActionGateContext = {
      focused,
      snapshots: snaps,
      navDepth: 0,
    };
    const keys = actionHintSegments('workspace', 0, 'left', null, scope).map((s) => s.key);
    assert.deepEqual(keys, ['f']);
  });

  it('lists fetch/pull/default on depth-0 left workspace when gates allow', () => {
    const snaps = [base({ repo: 'a', branch: 'feat', syncStatus: 'behind' })];
    const focused = rowFor(snaps, 'workspace');
    const scope: ActionGateContext = {
      focused,
      snapshots: snaps,
      navDepth: 0,
    };
    const keys = actionHintSegments('workspace', 0, 'left', null, scope).map((s) => s.key);
    assert.deepEqual(keys, ['f', 'p', 'd']);
  });

  it('hides file write hints at depth 2 even when the row looks stageable', () => {
    const snaps = [
      base({
        repo: 'a',
        hasUnstaged: true,
        unstagedFiles: 'M\ta.ts',
      }),
    ];
    const focused = rowFor(snaps, 'file');
    const scope: ActionGateContext = {
      focused,
      snapshots: snaps,
      navDepth: 2,
    };
    const keys = actionHintSegments('file', 2, 'left', null, scope).map((s) => s.key);
    assert.deepEqual(keys, ['e', 'ctrl+o']);
  });

  it('skips hidden ignored repos on workspace and focused pull/default-branch', () => {
    const snaps = [
      base({ repo: 'app', branch: 'main', syncStatus: 'up-to-date' }),
      base({ repo: 'notes', branch: 'feature/x', syncStatus: 'behind' }),
    ];
    const ignored = new Set(['notes']);
    const tree = buildTree({
      snapshots: snaps,
      ignoredRepos: ignored,
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const workspace = rows.find((r) => r.node.kind === 'workspace');
    const notes = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'notes');
    assert.ok(workspace);
    assert.ok(notes);
    assert.equal(canPull(workspace, snaps, ignored, false), false);
    assert.equal(canPull(notes, snaps, ignored, false), false);
    assert.equal(canDefaultBranch(workspace, snaps, ignored, false), false);
    assert.equal(canDefaultBranch(notes, snaps, ignored, false), false);
    assert.equal(
      actionVisibleForScope(spec('pull'), {
        focused: workspace,
        snapshots: snaps,
        navDepth: 0,
        ignoredRepos: ignored,
        showIgnored: false,
      }),
      false,
    );
    assert.equal(
      actionVisibleForScope(spec('pull'), {
        focused: notes,
        snapshots: snaps,
        navDepth: 0,
        ignoredRepos: ignored,
        showIgnored: false,
      }),
      false,
    );
  });

  it('includes shown ignored repos on workspace and focused pull/default-branch', () => {
    const snaps = [
      base({ repo: 'app', branch: 'main', syncStatus: 'up-to-date' }),
      base({ repo: 'notes', branch: 'feature/x', syncStatus: 'behind' }),
    ];
    const ignored = new Set(['notes']);
    const tree = buildTree({
      snapshots: snaps,
      ignoredRepos: ignored,
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, new Set());
    const workspace = rows.find((r) => r.node.kind === 'workspace');
    const notes = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'notes');
    assert.ok(workspace);
    assert.ok(notes);
    assert.equal(canPull(workspace, snaps, ignored, true), true);
    assert.equal(canPull(notes, snaps, ignored, true), true);
    assert.equal(canDefaultBranch(workspace, snaps, ignored, true), true);
    assert.equal(canDefaultBranch(notes, snaps, ignored, true), true);
    assert.equal(
      actionVisibleForScope(spec('pull'), {
        focused: workspace,
        snapshots: snaps,
        navDepth: 0,
        ignoredRepos: ignored,
        showIgnored: true,
      }),
      true,
    );
    assert.equal(
      actionVisibleForScope(spec('pull'), {
        focused: notes,
        snapshots: snaps,
        navDepth: 0,
        ignoredRepos: ignored,
        showIgnored: true,
      }),
      true,
    );
  });
});

describe('canToggleViewed', () => {
  it('accepts a dirty workspace-tree file at depth 0 only', () => {
    const dirty = fileRow(fileNode({ unstaged: true }));
    assert.equal(canToggleViewed(dirty, 0), true);
    assert.equal(canToggleViewed(dirty, 1), false);
    assert.equal(canToggleViewed(dirty, 2), false);
  });

  it('rejects clean files and non-file rows', () => {
    const clean = fileRow(
      fileNode({ staged: false, unstaged: false, untracked: false, change: { path: 'src/a.ts' } }),
    );
    assert.equal(canToggleViewed(clean, 0), false);
    const snaps = [base({ repo: 'demo' })];
    assert.equal(canToggleViewed(rowFor(snaps, 'repo'), 0), false);
    assert.equal(canToggleViewed(rowFor(snaps, 'workspace'), 0), false);
    assert.equal(canToggleViewed(null, 0), false);
  });
});
