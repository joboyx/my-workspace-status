import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  HELP_GROUPS,
  HELP_IDLE_FOOTER_SNIPPET,
  HELP_KEY_WIDTH,
  HELP_SEARCH_ESC_HINT,
  SEARCH_TYPING_HINT,
  helpStatusLines,
} from '../src/tui/StatusBar.js';
import { diffModeUserLabel } from '../src/tui/diffModeLabel.js';

/** Every chip token shown in the `?` help overlay. */
function helpChips(): string[] {
  return HELP_GROUPS.flatMap((group) =>
    group.keys.flatMap(([keys]) => keys.split(' ').filter(Boolean)),
  );
}

describe('HELP_GROUPS', () => {
  it('documents theme cycle, full-file, Esc, and mouse bindings', () => {
    const chips = new Set(helpChips());
    for (const required of ['T', 'Ctrl-o', 'Esc', 'm', 't', 'i', '.', 'Ctrl-C', '?', 'dblclick']) {
      assert.ok(chips.has(required), `help missing chip: ${required}`);
    }
  });

  it('documents git scope and workspace actions', () => {
    const chips = new Set(helpChips());
    for (const required of ['s', 'S', 'u', 'x', 'e', 'space', 'f', 'p', 'P', 'd', 'b', 'r']) {
      assert.ok(chips.has(required), `help missing chip: ${required}`);
    }
  });

  it('documents depth-0 picker and graph origin checkout for b', () => {
    const row = HELP_GROUPS.flatMap((g) => g.keys).find(([keys]) => keys === 'b');
    assert.ok(row, 'help missing b row');
    const [, desc] = row;
    assert.match(desc, /picker/i);
    assert.match(desc, /origin/);
  });

  it('documents EasyMotion on the focused list', () => {
    const row = HELP_GROUPS.flatMap((g) => g.keys).find(([keys]) => keys.includes('Ctrl-Space'));
    assert.ok(row);
    assert.match(row![0], /;/);
    assert.match(row![1], /focused list/i);
  });

  it('documents instant fold and drops z-chord chips', () => {
    const chips = new Set(helpChips());
    assert.ok(chips.has('space'));
    assert.ok(chips.has('z'));
    for (const gone of ['za', 'zo', 'zc', 'zR', 'zM']) {
      assert.ok(!chips.has(gone), `stale chord chip: ${gone}`);
    }
    const fold = HELP_GROUPS.flatMap((g) => g.keys).find(([keys]) => keys === 'z');
    assert.ok(fold);
    assert.match(fold![1], /no-op on graph\/diff/i);
    const reviewed = HELP_GROUPS.flatMap((g) => g.keys).find(([keys]) => keys === 'space');
    assert.ok(reviewed);
    assert.match(reviewed![1], /reviewed/i);
    assert.match(reviewed![1], /eye/i);
    assert.ok(!HELP_GROUPS.flatMap((g) => g.keys).some(([keys]) => keys === 'v'));
  });

  it('sizes the overlay from terminal width so wrapped copy grows height', () => {
    const rowCount = Math.max(...HELP_GROUPS.map((g) => g.keys.length));
    const wide = helpStatusLines(300);
    const mid = helpStatusLines(128);
    const narrow = helpStatusLines(80);
    assert.equal(wide, 2 + 1 + rowCount + 1);
    assert.ok(mid > wide, '128 cols still wraps some descriptions');
    assert.ok(narrow > mid, 'narrow terminals wrap more and take more rows');
  });

  it('reserves chip column width for separation', () => {
    assert.ok(HELP_KEY_WIDTH >= 18);
  });

  it('documents help-local / search in the overlay footer contract', () => {
    const chips = new Set(helpChips());
    assert.ok(chips.has('/'), '/ chip still documented');
    assert.match(HELP_IDLE_FOOTER_SNIPPET, /\/ search help/);
    assert.match(HELP_SEARCH_ESC_HINT, /Esc clears search/);
  });

  it('says Enter arms the query before n/N step matches', () => {
    const slash = HELP_GROUPS.flatMap((g) => g.keys).find(([keys]) => keys === '/');
    const np = HELP_GROUPS.flatMap((g) => g.keys).find(([keys]) => keys === 'n N');
    assert.ok(slash);
    assert.ok(np);
    assert.match(slash![1], /Enter arms/);
    assert.match(np![1], /after Enter/);
    assert.match(SEARCH_TYPING_HINT, /Enter arms query/);
    assert.match(SEARCH_TYPING_HINT, /n\/N after Enter/);
    assert.ok(!SEARCH_TYPING_HINT.includes('n/N next/prev'));
  });

  it('uses split (not side-by-side) for the i-key help copy', () => {
    const view = HELP_GROUPS.find((g) => g.title === 'VIEW');
    assert.ok(view);
    const row = view.keys.find(([keys]) => keys === 'i');
    assert.ok(row);
    assert.equal(row[1], 'inline / split');
    assert.equal(row[1], `inline / ${diffModeUserLabel('sideBySide')}`);
  });

  it('documents the ignored-repo visibility toggle', () => {
    const view = HELP_GROUPS.find((g) => g.title === 'VIEW');
    assert.ok(view);
    const row = view.keys.find(([keys]) => keys === '.');
    assert.ok(row, 'help missing . row');
    assert.match(row[1], /ignored/i);
  });
});
