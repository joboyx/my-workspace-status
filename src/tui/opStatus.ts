/**
 * Top-chrome op status: fetch age/progress, long-running pull / push /
 * default-branch, and ephemeral toasts (status messages).
 *
 * Pure formatting + row allocation so the breadcrumb row can host a trailing
 * status without crowding the bottom action hints.
 */

/** Long-running actions that share the top op-status slot with fetch. */
export type ActionOpKind = 'pull' | 'push' | 'defaultBranch';

/** Max ephemeral toast fragments appended after the primary op/fetch slot. */
const MAX_TOASTS = 3;

/**
 * Settled/total repo counts for an in-progress pull / push / default-branch.
 */
export type ActionOpProgress = {
  done: number;
  total: number;
};

/**
 * Inputs for {@link formatTopOpStatus}.
 */
export type TopOpStatusInput = {
  /** In-progress pull / push / default-branch (takes priority over fetch). */
  actionOp: ActionOpKind | null;
  /** Settled/total repos for the in-progress action (same shape as fetch). */
  actionOpProgress?: ActionOpProgress | null;
  /** Fetch age or in-flight progress already formatted by useFetch. */
  fetchStatusLine: string;
  /** Ephemeral status fragments (e.g. from setStatusMessage). */
  toasts?: readonly string[];
};

/**
 * Short in-progress label for pull / push / default-branch.
 * Matches fetch: `Verb done/total…` (e.g. `Pulling 2/18…`).
 */
export function formatActionOpProgress(kind: ActionOpKind, done = 0, total = 0): string {
  const verb = kind === 'pull' ? 'Pulling' : kind === 'push' ? 'Pushing' : 'Switching';
  if (total <= 0) return `${verb}…`;
  return `${verb} ${done}/${total}…`;
}

/**
 * True when op-status text looks like a failure (same cue as old StatusBar).
 */
export function isOpStatusError(text: string): boolean {
  return /failed|error/i.test(text);
}

/**
 * Trailing top-chrome status: action/fetch primary slot, then up to 3 toasts.
 * Fragments join with `' · '`.
 */
export function formatTopOpStatus(input: TopOpStatusInput): string {
  const primary = input.actionOp
    ? formatActionOpProgress(
        input.actionOp,
        input.actionOpProgress?.done ?? 0,
        input.actionOpProgress?.total ?? 0,
      )
    : input.fetchStatusLine;
  const toastParts = (input.toasts ?? []).filter((t) => t.length > 0).slice(0, MAX_TOASTS);
  const parts = [primary, ...toastParts].filter((p) => p.length > 0);
  return parts.join(' · ');
}

/**
 * Allocate breadcrumb vs trailing op-status columns on one chrome row.
 * Op status keeps its width when it fits; breadcrumb truncates first.
 */
export function allocateChromeRow(
  totalWidth: number,
  opStatusLen: number,
  gap = 1,
): { breadcrumbMax: number; opStatusMax: number } {
  const width = Math.max(0, totalWidth);
  if (opStatusLen <= 0 || width <= 0) {
    return { breadcrumbMax: width, opStatusMax: 0 };
  }
  const opStatusMax = Math.min(opStatusLen, width);
  const breadcrumbMax = Math.max(0, width - opStatusMax - gap);
  return { breadcrumbMax, opStatusMax };
}
