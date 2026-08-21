import assert from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';
import {
  activeRepoPath,
  applySelectableGraphPageMove,
  buildGraphListRows,
  ensureLaidOut,
  firstSelectableGraphIndex,
  graphCursorAfterRowsReload,
  graphLayoutCommits,
  graphRowId,
  graphStashSpacerId,
  isGraphRowPairHighlighted,
  isSelectableGraphRow,
  lastSelectableGraphIndex,
  nearestSelectableGraphIndex,
  selectableGraphIndexFromClick,
  shouldShowFileDiff,
  shouldShowGraphDetail,
  stepSelectableGraphCursor,
  type GraphListRow,
} from '../src/tui/graph/list.js';
import { CELL_W } from '../src/tui/graph/glyphs.js';
import { layoutCommits } from '../src/tui/graph/layout.js';
import type {
  GraphCommit,
  GraphModel,
  GraphStash,
  GraphUncommitted,
  LaidOutCommit,
} from '../src/tui/graph/types.js';
import { DEFAULT_GRAPH_WINDOW } from '../src/tui/graph/types.js';
import type { NavState } from '../src/tui/nav/stack.js';
import type { VisibleRow } from '../src/tui/model/types.js';
import { segmentsText } from '../src/tui/theme.js';

const FIX = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/graph');

/** Lane → gutter column (topology uses CELL_W-wide cells). */
function laneCol(lane: number): number {
  return lane * CELL_W;
}

function gutterOf(row: GraphListRow, gw: number): string {
  return segmentsText(row.segments).slice(0, gw);
}

/** Short gutter dump for stash/park regressions (not one-off glyph asserts). */
function gutterDump(rows: GraphListRow[], gw: number): string {
  return rows
    .map((row) => {
      const g = gutterOf(row, gw).replace(/\s+$/, '');
      return `${row.kind.padEnd(11)} ${g.length > 0 ? g : '·'}`;
    })
    .join('\n');
}

/** Column of stash diamond (`◇` / ASCII `s` as a gutter node), or -1. */
function diamondCol(gutter: string): number {
  const chars = [...gutter];
  return chars.findIndex((ch) => ch === '◇' || ch === 's');
}

/**
 * True when the gutter slice contains a stash tip glyph.
 * Unicode tests use `◇`. Avoid matching the leading `s` of `stash@{n}` when the
 * helper window is wider than the painted rail budget (truncated `s` false positive).
 */
function hasStashNodeGlyph(gutter: string): boolean {
  return gutter.includes('◇');
}

/** Close-elbow family joining a 1-node side leaf onto a commit/parent row. */
function hasCloseElbow(gutter: string): boolean {
  return /[●⊙*@o][─\-┼+]*.*[╯)\\/]/.test(gutter.trimEnd());
}

function graphWidthFor(laidOut: LaidOutCommit[], extraLanes = 1): number {
  const maxLane = laidOut.reduce((m, r) => Math.max(m, r.lane), 0);
  return Math.max(
    laidOut.reduce((m, r) => Math.max(m, r.cells.length), 0),
    laneCol(maxLane + extraLanes) + 1,
  );
}

function loadCommits(name: string): GraphCommit[] {
  return JSON.parse(readFileSync(join(FIX, name), 'utf8')) as GraphCommit[];
}

function stashOf(
  partial: Pick<GraphStash, 'id' | 'stashRef' | 'authorDateUnix'> & Partial<GraphStash>,
): GraphStash {
  return {
    index: 0,
    subject: 'wip',
    authorName: 'Ada',
    parentId: '',
    ...partial,
  };
}

function modelOf(
  commits: GraphCommit[],
  opts: {
    stashes?: GraphStash[];
    uncommitted?: GraphUncommitted | null;
  } = {},
): GraphModel {
  return {
    repoPath: '/ws/demo',
    commits,
    stashes: opts.stashes ?? [
      stashOf({
        id: 's1',
        stashRef: 'stash@{0}',
        authorDateUnix: 1700000099,
      }),
    ],
    uncommitted:
      opts.uncommitted === undefined ? { kind: 'uncommitted', hasChanges: true } : opts.uncommitted,
    headId: commits[0]?.id ?? null,
    refsFingerprint: 'fp',
    skip: 0,
    limit: DEFAULT_GRAPH_WINDOW,
    hasMore: false,
  };
}

/** Row kinds (+ stashRef/commitId) for assert-friendly order checks. */
function rowOrder(rows: ReturnType<typeof buildGraphListRows>): string[] {
  return rows.map((r) => {
    if (r.kind === 'uncommitted') return 'uncommitted';
    if (r.kind === 'stash') return `stash:${r.stashRef}`;
    if (r.kind === 'spacer') return 'spacer';
    return `commit:${r.commitId}`;
  });
}

function repoRow(): VisibleRow {
  return {
    id: 'repo:demo',
    depth: 1,
    label: 'demo',
    segments: [],
    trailing: [],
    node: {
      kind: 'repo',
      id: 'repo:demo',
      path: '/ws/demo',
      branch: 'main',
      checkoutKind: 'primary',
      mergedIntoDefault: null,
      sync: '',
      syncStatus: 'up-to-date',
      ignored: false,
      changeCount: 0,
      children: [],
    },
  };
}

function fileRow(): VisibleRow {
  return {
    id: 'file:demo:a.ts',
    depth: 2,
    label: 'a.ts',
    segments: [],
    trailing: [],
    node: {
      kind: 'file',
      id: 'file:demo:a.ts',
      repoPath: '/ws/demo',
      path: 'a.ts',
      status: 'M',
      staged: false,
      unstaged: true,
      untracked: false,
      change: { path: 'a.ts', unstagedStatus: 'M' },
    },
  };
}

describe('graphRowId', () => {
  it('builds stable ids', () => {
    assert.equal(graphRowId('commit', 'aaa'), 'graph:commit:aaa');
    assert.equal(graphRowId('stash', 'stash@{0}'), 'graph:stash:stash@{0}');
    assert.equal(graphRowId('uncommitted', 'wt'), 'graph:uncommitted');
    assert.equal(graphRowId('spacer', 'aaa'), 'graph:spacer:aaa');
    assert.equal(graphStashSpacerId('stash@{0}'), 'graph:spacer:stash:stash@{0}');
  });
});

describe('isGraphRowPairHighlighted', () => {
  it('highlights commit+spacer and stash+spacer pairs', () => {
    const rows: GraphListRow[] = [
      {
        id: 'graph:commit:aaa',
        kind: 'commit',
        commitId: 'aaa',
        segments: [],
      },
      {
        id: 'graph:spacer:aaa',
        kind: 'spacer',
        commitId: null,
        segments: [],
      },
      {
        id: 'graph:stash:stash@{0}',
        kind: 'stash',
        commitId: 's1',
        stashRef: 'stash@{0}',
        segments: [],
      },
      {
        id: graphStashSpacerId('stash@{0}'),
        kind: 'spacer',
        commitId: null,
        stashRef: 'stash@{0}',
        segments: [],
      },
    ];
    assert.equal(isGraphRowPairHighlighted(rows, 0, 0), true);
    assert.equal(isGraphRowPairHighlighted(rows, 0, 1), true);
    assert.equal(isGraphRowPairHighlighted(rows, 0, 2), false);
    assert.equal(isGraphRowPairHighlighted(rows, 2, 2), true);
    assert.equal(isGraphRowPairHighlighted(rows, 2, 3), true);
    assert.equal(isGraphRowPairHighlighted(rows, 2, 1), false);
  });
});

describe('buildGraphListRows', () => {
  it('pins uncommitted first; orphan stashes (no parent in window) sit next', () => {
    const commits = loadCommits('linear-three.json');
    const model = modelOf(commits, {
      stashes: [
        stashOf({
          id: 's-orphan',
          stashRef: 'stash@{0}',
          authorDateUnix: 1700000001.5,
        }),
      ],
    });
    const laidOut = layoutCommits(commits);
    const rows = buildGraphListRows(model, laidOut, { width: 80 });
    assert.deepEqual(rowOrder(rows), [
      'uncommitted',
      'stash:stash@{0}',
      'spacer',
      'commit:aaa111',
      'spacer',
      'commit:bbb222',
      'spacer',
      'commit:ccc333',
      'spacer',
    ]);
    assert.ok(rows[1]!.segments.length > 0);
  });

  it('parks each stash immediately above its stash^1 parent', () => {
    const commits = loadCommits('linear-three.json');
    const model = modelOf(commits, {
      stashes: [
        stashOf({
          id: 's-mid',
          stashRef: 'stash@{0}',
          authorDateUnix: 1700000099, // newer than tip — still parks on parent
          parentId: 'bbb222',
        }),
      ],
    });
    const rows = buildGraphListRows(model, layoutCommits(commits), {
      width: 80,
    });
    assert.deepEqual(rowOrder(rows), [
      'uncommitted',
      'commit:aaa111',
      'spacer',
      'stash:stash@{0}',
      'spacer',
      'commit:bbb222',
      'spacer',
      'commit:ccc333',
      'spacer',
    ]);
  });

  it('stacks multiple stashes on the same parent newest-first', () => {
    const commits = loadCommits('linear-three.json');
    const model = modelOf(commits, {
      stashes: [
        stashOf({
          id: 's0',
          stashRef: 'stash@{0}',
          index: 0,
          authorDateUnix: 1700000099,
          parentId: 'aaa111',
        }),
        stashOf({
          id: 's1',
          stashRef: 'stash@{1}',
          index: 1,
          authorDateUnix: 1700000090,
          parentId: 'aaa111',
        }),
      ],
    });
    const rows = buildGraphListRows(model, layoutCommits(commits), {
      width: 80,
    });
    assert.deepEqual(rowOrder(rows), [
      'uncommitted',
      'stash:stash@{0}',
      'spacer',
      'stash:stash@{1}',
      'spacer',
      'commit:aaa111',
      'spacer',
      'commit:bbb222',
      'spacer',
      'commit:ccc333',
      'spacer',
    ]);
  });

  it('keeps commit-only order when stashes are empty', () => {
    const commits = loadCommits('linear-three.json');
    const model = modelOf(commits, { stashes: [] });
    const rows = buildGraphListRows(model, layoutCommits(commits), {
      width: 80,
    });
    assert.deepEqual(rowOrder(rows), [
      'uncommitted',
      'commit:aaa111',
      'spacer',
      'commit:bbb222',
      'spacer',
      'commit:ccc333',
      'spacer',
    ]);
  });

  it('omits uncommitted when absent; still parks stash on parent', () => {
    const commits = loadCommits('linear-three.json');
    const model = modelOf(commits, {
      uncommitted: null,
      stashes: [
        stashOf({
          id: 's-mid',
          stashRef: 'stash@{0}',
          authorDateUnix: 1700000001.5,
          parentId: 'bbb222',
        }),
      ],
    });
    const rows = buildGraphListRows(model, layoutCommits(commits), {
      width: 80,
    });
    assert.deepEqual(rowOrder(rows), [
      'commit:aaa111',
      'spacer',
      'stash:stash@{0}',
      'spacer',
      'commit:bbb222',
      'spacer',
      'commit:ccc333',
      'spacer',
    ]);
  });

  it('ignores stash authorDate when parking on parent (no chrono gap)', () => {
    // notes-shaped: stash date sits between tip and mid, but parent is root —
    // chrono interleave would leave a dangling spur; park keeps tip 1 node away.
    const commits = loadCommits('linear-three.json');
    const model = modelOf(commits, {
      uncommitted: null,
      stashes: [
        stashOf({
          id: 's-far',
          stashRef: 'stash@{0}',
          authorDateUnix: 1700000001.5,
          parentId: 'ccc333',
        }),
      ],
    });
    const rows = buildGraphListRows(model, layoutCommits(commits), {
      width: 80,
    });
    assert.deepEqual(rowOrder(rows), [
      'commit:aaa111',
      'spacer',
      'commit:bbb222',
      'spacer',
      'stash:stash@{0}',
      'spacer',
      'commit:ccc333',
      'spacer',
    ]);
    const order = rowOrder(rows);
    assert.equal(order.indexOf('stash:stash@{0}') + 2, order.indexOf('commit:ccc333'));
  });

  it('caps shared graphWidth from pane size (not hardcoded 80)', () => {
    const commits = loadCommits('merge-diamond.json');
    const laid = layoutCommits(commits);
    const topo = laid.reduce((m, r) => Math.max(m, r.cells.length), 0);
    assert.ok(topo >= 2);
    const narrow = buildGraphListRows(modelOf(commits), laid, { width: 40 });
    const wide = buildGraphListRows(modelOf(commits), laid, { width: 160 });
    const narrowText = segmentsText(narrow.find((r) => r.kind === 'commit')!.segments);
    const wideText = segmentsText(wide.find((r) => r.kind === 'commit')!.segments);
    // Wide pane paints a longer row (subject/meta flex into leftover space).
    assert.ok(
      wideText.length > narrowText.length,
      `wide=${wideText.length} narrow=${narrowText.length}`,
    );
    assert.ok(wideText.length > 80, `expected >80 on wide pane, got ${wideText.length}`);
  });

  it('preserves laid-out commit edges when interleaved with stashes', () => {
    const commits = loadCommits('linear-three.json');
    const laidOut = layoutCommits(commits);
    const model = modelOf(commits, {
      stashes: [
        stashOf({
          id: 's-mid',
          stashRef: 'stash@{0}',
          authorDateUnix: 1700000001.5,
        }),
      ],
    });
    const rows = buildGraphListRows(model, laidOut, { width: 80 });
    const tip = rows.find((r) => r.commitId === 'aaa111');
    assert.ok(tip);
    // layoutCommits paints ● (or ⊙/@ when that commit is HEAD via model.headId)
    assert.ok(
      tip!.segments.some((s) => /[●⊙*@]/.test(String(s.text))),
      `expected a node glyph in ${segmentsText(tip!.segments)}`,
    );
  });

  it('stash rows are 3b side leaf tips (◇ free lane; join on stash^1)', () => {
    const commits = loadCommits('linear-three.json');
    const laidOut = layoutCommits(commits);
    const parent = laidOut.find((r) => r.commit.id === 'bbb222')!;
    const model = modelOf(commits, {
      uncommitted: null,
      stashes: [
        stashOf({
          id: 's-mid',
          stashRef: 'stash@{0}',
          authorDateUnix: 1700000001.5,
          parentId: parent.commit.id,
        }),
      ],
    });
    const rows = buildGraphListRows(model, laidOut, { width: 80 });
    const stash = rows.find((r) => r.kind === 'stash');
    assert.ok(stash);
    const text = segmentsText(stash!.segments);
    const gw = graphWidthFor(laidOut);
    const gutter = gutterOf(stash!, gw);
    const parentCol = laneCol(parent.lane);
    const dCol = diamondCol(gutter);
    assert.ok(dCol >= 0, `expected ◇ in gutter, got ${JSON.stringify(gutter)}`);
    assert.notEqual(dCol, parentCol, '◇ must not sit on stash^1 lane');
    // 3b grammar: through-rail on parent lane at the stash tip — not a mid-rail ├─◇ tee.
    assert.match(
      gutter[parentCol] ?? '',
      /[│|]/,
      `expected through-rail on stash^1 lane, got ${JSON.stringify(gutter)}`,
    );
    assert.doesNotMatch(
      gutter[parentCol] ?? '',
      /[├|+]/,
      `stash tip must not tee on parent lane (├─◇), got ${JSON.stringify(gutter)}`,
    );
    assert.doesNotMatch(
      text.slice(gw),
      /^◇|^s/,
      `subject must not start with diamond, got ${JSON.stringify(text)}`,
    );
    assert.match(text, /wip/);

    const parentRow = rows.find((r) => r.commitId === parent.commit.id)!;
    const parentGutter = gutterOf(parentRow, gw);
    assert.ok(
      hasCloseElbow(parentGutter),
      `expected close elbow on stash^1 row, got ${JSON.stringify(parentGutter)}`,
    );

    const spacer = rows.find((r) => r.id === graphStashSpacerId('stash@{0}'))!;
    const spacerGutter = gutterOf(spacer, gw);
    assert.ok(
      !hasStashNodeGlyph(spacerGutter),
      `spacer must not paint a second stash node, got ${JSON.stringify(spacerGutter)}`,
    );
  });

  it('commit spacer densifies across a parked stash leaf', () => {
    // tipA/tipB join at base while outerTip→outerBase continues on an outer lane;
    // densify remaps that outer rail left. Stash parked on outerBase sits between
    // base and outerBase as leaf chrome — densify stays on the commit spacer.
    const commits: GraphCommit[] = [
      {
        id: 'tipA',
        parents: ['base'],
        subject: 'tip A',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'tipB',
        parents: ['base'],
        subject: 'tip B',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'outerTip',
        parents: ['outerBase'],
        subject: 'outer tip',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'base',
        parents: ['older'],
        subject: 'join',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'outerBase',
        parents: ['older'],
        subject: 'outer base',
        authorName: 'Ada',
        authorDateUnix: 15,
        refs: [],
      },
      {
        id: 'older',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const base = laidOut.find((r) => r.commit.id === 'base')!;
    const outerBase = laidOut.find((r) => r.commit.id === 'outerBase')!;
    assert.equal(base.edges.trimEnd(), '●─╯ │');
    assert.equal(outerBase.edges.trimEnd(), '│ ●');
    // Outer rail leaves base at col 4 and arrives on outerBase at col 2.
    assert.ok(base.stemDown.some((s) => s.id === 'outerBase' && s.col === 4));
    assert.ok(outerBase.stemUp.some((s) => s.id === 'outerBase' && s.col === 2));

    const model = modelOf(commits, {
      uncommitted: null,
      stashes: [
        stashOf({
          id: 's-densify',
          stashRef: 'stash@{0}',
          authorDateUnix: 17,
          parentId: 'outerBase',
        }),
      ],
    });
    const rows = buildGraphListRows(model, laidOut, { width: 80 });
    const order = rowOrder(rows);
    const baseIdx = order.indexOf('commit:base');
    assert.equal(order[baseIdx], 'commit:base');
    assert.equal(order[baseIdx + 1], 'spacer');
    assert.equal(order[baseIdx + 2], 'stash:stash@{0}');
    assert.equal(order[baseIdx + 3], 'spacer');
    assert.equal(order[baseIdx + 4], 'commit:outerBase');

    const commitSpacer = rows[baseIdx + 1]!;
    const gw = laidOut[0]!.cells.length;
    const spacerGutter = segmentsText(commitSpacer.segments).slice(0, gw).trimEnd();
    assert.equal(
      spacerGutter,
      '│ ╭─╯',
      `expected densify on commit spacer across stash, got ${JSON.stringify(spacerGutter)}`,
    );

    const stash = rows.find((r) => r.kind === 'stash')!;
    const stashGutter = gutterOf(stash, graphWidthFor(laidOut));
    const parentCol = laneCol(outerBase.lane);
    const dCol = diamondCol(stashGutter);
    assert.ok(dCol >= 0, `expected ◇ in stash gutter, got ${JSON.stringify(stashGutter)}`);
    assert.notEqual(dCol, parentCol, '◇ must not sit on stash^1 lane');
    assert.match(
      stashGutter[parentCol] ?? '',
      /[│|]/,
      `expected through-rail on parent lane, got ${JSON.stringify(stashGutter)}`,
    );
    assert.doesNotMatch(
      stashGutter[parentCol] ?? '',
      /[├|+]/,
      `stash must not mid-rail tee (├─◇), got ${JSON.stringify(stashGutter)}`,
    );
    // Must not own densify elbows (those stay on the commit spacer).
    assert.doesNotMatch(stashGutter, /[╭╮╰╯]/);

    // outerBase stays layout-only for densify, but must close the stash leaf.
    const outerRow = rows.find((r) => r.commitId === 'outerBase')!;
    const outerGutter = segmentsText(outerRow.segments).slice(0, gw).trimEnd();
    assert.ok(
      hasCloseElbow(outerGutter),
      `outerBase must close stash leaf, got ${JSON.stringify(outerGutter)}`,
    );
    // Densify remap itself is not re-painted on the commit row.
    assert.doesNotMatch(
      outerGutter,
      /╭/,
      `outerBase must not double-paint densify open, got ${JSON.stringify(outerGutter)}`,
    );
  });

  it('adjacent commits connect densify-remapped rails without a stash', () => {
    // Same densify fixture as the stash test, but no stash between base and
    // outerBase — elbows must paint on the older commit row (list-layer overlay).
    const commits: GraphCommit[] = [
      {
        id: 'tipA',
        parents: ['base'],
        subject: 'tip A',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'tipB',
        parents: ['base'],
        subject: 'tip B',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'outerTip',
        parents: ['outerBase'],
        subject: 'outer tip',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'base',
        parents: ['older'],
        subject: 'join',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'outerBase',
        parents: ['older'],
        subject: 'outer base',
        authorName: 'Ada',
        authorDateUnix: 15,
        refs: [],
      },
      {
        id: 'older',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const base = laidOut.find((r) => r.commit.id === 'base')!;
    const outerBase = laidOut.find((r) => r.commit.id === 'outerBase')!;
    assert.equal(base.edges.trimEnd(), '●─╯ │');
    assert.equal(outerBase.edges.trimEnd(), '│ ●');
    assert.ok(base.stemDown.some((s) => s.id === 'outerBase' && s.col === 4));
    assert.ok(outerBase.stemUp.some((s) => s.id === 'outerBase' && s.col === 2));

    const model = modelOf(commits, { uncommitted: null, stashes: [] });
    const rows = buildGraphListRows(model, laidOut, { width: 80 });
    const order = rowOrder(rows);
    const baseIdx = order.indexOf('commit:base');
    assert.equal(order[baseIdx], 'commit:base');
    assert.equal(order[baseIdx + 1], 'spacer');
    assert.equal(order[baseIdx + 2], 'commit:outerBase');

    const spacer = rows[baseIdx + 1]!;
    const gw = laidOut[0]!.cells.length;
    const spacerGutter = segmentsText(spacer.segments).slice(0, gw).trimEnd();
    assert.equal(
      spacerGutter,
      '│ ╭─╯',
      `expected densify elbows on spacer, got ${JSON.stringify(spacerGutter)}`,
    );
    // Older commit stays layout-only — spacer carries the densify paint.
    const outerRow = rows.find((r) => r.commitId === 'outerBase')!;
    const outerGutter = segmentsText(outerRow.segments).slice(0, gw).trimEnd();
    assert.equal(
      outerGutter,
      '│ ●',
      `outer commit should not re-paint densify, got ${JSON.stringify(outerGutter)}`,
    );
  });

  it('stash leaf does not invent rails above a later lane open', () => {
    // Newer linear tip (lane 0 only) → stash parked on merge → merge opens lane 1.
    // Live rails at the stash gap must not invent the merge open early.
    const commits: GraphCommit[] = [
      {
        id: 'tip',
        parents: ['merge'],
        subject: 'tip',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'merge',
        parents: ['main', 'side'],
        subject: 'merge opens side',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
      {
        id: 'main',
        parents: ['base'],
        subject: 'main',
        authorName: 'Ada',
        authorDateUnix: 5,
        refs: [],
      },
      {
        id: 'side',
        parents: ['base'],
        subject: 'side',
        authorName: 'Ada',
        authorDateUnix: 4,
        refs: [],
      },
      {
        id: 'base',
        parents: [],
        subject: 'base',
        authorName: 'Ada',
        authorDateUnix: 1,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const merge = laidOut.find((r) => r.commit.id === 'merge')!;
    assert.ok(
      merge.edges.includes('╮') || merge.edges.includes('\\'),
      `expected open corner on merge, got ${JSON.stringify(merge.edges)}`,
    );
    const model = modelOf(commits, {
      uncommitted: null,
      stashes: [
        stashOf({
          id: 's-gap',
          stashRef: 'stash@{0}',
          authorDateUnix: 99, // date ignored — parked on merge
          parentId: 'merge',
        }),
      ],
    });
    const rows = buildGraphListRows(model, laidOut, { width: 80 });
    assert.deepEqual(rowOrder(rows).slice(0, 5), [
      'commit:tip',
      'spacer',
      'stash:stash@{0}',
      'spacer',
      'commit:merge',
    ]);
    const mergeLaid = laidOut.find((r) => r.commit.id === 'merge')!;
    const stash = rows.find((r) => r.kind === 'stash')!;
    const gw = graphWidthFor(laidOut);
    const gutter = gutterOf(stash, gw);
    const parentCol = laneCol(mergeLaid.lane);
    const dCol = diamondCol(gutter);
    assert.ok(dCol >= 0, `expected ◇, got ${JSON.stringify(gutter)}`);
    assert.notEqual(dCol, parentCol);
    assert.match(gutter[parentCol] ?? '', /[│|]/);
    assert.doesNotMatch(gutter[parentCol] ?? '', /[├|+]/);
    // Spur is a dead-end leaf — no vertical under/beyond the diamond inventing
    // a later merge open on a third lane.
    assert.ok(
      !/[│|]/.test(gutter.slice(dCol + 1)),
      `phantom rail beyond spur: ${JSON.stringify(gutter)}`,
    );
    const mergeRow = rows.find((r) => r.commitId === 'merge')!;
    assert.ok(
      hasCloseElbow(gutterOf(mergeRow, gw)),
      `join elbow on stash^1: ${JSON.stringify(gutterOf(mergeRow, gw))}`,
    );
  });

  it('puts ref chips on the spacer under each commit (subject stays on commit row)', () => {
    const commits = loadCommits('linear-three.json');
    const tipId = commits[0]!.id;
    const withRefs = [
      {
        ...commits[0]!,
        refs: [
          { kind: 'local' as const, name: 'main', commitId: tipId },
          { kind: 'remote' as const, name: 'origin/main', commitId: tipId },
        ],
      },
      ...commits.slice(1),
    ];
    const laid = layoutCommits(withRefs);
    const rows = buildGraphListRows(modelOf(withRefs, { stashes: [] }), laid, {
      width: 80,
      ascii: true,
      headId: tipId,
      headBranch: 'main',
    });
    const spacer = rows.find((r) => r.kind === 'spacer')!;
    const commit = rows.find((r) => r.kind === 'commit')!;
    const gw = laid[0]!.cells.length;
    const gutter = segmentsText(spacer.segments).slice(0, gw).trimEnd();
    assert.match(gutter, /[│|]/);
    assert.doesNotMatch(segmentsText(spacer.segments), /\[HEAD\]/);
    assert.match(segmentsText(spacer.segments), /\[\+=main\]/);
    assert.doesNotMatch(segmentsText(commit.segments), /\[HEAD\]|\[main/);
    assert.match(segmentsText(commit.segments), /\S/);
  });
});

describe('stash leaf tip invariants (3b→stash S0–S7)', () => {
  /** Assert shared leaf-tip cell rules for a stash row + its stash^1 join. */
  function assertStashLeafTip(opts: {
    rows: GraphListRow[];
    laidOut: LaidOutCommit[];
    stashRef: string;
    parentId: string;
    /** Lane columns that are live commit DAG rails at the stash gap — ◇ must avoid. */
    liveLaneCols?: number[];
  }) {
    const { rows, laidOut, stashRef, parentId, liveLaneCols = [] } = opts;
    const gw = graphWidthFor(laidOut, 2);
    const stash = rows.find((r) => r.kind === 'stash' && r.stashRef === stashRef);
    assert.ok(stash, `missing stash ${stashRef}`);
    const parentLaid = laidOut.find((r) => r.commit.id === parentId);
    assert.ok(parentLaid, `missing laid parent ${parentId}`);
    const gutter = gutterOf(stash!, gw);
    const parentCol = laneCol(parentLaid!.lane);
    const dCol = diamondCol(gutter);
    assert.ok(dCol >= 0, `◇ missing in ${JSON.stringify(gutter)}`);
    assert.notEqual(dCol, parentCol, '◇ must not sit on stash^1 lane');
    for (const liveCol of liveLaneCols) {
      assert.notEqual(
        dCol,
        liveCol,
        `◇ must not steal live DAG col ${liveCol}: ${JSON.stringify(gutter)}`,
      );
    }
    assert.match(gutter[parentCol] ?? '', /[│|]/);
    assert.doesNotMatch(
      gutter[parentCol] ?? '',
      /[├|+]/,
      `no mid-rail ├─◇ tee: ${JSON.stringify(gutter)}`,
    );
    assert.doesNotMatch(segmentsText(stash!.segments).slice(gw), /^◇|^s/);

    const parentRow = rows.find((r) => r.commitId === parentId);
    assert.ok(parentRow);
    assert.ok(
      hasCloseElbow(gutterOf(parentRow!, gw)),
      `stash^1 ${parentId} needs close elbow, got ${JSON.stringify(gutterOf(parentRow!, gw))}`,
    );

    // One node only: no ◇ on spacer; spur must not dangle past the parent join.
    const stashIdx = rows.indexOf(stash!);
    const parentIdx = rows.indexOf(parentRow!);
    assert.ok(stashIdx >= 0 && parentIdx > stashIdx);
    for (let i = stashIdx + 1; i < parentIdx; i++) {
      const mid = gutterOf(rows[i]!, gw);
      assert.ok(
        !hasStashNodeGlyph(mid),
        `no stash node between tip and join at row ${i}: ${JSON.stringify(mid)}`,
      );
    }
    // Spur ends at the join — no stem or node on this free leaf col below.
    // Stop before a later stash tip (may reuse the same free lane).
    for (let i = parentIdx + 1; i < rows.length; i++) {
      const row = rows[i]!;
      if (row.kind === 'stash') break;
      const below = gutterOf(row, gw);
      const cell = below[dCol] ?? '';
      assert.ok(
        !/[│|◇s]/.test(cell),
        `stash spur must not continue below join at col ${dCol}: ${JSON.stringify(below)}`,
      );
    }
  }

  it('S0 linear + stash: ◇ on free side lane; join close on stash^1', () => {
    const commits = loadCommits('linear-three.json');
    const laidOut = layoutCommits(commits);
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's0',
            stashRef: 'stash@{0}',
            authorDateUnix: 1700000001.5,
            parentId: 'bbb222',
          }),
        ],
      }),
      laidOut,
      { width: 80 },
    );
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{0}',
      parentId: 'bbb222',
      liveLaneCols: [laneCol(0)],
    });
  });

  it('S2 mid-spine: no dangling side rail between ◇ and join', () => {
    const commits: GraphCommit[] = [
      {
        id: 'aaa',
        parents: ['bbb'],
        subject: 'tip',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'bbb',
        parents: ['ccc'],
        subject: 'above',
        authorName: 'Ada',
        authorDateUnix: 40,
        refs: [],
      },
      {
        id: 'ccc',
        parents: ['ddd'],
        subject: 'join-parent',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'ddd',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's2',
            stashRef: 'stash@{0}',
            authorDateUnix: 30,
            parentId: 'ccc',
          }),
        ],
      }),
      laidOut,
      { width: 80 },
    );
    // Spine commits above the stash are fine.
    assert.equal(rowOrder(rows)[0], 'commit:aaa');
    assert.equal(rowOrder(rows)[2], 'commit:bbb');
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{0}',
      parentId: 'ccc',
      liveLaneCols: [laneCol(0)],
    });
    const stash = rows.find((r) => r.kind === 'stash')!;
    const parent = rows.find((r) => r.commitId === 'ccc')!;
    const gw = graphWidthFor(laidOut, 2);
    const dCol = diamondCol(gutterOf(stash, gw));
    // Immediate join: only stash spacer between ◇ and stash^1 (no chrono gap rows).
    assert.equal(rows.indexOf(parent), rows.indexOf(stash) + 2);
    const spacerGutter = gutterOf(rows[rows.indexOf(stash) + 1]!, gw);
    // Short spur rail toward join is OK; no second node.
    assert.ok(!hasStashNodeGlyph(spacerGutter));
    if (dCol >= 0) {
      assert.match(
        spacerGutter[dCol] ?? '',
        /[│| ]/,
        `spur under ◇ should be short rail or blank, got ${JSON.stringify(spacerGutter)}`,
      );
    }
  });

  it('S4 inside live merge rails: ◇ avoids live lanes; live rails pass through', () => {
    const commits: GraphCommit[] = [
      {
        id: 'merge',
        parents: ['main', 'side'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'side',
        parents: ['base'],
        subject: 'side',
        authorName: 'Ada',
        authorDateUnix: 40,
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
        id: 'base',
        parents: [],
        subject: 'base',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const side = laidOut.find((r) => r.commit.id === 'side')!;
    const main = laidOut.find((r) => r.commit.id === 'main')!;
    assert.ok(side.lane !== main.lane, 'fixture needs distinct side lane');
    const liveSideCol = laneCol(side.lane);
    const liveMainCol = laneCol(main.lane);
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's4',
            stashRef: 'stash@{0}',
            authorDateUnix: 30,
            parentId: 'main',
          }),
        ],
      }),
      laidOut,
      { width: 80 },
    );
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{0}',
      parentId: 'main',
      liveLaneCols: [liveMainCol, liveSideCol],
    });
    const stash = rows.find((r) => r.kind === 'stash')!;
    const gw = graphWidthFor(laidOut, 2);
    const gutter = gutterOf(stash, gw);
    // Live merge rails must still be present on the stash tip row.
    assert.match(
      gutter[liveMainCol] ?? '',
      /[│|├┤┼+]/,
      `live spine rail missing on stash row: ${JSON.stringify(gutter)}`,
    );
    assert.match(
      gutter[liveSideCol] ?? '',
      /[│|├┤┼+]/,
      `live side rail missing on stash row: ${JSON.stringify(gutter)}`,
    );
  });

  it('S5 parent on side lane: 1-node leaf off parent side line', () => {
    const commits: GraphCommit[] = [
      {
        id: 'merge',
        parents: ['main', 'side'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'side',
        parents: ['base'],
        subject: 'side',
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
        id: 'base',
        parents: [],
        subject: 'base',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const side = laidOut.find((r) => r.commit.id === 'side')!;
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's5',
            stashRef: 'stash@{0}',
            authorDateUnix: 40,
            parentId: 'side',
          }),
        ],
      }),
      laidOut,
      { width: 80 },
    );
    // Stash above its side-lane parent.
    assert.ok(rowOrder(rows).indexOf('stash:stash@{0}') < rowOrder(rows).indexOf('commit:side'));
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{0}',
      parentId: 'side',
      liveLaneCols: [laneCol(0), laneCol(side.lane)],
    });
  });

  it('S6 two stashes: independent tips; each joins own ^1', () => {
    const commits: GraphCommit[] = [
      {
        id: 'aaa',
        parents: ['bbb'],
        subject: 'tip',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'bbb',
        parents: ['ccc'],
        subject: 'mid',
        authorName: 'Ada',
        authorDateUnix: 30,
        refs: [],
      },
      {
        id: 'ccc',
        parents: ['ddd'],
        subject: 'low',
        authorName: 'Ada',
        authorDateUnix: 20,
        refs: [],
      },
      {
        id: 'ddd',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's0',
            stashRef: 'stash@{0}',
            index: 0,
            authorDateUnix: 40,
            parentId: 'bbb',
            subject: 'wip0',
          }),
          stashOf({
            id: 's1',
            stashRef: 'stash@{1}',
            index: 1,
            authorDateUnix: 25,
            parentId: 'ccc',
            subject: 'wip1',
          }),
        ],
      }),
      laidOut,
      { width: 80 },
    );
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{0}',
      parentId: 'bbb',
      liveLaneCols: [laneCol(0)],
    });
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{1}',
      parentId: 'ccc',
      liveLaneCols: [laneCol(0)],
    });
    const gw = graphWidthFor(laidOut, 2);
    const s0 = rows.find((r) => r.stashRef === 'stash@{0}' && r.kind === 'stash')!;
    const s1 = rows.find((r) => r.stashRef === 'stash@{1}' && r.kind === 'stash')!;
    const g0 = gutterOf(s0, gw);
    const g1 = gutterOf(s1, gw);
    const d0 = diamondCol(g0);
    const d1 = diamondCol(g1);
    assert.ok(d0 >= 0 && d1 >= 0, `both tips need ◇ (d0=${d0} d1=${d1})`);

    // Independent leaf tips: when spur columns differ, neither tip hosts the other's node.
    if (d0 !== d1) {
      assert.ok(
        !/[◇s]/.test(g1[d0] ?? ''),
        `stash@{1} must not carry stash@{0} diamond col ${d0}: ${JSON.stringify(g1)}`,
      );
      assert.ok(
        !/[◇s]/.test(g0[d1] ?? ''),
        `stash@{0} must not carry stash@{1} diamond col ${d1}: ${JSON.stringify(g0)}`,
      );
    }

    // Neither spur continues into the other: after s0's join (bbb), col d0 must be
    // clear through the s1 region (stem `|`/`│` or node). Shared free-lane reuse at
    // the s1 tip itself is OK when d0 === d1.
    const s0Idx = rows.indexOf(s0);
    const bbbIdx = rows.findIndex((r) => r.commitId === 'bbb');
    const s1Idx = rows.indexOf(s1);
    assert.ok(s0Idx >= 0 && bbbIdx > s0Idx && s1Idx > bbbIdx);

    if (d0 !== d1) {
      for (let i = s0Idx; i <= bbbIdx; i++) {
        const g = gutterOf(rows[i]!, gw);
        assert.ok(
          !/[│|◇s]/.test(g[d1] ?? ''),
          `stash@{1} spur col ${d1} must not appear in stash@{0} tip→join region at row ${i}: ${JSON.stringify(g)}`,
        );
      }
    }

    for (let i = bbbIdx + 1; i <= s1Idx; i++) {
      const g = gutterOf(rows[i]!, gw);
      const cell = g[d0] ?? '';
      if (i === s1Idx && d0 === d1) {
        assert.match(
          cell,
          /[◇s]/,
          `shared free lane at stash@{1} tip should be ◇, got ${JSON.stringify(g)}`,
        );
        continue;
      }
      assert.ok(
        !/[│|◇s]/.test(cell),
        `stash@{0} spur col ${d0} must not continue into stash@{1} region at row ${i}: ${JSON.stringify(g)}`,
      );
    }
  });

  it('S7 parent is HEAD tip: close elbow on tip / HEAD row', () => {
    const commits = loadCommits('linear-three.json');
    const laidOut = layoutCommits(commits);
    const tipId = commits[0]!.id;
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's7',
            stashRef: 'stash@{0}',
            authorDateUnix: 1700000003,
            parentId: tipId,
          }),
        ],
      }),
      laidOut,
      { width: 80, headId: tipId },
    );
    assert.equal(rowOrder(rows)[0], 'stash:stash@{0}');
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{0}',
      parentId: tipId,
      liveLaneCols: [laneCol(0)],
    });
    const tipRow = rows.find((r) => r.commitId === tipId)!;
    const tipGutter = gutterOf(tipRow, graphWidthFor(laidOut, 2));
    assert.match(tipGutter, /[⊙@]/, `HEAD glyph on tip, got ${JSON.stringify(tipGutter)}`);
    assert.ok(
      hasCloseElbow(tipGutter),
      `expected ⊙─╯ family on HEAD stash^1, got ${JSON.stringify(tipGutter)}`,
    );
  });

  it('parent outside window: lone ◇ on free lane; no fake spine tee', () => {
    const commits = loadCommits('linear-three.json');
    const laidOut = layoutCommits(commits);
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's-out',
            stashRef: 'stash@{0}',
            authorDateUnix: 1700000001.5,
            parentId: 'NOT_IN_WINDOW',
          }),
        ],
      }),
      laidOut,
      { width: 80 },
    );
    const stash = rows.find((r) => r.kind === 'stash')!;
    const gw = graphWidthFor(laidOut, 2);
    const gutter = gutterOf(stash, gw);
    const dCol = diamondCol(gutter);
    assert.ok(dCol >= 0, `expected lone ◇, got ${JSON.stringify(gutter)}`);
    // No invented attachment tee when stash^1 is missing.
    assert.doesNotMatch(
      gutter,
      /[├|+]/,
      `no fake spine tee when parent outside window: ${JSON.stringify(gutter)}`,
    );
    assert.doesNotMatch(segmentsText(stash.segments).slice(gw), /^◇|^s/);
  });

  it('S6b same-parent siblings: distinct leaf lanes; both join on stash^1', () => {
    const commits = loadCommits('linear-three.json');
    const laidOut = layoutCommits(commits);
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's0',
            stashRef: 'stash@{0}',
            index: 0,
            authorDateUnix: 1700000099,
            parentId: 'aaa111',
            subject: 'wip0',
          }),
          stashOf({
            id: 's1',
            stashRef: 'stash@{1}',
            index: 1,
            authorDateUnix: 1700000090,
            parentId: 'aaa111',
            subject: 'wip1',
          }),
        ],
      }),
      laidOut,
      { width: 80 },
    );
    const gw = graphWidthFor(laidOut, 3);
    const dump = gutterDump(rows, gw);
    const s0 = rows.find((r) => r.kind === 'stash' && r.stashRef === 'stash@{0}')!;
    const s1 = rows.find((r) => r.kind === 'stash' && r.stashRef === 'stash@{1}')!;
    const d0 = diamondCol(gutterOf(s0, gw));
    const d1 = diamondCol(gutterOf(s1, gw));
    assert.ok(d0 >= 0 && d1 >= 0, `both tips need ◇\n${dump}`);
    assert.notEqual(d0, d1, `sibling tips must not share a leaf col\n${dump}`);
    const s1Gutter = gutterOf(s1, gw);
    assert.match(
      s1Gutter[d0] ?? '',
      /[│|]/,
      `newer sibling spur must continue through the older tip row\n${dump}`,
    );
    // Lower tip sits immediately above stash^1 — full 1-node leaf grammar.
    assertStashLeafTip({
      rows,
      laidOut,
      stashRef: 'stash@{1}',
      parentId: 'aaa111',
      liveLaneCols: [laneCol(0)],
    });
    const parentRow = rows.find((r) => r.commitId === 'aaa111')!;
    const parentGutter = gutterOf(parentRow, gw);
    const joinMarks = parentGutter.match(/[╯┴]/g) ?? [];
    assert.ok(
      joinMarks.length >= 2,
      `same-parent siblings need two join marks (╯/┴), got ${JSON.stringify(parentGutter)}\n${dump}`,
    );
  });

  it('layouts a newer extra stash parent before the log window', () => {
    const window = loadCommits('linear-three.json');
    const extra = {
      id: 'extra-head',
      parents: ['ghost-missing'],
      subject: 'deleted-branch',
      authorName: 'Ada',
      authorDateUnix: 1_700_000_099,
      refs: [],
    };
    const model = modelOf([...window, extra], {
      uncommitted: null,
      stashes: [
        stashOf({
          id: 's-extra',
          stashRef: 'stash@{0}',
          authorDateUnix: 1_700_000_100,
          parentId: 'extra-head',
        }),
      ],
    });
    model.windowCount = window.length;
    const laid = ensureLaidOut(model);
    assert.equal(laid[0]!.commit.id, 'extra-head');
    assert.deepEqual(laid[0]!.commit.parents, [], 'missing extra %P must not plant a waiter');
    for (const row of laid.slice(1)) {
      assert.equal(
        row.laneCount,
        1,
        `window commit ${row.commit.id} must not inherit a ghost extra rail`,
      );
    }
    const rows = buildGraphListRows(model, laid, { width: 80 });
    const gw = graphWidthFor(laid, 2);
    const dump = gutterDump(rows, gw);
    const order = rowOrder(rows);
    assert.equal(order[0], 'stash:stash@{0}');
    assert.equal(order[2], 'commit:extra-head');
    const parent = rows.find((r) => r.commitId === 'extra-head')!;
    assert.ok(
      hasCloseElbow(gutterOf(parent, gw)),
      `extra stash^1 must still join, got ${JSON.stringify(gutterOf(parent, gw))}\n${dump}`,
    );
    for (const id of ['aaa111', 'bbb222', 'ccc333']) {
      const row = rows.find((r) => r.commitId === id)!;
      const g = gutterOf(row, gw);
      assert.equal(
        [...g].filter((ch) => ch === '│' || ch === '|').length,
        0,
        `window row ${id} must not carry a ghost waiter rail, got ${JSON.stringify(g)}\n${dump}`,
      );
    }
  });

  it('inserts extras into the git window without re-sorting window rows', () => {
    const child: GraphCommit = {
      id: 'child',
      parents: ['parent'],
      subject: 'child',
      authorName: 'Ada',
      authorDateUnix: 50,
      refs: [],
    };
    const parent: GraphCommit = {
      id: 'parent',
      parents: [],
      subject: 'parent',
      authorName: 'Ada',
      authorDateUnix: 100,
      refs: [],
    };
    const extra: GraphCommit = {
      id: 'extra',
      parents: ['ghost-missing'],
      subject: 'deleted-branch',
      authorName: 'Ada',
      authorDateUnix: 75,
      refs: [],
    };
    const model = modelOf([child, parent, extra], {
      uncommitted: null,
      stashes: [],
    });
    model.windowCount = 2;
    const layout = graphLayoutCommits(model);
    assert.deepEqual(
      layout.map((c) => c.id),
      ['extra', 'child', 'parent'],
      'date-sort must not put parent before child',
    );
    assert.deepEqual(layout.find((c) => c.id === 'extra')!.parents, []);
  });

  it('keeps an extra stash parent edge when that parent is in the window', () => {
    const window = loadCommits('linear-three.json');
    const extra: GraphCommit = {
      id: 'extra-head',
      parents: ['aaa111'],
      subject: 'deleted-branch',
      authorName: 'Ada',
      authorDateUnix: 1_700_000_099,
      refs: [],
    };
    const model = modelOf([...window, extra], {
      uncommitted: null,
      stashes: [],
    });
    model.windowCount = window.length;
    const layout = graphLayoutCommits(model);
    assert.deepEqual(layout.find((c) => c.id === 'extra-head')!.parents, ['aaa111']);
  });

  it('crowded gutter: clipped window still shows ◇ and the stash^1 join', () => {
    const commits: GraphCommit[] = [
      {
        id: 'merge',
        parents: ['main', 'side'],
        subject: 'merge',
        authorName: 'Ada',
        authorDateUnix: 50,
        refs: [],
      },
      {
        id: 'side',
        parents: ['base'],
        subject: 'side',
        authorName: 'Ada',
        authorDateUnix: 40,
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
        id: 'base',
        parents: [],
        subject: 'base',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laidOut = layoutCommits(commits);
    const rows = buildGraphListRows(
      modelOf(commits, {
        uncommitted: null,
        stashes: [
          stashOf({
            id: 's-crowd',
            stashRef: 'stash@{0}',
            authorDateUnix: 30,
            parentId: 'main',
          }),
        ],
      }),
      laidOut,
      { width: 40, graphWidth: 3 },
    );
    const gw = 3;
    const dump = gutterDump(rows, gw);
    const stash = rows.find((r) => r.kind === 'stash')!;
    const parent = rows.find((r) => r.commitId === 'main')!;
    assert.ok(diamondCol(gutterOf(stash, gw)) >= 0, `clipped stash row must keep ◇\n${dump}`);
    const parentGutter = gutterOf(parent, gw);
    assert.match(
      parentGutter,
      /[╯┼┴\\/+]/,
      `clipped stash^1 row must keep a join mark, got ${JSON.stringify(parentGutter)}\n${dump}`,
    );
  });
});

describe('selectable graph navigation', () => {
  it('skips spacers for j/k, page, and nearest snap', () => {
    const commits = loadCommits('linear-three.json');
    const rows = buildGraphListRows(
      modelOf(commits, { stashes: [], uncommitted: null }),
      layoutCommits(commits),
      { width: 80 },
    );
    assert.deepEqual(
      rows.map((r) => r.kind),
      ['commit', 'spacer', 'commit', 'spacer', 'commit', 'spacer'],
    );
    assert.equal(firstSelectableGraphIndex(rows), 0);
    assert.equal(lastSelectableGraphIndex(rows), 4);
    assert.equal(stepSelectableGraphCursor(rows, 0, 1), 2);
    assert.equal(stepSelectableGraphCursor(rows, 2, 1), 4);
    assert.equal(stepSelectableGraphCursor(rows, 4, 1), 4);
    assert.equal(stepSelectableGraphCursor(rows, 4, -1), 2);
    assert.equal(nearestSelectableGraphIndex(rows, 1), 2);
    assert.equal(applySelectableGraphPageMove(rows, 0, 2, 1), 2);
    assert.ok(rows.every((r) => (r.kind === 'spacer') !== isSelectableGraphRow(r)));
  });

  it('resets to first selectable on a new dataset, keeps nearest on same-view', () => {
    const commits = loadCommits('linear-three.json');
    const rows = buildGraphListRows(
      modelOf(commits, { stashes: [], uncommitted: null }),
      layoutCommits(commits),
      { width: 80 },
    );
    assert.equal(graphCursorAfterRowsReload(rows, 4, true), 0);
    assert.equal(graphCursorAfterRowsReload(rows, 4, false), 4);
    assert.equal(graphCursorAfterRowsReload(rows, 3, false), 4);
  });

  it('selectableGraphIndexFromClick prefers spacer parent above', () => {
    const rows: GraphListRow[] = [
      {
        id: graphRowId('commit', 'aaa'),
        kind: 'commit',
        commitId: 'aaa',
        segments: [],
      },
      {
        id: graphRowId('spacer', 'aaa'),
        kind: 'spacer',
        commitId: null,
        segments: [],
      },
      {
        id: graphRowId('commit', 'bbb'),
        kind: 'commit',
        commitId: 'bbb',
        segments: [],
      },
    ];
    assert.equal(selectableGraphIndexFromClick(rows, 0), 0);
    assert.equal(selectableGraphIndexFromClick(rows, 1), 0);
    assert.equal(selectableGraphIndexFromClick(rows, 2), 2);
    // nearestSelectable prefers forward — click helper must differ on spacer.
    assert.equal(nearestSelectableGraphIndex(rows, 1), 2);
  });
});

describe('ensureLaidOut', () => {
  it('reuses matching layout and rebuilds when lengths diverge', () => {
    const commits = loadCommits('linear-three.json');
    const model = modelOf(commits);
    const laid = layoutCommits(commits);
    assert.equal(ensureLaidOut(model, laid), laid);
    const shorter = laid.slice(0, 1);
    assert.notEqual(ensureLaidOut(model, shorter), shorter);
    assert.equal(ensureLaidOut(model, shorter).length, 3);
  });
});

describe('activeRepoPath / visibility', () => {
  const depth0: NavState = { stack: [{ kind: 'workspace' }], focusPane: 'left' };
  const depth1: NavState = {
    stack: [{ kind: 'workspace' }, { kind: 'repoGraph', repo: '/ws/demo', commitId: null }],
    focusPane: 'left',
  };

  it('uses focused tree repo at depth 0 and stack repo at depth 1', () => {
    assert.equal(activeRepoPath(depth0, repoRow()), '/ws/demo');
    assert.equal(activeRepoPath(depth0, fileRow()), '/ws/demo');
    assert.equal(activeRepoPath(depth0, undefined), null);
    assert.equal(activeRepoPath(depth1, undefined), '/ws/demo');
  });

  it('shows graph for repo/dir at depth 0, not for files; always at depth 1', () => {
    assert.equal(shouldShowGraphDetail(depth0, repoRow()), true);
    assert.equal(shouldShowGraphDetail(depth0, fileRow()), false);
    assert.equal(shouldShowFileDiff(depth0, fileRow()), true);
    assert.equal(shouldShowGraphDetail(depth1, undefined), true);
    assert.equal(shouldShowFileDiff(depth1, fileRow()), false);
  });
});
