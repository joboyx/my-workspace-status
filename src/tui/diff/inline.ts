/**
 * Hunk[] → inline unified-diff style string lines.
 */

import type { DiffLine, Hunk } from './parse.js';

function formatLine(line: DiffLine): string {
  switch (line.kind) {
    case 'add':
      return `+${line.text}`;
    case 'del':
      return `-${line.text}`;
    case 'ctx':
      return ` ${line.text}`;
    case 'meta':
      return line.text;
  }
}

/** Render hunks as classic inline unified-diff lines (header + prefixed body). */
export function renderInline(hunks: Hunk[]): string[] {
  const out: string[] = [];
  for (const hunk of hunks) {
    if (hunk.header) out.push(hunk.header);
    for (const line of hunk.lines) {
      out.push(formatLine(line));
    }
  }
  return out;
}
