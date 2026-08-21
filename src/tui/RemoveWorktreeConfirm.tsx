/**
 * Linked-worktree remove confirmation (`W` on a linked checkout row).
 */

import React from 'react';
import { Box, Text } from 'ink';
import { ICON_MERGED_INTO_DEFAULT, ICON_OPEN_VS_DEFAULT } from './icons.js';
import { useTheme } from './theme.js';

export interface RemoveWorktreeConfirmProps {
  /** Workspace-relative worktree path. */
  path: string;
  branch: string;
  /** `true` merged, `false` open, `null` unknown. */
  mergedIntoDefault: boolean | null;
  /** When true, confirm uses `--force` because the worktree is dirty. */
  force: boolean;
}

/**
 * Destructive y/n confirm for `git worktree remove`.
 * Copy always states merge status and whether `--force` will be used.
 */
export function RemoveWorktreeConfirm(
  props: RemoveWorktreeConfirmProps,
): React.ReactElement {
  const { path: wtPath, branch, mergedIntoDefault, force } = props;
  const { palette: PALETTE, surface } = useTheme();
  const accent = PALETTE.deleted;
  const mergeText =
    mergedIntoDefault === true
      ? `merged into default ${ICON_MERGED_INTO_DEFAULT}`
      : mergedIntoDefault === false
        ? `NOT merged into default ${ICON_OPEN_VS_DEFAULT}`
        : 'merge status unknown';

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={accent} paddingX={1}>
      <Text>
        <Text color={accent} bold>
          Remove worktree{' '}
        </Text>
        <Text color={PALETTE.file}>{wtPath}</Text>
        <Text color={accent}>?</Text>
      </Text>
      <Text color={PALETTE.muted}>
        {'  '}branch {branch} — {mergeText}
      </Text>
      {force ? (
        <Text color={accent}>
          {'  '}dirty worktree — will use --force
        </Text>
      ) : (
        <Text color={PALETTE.muted}>{'  '}clean worktree</Text>
      )}
      <Text>
        <Text backgroundColor={accent} color={surface} bold>
          {' y '}
        </Text>
        <Text color={PALETTE.muted}> remove   </Text>
        <Text backgroundColor={PALETTE.muted} color={surface} bold>
          {' n '}
        </Text>
        <Text color={PALETTE.muted}> cancel</Text>
      </Text>
    </Box>
  );
}
