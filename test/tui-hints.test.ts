import assert from 'node:assert';
import { describe, it } from 'node:test';

import type { ActionGateContext } from '../src/tui/actions/gates.js';
import type { GraphActionRow } from '../src/tui/graph/actions.js';
import type { FileNode, VisibleRow } from '../src/tui/model/types.js';
import {
  HINT_CHIP_GAP,
  actionHintSegments,
  fitHintSegments,
  formatHintPlain,
  navChromeHintSegments,
  type HintSegment,
} from '../src/tui/StatusBar.js';

/** Plain join for assertions — ellipsis has no label. */
const text = (segments: HintSegment[]): string =>
  segments.map((s) => (s.label.length > 0 ? formatHintPlain(s) : s.key)).join('  ');

/** Column budget matching `fitHintSegments` (pill + optional gap + label). */
function segmentColumns(s: HintSegment): number {
  if (s.label.length === 0) return s.key.length + 2;
  return s.key.length + 2 + HINT_CHIP_GAP + s.label.length;
}

function renderedColumns(segs: HintSegment[]): number {
  if (segs.length === 0) return 0;
  return segs.reduce((n, s) => n + segmentColumns(s), 0) + Math.max(0, segs.length - 1) * 2;
}

const commitGraphRow: GraphActionRow = {
  kind: 'commit',
  commit: {
    id: 'x',
    parents: [],
    subject: 's',
    authorName: 'A',
    authorDateUnix: 1,
    refs: [],
  },
};

function dirtyFileScope(navDepth: 0 | 1): ActionGateContext {
  const file: FileNode = {
    kind: 'file',
    id: 'file:repo:src/a.ts',
    path: 'src/a.ts',
    repoPath: 'repo',
    status: 'M',
    staged: false,
    unstaged: true,
    untracked: false,
    change: { path: 'src/a.ts', unstagedStatus: 'M' },
  };
  const focused: VisibleRow = {
    id: file.id,
    depth: 1,
    node: file,
    label: file.path,
    segments: [],
    trailing: [],
  };
  return { focused, snapshots: [], navDepth };
}

describe('hint chip/label split (B7)', () => {
  it('keeps key and label separate', () => {
    const segs = actionHintSegments('file');
    const stage = segs.find((s) => s.key === 's');
    assert.ok(stage);
    assert.equal(stage!.label, 'stage');
    assert.ok(!stage!.label.includes('s '));
  });

  it('formatHintPlain joins with chip gap', () => {
    assert.equal(HINT_CHIP_GAP, 2);
    assert.equal(
      formatHintPlain({ key: 's', label: 'stage', destructive: false }),
      's' + ' '.repeat(HINT_CHIP_GAP) + 'stage',
    );
  });
});

describe('navChromeHintSegments', () => {
  it('advertises Enter focus-right when left is focused', () => {
    assert.deepEqual(navChromeHintSegments(0, 'left'), [
      { key: '⏎', label: 'focus right', destructive: false },
    ]);
  });

  it('advertises Enter drill when right is focused below the leaf', () => {
    assert.deepEqual(navChromeHintSegments(0, 'right'), [
      { key: '⏎', label: 'drill', destructive: false },
      { key: 'Esc', label: 'back', destructive: false },
    ]);
  });

  it('omits Enter drill at depth 2 right (leaf)', () => {
    assert.deepEqual(navChromeHintSegments(2, 'right'), [
      { key: 'Esc', label: 'back', destructive: false },
    ]);
  });

  it('advertises Esc pop when left-focused below depth 0', () => {
    assert.deepEqual(navChromeHintSegments(1, 'left'), [
      { key: '⏎', label: 'focus right', destructive: false },
      { key: 'Esc', label: 'back', destructive: false },
    ]);
  });
});

describe('actionHintSegments with context', () => {
  it('still lists file actions at depth 0 left', () => {
    assert.equal(
      text(actionHintSegments('file', 0, 'left')),
      's  stage  u  unstage  x  revert  f  fetch  e  edit  space  reviewed  ctrl+o  full file  S  stash',
    );
  });

  it('hides tree writes on a focused file diff (depth 0 right)', () => {
    assert.equal(
      text(actionHintSegments('file', 0, 'right')),
      'e  edit  space  reviewed  ctrl+o  full file',
    );
  });

  it('hides fetch/pull/default on depth-0 right workspace (empty pane)', () => {
    const keys = actionHintSegments('workspace', 0, 'right').map((s) => s.key);
    assert.ok(!keys.includes('f'), `unexpected fetch: ${keys.join(' ')}`);
    assert.ok(!keys.includes('p'), `unexpected pull: ${keys.join(' ')}`);
    assert.ok(!keys.includes('d'), `unexpected default: ${keys.join(' ')}`);
  });

  it('hides fetch/pull/default on depth-0 right repo (empty pane)', () => {
    const keys = actionHintSegments('repo', 0, 'right').map((s) => s.key);
    assert.ok(!keys.includes('f'), `unexpected fetch: ${keys.join(' ')}`);
    assert.ok(!keys.includes('p'), `unexpected pull: ${keys.join(' ')}`);
    assert.ok(!keys.includes('d'), `unexpected default: ${keys.join(' ')}`);
  });

  it('hides tree writes on a focused file diff at depth 2 right', () => {
    assert.equal(text(actionHintSegments('file', 2, 'right')), 'e  edit  ctrl+o  full file');
  });

  it('hides tree writes on commit-file rows at depth 1 right', () => {
    assert.equal(text(actionHintSegments('file', 1, 'right')), 'e  edit  ctrl+o  full file');
  });

  it('shows graph checkout for origin-only commit', () => {
    const bare = {
      kind: 'commit' as const,
      commit: {
        id: 'x',
        parents: [],
        subject: 's',
        authorName: 'A',
        authorDateUnix: 1,
        refs: [{ kind: 'remote' as const, name: 'origin/x', commitId: 'x' }],
      },
    };
    const hints = text(actionHintSegments('graphCommit', 1, 'left', bare));
    assert.ok(hints.includes('b  checkout'));
    assert.ok(hints.includes('c  create branch'));
  });

  it('hides graph checkout when row has no local or origin ref', () => {
    const tagged = {
      kind: 'commit' as const,
      commit: {
        id: 'x',
        parents: [],
        subject: 's',
        authorName: 'A',
        authorDateUnix: 1,
        refs: [{ kind: 'tag' as const, name: 'v1', commitId: 'x' }],
      },
    };
    const hints = text(actionHintSegments('graphCommit', 1, 'left', tagged));
    assert.ok(!hints.includes('b  checkout'));
    assert.ok(hints.includes('c  create branch'));
  });

  it('hides S on a clean graph commit with no stashes and shows it when dirty', () => {
    const keysClean = actionHintSegments('graphCommit', 1, 'left', commitGraphRow, null, {
      dirty: false,
    }).map((s) => s.key);
    assert.ok(!keysClean.includes('S'), `clean commit listed S: ${keysClean.join(' ')}`);
    const keysDirty = actionHintSegments('graphCommit', 1, 'left', commitGraphRow, null, {
      dirty: true,
    }).map((s) => s.key);
    assert.ok(keysDirty.includes('S'), `dirty commit hid S: ${keysDirty.join(' ')}`);
    const keysStash = actionHintSegments('graphCommit', 1, 'left', commitGraphRow, null, {
      latestStashRef: 'stash@{0}',
    }).map((s) => s.key);
    assert.ok(keysStash.includes('S'), `commit with stashes hid S: ${keysStash.join(' ')}`);
  });

  it('lists stash apply/drop on graphStash', () => {
    const stash = {
      kind: 'stash' as const,
      stash: {
        id: 's',
        stashRef: 'stash@{0}',
        index: 0,
        subject: 'w',
        authorDateUnix: 1,
        parentId: '',
      },
    };
    assert.equal(
      text(actionHintSegments('graphStash', 1, 'left', stash)),
      'S  stash  a  apply stash  p  pop stash  D  drop stash',
    );
  });

  it('omits S on the right pane at depth 1 for file and graph kinds', () => {
    for (const kind of ['file', 'graphCommit', 'graphStash', 'graphUncommitted'] as const) {
      const keys = actionHintSegments(kind, 1, 'right').map((s) => s.key);
      assert.ok(!keys.includes('S'), `${kind} right listed S: ${keys.join(' ')}`);
    }
  });

  it('lists S on depth 0/1 left dirty file and graph when gates allow', () => {
    const dirty = dirtyFileScope(0);
    assert.ok(
      actionHintSegments('file', 0, 'left', null, dirty)
        .map((s) => s.key)
        .includes('S'),
    );
    assert.ok(
      actionHintSegments('file', 1, 'left', null, dirtyFileScope(1))
        .map((s) => s.key)
        .includes('S'),
    );
    assert.ok(
      actionHintSegments('graphStash', 1, 'left')
        .map((s) => s.key)
        .includes('S'),
    );
    assert.ok(
      actionHintSegments('graphUncommitted', 1, 'left')
        .map((s) => s.key)
        .includes('S'),
    );
  });

  it('keeps S on a dirty file even when a commit graph row is passed', () => {
    const keys = actionHintSegments('file', 0, 'left', commitGraphRow, dirtyFileScope(0)).map(
      (s) => s.key,
    );
    assert.ok(keys.includes('S'), `expected S on dirty file: ${keys.join(' ')}`);
  });

  it('lists graph checkout/create at depth 0 right, not tree pull/push/branch', () => {
    const withLocal = {
      kind: 'commit' as const,
      commit: {
        id: 'x',
        parents: [],
        subject: 's',
        authorName: 'A',
        authorDateUnix: 1,
        refs: [{ kind: 'local' as const, name: 'feat', commitId: 'x' }],
      },
    };
    const hints = text(actionHintSegments('graphCommit', 0, 'right', withLocal));
    assert.ok(hints.includes('b  checkout'));
    assert.ok(hints.includes('c  create branch'));
    assert.ok(!hints.includes('p  pull'));
    assert.ok(!hints.includes('P  push'));
    assert.ok(!hints.includes('b  branch'));
  });

  it('still lists tree writes on a repo at depth 0 left', () => {
    const hints = text(actionHintSegments('repo', 0, 'left'));
    assert.ok(hints.includes('p  pull'));
    assert.ok(hints.includes('P  push'));
    assert.ok(hints.includes('b  branch'));
    assert.ok(!hints.includes('checkout'));
  });

  it('hides stage when scope gate says nothing to stage', () => {
    const scope = {
      focused: null,
      snapshots: [] as const,
      navDepth: 0 as const,
    };
    const keys = actionHintSegments('file', 0, 'left', null, scope).map((s) => s.key);
    assert.ok(!keys.includes('s'));
    assert.ok(!keys.includes('u'));
    assert.ok(!keys.includes('x'));
    assert.ok(keys.includes('f'));
    assert.ok(keys.includes('e'));
  });
});

describe('actionHintSegments', () => {
  it('lists the workspace actions in registry order', () => {
    assert.equal(
      text(actionHintSegments('workspace', 0, 'left')),
      'f  fetch  p  pull  d  default branch',
    );
  });

  it('lists every file action, including the ctrl chord', () => {
    assert.equal(
      text(actionHintSegments('file', 0, 'left')),
      's  stage  u  unstage  x  revert  f  fetch  e  edit  space  reviewed  ctrl+o  full file  S  stash',
    );
  });

  it('returns nothing for group rows, which accept no actions', () => {
    assert.deepEqual(actionHintSegments('group', 0, 'left'), []);
  });

  it('marks revert as the only destructive hint', () => {
    const destructive = actionHintSegments('file', 0, 'left')
      .filter((s) => s.destructive)
      .map((s) => ({ key: s.key, label: s.label }));
    assert.deepEqual(destructive, [{ key: 'x', label: 'revert' }]);
  });
});

describe('fitHintSegments', () => {
  const segments = actionHintSegments('file', 0, 'left');

  it('keeps every hint when the width allows it', () => {
    assert.deepEqual(fitHintSegments(segments, 200), segments);
  });

  it('drops trailing hints and marks the truncation', () => {
    // Two file hints: (3+2+5)+(2)+(3+2+7) = 10+2+12 = 24; + sep + … pill = 24+2+3 = 29.
    const twoPlusEllipsis = 24 + 2 + (1 + 2);
    assert.equal(text(fitHintSegments(segments, twoPlusEllipsis)), 's  stage  u  unstage  …');
  });

  it('never exceeds the available width', () => {
    for (let width = 0; width <= 100; width += 1) {
      const fitted = fitHintSegments(segments, width);
      const cols = renderedColumns(fitted);
      assert.ok(cols <= width, `width ${width} produced ${cols} columns: ${text(fitted)}`);
    }
  });

  it('renders nothing when there is no room at all', () => {
    assert.deepEqual(fitHintSegments(segments, 3), []);
  });
});
