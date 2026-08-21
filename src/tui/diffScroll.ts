/**
 * Vertical scroll helpers for keeping a diff row in view (Track D / D2).
 */

import type { DiffRow } from './diff/rows.js';

/**
 * Clamp vertical diff scroll so PageDown / wheel deltas cannot grow past EOF
 * (B8). Matches DiffPane's paint window: max start is `rowCount - viewHeight`.
 */
export function clampDiffScroll(
  scroll: number,
  rowCount: number,
  viewHeight: number,
): number {
  return Math.max(0, Math.min(scroll, Math.max(0, rowCount - Math.max(1, viewHeight))));
}

/**
 * `gg` / `G` on a focused diff: start of file or EOF (same clamp as Page/j/k).
 */
export function diffScrollForMoveTo(
  edge: 'start' | 'end',
  rowCount: number,
  viewHeight: number,
): number {
  return clampDiffScroll(edge === 'start' ? 0 : rowCount, rowCount, viewHeight);
}

export function scrollToKeepRow(opts: {
  rowIndex: number;
  viewHeight: number;
  rowCount: number;
  prefer: 'center' | 'upperThird';
}): number {
  const viewHeight = Math.max(1, opts.viewHeight);
  const maxStart = Math.max(0, opts.rowCount - viewHeight);
  const offset =
    opts.prefer === 'center'
      ? Math.floor(viewHeight / 2)
      : Math.floor(viewHeight / 3);
  const start = opts.rowIndex - offset;
  return Math.max(0, Math.min(start, maxStart));
}

/**
 * Pick an anchor row for full-file toggle: first visible add/del in the
 * viewport, else nearest hunk header at or above `scroll`, else `scroll`.
 */
export function anchorRowIndex(rows: DiffRow[], scroll: number, viewHeight: number): number {
  const start = Math.max(0, scroll);
  const end = Math.min(rows.length, start + Math.max(1, viewHeight));
  for (let i = start; i < end; i++) {
    const row = rows[i]!;
    if (row.kind === 'line' && (row.left.kind === 'add' || row.left.kind === 'del')) {
      return i;
    }
    if (
      row.kind === 'line' &&
      row.right &&
      (row.right.kind === 'add' || row.right.kind === 'del')
    ) {
      return i;
    }
  }
  for (let i = start; i >= 0; i--) {
    if (rows[i]?.kind === 'hunk') return i;
  }
  return start;
}
