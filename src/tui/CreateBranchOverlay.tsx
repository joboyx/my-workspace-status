/**
 * Create-branch name prompt overlay (`c` on a graph commit row).
 */

import React from 'react';
import { Box, Text } from 'ink';
import { useTheme } from './theme.js';

export interface CreateBranchOverlayProps {
  /** Short commit id shown in the title. */
  commitId: string;
  /** Live branch name input. */
  name: string;
  /** App status line (validation / create fail). */
  statusMessage?: string;
}

/**
 * One-field branch name prompt: Enter confirm · Esc cancel.
 */
export function CreateBranchOverlay(props: CreateBranchOverlayProps): React.ReactElement {
  const { commitId, name, statusMessage = '' } = props;
  const { palette: PALETTE } = useTheme();
  const short = commitId.slice(0, 7);
  const statusFailed = /failed|error|invalid/i.test(statusMessage);

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={PALETTE.branchFeature} paddingX={1}>
      <Text>
        <Text color={PALETTE.branchFeature} bold>
          Create branch{' '}
        </Text>
        <Text color={PALETTE.muted}>at </Text>
        <Text color={PALETTE.repo}>{short}</Text>
      </Text>
      <Text>
        <Text color={PALETTE.muted}>  name: </Text>
        <Text color={PALETTE.cursor}>{name || '…'}</Text>
      </Text>
      {statusMessage ? (
        <Text color={statusFailed ? PALETTE.deleted : PALETTE.muted} wrap="truncate">
          {statusMessage}
        </Text>
      ) : null}
      <Text color={PALETTE.muted}>Enter confirm · Esc cancel</Text>
    </Box>
  );
}
