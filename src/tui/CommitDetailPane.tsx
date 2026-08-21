/**
 * Depth-1 right pane: commit/stash/worktree meta header + file tree.
 */

import React from 'react';
import { Box, Text } from 'ink';
import type { VisibleRow } from './model/types.js';
import { TreePane } from './TreePane.js';
import { useTheme } from './theme.js';

export interface CommitDetailPaneProps {
  title: string;
  subtitle?: string;
  rows: VisibleRow[];
  cursor: number;
  height: number;
  width: number;
  folds: Set<string>;
  loading?: boolean;
  /** Row ids matching the active `/` search (B10 search bg via TreePane). */
  searchMatchIds?: Set<string>;
  /** Node id → time of last change; paints a fading highlight. */
  flashes?: Map<string, number>;
  /** Wall-clock ms used as `now` for flash decay. */
  clock?: number;
  /** EasyMotion overlay — forwarded to the file TreePane when that list is focused. */
  easyMotion?: boolean;
  /** Partial EasyMotion label typed so far (dim unmatched). */
  easyMotionTyped?: string;
}

/**
 * Header rows CommitDetailPane reserves (title + optional subtitle).
 * Empty titles still occupy one blank line so the file list does not jump.
 */
export function commitDetailHeaderHeight(
  paneHeight: number,
  title: string,
  subtitle?: string,
): number {
  const lines = [title, subtitle].filter(
    (line): line is string => Boolean(line && line.trim()),
  );
  return Math.min(lines.length || 1, Math.max(1, paneHeight));
}

/**
 * Meta header plus an embedded TreePane for commit-scoped files (B10 via TreePane).
 */
export function CommitDetailPane(props: CommitDetailPaneProps): React.ReactElement {
  const { palette: PALETTE } = useTheme();
  const headerH = commitDetailHeaderHeight(
    props.height,
    props.title,
    props.subtitle,
  );
  const headerLines = [props.title, props.subtitle].filter(
    (line): line is string => Boolean(line && line.trim()),
  );
  const treeH = Math.max(1, props.height - headerH);

  return (
    <Box flexDirection="column" height={props.height} width={props.width}>
      <Box flexDirection="column" height={headerH} width={props.width}>
        {headerLines.length === 0 ? (
          <Text color={PALETTE.muted}> </Text>
        ) : (
          headerLines.map((line, i) => (
            <Text
              key={i}
              color={i === 0 ? PALETTE.repo : PALETTE.muted}
              wrap="truncate"
            >
              {line}
            </Text>
          ))
        )}
      </Box>
      {props.loading && props.rows.length === 0 ? (
        <Text color={PALETTE.muted}>loading files…</Text>
      ) : (
        <TreePane
          rows={props.rows}
          cursor={props.cursor}
          height={treeH}
          width={props.width}
          folds={props.folds}
          searchMatchIds={props.searchMatchIds}
          flashes={props.flashes}
          clock={props.clock}
          easyMotion={props.easyMotion}
          easyMotionTyped={props.easyMotionTyped}
        />
      )}
    </Box>
  );
}
