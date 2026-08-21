/**
 * Schedules background `git fetch` and exposes a manual run entry point.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';
import {
  fetchIntervalMs,
  fetchRepos,
  formatFetchAge,
  formatFetchProgress,
} from './fetch.js';

/**
 * Everything {@link useFetch} needs from the app / action layers.
 */
export interface FetchDeps {
  cwd: string;
  /** Current repo paths relative to cwd (from snapshots). */
  repoPaths: readonly string[];
  busyRef: MutableRefObject<boolean>;
  /**
   * Called after a batch finishes so sync marks refresh and status messages
   * can be set. `manual` is true when the user pressed `f`.
   */
  onFetched: (
    repos: readonly string[],
    result: { ok: number; failed: number },
    meta: { manual: boolean },
  ) => void | Promise<void>;
  /**
   * Called when a manual (`f`) fetch is refused because `busyRef` is set.
   * Background timer ticks stay silent.
   */
  onBusy?: () => void;
  /** Injectable clock for tests; defaults to Date.now. */
  now?: () => number;
  /** Override env for tests. */
  env?: NodeJS.ProcessEnv;
}

/**
 * Options for a single {@link FetchApi.runFetch} invocation.
 */
export interface RunFetchOptions {
  /** True when triggered by the `f` key (status bar says `Fetched` on success). */
  manual?: boolean;
}

/**
 * Public surface of the fetch layer.
 */
export interface FetchApi {
  /** Short status fragment for top-chrome op status (age or progress). */
  fetchStatusLine: string;
  /** Run fetch for the given repos immediately (manual `f` or the timer). */
  runFetch: (repos: readonly string[], opts?: RunFetchOptions) => void;
  lastFetchedAt: number | null;
}

/**
 * Own the background-fetch timer and the shared fetch runner used by `f`.
 */
export function useFetch(deps: FetchDeps): FetchApi {
  const {
    cwd,
    repoPaths,
    busyRef,
    onFetched,
    onBusy,
    now = Date.now,
    env = process.env,
  } = deps;
  const [lastFetchedAt, setLastFetchedAt] = useState<number | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(
    null,
  );
  const [tick, setTick] = useState(0);
  const reposRef = useRef(repoPaths);
  reposRef.current = repoPaths;

  const runFetch = useCallback(
    (repos: readonly string[], opts: RunFetchOptions = {}) => {
      if (repos.length === 0) return;
      if (busyRef.current) {
        if (opts.manual === true) onBusy?.();
        return;
      }
      busyRef.current = true;
      const manual = opts.manual === true;
      setProgress({ done: 0, total: repos.length });
      void (async () => {
        try {
          const result = await fetchRepos(cwd, [...repos], {
            onProgress: (done, total) => setProgress({ done, total }),
          });
          setLastFetchedAt(now());
          await onFetched(repos, result, { manual });
        } finally {
          busyRef.current = false;
          setProgress(null);
        }
      })();
    },
    [busyRef, cwd, now, onBusy, onFetched],
  );

  useEffect(() => {
    const ms = fetchIntervalMs(env);
    if (ms === 0) return;
    const id = setInterval(() => {
      runFetch(reposRef.current);
      setTick((t) => t + 1);
    }, ms);
    return () => clearInterval(id);
  }, [env, runFetch]);

  // Recompute age text periodically so "just now" → "1m ago" without a fetch.
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 15_000);
    return () => clearInterval(id);
  }, []);

  void tick;
  const fetchStatusLine = progress
    ? formatFetchProgress(progress.done, progress.total)
    : formatFetchAge(lastFetchedAt, now());

  return { fetchStatusLine, runFetch, lastFetchedAt };
}
