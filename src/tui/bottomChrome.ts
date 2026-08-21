/**
 * Bottom chrome row counts for StatusBar and replacing overlays.
 *
 * New overlays that replace StatusBar must be listed here so pane height
 * shrinks and the overlay stays on screen.
 */

export interface BottomChromeInput {
  /** `?` help overlay is open. */
  showHelp: boolean;
  /** Rows from {@link helpStatusLines} when help is open. */
  helpLines: number;
  /** Depth-0 confirm kind, or null when none. */
  pendingConfirmKind: 'removeWorktree' | string | null;
  /** Graph stash-drop overlay is open. */
  stashDropConfirm: boolean;
  /** Graph origin out-of-sync checkout confirm is open. */
  graphCheckoutConfirm: boolean;
  /** Stash menu overlay rows, or 0 when closed. */
  stashMenuLines: number;
  /** Graph create-branch overlay is open. */
  createBranchOverlay: boolean;
  /** Branch picker overlay rows, or 0 when closed. */
  branchPickerLines: number;
  /** Graph branch picker overlay rows, or 0 when closed. */
  graphBranchPickerLines: number;
}

/**
 * Rows reserved below the panes for StatusBar or a replacing overlay.
 */
export function bottomChromeRows(input: BottomChromeInput): number {
  if (input.showHelp) return input.helpLines;
  if (input.pendingConfirmKind === 'removeWorktree') return 6;
  if (input.pendingConfirmKind) return 7;
  if (input.stashDropConfirm) return 5;
  if (input.graphCheckoutConfirm) return 7;
  if (input.stashMenuLines > 0) return input.stashMenuLines;
  if (input.createBranchOverlay) return 5;
  if (input.branchPickerLines > 0) return input.branchPickerLines;
  if (input.graphBranchPickerLines > 0) return input.graphBranchPickerLines;
  return 1;
}
