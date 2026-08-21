import assert from 'node:assert';
import { describe, it } from 'node:test';
import { createGraphCache, shouldAutoload } from '../src/tui/graph/cache.js';
import { graphLayoutCommits } from '../src/tui/graph/list.js';
import { DEFAULT_GRAPH_WINDOW, type GraphModel } from '../src/tui/graph/types.js';

function model(partial: Partial<GraphModel> & Pick<GraphModel, 'repoPath' | 'refsFingerprint'>): GraphModel {
  return {
    commits: [],
    stashes: [],
    uncommitted: { kind: 'uncommitted', hasChanges: false },
    headId: null,
    skip: 0,
    limit: DEFAULT_GRAPH_WINDOW,
    hasMore: false,
    ...partial,
  };
}

describe('shouldAutoload', () => {
  it('is true only at end when hasMore and not loading', () => {
    assert.equal(shouldAutoload({ cursorIndex: 9, loadedCount: 10, hasMore: true, loading: false }), true);
    assert.equal(shouldAutoload({ cursorIndex: 8, loadedCount: 10, hasMore: true, loading: false }), false);
    assert.equal(shouldAutoload({ cursorIndex: 9, loadedCount: 10, hasMore: false, loading: false }), false);
    // loading:true must gate — useAppState reads this via a ref (not an effect dep).
    // Putting graphLoadingOlder in the autoload effect deps caused: set loading →
    // cleanup cancel → re-run with loading true → early return → stuck "loading older…".
    assert.equal(shouldAutoload({ cursorIndex: 9, loadedCount: 10, hasMore: true, loading: true }), false);
  });
});

describe('createGraphCache', () => {
  it('get/set keyed by repo + fingerprint + window', () => {
    const cache = createGraphCache();
    const m = model({
      repoPath: '/r',
      refsFingerprint: 'fp1',
      commits: [{ id: 'a', parents: [], subject: 's', authorName: 'n', authorDateUnix: 1, refs: [] }],
    });
    cache.set({ repoPath: '/r', refsFingerprint: 'fp1', skip: 0, limit: 300 }, { model: m, loadedAt: 1 });
    assert.ok(cache.get({ repoPath: '/r', refsFingerprint: 'fp1', skip: 0, limit: 300 }));
    assert.equal(cache.get({ repoPath: '/r', refsFingerprint: 'fp2', skip: 0, limit: 300 }), undefined);
  });

  it('invalidateRepo drops all windows for that path', () => {
    const cache = createGraphCache();
    cache.set(
      { repoPath: '/r', refsFingerprint: 'fp', skip: 0, limit: 300 },
      { model: model({ repoPath: '/r', refsFingerprint: 'fp' }), loadedAt: 1 },
    );
    cache.set(
      { repoPath: '/r', refsFingerprint: 'fp', skip: 300, limit: 300 },
      { model: model({ repoPath: '/r', refsFingerprint: 'fp', skip: 300 }), loadedAt: 1 },
    );
    cache.set(
      { repoPath: '/other', refsFingerprint: 'fp', skip: 0, limit: 300 },
      { model: model({ repoPath: '/other', refsFingerprint: 'fp' }), loadedAt: 1 },
    );
    cache.invalidateRepo('/r');
    assert.equal(cache.get({ repoPath: '/r', refsFingerprint: 'fp', skip: 0, limit: 300 }), undefined);
    assert.ok(cache.get({ repoPath: '/other', refsFingerprint: 'fp', skip: 0, limit: 300 }));
  });

  it('autoloadNext dedupes in-flight and merges commits', async () => {
    const cache = createGraphCache();
    let calls = 0;
    const loadFn = async (repoPath: string, opts: { skip: number; limit: number }) => {
      calls += 1;
      await new Promise((r) => setTimeout(r, 20));
      return model({
        repoPath,
        refsFingerprint: 'fp',
        skip: opts.skip,
        limit: opts.limit,
        hasMore: opts.skip === 0,
        commits: [
          {
            id: `id-${opts.skip}`,
            parents: [],
            subject: `s${opts.skip}`,
            authorName: 'n',
            authorDateUnix: 1,
            refs: [],
          },
        ],
      });
    };

    const base = model({
      repoPath: '/r',
      refsFingerprint: 'fp',
      hasMore: true,
      commits: [
        { id: 'id-0', parents: [], subject: 's0', authorName: 'n', authorDateUnix: 1, refs: [] },
      ],
    });

    const p1 = cache.autoloadNext('/r', base, loadFn);
    const p2 = cache.autoloadNext('/r', base, loadFn);
    assert.equal(p1, p2);
    const merged = await p1;
    assert.equal(calls, 1);
    assert.equal(merged.commits.map((c) => c.id).join(','), 'id-0,id-1');
    assert.equal(merged.hasMore, false); // second page length < limit? with limit 300 and 1 commit → hasMore false
  });

  it('autoloadNext skip ignores extra stash parents after the window prefix', async () => {
    const cache = createGraphCache();
    const skips: number[] = [];
    const loadFn = async (repoPath: string, opts: { skip: number; limit: number }) => {
      skips.push(opts.skip);
      return model({
        repoPath,
        refsFingerprint: 'fp',
        skip: opts.skip,
        limit: opts.limit,
        hasMore: false,
        windowCount: 1,
        commits: [
          {
            id: `win-${opts.skip}`,
            parents: [],
            subject: 'w',
            authorName: 'n',
            authorDateUnix: 1,
            refs: [],
          },
        ],
      });
    };
    const extra = {
      id: 'stash-parent',
      parents: [],
      subject: 'old',
      authorName: 'n',
      authorDateUnix: 0,
      refs: [],
    };
    const base = model({
      repoPath: '/r',
      refsFingerprint: 'fp',
      skip: 0,
      limit: 2,
      hasMore: true,
      windowCount: 2,
      commits: [
        { id: 'w0', parents: [], subject: 'a', authorName: 'n', authorDateUnix: 2, refs: [] },
        { id: 'w1', parents: [], subject: 'b', authorName: 'n', authorDateUnix: 1, refs: [] },
        extra,
      ],
    });
    const merged = await cache.autoloadNext('/r', base, loadFn);
    assert.deepEqual(skips, [2], 'autoload skip must be skip+windowCount, not commits.length');
    assert.equal(merged.windowCount, 3);
    assert.equal(merged.commits[merged.commits.length - 1]!.id, 'stash-parent');
    assert.ok(merged.commits.some((c) => c.id === 'win-2'));
  });

  it('autoloadNext keeps extra stash parent edges across pages', async () => {
    const cache = createGraphCache();
    const extra = {
      id: 'stash-parent',
      parents: ['w1'],
      subject: 'old',
      authorName: 'n',
      authorDateUnix: 3,
      refs: [],
    };
    const loadFn = async (repoPath: string, opts: { skip: number; limit: number }) =>
      model({
        repoPath,
        refsFingerprint: 'fp',
        skip: opts.skip,
        limit: opts.limit,
        hasMore: false,
        windowCount: 1,
        commits: [
          {
            id: 'w2',
            parents: [],
            subject: 'w',
            authorName: 'n',
            authorDateUnix: 0,
            refs: [],
          },
          { ...extra, parents: [] },
        ],
      });
    const base = model({
      repoPath: '/r',
      refsFingerprint: 'fp',
      skip: 0,
      limit: 2,
      hasMore: true,
      windowCount: 2,
      commits: [
        { id: 'w0', parents: ['w1'], subject: 'a', authorName: 'n', authorDateUnix: 2, refs: [] },
        { id: 'w1', parents: [], subject: 'b', authorName: 'n', authorDateUnix: 1, refs: [] },
        extra,
      ],
    });
    const merged = await cache.autoloadNext('/r', base, loadFn);
    const laidExtra = graphLayoutCommits(merged).find((c) => c.id === 'stash-parent')!;
    assert.deepEqual(
      laidExtra.parents,
      ['w1'],
      'page-local empty %P must not drop an extra edge into the merged window',
    );
  });

  it('autoloadNext restores extra→parent once that parent enters the window', async () => {
    const cache = createGraphCache();
    const extra = {
      id: 'stash-parent',
      parents: ['older'],
      subject: 'old',
      authorName: 'n',
      authorDateUnix: 3,
      refs: [],
    };
    const loadFn = async (repoPath: string, opts: { skip: number; limit: number }) =>
      model({
        repoPath,
        refsFingerprint: 'fp',
        skip: opts.skip,
        limit: opts.limit,
        hasMore: false,
        windowCount: 1,
        commits: [
          {
            id: 'older',
            parents: [],
            subject: 'p',
            authorName: 'n',
            authorDateUnix: 0,
            refs: [],
          },
        ],
      });
    const base = model({
      repoPath: '/r',
      refsFingerprint: 'fp',
      skip: 0,
      limit: 2,
      hasMore: true,
      windowCount: 2,
      commits: [
        { id: 'w0', parents: ['w1'], subject: 'a', authorName: 'n', authorDateUnix: 2, refs: [] },
        { id: 'w1', parents: [], subject: 'b', authorName: 'n', authorDateUnix: 1, refs: [] },
        extra,
      ],
    });
    const merged = await cache.autoloadNext('/r', base, loadFn);
    const laidExtra = graphLayoutCommits(merged).find((c) => c.id === 'stash-parent')!;
    assert.deepEqual(
      laidExtra.parents,
      ['older'],
      'extra %P must survive until the parent is in the merged layout set',
    );
  });

  it('autoloadNext is no-op when !hasMore', async () => {
    const cache = createGraphCache();
    let calls = 0;
    const base = model({ repoPath: '/r', refsFingerprint: 'fp', hasMore: false });
    const out = await cache.autoloadNext('/r', base, async () => {
      calls += 1;
      return base;
    });
    assert.equal(calls, 0);
    assert.equal(out, base);
  });
});
