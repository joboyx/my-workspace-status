import assert from 'node:assert';
import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import {
  computeRefsFingerprint,
  gitLogCommitsByIds,
  gitLogGraphWindow,
  listRefs,
  listStashes,
  repoHasPorcelainChanges,
} from '../src/git.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'git-graph.test',
  GIT_AUTHOR_EMAIL: 'git-graph.test@example.invalid',
  GIT_COMMITTER_NAME: 'git-graph.test',
  GIT_COMMITTER_EMAIL: 'git-graph.test@example.invalid',
  ...process.env,
};

let repoDir = '';

function git(cwd: string, args: string): void {
  execSync(`git -C "${cwd}" ${args}`, { stdio: 'pipe', env: GIT_ENV });
}

function initRepo(dir: string): void {
  execSync(`git init -q -b main "${dir}"`, { stdio: 'pipe', env: GIT_ENV });
  fs.writeFileSync(path.join(dir, 'a.txt'), '1\n');
  git(dir, 'add a.txt');
  git(dir, 'commit -q -m "c1"');
  fs.writeFileSync(path.join(dir, 'a.txt'), '2\n');
  git(dir, 'add a.txt');
  git(dir, 'commit -q -m "c2"');
  git(dir, 'checkout -q -b feature');
  fs.writeFileSync(path.join(dir, 'a.txt'), '3\n');
  git(dir, 'add a.txt');
  git(dir, 'commit -q -m "c3-feature"');
  git(dir, 'checkout -q main');
  git(dir, 'tag -a v1 -m "tag"');
  try {
    git(dir, 'stash push -u -m "wip" -- a.txt'); // may be empty; still ok if stash empty
  } catch {
    // no local changes to stash
  }
}

describe('git graph helpers', () => {
  before(() => {
    repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-git-graph-'));
    initRepo(repoDir);
    // Ensure at least one stash: dirty + stash
    fs.appendFileSync(path.join(repoDir, 'a.txt'), 'dirty\n');
    git(repoDir, 'stash push -m "wip-dirty" -- a.txt');
  });

  after(() => {
    if (repoDir) fs.rmSync(repoDir, { recursive: true, force: true });
  });

  it('gitLogGraphWindow returns commits with parents and respects limit', async () => {
    const page = await gitLogGraphWindow(repoDir, { skip: 0, limit: 2 });
    assert.equal(page.commits.length, 2);
    assert.equal(page.truncated, true);
    assert.ok(page.commits[0].id.length >= 7);
    assert.ok(Array.isArray(page.commits[0].parents));
    assert.ok(page.commits[0].subject.length > 0);
    assert.ok(Number.isFinite(page.commits[0].authorDateUnix));
  });

  it('gitLogGraphWindow hasMore false when under limit', async () => {
    const page = await gitLogGraphWindow(repoDir, { skip: 0, limit: 50 });
    assert.ok(page.commits.length < 50);
    assert.equal(page.truncated, false);
  });

  it('gitLogGraphWindow excludes stash WIP/index commits from history', async () => {
    const stashes = await listStashes(repoDir);
    assert.ok(stashes.length >= 1, 'fixture must have a stash');
    const stashIds = new Set(stashes.map((s) => s.id));

    const page = await gitLogGraphWindow(repoDir, { skip: 0, limit: 50 });
    for (const c of page.commits) {
      assert.ok(
        !stashIds.has(c.id),
        `stash tip ${c.id.slice(0, 7)} must not appear as a normal commit row`,
      );
      assert.ok(
        !/^(WIP|index|untracked files) on /.test(c.subject),
        `stash component subject leaked into graph: ${c.subject}`,
      );
    }
    // Branch commits still present.
    assert.ok(page.commits.some((c) => c.subject === 'c2'));
    assert.ok(page.commits.some((c) => c.subject === 'c3-feature'));
  });

  it('listRefs includes local, and tag when present', async () => {
    const refs = await listRefs(repoDir);
    const locals = refs.filter((r) => r.kind === 'local').map((r) => r.name);
    assert.ok(locals.includes('main'));
    assert.ok(locals.includes('feature'));
    const tags = refs.filter((r) => r.kind === 'tag').map((r) => r.name);
    assert.ok(tags.includes('v1'));
  });

  it('listStashes returns stash entries with stash^1 parentId', async () => {
    const stashes = await listStashes(repoDir);
    assert.ok(stashes.length >= 1);
    assert.match(stashes[0].stashRef, /^stash@\{/);
    assert.ok(stashes[0].id.length >= 7);
    assert.ok(stashes[0].parentId.length >= 7, 'parentId should be stash^1');
    const firstParent = execSync(
      `git -C "${repoDir}" rev-parse "${stashes[0].stashRef}^1"`,
      { encoding: 'utf8', env: GIT_ENV },
    ).trim();
    assert.equal(stashes[0].parentId, firstParent);
  });

  it('gitLogCommitsByIds resolves stash^1 and omits missing ids', async () => {
    const stashes = await listStashes(repoDir);
    assert.ok(stashes.length >= 1);
    const parentId = stashes[0]!.parentId;
    const found = await gitLogCommitsByIds(repoDir, [parentId, 'deadbeefdeadbeefdeadbeefdeadbeefdeadbeef']);
    assert.equal(found.length, 1);
    assert.equal(found[0]!.id, parentId);
    assert.ok(found[0]!.subject.length > 0);
    const empty = await gitLogCommitsByIds(repoDir, []);
    assert.deepEqual(empty, []);
  });

  it('computeRefsFingerprint changes when HEAD moves', async () => {
    const beforeFp = await computeRefsFingerprint(repoDir);
    git(repoDir, 'checkout -q feature');
    const afterFp = await computeRefsFingerprint(repoDir);
    assert.notEqual(beforeFp, afterFp);
    git(repoDir, 'checkout -q main');
  });

  it('repoHasPorcelainChanges detects dirty worktree', async () => {
    assert.equal(await repoHasPorcelainChanges(repoDir), false);
    fs.appendFileSync(path.join(repoDir, 'a.txt'), 'more\n');
    assert.equal(await repoHasPorcelainChanges(repoDir), true);
    git(repoDir, 'checkout -q -- a.txt');
  });
});
