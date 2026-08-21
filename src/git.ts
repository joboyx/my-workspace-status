/**
 * Git CLI invocation helpers (async via zx `$`, run under Node — not the zx CLI).
 */

import * as fs from 'fs';
import { $ } from 'zx';
import { parseNameStatusZ } from './nameStatus.js';
import { parseWorktreeListPorcelain, resolveWorktreeRemoveTarget } from './worktrees.js';
import type { FileChange } from './types.js';
import type { GraphRef, GraphStash } from './tui/graph/types.js';

export const GIT_BINARY =
  process.env.WORKSPACE_STATUS_GIT ?? (fs.existsSync('/usr/bin/git') ? '/usr/bin/git' : 'git');

const GIT_PREFIX = `${GIT_BINARY} `;

type GitRunOptions = {
  nothrow?: boolean;
  quiet?: boolean;
};

function git$(cwd: string, options: GitRunOptions = {}) {
  return $({
    cwd,
    prefix: GIT_PREFIX,
    quiet: options.quiet ?? true,
    nothrow: options.nothrow ?? true,
  });
}

/** Run git and return stdout (empty string on failure). */
export async function execGit(args: string[], cwd: string): Promise<string> {
  try {
    const result = await git$(cwd)`${args}`;
    return result.stdout.trim();
  } catch {
    return '';
  }
}

/** Returns exit code (0 = success). */
export async function execGitStatus(args: string[], cwd: string): Promise<number> {
  try {
    const result = await git$(cwd)`${args}`;
    return result.exitCode ?? -1;
  } catch {
    return -1;
  }
}

/** True when the worktree or index has changes (git diff / diff --cached). */
export async function repoHasLocalChanges(cwd: string): Promise<boolean> {
  return (
    (await execGitStatus(['diff', '--quiet'], cwd)) !== 0 ||
    (await execGitStatus(['diff', '--cached', '--quiet'], cwd)) !== 0
  );
}

/**
 * Resolve `ref` to a commit SHA via `rev-parse --verify --quiet`.
 * Missing or invalid refs return `null`.
 */
export async function revParseQuiet(ref: string, cwd: string): Promise<string | null> {
  const sha = await execGit(['rev-parse', '--verify', '--quiet', ref], cwd);
  return sha.length > 0 ? sha : null;
}

/** Checkout an existing branch, or create it tracking origin/<branch>. */
export async function checkoutBranch(branch: string, cwd: string): Promise<boolean> {
  if ((await execGitStatus(['checkout', branch, '--quiet'], cwd)) === 0) return true;
  return (
    (await execGitStatus(['checkout', '-b', branch, `origin/${branch}`, '--quiet'], cwd)) === 0
  );
}

/**
 * Fast-forward HEAD to an already-fetched remote-tracking ref (no fetch, no reset).
 *
 * Accepts `origin/foo` or `refs/remotes/origin/foo`. Uses `git merge --ff-only`
 * so an ahead or diverged local tip is left unchanged (no merge commit).
 *
 * @returns true when HEAD now matches the remote-tracking tip.
 */
export async function fastForwardToRemoteRef(remoteRef: string, cwd: string): Promise<boolean> {
  const ref = remoteRef.startsWith('refs/') ? remoteRef : `refs/remotes/${remoteRef}`;
  const targetSha = await revParseQuiet(ref, cwd);
  if (!targetSha) return false;
  if ((await execGitStatus(['merge', '--ff-only', '--quiet', ref], cwd)) !== 0) return false;
  const headSha = await revParseQuiet('HEAD', cwd);
  return headSha === targetSha;
}

/**
 * Outcome of `pullQuietDetailed`: pull plus optional auto-stash around it.
 */
export type PullQuietResult = {
  /** True when pull succeeded and any auto-stash was reapplied cleanly. */
  ok: boolean;
  /** True when local changes were stashed before pull. */
  stashed: boolean;
  /** True when stash pop failed after pull (conflicts left; stash kept). */
  stashPopFailed: boolean;
};

const AUTO_STASH_MESSAGE = 'ws-status: auto-stash before pull';

/**
 * `git pull --quiet`, stashing tracked local changes first when needed.
 *
 * When the worktree/index is dirty: `stash push` → `pull` → `stash pop`.
 * Pop always runs after pull (even if pull failed) so local work is restored.
 */
export async function pullQuietDetailed(cwd: string): Promise<PullQuietResult> {
  const dirty = await repoHasLocalChanges(cwd);
  let stashed = false;
  if (dirty) {
    const stashCode = await execGitStatus(
      ['stash', 'push', '-m', AUTO_STASH_MESSAGE, '--quiet'],
      cwd,
    );
    if (stashCode !== 0) {
      return { ok: false, stashed: false, stashPopFailed: false };
    }
    stashed = true;
  }

  const pullOk = (await execGitStatus(['pull', '--quiet'], cwd)) === 0;

  let stashPopFailed = false;
  if (stashed) {
    const popCode = await execGitStatus(['stash', 'pop', '--quiet'], cwd);
    if (popCode !== 0) stashPopFailed = true;
  }

  return {
    ok: pullOk && !stashPopFailed,
    stashed,
    stashPopFailed,
  };
}

/** Run git pull --quiet (auto-stash when dirty); returns whether the op fully succeeded. */
export async function pullQuiet(cwd: string): Promise<boolean> {
  return (await pullQuietDetailed(cwd)).ok;
}

/**
 * True when `P` should publish with `-u` (no upstream, or upstream branch name
 * differs from the current branch — e.g. feature tracking `origin/develop`).
 */
export async function needsUpstreamPublish(cwd: string): Promise<boolean> {
  const branch = await execGit(['branch', '--show-current'], cwd);
  if (!branch) return false;

  const upstream = await execGit(['rev-parse', '--abbrev-ref', '@{upstream}'], cwd);
  if (!upstream) return true;

  const remote = (await execGit(['config', '--get', `branch.${branch}.remote`], cwd)) || 'origin';
  const prefix = `${remote}/`;
  if (!upstream.startsWith(prefix)) return true;
  return upstream.slice(prefix.length) !== branch;
}

/** Prefer configured upstream remote, else first remote, else `origin`. */
async function pushRemoteName(cwd: string, branch: string): Promise<string> {
  const configured = await execGit(['config', '--get', `branch.${branch}.remote`], cwd);
  if (configured) return configured;
  const remotes = (await execGit(['remote'], cwd))
    .split('\n')
    .map((r) => r.trim())
    .filter(Boolean);
  return remotes[0] ?? 'origin';
}

/**
 * `git push --quiet` with no force and no auto-stash.
 *
 * First-time / mis-tracked branches use `git push -u <remote> HEAD --quiet` so a
 * feature branch tracking `develop` (or with no upstream) publishes under its
 * own name. Diverged remotes may still fail; callers count that as a failed push.
 */
export async function pushQuiet(cwd: string): Promise<boolean> {
  const branch = await execGit(['branch', '--show-current'], cwd);
  if (!branch) return false;

  if (await needsUpstreamPublish(cwd)) {
    const remote = await pushRemoteName(cwd, branch);
    return (await execGitStatus(['push', '-u', remote, 'HEAD', '--quiet'], cwd)) === 0;
  }
  return (await execGitStatus(['push', '--quiet'], cwd)) === 0;
}

/** Unified context large enough to show an entire typical source file in one hunk. */
export const FULL_DIFF_CONTEXT_LINES = 999_999;

function diffArgs(cached: boolean, filePath: string, contextLines?: number): string[] {
  const args = ['diff'];
  if (cached) args.push('--cached');
  if (contextLines !== undefined) args.push(`-U${contextLines}`);
  args.push('--', filePath);
  return args;
}

/** Unified diff for an unstaged tracked file. */
export async function diffFile(
  repoDir: string,
  filePath: string,
  contextLines?: number,
): Promise<string> {
  return execGit(diffArgs(false, filePath, contextLines), repoDir);
}

/** Unified diff for a staged (cached) file. */
export async function diffCachedFile(
  repoDir: string,
  filePath: string,
  contextLines?: number,
): Promise<string> {
  return execGit(diffArgs(true, filePath, contextLines), repoDir);
}

/**
 * Files changed in one commit (name-status -z).
 *
 * Uses the first-parent range (`commit^` → `commit`) so merge commits list the
 * same changes as `git show --first-parent` / `git diff commit^ commit`.
 * Plain `diff-tree <merge>` is empty; `diff-tree -m --first-parent <merge>`
 * still emits every parent patch, so the explicit range is required.
 * Root commits (no `^`) fall back to `--root`.
 */
export async function listCommitNameStatus(
  repoDir: string,
  commitId: string,
): Promise<FileChange[]> {
  const out = await execGit(
    ['diff-tree', '--no-commit-id', '--name-status', '-r', '-z', `${commitId}^`, commitId],
    repoDir,
  );
  if (out) return parseNameStatusZ(out);
  const root = await execGit(
    ['diff-tree', '--no-commit-id', '--name-status', '-r', '-z', '--root', commitId],
    repoDir,
  );
  return parseNameStatusZ(root);
}

/**
 * Worktree + index changes as FileChange rows (tracked name-status + untracked).
 */
export async function listWorktreeNameStatus(repoDir: string): Promise<FileChange[]> {
  const tracked = parseNameStatusZ(await execGit(['diff', 'HEAD', '--name-status', '-z'], repoDir));
  const porcelain = await execGit(['status', '--porcelain=v1', '-z'], repoDir);
  const byPath = new Map(tracked.map((c) => [c.path, c]));
  const parts = porcelain.split('\0').filter(Boolean);
  for (const entry of parts) {
    // "XY path" — untracked "?? path"
    if (entry.startsWith('?? ')) {
      const path = entry.slice(3);
      if (!byPath.has(path)) byPath.set(path, { path, untracked: true });
    }
  }
  return [...byPath.values()];
}

/**
 * Files in a stash entry.
 */
export async function listStashNameStatus(
  repoDir: string,
  stashRef: string,
): Promise<FileChange[]> {
  const out = await execGit(['stash', 'show', '--name-status', '-z', stashRef], repoDir);
  return parseNameStatusZ(out);
}

/**
 * Unified diff for one path in a commit (placed in unstaged slot by callers).
 *
 * Prefers the first-parent range (`commit^..commit`) so merges show the same
 * first-parent changes as `listCommitNameStatus`. Falls back to
 * `show --first-parent` for root commits / empty ranges.
 */
export async function diffCommitFile(
  repoDir: string,
  commitId: string,
  filePath: string,
  contextLines?: number,
): Promise<string> {
  const u = contextLines !== undefined ? [`-U${contextLines}`] : [];
  const primary = await execGit(['diff', ...u, `${commitId}^`, commitId, '--', filePath], repoDir);
  if (primary.trim()) return primary;
  return execGit(['show', ...u, '--first-parent', commitId, '--', filePath], repoDir);
}

/**
 * Unified diff for one path inside a stash.
 *
 * Uses `diff <stash>^1 <stash> -- <path>` (same first-parent range as
 * `stash show`). Pathspecs after `stash show -p <ref>` are rejected by git
 * ("Too many revisions specified"), so that form cannot drive per-file diffs.
 */
export async function diffStashFile(
  repoDir: string,
  stashRef: string,
  filePath: string,
  contextLines?: number,
): Promise<string> {
  const u = contextLines !== undefined ? [`-U${contextLines}`] : [];
  return execGit(['diff', ...u, `${stashRef}^1`, stashRef, '--', filePath], repoDir);
}

/** Stage a file (`git add`). */
export async function stageFile(
  repoDir: string,
  filePath: string,
): Promise<{ ok: boolean; error?: string }> {
  const code = await execGitStatus(['add', '--', filePath], repoDir);
  return code === 0
    ? { ok: true as const }
    : { ok: false as const, error: `git add failed (${code})` };
}

/** Unstage a file (`git restore --staged`). */
export async function unstageFile(
  repoDir: string,
  filePath: string,
): Promise<{ ok: boolean; error?: string }> {
  const code = await execGitStatus(['restore', '--staged', '--', filePath], repoDir);
  return code === 0
    ? { ok: true as const }
    : { ok: false as const, error: `git restore --staged failed (${code})` };
}

/** Discard unstaged changes to a tracked file (`git restore`). */
export async function revertTrackedFile(
  repoDir: string,
  filePath: string,
): Promise<{ ok: boolean; error?: string }> {
  const code = await execGitStatus(['restore', '--', filePath], repoDir);
  return code === 0
    ? { ok: true as const }
    : { ok: false as const, error: `git restore failed (${code})` };
}

/** Remove an untracked file (`git clean -f`). */
export async function removeUntrackedFile(
  repoDir: string,
  filePath: string,
): Promise<{ ok: boolean; error?: string }> {
  const code = await execGitStatus(['clean', '-f', '--', filePath], repoDir);
  return code === 0
    ? { ok: true as const }
    : { ok: false as const, error: `git clean failed (${code})` };
}

/**
 * One local branch from `git for-each-ref` on `refs/heads/`.
 */
export type LocalBranch = {
  name: string;
  /** Authored date unix seconds from `git for-each-ref`. */
  authordate: number;
  current: boolean;
};

/**
 * List local branches only (no remotes).
 *
 * Uses `for-each-ref` with short name, unix authordate, and HEAD marker.
 */
export async function listLocalBranches(repoDir: string): Promise<LocalBranch[]> {
  const out = await execGit(
    ['for-each-ref', '--format=%(refname:short)\t%(authordate:unix)\t%(HEAD)', 'refs/heads/'],
    repoDir,
  );
  if (!out) return [];
  const branches: LocalBranch[] = [];
  for (const line of out.split('\n')) {
    if (!line) continue;
    const [name, dateStr, head] = line.split('\t');
    if (!name) continue;
    const authordate = Number(dateStr);
    branches.push({
      name,
      authordate: Number.isFinite(authordate) ? authordate : 0,
      current: head === '*',
    });
  }
  return branches;
}

/** Run git asynchronously and return stdout (empty string on failure). */
export async function execGitOutputAsync(args: string[], cwd: string): Promise<string> {
  return execGit(args, cwd);
}

/**
 * Run git asynchronously. Rejects when the process exits non-zero.
 * Use for long-running ops (fetch, pull) to run in parallel.
 */
export async function execGitAsync(args: string[], cwd: string): Promise<void> {
  const result = await git$(cwd)`${args}`;
  if (!result.ok) {
    throw new Error(`git ${args[0]} exited with code ${result.exitCode}`);
  }
}

/** `git worktree list --porcelain` stdout for a repo (empty string on failure). */
export async function listWorktreesPorcelain(repoDir: string): Promise<string> {
  return execGit(['worktree', 'list', '--porcelain'], repoDir);
}

/**
 * Whether `maybeAncestor` is an ancestor of `tip` (`git merge-base --is-ancestor`).
 *
 * Exit 0 → true, 1 → false, any other outcome → null.
 */
export async function isAncestor(
  repoDir: string,
  maybeAncestor: string,
  tip: string,
): Promise<boolean | null> {
  const code = await execGitStatus(['merge-base', '--is-ancestor', maybeAncestor, tip], repoDir);
  if (code === 0) return true;
  if (code === 1) return false;
  return null;
}

/**
 * First existing tip ref among `origin/<defaultBranch>` then `<defaultBranch>`.
 */
export async function resolveDefaultBranchTipRef(
  repoDir: string,
  defaultBranch: string,
): Promise<string | null> {
  for (const ref of [`origin/${defaultBranch}`, defaultBranch]) {
    if (
      (await execGitStatus(['rev-parse', '--verify', '--quiet', `${ref}^{commit}`], repoDir)) === 0
    ) {
      return ref;
    }
  }
  return null;
}

/**
 * Default branch name for merge-into-default classification.
 *
 * Override wins; otherwise `origin/HEAD` short name with `origin/` stripped; else `main`.
 */
export async function resolveDefaultBranchName(
  repoDir: string,
  defaultBranchOverride?: string,
): Promise<string> {
  if (defaultBranchOverride) return defaultBranchOverride;
  const remoteHead = await execGit(
    ['symbolic-ref', '--quiet', '--short', 'refs/remotes/origin/HEAD'],
    repoDir,
  );
  if (remoteHead) {
    const m = remoteHead.match(/^origin\/(.+)$/);
    return m ? m[1]! : remoteHead;
  }
  return 'main';
}

/**
 * One commit line from `git log` for the graph window (no refs attached yet).
 */
export type RawGraphCommit = {
  id: string;
  parents: string[];
  subject: string;
  authorName: string;
  authorDateUnix: number;
};

const GRAPH_LOG_FORMAT = '%H%x00%P%x00%s%x00%an%x00%at';

/**
 * Load a topo-ordered window of commits across branch/remote/tag refs.
 *
 * Excludes `refs/stash` so stash WIP/index/untracked commits are not painted
 * as normal history above HEAD (stashes still come from `listStashes`).
 *
 * Args: `log --exclude=refs/stash --all --topo-order --date-order
 * --skip=<skip> --max-count=<limit> --format=%H%x00%P%x00%s%x00%an%x00%at`
 */
export async function gitLogGraphWindow(
  repoDir: string,
  opts: { skip: number; limit: number },
): Promise<{ commits: RawGraphCommit[]; truncated: boolean }> {
  const out = await execGit(
    [
      'log',
      // `--exclude` must precede `--all` or stash still gets included.
      '--exclude=refs/stash',
      '--all',
      '--topo-order',
      '--date-order',
      `--skip=${opts.skip}`,
      `--max-count=${opts.limit}`,
      `--format=${GRAPH_LOG_FORMAT}`,
    ],
    repoDir,
  );
  if (!out) return { commits: [], truncated: false };
  const commits = parseRawGraphLog(out);
  return { commits, truncated: commits.length === opts.limit };
}

/**
 * Parse `git log --format=%H%x00%P%x00%s%x00%an%x00%at` stdout into commits.
 */
function parseRawGraphLog(out: string): RawGraphCommit[] {
  if (!out) return [];
  const commits: RawGraphCommit[] = [];
  for (const line of out.split('\n')) {
    if (!line) continue;
    const [id, parentsRaw, subject, authorName, at] = line.split('\0');
    if (!id) continue;
    const parents = (parentsRaw ?? '').trim().split(/\s+/).filter(Boolean);
    commits.push({
      id,
      parents,
      subject: subject ?? '',
      authorName: authorName ?? '',
      authorDateUnix: Number(at) || 0,
    });
  }
  return commits;
}

/**
 * Load specific commits by id — used for `stash^1` outside the graph window.
 *
 * `log --all --exclude=refs/stash` omits parents that are only reachable
 * through the stash reflog (reset/deleted branch). `--no-walk` still
 * resolves those object ids. Missing ids are omitted.
 */
export async function gitLogCommitsByIds(
  repoDir: string,
  ids: readonly string[],
): Promise<RawGraphCommit[]> {
  const unique = [...new Set(ids.filter((id) => id.length > 0))];
  if (unique.length === 0) return [];
  const out = await execGit(
    ['log', '--no-walk', '--ignore-missing', `--format=${GRAPH_LOG_FORMAT}`, ...unique],
    repoDir,
  );
  return parseRawGraphLog(out);
}

/**
 * List local branches, remotes, and tags.
 *
 * Args: `for-each-ref --format=%(objectname)%09%(refname)%09%(refname:short)
 * refs/heads/ refs/remotes/ refs/tags/`
 */
export async function listRefs(repoDir: string): Promise<GraphRef[]> {
  const out = await execGit(
    [
      'for-each-ref',
      '--format=%(objectname)%09%(refname)%09%(refname:short)',
      'refs/heads/',
      'refs/remotes/',
      'refs/tags/',
    ],
    repoDir,
  );
  if (!out) return [];
  const refs: GraphRef[] = [];
  for (const line of out.split('\n')) {
    if (!line) continue;
    const [objectname, refname, short] = line.split('\t');
    if (!objectname || !refname || !short) continue;
    if (refname.endsWith('/HEAD')) continue;
    let kind: GraphRef['kind'];
    if (refname.startsWith('refs/heads/')) kind = 'local';
    else if (refname.startsWith('refs/remotes/')) kind = 'remote';
    else if (refname.startsWith('refs/tags/')) kind = 'tag';
    else continue;
    refs.push({ kind, name: short, commitId: objectname });
  }
  return refs;
}

/**
 * List stash entries.
 *
 * Args: `stash list --format=%gd%x00%H%x00%P%x00%s%x00%at%x00%an`
 * `%P` parents — first parent is HEAD at stash time (`stash^1`).
 */
export async function listStashes(repoDir: string): Promise<GraphStash[]> {
  const out = await execGit(
    ['stash', 'list', '--format=%gd%x00%H%x00%P%x00%s%x00%at%x00%an'],
    repoDir,
  );
  if (!out) return [];
  const stashes: GraphStash[] = [];
  for (const line of out.split('\n')) {
    if (!line) continue;
    const [stashRef, id, parents, subject, at, authorName] = line.split('\0');
    if (!stashRef || !id) continue;
    const m = /^stash@\{(\d+)\}$/.exec(stashRef);
    const index = m ? Number(m[1]) : stashes.length;
    const parentId = (parents ?? '').trim().split(/\s+/)[0] ?? '';
    stashes.push({
      id,
      stashRef,
      index,
      subject: subject ?? '',
      authorName: authorName ?? '',
      authorDateUnix: Number(at) || 0,
      parentId,
    });
  }
  return stashes;
}

/**
 * Stable fingerprint of HEAD + refs + stashes + dirty bit.
 * Cache keys include this so fetch/checkout/stash/`r` invalidate automatically.
 */
export async function computeRefsFingerprint(repoDir: string): Promise<string> {
  const [head, refsOut, stashOut, porcelain] = await Promise.all([
    execGit(['rev-parse', 'HEAD'], repoDir),
    execGit(
      [
        'for-each-ref',
        '--format=%(objectname) %(refname)',
        'refs/heads/',
        'refs/remotes/',
        'refs/tags/',
      ],
      repoDir,
    ),
    execGit(['stash', 'list', '--format=%H %gd'], repoDir),
    execGit(['status', '--porcelain=v1'], repoDir),
  ]);
  const dirty = porcelain.trim() ? '1' : '0';
  const refLines = refsOut
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .sort();
  const stashLines = stashOut
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .sort();
  return ['HEAD ' + head, ...refLines, ...stashLines, 'dirty ' + dirty].join('\n');
}

/**
 * True when worktree or index has any porcelain entry (incl. untracked).
 */
export async function repoHasPorcelainChanges(cwd: string): Promise<boolean> {
  const out = await execGit(['status', '--porcelain=v1'], cwd);
  return out.trim().length > 0;
}

/**
 * Create a local branch at commitId without checking it out.
 */
export async function createBranchAt(
  repoDir: string,
  name: string,
  commitId: string,
): Promise<{ ok: boolean; error?: string }> {
  const code = await execGitStatus(['branch', '--', name, commitId], repoDir);
  return code === 0 ? { ok: true } : { ok: false, error: `git branch failed (${code})` };
}

/**
 * Push worktree changes onto the stash.
 *
 * Includes untracked files unless `includeUntracked` is false (`-u`, not `-a`).
 * Optional `-m` message and pathspecs after `--`.
 */
export async function stashPush(
  repoDir: string,
  opts?: {
    message?: string;
    includeUntracked?: boolean;
    paths?: readonly string[];
  },
): Promise<{ ok: boolean; error?: string }> {
  const args = ['stash', 'push'];
  if (opts?.includeUntracked !== false) args.push('-u');
  if (opts?.message) args.push('-m', opts.message);
  if (opts?.paths && opts.paths.length > 0) args.push('--', ...opts.paths);
  const before = await execGit(['stash', 'list'], repoDir);
  const code = await execGitStatus(args, repoDir);
  if (code !== 0) {
    return { ok: false, error: `git stash push failed (${code})` };
  }
  // Apple Git 2.50 prints "No local changes to save" but still exits 0.
  const after = await execGit(['stash', 'list'], repoDir);
  if (after === before) {
    return { ok: false, error: 'git stash push failed (no local changes to save)' };
  }
  return { ok: true };
}

/**
 * Apply a stash (keeps the stash entry).
 */
export async function stashApply(
  repoDir: string,
  stashRef: string,
): Promise<{ ok: boolean; error?: string }> {
  const code = await execGitStatus(['stash', 'apply', stashRef], repoDir);
  return code === 0 ? { ok: true } : { ok: false, error: `git stash apply failed (${code})` };
}

/**
 * Pop a stash entry (applies then drops).
 */
export async function stashPop(
  repoDir: string,
  stashRef?: string,
): Promise<{ ok: boolean; error?: string }> {
  const args = stashRef ? ['stash', 'pop', stashRef] : ['stash', 'pop'];
  const code = await execGitStatus(args, repoDir);
  return code === 0 ? { ok: true } : { ok: false, error: `git stash pop failed (${code})` };
}

/**
 * Drop a stash entry.
 */
export async function stashDrop(
  repoDir: string,
  stashRef: string,
): Promise<{ ok: boolean; error?: string }> {
  const code = await execGitStatus(['stash', 'drop', stashRef], repoDir);
  return code === 0 ? { ok: true } : { ok: false, error: `git stash drop failed (${code})` };
}

/**
 * Drop a linked worktree checkout.
 *
 * Runs `git worktree remove [--force] <path>` from the primary. When the TUI
 * path is a bind-mount alias of the registered worktree, remaps to git's
 * porcelain path and runs from that primary prefix so gitdir back-pointers
 * match. Callers must confirm and set `force` when the worktree is dirty.
 */
export async function removeWorktree(
  primaryAbs: string,
  worktreePath: string,
  opts: { force: boolean },
): Promise<{ ok: boolean; error?: string }> {
  const porcelain = await listWorktreesPorcelain(primaryAbs);
  const { gitCwd, gitPath } = resolveWorktreeRemoveTarget(
    parseWorktreeListPorcelain(porcelain),
    primaryAbs,
    worktreePath,
  );
  const args = opts.force
    ? ['worktree', 'remove', '--force', gitPath]
    : ['worktree', 'remove', gitPath];
  try {
    const result = await git$(gitCwd)`${args}`;
    const code = result.exitCode ?? -1;
    if (code === 0) return { ok: true };
    const detail = [result.stderr, result.stdout].map((s) => s.trim()).find((s) => s.length > 0);
    return {
      ok: false,
      error: detail
        ? `git worktree remove failed: ${detail}`
        : `git worktree remove failed (${code})`,
    };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { ok: false, error: `git worktree remove failed: ${message}` };
  }
}
