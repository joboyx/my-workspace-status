/**
 * Bottom status bar — mode pills, filter, key hints.
 * Ephemeral status/toasts live in the breadcrumb op-status (right), not here.
 */

import React from 'react';
import { Box, Text } from 'ink';
import { ICON_BRANCH, ICON_DIFF, ICON_MOVE, REQUIRED_FONT } from './icons.js';
import {
  HELP_KEY_WIDTH,
  helpChipGapSpaces,
  helpColumnWidth,
  helpEntryVisualLines,
  helpInnerWidth,
  helpOverlayHeight,
  wrapHelpFooter,
} from './helpLayout.js';
import { helpEntryMatches } from './helpSearch.js';
import { useTheme } from './theme.js';
import type { NavDepthIndex, RowKind } from './actions/registry.js';
import { actionVisibleForGraphRow, actionsForContext } from './actions/registry.js';
import {
  TREE_WRITE_BLOCKED_IDS,
  actionVisibleForScope,
  removeWorktreeHintLabel,
  treeWritesHiddenForContext,
  type ActionGateContext,
} from './actions/gates.js';
import type { GraphActionRow, GraphStashMenuExtras } from './graph/actions.js';
import type { FocusPane } from './nav/stack.js';
import type { CheckoutNode, RepoNode } from './model/types.js';
import { diffModeUserLabel } from './diffModeLabel.js';

export interface StatusBarProps {
  treeMode: boolean;
  /** Unused legacy filter string (search uses `search`). */
  filter: string;
  searchMode: boolean;
  /** Armed search query for pill chrome; null when idle. */
  search: { query: string; matchIndex: number } | null;
  easyMotion?: boolean;
  easyMotionTyped?: string;
  showHelp: boolean;
  /**
   * Help-overlay `/` query while `?` help is open.
   * `null` = not searching help; string (possibly empty) = help search active.
   */
  helpSearchQuery?: string | null;
  zPending: boolean;
  diffMode: 'inline' | 'sideBySide';
  /** Kind of the highlighted row — selects which action hints are shown. */
  rowKind: RowKind;
  /** Terminal width, used to truncate the hint list instead of wrapping. */
  width: number;
  /** ViewStack depth for context-aware hints. */
  navDepth: NavDepthIndex;
  /** Focused pane for context-aware hints. */
  focusPane: FocusPane;
  /** Depth-1 graph selection — gates `b` when no checkoutable ref, etc. */
  graphActionRow?: GraphActionRow | null;
  /** Scope + sync for stage/pull/push/default-branch hint gates. */
  actionGate?: ActionGateContext | null;
  /** Worktree dirty / latest stash — hides `S` on a clean commit with no stashes. */
  graphStashExtras?: GraphStashMenuExtras;
}

/**
 * One rendered action hint: key chip text and description label, plus whether
 * the action is destructive (danger colour on chip / label).
 */
export interface HintSegment {
  readonly key: string;
  readonly label: string;
  readonly destructive: boolean;
}

/** Columns between chip and label (B7). */
export const HINT_CHIP_GAP = 2;

/** Gap rendered between two hints. */
const HINT_SEPARATOR = '  ';
/** Marker appended when hints were dropped to fit the terminal width. */
const HINT_ELLIPSIS = '…';

/** Plain-text join of chip key + gap + label (tests / width math). */
export function formatHintPlain(segment: HintSegment): string {
  if (segment.label.length === 0) return segment.key;
  return `${segment.key}${' '.repeat(HINT_CHIP_GAP)}${segment.label}`;
}

/** Visual columns for one hint: pill (` key `) + optional gap + label. */
function hintSegmentColumns(segment: HintSegment): number {
  // Empty label (ellipsis): render is pill only — no chip/label gap.
  if (segment.label.length === 0) return segment.key.length + 2;
  return segment.key.length + 2 + HINT_CHIP_GAP + segment.label.length;
}

const GRAPH_HINT_KINDS: ReadonlySet<RowKind> = new Set([
  'graphCommit',
  'graphStash',
  'graphUncommitted',
]);

/**
 * Hints for every action valid on `rowKind` at the given nav dims, in registry
 * order. Pure so the hint text can be unit-tested without rendering React.
 * Graph payload gates (`actionVisibleForGraphRow`) apply only when `rowKind`
 * is a graph kind — a leftover graph selection must not hide tree hints such
 * as stash (`S`). Hide `b` without a local or origin ref. Pass `extras` so
 * `S` hides on a clean commit with no stashes. When `scope` is set, tree write
 * actions are filtered by focused files / sync (e.g. hide `s` with nothing to
 * stage).
 */
export function actionHintSegments(
  rowKind: RowKind,
  depth: NavDepthIndex = 0,
  focusPane: FocusPane = 'left',
  graphRow?: GraphActionRow | null,
  scope?: ActionGateContext | null,
  extras?: GraphStashMenuExtras,
): HintSegment[] {
  const hideTreeWrites = treeWritesHiddenForContext(depth, focusPane);
  return actionsForContext(rowKind, depth, focusPane)
    .filter((action) =>
      graphRow && GRAPH_HINT_KINDS.has(rowKind)
        ? actionVisibleForGraphRow(action, graphRow, extras)
        : true,
    )
    .filter((action) => (scope ? actionVisibleForScope(action, scope) : true))
    .filter((action) => !(hideTreeWrites && TREE_WRITE_BLOCKED_IDS.has(action.id)))
    .map((action) => {
      let label = action.label;
      if (
        action.id === 'removeWorktree' &&
        (scope?.focused?.node.kind === 'checkout' || scope?.focused?.node.kind === 'repo')
      ) {
        label = removeWorktreeHintLabel(scope.focused.node as CheckoutNode | RepoNode);
      }
      return {
        key: action.key,
        label,
        destructive: action.destructive,
      };
    });
}

/**
 * Enter/Esc chrome hints for the status bar (not registry actions).
 */
export function navChromeHintSegments(depth: NavDepthIndex, focusPane: FocusPane): HintSegment[] {
  const out: HintSegment[] = [];
  if (focusPane === 'left') {
    out.push({ key: '⏎', label: 'focus right', destructive: false });
    if (depth > 0) out.push({ key: 'Esc', label: 'back', destructive: false });
  } else {
    if (depth < 2) out.push({ key: '⏎', label: 'drill', destructive: false });
    out.push({ key: 'Esc', label: 'back', destructive: false });
  }
  return out;
}

/**
 * The longest prefix of `segments` that fits in `available` columns.
 *
 * The status bar is one line and the layout sizes the panes assuming that, so
 * an over-long hint list must be cut rather than wrapped. When anything is
 * dropped a `…` segment marks the truncation; if even that does not fit, the
 * list comes back empty.
 */
export function fitHintSegments(segments: HintSegment[], available: number): HintSegment[] {
  const width = (kept: HintSegment[]): number =>
    kept.reduce((n, s) => n + hintSegmentColumns(s), 0) +
    Math.max(0, kept.length - 1) * HINT_SEPARATOR.length;

  const kept: HintSegment[] = [];
  for (const segment of segments) {
    const next = [...kept, segment];
    if (width(next) > available) break;
    kept.push(segment);
  }

  if (kept.length === segments.length) return kept;

  const ellipsis: HintSegment = {
    key: HINT_ELLIPSIS,
    label: '',
    destructive: false,
  };
  while (kept.length > 0 && width([...kept, ellipsis]) > available) kept.pop();
  return kept.length > 0 ? [...kept, ellipsis] : [];
}

interface HelpGroupDef {
  title: string;
  /** Nerd Font glyph shown beside the group title. */
  icon: string;
  /** Palette key resolved at render time via `useTheme()`. */
  colorKey: 'cursor' | 'added' | 'modified';
  /** `[keys, description]`; keys are space-separated chips. */
  keys: [string, string][];
}

/** Keymap rows shown in the `?` help overlay (source of truth for in-TUI shortcuts). */
export const HELP_GROUPS: HelpGroupDef[] = [
  {
    title: 'MOVE',
    icon: ICON_MOVE,
    colorKey: 'cursor',
    keys: [
      ['j k', 'down / up'],
      ['h l', 'fold · pan when right+diff'],
      ['z', 'toggle fold (instant; no-op on graph/diff)'],
      ['zz', 'toggle subtree (no-op on graph/diff)'],
      ['gg G', 'top / bottom of focused pane'],
      ['/', 'search focused pane (Enter arms)'],
      ['n N', 'next / prev match (after Enter)'],
      ['Ctrl-Space ;', 'EasyMotion on focused list'],
    ],
  },
  {
    title: 'GIT',
    icon: ICON_BRANCH,
    colorKey: 'added',
    keys: [
      ['s', 'stage scope'],
      ['S', 'stash menu'],
      ['u', 'unstage scope'],
      ['x', 'revert (y/Y)'],
      ['e', 'open in editor'],
      ['space', 'mark dirty file reviewed (eye)'],
      ['f', 'fetch remotes'],
      ['p', 'pull behind'],
      ['P', 'push ahead/diverged/new'],
      ['d', 'default branch'],
      ['b', 'depth 0 picker · graph local/origin/*'],
      ['W', 'remove linked worktree'],
      ['r', 'refresh now'],
    ],
  },
  {
    title: 'VIEW',
    icon: ICON_DIFF,
    colorKey: 'modified',
    keys: [
      ['i', 'inline / split'],
      ['t', 'flat / tree'],
      ['.', 'show / hide ignored repos'],
      ['T', 'cycle theme'],
      ['Ctrl-o', 'full-file · keep hunk in view'],
      ['PgUp PgDn', 'page focused pane'],
      ['Ctrl-u Ctrl-d', 'page focused ±5'],
      ['m', 'mouse · drag pane or split divider'],
      ['Esc', 'back / unfocus · never quit'],
      ['Enter dblclick', 'focus right / drill'],
      ['?', 'this help'],
      ['Ctrl-C Ctrl-C', 'quit (press twice)'],
    ],
  },
];

const HELP_ROW_COUNT = Math.max(...HELP_GROUPS.map((g) => g.keys.length));

export { HELP_KEY_WIDTH };

/** Idle help footer must mention overlay-local `/` search. */
export const HELP_IDLE_FOOTER_SNIPPET = '/ search help';

/** Active help-search footer Esc hint. */
export const HELP_SEARCH_ESC_HINT = 'Esc clears search';

/**
 * Status-bar copy while `/` search is in typing mode.
 * Enter arms the query; until then `n`/`N` append to the query.
 */
export const SEARCH_TYPING_HINT = 'Enter arms query · Esc clears · n/N after Enter';

function helpIdleFooterText(): string {
  return `Needs a Nerd Font · ${REQUIRED_FONT} · ${HELP_IDLE_FOOTER_SNIPPET} · Esc closes`;
}

/**
 * Rows the help overlay occupies at `termWidth`: border, title, wrapped body,
 * and footer. App uses this so panes shrink instead of overlapping the overlay.
 */
export function helpStatusLines(termWidth: number): number {
  return helpOverlayHeight(HELP_GROUPS, termWidth, helpIdleFooterText());
}

function Pill(props: { label: string; bg: string; fg: string }): React.ReactElement {
  return (
    <Text backgroundColor={props.bg} color={props.fg} bold>
      {` ${props.label} `}
    </Text>
  );
}

/**
 * Key chips: each key gets an inverse block so bindings are findable without
 * reading the descriptions. Padded to a fixed width to keep columns aligned.
 */
function HelpKey(props: { keys: string; color: string; surface: string }): React.ReactElement {
  const chips = props.keys.split(' ');
  return (
    <Text>
      {chips.map((chip, i) => (
        <Text key={i}>
          <Text backgroundColor={props.color} color={props.surface} bold>
            {` ${chip} `}
          </Text>
          <Text> </Text>
        </Text>
      ))}
      <Text>{' '.repeat(helpChipGapSpaces(props.keys))}</Text>
    </Text>
  );
}

function HelpPanel(props: { helpSearchQuery: string | null; width: number }): React.ReactElement {
  const { helpSearchQuery, width } = props;
  const { palette: PALETTE, pill: PILL, surface } = useTheme();
  const groups = HELP_GROUPS.map((g) => ({
    ...g,
    color: PALETTE[g.colorKey],
  }));
  const searching = helpSearchQuery !== null;
  const columnWidth = helpColumnWidth(width);
  const innerWidth = helpInnerWidth(width);
  const idleFooterLines = wrapHelpFooter(helpIdleFooterText(), innerWidth);
  const searchQueryText = ` /${helpSearchQuery ?? ''}`;
  const searchHint = `   ${HELP_SEARCH_ESC_HINT}`;
  const helpPillCols = 2 + 'HELP'.length;
  const searchHintFits =
    helpPillCols + searchQueryText.length + 1 + searchHint.length <= innerWidth;

  return (
    <Box flexDirection="column" borderStyle="round" borderColor={PALETTE.cursor} paddingX={1}>
      <Box flexDirection="row">
        {groups.map((group) => (
          <Box key={group.title} width={columnWidth}>
            <Text color={group.color} bold wrap="truncate">
              {`${group.icon}  ${group.title}`}
            </Text>
          </Box>
        ))}
      </Box>
      {Array.from({ length: HELP_ROW_COUNT }, (_, row) => (
        <Box key={row} flexDirection="row">
          {groups.map((group) => {
            const entry = group.keys[row];
            const matched =
              searching &&
              entry !== undefined &&
              helpEntryMatches(entry[0], entry[1], helpSearchQuery);
            if (!entry) {
              return (
                <Box key={group.title} width={columnWidth}>
                  <Text> </Text>
                </Box>
              );
            }
            const visualLines = helpEntryVisualLines(entry[1], columnWidth, entry[0]);
            return (
              <Box key={group.title} width={columnWidth} flexDirection="column">
                {/* Pre-wrapped to columnWidth; truncate is a no-op height guard. */}
                {visualLines.map((line, i) => (
                  <Text
                    key={i}
                    wrap="truncate"
                    backgroundColor={matched ? PILL.filter.bg : undefined}
                  >
                    {line.chips ? (
                      <HelpKey keys={entry[0]} color={group.color} surface={surface} />
                    ) : (
                      <Text>{' '.repeat(line.indent)}</Text>
                    )}
                    {line.text.length > 0 ? <Text color={PALETTE.muted}>{line.text}</Text> : null}
                  </Text>
                ))}
              </Box>
            );
          })}
        </Box>
      ))}
      {searching ? (
        <Box flexDirection="column">
          <Box>
            <Pill label="HELP" bg={PILL.filter.bg} fg={PILL.filter.fg} />
            <Text color={PALETTE.repo}>{searchQueryText}</Text>
            <Text color={PALETTE.cursor}>▏</Text>
            {searchHintFits ? <Text color={PALETTE.muted}>{searchHint}</Text> : null}
          </Box>
          {searchHintFits
            ? null
            : wrapHelpFooter(HELP_SEARCH_ESC_HINT, innerWidth).map((line, i) => (
                <Text key={i} color={PALETTE.muted}>
                  {line}
                </Text>
              ))}
        </Box>
      ) : (
        <Box flexDirection="column">
          {idleFooterLines.map((line, i) => (
            <Text key={i} color={PALETTE.muted}>
              {line}
            </Text>
          ))}
        </Box>
      )}
    </Box>
  );
}

/**
 * Single-line status under the panes, or the multi-column help panel.
 */
export function StatusBar(props: StatusBarProps): React.ReactElement {
  const {
    treeMode,
    searchMode,
    search,
    showHelp,
    helpSearchQuery = null,
    zPending,
    diffMode,
    rowKind,
    width,
    navDepth,
    focusPane,
    graphActionRow = null,
    actionGate = null,
    graphStashExtras,
  } = props;
  const { palette: PALETTE, pill: PILL, surface } = useTheme();

  if (showHelp) {
    return <HelpPanel helpSearchQuery={helpSearchQuery} width={width} />;
  }

  if (searchMode) {
    const query = search?.query ?? '';
    return (
      <Box>
        <Pill label="SEARCH" bg={PILL.filter.bg} fg={PILL.filter.fg} />
        <Text color={PALETTE.repo}> {query}</Text>
        <Text color={PALETTE.cursor}>▏</Text>
        <Text color={PALETTE.muted}>{`   ${SEARCH_TYPING_HINT}`}</Text>
      </Box>
    );
  }

  if (props.easyMotion) {
    return (
      <Box>
        <Pill label="EASY" bg={PILL.filter.bg} fg={PILL.filter.fg} />
        <Text color={PALETTE.repo}> {props.easyMotionTyped || '…'}</Text>
        <Text color={PALETTE.muted}>{'   type label · Esc cancels'}</Text>
      </Box>
    );
  }

  // Hint slot stays key chrome only — never replace with statusMessage toasts.
  // Quit is double Ctrl+C; App pins the ephemeral "press again" prompt — no standing ×2 hint.
  const hint = zPending ? 'z…' : '? help';
  const message = hint;

  const modeLabel = treeMode ? 'tree' : 'flat';
  const diffLabel = diffModeUserLabel(diffMode);
  const searchQuery = search?.query?.trim() ?? '';
  // Each pill renders as ` label ` (+2), plus the space before the message.
  const used =
    modeLabel.length +
    2 +
    diffLabel.length +
    2 +
    (searchQuery ? searchQuery.length + 3 : 0) +
    1 +
    message.length +
    HINT_SEPARATOR.length;
  const hints = fitHintSegments(
    [
      ...navChromeHintSegments(navDepth, focusPane),
      ...actionHintSegments(
        rowKind,
        navDepth,
        focusPane,
        graphActionRow,
        actionGate,
        graphStashExtras,
      ),
    ],
    width - used,
  );

  return (
    <Box>
      <Pill label={modeLabel} bg={PILL.mode.bg} fg={PILL.mode.fg} />
      <Pill label={diffLabel} bg={PILL.diff.bg} fg={PILL.diff.fg} />
      {searchQuery ? (
        <Pill label={`/${searchQuery}`} bg={PILL.filter.bg} fg={PILL.filter.fg} />
      ) : null}
      <Text color={PALETTE.file} wrap="truncate">
        {' '}
        {message}
      </Text>
      {hints.length > 0 ? (
        <Text wrap="truncate">
          {HINT_SEPARATOR}
          {hints.map((segment, i) => (
            <Text key={i}>
              {i > 0 ? HINT_SEPARATOR : ''}
              <Text
                backgroundColor={segment.destructive ? PALETTE.deleted : PALETTE.cursor}
                color={surface}
                bold
              >
                {` ${segment.key} `}
              </Text>
              {segment.label.length > 0 ? (
                <>
                  <Text>{' '.repeat(HINT_CHIP_GAP)}</Text>
                  <Text color={segment.destructive ? PALETTE.deleted : PALETTE.muted}>
                    {segment.label}
                  </Text>
                </>
              ) : null}
            </Text>
          ))}
        </Text>
      ) : null}
    </Box>
  );
}
