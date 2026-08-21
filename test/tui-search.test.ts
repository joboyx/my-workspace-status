import assert from 'node:assert';
import { describe, it } from 'node:test';
import { buildDiffRows } from '../src/tui/diff/rows.js';
import { scrollToKeepRow } from '../src/tui/diffScroll.js';
import { listFocusTarget } from '../src/tui/graph/focus.js';
import { helpEntryMatches } from '../src/tui/helpSearch.js';
import {
  collectMatchIds,
  collectSearchMatchIds,
  firstMatchIndex,
  focusTreeSearchMatch,
  matchDiffRowIndices,
  matchIndices,
  stepMatch,
  stepMatchId,
} from '../src/tui/search.js';
import { buildTree } from '../src/tui/model/tree.js';
import { flatten } from '../src/tui/model/flatten.js';
import { createFoldState } from '../src/tui/model/fold.js';
import type { RepoSnapshot } from '../src/types.js';
import { HELP_GROUPS } from '../src/tui/StatusBar.js';
import { createKeyState, handleKey } from '../src/tui/keys.js';

const rows = [{ label: 'Alpha' }, { label: 'beta' }, { label: 'Alphabet' }];

describe('matchIndices', () => {
  it('finds case-insensitive substring matches without filtering rows', () => {
    assert.deepEqual(matchIndices(rows, 'alp'), [0, 2]);
    assert.deepEqual(matchIndices(rows, ''), []);
  });
});

describe('firstMatchIndex', () => {
  it('returns first or null', () => {
    assert.equal(firstMatchIndex([0, 2]), 0);
    assert.equal(firstMatchIndex([]), null);
  });
});

describe('stepMatch', () => {
  it('wraps n/N', () => {
    const idx = [0, 2];
    assert.equal(stepMatch(idx, 0, 1), 2);
    assert.equal(stepMatch(idx, 2, 1), 0);
    assert.equal(stepMatch(idx, 2, -1), 0);
  });
});

const treeRows = [
  { id: 'repo:notes', label: 'notes' },
  { id: 'repo:dotfiles', label: 'dotfiles' },
];
const graphRows = [
  { id: 'graph:commit:aaa', label: 'fix search highlight', selectable: true },
  { id: 'graph:spacer:aaa', label: 'alice 2h ago', selectable: false },
  { id: 'graph:commit:bbb', label: 'notes: tweak copy', selectable: true },
];
const commitFileRows = [
  { id: 'file:src/search.ts', label: 'search.ts' },
  { id: 'file:src/App.tsx', label: 'App.tsx' },
];

describe('collectSearchMatchIds', () => {
  it('matches workspace tree rows when the bound target is tree', () => {
    const ids = collectSearchMatchIds({
      target: 'tree',
      query: 'notes',
      treeRows,
      graphRows,
      commitFileRows,
    });
    assert.deepEqual([...ids], ['repo:notes']);
  });

  it('matches selectable graph rows when bound to the graph pane', () => {
    const target = listFocusTarget({
      depth: 0,
      focusPane: 'right',
      graphVisible: true,
    });
    assert.equal(target, 'graph');
    const ids = collectSearchMatchIds({
      target,
      query: 'search',
      treeRows,
      graphRows,
      commitFileRows,
    });
    assert.deepEqual([...ids], ['graph:commit:aaa']);
  });

  it('skips non-selectable graph spacer rows', () => {
    const ids = collectSearchMatchIds({
      target: 'graph',
      query: 'alice',
      treeRows,
      graphRows,
      commitFileRows,
    });
    assert.equal(ids.size, 0);
  });

  it('matches commit-file rows when bound to the commit-files pane', () => {
    const target = listFocusTarget({
      depth: 1,
      focusPane: 'right',
      graphVisible: true,
    });
    assert.equal(target, 'commitFiles');
    const ids = collectSearchMatchIds({
      target,
      query: 'search',
      treeRows,
      graphRows,
      commitFileRows,
    });
    assert.deepEqual([...ids], ['file:src/search.ts']);
  });

  it('returns no list ids when the bound target is the diff pane', () => {
    const target = listFocusTarget({
      depth: 2,
      focusPane: 'right',
      graphVisible: true,
    });
    assert.equal(target, 'none');
    const ids = collectSearchMatchIds({
      target,
      query: 'search',
      treeRows,
      graphRows,
      commitFileRows,
    });
    assert.equal(ids.size, 0);
  });

  it('keeps matching the pane bound at search start after focus would change', () => {
    const bound = listFocusTarget({
      depth: 0,
      focusPane: 'right',
      graphVisible: true,
    });
    const laterFocus = listFocusTarget({
      depth: 0,
      focusPane: 'left',
      graphVisible: true,
    });
    assert.equal(bound, 'graph');
    assert.equal(laterFocus, 'tree');
    const ids = collectSearchMatchIds({
      target: bound,
      query: 'notes',
      treeRows,
      graphRows,
      commitFileRows,
    });
    assert.deepEqual([...ids], ['graph:commit:bbb']);
  });

  it('returns an empty set for a blank query', () => {
    const ids = collectSearchMatchIds({
      target: 'tree',
      query: '   ',
      treeRows,
      graphRows,
      commitFileRows,
    });
    assert.equal(ids.size, 0);
  });
});

const INLINE_DIFF = `diff --git a/hello.ts b/hello.ts
index 1111111..2222222 100644
--- a/hello.ts
+++ b/hello.ts
@@ -10,3 +10,4 @@
 line1
-line2
+line2 changed
 line3
+uniqueRight
`;

describe('matchDiffRowIndices', () => {
  it('matches inline left-cell text and does not drop unmatched rows', () => {
    const diffRows = buildDiffRows({
      staged: INLINE_DIFF,
      unstaged: '',
      mode: 'inline',
    });
    const hits = matchDiffRowIndices(diffRows, 'line2');
    assert.ok(hits.length >= 2);
    for (const i of hits) {
      const row = diffRows[i]!;
      assert.equal(row.kind, 'line');
      assert.match(row.left.text, /line2/i);
    }
    assert.ok(diffRows.length > hits.length);
  });

  it('matches side-by-side left and/or right cell text', () => {
    const diffRows = buildDiffRows({
      staged: INLINE_DIFF,
      unstaged: '',
      mode: 'sideBySide',
    });
    const leftHits = matchDiffRowIndices(diffRows, 'line2');
    assert.ok(leftHits.length >= 1);
    const rightOnly = matchDiffRowIndices(diffRows, 'uniqueRight');
    assert.equal(rightOnly.length, 1);
    const row = diffRows[rightOnly[0]!]!;
    assert.equal(row.kind, 'line');
    assert.ok(
      row.left.text.toLowerCase().includes('uniqueright') ||
        (row.right?.text.toLowerCase().includes('uniqueright') ?? false),
    );
  });

  it('does not treat section headers as matches', () => {
    const diffRows = buildDiffRows({
      staged: INLINE_DIFF,
      unstaged: '',
      mode: 'inline',
    });
    assert.equal(diffRows[0]?.kind, 'section');
    assert.deepEqual(matchDiffRowIndices(diffRows, 'staged'), []);
  });

  it('wraps n/N among diff matches', () => {
    const diffRows = buildDiffRows({
      staged: INLINE_DIFF,
      unstaged: '',
      mode: 'inline',
    });
    const hits = matchDiffRowIndices(diffRows, 'line');
    assert.ok(hits.length >= 2);
    const first = hits[0]!;
    const last = hits[hits.length - 1]!;
    assert.equal(stepMatch(hits, last, 1), first);
    assert.equal(stepMatch(hits, first, -1), last);
  });

  it('scrolls so the current diff match stays in view', () => {
    const diffRows = buildDiffRows({
      staged: INLINE_DIFF,
      unstaged: '',
      mode: 'inline',
    });
    const hits = matchDiffRowIndices(diffRows, 'uniqueRight');
    assert.equal(hits.length, 1);
    const rowIndex = hits[0]!;
    const viewHeight = 3;
    const scroll = scrollToKeepRow({
      rowIndex,
      viewHeight,
      rowCount: diffRows.length,
      prefer: 'center',
    });
    assert.ok(rowIndex >= scroll);
    assert.ok(rowIndex < scroll + viewHeight);
  });
});

describe('help overlay slash', () => {
  it('stays help-local and does not describe left-only pane search', () => {
    const slash = HELP_GROUPS.flatMap((g) => g.keys).find(([keys]) => keys === '/');
    assert.ok(slash);
    assert.equal(slash[1], 'search focused pane (Enter arms)');
    assert.equal(helpEntryMatches(slash[0], slash[1], 'focused'), true);
    assert.equal(helpEntryMatches('j k', 'down / up', 'focused'), false);
  });
});

describe('armed search keys', () => {
  it('armed p is no longer searchPrev (falls through to pull)', () => {
    const armed = { ...createKeyState(), searchActive: true };
    assert.notEqual(handleKey(armed, 'p', {}, 'repo').action.type, 'searchPrev');
    assert.deepEqual(handleKey(armed, 'p', {}, 'repo').action, { type: 'pull' });
    assert.deepEqual(handleKey(armed, 'N', {}, 'repo').action, { type: 'searchPrev' });
  });
});


function snap(partial: Partial<RepoSnapshot>): RepoSnapshot {
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

describe('folded tree search', () => {
  function foldedRepos() {
    const snapshots = [
      snap({
        repo: 'alpha-repo',
        hasUnstaged: true,
        unstagedFiles: 'M\talpha-unique.ts',
      }),
      snap({
        repo: 'beta-repo',
        hasUnstaged: true,
        unstagedFiles: 'M\talphabet-unique.ts',
      }),
    ];
    const tree = buildTree({
      snapshots,
      ignoredRepos: new Set(['alpha-repo', 'beta-repo']),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const folds = createFoldState(tree);
    return { tree, folds };
  }

  it('includes matches inside folded rows', () => {
    const { tree, folds } = foldedRepos();
    const visible = flatten(tree, folds);
    const hidden = flatten(tree, new Set());
    assert.equal(
      visible.some((r) => r.id.includes('alpha-unique.ts')),
      false,
    );
    const ids = collectMatchIds(hidden, 'unique');
    assert.ok(ids.some((id) => id.includes('alpha-unique.ts')));
    assert.ok(ids.some((id) => id.includes('alphabet-unique.ts')));
  });

  it('unfolds only the match about to be focused', () => {
    const { tree, folds } = foldedRepos();
    assert.equal(folds.has('repo:alpha-repo'), true);
    assert.equal(folds.has('repo:beta-repo'), true);

    const first = focusTreeSearchMatch({
      tree,
      folds,
      query: 'unique',
      currentId: null,
      dir: 0,
    });
    assert.ok(first.focusId);
    assert.match(first.focusId, /alpha-unique/);
    assert.equal(first.folds.has('repo:alpha-repo'), false);
    assert.equal(first.folds.has('repo:beta-repo'), true);
    assert.ok(flatten(tree, first.folds).some((r) => r.id === first.focusId));
    assert.equal(
      flatten(tree, first.folds).some((r) => r.id.includes('alphabet-unique')),
      false,
    );

    const next = focusTreeSearchMatch({
      tree,
      folds: first.folds,
      query: 'unique',
      currentId: first.focusId,
      dir: 1,
    });
    assert.ok(next.focusId);
    assert.match(next.focusId, /alphabet-unique/);
    assert.equal(next.folds.has('repo:beta-repo'), false);
    assert.ok(flatten(tree, next.folds).some((r) => r.id === next.focusId));
  });

  it('N wraps to the previous folded match without pre-expanding', () => {
    const { tree, folds } = foldedRepos();
    const last = focusTreeSearchMatch({
      tree,
      folds,
      query: 'unique',
      currentId: null,
      dir: -1,
    });
    assert.ok(last.focusId);
    assert.match(last.focusId, /alphabet-unique/);
    assert.equal(last.folds.has('repo:alpha-repo'), true);
    assert.equal(last.folds.has('repo:beta-repo'), false);
    assert.deepEqual(stepMatchId(['a', 'b'], 'b', -1), 'a');
  });
});
