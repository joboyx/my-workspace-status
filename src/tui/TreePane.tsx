/**
 * Left tree pane — VS Code SCM style rows with a cursor accent bar,
 * Nerd Font icons and a right-aligned status column.
 *
 * Renders only the viewport slice (`visibleTreeWindow`) so large workspaces
 * do not remount every row on cursor moves (B2/B5). Pane width is fixed by
 * the parent — never content-driven.
 */

import React from 'react';
import { Box, Text } from 'ink';
import { visibleWidth } from '../helpers.js';
import { Segments } from './Segments.js';
import {
  CURSOR_BAR,
  FOLD_COLLAPSED,
  FOLD_EXPANDED,
  truncateSegments,
} from './icons.js';
import { treeViewportStart } from './hitTest.js';
import { treeRowEmphasis } from './treeEmphasis.js';
import { useTheme, segmentsText } from './theme.js';
import { easyMotionLabels } from './easyMotion.js';
import type { TreeNode, VisibleRow } from './model/types.js';

export interface TreePaneProps {
  rows: VisibleRow[];
  cursor: number;
  /** Max rows to paint (viewport height). */
  height: number;
  /** Inner content width (cols) for safe truncation. */
  width: number;
  /** Collapsed node ids — drives fold chevron. */
  folds: Set<string>;
  /** Node ids matching the active `/` search (B10 search bg). */
  searchMatchIds?: Set<string>;
  /** EasyMotion overlay — assign a–z labels on visible rows. */
  easyMotion?: boolean;
  /** Partial EasyMotion label typed so far (dim unmatched). */
  easyMotionTyped?: string;
  /** Node id → time of last change; paints a fading highlight. */
  flashes?: Map<string, number>;
  /**
   * Wall-clock ms used as `now` for flash decay. Bumped while flashes/ghosts
   * live so React.memo can skip cursor-only moves (B5) without treating a
   * tick counter as elapsed time.
   */
  clock?: number;
}

/**
 * Resolve `now` for flash strength.
 * `clock` must be wall-clock ms (same domain as `flashedAt`). A bare tick
 * counter (0, 1, 2…) makes elapsed negative and kills the highlight.
 */
export function treeFlashNow(
  clock: number | undefined,
  hasFlashes: boolean,
  wallMs: () => number = Date.now,
): number {
  if (clock !== undefined && clock > 0) return clock;
  return hasFlashes ? wallMs() : 0;
}

/** Rows with a stable `id` — enough for pure viewport window tests. */
export type TreeWindowRow = { id: string };

/**
 * First visible index + sliced window centred on `cursor`.
 * Shared formula with hit-testing (`treeViewportStart`).
 */
export function visibleTreeWindow<T extends TreeWindowRow>(
  rows: ReadonlyArray<T>,
  cursor: number,
  height: number,
): { start: number; visible: T[] } {
  const viewHeight = Math.max(1, height);
  const start = treeViewportStart(rows.length, cursor, viewHeight);
  return { start, visible: rows.slice(start, start + viewHeight) as T[] };
}

function hasChildren(node: TreeNode): boolean {
  return (
    (node.kind === 'workspace' ||
      node.kind === 'repo' ||
      node.kind === 'checkout' ||
      node.kind === 'group' ||
      node.kind === 'dir') &&
    node.children.length > 0
  );
}

function foldChevron(row: VisibleRow, folds: Set<string>): string {
  if (row.node.kind === 'file' || !hasChildren(row.node)) return ' ';
  return folds.has(row.id) ? FOLD_COLLAPSED : FOLD_EXPANDED;
}

interface TreeRowViewProps {
  row: VisibleRow;
  selected: boolean;
  colWidth: number;
  folds: Set<string>;
  searchMatchIds?: Set<string>;
  easyMotion?: boolean;
  easyMotionTyped: string;
  motionLabel: string | undefined;
  flashedAt: number | undefined;
  now: number;
}

/**
 * Single painted tree row. Memoized so cursor-only moves only re-render
 * the previously/newly selected rows when props are otherwise stable.
 */
const TreeRowView = React.memo(function TreeRowView({
  row,
  selected,
  colWidth,
  folds,
  searchMatchIds,
  easyMotion,
  easyMotionTyped,
  motionLabel,
  flashedAt,
  now,
}: TreeRowViewProps): React.ReactElement {
  const { palette: PALETTE, pill: PILL } = useTheme();
  const indent = '  '.repeat(row.depth);
  const chevron = foldChevron(row, folds);
  // EasyMotion: replace indent+chevron prefix with a fixed-width label
  // so the divider / treeWidth stays put.
  const prefix =
    easyMotion && motionLabel ? motionLabel.padEnd(2, ' ') : null;

  // Reserve: cursor bar (1) + indent + chevron and its trailing space (2)
  // or EasyMotion label width (2).
  const prefixWidth = 1 + (prefix !== null ? 2 : indent.length + 2);
  const trailingWidth = visibleWidth(segmentsText(row.trailing));
  const pad = trailingWidth > 0 ? 1 : 0;
  const labelBudget = Math.max(
    1,
    colWidth - prefixWidth - trailingWidth - pad,
  );
  const label = truncateSegments(row.segments, labelBudget);
  const gap =
    pad + Math.max(0, labelBudget - visibleWidth(segmentsText(label)));

  // B10: cursor = edge bar + background only — never wash status fg.
  // Search match bg reuses pill.filter (same as P8 flash stack priority).
  const emphasis = treeRowEmphasis({
    selected,
    flashedAt,
    now,
    searchMatch: searchMatchIds?.has(row.id) === true,
    searchBg: PILL.filter.bg,
    cursorBg: PALETTE.cursorBg,
  });
  const rowBg = emphasis.backgroundColor;
  const labelMatches =
    !easyMotionTyped || (motionLabel?.startsWith(easyMotionTyped) ?? false);

  return (
    <Text wrap="truncate" backgroundColor={rowBg}>
      <Text color={PALETTE.cursor} bold>
        {emphasis.edge ? CURSOR_BAR : ' '}
      </Text>
      {prefix !== null ? (
        <Text
          color={labelMatches ? PALETTE.cursor : PALETTE.muted}
          bold={labelMatches}
        >
          {prefix}
        </Text>
      ) : (
        <>
          <Text>{indent}</Text>
          <Text color={PALETTE.muted}>{chevron} </Text>
        </>
      )}
      <Segments segments={label} backgroundColor={rowBg} />
      <Text>{' '.repeat(gap)}</Text>
      <Segments segments={row.trailing} backgroundColor={rowBg} />
    </Text>
  );
});

/**
 * Render a window of tree rows centred on the cursor.
 *
 * Row geometry is computed in columns rather than delegated to flexbox so the
 * right-aligned status column stays put when labels are truncated.
 * Do not remount this pane on cursor-only moves (no `key={cursor}` on root).
 */
export function TreePane({
  rows,
  cursor,
  height,
  width,
  folds,
  searchMatchIds,
  easyMotion,
  easyMotionTyped = '',
  flashes,
  clock,
}: TreePaneProps): React.ReactElement {
  const { palette: PALETTE } = useTheme();
  const { start, visible } = visibleTreeWindow(rows, cursor, height);
  const colWidth = Math.max(1, width);
  // Prefer the fade-timer wall clock so cursor-only moves keep stable `now`
  // props and React.memo can skip unchanged rows (B5). Must be ms timestamps
  // (same domain as `flashedAt`), never a 0/1/2… tick counter.
  const now = treeFlashNow(clock, flashes !== undefined && flashes.size > 0);
  const motionLabels = easyMotion ? easyMotionLabels(visible.length) : [];

  return (
    <Box flexDirection="column" width={width} overflow="hidden">
      {visible.length === 0 ? (
        <Text color={PALETTE.muted}>No matching rows</Text>
      ) : (
        visible.map((row, i) => {
          const index = start + i;
          return (
            <TreeRowView
              key={row.id}
              row={row}
              selected={index === cursor}
              colWidth={colWidth}
              folds={folds}
              searchMatchIds={searchMatchIds}
              easyMotion={easyMotion}
              easyMotionTyped={easyMotionTyped}
              motionLabel={motionLabels[i]}
              flashedAt={flashes?.get(row.id)}
              now={now}
            />
          );
        })
      )}
    </Box>
  );
}
