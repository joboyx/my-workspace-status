import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';
import { easyMotionPaintSlot } from '../src/tui/activeContext.js';
import { commitDetailHeaderHeight } from '../src/tui/CommitDetailPane.js';
import {
  easyMotionLabels,
  resolveEasyMotionJump,
  resolveEasyMotionLabel,
} from '../src/tui/easyMotion.js';
import { createKeyState, handleKey } from '../src/tui/keys.js';

const SRC_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), '../src/tui');

describe('easyMotionLabels', () => {
  it('uses a-z then aa…', () => {
    const labels = easyMotionLabels(28);
    assert.equal(labels[0], 'a');
    assert.equal(labels[25], 'z');
    assert.equal(labels[26], 'aa');
    assert.equal(labels[27], 'ab');
  });
});

describe('resolveEasyMotionLabel', () => {
  it('hits, partials, and misses', () => {
    const labels = easyMotionLabels(28);
    assert.deepEqual(resolveEasyMotionLabel(labels, 'a'), { status: 'hit', index: 0 });
    assert.deepEqual(resolveEasyMotionLabel(labels, 'a'), { status: 'hit', index: 0 });
    // 'a' alone is exact hit for single-letter; two-char prefix:
    assert.equal(resolveEasyMotionLabel(labels, 'aa').status, 'hit');
    assert.equal(resolveEasyMotionLabel(labels, 'q').status, 'hit');
    assert.equal(resolveEasyMotionLabel(['aa', 'ab'], 'a').status, 'partial');
    assert.equal(resolveEasyMotionLabel(labels, 'zz').status, 'miss');
  });
});

describe('EasyMotion start keys', () => {
  it('starts EasyMotion from Ink Ctrl+Space encodings and semicolon', () => {
    const cases: Array<[string, { ctrl?: boolean }]> = [
      [' ', { ctrl: true }],
      ['space', { ctrl: true }],
      ['`', { ctrl: true }],
      ['\0', {}],
      [';', {}],
    ];
    for (const [input, key] of cases) {
      const r = handleKey(createKeyState(), input, key, 'repo');
      assert.deepEqual(
        r.action,
        { type: 'easyMotionStart' },
        `expected easyMotionStart for ${JSON.stringify(input)}`,
      );
    }
  });

  it('bare space marks reviewed on a file and no-ops on other rows', () => {
    assert.deepEqual(handleKey(createKeyState(), ' ', {}, 'file').action, { type: 'toggleViewed' });
    assert.deepEqual(handleKey(createKeyState(), ' ', {}, 'repo').action, { type: 'none' });
  });

  it('does not start EasyMotion on bare backtick, Ctrl+C, or during search', () => {
    assert.notDeepEqual(handleKey(createKeyState(), '`', {}, 'repo').action, {
      type: 'easyMotionStart',
    });
    assert.notDeepEqual(handleKey(createKeyState(), 'c', { ctrl: true }, 'repo').action, {
      type: 'easyMotionStart',
    });
    const searching = { ...createKeyState(), searchMode: true };
    assert.deepEqual(handleKey(searching, ';', {}, 'repo').action, {
      type: 'none',
    });
  });
});

describe('easyMotionPaintSlot', () => {
  it('paints glyphs only on the focused jump list', () => {
    assert.equal(
      easyMotionPaintSlot({ depth: 0, focusPane: 'left', graphVisible: true }),
      'leftTree',
    );
    assert.equal(
      easyMotionPaintSlot({ depth: 0, focusPane: 'right', graphVisible: true }),
      'rightGraph',
    );
    assert.equal(
      easyMotionPaintSlot({ depth: 1, focusPane: 'left', graphVisible: true }),
      'leftGraph',
    );
    assert.equal(
      easyMotionPaintSlot({ depth: 1, focusPane: 'right', graphVisible: true }),
      'rightCommitFiles',
    );
    assert.equal(
      easyMotionPaintSlot({ depth: 2, focusPane: 'left', graphVisible: true }),
      'leftCommitFiles',
    );
  });

  it('is null on a focused diff', () => {
    assert.equal(easyMotionPaintSlot({ depth: 0, focusPane: 'right', graphVisible: false }), null);
    assert.equal(easyMotionPaintSlot({ depth: 2, focusPane: 'right', graphVisible: true }), null);
  });
});

describe('commitDetailHeaderHeight', () => {
  it('reserves title/subtitle so EasyMotion matches the file-list window', () => {
    assert.equal(commitDetailHeaderHeight(20, 'abc123', 'subject line'), 2);
    assert.equal(commitDetailHeaderHeight(20, 'abc123'), 1);
    assert.equal(commitDetailHeaderHeight(20, '', ''), 1);
  });
});

describe('resolveEasyMotionJump', () => {
  it('maps the first visible label to the viewport start index', () => {
    assert.deepEqual(resolveEasyMotionJump(20, 30, 'a'), { status: 'hit', index: 30 });
    assert.deepEqual(resolveEasyMotionJump(20, 30, 'b'), { status: 'hit', index: 31 });
  });

  it('hits two-character labels at start plus the two-letter slot', () => {
    assert.equal(resolveEasyMotionJump(28, 0, 'a').status, 'hit');
    assert.deepEqual(resolveEasyMotionJump(28, 10, 'aa'), { status: 'hit', index: 36 });
    assert.equal(resolveEasyMotionJump(28, 0, 'zz').status, 'miss');
  });
});

describe('EasyMotion wiring (paint only the focused slot)', () => {
  it('App gates pane EasyMotion props on easyMotionPaintSlot', () => {
    const src = fs.readFileSync(path.join(SRC_DIR, 'App.tsx'), 'utf8');
    assert.match(src, /easyMotion=\{motionSlot === 'leftTree' && state\.easyMotion\}/);
    assert.match(src, /easyMotion=\{motionSlot === 'leftGraph' && state\.easyMotion\}/);
    assert.match(src, /easyMotion=\{motionSlot === 'leftCommitFiles' && state\.easyMotion\}/);
    assert.match(src, /motionSlot === 'rightGraph' \|\| motionSlot === 'rightCommitFiles'/);
  });

  it('useAppState no-ops easyMotionStart when the focused pane is a diff', () => {
    const src = fs.readFileSync(path.join(SRC_DIR, 'useAppState.ts'), 'utf8');
    assert.match(
      src,
      /case 'easyMotionStart':\s*\{\s*if\s*\(\s*easyMotionListTarget\(/,
      'start must call easyMotionListTarget and return when the focused pane is a diff',
    );
  });

  it('useAppState jumps against the painted tree/graph window', () => {
    const src = fs.readFileSync(path.join(SRC_DIR, 'useAppState.ts'), 'utf8');
    assert.match(
      src,
      /const win = visibleTreeWindow\(list, listCursor, h\);/,
      'tree/commit-file jump must use the same window TreePane paints',
    );
    assert.match(
      src,
      /const resolved = resolveEasyMotionJump\(visibleCount, start, typed\);/,
      'typed labels must resolve against that painted window',
    );
  });
});
