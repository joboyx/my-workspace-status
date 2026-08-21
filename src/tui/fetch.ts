/**
 * Background and manual fetch scheduling helpers for the TUI.
 *
 * Modelled on `watch.ts`: env parsing is pure; the actual git work is a
 * bounded batch so the keymap stays responsive.
 */

import * as path from 'node:path';
import { mapWithConcurrency } from '../concurrency.js';
import { execGitAsync } from '../git.js';

/** Default background-fetch period (5 minutes). */
export const DEFAULT_FETCH_MS = 300_000;

/** Floor when fetch is enabled — faster burns the network for little gain. */
export const MIN_FETCH_MS = 30_000;

/** How many repos fetch at once. */
export const FETCH_CONCURRENCY = 4;

/**
 * Poll period from `WS_STATUS_FETCH_MS`.
 * Returns 0 when disabled. Defaults to {@link DEFAULT_FETCH_MS}.
 */
export function fetchIntervalMs(env: NodeJS.ProcessEnv = process.env): number {
  const raw = env.WS_STATUS_FETCH_MS;
  if (raw === undefined || raw === '') return DEFAULT_FETCH_MS;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed) || parsed < 0) return DEFAULT_FETCH_MS;
  if (parsed === 0) return 0;
  return Math.max(MIN_FETCH_MS, parsed);
}

/**
 * Human-readable age for top-chrome op status, or empty when never fetched.
 */
export function formatFetchAge(
  lastFetchedAt: number | null,
  now: number,
): string {
  if (lastFetchedAt === null) return '';
  const elapsed = Math.max(0, now - lastFetchedAt);
  if (elapsed < 30_000) return 'fetched just now';
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 1) return 'fetched just now';
  return `fetched ${minutes}m ago`;
}

/**
 * In-flight progress line while a batch is running (top-chrome op status).
 */
export function formatFetchProgress(done: number, total: number): string {
  return `Fetching ${done}/${total}…`;
}

export type FetchReposResult = {
  ok: number;
  failed: number;
};

/**
 * Options for {@link fetchRepos}.
 */
export type FetchReposOptions = {
  /** Max concurrent `git fetch` processes. Defaults to {@link FETCH_CONCURRENCY}. */
  concurrency?: number;
  /** Called after each repo settles with settled count and batch size. */
  onProgress?: (done: number, total: number) => void;
};

/**
 * `git fetch --quiet` each repo under `cwd`. Failures are counted, not thrown.
 */
export async function fetchRepos(
  cwd: string,
  repos: readonly string[],
  opts: FetchReposOptions = {},
): Promise<FetchReposResult> {
  const concurrency = opts.concurrency ?? FETCH_CONCURRENCY;
  const total = repos.length;
  let ok = 0;
  let failed = 0;
  let done = 0;
  await mapWithConcurrency([...repos], concurrency, async (repo) => {
    try {
      await execGitAsync(['fetch', '--quiet'], path.join(cwd, repo));
      ok += 1;
    } catch {
      failed += 1;
    }
    done += 1;
    opts.onProgress?.(done, total);
  });
  return { ok, failed };
}
