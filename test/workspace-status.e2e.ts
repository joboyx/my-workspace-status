/**
 * CLI process e2e for workspace-status.sh (execSync, no -i / no Ink).
 * This is not TUI e2e. Live Ink App coverage lives in test/tui-e2e/*.e2e.ts.
 *
 * End-to-end contract tests for workspace-status.sh.
 * Each scenario creates a fresh temp workspace and destroys it after the run.
 */

import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it, before, afterEach, beforeEach } from 'node:test';
import assert from 'node:assert';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WORKSPACE_STATUS_SCRIPT =
  process.env.WORKSPACE_STATUS_SCRIPT ?? path.join(__dirname, '..', 'workspace-status.sh');
const KEEP_E2E_WORKDIR = process.env.KEEP_E2E_WORKDIR === '1';

const GIT_ENV = {
  GIT_AUTHOR_NAME: 'workspace-status e2e',
  GIT_AUTHOR_EMAIL: 'workspace-status-e2e@example.invalid',
  GIT_COMMITTER_NAME: 'workspace-status e2e',
  GIT_COMMITTER_EMAIL: 'workspace-status-e2e@example.invalid',
  ...process.env,
};

let SCENARIO_ROOT = '';
let WORKSPACE_ROOT = '';
let REMOTES_ROOT = '';
let SCRATCH_ROOT = '';

function fail(message: string): never {
  throw new Error(`FAIL: ${message}`);
}

function gitInitWithBranch(repoPath: string, branchName: string): void {
  try {
    execSync(`git init -q -b ${branchName} "${repoPath}"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    return;
  } catch {
    execSync(`git init -q "${repoPath}"`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoPath}" checkout -q -b ${branchName}`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
  }
}

function repoPath(name: string): string {
  return path.join(WORKSPACE_ROOT, name);
}

function remotePath(name: string): string {
  return path.join(REMOTES_ROOT, `${name}.git`);
}

function scenarioPath(name: string): string {
  return path.join(SCENARIO_ROOT, name);
}

function startScenario(): void {
  SCENARIO_ROOT = fs.mkdtempSync(path.join(os.tmpdir(), 'workspace-status-e2e.'));
  WORKSPACE_ROOT = path.join(SCENARIO_ROOT, 'workspace');
  REMOTES_ROOT = path.join(SCENARIO_ROOT, 'remotes');
  SCRATCH_ROOT = path.join(SCENARIO_ROOT, 'scratch');
  fs.mkdirSync(WORKSPACE_ROOT, { recursive: true });
  fs.mkdirSync(REMOTES_ROOT, { recursive: true });
  fs.mkdirSync(SCRATCH_ROOT, { recursive: true });
}

function cleanupScenario(): void {
  if (!SCENARIO_ROOT || !fs.existsSync(SCENARIO_ROOT)) return;
  if (KEEP_E2E_WORKDIR) {
    console.log(`  kept workspace: ${SCENARIO_ROOT}`);
    return;
  }
  fs.rmSync(SCENARIO_ROOT, { recursive: true });
}

function runWorkspaceStatus(...args: string[]): string {
  const cmd = `"${WORKSPACE_STATUS_SCRIPT}" ${args.map((a) => `"${a}"`).join(' ')}`;
  return execSync(cmd, {
    cwd: WORKSPACE_ROOT,
    encoding: 'utf-8',
    env: GIT_ENV,
  });
}

function writeWorkspaceStatusConfig(
  ignoredRepos: string[],
  extras: { maxDepth?: number; defaultBranches?: Record<string, string> } = {},
): void {
  const body: Record<string, unknown> = { ignoredRepos };
  if (extras.maxDepth !== undefined) body.maxDepth = extras.maxDepth;
  if (extras.defaultBranches !== undefined) body.defaultBranches = extras.defaultBranches;
  fs.writeFileSync(
    path.join(WORKSPACE_ROOT, '.workspace-status-config.json'),
    JSON.stringify(body, null, 2) + '\n',
    'utf-8',
  );
}

function assertContains(output: string, expected: string): void {
  if (!output.includes(expected)) {
    fail(`expected output to contain: ${expected}`);
  }
}

function assertNoLeadingBlankLines(output: string): void {
  if (output.startsWith('\n')) {
    fail('expected output to start with content, not blank lines');
  }
}

function assertNotContains(output: string, unexpected: string): void {
  if (output.includes(unexpected)) {
    fail(`expected output to not contain: ${unexpected}`);
  }
}

function assertLineMatches(output: string, regex: RegExp): void {
  const lines = output.split('\n');
  if (!lines.some((line) => regex.test(line))) {
    fail(`expected a line matching: ${regex}`);
  }
}

function assertNoLineMatches(output: string, regex: RegExp): void {
  const lines = output.split('\n');
  if (lines.some((line) => regex.test(line))) {
    fail(`expected no line matching: ${regex}`);
  }
}

function lineNumberFor(output: string, marker: string): number | undefined {
  const lines = output.split('\n');
  const idx = lines.findIndex((line) => line.includes(marker));
  return idx >= 0 ? idx + 1 : undefined;
}

function assertLineBefore(output: string, firstMarker: string, secondMarker: string): void {
  const firstLine = lineNumberFor(output, firstMarker);
  const secondLine = lineNumberFor(output, secondMarker);
  if (firstLine == null) fail(`expected output to contain line marker: ${firstMarker}`);
  if (secondLine == null) fail(`expected output to contain line marker: ${secondMarker}`);
  if (firstLine >= secondLine) {
    fail(`expected '${firstMarker}' before '${secondMarker}'`);
  }
}

function assertCurrentBranch(repoName: string, expectedBranch: string): void {
  const rp = repoPath(repoName);
  const actual = execSync(`git -C "${rp}" branch --show-current`, {
    encoding: 'utf-8',
    env: GIT_ENV,
  }).trim();
  if (actual !== expectedBranch) {
    fail(`expected ${repoName} on ${expectedBranch}, got ${actual}`);
  }
}

function assertRefsEqual(repoName: string, firstRef: string, secondRef: string): void {
  const rp = repoPath(repoName);
  const first = execSync(`git -C "${rp}" rev-parse ${firstRef}`, {
    encoding: 'utf-8',
    env: GIT_ENV,
  }).trim();
  const second = execSync(`git -C "${rp}" rev-parse ${secondRef}`, {
    encoding: 'utf-8',
    env: GIT_ENV,
  }).trim();
  if (first !== second) {
    fail(`expected ${repoName} refs to match: ${firstRef} vs ${secondRef}`);
  }
}

function createRemoteRepo(name: string, defaultBranch = 'main'): void {
  const remoteRepo = remotePath(name);
  const seedRepo = scenarioPath(`seed-${name}`);
  const repoDir = repoPath(name);

  execSync(`git init -q --bare "${remoteRepo}"`, { stdio: 'pipe', env: GIT_ENV });
  gitInitWithBranch(seedRepo, defaultBranch);
  fs.writeFileSync(path.join(seedRepo, 'README.md'), `# ${name}\n`, 'utf-8');
  execSync(`git -C "${seedRepo}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${seedRepo}" commit -q -m "seed ${name}"`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${seedRepo}" remote add origin "${remoteRepo}"`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  execSync(`git -C "${seedRepo}" push -q -u origin ${defaultBranch}`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  execSync(`git --git-dir="${remoteRepo}" symbolic-ref HEAD "refs/heads/${defaultBranch}"`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  fs.rmSync(seedRepo, { recursive: true });

  execSync(`git clone -q "${remoteRepo}" "${repoDir}"`, { stdio: 'pipe', env: GIT_ENV });
  try {
    execSync(`git -C "${repoDir}" remote set-head origin -a`, { stdio: 'pipe', env: GIT_ENV });
  } catch {
    /* ignore */
  }
}

function createLocalRepo(name: string, branchName = 'main'): void {
  const repoDir = repoPath(name);
  gitInitWithBranch(repoDir, branchName);
  fs.writeFileSync(path.join(repoDir, 'README.md'), `# ${name}\n`, 'utf-8');
  execSync(`git -C "${repoDir}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${repoDir}" commit -q -m "seed ${name}"`, { stdio: 'pipe', env: GIT_ENV });
}

function writeRepoFile(repoName: string, relativePath: string, content: string): void {
  const repoDir = repoPath(repoName);
  const fullPath = path.join(repoDir, relativePath);
  fs.mkdirSync(path.dirname(fullPath), { recursive: true });
  fs.writeFileSync(fullPath, content + '\n', 'utf-8');
}

function appendRepoFile(repoName: string, relativePath: string, content: string): void {
  const repoDir = repoPath(repoName);
  const fullPath = path.join(repoDir, relativePath);
  fs.mkdirSync(path.dirname(fullPath), { recursive: true });
  fs.appendFileSync(fullPath, content + '\n', 'utf-8');
}

function createTrackingBranch(repoName: string, branchName: string): void {
  const repoDir = repoPath(repoName);
  const markerFile = branchName.replace(/\//g, '-') + '.txt';

  execSync(`git -C "${repoDir}" checkout -q -b ${branchName}`, { stdio: 'pipe', env: GIT_ENV });
  writeRepoFile(repoName, markerFile, branchName);
  execSync(`git -C "${repoDir}" add ${markerFile}`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${repoDir}" commit -q -m "seed ${branchName}"`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  execSync(`git -C "${repoDir}" push -q -u origin ${branchName}`, { stdio: 'pipe', env: GIT_ENV });
}

function makeLocalCommit(
  repoName: string,
  relativePath: string,
  content: string,
  commitMessage: string,
): void {
  appendRepoFile(repoName, relativePath, content);
  execSync(`git -C "${repoPath(repoName)}" add ${relativePath}`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${repoPath(repoName)}" commit -q -m "${commitMessage}"`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
}

function advanceRemoteBranch(
  repoName: string,
  branchName: string,
  relativePath: string,
  content: string,
  commitMessage: string,
): void {
  const collaboratorDir = scenarioPath(`remote-${repoName}-${branchName.replace(/\//g, '-')}`);
  execSync(`git clone -q "${remotePath(repoName)}" "${collaboratorDir}"`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  execSync(`git -C "${collaboratorDir}" checkout -q ${branchName}`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  fs.mkdirSync(path.dirname(path.join(collaboratorDir, relativePath)), { recursive: true });
  fs.appendFileSync(path.join(collaboratorDir, relativePath), content + '\n', 'utf-8');
  execSync(`git -C "${collaboratorDir}" add ${relativePath}`, { stdio: 'pipe', env: GIT_ENV });
  execSync(`git -C "${collaboratorDir}" commit -q -m "${commitMessage}"`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  execSync(`git -C "${collaboratorDir}" push -q origin ${branchName}`, {
    stdio: 'pipe',
    env: GIT_ENV,
  });
  fs.rmSync(collaboratorDir, { recursive: true });
}

// --- Tests ---

describe('workspace-status e2e', () => {
  before(() => {
    try {
      execSync('git --version', { stdio: 'pipe' });
    } catch {
      throw new Error('git is required');
    }
    if (!fs.existsSync(WORKSPACE_STATUS_SCRIPT)) {
      throw new Error(`script under test not found: ${WORKSPACE_STATUS_SCRIPT}`);
    }
  });

  beforeEach(() => {
    startScenario();
  });

  afterEach(() => {
    cleanupScenario();
  });

  it('help output documents all flags', () => {
    const output = runWorkspaceStatus('--help');
    assertContains(output, 'Usage: workspace-status.sh [OPTIONS] [REPO...]');
    assertContains(output, '.workspace-status-config.json');
    assertContains(output, '"ignoredRepos": ["notes"]');
    assertContains(output, 'maxDepth');
    assertContains(output, 'defaultBranches');
    assertContains(output, 'workspace-status.sh dotfiles');
    assertContains(output, '-a, --all');
    assertContains(output, '-f, --fetch');
    assertContains(output, '-v, --verbose');
    assertContains(output, '-p, --pull');
    assertContains(output, '-d, --default-branch');
    assertContains(output, '--plain');
    assertContains(output, '--json');
  });

  it('clustered short flags are parsed like separate short flags', () => {
    createRemoteRepo('active-repo', 'main');
    createRemoteRepo('notes', 'main');
    writeWorkspaceStatusConfig(['notes']);

    const output = runWorkspaceStatus('-av');

    assertLineMatches(output, /^active-repo\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertLineMatches(output, /^notes\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('fpda clustered flags fetch, pull, switch defaults, and include ignored repos', () => {
    createRemoteRepo('notes', 'main');
    createTrackingBranch('notes', 'feature/ABCD-9003-clustered-flags');
    advanceRemoteBranch(
      'notes',
      'feature/ABCD-9003-clustered-flags',
      'remote.txt',
      'remote included',
      'advance included notes',
    );
    writeWorkspaceStatusConfig(['notes']);

    const output = runWorkspaceStatus('-fpda');

    assertContains(output, '🔄 Fetching from remotes (this may take a moment)...');
    assertContains(output, '⬇️ Pulling repos that are behind...');
    assertContains(output, '  Pulling notes...');
    assertContains(output, '🔄 Switching to default branch and pulling...');
    assertContains(output, '  🔄 notes: Switching from feature/ABCD-9003-clustered-flags to main');
    assertCurrentBranch('notes', 'main');
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('clean repos report all clean and up-to-date', () => {
    createRemoteRepo('alpha-main', 'main');
    createRemoteRepo('beta-develop', 'develop');

    const output = runWorkspaceStatus();
    assertNoLeadingBlankLines(output);
    assertContains(output, '✅ All repos clean and up-to-date');
    assertNotContains(output, 'File changes');
    assertNotContains(output, '🔄 Sync status');
    assertNotContains(output, '🌿 Branches');
  });

  it('empty workspace does not claim all-clean', () => {
    const output = runWorkspaceStatus();
    assertNoLeadingBlankLines(output);
    assertContains(output, 'ℹ️ No git repos found');
    assertNotContains(output, '✅ All repos clean and up-to-date');
  });

  it('unborn and broken repos appear under Attention', () => {
    const unbornDir = repoPath('unborn');
    fs.mkdirSync(unbornDir, { recursive: true });
    gitInitWithBranch(unbornDir, 'main');

    fs.mkdirSync(path.join(repoPath('broken'), '.git'), { recursive: true });

    const output = runWorkspaceStatus();
    assertContains(output, '⚠️ Attention (2):');
    assertContains(output, '    - broken [(unknown)] - status failed');
    assertContains(output, '    - unborn [main] - no commits yet');
    assertNotContains(output, '✅ All repos clean and up-to-date');
  });

  it('verbose output orders repos and surfaces no-upstream', () => {
    createRemoteRepo('alpha-main', 'main');
    createRemoteRepo('beta-develop', 'develop');
    createLocalRepo('gamma-main-local', 'main');
    createLocalRepo('delta-feature-local', 'feature/ABCD-4004-local-only');
    createRemoteRepo('omega-dirty', 'main');
    appendRepoFile('omega-dirty', 'README.md', 'dirty change');

    const output = runWorkspaceStatus('--verbose');

    assertLineBefore(output, 'alpha-main', 'beta-develop');
    assertLineBefore(output, 'beta-develop', 'gamma-main-local');
    assertLineBefore(output, 'gamma-main-local', 'delta-feature-local');
    assertLineBefore(output, 'delta-feature-local', 'omega-dirty');
    assertLineMatches(output, /^gamma-main-local\s+🔥\s+main\s+❓ no upstream\s+💾 clean/);
    assertLineMatches(
      output,
      /^delta-feature-local\s+🚧\s+feature\/ABCD-4004-local-only\s+❓ no upstream\s+💾 clean/,
    );
    assertLineMatches(output, /^omega-dirty\s+🔥\s+main\s+✅ current\s+📝 \d+ files/);
    assertContains(output, '🌿 Branches (1):');
    assertContains(output, '    - delta-feature-local (ABCD-4004)');
  });

  it('detached head repos are still reported in verbose output', () => {
    createRemoteRepo('detached-repo', 'main');
    execSync(`git -C "${repoPath('detached-repo')}" checkout -q --detach HEAD`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(
      output,
      /^detached-repo\s+🌿\s+HEAD \(detached\)\s+❓ no upstream\s+💾 clean/,
    );
    assertContains(output, '🌿 Branches (1):');
    assertContains(output, '  🌿 unknown:');
    assertContains(output, '    - detached-repo [HEAD (detached)]');
    assertNotContains(output, '✅ All repos clean and up-to-date');
  });

  it('symlinked repos are discovered and report status correctly', () => {
    const linkName = 'symlinked-dotfiles';
    createRemoteRepo('regular-repo', 'main');
    const targetDir = path.join(SCRATCH_ROOT, 'symlink-target');
    gitInitWithBranch(targetDir, 'main');
    fs.writeFileSync(path.join(targetDir, 'README.md'), '# symlink-target\n', 'utf-8');
    execSync(`git -C "${targetDir}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${targetDir}" commit -q -m "seed symlink target"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    fs.symlinkSync(targetDir, path.join(WORKSPACE_ROOT, linkName));

    let output = runWorkspaceStatus('--verbose');

    assertLineMatches(
      output,
      new RegExp(`^${linkName}\\s+🔥\\s+main\\s+❓ no upstream\\s+💾 clean`),
    );
    assertContains(output, '✅ All repos clean and up-to-date');

    appendRepoFile(linkName, 'notes.txt', 'change in symlinked repo');
    output = runWorkspaceStatus();
    assertContains(output, 'File changes');
    assertContains(output, `${linkName}`);
    assertContains(output, '🟢A notes.txt');
  });

  it('config ignoredRepos skips repos from verbose, summary, pull, and default-branch processing', () => {
    createRemoteRepo('active-repo', 'main');
    createRemoteRepo('notes', 'main');
    createTrackingBranch('notes', 'feature/ABCD-9001-ignore-notes');
    advanceRemoteBranch(
      'notes',
      'feature/ABCD-9001-ignore-notes',
      'remote.txt',
      'remote ignored',
      'advance ignored notes',
    );
    appendRepoFile('notes', 'README.md', 'ignored dirty change');
    writeWorkspaceStatusConfig(['notes']);

    const output = runWorkspaceStatus('--fetch', '--verbose', '--pull', '--default-branch');

    assertLineMatches(output, /^active-repo\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertContains(output, '  ℹ️ No non-default branches found to switch');
    assertContains(output, '✅ All repos clean and up-to-date');
    assertNotContains(output, 'notes');
    assertNotContains(output, 'ABCD-9001');
    assertCurrentBranch('notes', 'feature/ABCD-9001-ignore-notes');
  });

  it('all flag includes repos ignored by workspace config', () => {
    createRemoteRepo('active-repo', 'main');
    createRemoteRepo('notes', 'main');
    createTrackingBranch('notes', 'feature/ABCD-9002-include-notes');
    appendRepoFile('notes', 'README.md', 'ignored dirty change');
    writeWorkspaceStatusConfig(['notes']);

    const output = runWorkspaceStatus('--all', '--verbose');

    assertLineMatches(output, /^active-repo\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertLineMatches(
      output,
      /^notes\s+🚧\s+feature\/ABCD-9002-include-notes 🌱\s+✅ current\s+📝 \d+ files/,
    );
    assertContains(output, 'File changes');
    assertContains(output, 'notes (ABCD-9002)');
    assertContains(output, '🌿 Branches (1):');
    assertContains(output, '    - notes (ABCD-9002)');
  });

  it('repo filter limits output to named repos and bypasses ignoredRepos', () => {
    createRemoteRepo('active-repo', 'main');
    createRemoteRepo('notes', 'main');
    appendRepoFile('active-repo', 'README.md', 'active dirty change');
    appendRepoFile('notes', 'README.md', 'notes dirty change');
    writeWorkspaceStatusConfig(['notes']);

    const output = runWorkspaceStatus('notes');

    assertContains(output, 'File changes');
    assertContains(output, 'notes');
    assertContains(output, '🟡M README.md');
    assertNotContains(output, 'active-repo');
    assertNotContains(output, '✅ All repos clean and up-to-date');
  });

  it('repo filter accepts multiple repos and works with verbose', () => {
    createRemoteRepo('alpha-main', 'main');
    createRemoteRepo('beta-main', 'main');
    createRemoteRepo('gamma-main', 'main');
    appendRepoFile('alpha-main', 'README.md', 'alpha change');

    const output = runWorkspaceStatus('-v', 'alpha-main', 'beta-main');

    assertLineMatches(output, /^alpha-main\s+🔥\s+main\s+✅ current\s+📝 \d+ files/);
    assertLineMatches(output, /^beta-main\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertNotContains(output, 'gamma-main');
    assertContains(output, 'File changes');
    assertContains(output, 'alpha-main');
  });

  it('repo filter rejects unknown repos', () => {
    createRemoteRepo('active-repo', 'main');

    assert.throws(
      () =>
        execSync(`"${WORKSPACE_STATUS_SCRIPT}" "missing-repo"`, {
          cwd: WORKSPACE_ROOT,
          encoding: 'utf-8',
          stdio: 'pipe',
          env: GIT_ENV,
        }),
      (err: unknown) => {
        const execErr = err as { status?: number; stderr?: string | Buffer };
        const stderr =
          typeof execErr.stderr === 'string' ? execErr.stderr : (execErr.stderr?.toString() ?? '');
        return execErr.status === 1 && stderr.includes('Unknown repo: missing-repo');
      },
    );
  });

  it('discovers depth-3 repos by default', () => {
    createLocalRepo('top-repo', 'main');
    createLocalRepo('group/nested-repo', 'main');
    createLocalRepo('group/mid/deep-repo', 'main');
    appendRepoFile('group/mid/deep-repo', 'README.md', 'deep dirty');

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^top-repo\s+🔥\s+main\s+❓ no upstream\s+💾 clean/);
    assertLineMatches(output, /^group\/nested-repo\s+🔥\s+main\s+❓ no upstream\s+💾 clean/);
    assertLineMatches(output, /^group\/mid\/deep-repo\s+🔥\s+main\s+❓ no upstream\s+📝 \d+ files/);
    assertContains(output, 'group/mid/deep-repo');
    assertContains(output, '🟡M README.md');
  });

  it('maxDepth config limits discovery depth', () => {
    createLocalRepo('top-repo', 'main');
    createLocalRepo('group/nested-repo', 'main');
    createLocalRepo('group/mid/deep-repo', 'main');
    appendRepoFile('group/mid/deep-repo', 'README.md', 'deep dirty');
    writeWorkspaceStatusConfig([], { maxDepth: 2 });

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^top-repo\s+🔥\s+main\s+❓ no upstream\s+💾 clean/);
    assertLineMatches(output, /^group\/nested-repo\s+🔥\s+main\s+❓ no upstream\s+💾 clean/);
    assertNotContains(output, 'deep-repo');
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('unstaged-only repos show unstaged summaries and change emoji', () => {
    createRemoteRepo('unstaged-main', 'main');
    createRemoteRepo('unstaged-develop', 'develop');
    appendRepoFile('unstaged-main', 'README.md', 'unstaged main change');
    appendRepoFile('unstaged-develop', 'README.md', 'unstaged develop change');

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^unstaged-main\s+🔥\s+main\s+✅ current\s+📝 \d+ files/);
    assertLineMatches(output, /^unstaged-develop\s+🌿\s+develop\s+✅ current\s+📝 \d+ files/);
    assertContains(output, 'File changes');
    assertContains(output, 'unstaged-main');
    assertContains(output, 'unstaged-develop');
    assertContains(output, '🟡M README.md');
    assertNotContains(output, '✨ staged');
    assertNotContains(output, '📄 untracked');
    assertNotContains(output, '🔄 Sync status');
    assertNotContains(output, '🌿 Branches');
  });

  it('untracked-only repos show untracked summaries and change emoji', () => {
    createRemoteRepo('untracked-main', 'main');
    createRemoteRepo('untracked-develop', 'develop');
    writeRepoFile('untracked-main', 'notes.txt', 'untracked main');
    writeRepoFile('untracked-develop', 'todo.txt', 'untracked develop');

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^untracked-main\s+🔥\s+main\s+✅ current\s+📝 \d+ files/);
    assertLineMatches(output, /^untracked-develop\s+🌿\s+develop\s+✅ current\s+📝 \d+ files/);
    assertContains(output, 'File changes');
    assertContains(output, 'untracked-main');
    assertContains(output, 'untracked-develop');
    assertContains(output, '🟢A notes.txt');
    assertContains(output, '🟢A todo.txt');
    assertNotContains(output, '📄 unstaged');
    assertNotContains(output, '✨ staged');
    assertNotContains(output, '🔄 Sync status');
    assertNotContains(output, '🌿 Branches');
  });

  it('multi-repo file changes use repo headers, deeper tree indentation, and blank lines between repos', () => {
    createRemoteRepo('dotfiles', 'main');
    writeRepoFile('dotfiles', 'ai/agents/codex/config.toml', 'codex baseline');
    writeRepoFile('dotfiles', 'ai/agents/cursor/hooks.json', 'hooks baseline');
    execSync(
      `git -C "${repoPath('dotfiles')}" add ai/agents/codex/config.toml ai/agents/cursor/hooks.json`,
      {
        stdio: 'pipe',
        env: GIT_ENV,
      },
    );
    execSync(`git -C "${repoPath('dotfiles')}" commit -q -m "seed dotfiles paths"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoPath('dotfiles')}" push -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    createRemoteRepo('notes', 'main');
    createRemoteRepo('billing-service', 'main');
    writeRepoFile('billing-service', 'serverless-config.yml', 'stage: baseline');
    execSync(`git -C "${repoPath('billing-service')}" add serverless-config.yml`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoPath('billing-service')}" commit -q -m "seed billing service path"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoPath('billing-service')}" push -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    appendRepoFile('dotfiles', 'ai/agents/codex/config.toml', 'codex change');
    appendRepoFile('dotfiles', 'ai/agents/cursor/hooks.json', 'hooks change');
    writeRepoFile('notes', 'tmp/monitor-order-vbrqjnfplq.log', 'monitor log');
    appendRepoFile('billing-service', 'serverless-config.yml', 'stage: changed');

    assert.strictEqual(
      runWorkspaceStatus().trimEnd(),
      [
        'File changes',
        '  📦 billing-service',
        '     └─ 🟡M serverless-config.yml',
        '',
        '  📦 dotfiles',
        '     └─ ai/agents',
        '        ├─ codex',
        '        │  └─ 🟡M config.toml',
        '        └─ cursor',
        '           └─ 🟡M hooks.json',
        '',
        '  📦 notes',
        '     └─ tmp',
        '        └─ 🟢A monitor-order-vbrqjnfplq.log',
      ].join('\n'),
    );
  });

  it('staged-only repos show staged summaries and change emoji', () => {
    createRemoteRepo('staged-main', 'main');
    createRemoteRepo('staged-develop', 'develop');
    appendRepoFile('staged-main', 'README.md', 'staged main change');
    execSync(`git -C "${repoPath('staged-main')}" add README.md`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    writeRepoFile('staged-develop', 'new-feature.txt', 'staged develop file');
    execSync(`git -C "${repoPath('staged-develop')}" add new-feature.txt`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^staged-main\s+🔥\s+main\s+✅ current\s+✨ staged/);
    assertLineMatches(output, /^staged-develop\s+🌿\s+develop\s+✅ current\s+✨ staged/);
    assertContains(output, 'File changes');
    assertContains(output, 'staged-main');
    assertContains(output, 'staged-develop');
    assertContains(output, '🔵S README.md');
    assertContains(output, '🟢A new-feature.txt');
    assertNotContains(output, '📄 unstaged');
    assertNotContains(output, '📄 untracked');
    assertNotContains(output, '🔄 Sync status');
    assertNotContains(output, '🌿 Branches');
  });

  it('repos with staged and unstaged changes show both sections', () => {
    const repoDir = repoPath('mixed-default');
    createRemoteRepo('mixed-default', 'main');
    writeRepoFile('mixed-default', 'delete-me.txt', 'delete me');
    execSync(`git -C "${repoDir}" add delete-me.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${repoDir}" commit -q -m "seed mixed files"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoDir}" push -q origin main`, { stdio: 'pipe', env: GIT_ENV });

    appendRepoFile('mixed-default', 'README.md', 'staged change');
    execSync(`git -C "${repoDir}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
    appendRepoFile('mixed-default', 'README.md', 'unstaged change');
    fs.unlinkSync(path.join(repoDir, 'delete-me.txt'));

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^mixed-default\s+🔥\s+main\s+✅ current\s+⚠️ staged\+dirty/);
    assertContains(output, 'File changes');
    assertContains(output, 'mixed-default');
    assertContains(output, '🟠MS README.md');
    assertContains(output, '🔴D delete-me.txt');
    assertNotContains(output, '📄 untracked');
    assertNotContains(output, '🔄 Sync status');
    assertNotContains(output, '🌿 Branches');
  });

  it('file changes include staged, unstaged, untracked, and ticket labels', () => {
    const repoDir = repoPath('media-service');
    createRemoteRepo('media-service', 'main');
    createTrackingBranch('media-service', 'feature/ABCD-1234-status-contract');

    writeRepoFile('media-service', 'tracked.txt', 'tracked baseline');
    writeRepoFile('media-service', 'rename-me.txt', 'rename baseline');
    writeRepoFile('media-service', 'delete-me.txt', 'delete baseline');
    execSync(`git -C "${repoDir}" add tracked.txt rename-me.txt delete-me.txt`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoDir}" commit -q -m "seed tracked files"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoDir}" push -q origin HEAD`, { stdio: 'pipe', env: GIT_ENV });

    execSync(`git -C "${repoDir}" mv rename-me.txt renamed.txt`, { stdio: 'pipe', env: GIT_ENV });
    writeRepoFile('media-service', 'staged-added.txt', 'staged add');
    fs.unlinkSync(path.join(repoDir, 'delete-me.txt'));
    execSync(`git -C "${repoDir}" add -A`, { stdio: 'pipe', env: GIT_ENV });
    appendRepoFile('media-service', 'tracked.txt', 'unstaged change');
    writeRepoFile('media-service', 'untracked.txt', 'untracked');

    const output = runWorkspaceStatus();
    assertNoLeadingBlankLines(output);

    assertContains(output, 'File changes');
    assertContains(output, 'media-service (ABCD-1234)');
    assertContains(output, '🟣R rename-me.txt -> renamed.txt');
    assertContains(output, '🟢A staged-added.txt');
    assertContains(output, '🔴D delete-me.txt');
    assertContains(output, '🟡M tracked.txt');
    assertContains(output, '🟢A untracked.txt');
    assertContains(output, '🌿 Branches (1):');
    assertContains(output, '  🚧 feature:');
    assertContains(output, '    - media-service (ABCD-1234)');
  });

  it('file changes collapse nested directory trees and keep file badges inline', () => {
    const repoDir = repoPath('tree-service');
    createRemoteRepo('tree-service', 'main');
    createTrackingBranch('tree-service', 'feature/ABCD-4321-tree-output');

    writeRepoFile('tree-service', 'src/index.ts', 'index baseline');
    writeRepoFile('tree-service', 'src/lib/old-name.ts', 'rename baseline');
    writeRepoFile('tree-service', 'src/lib/deprecated.ts', 'delete baseline');
    execSync(`git -C "${repoDir}" add src/index.ts src/lib/old-name.ts src/lib/deprecated.ts`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoDir}" commit -q -m "seed tree files"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${repoDir}" push -q origin HEAD`, { stdio: 'pipe', env: GIT_ENV });

    execSync(`git -C "${repoDir}" mv src/lib/old-name.ts src/lib/new-name.ts`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    fs.unlinkSync(path.join(repoDir, 'src/lib/deprecated.ts'));
    writeRepoFile('tree-service', 'src/components/StatusPanel.ts', 'staged component');
    execSync(`git -C "${repoDir}" add -A`, { stdio: 'pipe', env: GIT_ENV });
    appendRepoFile('tree-service', 'src/index.ts', 'unstaged index change');

    const output = runWorkspaceStatus();

    assertContains(output, 'File changes');
    assertContains(output, '  📦 tree-service (ABCD-4321)');
    assertContains(output, '     └─ src');
    assertContains(output, '        ├─ components');
    assertContains(output, '        │  └─ 🟢A StatusPanel.ts');
    assertContains(output, '        ├─ lib');
    assertContains(output, '        │  ├─ 🔴D deprecated.ts');
    assertContains(output, '        │  └─ 🟣R old-name.ts -> new-name.ts');
    assertContains(output, '        └─ 🟡M index.ts');
    assertContains(output, '🌿 Branches (1):');
    assertContains(output, '    - tree-service (ABCD-4321)');
  });

  it('behind-only sync summary uses the behind section only', () => {
    createRemoteRepo('behind-main', 'main');
    advanceRemoteBranch('behind-main', 'main', 'README.md', 'remote behind', 'advance behind');
    execSync(`git -C "${repoPath('behind-main')}" fetch -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^behind-main\s+🔥\s+main\s+⬇️ behind 1\s+💾 clean/);
    assertContains(output, '🔄 Sync status (1):');
    assertContains(output, '  ⬇️ behind:');
    assertContains(output, '    - behind-main [main] - behind by 1 commits');
    assertNotContains(output, '  ⬆️ ahead:');
    assertNotContains(output, '  🔀 diverged:');
    assertNotContains(output, 'File changes');
    assertNotContains(output, '🌿 Branches');
  });

  it('ahead-only sync summary uses the ahead section only', () => {
    createRemoteRepo('ahead-main', 'main');
    makeLocalCommit('ahead-main', 'ahead.txt', 'local ahead', 'local ahead');

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^ahead-main\s+🔥\s+main\s+⬆️ ahead 1\s+💾 clean/);
    assertContains(output, '🔄 Sync status (1):');
    assertContains(output, '  ⬆️ ahead:');
    assertContains(output, '    - ahead-main [main] - ahead by 1 commits');
    assertNotContains(output, '  ⬇️ behind:');
    assertNotContains(output, '  🔀 diverged:');
    assertNotContains(output, 'File changes');
    assertNotContains(output, '🌿 Branches');
  });

  it('diverged-only sync summary uses the diverged section only', () => {
    createRemoteRepo('diverged-main', 'main');
    advanceRemoteBranch(
      'diverged-main',
      'main',
      'remote.txt',
      'remote diverged',
      'remote diverged',
    );
    execSync(`git -C "${repoPath('diverged-main')}" fetch -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    makeLocalCommit('diverged-main', 'local.txt', 'local diverged', 'local diverged');

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^diverged-main\s+🔥\s+main\s+🔀 1\/1\s+💾 clean/);
    assertContains(output, '🔄 Sync status (1):');
    assertContains(output, '  🔀 diverged:');
    assertContains(output, '    - diverged-main [main] - diverged (ahead 1, behind 1)');
    assertNotContains(output, '  ⬇️ behind:');
    assertNotContains(output, '  ⬆️ ahead:');
    assertNotContains(output, 'File changes');
    assertNotContains(output, '🌿 Branches');
  });

  it('sync summary reports behind, ahead, and diverged task branches', () => {
    createRemoteRepo('behind-repo', 'main');
    createTrackingBranch('behind-repo', 'feature/ABCD-2001-behind');
    advanceRemoteBranch(
      'behind-repo',
      'feature/ABCD-2001-behind',
      'remote.txt',
      'remote behind',
      'advance behind',
    );
    execSync(`git -C "${repoPath('behind-repo')}" fetch -q origin "feature/ABCD-2001-behind"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    createRemoteRepo('ahead-repo', 'main');
    createTrackingBranch('ahead-repo', 'bugfix/ABCD-2002-ahead');
    makeLocalCommit('ahead-repo', 'local-only.txt', 'ahead change', 'local ahead');

    createRemoteRepo('diverged-repo', 'main');
    createTrackingBranch('diverged-repo', 'chore/ABCD-2003-diverged');
    advanceRemoteBranch(
      'diverged-repo',
      'chore/ABCD-2003-diverged',
      'remote.txt',
      'remote diverged',
      'remote diverged',
    );
    execSync(`git -C "${repoPath('diverged-repo')}" fetch -q origin "chore/ABCD-2003-diverged"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    makeLocalCommit('diverged-repo', 'local.txt', 'local diverged', 'local diverged');

    const output = runWorkspaceStatus();

    assertContains(output, '🔄 Sync status (3):');
    assertContains(
      output,
      '    - behind-repo [feature/ABCD-2001-behind] (ABCD-2001) - behind by 1 commits',
    );
    assertContains(
      output,
      '    - ahead-repo [bugfix/ABCD-2002-ahead] (ABCD-2002) - ahead by 1 commits',
    );
    assertContains(
      output,
      '    - diverged-repo [chore/ABCD-2003-diverged] (ABCD-2003) - diverged (ahead 1, behind 1)',
    );
    assertContains(output, '🌿 Branches (3):');
    assertContains(output, '  🚧 feature:');
    assertContains(output, '    - behind-repo (ABCD-2001)');
    assertContains(output, '  🐛 bugfix:');
    assertContains(output, '    - ahead-repo (ABCD-2002)');
    assertContains(output, '  🔧 chore:');
    assertContains(output, '    - diverged-repo (ABCD-2003)');
  });

  it('branch-only summary lists clean non-default branch kinds', () => {
    createRemoteRepo('feature-repo', 'main');
    createTrackingBranch('feature-repo', 'feature/ABCD-6001-feature');
    createRemoteRepo('bugfix-repo', 'main');
    createTrackingBranch('bugfix-repo', 'bugfix/ABCD-6002-fix');
    createRemoteRepo('chore-repo', 'main');
    createTrackingBranch('chore-repo', 'chore/release-hygiene');
    createRemoteRepo('release-repo', 'main');
    createTrackingBranch('release-repo', 'release/2026-07-20_DEMO2026053300');
    createRemoteRepo('unknown-repo', 'main');
    createTrackingBranch('unknown-repo', 'hotfix/urgent');

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(
      output,
      /^feature-repo\s+🚧\s+feature\/ABCD-6001-feature 🌱\s+✅ current\s+💾 clean/,
    );
    assertLineMatches(
      output,
      /^bugfix-repo\s+🐛\s+bugfix\/ABCD-6002-fix 🌱\s+✅ current\s+💾 clean/,
    );
    assertLineMatches(
      output,
      /^chore-repo\s+🔧\s+chore\/release-hygiene 🌱\s+✅ current\s+💾 clean/,
    );
    assertLineMatches(
      output,
      /^release-repo\s+🚀\s+release\/2026-07-20_DEMO2026053300 🌱\s+✅ current\s+💾 clean/,
    );
    assertLineMatches(
      output,
      /^unknown-repo\s+🌿\s+hotfix\/urgent 🌱\s+✅ current\s+💾 clean/,
    );
    assertContains(output, '🌿 Branches (5):');
    assertContains(output, '  🚧 feature:');
    assertContains(output, '    - feature-repo (ABCD-6001)');
    assertContains(output, '  🐛 bugfix:');
    assertContains(output, '    - bugfix-repo (ABCD-6002)');
    assertContains(output, '  🔧 chore:');
    assertContains(output, '    - chore-repo [chore/release-hygiene]');
    assertContains(output, '  🚀 release:');
    assertContains(output, '    - release-repo [release/2026-07-20_DEMO2026053300]');
    assertContains(output, '  🌿 unknown:');
    assertContains(output, '    - unknown-repo [hotfix/urgent]');
    assertNotContains(output, 'File changes');
    assertNotContains(output, '🔄 Sync status');
  });

  it('fetch updates remote tracking state before reporting', () => {
    createRemoteRepo('fetch-repo', 'main');
    advanceRemoteBranch('fetch-repo', 'main', 'README.md', 'remote fetch', 'advance fetch');

    const withoutFetchOutput = runWorkspaceStatus();
    assertContains(withoutFetchOutput, '✅ All repos clean and up-to-date');
    assertNotContains(withoutFetchOutput, '🔄 Sync status');

    const withFetchOutput = runWorkspaceStatus('--fetch', '--verbose');
    assertContains(withFetchOutput, '🔄 Fetching from remotes (this may take a moment)...');
    assertLineBefore(
      withFetchOutput,
      '🔄 Fetching from remotes (this may take a moment)...',
      'fetch-repo',
    );
    assertLineMatches(withFetchOutput, /^fetch-repo\s+🔥\s+main\s+⬇️ behind 1\s+💾 clean/);
    assertContains(withFetchOutput, '🔄 Sync status (1):');
    assertContains(withFetchOutput, '    - fetch-repo [main] - behind by 1 commits');
  });

  it('pull updates behind repos and refreshes the summary', () => {
    createRemoteRepo('pull-repo', 'main');
    advanceRemoteBranch('pull-repo', 'main', 'README.md', 'remote pull', 'advance pull');
    execSync(`git -C "${repoPath('pull-repo')}" fetch -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    const output = runWorkspaceStatus('--pull');

    assertContains(output, '⬇️ Pulling repos that are behind...');
    assertContains(output, '  Pulling pull-repo...');
    assertContains(output, '    ✅ Success');
    assertContains(output, '🔄 Re-checking status after pull...');
    assertContains(output, '✅ All repos clean and up-to-date');
    assertRefsEqual('pull-repo', 'HEAD', 'origin/main');
  });

  it('pull stashes dirty worktrees, pulls, then reapplies local changes', () => {
    createRemoteRepo('dirty-pull-repo', 'main');
    advanceRemoteBranch(
      'dirty-pull-repo',
      'main',
      'remote.txt',
      'remote advance',
      'advance remote',
    );
    execSync(`git -C "${repoPath('dirty-pull-repo')}" fetch -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    writeRepoFile('dirty-pull-repo', 'local.txt', 'keep me');
    appendRepoFile('dirty-pull-repo', 'README.md', 'local dirty edit');

    const output = runWorkspaceStatus('--pull');

    assertContains(output, '  Pulling dirty-pull-repo...');
    assertContains(output, '    ✅ Success (stashed local changes, reapplied)');
    assertContains(output, '🔄 Re-checking status after pull...');
    assertRefsEqual('dirty-pull-repo', 'HEAD', 'origin/main');
    assertContains(
      fs.readFileSync(path.join(repoPath('dirty-pull-repo'), 'README.md'), 'utf-8'),
      'local dirty edit',
    );
    assertContains(
      fs.readFileSync(path.join(repoPath('dirty-pull-repo'), 'local.txt'), 'utf-8'),
      'keep me',
    );
    assertContains(output, 'File changes');
    assertContains(output, 'dirty-pull-repo');
    const stashList = execSync(`git -C "${repoPath('dirty-pull-repo')}" stash list`, {
      encoding: 'utf-8',
      env: GIT_ENV,
    }).trim();
    assert.equal(stashList, '', 'expected auto-stash to be popped');
  });

  it('verbose pull renders the final table state', () => {
    createRemoteRepo('verbose-pull-repo', 'main');
    advanceRemoteBranch('verbose-pull-repo', 'main', 'README.md', 'remote pull', 'advance pull');
    execSync(`git -C "${repoPath('verbose-pull-repo')}" fetch -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    const output = runWorkspaceStatus('--verbose', '--pull');

    assertLineMatches(output, /^verbose-pull-repo\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertNoLineMatches(output, /^verbose-pull-repo\s+🔥\s+main\s+⬇️/);
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('complete mixed workspace snapshot exercises all main sections together', () => {
    createRemoteRepo('alpha-clean-main', 'main');
    createRemoteRepo('beta-behind-main', 'main');
    advanceRemoteBranch('beta-behind-main', 'main', 'README.md', 'remote behind', 'advance behind');
    execSync(`git -C "${repoPath('beta-behind-main')}" fetch -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    createRemoteRepo('gamma-clean-develop', 'develop');

    createRemoteRepo('delta-ahead-develop', 'develop');
    makeLocalCommit('delta-ahead-develop', 'ahead.txt', 'ahead develop', 'ahead develop');

    createRemoteRepo('epsilon-default-dirty', 'main');
    appendRepoFile('epsilon-default-dirty', 'README.md', 'dirty default change');

    createRemoteRepo('eta-chore-clean', 'main');
    createTrackingBranch('eta-chore-clean', 'chore/release-hygiene');

    createRemoteRepo('theta-feature-staged', 'main');
    createTrackingBranch('theta-feature-staged', 'feature/ABCD-7001-feature');
    writeRepoFile('theta-feature-staged', 'feature.txt', 'feature staged');
    execSync(`git -C "${repoPath('theta-feature-staged')}" add feature.txt`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    createRemoteRepo('zeta-diverged-main', 'main');
    advanceRemoteBranch(
      'zeta-diverged-main',
      'main',
      'remote.txt',
      'remote diverged',
      'remote diverged',
    );
    execSync(`git -C "${repoPath('zeta-diverged-main')}" fetch -q origin main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    makeLocalCommit('zeta-diverged-main', 'local.txt', 'local diverged', 'local diverged');

    const iotaRepo = repoPath('iota-bugfix-both');
    createRemoteRepo('iota-bugfix-both', 'main');
    createTrackingBranch('iota-bugfix-both', 'bugfix/ABCD-7002-bug');
    writeRepoFile('iota-bugfix-both', 'delete-me.txt', 'delete me');
    execSync(`git -C "${iotaRepo}" add delete-me.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${iotaRepo}" commit -q -m "seed delete file"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    execSync(`git -C "${iotaRepo}" push -q origin HEAD`, { stdio: 'pipe', env: GIT_ENV });
    appendRepoFile('iota-bugfix-both', 'README.md', 'staged bugfix change');
    execSync(`git -C "${iotaRepo}" add README.md`, { stdio: 'pipe', env: GIT_ENV });
    appendRepoFile('iota-bugfix-both', 'README.md', 'unstaged bugfix change');
    fs.unlinkSync(path.join(iotaRepo, 'delete-me.txt'));

    const output = runWorkspaceStatus('--verbose');

    assertLineBefore(output, 'alpha-clean-main', 'gamma-clean-develop');
    assertLineBefore(output, 'gamma-clean-develop', 'beta-behind-main');
    assertLineBefore(output, 'beta-behind-main', 'delta-ahead-develop');
    assertLineBefore(output, 'delta-ahead-develop', 'zeta-diverged-main');
    assertLineBefore(output, 'zeta-diverged-main', 'eta-chore-clean');
    assertLineBefore(output, 'eta-chore-clean', 'epsilon-default-dirty');
    assertLineBefore(output, 'epsilon-default-dirty', 'iota-bugfix-both');
    assertLineBefore(output, 'iota-bugfix-both', 'theta-feature-staged');
    assertLineMatches(output, /^epsilon-default-dirty\s+🔥\s+main\s+✅ current\s+📝 \d+ files/);
    assertLineMatches(
      output,
      /^theta-feature-staged\s+🚧\s+feature\/ABCD-7001-feature 🌱\s+✅ current\s+✨ staged/,
    );
    assertLineMatches(
      output,
      /^iota-bugfix-both\s+🐛\s+bugfix\/ABCD-7002-bug 🌱\s+✅ current\s+⚠️ staged\+dirty/,
    );
    assertContains(output, 'File changes');
    assertContains(output, 'epsilon-default-dirty');
    assertContains(output, 'iota-bugfix-both (ABCD-7002)');
    assertContains(output, 'theta-feature-staged (ABCD-7001)');
    assertContains(output, '🔄 Sync status (3):');
    assertContains(output, '    - beta-behind-main [main] - behind by 1 commits');
    assertContains(output, '    - delta-ahead-develop [develop] - ahead by 1 commits');
    assertContains(output, '    - zeta-diverged-main [main] - diverged (ahead 1, behind 1)');
    assertContains(output, '🌿 Branches (3):');
    assertContains(output, '    - theta-feature-staged (ABCD-7001)');
    assertContains(output, '    - iota-bugfix-both (ABCD-7002)');
    assertContains(output, '    - eta-chore-clean [chore/release-hygiene]');
  });

  it('default-branch switches clean task branches and skips dirty repos', () => {
    createRemoteRepo('switch-feature', 'main');
    createTrackingBranch('switch-feature', 'feature/ABCD-5001-switch');

    createRemoteRepo('switch-bugfix', 'develop');
    createTrackingBranch('switch-bugfix', 'bugfix/ABCD-5002-switch');

    createRemoteRepo('stay-dirty', 'main');
    createTrackingBranch('stay-dirty', 'chore/ABCD-5003-dirty');
    const dirtyRepoDir = repoPath('stay-dirty');
    appendRepoFile('stay-dirty', 'README.md', 'dirty branch change');

    const output = runWorkspaceStatus('--default-branch');

    assertContains(output, '🔄 Switching to default branch and pulling...');
    assertContains(output, '  🔄 switch-feature: Switching from feature/ABCD-5001-switch to main');
    assertContains(output, '  🔄 switch-bugfix: Switching from bugfix/ABCD-5002-switch to develop');
    assertContains(
      output,
      '  ⚠️ stay-dirty (chore/ABCD-5003-dirty): Has uncommitted changes, skipping',
    );
    assertContains(output, '🔄 Re-checking status after switch...');
    assertCurrentBranch('switch-feature', 'main');
    assertCurrentBranch('switch-bugfix', 'develop');
    assertCurrentBranch('stay-dirty', 'chore/ABCD-5003-dirty');
    assertContains(output, 'File changes');
    assertContains(output, 'stay-dirty (ABCD-5003)');
    assertContains(output, '🌿 Branches (1):');
    assertContains(output, '    - stay-dirty (ABCD-5003)');
    assert(
      fs.existsSync(path.join(dirtyRepoDir, 'README.md')),
      'expected dirty repo to remain intact',
    );
  });

  it('verbose default-branch renders the final table state', () => {
    createRemoteRepo('verbose-switch-feature', 'main');
    createTrackingBranch('verbose-switch-feature', 'feature/ABCD-8001-switch');

    const output = runWorkspaceStatus('--verbose', '--default-branch');

    assertLineMatches(output, /^verbose-switch-feature\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertNoLineMatches(output, /^verbose-switch-feature\s+🚧\s+feature\/ABCD-8001-switch/);
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('default-branch switches multiple clean feature repos concurrently', () => {
    for (const name of ['parallel-a', 'parallel-b', 'parallel-c']) {
      createRemoteRepo(name, 'main');
      createTrackingBranch(name, `feature/ABCD-9001-${name}`);
    }

    const output = runWorkspaceStatus('--default-branch');

    assertContains(output, '🔄 Re-checking status after switch...');
    assertCurrentBranch('parallel-a', 'main');
    assertCurrentBranch('parallel-b', 'main');
    assertCurrentBranch('parallel-c', 'main');
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('defaultBranches override treats only that branch as default for classification', () => {
    createRemoteRepo('override-app', 'main');
    createTrackingBranch('override-app', 'develop');
    execSync(`git -C "${repoPath('override-app')}" checkout -q main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    writeWorkspaceStatusConfig([], {
      defaultBranches: { 'override-app': 'develop' },
    });

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^override-app\s+🔥\s+main ✅\s+✅ current\s+💾 clean/);
    assertContains(output, '🌿 Branches (1):');
    assertContains(output, '    - override-app [main]');
    assertNotContains(output, '✅ All repos clean and up-to-date');
  });

  it('defaultBranches override is used by --default-branch instead of origin/HEAD', () => {
    createRemoteRepo('override-switch', 'main');
    createTrackingBranch('override-switch', 'develop');
    createTrackingBranch('override-switch', 'feature/ABCD-9101-override');
    writeWorkspaceStatusConfig([], {
      defaultBranches: { 'override-switch': 'develop' },
    });

    const output = runWorkspaceStatus('--default-branch');

    assertContains(
      output,
      '  🔄 override-switch: Switching from feature/ABCD-9101-override to develop',
    );
    assertCurrentBranch('override-switch', 'develop');
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('defaultBranches override switches clean main to configured develop', () => {
    createRemoteRepo('override-main', 'main');
    createTrackingBranch('override-main', 'develop');
    execSync(`git -C "${repoPath('override-main')}" checkout -q main`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    writeWorkspaceStatusConfig([], {
      defaultBranches: { 'override-main': 'develop' },
    });

    const output = runWorkspaceStatus('--default-branch');

    assertContains(output, '  🔄 override-main: Switching from main to develop');
    assertCurrentBranch('override-main', 'develop');
    assertContains(output, '✅ All repos clean and up-to-date');
  });

  it('without defaultBranches override, main stays a clean default branch', () => {
    createRemoteRepo('legacy-main', 'main');

    const output = runWorkspaceStatus('--verbose');

    assertLineMatches(output, /^legacy-main\s+🔥\s+main\s+✅ current\s+💾 clean/);
    assertContains(output, '✅ All repos clean and up-to-date');
    assertNotContains(output, '🌿 Branches');
  });

  it.skip('linked .worktrees show 🔗 / Files / merge marks, then ✅ after merge into main', () => {
    createLocalRepo('app', 'main');
    const appDir = repoPath('app');
    const worktreeDir = path.join(appDir, '.worktrees', 'feature-demo');
    fs.mkdirSync(path.join(appDir, '.worktrees'), { recursive: true });
    execSync(`git -C "${appDir}" worktree add -q "${worktreeDir}" -b feature/demo`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });
    fs.writeFileSync(path.join(worktreeDir, 'feature.txt'), 'unique tip\n', 'utf-8');
    execSync(`git -C "${worktreeDir}" add feature.txt`, { stdio: 'pipe', env: GIT_ENV });
    execSync(`git -C "${worktreeDir}" commit -q -m "open feature/demo"`, {
      stdio: 'pipe',
      env: GIT_ENV,
    });

    const openOutput = runWorkspaceStatus('--verbose');

    assertLineMatches(openOutput, /^Repo\s+Branch\s+Sync\s+Files/);
    assertLineMatches(openOutput, /^app\s+🔥\s+main\s+❓ no upstream\s+💾 clean/);
    assertLineMatches(
      openOutput,
      /^🔗 app\/\.worktrees\/feature-demo\s+🚧\s+feature\/demo 🌱\s+❓ no upstream\s+💾 clean/,
    );
    assertContains(openOutput, '🔗 Linked worktrees (1):');
    assertContains(openOutput, '    - 🔗 app/.worktrees/feature-demo [feature/demo] 🌱');
    assertContains(openOutput, '🌿 Branches (1):');
    assertContains(openOutput, '    - 🔗 app/.worktrees/feature-demo [feature/demo] 🌱');
    assertNotContains(openOutput, 'feature/demo ✅');
    assertCurrentBranch('app', 'main');

    execSync(`git -C "${appDir}" merge -q feature/demo`, { stdio: 'pipe', env: GIT_ENV });
    assertCurrentBranch('app', 'main');
    const wtBranch = execSync(`git -C "${worktreeDir}" branch --show-current`, {
      encoding: 'utf-8',
      env: GIT_ENV,
    }).trim();
    assert.equal(wtBranch, 'feature/demo');

    const mergedOutput = runWorkspaceStatus('--verbose');

    assertLineMatches(mergedOutput, /^Repo\s+Branch\s+Sync\s+Files/);
    assertLineMatches(mergedOutput, /^app\s+🔥\s+main\s+❓ no upstream\s+💾 clean/);
    assertLineMatches(
      mergedOutput,
      /^🔗 app\/\.worktrees\/feature-demo\s+🚧\s+feature\/demo ✅\s+❓ no upstream\s+💾 clean/,
    );
    assertContains(mergedOutput, '🔗 Linked worktrees (1):');
    assertContains(mergedOutput, '    - 🔗 app/.worktrees/feature-demo [feature/demo] ✅');
    assertNotContains(mergedOutput, 'feature/demo 🌱');
  });
});
