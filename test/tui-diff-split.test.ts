import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  MIN_DIFF_COL,
  NARROW_SXS,
  clampDiffLeftWidth,
  diffContentOriginX,
  diffSplitFractionFromLeftWidth,
  diffSplitFractionFromTerminalX,
  diffSplitRuleX,
  isSideBySideSplit,
  sideBySideColumnWidths,
} from '../src/tui/diffSplit.js';

describe('sideBySideColumnWidths', () => {
  it('defaults to a 50/50 split matching floor((width-1)/2)', () => {
    for (const width of [100, 117, 118, 141]) {
      const { leftWidth, rightWidth } = sideBySideColumnWidths(width);
      assert.equal(leftWidth, Math.floor((width - 1) / 2), `left width=${width}`);
      assert.equal(leftWidth + 1 + rightWidth, width, `sum width=${width}`);
    }
  });

  it('honours a custom fraction', () => {
    const wide = sideBySideColumnWidths(120, 0.7);
    const narrow = sideBySideColumnWidths(120, 0.3);
    assert.ok(wide.leftWidth > narrow.leftWidth);
    assert.equal(wide.leftWidth + 1 + wide.rightWidth, 120);
    assert.equal(narrow.leftWidth + 1 + narrow.rightWidth, 120);
  });

  it('clamps so both columns stay ≥ MIN_DIFF_COL', () => {
    const tooWide = sideBySideColumnWidths(120, 0.95);
    const tooNarrow = sideBySideColumnWidths(120, 0.05);
    assert.ok(tooWide.leftWidth >= MIN_DIFF_COL);
    assert.ok(tooWide.rightWidth >= MIN_DIFF_COL);
    assert.ok(tooNarrow.leftWidth >= MIN_DIFF_COL);
    assert.ok(tooNarrow.rightWidth >= MIN_DIFF_COL);
  });
});

describe('clamp / fraction round-trip', () => {
  it('diffSplitFractionFromLeftWidth round-trips through sideBySideColumnWidths', () => {
    const pane = 120;
    const fraction = diffSplitFractionFromLeftWidth(pane, 70);
    const { leftWidth } = sideBySideColumnWidths(pane, fraction);
    assert.equal(leftWidth, 70);
  });

  it('maps a terminal x onto the same left width as a direct clamp', () => {
    const treeWidth = 80;
    const pane = 118;
    const x = diffSplitRuleX(treeWidth, 70);
    const fraction = diffSplitFractionFromTerminalX(treeWidth, pane, x);
    const { leftWidth } = sideBySideColumnWidths(pane, fraction);
    assert.equal(leftWidth, clampDiffLeftWidth(pane, 70));
    assert.equal(diffContentOriginX(treeWidth), treeWidth + 2);
  });
});

describe('isSideBySideSplit', () => {
  it('is true only for sideBySide at or above NARROW_SXS', () => {
    assert.equal(isSideBySideSplit('sideBySide', NARROW_SXS), true);
    assert.equal(isSideBySideSplit('sideBySide', NARROW_SXS - 1), false);
    assert.equal(isSideBySideSplit('inline', 200), false);
  });
});
