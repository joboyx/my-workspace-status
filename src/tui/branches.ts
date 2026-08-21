/**
 * Pure helpers for the local branch picker (`b`).
 */

import type { LocalBranch } from '../git.js';
import { canBranch } from './actions/gates.js';
import { primaryCheckoutPath } from './model/tree.js';
import type { VisibleRow } from './model/types.js';

/**
 * Local branches, default branch name pinned first, then newest authordate.
 */
export function sortBranchesForPicker(
  branches: LocalBranch[],
  defaultBranch: string | null,
): LocalBranch[] {
  return [...branches].sort((a, b) => {
    if (defaultBranch) {
      const aDefault = a.name === defaultBranch;
      const bDefault = b.name === defaultBranch;
      if (aDefault !== bDefault) return aDefault ? -1 : 1;
    }
    return b.authordate - a.authordate;
  });
}

/**
 * Case-insensitive substring filter on branch name.
 */
export function filterBranches(
  branches: LocalBranch[],
  query: string,
): LocalBranch[] {
  const q = query.trim().toLowerCase();
  if (!q) return branches;
  return branches.filter((b) => b.name.toLowerCase().includes(q));
}

/**
 * Checkout path the depth-0 local picker should open, or `null` when `b`
 * must not open (family container, file, missing focus).
 */
export function branchPickerPath(focused: VisibleRow | null): string | null {
  if (!canBranch(focused) || !focused) return null;
  const node = focused.node;
  if (node.kind === 'checkout') return node.path;
  if (node.kind === 'repo') return primaryCheckoutPath(node);
  return null;
}
