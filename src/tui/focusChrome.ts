/**
 * Focused-pane title chrome: TREE / GRAPH / DIFF labels for left & right columns.
 *
 * Pure helpers so unit tests can assert focus affordance without Ink.
 */

import type { FocusPane } from './nav/stack.js';

/** Rows reserved at the top of each pane for the title chip. */
export const PANE_TITLE_ROWS = 1;

/** Short pane title shown in the focus chip. */
export type PaneTitle = 'TREE' | 'GRAPH' | 'DIFF' | '';

/** Right-host modes that map to a title (mirrors `RightPaneMode`). */
export type RightPaneTitleMode = 'diff' | 'graph' | 'commitMeta' | 'empty';

/**
 * Left-column label from ViewStack depth.
 */
export function leftPaneTitle(navDepth: number): PaneTitle {
  if (navDepth === 1) return 'GRAPH';
  return 'TREE';
}

/**
 * Right-column label from right-pane host mode.
 */
export function rightPaneTitle(mode: RightPaneTitleMode): PaneTitle {
  switch (mode) {
    case 'graph':
      return 'GRAPH';
    case 'diff':
      return 'DIFF';
    case 'commitMeta':
      return 'TREE';
    case 'empty':
      return '';
  }
}

/**
 * Plain-text chip: focused gets a marker + label; unfocused is indented muted form.
 */
export function formatPaneTitle(label: PaneTitle, focused: boolean): string {
  if (!label) return '';
  return focused ? `▶ ${label}` : `  ${label}`;
}

/**
 * Inputs for {@link focusPaneChromePlain}.
 */
export type FocusPaneChromeInput = {
  navDepth: number;
  focusPane: FocusPane;
  rightMode: RightPaneTitleMode;
};

/**
 * Plain-text left/right chips for tests — focus states must differ.
 */
export function focusPaneChromePlain(input: FocusPaneChromeInput): {
  left: string;
  right: string;
} {
  return {
    left: formatPaneTitle(
      leftPaneTitle(input.navDepth),
      input.focusPane === 'left',
    ),
    right: formatPaneTitle(
      rightPaneTitle(input.rightMode),
      input.focusPane === 'right',
    ),
  };
}
