import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createGraphCache, shouldAutoload } from '../src/tui/graph/cache.js';
import { isGraphListFocused, listFocusTarget } from '../src/tui/graph/focus.js';
import type { GraphModel } from '../src/tui/graph/types.js';

describe('listFocusTarget', () => {
  it('depth 0 left → tree; depth 0 right + graph → graph', () => {
    assert.equal(listFocusTarget({ depth: 0, focusPane: 'left', graphVisible: true }), 'tree');
    assert.equal(listFocusTarget({ depth: 0, focusPane: 'right', graphVisible: true }), 'graph');
    assert.equal(listFocusTarget({ depth: 0, focusPane: 'right', graphVisible: false }), 'none');
  });

  it('depth 1 left → graph; depth 1 right → commitFiles', () => {
    assert.equal(listFocusTarget({ depth: 1, focusPane: 'left', graphVisible: true }), 'graph');
    assert.equal(
      listFocusTarget({ depth: 1, focusPane: 'right', graphVisible: true }),
      'commitFiles',
    );
  });

  it('depth 2 left → commitFiles', () => {
    assert.equal(
      listFocusTarget({ depth: 2, focusPane: 'left', graphVisible: true }),
      'commitFiles',
    );
    assert.equal(listFocusTarget({ depth: 2, focusPane: 'right', graphVisible: true }), 'none');
  });
});

describe('isGraphListFocused', () => {
  it('is true at depth 0 right when the graph is visible', () => {
    assert.equal(isGraphListFocused({ depth: 0, focusPane: 'right', graphVisible: true }), true);
  });

  it('is false at depth 0 left (tree focused — do not steal branch picker)', () => {
    assert.equal(isGraphListFocused({ depth: 0, focusPane: 'left', graphVisible: true }), false);
  });

  it('is true at depth 1 left when the graph is visible', () => {
    assert.equal(isGraphListFocused({ depth: 1, focusPane: 'left', graphVisible: true }), true);
  });

  it('is false at depth 0 right when the graph is hidden (file diff)', () => {
    assert.equal(isGraphListFocused({ depth: 0, focusPane: 'right', graphVisible: false }), false);
  });

  it('is false at depth 1 right (commit files) and depth 2', () => {
    assert.equal(isGraphListFocused({ depth: 1, focusPane: 'right', graphVisible: true }), false);
    assert.equal(isGraphListFocused({ depth: 2, focusPane: 'left', graphVisible: true }), false);
  });
});

describe('useAppState graph-list write gate', () => {
  it('graph pane writes use isGraphListFocused, not depth-1-left', () => {
    const src = fs.readFileSync(
      path.join(path.dirname(fileURLToPath(import.meta.url)), '../src/tui/useAppState.ts'),
      'utf8',
    );
    assert.match(src, /isGraphListFocused\(/);
    for (const id of [
      'graphCheckout',
      'graphCreateBranch',
      'stashApply',
      'stashDrop',
      'stashPop',
    ]) {
      assert.match(
        src,
        new RegExp(`case '${id}': \\{[\\s\\S]*?if \\(!graphListFocused\\) return;`),
        id,
      );
    }
    assert.doesNotMatch(src, /if \(depth !== 1 \|\| nav\.focusPane !== 'left'\) return;/);
  });
});

describe('useAppState right-pane graph cursor reset', () => {
  it('rebuilds graph cursor via shouldResetGraphCursor + graphCursorAfterRowsReload', () => {
    const src = fs.readFileSync(
      path.join(path.dirname(fileURLToPath(import.meta.url)), '../src/tui/useAppState.ts'),
      'utf8',
    );
    assert.match(src, /shouldResetGraphCursor\(/);
    assert.match(src, /graphCursorAfterRowsReload\(/);
    assert.doesNotMatch(src, /setGraphCursor\(\(c\) =>\s*nearestSelectableGraphIndex\(/);
  });
});

describe('autoload gate (engine)', () => {
  it('fires at last commit index when hasMore', () => {
    assert.equal(
      shouldAutoload({
        cursorIndex: 299,
        loadedCount: 300,
        hasMore: true,
        loading: false,
      }),
      true,
    );
  });
});

describe('invalidateRepo clears cache entries', () => {
  it('drops snapshots for one repo', () => {
    const cache = createGraphCache();
    const model = {
      repoPath: '/ws/demo',
      commits: [],
      stashes: [],
      uncommitted: null,
      headId: null,
      refsFingerprint: 'a',
      skip: 0,
      limit: 300,
      hasMore: false,
    } satisfies GraphModel;
    cache.set(
      { repoPath: '/ws/demo', refsFingerprint: 'a', skip: 0, limit: 300 },
      { model, loadedAt: 1 },
    );
    cache.invalidateRepo('/ws/demo');
    assert.equal(
      cache.get({ repoPath: '/ws/demo', refsFingerprint: 'a', skip: 0, limit: 300 }),
      undefined,
    );
  });
});
