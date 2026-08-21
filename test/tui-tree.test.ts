import assert from 'node:assert';
import { describe, it } from 'node:test';
import type { RepoSnapshot } from '../src/types.js';
import {
  applyFold,
  collectFoldableIds,
  collectFoldableSubtreeIds,
  createFoldState,
  unfoldAncestors,
} from '../src/tui/model/fold.js';
import { flatten } from '../src/tui/model/flatten.js';
import {
  buildTree,
  compareRepoPathsForDisplay,
  nodeSegments,
  showCleanCheck,
  snapshotsForView,
} from '../src/tui/model/tree.js';
import {
  ICON_BRANCH,
  ICON_CLEAN,
  ICON_LINKED_WORKTREE,
  ICON_MERGED_INTO_DEFAULT,
  ICON_OPEN_VS_DEFAULT,
  ICON_SYNCED,
} from '../src/tui/icons.js';
import { isDefaultBranch } from '../src/helpers.js';

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

const dirtyIgnored = base({
  repo: 'dotfiles',
  branch: 'feature/JBY-035-workspace-status-tui',
  hasUnstaged: true,
  unstagedFiles: 'M\tai/common/skills/my-workspace-status/src/index.ts',
});

const dirtyNormal = base({
  repo: 'app',
  branch: 'feature/ABC-1-thing',
  hasUnstaged: true,
  unstagedFiles: 'M\tsrc/main.ts|||M\tsrc/util/helpers.ts',
});

const cleanBehind = base({
  repo: 'notes',
  branch: 'main',
  syncStatus: 'behind',
  syncNote: 'behind by 3',
});

const cleanAhead = base({
  repo: 'lib',
  branch: 'main',
  syncStatus: 'ahead',
  syncNote: 'ahead by 2',
});

const cleanDiverged = base({
  repo: 'ops',
  branch: 'main',
  syncStatus: 'diverged',
  syncNote: 'ahead 1, behind 1',
});

describe('buildTree + fold + flatten', () => {
  it('renders workspace root label in natural case', () => {
    const tree = buildTree({
      snapshots: [dirtyNormal],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    assert.equal(tree.kind, 'workspace');
    if (tree.kind !== 'workspace') return;
    const segs = nodeSegments(tree, true).segments;
    const labelSeg = segs.find((s) => s.text === 'crescendo' || s.text === 'CRESCENDO');
    assert.ok(labelSeg, 'expected a workspace label segment');
    assert.equal(labelSeg.text, 'crescendo');
    assert.notEqual(labelSeg.text, 'CRESCENDO');
  });

  it('expands dirty repos; keeps ignored dirty repos collapsed (no file rows)', () => {
    const tree = buildTree({
      snapshots: [dirtyIgnored, dirtyNormal, cleanBehind],
      ignoredRepos: new Set(['dotfiles']),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    const folds = createFoldState(tree);
    const rows = flatten(tree, folds);

    assert.ok(rows.some((r) => r.node.kind === 'repo' && r.node.path === 'app'));
    assert.ok(rows.some((r) => r.node.kind === 'repo' && r.node.path === 'dotfiles'));
    assert.ok(rows.some((r) => r.node.kind === 'file' && r.id === 'file:app:src/main.ts'));
    assert.ok(!rows.some((r) => r.node.kind === 'file' && r.id.includes('dotfiles/')));
    assert.ok(folds.has('repo:dotfiles'));
    assert.ok(!folds.has('repo:app'));
  });

  it('places clean behind repo at top level outside No updates group', () => {
    const cleanCurrent = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [dirtyIgnored, dirtyNormal, cleanBehind, cleanCurrent],
      ignoredRepos: new Set(['dotfiles']),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    assert.equal(tree.kind, 'workspace');
    if (tree.kind !== 'workspace') return;

    const topRepos = tree.children
      .filter((c) => c.kind === 'repo')
      .map((c) => {
        assert.equal(c.kind, 'repo');
        return c.path;
      });
    assert.deepEqual(topRepos.sort(), ['app', 'dotfiles', 'notes']);

    const group = tree.children.find((c) => c.kind === 'group');
    assert.ok(group && group.kind === 'group');
    assert.equal(group.id, 'group:no-updates');
    assert.ok(group.children.every((c) => c.kind === 'repo' && c.path !== 'notes'));
    assert.ok(group.children.some((c) => c.kind === 'repo' && c.path === 'docs'));
    const labelText = nodeSegments(group, false)
      .segments.map((s) => s.text)
      .join('');
    assert.match(labelText, /No updates/);

    const folds = createFoldState(tree);
    assert.ok(folds.has('group:no-updates'));
    const rows = flatten(tree, folds);
    assert.ok(rows.some((r) => r.id === 'group:no-updates'));
    assert.ok(rows.some((r) => r.node.kind === 'repo' && r.node.path === 'notes'));
    assert.ok(!rows.some((r) => r.node.kind === 'repo' && r.node.path === 'docs'));
  });

  it('flat mode lists full paths; treeMode inserts dir nodes', () => {
    const flatTree = buildTree({
      snapshots: [dirtyNormal],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    const flatRows = flatten(flatTree, createFoldState(flatTree));
    const flatFile = flatRows.find((r) => r.id === 'file:app:src/util/helpers.ts');
    assert.ok(flatFile);
    // Flat mode: file name, then the dimmed containing directory.
    assert.match(flatFile.label, /helpers\.ts {2}src\/util/);
    assert.match(flatFile.label, /M$/);
    assert.ok(!flatRows.some((r) => r.node.kind === 'dir'));

    const nestedTree = buildTree({
      snapshots: [dirtyNormal],
      ignoredRepos: new Set(),
      treeMode: true,
      workspaceLabel: 'crescendo',
    });
    const nestedRows = flatten(nestedTree, createFoldState(nestedTree));
    assert.ok(nestedRows.some((r) => r.node.kind === 'dir' && r.node.path === 'src'));
    const nestedFile = nestedRows.find((r) => r.id === 'file:app:src/util/helpers.ts');
    assert.ok(nestedFile);
    assert.match(nestedFile.label, /helpers\.ts/);
    assert.match(nestedFile.label, /M$/);
    assert.ok(!nestedFile.label.includes('src/util/'));
  });

  it('repo labels carry branch name and sync mark without emoji', () => {
    const tree = buildTree({
      snapshots: [dirtyNormal, cleanBehind],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    const rows = flatten(tree, createFoldState(tree));
    const app = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'app');
    assert.ok(app);
    assert.match(app.label, /feature\/ABC-1-thing/);
    assert.doesNotMatch(app.label, /\*feature\//);
    assert.match(app.label, /  2$/);
    assert.ok(!app.label.includes(ICON_SYNCED), app.label);
    assert.ok(!app.label.includes(ICON_CLEAN), app.label);
    assert.ok(!/[\u{1F300}-\u{1FAFF}]/u.test(app.label));
  });

  it('sorts linked worktrees adjacent after their primary', () => {
    const primary = base({
      repo: 'rsps-api',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts',
    });
    const linkedOpen = base({
      repo: 'rsps-api/.worktrees/feat-open',
      branch: 'feature/open',
      checkoutKind: 'linked',
      primaryRepo: 'rsps-api',
      mergedIntoDefault: false,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/b.ts',
    });
    const linkedMerged = base({
      repo: 'rsps-api/.worktrees/feat-done',
      branch: 'feature/done',
      checkoutKind: 'linked',
      primaryRepo: 'rsps-api',
      mergedIntoDefault: true,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/c.ts',
    });
    const other = base({
      repo: 'tiger',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/d.ts',
    });
    assert.ok(compareRepoPathsForDisplay(primary, linkedOpen) < 0);
    assert.ok(compareRepoPathsForDisplay(linkedOpen, other) < 0);

    const tree = buildTree({
      snapshots: [other, linkedMerged, primary, linkedOpen],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const topRepos = tree.children
      .filter((c) => c.kind === 'repo')
      .map((c) => {
        assert.equal(c.kind, 'repo');
        return c.path;
      });
    assert.deepEqual(topRepos, ['rsps-api', 'tiger']);

    const family = tree.children.find((c) => c.kind === 'repo' && c.path === 'rsps-api');
    assert.ok(family && family.kind === 'repo');
    assert.ok(family.children.every((c) => c.kind === 'checkout'));
    assert.deepEqual(
      family.children.map((c) => {
        assert.equal(c.kind, 'checkout');
        return c.path;
      }),
      ['rsps-api', 'rsps-api/.worktrees/feat-done', 'rsps-api/.worktrees/feat-open'],
    );
  });

  it('nests linked worktrees under a family container; flat when alone', () => {
    const primary = base({
      repo: 'rsps-api',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts',
    });
    const linked = base({
      repo: 'rsps-api/.worktrees/feat',
      branch: 'feature/x',
      checkoutKind: 'linked',
      primaryRepo: 'rsps-api',
      mergedIntoDefault: false,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/b.ts',
    });
    const alone = base({
      repo: 'tiger',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/c.ts',
    });
    const nested = buildTree({
      snapshots: [primary, linked, alone],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const family = nested.children.find((c) => c.kind === 'repo' && c.path === 'rsps-api');
    assert.ok(family && family.kind === 'repo');
    assert.equal(family.branch, '');
    assert.equal(family.mergedIntoDefault, null);
    assert.equal(family.changeCount, 2);
    assert.match(
      nodeSegments(family, false)
        .trailing.map((s) => s.text)
        .join(''),
      /2 wt/,
    );
    assert.ok(family.children.every((c) => c.kind === 'checkout'));

    const flat = buildTree({
      snapshots: [alone],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const tiger = flat.children.find((c) => c.kind === 'repo' && c.path === 'tiger');
    assert.ok(tiger && tiger.kind === 'repo');
    assert.ok(tiger.children.every((c) => c.kind === 'file' || c.kind === 'dir'));
    assert.ok(!tiger.children.some((c) => c.kind === 'checkout'));
  });

  it('flat-renders linked-only snapshots (named filter) without a phantom primary container', () => {
    const linked = base({
      repo: 'app/.worktrees/feat',
      branch: 'feature/x',
      checkoutKind: 'linked',
      primaryRepo: 'app',
      mergedIntoDefault: false,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts',
    });
    const tree = buildTree({
      snapshots: [linked],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const topRepos = tree.children.filter((c) => c.kind === 'repo');
    assert.equal(topRepos.length, 1);
    const row = topRepos[0]!;
    assert.equal(row.kind, 'repo');
    assert.equal(row.path, 'app/.worktrees/feat');
    assert.equal(row.checkoutKind, 'linked');
    assert.equal(row.primaryRepo, 'app');
    assert.ok(!row.children.some((c) => c.kind === 'checkout'));
    assert.ok(!tree.children.some((c) => c.kind === 'repo' && c.path === 'app'));
  });

  it('linked detached checkout labels with short worktree path', () => {
    const primary = base({
      repo: 'app',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/p.ts',
    });
    const linked = base({
      repo: 'app/.worktrees/detached-feat',
      branch: 'HEAD (detached)',
      checkoutKind: 'linked',
      primaryRepo: 'app',
      mergedIntoDefault: null,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/x.ts',
    });
    const tree = buildTree({
      snapshots: [primary, linked],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, createFoldState(tree));
    const detachedRow = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.path === 'app/.worktrees/detached-feat',
    );
    assert.ok(detachedRow);
    assert.match(detachedRow.label, new RegExp(`^${ICON_LINKED_WORKTREE}`));
    assert.match(detachedRow.label, /\.worktrees\/detached-feat|detached-feat/);
    assert.ok(!detachedRow.label.includes('HEAD (detached)'));
  });

  it('linked checkout rows use linked icon, branch label, and merge mark', () => {
    const primary = base({
      repo: 'rsps-api',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/p.ts',
    });
    const linked = base({
      repo: 'rsps-api/.worktrees/NDRMD-1422',
      branch: 'feature/NDRMD-1422',
      checkoutKind: 'linked',
      primaryRepo: 'rsps-api',
      mergedIntoDefault: false,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/x.ts',
    });
    const linkedMerged = base({
      repo: 'app/.worktrees/done',
      branch: 'feature/done',
      checkoutKind: 'linked',
      primaryRepo: 'app',
      mergedIntoDefault: true,
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/y.ts',
    });
    const appPrimary = base({
      repo: 'app',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/z.ts',
    });
    const tree = buildTree({
      snapshots: [linked, linkedMerged, primary, appPrimary],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, createFoldState(tree));
    const openRow = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.path === 'rsps-api/.worktrees/NDRMD-1422',
    );
    const mergedRow = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.path === 'app/.worktrees/done',
    );
    const primaryRow = rows.find(
      (r) =>
        r.node.kind === 'checkout' &&
        r.node.path === 'rsps-api' &&
        r.node.checkoutKind === 'primary',
    );
    assert.ok(openRow);
    assert.ok(mergedRow);
    assert.ok(primaryRow);
    assert.match(openRow.label, new RegExp(`^${ICON_LINKED_WORKTREE}`));
    assert.match(openRow.label, new RegExp(`feature\\/NDRMD-1422 ${ICON_OPEN_VS_DEFAULT}`));
    assert.match(mergedRow.label, new RegExp(`feature\\/done ${ICON_MERGED_INTO_DEFAULT}`));
    assert.doesNotMatch(openRow.label, /\*feature\//);
    assert.doesNotMatch(mergedRow.label, /\*feature\//);
    assert.match(primaryRow.label, new RegExp(`^${ICON_BRANCH}`));
    assert.ok(!/[\u{1F300}-\u{1FAFF}]/u.test(openRow.label));
    assert.ok(!/[\u{1F300}-\u{1FAFF}]/u.test(mergedRow.label));
  });

  it('applyFold supports toggle, openAll, and closeAll', () => {
    const cleanCurrent = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [dirtyIgnored, dirtyNormal, cleanBehind, cleanCurrent],
      ignoredRepos: new Set(['dotfiles']),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    const folds = createFoldState(tree);
    assert.ok(folds.has('repo:dotfiles'));

    const openedDotfiles = applyFold(folds, 'toggle', 'repo:dotfiles');
    assert.ok(!openedDotfiles.has('repo:dotfiles'));

    const collapsedAgain = applyFold(openedDotfiles, 'toggle', 'repo:dotfiles');
    assert.ok(collapsedAgain.has('repo:dotfiles'));

    const allOpen = applyFold(collapsedAgain, 'openAll', 'repo:app');
    assert.equal(allOpen.size, 0);

    const foldable = collectFoldableIds(tree);
    const allClosed = applyFold(allOpen, 'closeAll', 'repo:app', foldable);
    assert.ok(allClosed.has('repo:app'));
    assert.ok(allClosed.has('repo:dotfiles'));
    assert.ok(allClosed.has('group:no-updates'));
    assert.deepEqual([...allClosed].sort(), [...foldable].sort());
  });

  it('unfoldAncestors opens folded parents so a relocated id is visible', () => {
    const cleanCurrent = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [dirtyNormal, cleanCurrent],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    const folds = createFoldState(tree);
    assert.ok(folds.has('group:no-updates'));
    const hidden = flatten(tree, folds);
    assert.ok(!hidden.some((r) => r.id === 'repo:docs'));

    const revealed = unfoldAncestors(tree, folds, 'repo:docs');
    assert.ok(!revealed.has('group:no-updates'));
    const visible = flatten(tree, revealed);
    assert.ok(visible.some((r) => r.id === 'repo:docs'));
    // Missing id is a no-op (same Set).
    assert.equal(unfoldAncestors(tree, folds, 'repo:gone'), folds);
  });
});

describe('toggleSubtree', () => {
  it('collects the focus id and foldable descendants', () => {
    const tree = buildTree({
      snapshots: [dirtyNormal],
      ignoredRepos: new Set(),
      treeMode: true,
      workspaceLabel: 'ws',
    });
    const ids = collectFoldableSubtreeIds(tree, 'repo:app');
    assert.ok(ids.includes('repo:app'));
    assert.ok(ids.every((id) => !id.startsWith('file:')));
  });

  it('returns [] for a file id', () => {
    const tree = buildTree({
      snapshots: [dirtyNormal],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const file = flatten(tree, new Set()).find((r) => r.node.kind === 'file');
    assert.ok(file);
    const ids = collectFoldableSubtreeIds(tree, file.id);
    assert.deepEqual(ids, []);
    // applyFold throws on []; useAppState must no-op before calling (file double-tap).
    assert.throws(() => applyFold(new Set(), 'toggleSubtree', file.id, ids));
  });

  it('opens the whole subtree when focus is folded, else closes it', () => {
    const tree = buildTree({
      snapshots: [dirtyIgnored, dirtyNormal, cleanBehind],
      ignoredRepos: new Set(['dotfiles']),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    const folds = createFoldState(tree);
    assert.ok(folds.has('repo:dotfiles'));
    const subtree = collectFoldableSubtreeIds(tree, 'repo:dotfiles');
    const opened = applyFold(folds, 'toggleSubtree', 'repo:dotfiles', subtree);
    assert.ok(!opened.has('repo:dotfiles'));
    const closed = applyFold(opened, 'toggleSubtree', 'repo:dotfiles', subtree);
    for (const id of subtree) assert.ok(closed.has(id));
  });
});

describe('status semantics — off-default / behind / ahead is not idle', () => {
  it('keeps a clean feature-branch repo at the top level', () => {
    const cleanFeature = base({
      repo: 'checkout-service',
      branch: 'feature/ABCD-1234-add-new-feature',
    });
    const cleanCurrent = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [cleanFeature, cleanBehind, cleanCurrent],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.kind, 'workspace');
    const topRepos = tree.children
      .filter((c) => c.kind === 'repo')
      .map((c) => (c.kind === 'repo' ? c.path : ''));
    assert.ok(topRepos.includes('checkout-service'));
    assert.ok(topRepos.includes('notes'));
    const group = tree.children.find((c) => c.kind === 'group');
    assert.ok(group && group.kind === 'group');
    assert.equal(group.id, 'group:no-updates');
    assert.ok(group.children.every((c) => c.kind === 'repo' && isDefaultBranch(c.branch)));
    assert.ok(group.children.some((c) => c.kind === 'repo' && c.path === 'docs'));
    assert.ok(group.children.every((c) => c.kind === 'repo' && c.path !== 'notes'));
  });

  it('still folds a clean up-to-date default-branch repo under No updates', () => {
    const cleanCurrent = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [cleanCurrent],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.children.filter((c) => c.kind === 'repo').length, 0);
    const group = tree.children.find((c) => c.kind === 'group');
    assert.ok(group && group.kind === 'group');
    assert.equal(group.id, 'group:no-updates');
    assert.equal(group.children.length, 1);
    const labelText = nodeSegments(group, false)
      .segments.map((s) => s.text)
      .join('');
    assert.match(labelText, /No updates/);
  });

  it('keeps clean main top-level when defaultBranchOverride is develop', () => {
    const onMain = base({
      repo: 'opella-main',
      branch: 'main',
      defaultBranchOverride: 'develop',
    });
    const onDevelop = base({
      repo: 'opl-frontend',
      branch: 'develop',
      defaultBranchOverride: 'develop',
    });
    const tree = buildTree({
      snapshots: [onMain, onDevelop],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const topRepos = tree.children
      .filter((c) => c.kind === 'repo')
      .map((c) => (c.kind === 'repo' ? c.path : ''));
    assert.deepEqual(topRepos, ['opella-main']);
    const group = tree.children.find((c) => c.kind === 'group');
    assert.ok(group && group.kind === 'group');
    assert.equal(group.children.length, 1);
    assert.ok(group.children[0]?.kind === 'repo' && group.children[0].path === 'opl-frontend');
    assert.ok(
      group.children[0]?.kind === 'repo' && group.children[0].defaultBranchOverride === 'develop',
    );
  });

  it('keeps a behind-only default-branch repo at the top level', () => {
    const tree = buildTree({
      snapshots: [cleanBehind],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.children.filter((c) => c.kind === 'repo').length, 1);
    assert.ok(tree.children.some((c) => c.kind === 'repo' && c.path === 'notes'));
    assert.ok(!tree.children.some((c) => c.kind === 'group'));
  });

  it('places clean ahead repo at top level outside No updates group', () => {
    const cleanCurrent = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [dirtyIgnored, dirtyNormal, cleanAhead, cleanCurrent],
      ignoredRepos: new Set(['dotfiles']),
      treeMode: false,
      workspaceLabel: 'crescendo',
    });
    assert.equal(tree.kind, 'workspace');
    if (tree.kind !== 'workspace') return;

    const topRepos = tree.children
      .filter((c) => c.kind === 'repo')
      .map((c) => {
        assert.equal(c.kind, 'repo');
        return c.path;
      });
    assert.deepEqual(topRepos.sort(), ['app', 'dotfiles', 'lib']);

    const group = tree.children.find((c) => c.kind === 'group');
    assert.ok(group && group.kind === 'group');
    assert.equal(group.id, 'group:no-updates');
    assert.ok(group.children.every((c) => c.kind === 'repo' && c.path !== 'lib'));
    assert.ok(group.children.some((c) => c.kind === 'repo' && c.path === 'docs'));
    const labelText = nodeSegments(group, false)
      .segments.map((s) => s.text)
      .join('');
    assert.match(labelText, /No updates/);

    const folds = createFoldState(tree);
    assert.ok(folds.has('group:no-updates'));
    const rows = flatten(tree, folds);
    assert.ok(rows.some((r) => r.id === 'group:no-updates'));
    assert.ok(rows.some((r) => r.node.kind === 'repo' && r.node.path === 'lib'));
    assert.ok(!rows.some((r) => r.node.kind === 'repo' && r.node.path === 'docs'));
  });

  it('keeps an ahead-only default-branch repo at the top level', () => {
    const tree = buildTree({
      snapshots: [cleanAhead],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.children.filter((c) => c.kind === 'repo').length, 1);
    assert.ok(tree.children.some((c) => c.kind === 'repo' && c.path === 'lib'));
    assert.ok(!tree.children.some((c) => c.kind === 'group'));
  });

  it('keeps a diverged-only default-branch repo at the top level', () => {
    const tree = buildTree({
      snapshots: [cleanDiverged],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.children.filter((c) => c.kind === 'repo').length, 1);
    assert.ok(tree.children.some((c) => c.kind === 'repo' && c.path === 'ops'));
    assert.ok(!tree.children.some((c) => c.kind === 'group'));
  });

  it('keeps an unborn default-branch repo at the top level (not under No updates)', () => {
    const unbornMain = base({
      repo: 'scratch',
      branch: 'main',
      syncStatus: 'no-upstream',
      syncNote: 'no commits yet',
    });
    const cleanCurrent = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [unbornMain, cleanCurrent],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.kind, 'workspace');
    if (tree.kind !== 'workspace') return;
    assert.ok(tree.children.some((c) => c.kind === 'repo' && c.path === 'scratch'));
    const group = tree.children.find((c) => c.kind === 'group');
    assert.ok(group && group.kind === 'group');
    assert.equal(group.id, 'group:no-updates');
    assert.ok(group.children.every((c) => c.kind === 'repo' && c.path !== 'scratch'));
    assert.ok(group.children.some((c) => c.kind === 'repo' && c.path === 'docs'));
    assert.equal(tree.syncSummary, '1 attention');
  });

  it('keeps a status-failed repo at the top level and counts it in syncSummary', () => {
    const failed = base({
      repo: 'broken',
      branch: '(unknown)',
      syncStatus: 'no-upstream',
      syncNote: 'status failed',
    });
    const tree = buildTree({
      snapshots: [failed],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.kind, 'workspace');
    if (tree.kind !== 'workspace') return;
    assert.ok(tree.children.some((c) => c.kind === 'repo' && c.path === 'broken'));
    assert.ok(!tree.children.some((c) => c.kind === 'group'));
    assert.equal(tree.syncSummary, '1 attention');
  });

  it('reports no repos in syncSummary when discovery is empty (not all current)', () => {
    const tree = buildTree({
      snapshots: [],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    assert.equal(tree.kind, 'workspace');
    if (tree.kind !== 'workspace') return;
    assert.equal(tree.children.length, 0);
    assert.equal(tree.changeCount, 0);
    assert.equal(tree.syncSummary, 'no repos');
    const trailing = nodeSegments(tree, false)
      .trailing.map((s) => s.text)
      .join('');
    assert.match(trailing, /no repos/);
    assert.doesNotMatch(trailing, /all current/);
  });
});

describe('snapshotsForView', () => {
  const ignored = base({ repo: 'notes', hasUnstaged: true, unstagedFiles: 'M\tREADME.md' });
  const visible = base({ repo: 'dotfiles', hasUnstaged: true, unstagedFiles: 'M\tsrc/a.ts' });
  const linked = base({
    repo: 'notes/.worktrees/wip',
    primaryRepo: 'notes',
    checkoutKind: 'linked',
  });

  it('hides ignored primaries and their linked checkouts when showIgnored is false', () => {
    const next = snapshotsForView([ignored, visible, linked], new Set(['notes']), false);
    assert.deepEqual(
      next.map((s) => s.repo),
      ['dotfiles'],
    );
  });

  it('keeps ignored repos when showIgnored is true', () => {
    const next = snapshotsForView([ignored, visible, linked], new Set(['notes']), true);
    assert.deepEqual(
      next.map((s) => s.repo),
      ['notes', 'dotfiles', 'notes/.worktrees/wip'],
    );
  });

  it('keeps named filter repos even when they are ignored', () => {
    const next = snapshotsForView(
      [ignored, visible, linked],
      new Set(['notes']),
      false,
      new Set(['notes']),
    );
    assert.deepEqual(
      next.map((s) => s.repo),
      ['notes', 'dotfiles', 'notes/.worktrees/wip'],
    );
  });
});

describe('clean-check gating', () => {
  it('showCleanCheck is true only inside No updates', () => {
    assert.equal(showCleanCheck(false), false);
    assert.equal(showCleanCheck(true), true);
  });

  it('paints ICON_CLEAN on clean repos inside No updates only', () => {
    const dirty = base({
      repo: 'app',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/main.ts',
    });
    const clean = base({
      repo: 'docs',
      branch: 'main',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [dirty, clean],
      ignoredRepos: new Set(),
      treeMode: true,
      workspaceLabel: 'ws',
    });
    const folds = createFoldState(tree);
    folds.delete('group:no-updates');
    const rows = flatten(tree, folds);

    const app = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'app');
    const docs = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'docs');
    const group = rows.find((r) => r.id === 'group:no-updates');
    const dir = rows.find((r) => r.node.kind === 'dir');
    assert.ok(app && docs && group && dir);

    assert.ok(!app.label.includes(ICON_CLEAN), app.label);
    assert.ok(!app.label.includes(ICON_SYNCED), app.label);
    assert.ok(docs.label.includes(ICON_CLEAN), docs.label);
    assert.ok(group.label.includes(ICON_CLEAN), group.label);
    assert.ok(!dir.label.includes(ICON_CLEAN), dir.label);
    assert.ok(!dir.label.includes(ICON_SYNCED), dir.label);
  });

  it('omits the clean check on a clean feature-branch repo outside No updates', () => {
    const cleanFeature = base({
      repo: 'checkout-service',
      branch: 'feature/ABCD-1',
      syncStatus: 'up-to-date',
    });
    const tree = buildTree({
      snapshots: [cleanFeature],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const rows = flatten(tree, createFoldState(tree));
    const repo = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'checkout-service');
    assert.ok(repo);
    assert.ok(!rows.some((r) => r.id === 'group:no-updates'));
    assert.ok(!repo.label.includes(ICON_CLEAN), repo.label);
    assert.ok(!repo.label.includes(ICON_SYNCED), repo.label);
  });

  it('omits the clean check on dirty family / checkout / worktree rows', () => {
    const primary = base({
      repo: 'rsps-api',
      branch: 'main',
      hasUnstaged: true,
      unstagedFiles: 'M\tsrc/a.ts',
    });
    const linked = base({
      repo: 'rsps-api/.worktrees/feat',
      branch: 'feature/x',
      checkoutKind: 'linked',
      primaryRepo: 'rsps-api',
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
    const rows = flatten(tree, createFoldState(tree));
    const family = rows.find((r) => r.node.kind === 'repo' && r.node.path === 'rsps-api');
    const checkouts = rows.filter((r) => r.node.kind === 'checkout');
    assert.ok(family);
    assert.ok(checkouts.length >= 2);
    assert.ok(!family.label.includes(ICON_CLEAN), family.label);
    for (const row of checkouts) {
      assert.ok(!row.label.includes(ICON_CLEAN), row.label);
    }
  });
});
