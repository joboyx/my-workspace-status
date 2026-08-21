import assert from 'node:assert';
import { describe, it } from 'node:test';
import * as fs from 'node:fs/promises';
import * as os from 'node:os';
import * as path from 'node:path';
import {
  DEFAULT_WATCH_MS,
  FLASH_MS,
  MIN_WATCH_MS,
  changeSignatures,
  changedNodeIds,
  fileNodeId,
  flashStrength,
  flashableNodeIds,
  mergeGhostRows,
  pruneFlashes,
  removalGhosts,
  removedNodeIds,
  repoNodeId,
  watchIntervalMs,
} from '../src/tui/watch.js';
import { flashBackground } from '../src/tui/theme.js';
import type { RepoSnapshot } from '../src/types.js';
import type { VisibleRow } from '../src/tui/model/types.js';

function snapshot(partial: Partial<RepoSnapshot>): RepoSnapshot {
  return {
    repo: 'demo',
    branch: 'main',
    syncStatus: 'up-to-date',
    syncNote: '',
    hasUnstaged: false,
    hasStaged: false,
    hasUntracked: false,
    unstagedInfo: '',
    stagedFiles: '',
    unstagedFiles: '',
    untrackedFiles: '',
    checkoutKind: 'primary',
    mergedIntoDefault: null,
    ...partial,
  };
}

function visibleRow(id: string, label: string): VisibleRow {
  return {
    id,
    label,
    depth: 1,
    node: {} as never,
    segments: [],
    trailing: [],
  };
}

describe('watchIntervalMs', () => {
  it('defaults when unset or unparseable', () => {
    assert.equal(watchIntervalMs({}), DEFAULT_WATCH_MS);
    assert.equal(watchIntervalMs({ WS_STATUS_WATCH_MS: '' }), DEFAULT_WATCH_MS);
    assert.equal(watchIntervalMs({ WS_STATUS_WATCH_MS: 'soon' }), DEFAULT_WATCH_MS);
    assert.equal(watchIntervalMs({ WS_STATUS_WATCH_MS: '-5' }), DEFAULT_WATCH_MS);
  });

  it('treats 0 as disabled and clamps anything too aggressive', () => {
    assert.equal(watchIntervalMs({ WS_STATUS_WATCH_MS: '0' }), 0);
    assert.equal(watchIntervalMs({ WS_STATUS_WATCH_MS: '10' }), MIN_WATCH_MS);
    assert.equal(watchIntervalMs({ WS_STATUS_WATCH_MS: '8000' }), 8000);
  });
});

describe('changeSignatures', () => {
  it('signs every changed file and survives files missing from disk', async () => {
    const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'ws-watch-'));
    try {
      await fs.mkdir(path.join(dir, 'demo/src'), { recursive: true });
      await fs.writeFile(path.join(dir, 'demo/src/main.ts'), 'a\n');

      const snap = snapshot({
        hasUnstaged: true,
        // `gone.ts` is in git status but absent from the worktree.
        unstagedFiles: 'M\tsrc/main.ts|||D\tsrc/gone.ts',
      });

      const first = await changeSignatures(dir, [snap]);
      assert.equal(first.size, 2);
      assert.ok(first.has(fileNodeId('demo', 'src/main.ts')));
      assert.match(first.get(fileNodeId('demo', 'src/gone.ts')) ?? '', /gone$/);

      // Same content, same signature — a poll tick must be a no-op.
      // Compare sorted: signatures are gathered concurrently, so Map insertion
      // order varies between runs and is not part of the contract.
      const second = await changeSignatures(dir, [snap]);
      assert.deepEqual([...second].sort(), [...first].sort());
      assert.deepEqual(changedNodeIds(first, second), []);

      // Editing the file changes its signature.
      await fs.writeFile(path.join(dir, 'demo/src/main.ts'), 'a\nb\n');
      const third = await changeSignatures(dir, [snap]);
      assert.deepEqual(changedNodeIds(first, third), [fileNodeId('demo', 'src/main.ts')]);
    } finally {
      await fs.rm(dir, { recursive: true, force: true });
    }
  });

  it('is empty for a clean workspace', async () => {
    assert.equal((await changeSignatures('/nonexistent', [snapshot({})])).size, 0);
  });
});

describe('changedNodeIds', () => {
  it('reports new and altered ids but not removed ones', () => {
    const before = new Map([
      ['file:a:x', 'M:1'],
      ['file:a:y', 'M:1'],
    ]);
    const after = new Map([
      ['file:a:x', 'M:2'], // changed
      ['file:a:z', 'A:1'], // new
    ]);
    assert.deepEqual(changedNodeIds(before, after).sort(), ['file:a:x', 'file:a:z']);
  });
});

describe('repoNodeId', () => {
  it('matches tree repo ids', () => {
    assert.equal(repoNodeId('demo-services'), 'repo:demo-services');
  });
});

describe('removedNodeIds + flashableNodeIds', () => {
  it('reports ids that disappeared from the signature map', () => {
    const before = new Map([
      ['file:a:x', 'M:1'],
      ['file:a:y', 'M:1'],
    ]);
    const after = new Map([
      ['file:a:x', 'M:2'],
      ['file:a:z', 'A:1'],
    ]);
    assert.deepEqual(removedNodeIds(before, after), ['file:a:y']);
    assert.deepEqual(
      flashableNodeIds(before, after).sort(),
      ['file:a:x', 'file:a:y', 'file:a:z'],
    );
  });
});

describe('removalGhosts', () => {
  it('captures the last live index for each removed id', () => {
    const prev = [
      visibleRow('file:a:x', 'x'),
      visibleRow('file:a:y', 'y'),
      visibleRow('file:a:z', 'z'),
    ];
    const ghosts = removalGhosts(prev, ['file:a:y'], 1000);
    assert.equal(ghosts.length, 1);
    assert.equal(ghosts[0]?.id, 'file:a:y');
    assert.equal(ghosts[0]?.index, 1);
    assert.equal(ghosts[0]?.flashedAt, 1000);
  });

  it('skips ids that were already absent from the live list', () => {
    const prev = [visibleRow('file:a:x', 'x')];
    assert.deepEqual(removalGhosts(prev, ['file:a:missing'], 1000), []);
  });
});

describe('mergeGhostRows', () => {
  it('keeps a removed row visible in place until FLASH_MS elapses', () => {
    const live: VisibleRow[] = [
      visibleRow('file:a:x', 'x'),
      visibleRow('file:a:z', 'z'),
    ];
    const ghostRow = visibleRow('file:a:y', 'y');
    const ghosts = [{ id: 'file:a:y', row: ghostRow, flashedAt: 1000, index: 1 }];
    const merged = mergeGhostRows(live, ghosts, 1000 + FLASH_MS / 2);
    assert.deepEqual(
      merged.map((r) => r.id),
      ['file:a:x', 'file:a:y', 'file:a:z'],
    );
    const after = mergeGhostRows(live, ghosts, 1000 + FLASH_MS);
    assert.deepEqual(
      after.map((r) => r.id),
      ['file:a:x', 'file:a:z'],
    );
  });

  it('does not duplicate an id that returned to the live list', () => {
    const live: VisibleRow[] = [visibleRow('file:a:y', 'y')];
    const ghosts = [
      {
        id: 'file:a:y',
        row: live[0]!,
        flashedAt: 1000,
        index: 0,
      },
    ];
    const merged = mergeGhostRows(live, ghosts, 1000);
    assert.equal(merged.length, 1);
  });

  it('re-inserts multiple removals at their original indices', () => {
    // Live list after b@1 and d@3 left [a,b,c,d,e].
    const live: VisibleRow[] = [
      visibleRow('file:a:a', 'a'),
      visibleRow('file:a:c', 'c'),
      visibleRow('file:a:e', 'e'),
    ];
    const ghosts = [
      {
        id: 'file:a:d',
        row: visibleRow('file:a:d', 'd'),
        flashedAt: 1000,
        index: 3,
      },
      {
        id: 'file:a:b',
        row: visibleRow('file:a:b', 'b'),
        flashedAt: 1000,
        index: 1,
      },
    ];
    const merged = mergeGhostRows(live, ghosts, 1000);
    assert.deepEqual(
      merged.map((r) => r.id),
      ['file:a:a', 'file:a:b', 'file:a:c', 'file:a:d', 'file:a:e'],
    );
  });
});

describe('flash decay', () => {
  it('defaults FLASH_MS to 800', () => {
    assert.equal(FLASH_MS, 800);
  });

  it('eases from 1 to 0 across the flash duration', () => {
    assert.equal(flashStrength(undefined, 1000), 0);
    assert.equal(flashStrength(1000, 1000), 1);
    assert.equal(flashStrength(1000, 1000 + FLASH_MS), 0);
    assert.equal(flashStrength(1000, 1000 + FLASH_MS * 2), 0);

    const half = flashStrength(1000, 1000 + FLASH_MS / 2);
    assert.ok(half > 0.4 && half < 0.6, String(half));

    // Monotonically decreasing.
    let previous = 1.1;
    for (let t = 0; t < FLASH_MS; t += FLASH_MS / 8) {
      const strength = flashStrength(0, t);
      assert.ok(strength < previous, `${t}: ${strength} !< ${previous}`);
      previous = strength;
    }
  });

  it('maps strength onto a fading background, then nothing', () => {
    assert.equal(flashBackground(0), undefined);
    assert.equal(flashBackground(-1), undefined);
    const bright = flashBackground(1);
    const dim = flashBackground(0.1);
    assert.match(bright ?? '', /^#[0-9a-f]{6}$/);
    assert.match(dim ?? '', /^#[0-9a-f]{6}$/);
    assert.notEqual(bright, dim);
  });

  it('pruneFlashes drops only expired entries', () => {
    const now = 10_000;
    const flashes = new Map([
      ['fresh', now - 100],
      ['stale', now - FLASH_MS - 1],
    ]);
    const pruned = pruneFlashes(flashes, now);
    assert.deepEqual([...pruned.keys()], ['fresh']);
    // Original map is untouched — callers rely on getting a new instance.
    assert.equal(flashes.size, 2);
  });
});
