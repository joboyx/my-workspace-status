import type { CommitFileSource } from './types.js';

/**
 * Stable identity for the commit-file list (repo + load source).
 * Used so rematerialize effects do not re-fire when only breadcrumb
 * `filePath` on the nav stack changes.
 *
 * Examples: `repo|worktree`, `repo|commit:abc`, `repo|stash:stash@{0}`, `''`.
 */
export function commitFilesListKey(
  repo: string | null,
  source: CommitFileSource | null,
): string {
  if (!repo || !source) return '';
  if (source.kind === 'worktree') return `${repo}|worktree`;
  if (source.kind === 'commit') return `${repo}|commit:${source.commitId}`;
  return `${repo}|stash:${source.stashRef}`;
}
