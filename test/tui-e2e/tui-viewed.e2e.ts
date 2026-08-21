/**
 * Live Ink App e2e: reviewed (space) marks on dirty file rows.
 */
import assert from 'node:assert';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import { ICON_CLEAN, ICON_VIEWED } from '../../src/tui/icons.js';
import {
  createWorkspace,
  destroyWorkspace,
  git,
  initRepo,
  loadSnapshots,
  writeRepoFile,
  type WorkspaceHandle,
} from './workspace.js';
import { delay, press, type, waitFor, withTui, type MountedTui } from './harness.js';

describe('live TUI viewed files', () => {
  let ws: WorkspaceHandle;
  let storePath: string;

  before(() => {
    ws = createWorkspace();
    initRepo(ws, 'alpha-dirty');
    writeRepoFile(ws, 'alpha-dirty', 'src/note.ts', 'dirty\n');
    writeRepoFile(ws, 'alpha-dirty', 'src/other.ts', 'other\n');
    storePath = path.join(os.tmpdir(), `ws-viewed-e2e.${process.pid}.json`);
    process.env.WS_STATUS_VIEWED_STORE = storePath;
  });

  after(() => {
    destroyWorkspace(ws);
    try {
      fs.unlinkSync(storePath);
    } catch {
      /* ignore */
    }
    delete process.env.WS_STATUS_VIEWED_STORE;
  });

  function resetStore() {
    try {
      fs.unlinkSync(storePath);
    } catch {
      /* first run */
    }
  }

  async function mount(fn: (tui: MountedTui) => Promise<void>) {
    const snapshots = await loadSnapshots(ws.root);
    await withTui({ cwd: ws.root, snapshots }, fn);
  }

  async function searchJump(tui: MountedTui, query: string) {
    await press(tui, '/');
    await type(tui, query);
    await press(tui, '\r');
    await delay(40);
  }

  /** Tree-pane file row, not the diff path header. */
  function treeFileLine(frame: string, name: string): string {
    const lines = frame.split('\n').filter((row) => row.includes(name));
    return (
      lines.find(
        (row) => /\s[AMSDU]/.test(row) && !row.includes(`${name}.`) && !row.includes('/'),
      ) ??
      lines.find((row) => /\s[AMSDU]/.test(row)) ??
      ''
    );
  }

  function isViewed(frame: string, name: string): boolean {
    return treeFileLine(frame, name).includes(ICON_VIEWED);
  }

  it('marks a dirty file with space and unmarks on a second space', async () => {
    resetStore();
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      assert.equal(isViewed(tui.lastFrame(), 'note.ts'), false);
      await press(tui, ' ');
      await waitFor(tui, (frame) => isViewed(frame, 'note.ts'));
      assert.notEqual(ICON_VIEWED, ICON_CLEAN);
      assert.ok(!treeFileLine(tui.lastFrame(), 'note.ts').includes(ICON_CLEAN));
      assert.equal(isViewed(tui.lastFrame(), 'other.ts'), false);
      await press(tui, ' ');
      await waitFor(tui, (frame) => !isViewed(frame, 'note.ts'));
    });
  });

  it('ignores space on repo and workspace rows (does not fold)', async () => {
    resetStore();
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      await searchJump(tui, 'alpha-dirty');
      await press(tui, ' ');
      await delay(80);
      assert.match(tui.lastFrame(), /note\.ts/);
      assert.equal(isViewed(tui.lastFrame(), 'note.ts'), false);
      assert.equal(isViewed(tui.lastFrame(), 'other.ts'), false);
      await type(tui, 'gg');
      await waitFor(tui, (_f, s) => s.cursorId === 'workspace');
      await press(tui, ' ');
      await delay(80);
      assert.equal(isViewed(tui.lastFrame(), 'note.ts'), false);
    });
  });

  it('keeps the mark after remount when contents are unchanged', async () => {
    resetStore();
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      await press(tui, ' ');
      await waitFor(tui, (frame) => isViewed(frame, 'note.ts'));
    });
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await waitFor(tui, (frame) => isViewed(frame, 'note.ts'));
    });
  });

  it('clears the mark when file contents change', async () => {
    resetStore();
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('other.ts'));
      await searchJump(tui, 'other.ts');
      await press(tui, ' ');
      await waitFor(tui, (frame) => isViewed(frame, 'other.ts'));
    });
    writeRepoFile(ws, 'alpha-dirty', 'src/other.ts', 'other-changed\n');
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('other.ts'));
      await delay(80);
      assert.equal(isViewed(tui.lastFrame(), 'other.ts'), false);
    });
  });

  it('clears the mark when stage changes the status token', async () => {
    resetStore();
    writeRepoFile(ws, 'alpha-dirty', 'src/stage-me.ts', 'stage-this\n');
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('stage-me.ts'));
      await searchJump(tui, 'stage-me.ts');
      await press(tui, ' ');
      await waitFor(tui, (frame) => isViewed(frame, 'stage-me.ts'));
      await press(tui, 's');
      await waitFor(tui, (frame) => !isViewed(frame, 'stage-me.ts'), 6000);
      git(ws.path('alpha-dirty'), 'restore --staged src/stage-me.ts');
    });
  });
});
