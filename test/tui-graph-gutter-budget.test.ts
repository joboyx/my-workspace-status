import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  GUTTER_MAX_FRACTION,
  MIN_SUBJECT_FLOOR,
  graphGutterCap,
  gutterFocusCol,
  resolveGraphWidth,
  sliceCellsAroundLane,
} from '../src/tui/graph/gutterBudget.js';
import type { GraphCell } from '../src/tui/graph/types.js';

function cell(ch: string, role: GraphCell['role'] = 'pipe'): GraphCell {
  return { ch, colorLane: 0, role };
}

function cellsOf(n: number, nodeAt = -1): GraphCell[] {
  return Array.from({ length: n }, (_, i) =>
    cell(i === nodeAt ? '○' : '│', i === nodeAt ? 'node' : 'pipe'),
  );
}

describe('graphGutterCap', () => {
  it('is the tighter of fraction and subject-floor residual', () => {
    // pane 100 → fraction 30, floor residual 100-1-24=75 → 30
    assert.equal(graphGutterCap(100), Math.floor(100 * GUTTER_MAX_FRACTION));
    // pane 40 → fraction 12, floor residual 40-1-24=15 → 12
    assert.equal(graphGutterCap(40), Math.floor(40 * GUTTER_MAX_FRACTION));
    // pane 30 → fraction 9, floor residual 30-1-24=5 → 5
    assert.equal(graphGutterCap(30), 30 - 1 - MIN_SUBJECT_FLOOR);
  });
});

describe('resolveGraphWidth', () => {
  it('never exceeds topology or the hybrid cap', () => {
    assert.equal(resolveGraphWidth(4, 200), 4);
    assert.equal(resolveGraphWidth(80, 100), graphGutterCap(100));
    assert.equal(resolveGraphWidth(0, 100), 0);
  });
});

describe('sliceCellsAroundLane', () => {
  it('returns a copy when topology fits the budget', () => {
    const cells = cellsOf(4, 0);
    const out = sliceCellsAroundLane(cells, 6, 0);
    assert.equal(out.length, 4);
    assert.equal(out[0]!.role, 'node');
  });

  it('keeps the commit node when clipping a wide gutter', () => {
    // 20 cols, node at col 14 (lane 7 * CELL_W≈2 → we'll place node explicitly)
    const cells = cellsOf(20, 14);
    const out = sliceCellsAroundLane(cells, 8, 7);
    assert.equal(out.length, 8);
    assert.ok(out.some((c) => c.role === 'node'));
    assert.equal(gutterFocusCol(out, 7), out.findIndex((c) => c.role === 'node'));
  });

  it('clamps the window to the left edge when the node is near the start', () => {
    const cells = cellsOf(20, 1);
    const out = sliceCellsAroundLane(cells, 6, 0);
    assert.equal(out.length, 6);
    assert.equal(out[1]!.role, 'node');
  });

  it('clamps the window to the right edge when the node is near the end', () => {
    const cells = cellsOf(20, 18);
    const out = sliceCellsAroundLane(cells, 6, 9);
    assert.equal(out.length, 6);
    assert.equal(out[out.length - 2]!.role, 'node');
  });

  it('shifts the window to include extra join columns when they fit', () => {
    const cells = cellsOf(20, 0);
    cells[0] = cell('●', 'node');
    cells[5] = cell('╯', 'pipe');
    const out = sliceCellsAroundLane(cells, 8, 0, [5]);
    assert.equal(out.length, 8);
    assert.ok(out.some((c) => c.role === 'node'));
    assert.ok(out.some((c) => c.ch === '╯'));
  });

  it('keeps the node when extra join columns cannot fit the budget', () => {
    const cells = cellsOf(20, 0);
    cells[0] = cell('●', 'node');
    cells[8] = cell('╯', 'pipe');
    const out = sliceCellsAroundLane(cells, 4, 0, [8]);
    assert.equal(out.length, 4);
    assert.ok(out.some((c) => c.role === 'node'));
  });
});
