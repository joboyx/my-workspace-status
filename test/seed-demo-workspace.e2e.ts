/**
 * Smoke check for scripts/seed-demo-workspace.sh.
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
  }).replace(/\n+$/, '');
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

  it('builds the demo workspace states', () => {
    const app = path.join(dest, 'app');
    assert.equal(git(app, ['branch', '--show-current']), 'feature/auth-refresh');
    assert.match(git(app, ['status', '--short', '--branch']), /ahead 1/);
    const appPorcelain = git(app, ['status', '--porcelain']);
    assert.match(appPorcelain, /^ M src\/auth\.ts$/m);
    assert.match(appPorcelain, /^M  src\/session\.ts$/m);
    assert.match(appPorcelain, /^\?\? src\/login\.ts$/m);
    assert.match(git(app, ['stash', 'list']), /WIP: in-memory token cache/);
    const worktrees = git(app, ['worktree', 'list']);
    assert.match(worktrees, /feat-login/);
    assert.match(worktrees, /\[feature\/login-page\]/);

    const first = git(app, ['log', '--reverse', '--format=%at %an <%ae> %s']).split('\n')[0];
    assert.equal(first, '1786928400 Demo User <demo@example.invalid> seed app client');
    assert.match(fs.readFileSync(path.join(app, 'src/auth.ts'), 'utf-8'), /withRefreshedExpiry/);

    const api = path.join(dest, 'services/api');
    assert.equal(git(api, ['branch', '--show-current']), 'feature/rate-limit');
    assert.match(git(api, ['status', '--short', '--branch']), /ahead 1, behind 1/);
    assert.match(git(api, ['status', '--porcelain']), /src\/server\.ts/);

    const lib = path.join(dest, 'lib');
    assert.equal(git(lib, ['branch', '--show-current']), 'main');
    assert.equal(git(lib, ['status', '--porcelain']), '');

    const notes = path.join(dest, 'notes');
    assert.equal(git(notes, ['branch', '--show-current']), 'main');
    assert.match(git(notes, ['status', '--porcelain']), /inbox\.md/);

    const merger = path.join(dest, 'merger');
    assert.equal(git(merger, ['branch', '--show-current']), 'feature/reconciliation');
    const parents = git(merger, ['rev-list', '--parents', '-n', '1', 'HEAD^']).split(' ');
    assert.ok(parents.length >= 3, 'expected a merge commit on merger');
    assert.match(git(merger, ['log', '--oneline', '--decorate']), /merge billing into main/);
    assert.match(git(merger, ['stash', 'list']), /WIP: reconcile totals/);

    const config = JSON.parse(
      fs.readFileSync(path.join(dest, '.workspace-status-config.json'), 'utf-8'),
    );
    assert.deepEqual(config.ignoredRepos, ['notes']);
    assert.equal(config.editor, 'vim');
    assert.ok(fs.existsSync(path.join(dest, '.remotes', 'app.git')));
    assert.ok(fs.statSync(path.join(dest, '.scratch')).isDirectory());
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
    assert.equal(app.branch, 'feature/auth-refresh');
    assert.equal(app.hasUnstaged, true);
    assert.equal(app.hasStaged, true);
    assert.equal(app.hasUntracked, true);
    assert.equal(app.syncStatus, 'ahead');
    const appFiles = (app.changes ?? []).map((f: { path: string }) => f.path);
    assert.ok(appFiles.includes('src/auth.ts'));

    const api = snapshot.repos.find((r: { repo: string }) => r.repo === 'services/api');
    assert.equal(api.branch, 'feature/rate-limit');
    assert.equal(api.hasUnstaged, true);
    assert.equal(api.syncStatus, 'diverged');

    const merger = snapshot.repos.find((r: { repo: string }) => r.repo === 'merger');
    assert.equal(merger.branch, 'feature/reconciliation');

    const plain = runStatus('--plain');
    assert.match(plain, /app/);
    assert.match(plain, /auth\.ts/);
    assert.match(plain, /feat-login/);
    assert.match(plain, /services\/api/);
    assert.match(plain, /merger/);
    assert.match(plain, /feature\/auth-refresh/);
    assert.match(plain, /feature\/reconciliation/);
    assert.doesNotMatch(plain, /\bnotes\b/);
  });

  it('includes notes when --all is set', () => {
    const snapshot = JSON.parse(runStatus('--json', '--all'));
    const notes = snapshot.repos.find((r: { repo: string }) => r.repo === 'notes');
    assert.ok(notes);
    assert.equal(notes.ignored, true);
    assert.equal(notes.hasUnstaged, true);
    const plainAll = runStatus('--plain', '--all');
    assert.match(plainAll, /notes/);
    assert.match(plainAll, /inbox\.md/);
  });
});
