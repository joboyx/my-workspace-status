use crate::common::hscroll::{TREE_HSCROLL_DIR, TREE_HSCROLL_FILE, TREE_HSCROLL_TAIL};
use crate::harness::PtySession;
use crate::seed::{daily_workspace, seed_long_path_file};
use crate::support::{right_pane, tree_cursor_on, GIT_WAIT, SETTLE_MS, WAIT};

/// Body line of the seeded untracked file.
const BODY: &str = "export const pan";

/// Launch grid. The right pane is far narrower than the header path.
const COLS: u16 = 80;
const ROWS: u16 = 24;

/// Short grid after resize. Same width, so the header still wraps.
const SHORT_ROWS: u16 = 12;

fn full_path() -> String {
    format!("{TREE_HSCROLL_DIR}/{TREE_HSCROLL_FILE}")
}

/// Right-pane rows with padding and the right border trimmed.
fn right_rows(screen: &str) -> Vec<String> {
    right_pane(screen)
        .lines()
        .map(|line| {
            line.trim()
                .trim_end_matches(['│', '║', '┃'])
                .trim()
                .to_string()
        })
        .collect()
}

/// Right-pane rows above the first diff body row, joined without gaps.
///
/// Works whether the header wraps mid-word or at `/`. `None` when the
/// body row is not on screen.
fn header_text(screen: &str) -> Option<String> {
    let rows = right_rows(screen);
    let body_at = rows.iter().position(|row| row.contains(BODY))?;
    Some(rows[..body_at].concat())
}

/// The file diff is loaded: header starts with the path prefix and the
/// body line is painted below it. Right-pane cells only.
fn long_path_diff_loaded(screen: &str) -> bool {
    let rows = right_rows(screen);
    tree_cursor_on(screen, "very-long")
        && rows
            .first()
            .is_some_and(|row| row.starts_with(&format!("{TREE_HSCROLL_DIR}/")))
        && header_text(screen).is_some()
}

/// Body line still painted below the header on a short grid.
fn body_visible_below_header(screen: &str) -> bool {
    let rows = right_rows(screen);
    rows.first()
        .is_some_and(|row| row.starts_with(&format!("{TREE_HSCROLL_DIR}/")))
        && rows.iter().skip(1).any(|row| row.contains(BODY))
}

/// Depth 0 file-diff header shows the full path of a long file.
///
/// The right-pane header wraps the repo-relative path over several rows
/// when it is wider than the pane, so the tail `TAIL99.ts` is readable
/// and the diff body still paints below. A short grid keeps the body
/// row visible: the header must not take the whole pane.
///
/// Live PTY at 80×24 so the path is wider than the right pane. Oracle
/// reads right-pane cells only; a search chip, the status row, or the
/// left tree that contains `TAIL99` does not count. A one-row clipped
/// header, or a wrap that pushes the body off the pane, is red.
#[test]
fn pty_diff_path_header_wraps_long_path() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open_size(&workspace, COLS, ROWS);
    tui.wait_contains("README.md", WAIT);

    tui.search("very-long");
    tui.wait_pred(
        long_path_diff_loaded,
        "search loads the long-path file diff; header starts with the path prefix",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let screen = tui.screen();
    let path = full_path();
    let header = header_text(&screen).unwrap_or_default();
    assert!(
        header.contains(&path) && right_pane(&screen).contains(TREE_HSCROLL_TAIL),
        "right-pane header must show the full path `{path}` (wrapped rows joined: `{header}`)\n\
         right pane:\n{}\n\nscreen:\n{screen}",
        right_pane(&screen),
    );
    assert!(
        right_rows(&screen)
            .iter()
            .skip(1)
            .any(|row| row.contains(BODY)),
        "diff body `{BODY}` must paint below the header:\n{screen}"
    );

    tui.resize(COLS, SHORT_ROWS);
    tui.wait_pred(
        |screen| screen.lines().count() <= SHORT_ROWS as usize && body_visible_below_header(screen),
        "short grid keeps the diff body below the wrapped header",
        WAIT,
    );
}
