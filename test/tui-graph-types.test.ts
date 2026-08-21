import assert from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';
import { DEFAULT_GRAPH_WINDOW, type GraphCommit, type GraphModel } from '../src/tui/graph/types.js';
import { DEFAULT_LANE_COLORS } from '../src/tui/graph/laneColors.js';

const FIX = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/graph');

function loadCommits(name: string): GraphCommit[] {
  return JSON.parse(readFileSync(join(FIX, name), 'utf8')) as GraphCommit[];
}

describe('graph types', () => {
  it('DEFAULT_GRAPH_WINDOW is 300', () => {
    assert.equal(DEFAULT_GRAPH_WINDOW, 300);
  });

  it('DEFAULT_LANE_COLORS has at least 6 distinct hex colours', () => {
    assert.ok(DEFAULT_LANE_COLORS.length >= 6);
    assert.equal(new Set(DEFAULT_LANE_COLORS).size, DEFAULT_LANE_COLORS.length);
    for (const c of DEFAULT_LANE_COLORS) {
      assert.match(c, /^#[0-9a-fA-F]{6}$/);
    }
  });

  it('linear-three fixture has parent chain tip→root', () => {
    const commits = loadCommits('linear-three.json');
    assert.equal(commits.length, 3);
    assert.deepEqual(commits[0].parents, [commits[1].id]);
    assert.deepEqual(commits[1].parents, [commits[2].id]);
    assert.deepEqual(commits[2].parents, []);
  });

  it('merge-diamond fixture has a merge with two parents', () => {
    const commits = loadCommits('merge-diamond.json');
    const merge = commits.find((c) => c.parents.length === 2);
    assert.ok(merge, 'expected a merge commit');
    assert.equal(merge.parents.length, 2);
  });

  it('GraphModel shape accepts fixture commits', () => {
    const commits = loadCommits('linear-three.json');
    const model: GraphModel = {
      repoPath: '/tmp/repo',
      commits,
      stashes: [],
      uncommitted: { kind: 'uncommitted', hasChanges: false },
      headId: commits[0]?.id ?? null,
      refsFingerprint: 'fp',
      skip: 0,
      limit: DEFAULT_GRAPH_WINDOW,
      hasMore: false,
    };
    assert.equal(model.commits.length, 3);
  });
});
