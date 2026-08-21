/**
 * Display-only breadcrumb mirroring ViewStack + focusPane.
 * Not focusable; Esc remains the sole back key.
 * Trailing op-status (fetch / pull / default-branch / toasts) sits opposite the path.
 */

import React from 'react';
import { Box, Text } from 'ink';
import type { FocusPane, NavState } from './nav/stack.js';
import {
  breadcrumbSegments,
  formatBreadcrumb,
} from './nav/stack.js';
import { allocateChromeRow, isOpStatusError } from './opStatus.js';
import { useTheme } from './theme.js';

/**
 * Plain-text breadcrumb for unit tests (includes focus marker).
 */
export function breadcrumbPlain(nav: NavState, workspaceLabel: string): string {
  return formatBreadcrumb(breadcrumbSegments(nav, workspaceLabel), nav.focusPane);
}

/**
 * Plain-text chrome row for tests: breadcrumb + trailing op status.
 */
export function breadcrumbChromePlain(
  nav: NavState,
  workspaceLabel: string,
  opStatusLine = '',
): string {
  const left = breadcrumbPlain(nav, workspaceLabel);
  if (!opStatusLine) return left;
  return `${left} ${opStatusLine}`;
}

export interface BreadcrumbProps {
  nav: NavState;
  workspaceLabel: string;
  width: number;
  /** Trailing fetch / long-op status (right-aligned). */
  opStatusLine?: string;
}

/**
 * One Ink row: segments joined with › ; current segment styled by focusPane.
 * Optional op status is right-aligned opposite the breadcrumb.
 */
export function Breadcrumb(props: BreadcrumbProps): React.ReactElement {
  const { nav, workspaceLabel, width, opStatusLine = '' } = props;
  const { palette: PALETTE } = useTheme();
  const segments = breadcrumbSegments(nav, workspaceLabel);
  const focusPane: FocusPane = nav.focusPane;
  const { breadcrumbMax, opStatusMax } = allocateChromeRow(
    width,
    opStatusLine.length,
  );
  const opStatusColor = isOpStatusError(opStatusLine)
    ? PALETTE.deleted
    : PALETTE.muted;

  return (
    <Box width={width} justifyContent="space-between">
      <Box width={breadcrumbMax} flexGrow={opStatusMax > 0 ? 0 : 1}>
        <Text wrap="truncate">
          {segments.map((seg, i) => {
            const isLast = i === segments.length - 1;
            const color = isLast
              ? focusPane === 'right'
                ? PALETTE.cursor
                : PALETTE.heading
              : PALETTE.muted;
            return (
              <Text key={`${i}:${seg}`} color={color}>
                {i > 0 ? ' › ' : ''}
                {isLast && focusPane === 'right' ? `[${seg}]` : seg}
              </Text>
            );
          })}
        </Text>
      </Box>
      {opStatusMax > 0 ? (
        <Box width={opStatusMax}>
          <Text color={opStatusColor} wrap="truncate">
            {opStatusLine}
          </Text>
        </Box>
      ) : null}
    </Box>
  );
}
