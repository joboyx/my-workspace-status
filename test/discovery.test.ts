/**
 * Unit tests for repo discovery depth and config filtering.
 */

import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import assert from 'node:assert';

import { badgeForChange, fileChangesFromSnapshot } from '../src/changes.js';
import { workspaceStatusConfig } from '../src/config.js';
import {
  findReposWithConfig,
  parsePorcelainChangeLines,
  processRepo,
} from '../src/discovery.js';
import { statusLetterFromChange } from '../src/tui/icons.js';
import { SEP } from '../src/helpers.js';
import type { RepoSnapshot } from '../src/types.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'discovery.test',
  GIT_AUTHOR_EMAIL: 'discovery.test@example.invalid',
  GIT_COMMITTER_NAME: 'discovery.test',
  GIT_COMMITTER_EMAIL: 'discovery.test@example.invalid',
  ...process.env,
};

function gitInit(repoPath: string): void {
  fs.mkdirSync(repoPath, { recursive: true });
  execSync(`git init -q -b main "${repoPath}"`, { stdio: 'pipe', env: GIT_ENV });
  fs.writeFileSync(path.join(repoPath, 'README.md'), 'seed\n', 'utf-8');
  execSync(`git -C "${repoPath}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${repoPath}" commit -q -m "seed"`, { stdio: 'pipe', env: GIT_ENV });
}

describe('findReposWithConfig', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-discovery-'));
    gitInit(path.join(workspaceRoot, 'top'));
    gitInit(path.join(workspaceRoot, 'group', 'nested'));
    gitInit(path.join(workspaceRoot, 'group', 'mid', 'deep'));
    gitInit(path.join(workspaceRoot, 'group', 'mid', 'also-deep'));
    // Non-repo intermediate dirs only — discovery should still walk them.
    fs.mkdirSync(path.join(workspaceRoot, 'empty', 'path'), { recursive: true });
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true });
    }
  });

  it('defaults to depth 3 and finds great-grandchild repos', async () => {
    const repos = await findReposWithConfig(workspaceRoot, workspaceStatusConfig());
    assert.deepEqual(repos, ['group/mid/also-deep', 'group/mid/deep', 'group/nested', 'top']);
  });

  it('respects maxDepth 2', async () => {
    const repos = await findReposWithConfig(workspaceRoot, workspaceStatusConfig({ maxDepth: 2 }));
    assert.deepEqual(repos, ['group/nested', 'top']);
  });

  it('respects maxDepth 1', async () => {
    const repos = await findReposWithConfig(workspaceRoot, workspaceStatusConfig({ maxDepth: 1 }));
    assert.deepEqual(repos, ['top']);
  });

  it('skips ignored repos and does not descend into them', async () => {
    const repos = await findReposWithConfig(
      workspaceRoot,
      workspaceStatusConfig({ ignoredRepos: ['group'] }),
    );
    assert.deepEqual(repos, ['top']);
  });

  it('filters to a depth-3 named repo via onlyRepos', async () => {
    const repos = await findReposWithConfig(
      workspaceRoot,
      workspaceStatusConfig({ ignoredRepos: ['group'] }),
      new Set(['group/mid/deep']),
    );
    assert.deepEqual(repos, ['group/mid/deep']);
  });
});

describe('findReposWithConfig gitfile .git', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-gitfile-'));
    const mainRepo = path.join(workspaceRoot, 'gitfile-main');
    gitInit(mainRepo);
    const linkedPath = path.join(workspaceRoot, 'gitfile-linked');
    execSync(`git -C "${mainRepo}" worktree add -q "${linkedPath}" -b gitfile-branch`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    assert.ok(
      fs.statSync(path.join(linkedPath, '.git')).isFile(),
      'expected linked worktree .git to be a file',
    );
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true });
    }
  });

  it('discovers repos whose .git is a gitfile (linked worktree)', async () => {
    const repos = await findReposWithConfig(workspaceRoot, workspaceStatusConfig());
    assert.deepEqual(repos, ['gitfile-linked', 'gitfile-main']);
  });
});

describe('parsePorcelainChangeLines unmerged', () => {
  function snapshotFromParsed(parsed: ReturnType<typeof parsePorcelainChangeLines>): RepoSnapshot {
    return {
      repo: 'demo',
      branch: 'main',
      syncStatus: 'up-to-date',
      syncNote: '',
      hasUnstaged: parsed.unstagedEntries.length > 0,
      hasStaged: parsed.stagedEntries.length > 0,
      hasUntracked: parsed.untrackedEntries.length > 0,
      unstagedInfo: '',
      stagedFiles: parsed.stagedEntries.join(SEP),
      unstagedFiles: parsed.unstagedEntries.join(SEP),
      untrackedFiles: parsed.untrackedEntries.join(SEP),
      checkoutKind: 'primary',
      mergedIntoDefault: null,
    };
  }

  for (const xy of ['UU', 'AA', 'DD', 'AU', 'UA', 'DU', 'UD'] as const) {
    it(`maps ${xy} to a single unstaged U (not MS)`, () => {
      const parsed = parsePorcelainChangeLines([`${xy} conflict.txt`]);
      assert.deepEqual(parsed.stagedEntries, []);
      assert.deepEqual(parsed.unstagedEntries, ['U\tconflict.txt']);
      assert.deepEqual(parsed.untrackedEntries, []);

      const changes = fileChangesFromSnapshot(snapshotFromParsed(parsed));
      assert.equal(changes.length, 1);
      assert.equal(changes[0]!.unstagedStatus, 'U');
      assert.equal(changes[0]!.stagedStatus, undefined);
      assert.equal(statusLetterFromChange(changes[0]!), 'U');
      assert.equal(badgeForChange(changes[0]!), '⚠️U');
    });
  }

  it('still maps ordinary M and type-change T to M', () => {
    const parsed = parsePorcelainChangeLines([' M plain.txt', ' T typed.txt']);
    assert.deepEqual(parsed.stagedEntries, []);
    assert.deepEqual(parsed.unstagedEntries, ['M\tplain.txt', 'M\ttyped.txt']);
  });
});

describe('processRepo unborn and failed status', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-unborn-'));
    const unbornDir = path.join(workspaceRoot, 'unborn');
    fs.mkdirSync(unbornDir, { recursive: true });
    execSync(`git init -q -b main "${unbornDir}"`, { stdio: 'pipe', env: GIT_ENV });

    // .git exists but is not a usable repository — status output is empty/unparseable.
    fs.mkdirSync(path.join(workspaceRoot, 'broken', '.git'), { recursive: true });
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true });
    }
  });

  it('snapshots unborn repos with no commits yet', async () => {
    const snapshot = await processRepo('unborn', workspaceRoot, false);
    assert.ok(snapshot, 'expected a RepoSnapshot');
    assert.equal(snapshot.branch, 'main');
    assert.equal(snapshot.syncStatus, 'no-upstream');
    assert.equal(snapshot.syncNote, 'no commits yet');
    assert.equal(snapshot.hasUnstaged, false);
    assert.equal(snapshot.hasStaged, false);
  });

  it('returns a failure snapshot when git status is unusable', async () => {
    const snapshot = await processRepo('broken', workspaceRoot, false);
    assert.ok(snapshot, 'expected a RepoSnapshot');
    assert.equal(snapshot.branch, '(unknown)');
    assert.equal(snapshot.syncStatus, 'no-upstream');
    assert.equal(snapshot.syncNote, 'status failed');
  });
});

describe('processRepo merge conflicts', () => {
  let workspaceRoot = '';

  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-conflict-'));
    const repoDir = path.join(workspaceRoot, 'conflicted');
    gitInit(repoDir);

    // Both-modified (UU): diverge on shared.txt from a common base.
    fs.writeFileSync(path.join(repoDir, 'shared.txt'), 'base\n', 'utf-8');
    execSync(`git -C "${repoDir}" add shared.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" commit -q -m "shared base"`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" checkout -q -b other`, { stdio: 'pipe', env: GIT_ENV });
    fs.writeFileSync(path.join(repoDir, 'shared.txt'), 'theirs\n', 'utf-8');
    execSync(`git -C "${repoDir}" add shared.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" commit -q -m "theirs"`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" checkout -q main`, { stdio: 'pipe', env: GIT_ENV });
    fs.writeFileSync(path.join(repoDir, 'shared.txt'), 'ours\n', 'utf-8');
    execSync(`git -C "${repoDir}" add shared.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" commit -q -m "ours"`, { stdio: 'pipe', env: GIT_ENV });
    try {
      execSync(`git -C "${repoDir}" merge other`, { stdio: 'pipe', env: GIT_ENV });
    } catch {
      // Expected: conflict leaves UU in porcelain.
    }
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true });
    }
  });

  it('emits a single unstaged U for real UU merge', async () => {
    const snapshot = await processRepo('conflicted', workspaceRoot, false);
    assert.ok(snapshot, 'expected a RepoSnapshot');
    assert.equal(snapshot.hasStaged, false);
    assert.equal(snapshot.hasUnstaged, true);
    assert.match(snapshot.unstagedFiles, /^U\tshared\.txt$/);
    assert.equal(snapshot.stagedFiles, '');

    const changes = fileChangesFromSnapshot(snapshot);
    assert.equal(changes.length, 1);
    assert.equal(statusLetterFromChange(changes[0]!), 'U');
    assert.equal(badgeForChange(changes[0]!), '⚠️U');
  });
});
