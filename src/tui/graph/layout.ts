/**
 * Assign lanes and paint a lazygit-style coloured cell matrix.
 *
 * Lane assignment matches the prior walk; paint builds an internal directional
 * connection model (`topology.ts`) then resolves Unicode/ASCII junctions.
 *
 * Duplicate first-parent waiters are intentional: sibling tips that share a
 * parent keep distinct lanes until the parent row joins them via `incoming`.
 * After each join, active lanes densify left so history returns to lane 0.
 */

import { CELL_W, graphGlyphs, type GraphGlyphSet } from './glyphs.js';
import {
  addJoinCorner,
  addLinkTee,
  addNode,
  addOpenCorner,
  addVertical,
  ensureTopoWidth,
  topoStemDirs,
  topoToCells,
  type TopoCell,
} from './topology.js';
import type {
  GraphCell,
  GraphCommit,
  GraphStemRef,
  LaidOutCommit,
} from './types.js';

function blankCell(): GraphCell {
  return { ch: ' ', colorLane: null, role: 'blank' };
}

function ensureWidth(cells: GraphCell[], width: number): void {
  while (cells.length < width) cells.push(blankCell());
}

/**
 * Plan next active vector + secondary opens + links to already-live parents.
 *
 * Always keep a first-parent waiter on `commitLane` (duplicate ids OK). Sibling
 * tips that share a parent must occupy distinct lanes until the parent row
 * joins them via `incoming`.
 */
function planParents(
  active: (string | null)[],
  commitLane: number,
  parents: readonly string[],
): { next: (string | null)[]; opened: number[]; linked: number[] } {
  const next = active.slice();
  while (next.length <= commitLane) next.push(null);
  next[commitLane] = null;

  const opened: number[] = [];
  const linked: number[] = [];
  if (parents.length === 0) {
    return { next, opened, linked };
  }

  // Always keep a waiter on this lane (duplicate ids OK). Sibling tips that
  // share a parent must occupy distinct lanes until the parent row joins them
  // via `incoming`.
  next[commitLane] = parents[0]!;

  for (let p = 1; p < parents.length; p++) {
    const parent = parents[p]!;
    const existing = next.findIndex((id) => id === parent);
    if (existing !== -1) {
      if (existing !== commitLane) linked.push(existing);
      continue;
    }
    // Prefer opening to the right of the commit lane (Git Graph style).
    let pl = -1;
    for (let i = commitLane + 1; i < next.length; i++) {
      if (next[i] === null) {
        pl = i;
        break;
      }
    }
    if (pl === -1) {
      while (next.length <= commitLane) next.push(null);
      pl = next.length;
      next.push(parent);
    } else {
      next[pl] = parent;
    }
    opened.push(pl);
  }

  return { next, opened, linked };
}

function trimTrailingNulls(active: (string | null)[]): void {
  while (active.length > 0 && active[active.length - 1] === null) active.pop();
}

/**
 * Densify active lanes left — remove holes left by closes.
 */
function compactActive(active: (string | null)[]): void {
  const live = active.filter((id): id is string => id !== null);
  active.length = 0;
  active.push(...live);
}

type PaintResult = {
  cells: GraphCell[];
  stemUp: GraphStemRef[];
  stemDown: GraphStemRef[];
};

/**
 * Resolve rail identity for a non-node lane column at paint time.
 */
function laneStemId(
  lane: number,
  commitId: string,
  active: readonly (string | null)[],
  next: readonly (string | null)[],
  joinFrom: readonly number[],
  opened: readonly number[],
): string | null {
  if (joinFrom.includes(lane)) return commitId;
  if (opened.includes(lane)) return next[lane] ?? null;
  return active[lane] ?? next[lane] ?? null;
}

/**
 * Collect identity-keyed stems from topology + lane plan (before addNode).
 */
function collectStemRefs(args: {
  topo: readonly TopoCell[];
  commit: GraphCommit;
  commitLane: number;
  active: readonly (string | null)[];
  next: readonly (string | null)[];
  joinFrom: readonly number[];
  opened: readonly number[];
}): { stemUp: GraphStemRef[]; stemDown: GraphStemRef[] } {
  const { topo, commit, commitLane, active, next, joinFrom, opened } = args;
  const stemUp: GraphStemRef[] = [];
  const stemDown: GraphStemRef[] = [];
  const nodeCol = commitLane * CELL_W;
  const laneCount = Math.ceil(topo.length / CELL_W);

  for (let lane = 0; lane < laneCount; lane++) {
    const col = lane * CELL_W;
    const dirs = topoStemDirs(topo, col, commitLane);
    if (col === nodeCol) {
      if (dirs.up) {
        stemUp.push({ col, id: commit.id, colorLane: commitLane });
      }
      if (dirs.down && commit.parents[0]) {
        stemDown.push({ col, id: commit.parents[0], colorLane: commitLane });
      }
      continue;
    }
    const id = laneStemId(lane, commit.id, active, next, joinFrom, opened);
    if (!id) continue;
    const colorLane = topo[col]?.colorLane ?? lane;
    if (dirs.up) stemUp.push({ col, id, colorLane });
    if (dirs.down) stemDown.push({ col, id, colorLane });
  }

  return { stemUp, stemDown };
}

/**
 * Paint one row into a cell buffer (not yet padded to window width).
 */
function paintCells(args: {
  g: GraphGlyphSet;
  commit: GraphCommit;
  commitLane: number;
  active: readonly (string | null)[];
  next: readonly (string | null)[];
  verticalLanes: readonly number[];
  opened: readonly number[];
  linked: readonly number[];
  joinFrom: readonly number[];
  colCount: number;
}): PaintResult {
  const {
    g,
    commit,
    commitLane,
    active,
    next,
    verticalLanes,
    opened,
    linked,
    joinFrom,
    colCount,
  } = args;
  const topo: TopoCell[] = [];
  ensureTopoWidth(topo, colCount);

  for (const lane of verticalLanes) {
    if (lane === commitLane) continue;
    addVertical(topo, lane, lane);
  }

  for (const from of joinFrom) {
    if (from === commitLane) continue;
    addJoinCorner(topo, commitLane, from, commitLane);
  }

  for (const to of linked) {
    if (to === commitLane) continue;
    addLinkTee(topo, commitLane, to, to);
  }

  for (const to of opened) {
    if (to === commitLane) continue;
    addOpenCorner(topo, commitLane, to, to);
  }

  const stems = collectStemRefs({
    topo,
    commit,
    commitLane,
    active,
    next,
    joinFrom,
    opened,
  });
  addNode(topo, commitLane);

  const cells = topoToCells(topo, g);
  const nodeSpacer = commitLane * CELL_W + 1;
  if (!cells[nodeSpacer] || cells[nodeSpacer]!.role === 'blank') {
    ensureWidth(cells, nodeSpacer + 1);
    cells[nodeSpacer] = blankCell();
  }

  return { cells, ...stems };
}

/**
 * Pad every row's cells to `width` (blank on the right — nodes stay left-anchored).
 */
export function padGraphCells(rows: LaidOutCommit[], width: number): void {
  for (const row of rows) {
    ensureWidth(row.cells, width);
    if (row.cells.length > width) row.cells.length = width;
    row.edges = row.cells.map((c) => c.ch).join('');
  }
}

/**
 * Options for {@link layoutCommits}.
 */
export type LayoutCommitsOptions = {
  /** Force ASCII glyph set (tests); default follows `WS_STATUS_GLYPHS`. */
  ascii?: boolean;
};

/**
 * Assign lanes and Unicode/ASCII edge cells for a newest-first commit list.
 */
export function layoutCommits(
  commits: GraphCommit[],
  opts?: LayoutCommitsOptions,
): LaidOutCommit[] {
  const glyphs = opts?.ascii !== undefined ? graphGlyphs(opts.ascii) : graphGlyphs();
  const active: (string | null)[] = [];
  const out: LaidOutCommit[] = [];

  for (const commit of commits) {
    let lane = active.findIndex((id) => id === commit.id);
    if (lane === -1) {
      lane = active.findIndex((id) => id === null);
      if (lane === -1) {
        lane = active.length;
        active.push(commit.id);
      } else {
        active[lane] = commit.id;
      }
    }

    const incoming: number[] = [];
    for (let i = 0; i < active.length; i++) {
      if (i !== lane && active[i] === commit.id) incoming.push(i);
    }

    const { next, opened, linked } = planParents(active, lane, commit.parents);
    const joinFrom = [...incoming];
    // Duplicate waiters that join here are consumed — clear them so densify
    // collapses after the parent row (else a ghost │ remains on that lane).
    for (const i of incoming) {
      if (i !== lane && i < next.length) next[i] = null;
    }

    const verticalLanes: number[] = [];
    for (let i = 0; i < active.length; i++) {
      if (active[i] && i !== lane && !joinFrom.includes(i)) verticalLanes.push(i);
    }

    let highest = lane;
    for (let i = 0; i < next.length; i++) {
      if (next[i] !== null) highest = Math.max(highest, i);
    }
    for (const i of opened) highest = Math.max(highest, i);
    for (const i of linked) highest = Math.max(highest, i);
    for (const i of joinFrom) highest = Math.max(highest, i);
    for (let i = 0; i < active.length; i++) {
      if (active[i] !== null) highest = Math.max(highest, i);
    }
    const laneCount = Math.max(highest + 1, 1);
    const colCount = laneCount * CELL_W;

    const painted = paintCells({
      g: glyphs,
      commit,
      commitLane: lane,
      active,
      next,
      verticalLanes,
      opened,
      linked,
      joinFrom,
      colCount,
    });

    out.push({
      commit,
      lane,
      laneCount,
      cells: painted.cells,
      edges: painted.cells.map((c) => c.ch).join(''),
      stemUp: painted.stemUp,
      stemDown: painted.stemDown,
    });

    active.length = 0;
    active.push(...next);
    trimTrailingNulls(active);
    compactActive(active);
  }

  const maxWidth = out.reduce((m, r) => Math.max(m, r.cells.length), 0);
  padGraphCells(out, maxWidth);

  return out;
}
