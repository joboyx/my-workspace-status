/**
 * Flatten a GraphModel into list rows for GraphPane (JBY-037 P3).
 */

import type { VisibleRow } from '../model/types.js';
import type { NavState } from '../nav/stack.js';
import { flashableNodeIds, removalGhosts, type ChangeSignatures, type GhostRow } from '../watch.js';
import { currentView, navDepth } from '../nav/stack.js';
import { clampIndex } from '../pageNav.js';
import type { Segment } from '../theme.js';
import { CELL_W } from './glyphs.js';
import { resolveGraphWidth } from './gutterBudget.js';
import { layoutCommits } from './layout.js';
import {
  buildStashRailContext,
  formatRelativeDate,
  graphCommitSegments,
  graphSpacerSegments,
  graphStashSegments,
  graphStashSpacerSegments,
  graphUncommittedSegments,
  type GraphRowOptions,
  type StashRailContext,
} from './rows.js';
import type { GraphCommit, GraphModel, LaidOutCommit } from './types.js';

/** Kind of a visible graph list row. */
export type GraphRowKind = 'uncommitted' | 'stash' | 'commit' | 'spacer';

/**
 * One painted row in GraphPane.
 */
export type GraphListRow = {
  id: string;
  kind: GraphRowKind;
  /** Commit hash, stash commit id, or null for uncommitted / spacer. */
  commitId: string | null;
  stashRef?: string;
  segments: Segment[];
};

/**
 * Stable row id for cursor restore.
 */
export function graphRowId(kind: GraphRowKind, key: string): string {
  if (kind === 'uncommitted') return 'graph:uncommitted';
  return `graph:${kind}:${key}`;
}

/** Spacer id under a stash (`graph:spacer:stash:stash@{n}`). */
export function graphStashSpacerId(stashRef: string): string {
  return graphRowId('spacer', `stash:${stashRef}`);
}

/** Selectable list kinds — spacers are display-only. */
export function isSelectableGraphRow(row: GraphListRow | null | undefined): boolean {
  return Boolean(row && row.kind !== 'spacer');
}

/**
 * Whether list index `index` is the focused selectable row or its paired spacer.
 *
 * Cursor bar stays on the selectable row; highlight covers the full 2-row pair.
 */
export function isGraphRowPairHighlighted(
  rows: readonly GraphListRow[],
  cursor: number,
  index: number,
): boolean {
  const focused = rows[cursor];
  if (!focused || !isSelectableGraphRow(focused)) return false;
  if (index === cursor) return true;
  const row = rows[index];
  if (!row || row.kind !== 'spacer') return false;
  if (focused.kind === 'commit' && focused.commitId) {
    return row.id === graphRowId('spacer', focused.commitId);
  }
  if (focused.kind === 'stash' && focused.stashRef) {
    return row.id === graphStashSpacerId(focused.stashRef);
  }
  return false;
}

/**
 * Selectable id that owns a graph row's flash (spacers follow their pair).
 * Pair id is list-local (`graph:commit:<sha>`); paint/compare via
 * {@link graphRowIdentity} so the same sha in another repo is a different row.
 */
export function graphRowFlashId(row: GraphListRow): string {
  if (row.kind !== 'spacer') return row.id;
  if (row.stashRef) return graphRowId('stash', row.stashRef);
  const prefix = 'graph:spacer:';
  if (row.id.startsWith(prefix)) {
    return graphRowId('commit', row.id.slice(prefix.length));
  }
  return row.id;
}

/**
 * Stable flash identity for one graph row: repo path + pair id.
 * Repo switch / a new commit window with no shared ids is a new row set.
 */
export function graphRowIdentity(row: GraphListRow, repoPath: string): string {
  return `${repoPath}#${graphRowFlashId(row)}`;
}

/**
 * True when `before` and `after` share no row identity.
 * Empty `before` is a new set (first paint). A different repo's commits
 * never overlap once keys include {@link graphRowIdentity}.
 */
export function isNewGraphRowSet(before: ChangeSignatures, after: ChangeSignatures): boolean {
  if (before.size === 0) return true;
  for (const id of before.keys()) {
    if (after.has(id)) return false;
  }
  return true;
}

/**
 * Graph flash ids for one signature transition.
 * A new row set (repo switch, first paint, no shared identities) flashes
 * nothing — those rows were not added or removed; they are a different list.
 * Same-identity appear / update / leave still flashes.
 */
export function graphRowFlashIds(
  before: ChangeSignatures,
  after: ChangeSignatures,
  opts?: { includeAdds?: boolean },
): string[] {
  if (isNewGraphRowSet(before, after)) return [];
  return flashableNodeIds(before, after, opts);
}

/**
 * Semantic signatures for selectable graph rows, looked up from `model`
 * (subject / refs / HEAD / stash parent / uncommitted dirty) — never segments.
 * Keys are {@link graphRowIdentity} (repo + pair id). Spacers are omitted;
 * paint pairs via {@link graphRowFlashId} / {@link graphRowIdentity}.
 */
export function graphRowSignatures(
  rows: readonly GraphListRow[],
  model: GraphModel,
): ChangeSignatures {
  const commits = new Map(model.commits.map((c) => [c.id, c]));
  const stashes = new Map(model.stashes.map((s) => [s.stashRef, s]));
  const signatures: ChangeSignatures = new Map();
  for (const row of rows) {
    if (row.kind === 'spacer') continue;
    const id = graphRowIdentity(row, model.repoPath);
    if (row.kind === 'commit' && row.commitId) {
      const commit = commits.get(row.commitId);
      if (!commit) continue;
      const refNames = commit.refs
        .map((r) => r.name)
        .sort()
        .join(',');
      signatures.set(id, `${commit.subject}|${refNames}|${commit.id === model.headId}`);
    } else if (row.kind === 'stash' && row.stashRef) {
      const stash = stashes.get(row.stashRef);
      if (!stash) continue;
      signatures.set(id, `${stash.subject}|${stash.parentId}`);
    } else if (row.kind === 'uncommitted') {
      signatures.set(id, `${Boolean(model.uncommitted?.hasChanges)}`);
    }
  }
  return signatures;
}

/**
 * Window identity for graph flash policy.
 * `repoPath` is the painted model's repo, never the focused tree row.
 * `commitIds` is newest-first model order — autoload appends older ids
 * without changing skip/limit; new tips prepend and are not a prefix.
 */
export type GraphFlashMeta = {
  repoPath: string;
  skip: number;
  limit: number;
  commitIds: string[];
};

/**
 * Log-window prefix of `model.commits` (excludes extra stash parents).
 */
export function graphWindowCommits(model: GraphModel): GraphCommit[] {
  const n = model.windowCount ?? model.commits.length;
  return model.commits.slice(0, Math.max(0, Math.min(n, model.commits.length)));
}

/**
 * Commits for lane layout: insert extra stash parents into the git log window
 * by author date. Never re-sort window rows (topo/date-order must stay).
 * Drop extra `%P` ids that are not in this set so they cannot plant a waiter
 * through the whole window.
 */
export function graphLayoutCommits(model: GraphModel): GraphCommit[] {
  const window = graphWindowCommits(model);
  const extras = model.commits.slice(window.length);
  if (extras.length === 0) return window;

  const extrasNewestFirst = [...extras].sort((a, b) => {
    if (a.authorDateUnix !== b.authorDateUnix) {
      return b.authorDateUnix - a.authorDateUnix;
    }
    return 0;
  });

  const merged: GraphCommit[] = [];
  let ei = 0;
  for (const w of window) {
    while (
      ei < extrasNewestFirst.length &&
      extrasNewestFirst[ei]!.authorDateUnix > w.authorDateUnix
    ) {
      merged.push(extrasNewestFirst[ei]!);
      ei += 1;
    }
    merged.push(w);
  }
  while (ei < extrasNewestFirst.length) {
    merged.push(extrasNewestFirst[ei]!);
    ei += 1;
  }

  const layoutIds = new Set(merged.map((c) => c.id));
  const extraIds = new Set(extras.map((c) => c.id));
  return merged.map((c) => {
    if (!extraIds.has(c.id)) return c;
    const parents = c.parents.filter((id) => layoutIds.has(id));
    if (parents.length === c.parents.length) return c;
    return { ...c, parents };
  });
}

/**
 * Flash window meta keyed from the painted model, never the focused repo.
 */
export function graphFlashMetaFromModel(model: GraphModel): GraphFlashMeta {
  return {
    repoPath: model.repoPath,
    skip: model.skip,
    limit: model.limit,
    commitIds: graphWindowCommits(model).map((c) => c.id),
  };
}

/**
 * True when `nextIds` is `prevIds` plus a non-empty older suffix (autoload).
 * New tips prepend ids, so they are not a prefix.
 */
function isAutoloadCommitPrefix(prevIds: readonly string[], nextIds: readonly string[]): boolean {
  if (prevIds.length === 0 || nextIds.length <= prevIds.length) return false;
  for (let i = 0; i < prevIds.length; i++) {
    if (prevIds[i] !== nextIds[i]) return false;
  }
  return true;
}

/**
 * Whether a graph list paint should flash, and whether new ids count.
 * Stale: focused repo and painted model disagree — skip signature/flash
 * updates (depth-0 j/k leaves the previous graph on screen).
 * Seed (no flash): empty signatures, repo switch, or first non-empty paint.
 * Autoload (same skip/limit, next commit ids are prev plus older suffix):
 * changes + removes only.
 * Watch / invalidate (new tips or same-window reload): adds + changes + removes.
 */
export function graphFlashDecision(opts: {
  focusedRepo: string | null;
  beforeSize: number;
  prevRowCount: number;
  nextRowCount: number;
  prev: GraphFlashMeta | null;
  next: GraphFlashMeta;
}): { stale: boolean; seed: boolean; includeAdds: boolean } {
  if (opts.focusedRepo !== opts.next.repoPath) {
    return { stale: true, seed: false, includeAdds: false };
  }
  if (
    opts.beforeSize === 0 ||
    opts.prev === null ||
    opts.prev.repoPath !== opts.next.repoPath ||
    (opts.prevRowCount === 0 && opts.nextRowCount > 0)
  ) {
    return { stale: false, seed: true, includeAdds: true };
  }
  const autoload =
    opts.next.skip === opts.prev.skip &&
    opts.next.limit === opts.prev.limit &&
    isAutoloadCommitPrefix(opts.prev.commitIds, opts.next.commitIds);
  return { stale: false, seed: false, includeAdds: !autoload };
}

function sameCommitIds(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

/**
 * Whether a graph list rebuild is a new right-pane dataset (reset cursor
 * to first selectable) vs a same-view refresh (keep nearest index).
 *
 * New dataset: first paint, repo switch, or commit-id identity change
 * that is not an autoload suffix. Same-view: width/theme re-segment,
 * autoload older commits, identical window, or stale (focused repo
 * disagrees with the painted model).
 */
export function shouldResetGraphCursor(opts: {
  stale: boolean;
  seed: boolean;
  prev: GraphFlashMeta | null;
  next: GraphFlashMeta;
}): boolean {
  if (opts.stale) return false;
  if (opts.seed) return true;
  if (opts.prev === null) return true;
  if (opts.prev.repoPath !== opts.next.repoPath) return true;
  if (sameCommitIds(opts.prev.commitIds, opts.next.commitIds)) return false;
  const autoload =
    opts.next.skip === opts.prev.skip &&
    opts.next.limit === opts.prev.limit &&
    isAutoloadCommitPrefix(opts.prev.commitIds, opts.next.commitIds);
  return !autoload;
}

/**
 * Cursor after a graph list rebuild. New dataset -> first selectable;
 * same-view refresh -> nearest to the previous index.
 */
export function graphCursorAfterRowsReload(
  rows: readonly GraphListRow[],
  prevCursor: number,
  reset: boolean,
): number {
  return reset ? firstSelectableGraphIndex(rows) : nearestSelectableGraphIndex(rows, prevCursor);
}

/**
 * Ghosts for disappeared selectable graph rows plus their paired spacers.
 * `removedIdentities` are {@link graphRowIdentity} keys (repo + pair id).
 */
export function graphRemovalGhosts(
  prevRows: readonly GraphListRow[],
  removedIdentities: readonly string[],
  now: number,
  repoPath: string,
): GhostRow<GraphListRow>[] {
  const removed = new Set(removedIdentities);
  const ids: string[] = [];
  for (const row of prevRows) {
    if (removed.has(graphRowIdentity(row, repoPath))) ids.push(row.id);
  }
  return removalGhosts(prevRows, ids, now);
}

/**
 * Indices of selectable rows (uncommitted / stash / commit).
 */
export function selectableGraphIndices(rows: readonly GraphListRow[]): number[] {
  const out: number[] = [];
  for (let i = 0; i < rows.length; i++) {
    if (isSelectableGraphRow(rows[i])) out.push(i);
  }
  return out;
}

/**
 * Snap `index` onto the nearest selectable row (prefer forward, then back).
 * Empty / all-spacer list → 0.
 */
export function nearestSelectableGraphIndex(rows: readonly GraphListRow[], index: number): number {
  if (rows.length === 0) return 0;
  const clamped = clampIndex(index, rows.length);
  if (isSelectableGraphRow(rows[clamped])) return clamped;
  for (let i = clamped + 1; i < rows.length; i++) {
    if (isSelectableGraphRow(rows[i])) return i;
  }
  for (let i = clamped - 1; i >= 0; i--) {
    if (isSelectableGraphRow(rows[i])) return i;
  }
  return 0;
}

/**
 * Map a mouse click index onto a selectable graph row.
 * Spacers prefer the paired parent above (commit/stash), then fall back to
 * {@link nearestSelectableGraphIndex}.
 */
export function selectableGraphIndexFromClick(
  rows: readonly GraphListRow[],
  index: number,
): number {
  if (rows.length === 0) return 0;
  const clamped = clampIndex(index, rows.length);
  if (isSelectableGraphRow(rows[clamped])) return clamped;
  for (let i = clamped - 1; i >= 0; i--) {
    if (isSelectableGraphRow(rows[i])) return i;
  }
  return nearestSelectableGraphIndex(rows, clamped);
}

/** First selectable index, or 0 when none. */
export function firstSelectableGraphIndex(rows: readonly GraphListRow[]): number {
  return nearestSelectableGraphIndex(rows, 0);
}

/** Last selectable index, or 0 when none. */
export function lastSelectableGraphIndex(rows: readonly GraphListRow[]): number {
  if (rows.length === 0) return 0;
  return nearestSelectableGraphIndex(rows, rows.length - 1);
}

/**
 * Move by `delta` selectable steps (±1 = j/k). Stays on selectable rows.
 */
export function stepSelectableGraphCursor(
  rows: readonly GraphListRow[],
  cursor: number,
  delta: number,
): number {
  const indices = selectableGraphIndices(rows);
  if (indices.length === 0) return 0;
  const cur = nearestSelectableGraphIndex(rows, cursor);
  const pos = indices.indexOf(cur);
  const nextPos = Math.max(0, Math.min(indices.length - 1, pos + delta));
  return indices[nextPos]!;
}

/**
 * Page through selectable rows by about `page` list rows, landing on selectable.
 */
export function applySelectableGraphPageMove(
  rows: readonly GraphListRow[],
  cursor: number,
  page: number,
  dir: 1 | -1,
): number {
  if (rows.length === 0) return 0;
  const target = clampIndex(cursor + dir * page, rows.length);
  return nearestSelectableGraphIndex(rows, target);
}

/**
 * Reuse memoized layout when commit count still matches.
 */
export function ensureLaidOut(model: GraphModel, existing?: LaidOutCommit[]): LaidOutCommit[] {
  const input = graphLayoutCommits(model);
  if (existing && existing.length === input.length) return existing;
  return layoutCommits(input);
}

type MergeItem =
  | { kind: 'stash'; date: number; ord: number; stash: GraphModel['stashes'][number] }
  | { kind: 'commit'; date: number; ord: number; laid: LaidOutCommit };

/**
 * Build ordered list rows: optional uncommitted, then commits newest-first by
 * authorDateUnix, with each stash **parked immediately above** its `stash^1`
 * (1-node leaf geometry — no chrono gap between tip and join). Orphan stashes
 * (parent object missing after load) sit after uncommitted, newest-first.
 */
export function buildGraphListRows(
  model: GraphModel,
  laidOut: LaidOutCommit[],
  opts: GraphRowOptions,
): GraphListRow[] {
  const now = opts.nowUnix ?? Math.floor(Date.now() / 1000);
  const laidById = new Map(laidOut.map((r) => [r.commit.id, r]));
  let topologyWidth = laidOut.reduce((m, r) => Math.max(m, r.cells.length), 0);
  // Stash tips need free leaf lanes beyond live stem width — reserve gutter
  // room so ◇ is not clipped onto a live spine column. Count the busiest
  // parent (or orphan pile) so sibling tips each get a column.
  if (model.stashes.length > 0) {
    let maxStemLane = laidOut.reduce((m, r) => Math.max(m, r.lane), 0);
    for (const r of laidOut) {
      for (const ref of [...r.stemUp, ...r.stemDown]) {
        if (ref.col >= 0) {
          maxStemLane = Math.max(maxStemLane, Math.floor(ref.col / CELL_W));
        }
      }
    }
    const parkedOnParent = new Map<string, number>();
    let orphanCount = 0;
    for (const stash of model.stashes) {
      if (stash.parentId.length > 0 && laidById.has(stash.parentId)) {
        parkedOnParent.set(stash.parentId, (parkedOnParent.get(stash.parentId) ?? 0) + 1);
      } else {
        orphanCount += 1;
      }
    }
    const extraLeafLanes = Math.max(1, orphanCount, ...parkedOnParent.values());
    topologyWidth = Math.max(
      topologyWidth,
      (maxStemLane + extraLeafLanes) * CELL_W + 1,
      CELL_W * extraLeafLanes + 1,
    );
  }
  const graphWidth = opts.graphWidth ?? resolveGraphWidth(topologyWidth, opts.width);
  const dateWidth =
    opts.dateWidth ??
    Math.max(1, ...laidOut.map((r) => formatRelativeDate(r.commit.authorDateUnix, now).length));
  const authorWidth =
    opts.authorWidth ??
    Math.min(
      16,
      Math.max(
        1,
        ...laidOut.map((r) => r.commit.authorName.length),
        ...model.stashes.map((s) => (s.authorName ?? '').length),
        1,
      ),
    );

  const rowOpts: GraphRowOptions = {
    ...opts,
    graphWidth,
    dateWidth,
    authorWidth,
    nowUnix: now,
    headId: opts.headId ?? model.headId,
  };

  const rows: GraphListRow[] = [];
  if (model.uncommitted) {
    rows.push({
      id: graphRowId('uncommitted', 'wt'),
      kind: 'uncommitted',
      commitId: null,
      segments: graphUncommittedSegments(model.uncommitted, rowOpts),
    });
  }

  const commitItems: Extract<MergeItem, { kind: 'commit' }>[] = laidOut.map((laid, ord) => ({
    kind: 'commit' as const,
    date: laid.commit.authorDateUnix,
    ord,
    laid,
  }));
  commitItems.sort((a, b) => {
    if (a.date !== b.date) return b.date - a.date;
    return a.ord - b.ord;
  });

  const stashesByParent = new Map<string, GraphModel['stashes'][number][]>();
  const orphanStashes: GraphModel['stashes'][number][] = [];
  for (const stash of model.stashes) {
    if (stash.parentId.length > 0 && laidById.has(stash.parentId)) {
      const list = stashesByParent.get(stash.parentId) ?? [];
      list.push(stash);
      stashesByParent.set(stash.parentId, list);
    } else {
      orphanStashes.push(stash);
    }
  }
  const byStashDateDesc = (a: GraphModel['stashes'][number], b: GraphModel['stashes'][number]) => {
    if (a.authorDateUnix !== b.authorDateUnix) {
      return b.authorDateUnix - a.authorDateUnix;
    }
    return a.stashRef.localeCompare(b.stashRef);
  };
  for (const list of stashesByParent.values()) list.sort(byStashDateDesc);
  orphanStashes.sort(byStashDateDesc);

  const items: MergeItem[] = [];
  for (const [ord, stash] of orphanStashes.entries()) {
    items.push({
      kind: 'stash',
      date: stash.authorDateUnix,
      ord,
      stash,
    });
  }
  for (const commit of commitItems) {
    const parked = stashesByParent.get(commit.laid.commit.id) ?? [];
    for (const [ord, stash] of parked.entries()) {
      items.push({
        kind: 'stash',
        date: stash.authorDateUnix,
        ord,
        stash,
      });
    }
    items.push(commit);
  }

  // Precompute stash leaf contexts + parent-row join lanes (tip above parent).
  // Sibling tips on the same parent (or stacked true orphans) reserve distinct
  // leaf lanes so ◇ does not stack on one column.
  const stashCtxByRef = new Map<string, StashRailContext>();
  const stashJoinsByParent = new Map<string, number[]>();
  const reservedByKey = new Map<string, Set<number>>();
  const maxLeafLane =
    graphWidth > 0 ? Math.max(0, Math.floor((graphWidth - 1) / CELL_W)) : undefined;
  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    if (item.kind !== 'stash') continue;
    const { stash } = item;
    let prevLaid: LaidOutCommit | null = null;
    let nextLaid: LaidOutCommit | null = null;
    for (let j = i - 1; j >= 0; j--) {
      const it = items[j]!;
      if (it.kind === 'commit') {
        prevLaid = it.laid;
        break;
      }
    }
    for (let j = i + 1; j < items.length; j++) {
      const it = items[j]!;
      if (it.kind === 'commit') {
        nextLaid = it.laid;
        break;
      }
    }
    const parentLaid = stash.parentId.length > 0 ? (laidById.get(stash.parentId) ?? null) : null;
    let tipAboveParent = false;
    if (parentLaid) {
      let parentItemIdx = -1;
      for (let j = 0; j < items.length; j++) {
        const it = items[j]!;
        if (it.kind === 'commit' && it.laid.commit.id === parentLaid.commit.id) {
          parentItemIdx = j;
          break;
        }
      }
      // Parked stashes sit above parent → join + short spur. Orphans: no join.
      tipAboveParent = parentItemIdx > i;
    }
    const reserveKey = parentLaid?.commit.id ?? '__orphan__';
    const reserved = reservedByKey.get(reserveKey) ?? new Set<number>();
    const ctx = buildStashRailContext(parentLaid, prevLaid, nextLaid, tipAboveParent, {
      reservedLanes: reserved,
      maxLane: maxLeafLane,
    });
    reserved.add(ctx.leafLane);
    reservedByKey.set(reserveKey, reserved);
    stashCtxByRef.set(stash.stashRef, ctx);
    if (parentLaid && tipAboveParent) {
      const lanes = stashJoinsByParent.get(parentLaid.commit.id) ?? [];
      if (!lanes.includes(ctx.leafLane)) lanes.push(ctx.leafLane);
      stashJoinsByParent.set(parentLaid.commit.id, lanes);
    }
  }

  for (let i = 0; i < items.length; i++) {
    const item = items[i]!;
    if (item.kind === 'stash') {
      const { stash } = item;
      const ctx =
        stashCtxByRef.get(stash.stashRef) ?? buildStashRailContext(null, null, null, false);
      rows.push({
        id: graphRowId('stash', stash.stashRef),
        kind: 'stash',
        commitId: stash.id,
        stashRef: stash.stashRef,
        segments: graphStashSegments(stash, rowOpts, ctx),
      });
      rows.push({
        id: graphStashSpacerId(stash.stashRef),
        kind: 'spacer',
        commitId: null,
        stashRef: stash.stashRef,
        segments: graphStashSpacerSegments(stash, rowOpts, ctx),
      });
    } else {
      const { laid } = item;
      // Second line under every commit: rails + HEAD/branch/tag chips.
      // Densify to the next laid-out commit even when stash chrome sits between
      // them — stash is a leaf and must not own DAG rail transitions.
      const stashJoins = stashJoinsByParent.get(laid.commit.id);
      rows.push({
        id: graphRowId('commit', laid.commit.id),
        kind: 'commit',
        commitId: laid.commit.id,
        // Densify elbows stay on commit spacers — only overlay stash join closes here.
        segments: graphCommitSegments(
          laid,
          rowOpts,
          stashJoins && stashJoins.length > 0 ? { stashJoins } : undefined,
        ),
      });
      let nextLaid: LaidOutCommit | null = null;
      for (let j = i + 1; j < items.length; j++) {
        const it = items[j]!;
        if (it.kind === 'commit') {
          nextLaid = it.laid;
          break;
        }
      }
      const railMode = nextLaid ? 'densify' : 'through';
      rows.push({
        id: graphRowId('spacer', laid.commit.id),
        kind: 'spacer',
        commitId: null,
        segments: graphSpacerSegments(rowOpts, laid, nextLaid, railMode),
      });
    }
  }
  return rows;
}

function repoFromFocused(focused: VisibleRow | undefined): string | null {
  if (!focused) return null;
  const n = focused.node;
  if (n.kind === 'checkout') return n.path;
  if (n.kind === 'repo') {
    const primary = n.children.find((c) => c.kind === 'checkout' && c.checkoutKind === 'primary');
    return primary && primary.kind === 'checkout' ? primary.path : n.path;
  }
  if (n.kind === 'dir' || n.kind === 'file') return n.repoPath;
  return null;
}

/**
 * Repo path whose graph should load, or null when none.
 */
export function activeRepoPath(nav: NavState, focused: VisibleRow | undefined): string | null {
  if (navDepth(nav) >= 1) {
    const view = currentView(nav);
    if (view.kind === 'repoGraph' || view.kind === 'commitFiles') return view.repo;
    return null;
  }
  return repoFromFocused(focused);
}

/**
 * Right (or depth-1 left) should show the graph for the active repo.
 */
export function shouldShowGraphDetail(nav: NavState, focused: VisibleRow | undefined): boolean {
  if (!activeRepoPath(nav, focused)) return false;
  if (navDepth(nav) >= 1) return true;
  const kind = focused?.node.kind;
  return kind === 'repo' || kind === 'checkout' || kind === 'dir';
}

/**
 * Depth-0 file focus keeps DiffPane.
 */
export function shouldShowFileDiff(nav: NavState, focused: VisibleRow | undefined): boolean {
  return navDepth(nav) === 0 && focused?.node.kind === 'file';
}
