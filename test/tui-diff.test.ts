import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  buildDiffPaneLines,
  parseUnifiedDiff,
} from '../src/tui/diff/parse.js';
import { synthesizeAllAddDiff } from '../src/tui/diff/newFile.js';
import { renderInline } from '../src/tui/diff/inline.js';
import { renderSideBySide } from '../src/tui/diff/sideBySide.js';

const FIXTURE = `diff --git a/hello.txt b/hello.txt
index 1111111..2222222 100644
--- a/hello.txt
+++ b/hello.txt
@@ -1,3 +1,4 @@
 line1
-line2
+line2 changed
 line3
+line4
`;

describe('parseUnifiedDiff', () => {
  it('parses hunk header and ctx/add/del line kinds', () => {
    const hunks = parseUnifiedDiff(FIXTURE);
    assert.equal(hunks.length, 1);
    assert.equal(hunks[0].header, '@@ -1,3 +1,4 @@');
    assert.deepEqual(
      hunks[0].lines.map((l) => l.kind),
      ['ctx', 'del', 'add', 'ctx', 'add'],
    );
    assert.deepEqual(
      hunks[0].lines.map((l) => l.text),
      ['line1', 'line2', 'line2 changed', 'line3', 'line4'],
    );
  });

  it('returns a binary stub hunk for Binary files … differ', () => {
    const hunks = parseUnifiedDiff('Binary files a/foo.bin and b/foo.bin differ\n');
    assert.equal(hunks.length, 1);
    assert.equal(hunks[0].header, '');
    assert.deepEqual(hunks[0].lines, [
      { kind: 'meta', text: 'Binary files a/foo.bin and b/foo.bin differ' },
    ]);
  });

  it('returns empty array for empty input', () => {
    assert.deepEqual(parseUnifiedDiff(''), []);
    assert.deepEqual(parseUnifiedDiff('   \n'), []);
  });
});

describe('renderInline', () => {
  it('renders hunk header and +/-/space prefixed lines', () => {
    const lines = renderInline(parseUnifiedDiff(FIXTURE));
    assert.deepEqual(lines, [
      '@@ -1,3 +1,4 @@',
      ' line1',
      '-line2',
      '+line2 changed',
      ' line3',
      '+line4',
    ]);
  });
});

describe('renderSideBySide', () => {
  it('splits columns at even width with | separator', () => {
    const width = 40;
    const lines = renderSideBySide(parseUnifiedDiff(FIXTURE), width);
    assert.ok(lines.length > 0);
    for (const line of lines) {
      assert.equal(line.length, width, `line length ${line.length}: ${JSON.stringify(line)}`);
      const mid = Math.floor(width / 2);
      // Separator sits just left of the right half start when width even:
      // left = width/2 - 1? Prefer: leftCol = floor((width-1)/2), sep, right = rest
      assert.equal(line[Math.floor((width - 1) / 2)], '|');
    }
    // Header spans full row as meta (both sides or left-only with empty right)
    assert.ok(lines[0].includes('@@ -1,3 +1,4 @@'));
    // del/add pair on same visual row
    const changeRow = lines.find((l) => l.includes('line2') && l.includes('line2 changed'));
    assert.ok(changeRow, 'expected paired del/add row');
  });

  it('zips all-dels-then-all-adds runs by index', () => {
    const diff = `@@ -1,2 +1,2 @@
-a
-b
+a2
+b2
`;
    const lines = renderSideBySide(parseUnifiedDiff(diff), 40);
    const rowA = lines.find((l) => l.includes('-a') && l.includes('+a2'));
    const rowB = lines.find((l) => l.includes('-b') && l.includes('+b2'));
    assert.ok(rowA, 'expected -a|+a2 pair');
    assert.ok(rowB, 'expected -b|+b2 pair');
  });
});

describe('buildDiffPaneLines', () => {
  it('inserts STAGED / UNSTAGED headers and omits empty sections', () => {
    const lines = buildDiffPaneLines({
      staged: FIXTURE,
      unstaged: '',
      mode: 'inline',
      width: 80,
    });
    assert.equal(lines[0], 'STAGED');
    assert.ok(lines.includes('@@ -1,3 +1,4 @@'));
    assert.ok(!lines.some((l) => l.includes('UNSTAGED')));

    const both = buildDiffPaneLines({
      staged: FIXTURE,
      unstaged: FIXTURE,
      mode: 'inline',
      width: 80,
    });
    assert.equal(both[0], 'STAGED');
    const unstagedIdx = both.indexOf('UNSTAGED');
    assert.ok(unstagedIdx > 0);

    const empty = buildDiffPaneLines({
      staged: '',
      unstaged: '',
      mode: 'inline',
      width: 80,
    });
    assert.deepEqual(empty, []);
  });

  it('renders binary stub via pane builder', () => {
    const lines = buildDiffPaneLines({
      staged: '',
      unstaged: 'Binary files a/x and b/x differ\n',
      mode: 'inline',
      width: 80,
    });
    assert.equal(lines[0], 'UNSTAGED');
    assert.ok(lines.some((l) => l.includes('Binary files')));
  });

  it('supports sideBySide mode', () => {
    const lines = buildDiffPaneLines({
      staged: FIXTURE,
      unstaged: '',
      mode: 'sideBySide',
      width: 40,
    });
    assert.equal(lines[0], 'STAGED');
    const body = lines.slice(1);
    assert.ok(body.every((l) => l.length === 40));
  });
});

describe('synthesizeAllAddDiff', () => {
  it('builds all-add hunk for new file text', () => {
    const diff = synthesizeAllAddDiff('a\nb\n');
    const hunks = parseUnifiedDiff(diff);
    assert.equal(hunks.length, 1);
    assert.equal(hunks[0].header, '@@ -0,0 +1,2 @@');
    assert.deepEqual(
      hunks[0].lines.map((l) => l.kind),
      ['add', 'add'],
    );
    const pane = buildDiffPaneLines({
      staged: '',
      unstaged: diff,
      mode: 'inline',
      width: 80,
    });
    assert.equal(pane[0], 'UNSTAGED');
    assert.ok(pane.includes('+a'));
    assert.ok(pane.includes('+b'));
  });

  it('handles empty file', () => {
    const diff = synthesizeAllAddDiff('');
    assert.ok(diff.includes('@@'));
  });
});
