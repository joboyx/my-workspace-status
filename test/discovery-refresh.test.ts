/**
 * Unit tests for single-repo snapshot refresh (real git in a temp workspace).
 */

import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import assert from 'node:assert';

import { refreshRepoSnapshot } from '../src/discovery.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'discovery-refresh.test',
  GIT_AUTHOR_EMAIL: 'discovery-refresh.test@example.invalid',
  GIT_COMMITTER_NAME: 'discovery-refresh.test',
  GIT_COMMITTER_EMAIL: 'discovery-refresh.test@example.invalid',
  ...process.env,
};

let workspaceRoot = '';
const REPO_NAME = 'demo-repo';

function gitInit(repoPath: string): void {
  execSync(`git init -q -b main "${repoPath}"`, { stdio: 'pipe', env: GIT_ENV });
  fs.writeFileSync(path.join(repoPath, 'README.md'), 'seed\n', 'utf-8');
  execSync(`git -C "${repoPath}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${repoPath}" commit -q -m "seed"`, { stdio: 'pipe', env: GIT_ENV });
}

describe('refreshRepoSnapshot', () => {
  before(() => {
    workspaceRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-refresh-'));
    const repoDir = path.join(workspaceRoot, REPO_NAME);
    fs.mkdirSync(repoDir);
    gitInit(repoDir);
  });

  after(() => {
    if (workspaceRoot && fs.existsSync(workspaceRoot)) {
      fs.rmSync(workspaceRoot, { recursive: true });
    }
  });

  it('reports hasUnstaged after mutating a tracked file', async () => {
    fs.appendFileSync(path.join(workspaceRoot, REPO_NAME, 'README.md'), 'dirty\n', 'utf-8');
    const snapshot = await refreshRepoSnapshot(workspaceRoot, REPO_NAME);
    assert.ok(snapshot, 'expected a RepoSnapshot');
    assert.equal(snapshot.hasUnstaged, true);
  });

  it('preserves checkoutKind and primaryRepo when meta is passed', async () => {
    const snapshot = await refreshRepoSnapshot(workspaceRoot, REPO_NAME, undefined, {
      checkoutKind: 'linked',
      primaryRepo: 'parent-repo',
    });
    assert.ok(snapshot, 'expected a RepoSnapshot');
    assert.equal(snapshot.checkoutKind, 'linked');
    assert.equal(snapshot.primaryRepo, 'parent-repo');
  });
});
