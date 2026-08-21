import assert from 'node:assert';
import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import {
  diffCommitFile,
  diffStashFile,
  listCommitNameStatus,
  listStashNameStatus,
  listWorktreeNameStatus,
} from '../src/git.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'commit-files.test',
  GIT_AUTHOR_EMAIL: 'commit-files.test@example.invalid',
  GIT_COMMITTER_NAME: 'commit-files.test',
  GIT_COMMITTER_EMAIL: 'commit-files.test@example.invalid',
  ...process.env,
};

let repoDir = '';

function git(cwd: string, args: string): string {
  return execSync(`git -C "${cwd}" ${args}`, { encoding: 'utf8', env: GIT_ENV }).trim();
}

describe('git commit file helpers', () => {
  before(() => {
    repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-commit-files-'));
    execSync(`git init -q -b main "${repoDir}"`, { env: GIT_ENV });
    fs.writeFileSync(path.join(repoDir, 'a.txt'), '1\n');
    git(repoDir, 'add a.txt');
    git(repoDir, 'commit -q -m c1');
    fs.writeFileSync(path.join(repoDir, 'a.txt'), '2\n');
    fs.mkdirSync(path.join(repoDir, 'src'));
    fs.writeFileSync(path.join(repoDir, 'src/b.ts'), 'x\n');
    git(repoDir, 'add a.txt src/b.ts');
    git(repoDir, 'commit -q -m c2');
  });

  after(() => {
    if (repoDir) fs.rmSync(repoDir, { recursive: true, force: true });
  });

  it('listCommitNameStatus lists files from tip commit', async () => {
    const tip = git(repoDir, 'rev-parse HEAD');
    const files = await listCommitNameStatus(repoDir, tip);
    assert.ok(files.some((f) => f.path === 'a.txt' || f.path === 'src/b.ts'));
  });

  it('diffCommitFile returns unified diff for a path', async () => {
    const tip = git(repoDir, 'rev-parse HEAD');
    const diff = await diffCommitFile(repoDir, tip, 'a.txt');
    assert.match(diff, /diff --git|@@|2/);
  });

  it('listCommitNameStatus + diffCommitFile use --root / show --first-parent for root', async () => {
    const root = git(repoDir, 'rev-list --max-parents=0 HEAD');
    assert.ok(root, 'expected a root commit');
    const files = await listCommitNameStatus(repoDir, root);
    assert.ok(
      files.some((f) => f.path === 'a.txt'),
      `expected a.txt from root --root fallback, got ${JSON.stringify(files)}`,
    );
    const diff = await diffCommitFile(repoDir, root, 'a.txt');
    assert.match(diff, /a\.txt|^\+1$/m);
  });

  it('listCommitNameStatus + diffCommitFile use first-parent for merges', async () => {
    // Build: main has a.txt; side branch adds side.txt; merge side into main.
    git(repoDir, 'checkout -q -b side');
    fs.writeFileSync(path.join(repoDir, 'side.txt'), 'side\n');
    git(repoDir, 'add side.txt');
    git(repoDir, 'commit -q -m side');
    git(repoDir, 'checkout -q main');
    fs.writeFileSync(path.join(repoDir, 'main-only.txt'), 'main\n');
    git(repoDir, 'add main-only.txt');
    git(repoDir, 'commit -q -m main-only');
    git(repoDir, 'merge -q --no-ff side -m merge-side');
    const merge = git(repoDir, 'rev-parse HEAD');
    const parents = git(repoDir, 'rev-list --parents -n 1 HEAD').split(' ');
    assert.ok(parents.length >= 3, 'expected a merge commit with 2 parents');

    const files = await listCommitNameStatus(repoDir, merge);
    // First-parent diff: main..merge brings in side.txt (not main-only.txt).
    assert.ok(
      files.some((f) => f.path === 'side.txt'),
      `expected side.txt in first-parent merge files, got ${JSON.stringify(files)}`,
    );
    assert.ok(
      !files.some((f) => f.path === 'main-only.txt'),
      'main-only.txt is on first-parent history, not the merge diff',
    );

    const diff = await diffCommitFile(repoDir, merge, 'side.txt');
    assert.match(diff, /side\.txt|side/);
  });

  it('listWorktreeNameStatus sees dirty files', async () => {
    fs.appendFileSync(path.join(repoDir, 'a.txt'), 'dirty\n');
    const files = await listWorktreeNameStatus(repoDir);
    assert.ok(files.some((f) => f.path === 'a.txt'));
  });

  it('listStashNameStatus returns paths when a stash exists', async () => {
    fs.writeFileSync(path.join(repoDir, 'stash-me.txt'), 's\n');
    git(repoDir, 'add stash-me.txt');
    git(repoDir, 'stash push -m "p4-stash" -- stash-me.txt');
    const files = await listStashNameStatus(repoDir, 'stash@{0}');
    assert.ok(files.some((f) => f.path.includes('stash-me') || f.path === 'stash-me.txt'));
  });

  it('diffStashFile returns unified diff for a path in the stash', async () => {
    fs.writeFileSync(path.join(repoDir, 'stash-diff.txt'), 'before\n');
    git(repoDir, 'add stash-diff.txt');
    git(repoDir, 'commit -m "stash-diff base"');
    fs.writeFileSync(path.join(repoDir, 'stash-diff.txt'), 'after\n');
    git(repoDir, 'add stash-diff.txt');
    git(repoDir, 'stash push -m "p4-stash-diff" -- stash-diff.txt');
    const diff = await diffStashFile(repoDir, 'stash@{0}', 'stash-diff.txt');
    assert.match(diff, /diff --git/);
    assert.match(diff, /stash-diff\.txt/);
    assert.match(diff, /-before/);
    assert.match(diff, /\+after/);
  });
});
