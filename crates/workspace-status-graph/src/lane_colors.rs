//! Default lane colours and `cellsToSegments` paint helpers.
//!
//! Matches Ink `src/tui/graph/laneColors.ts` `DEFAULT_LANE_COLORS` plus
//! `cellsToSegments` in `rows.ts`. Each gutter cell uses `color_lane`.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::topology::GraphCell;

/// Tokyo-night inspired lane cycle (Ink `DEFAULT_LANE_COLORS`).
pub const DEFAULT_LANE_COLORS: [&str; 8] = [
    "#7aa2f7", // blue
    "#bb9af7", // purple
    "#7dcfff", // cyan
    "#9ece6a", // green
    "#e0af68", // yellow
    "#f7768e", // red
    "#ff9e64", // orange
    "#73daca", // teal
];

/// Parse `#rrggbb` into a ratatui colour. Invalid hex falls back to white.
pub fn hex_color(hex: &str) -> Color {
    let bytes = hex.strip_prefix('#').unwrap_or(hex);
    if bytes.len() != 6 {
        return Color::White;
    }
    let Ok(r) = u8::from_str_radix(&bytes[0..2], 16) else {
        return Color::White;
    };
    let Ok(g) = u8::from_str_radix(&bytes[2..4], 16) else {
        return Color::White;
    };
    let Ok(b) = u8::from_str_radix(&bytes[4..6], 16) else {
        return Color::White;
    };
    Color::Rgb(r, g, b)
}

/// Resolved [`DEFAULT_LANE_COLORS`] as ratatui colours.
pub fn default_lane_colors() -> [Color; 8] {
    DEFAULT_LANE_COLORS.map(hex_color)
}

/// Foreground for `color_lane`, wrapping the palette. `None` uses `fallback`.
pub fn lane_fg(color_lane: Option<usize>, lane_colors: &[Color], fallback: Color) -> Color {
    match color_lane {
        Some(lane) if !lane_colors.is_empty() => lane_colors[lane % lane_colors.len()],
        _ => fallback,
    }
}

/// Collapse adjacent same-colour gutter cells into spans (Ink `cellsToSegments`).
pub fn cells_to_spans<'a>(
    cells: &'a [GraphCell],
    lane_colors: &[Color],
    fallback: Color,
) -> Vec<Span<'a>> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    for cell in cells {
        let fg = lane_fg(cell.color_lane, lane_colors, fallback);
        if let Some(last) = spans.last_mut() {
            if last.style.fg == Some(fg) {
                last.content.to_mut().push_str(&cell.ch);
                continue;
            }
        }
        spans.push(Span::styled(cell.ch.clone(), Style::default().fg(fg)));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::{CellRole, GraphCell};

    #[test]
    fn default_lane_colors_are_eight_distinct_hex() {
        assert_eq!(DEFAULT_LANE_COLORS.len(), 8);
        let set: std::collections::HashSet<_> = DEFAULT_LANE_COLORS.iter().copied().collect();
        assert_eq!(set.len(), 8);
        for c in DEFAULT_LANE_COLORS {
            assert!(c.starts_with('#') && c.len() == 7, "{c}");
        }
    }

    #[test]
    fn cells_to_spans_colors_by_lane() {
        let colors = default_lane_colors();
        let cells = [
            GraphCell {
                ch: "│".into(),
                color_lane: Some(0),
                role: CellRole::Pipe,
            },
            GraphCell {
                ch: "│".into(),
                color_lane: Some(1),
                role: CellRole::Pipe,
            },
            GraphCell {
                ch: " ".into(),
                color_lane: None,
                role: CellRole::Blank,
            },
        ];
        let spans = cells_to_spans(&cells, &colors, Color::White);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].style.fg, Some(colors[0]));
        assert_eq!(spans[1].style.fg, Some(colors[1]));
        assert_eq!(spans[2].style.fg, Some(Color::White));
    }

    #[test]
    fn cells_to_spans_merges_adjacent_same_lane() {
        let colors = default_lane_colors();
        let cells = [
            GraphCell {
                ch: "│".into(),
                color_lane: Some(0),
                role: CellRole::Pipe,
            },
            GraphCell {
                ch: "─".into(),
                color_lane: Some(0),
                role: CellRole::Pipe,
            },
        ];
        let spans = cells_to_spans(&cells, &colors, Color::White);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "│─");
    }
}
