import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  HELP_CHROME_COLS,
  HELP_COLUMN_COUNT,
  HELP_KEY_WIDTH,
  helpBodyLineCount,
  helpChipPadWidth,
  helpColumnWidth,
  helpDescLayout,
  helpEntryVisualLines,
  helpInnerWidth,
  helpOverlayRowCount,
  wrapHelpDescription,
  wrapHelpFooter,
} from '../src/tui/helpLayout.js';

describe('helpColumnWidth', () => {
  it('shares inner width across three columns', () => {
    const termWidth = 128;
    const inner = helpInnerWidth(termWidth);
    const col = helpColumnWidth(termWidth);
    assert.equal(inner, termWidth - HELP_CHROME_COLS);
    assert.equal(col, Math.floor(inner / HELP_COLUMN_COUNT));
    assert.ok(col > 40, 'wider than the old fixed 40-col column');
  });

  it('grows description columns on a wider terminal', () => {
    assert.ok(helpColumnWidth(200) > helpColumnWidth(128));
    assert.ok(helpColumnWidth(80) < helpColumnWidth(128));
  });
});

describe('wrapHelpDescription', () => {
  it('word-wraps on spaces without ellipsis', () => {
    const lines = wrapHelpDescription('full-file · keep hunk in view', 12);
    assert.ok(lines.length > 1);
    for (const line of lines) {
      assert.ok(line.length <= 12, `overflow: ${JSON.stringify(line)}`);
      assert.ok(!line.includes('…'), `ellipsis: ${JSON.stringify(line)}`);
    }
    assert.equal(lines.join(' '), 'full-file · keep hunk in view');
  });

  it('breaks an overlong word instead of clipping it', () => {
    const lines = wrapHelpDescription('abcdefghijklmnopqrstuvwxyz', 10);
    assert.deepEqual(lines, ['abcdefghij', 'klmnopqrst', 'uvwxyz']);
  });

  it('keeps a short phrase on one line', () => {
    assert.deepEqual(wrapHelpDescription('down / up', 40), ['down / up']);
  });
});

describe('helpDescLayout', () => {
  it('puts the description beside chips when the column is wide enough', () => {
    const layout = helpDescLayout(40);
    assert.equal(layout.indent, HELP_KEY_WIDTH);
    assert.equal(layout.width, 40 - HELP_KEY_WIDTH);
    assert.equal(layout.descOnFirstLine, true);
  });

  it('wraps the description below chips when the column is narrower than the chip pad', () => {
    const layout = helpDescLayout(12);
    assert.equal(layout.descOnFirstLine, false);
    assert.equal(layout.indent, 0);
    assert.equal(layout.width, 12);
  });
});

describe('helpChipPadWidth', () => {
  it('grows past HELP_KEY_WIDTH for a wide chip cluster', () => {
    const pad = helpChipPadWidth('Ctrl-Space extra extra extra');
    assert.ok(pad > HELP_KEY_WIDTH);
  });
});

describe('helpEntryVisualLines', () => {
  it('keeps chips on the first line and indents every wrapped description line', () => {
    const col = 40;
    const lines = helpEntryVisualLines('mouse · drag divider to resize', col);
    assert.ok(lines.length > 1);
    assert.equal(lines[0]?.chips, true);
    assert.equal(lines[0]?.indent, 0);
    for (const line of lines.slice(1)) {
      assert.equal(line.chips, false);
      assert.equal(line.indent, HELP_KEY_WIDTH);
      assert.ok(line.text.length > 0);
    }
    assert.equal(lines.map((l) => l.text).join(' '), 'mouse · drag divider to resize');
  });

  it('sizes wrap from the painted chip pad when chips exceed HELP_KEY_WIDTH', () => {
    const keys = 'Ctrl-Space extra extra extra';
    const pad = helpChipPadWidth(keys);
    const col = 40;
    const lines = helpEntryVisualLines('toggle subtree now with extra words', col, keys);
    assert.ok(pad > HELP_KEY_WIDTH);
    assert.ok(lines.length > 1);
    assert.ok((lines[0]?.text.length ?? 0) <= col - pad);
    for (const line of lines.slice(1)) {
      assert.equal(line.indent, pad);
      assert.ok(line.text.length <= col - pad);
    }
  });
});

describe('helpOverlayRowCount', () => {
  it('counts border, title, body, and footer', () => {
    assert.equal(helpOverlayRowCount(11, 1), 2 + 1 + 11 + 1);
    assert.equal(helpOverlayRowCount(14, 2), 2 + 1 + 14 + 2);
  });
});

describe('helpBodyLineCount', () => {
  it('uses the tallest wrapped cell in each aligned row', () => {
    const groups = [
      { keys: [['j k', 'down / up'] as const] },
      { keys: [['Ctrl-o', 'full-file · keep hunk in view'] as const] },
      { keys: [['?', 'this help'] as const] },
    ];
    const wide = helpBodyLineCount(groups, 80);
    const narrow = helpBodyLineCount(groups, 28);
    assert.equal(wide, 1);
    assert.ok(narrow > wide);
  });
});

describe('wrapHelpFooter', () => {
  it('wraps a long footer instead of returning one clipped line', () => {
    const footer = 'Needs a Nerd Font · MesloLGM Nerd Font Mono · / search help · Esc closes';
    const lines = wrapHelpFooter(footer, 36);
    assert.ok(lines.length > 1);
    for (const line of lines) {
      assert.ok(line.length <= 36, `overflow: ${JSON.stringify(line)}`);
      assert.ok(!line.includes('…'));
    }
  });
});
