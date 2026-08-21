import assert from 'node:assert';
import { describe, it } from 'node:test';
import { applyPageMove, clampIndex, pageDelta } from '../src/tui/pageNav.js';
import {
  createKeyState,
  handleKey,
  pageKeyFlagsFromInput,
} from '../src/tui/keys.js';

describe('applyPageMove', () => {
  it('clamps at ends (B8)', () => {
    assert.equal(applyPageMove(0, 100, 10, -1), 0);
    assert.equal(applyPageMove(95, 100, 10, 1), 99);
    assert.equal(applyPageMove(0, 0, 10, 1), 0);
  });
});

describe('pageDelta / clampIndex', () => {
  it('pageDelta leaves one-row overlap', () => {
    assert.equal(pageDelta(11), 10);
    assert.equal(pageDelta(1), 1);
  });

  it('clampIndex handles empty and edges', () => {
    assert.equal(clampIndex(-1, 0), 0);
    assert.equal(clampIndex(5, 3), 2);
  });
});

describe('pageKeyFlagsFromInput', () => {
  it('detects standard PageUp/PageDown CSI', () => {
    assert.deepEqual(pageKeyFlagsFromInput('\x1b[5~'), { pageUp: true });
    assert.deepEqual(pageKeyFlagsFromInput('\x1b[6~'), { pageDown: true });
  });

  it('detects legacy double-bracket PageUp/PageDown CSI', () => {
    assert.deepEqual(pageKeyFlagsFromInput('\x1b[[5~'), { pageUp: true });
    assert.deepEqual(pageKeyFlagsFromInput('\x1b[[6~'), { pageDown: true });
  });

  it('detects CSI after Ink useInput strips the leading ESC', () => {
    assert.deepEqual(pageKeyFlagsFromInput('[5~'), { pageUp: true });
    assert.deepEqual(pageKeyFlagsFromInput('[6~'), { pageDown: true });
    assert.deepEqual(pageKeyFlagsFromInput('[[5~'), { pageUp: true });
    assert.deepEqual(pageKeyFlagsFromInput('[[6~'), { pageDown: true });
  });

  it('returns empty flags for unrelated input', () => {
    assert.deepEqual(pageKeyFlagsFromInput('j'), {});
    assert.deepEqual(pageKeyFlagsFromInput('\x1b[A'), {});
    assert.deepEqual(pageKeyFlagsFromInput('[A'), {});
    assert.deepEqual(pageKeyFlagsFromInput(''), {});
  });
});

describe('page keys', () => {
  it('PageUp emits pageMove -1', () => {
    const r = handleKey(createKeyState(), '', { pageUp: true }, 'file');
    assert.deepEqual(r.action, { type: 'pageMove', deltaPages: -1 });
  });

  it('PageDown emits pageMove +1', () => {
    const r = handleKey(createKeyState(), '', { pageDown: true }, 'file');
    assert.deepEqual(r.action, { type: 'pageMove', deltaPages: 1 });
  });

  it('PageUp CSI input with missing Ink flags still pages via handleKey', () => {
    const flags = pageKeyFlagsFromInput('\x1b[5~');
    const r = handleKey(createKeyState(), '\x1b[5~', flags, 'file');
    assert.deepEqual(r.action, { type: 'pageMove', deltaPages: -1 });
  });

  it('PageDown CSI input with missing Ink flags still pages via handleKey', () => {
    const flags = pageKeyFlagsFromInput('\x1b[[6~');
    const r = handleKey(createKeyState(), '\x1b[[6~', flags, 'file');
    assert.deepEqual(r.action, { type: 'pageMove', deltaPages: 1 });
  });

  it('Ink-stripped PageUp CSI with missing flags still pages via handleKey', () => {
    const flags = pageKeyFlagsFromInput('[5~');
    const r = handleKey(createKeyState(), '[5~', flags, 'file');
    assert.deepEqual(r.action, { type: 'pageMove', deltaPages: -1 });
  });

  it('page flags win over escape when both are set', () => {
    const r = handleKey(
      createKeyState(),
      '[5~',
      { escape: true, pageUp: true },
      'file',
    );
    assert.deepEqual(r.action, { type: 'pageMove', deltaPages: -1 });
  });
});
