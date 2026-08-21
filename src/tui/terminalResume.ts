/**
 * Hand the TTY to a blocking `$EDITOR` child, then restore it for Ink.
 *
 * Ink raw mode plus a still-flowing parent stdin will steal bytes from vim
 * when the child is spawned with `stdio: 'inherit'`. Prepare cooked + paused
 * stdin (and drop mouse reporting) before spawn; restore raw + flowing after
 * the child exits so the remounted TUI sees the next keypress.
 */

import { MOUSE_DISABLE } from './mouse.js';

/**
 * Release the TTY so a blocking editor can own stdin.
 *
 * Turns off SGR mouse reporting, leaves raw mode, pauses Node's stdin
 * reader, and discards bytes already sitting in Node's buffer. Those bytes
 * cannot be forwarded to the child (Node already consumed them).
 */
export function prepareTerminalForEditor(
  stdin: NodeJS.ReadStream = process.stdin,
  stdout: NodeJS.WriteStream = process.stdout,
): void {
  if (stdout.isTTY && typeof stdout.write === 'function') {
    stdout.write(MOUSE_DISABLE);
  }
  if (!stdin.isTTY) return;
  const raw = stdin as NodeJS.ReadStream & {
    setRawMode?: (mode: boolean) => void;
    pause?: () => void;
    read?: () => unknown;
  };
  if (typeof raw.setRawMode === 'function') {
    raw.setRawMode(false);
  }
  if (typeof raw.pause === 'function') {
    raw.pause();
  }
  if (typeof raw.read === 'function') {
    for (;;) {
      const chunk = raw.read();
      if (chunk == null) break;
      if (typeof chunk === 'string' && chunk.length === 0) break;
      if (Buffer.isBuffer(chunk) && chunk.length === 0) break;
    }
  }
}

/**
 * Restore stdin so Ink can consume the next keypress after an external
 * `$EDITOR` child exits with `stdio: 'inherit'`.
 *
 * Without this, many terminals leave the stream cooked / paused and the first
 * keystroke after remount is swallowed (often requiring an extra Enter).
 */
export function restoreTerminalAfterEditor(
  stdin: NodeJS.ReadStream = process.stdin,
  _stdout: NodeJS.WriteStream = process.stdout,
): void {
  if (!stdin.isTTY) return;
  const raw = stdin as NodeJS.ReadStream & {
    setRawMode?: (mode: boolean) => void;
    isPaused?: () => boolean;
  };
  if (typeof raw.setRawMode === 'function') {
    raw.setRawMode(true);
  }
  if (typeof raw.isPaused === 'function' && raw.isPaused()) {
    stdin.resume();
  } else if (typeof stdin.resume === 'function') {
    // Ensure the stream is flowing even when isPaused is unavailable.
    stdin.resume();
  }
}
