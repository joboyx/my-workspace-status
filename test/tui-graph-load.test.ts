import assert from 'node:assert';
import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import { DEFAULT_GRAPH_WINDOW } from '../src/tui/graph/types.js';
import { loadGraphModel } from '../src/tui/graph/load.js';
import { gitLogGraphWindow } from '../src/git.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'load.test',
  GIT_AUTHOR_EMAIL: 'load.test@example.invalid',
  GIT_COMMITTER_NAME: 'load.test',
  GIT_COMMITTER_EMAIL: 'load.test@example.invalid',
  ...process.env,
};

let repoDir = '';

describe('loadGraphModel', () => {
  before(() => {
    repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-graph-load-'));
    execSync(`git init -q -b main "${repoDir}"`, { stdio: 'pipe', env: GIT_ENV });
    fs.writeFileSync(path.join(repoDir, 'f.txt'), 'a\n');
    execSync(`git -C "${repoDir}" add f.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" commit -q -m "first"`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" branch feature`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" tag v0`, { stdio: 'pipe', env: GIT_ENV });
  });

  after(() => {
    if (repoDir) fs.rmSync(repoDir, { recursive: true, force: true });
  });

  it('loads commits with local + tag refs attached', async () => {
    const model = await loadGraphModel(repoDir);
    assert.equal(model.repoPath, repoDir);
    assert.ok(model.commits.length >= 1);
    assert.equal(model.skip, 0);
    assert.equal(model.limit, DEFAULT_GRAPH_WINDOW);
    assert.equal(model.hasMore, false);
    assert.ok(model.refsFingerprint.length > 0);
    assert.ok(model.headId);
    assert.equal(model.headId, model.commits[0]!.id);
    const tip = model.commits[0];
    const kinds = new Set(tip.refs.map((r) => r.kind));
    assert.ok(kinds.has('local'));
    assert.ok(tip.refs.some((r) => r.name === 'main' || r.name === 'feature'));
    assert.ok(tip.refs.some((r) => r.kind === 'tag' && r.name === 'v0'));
  });

  it('reports uncommitted when dirty', async () => {
    fs.appendFileSync(path.join(repoDir, 'f.txt'), 'x\n');
    const model = await loadGraphModel(repoDir);
    assert.ok(model.uncommitted);
    assert.equal(model.uncommitted.hasChanges, true);
    // cleanup for other tests
    execSync(`git -C "${repoDir}" checkout -- f.txt`, { stdio: 'pipe', env: GIT_ENV });
  });

  it('respects skip/limit window', async () => {
    // add more commits
    for (let i = 0; i < 5; i++) {
      fs.writeFileSync(path.join(repoDir, 'f.txt'), `c${i}\n`);
      execSync(`git -C "${repoDir}" add f.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${repoDir}" commit -q -m "c${i}"`, { stdio: 'pipe', env: GIT_ENV });
    }
    const page = await loadGraphModel(repoDir, { skip: 0, limit: 3 });
    assert.equal(page.commits.length, 3);
    assert.equal(page.hasMore, true);
    const page2 = await loadGraphModel(repoDir, { skip: 3, limit: 3 });
    assert.ok(page2.commits.length >= 1);
    assert.notEqual(page2.commits[0].id, page.commits[0].id);
  });
});

describe('loadGraphModel stash^1 outside the log window', () => {
  let stashRepo = '';

  before(() => {
    stashRepo = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-graph-stash-parent-'));
    execSync(`git init -q -b main "${stashRepo}"`, { stdio: 'pipe', env: GIT_ENV });
    fs.writeFileSync(path.join(stashRepo, 'f.txt'), 'base\n');
    execSync(`git -C "${stashRepo}" add f.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${stashRepo}" commit -q -m "old-parent"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    fs.appendFileSync(path.join(stashRepo, 'f.txt'), 'stash-me\n');
    execSync(`git -C "${stashRepo}" stash push -m "wip-old" -- f.txt`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    for (let i = 0; i < 8; i++) {
      fs.writeFileSync(path.join(stashRepo, 'f.txt'), `c${i}\n`);
      execSync(`git -C "${stashRepo}" add f.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${stashRepo}" commit -q -m "c${i}"`, {
        stdio: 'pipe',
        env: GIT_ENV,
      });
    }
  });

  after(() => {
    if (stashRepo) fs.rmSync(stashRepo, { recursive: true, force: true });
  });

  it('fetches missing stash^1 into the model without changing limit/hasMore', async () => {
    const model = await loadGraphModel(stashRepo, { skip: 0, limit: 3 });
    const page = await gitLogGraphWindow(stashRepo, { skip: 0, limit: 3 });
    assert.equal(model.limit, 3);
    assert.equal(model.hasMore, true);
    assert.equal(model.windowCount, 3);
    assert.equal(model.windowCount, page.commits.length);
    assert.ok(model.stashes.length >= 1);
    const parentId = model.stashes[0]!.parentId;
    assert.ok(parentId.length > 0);
    const windowIds = new Set(page.commits.map((c) => c.id));
    assert.equal(windowIds.has(parentId), false, 'fixture parent must sit outside the log window');
    assert.ok(
      model.commits.some((c) => c.id === parentId),
      'stash^1 outside the log window must be loaded so the tip can park',
    );
    assert.ok(model.commits.length > 3);
    assert.equal(
      model.commits.slice(0, model.windowCount).some((c) => c.id === parentId),
      false,
      'extra stash parent must sit after the log-window prefix',
    );
  });
});
