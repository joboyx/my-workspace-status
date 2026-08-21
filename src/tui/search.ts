/**
 * Pure vim search helpers (C5).
 * Matches only — does not filter or hide rows.
 */

import type { DiffRow } from './diff/rows.js';
import type { GraphFocusTarget } from './graph/focus.js';
import { flatten } from './model/flatten.js';
import { unfoldAncestors } from './model/fold.js';
import type { TreeNode } from './model/types.js';

/**
 * Pane `/` was opened against.
 * `n`/`p` stay on this pane even if focus moves later.
 */
export type SearchPaneTarget = GraphFocusTarget;

/**
 * Armed `/` search. `target` is captured at search start so later focus
 * changes do not retarget `n`/`p`.
 */
export type SearchState = {
  query: string;
  matchIndex: number;
  target: SearchPaneTarget;
};

/**
 * Rows that `/` can highlight by stable id.
 * Graph spacers set `selectable: false` and are not search hits.
 */
export type SearchLabeledRow = {
  id: string;
  label: string;
  selectable?: boolean;
};

/** Case-insensitive substring matches on `label`; empty query → []. */
export function matchIndices(rows: { label: string }[], query: string): number[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const out: number[] = [];
  for (let i = 0; i < rows.length; i++) {
    if (rows[i]!.label.toLowerCase().includes(q)) out.push(i);
  }
  return out;
}

/** First match index, or null when none. */
export function firstMatchIndex(indices: number[]): number | null {
  return indices.length ? indices[0]! : null;
}

/**
 * Next/prev match with wrap. If `current` is not in `indices`, jump to
 * first (dir +1) or last (dir -1). Empty indices → `current` unchanged.
 */
export function stepMatch(indices: number[], current: number, dir: 1 | -1): number {
  if (indices.length === 0) return current;
  const pos = indices.indexOf(current);
  if (pos < 0) return dir === 1 ? indices[0]! : indices[indices.length - 1]!;
  const next = (pos + dir + indices.length) % indices.length;
  return indices[next]!;
}

/**
 * Visible text used to match one diff row (cells and hunk headers).
 * Section headers return empty so they are never hits.
 */
function diffRowSearchText(row: DiffRow): string {
  if (row.kind === 'section') return '';
  if (row.kind === 'hunk') return row.text;
  const left = row.left.text;
  const right = row.right?.text ?? '';
  return right ? `${left}\n${right}` : left;
}

/**
 * Indices of diff rows whose cell or hunk text contains `query`.
 * Section headers are not matches. Rows are never removed.
 */
export function matchDiffRowIndices(rows: DiffRow[], query: string): number[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const out: number[] = [];
  for (let i = 0; i < rows.length; i++) {
    if (diffRowSearchText(rows[i]!).toLowerCase().includes(q)) out.push(i);
  }
  return out;
}

/**
 * Stable ids of rows whose label matches `query`, in list order.
 * Rows with `selectable: false` are skipped. Empty query → [].
 */
export function collectMatchIds(rows: SearchLabeledRow[], query: string): string[] {
  const q = query.trim();
  if (!q) return [];
  return matchIndices(rows, query)
    .filter((i) => rows[i]!.selectable !== false)
    .map((i) => rows[i]!.id);
}

/**
 * Next/prev match id with wrap. If `currentId` is not in `ids`, jump to
 * first (dir +1) or last (dir -1). Empty ids → null.
 */
export function stepMatchId(
  ids: readonly string[],
  currentId: string | null,
  dir: 1 | -1,
): string | null {
  if (ids.length === 0) return null;
  const pos = currentId ? ids.indexOf(currentId) : -1;
  if (pos < 0) return dir === 1 ? ids[0]! : ids[ids.length - 1]!;
  return ids[(pos + dir + ids.length) % ids.length]!;
}

/**
 * Match to focus: first hit when `dir` is 0, otherwise next/prev from
 * `currentId`. Empty query or no hits → null.
 */
export function nextSearchMatchId(
  rows: SearchLabeledRow[],
  query: string,
  currentId: string | null,
  dir: 1 | -1 | 0,
): string | null {
  const ids = collectMatchIds(rows, query);
  if (ids.length === 0) return null;
  if (dir === 0) return ids[0]!;
  return stepMatchId(ids, currentId, dir);
}

/**
 * Focus a workspace-tree search match. Matches include folded rows.
 * Unfolds ancestors of the match about to receive focus. Other matches
 * stay folded until visited.
 */
export function focusTreeSearchMatch(opts: {
  tree: TreeNode;
  folds: Set<string>;
  query: string;
  currentId: string | null;
  dir: 1 | -1 | 0;
}): { folds: Set<string>; focusId: string | null } {
  const allRows = flatten(opts.tree, new Set());
  const focusId = nextSearchMatchId(allRows, opts.query, opts.currentId, opts.dir);
  if (!focusId) return { folds: opts.folds, focusId: null };
  return { folds: unfoldAncestors(opts.tree, opts.folds, focusId), focusId };
}

/**
 * Row ids to highlight for the pane bound at search start.
 * Diff targets return an empty set — callers use {@link matchDiffRowIndices}.
 */
export function collectSearchMatchIds(opts: {
  target: SearchPaneTarget;
  query: string;
  treeRows: SearchLabeledRow[];
  graphRows: SearchLabeledRow[];
  commitFileRows: SearchLabeledRow[];
}): Set<string> {
  const q = opts.query.trim();
  if (!q) return new Set();
  if (opts.target === 'none') return new Set();
  const rows =
    opts.target === 'graph'
      ? opts.graphRows
      : opts.target === 'commitFiles'
        ? opts.commitFileRows
        : opts.treeRows;
  return new Set(collectMatchIds(rows, q));
}
