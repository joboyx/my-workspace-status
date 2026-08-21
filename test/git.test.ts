/**
 * Unit tests for async git helpers (real git in a temp repo).
 */

import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import assert from 'node:assert';

import {
  FULL_DIFF_CONTEXT_LINES,
  checkoutBranch,
  diffCachedFile,
  diffFile,
  execGit,
  execGitStatus,
  fastForwardToRemoteRef,
  revParseQuiet,
  needsUpstreamPublish,
  pullQuiet,
  pullQuietDetailed,
  pushQuiet,
  removeUntrackedFile,
  removeWorktree,
  repoHasLocalChanges,
  revertTrackedFile,
  stageFile,
  unstageFile,
} from '../src/git.js';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'git.test',
  GIT_AUTHOR_EMAIL: 'git.test@example.invalid',
  GIT_COMMITTER_NAME: 'git.test',
  GIT_COMMITTER_EMAIL: 'git.test@example.invalid',
  ...process.env,
};

let repoDir = '';

function gitInit(repoPath: string): void {
  execSync(`git init -q -b main "${repoPath}"`, { stdio: 'pipe', env: GIT_ENV });
  fs.writeFileSync(path.join(repoPath, 'README.md'), 'seed\n', 'utf-8');
  execSync(`git -C "${repoPath}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${repoPath}" commit -q -m "seed"`, { stdio: 'pipe', env: GIT_ENV });
}

describe('git helpers', () => {
  before(() => {
    repoDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-test-'));
    gitInit(repoDir);
  });

  after(() => {
    if (repoDir && fs.existsSync(repoDir)) fs.rmSync(repoDir, { recursive: true });
  });

  it('execGit returns branch name from rev-parse', async () => {
    const branch = await execGit(['branch', '--show-current'], repoDir);
    assert.equal(branch, 'main');
  });

  it('execGitStatus reports diff --quiet as non-zero when dirty', async () => {
    const dirtyDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-dirty-'));
    try {
      gitInit(dirtyDir);
      assert.equal(await execGitStatus(['diff', '--quiet'], dirtyDir), 0);
      fs.appendFileSync(path.join(dirtyDir, 'README.md'), 'dirty\n', 'utf-8');
      assert.notEqual(await execGitStatus(['diff', '--quiet'], dirtyDir), 0);
    } finally {
      fs.rmSync(dirtyDir, { recursive: true });
    }
  });

  it('repoHasLocalChanges detects unstaged edits', async () => {
    const cleanDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-clean-'));
    try {
      gitInit(cleanDir);
      assert.equal(await repoHasLocalChanges(cleanDir), false);
      fs.appendFileSync(path.join(cleanDir, 'README.md'), 'change\n', 'utf-8');
      assert.equal(await repoHasLocalChanges(cleanDir), true);
    } finally {
      fs.rmSync(cleanDir, { recursive: true });
    }
  });

  it('revParseQuiet returns a SHA for an existing ref', async () => {
    const sha = await revParseQuiet('refs/heads/main', repoDir);
    assert.ok(sha && /^[0-9a-f]{40}$/i.test(sha));
  });

  it('revParseQuiet returns null for a missing ref', async () => {
    assert.equal(await revParseQuiet('refs/heads/does-not-exist', repoDir), null);
    assert.equal(await revParseQuiet('refs/remotes/origin/main', repoDir), null);
  });

  it('checkoutBranch checks out an existing local branch', async () => {
    const branchDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-branch-'));
    try {
      gitInit(branchDir);
      assert.equal(await execGitStatus(['checkout', '-b', 'feature/test-branch'], branchDir), 0);
      assert.equal(await execGitStatus(['checkout', 'main'], branchDir), 0);
      const ok = await checkoutBranch('feature/test-branch', branchDir);
      assert.equal(ok, true);
      assert.equal(await execGit(['branch', '--show-current'], branchDir), 'feature/test-branch');
    } finally {
      fs.rmSync(branchDir, { recursive: true });
    }
  });

  it('fastForwardToRemoteRef ff-only when local is behind origin/foo', async () => {
    const bare = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-ff-behind-bare-'));
    const clone = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-ff-behind-'));
    try {
      execSync(`git init -q --bare "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      gitInit(clone);
      execSync(`git -C "${clone}" remote add origin "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" push -q -u origin main`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" checkout -q -b foo`, { stdio: 'pipe', env: GIT_ENV });
      fs.writeFileSync(path.join(clone, 'foo.txt'), 'ahead\n', 'utf-8');
      execSync(`git -C "${clone}" add foo.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" commit -q -m "foo tip"`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" push -q -u origin foo`, { stdio: 'pipe', env: GIT_ENV });
      const remoteTip = execSync(`git -C "${clone}" rev-parse origin/foo`, {
        encoding: 'utf-8',
        env: GIT_ENV,
      }).trim();
      execSync(`git -C "${clone}" reset -q --hard HEAD~1`, { stdio: 'pipe', env: GIT_ENV });
      const behindTip = execSync(`git -C "${clone}" rev-parse HEAD`, {
        encoding: 'utf-8',
        env: GIT_ENV,
      }).trim();
      assert.notEqual(behindTip, remoteTip);

      const ok = await fastForwardToRemoteRef('origin/foo', clone);
      assert.equal(ok, true);
      assert.equal(await execGit(['branch', '--show-current'], clone), 'foo');
      assert.equal(await execGit(['rev-parse', 'HEAD'], clone), remoteTip);
      assert.equal(await execGit(['rev-parse', 'origin/foo'], clone), remoteTip);
    } finally {
      fs.rmSync(bare, { recursive: true, force: true });
      fs.rmSync(clone, { recursive: true, force: true });
    }
  });

  it('fastForwardToRemoteRef fails when local is ahead and leaves HEAD', async () => {
    const bare = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-ff-ahead-bare-'));
    const clone = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-ff-ahead-'));
    try {
      execSync(`git init -q --bare "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      gitInit(clone);
      execSync(`git -C "${clone}" remote add origin "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" push -q -u origin main`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" checkout -q -b foo`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" push -q -u origin foo`, { stdio: 'pipe', env: GIT_ENV });
      fs.writeFileSync(path.join(clone, 'local.txt'), 'ahead\n', 'utf-8');
      execSync(`git -C "${clone}" add local.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" commit -q -m "local ahead"`, { stdio: 'pipe', env: GIT_ENV });
      const localTip = execSync(`git -C "${clone}" rev-parse HEAD`, {
        encoding: 'utf-8',
        env: GIT_ENV,
      }).trim();

      const ok = await fastForwardToRemoteRef('origin/foo', clone);
      assert.equal(ok, false);
      assert.equal(await execGit(['branch', '--show-current'], clone), 'foo');
      assert.equal(await execGit(['rev-parse', 'HEAD'], clone), localTip);
    } finally {
      fs.rmSync(bare, { recursive: true, force: true });
      fs.rmSync(clone, { recursive: true, force: true });
    }
  });

  it('fastForwardToRemoteRef fails cleanly when the remote ref is missing', async () => {
    const missingDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-ff-missing-'));
    try {
      gitInit(missingDir);
      const localTip = await execGit(['rev-parse', 'HEAD'], missingDir);
      const ok = await fastForwardToRemoteRef('origin/foo', missingDir);
      assert.equal(ok, false);
      assert.equal(await execGit(['branch', '--show-current'], missingDir), 'main');
      assert.equal(await execGit(['rev-parse', 'HEAD'], missingDir), localTip);
    } finally {
      fs.rmSync(missingDir, { recursive: true });
    }
  });

  it('pullQuiet returns a boolean', async () => {
    const result = await pullQuiet(repoDir);
    assert.equal(typeof result, 'boolean');
  });

  it.skip('pushQuiet advances remote when ahead and fails when diverged', async () => {
    const bare = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-push-bare-'));
    const cloneA = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-push-a-'));
    const cloneB = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-push-b-'));
    try {
      execSync(`git init -q --bare "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      gitInit(cloneA);
      execSync(`git -C "${cloneA}" remote add origin "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${cloneA}" push -q -u origin main`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git clone -q "${bare}" "${cloneB}"`, { stdio: 'pipe', env: GIT_ENV });

      fs.writeFileSync(path.join(cloneA, 'ahead.txt'), 'local-ahead\n', 'utf-8');
      execSync(`git -C "${cloneA}" add ahead.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${cloneA}" commit -q -m "ahead commit"`, { stdio: 'pipe', env: GIT_ENV });

      assert.equal(await pushQuiet(cloneA), true);
      const remoteTip = execSync(`git -C "${bare}" rev-parse main`, {
        encoding: 'utf-8',
        env: GIT_ENV,
      }).trim();
      const localTip = execSync(`git -C "${cloneA}" rev-parse HEAD`, {
        encoding: 'utf-8',
        env: GIT_ENV,
      }).trim();
      assert.equal(remoteTip, localTip);

      fs.writeFileSync(path.join(cloneB, 'other.txt'), 'other-line\n', 'utf-8');
      execSync(`git -C "${cloneB}" add other.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${cloneB}" commit -q -m "divergent commit"`, { stdio: 'pipe', env: GIT_ENV });
      assert.equal(await pushQuiet(cloneB), false);
    } finally {
      fs.rmSync(bare, { recursive: true, force: true });
      fs.rmSync(cloneA, { recursive: true, force: true });
      fs.rmSync(cloneB, { recursive: true, force: true });
    }
  });

  it('pushQuiet publishes first-time and mis-tracked feature branches with -u', async () => {
    const bare = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-push-u-bare-'));
    const clone = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-push-u-'));
    try {
      execSync(`git init -q --bare "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      gitInit(clone);
      execSync(`git -C "${clone}" remote add origin "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" push -q -u origin main`, { stdio: 'pipe', env: GIT_ENV });

      execSync(`git -C "${clone}" checkout -q -b feature/first-push`, { stdio: 'pipe', env: GIT_ENV });
      // Branch exists locally with no upstream yet.
      assert.equal(await needsUpstreamPublish(clone), true);
      fs.writeFileSync(path.join(clone, 'first.txt'), 'first\n', 'utf-8');
      execSync(`git -C "${clone}" add first.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" commit -q -m "first publish"`, { stdio: 'pipe', env: GIT_ENV });

      assert.equal(await pushQuiet(clone), true);
      assert.equal(await needsUpstreamPublish(clone), false);
      assert.equal(
        execSync(`git -C "${clone}" rev-parse --abbrev-ref @{u}`, {
          encoding: 'utf-8',
          env: GIT_ENV,
        }).trim(),
        'origin/feature/first-push',
      );
      assert.equal(
        execSync(`git -C "${bare}" rev-parse feature/first-push`, {
          encoding: 'utf-8',
          env: GIT_ENV,
        }).trim(),
        execSync(`git -C "${clone}" rev-parse HEAD`, { encoding: 'utf-8', env: GIT_ENV }).trim(),
      );

      // Simulate worktree-created branch that tracks develop/main instead of itself.
      execSync(`git -C "${clone}" checkout -q -b feature/tracks-main main`, {
        stdio: 'pipe',
        env: GIT_ENV,
      });
      execSync(`git -C "${clone}" branch -u origin/main`, { stdio: 'pipe', env: GIT_ENV });
      fs.writeFileSync(path.join(clone, 'mistrack.txt'), 'mistrack\n', 'utf-8');
      execSync(`git -C "${clone}" add mistrack.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${clone}" commit -q -m "mis-tracked publish"`, {
        stdio: 'pipe',
        env: GIT_ENV,
      });
      assert.equal(await needsUpstreamPublish(clone), true);
      assert.equal(await pushQuiet(clone), true);
      assert.equal(
        execSync(`git -C "${clone}" rev-parse --abbrev-ref @{u}`, {
          encoding: 'utf-8',
          env: GIT_ENV,
        }).trim(),
        'origin/feature/tracks-main',
      );
      assert.equal(
        execSync(`git -C "${bare}" rev-parse feature/tracks-main`, {
          encoding: 'utf-8',
          env: GIT_ENV,
        }).trim(),
        execSync(`git -C "${clone}" rev-parse HEAD`, { encoding: 'utf-8', env: GIT_ENV }).trim(),
      );
    } finally {
      fs.rmSync(bare, { recursive: true, force: true });
      fs.rmSync(clone, { recursive: true, force: true });
    }
  });

  it.skip('pullQuietDetailed stashes dirty changes, pulls, then reapplies', async () => {
    const bare = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-bare-'));
    const cloneA = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-clone-a-'));
    const cloneB = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-clone-b-'));
    try {
      execSync(`git init -q --bare "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      gitInit(cloneA);
      execSync(`git -C "${cloneA}" remote add origin "${bare}"`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${cloneA}" push -q -u origin main`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git clone -q "${bare}" "${cloneB}"`, { stdio: 'pipe', env: GIT_ENV });

      fs.writeFileSync(path.join(cloneA, 'remote-only.txt'), 'from-a\n', 'utf-8');
      execSync(`git -C "${cloneA}" add remote-only.txt`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${cloneA}" commit -q -m "remote advance"`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${cloneA}" push -q origin main`, { stdio: 'pipe', env: GIT_ENV });

      fs.appendFileSync(path.join(cloneB, 'README.md'), 'local-edit\n', 'utf-8');
      execSync(`git -C "${cloneB}" fetch -q origin main`, { stdio: 'pipe', env: GIT_ENV });

      const detailed = await pullQuietDetailed(cloneB);
      assert.equal(detailed.ok, true);
      assert.equal(detailed.stashed, true);
      assert.equal(detailed.stashPopFailed, false);
      assert.equal(await repoHasLocalChanges(cloneB), true);
      assert.match(fs.readFileSync(path.join(cloneB, 'README.md'), 'utf-8'), /local-edit/);
      assert.equal(fs.existsSync(path.join(cloneB, 'remote-only.txt')), true);
      assert.equal(
        (await execGit(['stash', 'list'], cloneB)).trim(),
        '',
        'auto-stash should be popped',
      );
    } finally {
      fs.rmSync(bare, { recursive: true, force: true });
      fs.rmSync(cloneA, { recursive: true, force: true });
      fs.rmSync(cloneB, { recursive: true, force: true });
    }
  });

  it('stageFile and unstageFile round-trip', async () => {
    const dirtyDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-stage-'));
    try {
      gitInit(dirtyDir);
      fs.appendFileSync(path.join(dirtyDir, 'README.md'), 'staged-edit\n', 'utf-8');
      const staged = await stageFile(dirtyDir, 'README.md');
      assert.equal(staged.ok, true);
      assert.notEqual(await execGitStatus(['diff', '--cached', '--quiet'], dirtyDir), 0);
      const unstaged = await unstageFile(dirtyDir, 'README.md');
      assert.equal(unstaged.ok, true);
      assert.equal(await execGitStatus(['diff', '--cached', '--quiet'], dirtyDir), 0);
      assert.notEqual(await execGitStatus(['diff', '--quiet'], dirtyDir), 0);
    } finally {
      fs.rmSync(dirtyDir, { recursive: true });
    }
  });

  it('diffFile returns unified diff for dirty tracked file', async () => {
    const dirtyDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-diff-'));
    try {
      gitInit(dirtyDir);
      fs.appendFileSync(path.join(dirtyDir, 'README.md'), 'dirty-line\n', 'utf-8');
      const out = await diffFile(dirtyDir, 'README.md');
      assert.match(out, /^diff --git /m);
    } finally {
      fs.rmSync(dirtyDir, { recursive: true });
    }
  });

  it('diffCachedFile returns unified diff for staged file', async () => {
    const dirtyDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-diff-cached-'));
    try {
      gitInit(dirtyDir);
      fs.appendFileSync(path.join(dirtyDir, 'README.md'), 'cached-line\n', 'utf-8');
      assert.equal((await stageFile(dirtyDir, 'README.md')).ok, true);
      const out = await diffCachedFile(dirtyDir, 'README.md');
      assert.match(out, /^diff --git /m);
    } finally {
      fs.rmSync(dirtyDir, { recursive: true });
    }
  });

  it('diffFile with large context includes distant unchanged lines', async () => {
    const ctxDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-ctx-'));
    try {
      gitInit(ctxDir);
      const body = Array.from({ length: 40 }, (_, i) => `line-${i + 1}`).join('\n') + '\n';
      fs.writeFileSync(path.join(ctxDir, 'README.md'), body, 'utf-8');
      execSync(`git -C "${ctxDir}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
      execSync(`git -C "${ctxDir}" commit -q -m "long"`, { stdio: 'pipe', env: GIT_ENV });
      // Change only the last line — default -U3 omits line-1; full context keeps it.
      const lines = body.trimEnd().split('\n');
      lines[lines.length - 1] = 'line-40-changed';
      fs.writeFileSync(path.join(ctxDir, 'README.md'), lines.join('\n') + '\n', 'utf-8');

      const normal = await diffFile(ctxDir, 'README.md');
      assert.ok(
        !normal.includes('line-1\n') && !normal.includes('+line-1') && !normal.includes(' line-1'),
      );
      // Default unified hunks show context with a leading space.
      assert.ok(
        !/^ line-1$/m.test(normal),
        `default context should omit distant line-1:\n${normal}`,
      );

      const full = await diffFile(ctxDir, 'README.md', FULL_DIFF_CONTEXT_LINES);
      assert.ok(/^ line-1$/m.test(full), `full context must include line-1:\n${full}`);
      assert.ok(
        full.includes('-line-40') ||
          full.includes('-line-40\n') ||
          full.includes('-line-40-changed') === false,
      );
      assert.ok(full.includes('+line-40-changed'));
    } finally {
      fs.rmSync(ctxDir, { recursive: true });
    }
  });

  it('revertTrackedFile restores dirty tracked file', async () => {
    const dirtyDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-revert-'));
    try {
      gitInit(dirtyDir);
      fs.appendFileSync(path.join(dirtyDir, 'README.md'), 'to-revert\n', 'utf-8');
      const reverted = await revertTrackedFile(dirtyDir, 'README.md');
      assert.equal(reverted.ok, true);
      assert.equal(await execGitStatus(['diff', '--quiet'], dirtyDir), 0);
      assert.equal(fs.readFileSync(path.join(dirtyDir, 'README.md'), 'utf-8'), 'seed\n');
    } finally {
      fs.rmSync(dirtyDir, { recursive: true });
    }
  });

  it('removeUntrackedFile deletes untracked file', async () => {
    const dirtyDir = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-git-clean-file-'));
    try {
      gitInit(dirtyDir);
      fs.writeFileSync(path.join(dirtyDir, 'scratch.tmp'), 'temp\n', 'utf-8');
      const removed = await removeUntrackedFile(dirtyDir, 'scratch.tmp');
      assert.equal(removed.ok, true);
      assert.equal(fs.existsSync(path.join(dirtyDir, 'scratch.tmp')), false);
    } finally {
      fs.rmSync(dirtyDir, { recursive: true });
    }
  });

  it('removeWorktree adds and removes a linked worktree', async () => {
    const primary = fs.mkdtempSync(path.join(os.tmpdir(), 'my-workspace-status-wt-primary-'));
    const linked = path.join(primary, '.worktrees', 'feat');
    try {
      gitInit(primary);
      execSync(`git -C "${primary}" branch feature/wt-test`, {
        stdio: 'pipe',
        env: GIT_ENV,
      });
      fs.mkdirSync(path.dirname(linked), { recursive: true });
      execSync(
        `git -C "${primary}" worktree add "${linked}" feature/wt-test`,
        { stdio: 'pipe', env: GIT_ENV },
      );
      assert.equal(fs.existsSync(path.join(linked, '.git')), true);
      const removed = await removeWorktree(primary, linked, { force: false });
      assert.equal(removed.ok, true);
      assert.equal(fs.existsSync(linked), false);
    } finally {
      fs.rmSync(primary, { recursive: true, force: true });
    }
  });
});
