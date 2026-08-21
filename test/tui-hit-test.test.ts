import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  foldChevronContentCol,
  hitTest,
  listRowIndexAtBodyY,
  treeViewportStart,
} from '../src/tui/hitTest.js';
import type { HitListLayout } from '../src/tui/hitTest.js';
import { paneWidths } from '../src/tui/layoutWidths.js';

describe('treeViewportStart', () => {
  it('matches the TreePane centering formula', () => {
    assert.equal(treeViewportStart(100, 0, 10), 0);
    assert.equal(treeViewportStart(100, 50, 10), 45);
    assert.equal(treeViewportStart(5, 4, 10), 0);
  });
});

describe('hitTest', () => {
  const rows = [
    { id: 'workspace', depth: 0, node: { kind: 'workspace' } },
    { id: 'repo:a', depth: 1, node: { kind: 'repo' } },
    { id: 'file:a:f', depth: 2, node: { kind: 'file' } },
  ];

  it('maps a click in the tree body to a row index', () => {
    // y=1 is the pane title chip; list rows start at y=2.
    const hit = hitTest({
      x: 10,
      y: 3,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set(),
    });
    assert.equal(hit.pane, 'tree');
    if (hit.pane === 'tree') assert.equal(hit.rowIndex, 1);
  });

  it('ignores selection on the pane title chip row but keeps the list pane', () => {
    const hit = hitTest({
      x: 10,
      y: 1,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set(),
    });
    assert.equal(hit.pane, 'tree');
    if (hit.pane === 'tree') assert.equal(hit.rowIndex, null);
  });

  it('marks the fold chevron column', () => {
    // Content starts at terminal x=2 (left pad). Chevron contentCol =
    // 1 + depth*2; for depth-1 that is 3 → terminal x = 2 + 3 = 5.
    const depth = 1;
    const x = 2 + foldChevronContentCol(depth);
    const hit = hitTest({
      x,
      y: 3,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set(),
    });
    assert.equal(hit.pane, 'tree');
    if (hit.pane === 'tree') {
      assert.equal(hit.rowIndex, 1);
      assert.equal(hit.foldChevron, true);
    }
  });

  it('maps clicks right of treeWidth to the diff pane', () => {
    const hit = hitTest({
      x: 50,
      y: 5,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set(),
    });
    assert.equal(hit.pane, 'diff');
  });

  it('hits the divider at treeWidth and ±1', () => {
    const base = {
      y: 3,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set<string>(),
    };
    for (const x of [39, 40, 41]) {
      assert.equal(hitTest({ ...base, x }).pane, 'divider', `x=${x}`);
    }
    // Outside the band → tree / diff as usual.
    assert.equal(hitTest({ ...base, x: 38 }).pane, 'tree');
    assert.equal(hitTest({ ...base, x: 42 }).pane, 'diff');
  });

  it('clamps divider band to valid columns', () => {
    // treeWidth at left edge: treeWidth-1 is invalid → only 1 and 2 hit.
    const base = {
      y: 5,
      termCols: 80,
      termRows: 30,
      treeWidth: 1,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set(),
    };
    assert.equal(hitTest({ ...base, x: 1 }).pane, 'divider');
    assert.equal(hitTest({ ...base, x: 2 }).pane, 'divider');
    assert.equal(hitTest({ ...base, x: 3 }).pane, 'diff');
  });

  it('hits the in-diff split RULE at ruleX and ±1', () => {
    const base = {
      y: 5,
      termCols: 200,
      termRows: 30,
      treeWidth: 80,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set<string>(),
      diffSplitRuleX: 140,
    };
    for (const x of [139, 140, 141]) {
      assert.equal(hitTest({ ...base, x }).pane, 'diffSplit', `x=${x}`);
    }
    assert.equal(hitTest({ ...base, x: 138 }).pane, 'diff');
    assert.equal(hitTest({ ...base, x: 142 }).pane, 'diff');
    // Pane divider still wins when both bands could theoretically overlap.
    assert.equal(hitTest({ ...base, x: 80, treeWidth: 80 }).pane, 'divider');
  });

  it('does not hit diffSplit when ruleX is omitted', () => {
    const hit = hitTest({
      x: 140,
      y: 5,
      termCols: 200,
      termRows: 30,
      treeWidth: 80,
      paneHeight: 28,
      statusLines: 1,
      rowCount: rows.length,
      cursor: 0,
      rows,
      folds: new Set(),
    });
    assert.equal(hit.pane, 'diff');
  });

  it('hit columns ignore label length (B1)', () => {
    const makeRows = (label: string) => [
      { id: 'workspace', depth: 0, node: { kind: 'workspace' } },
      { id: `repo:${label}`, depth: 1, node: { kind: 'repo' } },
      { id: 'file:a:f', depth: 2, node: { kind: 'file' } },
    ];
    const short = makeRows('a');
    const long = makeRows('a'.repeat(200));
    const { treeWidth } = paneWidths(100);
    const base = {
      x: 5,
      y: 3,
      termCols: 100,
      termRows: 30,
      treeWidth,
      paneHeight: 28,
      statusLines: 1,
      cursor: 0,
      folds: new Set<string>(),
    };
    const h1 = hitTest({ ...base, rowCount: short.length, rows: short });
    const h2 = hitTest({ ...base, rowCount: long.length, rows: long });
    assert.deepEqual(h1, h2);
    assert.equal(h1.pane, 'tree');
    if (h1.pane === 'tree') {
      assert.equal(h1.rowIndex, 1);
      assert.equal(h1.foldChevron, true);
    }
  });

  it('maps right-pane graph clicks to graph row indices', () => {
    const graphRows = [{ depth: 0 }, { depth: 0 }, { depth: 0 }, { depth: 0 }];
    const right: HitListLayout = {
      kind: 'graph',
      rowCount: graphRows.length,
      cursor: 0,
      rows: graphRows,
      headerLines: 1,
      footerLines: 2,
      listHeight: 10,
    };
    // y=2 is first body row after title → header chrome → graph, no row
    const headerHit = hitTest({
      x: 50,
      y: 2,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      right,
    });
    assert.equal(headerHit.pane, 'graph');
    if (headerHit.pane === 'graph') {
      assert.equal(headerHit.side, 'right');
      assert.equal(headerHit.rowIndex, null);
    }
    // y=3 → first list row (index 0)
    const hit = hitTest({
      x: 50,
      y: 3,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      right,
    });
    assert.equal(hit.pane, 'graph');
    if (hit.pane === 'graph') {
      assert.equal(hit.side, 'right');
      assert.equal(hit.rowIndex, 0);
    }
  });

  it('maps left-pane commit-file clicks with fold chevron', () => {
    const fileRows = [{ depth: 0 }, { depth: 1 }, { depth: 2 }];
    const left: HitListLayout = {
      kind: 'commitFiles',
      rowCount: fileRows.length,
      cursor: 0,
      rows: fileRows,
      headerLines: 0,
      footerLines: 0,
      listHeight: 20,
    };
    const depth = 1;
    const x = 2 + foldChevronContentCol(depth);
    const hit = hitTest({
      x,
      y: 3,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      left,
      right: {
        kind: 'diff',
        rowCount: 0,
        cursor: 0,
        rows: [],
        headerLines: 0,
        footerLines: 0,
        listHeight: 20,
      },
    });
    assert.equal(hit.pane, 'commitFiles');
    if (hit.pane === 'commitFiles') {
      assert.equal(hit.side, 'left');
      assert.equal(hit.rowIndex, 1);
      assert.equal(hit.foldChevron, true);
    }
  });

  it('maps right commitMeta file tree below header lines', () => {
    const fileRows = [{ depth: 0 }, { depth: 1 }];
    const right: HitListLayout = {
      kind: 'commitFiles',
      rowCount: fileRows.length,
      cursor: 0,
      rows: fileRows,
      headerLines: 2,
      footerLines: 0,
      listHeight: 15,
    };
    const headerHit = hitTest({
      x: 50,
      y: 3,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      right,
    });
    assert.equal(headerHit.pane, 'commitFiles');
    if (headerHit.pane === 'commitFiles') {
      assert.equal(headerHit.rowIndex, null);
    }
    const hit = hitTest({
      x: 50,
      y: 4,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      right,
    });
    assert.equal(hit.pane, 'commitFiles');
    if (hit.pane === 'commitFiles') {
      assert.equal(hit.side, 'right');
      assert.equal(hit.rowIndex, 0);
    }
  });

  it('keeps graph pane on footer chrome for wheel (no row select)', () => {
    const right: HitListLayout = {
      kind: 'graph',
      rowCount: 4,
      cursor: 0,
      rows: [{ depth: 0 }, { depth: 0 }, { depth: 0 }, { depth: 0 }],
      headerLines: 1,
      footerLines: 2,
      listHeight: 3,
    };
    // bodyY = header(1) + list(3) = 4 → first footer line → y = 2 + 4 = 6
    const hit = hitTest({
      x: 50,
      y: 6,
      termCols: 100,
      termRows: 30,
      treeWidth: 40,
      paneHeight: 28,
      statusLines: 1,
      right,
    });
    assert.equal(hit.pane, 'graph');
    if (hit.pane === 'graph') assert.equal(hit.rowIndex, null);
  });
});

describe('listRowIndexAtBodyY', () => {
  it('skips header chrome before mapping into the viewport', () => {
    const layout: HitListLayout = {
      kind: 'graph',
      rowCount: 20,
      cursor: 0,
      rows: Array.from({ length: 20 }, () => ({ depth: 0 })),
      headerLines: 1,
      footerLines: 2,
      listHeight: 5,
    };
    assert.equal(listRowIndexAtBodyY(0, layout), null);
    assert.equal(listRowIndexAtBodyY(1, layout), 0);
    assert.equal(listRowIndexAtBodyY(5, layout), 4);
    assert.equal(listRowIndexAtBodyY(6, layout), null);
  });
});
