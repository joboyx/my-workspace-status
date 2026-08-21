/**
 * Validate a new branch name (non-empty, no spaces, no leading '-').
 */
export function isValidBranchName(name: string): boolean {
  const t = name.trim();
  return t.length > 0 && !/\s/.test(t) && !t.startsWith('-');
}
