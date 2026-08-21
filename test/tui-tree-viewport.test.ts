import assert from 'node:assert';
import { describe, it } from 'node:test';
import { treeFlashNow, visibleTreeWindow } from '../src/tui/TreePane.js';
import { flashStrength } from '../src/tui/watch.js';

describe('visibleTreeWindow', () => {
  it('returns at most height rows', () => {
    const rows = Array.from({ length: 80 }, (_, i) => ({ id: `r${i}` }));
    const { visible, start } = visibleTreeWindow(rows, 40, 20);
    assert.ok(visible.length <= 20);
    assert.equal(visible.length, 20);
    assert.equal(start, 30);
    assert.equal(visible[0]?.id, 'r30');
    assert.equal(visible[19]?.id, 'r49');
  });

  it('clamps when fewer rows than height', () => {
    const rows = [{ id: 'a' }, { id: 'b' }];
    const { visible, start } = visibleTreeWindow(rows, 1, 20);
    assert.equal(start, 0);
    assert.equal(visible.length, 2);
  });
});

describe('treeFlashNow', () => {
  it('prefers a positive wall-clock stamp over Date.now', () => {
    assert.equal(treeFlashNow(1_700_000_000_000, true, () => 99), 1_700_000_000_000);
    assert.equal(treeFlashNow(0, false, () => 99), 0);
  });

  it('falls back to wall time when clock is unset but flashes are active', () => {
    assert.equal(treeFlashNow(undefined, true, () => 42), 42);
    assert.equal(treeFlashNow(0, true, () => 42), 42);
  });

  it('keeps flash strength alive with wall clock, not a tick counter', () => {
    const flashedAt = 1_700_000_000_500;
    // Regression: post-virtualization TreePane used clock as 0/1/2… ticks.
    const tickNow = treeFlashNow(1, true, () => flashedAt);
    assert.equal(flashStrength(flashedAt, tickNow), 0);

    const wallNow = treeFlashNow(flashedAt, true, () => 0);
    assert.equal(flashStrength(flashedAt, wallNow), 1);
  });
});
