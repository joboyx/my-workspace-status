import assert from 'node:assert';
import { describe, it } from 'node:test';
import { bottomChromeRows, type BottomChromeInput } from '../src/tui/bottomChrome.js';

function idle(over: Partial<BottomChromeInput> = {}): BottomChromeInput {
  return {
    showHelp: false,
    helpLines: 10,
    pendingConfirmKind: null,
    stashDropConfirm: false,
    graphCheckoutConfirm: false,
    stashMenuLines: 0,
    createBranchOverlay: false,
    branchPickerLines: 0,
    graphBranchPickerLines: 0,
    ...over,
  };
}

describe('bottomChromeRows', () => {
  it('defaults to StatusBar height', () => {
    assert.equal(bottomChromeRows(idle()), 1);
  });

  it('reserves overlay height for stash-drop confirm', () => {
    assert.equal(bottomChromeRows(idle({ stashDropConfirm: true })), 5);
  });

  it('reserves overlay height for graph-checkout confirm', () => {
    assert.equal(bottomChromeRows(idle({ graphCheckoutConfirm: true })), 7);
  });

  it('reserves overlay height for stash menu', () => {
    assert.equal(bottomChromeRows(idle({ stashMenuLines: 6 })), 6);
  });
});
