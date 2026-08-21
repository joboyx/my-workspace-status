/**
 * Decide whether `e` unmounts Ink, and spawn GUI editors while mounted.
 *
 * Blocking TTY editors (vim) still go through `run.ts` after unmount.
 * Detached GUI editors (Cursor, VS Code) spawn here so fold/focus/scroll
 * stay in the live React tree instead of a remount.
 */

import { spawn } from 'node:child_process';
import path from 'node:path';
import { editorCommand, isDetachedEditor } from './editor.js';
import type { EditRequest } from './session.js';

/**
 * Minimal spawn seam for tests. Matches the detached `e` options only.
 */
export type SpawnLike = (
  command: string,
  args: string[],
  options: { cwd: string; stdio: 'ignore'; detached: boolean },
) => {
  on: (event: string, listener: (err: Error) => void) => void;
  unref: () => void;
};

/**
 * Open a GUI editor without inheriting the TUI stdin/stdout.
 *
 * `stdio: 'ignore'` keeps Ink on the TTY. `detached` + `unref` lets the CLI
 * exit (or `--wait`) without tearing down the mount.
 */
export function spawnDetachedEditor(
  command: string,
  args: string[],
  cwd: string,
  hooks: {
    spawn?: SpawnLike;
    onError?: (message: string) => void;
  } = {},
): void {
  const spawnImpl = hooks.spawn ?? (spawn as unknown as SpawnLike);
  const child = spawnImpl(command, args, { cwd, stdio: 'ignore', detached: true });
  child.on('error', (err) => {
    const message = err instanceof Error ? err.message : String(err);
    hooks.onError?.(message);
    console.error(`Failed to launch editor (${command}): ${message}`);
  });
  child.unref();
}

/**
 * Start an edit. Detached editors spawn and return (TUI stays mounted).
 * Blocking TTY editors record `pendingEdit` and return `'quit'` so `run.ts`
 * can unmount, hand over the TTY, then remount.
 */
export function startEdit(opts: {
  editor: string;
  request: EditRequest;
  cwd: string;
  onEditRequest: (request: EditRequest) => void;
  onDetachedError?: (message: string) => void;
  spawn?: SpawnLike;
}): 'quit' | void {
  if (!isDetachedEditor(opts.editor)) {
    opts.onEditRequest(opts.request);
    return 'quit';
  }
  const { command, args } = editorCommand(opts.editor, opts.request.filePath, opts.request.line);
  spawnDetachedEditor(command, args, path.join(opts.cwd, opts.request.repoPath), {
    spawn: opts.spawn,
    onError: opts.onDetachedError,
  });
}
