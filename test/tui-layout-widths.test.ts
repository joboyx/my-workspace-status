import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  TREE_WIDTH_FRACTION,
  clampTreeFraction,
  paneWidths,
  treeFractionFromWidth,
  widthsEqual,
} from '../src/tui/layoutWidths.js';

describe('paneWidths', () => {
  it('depends only on termCols at default fraction', () => {
    const a = paneWidths(120);
    const b = paneWidths(120);
    assert.deepEqual(a, b);
    assert.equal(a.treeWidth, Math.max(20, Math.floor(120 * TREE_WIDTH_FRACTION)));
    assert.equal(a.treeInnerWidth, Math.max(8, a.treeWidth - 3));
    assert.equal(a.diffWidth, Math.max(20, 120 - a.treeWidth - 2));
  });

  it('honours a custom fraction', () => {
    const wide = paneWidths(120, 0.6);
    const narrow = paneWidths(120, 0.25);
    assert.equal(wide.treeWidth, Math.floor(120 * 0.6));
    assert.equal(narrow.treeWidth, Math.floor(120 * 0.25));
    assert.ok(wide.treeWidth > narrow.treeWidth);
    assert.ok(wide.diffWidth >= 20);
    assert.ok(narrow.diffWidth >= 20);
  });

  it('widthsEqual is structural', () => {
    assert.equal(widthsEqual(paneWidths(100), paneWidths(100)), true);
    assert.equal(widthsEqual(paneWidths(100), paneWidths(101)), false);
  });
});

describe('clampTreeFraction / treeFractionFromWidth', () => {
  it('clamps so both panes stay ≥ 20 cols', () => {
    const cols = 100;
    const tooWide = clampTreeFraction(cols, 0.95);
    const tooNarrow = clampTreeFraction(cols, 0.05);
    const wide = paneWidths(cols, tooWide);
    const narrow = paneWidths(cols, tooNarrow);
    assert.ok(wide.treeWidth >= 20);
    assert.ok(wide.diffWidth >= 20);
    assert.ok(narrow.treeWidth >= 20);
    assert.ok(narrow.diffWidth >= 20);
    // Max tree leaves room for diff ≥ 20 + pad 2 → tree ≤ 78.
    assert.ok(wide.treeWidth <= cols - 22);
  });

  it('treeFractionFromWidth round-trips through paneWidths', () => {
    const cols = 120;
    const fraction = treeFractionFromWidth(cols, 55);
    const { treeWidth } = paneWidths(cols, fraction);
    assert.equal(treeWidth, 55);
  });

  it('default fraction matches TREE_WIDTH_FRACTION', () => {
    assert.equal(paneWidths(80).treeWidth, paneWidths(80, TREE_WIDTH_FRACTION).treeWidth);
  });
});
