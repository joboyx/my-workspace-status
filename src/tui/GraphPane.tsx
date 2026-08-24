/**
 * Commit-graph list pane (depth 0 right / depth 1 left).
 */

import React from 'react';
import { Box, Text } from 'ink';
import {
  graphRowIdentity,
  isGraphRowPairHighlighted,
  isSelectableGraphRow,
  type GraphListRow,
} from './graph/list.js';
import { listRowBackground } from './listEmphasis.js';
import { treeFlashNow } from './TreePane.js';
import { flashBackground, useTheme } from './theme.js';
import { flashStrength } from './watch.js';
import {
  graphChromeBudget,
  graphSelectionDetailLines,
  graphSyncHeaderSegments,
  type GraphSyncChrome,
} from './graph/selectionDetail.js';
import type { GraphModel } from './graph/types.js';
import { treeViewportStart } from './hitTest.js';
import { easyMotionLabels } from './easyMotion.js';
import { CURSOR_BAR, truncateSegments } from './icons.js';
import { Segments } from './Segments.js';
import { isDefaultBranch } from '../helpers.js';

/** Same windowing as TreePane — height is the list area only. */
export function graphViewportStart(rowCount: number, cursor: number, height: number): number {
  return treeViewportStart(rowCount, cursor, height);
}

/**
 * Visible graph rows after header/footer chrome — same window GraphPane paints
 * and EasyMotion jumps must resolve against.
 */
export function visibleGraphWindow<T>(
  rows: ReadonlyArray<T>,
  cursor: number,
  height: number,
  loadingOlder = false,
  wantHeader = true,
): {
  start: number;
  visible: T[];
  listHeight: number;
  header: boolean;
  footer: boolean;
} {
  const chrome = graphChromeBudget(height, loadingOlder, wantHeader);
  const start = graphViewportStart(rows.length, cursor, chrome.listHeight);
  return {
    start,
    visible: rows.slice(start, start + chrome.listHeight) as T[],
    listHeight: chrome.listHeight,
    header: chrome.header,
    footer: chrome.footer,
  };
}

export interface GraphPaneProps {
  rows: GraphListRow[];
  cursor: number;
  height: number;
  width: number;
  loading?: boolean;
  loadingOlder?: boolean;
  /** When true, use cursor accent border colour via parent; row uses cursorBg. */
  focused?: boolean;
  /** Focused repo sync chrome for the header line. */
  sync?: GraphSyncChrome | null;
  /** Full model for selection footer (refs / subject). */
  model?: GraphModel | null;
  /** Row ids matching the active `/` search (B10 search bg). */
  searchMatchIds?: Set<string>;
  /** Node id → time of last change; paints a fading highlight. */
  flashes?: Map<string, number>;
  /**
   * Wall-clock ms used as `now` for flash decay. Same domain as `flashedAt`.
   */
  clock?: number;
  /** EasyMotion overlay — assign a–z labels on the visible list window. */
  easyMotion?: boolean;
  /** Partial EasyMotion label typed so far (dim unmatched). */
  easyMotionTyped?: string;
}

/**
 * Paint a window of graph rows centred on the cursor, with optional sync
 * header and selection detail footer.
 */
export function GraphPane(props: GraphPaneProps): React.ReactElement {
  const {
    rows,
    cursor,
    height,
    width,
    loading,
    loadingOlder,
    focused,
    sync,
    model,
    searchMatchIds,
    flashes,
    clock,
    easyMotion,
    easyMotionTyped = '',
  } = props;
  const { palette: PALETTE, pill: PILL } = useTheme();
  // Only reserve header space when sync chrome will actually paint.
  const {
    start,
    visible,
    header,
    footer: wantFooter,
  } = visibleGraphWindow(rows, cursor, height, Boolean(loadingOlder), Boolean(sync));
  const colWidth = Math.max(1, width);
  const motionLabels = easyMotion ? easyMotionLabels(visible.length) : [];
  const selected = rows[cursor] ?? null;
  const now = treeFlashNow(clock, flashes !== undefined && flashes.size > 0);

  const headerSegs =
    header && sync
      ? graphSyncHeaderSegments(sync, {
          width: colWidth,
          branchColor: isDefaultBranch(sync.branch, sync.defaultBranchOverride)
            ? PALETTE.branchDefault
            : PALETTE.branchFeature,
          mutedColor: PALETTE.muted,
        })
      : null;

  const footer = wantFooter
    ? graphSelectionDetailLines(selected, model ?? null, {
        width: colWidth,
        mutedColor: PALETTE.muted,
        subjectColor: PALETTE.repo,
        refLocalColor: PALETTE.branchFeature,
        refDefaultColor: PALETTE.branchDefault,
        refRemoteColor: PALETTE.dir,
        refTagColor: PALETTE.modified,
        headMarkColor: PALETTE.headMark,
        overflowColor: PALETTE.heading,
        headBranch: sync?.branch,
        defaultBranchOverride: sync?.defaultBranchOverride,
      })
    : null;

  if (loading && rows.length === 0) {
    return (
      <Box flexDirection="column" height={height} width={width}>
        <Text color={PALETTE.muted}>loading graph…</Text>
      </Box>
    );
  }

  return (
    <Box flexDirection="column" flexGrow={1} overflow="hidden" height={height} width={width}>
      {headerSegs ? (
        <Text wrap="truncate">
          <Segments segments={headerSegs} />
        </Text>
      ) : null}
      {visible.length === 0 ? (
        <Text color={PALETTE.muted}>No commits</Text>
      ) : (
        visible.map((row, i) => {
          const index = start + i;
          const pairHighlighted = isGraphRowPairHighlighted(rows, cursor, index);
          const cursorHere = index === cursor && isSelectableGraphRow(row);
          const flashedAt = flashes?.get(graphRowIdentity(row, model?.repoPath ?? ''));
          const rowBg = listRowBackground({
            selected: pairHighlighted && focused !== false,
            cursorBg: PALETTE.cursorBg,
            searchMatch: searchMatchIds?.has(row.id) === true,
            searchBg: PILL.filter.bg,
            flashBg: flashBackground(flashStrength(flashedAt, now)),
          });
          const motionLabel = motionLabels[i];
          const prefix = easyMotion && motionLabel ? motionLabel.padEnd(2, ' ') : null;
          const labelMatches =
            !easyMotionTyped || (motionLabel?.startsWith(easyMotionTyped) ?? false);
          const segBudget = Math.max(1, colWidth - 1 - (prefix !== null ? 2 : 0));
          return (
            <Text key={row.id} wrap="truncate" backgroundColor={rowBg}>
              <Text color={PALETTE.cursor} bold>
                {cursorHere ? CURSOR_BAR : ' '}
              </Text>
              {prefix !== null ? (
                <Text color={labelMatches ? PALETTE.cursor : PALETTE.muted} bold={labelMatches}>
                  {prefix}
                </Text>
              ) : null}
              <Segments
                segments={truncateSegments(row.segments, segBudget)}
                backgroundColor={rowBg}
                bold={pairHighlighted}
              />
            </Text>
          );
        })
      )}
      {footer
        ? footer.footer.map((line, i) => (
            <Text key={`footer-${i}`} wrap="truncate">
              <Segments segments={line} />
            </Text>
          ))
        : null}
      {loadingOlder ? (
        <Text color={PALETTE.muted} dimColor>
          loading older…
        </Text>
      ) : null}
    </Box>
  );
}
