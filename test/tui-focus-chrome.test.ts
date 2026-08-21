import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  PANE_TITLE_ROWS,
  focusPaneChromePlain,
  formatPaneTitle,
  leftPaneTitle,
  rightPaneTitle,
} from '../src/tui/focusChrome.js';

describe('leftPaneTitle', () => {
  it('maps nav depth to TREE / GRAPH', () => {
    assert.equal(leftPaneTitle(0), 'TREE');
    assert.equal(leftPaneTitle(1), 'GRAPH');
    assert.equal(leftPaneTitle(2), 'TREE');
  });
});

describe('rightPaneTitle', () => {
  it('maps right host mode to GRAPH / DIFF / TREE', () => {
    assert.equal(rightPaneTitle('graph'), 'GRAPH');
    assert.equal(rightPaneTitle('diff'), 'DIFF');
    assert.equal(rightPaneTitle('commitMeta'), 'TREE');
    assert.equal(rightPaneTitle('empty'), '');
  });
});

describe('formatPaneTitle', () => {
  it('marks focused vs muted plain-text chrome', () => {
    assert.equal(formatPaneTitle('TREE', true), '▶ TREE');
    assert.equal(formatPaneTitle('DIFF', false), '  DIFF');
    assert.equal(formatPaneTitle('', true), '');
  });
});

describe('focusPaneChromePlain', () => {
  it('differs for left vs right focus at depth 0', () => {
    const leftFocus = focusPaneChromePlain({
      navDepth: 0,
      focusPane: 'left',
      rightMode: 'graph',
    });
    const rightFocus = focusPaneChromePlain({
      navDepth: 0,
      focusPane: 'right',
      rightMode: 'graph',
    });
    assert.deepEqual(leftFocus, { left: '▶ TREE', right: '  GRAPH' });
    assert.deepEqual(rightFocus, { left: '  TREE', right: '▶ GRAPH' });
    assert.notDeepEqual(leftFocus, rightFocus);
  });

  it('uses GRAPH / TREE at depth 1 and TREE / DIFF at depth 2', () => {
    assert.deepEqual(
      focusPaneChromePlain({
        navDepth: 1,
        focusPane: 'left',
        rightMode: 'commitMeta',
      }),
      { left: '▶ GRAPH', right: '  TREE' },
    );
    assert.deepEqual(
      focusPaneChromePlain({
        navDepth: 2,
        focusPane: 'right',
        rightMode: 'diff',
      }),
      { left: '  TREE', right: '▶ DIFF' },
    );
  });
});

describe('PANE_TITLE_ROWS', () => {
  it('reserves one chrome row for the title chip', () => {
    assert.equal(PANE_TITLE_ROWS, 1);
  });
});
