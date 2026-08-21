import type { RowKind } from './actions/registry.js';
import type { GraphActionRow } from './graph/actions.js';
import type { VisibleRow } from './model/types.js';
import type { FocusPane } from './nav/stack.js';
import { collectFiles } from './scope.js';

/** Overlay op id for the stash family (`s` / `a` / `p` / `d`). */
export type StashOpId = 'push' | 'apply' | 'pop' | 'drop';

/** One stash overlay row: key, label, and optional git target. */
export type StashOp = {
  readonly id: StashOpId;
  readonly key: 's' | 'a' | 'p' | 'd';
  readonly label: string;
  readonly stashRef?: string;
  readonly paths?: readonly string[];
};

/**
 * Focused-row facts used to list valid stash overlay ops.
 * `dirtyPaths` are file/dir pathspecs; omit for a whole-tree stash.
 */
export type StashOpsContext = {
  readonly kind: RowKind;
  readonly dirty: boolean;
  readonly dirtyPaths?: readonly string[];
  readonly focusedStashRef?: string;
  readonly latestStashRef?: string;
};

/**
 * Overlay ops valid for `ctx`, in push → apply → pop → drop order.
 * Empty when nothing applies. Drop requires a focused stash ref (never
 * “drop latest” from a non-stash row).
 */
export function stashOpsForContext(ctx: StashOpsContext): StashOp[] {
  const ops: StashOp[] = [];
  if (ctx.dirty) {
    const paths = ctx.dirtyPaths;
    ops.push(
      paths && paths.length > 0
        ? { id: 'push', key: 's', label: 'stash', paths }
        : { id: 'push', key: 's', label: 'stash' },
    );
  }
  const applyRef = ctx.focusedStashRef ?? ctx.latestStashRef;
  if (applyRef) {
    ops.push({ id: 'apply', key: 'a', label: 'apply stash', stashRef: applyRef });
    ops.push({ id: 'pop', key: 'p', label: 'pop stash', stashRef: applyRef });
  }
  if (ctx.focusedStashRef) {
    ops.push({
      id: 'drop',
      key: 'd',
      label: 'drop stash',
      stashRef: ctx.focusedStashRef,
    });
  }
  return ops;
}

/** Nav + focus facts used to build overlay context. */
export type BuildStashOpsContextInput = {
  readonly navDepth: number;
  readonly focusPane: FocusPane;
  readonly focused: VisibleRow | null;
  readonly graphRow: GraphActionRow | null;
  readonly graphDirty: boolean;
  readonly latestStashRef?: string;
};

function isDirtyFile(file: { staged: boolean; unstaged: boolean; untracked: boolean }): boolean {
  return file.staged || file.unstaged || file.untracked;
}

function treeStashOpsContext(focused: VisibleRow): StashOpsContext {
  const kind = focused.node.kind;
  const files = collectFiles(focused).filter(isDirtyFile);
  const dirty = files.length > 0;
  if (kind === 'file' || kind === 'dir') {
    return {
      kind,
      dirty,
      dirtyPaths: dirty ? files.map((f) => f.path) : undefined,
    };
  }
  return { kind, dirty };
}

function graphStashOpsContext(
  row: GraphActionRow,
  graphDirty: boolean,
  latestStashRef?: string,
): StashOpsContext {
  return {
    kind:
      row.kind === 'stash'
        ? 'graphStash'
        : row.kind === 'uncommitted'
          ? 'graphUncommitted'
          : 'graphCommit',
    dirty: graphDirty,
    focusedStashRef: row.kind === 'stash' ? row.stash.stashRef : undefined,
    latestStashRef,
  };
}

/**
 * Overlay context from the focused tree row (depth 0) or graph selection
 * (depth 1 left). Null on the right pane or at depth ≥ 2.
 */
export function buildStashOpsContext(
  input: BuildStashOpsContextInput,
): StashOpsContext | null {
  if (input.navDepth >= 2 || input.focusPane !== 'left') return null;
  if (input.navDepth === 1) {
    if (!input.graphRow) return null;
    return graphStashOpsContext(
      input.graphRow,
      input.graphDirty,
      input.latestStashRef,
    );
  }
  if (!input.focused) return null;
  return treeStashOpsContext(input.focused);
}

/**
 * Workspace-relative git cwd for a tree row: `repoPath` on file/dir,
 * `path` on repo/checkout. Null for containers with no checkout.
 */
export function stashRepoRelPath(focused: VisibleRow | null): string | null {
  if (!focused) return null;
  const node = focused.node;
  if (node.kind === 'file' || node.kind === 'dir') return node.repoPath;
  if (node.kind === 'repo' || node.kind === 'checkout') return node.path;
  return null;
}

/** Overlay key outcome while the stash menu is open. */
export type StashMenuKeyResult =
  | { readonly type: 'cancel' }
  | { readonly type: 'run'; readonly op: StashOp }
  | { readonly type: 'ignore' };

/**
 * Map overlay input to cancel / run / ignore. Enter runs the first listed op.
 * Unknown keys (including Shift+s) are ignored.
 */
export function resolveStashMenuKey(
  input: string,
  flags: { readonly return?: boolean; readonly escape?: boolean },
  ops: readonly StashOp[],
): StashMenuKeyResult {
  if (flags.escape) return { type: 'cancel' };
  if (flags.return) {
    const first = ops[0];
    return first ? { type: 'run', op: first } : { type: 'ignore' };
  }
  if (!input || input.length !== 1) return { type: 'ignore' };
  const op = ops.find((item) => item.key === input);
  return op ? { type: 'run', op } : { type: 'ignore' };
}

/** Status after a successful stash push. Paths → `Stashed 1 file` / `Stashed N files`. */
export function stashPushStatus(paths?: readonly string[]): string {
  if (!paths || paths.length === 0) return 'Stashed';
  const n = paths.length;
  return n === 1 ? 'Stashed 1 file' : `Stashed ${n} files`;
}

/** Overlay subtitle: focused stash ref, else repo path. */
export function stashMenuSubtitle(input: {
  readonly focusedStashRef?: string;
  readonly repoPath?: string;
}): string {
  return input.focusedStashRef ?? input.repoPath ?? '';
}

/** Extra muted text on an overlay row — stash ref for apply/pop/drop. */
export function stashMenuOpDetail(op: StashOp): string | undefined {
  if (op.id === 'apply' || op.id === 'pop' || op.id === 'drop') return op.stashRef;
  return undefined;
}
