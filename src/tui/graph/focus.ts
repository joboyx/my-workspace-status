/**
 * Which list cursor j/k should drive given nav + visibility.
 * `'none'` means the focused pane is not a list (e.g. DiffPane) — App scrolls
 * the diff instead of moving a cursor.
 */

/** Focus target for list navigation keys. */
export type GraphFocusTarget = 'tree' | 'graph' | 'commitFiles' | 'none';

/**
 * Resolve whether move/moveTo adjusts the tree cursor, graph cursor, commit
 * files, or neither.
 */
export function listFocusTarget(args: {
  depth: 0 | 1 | 2;
  focusPane: 'left' | 'right';
  graphVisible: boolean;
}): GraphFocusTarget {
  const { depth, focusPane, graphVisible } = args;
  if (depth === 0 && focusPane === 'left') return 'tree';
  if (depth === 0 && focusPane === 'right' && graphVisible) return 'graph';
  if (depth === 1 && focusPane === 'left' && graphVisible) return 'graph';
  if (depth === 1 && focusPane === 'right') return 'commitFiles';
  if (depth === 2 && focusPane === 'left') return 'commitFiles';
  return 'none';
}

/**
 * True when the focused pane is the graph list: depth 0 right, depth 1 left,
 * or any later depth where that pane is the graph.
 */
export function isGraphListFocused(args: {
  depth: 0 | 1 | 2;
  focusPane: 'left' | 'right';
  graphVisible: boolean;
}): boolean {
  return listFocusTarget(args) === 'graph';
}
