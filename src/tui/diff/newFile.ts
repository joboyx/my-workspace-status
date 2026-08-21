/**
 * Synthesize unified diffs for untracked / new files (no git diff output).
 */

import * as fs from 'node:fs/promises';
import * as path from 'node:path';

/** Design: stub binary / huge files above ~1 MB. */
export const HUGE_FILE_BYTES = 1_000_000;

/**
 * Build a unified all-add diff from file text (no file headers).
 */
export function synthesizeAllAddDiff(text: string): string {
  const raw = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n');
  if (raw.length > 0 && raw[raw.length - 1] === '') raw.pop();

  if (raw.length === 0) {
    return '@@ -0,0 +0,0 @@\n';
  }

  const body = raw.map((line) => `+${line}`).join('\n');
  return `@@ -0,0 +1,${raw.length} @@\n${body}\n`;
}

function binaryStub(relPath: string): string {
  return `Binary files /dev/null and b/${relPath} differ\n`;
}

/**
 * Read an untracked worktree file as a unified diff body.
 * Huge / binary → one-line stub that `parseUnifiedDiff` understands.
 */
export async function readUntrackedAsDiff(
  absPath: string,
  relPath: string,
): Promise<string> {
  try {
    const st = await fs.stat(absPath);
    if (!st.isFile()) return '';
    if (st.size > HUGE_FILE_BYTES) return binaryStub(relPath);

    const buf = await fs.readFile(absPath);
    if (buf.includes(0)) return binaryStub(relPath);

    return synthesizeAllAddDiff(buf.toString('utf8'));
  } catch {
    return '';
  }
}

/**
 * Cache invalidation key from worktree mtime (and size as practical).
 */
export async function fileMtimeKey(absPath: string): Promise<string> {
  try {
    const st = await fs.stat(absPath);
    return `${st.size}:${st.mtimeMs}`;
  } catch {
    return 'missing';
  }
}

/** Absolute path for a repo-relative file. */
export function repoFileAbs(cwd: string, repoPath: string, filePath: string): string {
  return path.join(cwd, repoPath, filePath);
}
