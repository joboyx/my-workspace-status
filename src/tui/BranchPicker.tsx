/**
 * Local-branch checkout overlay (`b` on a repo row).
 */

import React from 'react';
import { Box, Text } from 'ink';
import type { LocalBranch } from '../git.js';
import { useTheme } from './theme.js';

export interface BranchPickerProps {
  /** Repo path shown in the title. */
  repoPath: string;
  /** Visible (already filtered) branches. */
  branches: LocalBranch[];
  /** Highlighted index into `branches`. */
  cursor: number;
  /** Live filter substring. */
  filter: string;
  /** True while `listLocalBranches` is in flight. */
  loading?: boolean;
  /**
   * App status line (dirty refuse, checkout fail, Busy…).
   * Shown as a footer while the picker replaces StatusBar.
   */
  statusMessage?: string;
}

/**
 * Branch picker panel: filter query, highlighted row, Esc/Enter hints.
 */
export function BranchPicker(props: BranchPickerProps): React.ReactElement {
  const { repoPath, branches, cursor, filter, loading, statusMessage = '' } = props;
  const { palette: PALETTE } = useTheme();
  const maxRows = 12;
  const start = Math.max(0, Math.min(cursor - Math.floor(maxRows / 2), branches.length - maxRows));
  const visible = branches.slice(Math.max(0, start), Math.max(0, start) + maxRows);
  const statusFailed = /failed|error|dirty/i.test(statusMessage);

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={PALETTE.branchFeature} paddingX={1}>
      <Text>
        <Text color={PALETTE.branchFeature} bold>
          Branch{' '}
        </Text>
        <Text color={PALETTE.repo}>{repoPath}</Text>
        <Text color={PALETTE.muted}>
          {'  '}filter:{' '}
        </Text>
        <Text color={PALETTE.cursor}>{filter || '…'}</Text>
      </Text>
      {loading ? (
        <Text color={PALETTE.muted}>  Loading…</Text>
      ) : branches.length === 0 ? (
        <Text color={PALETTE.muted}>  No matching branches</Text>
      ) : (
        visible.map((b, i) => {
          const index = Math.max(0, start) + i;
          const selected = index === cursor;
          const mark = b.current ? '* ' : '  ';
          return (
            <Text key={b.name} backgroundColor={selected ? PALETTE.cursorBg : undefined}>
              <Text color={selected ? PALETTE.cursor : PALETTE.muted}>{selected ? '❯ ' : '  '}</Text>
              <Text color={b.current ? PALETTE.added : selected ? PALETTE.file : PALETTE.muted}>
                {mark}
                {b.name}
              </Text>
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
        j/k move · type to filter · Enter checkout · Esc close
      </Text>
    </Box>
  );
}
