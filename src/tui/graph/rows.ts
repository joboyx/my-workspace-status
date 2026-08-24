/**
 * Graph row segment builders — per-cell gutter colours + aligned meta columns.
 */

import { isDefaultBranch, isDetachedHeadBranch } from '../../helpers.js';
import { truncateSegments } from '../icons.js';
import type { Segment } from '../theme.js';
import { segmentsText } from '../theme.js';
import { CELL_W, graphGlyphs, type GraphGlyphSet } from './glyphs.js';
import { sliceCellsAroundLane } from './gutterBudget.js';
import { DEFAULT_LANE_COLORS } from './laneColors.js';
import {
  addHorizontalBridge,
  addJoinCorner,
  addVertical,
  connect,
  emptyTopo,
  ensureTopoWidth,
  topoToCells,
  type TopoCell,
} from './topology.js';
import type {
  GraphCell,
  GraphRef,
  GraphStemRef,
  GraphStash,
  GraphUncommitted,
  LaidOutCommit,
} from './types.js';

/**
 * Resolve the glyph set for row painting (`ascii` override or process env).
 */
function glyphsFor(opts: Pick<GraphRowOptions, 'ascii'>): GraphGlyphSet {
  return opts.ascii !== undefined ? graphGlyphs(opts.ascii) : graphGlyphs();
}

/**
 * Options for building graph row segments (P3 passes theme colours later).
 */
export type GraphRowOptions = {
  width: number;
  laneColors?: readonly string[];
  nowUnix?: number;
  mutedColor?: string;
  subjectColor?: string;
  /**
   * Stable gutter width for the loaded window (spaces when row has no cells).
   * Defaults to `row.cells.length` for commits.
   */
  graphWidth?: number;
  /** Padded relative-date column width (rails across rows). */
  dateWidth?: number;
  /** Padded author column width (rails across rows). */
  authorWidth?: number;
  /** Full SHA of HEAD — paints filled node + checkout/`[HEAD]` chips when matched. */
  headId?: string | null;
  /**
   * Checked-out branch name from repo sync chrome.
   * Detached values (`HEAD (detached)`, …) keep a standalone `[HEAD]` chip.
   * When omitted and `headId` matches, falls back to a standalone `[HEAD]` chip.
   */
  headBranch?: string | null;
  /** Local non-default branch ref colour. */
  refLocalColor?: string;
  /** Local main/master/develop (or configured override) ref colour. */
  refDefaultColor?: string;
  /** When set, only this local branch name is treated as the default ref. */
  defaultBranchOverride?: string;
  /** Remote tracking ref colour. */
  refRemoteColor?: string;
  /** Tag ref colour. */
  refTagColor?: string;
  /** Accent for checkout mark / detached `[HEAD]`. */
  headMarkColor?: string;
  /**
   * Hidden leftover branch/tag chip (`[+N]`). Distinct from muted meta so
   * overflow is obvious on the row.
   */
  overflowColor?: string;
  /** Force ASCII glyph set (tests); default follows `WS_STATUS_GLYPHS`. */
  ascii?: boolean;
};

/** Relative ages only up to 3 hours (iOS notification style). */
export const RELATIVE_DATE_LIMIT_SECS = 3 * 3600;

/** UTC `YYYY-MM-DD HH:MM` for timestamps older than {@link RELATIVE_DATE_LIMIT_SECS}. */
export function formatUtcTimestamp(unix: number): string {
  const d = new Date(Math.max(0, unix) * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${p(d.getUTCMonth() + 1)}-${p(d.getUTCDate())} ${p(d.getUTCHours())}:${p(d.getUTCMinutes())}`;
}

/**
 * Compact relative date for graph rows. Older than 3h → UTC timestamp.
 */
export function formatRelativeDate(unix: number, nowUnix: number): string {
  const delta = Math.max(0, nowUnix - unix);
  if (delta <= RELATIVE_DATE_LIMIT_SECS) {
    if (delta < 60) return 'just now';
    if (delta < 3600) return `${Math.floor(delta / 60)}m`;
    return `${Math.floor(delta / 3600)}h`;
  }
  return formatUtcTimestamp(unix);
}

function trunc(text: string, max: number): string {
  if (max <= 0) return '';
  if (text.length <= max) return text;
  if (max === 1) return '…';
  return text.slice(0, max - 1) + '…';
}

function padLeft(text: string, width: number): string {
  if (text.length >= width) return text.slice(0, width);
  return text + ' '.repeat(width - text.length);
}

/**
 * Local short name for a remote-tracking ref (`origin/main` → `main`,
 * `upstream/feature/x` → `feature/x`).
 */
export function remoteShortName(remoteRef: string): string {
  const slash = remoteRef.indexOf('/');
  return slash >= 0 ? remoteRef.slice(slash + 1) : remoteRef;
}

/**
 * Branch name used for default-branch colour checks.
 * Remotes compare the short name so `origin/<default>` matches config.
 */
export function refDefaultCompareName(ref: GraphRef): string {
  return ref.kind === 'remote' ? remoteShortName(ref.name) : ref.name;
}

/**
 * Colour for one annotated ref chip (pre-merge).
 */
export function refChipColor(ref: GraphRef, opts: GraphRowOptions): string {
  if (ref.kind === 'tag') return opts.refTagColor ?? '#e0af68';
  if (isDefaultBranch(refDefaultCompareName(ref), opts.defaultBranchOverride)) {
    return opts.refDefaultColor ?? '#7aa2f7';
  }
  if (ref.kind === 'remote') return opts.refRemoteColor ?? '#7aa2f7';
  return opts.refLocalColor ?? '#bb9af7';
}

/** Display chip after local↔remote merge (HEAD is separate). */
export type MergedRefChip =
  | { kind: 'merged'; name: string }
  | { kind: 'local'; name: string }
  | { kind: 'remote'; name: string }
  | { kind: 'tag'; name: string };

function chipSortKey(
  chip: MergedRefChip,
  opts: Pick<GraphRowOptions, 'defaultBranchOverride'>,
): number {
  const def = (name: string) => isDefaultBranch(name, opts.defaultBranchOverride);
  switch (chip.kind) {
    case 'merged':
    case 'local':
      return def(chip.name) ? 0 : 1;
    case 'remote':
      return def(remoteShortName(chip.name)) ? 0 : 2;
    case 'tag':
      return 3;
  }
}

/**
 * Merge local foo + remote star/foo into one chip; leave unmatched remotes/tags.
 *
 * Order: defaults → locals/merged → remotes → tags (stable within band).
 */
export function mergeCommitRefChips(
  refs: readonly GraphRef[],
  opts: Pick<GraphRowOptions, 'defaultBranchOverride'> = {},
): MergedRefChip[] {
  const locals = refs.filter((r) => r.kind === 'local');
  const remotes = refs.filter((r) => r.kind === 'remote');
  const tags = refs.filter((r) => r.kind === 'tag');
  const usedRemote = new Set<number>();
  const chips: MergedRefChip[] = [];

  for (const local of locals) {
    const ri = remotes.findIndex(
      (r, i) => !usedRemote.has(i) && remoteShortName(r.name) === local.name,
    );
    if (ri >= 0) {
      usedRemote.add(ri);
      chips.push({ kind: 'merged', name: local.name });
    } else {
      chips.push({ kind: 'local', name: local.name });
    }
  }
  for (let i = 0; i < remotes.length; i++) {
    if (!usedRemote.has(i)) chips.push({ kind: 'remote', name: remotes[i]!.name });
  }
  for (const tag of tags) {
    chips.push({ kind: 'tag', name: tag.name });
  }

  return chips
    .map((chip, index) => ({ chip, index, key: chipSortKey(chip, opts) }))
    .sort((a, b) => (a.key !== b.key ? a.key - b.key : a.index - b.index))
    .map((x) => x.chip);
}

/**
 * Colour the name portion of a merged/local/remote/tag chip.
 */
export function mergedRefNameColor(chip: MergedRefChip, opts: GraphRowOptions): string {
  if (chip.kind === 'tag') return opts.refTagColor ?? '#e0af68';
  const compare =
    chip.kind === 'remote' ? remoteShortName(chip.name) : chip.name;
  if (isDefaultBranch(compare, opts.defaultBranchOverride)) {
    return opts.refDefaultColor ?? '#7aa2f7';
  }
  if (chip.kind === 'remote') return opts.refRemoteColor ?? '#7aa2f7';
  return opts.refLocalColor ?? '#bb9af7';
}

/**
 * Ink segments for one ref chip.
 *
 * Mark order inside brackets: checkout mark (optional) → sync mark (merged) →
 * name. Unicode mode uses Nerd Font PUA icons (1 terminal cell in MesloLGM);
 * ASCII mode uses `+` / `=`.
 */
export function mergedRefChipSegments(
  chip: MergedRefChip,
  opts: GraphRowOptions,
  isCheckout = false,
): Segment[] {
  const nameColor = mergedRefNameColor(chip, opts);
  const g = glyphsFor(opts);
  const headMark = opts.headMarkColor ?? '#e0af68';
  const remoteColor = opts.refRemoteColor ?? '#7aa2f7';
  const showCheckout =
    isCheckout && (chip.kind === 'merged' || chip.kind === 'local');
  const synced = chip.kind === 'merged';

  if (chip.kind === 'tag' || chip.kind === 'remote') {
    return [{ text: `[${chip.name}]`, color: nameColor }];
  }

  const segs: Segment[] = [{ text: '[', color: nameColor }];
  if (showCheckout) {
    segs.push({ text: g.checkoutMark, color: headMark, bold: true });
  }
  if (synced) {
    segs.push({ text: g.syncMark, color: remoteColor });
  }
  segs.push({ text: chip.name, color: nameColor });
  segs.push({ text: ']', color: nameColor });
  return segs;
}

/**
 * Collapse adjacent same-colour cells into fewer Ink segments.
 */
export function cellsToSegments(
  cells: readonly GraphCell[],
  laneColors: readonly string[],
  fallbackColor: string,
): Segment[] {
  const segs: Segment[] = [];
  for (const cell of cells) {
    const color =
      cell.colorLane === null
        ? fallbackColor
        : (laneColors[cell.colorLane % laneColors.length] ?? fallbackColor);
    const last = segs[segs.length - 1];
    if (last && last.color === color && !last.bold && !last.backgroundColor && !last.dim) {
      last.text += cell.ch;
    } else {
      segs.push({ text: cell.ch, color });
    }
  }
  return segs;
}

function blankGutter(width: number): GraphCell[] {
  return Array.from({ length: width }, () => ({
    ch: ' ',
    colorLane: null,
    role: 'blank' as const,
  }));
}

/**
 * Swap commit node → HEAD glyph (`⊙` / `@`), including merge-at-HEAD.
 */
export function applyHeadNodeGlyph(
  cells: readonly GraphCell[],
  isHead: boolean,
  ascii?: boolean,
): GraphCell[] {
  if (!isHead) return [...cells];
  const headCh =
    ascii !== undefined ? graphGlyphs(ascii).headCommit : graphGlyphs().headCommit;
  return cells.map((c) => (c.role === 'node' ? { ...c, ch: headCh } : c));
}

/**
 * True when HEAD is detached (or branch unknown) so a standalone `[HEAD]` chip is needed.
 */
function showDetachedHeadChip(opts: GraphRowOptions, isHead: boolean): boolean {
  if (!isHead) return false;
  if (opts.headBranch == null || opts.headBranch === '') return true;
  return isDetachedHeadBranch(opts.headBranch);
}

/**
 * Whether this merged/local chip is the checked-out branch tip.
 */
function chipIsCheckout(
  chip: MergedRefChip,
  opts: GraphRowOptions,
  isHead: boolean,
): boolean {
  if (!isHead) return false;
  if (opts.headBranch == null || opts.headBranch === '') return false;
  if (isDetachedHeadBranch(opts.headBranch)) return false;
  if (chip.kind !== 'local' && chip.kind !== 'merged') return false;
  return chip.name === opts.headBranch;
}

/**
 * Optional `[HEAD]` (detached only) + merged ref chips.
 *
 * Matching local + remote tips merge into sync-prefixed chips; checkout gets
 * the Nerd Font crosshairs mark when HEAD is on that branch.
 * Shared by commit spacers and the GraphPane selection footer.
 */
/**
 * Whole chips as groups so a narrow spacer can hide leftover chips as `[+N]`
 * instead of mid-chip ellipsis.
 */
export function graphRefChipGroups(
  refs: readonly GraphRef[],
  opts: GraphRowOptions,
  isHead: boolean,
): Segment[][] {
  const headMark = opts.headMarkColor ?? '#e0af68';
  const groups: Segment[][] = [];
  if (showDetachedHeadChip(opts, isHead)) {
    groups.push([{ text: '[HEAD]', color: headMark, bold: true }]);
  }
  for (const chip of mergeCommitRefChips(refs, opts)) {
    groups.push(mergedRefChipSegments(chip, opts, chipIsCheckout(chip, opts, isHead)));
  }
  return groups;
}

function segsWidth(segs: readonly Segment[]): number {
  return segs.reduce((n, s) => n + s.text.length, 0);
}

/**
 * Chip text for leftover hidden branch/tag refs (`[+N]`).
 */
export function overflowChipText(hidden: number): string {
  return `[+${hidden}]`;
}

function overflowToken(hidden: number, budget: number): string | null {
  const chip = overflowChipText(hidden);
  if (chip.length <= budget) return chip;
  const bare = `+${hidden}`;
  if (bare.length <= budget) return bare;
  return null;
}

/**
 * Keep whole chip groups that fit; leftover branch/tag count becomes `[+N]`.
 */
export function fitChipGroups(
  groups: readonly Segment[][],
  budget: number,
  overflowColor: string,
  gapColor: string = overflowColor,
): Segment[] {
  if (groups.length === 0 || budget <= 0) return [];
  const n = groups.length;
  for (let k = n; k >= 0; k--) {
    const hidden = n - k;
    let width = 0;
    for (let i = 0; i < k; i++) {
      if (i > 0) width += 1;
      width += segsWidth(groups[i]!);
    }
    const ov =
      hidden > 0 ? overflowChipText(hidden).length + (k > 0 ? 1 : 0) : 0;
    if (width + ov <= budget) {
      const out: Segment[] = [];
      for (let i = 0; i < k; i++) {
        if (i > 0) out.push({ text: ' ', color: gapColor });
        out.push(...groups[i]!);
      }
      if (hidden > 0) {
        if (k > 0) out.push({ text: ' ', color: gapColor });
        out.push({ text: overflowChipText(hidden), color: overflowColor });
      }
      return out;
    }
  }
  const token = overflowToken(n, budget);
  if (!token) return [];
  return [{ text: token, color: overflowColor }];
}

export function graphRefChipSegments(
  refs: readonly GraphRef[],
  opts: GraphRowOptions,
  isHead: boolean,
): Segment[] {
  const gapColor = opts.subjectColor ?? '#c0caf5';
  const groups = graphRefChipGroups(refs, opts, isHead);
  const segs: Segment[] = [];
  for (const g of groups) {
    if (segs.length > 0) segs.push({ text: ' ', color: gapColor });
    segs.push(...g);
  }
  return segs;
}

/**
 * Optional `[HEAD]` (detached only) + merged ref chips for the spacer under a commit.
 */
export function commitRefChipSegments(
  row: LaidOutCommit,
  opts: GraphRowOptions,
  isHead: boolean,
): Segment[] {
  return graphRefChipSegments(row.commit.refs, opts, isHead);
}

/**
 * Subject-only label for the commit row (refs live on the spacer beneath).
 */
export function commitLabelSegments(
  row: LaidOutCommit,
  opts: GraphRowOptions,
  _isHead: boolean,
): { prefix: Segment[]; subject: Segment } {
  const subjectColor = opts.subjectColor ?? '#c0caf5';
  return {
    prefix: [],
    subject: { text: row.commit.subject, color: subjectColor },
  };
}

/** Minimum subject columns to keep when refs also want space. */
const MIN_SUBJECT_KEEP = 12;

/**
 * Fit HEAD/refs + subject into `budget`, preferring a readable subject over refs.
 */
export function fitLabelSegments(
  prefix: Segment[],
  subject: Segment,
  budget: number,
  padColor: string,
): Segment[] {
  if (budget <= 0) return [];

  const prefixText = segmentsText(prefix);
  const full =
    prefix.length > 0 ? `${prefixText} ${subject.text}` : subject.text;
  if (full.length <= budget) {
    const segs =
      prefix.length > 0
        ? [...prefix, { text: ' ', color: padColor }, { ...subject }]
        : [{ ...subject }];
    if (full.length < budget) {
      segs.push({ text: ' '.repeat(budget - full.length), color: padColor });
    }
    return segs;
  }

  // Subject-first: when tight, drop/truncate refs before cutting the subject hard.
  let keepSubject: number;
  if (budget <= MIN_SUBJECT_KEEP || prefix.length === 0) {
    keepSubject = Math.min(subject.text.length, budget);
  } else {
    keepSubject = Math.min(subject.text.length, Math.max(MIN_SUBJECT_KEEP, budget - prefixText.length - 1));
  }
  const gap = prefix.length > 0 && keepSubject > 0 ? 1 : 0;
  const prefixBudget = Math.max(0, budget - keepSubject - gap);

  const out: Segment[] = [];
  if (prefixBudget > 0 && prefix.length > 0) {
    // Preserve chip colours when cutting — never flatten to padColor.
    out.push(...truncateSegments(prefix, prefixBudget));
    if (keepSubject > 0) out.push({ text: ' ', color: padColor });
  }
  if (keepSubject > 0) {
    out.push({ ...subject, text: trunc(subject.text, keepSubject) });
  }
  const len = segmentsText(out).length;
  if (len < budget) out.push({ text: ' '.repeat(budget - len), color: padColor });
  return out;
}

/**
 * Overlay densify elbows onto a commit row when the previous list neighbour
 * remapped a live rail (`stemDown` → `stemUp` column change).
 *
 * Same-column matches are a no-op (the row already has the through-rail).
 * Never overwrites `role === 'node'`; fills blanks from the transition paint.
 * Does not change layout stem metadata — list-layer only.
 */
export function applyCommitDensifyCells(
  prev: LaidOutCommit,
  next: LaidOutCommit,
  ascii?: boolean,
): GraphCell[] {
  const remaps = matchStemRefs(prev.stemDown, next.stemUp).filter(
    ({ from, to }) => from.col !== to.col && from.col >= 0 && to.col >= 0,
  );
  if (remaps.length === 0) return next.cells.map((c) => ({ ...c }));

  const g = ascii !== undefined ? graphGlyphs(ascii) : graphGlyphs();
  let width = next.cells.length;
  for (const { from, to } of remaps) {
    width = Math.max(width, from.col + 1, to.col + 1);
  }

  const out: GraphCell[] = next.cells.map((c) => ({ ...c }));
  while (out.length < width) {
    out.push({ ch: ' ', colorLane: null, role: 'blank' });
  }

  const topo: TopoCell[] = [];
  ensureTopoWidth(topo, width);
  for (let i = 0; i < width; i++) topo[i] = emptyTopo();

  for (const { from, to } of remaps) {
    paintStemTransition(topo, from.col, to.col, from.colorLane);
  }

  const overlay = topoToCells(topo, g);
  for (let i = 0; i < width; i++) {
    const over = overlay[i];
    if (!over || over.role === 'blank') continue;
    const base = out[i]!;
    if (base.role === 'node') continue;
    if (base.role === 'blank') {
      out[i] = { ...over };
      continue;
    }
    // Existing pipe + overlay pipe: prefer cross when a vertical meets a
    // horizontal densify bridge; otherwise keep the densify glyph.
    if (
      (base.ch === g.vertical || base.ch === g.cross) &&
      (over.ch === g.horizontal ||
        over.ch === g.teeUp ||
        over.ch === g.teeDown ||
        over.ch === g.cross)
    ) {
      out[i] = {
        ch: g.cross,
        colorLane: base.colorLane ?? over.colorLane,
        role: 'pipe',
      };
    } else if (base.ch === g.vertical && over.role === 'pipe') {
      // Elbows / tees that land on a blank spacer are already handled; if an
      // overlay elbow somehow hits a through-rail, take the overlay glyph.
      out[i] = { ...over };
    }
  }
  return out;
}

/** Prefer keeping at least this many left columns before dropping right meta. */
const MIN_LEFT_KEEP = 12;

type MetaCols = { hash: boolean; date: boolean; author: boolean };

/**
 * Right-anchored ` hash date author` string; drop order hash → date → author.
 * Empty author omits the author column even when `cols.author` is true.
 */
export function metaColumnsText(
  hash: string,
  date: string,
  author: string,
  cols: MetaCols,
): string {
  let meta = '';
  if (cols.hash) meta += ' ' + hash;
  if (cols.date) meta += ' ' + date;
  if (cols.author && author) meta += ' ' + author;
  return meta;
}

/**
 * Choose which right-meta columns fit beside a left flex region.
 *
 * Prefers keeping `minLeftKeep` columns for the left side (subject/refs).
 */
export function pickMetaColumns(
  availableAfterGutter: number,
  hash: string,
  date: string,
  author: string,
  minLeftKeep: number = MIN_LEFT_KEEP,
): MetaCols {
  const candidates: MetaCols[] = [
    { hash: true, date: true, author: true },
    { hash: false, date: true, author: true },
    { hash: false, date: false, author: true },
    { hash: false, date: false, author: false },
  ];
  for (const cols of candidates) {
    const meta = metaColumnsText(hash, date, author, cols);
    const room = availableAfterGutter - meta.length;
    if (room >= minLeftKeep || (!cols.hash && !cols.date && !cols.author)) {
      return cols;
    }
  }
  return { hash: false, date: false, author: false };
}

/**
 * Ink segments for one laid-out commit row.
 *
 * Layout A row 1: `[gutter][ ][subject flex]`
 * Hash / date / author and ref chips live on the spacer beneath.
 * Optional `stashJoins` overlays 3b-style close elbows for stash leaf tips
 * whose `stash^1` is this commit.
 */
export function graphCommitSegments(
  row: LaidOutCommit,
  opts: GraphRowOptions,
  neighbors?: { prev?: LaidOutCommit | null; stashJoins?: readonly number[] },
): Segment[] {
  const laneColors = opts.laneColors ?? DEFAULT_LANE_COLORS;
  const muted = opts.mutedColor ?? '#565f89';
  const subjectColor = opts.subjectColor ?? '#c0caf5';
  const gutterWidth = opts.graphWidth ?? row.cells.length;

  const isHead = Boolean(opts.headId && row.commit.id === opts.headId);

  // Densify overlay on full topology before windowing (same order as stash).
  let topoCells =
    neighbors?.prev != null
      ? applyCommitDensifyCells(neighbors.prev, row, opts.ascii)
      : row.cells.map((c) => ({ ...c }));

  const stashJoins = neighbors?.stashJoins;
  if (stashJoins && stashJoins.length > 0) {
    topoCells = applyStashJoinCells(
      { ...row, cells: topoCells },
      stashJoins,
      opts.ascii,
    );
  }

  // Cap may be below topology width — keep the commit node in the window,
  // and stash join columns when they fit (avoid clipping ●─╯ onto a live rail).
  const joinCols = (stashJoins ?? []).map((l) => l * CELL_W);
  const windowed =
    topoCells.length > gutterWidth
      ? sliceCellsAroundLane(topoCells, gutterWidth, row.lane, joinCols)
      : topoCells;
  const baseCells =
    windowed.length >= gutterWidth
      ? windowed.slice(0, gutterWidth)
      : [...windowed, ...blankGutter(gutterWidth - windowed.length)];
  const cells = applyHeadNodeGlyph(baseCells, isHead, opts.ascii);

  const { prefix, subject } = commitLabelSegments(row, opts, isHead);
  const gutterSegs = cellsToSegments(cells, laneColors, muted);
  const subjectBudget = Math.max(1, opts.width - gutterWidth - 1);
  return [
    ...gutterSegs,
    { text: ' ', color: muted },
    ...fitLabelSegments(prefix, subject, subjectBudget, subjectColor),
  ];
}

/**
 * Pair previous stem-down refs to next stem-up refs by rail identity.
 *
 * Prefer same-column matches when duplicate ids exist (sibling waiters).
 */
export function matchStemRefs(
  down: readonly GraphStemRef[],
  up: readonly GraphStemRef[],
): Array<{ from: GraphStemRef; to: GraphStemRef }> {
  const remaining = up.map((ref, index) => ({ ref, index }));
  const pairs: Array<{ from: GraphStemRef; to: GraphStemRef }> = [];
  const used = new Set<number>();

  for (const from of down) {
    let hit = remaining.find(
      (c) => !used.has(c.index) && c.ref.id === from.id && c.ref.col === from.col,
    );
    if (!hit) {
      hit = remaining.find((c) => !used.has(c.index) && c.ref.id === from.id);
    }
    if (!hit) continue;
    used.add(hit.index);
    pairs.push({ from, to: hit.ref });
  }
  return pairs;
}

/**
 * Paint one densify remapped rail on a commit spacer.
 *
 * Same column → vertical. Different columns → elbows + horizontal so the rail
 * stays visually connected across the spacer gap.
 */
function paintStemTransition(
  topo: TopoCell[],
  fromCol: number,
  toCol: number,
  colorLane: number,
): void {
  const fromLane = Math.floor(fromCol / CELL_W);
  const toLane = Math.floor(toCol / CELL_W);
  ensureTopoWidth(topo, Math.max(fromCol, toCol) + 1);

  if (fromCol === toCol) {
    addVertical(topo, fromLane, colorLane);
    return;
  }

  // Arrive from above at fromCol, depart downward at toCol.
  if (fromCol > toCol) {
    connect(topo[fromCol]!, { up: true, left: true }, colorLane, 'pipe');
    connect(topo[toCol]!, { down: true, right: true }, colorLane, 'pipe');
  } else {
    connect(topo[fromCol]!, { up: true, right: true }, colorLane, 'pipe');
    connect(topo[toCol]!, { down: true, left: true }, colorLane, 'pipe');
  }
  addHorizontalBridge(topo, fromLane, toLane, colorLane);
}

/**
 * Anchor lane for densify gutter windowing — prefer the newer commit
 * (prev in newest-first order), matching that row's paint focus.
 */
export function stashRailAnchorLane(
  prev: LaidOutCommit | null,
  next: LaidOutCommit | null,
): number {
  return prev?.lane ?? next?.lane ?? 0;
}

/**
 * Build a densify gutter that preserves live rails between two commits.
 *
 * Historical name (`stashRail*`) — used for **commit↔commit** spacer densify
 * only. Stash leaf tips use `stashLeafRailCells` on `stash^1`'s lane.
 * Matches neighbouring laid-out commits by stem **identity** (waiter commit
 * id), not absolute column. Missing either neighbour → blank gutter
 * (tip through-rails use `stemDownRailCells` directly).
 *
 * Paint uses full topology coordinates, then the same `sliceCellsAroundLane`
 * window as commit rows when `displayWidth` is below topology width.
 */
export function stashRailCells(
  displayWidth: number,
  prev: LaidOutCommit | null,
  next: LaidOutCommit | null,
  ascii?: boolean,
): GraphCell[] {
  const budget = Math.max(0, Math.floor(displayWidth));
  if (budget <= 0) return [];
  if (!prev || !next) return blankGutter(budget);

  const anchorLane = stashRailAnchorLane(prev, next);
  const g = ascii !== undefined ? graphGlyphs(ascii) : graphGlyphs();
  let topologyWidth = Math.max(prev.cells.length, next.cells.length, budget);
  for (const ref of prev.stemDown) {
    if (ref.col >= 0) topologyWidth = Math.max(topologyWidth, ref.col + 1);
  }
  for (const ref of next.stemUp) {
    if (ref.col >= 0) topologyWidth = Math.max(topologyWidth, ref.col + 1);
  }

  const topo: TopoCell[] = [];
  ensureTopoWidth(topo, topologyWidth);
  for (let i = 0; i < topologyWidth; i++) topo[i] = emptyTopo();

  for (const { from, to } of matchStemRefs(prev.stemDown, next.stemUp)) {
    if (from.col < 0 || to.col < 0) continue;
    paintStemTransition(topo, from.col, to.col, from.colorLane);
  }

  let cells = topoToCells(topo, g);
  while (cells.length < topologyWidth) {
    cells.push({ ch: ' ', colorLane: null, role: 'blank' });
  }

  if (cells.length > budget) {
    cells = sliceCellsAroundLane(cells, budget, anchorLane);
  } else if (cells.length < budget) {
    cells = [...cells, ...blankGutter(budget - cells.length)];
  }
  return cells;
}

/**
 * Through-rails only from `prev.stemDown` (no densify to a next commit).
 *
 * Used for the spacer under a tip commit (no following commit).
 */
export function stemDownRailCells(
  displayWidth: number,
  prev: LaidOutCommit,
  ascii?: boolean,
): GraphCell[] {
  const g = ascii !== undefined ? graphGlyphs(ascii) : graphGlyphs();
  const budget = Math.max(0, Math.floor(displayWidth));
  if (budget <= 0) return [];

  let topologyWidth = Math.max(prev.cells.length, budget);
  for (const ref of prev.stemDown) {
    if (ref.col >= 0) topologyWidth = Math.max(topologyWidth, ref.col + 1);
  }

  const topo: TopoCell[] = [];
  ensureTopoWidth(topo, topologyWidth);
  for (let i = 0; i < topologyWidth; i++) topo[i] = emptyTopo();

  for (const ref of prev.stemDown) {
    if (ref.col < 0) continue;
    paintStemTransition(topo, ref.col, ref.col, ref.colorLane);
  }

  let cells = topoToCells(topo, g);
  while (cells.length < topologyWidth) {
    cells.push({ ch: ' ', colorLane: null, role: 'blank' });
  }

  const anchorLane = prev.lane;
  if (cells.length > budget) {
    cells = sliceCellsAroundLane(cells, budget, anchorLane);
  } else if (cells.length < budget) {
    cells = [...cells, ...blankGutter(budget - cells.length)];
  }
  return cells;
}

/**
 * Non-selectable row under a commit: stem rails + ref chips left + meta right.
 *
 * Layout A row 2: `[gutter][ ][refs…][pad][hash][ ][date][ ][author]`
 * Drop order when narrow: hash → date → author (prefer keeping refs).
 * When `next` is set and `railMode` is `densify`, gutters match adjacent-commit
 * stem remaps; otherwise through-rails only (`stemDown`).
 */
export function graphSpacerSegments(
  opts: GraphRowOptions,
  prev: LaidOutCommit,
  next: LaidOutCommit | null,
  railMode: 'densify' | 'through' = next ? 'densify' : 'through',
): Segment[] {
  const muted = opts.mutedColor ?? '#565f89';
  const now = opts.nowUnix ?? Math.floor(Date.now() / 1000);
  const gutterWidth =
    opts.graphWidth ??
    Math.max(prev.cells.length, next?.cells.length ?? 0);
  const laneColors = opts.laneColors ?? DEFAULT_LANE_COLORS;
  const isHead = Boolean(opts.headId && prev.commit.id === opts.headId);
  const segs: Segment[] = [];
  if (gutterWidth > 0) {
    const railCells =
      railMode === 'densify' && next
        ? stashRailCells(gutterWidth, prev, next, opts.ascii)
        : stemDownRailCells(gutterWidth, prev, opts.ascii);
    segs.push(...cellsToSegments(railCells, laneColors, muted));
    segs.push({ text: ' ', color: muted });
  }

  const hash = prev.commit.id.slice(0, 7);
  const dateRaw = formatRelativeDate(prev.commit.authorDateUnix, now);
  const dateWidth = opts.dateWidth ?? Math.max(dateRaw.length, 4);
  const date = padLeft(trunc(dateRaw, dateWidth), dateWidth);
  const authorRaw = prev.commit.authorName;
  const authorWidth = opts.authorWidth ?? Math.min(16, Math.max(authorRaw.length, 1));
  const author = padLeft(trunc(authorRaw, authorWidth), authorWidth);

  const used = segmentsText(segs).length;
  const available = Math.max(0, opts.width - used);
  const cols = pickMetaColumns(available, hash, date, author);
  const meta = metaColumnsText(hash, date, author, cols);
  const refBudget = Math.max(0, available - meta.length);

  const refGroups = graphRefChipGroups(prev.commit.refs, opts, isHead);
  if (refGroups.length > 0 && refBudget > 0) {
    const overflow =
      opts.overflowColor ?? opts.headMarkColor ?? opts.subjectColor ?? muted;
    segs.push(...fitChipGroups(refGroups, refBudget, overflow, muted));
  }
  if (meta.length > 0) {
    const leftLen = segmentsText(segs).length;
    const pad = Math.max(0, opts.width - leftLen - meta.length);
    if (pad > 0) segs.push({ text: ' '.repeat(pad), color: muted });
    segs.push({ text: meta, color: muted });
  } else {
    const len = segmentsText(segs).length;
    if (len < opts.width) {
      segs.push({ text: ' '.repeat(opts.width - len), color: muted });
    }
  }
  return segs;
}

/**
 * Paint context for a stash as a 1-node side leaf tip (commit 3b → `◇`).
 *
 * Live rails pass through the chrono gap; `leafLane` is a free lane (never a
 * live DAG column). Join close elbows land on `stash^1`, not on the tip row.
 */
export type StashRailContext = {
  /** Laid-out `stash^1`, or null when outside the loaded window. */
  parent: LaidOutCommit | null;
  /** Live commit-DAG stems at this chrono gap (prefer `next.stemUp`). */
  liveRails: readonly GraphStemRef[];
  /** Free leaf lane for `◇` — not in the live set / not on `parent.lane`. */
  leafLane: number;
  /**
   * True when the stash tip sits above `stash^1` in list order so the join
   * elbow closes. Gates spacer spur rail the same way list gates the elbow —
   * inverted chrono (tip below parent) may still show ◇ but must not continue a
   * dangling side rail under the tip.
   */
  tipAboveParent: boolean;
  /**
   * Leaf lanes of newer sibling tips already parked above this row on the
   * same parent. Painted as through-rails so those spurs reach the join.
   */
  siblingSpurLanes: readonly number[];
};

/**
 * Live stem refs at a stash chrono gap.
 *
 * Prefer `next.stemUp` (post-densify arrival columns); fall back to
 * `prev.stemDown` at the window tail.
 */
export function stashLiveRailsAtGap(
  prev: LaidOutCommit | null,
  next: LaidOutCommit | null,
): GraphStemRef[] {
  const refs = next?.stemUp ?? prev?.stemDown ?? [];
  const seen = new Set<number>();
  const out: GraphStemRef[] = [];
  for (const ref of refs) {
    if (ref.col < 0 || seen.has(ref.col)) continue;
    seen.add(ref.col);
    out.push(ref);
  }
  return out;
}

/**
 * Lane indices occupied by live stem columns.
 */
export function liveLanesFromRails(
  liveRails: readonly GraphStemRef[],
): Set<number> {
  const lanes = new Set<number>();
  for (const ref of liveRails) {
    if (ref.col >= 0) lanes.add(Math.floor(ref.col / CELL_W));
  }
  return lanes;
}

/** Options for {@link allocateStashLeafLane} / {@link buildStashRailContext}. */
export type StashLeafAlloc = {
  /** Lanes already taken by sibling stash tips on this parent / orphan pile. */
  reservedLanes?: ReadonlySet<number>;
  /**
   * Inclusive max lane that still fits the painted gutter. Allocator prefers
   * in-budget free lanes so ◇ / join elbows are not clipped off the right.
   */
  maxLane?: number;
};

/**
 * Allocate a free leaf lane for a stash tip.
 *
 * Lowest free lane that is not live, not the parent, and not reserved by a
 * sibling tip. When `maxLane` is set, search that range first so the diamond
 * stays inside the clipped gutter. Does not steal a live DAG column.
 */
export function allocateStashLeafLane(
  liveLanes: ReadonlySet<number>,
  parentLane: number | null,
  opts?: StashLeafAlloc,
): number {
  const blocked = new Set(liveLanes);
  if (parentLane != null) blocked.add(parentLane);
  if (opts?.reservedLanes) {
    for (const lane of opts.reservedLanes) blocked.add(lane);
  }
  if (opts?.maxLane != null) {
    for (let lane = 0; lane <= opts.maxLane; lane++) {
      if (!blocked.has(lane)) return lane;
    }
  }
  for (let lane = 0; ; lane++) {
    if (!blocked.has(lane)) return lane;
  }
}

/**
 * Build {@link StashRailContext} for one stash at its chrono gap.
 *
 * `tipAboveParent` defaults to `parent != null` (unit helpers that pass a bare
 * parent assume immediate join). List paint must pass the chrono-order flag so
 * inverted tips do not get a spacer spur without a close elbow.
 */
export function buildStashRailContext(
  parent: LaidOutCommit | null,
  prev: LaidOutCommit | null,
  next: LaidOutCommit | null,
  tipAboveParent: boolean = parent != null,
  alloc?: StashLeafAlloc,
): StashRailContext {
  const liveRails = stashLiveRailsAtGap(prev, next);
  const liveLanes = liveLanesFromRails(liveRails);
  // Never park ◇ on stash^1 even if that lane is somehow absent from stems.
  if (parent) liveLanes.add(parent.lane);
  const leafLane = allocateStashLeafLane(liveLanes, parent?.lane ?? null, alloc);
  const siblingSpurLanes =
    parent != null && tipAboveParent
      ? [...(alloc?.reservedLanes ?? [])].filter((lane) => lane !== leafLane)
      : [];
  return {
    parent,
    liveRails,
    leafLane,
    tipAboveParent: parent != null && tipAboveParent,
    siblingSpurLanes,
  };
}

/**
 * Resolve paint context from a full {@link StashRailContext}, a bare parent, or null.
 *
 * Bare parent → treat as immediate join (`next = parent`) so unit helpers still
 * get through-rails + free leaf without list wiring.
 */
export function resolveStashRailContext(
  parentOrCtx: LaidOutCommit | StashRailContext | null,
): StashRailContext {
  if (parentOrCtx == null) {
    return buildStashRailContext(null, null, null);
  }
  if (
    'leafLane' in parentOrCtx &&
    'liveRails' in parentOrCtx &&
    'parent' in parentOrCtx
  ) {
    return parentOrCtx;
  }
  const parent = parentOrCtx as LaidOutCommit;
  return buildStashRailContext(parent, null, parent);
}

/**
 * Overlay close-elbow joins for stash leaf tips onto a `stash^1` commit row.
 *
 * Same family as a 1-commit side-branch close (`●─╯` / `⊙─╯`). Does not
 * overwrite the commit node. List-layer only — densify stays on commit spacers.
 */
export function applyStashJoinCells(
  parent: LaidOutCommit,
  leafLanes: readonly number[],
  ascii?: boolean,
): GraphCell[] {
  const unique = [...new Set(leafLanes.filter((l) => l !== parent.lane))];
  if (unique.length === 0) return parent.cells.map((c) => ({ ...c }));

  const g = ascii !== undefined ? graphGlyphs(ascii) : graphGlyphs();
  let width = parent.cells.length;
  for (const lane of unique) {
    width = Math.max(width, lane * CELL_W + 1, parent.lane * CELL_W + 1);
  }

  const out: GraphCell[] = parent.cells.map((c) => ({ ...c }));
  while (out.length < width) {
    out.push({ ch: ' ', colorLane: null, role: 'blank' });
  }

  const topo: TopoCell[] = [];
  ensureTopoWidth(topo, width);
  for (let i = 0; i < width; i++) topo[i] = emptyTopo();

  for (const leafLane of unique) {
    addJoinCorner(topo, parent.lane, leafLane, parent.lane);
  }

  const overlay = topoToCells(topo, g);
  for (let i = 0; i < width; i++) {
    const over = overlay[i];
    if (!over || over.role === 'blank') continue;
    out[i] = mergeJoinOverlayCell(out[i]!, over, g);
  }
  return out;
}

/**
 * Compose a stash-join overlay onto an existing parent-row cell.
 *
 * Through-rails stay: a horizontal/elbow landing on a live vertical becomes a
   * cross instead of replacing the rail (unrelated through-rails must stay visible).
 */
function mergeJoinOverlayCell(
  base: GraphCell,
  over: GraphCell,
  g: GraphGlyphSet,
): GraphCell {
  if (base.role === 'node') return base;
  if (over.role === 'blank') return base;
  if (base.role === 'blank') return { ...over };

  const baseVertical =
    base.ch === g.vertical ||
    base.ch === g.cross ||
    base.ch === g.teeLeft ||
    base.ch === g.teeRight ||
    base.ch === g.teeUp ||
    base.ch === g.teeDown;
  const overHorizontal =
    over.ch === g.horizontal ||
    over.ch === g.teeUp ||
    over.ch === g.teeDown ||
    over.ch === g.cross ||
    over.ch === g.cornerUpRight ||
    over.ch === g.cornerUpLeft ||
    over.ch === g.cornerDownRight ||
    over.ch === g.cornerDownLeft;
  if (baseVertical && overHorizontal) {
    return {
      ch: g.cross,
      colorLane: base.colorLane ?? over.colorLane,
      role: 'pipe',
    };
  }
  if (base.role === 'pipe' && over.role === 'pipe') {
    return { ...over, colorLane: over.colorLane ?? base.colorLane };
  }
  return base;
}

/**
 * Leaf-tip gutter for a stash: 1-node side tip on a free lane (3b → `◇`).
 *
 * Tip row: through-rails for live stems + `◇` on `leafLane` (no mid-rail
 * `├─◇` tee, no `down` on the tip column). Spacer: live rails + optional short
 * spur rail toward the `stash^1` join (never a second node). Does not densify
 * commit↔commit rails.
 */
export function stashLeafRailCells(
  displayWidth: number,
  parentOrCtx: LaidOutCommit | StashRailContext | null,
  opts: { ascii?: boolean; node: boolean; spurRail?: boolean },
): GraphCell[] {
  const budget = Math.max(0, Math.floor(displayWidth));
  if (budget <= 0) return [];
  const g = glyphsFor(opts);
  const ctx = resolveStashRailContext(parentOrCtx);
  const leafLane = ctx.leafLane;
  const tipColor = ctx.parent?.lane ?? leafLane;
  let topologyWidth = Math.max(budget, leafLane * CELL_W + 1);
  for (const ref of ctx.liveRails) {
    if (ref.col >= 0) topologyWidth = Math.max(topologyWidth, ref.col + 1);
  }
  if (ctx.parent) {
    topologyWidth = Math.max(topologyWidth, ctx.parent.cells.length);
  }

  const topo: TopoCell[] = [];
  ensureTopoWidth(topo, topologyWidth);
  for (let i = 0; i < topologyWidth; i++) topo[i] = emptyTopo();

  // Base layer: genuinely live commit rails at this gap.
  for (const ref of ctx.liveRails) {
    if (ref.col < 0) continue;
    const lane = Math.floor(ref.col / CELL_W);
    addVertical(topo, lane, ref.colorLane);
  }
  // Newer sibling tips on the same parent: keep their spur rails alive
  // through this row so the join is not a dangling 1-row stub.
  for (const lane of ctx.siblingSpurLanes ?? []) {
    if (lane === leafLane) continue;
    addVertical(topo, lane, tipColor);
  }

  const leafCol = leafLane * CELL_W;
  // Spur only when join will close (same gate as list join elbow).
  const paintSpur =
    !opts.node &&
    Boolean(opts.spurRail) &&
    ctx.parent != null &&
    ctx.tipAboveParent;
  if (paintSpur) {
    // Short rail toward the join only — never a multi-node spur.
    ensureTopoWidth(topo, leafCol + 1);
    connect(topo[leafCol]!, { up: true, down: true }, tipColor, 'pipe');
  }

  let cells = topoToCells(topo, g);
  while (cells.length < topologyWidth) {
    cells.push({ ch: ' ', colorLane: null, role: 'blank' });
  }
  if (opts.node) {
    while (cells.length <= leafCol) {
      cells.push({ ch: ' ', colorLane: null, role: 'blank' });
    }
    // Tip node replaces any pipe on this column — no down stem under ◇.
    cells[leafCol] = {
      ch: g.stash,
      colorLane: tipColor,
      role: 'node',
    };
  }

  const anchorLane =
    opts.node || paintSpur ? leafLane : (ctx.parent?.lane ?? leafLane);
  const extraCols = [
    ...ctx.liveRails.map((ref) => ref.col),
    ...(ctx.siblingSpurLanes ?? []).map((lane) => lane * CELL_W),
    ...(ctx.parent ? [ctx.parent.lane * CELL_W] : []),
    leafLane * CELL_W,
  ];
  if (cells.length > budget) {
    cells = sliceCellsAroundLane(cells, budget, anchorLane, extraCols);
  } else if (cells.length < budget) {
    cells = [...cells, ...blankGutter(budget - cells.length)];
  }
  return cells;
}

/**
 * Segments for a stash row — 1-node side leaf tip (`◇` on a free lane).
 *
 * Layout A row 1 (stash): `[gutter │ ◇][ ][subject flex]`
 * Diamond is gutter-only; subject has no leading glyph.
 * `stash@{n}` + hash/date/author live on the spacer beneath.
 */
export function graphStashSegments(
  stash: GraphStash,
  opts: GraphRowOptions,
  parentOrCtx: LaidOutCommit | StashRailContext | null = null,
): Segment[] {
  const subjectColor = opts.subjectColor ?? '#c0caf5';
  const muted = opts.mutedColor ?? '#565f89';
  const gutterWidth = opts.graphWidth ?? 0;
  const laneColors = opts.laneColors ?? DEFAULT_LANE_COLORS;
  const segs: Segment[] = [];
  if (gutterWidth > 0) {
    const railCells = stashLeafRailCells(gutterWidth, parentOrCtx, {
      ascii: opts.ascii,
      node: true,
    });
    segs.push(...cellsToSegments(railCells, laneColors, muted));
    segs.push({ text: ' ', color: muted });
  }
  const used = segmentsText(segs).length;
  const subjectBudget = Math.max(1, opts.width - used);
  segs.push({
    text: trunc(stash.subject, subjectBudget),
    color: subjectColor,
  });
  const len = segmentsText(segs).length;
  if (len < opts.width) {
    segs.push({ text: ' '.repeat(opts.width - len), color: subjectColor });
  }
  return segs;
}

/**
 * Non-selectable row under a stash: live rails + short spur toward join + meta.
 *
 * Spur continues one row only (3b spacer); never a second stash node.
 * No densify (commit spacers own DAG transitions).
 */
export function graphStashSpacerSegments(
  stash: GraphStash,
  opts: GraphRowOptions,
  parentOrCtx: LaidOutCommit | StashRailContext | null = null,
): Segment[] {
  const muted = opts.mutedColor ?? '#565f89';
  const now = opts.nowUnix ?? Math.floor(Date.now() / 1000);
  const gutterWidth = opts.graphWidth ?? 0;
  const laneColors = opts.laneColors ?? DEFAULT_LANE_COLORS;
  const segs: Segment[] = [];
  if (gutterWidth > 0) {
    const railCells = stashLeafRailCells(gutterWidth, parentOrCtx, {
      ascii: opts.ascii,
      node: false,
      spurRail: true,
    });
    segs.push(...cellsToSegments(railCells, laneColors, muted));
    segs.push({ text: ' ', color: muted });
  }

  const hash = stash.id.slice(0, 7);
  const dateRaw = formatRelativeDate(stash.authorDateUnix, now);
  const dateWidth = opts.dateWidth ?? Math.max(dateRaw.length, 4);
  const date = padLeft(trunc(dateRaw, dateWidth), dateWidth);
  const authorRaw = stash.authorName ?? '';
  const authorWidth =
    opts.authorWidth ?? Math.min(16, Math.max(authorRaw.length || 1, 1));
  const author = authorRaw
    ? padLeft(trunc(authorRaw, authorWidth), authorWidth)
    : '';

  const used = segmentsText(segs).length;
  const available = Math.max(0, opts.width - used);
  const cols = pickMetaColumns(available, hash, date, author);
  const meta = metaColumnsText(hash, date, author, cols);
  const leftBudget = Math.max(0, available - meta.length);
  const leftLabel = stash.stashRef;
  if (leftBudget > 0) {
    segs.push({
      text: trunc(leftLabel, leftBudget),
      color: muted,
    });
  }
  if (meta.length > 0) {
    const leftLen = segmentsText(segs).length;
    const pad = Math.max(0, opts.width - leftLen - meta.length);
    if (pad > 0) segs.push({ text: ' '.repeat(pad), color: muted });
    segs.push({ text: meta, color: muted });
  } else {
    const len = segmentsText(segs).length;
    if (len < opts.width) {
      segs.push({ text: ' '.repeat(opts.width - len), color: muted });
    }
  }
  return segs;
}

/**
 * Segments for the synthetic uncommitted row — padded gutter for column rails.
 */
export function graphUncommittedSegments(
  u: GraphUncommitted,
  opts: GraphRowOptions,
): Segment[] {
  const color = opts.subjectColor ?? '#e0af68';
  const muted = opts.mutedColor ?? '#565f89';
  const gutterWidth = opts.graphWidth ?? 0;
  const mark = glyphsFor(opts).uncommitted;
  const label = u.hasChanges
    ? `${mark} uncommitted changes`
    : `${mark} working tree clean`;
  const segs: Segment[] = [];
  if (gutterWidth > 0) {
    segs.push({ text: ' '.repeat(gutterWidth), color: muted });
    segs.push({ text: ' ', color: muted });
  }
  const used = segmentsText(segs).length;
  segs.push({ text: trunc(label, Math.max(1, opts.width - used)), color });
  return segs;
}
