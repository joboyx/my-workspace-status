/**
 * Change detection for the live-refresh poll.
 *
 * Polling beats `fs.watch` here: the workspace holds 30+ repos, and inotify on
 * a WSL2 mount is both limited and unreliable. One interval tick re-runs the
 * same snapshot collection the `r` key uses, then this module works out which
 * rows actually moved so the tree can flash only those.
 *
 * File signatures combine the git status letter with the worktree mtime, so
 * both `M → MS` (staged elsewhere) and an in-place edit of an already-modified
 * file register as changes. Chrome signatures cover repo / checkout / dir /
 * workspace / group rows (branch, sync, child set) without walking file mtimes.
 */

import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import { fileChangesFromSnapshot } from '../changes.js';
import { mapWithConcurrency } from '../concurrency.js';
import { statusLetterFromChange } from './icons.js';
import type { RepoSnapshot } from '../types.js';
import type { TreeNode, VisibleRow, WorkspaceNode } from './model/types.js';

/** Default poll period. Override with `WS_STATUS_WATCH_MS`. */
export const DEFAULT_WATCH_MS = 3000;

/** Lower bound — anything faster spends more time in git than in the UI. */
export const MIN_WATCH_MS = 500;

/** How long a changed row stays highlighted. Spec B3: ~800ms. */
export const FLASH_MS = 800;

const STAT_CONCURRENCY = 16;

/**
 * Poll period from the environment, clamped to something sane.
 * Returns 0 when watching is disabled (`WS_STATUS_WATCH_MS=0`).
 */
export function watchIntervalMs(
  env: NodeJS.ProcessEnv = process.env,
): number {
  const raw = env.WS_STATUS_WATCH_MS;
  if (raw === undefined || raw === '') return DEFAULT_WATCH_MS;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || parsed < 0) return DEFAULT_WATCH_MS;
  if (parsed === 0) return 0;
  return Math.max(MIN_WATCH_MS, parsed);
}

/** Tree node id for a changed file — must match `makeFileNode` in model/tree. */
export function fileNodeId(repo: string, filePath: string): string {
  return `file:${repo}:${filePath}`;
}

/** Tree node id for a repo — must match `makeRepoNode` in model/tree. */
export function repoNodeId(repo: string): string {
  return `repo:${repo}`;
}

/** Map of tree node id → change signature. */
export type ChangeSignatures = Map<string, string>;

/**
 * Signature every changed file across all snapshots.
 * Missing files (deleted from disk) still get a stable signature.
 */
export async function changeSignatures(
  cwd: string,
  snapshots: RepoSnapshot[],
): Promise<ChangeSignatures> {
  const entries: { id: string; abs: string; status: string }[] = [];
  for (const snapshot of snapshots) {
    for (const change of fileChangesFromSnapshot(snapshot)) {
      entries.push({
        id: fileNodeId(snapshot.repo, change.path),
        abs: path.join(cwd, snapshot.repo, change.path),
        status: statusLetterFromChange(change),
      });
    }
  }

  const signatures: ChangeSignatures = new Map();
  await mapWithConcurrency(entries, STAT_CONCURRENCY, async (entry) => {
    let mtime = 'gone';
    try {
      const st = await fs.stat(entry.abs);
      mtime = `${st.size}:${st.mtimeMs}`;
    } catch {
      // Deleted from the worktree — status alone identifies the row.
    }
    signatures.set(entry.id, `${entry.status}:${mtime}`);
  });
  return signatures;
}

function chromeSignature(node: TreeNode): string | null {
  switch (node.kind) {
    case 'file':
      return null;
    case 'workspace':
      return `${node.changeCount}|${node.syncSummary}`;
    case 'repo':
    case 'checkout':
      return [
        node.branch,
        node.sync,
        node.syncStatus,
        node.changeCount,
        node.mergedIntoDefault,
        node.checkoutKind,
        node.children.length,
      ].join('|');
    case 'dir':
    case 'group':
      return node.children.map((c) => c.id).sort().join(',');
  }
}

/**
 * Semantic signatures for non-file tree rows (workspace, repo, checkout, dir,
 * group). File ids stay on `changeSignatures` (status + mtime).
 */
export function treeChromeSignatures(root: WorkspaceNode): ChangeSignatures {
  const signatures: ChangeSignatures = new Map();
  const walk = (node: TreeNode): void => {
    if (node.kind !== 'file') {
      const signature = chromeSignature(node);
      if (signature !== null) signatures.set(node.id, signature);
      for (const child of node.children) walk(child);
    }
  };
  walk(root);
  return signatures;
}

/**
 * Union two signature maps. Entries in `b` overwrite the same id in `a`.
 */
export function mergeSignatures(
  a: ChangeSignatures,
  b: ChangeSignatures,
): ChangeSignatures {
  const merged: ChangeSignatures = new Map(a);
  for (const [id, signature] of b) merged.set(id, signature);
  return merged;
}

/**
 * Node ids that appeared or changed between two signature maps.
 * Removals are handled separately via `removedNodeIds` / ghost rows.
 */
export function changedNodeIds(
  before: ChangeSignatures,
  after: ChangeSignatures,
): string[] {
  const changed: string[] = [];
  for (const [id, signature] of after) {
    if (before.get(id) !== signature) changed.push(id);
  }
  return changed;
}

/**
 * Node ids present in `before` but absent from `after`.
 */
export function removedNodeIds(
  before: ChangeSignatures,
  after: ChangeSignatures,
): string[] {
  const removed: string[] = [];
  for (const id of before.keys()) {
    if (!after.has(id)) removed.push(id);
  }
  return removed;
}

/**
 * Ids that should flash: appeared, altered, or removed.
 * Pass `{ includeAdds: false }` to skip brand-new ids (graph autoload).
 */
export function flashableNodeIds(
  before: ChangeSignatures,
  after: ChangeSignatures,
  opts?: { includeAdds?: boolean },
): string[] {
  const includeAdds = opts?.includeAdds !== false;
  const changed = includeAdds
    ? changedNodeIds(before, after)
    : changedNodeIds(before, after).filter((id) => before.has(id));
  return [...new Set([...changed, ...removedNodeIds(before, after)])];
}

/**
 * A list row kept visible briefly after it disappears.
 * Tree ghosts default to `VisibleRow`; graph ghosts use `GraphListRow`.
 */
export interface GhostRow<T extends { id: string } = VisibleRow> {
  id: string;
  row: T;
  flashedAt: number;
  /** Index in the visible list when the row was last live — merge inserts here. */
  index: number;
}

/** Drop ghosts whose flash window has elapsed. */
export function pruneGhosts<T extends { id: string }>(
  ghosts: GhostRow<T>[],
  now: number,
  durationMs: number = FLASH_MS,
): GhostRow<T>[] {
  return ghosts.filter((g) => now - g.flashedAt < durationMs);
}

/**
 * Build ghosts for ids that left the signature map, capturing each row at its
 * last live index so merge can flash in place (not off-screen at the list end).
 */
export function removalGhosts<T extends { id: string }>(
  prevRows: readonly T[],
  removedIds: readonly string[],
  now: number,
): GhostRow<T>[] {
  const indexById = new Map(prevRows.map((r, i) => [r.id, i]));
  const rowById = new Map(prevRows.map((r) => [r.id, r]));
  const ghosts: GhostRow<T>[] = [];
  for (const id of removedIds) {
    const row = rowById.get(id);
    const index = indexById.get(id);
    if (row === undefined || index === undefined) continue;
    ghosts.push({ id, row, flashedAt: now, index });
  }
  return ghosts;
}

/**
 * Re-insert still-live ghosts at their original indices so the flash stays in
 * the viewport where the row disappeared (appending hid removals off-screen).
 */
export function mergeGhostRows<T extends { id: string }>(
  live: T[],
  ghosts: GhostRow<T>[],
  now: number,
  durationMs: number = FLASH_MS,
): T[] {
  const liveIds = new Set(live.map((r) => r.id));
  const active = pruneGhosts(ghosts, now, durationMs)
    .filter((g) => !liveIds.has(g.id))
    .slice()
    .sort((a, b) => a.index - b.index);
  if (active.length === 0) return live;
  const result = [...live];
  for (const g of active) {
    result.splice(Math.min(Math.max(0, g.index), result.length), 0, g.row);
  }
  return result;
}

/**
 * Flash opacity for a row: 1 immediately after a change, easing to 0 at
 * `FLASH_MS`. Rows with no recorded flash return 0.
 */
export function flashStrength(
  flashedAt: number | undefined,
  now: number,
  durationMs: number = FLASH_MS,
): number {
  if (flashedAt === undefined) return 0;
  const elapsed = now - flashedAt;
  if (elapsed < 0 || elapsed >= durationMs) return 0;
  return 1 - elapsed / durationMs;
}

/** Drop expired entries so the flash map cannot grow without bound. */
export function pruneFlashes(
  flashes: Map<string, number>,
  now: number,
  durationMs: number = FLASH_MS,
): Map<string, number> {
  const next = new Map<string, number>();
  for (const [id, at] of flashes) {
    if (now - at < durationMs) next.set(id, at);
  }
  return next;
}
