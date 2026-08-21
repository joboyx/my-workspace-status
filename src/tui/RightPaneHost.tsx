/**
 * Right-pane switch: DiffPane / GraphPane / CommitMetaStub by nav depth.
 */

import React from 'react';
import { Box, Text } from 'ink';
import type { NavState } from './nav/stack.js';
import { navDepth } from './nav/stack.js';
import { DiffPane } from './DiffPane.js';
import type { DiffPaneContent } from './DiffPane.js';
import { GraphPane } from './GraphPane.js';
import { CommitDetailPane } from './CommitDetailPane.js';
import { shouldShowFileDiff, shouldShowGraphDetail, type GraphListRow } from './graph/list.js';
import type { VisibleRow } from './model/types.js';
import { useTheme } from './theme.js';

/** Which widget the right column should show. */
export type RightPaneMode = 'diff' | 'graph' | 'commitMeta' | 'empty';

/**
 * Choose right-pane mode from nav depth and focused tree row.
 */
export function rightPaneMode(nav: NavState, focused: VisibleRow | undefined): RightPaneMode {
  const depth = navDepth(nav);
  if (depth >= 2) return 'diff';
  if (depth === 1) return 'commitMeta';
  if (shouldShowFileDiff(nav, focused)) return 'diff';
  if (shouldShowGraphDetail(nav, focused)) return 'graph';
  return 'empty';
}

export interface RightPaneHostProps {
  nav: NavState;
  focusedRow: VisibleRow | undefined;
  content: DiffPaneContent | null;
  loading: boolean;
  mode: 'inline' | 'sideBySide';
  scroll: number;
  height: number;
  width: number;
  focusHint: string;
  fullContext: boolean;
  /** Horizontal pan columns (Track D). */
  colOffset?: number;
  graphRows: GraphListRow[];
  graphCursor: number;
  graphLoading: boolean;
  graphLoadingOlder: boolean;
  graphRepoPath: string | null;
  graphModel: import('./graph/types.js').GraphModel | null;
  graphSync: import('./graph/selectionDetail.js').GraphSyncChrome | null;
  /** Depth-1 commit file tree (P4). */
  commitFileRows: VisibleRow[];
  commitFileCursor: number;
  commitFileFolds: Set<string>;
  commitFilesLoading: boolean;
  commitDetailTitle: string;
  commitDetailSubtitle?: string;
  /** Row ids matching the active `/` search (graph + commit-file lists). */
  searchMatchIds?: Set<string>;
  /** Diff row indices matching the active `/` search. */
  searchMatchDiffIndices?: ReadonlySet<number>;
  /** Shared flash map (graph rows + commit-file tree). */
  flashes?: Map<string, number>;
  /** Wall-clock ms for flash decay. */
  clock?: number;
  /** EasyMotion overlay — passed only when this column's list is the jump target. */
  easyMotion?: boolean;
  /** Partial EasyMotion label typed so far (dim unmatched). */
  easyMotionTyped?: string;
  /** Session-only side-by-side left-column fraction. */
  splitFraction?: number;
}

/**
 * Host for the right column — DiffPane, GraphPane, or commit detail.
 */
export function RightPaneHost(props: RightPaneHostProps): React.ReactElement {
  const { palette: PALETTE } = useTheme();
  const mode = rightPaneMode(props.nav, props.focusedRow);

  if (mode === 'diff') {
    return (
      <DiffPane
        content={props.content}
        loading={props.loading}
        mode={props.mode}
        scroll={props.scroll}
        height={props.height}
        width={props.width}
        focusHint={props.focusHint}
        fullContext={props.fullContext}
        colOffset={props.colOffset}
        searchMatchIndices={props.searchMatchDiffIndices}
        splitFraction={props.splitFraction}
      />
    );
  }

  if (mode === 'graph') {
    return (
      <GraphPane
        rows={props.graphRows}
        cursor={props.graphCursor}
        height={props.height}
        width={props.width}
        loading={props.graphLoading}
        loadingOlder={props.graphLoadingOlder}
        focused={props.nav.focusPane === 'right'}
        sync={props.graphSync}
        model={props.graphModel}
        searchMatchIds={props.searchMatchIds}
        flashes={props.flashes}
        clock={props.clock}
        easyMotion={props.easyMotion}
        easyMotionTyped={props.easyMotionTyped}
      />
    );
  }

  if (mode === 'commitMeta') {
    return (
      <CommitDetailPane
        title={props.commitDetailTitle}
        subtitle={props.commitDetailSubtitle}
        rows={props.commitFileRows}
        cursor={props.commitFileCursor}
        height={props.height}
        width={props.width}
        folds={props.commitFileFolds}
        loading={props.commitFilesLoading}
        searchMatchIds={props.searchMatchIds}
        flashes={props.flashes}
        clock={props.clock}
        easyMotion={props.easyMotion}
        easyMotionTyped={props.easyMotionTyped}
      />
    );
  }

  return (
    <Box flexDirection="column" height={props.height} width={props.width}>
      <Text color={PALETTE.muted} wrap="truncate">
        select a repo
      </Text>
    </Box>
  );
}
