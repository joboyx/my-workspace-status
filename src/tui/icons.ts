/**
 * Nerd Font glyph registry for the Ink TUI.
 *
 * A patched **Nerd Font** is a hard requirement for the TUI. Set
 * `WS_STATUS_GLYPHS=ascii` to fall back to plain markers if a terminal
 * renders the private-use area as tofu.
 *
 * Every glyph here occupies exactly **one** terminal column: Nerd Font icons
 * live in the private-use area (U+E000–U+F8FF), which `visibleWidth` counts as
 * width 1. Do not add codepoints from the emoji or CJK ranges — those are
 * double-width and break Ink's box layout. Plain-report badges
 * (`badgeForChange` in `changes.ts`) stay emoji; they are a separate surface.
 */

import { visibleWidth } from '../helpers.js';
import { getTheme } from './theme.js';
import type { Segment } from './theme.js';
import type { FileChange, SyncStatus } from '../types.js';
import type { FileStatusLetter } from './model/types.js';

/** True when the operator opted out of Nerd Font glyphs. */
export const asciiGlyphs = process.env.WS_STATUS_GLYPHS === 'ascii';

/** Pick the Nerd Font glyph unless ASCII fallback is active. */
function glyph(nerd: string, ascii: string): string {
  return asciiGlyphs ? ascii : nerd;
}

/* ── Structure ──────────────────────────────────────────────────────────── */

/** Fold chevrons — width 1 in both modes. */
export const FOLD_EXPANDED = '▾';
export const FOLD_COLLAPSED = '▸';

/** Cursor accent bar painted in the left-most tree column. */
export const CURSOR_BAR = '▌';

/** Vertical rule between panes and inside the diff gutter. */
export const RULE = '│';

export const ICON_WORKSPACE = glyph('', '#');
export const ICON_REPO = glyph('', '@');
/**
 * Linked `git worktree` checkout (plain-report `🔗`).
 * Nerd: nf-oct-link (``); ASCII: `L`.
 */
export const ICON_LINKED_WORKTREE = glyph('', 'L');
export const ICON_BRANCH = glyph('', '&');
export const ICON_FOLDER = glyph('', '/');
export const ICON_FOLDER_OPEN = glyph('', '/');
export const ICON_CLEAN = glyph('', '.');
export const ICON_IGNORED = glyph('', '~');
export const ICON_AHEAD = glyph('', '^');
export const ICON_BEHIND = glyph('', 'v');
export const ICON_DIVERGED = glyph('', 'Y');
export const ICON_NO_UPSTREAM = glyph('', '?');
export const ICON_SYNCED = glyph('', '=');
/**
 * Plain-report `✅` — HEAD is an ancestor of the default-branch tip.
 * Nerd: nf-fa-check-circle (``); ASCII: `M`.
 */
export const ICON_MERGED_INTO_DEFAULT = glyph('', 'M');
/**
 * Plain-report `🌱` — HEAD is not merged into default.
 * Nerd: nf-fa-tree (``); ASCII: `o`.
 */
export const ICON_OPEN_VS_DEFAULT = glyph('', 'o');
/**
 * Reviewed mark on a dirty file row (GitLens-style "viewed").
 * Nerd: nf-fa-eye (``); ASCII: `*`. Distinct from ICON_CLEAN / ICON_SYNCED.
 */
export const ICON_VIEWED = glyph('', '*');

/** Help-panel group icons. */
export const ICON_MOVE = glyph('', '+');
export const ICON_DIFF = glyph('', '%');
export const ICON_APP = glyph('', '!');

/** Font the TUI is designed against — surfaced in `--help` and the help panel. */
export const REQUIRED_FONT = 'MesloLGM Nerd Font Mono';
/* ── File type devicons ─────────────────────────────────────────────────── */

interface FileIcon {
  glyph: string;
  color: string;
}

const DEFAULT_FILE_GLYPH = '';

/** Extension → devicon. Codepoints match the nvim-web-devicons vocabulary. */
const EXTENSION_ICONS: Record<string, Omit<FileIcon, 'color'> & { color?: string }> = {
  ts: { glyph: '', color: '#519aba' },
  mts: { glyph: '', color: '#519aba' },
  cts: { glyph: '', color: '#519aba' },
  tsx: { glyph: '', color: '#519aba' },
  js: { glyph: '', color: '#cbcb41' },
  mjs: { glyph: '', color: '#cbcb41' },
  cjs: { glyph: '', color: '#cbcb41' },
  jsx: { glyph: '', color: '#519aba' },
  json: { glyph: '', color: '#cbcb41' },
  md: { glyph: '', color: '#519aba' },
  mdx: { glyph: '', color: '#519aba' },
  py: { glyph: '', color: '#ffbc03' },
  sh: { glyph: '', color: '#89e051' },
  bash: { glyph: '', color: '#89e051' },
  zsh: { glyph: '', color: '#89e051' },
  html: { glyph: '', color: '#e34c26' },
  css: { glyph: '', color: '#563d7c' },
  scss: { glyph: '', color: '#f55385' },
  yml: { glyph: '', color: '#6d8086' },
  yaml: { glyph: '', color: '#6d8086' },
  toml: { glyph: '', color: '#6d8086' },
  java: { glyph: '', color: '#cc3e44' },
  cs: { glyph: '', color: '#596706' },
  go: { glyph: '', color: '#519aba' },
  rs: { glyph: '', color: '#dea584' },
  sql: { glyph: '', color: '#dad8d8' },
  png: { glyph: '', color: '#a074c4' },
  jpg: { glyph: '', color: '#a074c4' },
  jpeg: { glyph: '', color: '#a074c4' },
  gif: { glyph: '', color: '#a074c4' },
  svg: { glyph: '', color: '#ffb13b' },
  lock: { glyph: '', color: '#bbbbbb' },
  txt: { glyph: '' },
};

/** Exact filenames that deserve their own icon regardless of extension. */
const FILENAME_ICONS: Record<string, FileIcon> = {
  '.gitignore': { glyph: '', color: '#e24329' },
  '.gitattributes': { glyph: '', color: '#e24329' },
  '.gitmodules': { glyph: '', color: '#e24329' },
  'package.json': { glyph: '', color: '#e8274b' },
  'package-lock.json': { glyph: '', color: '#7a0d21' },
  dockerfile: { glyph: '', color: '#458ee6' },
  makefile: { glyph: '', color: '#6d8086' },
  'readme.md': { glyph: '', color: '#519aba' },
  '.envrc': { glyph: '', color: '#faf743' },
  '.env': { glyph: '', color: '#faf743' },
};

/**
 * Devicon for a repo-relative file path — matched on exact filename first,
 * then extension, then a generic file glyph.
 */
export function fileIcon(filePath: string): FileIcon {
  const fileColour = getTheme().palette.file;
  if (asciiGlyphs) return { glyph: '·', color: fileColour };
  const name = (filePath.split('/').pop() ?? filePath).toLowerCase();
  const byName = FILENAME_ICONS[name];
  if (byName) return byName;
  const ext = name.includes('.') ? name.slice(name.lastIndexOf('.') + 1) : '';
  const byExt = EXTENSION_ICONS[ext];
  if (byExt) return { glyph: byExt.glyph, color: byExt.color ?? fileColour };
  return { glyph: DEFAULT_FILE_GLYPH, color: fileColour };
}

/* ── File status ────────────────────────────────────────────────────────── */

const FILE_BADGES: Record<FileStatusLetter, string> = {
  A: 'A ',
  M: 'M ',
  S: 'S ',
  MS: 'MS',
  D: 'D ',
  R: 'R ',
  U: 'U ',
  C: 'C ',
};

/** Exactly 2 display columns — right-aligned like the VS Code SCM gutter. */
export function tuiFileBadge(status: FileStatusLetter): string {
  return FILE_BADGES[status];
}

/** Semantic colour for a status letter. */
export function statusColor(status: FileStatusLetter): string {
  const p = getTheme().palette;
  switch (status) {
    case 'A':
    case 'S':
      return p.added;
    case 'M':
    case 'MS':
      return p.modified;
    case 'D':
    case 'U':
      return p.deleted;
    case 'R':
    case 'C':
      return p.renamed;
    default:
      return p.file;
  }
}

/** Map a FileChange to the same letter vocabulary as tree `statusLetter`. */
export function statusLetterFromChange(change: FileChange): FileStatusLetter {
  // Conflict before MS — staged+unstaged both set must not swallow U.
  if (change.unstagedStatus === 'U' || change.stagedStatus === 'U') return 'U';
  if (change.stagedStatus && change.unstagedStatus) return 'MS';
  const status = change.unstagedStatus ?? change.stagedStatus;
  if (status === 'R') return 'R';
  if (status === 'D') return 'D';
  if (change.untracked || status === 'A') return 'A';
  if (change.stagedStatus && !change.unstagedStatus) return 'S';
  if (status === 'C') return 'C';
  return 'M';
}

export function tuiFileBadgeForChange(change: FileChange): string {
  return tuiFileBadge(statusLetterFromChange(change));
}

/* ── Branch / sync ──────────────────────────────────────────────────────── */

/**
 * Merge-into-default mark for TUI branch chrome.
 * Maps plain `✅` / `🌱` to single-column glyphs (never emoji — Ink width).
 */
export function tuiMergeMark(merged: boolean | null): string {
  if (merged === true) return ICON_MERGED_INTO_DEFAULT;
  if (merged === false) return ICON_OPEN_VS_DEFAULT;
  return '';
}

/**
 * Sync mark: glyph plus commit count, e.g. ahead-by-2 or behind-by-3.
 */
export function tuiSyncMark(status: SyncStatus, note = ''): string {
  if (status === 'no-upstream') return ICON_NO_UPSTREAM;
  if (status === 'behind') {
    const count = note.match(/behind by (\d+)/)?.[1] ?? '';
    return `${ICON_BEHIND}${count}`;
  }
  if (status === 'ahead') {
    const count = note.match(/ahead by (\d+)/)?.[1] ?? '';
    return `${ICON_AHEAD}${count}`;
  }
  if (status === 'diverged') return ICON_DIVERGED;
  return ICON_SYNCED;
}

/** Colour matching `tuiSyncMark` semantics. */
export function syncColor(status: SyncStatus): string {
  const p = getTheme().palette;
  if (status === 'behind') return p.deleted;
  if (status === 'ahead') return p.added;
  if (status === 'diverged') return p.modified;
  return p.muted;
}

/**
 * Accent for {@link ICON_VIEWED}. Uses the theme cyan/blue token (`renamed`),
 * never the clean/success green (`added`).
 */
export function viewedColor(): string {
  return getTheme().palette.renamed;
}

/* ── Diff sections ──────────────────────────────────────────────────────── */

export type TuiSectionKind = 'staged' | 'unstaged' | 'new';

/** Diff pane section headers. */
export function tuiSectionHeader(kind: TuiSectionKind): string {
  if (kind === 'staged') return 'STAGED';
  if (kind === 'unstaged') return 'UNSTAGED';
  return 'NEW';
}

/** Accent colour for a diff section header. */
export function sectionColor(kind: TuiSectionKind): string {
  const p = getTheme().palette;
  if (kind === 'staged') return p.added;
  if (kind === 'new') return p.untracked;
  return p.modified;
}

/* ── Width helpers ──────────────────────────────────────────────────────── */

/** Truncate a string to at most `width` terminal columns (via `visibleWidth`). */
export function truncateVisible(value: string, width: number): string {
  if (width <= 0) return '';
  if (visibleWidth(value) <= width) return value;
  let out = '';
  let w = 0;
  for (const char of [...value]) {
    const cw = visibleWidth(char);
    if (w + cw > width) break;
    out += char;
    w += cw;
  }
  return out;
}

/**
 * Truncate a styled run to `width` columns, appending `…`.
 * Segments past the budget are dropped; the segment straddling the boundary is
 * cut mid-text so colour boundaries stay intact.
 */
export function truncateSegments(segments: Segment[], width: number): Segment[] {
  if (width <= 0) return [];
  let total = 0;
  for (const seg of segments) total += visibleWidth(seg.text);
  if (total <= width) return segments;

  const budget = Math.max(0, width - 1);
  const out: Segment[] = [];
  let used = 0;
  for (const seg of segments) {
    const segWidth = visibleWidth(seg.text);
    if (used + segWidth <= budget) {
      out.push(seg);
      used += segWidth;
      continue;
    }
    out.push({ ...seg, text: truncateVisible(seg.text, budget - used) });
    break;
  }
  out.push({ text: '…', dim: true });
  return out;
}

/** True if any codepoint is in the emoji / pictograph range (≥ U+1F300). */
export function hasWideEmoji(value: string): boolean {
  for (const char of [...value]) {
    const code = char.codePointAt(0) ?? 0;
    if (code >= 0x1f300) return true;
  }
  return false;
}
