/**
 * Unit tests for workspace-status config loading.
 */

import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { afterEach, describe, it } from 'node:test';
import assert from 'node:assert';

import {
  CONFIG_FILENAME,
  DEFAULT_MAX_DEPTH,
  defaultBranchOverrideFor,
  loadWorkspaceStatusConfig,
  workspaceStatusConfig,
} from '../src/config.js';
import { getBranchKind, isDefaultBranch } from '../src/helpers.js';

describe('loadWorkspaceStatusConfig', () => {
  let cwd = '';

  afterEach(() => {
    if (cwd && fs.existsSync(cwd)) {
      fs.rmSync(cwd, { recursive: true });
    }
    cwd = '';
  });

  function writeConfig(body: unknown): void {
    cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-config-'));
    fs.writeFileSync(path.join(cwd, CONFIG_FILENAME), JSON.stringify(body) + '\n', 'utf-8');
  }

  it('defaults ignoredRepos, maxDepth, and defaultBranches when config is missing', async () => {
    cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-status-config-'));
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.deepEqual(config, {
      ignoredRepos: [],
      maxDepth: DEFAULT_MAX_DEPTH,
      defaultBranches: {},
    });
    assert.equal(config.editor, undefined);
  });

  it('loads ignoredRepos and defaults maxDepth and defaultBranches', async () => {
    writeConfig({ ignoredRepos: [' notes ', 'dotfiles', 'notes'] });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.deepEqual(config.ignoredRepos, ['dotfiles', 'notes']);
    assert.equal(config.maxDepth, DEFAULT_MAX_DEPTH);
    assert.deepEqual(config.defaultBranches, {});
  });

  it('normalizes ignoredRepos path forms and de-duplicates', async () => {
    writeConfig({
      ignoredRepos: ['notes/', './notes', 'notes\\', ' ./dotfiles/ ', 'services\\api/'],
    });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.deepEqual(config.ignoredRepos, ['dotfiles', 'notes', 'services/api']);
  });

  it('loads custom maxDepth', async () => {
    writeConfig({ ignoredRepos: [], maxDepth: 4 });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.equal(config.maxDepth, 4);
  });

  it('rejects non-positive maxDepth', async () => {
    writeConfig({ ignoredRepos: [], maxDepth: 0 });
    await assert.rejects(
      () => loadWorkspaceStatusConfig(cwd),
      /maxDepth must be a positive integer/,
    );
  });

  it('loads and normalizes defaultBranches', async () => {
    writeConfig({
      ignoredRepos: [],
      defaultBranches: {
        ' ./acme/acme-main/ ': ' develop ',
        'acme/acme-frontend': 'develop',
      },
    });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.deepEqual(config.defaultBranches, {
      'acme/acme-main': 'develop',
      'acme/acme-frontend': 'develop',
    });
    assert.equal(defaultBranchOverrideFor('acme/acme-main', config.defaultBranches), 'develop');
    assert.equal(defaultBranchOverrideFor('other', config.defaultBranches), undefined);
  });

  it('rejects non-object defaultBranches', async () => {
    writeConfig({ ignoredRepos: [], defaultBranches: ['develop'] });
    await assert.rejects(
      () => loadWorkspaceStatusConfig(cwd),
      /defaultBranches must be an object/,
    );
  });

  it('rejects non-string defaultBranches values', async () => {
    writeConfig({ ignoredRepos: [], defaultBranches: { app: 1 } });
    await assert.rejects(
      () => loadWorkspaceStatusConfig(cwd),
      /defaultBranches values must be strings/,
    );
  });

  it('omits editor when the key is absent', async () => {
    writeConfig({ ignoredRepos: [] });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.equal(config.editor, undefined);
  });

  it('loads editor cursor', async () => {
    writeConfig({ ignoredRepos: [], editor: 'cursor' });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.equal(config.editor, 'cursor');
  });

  it('loads editor vim', async () => {
    writeConfig({ ignoredRepos: [], editor: 'vim' });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.equal(config.editor, 'vim');
  });

  it('treats blank editor as unset', async () => {
    writeConfig({ ignoredRepos: [], editor: '  ' });
    const config = await loadWorkspaceStatusConfig(cwd);
    assert.equal(config.editor, undefined);
  });

  it('rejects non-string editor', async () => {
    writeConfig({ ignoredRepos: [], editor: 1 });
    await assert.rejects(
      () => loadWorkspaceStatusConfig(cwd),
      /editor must be a string/,
    );
  });

  it('workspaceStatusConfig merges overrides onto defaults', () => {
    assert.deepEqual(
      workspaceStatusConfig({
        ignoredRepos: ['a'],
        maxDepth: 2,
        defaultBranches: { app: 'develop' },
      }),
      {
        ignoredRepos: ['a'],
        maxDepth: 2,
        defaultBranches: { app: 'develop' },
      },
    );
  });
});

describe('isDefaultBranch / getBranchKind with override', () => {
  it('uses legacy main|master|develop when no override', () => {
    assert.equal(isDefaultBranch('main'), true);
    assert.equal(isDefaultBranch('develop'), true);
    assert.equal(isDefaultBranch('feature/x'), false);
    assert.equal(getBranchKind('main'), 'default');
    assert.equal(getBranchKind('feature/x'), 'feature');
  });

  it('treats only the override as default when configured', () => {
    assert.equal(isDefaultBranch('develop', 'develop'), true);
    assert.equal(isDefaultBranch('main', 'develop'), false);
    assert.equal(getBranchKind('main', 'develop'), 'unknown');
    assert.equal(getBranchKind('develop', 'develop'), 'default');
    assert.equal(getBranchKind('feature/x', 'develop'), 'feature');
  });
});
