/**
 * Live Ink App e2e: clean-check gating + viewed eye glyph.
 */
import assert from 'node:assert';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { after, before, describe, it } from 'node:test';
import { ICON_CLEAN, ICON_SYNCED, ICON_VIEWED } from '../../src/tui/icons.js';
import {
  addOrigin,
  createWorkspace,
  destroyWorkspace,
  initRepo,
  loadSnapshots,
  writeRepoFile,
  type WorkspaceHandle,
} from './workspace.js';
import { delay, press, type, waitFor, withTui, type MountedTui } from './harness.js';

describe('live TUI check glyphs', () => {
  let ws: WorkspaceHandle;
  let storePath: string;

  before(() => {
    ws = createWorkspace();
    initRepo(ws, 'alpha-dirty');
    addOrigin(ws, 'alpha-dirty');
    writeRepoFile(ws, 'alpha-dirty', 'src/note.ts', 'dirty\n');

    initRepo(ws, 'clean-repo');
    addOrigin(ws, 'clean-repo');

    storePath = path.join(os.tmpdir(), `ws-check-glyphs-e2e.${process.pid}.json`);
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

  function lineWith(frame: string, token: string): string {
    return frame.split('\n').find((row) => row.includes(token)) ?? '';
  }

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

  it('does not paint the clean check on dirty repo or folder rows', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty') && frame.includes('note.ts'));
      const repoLine = lineWith(tui.lastFrame(), 'alpha-dirty');
      const folderLine = lineWith(tui.lastFrame(), 'src');
      assert.ok(repoLine, 'expected dirty repo row');
      assert.ok(folderLine, 'expected src folder row');
      assert.ok(!repoLine.includes(ICON_CLEAN), repoLine);
      assert.ok(!repoLine.includes(ICON_SYNCED) || ICON_SYNCED === ICON_CLEAN, repoLine);
      assert.ok(!folderLine.includes(ICON_CLEAN), folderLine);
    });
  });

  it('keeps the clean check on rows inside No updates', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('No updates'));
      await searchJump(tui, 'No updates');
      await press(tui, 'l');
      await waitFor(tui, (frame) => frame.includes('clean-repo'));
      const groupLine = lineWith(tui.lastFrame(), 'No updates');
      const cleanLine = lineWith(tui.lastFrame(), 'clean-repo');
      assert.ok(groupLine.includes(ICON_CLEAN), groupLine);
      assert.ok(cleanLine.includes(ICON_CLEAN), cleanLine);
    });
  });

  it('marks a dirty file with the eye, not the clean check', async () => {
    try {
      fs.unlinkSync(storePath);
    } catch {
      /* first run */
    }
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      await press(tui, ' ');
      await waitFor(tui, (frame) => treeFileLine(frame, 'note.ts').includes(ICON_VIEWED));
      const fileLine = treeFileLine(tui.lastFrame(), 'note.ts');
      assert.notEqual(ICON_VIEWED, ICON_CLEAN);
      assert.ok(fileLine.includes(ICON_VIEWED), fileLine);
      assert.ok(!fileLine.includes(ICON_CLEAN), fileLine);
    });
  });
});
