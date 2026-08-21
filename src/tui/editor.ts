/**
 * Resolving which editor to launch and how to tell it which line to open.
 *
 * Kept separate from the spawn itself so the argument logic is testable
 * without starting a process.
 */

import path from 'node:path';

/** A spawnable editor invocation. */
export interface EditorCommand {
  command: string;
  args: string[];
}

/** Editors that accept `+LINE` before the path. */
const PLUS_LINE = new Set(['vim', 'nvim', 'vi', 'nano', 'gvim']);

/** Editors that accept `-g path:LINE`. */
const GOTO_FLAG = new Set(['code', 'code-insiders', 'cursor', 'codium']);

/**
 * Split an editor string into argv tokens.
 *
 * Handles whitespace-separated tokens and simple single/double quoting
 * (`code --wait`, `nvim -p`, `"path with spaces/editor"`). Not a full shell —
 * no escapes, expansion, or operators.
 */
export function parseEditorArgv(editor: string): string[] {
  const tokens: string[] = [];
  let current = '';
  let quote: '"' | "'" | null = null;

  for (let i = 0; i < editor.length; i++) {
    const ch = editor[i]!;
    if (quote) {
      if (ch === quote) {
        quote = null;
      } else {
        current += ch;
      }
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      continue;
    }
    if (/\s/.test(ch)) {
      if (current) {
        tokens.push(current);
        current = '';
      }
      continue;
    }
    current += ch;
  }
  if (current) tokens.push(current);
  return tokens;
}

/**
 * The editor to launch: non-blank config `editor`, then `$EDITOR`, then
 * `$VISUAL`, then `vim`. Blank values are ignored so an exported-but-empty
 * variable or whitespace-only config does not win.
 */
export function resolveEditor(env: NodeJS.ProcessEnv, configEditor?: string): string {
  const fromConfig = configEditor?.trim();
  if (fromConfig) return fromConfig;
  const editor = env.EDITOR?.trim();
  if (editor) return editor;
  const visual = env.VISUAL?.trim();
  if (visual) return visual;
  return 'vim';
}

/**
 * Command and arguments to open `filePath`, at `line` when the editor is known
 * to support it. Unknown editors receive the path alone rather than a flag
 * they might treat as a filename.
 *
 * Multi-token values (`code --wait`, `nvim -p`) are split: the first token is
 * the executable, remaining tokens are fixed args prepended before line/path
 * args. PLUS_LINE / GOTO_FLAG matching uses the basename of the executable.
 */
export function editorCommand(editor: string, filePath: string, line?: number): EditorCommand {
  const argv = parseEditorArgv(editor);
  const command = argv[0] ?? editor;
  const fixedArgs = argv.slice(1);
  const name = path.basename(command);

  if (line === undefined) {
    return { command, args: [...fixedArgs, filePath] };
  }
  if (PLUS_LINE.has(name)) {
    return { command, args: [...fixedArgs, `+${line}`, filePath] };
  }
  if (GOTO_FLAG.has(name)) {
    return { command, args: [...fixedArgs, '-g', `${filePath}:${line}`] };
  }
  return { command, args: [...fixedArgs, filePath] };
}

/** GUI editors that return immediately and must not steal the TTY / remount Ink. */
const DETACHED_EDITORS = new Set(['code', 'code-insiders', 'cursor', 'codium', 'gvim']);

/**
 * True when `e` should spawn the editor without unmounting the TUI.
 *
 * Cursor / VS Code family (and gvim) are separate GUI processes. Their CLIs
 * typically exit as soon as the window or tab opens, so the old remount loop
 * rebuilt the whole tree immediately. TTY editors (vim, nvim, nano, and
 * unknown names) still use the blocking inherit path.
 *
 * `--wait` / `-w` does not change this. The GUI still does not need the TTY,
 * and staying mounted keeps fold, focus, and scroll.
 */
export function isDetachedEditor(editor: string): boolean {
  const argv = parseEditorArgv(editor);
  const command = argv[0] ?? editor;
  return DETACHED_EDITORS.has(path.basename(command));
}
