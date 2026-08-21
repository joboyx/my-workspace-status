/**
 * Persist GitLens-style "viewed" marks for dirty workspace-tree files.
 *
 * Identity is repo path + file path. The fingerprint is a hash of the current
 * staged/unstaged/untracked status plus worktree bytes. A mark stays until
 * that fingerprint changes, or the operator toggles space again.
 *
 * Store: `$XDG_STATE_HOME/my-workspace-status/viewed-files.json`
 * (override with `WS_STATUS_VIEWED_STORE`).
 */

import { createHash } from 'node:crypto';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { HUGE_FILE_BYTES, repoFileAbs } from './diff/newFile.js';
import { ICON_VIEWED, viewedColor } from './icons.js';
import type { FileNode, TreeNode, VisibleRow } from './model/types.js';
import type { Segment } from './theme.js';

/** On-disk store version. Unknown versions load as empty. */
export const VIEWED_STORE_VERSION = 1;

/** One persisted viewed mark. */
export interface ViewedEntry {
  readonly fingerprint: string;
}

/** identity → fingerprint captured at mark time. */
export type ViewedStore = Record<string, ViewedEntry>;

/** Inputs hashed into a viewed fingerprint. */
export interface ViewedFingerprintInput {
  readonly stagedStatus?: string;
  readonly unstagedStatus?: string;
  readonly untracked?: boolean;
  readonly oldPath?: string;
  readonly content: string | Buffer;
}

/**
 * Normalize a repo or file path for identity keys (posix, no `./`).
 */
export function normalizeViewedPath(value: string): string {
  return value.replace(/\\/g, '/').replace(/\/+/g, '/').replace(/^\.\//, '').replace(/\/+$/, '');
}

/**
 * Stable identity: workspace-relative repo path + repo-relative file path.
 */
export function viewedIdentity(repoPath: string, filePath: string): string {
  return `${normalizeViewedPath(repoPath)}\0${normalizeViewedPath(filePath)}`;
}

/**
 * Identity for a workspace-tree file node.
 */
export function fileNodeIdentity(file: FileNode): string {
  return viewedIdentity(file.repoPath, file.path);
}

/**
 * SHA-256 of status letters plus worktree (or supplied) content.
 * Stage/unstage that changes the status token is a new fingerprint.
 */
export function viewedFingerprint(input: ViewedFingerprintInput): string {
  const status = [
    input.stagedStatus ?? '',
    input.unstagedStatus ?? '',
    input.untracked ? '1' : '0',
    input.oldPath ?? '',
  ].join('\0');
  const hash = createHash('sha256');
  hash.update(status);
  hash.update('\n');
  hash.update(input.content);
  return hash.digest('hex');
}

/**
 * True when `store` has `identity` with this exact fingerprint.
 */
export function isViewed(store: ViewedStore, identity: string, fingerprint: string): boolean {
  return store[identity]?.fingerprint === fingerprint;
}

/**
 * Toggle a mark. Same identity + fingerprint unmarks; otherwise marks.
 */
export function toggleViewed(
  store: ViewedStore,
  identity: string,
  fingerprint: string,
): ViewedStore {
  if (isViewed(store, identity, fingerprint)) {
    const next = { ...store };
    delete next[identity];
    return next;
  }
  return { ...store, [identity]: { fingerprint } };
}

/**
 * Drop marks whose file is gone or whose fingerprint no longer matches.
 * Returns `store` when nothing changed.
 */
export function reconcileViewed(
  store: ViewedStore,
  current: ReadonlyMap<string, string>,
): ViewedStore {
  let changed = false;
  const next: ViewedStore = {};
  for (const [identity, entry] of Object.entries(store)) {
    const fingerprint = current.get(identity);
    if (fingerprint !== undefined && fingerprint === entry.fingerprint) {
      next[identity] = entry;
    } else {
      changed = true;
    }
  }
  return changed ? next : store;
}

/**
 * Default JSON path. `WS_STATUS_VIEWED_STORE` wins for tests.
 */
export function viewedStorePath(env: NodeJS.ProcessEnv = process.env): string {
  const override = env.WS_STATUS_VIEWED_STORE?.trim();
  if (override) return override;
  const home = env.HOME || os.homedir();
  const stateHome = env.XDG_STATE_HOME?.trim() || path.join(home, '.local', 'state');
  return path.join(stateHome, 'my-workspace-status', 'viewed-files.json');
}

/**
 * Load a viewed store. Missing or malformed files become `{}`.
 */
export function loadViewedStore(filePath: string = viewedStorePath()): ViewedStore {
  try {
    const parsed = JSON.parse(fs.readFileSync(filePath, 'utf8')) as {
      version?: number;
      entries?: unknown;
    };
    if (
      parsed.version !== VIEWED_STORE_VERSION ||
      !parsed.entries ||
      typeof parsed.entries !== 'object'
    ) {
      return {};
    }
    const out: ViewedStore = {};
    for (const [key, value] of Object.entries(parsed.entries as Record<string, unknown>)) {
      if (!value || typeof value !== 'object') continue;
      const fingerprint = (value as { fingerprint?: unknown }).fingerprint;
      if (typeof fingerprint === 'string' && fingerprint.length > 0) {
        out[key] = { fingerprint };
      }
    }
    return out;
  } catch {
    return {};
  }
}

/**
 * Persist `store` as versioned JSON. Best-effort: disk errors must not crash the TUI.
 */
export function saveViewedStore(store: ViewedStore, filePath: string = viewedStorePath()): void {
  try {
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    const body = `${JSON.stringify({ version: VIEWED_STORE_VERSION, entries: store }, null, 2)}\n`;
    fs.writeFileSync(filePath, body, 'utf8');
  } catch {
    /* ignore */
  }
}

/**
 * Depth-first file nodes (includes folded children).
 */
export function collectFileNodes(node: TreeNode): FileNode[] {
  if (node.kind === 'file') return [node];
  if (
    node.kind === 'workspace' ||
    node.kind === 'repo' ||
    node.kind === 'checkout' ||
    node.kind === 'group' ||
    node.kind === 'dir'
  ) {
    const out: FileNode[] = [];
    for (const child of node.children) out.push(...collectFileNodes(child));
    return out;
  }
  return [];
}

/**
 * Fingerprint a live file node: status token + worktree bytes (or `missing`).
 * Files above {@link HUGE_FILE_BYTES} hash size only so a 3s poll stays cheap.
 */
export function fingerprintFileNode(cwd: string, file: FileNode): string {
  const abs = repoFileAbs(cwd, file.repoPath, file.path);
  let content: string | Buffer = 'missing';
  try {
    const buf = fs.readFileSync(abs);
    content = buf.length > HUGE_FILE_BYTES ? `huge:${buf.length}` : buf;
  } catch {
    content = 'missing';
  }
  return viewedFingerprint({
    stagedStatus: file.change.stagedStatus,
    unstagedStatus: file.change.unstagedStatus,
    untracked: file.untracked,
    oldPath: file.renameFrom ?? file.change.oldPath,
    content,
  });
}

/**
 * Current identity → fingerprint map. When `onlyIdentities` is set, skip
 * unmarked files (paint / reconcile only need marks that already exist).
 */
export function collectCurrentFingerprints(
  files: readonly FileNode[],
  cwd: string,
  onlyIdentities?: ReadonlySet<string>,
): Map<string, string> {
  const out = new Map<string, string>();
  for (const file of files) {
    const identity = fileNodeIdentity(file);
    if (onlyIdentities && !onlyIdentities.has(identity)) continue;
    out.set(identity, fingerprintFileNode(cwd, file));
  }
  return out;
}

/**
 * Workspace-tree file row ids that are currently viewed (fingerprint still matches).
 */
export function viewedRowIds(
  files: readonly FileNode[],
  store: ViewedStore,
  cwd: string,
): Set<string> {
  const marked = new Set(Object.keys(store));
  if (marked.size === 0) return new Set();
  const current = collectCurrentFingerprints(files, cwd, marked);
  const ids = new Set<string>();
  for (const file of files) {
    const identity = fileNodeIdentity(file);
    const fingerprint = current.get(identity);
    if (fingerprint && isViewed(store, identity, fingerprint)) ids.add(file.id);
  }
  return ids;
}

/**
 * Trailing eye on viewed file rows. Other row kinds are unchanged.
 * Cyan/blue accent — not the muted clean check.
 */
export function applyViewedMarks(rows: VisibleRow[], viewedIds: ReadonlySet<string>): VisibleRow[] {
  if (viewedIds.size === 0) return rows;
  const color = viewedColor();
  return rows.map((row) => {
    if (row.node.kind !== 'file' || !viewedIds.has(row.id)) return row;
    const mark: Segment = { text: ICON_VIEWED, color };
    const gap: Segment = { text: ' ' };
    return { ...row, trailing: [mark, gap, ...row.trailing] };
  });
}
