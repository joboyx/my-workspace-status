import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  drillContextFromFocused,
  drillContextFromGraph,
} from '../src/tui/nav/drill.js';
import type { GraphListRow } from '../src/tui/graph/list.js';
import type { VisibleRow } from '../src/tui/model/types.js';

function fileRow(): VisibleRow {
  return {
    id: 'file:rs:src/a.ts',
    depth: 2,
    label: 'a.ts',
    segments: [],
    trailing: [],
    node: {
      kind: 'file',
      id: 'file:rs:src/a.ts',
      repoPath: '/ws/rs',
      path: 'src/a.ts',
      status: 'M',
      staged: false,
      unstaged: true,
      untracked: false,
      change: { path: 'src/a.ts', unstagedStatus: 'M' },
    },
  };
}

function repoRow(): VisibleRow {
  return {
    id: 'repo:rs',
    depth: 1,
    label: 'rs',
    segments: [],
    trailing: [],
    node: {
      kind: 'repo',
      id: 'repo:rs',
      path: '/ws/rs',
      branch: 'main',
      checkoutKind: 'primary',
      mergedIntoDefault: null,
      sync: '',
      syncStatus: 'up-to-date',
      ignored: false,
      changeCount: 0,
      children: [],
    },
  };
}

describe('drillContextFromFocused', () => {
  it('reads repo + file path from a file row', () => {
    assert.deepEqual(drillContextFromFocused(fileRow()), {
      repo: '/ws/rs',
      commitId: null,
      filePath: 'src/a.ts',
    });
  });

  it('reads repo path from a repo row', () => {
    assert.deepEqual(drillContextFromFocused(repoRow()), {
      repo: '/ws/rs',
      commitId: null,
      filePath: null,
    });
  });

  it('returns empty repo when nothing is focused', () => {
    assert.deepEqual(drillContextFromFocused(undefined), {
      repo: '',
      commitId: null,
      filePath: null,
    });
  });
});

describe('drillContextFromGraph', () => {
  it('maps commit and uncommitted rows', () => {
    const commit: GraphListRow = {
      id: 'graph:commit:abc',
      kind: 'commit',
      commitId: 'abcdef1',
      segments: [],
    };
    assert.deepEqual(drillContextFromGraph('/ws/rs', commit), {
      repo: '/ws/rs',
      commitId: 'abcdef1',
      filePath: null,
    });
    const wt: GraphListRow = {
      id: 'graph:uncommitted',
      kind: 'uncommitted',
      commitId: null,
      segments: [],
    };
    assert.deepEqual(drillContextFromGraph('/ws/rs', wt), {
      repo: '/ws/rs',
      commitId: null,
      filePath: null,
    });
  });
});
