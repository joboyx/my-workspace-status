/**
 * Map terminal mouse coordinates onto tree / graph / commit-file / diff /
 * divider / in-diff split / status hit targets.
 *
 * Tree / graph / commit-file row geometry must stay in sync with the painted
 * panes — all list windows use `treeViewportStart` (graph via `graphViewportStart`).
 *
 * `treeWidth` must come from `paneWidths(termCols, sessionFraction)` (frozen
 * in App until resize or drag). Hit columns are independent of tree label
 * lengths (B1).
 *
 * The divider band (`treeWidth` and `treeWidth ± 1`, clamped to valid cols)
 * is checked before left/right mapping so the border is not a list click.
 * When `diffSplitRuleX` is set, the in-diff RULE uses the same ±1 band.
 *
 * List hits always name the column's list kind for wheel routing, even on
 * header/footer/empty viewport cells (`rowIndex === null`). Clicks only
 * select when `rowIndex` is set.
 */

import { PANE_TITLE_ROWS } from './focusChrome.js';

/** Which list (or non-list) a column is currently painting. */
export type HitListKind = 'tree' | 'graph' | 'commitFiles' | 'diff' | 'empty';

/** Result of mapping a 1-based cell click onto a pane. */
export type HitTarget =
  | { pane: 'tree'; rowIndex: number | null; foldChevron: boolean }
  | { pane: 'graph'; side: 'left' | 'right'; rowIndex: number | null }
  | {
      pane: 'commitFiles';
      side: 'left' | 'right';
      rowIndex: number | null;
      foldChevron: boolean;
    }
  | { pane: 'diff' }
  | { pane: 'divider' }
  | { pane: 'diffSplit' }
  | { pane: 'status' }
  | { pane: 'none' };

/** Minimal row shape for fold-chevron column math. */
export type HitRow = { depth: number };

/** Layout for one column's list body (after the shared pane title chip). */
export interface HitListLayout {
  kind: HitListKind;
  rowCount: number;
  cursor: number;
  rows: ReadonlyArray<HitRow>;
  /**
   * Non-list chrome rows at the top of the body (graph sync header, commit
   * detail title/subtitle). Wheel still targets the list; clicks do not select.
   */
  headerLines: number;
  /**
   * Non-list chrome rows at the bottom (graph footer / loading-older).
   * Wheel still targets the list; clicks do not select.
   */
  footerLines: number;
  /**
   * Viewport height used for `treeViewportStart` — must match the pane's
   * list slice (GraphPane `chrome.listHeight`, TreePane `height`).
   */
  listHeight: number;
}

export interface HitTestArgs {
  /** 1-based column. */
  x: number;
  /** 1-based row. */
  y: number;
  termCols: number;
  termRows: number;
  /** Outer tree box width including the right border — from `paneWidths`, not labels. */
  treeWidth: number;
  paneHeight: number;
  statusLines: number;
  /**
   * @deprecated Prefer `left.rowCount`. Kept so existing call sites / tests
   * that only pass left-tree geometry keep working.
   */
  rowCount?: number;
  /**
   * @deprecated Prefer `left.cursor`.
   */
  cursor?: number;
  /**
   * @deprecated Prefer `left.rows`.
   */
  rows?: ReadonlyArray<HitRow & { id?: string; node?: { kind: string } }>;
  /**
   * @deprecated Unused for geometry; retained for call-site compatibility.
   */
  folds?: Set<string>;
  /** Left column body list (defaults to legacy tree fields when omitted). */
  left?: HitListLayout;
  /** Right column body list / diff (defaults to plain diff). */
  right?: HitListLayout;
  /**
   * 1-based terminal column of the in-diff side-by-side RULE when the right
   * pane is painting a split. Hit band is ruleX ± 1 (same as the pane divider).
   */
  diffSplitRuleX?: number | null;
  /** Frozen right-pane content width — used to map drag x onto a split fraction. */
  diffWidth?: number;
}

/**
 * First visible row index for a viewport centred on `cursor`.
 * Same formula `TreePane` / `GraphPane` use when slicing the row list.
 */
export function treeViewportStart(rowCount: number, cursor: number, height: number): number {
  const viewHeight = Math.max(1, height);
  const maxStart = Math.max(0, rowCount - viewHeight);
  const idealStart = Math.max(0, cursor - Math.floor(viewHeight / 2));
  return Math.min(idealStart, maxStart);
}

/**
 * Content-column of the fold chevron for a row at `depth`.
 *
 * Tree content (after left padding) is: cursor bar (1) + indent (`depth*2`) +
 * chevron. So chevron contentCol = `1 + depth * 2` (0-based within content).
 */
export function foldChevronContentCol(depth: number): number {
  return 1 + depth * 2;
}

/**
 * True when `x` is on/near the tree's right border (`treeWidth ± 1`).
 */
export function isDividerColumn(x: number, treeWidth: number, termCols: number): boolean {
  for (const col of [treeWidth - 1, treeWidth, treeWidth + 1]) {
    if (col >= 1 && col <= termCols && col === x) return true;
  }
  return false;
}

/**
 * Resolve a body-relative 0-based `bodyY` onto a selectable list row index,
 * or `null` when the cell is header/footer chrome or an empty viewport slot.
 *
 * Bounds: `[headerLines, headerLines + listHeight)` is the list window;
 * `[headerLines + listHeight, headerLines + listHeight + footerLines)` is
 * footer chrome (still the same pane for wheel, but not a row click).
 */
export function listRowIndexAtBodyY(bodyY: number, layout: HitListLayout): number | null {
  if (layout.kind === 'diff' || layout.kind === 'empty') return null;
  const listStart = layout.headerLines;
  const listEnd = layout.headerLines + layout.listHeight;
  const footerEnd = listEnd + Math.max(0, layout.footerLines);
  if (bodyY < listStart || bodyY >= listEnd) {
    // Explicitly acknowledge footer band (and anything past it).
    if (bodyY >= listEnd && bodyY < footerEnd) return null;
    return null;
  }
  const listY = bodyY - listStart;
  const start = treeViewportStart(layout.rowCount, layout.cursor, layout.listHeight);
  const rowIndex = start + listY;
  if (rowIndex < 0 || rowIndex >= layout.rowCount || rowIndex >= layout.rows.length) {
    return null;
  }
  return rowIndex;
}

/** True when this column paints a keyboard-selectable list. */
export function isListHitKind(kind: HitListKind): kind is 'tree' | 'graph' | 'commitFiles' {
  return kind === 'tree' || kind === 'graph' || kind === 'commitFiles';
}

function defaultLeft(args: HitTestArgs): HitListLayout {
  if (args.left) return args.left;
  const listHeight = Math.max(1, args.paneHeight - PANE_TITLE_ROWS);
  return {
    kind: 'tree',
    rowCount: args.rowCount ?? 0,
    cursor: args.cursor ?? 0,
    rows: args.rows ?? [],
    headerLines: 0,
    footerLines: 0,
    listHeight,
  };
}

function defaultRight(args: HitTestArgs): HitListLayout {
  if (args.right) return args.right;
  return {
    kind: 'diff',
    rowCount: 0,
    cursor: 0,
    rows: [],
    headerLines: 0,
    footerLines: 0,
    listHeight: Math.max(1, args.paneHeight - PANE_TITLE_ROWS),
  };
}

function hitForList(
  side: 'left' | 'right',
  layout: HitListLayout,
  rowIndex: number | null,
  foldChevron: boolean,
): HitTarget {
  if (layout.kind === 'graph') {
    return { pane: 'graph', side, rowIndex };
  }
  if (layout.kind === 'commitFiles') {
    return { pane: 'commitFiles', side, rowIndex, foldChevron };
  }
  // tree (left only in practice)
  return { pane: 'tree', rowIndex, foldChevron };
}

function rightColumnHit(
  layout: HitListLayout,
  rowIndex: number | null,
  foldChevron: boolean,
): HitTarget {
  if (layout.kind === 'diff') return { pane: 'diff' };
  if (layout.kind === 'empty') return { pane: 'none' };
  return hitForList('right', layout, rowIndex, foldChevron);
}

/**
 * Map a mouse cell onto a pane / row / fold-chevron / divider target.
 *
 * Layout assumptions (matches `App.tsx`):
 * - Columns `1..treeWidth` are the left box (no left border, `paddingX={1}`,
 *   right border at `treeWidth`).
 * - Divider hit band: `treeWidth` and `treeWidth ± 1` (clamped to valid cols),
 *   checked before left/right so the border is not a list click.
 * - Content starts at column `2` (left pad) on the left; right content starts
 *   at `treeWidth + 2` (gap after border + right pad).
 * - Rows `1..PANE_TITLE_ROWS` are the pane title chips — still attributed to
 *   the column's list/diff for wheel routing (`rowIndex` null).
 * - Rows `PANE_TITLE_ROWS+1..paneHeight` are the body; below that is status.
 */
export function hitTest(args: HitTestArgs): HitTarget {
  const { x, y, termCols, termRows, treeWidth, paneHeight, statusLines } = args;
  const left = defaultLeft(args);
  const right = defaultRight(args);

  if (x < 1 || y < 1 || x > termCols || y > termRows) {
    return { pane: 'none' };
  }

  if (y > paneHeight) {
    const statusBottom = paneHeight + statusLines;
    if (y <= statusBottom && y <= termRows) return { pane: 'status' };
    return { pane: 'none' };
  }

  // Divider before left/right so the border is not treated as a list click.
  if (isDividerColumn(x, treeWidth, termCols)) {
    return { pane: 'divider' };
  }

  // In-diff split RULE (only when App supplies a rule column — SxS active).
  if (
    args.diffSplitRuleX != null &&
    args.diffSplitRuleX >= 1 &&
    isDividerColumn(x, args.diffSplitRuleX, termCols)
  ) {
    return { pane: 'diffSplit' };
  }

  // Title chip: attribute to the column for wheel; no row select.
  if (y <= PANE_TITLE_ROWS) {
    if (x > treeWidth) {
      return rightColumnHit(right, null, false);
    }
    if (isListHitKind(left.kind)) {
      return hitForList('left', left, null, false);
    }
    return { pane: 'none' };
  }

  const bodyY = y - 1 - PANE_TITLE_ROWS;

  if (x > treeWidth) {
    if (right.kind === 'diff') return { pane: 'diff' };
    if (right.kind === 'empty') return { pane: 'none' };
    const rowIndex = listRowIndexAtBodyY(bodyY, right);
    // Right column also uses paddingX={1}; content col 0 is terminal x = treeWidth + 2.
    const contentCol = x - (treeWidth + 2);
    const row = rowIndex !== null ? right.rows[rowIndex] : undefined;
    const foldChevron =
      right.kind === 'commitFiles' && row ? contentCol === foldChevronContentCol(row.depth) : false;
    return hitForList('right', right, rowIndex, foldChevron);
  }

  // Left column — always the painted list kind (wheel on chrome / empty slots).
  if (!isListHitKind(left.kind)) {
    return { pane: 'none' };
  }
  const rowIndex = listRowIndexAtBodyY(bodyY, left);
  const contentCol = x - 2;
  const row = rowIndex !== null ? left.rows[rowIndex] : undefined;
  const foldChevron =
    (left.kind === 'tree' || left.kind === 'commitFiles') && row
      ? contentCol === foldChevronContentCol(row.depth)
      : false;
  return hitForList('left', left, rowIndex, foldChevron);
}
