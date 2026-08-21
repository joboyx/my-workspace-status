/**
 * Internal directional connection model for graph gutter painting.
 *
 * Cells accumulate `up`/`down`/`left`/`right` before a final glyph is chosen.
 * Public `GraphCell` remains the display model (`ch` + `colorLane` + `role`).
 */

import { CELL_W, type GraphGlyphSet } from './glyphs.js';
import type { GraphCell } from './types.js';

/**
 * Per-cell topology before glyph resolution.
 */
export type TopoCell = {
  up: boolean;
  down: boolean;
  left: boolean;
  right: boolean;
  colorLane: number | null;
  role: 'node' | 'pipe' | 'blank';
};

/**
 * Empty topology cell.
 */
export function emptyTopo(): TopoCell {
  return {
    up: false,
    down: false,
    left: false,
    right: false,
    colorLane: null,
    role: 'blank',
  };
}

/**
 * Merge connection flags into a cell.
 *
 * Colour priority: node wins; through-rail (`up`/`down` without being a fresh
 * overwrite of only horizontals) keeps its lane when adding horizontals; else
 * the incoming `colorLane` wins for new pipe content.
 */
export function connect(
  cell: TopoCell,
  dirs: Partial<Pick<TopoCell, 'up' | 'down' | 'left' | 'right'>>,
  colorLane: number | null,
  role: TopoCell['role'] = 'pipe',
): void {
  if (cell.role === 'node' && role !== 'node') {
    // Still record pipe dirs around a node? Nodes replace the glyph; skip.
    return;
  }
  const hadVertical = cell.up || cell.down;
  if (dirs.up) cell.up = true;
  if (dirs.down) cell.down = true;
  if (dirs.left) cell.left = true;
  if (dirs.right) cell.right = true;

  if (role === 'node') {
    cell.role = 'node';
    cell.colorLane = colorLane;
    return;
  }

  if (cell.role === 'blank') cell.role = role;

  // Through-rail keeps its colour when a horizontal is layered on.
  const addingHorizontal = Boolean(dirs.left || dirs.right);
  const addingVertical = Boolean(dirs.up || dirs.down);
  if (hadVertical && addingHorizontal && !addingVertical) {
    // keep existing colorLane
  } else if (colorLane !== null) {
    if (!hadVertical || addingVertical || cell.colorLane === null) {
      cell.colorLane = colorLane;
    }
  }
}

/**
 * Resolve Unicode/ASCII glyph from final connectivity.
 */
export function glyphFromTopo(cell: TopoCell, g: GraphGlyphSet): string {
  if (cell.role === 'node') return g.commit;
  if (cell.role === 'blank') return ' ';

  const { up, down, left, right } = cell;
  if (up && down && left && right) return g.cross;
  if (left && up && down && !right) return g.teeLeft;
  if (right && up && down && !left) return g.teeRight;
  if (left && right && down && !up) return g.teeDown;
  if (left && right && up && !down) return g.teeUp;
  if (left && down && !right && !up) return g.cornerDownRight;
  if (right && down && !left && !up) return g.cornerDownLeft;
  if (left && up && !right && !down) return g.cornerUpRight;
  if (right && up && !left && !down) return g.cornerUpLeft;
  if (up && down && !left && !right) return g.vertical;
  if (left && right && !up && !down) return g.horizontal;
  if (left || right) return g.horizontal;
  if (up || down) return g.vertical;
  return ' ';
}

/**
 * Materialise topology row into display cells.
 */
export function topoToCells(row: TopoCell[], g: GraphGlyphSet): GraphCell[] {
  return row.map((cell) => ({
    ch: glyphFromTopo(cell, g),
    colorLane: cell.colorLane,
    role: cell.role === 'blank' ? 'blank' : cell.role,
  }));
}

/**
 * Ensure topology row is at least `width` cells.
 */
export function ensureTopoWidth(row: TopoCell[], width: number): void {
  while (row.length < width) row.push(emptyTopo());
}

/**
 * Add a vertical through-rail on a lane column.
 */
export function addVertical(row: TopoCell[], lane: number, colorLane: number): void {
  const col = lane * CELL_W;
  ensureTopoWidth(row, col + 1);
  connect(row[col]!, { up: true, down: true }, colorLane, 'pipe');
}

/**
 * Add a horizontal run between lane columns (exclusive of endpoints' lane glyphs
 * are handled by callers via corners/tees; this fills bridge columns).
 */
export function addHorizontalBridge(
  row: TopoCell[],
  fromLane: number,
  toLane: number,
  colorLane: number,
): void {
  const lo = Math.min(fromLane, toLane);
  const hi = Math.max(fromLane, toLane);
  const start = lo * CELL_W;
  const end = hi * CELL_W;
  ensureTopoWidth(row, end + 1);
  for (let col = start + 1; col < end; col++) {
    // Lane columns and spacers both take left+right; through-rails compose to ┼.
    connect(row[col]!, { left: true, right: true }, colorLane, 'pipe');
  }
}

/**
 * Whether a topology cell has an upward / downward stem before node clear.
 */
export function topoStemDirs(
  row: readonly TopoCell[],
  col: number,
  commitLane: number,
): { up: boolean; down: boolean } {
  const nodeCol = commitLane * CELL_W;
  if (col === nodeCol) return { up: true, down: true };
  const cell = row[col];
  if (!cell) return { up: false, down: false };
  return { up: cell.up, down: cell.down };
}

/**
 * Open a new secondary lane: corner at `toLane` (down + toward commit).
 */
export function addOpenCorner(
  row: TopoCell[],
  commitLane: number,
  toLane: number,
  colorLane: number,
): void {
  const col = toLane * CELL_W;
  ensureTopoWidth(row, col + 1);
  if (toLane > commitLane) {
    connect(row[col]!, { left: true, down: true }, colorLane, 'pipe');
  } else {
    connect(row[col]!, { right: true, down: true }, colorLane, 'pipe');
  }
  addHorizontalBridge(row, commitLane, toLane, colorLane);
}

/**
 * Close an incoming waiter into the commit: up-corner at `fromLane`.
 */
export function addJoinCorner(
  row: TopoCell[],
  commitLane: number,
  fromLane: number,
  colorLane: number,
): void {
  const col = fromLane * CELL_W;
  ensureTopoWidth(row, col + 1);
  if (fromLane > commitLane) {
    connect(row[col]!, { left: true, up: true }, colorLane, 'pipe');
  } else {
    connect(row[col]!, { right: true, up: true }, colorLane, 'pipe');
  }
  addHorizontalBridge(row, commitLane, fromLane, colorLane);
}

/**
 * Link commit horizontally into an already-live secondary parent rail (tee).
 */
export function addLinkTee(
  row: TopoCell[],
  commitLane: number,
  targetLane: number,
  colorLane: number,
): void {
  const col = targetLane * CELL_W;
  ensureTopoWidth(row, col + 1);
  // Target already has up+down from addVertical; add horizontal toward commit.
  if (targetLane > commitLane) {
    connect(row[col]!, { left: true }, colorLane, 'pipe');
  } else {
    connect(row[col]!, { right: true }, colorLane, 'pipe');
  }
  addHorizontalBridge(row, commitLane, targetLane, colorLane);
}

/**
 * Place the commit/merge node on its lane.
 */
export function addNode(row: TopoCell[], commitLane: number): void {
  const col = commitLane * CELL_W;
  ensureTopoWidth(row, col + 1);
  const cell = row[col]!;
  cell.role = 'node';
  cell.colorLane = commitLane;
  // Node implies stem continuity when pipes already recorded; clear pipe dirs
  // so glyph resolution uses the node character.
  cell.up = false;
  cell.down = false;
  cell.left = false;
  cell.right = false;
}
