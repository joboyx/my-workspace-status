/**
 * Unified diff → Hunk[] parse + pane line composer.
 */

import { tuiSectionHeader } from '../icons.js';
import { renderInline } from './inline.js';
import { renderSideBySide } from './sideBySide.js';

export type DiffLineKind = 'ctx' | 'add' | 'del' | 'meta';

export type DiffLine = {
  kind: DiffLineKind;
  text: string;
  /** 1-based line number in the pre-image (`ctx` and `del` only). */
  oldNo?: number;
  /** 1-based line number in the post-image (`ctx` and `add` only). */
  newNo?: number;
};

export type Hunk = {
  header: string;
  lines: DiffLine[];
  /** First pre-image line number covered by this hunk. */
  oldStart?: number;
  /** First post-image line number covered by this hunk. */
  newStart?: number;
};

const BINARY_RE = /^Binary files .+ differ$/;
const HUNK_RE = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

/**
 * Parse a unified diff into hunks. File headers (`diff --git`, `---`, `+++`,
 * `index`) are skipped. Binary marker lines become a single meta hunk.
 */
export function parseUnifiedDiff(text: string): Hunk[] {
  if (!text || !text.trim()) return [];

  const rawLines = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n');
  // Drop trailing empty split from final newline
  if (rawLines.length > 0 && rawLines[rawLines.length - 1] === '') {
    rawLines.pop();
  }

  const hunks: Hunk[] = [];
  let current: Hunk | null = null;
  let oldNo = 0;
  let newNo = 0;

  for (const line of rawLines) {
    if (BINARY_RE.test(line)) {
      hunks.push({ header: '', lines: [{ kind: 'meta', text: line }] });
      current = null;
      continue;
    }

    if (line.startsWith('@@')) {
      const match = HUNK_RE.exec(line);
      oldNo = match ? Number(match[1]) : 0;
      newNo = match ? Number(match[2]) : 0;
      current = { header: line, lines: [], oldStart: oldNo, newStart: newNo };
      hunks.push(current);
      continue;
    }

    if (!current) {
      // Skip file-level headers until first hunk
      continue;
    }

    if (line.startsWith('\\')) {
      // e.g. "\ No newline at end of file"
      current.lines.push({ kind: 'meta', text: line });
      continue;
    }

    if (line.startsWith('+')) {
      current.lines.push({ kind: 'add', text: line.slice(1), newNo: newNo++ });
      continue;
    }
    if (line.startsWith('-')) {
      current.lines.push({ kind: 'del', text: line.slice(1), oldNo: oldNo++ });
      continue;
    }
    if (line.startsWith(' ') || line === '') {
      current.lines.push({
        kind: 'ctx',
        text: line.startsWith(' ') ? line.slice(1) : line,
        oldNo: oldNo++,
        newNo: newNo++,
      });
      continue;
    }

    // Unknown within hunk → meta
    current.lines.push({ kind: 'meta', text: line });
  }

  return hunks;
}

function renderSection(hunks: Hunk[], mode: 'inline' | 'sideBySide', width: number): string[] {
  if (hunks.length === 0) return [];
  return mode === 'sideBySide' ? renderSideBySide(hunks, width) : renderInline(hunks);
}

/**
 * Build full diff-pane lines for staged + unstaged sections.
 * Empty sections are omitted. Headers: `STAGED` / `UNSTAGED` (no emoji).
 */
export function buildDiffPaneLines(opts: {
  staged: string;
  unstaged: string;
  mode: 'inline' | 'sideBySide';
  width: number;
}): string[] {
  const out: string[] = [];

  const stagedHunks = parseUnifiedDiff(opts.staged);
  if (stagedHunks.length > 0) {
    out.push(tuiSectionHeader('staged'));
    out.push(...renderSection(stagedHunks, opts.mode, opts.width));
  }

  const unstagedHunks = parseUnifiedDiff(opts.unstaged);
  if (unstagedHunks.length > 0) {
    out.push(tuiSectionHeader('unstaged'));
    out.push(...renderSection(unstagedHunks, opts.mode, opts.width));
  }

  return out;
}
