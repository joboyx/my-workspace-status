import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  parseWorktreeListPorcelain,
  linkedWorktreesUnderCwd,
  mapLinkedWorktreeRelPath,
  resolveWorktreeRemoveTarget,
  classifyMergedIntoDefault,
  type LinkedWorktreePathIo,
  type PathIdentity,
} from '../src/worktrees.js';

describe('parseWorktreeListPorcelain', () => {
  it('parses primary + linked entries', () => {
    const text = [
      'worktree /tmp/ws/app',
      'HEAD abc',
      'branch refs/heads/main',
      '',
      'worktree /tmp/ws/app/.worktrees/feat',
      'HEAD def',
      'branch refs/heads/feature/x',
      '',
    ].join('\n');
    const entries = parseWorktreeListPorcelain(text);
    assert.equal(entries.length, 2);
    assert.equal(entries[0]?.path, '/tmp/ws/app');
    assert.equal(entries[0]?.branch, 'main');
    assert.equal(entries[0]?.bare, false);
    assert.equal(entries[1]?.path, '/tmp/ws/app/.worktrees/feat');
    assert.equal(entries[1]?.branch, 'feature/x');
  });

  it('marks bare and detached', () => {
    const text = [
      'worktree /tmp/bare',
      'bare',
      '',
      'worktree /tmp/d',
      'HEAD abc',
      'detached',
      '',
    ].join('\n');
    const entries = parseWorktreeListPorcelain(text);
    assert.equal(entries[0]?.bare, true);
    assert.equal(entries[1]?.detached, true);
  });
});

describe('linkedWorktreesUnderCwd', () => {
  it('keeps only non-primary paths under cwd', () => {
    const entries = parseWorktreeListPorcelain(
      [
        'worktree /tmp/ws/app',
        'branch refs/heads/main',
        '',
        'worktree /tmp/ws/app/.worktrees/feat',
        'branch refs/heads/feature/x',
        '',
        'worktree /tmp/elsewhere/feat',
        'branch refs/heads/feature/y',
        '',
        'worktree /tmp/ws/app-bare',
        'bare',
        '',
      ].join('\n'),
    );
    const linked = linkedWorktreesUnderCwd(entries, '/tmp/ws', '/tmp/ws/app');
    assert.deepEqual(linked, [
      { absPath: '/tmp/ws/app/.worktrees/feat', relPath: 'app/.worktrees/feat' },
    ]);
  });

  it('remaps bind-mount aliases of the primary onto the cwd-visible path', () => {
    // Simulate: cwd sees primary at /ws/workspace/dotfiles, but git registered
    // linked worktrees under /ws/dotfiles/... (same inode as workspace/dotfiles).
    const ids = new Map<string, PathIdentity>([
      ['/ws/workspace', { dev: 1, ino: 10 }],
      ['/ws/workspace/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/dotfiles/.worktrees', { dev: 1, ino: 200 }],
      ['/ws/dotfiles/.worktrees/feat', { dev: 1, ino: 201 }],
      ['/ws/workspace/dotfiles/.worktrees/feat', { dev: 1, ino: 201 }],
    ]);
    const io: LinkedWorktreePathIo = {
      realpath: (p) => p,
      identity: (p) => ids.get(p) ?? null,
    };
    const entries = parseWorktreeListPorcelain(
      [
        'worktree /ws/workspace/dotfiles',
        'branch refs/heads/main',
        '',
        'worktree /ws/dotfiles/.worktrees/feat',
        'branch refs/heads/feature/x',
        '',
        'worktree /ws/elsewhere/.worktrees/other',
        'branch refs/heads/feature/y',
        '',
      ].join('\n'),
    );
    const linked = linkedWorktreesUnderCwd(entries, '/ws/workspace', '/ws/workspace/dotfiles', io);
    assert.deepEqual(linked, [
      {
        absPath: '/ws/workspace/dotfiles/.worktrees/feat',
        relPath: 'dotfiles/.worktrees/feat',
      },
    ]);
  });

  it('skips primary when git lists it under a bind-mount alias path', () => {
    const ids = new Map<string, PathIdentity>([
      ['/ws/workspace/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/dotfiles', { dev: 1, ino: 100 }],
    ]);
    const io: LinkedWorktreePathIo = {
      realpath: (p) => p,
      identity: (p) => ids.get(p) ?? null,
    };
    assert.equal(
      mapLinkedWorktreeRelPath('/ws/dotfiles', '/ws/workspace', '/ws/workspace/dotfiles', io),
      null,
    );
  });

  it('keeps relative paths when primary equals cwd (no leading slash)', () => {
    const ids = new Map<string, PathIdentity>([
      ['/ws/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/alias-dotfiles', { dev: 1, ino: 100 }],
      ['/ws/alias-dotfiles/.worktrees/feat', { dev: 1, ino: 201 }],
    ]);
    const io: LinkedWorktreePathIo = {
      realpath: (p) => p,
      identity: (p) => ids.get(p) ?? null,
    };
    const mapped = mapLinkedWorktreeRelPath(
      '/ws/alias-dotfiles/.worktrees/feat',
      '/ws/dotfiles',
      '/ws/dotfiles',
      io,
    );
    assert.deepEqual(mapped, {
      absPath: '/ws/dotfiles/.worktrees/feat',
      relPath: '.worktrees/feat',
    });
    assert.equal(mapped && !mapped.relPath.startsWith('/'), true);
  });

  it('prefers cwd-visible primary prefix when alias path is also under cwd', () => {
    // cwd=/ws contains both workspace/dotfiles (primary) and sibling bind-mount
    // /ws/dotfiles — alias is under cwd but should still remap under primary.
    const ids = new Map<string, PathIdentity>([
      ['/ws', { dev: 1, ino: 1 }],
      ['/ws/workspace/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/dotfiles/.worktrees/feat', { dev: 1, ino: 201 }],
    ]);
    const io: LinkedWorktreePathIo = {
      realpath: (p) => p,
      identity: (p) => ids.get(p) ?? null,
    };
    const mapped = mapLinkedWorktreeRelPath(
      '/ws/dotfiles/.worktrees/feat',
      '/ws',
      '/ws/workspace/dotfiles',
      io,
    );
    assert.deepEqual(mapped, {
      absPath: '/ws/workspace/dotfiles/.worktrees/feat',
      relPath: 'workspace/dotfiles/.worktrees/feat',
    });
  });
});

describe('resolveWorktreeRemoveTarget', () => {
  it('rewrites bind-mount aliases to git-registered path and cwd', () => {
    // TUI sees workspace/dotfiles/...; git registered personal/dotfiles/...
    const ids = new Map<string, PathIdentity>([
      ['/ws/workspace/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/personal/dotfiles', { dev: 1, ino: 100 }],
      ['/ws/workspace/dotfiles/.worktrees/feat', { dev: 1, ino: 201 }],
      ['/ws/personal/dotfiles/.worktrees/feat', { dev: 1, ino: 201 }],
    ]);
    const io: LinkedWorktreePathIo = {
      realpath: (p) => p,
      identity: (p) => ids.get(p) ?? null,
    };
    const entries = parseWorktreeListPorcelain(
      [
        'worktree /ws/workspace/dotfiles',
        'branch refs/heads/main',
        '',
        'worktree /ws/personal/dotfiles/.worktrees/feat',
        'branch refs/heads/chore/x',
        '',
      ].join('\n'),
    );
    assert.deepEqual(
      resolveWorktreeRemoveTarget(
        entries,
        '/ws/workspace/dotfiles',
        '/ws/workspace/dotfiles/.worktrees/feat',
        io,
      ),
      {
        gitCwd: '/ws/personal/dotfiles',
        gitPath: '/ws/personal/dotfiles/.worktrees/feat',
      },
    );
  });

  it('keeps cwd-visible paths when git already registered them', () => {
    const ids = new Map<string, PathIdentity>([
      ['/ws/app', { dev: 1, ino: 100 }],
      ['/ws/app/.worktrees/feat', { dev: 1, ino: 201 }],
    ]);
    const io: LinkedWorktreePathIo = {
      realpath: (p) => p,
      identity: (p) => ids.get(p) ?? null,
    };
    const entries = parseWorktreeListPorcelain(
      [
        'worktree /ws/app',
        'branch refs/heads/main',
        '',
        'worktree /ws/app/.worktrees/feat',
        'branch refs/heads/feature/x',
        '',
      ].join('\n'),
    );
    assert.deepEqual(
      resolveWorktreeRemoveTarget(entries, '/ws/app', '/ws/app/.worktrees/feat', io),
      {
        gitCwd: '/ws/app',
        gitPath: '/ws/app/.worktrees/feat',
      },
    );
  });

  it('falls back to caller paths when the worktree is not listed', () => {
    const io: LinkedWorktreePathIo = {
      realpath: (p) => p,
      identity: () => null,
    };
    assert.deepEqual(
      resolveWorktreeRemoveTarget(
        parseWorktreeListPorcelain('worktree /ws/app\nbranch refs/heads/main\n'),
        '/ws/app',
        '/ws/app/.worktrees/missing',
        io,
      ),
      {
        gitCwd: '/ws/app',
        gitPath: '/ws/app/.worktrees/missing',
      },
    );
  });
});

describe('classifyMergedIntoDefault', () => {
  it('returns null on default branch', () => {
    assert.equal(
      classifyMergedIntoDefault({
        branch: 'main',
        defaultBranch: 'main',
        isAncestorOfDefault: true,
      }),
      null,
    );
  });
  it('returns true/false/null for feature branches', () => {
    assert.equal(
      classifyMergedIntoDefault({
        branch: 'feature/x',
        defaultBranch: 'main',
        isAncestorOfDefault: true,
      }),
      true,
    );
    assert.equal(
      classifyMergedIntoDefault({
        branch: 'feature/x',
        defaultBranch: 'main',
        isAncestorOfDefault: false,
      }),
      false,
    );
    assert.equal(
      classifyMergedIntoDefault({
        branch: 'feature/x',
        defaultBranch: 'main',
        isAncestorOfDefault: null,
      }),
      null,
    );
  });
  it('returns null for detached HEAD even when ancestry is known', () => {
    assert.equal(
      classifyMergedIntoDefault({
        branch: 'HEAD (detached)',
        defaultBranch: 'main',
        isAncestorOfDefault: true,
      }),
      null,
    );
  });
});
