/**
 * Checkoutable-branch picker at one graph commit (`b` when several names).
 */

import React from 'react';
import { Box, Text } from 'ink';
import { useTheme } from './theme.js';

export interface GraphBranchPickerProps {
  /** Commit the checkoutable names point at. */
  commitId: string;
  /** Checkoutable names — locals then `origin/*` (already grouped and sorted). */
  branches: string[];
  /** Highlighted index into `branches`. */
  cursor: number;
  /** Optional filter when list is long. */
  filter?: string;
  /** App status line (dirty refuse, checkout fail). */
  statusMessage?: string;
}

/**
 * Tiny branch list for checkout at a commit (BranchPicker spirit, fixed list).
 */
export function GraphBranchPicker(props: GraphBranchPickerProps): React.ReactElement {
  const { commitId, branches, cursor, filter, statusMessage = '' } = props;
  const { palette: PALETTE } = useTheme();
  const short = commitId.slice(0, 7);
  const maxRows = 12;
  const start = Math.max(0, Math.min(cursor - Math.floor(maxRows / 2), branches.length - maxRows));
  const visible = branches.slice(Math.max(0, start), Math.max(0, start) + maxRows);
  const statusFailed = /failed|error|dirty/i.test(statusMessage);
  const showFilter = branches.length > 8 || (filter !== undefined && filter.length > 0);

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={PALETTE.branchFeature} paddingX={1}>
      <Text>
        <Text color={PALETTE.branchFeature} bold>
          Checkout{' '}
        </Text>
        <Text color={PALETTE.muted}>at </Text>
        <Text color={PALETTE.repo}>{short}</Text>
        {showFilter ? (
          <>
            <Text color={PALETTE.muted}>
              {'  '}filter:{' '}
            </Text>
            <Text color={PALETTE.cursor}>{filter || '…'}</Text>
          </>
        ) : null}
      </Text>
      {branches.length === 0 ? (
        <Text color={PALETTE.muted}>  No matching branches</Text>
      ) : (
        visible.map((name, i) => {
          const index = Math.max(0, start) + i;
          const selected = index === cursor;
          return (
            <Text key={name} backgroundColor={selected ? PALETTE.cursorBg : undefined}>
              <Text color={selected ? PALETTE.cursor : PALETTE.muted}>{selected ? '❯ ' : '  '}</Text>
              <Text color={selected ? PALETTE.file : PALETTE.muted}>{name}</Text>
            </Text>
          );
        })
      )}
      {statusMessage ? (
        <Text color={statusFailed ? PALETTE.deleted : PALETTE.muted} wrap="truncate">
          {statusMessage}
        </Text>
      ) : null}
      <Text color={PALETTE.muted}>
        {showFilter
          ? 'j/k move · type to filter · Enter checkout · Esc cancel'
          : 'j/k move · Enter checkout · Esc cancel'}
      </Text>
    </Box>
  );
}
