import assert from 'node:assert';
import { describe, it } from 'node:test';
import { parseNameStatusLines, parseNameStatusZ } from '../src/tui/commitFiles/parseNameStatus.js';
import { statusLetterFromChange } from '../src/tui/icons.js';

describe('parseNameStatusLines', () => {
  it('maps A/M/D paths onto unstagedStatus', () => {
    const changes = parseNameStatusLines('A\tsrc/a.ts\nM\tsrc/b.ts\nD\told.ts\n');
    assert.deepEqual(
      changes.map((c) => ({ path: c.path, unstagedStatus: c.unstagedStatus })),
      [
        { path: 'src/a.ts', unstagedStatus: 'A' },
        { path: 'src/b.ts', unstagedStatus: 'M' },
        { path: 'old.ts', unstagedStatus: 'D' },
      ],
    );
  });

  it('maps renames with oldPath', () => {
    const changes = parseNameStatusLines('R100\told/name.ts\tnew/name.ts\n');
    assert.equal(changes.length, 1);
    assert.equal(changes[0].path, 'new/name.ts');
    assert.equal(changes[0].oldPath, 'old/name.ts');
    assert.equal(changes[0].unstagedStatus, 'R');
  });

  it('statusLetterFromChange keeps A/M/D/R (not S for M)', () => {
    const changes = parseNameStatusLines('A\ta.ts\nM\tb.ts\nD\tc.ts\nR100\told\tnew\n');
    assert.deepEqual(
      changes.map((c) => statusLetterFromChange(c)),
      ['A', 'M', 'D', 'R'],
    );
  });
});

describe('parseNameStatusZ', () => {
  it('parses NUL records including rename triples', () => {
    const raw = ['M', 'a.ts', 'R100', 'old.ts', 'new.ts', ''].join('\0');
    const changes = parseNameStatusZ(raw);
    assert.equal(changes.length, 2);
    assert.equal(changes[0].path, 'a.ts');
    assert.equal(changes[0].unstagedStatus, 'M');
    assert.equal(changes[1].path, 'new.ts');
    assert.equal(changes[1].oldPath, 'old.ts');
    assert.equal(statusLetterFromChange(changes[0]), 'M');
  });
});
