/**
 * Ink shell: tree + diff pane + status bar.
 */

import React, { useEffect, useRef, useState } from 'react';
import path from 'node:path';
import { Box, Text, useApp, useInput, useStdin, useStdout } from 'ink';
import { BranchPicker } from './BranchPicker.js';
import { Breadcrumb } from './Breadcrumb.js';
import { filterBranches } from './branches.js';
import { Confirm } from './Confirm.js';
import { CreateBranchOverlay } from './CreateBranchOverlay.js';
import { GraphBranchPicker } from './GraphBranchPicker.js';
import { GraphCheckoutConfirm } from './GraphCheckoutConfirm.js';
import { hitTest } from './hitTest.js';
import type { HitListLayout, HitTestArgs } from './hitTest.js';
import { bottomChromeRows } from './bottomChrome.js';
import {
  TREE_WIDTH_FRACTION,
  paneWidths,
  treeFractionFromWidth,
  widthsEqual,
} from './layoutWidths.js';
import {
  DIFF_SPLIT_FRACTION,
  diffSplitFractionFromTerminalX,
  diffSplitRuleX,
  isSideBySideSplit,
  sideBySideColumnWidths,
} from './diffSplit.js';
import { PANE_TITLE_ROWS, leftPaneTitle, rightPaneTitle } from './focusChrome.js';
import { PaneTitle } from './PaneTitle.js';
import { RightPaneHost, rightPaneMode } from './RightPaneHost.js';
import { GraphPane } from './GraphPane.js';
import { graphChromeBudget } from './graph/selectionDetail.js';
import { shouldShowGraphDetail } from './graph/list.js';
import { easyMotionPaintSlot } from './activeContext.js';
import { commitDetailHeaderHeight } from './CommitDetailPane.js';
import { helpStatusLines, StatusBar } from './StatusBar.js';
import { StashDropConfirm } from './StashDropConfirm.js';
import { StashMenuOverlay } from './StashMenuOverlay.js';
import { RemoveWorktreeConfirm } from './RemoveWorktreeConfirm.js';
import { TreePane } from './TreePane.js';
import {
  MOUSE_DISABLE,
  MOUSE_ENABLE,
  isDoubleClick,
  mouseClickFocus,
  mouseListPressAction,
  parseMouseChunk,
} from './mouse.js';
import type { ClickMemory } from './mouse.js';
import { THEMES, ThemeProvider, useTheme } from './theme.js';
import type { AppOptions, AppStateApi } from './useAppState.js';
import { useAppState } from './useAppState.js';
import { pageKeyFlagsFromInput, type KeyFlags } from './keys.js';
import {
  CTRL_C_EXIT_MS,
  CTRL_C_EXIT_PROMPT,
  handleCtrlC,
  isCtrlC,
  type CtrlCExitState,
} from './ctrlCExit.js';

export type { AppOptions };

/**
 * Interactive workspace-status application root.
 *
 * Wraps the shell in `ThemeProvider`. `T` cycles themes via `state.theme`.
 */
export function App(opts: AppOptions): React.ReactElement {
  const state = useAppState(opts);
  const theme = THEMES[state.theme];
  return (
    <ThemeProvider theme={theme}>
      <AppShell opts={opts} state={state} />
    </ThemeProvider>
  );
}

interface AppShellProps {
  opts: AppOptions;
  state: AppStateApi;
}

/**
 * Layout + input body under `ThemeProvider` so panes can call `useTheme()`.
 */
function AppShell({ opts, state }: AppShellProps): React.ReactElement {
  const { exit } = useApp();
  const { stdout } = useStdout();
  const { stdin } = useStdin();
  const { palette: PALETTE } = useTheme();

  /**
   * Double Ctrl+C exit arm (Ink `exitOnCtrlC: false`). Kept outside React
   * state so a status-message re-render cannot reset the window.
   */
  const ctrlCRef = useRef<CtrlCExitState>({ armedUntil: 0 });
  const ctrlCClearTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const statusMessageRef = useRef(state.statusMessage);
  statusMessageRef.current = state.statusMessage;

  useEffect(() => {
    return () => {
      if (ctrlCClearTimerRef.current) {
        clearTimeout(ctrlCClearTimerRef.current);
        ctrlCClearTimerRef.current = null;
      }
    };
  }, []);

  /**
   * Ink does not re-render on SIGWINCH, so track the size ourselves. Without
   * this every pane width stays frozen at the size the TUI started with.
   */
  const [size, setSize] = useState(() => ({
    rows: stdout?.rows ?? 24,
    columns: stdout?.columns ?? 80,
  }));

  useEffect(() => {
    if (!stdout) return;
    const onResize = (): void => {
      setSize({ rows: stdout.rows ?? 24, columns: stdout.columns ?? 80 });
    };
    stdout.on('resize', onResize);
    onResize();
    return () => {
      stdout.off('resize', onResize);
    };
  }, [stdout]);

  const termRows = size.rows;
  const termCols = size.columns;
  // Filtered list shared by height budget and BranchPicker props.
  const branchPickerBranches = state.branchPicker
    ? state.branchPicker.loading
      ? []
      : filterBranches(state.branchPicker.branches, state.branchPicker.filter)
    : [];
  const graphPickerBranches = state.graphBranchPicker
    ? state.graphBranchPicker.filter.length > 0
      ? state.graphBranchPicker.branches.filter((b) =>
          b.toLowerCase().includes(state.graphBranchPicker!.filter.toLowerCase()),
        )
      : state.graphBranchPicker.branches
    : [];
  // BranchPicker: border(2) + title + visible rows (≤12) + optional status + footer.
  const branchPickerStatusLines = state.branchPicker
    ? Math.min(
        17,
        4 +
          Math.min(12, Math.max(1, branchPickerBranches.length || 1)) +
          (state.statusMessage ? 1 : 0),
      )
    : 0;
  const graphPickerStatusLines = state.graphBranchPicker
    ? Math.min(
        17,
        4 +
          Math.min(12, Math.max(1, graphPickerBranches.length || 1)) +
          (state.statusMessage ? 1 : 0),
      )
    : 0;
  const bottomChrome = bottomChromeRows({
    showHelp: Boolean(state.showHelp),
    helpLines: helpStatusLines(termCols),
    pendingConfirmKind: state.pendingConfirm?.kind ?? null,
    stashDropConfirm: Boolean(state.stashDropConfirm),
    graphCheckoutConfirm: Boolean(state.graphCheckoutConfirm),
    stashMenuLines: state.stashMenuOps
      ? 4 + Math.max(1, state.stashMenuOps.length) + (state.statusMessage ? 1 : 0)
      : 0,
    createBranchOverlay: Boolean(state.createBranchOverlay),
    branchPickerLines: state.branchPicker ? branchPickerStatusLines : 0,
    graphBranchPickerLines: state.graphBranchPicker ? graphPickerStatusLines : 0,
  });
  /**
   * Ctrl+C exit prompt is never a breadcrumb toast — pin it whenever armed.
   * Overlay pickers already render `statusMessage` inline — no extra row.
   */
  const exitPromptPinned =
    state.statusMessage === CTRL_C_EXIT_PROMPT &&
    !state.branchPicker &&
    !state.createBranchOverlay &&
    !state.stashMenuOps &&
    !state.graphBranchPicker;
  // Breadcrumb is always shown except during the help overlay.
  const statusLines =
    (state.showHelp ? bottomChrome : bottomChrome + 1) + (exitPromptPinned ? 1 : 0);
  const paneHeight = Math.max(3, termRows - statusLines);
  // B1: freeze tree/diff widths from term cols + session fraction — never from labels.
  // `treeFraction` is session-only (resets to TREE_WIDTH_FRACTION on next launch).
  const [treeFraction, setTreeFraction] = useState(TREE_WIDTH_FRACTION);
  // Session-only in-diff split (resets on next launch, same as treeFraction).
  const [diffSplitFraction, setDiffSplitFraction] = useState(DIFF_SPLIT_FRACTION);
  const [widths, setWidths] = useState(() => paneWidths(termCols, TREE_WIDTH_FRACTION));
  useEffect(() => {
    const next = paneWidths(termCols, treeFraction);
    setWidths((prev) => (widthsEqual(prev, next) ? prev : next));
  }, [termCols, treeFraction]);
  const { treeWidth, treeInnerWidth, diffWidth } = widths;
  // Title chip + content; DiffPane spends one more row on its path header.
  const listHeight = Math.max(1, paneHeight - PANE_TITLE_ROWS);
  const rightMode = rightPaneMode(state.nav, state.rows[state.cursor]);
  const splitActive = rightMode === 'diff' && isSideBySideSplit(state.diffMode, diffWidth);
  const splitWidths = sideBySideColumnWidths(diffWidth, diffSplitFraction);
  const splitRuleX = splitActive ? diffSplitRuleX(treeWidth, splitWidths.leftWidth) : null;
  const bodyHeight = rightMode === 'diff' ? Math.max(1, listHeight - 1) : listHeight;

  useEffect(() => {
    // Tree / depth-2 commit-file EasyMotion windows. Graph jumps apply chrome
    // via `visibleGraphWindow` (not this raw left-list height).
    state.setListViewportHeight(listHeight);
  }, [listHeight, state.setListViewportHeight]);

  useEffect(() => {
    // DiffPane body = listHeight − path header; scroll clamp must match paint.
    state.setDiffViewportHeight(bodyHeight);
  }, [bodyHeight, state.setDiffViewportHeight]);

  useEffect(() => {
    // Outer pane width (narrow SxS) + codeWidth ≈ width − gutter − 4 for pan.
    state.setDiffPaneWidth(diffWidth);
  }, [diffWidth, state.setDiffPaneWidth]);

  // Graph list width follows the pane that hosts it (depth 0 right / depth 1 left).
  // Subtract 1 for the GraphPane cursor bar so segments fill the remaining cols.
  const leftIsGraph = state.navDepth === 1;
  const graphHostWidth = leftIsGraph ? treeInnerWidth : diffWidth;
  const motionSlot = easyMotionPaintSlot({
    depth: state.navDepth,
    focusPane: state.focusPane,
    graphVisible: shouldShowGraphDetail(state.nav, state.rows[state.cursor]),
  });
  useEffect(() => {
    state.setGraphPaneWidth(Math.max(1, graphHostWidth - 1));
  }, [graphHostWidth, state.setGraphPaneWidth]);

  // Graph chrome must match GraphPane paint (header / footer / listHeight).
  const graphWantHeader = Boolean(state.graphSync);
  const leftGraphChrome =
    state.navDepth === 1
      ? graphChromeBudget(listHeight, state.graphLoadingOlder, graphWantHeader)
      : null;
  const rightGraphChrome =
    rightMode === 'graph'
      ? graphChromeBudget(bodyHeight, state.graphLoadingOlder, graphWantHeader)
      : null;

  const leftHitList: HitListLayout =
    state.navDepth === 2
      ? {
          kind: 'commitFiles',
          rowCount: state.commitFileRows.length,
          cursor: state.commitFileCursor,
          rows: state.commitFileRows,
          headerLines: 0,
          footerLines: 0,
          listHeight,
        }
      : state.navDepth === 1 && leftGraphChrome
        ? {
            kind: 'graph',
            rowCount: state.graphRows.length,
            cursor: state.graphCursor,
            rows: state.graphRows.map(() => ({ depth: 0 })),
            headerLines: leftGraphChrome.header ? 1 : 0,
            footerLines: (leftGraphChrome.footer ? 2 : 0) + (state.graphLoadingOlder ? 1 : 0),
            listHeight: leftGraphChrome.listHeight,
          }
        : {
            kind: 'tree',
            rowCount: state.rows.length,
            cursor: state.cursor,
            rows: state.rows,
            headerLines: 0,
            footerLines: 0,
            listHeight,
          };

  const commitDetailHeaderLines = commitDetailHeaderHeight(
    bodyHeight,
    state.commitDetailTitle,
    state.commitDetailSubtitle,
  );

  const rightHitList: HitListLayout =
    rightMode === 'graph' && rightGraphChrome
      ? {
          kind: 'graph',
          rowCount: state.graphRows.length,
          cursor: state.graphCursor,
          rows: state.graphRows.map(() => ({ depth: 0 })),
          headerLines: rightGraphChrome.header ? 1 : 0,
          footerLines: (rightGraphChrome.footer ? 2 : 0) + (state.graphLoadingOlder ? 1 : 0),
          listHeight: rightGraphChrome.listHeight,
        }
      : rightMode === 'commitMeta'
        ? {
            kind: 'commitFiles',
            rowCount: state.commitFileRows.length,
            cursor: state.commitFileCursor,
            rows: state.commitFileRows,
            headerLines: commitDetailHeaderLines,
            footerLines: 0,
            listHeight: Math.max(1, bodyHeight - commitDetailHeaderLines),
          }
        : rightMode === 'empty'
          ? {
              kind: 'empty',
              rowCount: 0,
              cursor: 0,
              rows: [],
              headerLines: 0,
              footerLines: 0,
              listHeight: bodyHeight,
            }
          : {
              kind: 'diff',
              rowCount: 0,
              cursor: 0,
              rows: [],
              headerLines: 0,
              footerLines: 0,
              listHeight: bodyHeight,
            };

  /**
   * Latest layout + row list for the mouse stdin listener. Updated every
   * render so hit-tests never close over a stale viewport.
   */
  const layoutRef = useRef<HitTestArgs>({
    x: 0,
    y: 0,
    termCols,
    termRows,
    treeWidth,
    paneHeight,
    statusLines,
    left: leftHitList,
    right: rightHitList,
    diffSplitRuleX: splitRuleX,
    diffWidth,
  });
  layoutRef.current = {
    x: 0,
    y: 0,
    termCols,
    termRows,
    treeWidth,
    paneHeight,
    statusLines,
    left: leftHitList,
    right: rightHitList,
    diffSplitRuleX: splitRuleX,
    diffWidth,
  };

  const mouseHelpersRef = useRef({
    selectRow: state.selectRow,
    selectGraphRow: state.selectGraphRow,
    selectCommitFileRow: state.selectCommitFileRow,
    toggleFoldAt: state.toggleFoldAt,
    toggleCommitFileFoldAt: state.toggleCommitFileFoldAt,
    focusPaneSide: state.focusPaneSide,
    scrollDiffBy: state.scrollDiffBy,
    moveTreeCursorBy: state.moveTreeCursorBy,
    moveGraphCursorBy: state.moveGraphCursorBy,
    moveCommitFileCursorBy: state.moveCommitFileCursorBy,
    setTreeFraction,
    setDiffSplitFraction,
    navEnter: () => {
      void state.dispatchInput('', { return: true });
    },
    // Same modal gates as keyboard dispatch — mouse must not bypass them.
    pendingConfirm: state.pendingConfirm,
    filterMode: state.searchMode,
    branchMode: state.branchMode,
    createBranchMode: state.createBranchMode,
    graphBranchMode: state.graphBranchMode,
    stashDropMode: state.stashDropMode,
    graphCheckoutConfirmMode: state.graphCheckoutConfirmMode,
    stashMenuMode: state.stashMenuMode,
    showHelp: state.showHelp,
  });
  mouseHelpersRef.current = {
    selectRow: state.selectRow,
    selectGraphRow: state.selectGraphRow,
    selectCommitFileRow: state.selectCommitFileRow,
    toggleFoldAt: state.toggleFoldAt,
    toggleCommitFileFoldAt: state.toggleCommitFileFoldAt,
    focusPaneSide: state.focusPaneSide,
    scrollDiffBy: state.scrollDiffBy,
    moveTreeCursorBy: state.moveTreeCursorBy,
    moveGraphCursorBy: state.moveGraphCursorBy,
    moveCommitFileCursorBy: state.moveCommitFileCursorBy,
    setTreeFraction,
    setDiffSplitFraction,
    navEnter: () => {
      void state.dispatchInput('', { return: true });
    },
    pendingConfirm: state.pendingConfirm,
    filterMode: state.searchMode,
    branchMode: state.branchMode,
    createBranchMode: state.createBranchMode,
    graphBranchMode: state.graphBranchMode,
    stashDropMode: state.stashDropMode,
    graphCheckoutConfirmMode: state.graphCheckoutConfirmMode,
    stashMenuMode: state.stashMenuMode,
    showHelp: state.showHelp,
  };

  /** Last left press for double-click (same cell within DOUBLE_CLICK_MS). */
  const clickMemoryRef = useRef<ClickMemory>(null);
  /** Active left-drag resize: pane divider or in-diff split RULE. */
  const draggingDividerRef = useRef<false | 'divider' | 'diffSplit'>(false);

  // Keyboard-opened modals must clear drag without waiting for a mouse chunk.
  const mouseModalMute =
    Boolean(state.pendingConfirm) ||
    Boolean(state.searchMode) ||
    Boolean(state.branchMode) ||
    Boolean(state.createBranchMode) ||
    Boolean(state.graphBranchMode) ||
    Boolean(state.stashDropMode) ||
    Boolean(state.graphCheckoutConfirmMode) ||
    Boolean(state.stashMenuMode) ||
    Boolean(state.showHelp);
  useEffect(() => {
    if (mouseModalMute) {
      draggingDividerRef.current = false;
    }
  }, [mouseModalMute]);

  // Enable / disable SGR mouse reporting with the session flag.
  useEffect(() => {
    if (!stdout) return;
    if (state.mouseEnabled) {
      stdout.write(MOUSE_ENABLE);
    } else {
      // Release never arrives after disable — drop sticky divider drag.
      draggingDividerRef.current = false;
      stdout.write(MOUSE_DISABLE);
    }
    return () => {
      draggingDividerRef.current = false;
      stdout.write(MOUSE_DISABLE);
    };
  }, [stdout, state.mouseEnabled]);

  // Raw stdin mouse loop — only while reporting is on.
  useEffect(() => {
    if (!stdin || !state.mouseEnabled) return;

    let rest = '';
    const onData = (chunk: Buffer | string): void => {
      const text = typeof chunk === 'string' ? chunk : chunk.toString('utf8');
      const parsed = parseMouseChunk(rest + text);
      // Keep only an incomplete CSI prefix — keyboard bytes must not accumulate.
      const esc = parsed.rest.indexOf('\x1b');
      rest = esc >= 0 ? parsed.rest.slice(esc) : '';
      const layout = layoutRef.current;
      const helpers = mouseHelpersRef.current;
      // Confirm / filter / help gate keyboard; mute mouse the same way.
      if (
        helpers.pendingConfirm ||
        helpers.filterMode ||
        helpers.branchMode ||
        helpers.createBranchMode ||
        helpers.graphBranchMode ||
        helpers.stashDropMode ||
        helpers.graphCheckoutConfirmMode ||
        helpers.stashMenuMode ||
        helpers.showHelp
      ) {
        // Modal mid-drag must not leave resize sticky.
        draggingDividerRef.current = false;
        return;
      }

      for (const ev of parsed.events) {
        // End divider / split drag on left release (must run before press/wheel filter).
        if (ev.action === 'release' && ev.button === 'left') {
          draggingDividerRef.current = false;
          continue;
        }

        // While dragging: follow x until release — cursor need not stay on the bar.
        if (
          draggingDividerRef.current &&
          ev.button === 'left' &&
          (ev.action === 'drag' || ev.action === 'press')
        ) {
          if (draggingDividerRef.current === 'diffSplit') {
            helpers.setDiffSplitFraction(
              diffSplitFractionFromTerminalX(layout.treeWidth, layout.diffWidth ?? 0, ev.x),
            );
          } else {
            helpers.setTreeFraction(treeFractionFromWidth(layout.termCols, ev.x));
          }
          clickMemoryRef.current = null;
          continue;
        }

        if (ev.action === 'drag') {
          // Drag outside an active divider gesture is ignored.
          continue;
        }

        if (ev.action !== 'press' && ev.action !== 'wheel') continue;
        const hit = hitTest({ ...layout, x: ev.x, y: ev.y });

        if (ev.action === 'wheel') {
          const delta = ev.button === 'wheelUp' ? -1 : 1;
          // Wheel follows the pane under the pointer, not keyboard focus.
          if (hit.pane === 'tree') {
            helpers.moveTreeCursorBy(delta);
          } else if (hit.pane === 'graph') {
            helpers.moveGraphCursorBy(delta);
          } else if (hit.pane === 'commitFiles') {
            helpers.moveCommitFileCursorBy(delta);
          } else if (hit.pane === 'diff') {
            // Wheel over diff scrolls by a page-chunk, not one line.
            helpers.scrollDiffBy(delta * 3);
          }
          continue;
        }

        // Left press only (other buttons ignored).
        if (ev.button !== 'left') continue;

        if (hit.pane === 'divider') {
          draggingDividerRef.current = 'divider';
          helpers.setTreeFraction(treeFractionFromWidth(layout.termCols, ev.x));
          clickMemoryRef.current = null;
          continue;
        }

        if (hit.pane === 'diffSplit') {
          draggingDividerRef.current = 'diffSplit';
          helpers.setDiffSplitFraction(
            diffSplitFractionFromTerminalX(layout.treeWidth, layout.diffWidth ?? 0, ev.x),
          );
          clickMemoryRef.current = null;
          continue;
        }

        if (hit.pane === 'tree') {
          helpers.focusPaneSide('left');
          if (hit.rowIndex === null) {
            clickMemoryRef.current = null;
            continue;
          }
          const now = Date.now();
          const action = mouseListPressAction({
            pane: 'tree',
            rowIndex: hit.rowIndex,
            foldChevron: hit.foldChevron,
            doubleClick: isDoubleClick(clickMemoryRef.current, ev, now),
          });
          if (action === 'fold') {
            helpers.toggleFoldAt(hit.rowIndex);
            clickMemoryRef.current = null;
          } else if (action === 'navEnter') {
            helpers.selectRow(hit.rowIndex);
            helpers.navEnter();
            clickMemoryRef.current = null;
          } else {
            helpers.selectRow(hit.rowIndex);
            clickMemoryRef.current = { x: ev.x, y: ev.y, at: now };
          }
        } else if (hit.pane === 'graph') {
          helpers.focusPaneSide(hit.side);
          if (hit.rowIndex === null) {
            clickMemoryRef.current = null;
            continue;
          }
          const now = Date.now();
          const action = mouseListPressAction({
            pane: 'graph',
            rowIndex: hit.rowIndex,
            foldChevron: false,
            doubleClick: isDoubleClick(clickMemoryRef.current, ev, now),
          });
          if (action === 'navEnter') {
            helpers.selectGraphRow(hit.rowIndex);
            helpers.navEnter();
            clickMemoryRef.current = null;
          } else {
            helpers.selectGraphRow(hit.rowIndex);
            clickMemoryRef.current = { x: ev.x, y: ev.y, at: now };
          }
        } else if (hit.pane === 'commitFiles') {
          helpers.focusPaneSide(hit.side);
          if (hit.rowIndex === null) {
            clickMemoryRef.current = null;
            continue;
          }
          const now = Date.now();
          const action = mouseListPressAction({
            pane: 'commitFiles',
            rowIndex: hit.rowIndex,
            foldChevron: hit.foldChevron,
            doubleClick: isDoubleClick(clickMemoryRef.current, ev, now),
          });
          if (action === 'fold') {
            helpers.toggleCommitFileFoldAt(hit.rowIndex);
            clickMemoryRef.current = null;
          } else if (action === 'navEnter') {
            helpers.selectCommitFileRow(hit.rowIndex);
            helpers.navEnter();
            clickMemoryRef.current = null;
          } else {
            helpers.selectCommitFileRow(hit.rowIndex);
            clickMemoryRef.current = { x: ev.x, y: ev.y, at: now };
          }
        } else {
          const side = mouseClickFocus(hit.pane);
          if (side) helpers.focusPaneSide(side);
          clickMemoryRef.current = null;
        }
      }
    };

    stdin.on('data', onData);
    return () => {
      // Listener teardown (`m` off / unmount) drops mid-drag; release may never come.
      draggingDividerRef.current = false;
      stdin.off('data', onData);
    };
  }, [stdin, state.mouseEnabled]);

  useInput((input, key) => {
    // Mouse CSI can leak into useInput when the raw data listener and Ink
    // share stdin. Drop SGR mouse leftovers; keys are handled via KeyFlags.
    if (input.includes('\x1b[<') || input.includes('\x1b[M')) {
      return;
    }

    // Double Ctrl+C quit — same UX as Claude / Cursor (runs before overlays).
    if (isCtrlC(input, key.ctrl)) {
      const result = handleCtrlC(ctrlCRef.current);
      ctrlCRef.current = result.state;
      if (ctrlCClearTimerRef.current) {
        clearTimeout(ctrlCClearTimerRef.current);
        ctrlCClearTimerRef.current = null;
      }
      if (result.quit) {
        opts.onExit({ type: 'quit' });
        exit();
        return;
      }
      if (result.prompt) {
        state.setStatusMessage(CTRL_C_EXIT_PROMPT);
        const armedUntil = result.state.armedUntil;
        ctrlCClearTimerRef.current = setTimeout(() => {
          ctrlCClearTimerRef.current = null;
          // Disarm quietly. Only clear the bar if our prompt is still showing —
          // a later refresh/error message must not be wiped by this timer.
          if (ctrlCRef.current.armedUntil !== armedUntil) return;
          ctrlCRef.current = { armedUntil: 0 };
          if (statusMessageRef.current === CTRL_C_EXIT_PROMPT) {
            state.setStatusMessage('');
          }
        }, CTRL_C_EXIT_MS);
      }
      return;
    }

    const pageCsi = pageKeyFlagsFromInput(input);
    const flags: KeyFlags = {
      upArrow: key.upArrow,
      downArrow: key.downArrow,
      leftArrow: key.leftArrow,
      rightArrow: key.rightArrow,
      return: key.return,
      escape: key.escape,
      pageUp: key.pageUp || pageCsi.pageUp,
      pageDown: key.pageDown || pageCsi.pageDown,
      ctrl: key.ctrl,
    };

    // Ink backspace → filter / overlay delete (keys.ts does not model backspace).
    if (
      (state.searchMode ||
        state.branchMode ||
        state.createBranchMode ||
        state.graphBranchMode ||
        state.stashMenuMode ||
        state.helpSearchQuery !== null) &&
      key.backspace
    ) {
      const result = state.dispatchInput('\x7f', flags);
      if (result === 'quit') {
        opts.onExit({ type: 'quit' });
        exit();
      }
      return;
    }

    const result = state.dispatchInput(input, flags);
    if (result === 'quit') {
      opts.onExit({ type: 'quit' });
      exit();
    }
  });

  const focusHint =
    state.focused?.node.kind === 'file'
      ? `${state.focused.node.repoPath}/${state.focused.node.path}`
      : (state.focused?.label ?? '');

  const leftIsCommitFiles = state.navDepth === 2;
  const leftTitle = leftPaneTitle(state.navDepth);
  const rightTitle = rightPaneTitle(rightMode);

  return (
    <Box flexDirection="column" width={termCols} height={termRows}>
      <Box flexDirection="row" height={paneHeight}>
        <Box
          width={treeWidth}
          flexShrink={0}
          borderStyle="single"
          borderTop={false}
          borderBottom={false}
          borderLeft={false}
          borderRight={true}
          borderColor={state.focusPane === 'left' ? PALETTE.cursor : PALETTE.muted}
          flexDirection="column"
          paddingX={1}
        >
          <PaneTitle label={leftTitle} focused={state.focusPane === 'left'} />
          {leftIsCommitFiles ? (
            <TreePane
              rows={state.commitFileRows}
              cursor={state.commitFileCursor}
              height={listHeight}
              width={treeInnerWidth}
              folds={state.commitFileFolds}
              searchMatchIds={state.searchMatchIds}
              easyMotion={motionSlot === 'leftCommitFiles' && state.easyMotion}
              easyMotionTyped={state.easyMotionTyped}
              flashes={state.flashes}
              clock={state.clock}
            />
          ) : leftIsGraph ? (
            <GraphPane
              rows={state.graphRows}
              cursor={state.graphCursor}
              height={listHeight}
              width={treeInnerWidth}
              loading={state.graphLoading}
              loadingOlder={state.graphLoadingOlder}
              focused={state.focusPane === 'left'}
              sync={state.graphSync}
              model={state.graphModel}
              searchMatchIds={state.searchMatchIds}
              flashes={state.flashes}
              clock={state.clock}
              easyMotion={motionSlot === 'leftGraph' && state.easyMotion}
              easyMotionTyped={state.easyMotionTyped}
            />
          ) : (
            <TreePane
              rows={state.rows}
              cursor={state.cursor}
              height={listHeight}
              width={treeInnerWidth}
              folds={state.folds}
              searchMatchIds={state.searchMatchIds}
              easyMotion={motionSlot === 'leftTree' && state.easyMotion}
              easyMotionTyped={state.easyMotionTyped}
              flashes={state.flashes}
              clock={state.clock}
            />
          )}
        </Box>
        <Box flexGrow={1} flexDirection="column" paddingX={1}>
          <PaneTitle label={rightTitle} focused={state.focusPane === 'right'} />
          <RightPaneHost
            nav={state.nav}
            focusedRow={state.rows[state.cursor]}
            content={state.diffContent}
            loading={state.diffLoading}
            mode={state.diffMode}
            scroll={state.diffScroll}
            height={bodyHeight}
            width={diffWidth}
            focusHint={focusHint}
            fullContext={state.fullContextActive}
            colOffset={state.diffColOffset}
            splitFraction={diffSplitFraction}
            graphRows={state.graphRows}
            graphCursor={state.graphCursor}
            graphLoading={state.graphLoading}
            graphLoadingOlder={state.graphLoadingOlder}
            graphRepoPath={state.graphRepoPath}
            graphModel={state.graphModel}
            graphSync={state.graphSync}
            commitFileRows={state.commitFileRows}
            commitFileCursor={state.commitFileCursor}
            commitFileFolds={state.commitFileFolds}
            commitFilesLoading={state.commitFilesLoading}
            commitDetailTitle={state.commitDetailTitle}
            commitDetailSubtitle={state.commitDetailSubtitle}
            searchMatchIds={state.searchMatchIds}
            searchMatchDiffIndices={state.searchMatchDiffIndices}
            flashes={state.flashes}
            clock={state.clock}
            easyMotion={
              (motionSlot === 'rightGraph' || motionSlot === 'rightCommitFiles') && state.easyMotion
            }
            easyMotionTyped={state.easyMotionTyped}
          />
        </Box>
      </Box>
      {!state.showHelp ? (
        <Breadcrumb
          nav={state.nav}
          workspaceLabel={path.basename(opts.cwd) || opts.cwd}
          width={termCols}
          opStatusLine={state.opStatusLine}
        />
      ) : null}
      {exitPromptPinned ? (
        <Text color={PALETTE.modified} bold wrap="truncate">
          {CTRL_C_EXIT_PROMPT}
        </Text>
      ) : null}
      {state.pendingConfirm?.kind === 'revert' ? (
        <Confirm
          label={state.pendingConfirm.label}
          trackedCount={state.pendingConfirm.trackedCount}
          untrackedCount={state.pendingConfirm.untrackedCount}
        />
      ) : state.pendingConfirm?.kind === 'removeWorktree' ? (
        <RemoveWorktreeConfirm
          path={state.pendingConfirm.path}
          branch={state.pendingConfirm.branch}
          mergedIntoDefault={state.pendingConfirm.mergedIntoDefault}
          force={state.pendingConfirm.force}
        />
      ) : state.graphCheckoutConfirm ? (
        <GraphCheckoutConfirm
          localBranch={state.graphCheckoutConfirm.localBranch}
          remoteRef={state.graphCheckoutConfirm.remoteRef}
        />
      ) : state.stashDropConfirm ? (
        <StashDropConfirm stashRef={state.stashDropConfirm.stashRef} />
      ) : state.stashMenuOps ? (
        <StashMenuOverlay
          subtitle={state.stashMenuSubtitle}
          ops={state.stashMenuOps}
          statusMessage={state.statusMessage}
        />
      ) : state.createBranchOverlay ? (
        <CreateBranchOverlay
          commitId={state.createBranchOverlay.commitId}
          name={state.createBranchOverlay.name}
          statusMessage={state.statusMessage}
        />
      ) : state.graphBranchPicker ? (
        <GraphBranchPicker
          commitId={state.graphBranchPicker.commitId}
          branches={graphPickerBranches}
          cursor={state.graphBranchPicker.cursor}
          filter={
            state.graphBranchPicker.branches.length > 8 ? state.graphBranchPicker.filter : undefined
          }
          statusMessage={state.statusMessage}
        />
      ) : state.branchPicker ? (
        <BranchPicker
          repoPath={state.branchPicker.repoPath}
          branches={branchPickerBranches}
          cursor={state.branchPicker.cursor}
          filter={state.branchPicker.filter}
          loading={state.branchPicker.loading}
          statusMessage={state.statusMessage}
        />
      ) : (
        <StatusBar
          treeMode={state.navDepth >= 1 ? state.commitTreeMode : state.treeMode}
          filter={state.filter}
          searchMode={state.searchMode}
          search={state.search}
          easyMotion={state.easyMotion}
          easyMotionTyped={state.easyMotionTyped}
          showHelp={state.showHelp}
          helpSearchQuery={state.helpSearchQuery}
          zPending={state.zPending}
          diffMode={state.diffMode}
          rowKind={state.hintRowKind}
          graphActionRow={state.graphActionRow}
          actionGate={state.actionGate}
          graphStashExtras={{
            dirty: Boolean(state.graphModel?.uncommitted?.hasChanges),
            latestStashRef: state.graphModel?.stashes[0]?.stashRef,
          }}
          navDepth={state.navDepth}
          focusPane={state.focusPane}
          width={termCols}
        />
      )}
    </Box>
  );
}
