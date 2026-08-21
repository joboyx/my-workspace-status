/**
 * Depth-1 right pane until P4 ships the commit file tree.
 */

import React from 'react';
import { Box, Text } from 'ink';
import path from 'node:path';
import type { GraphListRow } from './graph/list.js';
import { useTheme } from './theme.js';

/**
 * Pure stub copy for tests and the placeholder Text.
 */
export function commitMetaStubLines(
  row: GraphListRow | null,
  repoPath: string,
): string[] {
  const repo = path.basename(repoPath) || repoPath;
  if (!row) {
    return [`${repo}`, 'select a commit (file tree in P4)'];
  }
  const subject = row.segments.map((s) => s.text).join('').trim();
  if (row.kind === 'uncommitted') {
    return [repo, 'uncommitted', subject || 'working tree', 'file tree in P4'];
  }
  if (row.kind === 'spacer') {
    return [repo, 'select a commit (file tree in P4)'];
  }
  if (row.kind === 'stash') {
    return [
      repo,
      'stash',
      row.stashRef ?? row.commitId ?? '',
      subject,
      'file tree in P4',
    ];
  }
  const short = (row.commitId ?? '').slice(0, 7);
  return [repo, 'commit', short, subject, 'file tree in P4'];
}

export interface CommitMetaStubProps {
  row: GraphListRow | null;
  repoPath: string;
  width: number;
  height: number;
}

/**
 * Commit / stash / uncommitted summary placeholder (P3).
 */
export function CommitMetaStub(props: CommitMetaStubProps): React.ReactElement {
  const { palette: PALETTE } = useTheme();
  const lines = commitMetaStubLines(props.row, props.repoPath);
  return (
    <Box flexDirection="column" height={props.height} width={props.width}>
      {lines.map((line, i) => (
        <Text key={i} color={i === 0 ? PALETTE.repo : PALETTE.muted} wrap="truncate">
          {line}
        </Text>
      ))}
    </Box>
  );
}
