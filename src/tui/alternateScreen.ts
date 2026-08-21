/**
 * Terminal alternate-screen buffer (DECSET/DECRST 1049), same idea as Vim/less.
 *
 * Enter swaps to a private buffer so Ink's frames do not accumulate in the
 * primary scrollback; leave restores the prior primary contents. Encapsulated
 * here so `runTui` can inject a fake stdout in tests without emitting real CSI.
 *
 * Process `beforeExit` / `exit` hooks call leave once so abrupt process exit
 * still restores the primary buffer (and shows the cursor) like Vim/less.
 * SIGKILL (`kill -9`) does not run JavaScript hooks or `finally` blocks, so
 * those exits cannot restore the primary buffer.
 */

/**
 * CSI to enter the alternate screen buffer (`DECSET 1049`).
 */
export const ALT_SCREEN_ENTER = '\x1b[?1049h';

/**
 * CSI to leave the alternate screen buffer (`DECRST 1049`).
 */
export const ALT_SCREEN_LEAVE = '\x1b[?1049l';

/**
 * CSI to show the cursor (`DECTCEM` set). Written on leave so a hung/hidden
 * cursor from Ink does not linger on the primary buffer after quit.
 */
export const SHOW_CURSOR = '\x1b[?25h';

/**
 * Minimal stdout seam: TTY check + write (real `process.stdout` or a fake).
 */
export type AlternateScreenStdout = {
  isTTY?: boolean;
  write: (chunk: string) => unknown;
};

/**
 * Injectable process exit listener seam (defaults to `process.on` / `off`).
 */
export type AlternateScreenExitHooks = {
  registerExitListener: (
    event: 'beforeExit' | 'exit',
    listener: () => void,
  ) => void;
  unregisterExitListener: (
    event: 'beforeExit' | 'exit',
    listener: () => void,
  ) => void;
};

/**
 * Enter/leave handle that tracks whether the alternate buffer is active so
 * leave is idempotent and re-enter works across an `$EDITOR` remount loop.
 */
export type AlternateScreen = {
  /**
   * Enter the alternate buffer (no-op when non-TTY or already entered).
   */
  enter: () => void;
  /**
   * Leave the alternate buffer (no-op when non-TTY or not entered).
   */
  leave: () => void;
};

const DEFAULT_EXIT_HOOKS: AlternateScreenExitHooks = {
  registerExitListener(event, listener) {
    process.on(event, listener);
  },
  unregisterExitListener(event, listener) {
    process.off(event, listener);
  },
};

/**
 * Create an enter/leave pair bound to `stdout`.
 *
 * Optional `registerExitListener` / `unregisterExitListener` let tests inject
 * fake process hooks instead of patching the real `process`.
 */
export function createAlternateScreen(
  stdout: AlternateScreenStdout = process.stdout,
  hooks: Partial<AlternateScreenExitHooks> = {},
): AlternateScreen {
  const register =
    hooks.registerExitListener ?? DEFAULT_EXIT_HOOKS.registerExitListener;
  const unregister =
    hooks.unregisterExitListener ?? DEFAULT_EXIT_HOOKS.unregisterExitListener;

  let entered = false;
  let hooksRegistered = false;

  const onProcessExit = (): void => {
    leave();
  };

  function registerHooks(): void {
    if (hooksRegistered) return;
    register('beforeExit', onProcessExit);
    register('exit', onProcessExit);
    hooksRegistered = true;
  }

  function unregisterHooks(): void {
    if (!hooksRegistered) return;
    unregister('beforeExit', onProcessExit);
    unregister('exit', onProcessExit);
    hooksRegistered = false;
  }

  function leave(): void {
    if (!stdout.isTTY || !entered) return;
    stdout.write(ALT_SCREEN_LEAVE + SHOW_CURSOR);
    entered = false;
    unregisterHooks();
  }

  return {
    enter() {
      if (!stdout.isTTY || entered) return;
      stdout.write(ALT_SCREEN_ENTER);
      entered = true;
      registerHooks();
    },
    leave,
  };
}

/**
 * Enter the alternate screen for the duration of `fn`, always leaving in
 * `finally` so render/wait failures still restore the primary buffer.
 *
 * The body receives the screen handle so it can `leave` before an external
 * `$EDITOR` (primary buffer) and `enter` again before remounting the TUI.
 */
export async function withAlternateScreen<T>(
  fn: (screen: AlternateScreen) => Promise<T>,
  stdout: AlternateScreenStdout = process.stdout,
  hooks: Partial<AlternateScreenExitHooks> = {},
): Promise<T> {
  const screen = createAlternateScreen(stdout, hooks);
  screen.enter();
  try {
    return await fn(screen);
  } finally {
    screen.leave();
  }
}
