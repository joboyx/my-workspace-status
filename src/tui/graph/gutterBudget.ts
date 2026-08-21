/**
 * Pane-relative graph gutter budget + commit-anchored cell window.
 *
 * Topology layout stays full-width; paint clips the gutter so subject/refs
 * can consume leftover horizontal space on wide panes.
 */

import { CELL_W } from './glyphs.js';
import type { GraphCell } from './types.js';

/** Gutter may use at most this fraction of the graph list inner width. */
export const GUTTER_MAX_FRACTION = 0.3;

/**
 * Always leave at least this many columns for refs+subject (after the
 * gutter/subject gap). Meta columns still drop via existing row logic.
 */
export const MIN_SUBJECT_FLOOR = 24;

/**
 * Hybrid max gutter columns for a graph list of `paneWidth` columns
 * (segment budget; cursor bar is already excluded by the caller).
 */
export function graphGutterCap(paneWidth: number): number {
  const w = Math.max(1, Math.floor(paneWidth));
  const byFraction = Math.max(1, Math.floor(w * GUTTER_MAX_FRACTION));
  // gap (1) + subject floor; meta competes later via drop order
  const byFloor = Math.max(1, w - 1 - MIN_SUBJECT_FLOOR);
  return Math.max(1, Math.min(byFraction, byFloor));
}

/**
 * Shared gutter width for a loaded window: topology need, capped by pane.
 */
export function resolveGraphWidth(topologyWidth: number, paneWidth: number): number {
  const topo = Math.max(0, Math.floor(topologyWidth));
  if (topo <= 0) return 0;
  return Math.min(topo, graphGutterCap(paneWidth));
}

/**
 * Focus column for clipping — prefer the painted node, else lane × CELL_W.
 */
export function gutterFocusCol(cells: readonly GraphCell[], lane: number): number {
  const nodeIdx = cells.findIndex((c) => c.role === 'node');
  if (nodeIdx >= 0) return nodeIdx;
  if (cells.length === 0) return 0;
  return Math.max(0, Math.min(lane * CELL_W, cells.length - 1));
}

/**
 * Window of `budget` cells anchored so the commit node stays visible
 * (centered when possible, clamped to the cell array).
 *
 * `extraCols` (stash join / leaf columns) are included in the window when
 * they fit beside the focus; if the span exceeds the budget, the node stays
 * and the window shifts toward those columns.
 */
export function sliceCellsAroundLane(
  cells: readonly GraphCell[],
  budget: number,
  lane: number,
  extraCols: readonly number[] = [],
): GraphCell[] {
  const b = Math.max(0, Math.floor(budget));
  if (b <= 0) return [];
  if (cells.length <= b) {
    return [...cells];
  }
  const focus = gutterFocusCol(cells, lane);
  const wanted = [
    focus,
    ...extraCols.filter((c) => Number.isFinite(c) && c >= 0 && c < cells.length),
  ];
  const minW = Math.min(...wanted);
  const maxW = Math.max(...wanted);
  const span = maxW - minW + 1;
  let start: number;
  if (span <= b) {
    const slack = b - span;
    start = Math.max(0, Math.min(minW - Math.floor(slack / 2), cells.length - b));
  } else {
    const extras = wanted.filter((c) => c !== focus);
    const extraBias =
      extras.length > 0 ? extras.reduce((sum, c) => sum + c, 0) / extras.length : focus;
    if (extraBias >= focus) {
      start = Math.max(0, Math.min(focus, cells.length - b));
    } else {
      start = Math.max(0, Math.min(focus - (b - 1), cells.length - b));
    }
  }
  return cells.slice(start, start + b);
}
