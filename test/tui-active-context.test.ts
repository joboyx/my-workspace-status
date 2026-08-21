import assert from 'node:assert';
import { describe, it } from 'node:test';

import {
  activeRowKind,
  easyMotionListTarget,
  fileWritesOnDiffAllowed,
  foldAllowed,
  fullContextToggleId,
  rightPaneLeftListAllowed,
} from '../src/tui/activeContext.js';
import { listFocusTarget } from '../src/tui/graph/focus.js';
import type { RowKind } from '../src/tui/actions/registry.js';
import { isLeftListAction, type Action } from '../src/tui/keys.js';

type Depth = 0 | 1 | 2;
type Pane = 'left' | 'right';

type KindCase = {
  name: string;
  depth: Depth;
  focusPane: Pane;
  graphVisible: boolean;
  treeKind: RowKind | null;
  graphKind: RowKind | null;
  commitFileKind: RowKind | null;
  expected: RowKind;
};

const CASES: KindCase[] = [
  {
    name: 'depth 0 left repo → tree repo (graph visible)',
    depth: 0,
    focusPane: 'left',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'repo',
  },
  {
    name: 'depth 0 left file → tree file (graph hidden)',
    depth: 0,
    focusPane: 'left',
    graphVisible: false,
    treeKind: 'file',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'depth 0 right + graph visible → graphCommit (tree is repo)',
    depth: 0,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'graphCommit',
  },
  {
    name: 'depth 0 right + graph visible → graphStash (tree is file)',
    depth: 0,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'file',
    graphKind: 'graphStash',
    commitFileKind: 'file',
    expected: 'graphStash',
  },
  {
    name: 'depth 0 right + graph visible → graphUncommitted',
    depth: 0,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphUncommitted',
    commitFileKind: 'file',
    expected: 'graphUncommitted',
  },
  {
    name: 'depth 0 right + graph hidden + tree file → tree file (diff)',
    depth: 0,
    focusPane: 'right',
    graphVisible: false,
    treeKind: 'file',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'depth 0 right + graph hidden + tree repo → tree repo (diff)',
    depth: 0,
    focusPane: 'right',
    graphVisible: false,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'repo',
  },
  {
    name: 'depth 1 left + graph visible → graphCommit',
    depth: 1,
    focusPane: 'left',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'graphCommit',
  },
  {
    name: 'depth 1 left + graph hidden + tree file → tree file (diff)',
    depth: 1,
    focusPane: 'left',
    graphVisible: false,
    treeKind: 'file',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'depth 1 left + graph hidden + tree repo → tree repo (diff)',
    depth: 1,
    focusPane: 'left',
    graphVisible: false,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'repo',
  },
  {
    name: 'depth 1 right + graph visible + commit file → commit file',
    depth: 1,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'depth 1 right + graph hidden + commit dir → commit dir',
    depth: 1,
    focusPane: 'right',
    graphVisible: false,
    treeKind: 'file',
    graphKind: 'graphCommit',
    commitFileKind: 'dir',
    expected: 'dir',
  },
  {
    name: 'depth 2 left + commit file → commit file',
    depth: 2,
    focusPane: 'left',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'depth 2 left + graph hidden + commit file → commit file',
    depth: 2,
    focusPane: 'left',
    graphVisible: false,
    treeKind: 'file',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'depth 2 right + commit file → commit file (diff)',
    depth: 2,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'depth 2 right + graph hidden + commit file → commit file (diff)',
    depth: 2,
    focusPane: 'right',
    graphVisible: false,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'file',
  },
  {
    name: 'tree target with null treeKind falls back to workspace',
    depth: 0,
    focusPane: 'left',
    graphVisible: true,
    treeKind: null,
    graphKind: 'graphCommit',
    commitFileKind: 'file',
    expected: 'workspace',
  },
  {
    name: 'graph target with null graphKind falls back to workspace',
    depth: 0,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: null,
    commitFileKind: 'file',
    expected: 'workspace',
  },
  {
    name: 'commitFiles target with null commitFileKind falls back to workspace',
    depth: 1,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'repo',
    graphKind: 'graphCommit',
    commitFileKind: null,
    expected: 'workspace',
  },
  {
    name: 'depth 2 right diff with null commitFileKind falls back to workspace',
    depth: 2,
    focusPane: 'right',
    graphVisible: true,
    treeKind: 'file',
    graphKind: 'graphCommit',
    commitFileKind: null,
    expected: 'workspace',
  },
];

describe('activeRowKind', () => {
  for (const row of CASES) {
    it(row.name, () => {
      assert.equal(
        activeRowKind({
          depth: row.depth,
          focusPane: row.focusPane,
          graphVisible: row.graphVisible,
          treeKind: row.treeKind,
          graphKind: row.graphKind,
          commitFileKind: row.commitFileKind,
        }),
        row.expected,
      );
    });
  }
});

describe('fileWritesOnDiffAllowed', () => {
  it('allows edit and fullFile only on a focused diff', () => {
    assert.equal(fileWritesOnDiffAllowed('none', 'edit'), true);
    assert.equal(fileWritesOnDiffAllowed('none', 'fullFile'), true);
    assert.equal(fileWritesOnDiffAllowed('none', 'toggleViewed'), true);
    assert.equal(fileWritesOnDiffAllowed('none', 'move'), false);
    assert.equal(fileWritesOnDiffAllowed('graph', 'edit'), false);
    assert.equal(fileWritesOnDiffAllowed('commitFiles', 'fullFile'), false);
    assert.equal(fileWritesOnDiffAllowed('tree', 'edit'), false);
  });
});

const GRAPH_WRITE_ACTIONS: Action[] = [
  { type: 'graphCheckout' },
  { type: 'graphCreateBranch' },
  { type: 'stashApply' },
  { type: 'stashDrop' },
  { type: 'stashPop' },
];

describe('rightPaneLeftListAllowed', () => {
  it('allows graph write types when target is graph', () => {
    for (const action of GRAPH_WRITE_ACTIONS) {
      assert.equal(rightPaneLeftListAllowed('graph', action.type), true, action.type);
    }
  });

  it('rejects graph write types when target is not graph', () => {
    for (const target of ['tree', 'commitFiles', 'none'] as const) {
      for (const action of GRAPH_WRITE_ACTIONS) {
        assert.equal(
          rightPaneLeftListAllowed(target, action.type),
          false,
          `${target} ${action.type}`,
        );
      }
    }
  });

  it('graph write types are left-list actions blocked on the right pane without the exception', () => {
    for (const action of GRAPH_WRITE_ACTIONS) {
      assert.equal(isLeftListAction(action), true, action.type);
      assert.equal(
        rightPaneLeftListAllowed('graph', action.type),
        true,
        `exception must keep ${action.type} on graph`,
      );
      assert.equal(
        rightPaneLeftListAllowed('tree', action.type),
        false,
        `without graph target, ${action.type} must be blocked`,
      );
    }
  });

  it('allows depth-0-right graph writes via listFocusTarget', () => {
    const target = listFocusTarget({
      depth: 0,
      focusPane: 'right',
      graphVisible: true,
    });
    assert.equal(target, 'graph');
    for (const action of GRAPH_WRITE_ACTIONS) {
      assert.equal(rightPaneLeftListAllowed(target, action.type), true, action.type);
    }
  });

  it('allows graph move, commit-files nav, and diff file writes', () => {
    assert.equal(rightPaneLeftListAllowed('graph', 'move'), true);
    assert.equal(rightPaneLeftListAllowed('graph', 'moveTo'), true);
    assert.equal(rightPaneLeftListAllowed('commitFiles', 'move'), true);
    assert.equal(rightPaneLeftListAllowed('commitFiles', 'edit'), true);
    assert.equal(rightPaneLeftListAllowed('commitFiles', 'fullFile'), true);
    assert.equal(rightPaneLeftListAllowed('none', 'move'), true);
    assert.equal(rightPaneLeftListAllowed('none', 'moveTo'), true);
    assert.equal(rightPaneLeftListAllowed('none', 'edit'), true);
    assert.equal(rightPaneLeftListAllowed('none', 'fullFile'), true);
  });

  it('rejects fold on a right-focused graph or diff', () => {
    assert.equal(rightPaneLeftListAllowed('graph', 'fold'), false);
    assert.equal(rightPaneLeftListAllowed('graph', 'expand'), false);
    assert.equal(rightPaneLeftListAllowed('graph', 'collapse'), false);
    assert.equal(rightPaneLeftListAllowed('none', 'fold'), false);
    assert.equal(rightPaneLeftListAllowed('none', 'expand'), false);
    assert.equal(rightPaneLeftListAllowed('none', 'collapse'), false);
  });

  it('rejects other left-list writes on the right pane', () => {
    assert.equal(rightPaneLeftListAllowed('graph', 'stage'), false);
    assert.equal(rightPaneLeftListAllowed('graph', 'branch'), false);
    assert.equal(rightPaneLeftListAllowed('tree', 'move'), false);
  });
});

describe('foldAllowed', () => {
  it('allows workspace-tree and commit-file folds only', () => {
    assert.equal(foldAllowed('tree'), true);
    assert.equal(foldAllowed('commitFiles'), true);
    assert.equal(foldAllowed('graph'), false);
    assert.equal(foldAllowed('none'), false);
  });
});

describe('easyMotionListTarget', () => {
  it('labels the focused list using the actual focus pane', () => {
    assert.equal(easyMotionListTarget({ depth: 0, focusPane: 'left', graphVisible: true }), 'tree');
    assert.equal(
      easyMotionListTarget({ depth: 0, focusPane: 'right', graphVisible: true }),
      'graph',
    );
    assert.equal(
      easyMotionListTarget({ depth: 1, focusPane: 'left', graphVisible: true }),
      'graph',
    );
    assert.equal(
      easyMotionListTarget({ depth: 1, focusPane: 'right', graphVisible: true }),
      'commitFiles',
    );
    assert.equal(
      easyMotionListTarget({ depth: 2, focusPane: 'left', graphVisible: true }),
      'commitFiles',
    );
  });

  it('is a no-op on a focused diff', () => {
    assert.equal(easyMotionListTarget({ depth: 0, focusPane: 'right', graphVisible: false }), null);
    assert.equal(easyMotionListTarget({ depth: 2, focusPane: 'right', graphVisible: true }), null);
  });
});

describe('fullContextToggleId', () => {
  it('uses the commit-file id at depth 2 and when commit files are focused', () => {
    assert.equal(
      fullContextToggleId({
        target: 'commitFiles',
        depth: 1,
        treeFileId: 'tree-file',
        commitFileId: 'commit-file',
      }),
      'commit-file',
    );
    assert.equal(
      fullContextToggleId({
        target: 'none',
        depth: 2,
        treeFileId: 'tree-file',
        commitFileId: 'commit-file',
      }),
      'commit-file',
    );
  });

  it('uses the workspace-tree file id for a depth-0 file diff', () => {
    assert.equal(
      fullContextToggleId({
        target: 'none',
        depth: 0,
        treeFileId: 'tree-file',
        commitFileId: 'commit-file',
      }),
      'tree-file',
    );
  });
});
