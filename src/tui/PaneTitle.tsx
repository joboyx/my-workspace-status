/**
 * One-line pane title chip — bold cursor colour when focused, muted otherwise.
 */

import React from 'react';
import { Text } from 'ink';
import { formatPaneTitle, type PaneTitle as PaneTitleLabel } from './focusChrome.js';
import { useTheme } from './theme.js';

export interface PaneTitleProps {
  label: PaneTitleLabel;
  focused: boolean;
}

/**
 * Focus affordance chip above pane content (`▶ TREE` / muted `  GRAPH`).
 * Always occupies one row so list height stays aligned with hit-testing.
 */
export function PaneTitle(props: PaneTitleProps): React.ReactElement {
  const { label, focused } = props;
  const { palette: PALETTE } = useTheme();
  const text = formatPaneTitle(label, focused);
  return (
    <Text
      color={focused ? PALETTE.cursor : PALETTE.muted}
      bold={focused}
      wrap="truncate"
    >
      {text || ' '}
    </Text>
  );
}
