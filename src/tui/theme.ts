/**
 * TUI colour themes.
 *
 * Semantic colours live on `Theme.palette` / `pill` / `flashRamp` / `surface`.
 * React panes read via `useTheme()`; non-React builders use `getTheme()`.
 */

import React, { createContext, useContext } from 'react';
import { DEFAULT_LANE_COLORS } from './graph/laneColors.js';

/** A styled run of text. Panes render arrays of these. */
export interface Segment {
  text: string;
  color?: string;
  backgroundColor?: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  /**
   * Pre-rendered ANSI (e.g. syntax-highlighted code). When true the text is
   * emitted verbatim and no other style props are applied.
   */
  raw?: boolean;
}

/** Built-in dark theme identifiers. */
export type ThemeId =
  | 'tokyo-night'
  | 'monokai'
  | 'dracula'
  | 'gruvbox-dark'
  | 'catppuccin-mocha';

/** Semantic foreground / background colours for tree, diff, and chrome. */
export interface ThemePalette {
  heading: string;
  repo: string;
  dir: string;
  file: string;
  branchDefault: string;
  branchFeature: string;
  muted: string;
  /**
   * High-contrast checkout / detached-HEAD mark (Nerd crosshairs / `[HEAD]`).
   * Must stay distinct from muted on every preset.
   */
  headMark: string;
  added: string;
  modified: string;
  deleted: string;
  renamed: string;
  untracked: string;
  cursor: string;
  cursorBg: string;
  diffAddBg: string;
  diffDelBg: string;
  diffHunk: string;
}

/** Status-bar pill background/foreground pairs. */
export interface ThemePill {
  mode: { bg: string; fg: string };
  diff: { bg: string; fg: string };
  filter: { bg: string; fg: string };
  busy: { bg: string; fg: string };
  error: { bg: string; fg: string };
}

/** One complete built-in theme. */
export interface Theme {
  id: ThemeId;
  label: string;
  /** Dark surface used as text on bright accent chips. */
  surface: string;
  palette: ThemePalette;
  pill: ThemePill;
  flashRamp: readonly [string, string, string, string];
  /** Graph lane cycle; falls back via resolveLaneColors. */
  laneColors: readonly string[];
}

/**
 * Build an 8-colour lane cycle from palette accents (stable order).
 */
function laneColorsFromPalette(palette: ThemePalette): readonly string[] {
  return [
    palette.dir,
    palette.branchFeature,
    palette.heading,
    palette.added,
    palette.modified,
    palette.deleted,
    palette.renamed,
    palette.untracked,
  ];
}

/** Default when `WS_STATUS_THEME` is unset or unknown. */
export const DEFAULT_THEME_ID: ThemeId = 'tokyo-night';

/** Cycle order for `T`. */
export const THEME_IDS: readonly ThemeId[] = [
  'tokyo-night',
  'monokai',
  'dracula',
  'gruvbox-dark',
  'catppuccin-mocha',
] as const;

function theme(
  id: ThemeId,
  label: string,
  surface: string,
  palette: ThemePalette,
  pill: ThemePill,
  flashRamp: readonly [string, string, string, string],
  laneColors: readonly string[] = laneColorsFromPalette(palette),
): Theme {
  return { id, label, surface, palette, pill, flashRamp, laneColors };
}

/** All built-in themes keyed by id. */
export const THEMES: Record<ThemeId, Theme> = {
  'tokyo-night': theme(
    'tokyo-night',
    'Tokyo Night',
    '#1a1b26',
    {
      heading: '#7dcfff',
      repo: '#c0caf5',
      dir: '#7aa2f7',
      file: '#a9b1d6',
      branchDefault: '#7aa2f7',
      branchFeature: '#bb9af7',
      muted: '#565f89',
      headMark: '#e0af68',
      added: '#9ece6a',
      modified: '#e0af68',
      deleted: '#f7768e',
      renamed: '#7dcfff',
      untracked: '#bb9af7',
      cursor: '#7aa2f7',
      cursorBg: '#283457',
      diffAddBg: '#1f2d1f',
      diffDelBg: '#33222c',
      diffHunk: '#7dcfff',
    },
    {
      mode: { bg: '#3d59a1', fg: '#c0caf5' },
      diff: { bg: '#33467c', fg: '#c0caf5' },
      filter: { bg: '#bb9af7', fg: '#1a1b26' },
      busy: { bg: '#e0af68', fg: '#1a1b26' },
      error: { bg: '#f7768e', fg: '#1a1b26' },
    },
    ['#3d5236', '#354830', '#2d3d2a', '#273324'],
    DEFAULT_LANE_COLORS,
  ),
  monokai: theme(
    'monokai',
    'Monokai',
    '#272822',
    {
      heading: '#66d9ef',
      repo: '#f8f8f2',
      dir: '#66d9ef',
      file: '#f8f8f2',
      branchDefault: '#66d9ef',
      branchFeature: '#ae81ff',
      muted: '#75715e',
      headMark: '#a6e22e',
      added: '#a6e22e',
      modified: '#e6db74',
      deleted: '#f92672',
      renamed: '#66d9ef',
      untracked: '#ae81ff',
      cursor: '#f8f8f2',
      cursorBg: '#3e3d32',
      diffAddBg: '#3e4a28',
      diffDelBg: '#4a2832',
      diffHunk: '#66d9ef',
    },
    {
      mode: { bg: '#49483e', fg: '#f8f8f2' },
      diff: { bg: '#3e3d32', fg: '#f8f8f2' },
      filter: { bg: '#ae81ff', fg: '#272822' },
      busy: { bg: '#e6db74', fg: '#272822' },
      error: { bg: '#f92672', fg: '#f8f8f2' },
    },
    ['#3e4a28', '#353f24', '#2c3420', '#242b1c'],
  ),
  dracula: theme(
    'dracula',
    'Dracula',
    '#282a36',
    {
      heading: '#8be9fd',
      repo: '#f8f8f2',
      dir: '#bd93f9',
      file: '#f8f8f2',
      branchDefault: '#bd93f9',
      branchFeature: '#bd93f9',
      muted: '#6272a4',
      headMark: '#50fa7b',
      added: '#50fa7b',
      modified: '#f1fa8c',
      deleted: '#ff5555',
      renamed: '#8be9fd',
      untracked: '#bd93f9',
      cursor: '#bd93f9',
      cursorBg: '#44475a',
      diffAddBg: '#2d4a3e',
      diffDelBg: '#4a2d35',
      diffHunk: '#8be9fd',
    },
    {
      mode: { bg: '#44475a', fg: '#f8f8f2' },
      diff: { bg: '#6272a4', fg: '#f8f8f2' },
      filter: { bg: '#bd93f9', fg: '#282a36' },
      busy: { bg: '#f1fa8c', fg: '#282a36' },
      error: { bg: '#ff5555', fg: '#f8f8f2' },
    },
    ['#2d4a3e', '#274038', '#213632', '#1c2d2b'],
  ),
  'gruvbox-dark': theme(
    'gruvbox-dark',
    'Gruvbox Dark',
    '#282828',
    {
      heading: '#83a598',
      repo: '#ebdbb2',
      dir: '#458588',
      file: '#ebdbb2',
      branchDefault: '#458588',
      branchFeature: '#d3869b',
      muted: '#928374',
      headMark: '#fe8019',
      added: '#b8bb26',
      modified: '#fabd2f',
      deleted: '#fb4934',
      renamed: '#83a598',
      untracked: '#d3869b',
      cursor: '#fe8019',
      cursorBg: '#3c3836',
      diffAddBg: '#32361a',
      diffDelBg: '#3c1f1e',
      diffHunk: '#83a598',
    },
    {
      mode: { bg: '#504945', fg: '#ebdbb2' },
      diff: { bg: '#3c3836', fg: '#ebdbb2' },
      filter: { bg: '#d3869b', fg: '#282828' },
      busy: { bg: '#fabd2f', fg: '#282828' },
      error: { bg: '#fb4934', fg: '#ebdbb2' },
    },
    ['#32361a', '#2c3018', '#262a15', '#202412'],
  ),
  'catppuccin-mocha': theme(
    'catppuccin-mocha',
    'Catppuccin Mocha',
    '#1e1e2e',
    {
      heading: '#89dceb',
      repo: '#cdd6f4',
      dir: '#89b4fa',
      file: '#cdd6f4',
      branchDefault: '#89b4fa',
      branchFeature: '#cba6f7',
      muted: '#6c7086',
      headMark: '#f9e2af',
      added: '#a6e3a1',
      modified: '#f9e2af',
      deleted: '#f38ba8',
      renamed: '#89dceb',
      untracked: '#cba6f7',
      cursor: '#89b4fa',
      cursorBg: '#313244',
      diffAddBg: '#1e2b1e',
      diffDelBg: '#2b1e24',
      diffHunk: '#89dceb',
    },
    {
      mode: { bg: '#45475a', fg: '#cdd6f4' },
      diff: { bg: '#313244', fg: '#cdd6f4' },
      filter: { bg: '#cba6f7', fg: '#1e1e2e' },
      busy: { bg: '#f9e2af', fg: '#1e1e2e' },
      error: { bg: '#f38ba8', fg: '#1e1e2e' },
    },
    ['#1e2b1e', '#1a261a', '#162116', '#121c12'],
  ),
};

/**
 * Lane colours for graph glyphs — theme override or DEFAULT_LANE_COLORS.
 */
export function resolveLaneColors(theme: Theme): readonly string[] {
  return theme.laneColors.length > 0 ? theme.laneColors : DEFAULT_LANE_COLORS;
}

/**
 * Map an env / session string to a built-in theme id.
 */
export function resolveThemeId(raw: string | undefined | null): ThemeId {
  if (raw && (THEME_IDS as readonly string[]).includes(raw)) {
    return raw as ThemeId;
  }
  return DEFAULT_THEME_ID;
}

/**
 * Next theme id in `THEME_IDS` order (wraps).
 */
export function cycleThemeId(current: ThemeId): ThemeId {
  const index = THEME_IDS.indexOf(current);
  const from = index >= 0 ? index : 0;
  return THEME_IDS[(from + 1) % THEME_IDS.length]!;
}

let activeTheme: Theme = THEMES[DEFAULT_THEME_ID];

/**
 * Currently active theme for non-React builders.
 */
export function getTheme(): Theme {
  return activeTheme;
}

/**
 * Point non-React colour readers at `theme`.
 */
export function setActiveTheme(theme: Theme): void {
  activeTheme = theme;
}

const ThemeContext = createContext<Theme>(THEMES[DEFAULT_THEME_ID]);

/**
 * Provide the active theme to Ink components without prop-drilling.
 */
export function ThemeProvider(props: {
  theme: Theme;
  children: React.ReactNode;
}): React.ReactElement {
  const { theme, children } = props;
  // Keep module active theme aligned for tree/icons built during this render.
  if (activeTheme !== theme) {
    activeTheme = theme;
  }
  return React.createElement(ThemeContext.Provider, { value: theme }, children);
}

/**
 * Read the theme from the nearest `ThemeProvider`.
 */
export function useTheme(): Theme {
  return useContext(ThemeContext);
}

/**
 * Flash background for a strength in [0, 1]; undefined once it has decayed.
 */
export function flashBackground(strength: number): string | undefined {
  if (strength <= 0) return undefined;
  const ramp = activeTheme.flashRamp;
  const index = Math.min(ramp.length - 1, Math.floor((1 - strength) * ramp.length));
  return ramp[index];
}

/**
 * Concatenate segment text — used for filtering, truncation and tests.
 */
export function segmentsText(segments: Segment[]): string {
  return segments.map((s) => s.text).join('');
}
