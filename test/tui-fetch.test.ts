import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  DEFAULT_FETCH_MS,
  MIN_FETCH_MS,
  fetchIntervalMs,
  formatFetchAge,
  formatFetchProgress,
} from '../src/tui/fetch.js';

describe('fetchIntervalMs', () => {
  it('defaults to 5 minutes when unset', () => {
    assert.equal(fetchIntervalMs({}), DEFAULT_FETCH_MS);
    assert.equal(fetchIntervalMs({ WS_STATUS_FETCH_MS: '' }), DEFAULT_FETCH_MS);
    assert.equal(fetchIntervalMs({ WS_STATUS_FETCH_MS: 'nope' }), DEFAULT_FETCH_MS);
  });

  it('disables on 0 and clamps small positives', () => {
    assert.equal(fetchIntervalMs({ WS_STATUS_FETCH_MS: '0' }), 0);
    assert.equal(fetchIntervalMs({ WS_STATUS_FETCH_MS: '1000' }), MIN_FETCH_MS);
    assert.equal(fetchIntervalMs({ WS_STATUS_FETCH_MS: '600000' }), 600000);
  });
});

describe('formatFetchAge / progress', () => {
  it('formats age buckets', () => {
    assert.equal(formatFetchAge(null, 1000), '');
    assert.equal(formatFetchAge(1000, 1000), 'fetched just now');
    assert.equal(formatFetchAge(1000, 1000 + 60_000), 'fetched 1m ago');
    assert.equal(formatFetchAge(1000, 1000 + 4 * 60_000), 'fetched 4m ago');
  });

  it('formats in-flight progress', () => {
    assert.equal(formatFetchProgress(2, 18), 'Fetching 2/18…');
  });
});

describe('fetchRepos onProgress', () => {
  it('reports settled counts after each repo (mocked)', async () => {
    const { fetchRepos } = await import('../src/tui/fetch.js');
    const progress: Array<[number, number]> = [];
    // Nonexistent repos → every fetch fails; still settle + progress.
    const result = await fetchRepos('/tmp', ['a', 'b', 'c'], {
      concurrency: 2,
      onProgress: (done, total) => progress.push([done, total]),
    });
    assert.equal(result.ok, 0);
    assert.equal(result.failed, 3);
    assert.equal(progress.length, 3);
    assert.deepEqual(
      progress.map(([, total]) => total),
      [3, 3, 3],
    );
    assert.deepEqual(
      progress.map(([done]) => done).sort((a, b) => a - b),
      [1, 2, 3],
    );
  });
});
