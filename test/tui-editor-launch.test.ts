import assert from 'node:assert';
import { describe, it } from 'node:test';
import { startEdit, type SpawnLike } from '../src/tui/editorLaunch.js';
import type { EditRequest } from '../src/tui/session.js';

function request(over: Partial<EditRequest> = {}): EditRequest {
  return { repoPath: 'dotfiles', filePath: 'src/a.ts', ...over };
}

function fakeSpawn(calls: { command: string; args: string[]; cwd: string }[]): SpawnLike {
  return (command, args, options) => {
    calls.push({ command, args, cwd: options.cwd });
    assert.equal(options.stdio, 'ignore');
    assert.equal(options.detached, true);
    return {
      on() {},
      unref() {},
    };
  };
}

describe('startEdit', () => {
  it('records pendingEdit and quits for vim so run.ts can remount', () => {
    const seen: EditRequest[] = [];
    const calls: { command: string; args: string[]; cwd: string }[] = [];
    const result = startEdit({
      editor: 'vim',
      request: request(),
      cwd: '/ws',
      onEditRequest: (r) => seen.push(r),
      spawn: fakeSpawn(calls),
    });
    assert.equal(result, 'quit');
    assert.deepEqual(seen, [request()]);
    assert.deepEqual(calls, []);
  });

  it('spawns Cursor detached and keeps the TUI mounted', () => {
    const seen: EditRequest[] = [];
    const calls: { command: string; args: string[]; cwd: string }[] = [];
    const result = startEdit({
      editor: 'cursor',
      request: request({ line: 12 }),
      cwd: '/ws',
      onEditRequest: (r) => seen.push(r),
      spawn: fakeSpawn(calls),
    });
    assert.equal(result, undefined);
    assert.deepEqual(seen, []);
    assert.deepEqual(calls, [
      {
        command: 'cursor',
        args: ['-g', 'src/a.ts:12'],
        cwd: '/ws/dotfiles',
      },
    ]);
  });

  it('stays mounted for code --wait so fold and focus do not remount', () => {
    const seen: EditRequest[] = [];
    const calls: { command: string; args: string[]; cwd: string }[] = [];
    const result = startEdit({
      editor: 'code --wait',
      request: request(),
      cwd: '/ws',
      onEditRequest: (r) => seen.push(r),
      spawn: fakeSpawn(calls),
    });
    assert.equal(result, undefined);
    assert.deepEqual(seen, []);
    assert.equal(calls.length, 1);
    assert.equal(calls[0]?.command, 'code');
    assert.ok(calls[0]?.args.includes('--wait'));
  });
});
