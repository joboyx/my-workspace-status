import assert from 'node:assert';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it } from 'node:test';
import { layoutCommits } from '../src/tui/graph/layout.js';
import { graphGlyphs } from '../src/tui/graph/glyphs.js';
import { sliceCellsAroundLane } from '../src/tui/graph/gutterBudget.js';
import {
  allocateStashLeafLane,
  applyCommitDensifyCells,
  applyStashJoinCells,
  formatRelativeDate,
  graphCommitSegments,
  graphSpacerSegments,
  graphStashSegments,
  graphStashSpacerSegments,
  graphUncommittedSegments,
  refChipColor,
  stashRailAnchorLane,
  stashRailCells,
} from '../src/tui/graph/rows.js';
import type { GraphCommit } from '../src/tui/graph/types.js';
import { segmentsText } from '../src/tui/theme.js';

const U = graphGlyphs(false);

const FIX = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/graph');

const commit: GraphCommit = {
  id: 'abcdef1234567890',
  parents: [],
  subject: 'fix the thing',
  authorName: 'Ada Lovelace',
  authorDateUnix: 1_700_000_000,
  refs: [
    { kind: 'local', name: 'main', commitId: 'abcdef1234567890' },
    { kind: 'remote', name: 'origin/main', commitId: 'abcdef1234567890' },
    { kind: 'tag', name: 'v1', commitId: 'abcdef1234567890' },
  ],
};

describe('formatRelativeDate', () => {
  it('formats minutes and days', () => {
    assert.equal(formatRelativeDate(100, 100), 'just now');
    assert.equal(formatRelativeDate(0, 120), '2m');
    assert.equal(formatRelativeDate(0, 3600 * 3), '3h');
    assert.equal(formatRelativeDate(0, 86400 * 2), '2d');
  });
});

describe('graphCommitSegments', () => {
  it('includes graph and subject on commit row; meta+refs on spacer (layout A)', () => {
    const [row] = layoutCommits([commit]);
    const segs = graphCommitSegments(row, { width: 200, nowUnix: 1_700_000_000 + 60 });
    const text = segmentsText(segs);
    assert.match(text, /●|│|\*/);
    assert.doesNotMatch(text, /\[main|\[v1\]|\[HEAD\]/);
    assert.match(text, /fix the thing/);
    assert.doesNotMatch(text, /abcdef1/);
    assert.doesNotMatch(text, /Ada Lovelace/);
    const spacer = segmentsText(
      graphSpacerSegments({ width: 200, nowUnix: 1_700_000_000 + 60 }, row, null),
    );
    assert.ok(spacer.includes(`[${U.syncMark}main]`) || /\[\+?=main\]/.test(spacer));
    assert.match(spacer, /abcdef1/);
    assert.match(spacer, /1m|just now/);
    assert.match(spacer, /Ada Lovelace/);
  });

  it('marks HEAD with filled glyph; checkout chip gets mark (no standalone [HEAD])', () => {
    const [row] = layoutCommits([commit]);
    const segs = graphCommitSegments(row, {
      width: 200,
      nowUnix: 1_700_000_100,
      headId: commit.id,
      headBranch: 'main',
      headMarkColor: '#head',
    });
    const text = segmentsText(segs);
    assert.doesNotMatch(text, /\[HEAD\]/);
    assert.match(text, /⊙|@/);
    assert.match(text, /fix the thing/);
    const spacer = graphSpacerSegments(
      {
        width: 200,
        nowUnix: 1_700_000_100,
        headId: commit.id,
        headBranch: 'main',
        headMarkColor: '#head',
        refDefaultColor: '#default',
        refRemoteColor: '#remote',
      },
      row,
      null,
    );
    const spacerText = segmentsText(spacer);
    assert.doesNotMatch(spacerText, /\[HEAD\]/);
    assert.ok(spacerText.includes(`[${U.checkoutMark}${U.syncMark}main]`));
    const cross = spacer.find((s) => s.text === U.checkoutMark);
    assert.ok(cross);
    assert.equal(cross.color, '#head');
    assert.equal(cross.bold, true);
    // HEAD glyph stays single-column (no wide emoji).
    const nodeSeg = segs.find((s) => /[⊙@]/.test(s.text));
    assert.ok(nodeSeg);
    assert.ok([...nodeSeg.text].every((ch) => ch.codePointAt(0)! < 0x1f300));
  });

  it('keeps standalone [HEAD] chip only when detached', () => {
    const [row] = layoutCommits([commit]);
    const spacer = graphSpacerSegments(
      {
        width: 200,
        nowUnix: 1_700_000_100,
        headId: commit.id,
        headBranch: 'HEAD (detached)',
        headMarkColor: '#head',
        refDefaultColor: '#default',
        refRemoteColor: '#remote',
      },
      row,
      null,
    );
    const headChip = spacer.find((s) => s.text === '[HEAD]');
    assert.ok(headChip);
    assert.equal(headChip.color, '#head');
    assert.equal(headChip.bold, true);
    assert.ok(!segmentsText(spacer).includes(U.checkoutMark));
    assert.ok(segmentsText(spacer).includes(`[${U.syncMark}main]`));
  });

  it('falls back to standalone [HEAD] when headBranch is missing', () => {
    const [row] = layoutCommits([commit]);
    const spacer = graphSpacerSegments(
      {
        width: 200,
        nowUnix: 1,
        headId: commit.id,
        headMarkColor: '#head',
      },
      row,
      null,
    );
    assert.match(segmentsText(spacer), /\[HEAD\]/);
    assert.ok(!segmentsText(spacer).includes(U.checkoutMark));
  });

  it('prefixes checkout mark on a local-only branch chip', () => {
    const localOnly: GraphCommit = {
      ...commit,
      refs: [{ kind: 'local', name: 'feature/x', commitId: commit.id }],
    };
    const [row] = layoutCommits([localOnly]);
    const text = segmentsText(
      graphSpacerSegments(
        {
          width: 200,
          nowUnix: 1,
          headId: commit.id,
          headBranch: 'feature/x',
          headMarkColor: '#head',
          refLocalColor: '#local',
        },
        row,
        null,
      ),
    );
    assert.ok(text.includes(`[${U.checkoutMark}feature/x]`));
    assert.doesNotMatch(text, /\[HEAD\]/);
    assert.ok(!text.includes(U.syncMark));
  });

  it('paints HEAD glyph on merge commits too (⊙)', () => {
    const merge: GraphCommit = {
      ...commit,
      id: 'merge000',
      parents: ['a', 'b'],
      refs: [],
    };
    const [row] = layoutCommits([merge]);
    const text = segmentsText(
      graphCommitSegments(row, { width: 80, headId: merge.id, nowUnix: 1 }),
    );
    assert.doesNotMatch(text, /\[HEAD\]/);
    assert.match(text, /⊙|@/);
    assert.doesNotMatch(text, /◎/);
  });

  it('colours each ref segment by kind on the spacer (merged local+remote)', () => {
    const [row] = layoutCommits([commit]);
    const segs = graphSpacerSegments(
      {
        width: 200,
        nowUnix: 1_700_000_100,
        refDefaultColor: '#default',
        refLocalColor: '#local',
        refRemoteColor: '#remote',
        refTagColor: '#tag',
        subjectColor: '#subject',
      },
      row,
      null,
    );
    const open = segs.find((s) => s.text === '[' && s.color === '#default');
    const arrow = segs.find((s) => s.text === U.syncMark);
    const name = segs.find((s) => s.text === 'main' && s.color === '#default');
    const close = segs.find((s) => s.text === ']' && s.color === '#default');
    const tag = segs.find((s) => s.text === '[v1]');
    assert.ok(open);
    assert.equal(arrow?.color, '#remote');
    assert.ok(name);
    assert.ok(close);
    assert.equal(tag?.color, '#tag');
    assert.equal(
      refChipColor(commit.refs[0]!, { width: 1, refDefaultColor: '#default' }),
      '#default',
    );
  });

  it('colours origin/<default> as default via short-name + override', () => {
    const remoteOnly: GraphCommit = {
      ...commit,
      refs: [{ kind: 'remote', name: 'origin/develop', commitId: commit.id }],
    };
    assert.equal(
      refChipColor(remoteOnly.refs[0]!, {
        width: 1,
        defaultBranchOverride: 'develop',
        refDefaultColor: '#default',
        refRemoteColor: '#remote',
      }),
      '#default',
    );
    assert.equal(
      refChipColor(remoteOnly.refs[0]!, {
        width: 1,
        defaultBranchOverride: 'main',
        refDefaultColor: '#default',
        refRemoteColor: '#remote',
      }),
      '#remote',
    );
  });

  it('merges matching local+remote into sync-prefixed name and keeps unmatched remotes', () => {
    const multi: GraphCommit = {
      ...commit,
      refs: [
        { kind: 'local', name: 'feature/x', commitId: commit.id },
        { kind: 'remote', name: 'origin/feature/x', commitId: commit.id },
        { kind: 'remote', name: 'origin/other', commitId: commit.id },
        { kind: 'tag', name: 'v2', commitId: commit.id },
      ],
    };
    const [row] = layoutCommits([multi]);
    const text = segmentsText(
      graphSpacerSegments(
        {
          width: 200,
          nowUnix: 1,
          refLocalColor: '#local',
          refRemoteColor: '#remote',
          refTagColor: '#tag',
        },
        row,
        null,
      ),
    );
    assert.ok(text.includes(`[${U.syncMark}feature/x]`));
    assert.match(text, /\[origin\/other\]/);
    assert.match(text, /\[v2\]/);
    assert.doesNotMatch(text, /\[origin\/feature\/x\]/);
  });

  it('ASCII mode uses +/= marks before the branch name', () => {
    const [row] = layoutCommits([commit]);
    const text = segmentsText(
      graphSpacerSegments(
        {
          width: 200,
          nowUnix: 1,
          ascii: true,
          headId: commit.id,
          headBranch: 'main',
          headMarkColor: '#head',
          refDefaultColor: '#default',
          refRemoteColor: '#remote',
        },
        row,
        null,
      ),
    );
    assert.match(text, /\[\+=main\]/);
    assert.doesNotMatch(text, /\[HEAD\]/);
  });

  it('drops hash → date → author on spacer as width shrinks', () => {
    const [row] = layoutCommits([commit]);
    const wide = segmentsText(
      graphSpacerSegments({ width: 200, nowUnix: 1_700_000_100 }, row, null),
    );
    assert.match(wide, /abcdef1/);
    assert.match(wide, /Ada Lovelace/);

    let text = wide;
    let width = wide.length;
    let sawDropHash = false;
    let sawDropDate = false;
    let sawDropAuthor = false;
    while (width > 20) {
      width -= 1;
      text = segmentsText(
        graphSpacerSegments({ width, nowUnix: 1_700_000_100 }, row, null),
      );
      if (!sawDropHash && !text.includes('abcdef1')) {
        sawDropHash = true;
        assert.match(text, /Ada Lovelace/);
      }
      if (sawDropHash && !sawDropDate && !/\b\d+[mhdwy]\b|just now/.test(text)) {
        sawDropDate = true;
        assert.match(text, /Ada Lovelace/);
      }
      if (sawDropDate && !sawDropAuthor && !text.includes('Ada Lovelace')) {
        sawDropAuthor = true;
        // Prefer refs over meta when tight.
        assert.ok(text.includes(`[${U.syncMark}main]`) || text.includes('[v1]'));
      }
    }
    assert.ok(sawDropHash && sawDropDate && sawDropAuthor);
  });

  it('keeps ref chip colours when spacer width forces truncation', () => {
    const multi: GraphCommit = {
      ...commit,
      refs: [
        {
          kind: 'local',
          name: 'feature/very-long-branch-name-alpha',
          commitId: commit.id,
        },
        {
          kind: 'local',
          name: 'feature/another-long-branch-beta',
          commitId: commit.id,
        },
        { kind: 'tag', name: 'v1.2.3-release-candidate', commitId: commit.id },
      ],
    };
    const [row] = layoutCommits([multi]);
    const colorOpts = {
      nowUnix: 1 as const,
      graphWidth: 2,
      mutedColor: '#muted',
      refLocalColor: '#local',
      refTagColor: '#tag',
      subjectColor: '#subject',
    };
    const wide = graphSpacerSegments({ ...colorOpts, width: 200 }, row, null);
    assert.ok(wide.some((s) => s.color === '#local'));
    assert.ok(wide.some((s) => s.color === '#tag'));

    // Depth-1 left panes are much narrower than depth-0 right; truncation
    // must keep chip colours (not flatten the whole ref run to muted).
    const narrow = graphSpacerSegments({ ...colorOpts, width: 36 }, row, null);
    const chipColors = new Set(
      narrow.filter((s) => s.color === '#local' || s.color === '#tag').map((s) => s.color),
    );
    assert.ok(chipColors.size > 0, 'narrow spacer must retain at least one chip colour');
    assert.ok(
      !narrow.some(
        (s) =>
          s.color === '#muted' &&
          (s.text.includes('feature/') || s.text.includes('v1.2.3')),
      ),
      'must not flatten truncated refs into a single muted blob',
    );
  });

  it('ref chip colours match across wide and narrow panes when chips fully fit', () => {
    const [row] = layoutCommits([commit]);
    const colorOpts = {
      nowUnix: 1_700_000_100 as const,
      mutedColor: '#muted',
      refDefaultColor: '#default',
      refLocalColor: '#local',
      refRemoteColor: '#remote',
      refTagColor: '#tag',
      subjectColor: '#subject',
    };
    const chipColorKey = (segs: ReturnType<typeof graphSpacerSegments>) =>
      segs
        .filter((s) => s.color === '#default' || s.color === '#remote' || s.color === '#tag')
        .map((s) => `${s.color}:${s.text}`)
        .join('|');

    // Both budgets fit the short synced main + tag chips; colours must match.
    const rightPane = chipColorKey(
      graphSpacerSegments({ ...colorOpts, width: 120 }, row, null),
    );
    const leftPane = chipColorKey(
      graphSpacerSegments({ ...colorOpts, width: 72 }, row, null),
    );
    assert.ok(rightPane.length > 0);
    assert.equal(leftPane, rightPane);
  });

  it('aligns hash columns across spacer rows with different lane counts', () => {
    const diamond = layoutCommits(
      JSON.parse(readFileSync(join(FIX, 'merge-diamond.json'), 'utf8')) as GraphCommit[],
    );
    const now = 1_700_000_100;
    const gw = diamond[0]!.cells.length;
    const texts = diamond.map((row, i) => {
      const next = diamond[i + 1] ?? null;
      return segmentsText(
        graphSpacerSegments(
          {
            width: 120,
            nowUnix: now,
            graphWidth: gw,
            dateWidth: 4,
            authorWidth: 3,
          },
          row,
          next,
          next ? 'densify' : 'through',
        ),
      );
    });
    const idxs = diamond.map((row, i) => {
      const hash = row.commit.id.slice(0, 7);
      return texts[i]!.lastIndexOf(hash);
    });
    assert.ok(idxs.every((i) => i > 0), `missing hash in\n${texts.join('\n')}`);
    assert.ok(
      idxs.every((i) => i === idxs[0]),
      `hash cols misaligned: ${idxs}\n${texts.join('\n')}`,
    );
  });

  it('uses capped gutter and keeps the node when topology exceeds budget', () => {
    const wide: GraphCommit = {
      ...commit,
      id: 'wide000',
      parents: [],
      subject: 'long enough subject to fill leftover pane space here',
      refs: [{ kind: 'local', name: 'feature/wide-gutter', commitId: 'wide000' }],
    };
    // Fabricate a fat gutter by cloning cells — layout of one commit is narrow,
    // so pad after layout to simulate many lanes.
    const [row] = layoutCommits([wide]);
    const fatCells = [
      ...Array.from({ length: 24 }, () => ({
        ch: '│',
        colorLane: 0 as number | null,
        role: 'pipe' as const,
      })),
      ...row.cells,
    ];
    // Node should be near the end of the fabricated gutter.
    const nodeIdx = fatCells.findIndex((c) => c.role === 'node');
    assert.ok(nodeIdx >= 0);
    const capped = graphCommitSegments(
      { ...row, cells: fatCells, lane: Math.floor(nodeIdx / 2), laneCount: 13 },
      { width: 100, nowUnix: 1_700_000_060, graphWidth: 20 },
    );
    const text = segmentsText(capped);
    assert.match(text, /○|●|\*/);
    assert.match(text, /long enough subject|feature\/wide/);
    // Gutter + gap + subject should consume most of the 100-col budget (not stuck at ~80).
    assert.ok(text.length >= 90, `expected wide paint, got ${text.length}: ${text}`);
  });

  it('stash and uncommitted helpers return non-empty segments', () => {
    const stash = {
      id: 's1abcdef',
      stashRef: 'stash@{0}',
      index: 0,
      subject: 'wip',
      authorName: 'Ada',
      authorDateUnix: 1,
      parentId: 'parent',
    };
    // 3b→stash tip: through-rail on parent lane + ◇ on a free side lane (not ├─◇).
    const [parentLaid] = layoutCommits([
      {
        id: 'parent',
        parents: [],
        subject: 'parent',
        authorName: 'Ada',
        authorDateUnix: 1,
        refs: [],
      },
    ]);
    const stashSegs = graphStashSegments(
      stash,
      { width: 80, nowUnix: 100, graphWidth: 4 },
      parentLaid,
    );
    const stashText = segmentsText(stashSegs);
    assert.match(stashText[0] ?? '', /[│|]/);
    assert.doesNotMatch(
      stashText[0] ?? '',
      /[├|+]/,
      `stash tip must not mid-rail tee, got ${JSON.stringify(stashText.slice(0, 4))}`,
    );
    assert.match(stashText.slice(0, 4), /◇|s/);
    assert.match(stashText, /wip/);
    // Subject follows gutter (4) + separator space — no leading diamond there.
    assert.doesNotMatch(stashText.slice(5), /^[◇s]/);
    assert.doesNotMatch(stashText, /stash@\{0\}/);
    const spacerText = segmentsText(
      graphStashSpacerSegments(
        stash,
        { width: 80, nowUnix: 100, graphWidth: 4 },
        parentLaid,
      ),
    );
    assert.match(spacerText, /stash@\{0\}/);
    assert.match(spacerText, /s1abcde/);
    assert.ok(
      !/[◇s]/.test(spacerText.slice(0, 4)),
      `spacer must not paint a second stash node, got ${JSON.stringify(spacerText.slice(0, 4))}`,
    );
    const uSegs = graphUncommittedSegments(
      { kind: 'uncommitted', hasChanges: true },
      { width: 40 },
    );
    assert.match(segmentsText(uSegs), /uncommitted|working tree/i);
  });

  it('stash with parent outside window is a lone free-lane ◇ (no fake tee)', () => {
    const stash = {
      id: 's1abcdef',
      stashRef: 'stash@{0}',
      index: 0,
      subject: 'wip',
      authorName: 'Ada',
      authorDateUnix: 1,
      parentId: 'missing',
    };
    const text = segmentsText(
      graphStashSegments(stash, { width: 80, nowUnix: 100, graphWidth: 4 }, null),
    );
    assert.match(text.slice(0, 4), /◇|s/);
    assert.doesNotMatch(
      text.slice(0, 4),
      /[├|+]/,
      `no fake spine tee when parent missing: ${JSON.stringify(text.slice(0, 4))}`,
    );
    assert.doesNotMatch(text.slice(5), /^[◇s]/);
  });

  it('locks uncommitted glyph to exact Unicode ○', () => {
    assert.equal(graphGlyphs(false).uncommitted, '○');
    const text = segmentsText(
      graphUncommittedSegments(
        { kind: 'uncommitted', hasChanges: true },
        { width: 40, ascii: false },
      ),
    );
    assert.equal([...text][0], '○');
    assert.ok(text.startsWith('○ uncommitted'), text);
  });

  it('locks uncommitted glyph to exact ASCII o', () => {
    assert.equal(graphGlyphs(true).uncommitted, 'o');
    const text = segmentsText(
      graphUncommittedSegments(
        { kind: 'uncommitted', hasChanges: true },
        { width: 40, ascii: true },
      ),
    );
    assert.equal([...text][0], 'o');
    assert.ok(text.startsWith('o uncommitted'), text);
  });

  it('ascii override applies to HEAD glyph and densify rails (no Unicode mix)', () => {
    const [row] = layoutCommits([commit], { ascii: true });
    const headText = segmentsText(
      graphCommitSegments(row, {
        width: 80,
        headId: commit.id,
        nowUnix: 1,
        ascii: true,
      }),
    );
    assert.match(headText, /@/);
    assert.doesNotMatch(headText, /[●⊙○│─╭╮╰╯├┤┬┴┼]/);

    const uni = layoutCommits([
      {
        id: 'a',
        parents: ['b'],
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
    ]);
    const asciiLaid = layoutCommits(
      [
        {
          id: 'a',
          parents: ['b'],
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
      ],
      { ascii: true },
    );
    const gutter = stashRailCells(
      asciiLaid[0]!.cells.length,
      asciiLaid[0]!,
      asciiLaid[1]!,
      true,
    )
      .map((c) => c.ch)
      .join('');
    assert.match(gutter, /\|/);
    assert.doesNotMatch(gutter, /[│─╭╮╰╯├┤┬┴┼●⊙○]/);
    void uni;
  });

  it('densify rails under gutter cap match sliceCellsAroundLane of full topology', () => {
    // Wide parallel tips keep concurrent lanes before the join densifies.
    const commits: GraphCommit[] = [
      {
        id: 't0',
        parents: ['base'],
        subject: 't0',
        authorName: 'Ada',
        authorDateUnix: 90,
        refs: [],
      },
      {
        id: 't1',
        parents: ['base'],
        subject: 't1',
        authorName: 'Ada',
        authorDateUnix: 80,
        refs: [],
      },
      {
        id: 't2',
        parents: ['base'],
        subject: 't2',
        authorName: 'Ada',
        authorDateUnix: 70,
        refs: [],
      },
      {
        id: 't3',
        parents: ['base'],
        subject: 't3',
        authorName: 'Ada',
        authorDateUnix: 60,
        refs: [],
      },
      {
        id: 't4',
        parents: ['base'],
        subject: 't4',
        authorName: 'Ada',
        authorDateUnix: 50,
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
        id: 'older',
        parents: [],
        subject: 'root',
        authorName: 'Ada',
        authorDateUnix: 10,
        refs: [],
      },
    ];
    const laid = layoutCommits(commits);
    // Stash between tip rows while many sibling lanes are still live.
    const prev = laid.find((r) => r.commit.id === 't0')!;
    const next = laid.find((r) => r.commit.id === 't1')!;
    const topo = Math.max(prev.cells.length, next.cells.length);
    assert.ok(topo >= 8, `expected wide topology, got ${topo}`);
    // Pane-ish budget below topology (e.g. width 50 → gutter ~15, here force 6).
    const graphWidth = 6;
    assert.ok(topo > graphWidth);

    const full = stashRailCells(topo, prev, next);
    const capped = stashRailCells(graphWidth, prev, next);
    const expected = sliceCellsAroundLane(
      full,
      graphWidth,
      stashRailAnchorLane(prev, next),
    );
    assert.equal(stashRailAnchorLane(prev, next), prev.lane);
    assert.deepEqual(
      capped.map((c) => c.ch).join(''),
      expected.map((c) => c.ch).join(''),
      'capped densify gutter must equal windowed full-topology paint',
    );
    // Align with the windowed commit above (same slice focus / budget).
    const prevWindow = sliceCellsAroundLane(prev.cells, graphWidth, prev.lane);
    assert.equal(capped.length, prevWindow.length);
    for (let i = 0; i < capped.length; i++) {
      if (prevWindow[i]!.ch === '│' || prevWindow[i]!.ch === '|') {
        assert.ok(
          capped[i]!.ch === '│' || capped[i]!.ch === '|',
          `col ${i}: prev has vertical but stash has ${JSON.stringify(capped[i]!.ch)}`,
        );
      }
    }
  });

  it('stash window follows prev.lane (not absolute left) when focus is right', () => {
    // After densify, outerBase sits on lane 1 — clipping must shift with that lane.
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
    const laid = layoutCommits(commits);
    const prev = laid.find((r) => r.commit.id === 'outerBase')!;
    const next = laid.find((r) => r.commit.id === 'older')!;
    assert.ok(prev.lane > 0, `expected outerBase off lane 0, got ${prev.lane}`);
    const topo = prev.cells.length;
    const graphWidth = 4;
    assert.ok(topo > graphWidth);

    const full = stashRailCells(topo, prev, next);
    const capped = stashRailCells(graphWidth, prev, next);
    const expected = sliceCellsAroundLane(full, graphWidth, prev.lane);
    assert.deepEqual(
      capped.map((c) => c.ch).join(''),
      expected.map((c) => c.ch).join(''),
    );
    const absoluteLeft = full.slice(0, graphWidth).map((c) => c.ch).join('');
    assert.notEqual(
      capped.map((c) => c.ch).join(''),
      absoluteLeft,
      'window must follow prev.lane, not absolute left columns',
    );
  });

  it('densify elbows survive graphWidth clip via the same window', () => {
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
    const laid = layoutCommits(commits);
    const prev = laid.find((r) => r.commit.id === 'base')!;
    const next = laid.find((r) => r.commit.id === 'outerBase')!;
    assert.ok(prev.stemDown.some((s) => s.id === 'outerBase' && s.col === 4));
    assert.ok(next.stemUp.some((s) => s.id === 'outerBase' && s.col === 2));

    const full = stashRailCells(prev.cells.length, prev, next);
    assert.equal(full.map((c) => c.ch).join('').trimEnd(), '│ ╭─╯');

    const graphWidth = 4;
    const capped = stashRailCells(graphWidth, prev, next);
    const expected = sliceCellsAroundLane(
      full,
      graphWidth,
      stashRailAnchorLane(prev, next),
    );
    assert.deepEqual(
      capped.map((c) => c.ch).join(''),
      expected.map((c) => c.ch).join(''),
    );
    // Old bug painted absolute cols into width 4 and dropped col-4→2 elbows.
    assert.match(
      capped.map((c) => c.ch).join(''),
      /[╭╯─]/,
      `densify elbow must remain in window, got ${JSON.stringify(capped.map((c) => c.ch).join(''))}`,
    );
    assert.equal(capped.length, graphWidth);
  });
});

describe('applyCommitDensifyCells', () => {
  /** Same densify fixture as stash elbow / list densify tests (col4 → col2). */
  function densifyFixture(): {
    prev: ReturnType<typeof layoutCommits>[number];
    next: ReturnType<typeof layoutCommits>[number];
  } {
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
    const laid = layoutCommits(commits);
    const prev = laid.find((r) => r.commit.id === 'base')!;
    const next = laid.find((r) => r.commit.id === 'outerBase')!;
    return { prev, next };
  }

  it('paints densify elbows on the older commit without overwriting the node', () => {
    const { prev, next } = densifyFixture();
    assert.equal(next.edges.trimEnd(), '│ ●');
    assert.ok(prev.stemDown.some((s) => s.id === 'outerBase' && s.col === 4));
    assert.ok(next.stemUp.some((s) => s.id === 'outerBase' && s.col === 2));

    const cells = applyCommitDensifyCells(prev, next);
    const gutter = cells.map((c) => c.ch).join('').trimEnd();
    assert.match(gutter, /│ ●─╯/);
    assert.equal(
      cells.filter((c) => c.role === 'node').length,
      next.cells.filter((c) => c.role === 'node').length,
    );
  });

  it('densify commit elbows survive graphWidth clip via the same window', () => {
    const { prev, next } = densifyFixture();
    assert.ok(prev.stemDown.some((s) => s.id === 'outerBase' && s.col === 4));
    assert.ok(next.stemUp.some((s) => s.id === 'outerBase' && s.col === 2));

    const overlay = applyCommitDensifyCells(prev, next);
    assert.match(overlay.map((c) => c.ch).join('').trimEnd(), /│ ●─╯/);

    const graphWidth = 4;
    const expected = sliceCellsAroundLane(overlay, graphWidth, next.lane);
    const capped = segmentsText(
      graphCommitSegments(
        next,
        { width: 80, graphWidth, nowUnix: 1 },
        { prev },
      ),
    ).slice(0, graphWidth);

    assert.equal(
      capped,
      expected.map((c) => c.ch).join(''),
      'capped commit densify must equal windowed full overlay',
    );
    // Old bug painted absolute cols into width 4 and dropped col-4→2 elbows.
    assert.match(
      capped,
      /[╭╯─]/,
      `densify elbow must remain in window, got ${JSON.stringify(capped)}`,
    );
    assert.equal(expected.length, graphWidth);
  });
});

describe('allocateStashLeafLane', () => {
  it('picks the lowest free lane and skips reserved sibling lanes', () => {
    const live = new Set([0, 1]);
    assert.equal(allocateStashLeafLane(live, 0), 2);
    assert.equal(
      allocateStashLeafLane(live, 0, { reservedLanes: new Set([2]) }),
      3,
    );
    assert.equal(
      allocateStashLeafLane(new Set([0, 1, 2]), 0, { maxLane: 2 }),
      3,
    );
  });
});

describe('applyStashJoinCells', () => {
  it('keeps an unrelated through-rail when a join bridge crosses it', () => {
    const [parent] = layoutCommits([
      {
        id: 'p',
        parents: ['root'],
        subject: 'parent',
        authorName: 'Ada',
        authorDateUnix: 20,
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
    ]);
    const wide = parent.cells.map((c) => ({ ...c }));
    while (wide.length < 6) {
      wide.push({ ch: ' ', colorLane: null, role: 'blank' });
    }
    // Live through-rail on lane 1 (col 2) that the stash join must not overwrite.
    wide[2] = { ch: U.vertical, colorLane: 1, role: 'pipe' };
    const joined = applyStashJoinCells({ ...parent, cells: wide }, [2], false);
    const dump = joined.map((c) => c.ch).join('');
    assert.equal(
      joined[2]!.ch,
      U.cross,
      `join must compose with live rail, got ${JSON.stringify(dump)}`,
    );
    assert.equal(joined[0]!.role, 'node');
  });
});
