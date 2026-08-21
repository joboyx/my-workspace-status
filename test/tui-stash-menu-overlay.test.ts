import assert from 'node:assert';
import { describe, it } from 'node:test';
import { CTRL_C_EXIT_PROMPT } from '../src/tui/ctrlCExit.js';
import { stashOverlayStatusTone } from '../src/tui/StashMenuOverlay.js';

describe('stashOverlayStatusTone', () => {
  it('returns none for empty', () => {
    assert.equal(stashOverlayStatusTone(''), 'none');
  });

  it('returns error for failed/error/invalid', () => {
    assert.equal(stashOverlayStatusTone('Stash failed'), 'error');
    assert.equal(stashOverlayStatusTone('error: conflict'), 'error');
    assert.equal(stashOverlayStatusTone('Invalid ref'), 'error');
  });

  it('returns info for Busy and Ctrl+C prompt', () => {
    assert.equal(stashOverlayStatusTone('Busy…'), 'info');
    assert.equal(stashOverlayStatusTone(CTRL_C_EXIT_PROMPT), 'info');
  });
});
