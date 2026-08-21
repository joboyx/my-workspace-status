import assert from 'node:assert';
import { describe, it } from 'node:test';
import { treeRowEmphasis } from '../src/tui/treeEmphasis.js';
import { FLASH_MS } from '../src/tui/watch.js';
import { setActiveTheme, THEMES } from '../src/tui/theme.js';

describe('treeRowEmphasis', () => {
  it('selected uses edge + cursorBg, not flash', () => {
    setActiveTheme(THEMES['tokyo-night']);
    const e = treeRowEmphasis({
      selected: true,
      flashedAt: Date.now(),
      now: Date.now(),
      cursorBg: '#283457',
    });
    assert.equal(e.edge, true);
    assert.equal(e.backgroundColor, '#283457');
  });

  it('flash uses background only when not selected', () => {
    setActiveTheme(THEMES['tokyo-night']);
    const now = 1000;
    const e = treeRowEmphasis({
      selected: false,
      flashedAt: now,
      now,
      cursorBg: '#283457',
    });
    assert.equal(e.edge, false);
    assert.ok(e.backgroundColor);
    assert.notEqual(e.backgroundColor, '#283457');
  });

  it('search match uses searchBg when not selected', () => {
    const e = treeRowEmphasis({
      selected: false,
      now: 1000,
      searchMatch: true,
      searchBg: '#bb9af7',
      cursorBg: '#283457',
    });
    assert.equal(e.edge, false);
    assert.equal(e.backgroundColor, '#bb9af7');
  });

  it('selected wins over search match', () => {
    const e = treeRowEmphasis({
      selected: true,
      now: 1000,
      searchMatch: true,
      searchBg: '#bb9af7',
      cursorBg: '#283457',
    });
    assert.equal(e.edge, true);
    assert.equal(e.backgroundColor, '#283457');
  });
});
