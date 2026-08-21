import assert from 'node:assert';
import { describe, it } from 'node:test';
import { applyPan, clampColOffset, maxColOffset, sliceVisible } from '../src/tui/diffPan.js';
import { codeWindow } from '../src/tui/DiffPane.js';

describe('diffPan', () => {
  it('clamps offset to line length window', () => {
    assert.equal(maxColOffset([10, 80, 20], 40), 40);
    assert.equal(clampColOffset(-3, 40), 0);
    assert.equal(clampColOffset(99, 40), 40);
    assert.equal(applyPan(0, -1, 40), 0);
    assert.equal(applyPan(40, 1, 40), 40);
    assert.equal(applyPan(5, 3, 40), 8);
  });

  it('slices the visible window', () => {
    assert.equal(sliceVisible('abcdefghijklmnopqrstuvwxyz', 10, 5), 'klmno');
  });
});

describe('codeWindow', () => {
  it('pans then windows to codeWidth', () => {
    assert.equal(codeWindow('abcdefghijklmnopqrstuvwxyz', 10, 5), 'klmno');
    assert.equal(codeWindow('short', 0, 10), 'short');
    assert.equal(codeWindow('abcdefghij', 0, 4), 'abcd');
  });
});
