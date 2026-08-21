import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  graphCursorAfterRowsReload,
  graphFlashDecision,
  graphFlashMetaFromModel,
  graphRemovalGhosts,
  graphRowFlashId,
  graphRowFlashIds,
  graphRowId,
  graphRowIdentity,
  graphRowSignatures,
  graphStashSpacerId,
  isNewGraphRowSet,
  shouldResetGraphCursor,
  type GraphFlashMeta,
  type GraphListRow,
} from '../src/tui/graph/list.js';
import { listRowBackground } from '../src/tui/listEmphasis.js';
import { flashBackground, getTheme } from '../src/tui/theme.js';
import {
  FLASH_MS,
  changedNodeIds,
  flashableNodeIds,
  flashStrength,
  mergeGhostRows,
} from '../src/tui/watch.js';
import { DEFAULT_GRAPH_WINDOW, type GraphCommit, type GraphModel } from '../src/tui/graph/types.js';

function commitOf(id: string, opts: Partial<GraphCommit> = {}): GraphCommit {
  return {
    id,
    parents: [],
    subject: opts.subject ?? `subject-${id}`,
    authorName: 'Ada',
    authorDateUnix: 1_700_000_000,
    refs: opts.refs ?? [],
    ...opts,
    id,
  };
}

function modelOf(partial: Partial<GraphModel> = {}): GraphModel {
  const commits = partial.commits ?? [commitOf('aaa')];
  return {
    repoPath: '/ws/rs',
    commits,
    stashes: partial.stashes ?? [],
    uncommitted: partial.uncommitted === undefined ? null : partial.uncommitted,
    headId: partial.headId === undefined ? (commits[0]?.id ?? null) : partial.headId,
    refsFingerprint: 'fp',
    skip: 0,
    limit: DEFAULT_GRAPH_WINDOW,
    hasMore: false,
    ...partial,
    commits: partial.commits ?? commits,
  };
}

function commitRow(id: string, segments: GraphListRow['segments'] = []): GraphListRow {
  return { id: graphRowId('commit', id), kind: 'commit', commitId: id, segments };
}

function commitSpacer(id: string, segments: GraphListRow['segments'] = []): GraphListRow {
  return { id: graphRowId('spacer', id), kind: 'spacer', commitId: null, segments };
}

function stashRow(ref: string, commitId: string): GraphListRow {
  return {
    id: graphRowId('stash', ref),
    kind: 'stash',
    commitId,
    stashRef: ref,
    segments: [],
  };
}

function stashSpacer(ref: string): GraphListRow {
  return {
    id: graphStashSpacerId(ref),
    kind: 'spacer',
    commitId: null,
    stashRef: ref,
    segments: [],
  };
}

function uncommittedRow(): GraphListRow {
  return { id: graphRowId('uncommitted', 'wt'), kind: 'uncommitted', commitId: null, segments: [] };
}

function flashMeta(over: Partial<GraphFlashMeta> = {}): GraphFlashMeta {
  return {
    repoPath: '/ws/rs',
    skip: 0,
    limit: DEFAULT_GRAPH_WINDOW,
    commitIds: ['aaa'],
    ...over,
  };
}

describe('graphRowSignatures', () => {
  it('signs commits as subject|sortedRefNames|isHead and omits spacers', () => {
    const rows = [
      commitRow('aaa', [{ text: 'gutter-wide', color: '#fff' }]),
      commitSpacer('aaa', [{ text: 'spacer-gutter', color: '#fff' }]),
    ];
    const model = modelOf({
      commits: [
        commitOf('aaa', {
          subject: 'tip',
          refs: [
            { kind: 'tag', name: 'v1', commitId: 'aaa' },
            { kind: 'local', name: 'main', commitId: 'aaa' },
          ],
        }),
      ],
      headId: 'aaa',
    });
    const sigs = graphRowSignatures(rows, model);
    assert.equal(sigs.get(graphRowIdentity(commitRow('aaa'), model.repoPath)), 'tip|main,v1|true');
    assert.equal(sigs.has('graph:spacer:aaa'), false);
  });

  it('signs stashes as subject|parentId and uncommitted as hasChanges', () => {
    const rows = [uncommittedRow(), stashRow('stash@{0}', 's1'), stashSpacer('stash@{0}')];
    const model = modelOf({
      commits: [commitOf('aaa')],
      stashes: [
        {
          id: 's1',
          stashRef: 'stash@{0}',
          index: 0,
          subject: 'wip',
          authorName: 'Ada',
          authorDateUnix: 1,
          parentId: 'aaa',
        },
      ],
      uncommitted: { kind: 'uncommitted', hasChanges: true },
    });
    const sigs = graphRowSignatures(rows, model);
    assert.equal(sigs.get(graphRowIdentity(uncommittedRow(), model.repoPath)), 'true');
    assert.equal(
      sigs.get(graphRowIdentity(stashRow('stash@{0}', 's1'), model.repoPath)),
      'wip|aaa',
    );
    assert.equal(sigs.has(graphStashSpacerId('stash@{0}')), false);
  });

  it('does not change when only painted segments change', () => {
    const model = modelOf({
      commits: [commitOf('aaa', { subject: 'tip' })],
    });
    const narrow = graphRowSignatures([commitRow('aaa', [{ text: '|', color: '#1' }])], model);
    const wide = graphRowSignatures(
      [commitRow('aaa', [{ text: '| | | | |', color: '#1' }])],
      model,
    );
    const id = graphRowIdentity(commitRow('aaa'), model.repoPath);
    assert.equal(narrow.get(id), wide.get(id));
    assert.ok(narrow.has(id));
    assert.deepEqual(changedNodeIds(narrow, wide), []);
  });

  it('reports subject, refs, and HEAD moves as changed commit ids', () => {
    const rows = [commitRow('aaa'), commitRow('bbb')];
    const before = graphRowSignatures(
      rows,
      modelOf({
        commits: [
          commitOf('aaa', {
            subject: 'old',
            refs: [{ kind: 'local', name: 'main', commitId: 'aaa' }],
          }),
          commitOf('bbb', { subject: 'other' }),
        ],
        headId: 'aaa',
      }),
    );
    const after = graphRowSignatures(
      rows,
      modelOf({
        commits: [
          commitOf('aaa', { subject: 'new', refs: [] }),
          commitOf('bbb', {
            subject: 'other',
            refs: [{ kind: 'local', name: 'main', commitId: 'bbb' }],
          }),
        ],
        headId: 'bbb',
      }),
    );
    assert.deepEqual(changedNodeIds(before, after).sort(), [
      graphRowIdentity(commitRow('aaa'), '/ws/rs'),
      graphRowIdentity(commitRow('bbb'), '/ws/rs'),
    ]);
  });
});

describe('graphFlashDecision + flashableNodeIds', () => {
  const meta = flashMeta();
  const focused = meta.repoPath;

  it('keys flash meta from the model repo and commit ids', () => {
    const model = modelOf({
      repoPath: '/ws/a',
      skip: 0,
      limit: 300,
      commits: [commitOf('aaa'), commitOf('bbb')],
    });
    assert.deepEqual(graphFlashMetaFromModel(model), {
      repoPath: '/ws/a',
      skip: 0,
      limit: 300,
      commitIds: ['aaa', 'bbb'],
    });
  });

  it('omits extra stash parents from flash commitIds so autoload stays a prefix', () => {
    const prev = graphFlashMetaFromModel(
      modelOf({
        commits: [commitOf('aaa'), commitOf('bbb'), commitOf('extra')],
        windowCount: 2,
      }),
    );
    assert.deepEqual(prev.commitIds, ['aaa', 'bbb']);
    const next = graphFlashMetaFromModel(
      modelOf({
        commits: [commitOf('aaa'), commitOf('bbb'), commitOf('ccc'), commitOf('extra')],
        windowCount: 3,
      }),
    );
    assert.deepEqual(next.commitIds, ['aaa', 'bbb', 'ccc']);
    assert.deepEqual(
      graphFlashDecision({
        focusedRepo: focused,
        beforeSize: 1,
        prevRowCount: 2,
        nextRowCount: 4,
        prev,
        next,
      }),
      { stale: false, seed: false, includeAdds: false },
    );
  });

  it('seeds (no flash) on empty before, repo switch, or first non-empty paint', () => {
    assert.deepEqual(
      graphFlashDecision({
        focusedRepo: focused,
        beforeSize: 0,
        prevRowCount: 0,
        nextRowCount: 4,
        prev: null,
        next: meta,
      }),
      { stale: false, seed: true, includeAdds: true },
    );
    assert.deepEqual(
      graphFlashDecision({
        focusedRepo: focused,
        beforeSize: 3,
        prevRowCount: 4,
        nextRowCount: 4,
        prev: flashMeta({ repoPath: '/ws/other' }),
        next: meta,
      }),
      { stale: false, seed: true, includeAdds: true },
    );
    assert.deepEqual(
      graphFlashDecision({
        focusedRepo: focused,
        beforeSize: 3,
        prevRowCount: 0,
        nextRowCount: 4,
        prev: meta,
        next: meta,
      }),
      { stale: false, seed: true, includeAdds: true },
    );
  });

  it('does not stamp flashes on empty before even when ids look new', () => {
    const after = new Map([['graph:commit:aaa', 'tip||true']]);
    const decision = graphFlashDecision({
      focusedRepo: focused,
      beforeSize: 0,
      prevRowCount: 0,
      nextRowCount: 2,
      prev: null,
      next: meta,
    });
    const ids = decision.seed
      ? []
      : flashableNodeIds(new Map(), after, { includeAdds: decision.includeAdds });
    assert.deepEqual(ids, []);
  });

  it('autoload (same skip/limit, older ids appended) skips pure adds but flashes in-place changes', () => {
    const decision = graphFlashDecision({
      focusedRepo: focused,
      beforeSize: 1,
      prevRowCount: 2,
      nextRowCount: 4,
      prev: flashMeta({ skip: 0, limit: 300, commitIds: ['aaa'] }),
      next: flashMeta({ skip: 0, limit: 300, commitIds: ['aaa', 'older'] }),
    });
    assert.deepEqual(decision, { stale: false, seed: false, includeAdds: false });

    const before = new Map([['graph:commit:aaa', 'old||true']]);
    const after = new Map([
      ['graph:commit:aaa', 'new||true'],
      ['graph:commit:older', 'ancient||false'],
    ]);
    assert.deepEqual(flashableNodeIds(before, after, { includeAdds: false }).sort(), [
      'graph:commit:aaa',
    ]);
  });

  it('new tip ids (not a commit-id prefix) still includeAdds', () => {
    const decision = graphFlashDecision({
      focusedRepo: focused,
      beforeSize: 2,
      prevRowCount: 4,
      nextRowCount: 6,
      prev: flashMeta({ skip: 0, limit: 300, commitIds: ['aaa', 'bbb'] }),
      next: flashMeta({ skip: 0, limit: 300, commitIds: ['tip', 'aaa', 'bbb'] }),
    });
    assert.deepEqual(decision, { stale: false, seed: false, includeAdds: true });
  });

  it('skips signature/flash updates while the painted model is for another repo', () => {
    const modelA = flashMeta({ repoPath: '/ws/a', commitIds: ['aaa'] });
    assert.deepEqual(
      graphFlashDecision({
        focusedRepo: '/ws/b',
        beforeSize: 2,
        prevRowCount: 4,
        nextRowCount: 4,
        prev: modelA,
        next: modelA,
      }),
      { stale: true, seed: false, includeAdds: false },
    );
  });

  it('seeds the new repo model after a focus change instead of comparing A to B', () => {
    assert.deepEqual(
      graphFlashDecision({
        focusedRepo: '/ws/b',
        beforeSize: 2,
        prevRowCount: 4,
        nextRowCount: 4,
        prev: flashMeta({ repoPath: '/ws/a', commitIds: ['aaa'] }),
        next: flashMeta({ repoPath: '/ws/b', commitIds: ['bbb'] }),
      }),
      { stale: false, seed: true, includeAdds: true },
    );
  });

  it('watch/invalidate (same window) flashes adds, changes, and removes', () => {
    const decision = graphFlashDecision({
      focusedRepo: focused,
      beforeSize: 2,
      prevRowCount: 4,
      nextRowCount: 4,
      prev: meta,
      next: meta,
    });
    assert.deepEqual(decision, { stale: false, seed: false, includeAdds: true });

    const before = new Map([
      ['graph:commit:aaa', 'old||true'],
      ['graph:commit:gone', 'x||false'],
    ]);
    const after = new Map([
      ['graph:commit:aaa', 'new||true'],
      ['graph:commit:tip', 'fresh||false'],
    ]);
    assert.deepEqual(flashableNodeIds(before, after).sort(), [
      'graph:commit:aaa',
      'graph:commit:gone',
      'graph:commit:tip',
    ]);
  });
});

describe('graphRowFlashId + graphRowIdentity + listRowBackground', () => {
  it('maps a spacer to its paired commit or stash id', () => {
    assert.equal(graphRowFlashId(commitRow('aaa')), 'graph:commit:aaa');
    assert.equal(graphRowFlashId(commitSpacer('aaa')), 'graph:commit:aaa');
    assert.equal(graphRowFlashId(stashRow('stash@{0}', 's1')), 'graph:stash:stash@{0}');
    assert.equal(graphRowFlashId(stashSpacer('stash@{0}')), 'graph:stash:stash@{0}');
    assert.equal(graphRowFlashId(uncommittedRow()), 'graph:uncommitted');
  });

  it('scopes flash identity by repo so the same sha is a different row', () => {
    const row = commitRow('aaa');
    assert.equal(graphRowIdentity(row, '/ws/a'), '/ws/a#graph:commit:aaa');
    assert.equal(graphRowIdentity(row, '/ws/b'), '/ws/b#graph:commit:aaa');
    assert.notEqual(
      graphRowIdentity(uncommittedRow(), '/ws/a'),
      graphRowIdentity(uncommittedRow(), '/ws/b'),
    );
  });

  it('paints flash from repo-scoped identity for commit and spacer; selected still wins', () => {
    const flashes = new Map([[graphRowIdentity(commitRow('aaa'), '/ws/rs'), 1000]]);
    const now = 1000;
    const cursorBg = getTheme().palette.cursorBg;
    const commit = commitRow('aaa');
    const spacer = commitSpacer('aaa');
    const paint = (row: GraphListRow, selected: boolean): string | undefined => {
      const flashedAt = flashes.get(graphRowIdentity(row, '/ws/rs'));
      return listRowBackground({
        selected,
        cursorBg,
        flashBg: flashBackground(flashStrength(flashedAt, now)),
      });
    };
    const expectedFlash = flashBackground(flashStrength(1000, now));
    assert.ok(expectedFlash);
    assert.equal(paint(commit, false), expectedFlash);
    assert.equal(paint(spacer, false), expectedFlash);
    assert.equal(paint(commit, true), cursorBg);
  });
});

describe('graphRowFlashIds + isNewGraphRowSet', () => {
  it('treats a disjoint identity set as a new row list (no add/remove flash)', () => {
    const repoA = modelOf({
      repoPath: '/ws/a',
      commits: [commitOf('aaa'), commitOf('bbb')],
      uncommitted: { kind: 'uncommitted', hasChanges: true },
    });
    const repoB = modelOf({
      repoPath: '/ws/b',
      commits: [commitOf('ccc'), commitOf('ddd')],
      uncommitted: { kind: 'uncommitted', hasChanges: true },
    });
    const before = graphRowSignatures(
      [uncommittedRow(), commitRow('aaa'), commitRow('bbb')],
      repoA,
    );
    const after = graphRowSignatures([uncommittedRow(), commitRow('ccc'), commitRow('ddd')], repoB);
    assert.equal(isNewGraphRowSet(before, after), true);
    assert.deepEqual(graphRowFlashIds(before, after), []);
    assert.deepEqual(flashableNodeIds(before, after).length > 0, true);
  });

  it('still flashes when the same-identity row appears, updates, or leaves', () => {
    const repo = '/ws/rs';
    const rows = [uncommittedRow(), commitRow('aaa'), commitRow('bbb')];
    const before = graphRowSignatures(
      rows,
      modelOf({
        commits: [commitOf('aaa', { subject: 'old' }), commitOf('bbb', { subject: 'keep' })],
        uncommitted: { kind: 'uncommitted', hasChanges: false },
      }),
    );
    const after = graphRowSignatures(
      [uncommittedRow(), commitRow('aaa'), commitRow('ccc')],
      modelOf({
        commits: [commitOf('aaa', { subject: 'new' }), commitOf('ccc', { subject: 'tip' })],
        uncommitted: { kind: 'uncommitted', hasChanges: true },
      }),
    );
    assert.equal(isNewGraphRowSet(before, after), false);
    assert.deepEqual(graphRowFlashIds(before, after).sort(), [
      graphRowIdentity(commitRow('aaa'), repo),
      graphRowIdentity(commitRow('bbb'), repo),
      graphRowIdentity(commitRow('ccc'), repo),
      graphRowIdentity(uncommittedRow(), repo),
    ]);
  });

  it('does not flash a same-sha commit that only exists after a repo switch', () => {
    const before = graphRowSignatures(
      [commitRow('aaa')],
      modelOf({ repoPath: '/ws/a', commits: [commitOf('aaa')] }),
    );
    const after = graphRowSignatures(
      [commitRow('aaa')],
      modelOf({ repoPath: '/ws/b', commits: [commitOf('aaa')] }),
    );
    assert.equal(isNewGraphRowSet(before, after), true);
    assert.deepEqual(graphRowFlashIds(before, after), []);
  });
});

describe('graph removal ghosts', () => {
  it('keeps disappeared selectable graph rows (and paired spacers) for FLASH_MS', () => {
    const live: GraphListRow[] = [commitRow('aaa'), commitSpacer('aaa')];
    const prev: GraphListRow[] = [
      commitRow('aaa'),
      commitSpacer('aaa'),
      commitRow('bbb'),
      commitSpacer('bbb'),
    ];
    const ghosts = graphRemovalGhosts(
      prev,
      [graphRowIdentity(commitRow('bbb'), '/ws/rs')],
      1000,
      '/ws/rs',
    );
    assert.deepEqual(
      ghosts.map((g) => g.id),
      ['graph:commit:bbb', 'graph:spacer:bbb'],
    );
    const merged = mergeGhostRows(live, ghosts, 1000 + FLASH_MS / 2);
    assert.deepEqual(
      merged.map((r) => r.id),
      ['graph:commit:aaa', 'graph:spacer:aaa', 'graph:commit:bbb', 'graph:spacer:bbb'],
    );
    const after = mergeGhostRows(live, ghosts, 1000 + FLASH_MS);
    assert.deepEqual(
      after.map((r) => r.id),
      ['graph:commit:aaa', 'graph:spacer:aaa'],
    );
  });
});

describe('shouldResetGraphCursor + graphCursorAfterRowsReload', () => {
  const meta = flashMeta({ commitIds: ['aaa', 'bbb'] });

  it('resets on first paint / seed and repo switch', () => {
    assert.equal(
      shouldResetGraphCursor({ stale: false, seed: true, prev: null, next: meta }),
      true,
    );
    assert.equal(
      shouldResetGraphCursor({
        stale: false,
        seed: true,
        prev: flashMeta({ repoPath: '/ws/other', commitIds: ['zzz'] }),
        next: meta,
      }),
      true,
    );
  });

  it('keeps the cursor on a same-window tiny refresh', () => {
    assert.equal(
      shouldResetGraphCursor({ stale: false, seed: false, prev: meta, next: meta }),
      false,
    );
  });

  it('keeps the cursor when autoload appends older commits', () => {
    assert.equal(
      shouldResetGraphCursor({
        stale: false,
        seed: false,
        prev: flashMeta({ skip: 0, limit: 300, commitIds: ['aaa'] }),
        next: flashMeta({ skip: 0, limit: 300, commitIds: ['aaa', 'older'] }),
      }),
      false,
    );
  });

  it('resets when a poll reload prepends new tips', () => {
    assert.equal(
      shouldResetGraphCursor({
        stale: false,
        seed: false,
        prev: flashMeta({ skip: 0, limit: 300, commitIds: ['aaa', 'bbb'] }),
        next: flashMeta({ skip: 0, limit: 300, commitIds: ['tip', 'aaa', 'bbb'] }),
      }),
      true,
    );
  });

  it('does not reset while the painted model is stale for another repo', () => {
    assert.equal(
      shouldResetGraphCursor({
        stale: true,
        seed: false,
        prev: meta,
        next: flashMeta({ repoPath: '/ws/other', commitIds: ['zzz'] }),
      }),
      false,
    );
  });

  it('snaps to first selectable on reset and keeps nearest on same-view', () => {
    const rows: GraphListRow[] = [
      { id: graphRowId('commit', 'aaa'), kind: 'commit', commitId: 'aaa', segments: [] },
      { id: graphRowId('spacer', 'aaa'), kind: 'spacer', commitId: null, segments: [] },
      { id: graphRowId('commit', 'bbb'), kind: 'commit', commitId: 'bbb', segments: [] },
      { id: graphRowId('spacer', 'bbb'), kind: 'spacer', commitId: null, segments: [] },
    ];
    assert.equal(graphCursorAfterRowsReload(rows, 2, true), 0);
    assert.equal(graphCursorAfterRowsReload(rows, 2, false), 2);
    assert.equal(graphCursorAfterRowsReload(rows, 1, false), 2);
  });
});
