import type { RowKind } from '../actions/registry.js';
import type { GraphListRow } from './list.js';
import type { GraphModel } from './types.js';
import type { GraphActionRow } from './actions.js';

/**
 * Map a visible graph list row to the registry RowKind used for hints/keymap.
 */
export function graphListRowKind(row: GraphListRow | null | undefined): RowKind | null {
  if (!row) return null;
  if (row.kind === 'spacer') return null;
  if (row.kind === 'commit') return 'graphCommit';
  if (row.kind === 'stash') return 'graphStash';
  return 'graphUncommitted';
}

/**
 * Build a GraphActionRow from the selected list row + model (for local-branch gates).
 */
export function graphActionRowFromSelection(
  row: GraphListRow | null | undefined,
  model: GraphModel | null | undefined,
): GraphActionRow | null {
  if (!row || row.kind === 'spacer') return null;
  if (row.kind === 'uncommitted') return { kind: 'uncommitted' };
  if (row.kind === 'stash') {
    const stash = model?.stashes.find((s) => s.stashRef === row.stashRef);
    if (!stash) return null;
    return { kind: 'stash', stash };
  }
  const commit = model?.commits.find((c) => c.id === row.commitId);
  if (!commit) return null;
  return { kind: 'commit', commit };
}
