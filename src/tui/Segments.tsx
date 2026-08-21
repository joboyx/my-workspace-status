/**
 * Render `Segment[]` as nested Ink `<Text>` runs.
 */

import React from 'react';
import { Text } from 'ink';
import type { Segment } from './theme.js';

export interface SegmentsProps {
  segments: Segment[];
  /** Applied to every segment that does not set its own background. */
  backgroundColor?: string;
  /** Force bold on every segment (active row emphasis). */
  bold?: boolean;
}

/**
 * Paint a styled run. Segments flagged `raw` carry pre-rendered ANSI and are
 * emitted untouched so highlighter output survives.
 */
export function Segments(props: SegmentsProps): React.ReactElement {
  const { segments, backgroundColor, bold } = props;
  return (
    <>
      {segments.map((seg, i) =>
        seg.raw ? (
          <Text key={i}>{seg.text}</Text>
        ) : (
          <Text
            key={i}
            color={seg.color}
            backgroundColor={seg.backgroundColor ?? backgroundColor}
            bold={bold || seg.bold}
            dimColor={seg.dim}
            italic={seg.italic}
          >
            {seg.text}
          </Text>
        ),
      )}
    </>
  );
}
