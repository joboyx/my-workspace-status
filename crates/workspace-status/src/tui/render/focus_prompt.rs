//! Paint of the tree focus prompt: a one-line query box like the graph
//! create-branch prompt.
//!
//! Rows: the `Focus tree` title, the `query:` line, the preview count of
//! the tree behind the box ([`FOCUS_PROMPT_ALL_REPOS`] when the draft has
//! no terms), and the key hint ([`FOCUS_PROMPT_HINT`]). The panes behind it
//! do not dim, so the live preview stays readable.

use super::*;

/// Title of the focus prompt.
pub(crate) const FOCUS_PROMPT_TITLE: &str = "Focus tree";
/// Key hint row of the focus prompt.
pub(crate) const FOCUS_PROMPT_HINT: &str = "Enter apply · Esc restore · empty clears";
/// Preview row while the draft has no terms: the tree shows every repo.
pub(crate) const FOCUS_PROMPT_ALL_REPOS: &str = "no focus · every repo shows";

/// Paint the focus prompt in `area` (the dialog box).
pub(super) fn draw_focus_prompt(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(prompt) = state.focus_prompt.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let accent = palette.branch_feature;
    let query = if prompt.query.is_empty() {
        "…"
    } else {
        prompt.query.as_str()
    };
    let preview = match &state.tree.chrome.focus_chip {
        Some(chip) => format!("{}/{} repos in focus", chip.focused, chip.total),
        None => FOCUS_PROMPT_ALL_REPOS.to_string(),
    };
    let lines = vec![
        Line::from(Span::styled(
            FOCUS_PROMPT_TITLE,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled("  query: ", Style::default().fg(palette.muted)),
            Span::styled(query.to_string(), Style::default().fg(palette.cursor)),
        ]),
        Line::from(Span::styled(
            format!("  {preview}"),
            Style::default().fg(palette.muted),
        )),
        Line::from(Span::styled(
            FOCUS_PROMPT_HINT,
            Style::default().fg(palette.muted),
        )),
    ];
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(accent, palette.panel))
            .wrap(Wrap { trim: false }),
        area,
    );
}
