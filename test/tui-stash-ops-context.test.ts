import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import type { GraphActionRow } from '../src/tui/graph/actions.js';
import type {
  CheckoutNode,
  DirNode,
  FileNode,
  RepoNode,
  VisibleRow,
} from '../src/tui/model/types.js';
import {
  buildStashOpsContext,
  resolveStashMenuKey,
  stashMenuOpDetail,
  stashMenuSubtitle,
  stashOpsForContext,
  stashPushStatus,
  stashRepoRelPath,
} from '../src/tui/stashOps.js';

function fileNode(
  partial: Partial<FileNode> & Pick<FileNode, 'path'>,
): FileNode {
  return {
    kind: 'file',
    id: `file:repo:${partial.path}`,
    repoPath: 'repo',
    status: 'M',
    staged: false,
    unstaged: true,
    untracked: false,
    change: { path: partial.path, unstagedStatus: 'M' },
    ...partial,
  };
}

function visible(node: VisibleRow['node']): VisibleRow {
  return {
    id: node.id,
    depth: 1,
    node,
    label: 'label',
    segments: [],
    trailing: [],
  };
}

const dirtyA = fileNode({ path: 'src/a.ts' });
const dirtyB = fileNode({ path: 'src/b.ts' });
const cleanC = fileNode({
  path: 'src/c.ts',
  unstaged: false,
  status: 'S',
  staged: true,
  change: { path: 'src/c.ts', stagedStatus: 'M' },
});

const dirNode: DirNode = {
  kind: 'dir',
  id: 'dir:repo:src',
  path: 'src',
  name: 'src',
  repoPath: 'repo',
  children: [dirtyA, dirtyB, cleanC],
};

const repoNode: RepoNode = {
  kind: 'repo',
  id: 'repo:repo',
  path: 'repo',
  branch: 'main',
  checkoutKind: 'primary',
  mergedIntoDefault: null,
  sync: '',
  syncStatus: 'up-to-date',
  ignored: false,
  changeCount: 2,
  children: [dirNode],
};

const checkoutNode: CheckoutNode = {
  kind: 'checkout',
  id: 'checkout:repo',
  path: 'repo',
  branch: 'feat',
  checkoutKind: 'primary',
  mergedIntoDefault: null,
  sync: '',
  syncStatus: 'up-to-date',
  changeCount: 2,
  children: [dirNode],
};

const stashRow: GraphActionRow = {
  kind: 'stash',
  stash: {
    id: 's',
    stashRef: 'stash@{0}',
    index: 0,
    subject: 'wip',
    authorName: 'A',
    authorDateUnix: 1,
    parentId: '',
  },
};

const commitRow: GraphActionRow = {
  kind: 'commit',
  commit: {
    id: 'abc',
    parents: [],
    subject: 's',
    authorName: 'A',
    authorDateUnix: 1,
    refs: [],
  },
};

describe('buildStashOpsContext — tree', () => {
  it('dirty file yields push with that path', () => {
    const ctx = buildStashOpsContext({
      navDepth: 0,
      focusPane: 'left',
      focused: visible(dirtyA),
      graphRow: null,
      graphDirty: false,
    });
    assert.ok(ctx);
    assert.deepEqual(stashOpsForContext(ctx), [
      { id: 'push', key: 's', label: 'stash', paths: ['src/a.ts'] },
    ]);
  });

  it('dirty dir yields push with dirty child paths only', () => {
    const ctx = buildStashOpsContext({
      navDepth: 0,
      focusPane: 'left',
      focused: visible(dirNode),
      graphRow: null,
      graphDirty: false,
    });
    assert.ok(ctx);
    const ops = stashOpsForContext(ctx);
    assert.equal(ops.length, 1);
    assert.equal(ops[0]!.id, 'push');
    assert.deepEqual(ops[0]!.paths, ['src/a.ts', 'src/b.ts', 'src/c.ts']);
  });

  it('dirty repo/checkout omit paths (whole tree)', () => {
    for (const node of [repoNode, checkoutNode]) {
      const ctx = buildStashOpsContext({
        navDepth: 0,
        focusPane: 'left',
        focused: visible(node),
        graphRow: null,
        graphDirty: false,
      });
      assert.ok(ctx, node.kind);
      const ops = stashOpsForContext(ctx);
      assert.deepEqual(ops, [{ id: 'push', key: 's', label: 'stash' }]);
      assert.equal('paths' in ops[0]!, false);
    }
  });

  it('workspace row yields no ops', () => {
    const ctx = buildStashOpsContext({
      navDepth: 0,
      focusPane: 'left',
      focused: visible({
        kind: 'workspace',
        id: 'workspace',
        label: 'ws',
        changeCount: 0,
        syncSummary: '',
        children: [],
      }),
      graphRow: null,
      graphDirty: false,
    });
    assert.ok(ctx);
    assert.deepEqual(stashOpsForContext(ctx), []);
  });
});

describe('buildStashOpsContext — graph', () => {
  it('stash row yields apply, pop, and drop', () => {
    const ctx = buildStashOpsContext({
      navDepth: 1,
      focusPane: 'left',
      focused: null,
      graphRow: stashRow,
      graphDirty: false,
      latestStashRef: 'stash@{0}',
    });
    assert.ok(ctx);
    assert.deepEqual(
      stashOpsForContext(ctx).map((op) => op.id),
      ['apply', 'pop', 'drop'],
    );
  });

  it('dirty commit with latest stash yields push, apply, pop (no drop)', () => {
    const ctx = buildStashOpsContext({
      navDepth: 1,
      focusPane: 'left',
      focused: null,
      graphRow: commitRow,
      graphDirty: true,
      latestStashRef: 'stash@{0}',
    });
    assert.ok(ctx);
    assert.deepEqual(
      stashOpsForContext(ctx).map((op) => op.id),
      ['push', 'apply', 'pop'],
    );
  });

  it('clean commit with no stashes yields no ops', () => {
    const ctx = buildStashOpsContext({
      navDepth: 1,
      focusPane: 'left',
      focused: null,
      graphRow: commitRow,
      graphDirty: false,
    });
    assert.ok(ctx);
    assert.deepEqual(stashOpsForContext(ctx), []);
  });

  it('uncommitted dirty yields push', () => {
    const ctx = buildStashOpsContext({
      navDepth: 1,
      focusPane: 'left',
      focused: null,
      graphRow: { kind: 'uncommitted' },
      graphDirty: true,
    });
    assert.ok(ctx);
    assert.deepEqual(stashOpsForContext(ctx), [
      { id: 'push', key: 's', label: 'stash' },
    ]);
  });

  it('returns null on the right pane or at depth 2', () => {
    assert.equal(
      buildStashOpsContext({
        navDepth: 1,
        focusPane: 'right',
        focused: visible(dirtyA),
        graphRow: stashRow,
        graphDirty: true,
        latestStashRef: 'stash@{0}',
      }),
      null,
    );
    assert.equal(
      buildStashOpsContext({
        navDepth: 2,
        focusPane: 'left',
        focused: visible(dirtyA),
        graphRow: stashRow,
        graphDirty: true,
      }),
      null,
    );
  });
});

describe('stashRepoRelPath', () => {
  it('reads repoPath from file/dir and path from repo/checkout', () => {
    assert.equal(stashRepoRelPath(visible(dirtyA)), 'repo');
    assert.equal(stashRepoRelPath(visible(dirNode)), 'repo');
    assert.equal(stashRepoRelPath(visible(repoNode)), 'repo');
    assert.equal(stashRepoRelPath(visible(checkoutNode)), 'repo');
    assert.equal(
      stashRepoRelPath(
        visible({
          kind: 'workspace',
          id: 'workspace',
          label: 'ws',
          changeCount: 0,
          syncSummary: '',
          children: [],
        }),
      ),
      null,
    );
  });
});

describe('resolveStashMenuKey', () => {
  const ops = stashOpsForContext({
    kind: 'graphStash',
    dirty: true,
    focusedStashRef: 'stash@{0}',
  });

  it('maps s/a/p/d to the matching op', () => {
    assert.deepEqual(resolveStashMenuKey('s', {}, ops), {
      type: 'run',
      op: ops[0],
    });
    assert.deepEqual(resolveStashMenuKey('a', {}, ops), {
      type: 'run',
      op: ops[1],
    });
    assert.deepEqual(resolveStashMenuKey('p', {}, ops), {
      type: 'run',
      op: ops[2],
    });
    assert.deepEqual(resolveStashMenuKey('d', {}, ops), {
      type: 'run',
      op: ops[3],
    });
  });

  it('Enter runs the first op and Esc cancels', () => {
    assert.deepEqual(resolveStashMenuKey('', { return: true }, ops), {
      type: 'run',
      op: ops[0],
    });
    assert.deepEqual(resolveStashMenuKey('', { escape: true }, ops), {
      type: 'cancel',
    });
  });

  it('ignores unknown keys and missing ops', () => {
    assert.deepEqual(resolveStashMenuKey('x', {}, ops), { type: 'ignore' });
    assert.deepEqual(resolveStashMenuKey('S', {}, ops), { type: 'ignore' });
    assert.deepEqual(
      resolveStashMenuKey('d', {}, [{ id: 'push', key: 's', label: 'stash' }]),
      { type: 'ignore' },
    );
  });
});

describe('stash overlay copy', () => {
  it('status is Stashed or Stashed n file(s) when paths are used', () => {
    assert.equal(stashPushStatus(), 'Stashed');
    assert.equal(stashPushStatus(['a.ts']), 'Stashed 1 file');
    assert.equal(stashPushStatus(['a.ts', 'b.ts']), 'Stashed 2 files');
  });

  it('subtitle prefers focused stash ref over repo path', () => {
    assert.equal(
      stashMenuSubtitle({ focusedStashRef: 'stash@{1}', repoPath: 'repo' }),
      'stash@{1}',
    );
    assert.equal(stashMenuSubtitle({ repoPath: 'repo' }), 'repo');
  });

  it('op detail is the stash ref for apply/pop/drop only', () => {
    assert.equal(stashMenuOpDetail({ id: 'push', key: 's', label: 'stash' }), undefined);
    assert.equal(
      stashMenuOpDetail({
        id: 'apply',
        key: 'a',
        label: 'apply stash',
        stashRef: 'stash@{0}',
      }),
      'stash@{0}',
    );
    assert.equal(
      stashMenuOpDetail({
        id: 'pop',
        key: 'p',
        label: 'pop stash',
        stashRef: 'stash@{2}',
      }),
      'stash@{2}',
    );
    assert.equal(
      stashMenuOpDetail({
        id: 'drop',
        key: 'd',
        label: 'drop stash',
        stashRef: 'stash@{0}',
      }),
      'stash@{0}',
    );
  });
});

describe('useAppState wires stashPop', () => {
  it('has a runAction case for stashPop', () => {
    const src = fs.readFileSync(
      path.join(path.dirname(fileURLToPath(import.meta.url)), '../src/tui/useAppState.ts'),
      'utf8',
    );
    assert.match(src, /case 'stashPop':/);
  });
});
