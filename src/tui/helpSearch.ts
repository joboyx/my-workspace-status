/**
 * Pure helpers for `?` help-overlay `/` search (highlight only — never hides rows).
 */

/** Concatenated text matched for a help key row (chips + description). */
export function helpEntryLabel(keys: string, desc: string): string {
  return `${keys} ${desc}`;
}

/**
 * Case-insensitive substring match on keys + description.
 * Empty / whitespace-only query → no match (no highlight while typing starts).
 */
export function helpEntryMatches(keys: string, desc: string, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return false;
  return helpEntryLabel(keys, desc).toLowerCase().includes(q);
}
