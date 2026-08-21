/**
 * Hunk[] → side-by-side string lines padded to `width`.
 *
 * Layout: leftCol | rightCol where leftCol width = floor((width-1)/2).
 * Consecutive del runs zip with following add runs by index (git often
 * emits all deletes then all adds).
 */

import type { DiffLine, Hunk } from './parse.js';

function padTrunc(text: string, colWidth: number): string {
  if (text.length > colWidth) return text.slice(0, colWidth);
  return text.padEnd(colWidth, ' ');
}

function joinRow(left: string, right: string, width: number): string {
  const leftW = Math.floor((width - 1) / 2);
  const rightW = width - 1 - leftW;
  return `${padTrunc(left, leftW)}|${padTrunc(right, rightW)}`;
}

type Pair = { left: string; right: string };

function pairsFromLines(lines: DiffLine[]): Pair[] {
  const pairs: Pair[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.kind === 'meta') {
      pairs.push({ left: line.text, right: '' });
      i += 1;
      continue;
    }
    if (line.kind === 'ctx') {
      const marked = ` ${line.text}`;
      pairs.push({ left: marked, right: marked });
      i += 1;
      continue;
    }
    if (line.kind === 'del' || line.kind === 'add') {
      const dels: DiffLine[] = [];
      const adds: DiffLine[] = [];
      while (i < lines.length && lines[i].kind === 'del') {
        dels.push(lines[i]);
        i += 1;
      }
      while (i < lines.length && lines[i].kind === 'add') {
        adds.push(lines[i]);
        i += 1;
      }
      const n = Math.max(dels.length, adds.length);
      for (let j = 0; j < n; j++) {
        const left = dels[j] ? `-${dels[j].text}` : '';
        const right = adds[j] ? `+${adds[j].text}` : '';
        pairs.push({ left, right });
      }
      continue;
    }
    i += 1;
  }
  return pairs;
}

/** Render hunks as fixed-width side-by-side rows. */
export function renderSideBySide(hunks: Hunk[], width: number): string[] {
  const w = Math.max(width, 3);
  const out: string[] = [];
  for (const hunk of hunks) {
    if (hunk.header) {
      out.push(joinRow(hunk.header, '', w));
    }
    for (const pair of pairsFromLines(hunk.lines)) {
      out.push(joinRow(pair.left, pair.right, w));
    }
  }
  return out;
}
