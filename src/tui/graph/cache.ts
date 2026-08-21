import { loadGraphModel } from './load.js';
import type { GraphModel, LaidOutCommit } from './types.js';

/**
 * Cache key dimensions for a graph window (A9).
 */
export type GraphCacheKey = {
  repoPath: string;
  refsFingerprint: string;
  skip: number;
  limit: number;
};

/**
 * Cached graph payload; optional memoized layout for P3.
 */
export type GraphSnapshot = {
  model: GraphModel;
  laidOut?: LaidOutCommit[];
  loadedAt: number;
};

type LoadFn = (
  repoPath: string,
  opts: { skip: number; limit: number },
) => Promise<GraphModel>;

function keyOf(k: GraphCacheKey): string {
  return `${k.repoPath}\0${k.refsFingerprint}\0${k.skip}\0${k.limit}`;
}

/**
 * Whether the graph cursor is at the end of the loaded window and should autoload.
 */
export function shouldAutoload(args: {
  cursorIndex: number;
  loadedCount: number;
  hasMore: boolean;
  loading: boolean;
}): boolean {
  if (!args.hasMore || args.loading || args.loadedCount <= 0) return false;
  return args.cursorIndex >= args.loadedCount - 1;
}

/**
 * Create an in-memory graph cache with fingerprint invalidation and autoload dedupe.
 */
export function createGraphCache() {
  const store = new Map<string, GraphSnapshot>();
  const inflight = new Map<string, Promise<GraphModel>>();

  return {
    get(key: GraphCacheKey): GraphSnapshot | undefined {
      return store.get(keyOf(key));
    },
    set(key: GraphCacheKey, snap: GraphSnapshot): void {
      store.set(keyOf(key), snap);
    },
    invalidateRepo(repoPath: string): void {
      for (const k of store.keys()) {
        if (k.startsWith(repoPath + '\0')) store.delete(k);
      }
      inflight.delete(repoPath);
    },
    clear(): void {
      store.clear();
      inflight.clear();
    },
    async getOrLoad(
      repoPath: string,
      refsFingerprint: string,
      opts: { skip?: number; limit?: number } = {},
      loadFn: LoadFn = loadGraphModel,
    ): Promise<GraphModel> {
      const skip = opts.skip ?? 0;
      const limit = opts.limit ?? 300;
      const hit = store.get(keyOf({ repoPath, refsFingerprint, skip, limit }));
      if (hit && hit.model.refsFingerprint === refsFingerprint) return hit.model;
      const model = await loadFn(repoPath, { skip, limit });
      store.set(keyOf({ repoPath, refsFingerprint: model.refsFingerprint, skip, limit }), {
        model,
        loadedAt: Date.now(),
      });
      return model;
    },
    autoloadNext(
      repoPath: string,
      current: GraphModel,
      loadFn: LoadFn = loadGraphModel,
    ): Promise<GraphModel> {
      if (!current.hasMore) return Promise.resolve(current);
      const existing = inflight.get(repoPath);
      if (existing) return existing;

      const promise = (async () => {
        const windowCount = current.windowCount ?? current.commits.length;
        const nextSkip = current.skip + windowCount;
        const page = await loadFn(repoPath, { skip: nextSkip, limit: current.limit });
        const pageWindowCount = page.windowCount ?? page.commits.length;
        const curWindow = current.commits.slice(0, windowCount);
        const curExtras = current.commits.slice(windowCount);
        const pageWindow = page.commits.slice(0, pageWindowCount);
        const pageExtras = page.commits.slice(pageWindowCount);
        const seenWindow = new Set(curWindow.map((c) => c.id));
        const mergedWindow = [
          ...curWindow,
          ...pageWindow.filter((c) => !seenWindow.has(c.id)),
        ];
        const seenAll = new Set(mergedWindow.map((c) => c.id));
        const extraById = new Map<string, (typeof curExtras)[number]>();
        for (const c of [...curExtras, ...pageExtras]) {
          if (seenAll.has(c.id)) continue;
          const prev = extraById.get(c.id);
          extraById.set(
            c.id,
            prev
              ? {
                  ...c,
                  parents: [...new Set([...prev.parents, ...c.parents])],
                }
              : c,
          );
        }
        const merged: GraphModel = {
          ...current,
          commits: [...mergedWindow, ...extraById.values()],
          stashes: page.stashes,
          uncommitted: page.uncommitted,
          headId: page.headId,
          refsFingerprint: page.refsFingerprint,
          hasMore: page.hasMore,
          windowCount: mergedWindow.length,
          // keep skip at original window start; limit unchanged
        };
        store.set(
          keyOf({
            repoPath,
            refsFingerprint: merged.refsFingerprint,
            skip: merged.skip,
            limit: merged.limit,
          }),
          { model: merged, loadedAt: Date.now() },
        );
        return merged;
      })().finally(() => {
        inflight.delete(repoPath);
      });

      inflight.set(repoPath, promise);
      return promise;
    },
  };
}

/**
 * In-memory graph cache instance type.
 */
export type GraphCache = ReturnType<typeof createGraphCache>;
