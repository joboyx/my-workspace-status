//! Scrolling message viewport shared by every commit-message footer.
//!
//! The graph selection footer ([`crate::GraphWidget`]) and the app's
//! commit-files footer both paint their message rows here, so a long
//! message scrolls, shows its scrollbar, and ends with the same
//! more-below hint on every surface.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget};

/// Max scroll of `message_lines` message lines shown in `rows` rows.
///
/// `0` when every line fits. A footer with a pinned meta row passes its
/// message rows only; [`crate::footer_message_scroll_max`] does that for
/// the graph selection footer.
pub(crate) fn footer_message_rows_scroll_max(message_lines: usize, rows: u16) -> usize {
    message_lines.saturating_sub(usize::from(rows))
}

/// What [`paint_footer_message`] painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FooterMessagePaint {
    /// Max scroll of the painted viewport: the message lines less its rows,
    /// `0` when every line fits. A caller records it so the wheel clamps
    /// to what was painted.
    pub scroll_max: usize,
    /// Cells of the more-below hint (`<arrow><K>`), its leading blank left
    /// out. `None` when no line is hidden below or the hint does not fit.
    pub more_below: Option<Rect>,
}

/// Paint `message_lines` message lines into `area` (the message rows only),
/// scrolled by `offset`.
///
/// `paint_line(buf, index, y)` paints message line `index` on row `y`; the
/// caller owns the line text and style. The offset clamps to
/// `0..=`[`FooterMessagePaint::scroll_max`]. A message taller than `area` also
/// gets a 1-column vertical scrollbar in the rightmost column, and while
/// lines are hidden below, the last row ends with ` <arrow><K>` (K hidden
/// lines) in `hint_fg`, left of the scrollbar. The hint is skipped when it
/// is wider than the area less the scrollbar column. A message that fits
/// gets neither.
pub fn paint_footer_message(
    buf: &mut Buffer,
    area: Rect,
    message_lines: usize,
    offset: usize,
    arrow: &str,
    hint_fg: Color,
    mut paint_line: impl FnMut(&mut Buffer, usize, u16),
) -> FooterMessagePaint {
    let rows = usize::from(area.height);
    let scroll_max = footer_message_rows_scroll_max(message_lines, area.height);
    let offset = offset.min(scroll_max);
    let mut painted = FooterMessagePaint {
        scroll_max,
        more_below: None,
    };
    for (i, index) in (offset..message_lines).take(rows).enumerate() {
        paint_line(buf, index, area.y.saturating_add(i as u16));
    }
    if message_lines <= rows || rows == 0 {
        return painted;
    }
    let bar_x = area.x.saturating_add(area.width.saturating_sub(1));
    let mut sb_state = ScrollbarState::new(scroll_max + 1)
        .position(offset)
        .viewport_content_length(rows);
    StatefulWidget::render(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        Rect {
            x: bar_x,
            y: area.y,
            width: 1,
            height: area.height,
        },
        buf,
        &mut sb_state,
    );
    let hidden = message_lines.saturating_sub(offset + rows);
    if hidden == 0 {
        return painted;
    }
    // One leading blank keeps the hint apart from the message text.
    let hint = format!(" {arrow}{hidden}");
    let hint_w = hint.chars().count() as u16;
    if hint_w > area.width.saturating_sub(1) {
        return painted;
    }
    let hint_x = bar_x.saturating_sub(hint_w);
    let hint_y = area.y.saturating_add(area.height - 1);
    Line::from(Span::styled(hint, Style::default().fg(hint_fg)))
        .render(Rect::new(hint_x, hint_y, hint_w, 1), buf);
    painted.more_below = Some(Rect::new(
        hint_x.saturating_add(1),
        hint_y,
        hint_w.saturating_sub(1),
        1,
    ));
    painted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(
        width: u16,
        rows: u16,
        lines: &[String],
        offset: usize,
    ) -> (Buffer, FooterMessagePaint) {
        let area = Rect::new(0, 0, width, rows);
        let mut buf = Buffer::empty(area);
        let painted = paint_footer_message(
            &mut buf,
            area,
            lines.len(),
            offset,
            "v",
            Color::Red,
            |buf, i, y| {
                buf.set_stringn(0, y, &lines[i], usize::from(width), Style::default());
            },
        );
        (buf, painted)
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line{i:02}")).collect()
    }

    #[test]
    fn rows_scroll_max_is_zero_when_the_message_fits() {
        assert_eq!(footer_message_rows_scroll_max(0, 0), 0);
        assert_eq!(footer_message_rows_scroll_max(3, 3), 0);
        assert_eq!(footer_message_rows_scroll_max(10, 4), 6);
        assert_eq!(footer_message_rows_scroll_max(2, 0), 2);
    }

    #[test]
    fn a_fitting_message_has_no_scrollbar_and_no_hint() {
        let (buf, painted) = paint(20, 4, &lines(3), 5);
        assert_eq!(painted.scroll_max, 0);
        assert_eq!(painted.more_below, None);
        assert_eq!(
            row(&buf, 0).trim_end(),
            "line00",
            "offset 5 clamps to 0: the first line paints first"
        );
        assert_eq!(row(&buf, 3).trim_end(), "", "short message pads");
        for y in 0..4 {
            assert_eq!(buf[(19, y)].symbol(), " ", "no scrollbar at y={y}");
        }
    }

    #[test]
    fn a_long_message_gets_scrollbar_and_hint_until_its_tail_shows() {
        let all = lines(10);
        let (buf, painted) = paint(20, 4, &all, 0);
        assert_eq!(painted.scroll_max, 6);
        assert_eq!(
            row(&buf, 0).trim_end_matches(['█', '║']).trim_end(),
            "line00"
        );
        assert!(row(&buf, 3).starts_with("line03"), "{}", row(&buf, 3));
        assert!(row(&buf, 3).contains(" v6"), "6 hidden: {}", row(&buf, 3));
        let hint = painted.more_below.expect("hint");
        assert_eq!((hint.x, hint.y, hint.width), (17, 3, 2), "left of the bar");
        assert_eq!(buf[(17, 3)].fg, Color::Red);
        assert!(
            (0..4).all(|y| matches!(buf[(19, y)].symbol(), "█" | "║")),
            "1-column scrollbar"
        );

        let (buf, painted) = paint(20, 4, &all, 99);
        assert_eq!(painted.more_below, None, "no hint at the tail");
        // Offset 99 clamps to the max (6): lines 06..09 paint.
        assert!(row(&buf, 0).starts_with("line06"), "{}", row(&buf, 0));
        assert!(row(&buf, 3).starts_with("line09"), "{}", row(&buf, 3));
        assert!(matches!(buf[(19, 3)].symbol(), "█" | "║"));
    }

    #[test]
    fn the_hint_is_skipped_when_it_does_not_fit() {
        let (_, painted) = paint(3, 2, &lines(20), 0);
        assert_eq!(painted.scroll_max, 18);
        assert_eq!(painted.more_below, None, "` v18` is wider than 2 cells");
    }
}
