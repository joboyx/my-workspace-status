import assert from 'node:assert';
import { describe, it } from 'node:test';
import { EventEmitter } from 'node:events';
import { prepareTerminalForEditor, restoreTerminalAfterEditor } from '../src/tui/terminalResume.js';
import { MOUSE_DISABLE } from '../src/tui/mouse.js';

function fakeStdin(opts: { isTTY: boolean; isPaused?: boolean; buffered?: unknown[] }) {
  const ee = new EventEmitter() as EventEmitter & {
    isTTY: boolean;
    isPaused: () => boolean;
    resume: () => void;
    pause: () => void;
    read: () => unknown;
    setRawMode?: (mode: boolean) => void;
    _raw?: boolean;
    _paused?: boolean;
    _buf: unknown[];
  };
  ee.isTTY = opts.isTTY;
  ee._paused = opts.isPaused ?? false;
  ee._buf = [...(opts.buffered ?? [])];
  ee.isPaused = () => !!ee._paused;
  ee.resume = () => {
    ee._paused = false;
  };
  ee.pause = () => {
    ee._paused = true;
  };
  ee.read = () => (ee._buf.length > 0 ? ee._buf.shift() : null);
  ee.setRawMode = (mode: boolean) => {
    ee._raw = mode;
  };
  return ee;
}

function fakeStdout(opts: { isTTY: boolean }) {
  const out = {
    isTTY: opts.isTTY,
    writes: [] as string[],
    write(chunk: string) {
      out.writes.push(chunk);
      return true;
    },
  };
  return out;
}

describe('restoreTerminalAfterEditor', () => {
  it('re-enables raw mode and resumes a TTY stdin', () => {
    const stdin = fakeStdin({ isTTY: true, isPaused: true });
    stdin._raw = false;
    restoreTerminalAfterEditor(stdin as unknown as NodeJS.ReadStream);
    assert.equal(stdin._raw, true);
    assert.equal(stdin.isPaused(), false);
  });

  it('is a no-op for non-TTY stdin', () => {
    const stdin = fakeStdin({ isTTY: false, isPaused: true });
    delete stdin.setRawMode;
    restoreTerminalAfterEditor(stdin as unknown as NodeJS.ReadStream);
    assert.equal(stdin.isPaused(), true);
  });

  /**
   * Contract (B4 / Joboy): after `$EDITOR` exits, the next `handleKey` must be
   * reachable without an extra Enter. Full Ink remount integration is manual;
   * unit coverage is this restore helper (wired from `run.ts` post-editor).
   */
  it('documents post-edit resume contract via helper coverage', () => {
    const stdin = fakeStdin({ isTTY: true, isPaused: true });
    stdin._raw = false;
    restoreTerminalAfterEditor(stdin as unknown as NodeJS.ReadStream);
    assert.equal(stdin._raw, true, 'setRawMode(true) so Ink can read keys');
    assert.equal(stdin.isPaused(), false, 'stdin.resume() so the stream flows');
  });
});

describe('prepareTerminalForEditor', () => {
  it('leaves raw mode, pauses stdin, drains leftover bytes, and disables mouse', () => {
    const stdin = fakeStdin({ isTTY: true, isPaused: false, buffered: ['x', 'y'] });
    stdin._raw = true;
    const stdout = fakeStdout({ isTTY: true });
    prepareTerminalForEditor(
      stdin as unknown as NodeJS.ReadStream,
      stdout as unknown as NodeJS.WriteStream,
    );
    assert.equal(stdin._raw, false);
    assert.equal(stdin.isPaused(), true);
    assert.deepEqual(stdin._buf, []);
    assert.deepEqual(stdout.writes, [MOUSE_DISABLE]);
  });

  it('is a no-op for non-TTY stdin besides mouse disable on a TTY stdout', () => {
    const stdin = fakeStdin({ isTTY: false, isPaused: false, buffered: ['x'] });
    delete stdin.setRawMode;
    const stdout = fakeStdout({ isTTY: true });
    prepareTerminalForEditor(
      stdin as unknown as NodeJS.ReadStream,
      stdout as unknown as NodeJS.WriteStream,
    );
    assert.equal(stdin.isPaused(), false);
    assert.deepEqual(stdin._buf, ['x']);
    assert.deepEqual(stdout.writes, [MOUSE_DISABLE]);
  });
});
