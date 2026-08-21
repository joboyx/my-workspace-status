/**
 * Live Ink App e2e: git ops, graph, stash, branch picker, worktree remove,
 * stage/revert confirms, edit return, worktree op scope.
 */
import assert from 'node:assert';
import { execSync } from 'node:child_process';
import { after, before, describe, it } from 'node:test';
import {
  GIT_ENV,
  addLinkedWorktree,
  addOrigin,
  commitAll,
  createWorkspace,
  destroyWorkspace,
  git,
  initRepo,
  loadSnapshots,
  makeAhead,
  makeBehind,
  readRepoFile,
  stagedNames,
  stashList,
  stashPush,
  writeRepoFile,
  type WorkspaceHandle,
} from './workspace.js';
import { KEY, delay, press, type, waitFor, withTui, type MountedTui } from './harness.js';

describe('live TUI actions', () => {
  let ws: WorkspaceHandle;

  before(() => {
    ws = createWorkspace();

    initRepo(ws, 'alpha-dirty');
    writeRepoFile(ws, 'alpha-dirty', 'src/note.ts', 'dirty\n');
    writeRepoFile(ws, 'alpha-dirty', 'src/revert-me.ts', 'toss\n');

    initRepo(ws, 'brancher', { branch: 'feature/b' });
    git(ws.path('brancher'), 'branch develop');

    initRepo(ws, 'stashy');
    writeRepoFile(ws, 'stashy', 'wip.txt', 'wip\n');
    stashPush(ws, 'stashy');
    writeRepoFile(ws, 'stashy', 'more.txt', 'more\n');

    initRepo(ws, 'popper');
    stashPush(ws, 'popper');
    writeRepoFile(ws, 'popper', 'more.txt', 'more\n');

    initRepo(ws, 'dropper');
    stashPush(ws, 'dropper');
    writeRepoFile(ws, 'dropper', 'more.txt', 'more\n');

    initRepo(ws, 'reverter');
    writeRepoFile(ws, 'reverter', 'toss.txt', 'keep-me\n');
    commitAll(ws, 'reverter', 'tracked toss');
    writeRepoFile(ws, 'reverter', 'toss.txt', 'discard-me\n');

    initRepo(ws, 'stager');
    writeRepoFile(ws, 'stager', 'ready.txt', 'stage-this\n');

    initRepo(ws, 'filer', { branch: 'feature/files' });
    writeRepoFile(ws, 'filer', 'src/a.ts', 'alpha\n');
    writeRepoFile(ws, 'filer', 'src/b.ts', 'bravo\n');
    commitAll(ws, 'filer', 'two files');

    initRepo(ws, 'family', { branch: 'feature/p' });
    addLinkedWorktree(ws, 'family', 'feature/l');

    initRepo(ws, 'behindy');
    addOrigin(ws, 'behindy');
    makeBehind(ws, 'behindy');

    initRepo(ws, 'aheady');
    addOrigin(ws, 'aheady');
    makeAhead(ws, 'aheady');
  });

  after(() => {
    destroyWorkspace(ws);
  });

  async function mount(fn: Parameters<typeof withTui>[1], editor?: string) {
    const snapshots = await loadSnapshots(ws.root);
    await withTui({ cwd: ws.root, snapshots, editor }, fn);
  }

  async function searchJump(tui: MountedTui, query: string) {
    await press(tui, '/');
    await type(tui, query);
    await press(tui, KEY.enter);
    await delay(40);
  }

  it('stages a file with s', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      await press(tui, 's');
      await waitFor(tui, (frame) => /staged|S  |MS/i.test(frame) || !frame.includes('A  '), 6000);
    });
  });

  it('opens revert confirm and cancels with n', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts') || frame.includes('alpha-dirty'));
      await searchJump(tui, 'revert-me');
      await press(tui, 'x');
      await waitFor(tui, (frame) => frame.includes('Revert') || frame.includes('revert'));
      await press(tui, 'n');
      await waitFor(tui, (frame) => !frame.includes('Revert'));
    });
  });

  it('opens stash menu with S and closes with Esc', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty') || frame.includes('stashy'));
      await searchJump(tui, 'stashy');
      await press(tui, 'S');
      await waitFor(
        tui,
        (frame) =>
          frame.includes('Stash') || frame.includes('push') || frame.includes('Esc cancel'),
      );
      await press(tui, KEY.esc);
      await waitFor(tui, (frame) => !frame.includes('Esc cancel'));
    });
  });

  it('opens branch picker with b', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('brancher') || frame.includes('alpha-dirty'));
      await searchJump(tui, 'brancher');
      await press(tui, 'b');
      await waitFor(
        tui,
        (frame) =>
          frame.includes('Branch') || frame.includes('develop') || frame.includes('filter:'),
        6000,
      );
      assert.match(tui.lastFrame(), /develop|Branch|filter/);
      await press(tui, KEY.esc);
    });
  });

  it('opens worktree remove confirm with W', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('family') || frame.includes('feature'));
      await searchJump(tui, 'feature/l');
      await delay(60);
      await press(tui, 'W');
      await waitFor(
        tui,
        (frame) =>
          frame.includes('Remove worktree') ||
          frame.includes('merged into default') ||
          frame.includes('NOT merged'),
        6000,
      );
      await press(tui, 'n');
      await waitFor(tui, (frame) => !frame.includes('Remove worktree'));
    });
  });

  it('workspace d skips unfocused linked worktrees', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('family') || frame.includes('feature'));
      await type(tui, 'gg');
      await waitFor(tui, (_f, s) => s.cursorId === 'workspace');
      await press(tui, 'd');
      await waitFor(
        tui,
        (frame) => /Switching|default|Pulled|Switched/i.test(frame) || frame.includes('feature/l'),
        8000,
      );
      await delay(400);
      const linkedBranch = execSync('git rev-parse --abbrev-ref HEAD', {
        cwd: ws.path('family/.worktrees/linked'),
        encoding: 'utf8',
        env: GIT_ENV,
      }).trim();
      assert.equal(linkedBranch, 'feature/l');
    });
  });

  it('fetches from the workspace row', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('TREE'));
      await type(tui, 'gg');
      await waitFor(tui, (_f, s) => s.cursorId === 'workspace');
      await press(tui, 'f');
      await waitFor(tui, (frame) => /Fetch|fetch/i.test(frame), 8000);
    });
  });

  it('pulls a behind repo from its row', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('behindy') || frame.includes('TREE'));
      await searchJump(tui, 'behindy');
      await press(tui, 'p');
      await waitFor(tui, (frame) => /Pull|pull|behind/i.test(frame), 8000);
    });
  });

  it('drills into graph and opens create-branch overlay', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('alpha-dirty'));
      await searchJump(tui, 'alpha-dirty');
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'right');
      await waitFor(tui, (frame) => /seed alpha-dirty|seed /i.test(frame), 8000);
      await press(tui, 'j');
      await delay(80);
      await press(tui, 'c');
      await waitFor(tui, (frame) => /Create branch|name:/i.test(frame), 4000);
      await press(tui, KEY.esc);
    });
  });

  it('unmounts on vim edit and stays mounted for Cursor', async () => {
    const snapshots = await loadSnapshots(ws.root);
    await withTui({ cwd: ws.root, snapshots, editor: 'vim' }, async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      await press(tui, 'e');
      await waitFor(tui, () => tui.edits.length > 0 || tui.exits.length > 0, 4000);
      assert.ok(tui.edits.length >= 1, 'vim should record pendingEdit');
    });

    await withTui({ cwd: ws.root, snapshots, editor: 'cursor' }, async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      await press(tui, 'e');
      await delay(200);
      assert.equal(tui.edits.length, 0, 'Cursor must not unmount via pendingEdit');
      assert.match(tui.lastFrame(), /alpha-dirty|note\.ts|TREE/);
    });
  });

  it('prompts on first Ctrl-C then quits on the second', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('TREE'));
      await press(tui, KEY.ctrlC);
      await waitFor(tui, (frame) => /Ctrl\+C again to exit/i.test(frame));
      await press(tui, KEY.ctrlC);
      await waitFor(tui, () => tui.exits.length > 0);
      assert.equal(tui.exits[0]?.type, 'quit');
    });
  });

  it('unstages a file with u', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      await press(tui, 's');
      await delay(200);
      await press(tui, 'u');
      await waitFor(tui, (frame) => /unstage|Unstaged|\?\?| A /i.test(frame), 6000);
    });
  });

  it('pushes an ahead repo with P', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('aheady') || frame.includes('TREE'));
      await searchJump(tui, 'aheady');
      await press(tui, 'P');
      await waitFor(tui, (frame) => /Pushed|push/i.test(frame), 8000);
    });
  });

  it('toggles full-file context with ctrl+o', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('note.ts'));
      await searchJump(tui, 'note.ts');
      await press(tui, KEY.ctrlO);
      await waitFor(tui, (frame, s) => s.fullContext.size > 0 || /full/i.test(frame), 4000);
    });
  });

  it('opens graph checkout picker with b on a multi-ref commit', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('TREE'));
      await press(tui, 'G');
      await press(tui, 'l');
      await waitFor(
        tui,
        (frame) => frame.includes('brancher') || frame.includes('No updates'),
        4000,
      );
      await searchJump(tui, 'brancher');
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'right');
      await waitFor(tui, (frame) => /seed brancher|seed /i.test(frame), 8000);
      await press(tui, 'j');
      await delay(80);
      await press(tui, 'b');
      await waitFor(tui, (frame) => /Checkout|develop|feature\/b/i.test(frame), 6000);
      await press(tui, KEY.esc);
    });
  });

  it('applies a graph stash with a', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('stashy'));
      await searchJump(tui, 'stashy');
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'right');
      await waitFor(tui, (frame) => /tui-e2e-stash|stash@/i.test(frame), 8000);
      await press(tui, 'j');
      await delay(80);
      await press(tui, 'a');
      await waitFor(tui, (frame) => /Applied|stash apply|stash@/i.test(frame), 8000);
    });
  });

  it('refreshes with r', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('TREE'));
      writeRepoFile(ws, 'alpha-dirty', 'src/fresh.ts', 'fresh\n');
      await press(tui, 'r');
      await waitFor(tui, (frame) => frame.includes('fresh.ts'), 8000);
    });
  });

  it('pops a graph stash with p', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('popper'));
      await searchJump(tui, 'popper');
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'right');
      await waitFor(tui, (frame) => /tui-e2e-stash|stash@/i.test(frame), 8000);
      await press(tui, 'j');
      await delay(80);
      await press(tui, 'p');
      await waitFor(tui, (frame) => /Popped|stash-me\.txt/i.test(frame), 8000);
      assert.equal(stashList(ws, 'popper').trim(), '');
      assert.equal(readRepoFile(ws, 'popper', 'stash-me.txt'), 'stash\n');
    });
  });

  it('drops a graph stash after confirm y', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('dropper'));
      await searchJump(tui, 'dropper');
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'right');
      await waitFor(tui, (frame) => /tui-e2e-stash|stash@/i.test(frame), 8000);
      await press(tui, 'j');
      await delay(80);
      await press(tui, 'D');
      await waitFor(tui, (frame) => /Drop.*stash@/i.test(frame));
      await press(tui, 'y');
      await waitFor(tui, (frame) => /Dropped|Drop stash failed/i.test(frame), 8000);
      assert.match(tui.lastFrame(), /Dropped/i);
      assert.equal(stashList(ws, 'dropper').trim(), '');
      assert.equal(readRepoFile(ws, 'dropper', 'stash-me.txt'), null);
    });
  });

  it('reverts a tracked file after confirm y', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('toss.txt') || frame.includes('reverter'));
      await searchJump(tui, 'toss.txt');
      await press(tui, 'x');
      await waitFor(tui, (frame) => frame.includes('Revert') && frame.includes('toss.txt'));
      await press(tui, 'y');
      await waitFor(tui, (frame) => /Reverted toss\.txt/i.test(frame), 8000);
      assert.equal(readRepoFile(ws, 'reverter', 'toss.txt'), 'keep-me\n');
    });
  });

  it('stages a file and writes the index', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('ready.txt') || frame.includes('stager'));
      await searchJump(tui, 'ready.txt');
      await press(tui, 's');
      await waitFor(tui, (frame) => /Staged ready\.txt/i.test(frame), 8000);
      assert.match(stagedNames(ws, 'stager'), /ready\.txt/);
    });
  });

  it('drills into commit files then navigates, selects, and exits', async () => {
    await mount(async (tui) => {
      await waitFor(tui, (frame) => frame.includes('filer'));
      await searchJump(tui, 'filer');
      await press(tui, KEY.esc);
      await waitFor(tui, (_f, s) => s.search === null);
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'right');
      await waitFor(tui, (frame) => /two files|seed filer/i.test(frame), 8000);
      await press(tui, 'j');
      await delay(80);
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.stack.some((v) => v.kind === 'repoGraph'));
      await waitFor(tui, (frame) => frame.includes('a.ts') && frame.includes('b.ts'), 8000);
      await press(tui, 'j');
      await delay(80);
      await press(tui, KEY.enter);
      await waitFor(tui, (_f, s) => s.nav.stack.some((v) => v.kind === 'commitFiles'));
      assert.equal(tui.session().nav.focusPane, 'right');
      await press(tui, KEY.esc);
      await waitFor(tui, (_f, s) => s.nav.focusPane === 'left');
      await press(tui, KEY.esc);
      await waitFor(tui, (_f, s) => {
        const top = s.nav.stack[s.nav.stack.length - 1];
        return top?.kind === 'repoGraph' && s.nav.focusPane === 'left';
      });
    });
  });
});
