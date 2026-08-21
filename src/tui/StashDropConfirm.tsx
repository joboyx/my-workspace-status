/**
 * Stash drop confirmation overlay (`D` on a graph stash row).
 */

import React from 'react';
import { Box, Text } from 'ink';
import { useTheme } from './theme.js';

export interface StashDropConfirmProps {
  /** Stash ref to drop, e.g. `stash@{0}`. */
  stashRef: string;
}

/**
 * Simple y/n drop confirm — Esc / `n` cancel, `y` confirm.
 */
export function StashDropConfirm(props: StashDropConfirmProps): React.ReactElement {
  const { stashRef } = props;
  const { palette: PALETTE, surface } = useTheme();
  const accent = PALETTE.deleted;

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={accent} paddingX={1}>
      <Text>
        <Text color={accent} bold>
          Drop{' '}
        </Text>
        <Text color={PALETTE.file}>{stashRef}</Text>
        <Text color={accent}>?</Text>
      </Text>
      <Text>
        <Text backgroundColor={accent} color={surface} bold>
          {' y '}
        </Text>
        <Text color={PALETTE.muted}> drop   </Text>
        <Text backgroundColor={PALETTE.muted} color={surface} bold>
          {' n '}
        </Text>
        <Text color={PALETTE.muted}> cancel</Text>
      </Text>
    </Box>
  );
}
