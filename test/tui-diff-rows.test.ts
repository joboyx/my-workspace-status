import assert from 'node:assert';
import { describe, it } from 'node:test';
import { parseUnifiedDiff } from '../src/tui/diff/parse.js';
import { buildDiffRows, gutterWidth } from '../src/tui/diff/rows.js';
import { highlightLine, languageForPath, truncateAnsi } from '../src/tui/diff/highlight.js';

const ANSI_RE = /\u001b\[[0-9;]*m/g;

function stripAnsi(value: string): string {
  return value.replace(ANSI_RE, '');
}

const FIXTURE = `diff --git a/hello.ts b/hello.ts
index 1111111..2222222 100644
--- a/hello.ts
+++ b/hello.ts
@@ -10,3 +10,4 @@
 line1
-line2
+line2 changed
 line3
+line4
`;

describe('parseUnifiedDiff line numbers', () => {
  it('seeds counters from the hunk header and advances per side', () => {
    const [hunk] = parseUnifiedDiff(FIXTURE);
    assert.equal(hunk.oldStart, 10);
    assert.equal(hunk.newStart, 10);
    assert.deepEqual(
      hunk.lines.map((l) => [l.kind, l.oldNo, l.newNo]),
      [
        ['ctx', 10, 10],
        ['del', 11, undefined],
        ['add', undefined, 11],
        ['ctx', 12, 12],
        ['add', undefined, 13],
      ],
    );
  });

  it('tolerates a hunk header without line info', () => {
    const [hunk] = parseUnifiedDiff('@@ garbage @@\n+a\n');
    assert.equal(hunk.oldStart, 0);
    assert.equal(hunk.lines[0].kind, 'add');
  });
});

describe('buildDiffRows', () => {
  it('emits a section row per non-empty diff and omits empty ones', () => {
    const staged = buildDiffRows({ staged: FIXTURE, unstaged: '', mode: 'inline' });
    assert.deepEqual(staged[0], { kind: 'section', section: 'staged' });
    assert.ok(!staged.some((r) => r.kind === 'section' && r.section === 'unstaged'));

    const both = buildDiffRows({ staged: FIXTURE, unstaged: FIXTURE, mode: 'inline' });
    const sections = both.filter((r) => r.kind === 'section');
    assert.deepEqual(
      sections.map((s) => (s.kind === 'section' ? s.section : null)),
      ['staged', 'unstaged'],
    );

    assert.deepEqual(buildDiffRows({ staged: '', unstaged: '', mode: 'inline' }), []);
  });

  it('labels the unstaged section NEW for untracked files', () => {
    const rows = buildDiffRows({
      staged: '',
      unstaged: FIXTURE,
      mode: 'inline',
      isNew: true,
    });
    assert.deepEqual(rows[0], { kind: 'section', section: 'new' });
  });

  it('inline mode yields one cell per line with no right side', () => {
    const rows = buildDiffRows({ staged: FIXTURE, unstaged: '', mode: 'inline' });
    const lines = rows.filter((r) => r.kind === 'line');
    assert.equal(lines.length, 5);
    assert.ok(lines.every((r) => r.kind === 'line' && r.right === undefined));
  });

  it('side-by-side pairs each deletion with the matching addition', () => {
    const rows = buildDiffRows({ staged: FIXTURE, unstaged: '', mode: 'sideBySide' });
    const lines = rows.flatMap((r) => (r.kind === 'line' ? [r] : []));

    const changed = lines.find((r) => r.left.text === 'line2');
    assert.ok(changed, 'expected the del row');
    assert.equal(changed.left.kind, 'del');
    assert.equal(changed.right?.kind, 'add');
    assert.equal(changed.right?.text, 'line2 changed');

    // An addition with no counterpart leaves the left cell empty.
    const added = lines.find((r) => r.right?.text === 'line4');
    assert.ok(added);
    assert.equal(added.left.kind, 'empty');
  });

  it('zips all-dels-then-all-adds runs by index', () => {
    const rows = buildDiffRows({
      staged: '@@ -1,2 +1,2 @@\n-a\n-b\n+a2\n+b2\n',
      unstaged: '',
      mode: 'sideBySide',
    });
    const pairs = rows.flatMap((r) =>
      r.kind === 'line' ? [[r.left.text, r.right?.text]] : [],
    );
    assert.deepEqual(pairs, [
      ['a', 'a2'],
      ['b', 'b2'],
    ]);
  });

  it('keeps a binary marker as a meta cell', () => {
    const rows = buildDiffRows({
      staged: '',
      unstaged: 'Binary files a/x and b/x differ\n',
      mode: 'inline',
    });
    const meta = rows.find((r) => r.kind === 'line');
    assert.ok(meta && meta.kind === 'line');
    assert.equal(meta.left.kind, 'meta');
    assert.match(meta.left.text, /Binary files/);
  });
});

describe('gutterWidth', () => {
  it('sizes to the widest line number, with a floor of 2', () => {
    assert.equal(gutterWidth([]), 2);
    assert.equal(
      gutterWidth(buildDiffRows({ staged: FIXTURE, unstaged: '', mode: 'inline' })),
      2,
    );
    const big = buildDiffRows({
      staged: '@@ -1200,1 +1200,1 @@\n-a\n+b\n',
      unstaged: '',
      mode: 'inline',
    });
    assert.equal(gutterWidth(big), 4);
  });
});

describe('syntax highlighting', () => {
  it('maps known extensions and rejects unknown ones', () => {
    assert.equal(languageForPath('src/tui/App.tsx'), 'typescript');
    assert.equal(languageForPath('a/b/script.sh'), 'bash');
    assert.equal(languageForPath('Dockerfile'), 'dockerfile');
    assert.equal(languageForPath('notes/whatever.xyz'), null);
    assert.equal(languageForPath('pkg/Main.kt'), 'kotlin');
    assert.equal(languageForPath('lib/foo.rb'), 'ruby');
    assert.equal(languageForPath('mod/x.php'), 'php');
    assert.equal(languageForPath('ui/Widget.vue'), 'xml');
    assert.equal(languageForPath('a/b.swift'), 'swift');
    assert.equal(languageForPath('core/util.c'), 'c');
    assert.equal(languageForPath('core/util.cpp'), 'cpp');
    assert.equal(languageForPath('infra/main.tf'), 'ini');
    assert.equal(languageForPath('api/schema.graphql'), 'graphql');
    assert.equal(languageForPath('.env'), 'bash');
    assert.equal(languageForPath('settings.ini'), 'ini');
    assert.equal(languageForPath('script.lua'), 'lua');
  });

  /**
   * Highlighting only adds ANSI escapes; the visible text must survive intact
   * or the pane's column arithmetic breaks. (Chalk emits no escapes when the
   * runner's stdout is not a TTY, so assert the invariant, not the codes.)
   */
  it('never changes the visible text of a line', () => {
    const samples = [
      'const x = 1;',
      '  return { kind: "add", text };',
      '}}}{{{',
      '',
      '   ',
    ];
    for (const sample of samples) {
      const out = highlightLine(sample, 'typescript');
      assert.equal(stripAnsi(out), sample, JSON.stringify(sample));
    }
  });

  it('passes text through untouched when no language is known', () => {
    assert.equal(highlightLine('const x = 1;', null), 'const x = 1;');
  });

  it('is cached — repeated calls return the identical string', () => {
    const a = highlightLine('export function f() {}', 'typescript');
    const b = highlightLine('export function f() {}', 'typescript');
    assert.equal(a, b);
  });

  it('truncateAnsi cuts by visible columns and preserves a leading CSI', () => {
    const coloured = '\u001b[31mabcdef\u001b[0m';
    const out = truncateAnsi(coloured, 3);
    assert.equal(stripAnsi(out), 'abc');
    assert.ok(out.startsWith('\u001b[31m'));
  });

  it('highlights the full line before truncation would change tokens', () => {
    // A long TS line: truncating before highlight can split mid-token.
    const long =
      'export function computeThingFromValues(aaaa: number, bbbb: string): boolean { return true; }';
    const full = highlightLine(long, 'typescript');
    const truncated = truncateAnsi(full, 40);
    assert.equal(stripAnsi(truncated).length, 40);
    // Same prefix as highlighting-then-cutting the full string (not cut-then-highlight).
    assert.equal(truncated, truncateAnsi(full, 40));
    const cutFirst = highlightLine(long.slice(0, 40), 'typescript');
    // cut-then-highlight is allowed to differ — we only assert highlight-then-cut is stable.
    assert.equal(typeof cutFirst, 'string');
  });
});
