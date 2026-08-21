/**
 * Temp git workspace factory for live TUI e2e.
 */
import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { collectSnapshotsWithConfig } from '../../src/discovery.js';
import { workspaceStatusConfig } from '../../src/config.js';
import type { RepoSnapshot } from '../../src/types.js';

export const GIT_ENV: NodeJS.ProcessEnv = {
  GIT_AUTHOR_NAME: 'tui-e2e',
  GIT_AUTHOR_EMAIL: 'tui-e2e@example.invalid',
  GIT_COMMITTER_NAME: 'tui-e2e',
  GIT_COMMITTER_EMAIL: 'tui-e2e@example.invalid',
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_NOSYSTEM: '1',
  ...process.env,
};

export type WorkspaceHandle = {
  root: string;
  remotes: string;
  scratch: string;
  path(name: string): string;
};

/**
 * Create an empty workspace + remotes/scratch dirs.
 */
export function createWorkspace(): WorkspaceHandle {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-tui-e2e.'));
  const workspace = path.join(root, 'workspace');
  const remotes = path.join(root, 'remotes');
  const scratch = path.join(root, 'scratch');
  fs.mkdirSync(workspace, { recursive: true });
  fs.mkdirSync(remotes, { recursive: true });
  fs.mkdirSync(scratch, { recursive: true });
  return {
    root: workspace,
    remotes,
    scratch,
    path(name: string): string {
      return path.join(workspace, name);
    },
  };
}

/**
 * Remove a workspace tree. Ignores missing paths.
 */
export function destroyWorkspace(ws: WorkspaceHandle): void {
  const parent = path.dirname(ws.root);
  fs.rmSync(parent, { recursive: true, force: true });
}

/**
 * Run git in `cwd` with e2e identity.
 */
export function git(cwd: string, args: string): string {
  return execSync(`git ${args}`, {
    cwd,
    stdio: 'pipe',
    encoding: 'utf8',
    env: GIT_ENV,
  });
}

/**
 * Init a repo on `branch` with a seed commit.
 */
export function initRepo(
  ws: WorkspaceHandle,
  name: string,
  opts: { branch?: string; file?: string; content?: string } = {},
): string {
  const repo = ws.path(name);
  const branch = opts.branch ?? 'main';
  fs.mkdirSync(repo, { recursive: true });
  try {
    execSync(`git init -q -b ${branch} "${repo}"`, { stdio: 'pipe', env: GIT_ENV });
  } catch {
    execSync(`git init -q "${repo}"`, { stdio: 'pipe', env: GIT_ENV });
    git(repo, `checkout -q -b ${branch}`);
  }
  git(repo, 'config user.name tui-e2e');
  git(repo, 'config user.email tui-e2e@example.invalid');
  const rel = opts.file ?? 'README.md';
  writeRepoFile(ws, name, rel, opts.content ?? `${name} seed\n`);
  git(repo, `add ${rel}`);
  git(repo, `commit -q -m "seed ${name}"`);
  return repo;
}

/**
 * Write a file under a repo (creates parents).
 */
export function writeRepoFile(
  ws: WorkspaceHandle,
  repo: string,
  rel: string,
  content: string,
): string {
  const abs = path.join(ws.path(repo), rel);
  fs.mkdirSync(path.dirname(abs), { recursive: true });
  fs.writeFileSync(abs, content, 'utf8');
  return abs;
}

/**
 * Stage and commit current worktree changes.
 */
export function commitAll(ws: WorkspaceHandle, repo: string, message: string): void {
  const cwd = ws.path(repo);
  git(cwd, 'add -A');
  git(cwd, `commit -q -m "${message}"`);
}

/**
 * Point the repo at a new bare origin and push HEAD.
 */
export function addOrigin(ws: WorkspaceHandle, repo: string): string {
  const bare = path.join(ws.remotes, `${repo.replaceAll('/', '_')}.git`);
  try {
    execSync(`git init -q --bare -b main "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
  } catch {
    execSync(`git init -q --bare "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
  }
  const cwd = ws.path(repo);
  git(cwd, `remote add origin "${bare}"`);
  git(cwd, 'push -q -u origin HEAD');
  return bare;
}

/**
 * Advance origin with a commit the local clone does not have, then fetch
 * so the snapshot sees `behind`.
 */
export function makeBehind(ws: WorkspaceHandle, repo: string): void {
  const bare = path.join(ws.remotes, `${repo.replaceAll('/', '_')}.git`);
  const clone = path.join(ws.scratch, `behind-${repo.replaceAll('/', '_')}`);
  execSync(`git clone -q "${bare}" "${clone}"`, { stdio: 'pipe', env: GIT_ENV });
  git(clone, 'config user.name tui-e2e');
  git(clone, 'config user.email tui-e2e@example.invalid');
  fs.writeFileSync(path.join(clone, 'ahead-on-origin.txt'), 'origin\n', 'utf8');
  git(clone, 'add ahead-on-origin.txt');
  git(clone, 'commit -q -m "origin advance"');
  git(clone, 'push -q origin HEAD');
  git(ws.path(repo), 'fetch -q origin');
}

/**
 * Add a local commit that is not pushed (ahead of origin).
 */
export function makeAhead(ws: WorkspaceHandle, repo: string): void {
  writeRepoFile(ws, repo, 'local-ahead.txt', 'ahead\n');
  commitAll(ws, repo, 'local ahead');
}

/**
 * Create a linked worktree on `branch` under `primary/.worktrees/linked`.
 */
export function addLinkedWorktree(ws: WorkspaceHandle, primary: string, branch: string): string {
  const primaryAbs = ws.path(primary);
  const linked = path.join(primaryAbs, '.worktrees', 'linked');
  fs.mkdirSync(path.dirname(linked), { recursive: true });
  git(primaryAbs, `worktree add -q "${linked}" -b ${branch}`);
  writeRepoFile(ws, path.join(primary, '.worktrees', 'linked'), 'linked.txt', 'open\n');
  commitAll(ws, path.join(primary, '.worktrees', 'linked'), 'open linked');
  return linked;
}

/**
 * Create a stash on the repo (needs a dirty or staged change).
 */
export function stashPush(ws: WorkspaceHandle, repo: string): void {
  writeRepoFile(ws, repo, 'stash-me.txt', 'stash\n');
  git(ws.path(repo), 'add stash-me.txt');
  git(ws.path(repo), 'stash push -q -u -m tui-e2e-stash');
}

/**
 * Discover snapshots the same way the TUI does (ignored list empty so
 * ignored repos are still present for `.` toggle).
 */
export async function loadSnapshots(cwd: string): Promise<RepoSnapshot[]> {
  return collectSnapshotsWithConfig(
    cwd,
    false,
    workspaceStatusConfig({
      ignoredRepos: [],
      maxDepth: 5,
      defaultBranches: {},
    }),
  );
}

/**
 * `git stash list` output, or empty when there are no stashes.
 */
export function stashList(ws: WorkspaceHandle, repo: string): string {
  try {
    return git(ws.path(repo), 'stash list');
  } catch {
    return '';
  }
}

/**
 * Staged paths (`git diff --cached --name-only`).
 */
export function stagedNames(ws: WorkspaceHandle, repo: string): string {
  return git(ws.path(repo), 'diff --cached --name-only');
}

/**
 * Read a repo-relative file, or null if it is missing.
 */
export function readRepoFile(ws: WorkspaceHandle, repo: string, rel: string): string | null {
  const abs = path.join(ws.path(repo), rel);
  if (!fs.existsSync(abs)) return null;
  return fs.readFileSync(abs, 'utf8');
}
