/**
 * EasyMotion label assign / resolve (C4).
 * Labels cover currently visible rows only (viewport-relative).
 */

const ALPHA = 'abcdefghijklmnopqrstuvwxyz';

/** Labels `a`–`z`, then `aa`, `ab`, … enough for `count`. */
export function easyMotionLabels(count: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < count; i++) {
    if (i < 26) {
      out.push(ALPHA[i]!);
      continue;
    }
    const n = i - 26;
    const first = Math.floor(n / 26);
    const second = n % 26;
    out.push(`${ALPHA[first]!}${ALPHA[second]!}`);
  }
  return out;
}

/** Resolve typed prefix against assigned labels. */
export function resolveEasyMotionLabel(
  labels: string[],
  typed: string,
): { status: 'partial' | 'hit' | 'miss'; index?: number } {
  if (!typed) return { status: 'partial' };
  const exact = labels.indexOf(typed);
  if (exact >= 0) return { status: 'hit', index: exact };
  const hasPrefix = labels.some((l) => l.startsWith(typed));
  if (hasPrefix) return { status: 'partial' };
  return { status: 'miss' };
}

/**
 * Resolve a typed EasyMotion prefix against the painted visible window.
 * On hit, `index` is the absolute list index (`start + label slot`).
 */
export function resolveEasyMotionJump(
  visibleCount: number,
  start: number,
  typed: string,
): { status: 'partial' | 'hit' | 'miss'; index?: number } {
  const resolved = resolveEasyMotionLabel(easyMotionLabels(visibleCount), typed);
  if (resolved.status === 'hit') {
    return { status: 'hit', index: start + (resolved.index ?? 0) };
  }
  return { status: resolved.status };
}
