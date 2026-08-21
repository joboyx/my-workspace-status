/**
 * Right diff pane — syntax-highlighted staged/unstaged (or NEW) sections with
 * a line-number gutter and a scroll window.
 */

import React, { useMemo } from 'react';
import { Box, Text } from 'ink';
import { Segments } from './Segments.js';
import { RULE, sectionColor, truncateVisible, tuiSectionHeader } from './icons.js';
import { useTheme } from './theme.js';
import type { Segment, ThemePalette } from './theme.js';
import { highlightLine, languageForPath, truncateAnsi } from './diff/highlight.js';
import { buildDiffRows, gutterWidth } from './diff/rows.js';
import type { DiffCell, DiffRow } from './diff/rows.js';
import { sliceVisible } from './diffPan.js';
import { diffPaneModeLabel } from './diffModeLabel.js';
import { DIFF_SPLIT_FRACTION, NARROW_SXS, sideBySideColumnWidths } from './diffSplit.js';

export interface DiffPaneContent {
  staged: string;
  unstaged: string;
  /** Untracked empty-git-diff → label unstaged section as NEW. */
  isNew: boolean;
}

export interface DiffPaneProps {
  /** null when focus is not a file. */
  content: DiffPaneContent | null;
  loading: boolean;
  mode: 'inline' | 'sideBySide';
  scroll: number;
  /** Visible body rows (excludes path header). */
  height: number;
  /** Content width for side-by-side columns. */
  width: number;
  focusHint: string;
  /** When true, the header shows that unlimited context is active. */
  fullContext?: boolean;
  /** Horizontal pan columns (Track D). */
  colOffset?: number;
  /** Diff row indices matching the active `/` search (B10 search bg). */
  searchMatchIndices?: ReadonlySet<number>;
  /** Session-only side-by-side left-column fraction (default 0.5). */
  splitFraction?: number;
}

/** Visible code window after horizontal pan (no wrap). */
export function codeWindow(text: string, colOffset: number, codeWidth: number): string {
  return sliceVisible(text, colOffset, codeWidth);
}

/** Re-export — canonical value lives in `diffSplit.ts`. */
export { NARROW_SXS } from './diffSplit.js';

/**
 * Resolve paint mode for DiffPane: side-by-side falls back to inline when
 * the pane is narrower than {@link NARROW_SXS}.
 */
export function effectiveDiffMode(
  mode: 'inline' | 'sideBySide',
  width: number,
): 'inline' | 'sideBySide' {
  const colWidth = Math.max(20, width);
  return mode === 'sideBySide' && colWidth < NARROW_SXS ? 'inline' : mode;
}

const SIGN: Record<DiffCell['kind'], string> = {
  add: '+',
  del: '-',
  ctx: ' ',
  meta: ' ',
  empty: ' ',
};

function cellColor(kind: DiffCell['kind'], palette: ThemePalette): string | undefined {
  if (kind === 'add') return palette.added;
  if (kind === 'del') return palette.deleted;
  if (kind === 'meta') return palette.muted;
  return undefined;
}

/**
 * `<lineNo> │ <sign> <code>` for one diff cell, padded to `width` columns.
 * Context lines get syntax highlighting; added and deleted lines keep a solid
 * accent colour so the change itself stays the loudest thing on the row.
 */
function cellSegments(
  cell: DiffCell,
  width: number,
  gutter: number,
  language: string | null,
  palette: ThemePalette,
  colOffset: number,
): Segment[] {
  const lineNo = cell.lineNo ? String(cell.lineNo) : '';
  const accent = cellColor(cell.kind, palette);
  const codeWidth = Math.max(1, width - gutter - 4);
  // Pan on plain text before highlight (v1: ctx may clip mid-token).
  const windowed = codeWindow(cell.text, colOffset, codeWidth);
  const plain = truncateVisible(windowed, codeWidth); // still used for pad width
  const pad = ' '.repeat(Math.max(0, codeWidth - plain.length));

  let code: Segment;
  if (accent && cell.kind !== 'meta') {
    code = { text: plain, color: accent };
  } else {
    const highlighted = highlightLine(windowed, language);
    code = { text: truncateAnsi(highlighted, codeWidth), raw: true };
  }

  return [
    { text: lineNo.padStart(gutter), color: palette.muted, dim: true },
    { text: ` ${RULE} `, color: palette.muted, dim: true },
    { text: SIGN[cell.kind], color: accent, bold: true },
    code,
    { text: pad },
  ];
}

function rowSegments(
  row: DiffRow,
  width: number,
  gutter: number,
  language: string | null,
  palette: ThemePalette,
  colOffset: number,
  splitFraction: number,
): Segment[] {
  if (row.kind === 'section') {
    const color = sectionColor(row.section);
    return [{ text: ` ${tuiSectionHeader(row.section)} `, color, bold: true }];
  }
  if (row.kind === 'hunk') {
    return [{ text: truncateVisible(row.text, width), color: palette.diffHunk }];
  }

  if (!row.right) {
    return cellSegments(row.left, width, gutter, language, palette, colOffset);
  }

  const { leftWidth, rightWidth } = sideBySideColumnWidths(width, splitFraction);
  return [
    ...cellSegments(row.left, leftWidth, gutter, language, palette, colOffset),
    { text: RULE, color: palette.muted, dim: true },
    ...cellSegments(row.right, rightWidth, gutter, language, palette, colOffset),
  ];
}

/**
 * Render a scrolled window of diff rows for the focused file.
 */
export function DiffPane(props: DiffPaneProps): React.ReactElement {
  const {
    content,
    loading,
    mode,
    scroll,
    height,
    width,
    focusHint,
    fullContext,
    colOffset = 0,
    searchMatchIndices,
    splitFraction = DIFF_SPLIT_FRACTION,
  } = props;
  const { palette: PALETTE, pill: PILL } = useTheme();
  const viewHeight = Math.max(1, height);
  const colWidth = Math.max(20, width);
  const pan = Math.max(0, colOffset);

  const effectiveMode = effectiveDiffMode(mode, width);

  const language = useMemo(() => languageForPath(focusHint), [focusHint]);

  const rows = useMemo(() => {
    if (!content) return [];
    return buildDiffRows({
      staged: content.staged,
      unstaged: content.unstaged,
      mode: effectiveMode,
      isNew: content.isNew,
    });
  }, [content, effectiveMode]);

  const gutter = useMemo(() => gutterWidth(rows), [rows]);

  const maxStart = Math.max(0, rows.length - viewHeight);
  const start = Math.min(Math.max(0, scroll), maxStart);
  const visible = rows.slice(start, start + viewHeight);

  const modeLabel = diffPaneModeLabel(mode, effectiveMode);

  const fullLabel = fullContext ? ' · full' : '';
  const panLabel = pan > 0 ? ` · pan ${pan}` : '';

  const scrollHint =
    rows.length > viewHeight ? `  ${Math.min(start + viewHeight, rows.length)}/${rows.length}` : '';

  return (
    <Box flexDirection="column" flexGrow={1} overflow="hidden">
      <Text wrap="truncate">
        <Text color={PALETTE.heading} bold>
          {focusHint || 'Diff'}
        </Text>
        <Text color={PALETTE.muted}>
          {'  '}
          {modeLabel}
          {fullLabel}
          {panLabel}
          {scrollHint}
        </Text>
      </Text>
      {!content && !loading ? (
        <Text color={PALETTE.muted}>Focus a file to see its diff</Text>
      ) : loading ? (
        <Text color={PALETTE.modified}>loading…</Text>
      ) : rows.length === 0 ? (
        <Text color={PALETTE.muted}>(no diff)</Text>
      ) : (
        visible.map((row, i) => {
          const rowIndex = start + i;
          const rowBg = searchMatchIndices?.has(rowIndex) ? PILL.filter.bg : undefined;
          return (
            <Text key={rowIndex} wrap="truncate" backgroundColor={rowBg}>
              <Segments
                segments={rowSegments(row, colWidth, gutter, language, PALETTE, pan, splitFraction)}
                backgroundColor={rowBg}
              />
            </Text>
          );
        })
      )}
    </Box>
  );
}
