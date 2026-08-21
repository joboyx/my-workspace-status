/**
 * Pure help-overlay wrap and height math (no Ink).
 *
 * Three columns share the panel inner width. Descriptions word-wrap under the
 * chip pad. {@link helpOverlayRowCount} is the pane-height contract.
 */

/** Help overlay stays on three groups (MOVE / GIT / VIEW). */
export const HELP_COLUMN_COUNT = 3;

/** Round border (2) plus `paddingX={1}` on each side. */
export const HELP_CHROME_COLS = 4;

/** Chip area per row: fits `Ctrl-u Ctrl-d` plus ≥2-col gap before description. */
export const HELP_KEY_WIDTH = 18;

/** One help row: key chips, then description. */
export type HelpKeyRow = readonly [string, string];

/** Group shape needed for wrap height (title/icon unused). */
export interface HelpGroupKeys {
  readonly keys: readonly HelpKeyRow[];
}

/**
 * Description placement inside one help column.
 */
export interface HelpDescLayout {
  /** Columns before wrapped continuation text. */
  readonly indent: number;
  /** Word-wrap width for the description. */
  readonly width: number;
  /** When false, chips occupy the first visual line alone. */
  readonly descOnFirstLine: boolean;
}

/**
 * One painted line of a help entry after wrap.
 */
export interface HelpVisualLine {
  /** True on the first line, which also shows the key chips. */
  readonly chips: boolean;
  /** Leading spaces before {@link text} (0 on the chip line). */
  readonly indent: number;
  /** Description fragment for this line (empty when chips stand alone). */
  readonly text: string;
}

/**
 * Content width inside the help border and horizontal padding.
 */
export function helpInnerWidth(termWidth: number): number {
  return Math.max(0, Math.floor(termWidth) - HELP_CHROME_COLS);
}

/**
 * Width of one of the three help columns at `termWidth`.
 */
export function helpColumnWidth(termWidth: number): number {
  return Math.max(1, Math.floor(helpInnerWidth(termWidth) / HELP_COLUMN_COUNT));
}

/**
 * Painted columns for a help key cluster (` chip ` plus trailing gap).
 *
 * Matches `HelpKey`: each token is 3 + token length, then pad to
 * {@link HELP_KEY_WIDTH} with at least one gap column.
 */
export function helpChipPadWidth(keys: string): number {
  const chips = keys.split(' ');
  const used = chips.reduce((n, chip) => n + chip.length + 3, 0);
  return used + Math.max(1, HELP_KEY_WIDTH - used);
}

/**
 * Trailing gap spaces after chips so the cluster occupies {@link helpChipPadWidth}.
 */
export function helpChipGapSpaces(keys: string): number {
  const chips = keys.split(' ');
  const used = chips.reduce((n, chip) => n + chip.length + 3, 0);
  return Math.max(1, HELP_KEY_WIDTH - used);
}

/**
 * Chip pad vs description wrap width for a column.
 *
 * Wide columns keep chips and the first description line together. Narrow
 * columns put the description on following lines at full column width.
 */
export function helpDescLayout(
  columnWidth: number,
  chipPad: number = HELP_KEY_WIDTH,
): HelpDescLayout {
  const col = Math.max(1, columnWidth);
  const pad = Math.max(0, chipPad);
  const remaining = col - pad;
  if (remaining >= 1) {
    return { indent: pad, width: remaining, descOnFirstLine: true };
  }
  return { indent: 0, width: col, descOnFirstLine: false };
}

/**
 * Word-wrap `text` to `width` columns. Breaks overlong words. Never ellipsizes.
 */
export function wrapHelpDescription(text: string, width: number): string[] {
  const col = Math.max(1, width);
  const words = text.trim().length === 0 ? [] : text.trim().split(/\s+/);
  if (words.length === 0) return [''];

  const lines: string[] = [];
  let current = '';

  const flush = (): void => {
    if (current.length === 0) return;
    lines.push(current);
    current = '';
  };

  const takeChunks = (word: string): void => {
    for (let i = 0; i < word.length; i += col) {
      const chunk = word.slice(i, i + col);
      if (chunk.length === col) {
        lines.push(chunk);
      } else {
        current = chunk;
      }
    }
  };

  for (const word of words) {
    if (word.length > col) {
      flush();
      takeChunks(word);
      continue;
    }
    const next = current.length > 0 ? `${current} ${word}` : word;
    if (next.length <= col) {
      current = next;
    } else {
      flush();
      current = word;
    }
  }
  flush();
  return lines.length > 0 ? lines : [''];
}

/**
 * Wrap the help footer to the overlay inner width.
 */
export function wrapHelpFooter(text: string, innerWidth: number): string[] {
  return wrapHelpDescription(text, Math.max(1, innerWidth));
}

/**
 * Visual lines for one help entry at `columnWidth`.
 *
 * Pass `keys` so wrap width matches the painted chip pad (wide clusters can
 * exceed {@link HELP_KEY_WIDTH}).
 */
export function helpEntryVisualLines(
  description: string,
  columnWidth: number,
  keys: string = '',
): HelpVisualLine[] {
  const chipPad = keys.length > 0 ? helpChipPadWidth(keys) : HELP_KEY_WIDTH;
  const layout = helpDescLayout(columnWidth, chipPad);
  const wrapped = wrapHelpDescription(description, layout.width);
  if (!layout.descOnFirstLine) {
    const rest = wrapped
      .filter((text) => text.length > 0)
      .map((text) => ({ chips: false, indent: layout.indent, text }));
    return [{ chips: true, indent: 0, text: '' }, ...rest];
  }
  return wrapped.map((text, i) => ({
    chips: i === 0,
    indent: i === 0 ? 0 : layout.indent,
    text,
  }));
}

/**
 * Body rows after wrap: each aligned index uses the tallest of the three cells.
 */
export function helpBodyLineCount(
  groups: readonly HelpGroupKeys[],
  columnWidth: number,
): number {
  const rowCount = Math.max(0, ...groups.map((group) => group.keys.length));
  let total = 0;
  for (let row = 0; row < rowCount; row++) {
    let height = 1;
    for (const group of groups) {
      const entry = group.keys[row];
      if (entry === undefined) continue;
      height = Math.max(
        height,
        helpEntryVisualLines(entry[1], columnWidth, entry[0]).length,
      );
    }
    total += height;
  }
  return total;
}

/**
 * Overlay rows: border (2) + title + wrapped body + footer.
 */
export function helpOverlayRowCount(bodyRows: number, footerRows: number): number {
  return 2 + 1 + Math.max(0, bodyRows) + Math.max(0, footerRows);
}

/**
 * Full overlay height for `groups` at `termWidth` with a wrappable footer.
 */
export function helpOverlayHeight(
  groups: readonly HelpGroupKeys[],
  termWidth: number,
  footer: string,
): number {
  const body = helpBodyLineCount(groups, helpColumnWidth(termWidth));
  const footerLines = wrapHelpFooter(footer, helpInnerWidth(termWidth)).length;
  return helpOverlayRowCount(body, footerLines);
}
