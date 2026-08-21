/**
 * Keypress → Action state machine (double-tap fold, confirm, filter gates).
 * Action keys are gated by the highlighted row kind via the action registry.
 */

import type { ActionId, RowKind } from './actions/registry.js';
import { CTRL_O_KEY, actionFor } from './actions/registry.js';

export const DOUBLE_TAP_MS = 400;

export type Action =
  | { type: 'move'; delta: 1 | -1 }
  | { type: 'moveTo'; edge: 'start' | 'end' }
  | {
      type: 'fold';
      op: 'toggle' | 'open' | 'close' | 'openAll' | 'closeAll' | 'toggleSubtree';
    }
  | { type: 'expand' }
  | { type: 'collapse' }
  | { type: 'toggleTreeMode' }
  | { type: 'toggleCommitTreeMode' }
  | { type: 'toggleShowIgnored' }
  | { type: 'stage' }
  | { type: 'unstage' }
  | { type: 'revert' }
  | { type: 'toggleDiffMode' }
  | { type: 'refresh' }
  | { type: 'edit' }
  | { type: 'toggleViewed' }
  | { type: 'fullFile' }
  | { type: 'branch' }
  | { type: 'removeWorktree' }
  | { type: 'graphCheckout' }
  | { type: 'graphCreateBranch' }
  | { type: 'stashApply' }
  | { type: 'stashDrop' }
  | { type: 'stashMenu' }
  | { type: 'stashPop' }
  | { type: 'fetch' }
  | { type: 'pull' }
  | { type: 'push' }
  | { type: 'defaultBranch' }
  | { type: 'searchStart' }
  | { type: 'searchNext' }
  | { type: 'searchPrev' }
  | { type: 'easyMotionStart' }
  | { type: 'help' }
  | { type: 'quit' }
  | { type: 'confirmYes' }
  | { type: 'confirmYesClean' }
  | { type: 'confirmNo' }
  | { type: 'scrollDiff'; delta: number }
  | { type: 'panDiff'; delta: number }
  | { type: 'pageMove'; deltaPages: 1 | -1 }
  | { type: 'toggleMouse' }
  | { type: 'cycleTheme' }
  | { type: 'navEnter' }
  | { type: 'navEsc' }
  | { type: 'none' };

/**
 * Actions that drive a list or row-scoped registry write.
 * When `focusPane === 'right'`, `runAction` no-ops these unless
 * `rightPaneLeftListAllowed` matches (graph move/write, commit-files nav,
 * diff `move`/`moveTo`, or `edit`/`fullFile` on a focused diff).
 * Nav chrome, quit/help/refresh, theme/mouse, view-mode toggles (`i`/`t`/`.`),
 * diff scroll, and overlay openers are intentionally excluded.
 */
const LEFT_LIST_ACTION_TYPES = new Set<Action['type']>([
  'move',
  'moveTo',
  'fold',
  'expand',
  'collapse',
  'stage',
  'unstage',
  'revert',
  'edit',
  'toggleViewed',
  'fullFile',
  'branch',
  'removeWorktree',
  'graphCheckout',
  'graphCreateBranch',
  'stashApply',
  'stashDrop',
  'stashMenu',
  'stashPop',
  'fetch',
  'pull',
  'push',
  'defaultBranch',
]);

/** True when `action` targets the left tree list (see LEFT_LIST_ACTION_TYPES). */
export function isLeftListAction(action: Action): boolean {
  return LEFT_LIST_ACTION_TYPES.has(action.type);
}

export type KeyState = {
  zPending: boolean;
  gPending: boolean;
  pendingAt: number | null;
  confirmMode: boolean;
  /** Typing a `/` search query (App consumes chars). */
  searchMode: boolean;
  /**
   * True when a search query is armed (session.search) so `n`/`N` step matches.
   * App sets this; keys only read it.
   */
  searchActive: boolean;
  /** EasyMotion overlay armed — App consumes label keys. */
  easyMotionMode: boolean;
  branchMode: boolean;
  /** Create-branch name overlay (graph `c` when the graph list is focused). */
  createBranchMode: boolean;
  /** Checkoutable-branch picker at a graph commit. */
  graphBranchMode: boolean;
  /** Stash drop y/n confirm. */
  stashDropMode: boolean;
  /** Origin out-of-sync checkout y/n confirm. */
  graphCheckoutConfirmMode: boolean;
  /** Stash overlay listing valid ops for the focused row. */
  stashMenuMode: boolean;
  /** ViewStack depth — gates `t` between workspace and commit tree modes. */
  navDepth: 0 | 1 | 2;
};

export type KeyFlags = {
  upArrow?: boolean;
  downArrow?: boolean;
  leftArrow?: boolean;
  rightArrow?: boolean;
  return?: boolean;
  escape?: boolean;
  pageUp?: boolean;
  pageDown?: boolean;
  ctrl?: boolean;
};

/**
 * Defensive PageUp/PageDown CSI detection when Ink's `key.pageUp` /
 * `key.pageDown` flags are missing (some terminals / Ink versions).
 *
 * Recognizes standard `\x1b[5~` / `\x1b[6~` and legacy `\x1b[[5~` / `\x1b[[6~`,
 * plus the same bodies after Ink `useInput` strips a leading ESC (`[5~`, …).
 * This cannot invent a key the terminal never delivers (common on macOS
 * when Fn+Up scrolls scrollback instead of sending CSI).
 */
export function pageKeyFlagsFromInput(input: string): Pick<KeyFlags, 'pageUp' | 'pageDown'> {
  if (input === '\x1b[5~' || input === '\x1b[[5~' || input === '[5~' || input === '[[5~') {
    return { pageUp: true };
  }
  if (input === '\x1b[6~' || input === '\x1b[[6~' || input === '[6~' || input === '[[6~') {
    return { pageDown: true };
  }
  return {};
}

/**
 * True when this keypress should start EasyMotion on the focused list.
 *
 * The documented binding is Ctrl+Space. Ink does not pass that as `' '` with
 * `ctrl`. Classic Ctrl-Space is NUL (`\x00`). Ink names that key `` ` `` and
 * sets `ctrl`. A space key with `ctrl` is passed as `input === 'space'`.
 *
 * macOS often does not deliver Ctrl+Space. The OS may switch input source, or
 * the terminal may bind the chord. `;` is the fallback. It arrives as a
 * normal character.
 */
export function isEasyMotionStart(input: string, key: Pick<KeyFlags, 'ctrl'>): boolean {
  if (input === ';') return true;
  if (input === '\0') return true;
  if (!key.ctrl) return false;
  return input === ' ' || input === 'space' || input === '`';
}

/** Optional pane context for h/l pan vs fold and j/k scroll vs move (Track D). */
export type HandleKeyCtx = {
  focusPane: 'left' | 'right';
  rightIsDiff: boolean;
};

const DEFAULT_HANDLE_KEY_CTX: HandleKeyCtx = {
  focusPane: 'left',
  rightIsDiff: false,
};

const NONE: Action = { type: 'none' };

/** Clear all pending double-tap / chord flags. */
function clearPending(state: KeyState): KeyState {
  return {
    ...state,
    zPending: false,
    gPending: false,
    pendingAt: null,
  };
}

/** Fresh keymap state — all modes off. */
export function createKeyState(): KeyState {
  return {
    zPending: false,
    gPending: false,
    pendingAt: null,
    confirmMode: false,
    searchMode: false,
    searchActive: false,
    easyMotionMode: false,
    branchMode: false,
    createBranchMode: false,
    graphBranchMode: false,
    stashDropMode: false,
    graphCheckoutConfirmMode: false,
    stashMenuMode: false,
    navDepth: 0,
  };
}

/**
 * Resolve an expired pending double-tap / chord.
 * Call from the App timer when `DOUBLE_TAP_MS` elapses after arming.
 * Instant fold already fired on first z — expired pending emits `none`.
 */
export function flushPending(state: KeyState, now: number): { state: KeyState; action: Action } {
  if (state.pendingAt === null) return { state, action: NONE };
  if (now - state.pendingAt < DOUBLE_TAP_MS) return { state, action: NONE };

  if (state.zPending) {
    return {
      state: {
        ...state,
        zPending: false,
        gPending: false,
        pendingAt: null,
      },
      action: NONE,
    };
  }
  if (state.gPending) {
    return {
      state: { ...state, gPending: false, pendingAt: null },
      action: NONE,
    };
  }
  return { state, action: NONE };
}

export type HandleKeyResult = {
  state: KeyState;
  action: Action;
  /**
   * Optional action to run *before* `action` — used when an expired or
   * cancelled pending chord still needs its flush before the redispatched key.
   */
  prelude?: Action;
};

function result(state: KeyState, action: Action, prelude?: Action): HandleKeyResult {
  return prelude !== undefined ? { state, action, prelude } : { state, action };
}

/**
 * Flush an expired pending chord, then redispatch `input` as a fresh key.
 * Used when a key arrives after `DOUBLE_TAP_MS` but before the App timer.
 */
function flushThenRedispatch(
  state: KeyState,
  input: string,
  key: KeyFlags,
  kind: RowKind,
  now: number,
  ctx: HandleKeyCtx = DEFAULT_HANDLE_KEY_CTX,
): HandleKeyResult {
  const flushed = flushPending(state, now);
  const next = handleKey(flushed.state, input, key, kind, now, ctx);
  if (flushed.action.type === 'none') {
    return next;
  }
  // Nested redispatches should not stack — pending was cleared by flush.
  return result(next.state, next.action, flushed.action);
}

/**
 * Map one keypress to an Action, updating pending flags when needed.
 * App owns confirmMode / searchMode / branchMode flags; this only reads them.
 * Fold actions are emitted here — App wires applyFold (+ collectFoldableIds for closeAll).
 * `kind` is the highlighted row's kind; action keys invalid there resolve to `none`.
 * `now` defaults to `Date.now()` — pass explicitly in tests for deterministic double-taps.
 * `ctx` gates h/l between fold and horizontal pan, and j/k between list move and
 * vertical diff scroll, when the right pane shows a file diff.
 */
export function handleKey(
  state: KeyState,
  input: string,
  key: KeyFlags,
  kind: RowKind,
  now: number = Date.now(),
  ctx: HandleKeyCtx = DEFAULT_HANDLE_KEY_CTX,
): HandleKeyResult {
  if (state.confirmMode) {
    return handleConfirm(state, input, key);
  }

  if (state.searchMode) {
    // App consumes search chars; Esc is handled by App clearing searchMode.
    return result(state, NONE);
  }

  if (state.easyMotionMode) {
    // App owns EasyMotion label typing; Esc cancels — never quits.
    return result(state, NONE);
  }

  if (state.branchMode) {
    // App owns picker filter / cursor / Esc / Enter; never quit from here.
    return result(state, NONE);
  }

  if (
    state.createBranchMode ||
    state.graphBranchMode ||
    state.stashDropMode ||
    state.graphCheckoutConfirmMode ||
    state.stashMenuMode
  ) {
    // App owns overlay input; Esc cancels overlay — never quits.
    return result(state, NONE);
  }

  if (state.zPending) {
    return handleZChord(state, input, key, kind, now, ctx);
  }

  if (state.gPending) {
    return handleGChord(state, input, key, kind, now, ctx);
  }

  /**
   * Everything from here to the registry lookup below consumes its key before
   * `actionFor` ever sees it, so a registry action bound to one of these keys
   * would be silently dead. `test/tui-keys.test.ts` ("registry / keymap key
   * disjointness") hardcodes that reserved list — add any new key branch here
   * to that list too, or the test goes stale without failing.
   */

  // Page before Esc: CSI PageUp starts with ESC. If both flags are set,
  // paging is the intended key. Esc never quits — exit is double Ctrl+C
  // in App (`exitOnCtrlC: false`). Overlays intercept Esc/Enter first.
  if (key.pageUp) {
    return result(state, { type: 'pageMove', deltaPages: -1 });
  }
  if (key.pageDown) {
    return result(state, { type: 'pageMove', deltaPages: 1 });
  }

  if (key.escape) {
    return result(state, { type: 'navEsc' });
  }

  if (key.return) {
    return result(state, { type: 'navEnter' });
  }

  if (isEasyMotionStart(input, key)) {
    return result(state, { type: 'easyMotionStart' });
  }

  if (key.ctrl) {
    if (input === 'o') {
      return result(state, actionFor(CTRL_O_KEY, kind) ? { type: 'fullFile' } : NONE);
    }
    // Ctrl+u / Ctrl+d: smaller page step; App routes by focusPane (same as pageMove).
    if (input === 'u') return result(state, { type: 'scrollDiff', delta: -5 });
    if (input === 'd') return result(state, { type: 'scrollDiff', delta: 5 });
    return result(state, NONE);
  }

  if (key.downArrow || input === 'j') {
    if (ctx.focusPane === 'right' && ctx.rightIsDiff) {
      return result(state, { type: 'scrollDiff', delta: 1 });
    }
    return result(state, { type: 'move', delta: 1 });
  }
  if (key.upArrow || input === 'k') {
    if (ctx.focusPane === 'right' && ctx.rightIsDiff) {
      return result(state, { type: 'scrollDiff', delta: -1 });
    }
    return result(state, { type: 'move', delta: -1 });
  }
  if (key.leftArrow || input === 'h') {
    if (ctx.focusPane === 'right' && ctx.rightIsDiff) {
      return result(state, { type: 'panDiff', delta: -1 });
    }
    return result(state, { type: 'collapse' });
  }
  if (key.rightArrow || input === 'l') {
    if (ctx.focusPane === 'right' && ctx.rightIsDiff) {
      return result(state, { type: 'panDiff', delta: 1 });
    }
    return result(state, { type: 'expand' });
  }

  if (input === 'z') {
    return result(
      {
        ...clearPending(state),
        zPending: true,
        pendingAt: now,
      },
      { type: 'fold', op: 'toggle' },
    );
  }

  if (input === ' ') {
    // Dirty-file gate lives in runAction / canToggleViewed. Non-file rows no-op.
    return result(clearPending(state), kind === 'file' ? { type: 'toggleViewed' } : NONE);
  }

  if (input === 'g') {
    return result(
      {
        ...clearPending(state),
        gPending: true,
        pendingAt: now,
      },
      NONE,
    );
  }

  if (input === 'G') {
    return result(clearPending(state), { type: 'moveTo', edge: 'end' });
  }

  // Armed search: n/N step matches (vim). Idle `p` stays pull; graph stash `p` stays stashPop.
  if (state.searchActive) {
    if (input === 'n') return result(state, { type: 'searchNext' });
    if (input === 'N') return result(state, { type: 'searchPrev' });
  }

  if (input === 'm') {
    return result(state, { type: 'toggleMouse' });
  }

  if (input === 'T') {
    return result(state, { type: 'cycleTheme' });
  }

  const spec = actionFor(input, kind);
  if (spec) {
    // Typed by ActionId (no fallback) so a new registry action fails to compile
    // until it is paired with an Action here.
    const byId: Record<ActionId, Action> = {
      stage: { type: 'stage' },
      unstage: { type: 'unstage' },
      revert: { type: 'revert' },
      fetch: { type: 'fetch' },
      pull: { type: 'pull' },
      push: { type: 'push' },
      defaultBranch: { type: 'defaultBranch' },
      branch: { type: 'branch' },
      removeWorktree: { type: 'removeWorktree' },
      edit: { type: 'edit' },
      toggleViewed: { type: 'toggleViewed' },
      fullFile: { type: 'fullFile' },
      graphCheckout: { type: 'graphCheckout' },
      graphCreateBranch: { type: 'graphCreateBranch' },
      stashApply: { type: 'stashApply' },
      stashDrop: { type: 'stashDrop' },
      stashMenu: { type: 'stashMenu' },
      stashPop: { type: 'stashPop' },
    };
    return result(state, byId[spec.id]);
  }

  switch (input) {
    case 'i':
      return result(state, { type: 'toggleDiffMode' });
    case '.':
      return result(state, { type: 'toggleShowIgnored' });
    case 't':
      return result(
        state,
        state.navDepth >= 1 ? { type: 'toggleCommitTreeMode' } : { type: 'toggleTreeMode' },
      );
    case 'r':
      return result(state, { type: 'refresh' });
    case '/':
      return result(state, { type: 'searchStart' });
    case '?':
      return result(state, { type: 'help' });
    case 'q':
      // Intentionally unbound — exit is double Ctrl+C (see ctrlCExit.ts).
      return result(state, NONE);
    default:
      return result(state, NONE);
  }
}

function handleConfirm(state: KeyState, input: string, key: KeyFlags): HandleKeyResult {
  if (input === 'Y') {
    return result(state, { type: 'confirmYesClean' });
  }
  if (input === 'y' || key.return) {
    return result(state, { type: 'confirmYes' });
  }
  if (input === 'n' || key.escape) {
    return result(state, { type: 'confirmNo' });
  }
  return result(state, NONE);
}

function withinWindow(state: KeyState, now: number): boolean {
  return state.pendingAt !== null && now - state.pendingAt < DOUBLE_TAP_MS;
}

function handleZChord(
  state: KeyState,
  input: string,
  key: KeyFlags,
  kind: RowKind,
  now: number,
  ctx: HandleKeyCtx = DEFAULT_HANDLE_KEY_CTX,
): HandleKeyResult {
  const cleared = clearPending(state);

  if (key.escape) {
    return result(cleared, NONE);
  }

  if (!withinWindow(state, now)) {
    // Expired — toggle already fired on first z; redispatch the new key.
    return flushThenRedispatch(state, input, key, kind, now, ctx);
  }

  if (input === 'z') {
    return result(cleared, { type: 'fold', op: 'toggleSubtree' });
  }
  // No z* chords — cancel pending, redispatch the new key.
  return handleKey(cleared, input, key, kind, now, ctx);
}

function handleGChord(
  state: KeyState,
  input: string,
  key: KeyFlags,
  kind: RowKind,
  now: number,
  ctx: HandleKeyCtx = DEFAULT_HANDLE_KEY_CTX,
): HandleKeyResult {
  const cleared = clearPending(state);

  if (key.escape) {
    return result(cleared, NONE);
  }

  if (!withinWindow(state, now)) {
    // Expired gPending flushes to none; redispatch the new key.
    return flushThenRedispatch(state, input, key, kind, now, ctx);
  }

  if (input === 'g') {
    return result(cleared, { type: 'moveTo', edge: 'start' });
  }
  // Unknown chord — cancel pending, redispatch the new key.
  return handleKey(cleared, input, key, kind, now, ctx);
}

