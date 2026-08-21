import assert from 'node:assert';
import { describe, it } from 'node:test';
import * as fs from 'node:fs/promises';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  changeSignatures,
  changedNodeIds,
  fileNodeId,
  mergeSignatures,
  treeChromeSignatures,
} from '../src/tui/watch.js';
import { buildTree } from '../src/tui/model/tree.js';
import type { RepoSnapshot } from '../src/types.js';

function snapshot(partial: Partial<RepoSnapshot>): RepoSnapshot {
  return {
    repo: 'demo',
    branch: 'main',
    syncStatus: 'up-to-date',
    syncNote: '',
    hasUnstaged: false,
    hasStaged: false,
    hasUntracked: false,
    unstagedInfo: '',
    stagedFiles: '',
    unstagedFiles: '',
    untrackedFiles: '',
    checkoutKind: 'primary',
    mergedIntoDefault: null,
    ...partial,
  };
}

function treeOf(snapshots: RepoSnapshot[], treeMode = true) {
  return buildTree({
    snapshots,
    ignoredRepos: new Set(),
    treeMode,
    workspaceLabel: 'ws',
  });
}

const dirtyApp = snapshot({
  repo: 'app',
  branch: 'feature/x',
  hasUnstaged: true,
  unstagedFiles: 'M\tsrc/main.ts|||M\tsrc/util/helpers.ts',
});

const cleanDocs = snapshot({ repo: 'docs' });

const familyPrimary = snapshot({
  repo: 'api',
  hasUnstaged: true,
  unstagedFiles: 'M\ta.ts',
});

const familyLinked = snapshot({
  repo: 'api/.worktrees/feat',
  branch: 'feature/feat',
  checkoutKind: 'linked',
  primaryRepo: 'api',
  mergedIntoDefault: false,
  hasUnstaged: true,
  unstagedFiles: 'M\tb.ts',
});

describe('treeChromeSignatures', () => {
  it('includes workspace, repo, checkout, dir, and group ids; excludes files', () => {
    const tree = treeOf(
      [dirtyApp, cleanDocs, familyPrimary, familyLinked],
      true,
    );
    const chrome = treeChromeSignatures(tree);

    assert.ok(chrome.has('workspace'));
    assert.ok(chrome.has('repo:app'));
    assert.ok(chrome.has('repo:api'));
    assert.ok(chrome.has('checkout:api'));
    assert.ok(chrome.has('checkout:api/.worktrees/feat'));
    assert.ok(chrome.has('dir:app:src'));
    assert.ok(chrome.has('dir:app:src/util'));
    assert.ok(chrome.has('group:no-updates'));

    for (const id of chrome.keys()) {
      assert.ok(!id.startsWith('file:'), `chrome map must skip files: ${id}`);
    }
  });

  it('changing only syncStatus updates the repo and workspace signatures', () => {
    const beforeSnap = snapshot({
      repo: 'demo',
      branch: 'feature/x',
      hasUnstaged: true,
      unstagedFiles: 'M\ta.ts',
    });
    const afterSnap = snapshot({
      ...beforeSnap,
      syncStatus: 'behind',
      syncNote: 'behind by 2',
    });
    const before = treeChromeSignatures(treeOf([beforeSnap]));
    const after = treeChromeSignatures(treeOf([afterSnap]));
    const changed = changedNodeIds(before, after);
    assert.ok(changed.includes('repo:demo'), String(changed));
    assert.ok(changed.includes('workspace'), String(changed));
  });

  it('changing only branch updates the repo signature', () => {
    const beforeSnap = snapshot({
      repo: 'demo',
      branch: 'feature/x',
      hasUnstaged: true,
      unstagedFiles: 'M\ta.ts',
    });
    const afterSnap = snapshot({ ...beforeSnap, branch: 'feature/y' });
    const changed = changedNodeIds(
      treeChromeSignatures(treeOf([beforeSnap])),
      treeChromeSignatures(treeOf([afterSnap])),
    );
    assert.ok(changed.includes('repo:demo'), String(changed));
  });

  it('changing only changeCount updates the repo and workspace signatures', () => {
    const beforeSnap = snapshot({
      repo: 'demo',
      branch: 'feature/x',
      hasUnstaged: true,
      unstagedFiles: 'M\ta.ts',
    });
    const afterSnap = snapshot({
      ...beforeSnap,
      unstagedFiles: 'M\ta.ts|||M\tb.ts',
    });
    const changed = changedNodeIds(
      treeChromeSignatures(treeOf([beforeSnap])),
      treeChromeSignatures(treeOf([afterSnap])),
    );
    assert.ok(changed.includes('repo:demo'), String(changed));
    assert.ok(changed.includes('workspace'), String(changed));
  });

  it('changing mergedIntoDefault or checkoutKind updates the repo signature', () => {
    const baseSnap = snapshot({
      repo: 'demo',
      branch: 'feature/x',
      mergedIntoDefault: false,
      hasUnstaged: true,
      unstagedFiles: 'M\ta.ts',
    });
    const merged = changedNodeIds(
      treeChromeSignatures(treeOf([baseSnap])),
      treeChromeSignatures(
        treeOf([{ ...baseSnap, mergedIntoDefault: true }]),
      ),
    );
    assert.ok(merged.includes('repo:demo'), String(merged));

    const kind = changedNodeIds(
      treeChromeSignatures(treeOf([baseSnap])),
      treeChromeSignatures(
        treeOf([
          {
            ...baseSnap,
            checkoutKind: 'linked',
            primaryRepo: 'app',
          },
        ]),
      ),
    );
    assert.ok(kind.includes('repo:demo'), String(kind));
  });

  it('childKindCount (children.length) flashes a family repo when a worktree is added', () => {
    const before = treeChromeSignatures(
      treeOf([familyPrimary, familyLinked]),
    );
    const extra = snapshot({
      repo: 'api/.worktrees/other',
      branch: 'feature/other',
      checkoutKind: 'linked',
      primaryRepo: 'api',
      hasUnstaged: true,
      unstagedFiles: 'M\tc.ts',
    });
    const after = treeChromeSignatures(
      treeOf([familyPrimary, familyLinked, extra]),
    );
    const changed = changedNodeIds(before, after);
    assert.ok(changed.includes('repo:api'), String(changed));
  });
});

describe('dir chrome vs file mtime signatures', () => {
  it('adding a file under a dir changes the dir signature; the file still comes from changeSignatures', async () => {
    const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'ws-chrome-'));
    try {
      await fs.mkdir(path.join(dir, 'demo/src'), { recursive: true });
      await fs.writeFile(path.join(dir, 'demo/src/a.ts'), 'a\n');
      await fs.writeFile(path.join(dir, 'demo/src/b.ts'), 'b\n');

      const one = snapshot({
        repo: 'demo',
        hasUnstaged: true,
        unstagedFiles: 'M\tsrc/a.ts',
      });
      const two = snapshot({
        repo: 'demo',
        hasUnstaged: true,
        unstagedFiles: 'M\tsrc/a.ts|||M\tsrc/b.ts',
      });

      const dirId = 'dir:demo:src';
      const chromeBefore = treeChromeSignatures(treeOf([one]));
      const chromeAfter = treeChromeSignatures(treeOf([two]));
      assert.ok(chromeBefore.has(dirId));
      assert.ok(chromeAfter.has(dirId));
      assert.notEqual(chromeBefore.get(dirId), chromeAfter.get(dirId));
      assert.ok(changedNodeIds(chromeBefore, chromeAfter).includes(dirId));

      const fileId = fileNodeId('demo', 'src/b.ts');
      assert.ok(!chromeAfter.has(fileId));

      const fileBefore = await changeSignatures(dir, [one]);
      const fileAfter = await changeSignatures(dir, [two]);
      assert.ok(fileAfter.has(fileId));
      assert.ok(!fileBefore.has(fileId));
      assert.ok(changedNodeIds(fileBefore, fileAfter).includes(fileId));
    } finally {
      await fs.rm(dir, { recursive: true, force: true });
    }
  });
});

describe('mergeSignatures', () => {
  it('unions without dropping mtime-only file changes', () => {
    const files = new Map([
      ['file:a:x', 'M:10:100'],
      ['file:a:y', 'M:20:200'],
    ]);
    const chrome = new Map([
      ['workspace', '2|all current'],
      ['repo:a', 'main|synced|up-to-date|2|null|primary|2'],
    ]);
    const merged = mergeSignatures(files, chrome);
    assert.equal(merged.get('file:a:x'), 'M:10:100');
    assert.equal(merged.get('file:a:y'), 'M:20:200');
    assert.equal(merged.get('workspace'), '2|all current');
    assert.equal(
      merged.get('repo:a'),
      'main|synced|up-to-date|2|null|primary|2',
    );
    assert.equal(merged.size, 4);

    const before = mergeSignatures(
      new Map([['file:a:x', 'M:1:1']]),
      new Map([
        ['workspace', '1|all current'],
        ['repo:a', 'old'],
      ]),
    );
    const after = mergeSignatures(
      new Map([['file:a:x', 'M:1:2']]),
      new Map([
        ['workspace', '1|all current'],
        ['repo:a', 'new'],
      ]),
    );
    assert.deepEqual(changedNodeIds(before, after).sort(), [
      'file:a:x',
      'repo:a',
    ]);
  });
});
