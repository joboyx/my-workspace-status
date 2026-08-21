/**
 * Which `Action` types `useAppState.runAction` must forward to
 * `useActions.dispatch`. Kept as data so a missing switch arm fails tests
 * instead of silently no-op'ing (the removeWorktree wire bug).
 */

import type { Action } from './keys.js';

/**
 * Tree write / confirm / picker actions owned by `useActions`.
 * `edit` is also listed — `runAction` may short-circuit for commit-files
 * before forwarding.
 */
export const USE_ACTIONS_FORWARDED: ReadonlySet<Action['type']> = new Set([
  'stage',
  'unstage',
  'revert',
  'confirmYes',
  'confirmYesClean',
  'confirmNo',
  'edit',
  'fetch',
  'pull',
  'push',
  'defaultBranch',
  'branch',
  'removeWorktree',
]);

/**
 * True when `runAction` should hand `action` to `useActions.dispatch`
 * (after depth / pane gates).
 */
export function shouldForwardToUseActions(type: Action['type']): boolean {
  return USE_ACTIONS_FORWARDED.has(type);
}
