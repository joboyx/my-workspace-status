import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  ALT_SCREEN_ENTER,
  ALT_SCREEN_LEAVE,
  SHOW_CURSOR,
  createAlternateScreen,
  withAlternateScreen,
} from '../src/tui/alternateScreen.js';

type FakeStdout = {
  isTTY: boolean;
  writes: string[];
  write: (s: string) => boolean;
};

function fakeStdout(isTTY: boolean): FakeStdout {
  const out: FakeStdout = {
    isTTY,
    writes: [],
    write(s: string) {
      out.writes.push(s);
      return true;
    },
  };
  return out;
}

/** Flatten writes so leave may emit leave+cursor as one chunk or two. */
function joinedWrites(stdout: FakeStdout): string {
  return stdout.writes.join('');
}

type FakeHooks = {
  beforeExit: Set<() => void>;
  exit: Set<() => void>;
  register: (event: 'beforeExit' | 'exit', listener: () => void) => void;
  unregister: (event: 'beforeExit' | 'exit', listener: () => void) => void;
};

function fakeProcessHooks(): FakeHooks {
  const beforeExit = new Set<() => void>();
  const exit = new Set<() => void>();
  return {
    beforeExit,
    exit,
    register(event, listener) {
      (event === 'beforeExit' ? beforeExit : exit).add(listener);
    },
    unregister(event, listener) {
      (event === 'beforeExit' ? beforeExit : exit).delete(listener);
    },
  };
}

describe('alternate screen sequences', () => {
  it('exports DECSET/DECRST 1049 (enter/leave alternate buffer)', () => {
    assert.equal(ALT_SCREEN_ENTER, '\x1b[?1049h');
    assert.equal(ALT_SCREEN_LEAVE, '\x1b[?1049l');
  });

  it('exports show-cursor CSI used on leave', () => {
    assert.equal(SHOW_CURSOR, '\x1b[?25h');
  });
});

describe('createAlternateScreen', () => {
  it('enters only when stdout is a TTY', () => {
    const tty = fakeStdout(true);
    const nonTty = fakeStdout(false);
    createAlternateScreen(tty).enter();
    createAlternateScreen(nonTty).enter();
    assert.deepEqual(tty.writes, [ALT_SCREEN_ENTER]);
    assert.deepEqual(nonTty.writes, []);
  });

  it('leave is a no-op until enter, then writes leave + show cursor once', () => {
    const stdout = fakeStdout(true);
    const screen = createAlternateScreen(stdout);
    screen.leave();
    assert.deepEqual(stdout.writes, []);
    screen.enter();
    screen.leave();
    assert.ok(joinedWrites(stdout).includes(ALT_SCREEN_ENTER));
    assert.ok(joinedWrites(stdout).includes(ALT_SCREEN_LEAVE));
    assert.ok(joinedWrites(stdout).includes(SHOW_CURSOR));
    assert.ok(
      joinedWrites(stdout).indexOf(ALT_SCREEN_LEAVE) <
        joinedWrites(stdout).indexOf(SHOW_CURSOR) ||
        joinedWrites(stdout).includes(ALT_SCREEN_LEAVE + SHOW_CURSOR),
    );
    const afterFirstLeave = joinedWrites(stdout);
    screen.leave();
    assert.equal(joinedWrites(stdout), afterFirstLeave);
  });

  it('re-enter after leave writes enter again (editor remount loop)', () => {
    const stdout = fakeStdout(true);
    const screen = createAlternateScreen(stdout);
    screen.enter();
    screen.leave();
    screen.enter();
    screen.leave();
    const joined = joinedWrites(stdout);
    const enterCount = joined.split(ALT_SCREEN_ENTER).length - 1;
    const leaveCount = joined.split(ALT_SCREEN_LEAVE).length - 1;
    assert.equal(enterCount, 2);
    assert.equal(leaveCount, 2);
    assert.ok(joined.includes(SHOW_CURSOR));
  });

  it('duplicate enter while already active is a no-op', () => {
    const stdout = fakeStdout(true);
    const screen = createAlternateScreen(stdout);
    screen.enter();
    screen.enter();
    assert.deepEqual(stdout.writes, [ALT_SCREEN_ENTER]);
  });

  it('registers beforeExit and exit listeners on enter; leave removes them', () => {
    const stdout = fakeStdout(true);
    const hooks = fakeProcessHooks();
    const screen = createAlternateScreen(stdout, {
      registerExitListener: hooks.register,
      unregisterExitListener: hooks.unregister,
    });
    assert.equal(hooks.beforeExit.size, 0);
    assert.equal(hooks.exit.size, 0);
    screen.enter();
    assert.equal(hooks.beforeExit.size, 1);
    assert.equal(hooks.exit.size, 1);
    screen.leave();
    assert.equal(hooks.beforeExit.size, 0);
    assert.equal(hooks.exit.size, 0);
  });

  it('exit hook leave restores primary buffer and is idempotent with explicit leave', () => {
    const stdout = fakeStdout(true);
    const hooks = fakeProcessHooks();
    const screen = createAlternateScreen(stdout, {
      registerExitListener: hooks.register,
      unregisterExitListener: hooks.unregister,
    });
    screen.enter();
    const [hook] = [...hooks.exit];
    assert.ok(hook);
    hook();
    assert.ok(joinedWrites(stdout).includes(ALT_SCREEN_LEAVE));
    assert.ok(joinedWrites(stdout).includes(SHOW_CURSOR));
    assert.equal(hooks.beforeExit.size, 0);
    assert.equal(hooks.exit.size, 0);
    const afterHook = joinedWrites(stdout);
    screen.leave();
    assert.equal(joinedWrites(stdout), afterHook);
  });

  it('does not register exit hooks when non-TTY', () => {
    const stdout = fakeStdout(false);
    const hooks = fakeProcessHooks();
    const screen = createAlternateScreen(stdout, {
      registerExitListener: hooks.register,
      unregisterExitListener: hooks.unregister,
    });
    screen.enter();
    assert.equal(hooks.beforeExit.size, 0);
    assert.equal(hooks.exit.size, 0);
  });
});

describe('withAlternateScreen', () => {
  it('enters before the body and leaves on normal completion', async () => {
    const stdout = fakeStdout(true);
    const order: string[] = [];
    await withAlternateScreen(async (screen) => {
      order.push('body');
      assert.deepEqual(stdout.writes, [ALT_SCREEN_ENTER]);
      assert.ok(screen);
    }, stdout);
    assert.deepEqual(order, ['body']);
    assert.ok(joinedWrites(stdout).includes(ALT_SCREEN_ENTER));
    assert.ok(joinedWrites(stdout).includes(ALT_SCREEN_LEAVE));
    assert.ok(joinedWrites(stdout).includes(SHOW_CURSOR));
  });

  it('leaves in finally when the body throws', async () => {
    const stdout = fakeStdout(true);
    await assert.rejects(
      () =>
        withAlternateScreen(async () => {
          throw new Error('render boom');
        }, stdout),
      /render boom/,
    );
    assert.ok(joinedWrites(stdout).includes(ALT_SCREEN_LEAVE));
    assert.ok(joinedWrites(stdout).includes(SHOW_CURSOR));
  });

  it('supports leave/enter around an external editor handoff', async () => {
    const stdout = fakeStdout(true);
    const phases: string[] = [];
    await withAlternateScreen(async (screen) => {
      phases.push('tui-1');
      screen.leave();
      phases.push('editor');
      screen.enter();
      phases.push('tui-2');
    }, stdout);
    assert.deepEqual(phases, ['tui-1', 'editor', 'tui-2']);
    const joined = joinedWrites(stdout);
    assert.equal(joined.split(ALT_SCREEN_ENTER).length - 1, 2);
    assert.equal(joined.split(ALT_SCREEN_LEAVE).length - 1, 2);
  });

  it('is a no-op wrapper when stdout is not a TTY', async () => {
    const stdout = fakeStdout(false);
    await withAlternateScreen(async () => undefined, stdout);
    assert.deepEqual(stdout.writes, []);
  });
});
