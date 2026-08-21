/**
 * Pure tree/diff pane widths from terminal columns (+ optional session fraction).
 *
 * Never derive these from row label lengths — frozen in App until resize or
 * session drag. Fraction is session-only; default is TREE_WIDTH_FRACTION.
 */

export const TREE_WIDTH_FRACTION = 0.4;

/** Minimum outer width for either pane (matches historical floor). */
const MIN_PANE_COLS = 20;

/** Diff pane horizontal padding (`paddingX={1}` each side). */
const DIFF_PAD_X = 2;

export interface PaneWidths {
  treeWidth: number;
  treeInnerWidth: number;
  diffWidth: number;
}

/**
 * Clamp an outer tree width so both panes stay ≥ {@link MIN_PANE_COLS}
 * (accounting for diff padding) when the terminal is wide enough.
 */
export function clampTreeWidth(termCols: number, treeWidth: number): number {
  const cols = Math.max(1, termCols);
  const minTree = MIN_PANE_COLS;
  const maxTree = Math.max(minTree, cols - MIN_PANE_COLS - DIFF_PAD_X);
  return Math.min(maxTree, Math.max(minTree, Math.round(treeWidth)));
}

/**
 * Convert an outer tree width into a fraction of `termCols` (after clamp).
 */
export function treeFractionFromWidth(termCols: number, treeWidth: number): number {
  const cols = Math.max(1, termCols);
  return clampTreeWidth(cols, treeWidth) / cols;
}

/**
 * Clamp a tree-width fraction so both panes stay ≥ {@link MIN_PANE_COLS}.
 */
export function clampTreeFraction(termCols: number, fraction: number): number {
  const cols = Math.max(1, termCols);
  const raw = Number.isFinite(fraction) ? fraction : TREE_WIDTH_FRACTION;
  return treeFractionFromWidth(cols, Math.floor(cols * raw));
}

/**
 * Compute outer tree width, inner content width, and diff pane width.
 *
 * Matches historical App layout math (default fraction 0.4):
 * - `treeWidth = clamp(floor(termCols * fraction))` (≥ 20; leave room for diff ≥ 20)
 * - `treeInnerWidth = max(8, treeWidth - 3)` (borderRight + paddingX)
 * - `diffWidth = max(20, termCols - treeWidth - 2)` (diff paddingX)
 *
 * @param fraction - Session split; defaults to {@link TREE_WIDTH_FRACTION}.
 */
export function paneWidths(
  termCols: number,
  fraction: number = TREE_WIDTH_FRACTION,
): PaneWidths {
  const cols = Math.max(1, termCols);
  const treeWidth = clampTreeWidth(cols, Math.floor(cols * fraction));
  const treeInnerWidth = Math.max(8, treeWidth - 3);
  const diffWidth = Math.max(MIN_PANE_COLS, cols - treeWidth - DIFF_PAD_X);
  return { treeWidth, treeInnerWidth, diffWidth };
}

/** Structural equality for frozen-width identity checks. */
export function widthsEqual(a: PaneWidths, b: PaneWidths): boolean {
  return (
    a.treeWidth === b.treeWidth &&
    a.treeInnerWidth === b.treeInnerWidth &&
    a.diffWidth === b.diffWidth
  );
}
