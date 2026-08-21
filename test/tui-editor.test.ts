import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  editorCommand,
  isDetachedEditor,
  parseEditorArgv,
  resolveEditor,
} from '../src/tui/editor.js';

describe('resolveEditor', () => {
  it('prefers EDITOR over VISUAL', () => {
    assert.equal(resolveEditor({ EDITOR: 'nvim', VISUAL: 'code' }), 'nvim');
  });

  it('falls back to VISUAL then vim', () => {
    assert.equal(resolveEditor({ VISUAL: 'code' }), 'code');
    assert.equal(resolveEditor({}), 'vim');
  });

  it('ignores empty values', () => {
    assert.equal(resolveEditor({ EDITOR: '', VISUAL: 'code' }), 'code');
    assert.equal(resolveEditor({ EDITOR: '   ' }), 'vim');
  });

  it('prefers non-blank config editor over EDITOR', () => {
    assert.equal(resolveEditor({ EDITOR: 'nvim', VISUAL: 'code' }, 'cursor'), 'cursor');
  });

  it('treats blank config editor as unset', () => {
    assert.equal(resolveEditor({ EDITOR: 'nvim' }, '  '), 'nvim');
    assert.equal(resolveEditor({}, ''), 'vim');
  });
});

describe('parseEditorArgv', () => {
  it('splits whitespace-separated tokens', () => {
    assert.deepEqual(parseEditorArgv('code --wait'), ['code', '--wait']);
    assert.deepEqual(parseEditorArgv('nvim -p'), ['nvim', '-p']);
  });

  it('keeps quoted paths with spaces as one token', () => {
    assert.deepEqual(parseEditorArgv('"path with spaces/editor" --flag'), [
      'path with spaces/editor',
      '--flag',
    ]);
    assert.deepEqual(parseEditorArgv("'my editor' -w"), ['my editor', '-w']);
  });
});

describe('editorCommand', () => {
  it('opens vim-family editors at a line with +N', () => {
    assert.deepEqual(editorCommand('vim', 'src/a.ts', 12), {
      command: 'vim',
      args: ['+12', 'src/a.ts'],
    });
    assert.deepEqual(editorCommand('nvim', 'src/a.ts', 12).args, ['+12', 'src/a.ts']);
    assert.deepEqual(editorCommand('nano', 'src/a.ts', 12).args, ['+12', 'src/a.ts']);
  });

  it('opens VS Code family with -g path:line', () => {
    assert.deepEqual(editorCommand('code', 'src/a.ts', 12), {
      command: 'code',
      args: ['-g', 'src/a.ts:12'],
    });
    assert.deepEqual(editorCommand('cursor', 'src/a.ts', 12).args, ['-g', 'src/a.ts:12']);
  });

  it('passes only the path for unknown editors', () => {
    assert.deepEqual(editorCommand('emacs', 'src/a.ts', 12), {
      command: 'emacs',
      args: ['src/a.ts'],
    });
  });

  it('passes only the path when no line is known', () => {
    assert.deepEqual(editorCommand('vim', 'src/a.ts'), { command: 'vim', args: ['src/a.ts'] });
  });

  it('matches on basename so absolute editor paths still work', () => {
    assert.deepEqual(editorCommand('/usr/bin/vim', 'src/a.ts', 3).args, ['+3', 'src/a.ts']);
  });

  it('parses multi-token EDITOR and keeps line-goto', () => {
    assert.deepEqual(editorCommand('code --wait', 'src/a.ts', 12), {
      command: 'code',
      args: ['--wait', '-g', 'src/a.ts:12'],
    });
    assert.deepEqual(editorCommand('nvim -p', 'src/a.ts', 12), {
      command: 'nvim',
      args: ['-p', '+12', 'src/a.ts'],
    });
  });

  it('matches basename after splitting absolute multi-token paths', () => {
    assert.deepEqual(editorCommand('/usr/bin/nvim -p', 'src/a.ts', 3), {
      command: '/usr/bin/nvim',
      args: ['-p', '+3', 'src/a.ts'],
    });
  });
});

describe('isDetachedEditor', () => {
  it('treats Cursor and VS Code family as detached GUI launches', () => {
    assert.equal(isDetachedEditor('cursor'), true);
    assert.equal(isDetachedEditor('cursor --wait'), true);
    assert.equal(isDetachedEditor('code'), true);
    assert.equal(isDetachedEditor('code --wait'), true);
    assert.equal(isDetachedEditor('code-insiders'), true);
    assert.equal(isDetachedEditor('codium'), true);
    assert.equal(isDetachedEditor('gvim'), true);
    assert.equal(isDetachedEditor('/usr/bin/cursor'), true);
  });

  it('keeps TTY editors and unknown names on the blocking remount path', () => {
    assert.equal(isDetachedEditor('vim'), false);
    assert.equal(isDetachedEditor('nvim'), false);
    assert.equal(isDetachedEditor('nvim -p'), false);
    assert.equal(isDetachedEditor('nano'), false);
    assert.equal(isDetachedEditor('emacs'), false);
    assert.equal(isDetachedEditor('helix'), false);
    assert.equal(isDetachedEditor(''), false);
  });
});
