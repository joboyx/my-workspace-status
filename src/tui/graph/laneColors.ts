/**
 * Default lane colours (tokyo-night inspired). Rows accept this as a parameter;
 * App theme wiring is P3 — do not import `getTheme` here.
 */
export const DEFAULT_LANE_COLORS: readonly string[] = [
  '#7aa2f7', // blue
  '#bb9af7', // purple
  '#7dcfff', // cyan
  '#9ece6a', // green
  '#e0af68', // yellow
  '#f7768e', // red
  '#ff9e64', // orange
  '#73daca', // teal
] as const;
