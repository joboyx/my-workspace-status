/**
 * Background for a list row under B10 rules (cursor / search / flash / none).
 */
export function listRowBackground(opts: {
  selected: boolean;
  flashBg?: string;
  cursorBg: string;
  searchMatch?: boolean;
  searchBg?: string;
}): string | undefined {
  if (opts.selected) return opts.cursorBg;
  if (opts.searchMatch && opts.searchBg) return opts.searchBg;
  return opts.flashBg;
}
