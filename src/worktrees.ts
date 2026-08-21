/**
 * Helpers for `git worktree list --porcelain` parsing, under-cwd path mapping,
 * and merge classification. Path mapping may touch the filesystem (realpath/stat).
 */

import * as fs from 'fs';
import * as path from 'path';
import { DETACHED_HEAD_BRANCH } from './helpers.js';

/**
 * One worktree from `git worktree list --porcelain`.
 */
export interface GitWorktreeListEntry {
  /** Absolute worktree path. */
  path: string;
  /** HEAD commit hash when present. */
  head?: string;
  /** Local branch name (no `refs/heads/` prefix) when checked out on a branch. */
  branch?: string;
  /** True when the worktree is bare. */
  bare: boolean;
  /** True when HEAD is detached. */
  detached: boolean;
}

/**
 * Device + inode identity for bind-mount / hard-link equivalence.
 */
export interface PathIdentity {
  dev: number;
  ino: number;
}

/**
 * Filesystem ops used by `linkedWorktreesUnderCwd` (injectable for tests).
 */
export interface LinkedWorktreePathIo {
  /** Resolve symlinks; return input path when the target is missing. */
  realpath: (absPath: string) => string;
  /** Stat identity, or null when the path does not exist. */
  identity: (absPath: string) => PathIdentity | null;
}

const defaultLinkedWorktreePathIo: LinkedWorktreePathIo = {
  realpath: (absPath) => {
    try {
      return fs.realpathSync(absPath);
    } catch {
      return absPath;
    }
  },
  identity: (absPath) => {
    try {
      const st = fs.statSync(absPath);
      return { dev: st.dev, ino: st.ino };
    } catch {
      return null;
    }
  },
};

function sameIdentity(a: PathIdentity | null, b: PathIdentity | null): boolean {
  return a !== null && b !== null && a.dev === b.dev && a.ino === b.ino;
}

function underDir(abs: string, dir: string): boolean {
  return abs === dir || abs.startsWith(dir + path.sep);
}

function posixRel(from: string, to: string): string {
  return path.relative(from, to).split(path.sep).join('/');
}

/** Join workspace-relative segments, skipping empties (primary may equal cwd). */
function joinWorkspaceRel(...parts: string[]): string {
  return parts.filter((p) => p.length > 0).join('/');
}

/**
 * Remap via primary inode when the registered path is a bind-mount alias.
 * Returns null when the entry is not under that primary identity.
 */
function remapViaPrimaryIdentity(
  abs: string,
  cwd: string,
  primary: string,
  primaryId: PathIdentity,
  io: LinkedWorktreePathIo,
): { absPath: string; relPath: string } | null {
  const primaryRel = posixRel(cwd, primary);
  const suffix: string[] = [];
  let cur = abs;
  for (;;) {
    if (sameIdentity(io.identity(cur), primaryId) || cur === primary) {
      const relPath = joinWorkspaceRel(primaryRel, [...suffix].reverse().join('/'));
      return {
        absPath: relPath ? path.resolve(cwd, ...relPath.split('/')) : path.resolve(cwd),
        relPath,
      };
    }
    const parent = path.dirname(cur);
    if (parent === cur) break;
    suffix.push(path.basename(cur));
    cur = parent;
  }
  return null;
}

/**
 * Map a linked worktree absolute path to a workspace-relative path under `cwd`,
 * or null when it is outside the workspace.
 *
 * Prefer the cwd-visible primary prefix: paths already under that primary use
 * string relative after realpath; bind-mount aliases (same inode, different
 * absolute prefix) remapped via inode walk even when the alias also sits under
 * cwd. Other under-cwd paths keep a plain relative mapping.
 */
export function mapLinkedWorktreeRelPath(
  entryAbs: string,
  cwdAbs: string,
  primaryAbs: string,
  io: LinkedWorktreePathIo = defaultLinkedWorktreePathIo,
): { absPath: string; relPath: string } | null {
  const cwd = io.realpath(path.resolve(cwdAbs));
  const primary = io.realpath(path.resolve(primaryAbs));
  const abs = io.realpath(path.resolve(entryAbs));
  const primaryId = io.identity(primary);

  if (sameIdentity(io.identity(abs), primaryId) || abs === primary) {
    return null;
  }

  if (!underDir(primary, cwd)) {
    return null;
  }

  // Already under the cwd-visible primary — no remapping needed.
  if (underDir(abs, primary)) {
    return {
      absPath: abs,
      relPath: posixRel(cwd, abs),
    };
  }

  // Bind-mount alias of a path under this primary (may still be under cwd).
  if (primaryId) {
    const remapped = remapViaPrimaryIdentity(abs, cwd, primary, primaryId, io);
    if (remapped) return remapped;
  }

  if (underDir(abs, cwd)) {
    return {
      absPath: abs,
      relPath: posixRel(cwd, abs),
    };
  }

  return null;
}

/**
 * Listed worktree path git will accept: exact string, else same inode.
 */
function listedWorktreePath(
  entries: GitWorktreeListEntry[],
  worktreeAbs: string,
  io: LinkedWorktreePathIo,
): string {
  const wt = path.resolve(worktreeAbs);
  for (const entry of entries) {
    if (entry.bare) continue;
    if (path.resolve(entry.path) === wt) return wt;
  }
  const wtId = io.identity(wt);
  if (!wtId) return wt;
  for (const entry of entries) {
    if (entry.bare) continue;
    if (sameIdentity(io.identity(path.resolve(entry.path)), wtId)) {
      return path.resolve(entry.path);
    }
  }
  return wt;
}

/**
 * Ancestor of `abs` with the same inode as `primaryAbs` (bind-mount alias
 * of the primary). Null when no ancestor matches.
 */
function registeredPrimaryAbs(
  abs: string,
  primaryAbs: string,
  io: LinkedWorktreePathIo,
): string | null {
  const primaryId = io.identity(path.resolve(primaryAbs));
  if (!primaryId) return null;
  let cur = path.resolve(abs);
  for (;;) {
    if (sameIdentity(io.identity(cur), primaryId)) return cur;
    const parent = path.dirname(cur);
    if (parent === cur) return null;
    cur = parent;
  }
}

/**
 * Paths git will accept for `worktree remove` when the TUI path is a
 * bind-mount alias of the registered worktree.
 *
 * `gitPath` is the porcelain `worktree` line (inode match when prefixes
 * differ). `gitCwd` is the registered primary prefix — the ancestor of
 * `gitPath` with the same inode as `primaryAbs` — so gitdir back-pointers
 * compare equal. Callers fall back to the TUI paths when nothing matches.
 */
export function resolveWorktreeRemoveTarget(
  entries: GitWorktreeListEntry[],
  primaryAbs: string,
  worktreeAbs: string,
  io: LinkedWorktreePathIo = defaultLinkedWorktreePathIo,
): { gitCwd: string; gitPath: string } {
  const gitPath = listedWorktreePath(entries, worktreeAbs, io);
  const gitCwd = registeredPrimaryAbs(gitPath, primaryAbs, io) ?? path.resolve(primaryAbs);
  return { gitCwd, gitPath };
}

/**
 * Parse `git worktree list --porcelain` stdout into entries.
 *
 * Strips `refs/heads/` from branch lines. Paths are normalized with `path.resolve`.
 */
export function parseWorktreeListPorcelain(text: string): GitWorktreeListEntry[] {
  const entries: GitWorktreeListEntry[] = [];
  let current: GitWorktreeListEntry | null = null;

  const flush = () => {
    if (current) {
      entries.push(current);
      current = null;
    }
  };

  for (const rawLine of text.split('\n')) {
    const line = rawLine.trimEnd();
    if (!line) {
      flush();
      continue;
    }
    if (line.startsWith('worktree ')) {
      flush();
      current = {
        path: path.resolve(line.slice('worktree '.length)),
        bare: false,
        detached: false,
      };
      continue;
    }
    if (!current) continue;
    if (line.startsWith('HEAD ')) {
      current.head = line.slice('HEAD '.length);
      continue;
    }
    if (line.startsWith('branch ')) {
      const ref = line.slice('branch '.length);
      current.branch = ref.startsWith('refs/heads/') ? ref.slice('refs/heads/'.length) : ref;
      continue;
    }
    if (line === 'bare') {
      current.bare = true;
      continue;
    }
    if (line === 'detached') {
      current.detached = true;
    }
  }
  flush();
  return entries;
}

/**
 * Linked (non-primary, non-bare) worktrees whose paths fall under `cwdAbs`.
 *
 * Uses realpath for symlink layouts. Bind-mount aliases of the primary (same
 * inode, different absolute prefix) are remapped to the cwd-visible relative path.
 *
 * `relPath` is relative to `cwdAbs` with `/` separators.
 */
export function linkedWorktreesUnderCwd(
  entries: GitWorktreeListEntry[],
  cwdAbs: string,
  primaryAbs: string,
  io: LinkedWorktreePathIo = defaultLinkedWorktreePathIo,
): { absPath: string; relPath: string }[] {
  const out: { absPath: string; relPath: string }[] = [];
  for (const entry of entries) {
    if (entry.bare) continue;
    const mapped = mapLinkedWorktreeRelPath(entry.path, cwdAbs, primaryAbs, io);
    if (!mapped) continue;
    out.push(mapped);
  }
  return out;
}

/**
 * Whether a branch tip is merged into the default branch, or unknown.
 *
 * - `null` when on the default branch, or when ancestry is unknown
 * - otherwise the `isAncestorOfDefault` boolean
 */
export type MergedIntoDefault = boolean | null;

/**
 * Classify merge-into-default from branch name + ancestry probe result.
 */
export function classifyMergedIntoDefault(args: {
  branch: string;
  defaultBranch: string;
  isAncestorOfDefault: boolean | null;
}): MergedIntoDefault {
  if (args.branch === args.defaultBranch) return null;
  // Detached HEAD is not a branch tip relative to default — omit merge marks.
  if (args.branch === DETACHED_HEAD_BRANCH) return null;
  if (args.isAncestorOfDefault === null) return null;
  return args.isAncestorOfDefault;
}
