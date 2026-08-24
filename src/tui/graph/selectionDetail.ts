/**
 * GraphPane chrome helpers — sync header + selection detail footer.
 */

import { isDefaultBranch } from '../../helpers.js';
import type { SyncStatus } from '../../types.js';
import { syncColor, truncateSegments, tuiSyncMark } from '../icons.js';
import type { Segment } from '../theme.js';
import { segmentsText } from '../theme.js';
import type { GraphListRow } from './list.js';
import {
  formatRelativeDate,
  graphRefChipSegments,
  type GraphRowOptions,
} from './rows.js';
import type { GraphModel, GraphRef } from './types.js';

/** Sync chrome passed into GraphPane from the focused repo snapshot. */
export type GraphSyncChrome = {
  branch: string;
  syncStatus: SyncStatus;
  syncNote: string;
  /** Configured default-branch override when present. */
  defaultBranchOverride?: string;
};

/**
 * How many header/footer lines GraphPane should reserve for a given height.
 * Prefer footer over header when space is tight (`drop header before footer`).
 * Pass `wantHeader=false` when sync chrome will not paint so we do not reserve
 * a blank header row.
 */
export function graphChromeBudget(
  height: number,
  loadingOlder = false,
  wantHeader = true,
): { header: boolean; footer: boolean; listHeight: number } {
  const older = loadingOlder ? 1 : 0;
  let avail = Math.max(1, height - older);
  // Footer first: needs 2 chrome + ≥1 list row.
  const footer = avail >= 3;
  if (footer) avail -= 2;
  // Header only if caller will paint it and ≥1 list row remains after.
  const header = wantHeader && avail >= 2;
  if (header) avail -= 1;
  return {
    header,
    footer,
    listHeight: Math.max(1, avail),
  };
}

/**
 * One-line sync header segments: `branch  ⬆️2`.
 */
export function graphSyncHeaderSegments(
  sync: GraphSyncChrome,
  opts: { width: number; branchColor?: string; mutedColor?: string } = { width: 80 },
): Segment[] {
  const muted = opts.mutedColor ?? '#565f89';
  const branchColor =
    opts.branchColor ??
    (isDefaultBranch(sync.branch, sync.defaultBranchOverride) ? muted : '#bb9af7');
  const mark = tuiSyncMark(sync.syncStatus, sync.syncNote);
  const markColor = syncColor(sync.syncStatus);
  const segs: Segment[] = [
    { text: sync.branch, color: branchColor, bold: true },
    { text: '  ', color: muted },
    { text: mark, color: markColor },
  ];
  const text = segmentsText(segs);
  if (text.length <= opts.width) return segs;
  // Truncate branch first so the sync mark stays visible.
  const markBudget = Math.min(mark.length + 2, opts.width);
  const branchBudget = Math.max(1, opts.width - markBudget);
  const branch =
    sync.branch.length <= branchBudget
      ? sync.branch
      : sync.branch.slice(0, Math.max(1, branchBudget - 1)) + '…';
  return [
    { text: branch, color: branchColor, bold: true },
    { text: '  ', color: muted },
    { text: mark.slice(0, Math.max(1, opts.width - branch.length - 2)), color: markColor },
  ];
}

function truncLine(text: string, width: number): string {
  if (width <= 0) return '';
  if (text.length <= width) return text;
  if (width === 1) return '…';
  return text.slice(0, width - 1) + '…';
}

function refSegments(
  refs: GraphRef[],
  opts: GraphRowOptions,
  muted: string,
  isHead: boolean,
): Segment[] {
  const segs = graphRefChipSegments(refs, opts, isHead);
  if (segs.length === 0) return [{ text: '(no refs)', color: muted }];
  return segs;
}

/**
 * Selection detail for the GraphPane footer (and tests).
 */
export function graphSelectionDetailLines(
  row: GraphListRow | null | undefined,
  model: GraphModel | null | undefined,
  opts: GraphRowOptions & { width: number },
): { footer: Segment[][] } {
  const muted = opts.mutedColor ?? '#565f89';
  const subjectColor = opts.subjectColor ?? '#c0caf5';
  const width = Math.max(1, opts.width);
  const now = opts.nowUnix ?? Math.floor(Date.now() / 1000);

  if (!row) {
    return {
      footer: [
        [{ text: truncLine('no selection', width), color: muted }],
        [{ text: '', color: muted }],
      ],
    };
  }

  if (row.kind === 'uncommitted') {
    const hasChanges = model?.uncommitted?.hasChanges ?? true;
    const line = hasChanges ? 'Uncommitted changes' : 'Working tree clean';
    const head = model?.headId
      ? model.commits.find((c) => c.id === model.headId)
      : undefined;
    const chipSegs = head
      ? graphRefChipSegments(head.refs, opts, true)
      : [];
    const line2 =
      chipSegs.length > 0
        ? truncateSegments(chipSegs, width)
        : [{ text: truncLine('worktree · not a commit', width), color: muted }];
    return {
      footer: [
        [{ text: truncLine(line, width), color: subjectColor }],
        line2,
      ],
    };
  }

  if (row.kind === 'spacer') {
    return {
      footer: [
        [{ text: truncLine('…', width), color: muted }],
        [{ text: truncLine('connector · not selectable', width), color: muted }],
      ],
    };
  }

  if (row.kind === 'stash') {
    const stash =
      model?.stashes.find((s) => s.stashRef === row.stashRef) ??
      model?.stashes.find((s) => s.id === row.commitId);
    const subject = stash?.subject ?? 'stash';
    const ref = row.stashRef ?? stash?.stashRef ?? 'stash';
    const date = stash
      ? formatRelativeDate(stash.authorDateUnix, now)
      : '';
    const meta = [ref, row.commitId?.slice(0, 7), date].filter(Boolean).join(' · ');
    return {
      footer: [
        [{ text: truncLine(subject, width), color: subjectColor }],
        [{ text: truncLine(meta, width), color: muted }],
      ],
    };
  }

  const commit = model?.commits.find((c) => c.id === row.commitId);
  const subject = commit?.subject ?? 'commit';
  const refs = commit?.refs ?? [];
  const isHead = Boolean(model?.headId && row.commitId === model.headId);
  const hash = (row.commitId ?? '').slice(0, 7);
  const author = commit?.authorName ?? '';
  const date = commit
    ? formatRelativeDate(commit.authorDateUnix, now)
    : '';

  const line2: Segment[] = [
    ...refSegments(refs, opts, muted, isHead),
    { text: ' · ', color: muted },
    { text: hash || '???????', color: muted },
  ];
  if (author) {
    line2.push({ text: ' · ', color: muted }, { text: author, color: muted });
  }
  if (date) {
    line2.push({ text: ' · ', color: muted }, { text: date, color: muted });
  }

  // Soft-truncate line 2 by dropping trailing segments if needed.
  let line2Text = segmentsText(line2);
  let trimmed = line2;
  const isSep = (s: Segment) => /^[ ·]+$/.test(s.text);
  while (line2Text.length > width && trimmed.length > 1) {
    trimmed = trimmed.slice(0, -1);
    while (trimmed.length > 0 && isSep(trimmed[trimmed.length - 1]!)) {
      trimmed = trimmed.slice(0, -1);
    }
    line2Text = segmentsText(trimmed);
  }
  if (line2Text.length > width) {
    // Keep chip colours; do not flatten the overflow run to muted.
    trimmed = truncateSegments(trimmed, width);
  }

  return {
    footer: [[{ text: truncLine(subject, width), color: subjectColor }], trimmed],
  };
}
