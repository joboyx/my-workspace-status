import assert from 'node:assert';
import { describe, it } from 'node:test';
import { isLeftListAction } from '../src/tui/keys.js';
import type { Action } from '../src/tui/keys.js';
import {
  applyNavEnter,
  applyNavEsc,
  breadcrumbSegments,
  createNavState,
  currentView,
  formatBreadcrumb,
  navDepth,
} from '../src/tui/nav/stack.js';
import type { NavDrillContext, NavState } from '../src/tui/nav/stack.js';

const drill = (partial: Partial<NavDrillContext> = {}): NavDrillContext => ({
  repo: 'demo-services',
  commitId: 'a1b2c3d',
  filePath: 'src/fetch.ts',
  ...partial,
});

describe('isLeftListAction', () => {
  it('gates left-tree list / row-scoped actions', () => {
    const left: Action[] = [
      { type: 'move', delta: 1 },
      { type: 'moveTo', edge: 'start' },
      { type: 'fold', op: 'toggle' },
      { type: 'expand' },
      { type: 'collapse' },
      { type: 'stage' },
      { type: 'unstage' },
      { type: 'revert' },
      { type: 'edit' },
      { type: 'fullFile' },
      { type: 'branch' },
      { type: 'fetch' },
      { type: 'pull' },
      { type: 'push' },
      { type: 'defaultBranch' },
    ];
    for (const action of left) {
      assert.equal(isLeftListAction(action), true, action.type);
    }
  });

  it('allows nav chrome, quit/help/refresh, theme/mouse, diff, overlays', () => {
    const allowed: Action[] = [
      { type: 'navEnter' },
      { type: 'navEsc' },
      { type: 'quit' },
      { type: 'help' },
      { type: 'refresh' },
      { type: 'cycleTheme' },
      { type: 'toggleMouse' },
      { type: 'scrollDiff', delta: 1 },
      { type: 'toggleDiffMode' },
      { type: 'toggleCommitTreeMode' },
      { type: 'searchStart' },
      { type: 'confirmYes' },
      { type: 'confirmNo' },
      { type: 'none' },
    ];
    for (const action of allowed) {
      assert.equal(isLeftListAction(action), false, action.type);
    }
  });
});

describe('createNavState', () => {
  it('starts at workspace depth with left focus', () => {
    const nav = createNavState();
    assert.equal(navDepth(nav), 0);
    assert.equal(nav.focusPane, 'left');
    assert.deepEqual(currentView(nav), { kind: 'workspace' });
    assert.equal(nav.stack.length, 1);
  });
});

describe('applyNavEnter', () => {
  it('left + Enter focuses right at the same depth', () => {
    const nav = createNavState();
    const next = applyNavEnter(nav, drill());
    assert.equal(next.focusPane, 'right');
    assert.equal(navDepth(next), 0);
    assert.deepEqual(next.stack, nav.stack);
  });

  it('right + Enter at depth 0 pushes repoGraph and stays on right', () => {
    let nav = applyNavEnter(createNavState(), drill());
    nav = applyNavEnter(nav, drill({ repo: 'demo-services', commitId: null }));
    assert.equal(navDepth(nav), 1);
    assert.equal(nav.focusPane, 'right');
    assert.deepEqual(currentView(nav), {
      kind: 'repoGraph',
      repo: 'demo-services',
      commitId: null,
    });
  });

  it('right + Enter at depth 0 is a no-op when repo is empty', () => {
    let nav = applyNavEnter(createNavState(), drill({ repo: '' }));
    const before = structuredClone(nav);
    nav = applyNavEnter(nav, drill({ repo: '' }));
    assert.deepEqual(nav, before);
  });

  it('right + Enter at depth 1 pushes commitFiles and stays on right', () => {
    let nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo-services', commitId: 'a1b2c3d' },
      ],
      focusPane: 'right',
    };
    nav = applyNavEnter(nav, drill());
    assert.equal(navDepth(nav), 2);
    assert.equal(nav.focusPane, 'right');
    assert.deepEqual(currentView(nav), {
      kind: 'commitFiles',
      repo: 'demo-services',
      commitId: 'a1b2c3d',
      filePath: null,
    });
  });

  it('right + Enter at depth 1 with null commitId uses WORKTREE stub id', () => {
    let nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo', commitId: null },
      ],
      focusPane: 'right',
    };
    nav = applyNavEnter(nav, drill({ repo: 'demo', commitId: null }));
    assert.deepEqual(currentView(nav), {
      kind: 'commitFiles',
      repo: 'demo',
      commitId: 'WORKTREE',
      filePath: null,
    });
  });

  it('right + Enter at depth 1 prefers drill.commitId over null stack commitId', () => {
    let nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo', commitId: null },
      ],
      focusPane: 'right',
    };
    nav = applyNavEnter(nav, drill({ repo: 'demo', commitId: 'deadbeefcafe' }));
    assert.deepEqual(currentView(nav), {
      kind: 'commitFiles',
      repo: 'demo',
      commitId: 'deadbeefcafe',
      filePath: null,
    });
  });

  it('right + Enter at depth 1 prefers drill.commitId over stack commitId', () => {
    let nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo', commitId: 'oldcommit' },
      ],
      focusPane: 'right',
    };
    nav = applyNavEnter(nav, drill({ repo: 'demo', commitId: 'newcommit' }));
    assert.deepEqual(currentView(nav), {
      kind: 'commitFiles',
      repo: 'demo',
      commitId: 'newcommit',
      filePath: null,
    });
  });

  it('right + Enter at depth 2 (leaf) is a no-op', () => {
    const nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo', commitId: 'abc' },
        { kind: 'commitFiles', repo: 'demo', commitId: 'abc', filePath: null },
      ],
      focusPane: 'right',
    };
    assert.deepEqual(applyNavEnter(nav, drill()), nav);
  });
});

describe('applyNavEsc', () => {
  it('right → left at the same depth', () => {
    const nav: NavState = { stack: [{ kind: 'workspace' }], focusPane: 'right' };
    const next = applyNavEsc(nav);
    assert.equal(next.focusPane, 'left');
    assert.equal(navDepth(next), 0);
  });

  it('left → pop one depth and stay on left', () => {
    const nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo', commitId: null },
      ],
      focusPane: 'left',
    };
    const next = applyNavEsc(nav);
    assert.equal(navDepth(next), 0);
    assert.equal(next.focusPane, 'left');
    assert.deepEqual(currentView(next), { kind: 'workspace' });
  });

  it('left at depth 0 is a no-op (never quits)', () => {
    const nav = createNavState();
    assert.deepEqual(applyNavEsc(nav), nav);
  });
});

describe('breadcrumbSegments / formatBreadcrumb', () => {
  it('mirrors the stack and focus pane', () => {
    const nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: '/ws/demo-services', commitId: 'a1b2c3d4e5' },
        {
          kind: 'commitFiles',
          repo: '/ws/demo-services',
          commitId: 'a1b2c3d4e5',
          filePath: 'src/fetch.ts',
        },
      ],
      focusPane: 'right',
    };
    assert.deepEqual(breadcrumbSegments(nav, 'workspace'), [
      'workspace',
      'demo-services',
      'a1b2c3d',
      'fetch.ts',
    ]);
    assert.equal(
      formatBreadcrumb(breadcrumbSegments(nav, 'workspace'), 'right'),
      'workspace › demo-services › a1b2c3d › fetch.ts · right',
    );
  });

  it('drops trailing segments on pop', () => {
    const nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo', commitId: null },
      ],
      focusPane: 'left',
    };
    const after = applyNavEsc(nav);
    assert.deepEqual(breadcrumbSegments(after, 'workspace'), ['workspace']);
  });

  it('shows uncommitted for WORKTREE without shortHash truncation', () => {
    const nav: NavState = {
      stack: [
        { kind: 'workspace' },
        { kind: 'repoGraph', repo: 'demo', commitId: null },
        { kind: 'commitFiles', repo: 'demo', commitId: 'WORKTREE', filePath: null },
      ],
      focusPane: 'left',
    };
    assert.deepEqual(breadcrumbSegments(nav, 'workspace'), [
      'workspace',
      'demo',
      'uncommitted',
    ]);
  });
});
