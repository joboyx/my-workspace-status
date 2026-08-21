/**
 * Horizontal pan helpers for long diff lines (Track D / D1).
 * No word-wrap — callers slice a column window from `diffColOffset`.
 */

export function clampColOffset(offset: number, maxOffset: number): number {
  const max = Math.max(0, maxOffset);
  return Math.max(0, Math.min(offset, max));
}

export function maxColOffset(lineLengths: number[], viewportCols: number): number {
  const longest = lineLengths.reduce((m, n) => Math.max(m, n), 0);
  return Math.max(0, longest - Math.max(1, viewportCols));
}

export function applyPan(offset: number, delta: number, maxOffset: number): number {
  return clampColOffset(offset + delta, maxOffset);
}

/** Plain code-unit slice for the visible column window. */
export function sliceVisible(text: string, offset: number, width: number): string {
  const start = Math.max(0, offset);
  const w = Math.max(0, width);
  return text.slice(start, start + w);
}
