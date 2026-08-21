/**
 * Full-file view helpers — unlimited git diff context per focused file row.
 */

/**
 * Toggle whether `id` is shown with unlimited unified context.
 */
export function toggleFullContext(id: string, prev: Set<string>): Set<string> {
  const next = new Set(prev);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

/**
 * Diff-cache key including the full-context flag so normal and full entries
 * do not collide.
 */
export function diffCacheKey(repo: string, filePath: string, full: boolean): string {
  return `${repo}::${filePath}${full ? '::full' : ''}`;
}
