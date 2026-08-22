/**
 * Hermetic smoke check for scripts/seed-demo-workspace.sh.
 * Seeds a temp dir, then asserts git state and --plain / --json.
 */

import assert from 'node:assert';
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.join(__dirname, '..');
const SEED_SCRIPT = path.join(REPO_ROOT, 'scripts', 'seed-demo-workspace.sh');
const WORKSPACE_STATUS_SCRIPT =
  process.env.WORKSPACE_STATUS_SCRIPT ?? path.join(REPO_ROOT, 'workspace-status.sh');

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'workspace-status e2e',
  GIT_AUTHOR_EMAIL: 'workspace-status-e2e@example.invalid',
  GIT_COMMITTER_NAME: 'workspace-status e2e',
  GIT_COMMITTER_EMAIL: 'workspace-status-e2e@example.invalid',
  ...process.env,
};

let dest = '';

function git(repo: string, args: string[]): string {
  return execFileSync('git', ['-C', repo, ...args], {
    encoding: 'utf-8',
    env: GIT_ENV,
  }).trim();
}

function runStatus(...args: string[]): string {
  return execFileSync(WORKSPACE_STATUS_SCRIPT, args, {
    cwd: dest,
    encoding: 'utf-8',
    env: { ...GIT_ENV, TERM: 'dumb' },
  });
}

describe('seed-demo-workspace.sh', () => {
  before(() => {
    dest = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-demo-seed.'));
    execFileSync(SEED_SCRIPT, [dest], { encoding: 'utf-8' });
  });

  after(() => {
    if (dest && process.env.KEEP_E2E_WORKDIR !== '1') {
      fs.rmSync(dest, { recursive: true, force: true });
    }
  });

  it('builds the shop workspace with the required git states', () => {
    assert.equal(git(path.join(dest, 'app'), ['branch', '--show-current']), 'feature/checkout');
    assert.match(git(path.join(dest, 'app'), ['status', '--short', '--branch']), /ahead 1/);
    const appPorcelain = git(path.join(dest, 'app'), ['status', '--porcelain']);
    assert.match(appPorcelain, /^M  src\/app\.ts$/m);
    assert.match(appPorcelain, /^ M src\/checkout\.ts$/m);
    assert.match(appPorcelain, /^\?\? src\/draft-banner\.ts$/m);

    const worktrees = git(path.join(dest, 'app'), ['worktree', 'list']);
    assert.match(worktrees, /feat-login/);
    assert.match(worktrees, /\[feature\/login\]/);

    assert.equal(git(path.join(dest, 'services/api'), ['branch', '--show-current']), 'feature/orders');
    assert.match(git(path.join(dest, 'services/api'), ['status', '--short', '--branch']), /ahead 1, behind 1/);
    assert.match(git(path.join(dest, 'services/api'), ['status', '--porcelain']), /src\/server\.ts/);

    assert.equal(git(path.join(dest, 'lib'), ['branch', '--show-current']), 'main');
    assert.equal(git(path.join(dest, 'lib'), ['status', '--porcelain']), '');

    assert.equal(git(path.join(dest, 'notes'), ['branch', '--show-current']), 'main');
    assert.match(git(path.join(dest, 'notes'), ['status', '--porcelain']), /standup\.md/);

    assert.equal(git(path.join(dest, 'merger'), ['branch', '--show-current']), 'feature/release-cut');
    const parents = git(path.join(dest, 'merger'), ['rev-list', '--parents', '-n', '1', 'HEAD']).split(' ');
    assert.ok(parents.length >= 3, 'expected a merge commit on merger HEAD');
    assert.match(git(path.join(dest, 'merger'), ['stash', 'list']), /wip: rate limit/);

    const config = JSON.parse(
      fs.readFileSync(path.join(dest, '.workspace-status-config.json'), 'utf-8'),
    );
    assert.deepEqual(config.ignoredRepos, ['notes']);
    assert.equal(config.maxDepth, 3);
  });

  it('hides notes in --plain / --json and shows dirty app plus the worktree', () => {
    const snapshot = JSON.parse(runStatus('--json'));
    const names = snapshot.repos.map((r: { repo: string }) => r.repo);
    assert.deepEqual(
      names.filter((n: string) => n === 'notes'),
      [],
    );
    assert.ok(names.includes('app'));
    assert.ok(names.includes('app/.worktrees/feat-login'));
    assert.ok(names.includes('services/api'));
    assert.ok(names.includes('lib'));
    assert.ok(names.includes('merger'));

    const lib = snapshot.repos.find((r: { repo: string }) => r.repo === 'lib');
    assert.equal(lib.hasUnstaged, false);
    assert.equal(lib.hasStaged, false);
    assert.equal(lib.hasUntracked, false);
    assert.equal(lib.branch, 'main');
    assert.equal(lib.syncStatus, 'up-to-date');

    const app = snapshot.repos.find((r: { repo: string }) => r.repo === 'app');
    assert.equal(app.hasUnstaged, true);
    assert.equal(app.hasStaged, true);
    assert.equal(app.hasUntracked, true);
    assert.equal(app.syncStatus, 'ahead');

    const api = snapshot.repos.find((r: { repo: string }) => r.repo === 'services/api');
    assert.equal(api.hasUnstaged, true);
    assert.equal(api.syncStatus, 'diverged');

    const plain = runStatus('--plain');
    assert.match(plain, /app/);
    assert.match(plain, /feat-login/);
    assert.match(plain, /services\/api/);
    assert.match(plain, /merger/);
    assert.doesNotMatch(plain, /\bnotes\b/);
  });

  it('includes notes when --all is set', () => {
    const snapshot = JSON.parse(runStatus('--json', '--all'));
    const notes = snapshot.repos.find((r: { repo: string }) => r.repo === 'notes');
    assert.ok(notes);
    assert.equal(notes.ignored, true);
    assert.equal(notes.hasUnstaged, true);
    assert.match(runStatus('--plain', '--all'), /notes/);
  });
});
