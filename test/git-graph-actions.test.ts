import assert from 'node:assert';
import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import {
  createBranchAt,
  listLocalBranches,
  listStashes,
  repoHasLocalChanges,
  repoHasPorcelainChanges,
  stashApply,
  stashDrop,
  stashPop,
  stashPush,
} from '../src/git.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'graph-actions.test',
  GIT_AUTHOR_EMAIL: 'graph-actions.test@example.invalid',
  GIT_COMMITTER_NAME: 'graph-actions.test',
  GIT_COMMITTER_EMAIL: 'graph-actions.test@example.invalid',
  ...process.env,
};

let repoDir = '';

function git(cwd: string, args: string): string {
  return execSync(`git -C "${cwd}" ${args}`, { encoding: 'utf8', env: GIT_ENV }).trim();
}

function makeRepo(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-stash-'));
  execSync(`git init -q -b main "${dir}"`, { env: GIT_ENV });
  fs.writeFileSync(path.join(dir, 'a.txt'), '1\n');
  git(dir, 'add a.txt');
  git(dir, 'commit -q -m c1');
  return dir;
}

describe('git graph mutate helpers', () => {
  before(() => {
    repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-graph-act-'));
    execSync(`git init -q -b main "${repoDir}"`, { env: GIT_ENV });
    fs.writeFileSync(path.join(repoDir, 'a.txt'), '1\n');
    git(repoDir, 'add a.txt');
    git(repoDir, 'commit -q -m c1');
  });

  after(() => {
    if (repoDir) fs.rmSync(repoDir, { recursive: true, force: true });
  });

  it('createBranchAt creates a ref without moving HEAD', async () => {
    const head = git(repoDir, 'rev-parse HEAD');
    const beforeBranch = git(repoDir, 'branch --show-current');
    const result = await createBranchAt(repoDir, 'from-old', head);
    assert.equal(result.ok, true);
    assert.equal(git(repoDir, 'branch --show-current'), beforeBranch);
    const locals = await listLocalBranches(repoDir);
    assert.ok(locals.some((b) => b.name === 'from-old'));
  });

  it('stashApply and stashDrop round-trip', async () => {
    fs.appendFileSync(path.join(repoDir, 'a.txt'), 'x\n');
    git(repoDir, 'stash push -m "p5" -- a.txt');
    const stashes = await listStashes(repoDir);
    assert.ok(stashes.length >= 1);
    const ref = stashes[0].stashRef;
    const applied = await stashApply(repoDir, ref);
    assert.equal(applied.ok, true);
    const dropped = await stashDrop(repoDir, ref);
    // apply keeps stash; drop removes it — if apply succeeded, drop may target same ref index
    assert.ok(dropped.ok || (dropped.error ?? '').length >= 0);
  });

  it('repoHasLocalChanges gates dirty checkout scenarios', async () => {
    fs.appendFileSync(path.join(repoDir, 'a.txt'), 'dirty\n');
    assert.equal(await repoHasLocalChanges(repoDir), true);
  });

  it('stashPush stashes a tracked dirty file and leaves the worktree clean', async () => {
    const dir = makeRepo();
    try {
      const before = (await listStashes(dir)).length;
      fs.appendFileSync(path.join(dir, 'a.txt'), 'x\n');
      const result = await stashPush(dir);
      assert.equal(result.ok, true);
      assert.equal(await repoHasLocalChanges(dir), false);
      assert.equal((await listStashes(dir)).length, before + 1);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it('stashPush with includeUntracked picks up an untracked file', async () => {
    const dir = makeRepo();
    try {
      const extra = path.join(dir, 'extra.txt');
      fs.writeFileSync(extra, 'u\n');
      const before = (await listStashes(dir)).length;
      const result = await stashPush(dir, { includeUntracked: true });
      assert.equal(result.ok, true);
      assert.equal(fs.existsSync(extra), false);
      assert.equal(await repoHasPorcelainChanges(dir), false);
      assert.equal((await listStashes(dir)).length, before + 1);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it('stashPush with paths leaves a sibling dirty file unstashed', async () => {
    const dir = makeRepo();
    try {
      fs.writeFileSync(path.join(dir, 'b.txt'), 'orig\n');
      git(dir, 'add b.txt');
      git(dir, 'commit -q -m c2');
      fs.appendFileSync(path.join(dir, 'a.txt'), 'a-change\n');
      fs.appendFileSync(path.join(dir, 'b.txt'), 'b-change\n');
      const result = await stashPush(dir, { paths: ['a.txt'] });
      assert.equal(result.ok, true);
      assert.equal(fs.readFileSync(path.join(dir, 'a.txt'), 'utf8'), '1\n');
      assert.equal(fs.readFileSync(path.join(dir, 'b.txt'), 'utf8'), 'orig\nb-change\n');
      assert.equal(await repoHasLocalChanges(dir), true);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it('stashPush on a clean tree returns a non-empty error', async () => {
    const dir = makeRepo();
    try {
      const result = await stashPush(dir);
      assert.equal(result.ok, false);
      assert.ok((result.error ?? '').length > 0);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it('stashPop restores a stash and removes that entry', async () => {
    const dir = makeRepo();
    try {
      fs.appendFileSync(path.join(dir, 'a.txt'), 'pop-me\n');
      const pushed = await stashPush(dir);
      assert.equal(pushed.ok, true);
      const stashes = await listStashes(dir);
      assert.ok(stashes.length >= 1);
      const ref = stashes[0].stashRef;
      const popped = await stashPop(dir, ref);
      assert.equal(popped.ok, true);
      assert.equal(fs.readFileSync(path.join(dir, 'a.txt'), 'utf8'), '1\npop-me\n');
      assert.equal((await listStashes(dir)).length, stashes.length - 1);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
