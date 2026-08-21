import assert from 'node:assert';
import { describe, it } from 'node:test';
import { helpEntryLabel, helpEntryMatches } from '../src/tui/helpSearch.js';
import { HELP_GROUPS } from '../src/tui/StatusBar.js';

describe('helpEntryLabel', () => {
  it('joins keys and description', () => {
    assert.equal(helpEntryLabel('j k', 'down / up'), 'j k down / up');
  });
});

describe('helpEntryMatches', () => {
  it('matches case-insensitive substrings on keys or description', () => {
    assert.equal(helpEntryMatches('Ctrl-o', 'full-file · keep hunk in view', 'ctrl'), true);
    assert.equal(helpEntryMatches('Ctrl-o', 'full-file · keep hunk in view', 'HUNK'), true);
    assert.equal(helpEntryMatches('Ctrl-o', 'full-file · keep hunk in view', 'zzz'), false);
  });

  it('treats empty or whitespace query as no match', () => {
    assert.equal(helpEntryMatches('j k', 'down / up', ''), false);
    assert.equal(helpEntryMatches('j k', 'down / up', '   '), false);
  });

  it('finds real HELP_GROUPS rows by description fragment', () => {
    const hits = HELP_GROUPS.flatMap((g) =>
      g.keys.filter(([keys, desc]) => helpEntryMatches(keys, desc, 'easymo')),
    );
    assert.ok(hits.some(([keys]) => keys.includes('Ctrl-Space')));
  });
});
