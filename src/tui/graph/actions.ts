import type { GraphCommit, GraphStash } from './types.js';
import { stashOpsForContext, type StashOpsContext } from '../stashOps.js';

/**
 * Selected graph row for action gating (P5).
 */
export type GraphActionRow =
  | { kind: 'commit'; commit: GraphCommit }
  | { kind: 'stash'; stash: GraphStash }
  | { kind: 'uncommitted' };

/** Optional worktree/stash facts for `stashMenu` visibility on a graph row. */
export type GraphStashMenuExtras = {
  readonly dirty?: boolean;
  readonly latestStashRef?: string;
};

/**
 * Build overlay context for a graph row. Uncommitted defaults to dirty so
 * existing call sites without extras still advertise `S`.
 */
function stashOpsContextForGraphRow(
  row: GraphActionRow,
  extras?: GraphStashMenuExtras,
): StashOpsContext {
  return {
    kind:
      row.kind === 'stash'
        ? 'graphStash'
        : row.kind === 'uncommitted'
          ? 'graphUncommitted'
          : 'graphCommit',
    dirty: extras?.dirty ?? row.kind === 'uncommitted',
    focusedStashRef: row.kind === 'stash' ? row.stash.stashRef : undefined,
    latestStashRef: extras?.latestStashRef,
  };
}

/**
 * True iff `name` is an origin remote-tracking ref (`origin/...`).
 */
export function isOriginRemoteRef(name: string): boolean {
  return name.startsWith('origin/');
}

/**
 * Local short name for an origin remote-tracking ref.
 * `origin/feature/x` → `feature/x`.
 */
export function localNameFromOriginRef(name: string): string {
  return name.slice('origin/'.length);
}

/**
 * Local branch names pointing at this commit row (sorted, unique).
 */
export function localBranchNames(row: GraphActionRow): string[] {
  if (row.kind !== 'commit') return [];
  const names = row.commit.refs.filter((r) => r.kind === 'local').map((r) => r.name);
  return [...new Set(names)].sort((a, b) => a.localeCompare(b));
}

/**
 * Local and origin-remote names `b` may checkout.
 * Locals first, then `origin/*` remotes; each group unique and sorted.
 */
export function checkoutableBranchNames(row: GraphActionRow): string[] {
  if (row.kind !== 'commit') return [];
  const locals = localBranchNames(row);
  const remotes = [
    ...new Set(
      row.commit.refs
        .filter((r) => r.kind === 'remote' && isOriginRemoteRef(r.name))
        .map((r) => r.name),
    ),
  ].sort((a, b) => a.localeCompare(b));
  return [...locals, ...remotes];
}

/**
 * True when `b` may checkout (at least one local branch or `origin/*` ref).
 */
export function canGraphCheckout(row: GraphActionRow): boolean {
  return checkoutableBranchNames(row).length > 0;
}

/** True when `c` create-branch overlay may open. */
export function canCreateBranch(row: GraphActionRow): boolean {
  return row.kind === 'commit';
}

/** True when stash `a` apply is valid. */
export function canStashApply(row: GraphActionRow): boolean {
  return row.kind === 'stash';
}

/** True when stash `D` drop is valid. */
export function canStashDrop(row: GraphActionRow): boolean {
  return row.kind === 'stash';
}

/** True when stash `p` pop is valid. */
export function canStashPop(row: GraphActionRow): boolean {
  return row.kind === 'stash';
}

/**
 * True when the stash menu (`S`) has at least one valid op for this graph row.
 * Stash and uncommitted rows are visible without extras; a commit row needs
 * `dirty` and/or `latestStashRef` from the caller.
 */
export function canStashMenu(
  row: GraphActionRow,
  extras?: GraphStashMenuExtras,
): boolean {
  return stashOpsForContext(stashOpsContextForGraphRow(row, extras)).length > 0;
}

/**
 * How `b` checkout should proceed given checkoutable names on the row.
 */
export function resolveCheckoutTarget(names: string[]): 'none' | 'single' | 'picker' {
  if (names.length === 0) return 'none';
  if (names.length === 1) return 'single';
  return 'picker';
}

/**
 * Pure checkout vs confirm-then-pull decision for a selected name (no git I/O).
 */
export type GraphCheckoutPlan =
  | { kind: 'checkout'; branch: string }
  | { kind: 'confirmLocalThenPull'; localBranch: string; remoteRef: string };

/**
 * Plan checkout for a picker/single selection.
 * Origin remotes with an out-of-sync (or unread) local counterpart confirm then
 * fast-forward to the selected `origin/*` ref.
 */
export function planGraphCheckout(input: {
  selectedName: string;
  localExists: boolean;
  localSha: string | null;
  remoteSha: string | null;
}): GraphCheckoutPlan {
  if (!isOriginRemoteRef(input.selectedName)) {
    return { kind: 'checkout', branch: input.selectedName };
  }
  const localBranch = localNameFromOriginRef(input.selectedName);
  if (
    input.localExists &&
    (input.localSha == null || input.remoteSha == null || input.localSha !== input.remoteSha)
  ) {
    return {
      kind: 'confirmLocalThenPull',
      localBranch,
      remoteRef: input.selectedName,
    };
  }
  return { kind: 'checkout', branch: localBranch };
}

/**
 * Shared `busyRef` gate: run `work` exclusively, then `afterRelease` with the
 * lock already cleared so a follow-up refresh/fetch can take it.
 *
 * Graph local `b` checkout uses this so snapshot refresh is not a Busy no-op.
 * Origin confirm returns from `work` with a null follow-up and still releases.
 */
export async function runBusyThenRefresh<T>(opts: {
  busyRef: { current: boolean };
  onBusy: () => void;
  work: () => Promise<T>;
  afterRelease?: (result: T) => Promise<void>;
}): Promise<void> {
  if (opts.busyRef.current) {
    opts.onBusy();
    return;
  }
  opts.busyRef.current = true;
  let result!: T;
  try {
    result = await opts.work();
  } finally {
    opts.busyRef.current = false;
  }
  if (opts.afterRelease) await opts.afterRelease(result);
}
