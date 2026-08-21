import assert from 'node:assert';
import { describe, it } from 'node:test';
import {
  DEFAULT_THEME_ID,
  THEME_IDS,
  THEMES,
  cycleThemeId,
  flashBackground,
  getTheme,
  resolveLaneColors,
  resolveThemeId,
  setActiveTheme,
} from '../src/tui/theme.js';
import type { Theme, ThemeId, ThemePalette } from '../src/tui/theme.js';
import { DEFAULT_LANE_COLORS } from '../src/tui/graph/laneColors.js';

const PALETTE_KEYS: (keyof ThemePalette)[] = [
  'heading',
  'repo',
  'dir',
  'file',
  'branchDefault',
  'branchFeature',
  'muted',
  'headMark',
  'added',
  'modified',
  'deleted',
  'renamed',
  'untracked',
  'cursor',
  'cursorBg',
  'diffAddBg',
  'diffDelBg',
  'diffHunk',
];

const PILL_KEYS = ['mode', 'diff', 'filter', 'busy', 'error'] as const;

describe('theme presets', () => {
  it('lists exactly the five dark presets in cycle order', () => {
    assert.deepEqual([...THEME_IDS], [
      'tokyo-night',
      'monokai',
      'dracula',
      'gruvbox-dark',
      'catppuccin-mocha',
    ]);
    assert.equal(DEFAULT_THEME_ID, 'tokyo-night');
  });

  it('every preset defines every palette, pill, flashRamp, and surface key', () => {
    for (const id of THEME_IDS) {
      const theme: Theme = THEMES[id];
      assert.equal(theme.id, id);
      assert.ok(theme.label.length > 0, id);
      assert.ok(/^#[0-9a-fA-F]{6}$/.test(theme.surface), `${id}.surface`);
      for (const key of PALETTE_KEYS) {
        const value = theme.palette[key];
        assert.ok(typeof value === 'string' && /^#[0-9a-fA-F]{6}$/.test(value), `${id}.palette.${key}`);
      }
      for (const key of PILL_KEYS) {
        const pill = theme.pill[key];
        assert.ok(/^#[0-9a-fA-F]{6}$/.test(pill.bg), `${id}.pill.${key}.bg`);
        assert.ok(/^#[0-9a-fA-F]{6}$/.test(pill.fg), `${id}.pill.${key}.fg`);
      }
      assert.equal(theme.flashRamp.length, 4, `${id}.flashRamp length`);
      for (const [i, colour] of theme.flashRamp.entries()) {
        assert.ok(/^#[0-9a-fA-F]{6}$/.test(colour), `${id}.flashRamp[${i}]`);
      }
    }
  });

  it('tokyo-night keeps the pre-Phase-5 heading colour', () => {
    assert.equal(THEMES['tokyo-night'].palette.heading, '#7dcfff');
    assert.equal(THEMES['tokyo-night'].surface, '#1a1b26');
  });

  it('default branch and headMark stay distinct from muted on every theme', () => {
    for (const id of THEME_IDS) {
      const p = THEMES[id].palette;
      assert.notEqual(p.branchDefault, p.muted, `${id}.branchDefault`);
      assert.notEqual(p.headMark, p.muted, `${id}.headMark`);
    }
    assert.equal(THEMES['tokyo-night'].palette.branchDefault, '#7aa2f7');
    assert.equal(THEMES['tokyo-night'].palette.headMark, '#e0af68');
    assert.equal(THEMES.monokai.palette.headMark, '#a6e22e');
    assert.equal(THEMES.dracula.palette.headMark, '#50fa7b');
    assert.equal(THEMES['gruvbox-dark'].palette.headMark, '#fe8019');
    assert.equal(THEMES['catppuccin-mocha'].palette.headMark, '#f9e2af');
  });
});

describe('resolveThemeId', () => {
  it('returns tokyo-night for empty, unknown, or missing values', () => {
    assert.equal(resolveThemeId(undefined), 'tokyo-night');
    assert.equal(resolveThemeId(null), 'tokyo-night');
    assert.equal(resolveThemeId(''), 'tokyo-night');
    assert.equal(resolveThemeId('nope'), 'tokyo-night');
  });

  it('accepts each known id', () => {
    for (const id of THEME_IDS) {
      assert.equal(resolveThemeId(id), id);
    }
  });
});

describe('cycleThemeId', () => {
  it('walks THEME_IDS and wraps', () => {
    let id: ThemeId = 'tokyo-night';
    const seen: ThemeId[] = [];
    for (let i = 0; i < THEME_IDS.length; i++) {
      id = cycleThemeId(id);
      seen.push(id);
    }
    assert.deepEqual(seen, [
      'monokai',
      'dracula',
      'gruvbox-dark',
      'catppuccin-mocha',
      'tokyo-night',
    ]);
  });
});

describe('active theme + flashBackground', () => {
  it('getTheme reflects setActiveTheme and flashBackground uses that ramp', () => {
    const previous = getTheme();
    try {
      setActiveTheme(THEMES.monokai);
      assert.equal(getTheme().id, 'monokai');
      assert.equal(flashBackground(1), THEMES.monokai.flashRamp[0]);
      assert.equal(flashBackground(0), undefined);
    } finally {
      setActiveTheme(previous);
    }
  });
});

describe('resolveLaneColors', () => {
  it('returns at least 6 colours for every built-in theme', () => {
    for (const id of Object.keys(THEMES) as (keyof typeof THEMES)[]) {
      const colors = resolveLaneColors(THEMES[id]);
      assert.ok(colors.length >= 6, id);
      for (const c of colors) assert.match(c, /^#[0-9a-fA-F]{6}$/);
    }
  });

  it('tokyo-night matches DEFAULT_LANE_COLORS', () => {
    assert.deepEqual(
      [...resolveLaneColors(THEMES['tokyo-night'])],
      [...DEFAULT_LANE_COLORS],
    );
  });
});
