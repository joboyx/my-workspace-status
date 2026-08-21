/**
 * Single source of truth for which actions exist and which row kinds accept
 * them. Both the keymap (`keys.ts`) and the hint bar (`StatusBar.tsx`) read
 * this, so what the user is told and what actually fires cannot drift apart.
 *
 * State-dependent visibility (nothing to stage, not behind, depth ≥ 1 write
 * block, …) lives in `gates.ts` and is applied by both hints and dispatch.
 */

import type { TreeNode } from '../model/types.js';
import type { FocusPane } from '../nav/stack.js';
import {
  canCreateBranch,
  canGraphCheckout,
  canStashApply,
  canStashDrop,
  canStashMenu,
  canStashPop,
  type GraphActionRow,
  type GraphStashMenuExtras,
} from '../graph/actions.js';

/**
 * Row kind of the highlighted row — tree node kinds plus graph list kinds.
 */
export type RowKind = TreeNode['kind'] | 'graphCommit' | 'graphStash' | 'graphUncommitted';

/** ViewStack depth index (0 workspace … 2 commitFiles). */
export type NavDepthIndex = 0 | 1 | 2;

/** Stable identifier for an action, used by the dispatcher. */
export type ActionId =
  | 'stage'
  | 'unstage'
  | 'revert'
  | 'fetch'
  | 'pull'
  | 'push'
  | 'defaultBranch'
  | 'branch'
  | 'removeWorktree'
  | 'edit'
  | 'toggleViewed'
  | 'fullFile'
  | 'graphCheckout'
  | 'graphCreateBranch'
  | 'stashApply'
  | 'stashDrop'
  | 'stashMenu'
  | 'stashPop';

/** Declaration of one action: its key, where it is valid, and its risk. */
export interface ActionSpec {
  readonly id: ActionId;
  /** Single input character, or 'ctrl+o' for the control chord. */
  readonly key: string;
  /** Short label shown in the hint bar. */
  readonly label: string;
  readonly kinds: readonly RowKind[];
  /** Destructive actions route through the confirmation flow. */
  readonly destructive: boolean;
  /** When set, action is hint-valid only at these depths. Omitted ⇒ all depths. */
  readonly depths?: readonly NavDepthIndex[];
  /** When set, action is hint-valid only for these panes. Omitted ⇒ both. */
  readonly focusPanes?: readonly FocusPane[];
}

/**
 * Sentinel key for the Ctrl-O chord.
 *
 * Ink reports a control chord as the bare character plus `key.ctrl`, so the
 * registry needs a key string that no raw input character can ever equal.
 * `keys.ts` looks the chord up with this same constant.
 */
export const CTRL_O_KEY = 'ctrl+o';

const SCOPED = ['repo', 'checkout', 'dir', 'file'] as const;

/**
 * Registry order is hint-bar order — most-used actions first.
 * `group` rows (the "no file changes" bucket) accept no actions.
 */
export const ACTIONS: readonly ActionSpec[] = [
  { id: 'stage', key: 's', label: 'stage', kinds: SCOPED, destructive: false },
  { id: 'unstage', key: 'u', label: 'unstage', kinds: SCOPED, destructive: false },
  { id: 'revert', key: 'x', label: 'revert', kinds: SCOPED, destructive: true },
  {
    id: 'fetch',
    key: 'f',
    label: 'fetch',
    kinds: ['workspace', ...SCOPED],
    destructive: false,
  },
  {
    id: 'pull',
    key: 'p',
    label: 'pull',
    kinds: ['workspace', 'repo', 'checkout'],
    destructive: false,
  },
  {
    id: 'push',
    key: 'P',
    label: 'push',
    kinds: ['repo', 'checkout'],
    destructive: false,
  },
  {
    id: 'defaultBranch',
    key: 'd',
    label: 'default branch',
    kinds: ['workspace', 'repo', 'checkout'],
    destructive: false,
  },
  {
    id: 'branch',
    key: 'b',
    label: 'branch',
    kinds: ['repo', 'checkout'],
    destructive: false,
  },
  {
    id: 'removeWorktree',
    key: 'W',
    label: 'remove worktree',
    // Nested linked rows are `checkout`; named-filter linked-only is a flat `repo`.
    // `canRemoveWorktree` still gates primary / family containers.
    kinds: ['checkout', 'repo'],
    destructive: true,
  },
  { id: 'edit', key: 'e', label: 'edit', kinds: ['file'], destructive: false },
  {
    id: 'toggleViewed',
    key: 'space',
    label: 'reviewed',
    kinds: ['file'],
    destructive: false,
    depths: [0],
  },
  { id: 'fullFile', key: CTRL_O_KEY, label: 'full file', kinds: ['file'], destructive: false },
  {
    id: 'graphCheckout',
    key: 'b',
    label: 'checkout',
    kinds: ['graphCommit'],
    destructive: false,
    depths: [0, 1],
  },
  {
    id: 'graphCreateBranch',
    key: 'c',
    label: 'create branch',
    kinds: ['graphCommit'],
    destructive: false,
    depths: [0, 1],
  },
  {
    id: 'stashMenu',
    key: 'S',
    label: 'stash',
    kinds: ['repo', 'checkout', 'dir', 'file', 'graphCommit', 'graphStash', 'graphUncommitted'],
    destructive: false,
    focusPanes: ['left'],
  },
  {
    id: 'stashApply',
    key: 'a',
    label: 'apply stash',
    kinds: ['graphStash'],
    destructive: false,
    depths: [0, 1],
  },
  {
    id: 'stashPop',
    key: 'p',
    label: 'pop stash',
    kinds: ['graphStash'],
    destructive: false,
    depths: [0, 1],
  },
  {
    id: 'stashDrop',
    key: 'D',
    label: 'drop stash',
    kinds: ['graphStash'],
    destructive: true,
    depths: [0, 1],
  },
];

/**
 * User-facing label for an action id, for status messages and prompts.
 * Falls back to the id when the registry has no entry.
 */
export function labelFor(id: ActionId): string {
  return ACTIONS.find((a) => a.id === id)?.label ?? id;
}

/**
 * True when `key` is a single ASCII letter a–z.
 * Used to fold terminal lowercase input onto Shift+letter registry bindings.
 */
function isAsciiLowerLetter(key: string): boolean {
  return key.length === 1 && key >= 'a' && key <= 'z';
}

/**
 * Resolve `key` against the registry for `kind`.
 *
 * Exact match first. If the terminal sent a lowercase letter and that letter
 * is not itself a registered action key (e.g. `d` vs `D`), also try the
 * uppercase form — Ink/terminals typically emit `w` for the W key without
 * Shift, while Shift+letter bindings are stored as `W` / `D` in the registry.
 */
function matchActionKey(
  key: string,
  kind: RowKind,
  predicate: (a: ActionSpec) => boolean = () => true,
): ActionSpec | undefined {
  if (!key) return undefined;
  const exact = ACTIONS.find((a) => a.key === key && a.kinds.includes(kind) && predicate(a));
  if (exact) return exact;
  if (!isAsciiLowerLetter(key)) return undefined;
  // Lowercase is a distinct binding somewhere — do not steal it for Shift+letter.
  if (ACTIONS.some((a) => a.key === key)) return undefined;
  const upper = key.toUpperCase();
  return ACTIONS.find((a) => a.key === upper && a.kinds.includes(kind) && predicate(a));
}

/**
 * The action bound to `key` when the highlighted row is `kind`, or undefined
 * when the key is not an action or is not valid there.
 */
export function actionFor(key: string, kind: RowKind): ActionSpec | undefined {
  return matchActionKey(key, kind);
}

/**
 * Graph-list action for `key` + graph row kind, or undefined.
 * Specs are depth 0/1 (no `focusPanes`); do not require left pane.
 */
export function graphActionForKey(key: string, kind: RowKind): ActionSpec | undefined {
  return matchActionKey(
    key,
    kind,
    (a) => (a.depths?.includes(0) ?? false) || (a.depths?.includes(1) ?? false),
  );
}

/**
 * Every action valid for `kind`, in registry order. Used to render the hint bar.
 */
export function actionsForKind(kind: RowKind): ActionSpec[] {
  return ACTIONS.filter((a) => a.kinds.includes(kind));
}

/**
 * Actions valid for row kind + nav dims (hint bar). Keymap still uses actionFor(kind).
 */
export function actionsForContext(
  kind: RowKind,
  depth: NavDepthIndex,
  focusPane: FocusPane,
): ActionSpec[] {
  return ACTIONS.filter((a) => {
    if (!a.kinds.includes(kind)) return false;
    if (a.depths && !a.depths.includes(depth)) return false;
    if (a.focusPanes && !a.focusPanes.includes(focusPane)) return false;
    return true;
  });
}

/**
 * Extra predicate for hints that depend on row payload (checkoutable refs).
 * `extras` threads worktree dirty / latest stash so `stashMenu` can hide on a
 * clean commit with no stashes; omitted extras keep stash and uncommitted rows
 * visible.
 */
export function actionVisibleForGraphRow(
  action: ActionSpec,
  row: GraphActionRow,
  extras?: GraphStashMenuExtras,
): boolean {
  if (action.id === 'graphCheckout') return canGraphCheckout(row);
  if (action.id === 'graphCreateBranch') return canCreateBranch(row);
  if (action.id === 'stashApply') return canStashApply(row);
  if (action.id === 'stashPop') return canStashPop(row);
  if (action.id === 'stashDrop') return canStashDrop(row);
  if (action.id === 'stashMenu') return canStashMenu(row, extras);
  return true;
}
