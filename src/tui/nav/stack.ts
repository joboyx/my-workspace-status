/**
 * ViewStack + focusPane transitions for the TUI navigation shell (JBY-037 P1).
 *
 * Pure functions only — App / useAppState own React state and persistence.
 */

import path from 'node:path';

/** One frame on the navigation stack (length 1..3; top is current). */
export type ViewDepth =
  | { kind: 'workspace' }
  | { kind: 'repoGraph'; repo: string; commitId: string | null }
  | { kind: 'commitFiles'; repo: string; commitId: string; filePath: string | null };

/** Which pane receives list navigation / Enter drill. */
export type FocusPane = 'left' | 'right';

/** Restorable navigation shell state. */
export interface NavState {
  stack: ViewDepth[];
  focusPane: FocusPane;
}

/**
 * Context gathered from the focused row when drilling (right + Enter).
 * P1 stubs use this; later phases fill real commit / file selection.
 */
export interface NavDrillContext {
  repo: string;
  commitId: string | null;
  filePath: string | null;
}

/** Fresh nav: depth 0 workspace, left pane focused. */
export function createNavState(): NavState {
  return {
    stack: [{ kind: 'workspace' }],
    focusPane: 'left',
  };
}

/** Zero-based depth index of the current frame (`stack.length - 1`). */
export function navDepth(nav: NavState): 0 | 1 | 2 {
  return (nav.stack.length - 1) as 0 | 1 | 2;
}

/** Top of the stack (current view). */
export function currentView(nav: NavState): ViewDepth {
  return nav.stack[nav.stack.length - 1]!;
}

/**
 * Enter ladder: left → focus right; right → push next depth and stay on right.
 * Leaf Enter at depth 2 is a no-op. Empty `drill.repo` blocks a depth-0 push.
 */
export function applyNavEnter(nav: NavState, drill: NavDrillContext): NavState {
  if (nav.focusPane === 'left') {
    return { ...nav, focusPane: 'right' };
  }

  const depth = navDepth(nav);
  if (depth === 2) {
    return nav;
  }

  if (depth === 0) {
    if (!drill.repo) return nav;
    return {
      stack: [
        ...nav.stack,
        { kind: 'repoGraph', repo: drill.repo, commitId: drill.commitId },
      ],
      focusPane: 'right',
    };
  }

  // depth === 1 — prefer drill (graph cursor) over stack commitId
  const top = currentView(nav);
  const repo = top.kind === 'repoGraph' ? top.repo : drill.repo;
  const commitId =
    drill.commitId ?? (top.kind === 'repoGraph' ? top.commitId : null) ?? 'WORKTREE';
  return {
    stack: [
      ...nav.stack,
      { kind: 'commitFiles', repo, commitId, filePath: null },
    ],
    focusPane: 'right',
  };
}

/**
 * Esc ladder (sole back key): right → left; left → pop and stay on left.
 * Left at depth 0 is a no-op.
 */
export function applyNavEsc(nav: NavState): NavState {
  if (nav.focusPane === 'right') {
    return { ...nav, focusPane: 'left' };
  }
  if (nav.stack.length <= 1) {
    return nav;
  }
  return {
    stack: nav.stack.slice(0, -1),
    focusPane: 'left',
  };
}

function shortHash(id: string): string {
  return id.length > 7 ? id.slice(0, 7) : id;
}

/** Breadcrumb label for a commit id — WORKTREE is not a git hash. */
function commitLabel(id: string): string {
  if (id === 'WORKTREE') return 'uncommitted';
  return shortHash(id);
}

function baseName(p: string): string {
  const base = path.basename(p);
  return base || p;
}

/**
 * Display segments for the breadcrumb (workspace label + stack frames).
 * Display-only — never used for focus navigation.
 *
 * Each deeper frame only appends parts not already shown by a shallower
 * frame (repo / commit), so depth-2 does not repeat `repo › hash`.
 */
export function breadcrumbSegments(nav: NavState, workspaceLabel: string): string[] {
  const out: string[] = [workspaceLabel];
  let seenRepo: string | null = null;
  let seenCommit: string | null = null;

  for (const frame of nav.stack) {
    if (frame.kind === 'workspace') continue;

    if (frame.kind === 'repoGraph') {
      const repo = baseName(frame.repo);
      out.push(repo);
      seenRepo = repo;
      if (frame.commitId) {
        const hash = commitLabel(frame.commitId);
        out.push(hash);
        seenCommit = hash;
      }
      continue;
    }

    // commitFiles — append only new parts
    const repo = baseName(frame.repo);
    const hash = commitLabel(frame.commitId);
    if (seenRepo !== repo) {
      out.push(repo);
      seenRepo = repo;
    }
    if (seenCommit !== hash) {
      out.push(hash);
      seenCommit = hash;
    }
    if (frame.filePath) out.push(baseName(frame.filePath));
  }
  return out;
}

/**
 * Single-line breadcrumb string including a focus marker for tests / ascii fallback.
 */
export function formatBreadcrumb(segments: string[], focusPane: FocusPane): string {
  return `${segments.join(' › ')} · ${focusPane}`;
}
