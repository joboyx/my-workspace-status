import assert from 'node:assert';
import { describe, it } from 'node:test';
import { listRowBackground } from '../src/tui/listEmphasis.js';

describe('listRowBackground', () => {
  it('prefers cursor bg over flash', () => {
    assert.equal(
      listRowBackground({ selected: true, flashBg: '#111', cursorBg: '#283457' }),
      '#283457',
    );
  });
  it('uses flash when not selected', () => {
    assert.equal(
      listRowBackground({ selected: false, flashBg: '#111', cursorBg: '#283457' }),
      '#111',
    );
  });
  it('uses search bg when not selected', () => {
    assert.equal(
      listRowBackground({
        selected: false,
        searchMatch: true,
        searchBg: '#bb9af7',
        cursorBg: '#283457',
      }),
      '#bb9af7',
    );
  });
  it('prefers cursor bg over search match', () => {
    assert.equal(
      listRowBackground({
        selected: true,
        searchMatch: true,
        searchBg: '#bb9af7',
        cursorBg: '#283457',
      }),
      '#283457',
    );
  });
});
