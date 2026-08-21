import assert from 'node:assert';
import { describe, it } from 'node:test';
import { effectiveDiffMode, NARROW_SXS } from '../src/tui/DiffPane.js';
import {
  diffModeToast,
  diffModeUserLabel,
  diffPaneModeLabel,
} from '../src/tui/diffModeLabel.js';
import {
  anchorRowIndex,
  clampDiffScroll,
  diffScrollForMoveTo,
  scrollToKeepRow,
} from '../src/tui/diffScroll.js';
import type { DiffRow } from '../src/tui/diff/rows.js';

describe('clampDiffScroll', () => {
  it('clamps to EOF so repeated PageDown is a no-op', () => {
    assert.equal(clampDiffScroll(0, 100, 20), 0);
    assert.equal(clampDiffScroll(80, 100, 20), 80);
    assert.equal(clampDiffScroll(81, 100, 20), 80);
    assert.equal(clampDiffScroll(999, 100, 20), 80);
    assert.equal(clampDiffScroll(-5, 100, 20), 0);
    assert.equal(clampDiffScroll(10, 5, 20), 0);
  });
});

describe('diffScrollForMoveTo', () => {
  it('gg jumps to start and G to EOF via clampDiffScroll', () => {
    assert.equal(diffScrollForMoveTo('start', 100, 20), 0);
    assert.equal(diffScrollForMoveTo('end', 100, 20), clampDiffScroll(100, 100, 20));
    assert.equal(diffScrollForMoveTo('end', 100, 20), 80);
    assert.equal(diffScrollForMoveTo('end', 5, 20), 0);
  });
});

describe('effectiveDiffMode', () => {
  it('keeps sideBySide at or above NARROW_SXS', () => {
    assert.equal(effectiveDiffMode('sideBySide', NARROW_SXS), 'sideBySide');
    assert.equal(effectiveDiffMode('sideBySide', NARROW_SXS + 1), 'sideBySide');
  });

  it('falls back to inline below NARROW_SXS', () => {
    assert.equal(effectiveDiffMode('sideBySide', NARROW_SXS - 1), 'inline');
    assert.equal(effectiveDiffMode('sideBySide', 20), 'inline');
  });

  it('leaves inline unchanged', () => {
    assert.equal(effectiveDiffMode('inline', 200), 'inline');
    assert.equal(effectiveDiffMode('inline', 40), 'inline');
  });
});

describe('diff mode user labels', () => {
  it('calls the non-inline layout split, matching help overlay wording', () => {
    assert.equal(diffModeUserLabel('inline'), 'inline');
    assert.equal(diffModeUserLabel('sideBySide'), 'split');
  });

  it('uses split on the diff pane header and i-toggle toast', () => {
    assert.equal(diffPaneModeLabel('sideBySide', 'sideBySide'), 'split');
    assert.equal(diffPaneModeLabel('inline', 'inline'), 'inline');
    assert.equal(diffPaneModeLabel('sideBySide', 'inline'), 'inline (too narrow)');
    assert.equal(diffModeToast('sideBySide'), 'Diff: split');
    assert.equal(diffModeToast('inline'), 'Diff: inline');
  });
});

describe('scrollToKeepRow', () => {
  it('places the target in the upper third and clamps', () => {
    assert.equal(
      scrollToKeepRow({ rowIndex: 50, viewHeight: 20, rowCount: 100, prefer: 'upperThird' }),
      50 - Math.floor(20 / 3),
    );
    assert.equal(
      scrollToKeepRow({ rowIndex: 0, viewHeight: 20, rowCount: 100, prefer: 'upperThird' }),
      0,
    );
    assert.equal(
      scrollToKeepRow({ rowIndex: 99, viewHeight: 20, rowCount: 100, prefer: 'upperThird' }),
      80,
    );
  });
});

describe('anchorRowIndex', () => {
  const rows: DiffRow[] = [
    { kind: 'section', section: 'staged' },
    { kind: 'hunk', text: '@@ -1,3 +1,4 @@' },
    { kind: 'line', left: { kind: 'ctx', text: 'a', lineNo: 1 } },
    { kind: 'line', left: { kind: 'del', text: 'b', lineNo: 2 } },
    { kind: 'line', left: { kind: 'add', text: 'c', lineNo: 2 } },
    { kind: 'hunk', text: '@@ -10,2 +10,2 @@' },
    { kind: 'line', left: { kind: 'ctx', text: 'd', lineNo: 10 } },
  ];

  it('prefers the first visible add/del line', () => {
    assert.equal(anchorRowIndex(rows, 2, 3), 3);
  });

  it('falls back to nearest hunk at or above scroll', () => {
    assert.equal(anchorRowIndex(rows, 6, 1), 5);
  });

  it('returns scroll when nothing matches', () => {
    const onlyCtx: DiffRow[] = [
      { kind: 'line', left: { kind: 'ctx', text: 'x', lineNo: 1 } },
    ];
    assert.equal(anchorRowIndex(onlyCtx, 0, 5), 0);
  });
});
