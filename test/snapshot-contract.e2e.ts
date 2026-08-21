/**
 * Fixture e2e for the workspace snapshot contract.
 * Builds a temp workspace, then runs --plain and --json without a TTY.
 */

import assert from 'node:assert';
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WORKSPACE_STATUS_SCRIPT =
  process.env.WORKSPACE_STATUS_SCRIPT ?? path.join(__dirname, '..', 'workspace-status.sh');

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'workspace-status e2e',
  GIT_AUTHOR_EMAIL: 'workspace-status-e2e@example.invalid',
  GIT_COMMITTER_NAME: 'workspace-status e2e',
  GIT_COMMITTER_EMAIL: 'workspace-status-e2e@example.invalid',
  ...process.env,
};

let scenarioRoot = '';
let workspaceRoot = '';

function gitInit(repoPath: string, branchName: string): void {
  try {
    execFileSync('git', ['init', '-q', '-b', branchName, repoPath], { env: GIT_ENV });
  } catch {
    execFileSync('git', ['init', '-q', repoPath], { env: GIT_ENV });
    execFileSync('git', ['-C', repoPath, 'checkout', '-q', '-b', branchName], { env: GIT_ENV });
  }
}

function seedRepo(name: string, branchName: string, dirty = false): void {
  const repoPath = path.join(workspaceRoot, name);
  fs.mkdirSync(repoPath, { recursive: true });
  gitInit(repoPath, branchName);
  fs.writeFileSync(path.join(repoPath, 'README.md'), `# ${name}\n`, 'utf-8');
  execFileSync('git', ['-C', repoPath, 'add', 'README.md'], { env: GIT_ENV });
  execFileSync('git', ['-C', repoPath, 'commit', '-q', '-m', `seed ${name}`], { env: GIT_ENV });
  if (dirty) {
    fs.appendFileSync(path.join(repoPath, 'README.md'), 'dirty\n', 'utf-8');
  }
}

describe('workspace snapshot fixture e2e', () => {
  before(() => {
    scenarioRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'workspace-status-snapshot.'));
    workspaceRoot = path.join(scenarioRoot, 'workspace');
    fs.mkdirSync(workspaceRoot, { recursive: true });
    seedRepo('app', 'main', true);
    seedRepo('lib', 'main', false);
    seedRepo('notes', 'main', true);
    fs.writeFileSync(
      path.join(workspaceRoot, '.workspace-status-config.json'),
      `${JSON.stringify({ ignoredRepos: ['notes'] }, null, 2)}\n`,
      'utf-8',
    );
  });

  after(() => {
    if (scenarioRoot && process.env.KEEP_E2E_WORKDIR !== '1') {
      fs.rmSync(scenarioRoot, { recursive: true, force: true });
    }
  });

  it('prints --json and --plain from the same snapshot without a TTY', () => {
    const jsonOut = execFileSync(WORKSPACE_STATUS_SCRIPT, ['--json'], {
      cwd: workspaceRoot,
      encoding: 'utf-8',
      env: { ...GIT_ENV, TERM: 'dumb' },
    });
    const plainOut = execFileSync(WORKSPACE_STATUS_SCRIPT, ['--plain'], {
      cwd: workspaceRoot,
      encoding: 'utf-8',
      env: { ...GIT_ENV, TERM: 'dumb' },
    });

    const snapshot = JSON.parse(jsonOut);
    assert.equal(snapshot.version, 1);
    assert.equal(snapshot.showIgnored, false);
    assert.deepEqual(snapshot.filterRepos, []);
    assert.deepEqual(snapshot.ignoredRepos, ['notes']);
    assert.deepEqual(
      snapshot.repos.map((r: { repo: string }) => r.repo),
      ['app', 'lib'],
    );

    const app = snapshot.repos.find((r: { repo: string }) => r.repo === 'app');
    assert.ok(app);
    assert.equal(app.ignored, false);
    assert.equal(app.branch, 'main');
    assert.equal(app.syncStatus, 'no-upstream');
    assert.equal(app.checkoutKind, 'primary');
    assert.equal(app.hasUnstaged, true);
    assert.deepEqual(app.changes, [{ path: 'README.md', unstagedStatus: 'M' }]);

    const lib = snapshot.repos.find((r: { repo: string }) => r.repo === 'lib');
    assert.ok(lib);
    assert.equal(lib.hasUnstaged, false);
    assert.deepEqual(lib.changes, []);

    assert.ok(!snapshot.repos.some((r: { repo: string }) => r.repo === 'notes'));

    assert.match(plainOut, /File changes/);
    assert.match(plainOut, /app/);
    assert.match(plainOut, /README\.md/);
    assert.doesNotMatch(plainOut, /\bnotes\b/);
    assert.doesNotMatch(jsonOut, /^\s*🔄/);
  });

  it('includes ignored repos in --json --all and --plain --all', () => {
    const snapshot = JSON.parse(
      execFileSync(WORKSPACE_STATUS_SCRIPT, ['--json', '--all'], {
        cwd: workspaceRoot,
        encoding: 'utf-8',
        env: { ...GIT_ENV, TERM: 'dumb' },
      }),
    );
    const notes = snapshot.repos.find((r: { repo: string }) => r.repo === 'notes');
    assert.ok(notes);
    assert.equal(notes.ignored, true);
    assert.equal(snapshot.showIgnored, true);
    assert.equal(notes.hasUnstaged, true);

    const plainOut = execFileSync(WORKSPACE_STATUS_SCRIPT, ['--plain', '--all'], {
      cwd: workspaceRoot,
      encoding: 'utf-8',
      env: { ...GIT_ENV, TERM: 'dumb' },
    });
    assert.match(plainOut, /notes/);
  });

  it('keeps --json stdout parseable when --fetch writes progress to stderr', () => {
    const result = execFileSync(WORKSPACE_STATUS_SCRIPT, ['--json', '--fetch'], {
      cwd: workspaceRoot,
      encoding: 'utf-8',
      env: { ...GIT_ENV, TERM: 'dumb' },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const snapshot = JSON.parse(result);
    assert.equal(snapshot.version, 1);
    assert.deepEqual(
      snapshot.repos.map((r: { repo: string }) => r.repo),
      ['app', 'lib'],
    );
  });
});
