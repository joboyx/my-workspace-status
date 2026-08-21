/**
 * Pure helpers for the full-file (unlimited context) view.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert';

import {
  diffCacheKey,
  toggleFullContext,
} from '../src/tui/fullContext.js';

describe('fullContext helpers', () => {
  it('toggleFullContext adds then removes an id', () => {
    const a = toggleFullContext('file:repo/a.ts', new Set());
    assert.ok(a.has('file:repo/a.ts'));
    const b = toggleFullContext('file:repo/a.ts', a);
    assert.equal(b.has('file:repo/a.ts'), false);
    assert.notEqual(a, b);
  });

  it('diffCacheKey distinguishes normal vs full', () => {
    assert.equal(diffCacheKey('notes', 'x.ts', false), 'notes::x.ts');
    assert.equal(diffCacheKey('notes', 'x.ts', true), 'notes::x.ts::full');
  });
});
