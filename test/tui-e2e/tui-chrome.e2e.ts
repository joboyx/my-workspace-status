/**
 * Live Ink App e2e: navigation, fold, search, help, theme, mouse,
 * view modes, ignore toggle. Drives App through stdin — not helpers.
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
import {
  KEY,
  delay,
  mouseClickSeq,
  mouseDragSeq,
  mouseWheelDownSeq,
  press,
  type,
  waitFor,
  withTui,
} from './harness.js';

describe('live TUI chrome', () => {
  let ws: WorkspaceHandle;

  before(() => {
    ws = createWorkspace();
    initRepo(ws, 'alpha-dirty');
    writeRepoFile(ws, 'alpha-dirty', 'src/note.ts', 'dirty\n');
    writeRepoFile(ws, 'alpha-dirty', 'src/other.ts', 'other\n');
    initRepo(ws, 'beta-clean');
    initRepo(ws, 'zeta-ignored');
    writeRepoFile(ws, 'zeta-ignored', 'hidden.txt', 'secret\n');
  });

  after(() => {
    destroyWorkspace(ws);
  });

  async function mount(fn: Parameters<typeof withTui>[1], showIgnored = false) {
    const snapshots = await loadSnapshots(ws.root);
    await withTui({ cwd: ws.root, snapshots, ignoredRepos: ['zeta-ignored'], showIgnored }, fn);
  }

  it('renders TREE/DIFF chrome and the dirty repo', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty') && frame.includes('TREE'));
      assert.match(tui.lastFrame(), /note\.ts/);
      assert.doesNotMatch(tui.lastFrame(), /zeta-ignored/);
    });
  });

  it('navigates j/k, gg/G, Enter/Esc pane focus, PageDown', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      const start = tui.session().cursorId;
      await press(tui, 'j');
      await waitFor(tui, (_f, s) => s.cursorId !== start);
      await type(tui, 'gg');
      await waitFor(tui, (_f, s) => s.cursorId === 'workspace');
      await press(tui, 'G');
      await waitFor(tui, (_f, s) => s.cursorId !== 'workspace');
      await type(tui, 'gg');
      await waitFor(tui, (_f, s) => s.cursorId === 'workspace');
      assert.equal(tui.session().nav.focusPane, 'left');
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'right');
      await press(tui, KEY.esc);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'left');
      await press(tui, KEY.pageDown);
      assert.equal(tui.session().nav.focusPane, 'left');
    });
  });

  it('folds and unfolds with z, h, and l (space does not fold)', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      // File is focused; walk up to the repo row.
      await press(tui, 'k');
      await press(tui, 'k');
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      const before = tui.lastFrame();
      assert.match(before, /note\.ts/);
      await press(tui, ' ');
      await delay(80);
      assert.match(tui.lastFrame(), /note\.ts/);
      await press(tui, 'z');
      await waitFor(tui, (frame) => !frame.includes('note.ts'));
      await press(tui, 'z');
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await press(tui, 'l');
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await press(tui, 'h');
      await waitFor(tui, (frame) => !frame.includes('note.ts'));
    });
  });

  it('searches with / then n/N after Enter', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      await press(tui, '/');
      await waitFor(tui, (frame) => /arms query|search/i.test(frame));
      await type(tui, 'note');
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => Boolean(s.search?.query));
      assert.equal(tui.session().search?.query, 'note');
      const at = tui.session().cursorId;
      await press(tui, 'n');
      await delay(40);
      await press(tui, 'N');
      await delay(40);
      assert.ok(tui.session().cursorId);
      assert.ok(at);
      await press(tui, KEY.esc);
      await waitFor(tui, (_f, s) => s.search === null);
    });
  });

  it('opens help, searches help, and closes', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      await delay(40);
      await press(tui, '?');
      await waitFor(
        tui,
        (frame) =>
          frame.includes('MOVE') || frame.includes('down / up') || frame.includes('search help'),
        4000,
      );
      await press(tui, '/');
      await type(tui, 'fold');
      await waitFor(tui, (frame) => /fold|Esc clears search/i.test(frame));
      await press(tui, KEY.esc);
      await press(tui, KEY.esc);
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
    });
  });

  it('cycles theme with T', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      const start = tui.session().theme;
      await press(tui, 'T');
      await waitFor(tui, (_f, s) => s.theme !== start);
      assert.notEqual(tui.session().theme, start);
    });
  });

  it('toggles mouse with m and accepts a click/wheel CSI', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      assert.equal(tui.session().mouseEnabled, true);
      await press(tui, 'm');
      await waitFor(tui, (_f, s) => s.mouseEnabled === false);
      await press(tui, 'm');
      await waitFor(tui, (_f, s) => s.mouseEnabled === true);
      const before = tui.session().cursorId;
      tui.stdin.write(mouseClickSeq(5, 4));
      await delay(80);
      tui.stdin.write(mouseWheelDownSeq(5, 8));
      await delay(80);
      assert.ok(tui.session().cursorId || before);
    });
  });

  it('toggles tree/flat and diff mode', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      const treeOn = tui.session().treeMode;
      await press(tui, 't');
      await waitFor(tui, (_f, s) => s.treeMode !== treeOn);
      await press(tui, 't');
      await waitFor(tui, (_f, s) => s.treeMode === treeOn);
      const mode = tui.session().diffMode;
      await press(tui, 'i');
      await waitFor(tui, (_f, s) => s.diffMode !== mode);
    });
  });

  it('toggles ignored repos with .', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      assert.doesNotMatch(tui.lastFrame(), /zeta-ignored/);
      await press(tui, '.');
      await waitFor(
        tui,
        (frame) => frame.includes('zeta-ignored') || frame.includes('Ignored repos shown'),
      );
      assert.match(tui.lastFrame(), /zeta-ignored|Ignored repos shown/);
      await press(tui, '.');
      await waitFor(tui, (frame) => !frame.includes('zeta-ignored'));
    });
  });

  it('shows ignored repos when launched with showIgnored (CLI -a)', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('zeta-ignored'));
    }, true);
  });

  it('starts EasyMotion with semicolon', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      await press(tui, ';');
      await waitFor(tui, (_f, s) => s.easyMotion === true);
      await press(tui, KEY.esc);
      await waitFor(tui, (_f, s) => s.easyMotion === false);
    });
  });

  it('drags the in-diff split separator with mouse CSI', async () => {
    const snapshots = await loadSnapshots(ws.root);
    await withTui(
      {
        cwd: ws.root,
        snapshots,
        ignoredRepos: ['zeta-ignored'],
        columns: 220,
        rows: 40,
      },
      async (tui) => {
        await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
        await press(tui, '/');
        await type(tui, 'note.ts');
        await waitFor(tui, (frame) => frame.includes('note.ts'));
        await press(tui, KEY.esc);
        await waitFor(
          tui,
          (frame) => /split/.test(frame) && /dirty|NEW|note\.ts/.test(frame),
          6000,
        );

        const splitCol = (frame: string): number | null => {
          for (const line of frame.split('\n')) {
            const cols: number[] = [];
            for (let i = 0; i < line.length; i++) {
              if (line[i] === '│') cols.push(i);
            }
            if (cols.length >= 3 && /[+-]/.test(line)) return cols[2];
          }
          return null;
        };

        const before = splitCol(tui.lastFrame());
        const treeWidth = Math.floor(220 * 0.4);
        const diffWidth = Math.max(20, 220 - treeWidth - 2);
        const leftWidth = Math.floor((diffWidth - 1) / 2);
        const ruleX = treeWidth + 2 + leftWidth;
        tui.stdin.write(mouseDragSeq(ruleX, 8, ruleX + 24, 8));
        await delay(150);
        await waitFor(
          tui,
          (frame) => {
            const after = splitCol(frame);
            return before !== null && after !== null && after > before;
          },
          4000,
        );
        const after = splitCol(tui.lastFrame());
        assert.ok(before !== null && after !== null, 'expected a painted split RULE');
        assert.ok(after > before, `split col ${before} -> ${after}`);
      },
    );
  });
});
