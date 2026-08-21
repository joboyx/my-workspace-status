/**
 * Graph gutter glyphs — Unicode (lazygit-class) or ASCII (`git log --graph`).
 *
 * Node semantics (Unicode): commit/merge `●`, HEAD `⊙`, uncommitted `○`,
 * stash `◇`. Merge topology (not a distinct node glyph) identifies merges.
 */

import { asciiGlyphs } from '../icons.js';

/** Columns occupied per logical lane (glyph + spacer / horizontal). */
export const CELL_W = 2;

/**
 * Glyph set for one paint mode.
 */
export type GraphGlyphSet = {
  readonly commit: string;
  /** HEAD commit — including merge-at-HEAD (`⊙` / `@`). */
  readonly headCommit: string;
  /** Alias of `commit` — merges share the commit node glyph. */
  readonly merge: string;
  /** Uncommitted working-tree marker (`○` / `o`). */
  readonly uncommitted: string;
  /** Stash side-leaf node in the gutter (`◇` / `s`) — spur off `stash^1`. */
  readonly stash: string;
  /** Checkout mark inside a branch chip (nf-fa-crosshairs / `+`). */
  readonly checkoutMark: string;
  /** Local↔remote sync mark inside a branch chip (nf-fa-exchange / `=`). */
  readonly syncMark: string;
  readonly vertical: string;
  readonly horizontal: string;
  readonly cornerDownRight: string; // ╮ — open lane to the right of node
  readonly cornerDownLeft: string; // ╭ — open lane to the left of node
  readonly cornerUpRight: string; // ╯ — close lane into node from the right
  readonly cornerUpLeft: string; // ╰ — close lane into node from the left
  readonly teeLeft: string;
  readonly teeRight: string;
  readonly teeDown: string;
  readonly teeUp: string;
  readonly cross: string;
};

const UNICODE: GraphGlyphSet = {
  commit: '●',
  headCommit: '⊙',
  merge: '●',
  uncommitted: '○',
  stash: '◇',
  // PUA Nerd Font icons — MesloLGM metrics stay 1-cell (misc unicode ⌖/⇄ bleed).
  checkoutMark: '', // nf-fa-crosshairs
  syncMark: '', // nf-fa-exchange
  vertical: '│',
  horizontal: '─',
  cornerDownRight: '╮',
  cornerDownLeft: '╭',
  cornerUpRight: '╯',
  cornerUpLeft: '╰',
  teeLeft: '┤',
  teeRight: '├',
  teeDown: '┬',
  teeUp: '┴',
  cross: '┼',
};

/** Classic `git log --graph` ASCII — single-column feel inside CELL_W pairs. */
const ASCII: GraphGlyphSet = {
  commit: '*',
  headCommit: '@',
  merge: '*',
  uncommitted: 'o',
  stash: 's',
  checkoutMark: '+',
  syncMark: '=',
  vertical: '|',
  horizontal: '-',
  cornerDownRight: '\\',
  cornerDownLeft: '/',
  cornerUpRight: '/',
  cornerUpLeft: '\\',
  teeLeft: '+',
  teeRight: '+',
  teeDown: '+',
  teeUp: '+',
  cross: '+',
};

/**
 * Active glyph set from `WS_STATUS_GLYPHS` (same flag as icons).
 *
 * Pass `ascii: true|false` to override the process env (tests).
 */
export function graphGlyphs(ascii: boolean = asciiGlyphs): GraphGlyphSet {
  return ascii ? ASCII : UNICODE;
}
