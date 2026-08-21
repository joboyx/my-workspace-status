/**
 * Build title/subtitle for CommitDetailPane from the selected graph row.
 */

import path from 'node:path';
import type { GraphListRow } from '../graph/list.js';
import type { GraphModel } from '../graph/types.js';

/**
 * Header copy for the depth-1 commit detail pane.
 */
export function commitDetailMetaFromRow(
  row: GraphListRow | null,
  repoPath: string,
  model: GraphModel | null,
): { title: string; subtitle?: string } {
  const repo = path.basename(repoPath) || repoPath || 'repo';
  if (!row) return { title: repo, subtitle: 'select a commit' };
  if (row.kind === 'uncommitted') {
    return { title: repo, subtitle: 'Uncommitted changes' };
  }
  if (row.kind === 'spacer') {
    return { title: repo, subtitle: 'select a commit' };
  }
  if (row.kind === 'stash') {
    const ref = row.stashRef ?? row.commitId ?? 'stash';
    const stash =
      model?.stashes.find((s) => s.stashRef === row.stashRef) ??
      model?.stashes.find((s) => s.id === row.commitId);
    const subject = stash?.subject ?? row.segments.map((s) => s.text).join('').trim();
    return { title: repo, subtitle: subject ? `${ref} · ${subject}` : ref };
  }
  const short = (row.commitId ?? '').slice(0, 7);
  const commit = model?.commits.find((c) => c.id === row.commitId);
  const subject =
    commit?.subject ?? row.segments.map((s) => s.text).join('').trim();
  const refNames = (commit?.refs ?? []).map((r) => r.name).join(', ');
  const bits = [short];
  if (refNames) bits.push(refNames);
  bits.push(subject);
  if (commit?.authorName) bits.push(commit.authorName);
  return { title: repo, subtitle: bits.filter(Boolean).join(' · ') };
}
