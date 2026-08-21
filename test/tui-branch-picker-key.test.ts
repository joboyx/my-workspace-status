/**
 * Helper-only (NOT live TUI e2e — does not mount Ink).
 * Live Ink App coverage lives in test/tui-e2e/*.e2e.ts.
 *
 * E2E-style path: depth-0 tree focus → handleKey `b` → gate → picker path,
 * plus the runAction→useActions wire.
 *
 * Would have caught: key folds to `branch` but runAction silently drops it,
 * or `b` on a family container opening a picker.
 */

import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';

import { activeRowKind } from '../src/tui/activeContext.js';
import {
  USE_ACTIONS_FORWARDED,
  shouldForwardToUseActions,
} from '../src/tui/actionRoute.js';
import { canBranch } from '../src/tui/actions/gates.js';
import { branchPickerPath } from '../src/tui/branches.js';
import { createKeyState, handleKey } from '../src/tui/keys.js';
import { flatten } from '../src/tui/model/flatten.js';
import { buildTree } from '../src/tui/model/tree.js';
import type { RepoSnapshot } from '../src/types.js';

const TUI_DIR = path.join(path.dirname(fileURLToPath(import.meta.url)), '../src/tui');

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
  // Empty folds so clean repos inside `no-updates` stay visible for focus.
  return flatten(tree, new Set());
}

/**
 * Production path without Ink: depth-0 left kind → `b` → gate → picker path.
 * Returns null when the key must not open the local branch picker.
 */
function localBranchPickerFromKey(
  focused: ReturnType<typeof buildRows>[number],
): string | null {
  const kind = activeRowKind({
    depth: 0,
    focusPane: 'left',
    graphVisible: true,
    treeKind: focused.node.kind,
    graphKind: 'graphCommit',
    commitFileKind: 'file',
  });
  const { action } = handleKey(createKeyState(), 'b', {}, kind);
  if (action.type !== 'branch') return null;
  assert.equal(
    shouldForwardToUseActions('branch'),
    true,
    'runAction must forward branch to useActions (was silently dropped)',
  );
  if (!canBranch(focused)) return null;
  return branchPickerPath(focused);
}

describe('depth-0 b → local branch picker (e2e path)', () => {
  const primary = snap({ repo: 'app', branch: 'main' });
  const linked = snap({
    repo: 'app/.worktrees/feat',
    branch: 'feature/x',
    checkoutKind: 'linked',
    primaryRepo: 'app',
    mergedIntoDefault: false,
  });

  it('wires branch through USE_ACTIONS_FORWARDED and useAppState / useActions', () => {
    assert.equal(shouldForwardToUseActions('branch'), true);
    assert.ok(USE_ACTIONS_FORWARDED.has('branch'));

    const appState = fs.readFileSync(path.join(TUI_DIR, 'useAppState.ts'), 'utf8');
    assert.match(
      appState,
      /case 'branch':/,
      'useAppState.runAction must have case branch — without it b is a silent no-op',
    );

    const actions = fs.readFileSync(path.join(TUI_DIR, 'useActions.ts'), 'utf8');
    assert.match(actions, /case 'branch':/);
    assert.match(actions, /openBranchPicker\(pickerPath\)/);
  });

  it('depth 0 left: b opens the picker on a flat repo and stays closed on a family container', () => {
    const flat = buildRows([primary]).find((r) => r.node.kind === 'repo');
    const container = buildRows([primary, linked]).find((r) => r.node.kind === 'repo');
    assert.ok(flat);
    assert.ok(container);
    assert.equal(localBranchPickerFromKey(flat), 'app');
    assert.equal(localBranchPickerFromKey(container), null);
  });
});
