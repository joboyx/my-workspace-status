/**
 * Pure side-by-side diff column widths from pane width + session fraction.
 *
 * Same interaction model as the tree/diff pane split (`layoutWidths.ts`):
 * session-only fraction, clamp so both columns stay usable, drag maps a
 * terminal x onto a left-column width.
 */

/** Below this pane width side-by-side columns fall back to inline. */
export const NARROW_SXS = 100;

/** Default 50/50 split — matches the historical `floor((width - 1) / 2)`. */
export const DIFF_SPLIT_FRACTION = 0.5;

/**
 * Minimum outer width for either side-by-side column (gutter + sign + some
 * code). When the pane is too narrow for two mins, both sides share the inner
 * width instead of overflowing.
 */
export const MIN_DIFF_COL = 16;

/**
 * Columns after the tree border before right-pane content starts
 * (`paddingX={1}` on the right box).
 */
export const DIFF_CONTENT_PAD = 2;

export interface SideBySideWidths {
  leftWidth: number;
  rightWidth: number;
}

/**
 * 1-based terminal column of the first right-pane content cell.
 */
export function diffContentOriginX(treeWidth: number): number {
  return treeWidth + DIFF_CONTENT_PAD;
}

/**
 * 1-based terminal column of the in-diff vertical RULE.
 */
export function diffSplitRuleX(treeWidth: number, leftWidth: number): number {
  return diffContentOriginX(treeWidth) + leftWidth;
}

/**
 * True when the right pane is painting a side-by-side split (not the
 * narrow-pane inline fallback).
 */
export function isSideBySideSplit(mode: 'inline' | 'sideBySide', paneWidth: number): boolean {
  return mode === 'sideBySide' && paneWidth >= NARROW_SXS;
}

/**
 * Clamp a left-column width so both sides stay ≥ {@link MIN_DIFF_COL}
 * when the pane is wide enough.
 */
export function clampDiffLeftWidth(paneWidth: number, leftWidth: number): number {
  const inner = Math.max(1, Math.round(paneWidth) - 1);
  const minCol = Math.min(MIN_DIFF_COL, Math.floor(inner / 2));
  const maxCol = Math.max(minCol, inner - minCol);
  const raw = Number.isFinite(leftWidth) ? leftWidth : inner * DIFF_SPLIT_FRACTION;
  return Math.min(maxCol, Math.max(minCol, Math.round(raw)));
}

/**
 * Convert a left-column width into a fraction of the inner (pane − RULE) width.
 */
export function diffSplitFractionFromLeftWidth(paneWidth: number, leftWidth: number): number {
  const inner = Math.max(1, Math.round(paneWidth) - 1);
  return clampDiffLeftWidth(paneWidth, leftWidth) / inner;
}

/**
 * Map a 1-based terminal x onto a split fraction (drag / press).
 */
export function diffSplitFractionFromTerminalX(
  treeWidth: number,
  paneWidth: number,
  x: number,
): number {
  return diffSplitFractionFromLeftWidth(paneWidth, x - diffContentOriginX(treeWidth));
}

/**
 * Left / right column widths for one side-by-side diff row (`left + RULE + right`).
 *
 * Default fraction 0.5 reproduces `Math.floor((width - 1) / 2)`.
 */
export function sideBySideColumnWidths(
  paneWidth: number,
  fraction: number = DIFF_SPLIT_FRACTION,
): SideBySideWidths {
  const inner = Math.max(1, Math.round(paneWidth) - 1);
  const raw = Number.isFinite(fraction) ? fraction : DIFF_SPLIT_FRACTION;
  const leftWidth = clampDiffLeftWidth(paneWidth, Math.floor(inner * raw));
  return { leftWidth, rightWidth: inner - leftWidth };
}
