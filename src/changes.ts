/**
 * Shared FileChange helpers for plain-text render and TUI.
 */

import type { FileChange, RepoSnapshot } from './types.js';
import { SEP, sanitizePath, splitEntries, trimVal } from './helpers.js';

export type { FileChange } from './types.js';

function parseTrackedChange(
  entry: string,
): { status: string; path: string; oldPath?: string } | null {
  const parts = entry.split('\t').map(trimVal);
  const status = parts[0] ?? '';
  if (!status) return null;
  if (status === 'R') {
    const oldPath = sanitizePath(parts[1] ?? '');
    const path = sanitizePath(parts[2] ?? '');
    return path ? { status, path, oldPath } : null;
  }
  const path = sanitizePath(parts[1] ?? '');
  return path ? { status, path } : null;
}

function upsertChange(changes: Map<string, FileChange>, path: string): FileChange {
  const existing = changes.get(path);
  if (existing) return existing;
  const created: FileChange = { path };
  changes.set(path, created);
  return created;
}

function formatTrackedEntry(status: string, path: string, oldPath?: string): string {
  if (status === 'R') {
    return `R\t${oldPath ?? ''}\t${path}`;
  }
  return `${status}\t${path}`;
}

/** Merge staged/unstaged/untracked snapshot fields into per-path FileChange rows. */
export function fileChangesFromSnapshot(snapshot: RepoSnapshot): FileChange[] {
  const changes = new Map<string, FileChange>();

  for (const entry of splitEntries(snapshot.stagedFiles)) {
    const parsed = parseTrackedChange(entry);
    if (!parsed) continue;
    const change = upsertChange(changes, parsed.path);
    change.stagedStatus = parsed.status;
    change.oldPath = parsed.oldPath;
  }

  for (const entry of splitEntries(snapshot.unstagedFiles)) {
    const parsed = parseTrackedChange(entry);
    if (!parsed) continue;
    const change = upsertChange(changes, parsed.path);
    change.unstagedStatus = parsed.status;
    change.oldPath ??= parsed.oldPath;
  }

  for (const entry of splitEntries(snapshot.untrackedFiles)) {
    const path = sanitizePath(trimVal(entry));
    if (!path) continue;
    upsertChange(changes, path).untracked = true;
  }

  return [...changes.values()].sort((a, b) => a.path.localeCompare(b.path));
}

/** Badge string for a single FileChange (emoji + letter code). */
export function badgeForChange(change: FileChange): string {
  // Conflict before MS — staged+unstaged both set must not swallow U.
  if (change.unstagedStatus === 'U' || change.stagedStatus === 'U') return '⚠️U';
  if (change.stagedStatus && change.unstagedStatus) return '🟠MS';
  const status = change.unstagedStatus ?? change.stagedStatus;
  if (status === 'R') return '🟣R';
  if (status === 'D') return '🔴D';
  if (change.untracked || status === 'A') return '🟢A';
  if (change.stagedStatus) return '🔵S';
  return '🟡M';
}

/**
 * Rebuild discovery-format snapshot file fields from FileChange rows
 * (`STATUS\\tpath` / `R\\told\\tnew`, joined with `|||`).
 */
export function fileChangesToSnapshotFields(
  changes: FileChange[],
): Pick<
  RepoSnapshot,
  'stagedFiles' | 'unstagedFiles' | 'untrackedFiles' | 'hasStaged' | 'hasUnstaged' | 'hasUntracked'
> {
  const stagedEntries: string[] = [];
  const unstagedEntries: string[] = [];
  const untrackedEntries: string[] = [];

  for (const change of changes) {
    if (change.stagedStatus) {
      stagedEntries.push(
        formatTrackedEntry(change.stagedStatus, change.path, change.oldPath),
      );
    }
    if (change.unstagedStatus) {
      unstagedEntries.push(
        formatTrackedEntry(change.unstagedStatus, change.path, change.oldPath),
      );
    }
    if (change.untracked) {
      untrackedEntries.push(change.path);
    }
  }

  return {
    stagedFiles: stagedEntries.join(SEP),
    unstagedFiles: unstagedEntries.join(SEP),
    untrackedFiles: untrackedEntries.join(SEP),
    hasStaged: stagedEntries.length > 0,
    hasUnstaged: unstagedEntries.length > 0,
    hasUntracked: untrackedEntries.length > 0,
  };
}
