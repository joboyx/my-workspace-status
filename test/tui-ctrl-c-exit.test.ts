import assert from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  CTRL_C_EXIT_MS,
  CTRL_C_EXIT_PROMPT,
  handleCtrlC,
  isCtrlC,
} from '../src/tui/ctrlCExit.js';

describe('StatusBar standing quit hint', () => {
  it('keeps ? help only — no standing Ctrl-C×2 (ephemeral prompt owns quit UX)', () => {
    const src = readFileSync(
      join(dirname(fileURLToPath(import.meta.url)), '../src/tui/StatusBar.tsx'),
      'utf8',
    );
    assert.match(src, /const hint = zPending \? 'z…' : '\? help';/);
    assert.doesNotMatch(src, /Ctrl-C×2/);
  });
});

describe('handleCtrlC', () => {
  it('arms on first press and quits on second within the window', () => {
    const t0 = 1_000_000;
    const first = handleCtrlC({ armedUntil: 0 }, t0);
    assert.equal(first.quit, false);
    assert.equal(first.prompt, true);
    assert.equal(first.state.armedUntil, t0 + CTRL_C_EXIT_MS);

    const second = handleCtrlC(first.state, t0 + 50);
    assert.equal(second.quit, true);
    assert.equal(second.prompt, false);
    assert.equal(second.state.armedUntil, 0);
  });

  it('treats an expired arm as a fresh first press', () => {
    const t0 = 1_000_000;
    const armed = handleCtrlC({ armedUntil: 0 }, t0);
    const late = handleCtrlC(armed.state, t0 + CTRL_C_EXIT_MS);
    assert.equal(late.quit, false);
    assert.equal(late.prompt, true);
    assert.equal(late.state.armedUntil, t0 + CTRL_C_EXIT_MS + CTRL_C_EXIT_MS);
  });

  it('exports the harness-style prompt string', () => {
    assert.match(CTRL_C_EXIT_PROMPT, /Ctrl\+C again/i);
  });
});

describe('isCtrlC', () => {
  it('matches Ink ctrl+c and raw ETX', () => {
    assert.equal(isCtrlC('c', true), true);
    assert.equal(isCtrlC('\x03', false), true);
    assert.equal(isCtrlC('c', false), false);
    assert.equal(isCtrlC('d', true), false);
  });
});
