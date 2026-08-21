/**
 * Origin out-of-sync checkout confirm (`b` when local exists and tips differ).
 */

import React from 'react';
import { Box, Text } from 'ink';
import { useTheme } from './theme.js';

export interface GraphCheckoutConfirmProps {
  /** Local branch that will be checked out. */
  localBranch: string;
  /** Origin remote-tracking ref the local is out of sync with. */
  remoteRef: string;
}

/**
 * Short y/n confirm — Esc / `n` cancel, `y` checkout then fast-forward to `remoteRef`.
 */
export function GraphCheckoutConfirm(props: GraphCheckoutConfirmProps): React.ReactElement {
  const { localBranch, remoteRef } = props;
  const { palette: PALETTE, surface } = useTheme();
  const accent = PALETTE.modified;

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={accent} paddingX={1}>
      <Text>
        <Text color={PALETTE.file}>{localBranch}</Text>
        <Text color={PALETTE.muted}> is not in sync with </Text>
        <Text color={PALETTE.file}>{remoteRef}</Text>
      </Text>
      <Text color={accent}>Checkout local then pull?</Text>
      <Text>
        <Text backgroundColor={accent} color={surface} bold>
          {' y '}
        </Text>
        <Text color={PALETTE.muted}> checkout then pull   </Text>
        <Text backgroundColor={PALETTE.muted} color={surface} bold>
          {' n '}
        </Text>
        <Text color={PALETTE.muted}> cancel</Text>
      </Text>
    </Box>
  );
}
