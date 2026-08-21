import assert from 'node:assert';
import { describe, it } from 'node:test';
import type { RepoSnapshot } from '../src/types.js';
import {
  createSessionState,
  cursorIndexFor,
  focusAncestorIds,
  resolveFocusAfterRebuild,
  resolveListFocus,
} from '../src/tui/session.js';
import type { SessionState } from '../src/tui/session.js';
import { createFoldState } from '../src/tui/model/fold.js';
import { flatten } from '../src/tui/model/flatten.js';
import { buildTree } from '../src/tui/model/tree.js';
import type { VisibleRow } from '../src/tui/model/types.js';
import { FLASH_MS, mergeGhostRows } from '../src/tui/watch.js';
import { DEFAULT_THEME_ID, THEMES, getTheme, setActiveTheme } from '../src/tui/theme.js';

function row(id: string): VisibleRow {
  return {
    id,
    depth: 0,
    node: { kind: 'group', id: 'group:no-updates', children: [] },
    label: id,
    segments: [],
    trailing: [],
  };
}

function snap(partial: Partial<RepoSnapshot>): RepoSnapshot {
  return {
    repo: 'demo',
    branch: 'main',
    syncStatus: 'up-to-date',
    syncNote: '',
    hasUnstaged: false,
    hasStaged: false,
    hasUntracked: false,
    unstagedInfo: '',
    stagedFiles: '',
    unstagedFiles: '',
    untrackedFiles: '',
    checkoutKind: 'primary',
    mergedIntoDefault: null,
    ...partial,
  };
}

describe('createSessionState', () => {
  it('starts with no cursor, no folds, no filter, side-by-side diff, tree mode on', () => {
    const previous = getTheme();
    try {
      const s = createSessionState({});
      assert.equal(s.restored, false);
      assert.equal(s.cursorId, null);
      assert.equal(s.folded.size, 0);
      assert.equal(s.filter, '');
      assert.equal(s.diffMode, 'sideBySide');
      assert.equal(s.fullContext.size, 0);
      assert.equal(s.treeMode, true);
      assert.equal(s.showIgnored, false);
      assert.equal(s.mouseEnabled, true);
      assert.equal(s.theme, DEFAULT_THEME_ID);
      assert.equal(getTheme().id, DEFAULT_THEME_ID);
      assert.deepEqual(s.nav, {
        stack: [{ kind: 'workspace' }],
        focusPane: 'left',
      });
      assert.equal(s.graphWindow, 300);
      assert.equal(s.graphCacheEpoch, 0);
      assert.equal(s.diffColOffset, 0);
      assert.equal(s.search, null);
      assert.equal(s.easyMotion, false);
    } finally {
      setActiveTheme(previous);
    }
  });

  it('seeds showIgnored from the optional launch flag', () => {
    const previous = getTheme();
    try {
      const hidden = createSessionState({});
      const shown = createSessionState({}, { showIgnored: true });
      assert.equal(hidden.showIgnored, false);
      assert.equal(shown.showIgnored, true);
    } finally {
      setActiveTheme(previous);
    }
  });

  it('seeds theme from WS_STATUS_THEME', () => {
    const previous = getTheme();
    try {
      const s = createSessionState({ WS_STATUS_THEME: 'dracula' });
      assert.equal(s.theme, 'dracula');
      assert.equal(getTheme().id, 'dracula');
      assert.equal(getTheme().palette.heading, THEMES.dracula.palette.heading);
    } finally {
      setActiveTheme(previous);
    }
  });

  it('falls back to tokyo-night for unknown WS_STATUS_THEME', () => {
    const previous = getTheme();
    try {
      const s = createSessionState({ WS_STATUS_THEME: 'not-a-theme' });
      assert.equal(s.theme, DEFAULT_THEME_ID);
    } finally {
      setActiveTheme(previous);
    }
  });
});

/**
 * `useAppState` re-applies the default fold state only for a fresh launch.
 * The rule under test is the one the boot block uses.
 */
function isFreshLaunch(session: SessionState): boolean {
  return !session.restored;
}

/** The inference this replaced, kept only to show the false positive it had. */
function wasFreshLaunchByInference(session: SessionState): boolean {
  return session.cursorId === null && session.folded.size === 0;
}

describe('fresh-launch detection', () => {
  it('treats a never-rendered session as fresh', () => {
    assert.equal(isFreshLaunch(createSessionState({})), true);
  });

  it('does not treat an expanded-all session with no matching rows as fresh', () => {
    /**
     * Reachable state: the user expands everything, so `folded` empties, while
     * a filter matches zero rows, so the reported `cursorId` is null.
     */
    const session: SessionState = {
      ...createSessionState({}),
      restored: true,
      folded: new Set(),
      cursorId: null,
      filter: 'zzz-no-match',
    };
    // The old rule misread this as a fresh launch and re-folded the tree.
    assert.equal(wasFreshLaunchByInference(session), true);
    assert.equal(isFreshLaunch(session), false);
  });

  it('treats any restored session as a restore', () => {
    const session: SessionState = {
      ...createSessionState({}),
      restored: true,
      folded: new Set(['repo:a']),
      cursorId: 'repo:a',
    };
    assert.equal(isFreshLaunch(session), false);
  });
});

describe('cursorIndexFor', () => {
  const rows = [row('a'), row('b'), row('c')];

  it('finds the row with the saved id regardless of position', () => {
    assert.equal(cursorIndexFor(rows, 'c'), 2);
    assert.equal(cursorIndexFor([row('x'), row('c')], 'c'), 1);
  });

  it('falls back to the first row when the id is gone', () => {
    assert.equal(cursorIndexFor(rows, 'deleted'), 0);
  });

  it('falls back to the first row when no cursor is saved', () => {
    assert.equal(cursorIndexFor(rows, null), 0);
  });

  it('returns 0 for an empty row list', () => {
    assert.equal(cursorIndexFor([], 'a'), 0);
  });
});

describe('resolveFocusAfterRebuild', () => {
  it('keeps selection when a repo moves under folded no-updates', () => {
    // Group already exists (docs) and is folded; lib is top-level attention.
    const before = buildTree({
      snapshots: [
        snap({
          repo: 'lib',
          branch: 'feature/x',
          syncStatus: 'up-to-date',
        }),
        snap({
          repo: 'docs',
          branch: 'main',
          syncStatus: 'up-to-date',
        }),
        snap({
          repo: 'app',
          branch: 'main',
          syncStatus: 'behind',
          hasUnstaged: true,
          unstagedFiles: 'a.ts',
        }),
      ],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const folds = createFoldState(before);
    assert.ok(folds.has('group:no-updates'));
    const beforeRows = flatten(before, folds);
    const libIndex = beforeRows.findIndex((r) => r.id === 'repo:lib');
    assert.ok(libIndex >= 0);
    assert.ok(!beforeRows.some((r) => r.id === 'repo:docs'));

    // After pull: lib is clean on default → moves into the still-folded group.
    const after = buildTree({
      snapshots: [
        snap({
          repo: 'lib',
          branch: 'main',
          syncStatus: 'up-to-date',
        }),
        snap({
          repo: 'docs',
          branch: 'main',
          syncStatus: 'up-to-date',
        }),
        snap({
          repo: 'app',
          branch: 'main',
          syncStatus: 'behind',
          hasUnstaged: true,
          unstagedFiles: 'a.ts',
        }),
      ],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const hidden = flatten(after, folds);
    assert.ok(!hidden.some((r) => r.id === 'repo:lib'));

    const restored = resolveFocusAfterRebuild(after, folds, 'repo:lib', libIndex);
    assert.ok(!restored.folds.has('group:no-updates'));
    const rows = flatten(after, restored.folds);
    assert.equal(rows[restored.cursor]?.id, 'repo:lib');
    assert.equal(restored.focusId, 'repo:lib');
  });

  it('keeps the focused repo at its merged-list index when a ghost inserts above', () => {
    const tree = buildTree({
      snapshots: [
        snap({
          repo: 'lib',
          branch: 'feature/x',
          syncStatus: 'up-to-date',
        }),
        snap({
          repo: 'app',
          branch: 'main',
          syncStatus: 'behind',
          hasUnstaged: true,
          unstagedFiles: 'a.ts',
        }),
      ],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const folds = createFoldState(tree);
    const live = flatten(tree, folds);
    const libFlat = live.findIndex((r) => r.id === 'repo:lib');
    assert.ok(libFlat >= 0);

    const ghost = row('file:ghost:gone.ts');
    const displayed = mergeGhostRows(
      live,
      [{ id: ghost.id, row: ghost, flashedAt: 1000, index: 0 }],
      1000 + FLASH_MS / 2,
    );
    const libDisplayed = displayed.findIndex((r) => r.id === 'repo:lib');
    assert.ok(libDisplayed > libFlat);

    const restored = resolveFocusAfterRebuild(tree, folds, 'repo:lib', libFlat, displayed);
    assert.equal(restored.folds, folds);
    assert.equal(restored.cursor, libDisplayed);
    assert.notEqual(restored.cursor, libFlat);
    assert.equal(displayed[restored.cursor]?.id, 'repo:lib');
    assert.equal(restored.focusId, 'repo:lib');
  });

  it('falls back to the repo ancestor when the focused file is gone', () => {
    const after = buildTree({
      snapshots: [
        snap({
          repo: 'lib',
          branch: 'main',
          syncStatus: 'up-to-date',
        }),
        snap({
          repo: 'app',
          branch: 'main',
          syncStatus: 'behind',
          hasUnstaged: true,
          unstagedFiles: 'a.ts',
        }),
      ],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const folds = createFoldState(after);
    assert.ok(folds.has('group:no-updates'));
    const hidden = flatten(after, folds);
    assert.ok(!hidden.some((r) => r.id === 'file:lib:src/a.ts'));
    assert.ok(!hidden.some((r) => r.id === 'repo:lib'));

    const restored = resolveFocusAfterRebuild(after, folds, 'file:lib:src/a.ts', 0);
    assert.ok(!restored.folds.has('group:no-updates'));
    const rows = flatten(after, restored.folds);
    assert.equal(rows[restored.cursor]?.id, 'repo:lib');
    assert.equal(restored.focusId, 'repo:lib');
  });

  it('clamps when the focused entry disappeared', () => {
    const tree = buildTree({
      snapshots: [snap({ repo: 'only', branch: 'main', syncStatus: 'behind' })],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const folds = createFoldState(tree);
    const rowCount = flatten(tree, folds).length;
    const restored = resolveFocusAfterRebuild(tree, folds, 'repo:gone', 99);
    assert.equal(restored.folds, folds);
    assert.equal(restored.cursor, Math.max(0, rowCount - 1));
  });

  it('keeps previousCursor when the id and its ancestors are gone', () => {
    const tree = buildTree({
      snapshots: [
        snap({ repo: 'alpha', branch: 'main', syncStatus: 'behind' }),
        snap({ repo: 'beta', branch: 'main', syncStatus: 'behind' }),
      ],
      ignoredRepos: new Set(),
      treeMode: false,
      workspaceLabel: 'ws',
    });
    const folds = createFoldState(tree);
    const rows = flatten(tree, folds);
    assert.ok(rows.length >= 3);
    const previousCursor = 1;
    assert.ok(previousCursor < rows.length);
    assert.notEqual(rows[previousCursor]?.id, rows[0]?.id);

    const restored = resolveFocusAfterRebuild(tree, folds, 'file:unknown:gone.ts', previousCursor);
    assert.equal(restored.folds, folds);
    assert.equal(restored.cursor, previousCursor);
    assert.notEqual(restored.cursor, 0);
    assert.equal(restored.focusId, rows[previousCursor]?.id);
  });
});

describe('focusAncestorIds', () => {
  it('lists parent dirs then repo and checkout for a file id', () => {
    assert.deepEqual(focusAncestorIds('file:lib:src/a.ts'), [
      'dir:lib:src',
      'repo:lib',
      'checkout:lib',
    ]);
    assert.deepEqual(focusAncestorIds('file:dotfiles:ai/common/skills/foo.ts'), [
      'dir:dotfiles:ai/common/skills',
      'dir:dotfiles:ai/common',
      'dir:dotfiles:ai',
      'repo:dotfiles',
      'checkout:dotfiles',
    ]);
  });

  it('lists parent-prefix dirs then repo and checkout for a dir id', () => {
    assert.deepEqual(focusAncestorIds('dir:dotfiles:ai/common/skills'), [
      'dir:dotfiles:ai/common',
      'dir:dotfiles:ai',
      'repo:dotfiles',
      'checkout:dotfiles',
    ]);
    assert.deepEqual(focusAncestorIds('dir:lib:src'), ['repo:lib', 'checkout:lib']);
  });

  it('maps a checkout id to its repo', () => {
    assert.deepEqual(focusAncestorIds('checkout:acme/acme-frontend'), [
      'repo:acme/acme-frontend',
    ]);
  });

  it('returns no fallbacks for a repo or other id', () => {
    assert.deepEqual(focusAncestorIds('repo:lib'), []);
    assert.deepEqual(focusAncestorIds('group:no-updates'), []);
    assert.deepEqual(focusAncestorIds('workspace'), []);
  });
});

describe('resolveListFocus', () => {
  it('keeps the focused id when rows above are inserted', () => {
    const rows = [row('file:lib:new.ts'), row('file:lib:src/a.ts'), row('repo:lib')];
    const restored = resolveListFocus(rows, 'file:lib:src/a.ts', 0);
    assert.equal(restored.cursor, 1);
    assert.equal(restored.focusId, 'file:lib:src/a.ts');
  });

  it('falls back to a visible ancestor when the focused file is gone', () => {
    const rows = [row('dir:lib:src'), row('repo:lib')];
    const restored = resolveListFocus(rows, 'file:lib:src/a.ts', 0);
    assert.equal(restored.cursor, 0);
    assert.equal(restored.focusId, 'dir:lib:src');
  });

  it('clamps previousCursor when the id and ancestors are gone', () => {
    const rows = [row('repo:alpha'), row('repo:beta'), row('repo:gamma')];
    const restored = resolveListFocus(rows, 'file:unknown:gone.ts', 1);
    assert.equal(restored.cursor, 1);
    assert.equal(restored.focusId, 'repo:beta');
  });

  it('returns a null focusId for an empty list', () => {
    const restored = resolveListFocus([], 'file:lib:src/a.ts', 4);
    assert.equal(restored.cursor, 0);
    assert.equal(restored.focusId, null);
  });
});
