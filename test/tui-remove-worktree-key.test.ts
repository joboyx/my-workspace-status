/**
 * Helper-only (NOT live TUI e2e — does not mount Ink).
 * Live Ink App coverage lives in test/tui-e2e/*.e2e.ts.
 *
 * E2E-style path: tree focus → handleKey (w/W) → gate → confirm pending,
 * plus the runAction→useActions wire that 0736973 missed.
 *
 * Would have caught: key folds to removeWorktree but runAction silently drops it.
 */

import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';

import {
  USE_ACTIONS_FORWARDED,
  shouldForwardToUseActions,
} from '../src/tui/actionRoute.js';
import {
  actionVisibleForScope,
  canRemoveWorktree,
} from '../src/tui/actions/gates.js';
import { actionFor } from '../src/tui/actions/registry.js';
import { createKeyState, handleKey } from '../src/tui/keys.js';
import { flatten } from '../src/tui/model/flatten.js';
import { createFoldState } from '../src/tui/model/fold.js';
import { buildTree } from '../src/tui/model/tree.js';
import type { RepoSnapshot } from '../src/types.js';
import { actionHintSegments } from '../src/tui/StatusBar.js';
import { buildRemoveWorktreeConfirm } from '../src/tui/useActions.js';

function snap(over: Partial<RepoSnapshot> & Pick<RepoSnapshot, 'repo'>): RepoSnapshot {
  return {
    branch: 'main',
    syncStatus: 'up-to-date',
    syncNote: '',
    hasStaged: false,
    hasUnstaged: false,
    hasUntracked: false,
    staged: [],
    unstaged: [],
    untracked: [],
    checkoutKind: 'primary',
    mergedIntoDefault: null,
    ...over,
  };
}

function buildRows(snapshots: RepoSnapshot[]) {
  const tree = buildTree({
    snapshots,
    ignoredRepos: new Set(),
    treeMode: false,
    workspaceLabel: 'ws',
  });
  return flatten(tree, createFoldState(tree));
}

/**
 * Production path without Ink: resolve key → action id → gate → confirm payload.
 * Returns null when the key does not open remove-worktree confirm.
 */
function removeWorktreeConfirmFromKey(
  input: string,
  focusedKind: 'checkout' | 'repo',
  focused: ReturnType<typeof buildRows>[number],
  snapshots: RepoSnapshot[],
): ReturnType<typeof buildRemoveWorktreeConfirm> | null {
  const { action } = handleKey(createKeyState(), input, {}, focusedKind);
  if (action.type !== 'removeWorktree') return null;
  assert.equal(
    shouldForwardToUseActions(action.type),
    true,
    'runAction must forward removeWorktree to useActions (was silently dropped)',
  );
  if (!canRemoveWorktree(focused)) return null;
  const node = focused.node;
  if (node.kind !== 'checkout' && node.kind !== 'repo') return null;
  if (!node.primaryRepo) return null;
  assert.equal(
    actionVisibleForScope(
      {
        id: 'removeWorktree',
        key: 'W',
        label: 'remove worktree',
        kinds: ['checkout', 'repo'],
        destructive: true,
      },
      { focused, snapshots, navDepth: 0 },
    ),
    true,
  );
  return buildRemoveWorktreeConfirm(node);
}

describe('remove-worktree key → action → confirm (e2e path)', () => {
  const primary = snap({ repo: 'app', branch: 'main' });
  const linked = snap({
    repo: 'app/.worktrees/feat',
    branch: 'feature/x',
    checkoutKind: 'linked',
    primaryRepo: 'app',
    mergedIntoDefault: false,
  });

  it('wires removeWorktree through USE_ACTIONS_FORWARDED and useAppState switch', () => {
    assert.equal(shouldForwardToUseActions('removeWorktree'), true);
    assert.ok(USE_ACTIONS_FORWARDED.has('removeWorktree'));

    const appStatePath = path.join(
      path.dirname(fileURLToPath(import.meta.url)),
      '../src/tui/useAppState.ts',
    );
    const src = fs.readFileSync(appStatePath, 'utf8');
    assert.match(
      src,
      /case 'removeWorktree':/,
      'useAppState.runAction must have case removeWorktree — without it W is a silent no-op',
    );
    for (const type of USE_ACTIONS_FORWARDED) {
      assert.match(
        src,
        new RegExp(`case '${type}':`),
        `useAppState.runAction missing case for forwarded action '${type}'`,
      );
    }
  });

  it('nested linked checkout: w and W open remove-worktree confirm', () => {
    const rows = buildRows([primary, linked]);
    const linkedRow = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'linked',
    );
    assert.ok(linkedRow);

    for (const key of ['w', 'W'] as const) {
      assert.equal(actionFor(key, 'checkout')?.id, 'removeWorktree');
      const pending = removeWorktreeConfirmFromKey(key, 'checkout', linkedRow, [
        primary,
        linked,
      ]);
      assert.ok(pending, `expected confirm for key ${key}`);
      assert.equal(pending.kind, 'removeWorktree');
      assert.equal(pending.path, 'app/.worktrees/feat');
      assert.equal(pending.primaryRepo, 'app');
      assert.equal(pending.branch, 'feature/x');
      assert.equal(pending.mergedIntoDefault, false);
      assert.equal(pending.force, false);
    }

    const hints = actionHintSegments('checkout', 0, 'left', null, {
      focused: linkedRow,
      snapshots: [primary, linked],
      navDepth: 0,
    });
    const removeHint = hints.find((h) => h.key === 'W');
    assert.ok(removeHint, 'StatusBar must advertise remove worktree on linked checkout');
    assert.match(removeHint.label, /remove worktree/);
  });

  it('family container / primary checkout: key does not open confirm', () => {
    const rows = buildRows([primary, linked]);
    const container = rows.find((r) => r.node.kind === 'repo');
    const primaryCheckout = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'primary',
    );
    assert.ok(container);
    assert.ok(primaryCheckout);

    // Key may resolve on repo (registry includes repo), but gate blocks confirm.
    assert.equal(canRemoveWorktree(container), false);
    assert.equal(canRemoveWorktree(primaryCheckout), false);
    assert.equal(
      removeWorktreeConfirmFromKey('w', 'repo', container, [primary, linked]),
      null,
    );
    assert.equal(
      removeWorktreeConfirmFromKey('w', 'checkout', primaryCheckout, [
        primary,
        linked,
      ]),
      null,
    );
  });

  it('flat linked-only repo (named filter): w opens confirm', () => {
    const rows = buildRows([linked]);
    const flat = rows.find(
      (r) => r.node.kind === 'repo' && r.node.checkoutKind === 'linked',
    );
    assert.ok(flat);

    const pending = removeWorktreeConfirmFromKey('w', 'repo', flat, [linked]);
    assert.ok(pending);
    assert.equal(pending.path, 'app/.worktrees/feat');
    assert.equal(pending.primaryRepo, 'app');

    const hints = actionHintSegments('repo', 0, 'left', null, {
      focused: flat,
      snapshots: [linked],
      navDepth: 0,
    });
    assert.ok(hints.some((h) => h.key === 'W'));
  });

  it('depth ≥ 1 write-block hides remove-worktree even when focused on linked checkout', () => {
    const rows = buildRows([primary, linked]);
    const linkedRow = rows.find(
      (r) => r.node.kind === 'checkout' && r.node.checkoutKind === 'linked',
    );
    assert.ok(linkedRow);
    assert.equal(
      actionVisibleForScope(
        {
          id: 'removeWorktree',
          key: 'W',
          label: 'remove worktree',
          kinds: ['checkout', 'repo'],
          destructive: true,
        },
        { focused: linkedRow, snapshots: [primary, linked], navDepth: 1 },
      ),
      false,
    );
  });
});
