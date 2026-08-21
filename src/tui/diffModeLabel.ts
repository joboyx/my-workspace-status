/**
 * User-facing copy for diff layout. Internal mode stays `sideBySide`.
 */

export type DiffLayoutMode = 'inline' | 'sideBySide';

/**
 * Bottom-bar pill / help word for a diff layout.
 */
export function diffModeUserLabel(mode: DiffLayoutMode): 'inline' | 'split' {
  return mode === 'inline' ? 'inline' : 'split';
}

/**
 * Diff pane header caption, including the narrow-fallback note.
 */
export function diffPaneModeLabel(
  mode: DiffLayoutMode,
  effectiveMode: DiffLayoutMode,
): string {
  if (mode === 'sideBySide' && effectiveMode === 'inline') {
    return 'inline (too narrow)';
  }
  return diffModeUserLabel(mode);
}

/**
 * Toast after toggling `i`.
 */
export function diffModeToast(mode: DiffLayoutMode): string {
  return `Diff: ${diffModeUserLabel(mode)}`;
}
