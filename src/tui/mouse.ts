/**
 * SGR mouse reporting helpers for the TUI.
 *
 * Enable/disable sequences are written to stdout; incoming CSI `<…M/m`
 * chunks are parsed incrementally so a split read never drops an event.
 * `mouseListPressAction` maps a left list-press to fold, select, navEnter, or ignore.
 * `mouseClickFocus` maps a pane hit to left/right focus (or none).
 */

export const MOUSE_ENABLE = '\x1b[?1000h\x1b[?1002h\x1b[?1006h';
export const MOUSE_DISABLE = '\x1b[?1006l\x1b[?1002l\x1b[?1000l';

/** Max gap between two left presses that still counts as a double-click. */
export const DOUBLE_CLICK_MS = 400;

/** Last left-press cell + time for double-click detection. */
export type ClickMemory = { x: number; y: number; at: number } | null;

/**
 * True when `ev` is the same cell as `prev` and within `DOUBLE_CLICK_MS`.
 * `prev === null` is never a double-click.
 */
export function isDoubleClick(
  prev: ClickMemory,
  ev: { x: number; y: number },
  now: number,
): boolean {
  if (prev === null) return false;
  if (prev.x !== ev.x || prev.y !== ev.y) return false;
  return now - prev.at < DOUBLE_CLICK_MS;
}

/**
 * Left-press outcome after hit-test and double-click detection.
 */
export type MouseListPressAction = 'fold' | 'select' | 'navEnter' | 'ignore';

/**
 * Hit fields the left-press mapper needs (pane plus optional row / chevron).
 */
export interface MouseListPressInput {
  pane: 'tree' | 'graph' | 'commitFiles' | 'diff' | 'divider' | 'diffSplit' | 'status' | 'none';
  rowIndex: number | null;
  foldChevron: boolean;
  doubleClick: boolean;
}

/**
 * Map a left list-press to fold, select, navEnter, or ignore.
 *
 * Chevron clicks always fold, including a double-click on the chevron.
 * A double-click on a concrete tree / graph / commit-files row is Enter.
 * Divider, diff-split, diff, status, empty, and `rowIndex === null` do not drill.
 */
export function mouseListPressAction(input: MouseListPressInput): MouseListPressAction {
  const { pane, rowIndex, foldChevron, doubleClick } = input;
  if (pane !== 'tree' && pane !== 'graph' && pane !== 'commitFiles') {
    return 'ignore';
  }
  if (rowIndex === null) {
    return 'ignore';
  }
  if (foldChevron) {
    return 'fold';
  }
  if (doubleClick) {
    return 'navEnter';
  }
  return 'select';
}

/**
 * Which column a left-press should focus, if any.
 *
 * Tree focuses left; diff focuses right. Graph and commit-files need
 * `hit.side` (App already calls `focusPaneSide` there). Divider, status,
 * empty, and in-diff split hits do not change focus.
 */
export function mouseClickFocus(pane: MouseListPressInput['pane']): 'left' | 'right' | null {
  if (pane === 'tree') return 'left';
  if (pane === 'diff') return 'right';
  return null;
}

/** One decoded SGR mouse event (1-based cell coordinates). */
export interface MouseEvent {
  button: 'left' | 'wheelUp' | 'wheelDown' | 'other';
  action: 'press' | 'release' | 'drag' | 'wheel';
  x: number;
  y: number;
}

const SGR_RE = /\x1b\[<(\d+);(\d+);(\d+)([Mm])/g;

/**
 * Pull complete SGR mouse events out of `chunk`, returning any trailing
 * incomplete CSI prefix as `rest` for the next read.
 */
export function parseMouseChunk(chunk: string): { events: MouseEvent[]; rest: string } {
  const events: MouseEvent[] = [];
  let lastIndex = 0;
  SGR_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = SGR_RE.exec(chunk)) !== null) {
    lastIndex = SGR_RE.lastIndex;
    const btn = Number(match[1]);
    const x = Number(match[2]);
    const y = Number(match[3]);
    const final = match[4];
    if (btn === 64) {
      events.push({ button: 'wheelUp', action: 'wheel', x, y });
    } else if (btn === 65) {
      events.push({ button: 'wheelDown', action: 'wheel', x, y });
    } else if (btn === 32 && final === 'M') {
      // Left-button drag motion (button 0 + 32 motion bit) under mode 1002.
      events.push({ button: 'left', action: 'drag', x, y });
    } else if (btn === 0) {
      events.push({
        button: 'left',
        action: final === 'M' ? 'press' : 'release',
        x,
        y,
      });
    } else {
      events.push({
        button: 'other',
        action: final === 'M' ? 'press' : 'release',
        x,
        y,
      });
    }
  }
  // Incomplete CSI naturally remains when the regex does not match.
  const rest = chunk.slice(lastIndex);
  return { events, rest };
}
