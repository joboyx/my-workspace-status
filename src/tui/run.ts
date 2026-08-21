/**
 * Interactive TUI entrypoint (Ink).
 *
 * `runTui` is a mount loop, not a single render: blocking TTY `e` (vim)
 * records an edit request, unmounts via the normal quit path, hands the
 * terminal to `$EDITOR`, then remounts with the same `SessionState` so
 * cursor and folds come back. GUI editors (Cursor, VS Code) spawn detached
 * while still mounted and never enter this loop. Plain quit (double Ctrl+C)
 * / hang-up leave with no pending edit and the loop ends.
 * Single Ctrl+C is handled in App (`exitOnCtrlC: false`) so the first press
 * only prompts; a second within the window exits.
 *
 * While mounted, the TUI uses the terminal alternate screen (DEC 1049) so
 * frames do not pollute primary scrollback; leave/re-enter brackets a blocking TTY `$EDITOR`.
 */

import path from 'node:path';
import { spawn } from 'node:child_process';
import React from 'react';
import { render } from 'ink';
import type { RepoSnapshot } from '../types.js';
import { withAlternateScreen } from './alternateScreen.js';
import { App } from './App.js';
import { editorCommand, resolveEditor } from './editor.js';
import type { EditRequest, ExitReason, SessionState } from './session.js';
import { createSessionState } from './session.js';
import { prepareTerminalForEditor, restoreTerminalAfterEditor } from './terminalResume.js';

export interface RunTuiOptions {
  cwd: string;
  snapshots: RepoSnapshot[];
  ignoredRepos: string[];
  /**
   * Initial ignored-repo visibility. True when the process started with `-a`.
   * `.` toggles this at runtime; the TUI still discovers ignored repos either way.
   */
  showIgnored: boolean;
  /** Discovery depth from workspace config (must match the initial snapshot pass). */
  maxDepth: number;
  /** Per-repo default branch overrides from workspace config. */
  defaultBranches: Record<string, string>;
  filterRepos: string[];
  /**
   * Optional editor command from workspace config. Non-blank values override
   * `$EDITOR` / `$VISUAL`; omitted or blank falls through to those, then `vim`.
   */
  editor?: string;
}

/**
 * Run a foreground command with the terminal attached, resolving when it
 * exits. Spawn failures also resolve (after logging) — a missing editor must
 * not throw out of `runTui` and leave the terminal unusable; the loop remounts
 * (or, if the caller prefers to stop, it can decide after this returns).
 */
function runInteractiveCommand(command: string, args: string[], cwd: string): Promise<void> {
  return new Promise((resolve) => {
    const child = spawn(command, args, { cwd, stdio: 'inherit' });
    child.on('error', (err) => {
      console.error(`Failed to launch editor (${command}): ${err.message}`);
      resolve();
    });
    child.on('close', () => resolve());
  });
}

/**
 * Launch the interactive workspace-status TUI.
 *
 * Uses the terminal alternate screen (DEC 1049) while mounted so Ink frames
 * do not remain in primary scrollback after exit — same idea as Vim/less.
 * The buffer is left before `$EDITOR` and re-entered on remount; `finally`
 * always restores the primary screen even if render/wait throws.
 * SIGKILL cannot run that `finally` or the process exit hooks in
 * `alternateScreen.ts`.
 */
export async function runTui(opts: RunTuiOptions): Promise<void> {
  await withAlternateScreen(async (screen) => {
    /**
     * View state lives outside the mount so it survives an unmount. The callback
     * identity is stable, and it only writes to this local — a state update here
     * would re-render the App and re-fire the reporting effect forever.
     */
    let session: SessionState = createSessionState(process.env, {
      showIgnored: opts.showIgnored,
    });
    const onSessionChange = (next: SessionState): void => {
      session = next;
    };
    /**
     * `exitReason` is authoritative for explicit exits App reports. The edit
     * path never writes `{ type: 'edit' }` from the action — it records
     * `pendingEdit` and unmounts as quit. The loop below keys off `pendingEdit`,
     * not `exitReason.type === 'edit'`, so Ctrl+C with no prior `e` cannot
     * launch an editor.
     */
    let exitReason: ExitReason = { type: 'quit' };
    const onExit = (reason: ExitReason): void => {
      exitReason = reason;
    };

    for (;;) {
      /**
       * Held on an object so TypeScript trusts the write from `onEditRequest`
       * after `await` (a bare `let` assigned only inside a closure stays
       * narrowed to its initial `null`).
       */
      const pending = { edit: null as EditRequest | null };
      const onEditRequest = (request: EditRequest): void => {
        pending.edit = request;
      };

      const instance = render(
        React.createElement(App, {
          cwd: opts.cwd,
          snapshots: opts.snapshots,
          ignoredRepos: opts.ignoredRepos,
          maxDepth: opts.maxDepth,
          defaultBranches: opts.defaultBranches,
          filterRepos: opts.filterRepos,
          editor: opts.editor,
          session,
          onSessionChange,
          onExit,
          onEditRequest,
        }),
        // App owns double Ctrl+C (prompt, then quit) — same pattern as LLM harnesses.
        { exitOnCtrlC: false },
      );
      await instance.waitUntilExit();

      const edit = pending.edit;
      pending.edit = null;
      if (!edit) {
        void exitReason;
        return;
      }

      const editor = resolveEditor(process.env, opts.editor);

      // Hand the primary screen to `$EDITOR`; re-enter alt buffer before remount.
      screen.leave();
      prepareTerminalForEditor(process.stdin, process.stdout);
      const { command, args } = editorCommand(editor, edit.filePath, edit.line);
      await runInteractiveCommand(command, args, path.join(opts.cwd, edit.repoPath));
      restoreTerminalAfterEditor(process.stdin, process.stdout);
      screen.enter();
    }
  });
}
