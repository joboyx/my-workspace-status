import assert from 'node:assert';
import { describe, it } from 'node:test';
import { visibleWidth } from '../src/helpers.js';
import {
  CURSOR_BAR,
  FOLD_COLLAPSED,
  FOLD_EXPANDED,
  ICON_AHEAD,
  ICON_BEHIND,
  ICON_BRANCH,
  ICON_CLEAN,
  ICON_DIVERGED,
  ICON_FOLDER,
  ICON_IGNORED,
  ICON_LINKED_WORKTREE,
  ICON_MERGED_INTO_DEFAULT,
  ICON_NO_UPSTREAM,
  ICON_OPEN_VS_DEFAULT,
  ICON_REPO,
  ICON_SYNCED,
  ICON_VIEWED,
  ICON_WORKSPACE,
  RULE,
  fileIcon,
  hasWideEmoji,
  statusColor,
  syncColor,
  truncateSegments,
  truncateVisible,
  tuiFileBadge,
  tuiFileBadgeForChange,
  tuiMergeMark,
  tuiSectionHeader,
  tuiSyncMark,
  viewedColor,
} from '../src/tui/icons.js';
import { nodeLabel, nodeSegments, buildTree } from '../src/tui/model/tree.js';
import { getTheme, segmentsText } from '../src/tui/theme.js';
import type { FileStatusLetter } from '../src/tui/model/types.js';
import type { RepoSnapshot } from '../src/types.js';

const ALL_BADGES: FileStatusLetter[] = ['A', 'M', 'S', 'MS', 'D', 'R', 'U', 'C'];

const STRUCTURE_GLYPHS = [
  FOLD_EXPANDED,
  FOLD_COLLAPSED,
  CURSOR_BAR,
  RULE,
  ICON_WORKSPACE,
  ICON_REPO,
  ICON_LINKED_WORKTREE,
  ICON_BRANCH,
  ICON_FOLDER,
  ICON_CLEAN,
  ICON_IGNORED,
  ICON_AHEAD,
  ICON_BEHIND,
  ICON_DIVERGED,
  ICON_NO_UPSTREAM,
  ICON_SYNCED,
  ICON_MERGED_INTO_DEFAULT,
  ICON_OPEN_VS_DEFAULT,
  ICON_VIEWED,
];

describe('tuiFileBadge', () => {
  it('every file badge is exactly 2 display columns', () => {
    for (const letter of ALL_BADGES) {
      const badge = tuiFileBadge(letter);
      assert.equal(
        visibleWidth(badge),
        2,
        `${letter} → ${JSON.stringify(badge)} width ${visibleWidth(badge)}`,
      );
    }
    assert.deepEqual(ALL_BADGES.map(tuiFileBadge), [
      'A ',
      'M ',
      'S ',
      'MS',
      'D ',
      'R ',
      'U ',
      'C ',
    ]);
  });

  it('maps FileChange to fixed-width badges', () => {
    assert.equal(tuiFileBadgeForChange({ path: 'a', untracked: true }), 'A ');
    assert.equal(
      tuiFileBadgeForChange({ path: 'a', stagedStatus: 'M', unstagedStatus: 'M' }),
      'MS',
    );
    assert.equal(tuiFileBadgeForChange({ path: 'a', stagedStatus: 'M' }), 'S ');
    assert.equal(tuiFileBadgeForChange({ path: 'a', unstagedStatus: 'M' }), 'M ');
  });

  it('gives every status letter a distinct-enough colour', () => {
    for (const letter of ALL_BADGES) {
      assert.match(statusColor(letter), /^#[0-9a-f]{6}$/);
    }
    assert.notEqual(statusColor('A'), statusColor('D'));
    assert.notEqual(statusColor('M'), statusColor('D'));
  });
});

describe('nerd font glyph registry', () => {
  it('every structural glyph occupies exactly one column', () => {
    for (const glyph of STRUCTURE_GLYPHS) {
      assert.equal(visibleWidth(glyph), 1, `${JSON.stringify(glyph)} width ${visibleWidth(glyph)}`);
      assert.ok(!hasWideEmoji(glyph), JSON.stringify(glyph));
    }
  });

  it('devicons are single column and fall back for unknown extensions', () => {
    for (const path of ['a.ts', 'a.tsx', 'a.js', 'a.json', 'a.md', 'a.py', 'a.sh']) {
      const icon = fileIcon(path);
      assert.equal(visibleWidth(icon.glyph), 1, path);
      assert.match(icon.color, /^#[0-9a-f]{6}$/i);
    }
    // Exact filename wins over extension.
    assert.notEqual(fileIcon('package.json').glyph, fileIcon('tsconfig.json').glyph);
    // Unknown extension still yields a usable icon.
    assert.equal(visibleWidth(fileIcon('a.wat').glyph), 1);
    assert.equal(visibleWidth(fileIcon('NOEXT').glyph), 1);
  });
});

describe('tui branch/sync/section marks', () => {
  it('sync marks carry glyph plus count', () => {
    assert.equal(tuiSyncMark('up-to-date'), ICON_SYNCED);
    assert.equal(tuiSyncMark('behind', 'behind by 3'), `${ICON_BEHIND}3`);
    assert.equal(tuiSyncMark('ahead', 'ahead by 2'), `${ICON_AHEAD}2`);
    assert.equal(tuiSyncMark('diverged'), ICON_DIVERGED);
    assert.equal(tuiSyncMark('no-upstream'), ICON_NO_UPSTREAM);
  });

  it('merge marks map plain ✅/🌱 to single-column glyphs (no emoji)', () => {
    assert.equal(tuiMergeMark(true), ICON_MERGED_INTO_DEFAULT);
    assert.equal(tuiMergeMark(false), ICON_OPEN_VS_DEFAULT);
    assert.equal(tuiMergeMark(null), '');
    assert.equal(visibleWidth(ICON_LINKED_WORKTREE), 1);
    assert.equal(visibleWidth(ICON_MERGED_INTO_DEFAULT), 1);
    assert.equal(visibleWidth(ICON_OPEN_VS_DEFAULT), 1);
    assert.ok(!hasWideEmoji(ICON_LINKED_WORKTREE));
    assert.ok(!hasWideEmoji(tuiMergeMark(true)));
    assert.ok(!hasWideEmoji(tuiMergeMark(false)));
  });

  it('colours behind/ahead/diverged differently', () => {
    assert.notEqual(syncColor('behind'), syncColor('ahead'));
    assert.notEqual(syncColor('diverged'), syncColor('up-to-date'));
  });

  it('section headers are plain STAGED / UNSTAGED / NEW', () => {
    assert.equal(tuiSectionHeader('staged'), 'STAGED');
    assert.equal(tuiSectionHeader('unstaged'), 'UNSTAGED');
    assert.equal(tuiSectionHeader('new'), 'NEW');
  });
});

describe('tree segments (TUI)', () => {
  const snapshot: RepoSnapshot = {
    repo: 'app',
    branch: 'feature/ABC-1-thing',
    syncStatus: 'behind',
    syncNote: 'behind by 3',
    hasUnstaged: true,
    hasStaged: false,
    hasUntracked: true,
    unstagedInfo: '',
    stagedFiles: '',
    unstagedFiles: 'M\tsrc/main.ts',
    untrackedFiles: 'new.txt',
    checkoutKind: 'primary',
    mergedIntoDefault: null,
  };

  const tree = buildTree({
    snapshots: [snapshot],
    ignoredRepos: new Set(),
    treeMode: false,
    workspaceLabel: 'ws',
  });
  const repo = tree.children.find((c) => c.kind === 'repo');
  assert.ok(repo && repo.kind === 'repo');

  it('repo row carries branch on the left and sync on the right', () => {
    const { segments, trailing } = nodeSegments(repo, false);
    const left = segmentsText(segments);
    assert.match(left, /app/);
    assert.match(left, /feature\/ABC-1-thing/);
    assert.doesNotMatch(left, /\*feature\//);
    assert.equal(segmentsText(trailing).trim(), `${ICON_BEHIND}3  2`);
    assert.ok(!hasWideEmoji(nodeLabel(repo, false)));
  });

  it('file row puts the status letter in the trailing column', () => {
    const file = repo.children.find((c) => c.kind === 'file' && c.path === 'src/main.ts');
    assert.ok(file && file.kind === 'file');
    const { segments, trailing } = nodeSegments(file, false);
    // Flat mode: name first, containing directory dimmed after it.
    assert.match(segmentsText(segments), /main\.ts {2}src$/);
    assert.equal(segmentsText(trailing), 'M ');
    assert.ok(!hasWideEmoji(nodeLabel(file, false)));
  });
});

describe('truncation', () => {
  it('keeps long repo labels within pane width (no mid-row wrap)', () => {
    const paneWidth = 40;
    const longLabel = 'very-long-repo-name-that-would-wrap  feature/JBY-035-workspace-status-tui';
    const truncated = truncateVisible(`${FOLD_EXPANDED} ${longLabel}`, paneWidth);
    assert.ok(visibleWidth(truncated) <= paneWidth);
    assert.ok(!hasWideEmoji(truncated));
    assert.ok(!truncated.includes('\n'));
  });

  it('truncateSegments respects the budget and marks the cut with an ellipsis', () => {
    const segments = [
      { text: 'aaaaaaaaaa' },
      { text: 'bbbbbbbbbb', color: '#ffffff' },
      { text: 'cccccccccc' },
    ];
    assert.deepEqual(truncateSegments(segments, 100), segments);

    const cut = truncateSegments(segments, 15);
    const text = segmentsText(cut);
    assert.ok(visibleWidth(text) <= 15, text);
    assert.ok(text.endsWith('…'));
    // The surviving styled segment keeps its colour.
    assert.equal(cut[1].color, '#ffffff');
  });
});

describe('viewed vs clean glyphs', () => {
  it('ICON_VIEWED is not the clean or synced check', () => {
    assert.notEqual(ICON_VIEWED, ICON_CLEAN);
    assert.notEqual(ICON_VIEWED, ICON_SYNCED);
    assert.notEqual(viewedColor(), getTheme().palette.added);
    assert.equal(viewedColor(), getTheme().palette.renamed);
  });
});
