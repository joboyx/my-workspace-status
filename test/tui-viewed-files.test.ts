import assert from 'node:assert';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { describe, it } from 'node:test';
import { ICON_CLEAN, ICON_SYNCED, ICON_VIEWED, viewedColor } from '../src/tui/icons.js';
import type { FileNode, VisibleRow } from '../src/tui/model/types.js';
import { segmentsText } from '../src/tui/theme.js';
import {
  applyViewedMarks,
  fileNodeIdentity,
  isViewed,
  loadViewedStore,
  reconcileViewed,
  saveViewedStore,
  toggleViewed,
  viewedFingerprint,
  viewedIdentity,
  viewedStorePath,
  type ViewedStore,
} from '../src/tui/viewedFiles.js';

function fileNode(over: Partial<FileNode> = {}): FileNode {
  const filePath = over.path ?? 'src/a.ts';
  const repoPath = over.repoPath ?? 'demo';
  return {
    kind: 'file',
    id: `file:${repoPath}:${filePath}`,
    path: filePath,
    repoPath,
    status: 'M',
    staged: false,
    unstaged: true,
    untracked: false,
    change: { path: filePath, unstagedStatus: 'M' },
    ...over,
  };
}

function fileRow(file: FileNode, trailing = 'M '): VisibleRow {
  return {
    id: file.id,
    depth: 1,
    node: file,
    label: file.path,
    segments: [{ text: file.path }],
    trailing: [{ text: trailing }],
  };
}

describe('viewed identity + fingerprint', () => {
  it('normalizes repo and file paths', () => {
    assert.equal(viewedIdentity('./demo/', 'src\\\\a.ts'), viewedIdentity('demo', 'src/a.ts'));
    assert.equal(fileNodeIdentity(fileNode()), viewedIdentity('demo', 'src/a.ts'));
  });

  it('changes when status letters change (stage/unstage)', () => {
    const content = 'hello\n';
    const dirty = viewedFingerprint({ unstagedStatus: 'M', content });
    const staged = viewedFingerprint({ stagedStatus: 'M', content });
    const both = viewedFingerprint({ stagedStatus: 'M', unstagedStatus: 'M', content });
    assert.notEqual(dirty, staged);
    assert.notEqual(dirty, both);
    assert.notEqual(staged, both);
  });

  it('changes when file bytes change and stays stable for the same bytes', () => {
    const a = viewedFingerprint({ unstagedStatus: 'M', content: 'one\n' });
    const b = viewedFingerprint({ unstagedStatus: 'M', content: 'two\n' });
    const again = viewedFingerprint({ unstagedStatus: 'M', content: 'one\n' });
    assert.notEqual(a, b);
    assert.equal(a, again);
  });
});

describe('toggleViewed + reconcileViewed', () => {
  it('marks, keeps across a reload-shaped store, and unmarks on toggle', () => {
    const identity = viewedIdentity('demo', 'src/a.ts');
    const fingerprint = viewedFingerprint({ unstagedStatus: 'M', content: 'x\n' });
    let store: ViewedStore = {};
    store = toggleViewed(store, identity, fingerprint);
    assert.equal(isViewed(store, identity, fingerprint), true);
    store = toggleViewed(store, identity, fingerprint);
    assert.equal(isViewed(store, identity, fingerprint), false);
  });

  it('clears a mark when the current fingerprint changes', () => {
    const identity = viewedIdentity('demo', 'src/a.ts');
    const marked = viewedFingerprint({ unstagedStatus: 'M', content: 'x\n' });
    const changed = viewedFingerprint({ unstagedStatus: 'M', content: 'y\n' });
    const store = toggleViewed({}, identity, marked);
    const next = reconcileViewed(store, new Map([[identity, changed]]));
    assert.equal(isViewed(next, identity, marked), false);
    assert.equal(Object.keys(next).length, 0);
  });

  it('clears a mark when the file is no longer dirty', () => {
    const identity = viewedIdentity('demo', 'src/a.ts');
    const marked = viewedFingerprint({ unstagedStatus: 'M', content: 'x\n' });
    const store = toggleViewed({}, identity, marked);
    const next = reconcileViewed(store, new Map());
    assert.deepEqual(next, {});
  });

  it('keeps a mark when identity + fingerprint still match', () => {
    const identity = viewedIdentity('demo', 'src/a.ts');
    const marked = viewedFingerprint({ unstagedStatus: 'M', content: 'x\n' });
    const store = toggleViewed({}, identity, marked);
    const next = reconcileViewed(store, new Map([[identity, marked]]));
    assert.equal(next, store);
    assert.equal(isViewed(next, identity, marked), true);
  });
});

describe('viewed store path + load/save', () => {
  it('prefers WS_STATUS_VIEWED_STORE then XDG_STATE_HOME', () => {
    assert.equal(
      viewedStorePath({ WS_STATUS_VIEWED_STORE: '/tmp/viewed.json' }),
      '/tmp/viewed.json',
    );
    assert.equal(
      viewedStorePath({ XDG_STATE_HOME: '/xdg/state', HOME: '/home/joboy' }),
      path.join('/xdg/state', 'my-workspace-status', 'viewed-files.json'),
    );
    assert.equal(
      viewedStorePath({ HOME: '/home/joboy' }),
      path.join('/home/joboy', '.local', 'state', 'my-workspace-status', 'viewed-files.json'),
    );
  });

  it('round-trips a store and treats a missing file as empty', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-viewed-'));
    const file = path.join(dir, 'viewed-files.json');
    const identity = viewedIdentity('demo', 'src/a.ts');
    const fingerprint = viewedFingerprint({ unstagedStatus: 'M', content: 'x\n' });
    saveViewedStore({ [identity]: { fingerprint } }, file);
    assert.deepEqual(loadViewedStore(file), { [identity]: { fingerprint } });
    assert.deepEqual(loadViewedStore(path.join(dir, 'missing.json')), {});
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('does not throw when the store path cannot be written', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ws-viewed-'));
    const blocker = path.join(dir, 'not-a-dir');
    fs.writeFileSync(blocker, 'x');
    assert.doesNotThrow(() => saveViewedStore({}, path.join(blocker, 'viewed-files.json')));
    fs.rmSync(dir, { recursive: true, force: true });
  });
});

describe('applyViewedMarks', () => {
  it('adds a trailing eye on viewed file rows only', () => {
    const file = fileNode();
    const repo: VisibleRow = {
      id: 'repo:demo',
      depth: 0,
      node: {
        kind: 'repo',
        id: 'repo:demo',
        path: 'demo',
        branch: 'main',
        checkoutKind: 'primary',
        mergedIntoDefault: null,
        sync: '=',
        syncStatus: 'up-to-date',
        ignored: false,
        changeCount: 1,
        children: [],
      },
      label: 'demo',
      segments: [{ text: 'demo' }],
      trailing: [],
    };
    const rows = applyViewedMarks([repo, fileRow(file)], new Set([file.id]));
    assert.equal(rows[0], repo);
    assert.ok(segmentsText(rows[1]!.trailing).includes(ICON_VIEWED));
    assert.notEqual(ICON_VIEWED, ICON_CLEAN);
    assert.notEqual(ICON_VIEWED, ICON_SYNCED);
    assert.equal(rows[1]!.trailing[0]?.text, ICON_VIEWED);
    assert.equal(rows[1]!.trailing[0]?.color, viewedColor());
    assert.notEqual(rows[1]!.trailing[0]?.dim, true);
    assert.ok(segmentsText(rows[1]!.trailing).includes('M '));
    assert.equal(rows[1]!.label, file.path);
  });

  it('does not mark a file that is not in the viewed set', () => {
    const file = fileNode();
    const row = fileRow(file);
    const rows = applyViewedMarks([row], new Set());
    assert.equal(rows[0], row);
  });
});
