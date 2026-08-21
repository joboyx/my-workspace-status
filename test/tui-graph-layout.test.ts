import assert from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';
import { layoutCommits } from '../src/tui/graph/layout.js';
import type { GraphCommit } from '../src/tui/graph/types.js';

const FIX = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/graph');

function load(name: string): GraphCommit[] {
  return JSON.parse(readFileSync(join(FIX, name), 'utf8')) as GraphCommit[];
}

describe('layoutCommits', () => {
  it('linear history stays on one lane', () => {
    const rows = layoutCommits(load('linear-three.json'));
    assert.equal(rows.length, 3);
    for (const r of rows) {
      assert.equal(r.lane, 0);
      assert.equal(r.laneCount, 1);
      assert.ok(r.edges.includes('●') || r.edges.includes('*'));
      assert.equal(r.cells.filter((c) => c.role === 'node').length, 1);
    }
    assert.deepEqual(
      rows.map((r) => r.edges.trimEnd()),
      ['●', '●', '●'],
    );
  });

  it('merge-diamond opens a second lane with connectors then collapses', () => {
    const rows = layoutCommits(load('merge-diamond.json'));
    assert.equal(rows.length, 4);
    const laneCounts = rows.map((r) => r.laneCount);
    assert.ok(Math.max(...laneCounts) >= 2, `expected ≥2 lanes, got ${laneCounts}`);

    // Side tip stays a normal `│ ●`; join elbow lands on the shared parent.
    const expected = [
      { id: 'm999', lane: 0, edges: '●─╮ ' },
      { id: 'a111', lane: 0, edges: '● │ ' },
      { id: 'b222', lane: 1, edges: '│ ● ' },
      { id: 'r000', lane: 0, edges: '●─╯ ' },
    ];
    assert.deepEqual(
      rows.map((r) => ({ id: r.commit.id, lane: r.lane, edges: r.edges })),
      expected,
    );

    assert.ok(rows[0]!.cells.some((c) => c.ch === '─' || c.ch === '-'));
    assert.ok(rows[3]!.cells.some((c) => c.ch === '─' || c.ch === '-'));
    assert.ok(rows.every((r) => r.cells.length === rows[0]!.cells.length));
    // Parent join row collapses incoming waiters; live tip stays lane 0.
    assert.equal(rows[3]!.lane, 0);
  });

  it('shared distant parent keeps sibling tip lanes then joins at ancestor', () => {
    const rows = layoutCommits(load('shared-distant-parent.json'));
    assert.equal(rows.length, 5);

    const tips = rows.filter((r) => r.commit.id.startsWith('tip'));
    const tipLanes = new Set(tips.map((r) => r.lane));
    assert.equal(
      tipLanes.size,
      tips.length,
      `sibling tips must use distinct lanes: ${tips.map((r) => `${r.commit.id}:${r.lane}`)}`,
    );

    const ancestor = rows.find((r) => r.commit.id === 'ancestor')!;
    assert.equal(ancestor.lane, 0);
    assert.ok(
      ancestor.edges.includes('╯') ||
        ancestor.edges.includes('┴') ||
        ancestor.edges.includes('/') ||
        ancestor.edges.includes('+'),
      `ancestor should show join, got ${JSON.stringify(ancestor.edges)}`,
    );
  });

  it('synthetic long chain collapses after merge-back', () => {
    // Newest-first: merge, main tip, side tip, shared base, older main…
    const commits: GraphCommit[] = [
      {
        id: 'merge',
        parents: ['main2', 'side1'],
        subject: 'merge side',
        authorName: 'Ada',
        authorDateUnix: 100,
        refs: [],
      },
      {
        id: 'main2',
        parents: ['base'],
        subject: 'main after fork',
        authorName: 'Ada',
        authorDateUnix: 90,
        refs: [],
      },
      {
        id: 'side1',
        parents: ['base'],
        subject: 'side',
        authorName: 'Ada',
        authorDateUnix: 80,
        refs: [],
      },
      {
        id: 'base',
        parents: ['old'],
        subject: 'fork point',
        authorName: 'Ada',
        authorDateUnix: 70,
        refs: [],
      },
      {
        id: 'old',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 60,
        refs: [],
      },
    ];
    const rows = layoutCommits(commits);
    const maxLanes = Math.max(...rows.map((r) => r.laneCount));
    assert.ok(maxLanes <= 2, `expected ≤2 lanes through merge-back, got ${maxLanes}`);
    // After side closes into base, remaining history is one lane on the left.
    const baseIdx = rows.findIndex((r) => r.commit.id === 'base');
    const after = rows.slice(baseIdx);
    assert.ok(
      after.every((r) => r.lane === 0),
      `expected lane 0 after merge-back, got ${after.map((r) => `${r.commit.id}:lane=${r.lane}/lc=${r.laneCount}`)}`,
    );
    // Rows after the join parent are single-lane (no ghost column).
    assert.ok(
      after.slice(1).every((r) => r.laneCount === 1),
      `expected lc=1 after join parent, got ${after.map((r) => `${r.commit.id}:${r.laneCount}`)}`,
    );
  });

  it('compacts left after join so history does not stay on lane 1', () => {
    // Mirrors the real dotfiles bug: merge → side chain → close → must return to lane 0.
    const commits: GraphCommit[] = [
      {
        id: 'merge',
        parents: ['mainKeep', 'sideTip'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'sideTip',
        parents: ['mainKeep'],
        subject: 'side',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'mainKeep',
        parents: ['older'],
        subject: 'main continues',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'older',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
    ];
    const rows = layoutCommits(commits);
    const edges = rows.map((r) => ({
      id: r.commit.id,
      lane: r.lane,
      edges: r.edges.trimEnd(),
    }));
    assert.deepEqual(edges, [
      { id: 'merge', lane: 0, edges: '●─╮' },
      { id: 'sideTip', lane: 1, edges: '│ ●' },
      { id: 'mainKeep', lane: 0, edges: '●─╯' },
      { id: 'older', lane: 0, edges: '●' },
    ]);
  });

  it('sibling tips sharing a parent keep distinct lanes', () => {
    // Mirrors DEMO-844-pages / DEMO-844-sevice both → c86662d4 while
    // another lane already waits on that parent.
    const commits: GraphCommit[] = [
      {
        id: 'mainTip',
        parents: ['base'],
        subject: 'main tip',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'tipA',
        parents: ['base'],
        subject: 'tip A',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'tipB',
        parents: ['base'],
        subject: 'tip B',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'base',
        parents: [],
        subject: 'shared parent',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const rows = layoutCommits(commits);
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]));
    assert.equal(byId.mainTip!.lane, 0);
    assert.equal(byId.tipA!.lane, 1);
    assert.equal(
      byId.tipB!.lane,
      2,
      `tipB must not reuse tipA lane; edges=${JSON.stringify(rows.map((r) => r.edges.trimEnd()))}`,
    );
    assert.notEqual(byId.tipA!.lane, byId.tipB!.lane);
    // Parent joins side lanes; live tip stays lane 0 after densify.
    assert.equal(byId.base!.lane, 0);
    assert.ok(
      byId.base!.edges.includes('╯') || byId.base!.edges.includes('/'),
      `base should show join elbow, got ${JSON.stringify(byId.base!.edges)}`,
    );
  });

  it('continuing side rail through a merge connector uses ┤ not dangling ╮', () => {
    // Newer tip keeps featBase live on lane 0; merge below links to that rail.
    const commits: GraphCommit[] = [
      {
        id: 'featTip',
        parents: ['featBase'],
        subject: 'feature tip',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'merge',
        parents: ['main', 'featBase'],
        subject: 'merge feature',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'main',
        parents: ['featBase'],
        subject: 'main',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'featBase',
        parents: [],
        subject: 'base',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const rows = layoutCommits(commits);
    const merge = rows.find((r) => r.commit.id === 'merge')!;
    assert.ok(
      /[├┤+]/.test(merge.edges),
      `expected tee into continuing rail, got ${JSON.stringify(merge.edges)}`,
    );
    assert.ok(
      !merge.edges.includes('╮'),
      `open-corner on a live rail looks dangling: ${JSON.stringify(merge.edges)}`,
    );
  });

  it('multi-lane tee/cross composes ┼ when a horizontal crosses a through-rail', () => {
    const commits: GraphCommit[] = [
      {
        id: 'after',
        parents: ['merge'],
        subject: 'after merge',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'sideLive',
        parents: ['sideBase'],
        subject: 'live side tip',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'merge',
        parents: ['main', 'feat'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'main',
        parents: ['root'],
        subject: 'main',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'feat',
        parents: ['root'],
        subject: 'feat',
        authorName: 'Ada',
        authorDateUnix: 15,
        refs: [],
      },
      {
        id: 'sideBase',
        parents: ['root'],
        subject: 'side base',
        authorName: 'Ada',
        authorDateUnix: 12,
        refs: [],
      },
      {
        id: 'root',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const rows = layoutCommits(commits);
    const merge = rows.find((r) => r.commit.id === 'merge')!;
    assert.ok(
      merge.edges.includes('┼') || merge.edges.includes('+'),
      `expected cross through live rail, got ${JSON.stringify(merge.edges)}`,
    );
  });

  it('genuine newly opened lane still uses a corner rather than a through-tee', () => {
    const commits: GraphCommit[] = [
      {
        id: 'merge',
        parents: ['main', 'side'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'main',
        parents: ['base'],
        subject: 'main',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'side',
        parents: ['base'],
        subject: 'side',
        authorName: 'Ada',
        authorDateUnix: 15,
        refs: [],
      },
      {
        id: 'base',
        parents: [],
        subject: 'base',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const rows = layoutCommits(commits);
    const merge = rows.find((r) => r.commit.id === 'merge')!;
    assert.ok(
      merge.edges.includes('╮') || merge.edges.includes('\\'),
      `new open should be a corner, got ${JSON.stringify(merge.edges)}`,
    );
    assert.ok(
      !/[├┤]/.test(merge.edges),
      `new open must not upgrade to tee: ${JSON.stringify(merge.edges)}`,
    );
  });

  it('locks node glyphs: commit/merge ●, distinct from uncommitted ○', () => {
    const commits: GraphCommit[] = [
      {
        id: 'merge',
        parents: ['a', 'b'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 3,
        refs: [],
      },
      {
        id: 'a',
        parents: [],
        subject: 'a',
        authorName: 'Ada',
        authorDateUnix: 2,
        refs: [],
      },
      {
        id: 'b',
        parents: [],
        subject: 'b',
        authorName: 'Ada',
        authorDateUnix: 1,
        refs: [],
      },
    ];
    const rows = layoutCommits(commits);
    for (const r of rows) {
      const node = r.cells.find((c) => c.role === 'node');
      assert.ok(node);
      assert.equal(node!.ch, '●');
    }
    assert.notEqual(rows[0]!.cells.find((c) => c.role === 'node')!.ch, '◎');
  });

  it('ASCII junctions derive from the same topology model', () => {
    const commits: GraphCommit[] = [
      {
        id: 'after',
        parents: ['merge'],
        subject: 'after merge',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'sideLive',
        parents: ['sideBase'],
        subject: 'live side tip',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'merge',
        parents: ['main', 'feat'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'main',
        parents: ['root'],
        subject: 'main',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'feat',
        parents: ['root'],
        subject: 'feat',
        authorName: 'Ada',
        authorDateUnix: 15,
        refs: [],
      },
      {
        id: 'sideBase',
        parents: ['root'],
        subject: 'side base',
        authorName: 'Ada',
        authorDateUnix: 12,
        refs: [],
      },
      {
        id: 'root',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const uni = layoutCommits(commits);
    const asc = layoutCommits(commits, { ascii: true });
    const uniMerge = uni.find((r) => r.commit.id === 'merge')!;
    const ascMerge = asc.find((r) => r.commit.id === 'merge')!;
    assert.ok(uniMerge.edges.includes('┼'), uniMerge.edges);
    assert.ok(
      ascMerge.edges.includes('+'),
      `ASCII should map the same cross topology to +, got ${JSON.stringify(ascMerge.edges)}`,
    );
    assert.ok(ascMerge.edges.includes('*'), ascMerge.edges);
    assert.equal(uniMerge.lane, ascMerge.lane);
  });
});
