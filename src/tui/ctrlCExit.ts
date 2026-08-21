/**
 * Double Ctrl+C exit (same UX as Claude / Cursor / similar LLM harnesses).
 *
 * First press arms a short window and prompts; second press within the window
 * quits. Expired arms are treated as a fresh first press.
 */

/** Window to confirm exit after the first Ctrl+C (ms). */
export const CTRL_C_EXIT_MS = 2000;

/** Status-bar prompt shown after the first Ctrl+C. */
export const CTRL_C_EXIT_PROMPT = 'Press Ctrl+C again to exit';

export type CtrlCExitState = {
  /** Epoch ms until which a second Ctrl+C quits; 0 = not armed. */
  armedUntil: number;
};

export type CtrlCExitResult = {
  state: CtrlCExitState;
  /** True when this press should quit the TUI. */
  quit: boolean;
  /** True when the status bar should show {@link CTRL_C_EXIT_PROMPT}. */
  prompt: boolean;
};

/**
 * Pure double-Ctrl+C state machine. Pass explicit `now` in tests.
 */
export function handleCtrlC(
  state: CtrlCExitState,
  now: number = Date.now(),
): CtrlCExitResult {
  if (state.armedUntil > 0 && now < state.armedUntil) {
    return { state: { armedUntil: 0 }, quit: true, prompt: false };
  }
  return {
    state: { armedUntil: now + CTRL_C_EXIT_MS },
    quit: false,
    prompt: true,
  };
}

/**
 * True when a Ctrl+C / SIGINT-style keypress should enter the exit path.
 * Ink reports Ctrl+C as `input === 'c'` with `key.ctrl` when `exitOnCtrlC` is off.
 */
export function isCtrlC(input: string, ctrl: boolean | undefined): boolean {
  return (ctrl === true && input === 'c') || input === '\x03';
}
