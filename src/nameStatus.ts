import type { FileChange } from './types.js';

function statusLetter(raw: string): string {
  const letter = raw.charAt(0);
  return letter || 'M';
}

/**
 * Single-sided name-status → FileChange.
 *
 * Uses `unstagedStatus` as the status carrier so `statusLetterFromChange`
 * maps A/M/D/R/C to the matching badge (not `S`). Workspace staged-only
 * rows still use `stagedStatus` from porcelain elsewhere.
 */
function changeFromNameStatus(
  path: string,
  status: string,
  oldPath?: string,
): FileChange {
  const letter = statusLetter(status);
  return oldPath
    ? { path, oldPath, unstagedStatus: letter }
    : { path, unstagedStatus: letter };
}

/**
 * Parse newline `git diff --name-status` / `show --name-status` output.
 */
export function parseNameStatusLines(stdout: string): FileChange[] {
  const out: FileChange[] = [];
  for (const line of stdout.split('\n')) {
    if (!line.trim()) continue;
    const parts = line.split('\t');
    const status = parts[0] ?? '';
    if (status.startsWith('R') || status.startsWith('C')) {
      const oldPath = parts[1];
      const path = parts[2];
      if (!path) continue;
      out.push(changeFromNameStatus(path, status, oldPath));
      continue;
    }
    const path = parts[1];
    if (!path) continue;
    out.push(changeFromNameStatus(path, status));
  }
  return out;
}

/**
 * Parse `-z` name-status: status\\0path\\0 or R###\\0old\\0new\\0 …
 */
export function parseNameStatusZ(stdout: string): FileChange[] {
  const parts = stdout.split('\0').filter((p, i, arr) => !(p === '' && i === arr.length - 1));
  const out: FileChange[] = [];
  for (let i = 0; i < parts.length; ) {
    const status = parts[i++] ?? '';
    if (!status) break;
    if (status.startsWith('R') || status.startsWith('C')) {
      const oldPath = parts[i++] ?? '';
      const path = parts[i++] ?? '';
      if (path) out.push(changeFromNameStatus(path, status, oldPath));
      continue;
    }
    const path = parts[i++] ?? '';
    if (path) out.push(changeFromNameStatus(path, status));
  }
  return out;
}
