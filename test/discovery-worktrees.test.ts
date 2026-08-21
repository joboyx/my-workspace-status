/**
 * Discovery of linked git worktrees under cwd + merge-into-default probe.
 */

import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import assert from 'node:assert';

import { workspaceStatusConfig } from '../src/config.js';
import { collectSnapshotsWithConfig, processRepo, validateFilterRepos } from '../src/discovery.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'discovery-worktrees.test',
  GIT_AUTHOR_EMAIL: 'discovery-worktrees.test@example.invalid',
  GIT_COMMITTER_NAME: 'discovery-worktrees.test',
  GIT_COMMITTER_EMAIL: 'discovery-worktrees.test@example.invalid',
  ...process.env,
};

function gitInit(repoPath: string): void {
  fs.mkdirSync(repoPath, { recursive: true });
  execSync(`git init -q -b main "${repoPath}"`, { stdio: 'pipe', env: GIT_ENV });
  fs.writeFileSync(path.join(repoPath, 'README.md'), 'seed\n', 'utf-8');
  execSync(`git -C "${repoPath}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${repoPath}" commit -q -m "seed"`, { stdio: 'pipe', env: GIT_ENV });
}

describe('collectSnapshotsWithConfig linked worktrees', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-ws-status-wt-discover-'));
    const app = path.join(workspaceRoot, 'app');
    gitInit(app);
    fs.mkdirSync(path.join(app, '.worktrees'), { recursive: true });
    const feat = path.join(app, '.worktrees', 'feat');
    execSync(`git -C "${app}" worktree add -q "${feat}" -b feature/x`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    // Unique tip so feature/x is not an ancestor of main (open / unmerged).
    fs.writeFileSync(path.join(feat, 'feat.txt'), 'open\n', 'utf-8');
    execSync(`git -C "${feat}" add feat.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${feat}" commit -q -m "open feature"`, { stdio: 'pipe', env: GIT_ENV });
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true, force: true });
    }
  });

  it('includes primary and linked .worktrees path with checkout metadata', async () => {
    const snapshots = await collectSnapshotsWithConfig(
      workspaceRoot,
      false,
      workspaceStatusConfig({ ignoredRepos: [], maxDepth: 3, defaultBranches: {} }),
    );
    const byRepo = new Map(snapshots.map((s) => [s.repo, s]));
    assert.ok(byRepo.has('app'), 'expected primary app');
    assert.ok(byRepo.has('app/.worktrees/feat'), 'expected linked worktree');

    const primary = byRepo.get('app')!;
    assert.equal(primary.checkoutKind, 'primary');
    assert.equal(primary.primaryRepo, undefined);
    assert.equal(primary.branch, 'main');
    assert.equal(primary.mergedIntoDefault, null);

    const linked = byRepo.get('app/.worktrees/feat')!;
    assert.equal(linked.checkoutKind, 'linked');
    assert.equal(linked.primaryRepo, 'app');
    assert.equal(linked.branch, 'feature/x');
    // Fresh feature branch tip is not an ancestor of main → open (false) when tip resolvable.
    assert.equal(linked.mergedIntoDefault, false);
  });

  it('omits linked children when primary is ignored', async () => {
    const snapshots = await collectSnapshotsWithConfig(
      workspaceRoot,
      false,
      workspaceStatusConfig({ ignoredRepos: ['app'], maxDepth: 3, defaultBranches: {} }),
    );
    const repos = snapshots.map((s) => s.repo).sort();
    assert.deepEqual(repos, []);
  });

  it('omits a specifically ignored linked path while keeping its primary', async () => {
    const snapshots = await collectSnapshotsWithConfig(
      workspaceRoot,
      false,
      workspaceStatusConfig({
        ignoredRepos: ['app/.worktrees/feat'],
        maxDepth: 3,
        defaultBranches: {},
      }),
    );
    const repos = snapshots.map((s) => s.repo).sort();
    assert.deepEqual(repos, ['app']);
    assert.equal(snapshots[0]!.checkoutKind, 'primary');
  });

  it('named filter on primary includes its linked worktrees under cwd', async () => {
    const snapshots = await collectSnapshotsWithConfig(
      workspaceRoot,
      false,
      workspaceStatusConfig({ ignoredRepos: [], maxDepth: 3, defaultBranches: {} }),
      new Set(['app']),
    );
    const repos = snapshots.map((s) => s.repo).sort();
    assert.deepEqual(repos, ['app', 'app/.worktrees/feat']);
  });

  it('named filter on linked path includes only that path', async () => {
    const snapshots = await collectSnapshotsWithConfig(
      workspaceRoot,
      false,
      workspaceStatusConfig({ ignoredRepos: [], maxDepth: 3, defaultBranches: {} }),
      new Set(['app/.worktrees/feat']),
    );
    const repos = snapshots.map((s) => s.repo).sort();
    assert.deepEqual(repos, ['app/.worktrees/feat']);
    assert.equal(snapshots[0]!.checkoutKind, 'linked');
    assert.equal(snapshots[0]!.primaryRepo, 'app');
  });

  it('validateFilterRepos accepts linked .worktrees paths', async () => {
    await validateFilterRepos(workspaceRoot, ['app/.worktrees/feat']);
  });

  it('linked children inherit primary defaultBranches override for merge classification', async () => {
    const snapshots = await collectSnapshotsWithConfig(
      workspaceRoot,
      false,
      workspaceStatusConfig({
        ignoredRepos: [],
        maxDepth: 3,
        defaultBranches: { app: 'main' },
      }),
    );
    const linked = snapshots.find((s) => s.repo === 'app/.worktrees/feat');
    assert.ok(linked);
    assert.equal(linked.defaultBranchOverride, 'main');
    assert.equal(linked.mergedIntoDefault, false);
  });
});

describe('collectSnapshotsWithConfig mergedIntoDefault true', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-ws-status-wt-merged-'));
    const app = path.join(workspaceRoot, 'app');
    gitInit(app);
    execSync(`git -C "${app}" checkout -q -b feature/merged`, { stdio: 'pipe', env: GIT_ENV });
    fs.writeFileSync(path.join(app, 'extra.txt'), 'x\n', 'utf-8');
    execSync(`git -C "${app}" add extra.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${app}" commit -q -m "on feature"`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${app}" checkout -q main`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${app}" merge -q feature/merged`, { stdio: 'pipe', env: GIT_ENV });
    fs.mkdirSync(path.join(app, '.worktrees'), { recursive: true });
    execSync(
      `git -C "${app}" worktree add -q "${path.join(app, '.worktrees', 'merged')}" feature/merged`,
      { stdio: 'pipe', env: GIT_ENV },
    );
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true, force: true });
    }
  });

  it('marks feature tip merged into default when ancestor of main', async () => {
    const snapshots = await collectSnapshotsWithConfig(
      workspaceRoot,
      false,
      workspaceStatusConfig(),
    );
    const linked = snapshots.find((s) => s.repo === 'app/.worktrees/merged');
    assert.ok(linked);
    assert.equal(linked.checkoutKind, 'linked');
    assert.equal(linked.mergedIntoDefault, true);
  });
});

describe('processRepo mergedIntoDefault on legacy default branch', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-ws-status-legacy-default-'));
    const app = path.join(workspaceRoot, 'app');
    gitInit(app);
    // origin/HEAD → main while local checkout is develop (still a legacy default).
    const tip = execSync(`git -C "${app}" rev-parse HEAD`, {
      encoding: 'utf-8',
      env: GIT_ENV,
    }).trim();
    execSync(`git -C "${app}" update-ref refs/remotes/origin/main ${tip}`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${app}" symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${app}" checkout -q -b develop`, { stdio: 'pipe', env: GIT_ENV });
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true, force: true });
    }
  });

  it('develop + origin/HEAD main → mergedIntoDefault null (no merge mark)', async () => {
    const snap = await processRepo('app', workspaceRoot, false);
    assert.ok(snap);
    assert.equal(snap.branch, 'develop');
    assert.equal(snap.mergedIntoDefault, null);
  });
});

describe('validateFilterRepos honors workspace maxDepth', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-ws-status-filter-depth-'));
    const deep = path.join(workspaceRoot, 'a', 'b', 'c', 'd');
    gitInit(deep);
    fs.writeFileSync(
      path.join(workspaceRoot, '.workspace-status-config.json'),
      JSON.stringify({ ignoredRepos: [], maxDepth: 4 }),
      'utf-8',
    );
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true, force: true });
    }
  });

  it('accepts a depth-4 repo when config maxDepth is 4', async () => {
    await validateFilterRepos(workspaceRoot, ['a/b/c/d']);
  });
});
