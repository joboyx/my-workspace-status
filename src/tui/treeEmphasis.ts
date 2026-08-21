/**
 * Tree-row emphasis (B10): cursor edge + background only.
 * Never returns a force-foreground flag — status / M·A·D colours stay sacred.
 */

import { flashBackground } from './theme.js';
import { flashStrength } from './watch.js';

/** Visual emphasis for one tree row. */
export interface RowEmphasis {
  /** Paint left `CURSOR_BAR` when true (selection). */
  edge: boolean;
  backgroundColor?: string;
}

/**
 * Pick edge bar + row background for cursor / search / flash.
 * Priority: selected → search match → flash → none.
 */
export function treeRowEmphasis(opts: {
  selected: boolean;
  flashedAt?: number;
  now: number;
  searchMatch?: boolean;
  searchBg?: string;
  cursorBg: string;
}): RowEmphasis {
  if (opts.selected) {
    return { edge: true, backgroundColor: opts.cursorBg };
  }
  if (opts.searchMatch && opts.searchBg) {
    return { edge: false, backgroundColor: opts.searchBg };
  }
  const bg = flashBackground(flashStrength(opts.flashedAt, opts.now));
  return { edge: false, backgroundColor: bg };
}
