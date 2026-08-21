/**
 * Default number of commits loaded per graph window (spec locked default).
 */
export const DEFAULT_GRAPH_WINDOW = 300;

/** Kind of annotated ref attached to a commit. */
export type GraphRefKind = 'local' | 'remote' | 'tag';

/**
 * A branch or tag label pointing at a commit.
 */
export type GraphRef = {
  kind: GraphRefKind;
  name: string;
  commitId: string;
};

/**
 * One commit in the loaded graph window (newest-first topo order from git).
 */
export type GraphCommit = {
  id: string;
  parents: string[];
  subject: string;
  authorName: string;
  authorDateUnix: number;
  refs: GraphRef[];
};

/**
 * One stash entry from `git stash list`.
 */
export type GraphStash = {
  id: string;
  stashRef: string;
  index: number;
  subject: string;
  authorName: string;
  authorDateUnix: number;
  /**
   * First parent of the stash tip (`stash^1`) — HEAD at stash time.
   * Empty when git did not report parents.
   */
  parentId: string;
};

/**
 * Synthetic row for dirty / clean worktree above the commit list.
 */
export type GraphUncommitted = {
  kind: 'uncommitted';
  hasChanges: boolean;
};

/**
 * Assembled graph payload for one repo + window.
 */
export type GraphModel = {
  repoPath: string;
  /**
   * Newest-first log window, then optional extra `stash^1` commits that were
   * outside that window. {@link windowCount} marks the log-window prefix.
   */
  commits: GraphCommit[];
  stashes: GraphStash[];
  uncommitted: GraphUncommitted | null;
  /** Full SHA of `HEAD` (incl. detached); null only when empty/unavailable. */
  headId: string | null;
  refsFingerprint: string;
  skip: number;
  limit: number;
  hasMore: boolean;
  /**
   * Length of the `git log` window prefix in {@link commits} (excludes extra
   * stash parents). Autoload skip uses this so extras do not drop history.
   * Omitted on older fixtures — treat as `commits.length`.
   */
  windowCount?: number;
};

/**
 * One terminal column in the graph gutter (lazygit-style cell matrix).
 */
export type GraphCell = {
  /** Exactly one terminal column. */
  ch: string;
  /** Lane colour role index, or null for blank. */
  colorLane: number | null;
  role: 'node' | 'pipe' | 'blank';
};

/**
 * One live stem endpoint identified by the commit id the rail is waiting on
 * (or the commit itself for an upward stem into a node).
 *
 * Used to keep densify rails continuous across remaps — match by `id`,
 * not absolute column.
 */
export type GraphStemRef = {
  /** Gutter column (lane * CELL_W). */
  col: number;
  /** Waiter / commit identity for this stem. */
  id: string;
  /** Lane colour role for the rail. */
  colorLane: number;
};

/**
 * Commit after lane assignment — coloured cell gutter (+ derived `edges` string).
 */
export type LaidOutCommit = {
  commit: GraphCommit;
  lane: number;
  laneCount: number;
  /** Padded to the window's max gutter width. */
  cells: GraphCell[];
  /** `cells.map(c => c.ch).join('')` — tests / debug. */
  edges: string;
  /**
   * Upward stems into this row (topology), keyed by rail identity.
   * Matched to the previous row's `stemDown` by `id` for densify gutters.
   */
  stemUp: GraphStemRef[];
  /**
   * Downward stems leaving this row (topology), keyed by rail identity.
   * Open corners count; close-only corners do not.
   */
  stemDown: GraphStemRef[];
};
