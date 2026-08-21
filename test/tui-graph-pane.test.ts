import assert from 'node:assert';
import { describe, it } from 'node:test';
import { graphViewportStart, visibleGraphWindow } from '../src/tui/GraphPane.js';
import { commitMetaStubLines } from '../src/tui/CommitMetaStub.js';
import type { GraphListRow } from '../src/tui/graph/list.js';
import {
  graphChromeBudget,
  graphSelectionDetailLines,
  graphSyncHeaderSegments,
} from '../src/tui/graph/selectionDetail.js';
import { graphGlyphs } from '../src/tui/graph/glyphs.js';
import { DEFAULT_GRAPH_WINDOW, type GraphModel } from '../src/tui/graph/types.js';
import { segmentsText } from '../src/tui/theme.js';
import { commitDetailMetaFromRow } from '../src/tui/commitFiles/meta.js';

describe('graphViewportStart', () => {
  it('clamps like the tree viewport', () => {
    assert.equal(graphViewportStart(100, 0, 10), 0);
    assert.equal(graphViewportStart(100, 99, 10), 90);
    assert.equal(graphViewportStart(5, 2, 10), 0);
  });
});

describe('visibleGraphWindow', () => {
  it('uses chrome listHeight, not the full pane height', () => {
    const rows = Array.from({ length: 100 }, (_, i) => ({ id: `c${i}` }));
    const paneHeight = 8;
    const chrome = graphChromeBudget(paneHeight, false, true);
    assert.equal(chrome.listHeight, 5);
    const win = visibleGraphWindow(rows, 50, paneHeight, false, true);
    assert.equal(win.listHeight, 5);
    assert.equal(win.visible.length, 5);
    assert.equal(win.start, graphViewportStart(100, 50, chrome.listHeight));
    assert.notEqual(win.start, graphViewportStart(100, 50, paneHeight));
    assert.equal(win.visible[0]?.id, `c${win.start}`);
  });

  it('matches GraphPane paint when header is omitted', () => {
    const rows = Array.from({ length: 40 }, (_, i) => ({ id: `c${i}` }));
    const win = visibleGraphWindow(rows, 0, 8, false, false);
    const chrome = graphChromeBudget(8, false, false);
    assert.equal(win.listHeight, chrome.listHeight);
    assert.equal(win.visible.length, chrome.listHeight);
    assert.equal(win.start, 0);
  });
});

describe('graphChromeBudget', () => {
  it('prefers footer over header; both when height allows', () => {
    assert.deepEqual(graphChromeBudget(2), { header: true, footer: false, listHeight: 1 });
    // avail 3 → footer + list; header dropped
    assert.deepEqual(graphChromeBudget(3), { header: false, footer: true, listHeight: 1 });
    assert.deepEqual(graphChromeBudget(4), { header: true, footer: true, listHeight: 1 });
    assert.deepEqual(graphChromeBudget(5), { header: true, footer: true, listHeight: 2 });
    assert.deepEqual(graphChromeBudget(8), { header: true, footer: true, listHeight: 5 });
  });

  it('drops header before footer when loadingOlder eats a line', () => {
    // height 4 − loadingOlder → avail 3 → footer only
    assert.deepEqual(graphChromeBudget(4, true), {
      header: false,
      footer: true,
      listHeight: 1,
    });
    // height 5 − loadingOlder → avail 4 → header + footer
    assert.deepEqual(graphChromeBudget(5, true), {
      header: true,
      footer: true,
      listHeight: 1,
    });
  });

  it('does not reserve header when wantHeader is false', () => {
    assert.deepEqual(graphChromeBudget(5, false, false), {
      header: false,
      footer: true,
      listHeight: 3,
    });
    assert.deepEqual(graphChromeBudget(2, false, false), {
      header: false,
      footer: false,
      listHeight: 2,
    });
  });
});

describe('graphSyncHeaderSegments', () => {
  it('shows branch and sync mark', () => {
    const segs = graphSyncHeaderSegments(
      { branch: 'main', syncStatus: 'ahead', syncNote: 'ahead by 2' },
      { width: 40 },
    );
    const text = segmentsText(segs);
    assert.match(text, /main/);
    assert.ok(text.length > 4);
  });
});

describe('graphSelectionDetailLines', () => {
  const model: GraphModel = {
    repoPath: '/ws/rs',
    commits: [
      {
        id: 'abcdef1234567890',
        parents: [],
        subject: 'full subject that should not be truncated harshly',
        authorName: 'Ada',
        authorDateUnix: 1_700_000_000,
        refs: [
          { kind: 'local', name: 'feature/x', commitId: 'abcdef1234567890' },
          { kind: 'tag', name: 'v1', commitId: 'abcdef1234567890' },
        ],
      },
    ],
    stashes: [
      {
        id: 's1',
        stashRef: 'stash@{0}',
        index: 0,
        subject: 'wip stash',
        authorDateUnix: 1_700_000_050,
        parentId: 'abcdef1234567890',
      },
    ],
    uncommitted: { kind: 'uncommitted', hasChanges: true },
    headId: 'abcdef1234567890',
    refsFingerprint: 'fp',
    skip: 0,
    limit: DEFAULT_GRAPH_WINDOW,
    hasMore: false,
  };

  it('footer shows full subject and coloured refs', () => {
    const row: GraphListRow = {
      id: 'graph:commit:abcdef1234567890',
      kind: 'commit',
      commitId: 'abcdef1234567890',
      segments: [],
    };
    const { footer } = graphSelectionDetailLines(row, model, {
      width: 80,
      nowUnix: 1_700_000_100,
      refLocalColor: '#local',
      refTagColor: '#tag',
      subjectColor: '#sub',
    });
    assert.equal(footer.length, 2);
    assert.equal(segmentsText(footer[0]!), model.commits[0]!.subject);
    assert.equal(footer[0]![0]!.color, '#sub');
    assert.match(segmentsText(footer[1]!), /\[feature\/x\]/);
    assert.ok(footer[1]!.some((s) => s.text === 'feature/x' && s.color === '#local'));
    assert.ok(footer[1]!.some((s) => s.text === '[v1]' && s.color === '#tag'));
    assert.match(segmentsText(footer[1]!), /abcdef1/);
  });

  it('footer keeps ref colours when width forces truncation', () => {
    const longModel: GraphModel = {
      ...model,
      commits: [
        {
          ...model.commits[0]!,
          refs: [
            {
              kind: 'local',
              name: 'feature/extremely-long-branch-name-for-footer',
              commitId: 'abcdef1234567890',
            },
            {
              kind: 'tag',
              name: 'v9.9.9-very-long-tag-name',
              commitId: 'abcdef1234567890',
            },
          ],
        },
      ],
    };
    const row: GraphListRow = {
      id: 'graph:commit:abcdef1234567890',
      kind: 'commit',
      commitId: 'abcdef1234567890',
      segments: [],
    };
    const { footer } = graphSelectionDetailLines(row, longModel, {
      width: 28,
      nowUnix: 1_700_000_100,
      mutedColor: '#muted',
      refLocalColor: '#local',
      refTagColor: '#tag',
      subjectColor: '#sub',
    });
    const line2 = footer[1]!;
    assert.ok(
      line2.some((s) => s.color === '#local' || s.color === '#tag'),
      'narrow footer must retain chip colour',
    );
    assert.ok(
      !line2.some(
        (s) =>
          s.color === '#muted' &&
          (s.text.includes('feature/') || s.text.includes('v9.9.9')),
      ),
      'must not flatten truncated footer refs to muted',
    );
  });

  it('footer prefixes checkout mark when selection is HEAD on a named branch', () => {
    const tip = model.commits[0]!;
    const row: GraphListRow = {
      id: `graph:commit:${tip.id}`,
      kind: 'commit',
      commitId: tip.id,
      segments: [],
    };
    const { footer } = graphSelectionDetailLines(row, model, {
      width: 120,
      nowUnix: 1_700_000_100,
      headBranch: 'feature/x',
      headMarkColor: '#head',
      refLocalColor: '#local',
      refTagColor: '#tag',
      subjectColor: '#sub',
    });
    assert.ok(segmentsText(footer[1]!).includes(`[${graphGlyphs(false).checkoutMark}feature/x]`));
    assert.doesNotMatch(segmentsText(footer[1]!), /\[HEAD\]/);
    const mark = footer[1]!.find((s) => s.text === graphGlyphs(false).checkoutMark);
    assert.ok(mark);
    assert.equal(mark.color, '#head');
    assert.equal(mark.bold, true);
  });

  it('footer summarises uncommitted and stash', () => {
    const u = graphSelectionDetailLines(
      { id: 'graph:uncommitted', kind: 'uncommitted', commitId: null, segments: [] },
      model,
      { width: 40 },
    );
    assert.match(segmentsText(u.footer[0]!), /Uncommitted/i);

    const s = graphSelectionDetailLines(
      {
        id: 'graph:stash:stash@{0}',
        kind: 'stash',
        commitId: 's1',
        stashRef: 'stash@{0}',
        segments: [],
      },
      model,
      { width: 40, nowUnix: 1_700_000_100 },
    );
    assert.match(segmentsText(s.footer[0]!), /wip stash/);
    assert.match(segmentsText(s.footer[1]!), /stash@\{0\}/);
  });
});

describe('commitDetailMetaFromRow', () => {
  it('includes ref names in commit subtitle', () => {
    const model: GraphModel = {
      repoPath: '/ws/rs',
      commits: [
        {
          id: 'abcdef1234567890',
          parents: [],
          subject: 'tip',
          authorName: 'Ada',
          authorDateUnix: 1,
          refs: [{ kind: 'local', name: 'main', commitId: 'abcdef1234567890' }],
        },
      ],
      stashes: [],
      uncommitted: null,
      headId: 'abcdef1234567890',
      refsFingerprint: 'fp',
      skip: 0,
      limit: DEFAULT_GRAPH_WINDOW,
      hasMore: false,
    };
    const meta = commitDetailMetaFromRow(
      {
        id: 'graph:commit:abcdef1234567890',
        kind: 'commit',
        commitId: 'abcdef1234567890',
        segments: [],
      },
      '/ws/rs',
      model,
    );
    assert.match(meta.subtitle ?? '', /main/);
    assert.match(meta.subtitle ?? '', /tip/);
  });
});

describe('commitMetaStubLines', () => {
  it('describes a commit row', () => {
    const row: GraphListRow = {
      id: 'graph:commit:abcdef0',
      kind: 'commit',
      commitId: 'abcdef012345',
      segments: [{ text: '[main] tip', color: '#fff' }],
    };
    const lines = commitMetaStubLines(row, '/ws/rs');
    assert.ok(lines.some((l) => l.includes('rs')));
    assert.ok(lines.some((l) => /abcdef0/i.test(l)));
    assert.ok(lines.some((l) => /commit/i.test(l)));
  });

  it('prompts when nothing selected', () => {
    const lines = commitMetaStubLines(null, '/ws/rs');
    assert.ok(lines.some((l) => /select/i.test(l)));
  });
});
