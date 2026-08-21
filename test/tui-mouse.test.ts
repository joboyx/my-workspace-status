import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  DOUBLE_CLICK_MS,
  isDoubleClick,
  mouseClickFocus,
  mouseListPressAction,
  MOUSE_DISABLE,
  MOUSE_ENABLE,
  parseMouseChunk,
} from '../src/tui/mouse.js';
import type { MouseListPressInput } from '../src/tui/mouse.js';

describe('mouse sequences', () => {
  it('exports matching enable/disable CSI pairs', () => {
    assert.match(MOUSE_ENABLE, /\x1b\[\?1000h/);
    assert.match(MOUSE_DISABLE, /\x1b\[\?1000l/);
  });

  it('parses a left press and a wheel tick', () => {
    const { events, rest } = parseMouseChunk('\x1b[<0;5;12M\x1b[<64;5;12M');
    assert.equal(rest, '');
    assert.deepEqual(events[0], { button: 'left', action: 'press', x: 5, y: 12 });
    assert.deepEqual(events[1], { button: 'wheelUp', action: 'wheel', x: 5, y: 12 });
  });

  it('parses left-button drag motion (SGR btn 32)', () => {
    const { events, rest } = parseMouseChunk('\x1b[<32;48;10M');
    assert.equal(rest, '');
    assert.deepEqual(events[0], { button: 'left', action: 'drag', x: 48, y: 10 });
  });

  it('parses left release', () => {
    const { events } = parseMouseChunk('\x1b[<0;48;10m');
    assert.deepEqual(events[0], { button: 'left', action: 'release', x: 48, y: 10 });
  });

  it('buffers an incomplete sequence in rest', () => {
    const first = parseMouseChunk('\x1b[<0;5;1');
    assert.deepEqual(first.events, []);
    assert.equal(first.rest, '\x1b[<0;5;1');
    const second = parseMouseChunk(first.rest + '2M');
    assert.equal(second.events.length, 1);
    assert.equal(second.rest, '');
  });
});

describe('isDoubleClick', () => {
  it('true for same cell within window', () => {
    assert.equal(isDoubleClick({ x: 3, y: 5, at: 1000 }, { x: 3, y: 5 }, 1300), true);
  });
  it('false when cell or time differs', () => {
    assert.equal(isDoubleClick({ x: 3, y: 5, at: 1000 }, { x: 4, y: 5 }, 1100), false);
    assert.equal(
      isDoubleClick({ x: 3, y: 5, at: 1000 }, { x: 3, y: 5 }, 1000 + DOUBLE_CLICK_MS),
      false,
    );
  });
});

function press(
  partial: Partial<MouseListPressInput> & Pick<MouseListPressInput, 'pane'>,
): MouseListPressInput {
  return {
    rowIndex: 2,
    foldChevron: false,
    doubleClick: false,
    ...partial,
  };
}

describe('mouseListPressAction', () => {
  it('folds on a chevron click for tree and commit-files', () => {
    for (const pane of ['tree', 'commitFiles'] as const) {
      assert.equal(mouseListPressAction(press({ pane, foldChevron: true })), 'fold', pane);
    }
  });

  it('folds on a chevron double-click and does not drill', () => {
    for (const pane of ['tree', 'commitFiles'] as const) {
      assert.equal(
        mouseListPressAction(press({ pane, foldChevron: true, doubleClick: true })),
        'fold',
        pane,
      );
    }
  });

  it('selects on a single click of a concrete list row', () => {
    for (const pane of ['tree', 'graph', 'commitFiles'] as const) {
      assert.equal(mouseListPressAction(press({ pane })), 'select', pane);
    }
  });

  it('dispatches navEnter on a double-click of a concrete list row', () => {
    for (const pane of ['tree', 'graph', 'commitFiles'] as const) {
      assert.equal(mouseListPressAction(press({ pane, doubleClick: true })), 'navEnter', pane);
    }
  });

  it('ignores a click with no row even when it is a double-click', () => {
    for (const pane of ['tree', 'graph', 'commitFiles'] as const) {
      assert.equal(
        mouseListPressAction(press({ pane, rowIndex: null, doubleClick: true })),
        'ignore',
        pane,
      );
    }
  });

  it('ignores divider, diff, status, and none hits', () => {
    for (const pane of ['divider', 'diffSplit', 'diff', 'status', 'none'] as const) {
      assert.equal(mouseListPressAction(press({ pane, doubleClick: true })), 'ignore', pane);
    }
  });
});

describe('mouseClickFocus', () => {
  it('focuses the right pane when clicking the diff', () => {
    assert.equal(mouseClickFocus('diff'), 'right');
  });

  it('focuses the left pane when clicking the tree', () => {
    assert.equal(mouseClickFocus('tree'), 'left');
  });

  it('leaves graph and commit-files to their hit.side handlers', () => {
    for (const pane of ['graph', 'commitFiles'] as const) {
      assert.equal(mouseClickFocus(pane), null, pane);
    }
  });

  it('does not change focus for divider, status, or empty hits', () => {
    for (const pane of ['divider', 'diffSplit', 'status', 'none'] as const) {
      assert.equal(mouseClickFocus(pane), null, pane);
    }
  });
});
