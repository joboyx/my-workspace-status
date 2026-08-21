/**
 * Stash family overlay (`S`) — lists only ops valid for the focused row.
 */

import React from 'react';
import { Box, Text } from 'ink';
import { useTheme } from './theme.js';
import { stashMenuOpDetail, type StashOp } from './stashOps.js';

export interface StashMenuOverlayProps {
  /** Muted repo path or focused stash ref under the title. */
  subtitle: string;
  /** Ops in push → apply → pop → drop order. */
  ops: readonly StashOp[];
  /** App status line (Busy… / Ctrl+C / fail). */
  statusMessage?: string;
}

/**
 * Status-line color for the stash overlay: error (failed/error/invalid),
 * info (Busy… / Ctrl+C), or none when empty.
 */
export function stashOverlayStatusTone(
  statusMessage: string,
): 'error' | 'info' | 'none' {
  if (!statusMessage) return 'none';
  return /failed|error|invalid/i.test(statusMessage) ? 'error' : 'info';
}

/**
 * Keyed stash menu: chip + label (+ stash ref for apply/pop/drop). Esc cancels.
 */
export function StashMenuOverlay(props: StashMenuOverlayProps): React.ReactElement {
  const { subtitle, ops, statusMessage = '' } = props;
  const { palette: PALETTE, surface } = useTheme();
  const accent = PALETTE.modified;
  const statusTone = stashOverlayStatusTone(statusMessage);

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={accent} paddingX={1}>
      <Text>
        <Text color={accent} bold>
          Stash{' '}
        </Text>
        {subtitle ? <Text color={PALETTE.muted}>{subtitle}</Text> : null}
      </Text>
      {ops.map((op) => {
        const detail = stashMenuOpDetail(op);
        return (
          <Text key={op.id}>
            <Text backgroundColor={accent} color={surface} bold>
              {` ${op.key} `}
            </Text>
            <Text color={PALETTE.file}> {op.label}</Text>
            {detail ? <Text color={PALETTE.muted}> {detail}</Text> : null}
          </Text>
        );
      })}
      {statusTone !== 'none' ? (
        <Text color={statusTone === 'error' ? PALETTE.deleted : PALETTE.muted} wrap="truncate">
          {statusMessage}
        </Text>
      ) : null}
      <Text color={PALETTE.muted}>Esc cancel</Text>
    </Box>
  );
}
