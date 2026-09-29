//! Mouse drag text selection inside one pane.
//!
//! A left press in a pane body arms a [`TextSelection`]. Drags move its head,
//! clamped to that pane. The shape is a terminal-style stream: the first row
//! runs from the start column to the pane right edge, middle rows span the
//! pane, and the last row runs from the pane left edge to the end column.
//! Text comes from the last painted frame, so the copy matches the screen.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::Span;

/// A left-button drag that selects on-screen text inside one pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSelection {
    /// Inner rect of the pane where the press landed. The selection never
    /// leaves it.
    pub pane: Rect,
    /// Press cell `(col, row)`.
    pub anchor: (u16, u16),
    /// Latest drag cell `(col, row)`, clamped into [`TextSelection::pane`].
    pub head: (u16, u16),
}

/// One painted row of a selection: `row`, first and last column (inclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionSpan {
    /// 0-based screen row.
    pub row: u16,
    /// First selected column.
    pub x0: u16,
    /// Last selected column (inclusive).
    pub x1: u16,
}

impl TextSelection {
    /// Arm a selection at the press cell when it lies inside `pane`.
    ///
    /// Returns `None` for a press outside `pane` or an empty pane.
    pub fn arm(pane: Rect, col: u16, row: u16) -> Option<Self> {
        if !pane.contains(Position::new(col, row)) {
            return None;
        }
        Some(Self {
            pane,
            anchor: (col, row),
            head: (col, row),
        })
    }

    /// Move the head to the drag cell, clamped into the pane.
    pub fn extend_to(&mut self, col: u16, row: u16) {
        let right = self.pane.right().saturating_sub(1);
        let bottom = self.pane.bottom().saturating_sub(1);
        self.head = (
            col.clamp(self.pane.x, right),
            row.clamp(self.pane.y, bottom),
        );
    }

    /// True once the head has left the press cell. A plain click never selects.
    pub fn is_active(&self) -> bool {
        self.head != self.anchor
    }

    /// Selected cells per row, top to bottom. Empty while not active.
    pub fn spans(&self) -> Vec<SelectionSpan> {
        if !self.is_active() {
            return Vec::new();
        }
        let (start, end) = if (self.anchor.1, self.anchor.0) <= (self.head.1, self.head.0) {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        let left = self.pane.x;
        let right = self.pane.right().saturating_sub(1);
        (start.1..=end.1)
            .map(|row| SelectionSpan {
                row,
                x0: if row == start.1 { start.0 } else { left },
                x1: if row == end.1 { end.0 } else { right },
            })
            .collect()
    }

    /// Selected text in `buf`: rows joined by `\n`, trailing spaces trimmed.
    ///
    /// A wide glyph is emitted once; its padding cells are skipped.
    pub fn text(&self, buf: &Buffer) -> String {
        let area = buf.area;
        let lines: Vec<String> = self
            .spans()
            .into_iter()
            .filter(|span| span.row >= area.y && span.row < area.bottom())
            .map(|span| {
                let mut line = String::new();
                let mut pad = 0usize;
                // Walk from the row start so a wide glyph left of `x0` still
                // marks its padding cells.
                for x in area.x..=span.x1.min(area.right().saturating_sub(1)) {
                    let cell = &buf[(x, span.row)];
                    if pad > 0 {
                        pad -= 1;
                        continue;
                    }
                    // Same unicode width ratatui used to place the glyph.
                    pad = Span::raw(cell.symbol()).width().saturating_sub(1);
                    if x >= span.x0 && !cell.skip {
                        line.push_str(cell.symbol());
                    }
                }
                line.trim_end().to_string()
            })
            .collect();
        lines.join("\n")
    }

    /// Add [`Modifier::REVERSED`] to the selected cells in `buf`.
    pub fn highlight(&self, buf: &mut Buffer) {
        let area = buf.area;
        for span in self.spans() {
            if span.row < area.y || span.row >= area.bottom() {
                continue;
            }
            for x in span.x0..=span.x1.min(area.right().saturating_sub(1)) {
                if x < area.x {
                    continue;
                }
                let cell = &mut buf[(x, span.row)];
                cell.modifier.insert(Modifier::REVERSED);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn pane() -> Rect {
        Rect::new(2, 1, 10, 5)
    }

    fn span(row: u16, x0: u16, x1: u16) -> SelectionSpan {
        SelectionSpan { row, x0, x1 }
    }

    #[test]
    fn arm_rejects_press_outside_pane() {
        assert!(TextSelection::arm(pane(), 1, 2).is_none());
        assert!(TextSelection::arm(pane(), 12, 2).is_none());
        assert!(TextSelection::arm(pane(), 4, 0).is_none());
        assert!(TextSelection::arm(pane(), 4, 6).is_none());
        assert!(TextSelection::arm(pane(), 2, 1).is_some());
        assert!(TextSelection::arm(pane(), 11, 5).is_some());
    }

    #[test]
    fn plain_press_is_not_active() {
        let mut sel = TextSelection::arm(pane(), 4, 2).unwrap();
        assert!(!sel.is_active());
        assert!(sel.spans().is_empty());
        sel.extend_to(4, 2);
        assert!(!sel.is_active());
    }

    #[test]
    fn stream_shape_spans_first_middle_last_rows() {
        let mut sel = TextSelection::arm(pane(), 5, 2).unwrap();
        sel.extend_to(3, 4);
        assert_eq!(
            sel.spans(),
            vec![span(2, 5, 11), span(3, 2, 11), span(4, 2, 3)]
        );
    }

    #[test]
    fn single_row_spans_between_columns_either_direction() {
        let mut sel = TextSelection::arm(pane(), 8, 3).unwrap();
        sel.extend_to(4, 3);
        assert_eq!(sel.spans(), vec![span(3, 4, 8)]);
    }

    #[test]
    fn upward_drag_normalizes_to_same_stream() {
        let mut down = TextSelection::arm(pane(), 5, 2).unwrap();
        down.extend_to(3, 4);
        let mut up = TextSelection::arm(pane(), 3, 4).unwrap();
        up.extend_to(5, 2);
        assert_eq!(down.spans(), up.spans());
    }

    #[test]
    fn drag_past_pane_clamps_to_edges() {
        let mut sel = TextSelection::arm(pane(), 5, 2).unwrap();
        sel.extend_to(40, 30);
        assert_eq!(sel.head, (11, 5));
        sel.extend_to(0, 0);
        assert_eq!(sel.head, (2, 1));
        assert_eq!(sel.spans(), vec![span(1, 2, 11), span(2, 2, 5)]);
    }

    #[test]
    fn text_trims_trailing_spaces_and_joins_rows() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 14, 7));
        buf.set_string(1, 2, "│ab  cd    │", Style::default());
        buf.set_string(1, 3, "│efg       │", Style::default());
        let mut sel = TextSelection::arm(pane(), 2, 2).unwrap();
        sel.extend_to(4, 3);
        assert_eq!(sel.text(&buf), "ab  cd\nefg");
    }

    #[test]
    fn text_emits_wide_glyph_once() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 14, 7));
        buf.set_string(2, 2, "a日本b", Style::default());
        let mut sel = TextSelection::arm(pane(), 2, 2).unwrap();
        sel.extend_to(7, 2);
        assert_eq!(sel.text(&buf), "a日本b");
        // Start on the padding cell of 日: the glyph is not duplicated.
        let mut mid = TextSelection::arm(pane(), 4, 2).unwrap();
        mid.extend_to(7, 2);
        assert_eq!(mid.text(&buf), "本b");
    }

    #[test]
    fn highlight_reverses_selected_cells_only() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 14, 7));
        let mut sel = TextSelection::arm(pane(), 10, 2).unwrap();
        sel.extend_to(3, 3);
        sel.highlight(&mut buf);
        let reversed: Vec<(u16, u16)> = (0..7)
            .flat_map(|y| (0..14).map(move |x| (x, y)))
            .filter(|&(x, y)| buf[(x, y)].modifier.contains(Modifier::REVERSED))
            .collect();
        assert_eq!(reversed, vec![(10, 2), (11, 2), (2, 3), (3, 3)]);
    }
}
