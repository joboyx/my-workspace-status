import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  createKeyState,
  DOUBLE_TAP_MS,
  flushPending,
  handleKey,
  isLeftListAction,
} from '../src/tui/keys.js';
import { ACTIONS } from '../src/tui/actions/registry.js';
import { activeRowKind } from '../src/tui/activeContext.js';
import { isGraphListFocused } from '../src/tui/graph/focus.js';

const emptyKey = {};

describe('createKeyState', () => {
  it('starts with zPending/confirmMode/searchMode/branchMode false', () => {
    assert.deepEqual(createKeyState(), {
      zPending: false,
      gPending: false,
      pendingAt: null,
      confirmMode: false,
      searchMode: false,
      searchActive: false,
      easyMotionMode: false,
      branchMode: false,
      createBranchMode: false,
      graphBranchMode: false,
      stashDropMode: false,
      graphCheckoutConfirmMode: false,
      stashMenuMode: false,
      navDepth: 0,
    });
  });

  it('branchMode swallows all keys (App owns picker input)', () => {
    const state = { ...createKeyState(), branchMode: true };
    assert.deepEqual(handleKey(state, 'j', emptyKey, 'repo').action, { type: 'none' });
    assert.deepEqual(handleKey(state, 'q', emptyKey, 'repo').action, { type: 'none' });
    assert.deepEqual(handleKey(state, '', { escape: true }, 'repo').action, { type: 'none' });
    assert.deepEqual(handleKey(state, '', { return: true }, 'repo').action, { type: 'none' });
  });

  it('swallows keys in createBranchMode', () => {
    const state = { ...createKeyState(), createBranchMode: true };
    assert.equal(handleKey(state, 'x', emptyKey, 'graphCommit').action.type, 'none');
    assert.equal(handleKey(state, 'q', emptyKey, 'graphCommit').action.type, 'none');
    assert.equal(handleKey(state, '', { escape: true }, 'graphCommit').action.type, 'none');
  });

  it('swallows keys in graphBranchMode, stashDropMode, and stashMenuMode', () => {
    assert.equal(
      handleKey({ ...createKeyState(), graphBranchMode: true }, 'j', emptyKey, 'graphCommit').action
        .type,
      'none',
    );
    assert.equal(
      handleKey({ ...createKeyState(), stashDropMode: true }, 'y', emptyKey, 'graphStash').action
        .type,
      'none',
    );
    assert.equal(
      handleKey({ ...createKeyState(), stashMenuMode: true }, 's', emptyKey, 'file').action.type,
      'none',
    );
    assert.equal(
      handleKey({ ...createKeyState(), stashMenuMode: true }, 'S', emptyKey, 'file').action.type,
      'none',
    );
  });

  it('swallows keys in graphCheckoutConfirmMode (Esc does not quit)', () => {
    const state = { ...createKeyState(), graphCheckoutConfirmMode: true };
    assert.equal(handleKey(state, 'j', emptyKey, 'graphCommit').action.type, 'none');
    assert.equal(handleKey(state, 'y', emptyKey, 'graphCommit').action.type, 'none');
    assert.equal(handleKey(state, '', { escape: true }, 'graphCommit').action.type, 'none');
  });
});

describe('handleKey — navigation', () => {
  it('maps j / downArrow to move +1 and k / upArrow to move -1', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'j', emptyKey, 'file').action, { type: 'move', delta: 1 });
    assert.deepEqual(handleKey(state, '', { downArrow: true }, 'file').action, {
      type: 'move',
      delta: 1,
    });
    assert.deepEqual(handleKey(state, 'k', emptyKey, 'file').action, { type: 'move', delta: -1 });
    assert.deepEqual(handleKey(state, '', { upArrow: true }, 'file').action, {
      type: 'move',
      delta: -1,
    });
  });

  it('maps h / leftArrow to collapse and l / rightArrow to expand', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'h', emptyKey, 'file').action, { type: 'collapse' });
    assert.deepEqual(handleKey(state, '', { leftArrow: true }, 'file').action, {
      type: 'collapse',
    });
    assert.deepEqual(handleKey(state, 'l', emptyKey, 'file').action, { type: 'expand' });
    assert.deepEqual(handleKey(state, '', { rightArrow: true }, 'file').action, { type: 'expand' });
  });

  it('j scrolls down when right diff focused', () => {
    const r = handleKey(createKeyState(), 'j', {}, 'file', 0, {
      focusPane: 'right',
      rightIsDiff: true,
    });
    assert.deepEqual(r.action, { type: 'scrollDiff', delta: 1 });
  });

  it('k scrolls up when right diff focused', () => {
    const r = handleKey(createKeyState(), 'k', {}, 'file', 0, {
      focusPane: 'right',
      rightIsDiff: true,
    });
    assert.deepEqual(r.action, { type: 'scrollDiff', delta: -1 });
  });

  it('arrows scroll when right diff focused', () => {
    assert.deepEqual(
      handleKey(createKeyState(), '', { downArrow: true }, 'file', 0, {
        focusPane: 'right',
        rightIsDiff: true,
      }).action,
      { type: 'scrollDiff', delta: 1 },
    );
    assert.deepEqual(
      handleKey(createKeyState(), '', { upArrow: true }, 'file', 0, {
        focusPane: 'right',
        rightIsDiff: true,
      }).action,
      { type: 'scrollDiff', delta: -1 },
    );
  });

  it('j moves when left focused even if right is a diff', () => {
    const r = handleKey(createKeyState(), 'j', {}, 'file', 0, {
      focusPane: 'left',
      rightIsDiff: true,
    });
    assert.deepEqual(r.action, { type: 'move', delta: 1 });
  });

  it('j moves when right focused but not a diff', () => {
    const r = handleKey(createKeyState(), 'j', {}, 'file', 0, {
      focusPane: 'right',
      rightIsDiff: false,
    });
    assert.deepEqual(r.action, { type: 'move', delta: 1 });
  });

  it('h pans left when right diff focused', () => {
    const r = handleKey(createKeyState(), 'h', {}, 'file', 0, {
      focusPane: 'right',
      rightIsDiff: true,
    });
    assert.deepEqual(r.action, { type: 'panDiff', delta: -1 });
  });

  it('l pans right when right diff focused', () => {
    const r = handleKey(createKeyState(), 'l', {}, 'file', 0, {
      focusPane: 'right',
      rightIsDiff: true,
    });
    assert.deepEqual(r.action, { type: 'panDiff', delta: 1 });
  });

  it('arrows pan when right diff focused', () => {
    assert.deepEqual(
      handleKey(createKeyState(), '', { leftArrow: true }, 'file', 0, {
        focusPane: 'right',
        rightIsDiff: true,
      }).action,
      { type: 'panDiff', delta: -1 },
    );
    assert.deepEqual(
      handleKey(createKeyState(), '', { rightArrow: true }, 'file', 0, {
        focusPane: 'right',
        rightIsDiff: true,
      }).action,
      { type: 'panDiff', delta: 1 },
    );
  });

  it('h collapses when left focused', () => {
    const r = handleKey(createKeyState(), 'h', {}, 'repo', 0, {
      focusPane: 'left',
      rightIsDiff: true,
    });
    assert.deepEqual(r.action, { type: 'collapse' });
  });

  it('h collapses when right focused but not a diff', () => {
    const r = handleKey(createKeyState(), 'h', {}, 'file', 0, {
      focusPane: 'right',
      rightIsDiff: false,
    });
    assert.deepEqual(r.action, { type: 'collapse' });
  });
});

describe('instant fold (C1)', () => {
  it('z toggles immediately; zz is subtree; zo is not a chord', () => {
    const r1 = handleKey(createKeyState(), 'z', {}, 'repo', 1000);
    assert.equal(r1.action.type, 'fold');
    if (r1.action.type === 'fold') assert.equal(r1.action.op, 'toggle');

    const r2 = handleKey(r1.state, 'z', {}, 'repo', 1100);
    assert.equal(r2.action.type, 'fold');
    if (r2.action.type === 'fold') assert.equal(r2.action.op, 'toggleSubtree');

    const r3 = handleKey(createKeyState(), 'z', {}, 'repo', 2000);
    const r4 = handleKey(r3.state, 'o', {}, 'repo', 2050);
    // Not fold-open — redispatched as registry/other (likely none for 'o').
    assert.notEqual(r4.action.type === 'fold' && r4.action.op === 'open', true);
  });

  it('flushPending after armed z does not emit a second toggle', () => {
    const armed = handleKey(createKeyState(), 'z', {}, 'repo', 1000);
    const flushed = flushPending(armed.state, 1000 + DOUBLE_TAP_MS);
    assert.equal(flushed.action.type, 'none');
    assert.equal(flushed.state.zPending, false);
  });

  it('Esc clears zPending without quitting', () => {
    const t0 = 1_000_000;
    const pending = handleKey(createKeyState(), 'z', emptyKey, 'file', t0).state;
    const result = handleKey(pending, '', { escape: true }, 'file', t0 + 50);
    assert.equal(result.action.type, 'none');
    assert.equal(result.state.zPending, false);
  });
});

describe('handleKey — file ops and modes', () => {
  it('maps s/u/x, i, ., t, r, e, /, ?; q is unbound (exit is double Ctrl+C)', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 's', emptyKey, 'file').action, { type: 'stage' });
    assert.deepEqual(handleKey(state, 'u', emptyKey, 'file').action, { type: 'unstage' });
    assert.deepEqual(handleKey(state, 'x', emptyKey, 'file').action, { type: 'revert' });
    assert.deepEqual(handleKey(state, 'i', emptyKey, 'file').action, { type: 'toggleDiffMode' });
    assert.deepEqual(handleKey(state, '.', emptyKey, 'file').action, { type: 'toggleShowIgnored' });
    assert.deepEqual(handleKey(state, 't', emptyKey, 'file').action, { type: 'toggleTreeMode' });
    assert.deepEqual(handleKey(state, 'r', emptyKey, 'file').action, { type: 'refresh' });
    assert.deepEqual(handleKey(state, 'e', emptyKey, 'file').action, { type: 'edit' });
    assert.deepEqual(handleKey(state, ' ', emptyKey, 'file').action, { type: 'toggleViewed' });
    assert.deepEqual(handleKey(state, ' ', emptyKey, 'repo').action, { type: 'none' });
    assert.deepEqual(handleKey(state, ' ', emptyKey, 'dir').action, { type: 'none' });
    assert.deepEqual(handleKey(state, ' ', emptyKey, 'workspace').action, { type: 'none' });
    assert.deepEqual(handleKey(state, 'v', emptyKey, 'file').action, { type: 'none' });
    assert.deepEqual(handleKey(state, '/', emptyKey, 'file').action, { type: 'searchStart' });
    assert.deepEqual(handleKey(state, '?', emptyKey, 'file').action, { type: 'help' });
    assert.deepEqual(handleKey(state, 'q', emptyKey, 'file').action, { type: 'none' });
  });

  it('maps PgUp/PgDn to pageMove and Ctrl-u/Ctrl-d to scrollDiff ±5', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, '', { pageUp: true }, 'file').action, {
      type: 'pageMove',
      deltaPages: -1,
    });
    assert.deepEqual(handleKey(state, '', { pageDown: true }, 'file').action, {
      type: 'pageMove',
      deltaPages: 1,
    });
    assert.deepEqual(handleKey(state, 'u', { ctrl: true }, 'file').action, {
      type: 'scrollDiff',
      delta: -5,
    });
    assert.deepEqual(handleKey(state, 'd', { ctrl: true }, 'file').action, {
      type: 'scrollDiff',
      delta: 5,
    });
  });
});

describe('handleKey — confirm mode', () => {
  it('only emits confirmYes / confirmNo / none (Esc → confirmNo)', () => {
    const state = { ...createKeyState(), confirmMode: true };

    assert.deepEqual(handleKey(state, 'y', emptyKey, 'file').action, { type: 'confirmYes' });
    assert.deepEqual(handleKey(state, 'n', emptyKey, 'file').action, { type: 'confirmNo' });
    assert.deepEqual(handleKey(state, '', { escape: true }, 'file').action, { type: 'confirmNo' });
    assert.deepEqual(handleKey(state, 'j', emptyKey, 'file').action, { type: 'none' });
    assert.deepEqual(handleKey(state, 'q', emptyKey, 'file').action, { type: 'none' });
    assert.deepEqual(handleKey(state, 's', emptyKey, 'file').action, { type: 'none' });
  });

  it('maps Y to confirmYesClean in confirm mode', () => {
    const state = { ...createKeyState(), confirmMode: true };
    assert.deepEqual(handleKey(state, 'Y', emptyKey, 'file').action, {
      type: 'confirmYesClean',
    });
    assert.deepEqual(handleKey(state, 'y', emptyKey, 'file').action, { type: 'confirmYes' });
  });
});

describe('handleKey — Enter / Esc nav shell', () => {
  it('maps Enter to navEnter and Esc to navEsc outside overlays', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, '', { return: true }, 'repo').action, {
      type: 'navEnter',
    });
    assert.deepEqual(handleKey(state, '', { escape: true }, 'repo').action, {
      type: 'navEsc',
    });
  });

  it('never quits from Esc or q — exit is double Ctrl+C in App', () => {
    const state = createKeyState();
    assert.notEqual(handleKey(state, '', { escape: true }, 'file').action.type, 'quit');
    assert.deepEqual(handleKey(state, 'q', emptyKey, 'file').action, { type: 'none' });
  });

  it('keeps overlay Esc/Enter ownership (search/branch return none from keys)', () => {
    const search = { ...createKeyState(), searchMode: true };
    assert.deepEqual(handleKey(search, '', { escape: true }, 'file').action, {
      type: 'none',
    });
    assert.deepEqual(handleKey(search, '', { return: true }, 'file').action, {
      type: 'none',
    });
    const branch = { ...createKeyState(), branchMode: true };
    assert.deepEqual(handleKey(branch, '', { escape: true }, 'repo').action, {
      type: 'none',
    });
  });

  it('keeps confirm Esc as confirmNo and Enter as confirmYes', () => {
    const state = { ...createKeyState(), confirmMode: true };
    assert.deepEqual(handleKey(state, '', { escape: true }, 'file').action, {
      type: 'confirmNo',
    });
    assert.deepEqual(handleKey(state, '', { return: true }, 'file').action, {
      type: 'confirmYes',
    });
  });
});

describe('registry / keymap key disjointness', () => {
  /**
   * `handleKey` resolves these keys before it consults the registry: navigation
   * and the z-chord prefix (j k h l z) run first, and the tail switch
   * (i . t r / ? q) is unreachable for any key the registry claims. A registry
   * entry bound to one of these would therefore be silently dead.
   */
  const RESERVED_KEYS = [
    'j',
    'k',
    'h',
    'l',
    'z',
    'i',
    '.',
    't',
    'T',
    'r',
    '/',
    '?',
    'q',
    ' ',
    'g',
    'G',
    'm',
    ';',
  ];

  it('binds no action to a key handled before the registry lookup', () => {
    for (const action of ACTIONS) {
      assert.ok(
        !RESERVED_KEYS.includes(action.key),
        `action '${action.id}' is bound to reserved key '${action.key}', which handleKey consumes before the registry lookup`,
      );
    }
  });
});

describe('handleKey — row-kind gating', () => {
  it('fires stage on a file row but not on the workspace row', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 's', emptyKey, 'file').action, { type: 'stage' });
    assert.deepEqual(handleKey(state, 's', emptyKey, 'workspace').action, { type: 'none' });
  });

  it('fires branch only on a repo row', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'b', emptyKey, 'repo').action, { type: 'branch' });
    assert.deepEqual(handleKey(state, 'b', emptyKey, 'file').action, { type: 'none' });
  });

  it('fires removeWorktree on W and w for checkout and repo rows', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'W', emptyKey, 'checkout').action, {
      type: 'removeWorktree',
    });
    assert.deepEqual(handleKey(state, 'w', emptyKey, 'checkout').action, {
      type: 'removeWorktree',
    });
    assert.deepEqual(handleKey(state, 'w', emptyKey, 'repo').action, {
      type: 'removeWorktree',
    });
    assert.deepEqual(handleKey(state, 'w', emptyKey, 'file').action, { type: 'none' });
  });

  it('maps graph keys at depth 1 via graph row kinds', () => {
    const state = { ...createKeyState(), navDepth: 1 as const };
    assert.deepEqual(handleKey(state, 'b', emptyKey, 'graphCommit').action, {
      type: 'graphCheckout',
    });
    assert.deepEqual(handleKey(state, 'c', emptyKey, 'graphCommit').action, {
      type: 'graphCreateBranch',
    });
    assert.deepEqual(handleKey(state, 'a', emptyKey, 'graphStash').action, {
      type: 'stashApply',
    });
    assert.deepEqual(handleKey(state, 'D', emptyKey, 'graphStash').action, {
      type: 'stashDrop',
    });
    assert.deepEqual(handleKey(state, 'p', emptyKey, 'graphStash').action, {
      type: 'stashPop',
    });
    assert.deepEqual(handleKey(state, 'S', emptyKey, 'graphStash').action, {
      type: 'stashMenu',
    });
    assert.deepEqual(handleKey(state, 'b', emptyKey, 'graphUncommitted').action, {
      type: 'none',
    });
  });

  it('fires stashMenu on S for a file and keeps s as stage', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'S', emptyKey, 'file').action, { type: 'stashMenu' });
    assert.deepEqual(handleKey(state, 's', emptyKey, 'file').action, { type: 'stage' });
    assert.deepEqual(handleKey(state, 'S', emptyKey, 'workspace').action, { type: 'none' });
  });

  it('fires pull on workspace and repo rows only', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'p', emptyKey, 'workspace').action, { type: 'pull' });
    assert.deepEqual(handleKey(state, 'p', emptyKey, 'repo').action, { type: 'pull' });
    assert.deepEqual(handleKey(state, 'p', emptyKey, 'dir').action, { type: 'none' });
  });

  it('fires push on P for repo and checkout rows only', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'P', emptyKey, 'repo').action, { type: 'push' });
    assert.deepEqual(handleKey(state, 'P', emptyKey, 'checkout').action, { type: 'push' });
    assert.deepEqual(handleKey(state, 'P', emptyKey, 'workspace').action, { type: 'none' });
    assert.deepEqual(handleKey(state, 'p', emptyKey, 'repo').action, { type: 'pull' });
  });

  it('fires ctrl+o as fullFile on a file row only', () => {
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'o', { ctrl: true }, 'file').action, { type: 'fullFile' });
    assert.deepEqual(handleKey(state, 'o', { ctrl: true }, 'repo').action, { type: 'none' });
  });

  it('emits edit and fullFile for a commit file at depth 2 left and right', () => {
    for (const focusPane of ['left', 'right'] as const) {
      const kind = activeRowKind({
        depth: 2,
        focusPane,
        graphVisible: true,
        treeKind: 'repo',
        graphKind: 'graphCommit',
        commitFileKind: 'file',
      });
      assert.equal(kind, 'file', focusPane);
      const state = { ...createKeyState(), navDepth: 2 as const };
      const ctx = { focusPane, rightIsDiff: focusPane === 'right' };
      assert.deepEqual(handleKey(state, 'e', emptyKey, kind, 0, ctx).action, { type: 'edit' });
      assert.deepEqual(handleKey(state, 'o', { ctrl: true }, kind, 0, ctx).action, {
        type: 'fullFile',
      });
    }
  });

  it('emits edit and fullFile for a commit file at depth 1 right', () => {
    const kind = activeRowKind({
      depth: 1,
      focusPane: 'right',
      graphVisible: true,
      treeKind: 'repo',
      graphKind: 'graphCommit',
      commitFileKind: 'file',
    });
    assert.equal(kind, 'file');
    const state = { ...createKeyState(), navDepth: 1 as const };
    const ctx = { focusPane: 'right' as const, rightIsDiff: false };
    assert.deepEqual(handleKey(state, 'e', emptyKey, kind, 0, ctx).action, { type: 'edit' });
    assert.deepEqual(handleKey(state, 'o', { ctrl: true }, kind, 0, ctx).action, {
      type: 'fullFile',
    });
  });

  it('maps graph keys at depth 0 right, not tree pull/push/branch', () => {
    const kind = activeRowKind({
      depth: 0,
      focusPane: 'right',
      graphVisible: true,
      treeKind: 'repo',
      graphKind: 'graphCommit',
      commitFileKind: 'file',
    });
    assert.equal(kind, 'graphCommit');
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'b', emptyKey, kind).action, { type: 'graphCheckout' });
    assert.deepEqual(handleKey(state, 'c', emptyKey, kind).action, {
      type: 'graphCreateBranch',
    });
    assert.deepEqual(handleKey(state, 'p', emptyKey, kind).action, { type: 'none' });
    assert.deepEqual(handleKey(state, 'P', emptyKey, kind).action, { type: 'none' });
    assert.equal(isGraphListFocused({ depth: 0, focusPane: 'right', graphVisible: true }), true);
  });

  it('maps tree writes at depth 0 left on a repo, not graph checkout', () => {
    const kind = activeRowKind({
      depth: 0,
      focusPane: 'left',
      graphVisible: true,
      treeKind: 'repo',
      graphKind: 'graphCommit',
      commitFileKind: 'file',
    });
    assert.equal(kind, 'repo');
    const state = createKeyState();
    assert.deepEqual(handleKey(state, 'p', emptyKey, kind).action, { type: 'pull' });
    assert.deepEqual(handleKey(state, 'P', emptyKey, kind).action, { type: 'push' });
    assert.deepEqual(handleKey(state, 'b', emptyKey, kind).action, { type: 'branch' });
    assert.equal(isGraphListFocused({ depth: 0, focusPane: 'left', graphVisible: true }), false);
  });

  it('depth 1 left graph-focused b is graphCheckout and the graph-list gate is open', () => {
    const kind = activeRowKind({
      depth: 1,
      focusPane: 'left',
      graphVisible: true,
      treeKind: 'repo',
      graphKind: 'graphCommit',
      commitFileKind: 'file',
    });
    assert.equal(kind, 'graphCommit');
    const state = { ...createKeyState(), navDepth: 1 as const };
    assert.deepEqual(handleKey(state, 'b', emptyKey, kind).action, { type: 'graphCheckout' });
    assert.equal(isGraphListFocused({ depth: 1, focusPane: 'left', graphVisible: true }), true);
  });

  it('gates every action key off on group rows', () => {
    const state = createKeyState();
    for (const key of ['s', 'u', 'x', 'f', 'b']) {
      assert.deepEqual(
        handleKey(state, key, emptyKey, 'group').action,
        { type: 'none' },
        `key '${key}' should be inert on a group row`,
      );
    }
  });

  it('leaves navigation keys ungated; q stays unbound', () => {
    const state = createKeyState();
    for (const kind of ['workspace', 'repo', 'group', 'dir', 'file'] as const) {
      assert.deepEqual(handleKey(state, 'j', emptyKey, kind).action, { type: 'move', delta: 1 });
      assert.deepEqual(handleKey(state, 'q', emptyKey, kind).action, { type: 'none' });
    }
  });
});

describe('handleKey — z double-tap', () => {
  it('space on a file is toggleViewed and does not arm a fold chord', () => {
    const r1 = handleKey(createKeyState(), ' ', emptyKey, 'file', 1_000_000);
    assert.deepEqual(r1.action, { type: 'toggleViewed' });
    const r2 = handleKey(r1.state, ' ', emptyKey, 'file', 1_000_050);
    assert.deepEqual(r2.action, { type: 'toggleViewed' });
    assert.equal(r2.state.zPending, false);
  });

  it('space on a non-file row is a no-op (does not fold)', () => {
    for (const kind of ['repo', 'dir', 'workspace', 'graphCommit'] as const) {
      const r = handleKey(createKeyState(), ' ', emptyKey, kind, 1_000_000);
      assert.deepEqual(r.action, { type: 'none' }, `space should no-op on ${kind}`);
    }
  });

  it('intervening j after g cancels and redispatches move (no prelude)', () => {
    const t0 = 1_000_000;
    const armed = handleKey(createKeyState(), 'g', emptyKey, 'file', t0).state;
    const result = handleKey(armed, 'j', emptyKey, 'file', t0 + 50);
    assert.equal(result.prelude, undefined);
    assert.deepEqual(result.action, { type: 'move', delta: 1 });
    assert.equal(result.state.gPending, false);
  });

  it('intervening j after armed z redispatches move (no prelude)', () => {
    const t0 = 1_000_000;
    const armed = handleKey(createKeyState(), 'z', emptyKey, 'repo', t0).state;
    const result = handleKey(armed, 'j', emptyKey, 'repo', t0 + 50);
    assert.equal(result.prelude, undefined);
    assert.deepEqual(result.action, { type: 'move', delta: 1 });
    assert.equal(result.state.zPending, false);
  });

  it('expired z then j redispatches move without prelude toggle', () => {
    const t0 = 1_000_000;
    const armed = handleKey(createKeyState(), 'z', emptyKey, 'repo', t0).state;
    const result = handleKey(armed, 'j', emptyKey, 'repo', t0 + DOUBLE_TAP_MS);
    assert.equal(result.prelude, undefined);
    assert.deepEqual(result.action, { type: 'move', delta: 1 });
    assert.equal(result.state.zPending, false);
  });
});

describe('handleKey — gg / G', () => {
  it('maps gg to moveTo start and G to moveTo end', () => {
    const t0 = 1_000_000;
    const g = handleKey(createKeyState(), 'g', emptyKey, 'file', t0);
    assert.equal(g.action.type, 'none');
    assert.equal(g.state.gPending, true);
    assert.deepEqual(handleKey(g.state, 'g', emptyKey, 'file', t0 + 50).action, {
      type: 'moveTo',
      edge: 'start',
    });
    assert.deepEqual(handleKey(createKeyState(), 'G', emptyKey, 'file', t0).action, {
      type: 'moveTo',
      edge: 'end',
    });
  });

  it('expired gPending flushes to none', () => {
    const t0 = 1_000_000;
    const g = handleKey(createKeyState(), 'g', emptyKey, 'file', t0).state;
    assert.deepEqual(flushPending(g, t0 + DOUBLE_TAP_MS).action, { type: 'none' });
  });
});

describe('handleKey — mouse toggle', () => {
  it('maps m to toggleMouse', () => {
    assert.deepEqual(handleKey(createKeyState(), 'm', emptyKey, 'file').action, {
      type: 'toggleMouse',
    });
  });
});

describe('handleKey — theme cycle', () => {
  it('T cycles theme; t still toggles tree mode', () => {
    const base = createKeyState();
    assert.equal(handleKey(base, 'T', emptyKey, 'file').action.type, 'cycleTheme');
    assert.equal(handleKey(base, 't', emptyKey, 'file').action.type, 'toggleTreeMode');
  });

  it('t at depth 1 emits toggleCommitTreeMode', () => {
    const state = { ...createKeyState(), navDepth: 1 as const };
    assert.equal(handleKey(state, 't', emptyKey, 'file').action.type, 'toggleCommitTreeMode');
  });

  it('t at depth 0 still emits toggleTreeMode', () => {
    const action = handleKey({ ...createKeyState(), navDepth: 0 }, 't', emptyKey, 'file');
    assert.equal(action.action.type, 'toggleTreeMode');
  });

  it('t is not a left-list action (view-mode like i)', () => {
    assert.equal(isLeftListAction({ type: 'toggleTreeMode' }), false);
    assert.equal(isLeftListAction({ type: 'toggleCommitTreeMode' }), false);
    assert.equal(isLeftListAction({ type: 'toggleDiffMode' }), false);
    assert.equal(isLeftListAction({ type: 'toggleShowIgnored' }), false);
  });

  it('dot is ignored while confirm or search mode is active', () => {
    const confirm = { ...createKeyState(), confirmMode: true };
    assert.deepEqual(handleKey(confirm, '.', emptyKey, 'file').action, { type: 'none' });
    const search = { ...createKeyState(), searchMode: true };
    assert.deepEqual(handleKey(search, '.', emptyKey, 'file').action, { type: 'none' });
  });

  it('T is ignored while confirm or search mode is active', () => {
    assert.equal(
      handleKey({ ...createKeyState(), confirmMode: true }, 'T', emptyKey, 'file').action.type,
      'none',
    );
    assert.equal(
      handleKey({ ...createKeyState(), searchMode: true }, 'T', emptyKey, 'file').action.type,
      'none',
    );
  });
});

describe('handleKey — search (C5)', () => {
  it('slash starts search, not filter', () => {
    const r = handleKey(createKeyState(), '/', {}, 'repo');
    assert.deepEqual(r.action, { type: 'searchStart' });
  });

  it('n/N emit searchStep when searchActive', () => {
    const armed = { ...createKeyState(), searchActive: true };
    assert.deepEqual(handleKey(armed, 'n', emptyKey, 'repo').action, { type: 'searchNext' });
    assert.deepEqual(handleKey(armed, 'N', emptyKey, 'repo').action, { type: 'searchPrev' });
  });

  it('p remains pull when search is idle', () => {
    assert.deepEqual(handleKey(createKeyState(), 'p', emptyKey, 'workspace').action, {
      type: 'pull',
    });
  });

  it('armed-search p falls through to stashPop on a graph stash row', () => {
    const armed = { ...createKeyState(), searchActive: true };
    assert.deepEqual(handleKey(armed, 'p', emptyKey, 'graphStash').action, {
      type: 'stashPop',
    });
  });
  it('armed-search p falls through to pull on a repo row', () => {
    const armed = { ...createKeyState(), searchActive: true };
    assert.deepEqual(handleKey(armed, 'p', emptyKey, 'repo').action, { type: 'pull' });
  });

  it('idle N is unbound', () => {
    assert.deepEqual(handleKey(createKeyState(), 'N', emptyKey, 'repo').action, {
      type: 'none',
    });
  });

  it('P remains push when search is armed', () => {
    const armed = { ...createKeyState(), searchActive: true };
    assert.deepEqual(handleKey(armed, 'P', emptyKey, 'repo').action, { type: 'push' });
  });
});
