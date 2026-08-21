/**
 * Structured diff rows for the Ink pane.
 *
 * The string renderers (`renderInline` / `renderSideBySide`) stay as the
 * plain-text contract; this module is the styled counterpart, carrying line
 * numbers and cell kinds so the pane can colour a gutter without re-parsing.
 */

import type { TuiSectionKind } from '../icons.js';
import { parseUnifiedDiff } from './parse.js';
import type { DiffLine, Hunk } from './parse.js';

export type DiffCellKind = 'add' | 'del' | 'ctx' | 'meta' | 'empty';

export interface DiffCell {
  kind: DiffCellKind;
  text: string;
  /** Line number shown in the gutter (absent for `meta` / `empty`). */
  lineNo?: number;
}

export type DiffRow =
  | { kind: 'section'; section: TuiSectionKind }
  | { kind: 'hunk'; text: string }
  | { kind: 'line'; left: DiffCell; right?: DiffCell };

function cellFromLine(line: DiffLine, side: 'old' | 'new'): DiffCell {
  return {
    kind: line.kind === 'meta' ? 'meta' : line.kind,
    text: line.text,
    lineNo: side === 'old' ? (line.oldNo ?? line.newNo) : (line.newNo ?? line.oldNo),
  };
}

function inlineRows(hunks: Hunk[]): DiffRow[] {
  const out: DiffRow[] = [];
  for (const hunk of hunks) {
    if (hunk.header) out.push({ kind: 'hunk', text: hunk.header });
    for (const line of hunk.lines) {
      // Inline view numbers by the post-image, falling back for deletions.
      out.push({ kind: 'line', left: cellFromLine(line, 'new') });
    }
  }
  return out;
}

const EMPTY_CELL: DiffCell = { kind: 'empty', text: '' };

/**
 * Pair a hunk's lines into left (pre-image) / right (post-image) cells.
 * Git emits deletions before additions, so consecutive runs zip by index.
 */
function pairHunk(hunk: Hunk): DiffRow[] {
  const out: DiffRow[] = [];
  const lines = hunk.lines;
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    if (line.kind === 'meta') {
      out.push({ kind: 'line', left: cellFromLine(line, 'old') });
      i += 1;
      continue;
    }

    if (line.kind === 'ctx') {
      out.push({
        kind: 'line',
        left: cellFromLine(line, 'old'),
        right: cellFromLine(line, 'new'),
      });
      i += 1;
      continue;
    }

    const dels: DiffLine[] = [];
    const adds: DiffLine[] = [];
    while (i < lines.length && lines[i].kind === 'del') dels.push(lines[i++]);
    while (i < lines.length && lines[i].kind === 'add') adds.push(lines[i++]);

    const pairCount = Math.max(dels.length, adds.length);
    for (let j = 0; j < pairCount; j++) {
      out.push({
        kind: 'line',
        left: dels[j] ? cellFromLine(dels[j], 'old') : EMPTY_CELL,
        right: adds[j] ? cellFromLine(adds[j], 'new') : EMPTY_CELL,
      });
    }
  }

  return out;
}

function sideBySideRows(hunks: Hunk[]): DiffRow[] {
  const out: DiffRow[] = [];
  for (const hunk of hunks) {
    if (hunk.header) out.push({ kind: 'hunk', text: hunk.header });
    out.push(...pairHunk(hunk));
  }
  return out;
}

export interface BuildDiffRowsOptions {
  staged: string;
  unstaged: string;
  mode: 'inline' | 'sideBySide';
  /** Label the unstaged section `NEW` (untracked file synthesised as all-add). */
  isNew?: boolean;
}

/**
 * Rows for both diff sections. Empty sections are omitted entirely.
 */
export function buildDiffRows(opts: BuildDiffRowsOptions): DiffRow[] {
  const render = opts.mode === 'sideBySide' ? sideBySideRows : inlineRows;
  const out: DiffRow[] = [];

  const staged = parseUnifiedDiff(opts.staged);
  if (staged.length > 0) {
    out.push({ kind: 'section', section: 'staged' });
    out.push(...render(staged));
  }

  const unstaged = parseUnifiedDiff(opts.unstaged);
  if (unstaged.length > 0) {
    out.push({ kind: 'section', section: opts.isNew ? 'new' : 'unstaged' });
    out.push(...render(unstaged));
  }

  return out;
}

/** Widest line number in the rows — sizes the gutter column. */
export function gutterWidth(rows: DiffRow[]): number {
  let max = 0;
  for (const row of rows) {
    if (row.kind !== 'line') continue;
    max = Math.max(max, row.left.lineNo ?? 0, row.right?.lineNo ?? 0);
  }
  return Math.max(2, String(max).length);
}
