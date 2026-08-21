import assert from 'node:assert';
import { describe, it } from 'node:test';
import { rightPaneMode } from '../src/tui/RightPaneHost.js';
import type { NavState } from '../src/tui/nav/stack.js';
import type { VisibleRow } from '../src/tui/model/types.js';

function repoRow(): VisibleRow {
  return {
    id: 'repo:demo',
    depth: 1,
    label: 'demo',
    segments: [],
    trailing: [],
    node: {
      kind: 'repo',
      id: 'repo:demo',
      path: '/ws/demo',
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

function fileRow(): VisibleRow {
  return {
    id: 'file:demo:a.ts',
    depth: 2,
    label: 'a.ts',
    segments: [],
    trailing: [],
    node: {
      kind: 'file',
      id: 'file:demo:a.ts',
      repoPath: '/ws/demo',
      path: 'a.ts',
      status: 'M',
      staged: false,
      unstaged: true,
      untracked: false,
      change: { path: 'a.ts', unstagedStatus: 'M' },
    },
  };
}

describe('rightPaneMode', () => {
  const d0: NavState = { stack: [{ kind: 'workspace' }], focusPane: 'left' };

  it('diff for files, graph for repos at depth 0', () => {
    assert.equal(rightPaneMode(d0, fileRow()), 'diff');
    assert.equal(rightPaneMode(d0, repoRow()), 'graph');
  });

  it('commitMeta at depth 1', () => {
    const d1: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: '/ws/demo', commitId: null },
      ],
      focusPane: 'left',
    };
    assert.equal(rightPaneMode(d1, undefined), 'commitMeta');
  });

  it('diff at depth 2 (commit file selection)', () => {
    const d2: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: '/ws/demo', commitId: 'abc' },
        { kind: 'commitFiles', repo: '/ws/demo', commitId: 'abc', filePath: null },
      ],
      focusPane: 'left',
    };
    assert.equal(rightPaneMode(d2, undefined), 'diff');
  });
});
