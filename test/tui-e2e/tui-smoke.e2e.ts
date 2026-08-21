/**
 * Live Ink App smoke — proves the harness mounts React/Ink, not helpers.
 */
import assert from 'node:assert';
import { after, before, describe, it } from 'node:test';
import {
  createWorkspace,
  destroyWorkspace,
  initRepo,
  loadSnapshots,
  writeRepoFile,
  type WorkspaceHandle,
} from './workspace.js';
import { KEY, press, type, waitFor, withTui } from './harness.js';

describe('live TUI smoke', () => {
  let ws: WorkspaceHandle;

  before(() => {
    ws = createWorkspace();
    initRepo(ws, 'alpha-dirty');
    writeRepoFile(ws, 'alpha-dirty', 'src/note.ts', 'dirty\n');
    initRepo(ws, 'beta-clean');
  });

  after(() => {
    destroyWorkspace(ws);
  });

  it('renders App and shows the dirty repo', async () => {
    const snapshots = await loadSnapshots(ws.root);
    await withTui({ cwd: ws.root, snapshots }, async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      assert.match(tui.lastFrame(), /alpha-dirty/);
      assert.match(tui.lastFrame(), /note\.ts|TREE|GRAPH|DIFF/i);
    });
  });

  it('moves the cursor with j/k and restores with gg', async () => {
    const snapshots = await loadSnapshots(ws.root);
    await withTui({ cwd: ws.root, snapshots }, async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      const startId = tui.session().cursorId;
      await press(tui, 'j');
      await waitFor(tui, (_f, s) => s.cursorId !== startId);
      await type(tui, 'gg');
      await waitFor(tui, (_f, s) => s.cursorId === 'workspace' || s.nav.focusPane === 'left');
      assert.ok(tui.session().cursorId);
    });
  });

  it('opens help and closes it', async () => {
    const snapshots = await loadSnapshots(ws.root);
    await withTui({ cwd: ws.root, snapshots }, async (tui) => {
      await press(tui, '?');
      await waitFor(tui, (frame) => frame.includes('MOVE') && frame.includes('GIT'));
      assert.match(tui.lastFrame(), /search help|VIEW/);
      await press(tui, KEY.esc);
      await waitFor(tui, (frame) => !frame.includes('search help') || frame.includes('alpha-dirty'));
    });
  });
});
