/**
 * Pure page-navigation helpers for focused-pane PageUp/Down (C3 + B8).
 */

/** Clamp `index` into `[0, max(0, len - 1)]`. Empty list → 0. */
export function clampIndex(index: number, len: number): number {
  if (len <= 0) return 0;
  return Math.max(0, Math.min(index, len - 1));
}

/** Rows to jump per page — leave one row of overlap. */
export function pageDelta(visibleRows: number): number {
  return Math.max(1, visibleRows - 1);
}

/**
 * Move `cursor` by ±`page` within `[0, len)`, clamping at ends (B8).
 * Repeated edge presses are no-ops (same index).
 */
export function applyPageMove(
  cursor: number,
  len: number,
  page: number,
  dir: 1 | -1,
): number {
  return clampIndex(cursor + dir * page, len);
}
