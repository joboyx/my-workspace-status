/**
 * Unattended live-TUI harness.
 *
 * Renders the real Ink/React App via ink-testing-library (same mount as
 * production, no helper-only shortcuts). Agents drive keys through stdin
 * and assert the last frame / session.
 */
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { render } from 'ink-testing-library';
import React from 'react';
import { App } from '../../src/tui/App.js';
import { createSessionState } from '../../src/tui/session.js';
import type { EditRequest, ExitReason, SessionState } from '../../src/tui/session.js';
import type { RepoSnapshot } from '../../src/types.js';

process.env.WS_STATUS_WATCH_MS ??= '0';
process.env.WS_STATUS_FETCH_MS ??= '0';

export const KEY = {
  enter: '\r',
  esc: '\u001b',
  ctrlC: '\u0003',
  ctrlO: '\u000f',
  backspace: '\u007f',
  up: '\u001b[A',
  down: '\u001b[B',
  right: '\u001b[C',
  left: '\u001b[D',
  pageUp: '\u001b[5~',
  pageDown: '\u001b[6~',
} as const;

export type MountedTui = {
  stdin: { write: (data: string) => void };
  lastFrame: () => string;
  session: () => SessionState;
  edits: EditRequest[];
  exits: ExitReason[];
  unmount: () => void;
};

export type MountOptions = {
  cwd: string;
  snapshots: RepoSnapshot[];
  ignoredRepos?: string[];
  showIgnored?: boolean;
  filterRepos?: string[];
  editor?: string;
  rows?: number;
  columns?: number;
  /** Shared viewed-files JSON. When omitted, isolate to a unique temp file. */
  viewedStore?: string;
};

/** Strip CSI / OSC so assertions can match user-visible copy. */
export function visible(frame: string): string {
  return frame
    .replace(/\u001b\][^\u0007]*\u0007/g, '')
    .replace(/\u001b\[[0-9;?]*[ -/]*[@-~]/g, '')
    .replace(/\r/g, '');
}

/** Sleep ms milliseconds. */
export function delay(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

/** Mount the live workspace-status App. */
export function mountTui(opts: MountOptions): MountedTui {
  // Graph git uses snapshot.repo relative to process.cwd() (prod launches from workspace).
  const prevCwd = process.cwd();
  const prevStore = process.env.WS_STATUS_VIEWED_STORE;
  const ephemeral = opts.viewedStore === undefined && !prevStore;
  const store =
    opts.viewedStore ??
    prevStore ??
    path.join(
      os.tmpdir(),
      `ws-viewed-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}.json`,
    );
  process.env.WS_STATUS_VIEWED_STORE = store;
  process.chdir(opts.cwd);
  let session = createSessionState(process.env, {
    showIgnored: opts.showIgnored ?? false,
  });
  const edits: EditRequest[] = [];
  const exits: ExitReason[] = [];
  let instance;
  try {
    instance = render(
      React.createElement(App, {
        cwd: opts.cwd,
        snapshots: opts.snapshots,
        ignoredRepos: opts.ignoredRepos ?? [],
        maxDepth: 5,
        defaultBranches: {},
        filterRepos: opts.filterRepos ?? [],
        editor: opts.editor,
        session,
        onSessionChange: (next) => {
          session = next;
        },
        onExit: (reason) => {
          exits.push(reason);
        },
        onEditRequest: (request) => {
          edits.push(request);
        },
      }),
    );
  } catch (err) {
    process.chdir(prevCwd);
    if (prevStore === undefined) delete process.env.WS_STATUS_VIEWED_STORE;
    else process.env.WS_STATUS_VIEWED_STORE = prevStore;
    throw err;
  }
  const rows = opts.rows ?? 40;
  const columns = opts.columns ?? 140;
  Object.defineProperty(instance.stdout, 'rows', { get: () => rows, configurable: true });
  Object.defineProperty(instance.stdout, 'columns', { get: () => columns, configurable: true });
  instance.stdout.emit('resize');

  return {
    stdin: instance.stdin,
    lastFrame: () => visible(instance.lastFrame() ?? ''),
    session: () => session,
    edits,
    exits,
    unmount: () => {
      instance.unmount();
      instance.cleanup();
      process.chdir(prevCwd);
      if (prevStore === undefined) delete process.env.WS_STATUS_VIEWED_STORE;
      else process.env.WS_STATUS_VIEWED_STORE = prevStore;
      if (ephemeral) {
        try {
          fs.unlinkSync(store);
        } catch {
          /* ignore */
        }
      }
    },
  };
}

/** Poll until pred is true or timeoutMs elapses. */
export async function waitFor(
  tui: MountedTui,
  pred: (frame: string, session: SessionState) => boolean,
  timeoutMs = 5000,
): Promise<void> {
  const start = Date.now();
  let last = '';
  while (Date.now() - start < timeoutMs) {
    last = tui.lastFrame();
    if (pred(last, tui.session())) return;
    await delay(20);
  }
  throw new Error('waitFor timeout after ' + timeoutMs + 'ms.\nLast frame:\n' + last);
}

/** Type text one character at a time so Ink useInput sees each press. */
export async function type(tui: MountedTui, text: string, gapMs = 12): Promise<void> {
  for (const ch of text) {
    tui.stdin.write(ch);
    await delay(gapMs);
  }
}

/** Send one key (or CSI sequence) and yield for a render. */
export async function press(tui: MountedTui, key: string, settleMs = 25): Promise<void> {
  tui.stdin.write(key);
  await delay(settleMs);
}

/** SGR left-press + release at 1-based col/row. */
export function mouseClickSeq(col: number, row: number): string {
  const esc = '\u001b';
  return esc + '[<0;' + col + ';' + row + 'M' + esc + '[<0;' + col + ';' + row + 'm';
}

/** SGR wheel-down at 1-based col/row. */
export function mouseWheelDownSeq(col: number, row: number): string {
  return '\u001b[<65;' + col + ';' + row + 'M';
}

/** SGR left-press, drag motion, and release (mode 1002 button 32). */
export function mouseDragSeq(
  fromCol: number,
  fromRow: number,
  toCol: number,
  toRow: number,
): string {
  const esc = '\u001b';
  return (
    esc +
    '[<0;' +
    fromCol +
    ';' +
    fromRow +
    'M' +
    esc +
    '[<32;' +
    toCol +
    ';' +
    toRow +
    'M' +
    esc +
    '[<0;' +
    toCol +
    ';' +
    toRow +
    'm'
  );
}

/** Mount, run fn, always unmount. */
export async function withTui(
  opts: MountOptions,
  fn: (tui: MountedTui) => Promise<void>,
): Promise<void> {
  const tui = mountTui(opts);
  try {
    await waitFor(tui, (frame) => frame.trim().length > 0, 3000);
    await delay(80);
    tui.stdin.write('q');
    await delay(40);
    await fn(tui);
  } finally {
    tui.unmount();
  }
}
