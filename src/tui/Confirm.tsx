/**
 * Revert confirmation overlay with tracked / untracked counts (`y` / `Y` / `n`).
 */

import React from 'react';
import { Box, Text } from 'ink';
import { useTheme } from './theme.js';

export interface ConfirmProps {
  /** Display label (file, dir, or repo path). */
  label: string;
  trackedCount: number;
  untrackedCount: number;
}

function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many;
}

/**
 * Destructive-action prompt: `y` discards tracked (keeps untracked, except a
 * single untracked file which still deletes), `Y` also deletes untracked,
 * `n`/Esc cancels.
 */
export function Confirm(props: ConfirmProps): React.ReactElement {
  const { label, trackedCount, untrackedCount } = props;
  const { palette: PALETTE, surface } = useTheme();
  const singleUntracked = trackedCount === 0 && untrackedCount === 1;
  const accent = singleUntracked ? PALETTE.deleted : PALETTE.modified;
  const untrackedFate = singleUntracked ? 'deleted' : 'kept';

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={accent} paddingX={1}>
      <Text>
        <Text color={accent} bold>
          Revert{' '}
        </Text>
        <Text color={PALETTE.file}>{label}</Text>
        <Text color={accent}>?</Text>
      </Text>
      <Text color={PALETTE.muted}>
        {'  '}
        {trackedCount} tracked {plural(trackedCount, 'file', 'files')} → discarded
      </Text>
      <Text color={singleUntracked ? accent : PALETTE.muted}>
        {'  '}
        {untrackedCount} untracked {plural(untrackedCount, 'file', 'files')} →{' '}
        {untrackedFate}
      </Text>
      <Text>
        <Text backgroundColor={accent} color={surface} bold>
          {' y '}
        </Text>
        <Text color={PALETTE.muted}> revert   </Text>
        <Text backgroundColor={PALETTE.deleted} color={surface} bold>
          {' Y '}
        </Text>
        <Text color={PALETTE.muted}> revert + delete untracked   </Text>
        <Text backgroundColor={PALETTE.muted} color={surface} bold>
          {' n '}
        </Text>
        <Text color={PALETTE.muted}> cancel</Text>
      </Text>
    </Box>
  );
}
