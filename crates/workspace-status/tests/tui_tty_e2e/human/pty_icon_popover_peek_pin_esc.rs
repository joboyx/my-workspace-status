use crate::harness::{tree_row_containing, PtySession};
use crate::seed::{behind_workspace, daily_workspace, primary_merged_workspace};
use crate::support::{
    crumb_row, documented_space_reviewed, idle_dirty_readme_unreviewed, syncbox_row_behind,
    tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT,
};

/// SGR pointer motion with no button held (`3 | 32`, any-event tracking).
const SGR_POINTER_MOVE: u8 = 3 | 32;
/// Catalog meaning of the behind mark: the first line of its popover.
const BEHIND_MEANING: &str = "Upstream commits not pulled";
/// Field line of a one-commit behind mark.
const BEHIND_COUNT: &str = "1 commit to pull";
/// Footer of a pinned popover. A peek has none.
const PINNED_FOOTER: &str = "Esc close";

/// 0-based cell of the `v1` behind mark on the syncbox tree row.
fn behind_mark_cell(screen: &str) -> Option<(u16, u16)> {
    let row = tree_row_containing(screen, "syncbox")?;
    let line = screen.lines().nth(usize::from(row))?;
    let name = line.find("syncbox")?;
    let at = name + line[name..].find("v1")?;
    // ASCII glyph mode: one column per char.
    Some((line[..at].chars().count() as u16, row))
}

fn peek_shown(screen: &str) -> bool {
    screen.contains(BEHIND_MEANING)
        && screen.contains(BEHIND_COUNT)
        && screen.contains("Pull behind")
        && !screen.contains(PINNED_FOOTER)
}

fn pinned(screen: &str) -> bool {
    screen.contains(BEHIND_MEANING)
        && screen.contains(BEHIND_COUNT)
        && screen.contains("y copy line · Enter run · Esc close")
}

fn closed(screen: &str) -> bool {
    !screen.contains(BEHIND_MEANING) && !screen.contains(PINNED_FOOTER)
}

/// Hovering the tree behind mark peeks; a click pins; `j` / Enter run the
/// focused action; `gh` pins again; Esc closes.
///
/// Docs: an icon peek opens after the pointer rests on it and takes no
/// keys. A click on the icon selects its row and pins the popover: `j` / `k`
/// move, Enter runs the focused action, Esc closes. `gh` pins the popover of
/// every icon on the focused row.
///
/// Live PTY on a fetched, one-commit-behind `syncbox`: SGR motion onto
/// `v1` paints the peek (meaning, `1 commit to pull`, `Pull behind`, no
/// footer). A click pins it (footer). `j` moves to Fetch remotes and Enter
/// runs it: the popover closes and the breadcrumb says `Fetched 1 repo`.
/// `gh` pins it again; Esc closes it and the row stays behind.
#[test]
fn pty_icon_popover_peek_pin_esc() {
    let (_root, workspace) = behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(syncbox_row_behind, "syncbox row shows v1", WAIT);
    let (col, row) = behind_mark_cell(&tui.screen())
        .unwrap_or_else(|| panic!("behind mark cell:\n{}", tui.screen()));

    tui.sgr_mouse(SGR_POINTER_MOVE, col, row);
    tui.wait_pred(peek_shown, "pointer rest on v1 opens the peek", WAIT);

    tui.sgr_click(col, row);
    tui.wait_pred(pinned, "a click on v1 pins the popover", WAIT);
    // Move the pointer off: a pinned popover has no leave grace.
    tui.sgr_mouse(SGR_POINTER_MOVE, 2, row);
    tui.wait_ms(SETTLE_MS * 3);
    assert!(pinned(&tui.screen()), "pinned stays:\n{}", tui.screen());

    tui.key('j');
    tui.wait_pred(
        |screen| screen.contains("❯ Fetch remotes"),
        "j focuses Fetch remotes",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| closed(screen) && crumb_row(screen).contains("Fetched 1 repo"),
        "Enter closes the popover and runs fetch",
        GIT_WAIT,
    );

    tui.keys("gh");
    tui.wait_pred(pinned, "gh pins the focused row's popover", WAIT);
    tui.esc();
    tui.wait_pred(closed, "Esc closes the pinned popover", WAIT);
    tui.wait_pred(syncbox_row_behind, "the row is still behind", WAIT);
}

/// Nested primary checkout of the merged family seed.
const PRIMARY: &str = "feature/primary-merged";
/// Catalog meanings: the first line of each section.
const BRANCH_MEANING: &str = "Checked-out branch";
const MERGED_MEANING: &str = "HEAD is merged into the default branch";
const NO_UPSTREAM_MEANING: &str = "Branch has no upstream";

/// `gh` on the primary checkout: branch, merge mark, and sync sections.
fn checkout_sections(screen: &str) -> bool {
    screen.contains(BRANCH_MEANING)
        && screen.contains("feature branch")
        && screen.contains("Branch picker")
        && screen.contains(MERGED_MEANING)
        && screen.contains("Diff vs default in new tab")
        && screen.contains(NO_UPSTREAM_MEANING)
        && screen.contains(PINNED_FOOTER)
}

/// A pinned popover of the merge mark alone.
fn merge_only(screen: &str) -> bool {
    screen.contains(MERGED_MEANING)
        && screen.contains("vs main")
        && screen.contains(PINNED_FOOTER)
        && !screen.contains(BRANCH_MEANING)
        && !screen.contains(NO_UPSTREAM_MEANING)
}

fn no_popover(screen: &str) -> bool {
    !screen.contains(PINNED_FOOTER) && !screen.contains(MERGED_MEANING)
}

/// 0-based cell of the `M` merge mark after the primary branch name.
fn merge_mark_cell(screen: &str) -> Option<(u16, u16)> {
    let row = tree_row_containing(screen, PRIMARY)?;
    let line = screen.lines().nth(usize::from(row))?;
    let at = line.find(&format!("{PRIMARY} M"))? + PRIMARY.len() + 1;
    // ASCII glyph mode: one column per char.
    Some((line[..at].chars().count() as u16, row))
}

/// `gh` on a checkout row lists one section per icon; the merge mark is
/// its own icon.
///
/// Docs: every icon on a branch-level tree row has a popover. `gh` pins
/// one section per icon of the focused row. The merge mark after a branch
/// is its own icon: a click on it pins its popover alone.
///
/// Live PTY on the merged family seed: `j` focuses `& feature/primary-merged
/// M`. `gh` paints the branch section (meaning, `feature branch`, Branch
/// picker), the merge mark section (meaning, Diff vs default), and the
/// no-upstream section, with the pinned footer. Esc closes it. A click on
/// the `M` cell pins the merge mark popover alone (`vs main`). Esc closes.
#[test]
fn pty_icon_popover_gh_on_a_checkout_row_lists_branch_merge_and_sync() {
    let (_root, workspace) = primary_merged_workspace();
    let mut tui = PtySession::open_size(&workspace, 120, 40);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && tree_has(screen, &format!("{PRIMARY} M")),
        "first paint: family with the merged primary checkout",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, PRIMARY),
        "j focuses the primary checkout",
        WAIT,
    );
    tui.keys("gh");
    tui.wait_pred(
        checkout_sections,
        "gh lists the branch, merge mark, and sync sections",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(no_popover, "Esc closes the gh popover", WAIT);

    let (col, row) = merge_mark_cell(&tui.screen())
        .unwrap_or_else(|| panic!("merge mark cell:\n{}", tui.screen()));
    tui.sgr_click(col, row);
    tui.wait_pred(merge_only, "a click on M pins the merge mark popover", WAIT);
    tui.esc();
    tui.wait_pred(no_popover, "Esc closes the merge mark popover", WAIT);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, PRIMARY),
        "the primary checkout stays focused",
        WAIT,
    );
}

/// Catalog meanings of the file-row icons.
const VIEWED_MEANING: &str = "Marked reviewed (space)";
const FILE_TYPE_MEANING: &str = "File type by name or extension";
const MODIFIED_MEANING: &str = "Modified, not staged";

/// 0-based cell of the ASCII viewed eye `*` after `README.md`.
fn viewed_eye_cell(screen: &str) -> Option<(u16, u16)> {
    let row = tree_row_containing(screen, "README.md")?;
    let line = screen.lines().nth(usize::from(row))?;
    let name = line.find("README.md")?;
    let at = name + line[name..].find('*')?;
    // ASCII glyph mode: one column per char.
    Some((line[..at].chars().count() as u16, row))
}

/// A peek of the viewed eye alone: meaning and Mark reviewed, no footer.
fn eye_peek(screen: &str) -> bool {
    screen.contains(VIEWED_MEANING)
        && screen.contains("Mark reviewed")
        && !screen.contains(FILE_TYPE_MEANING)
        && !screen.contains(PINNED_FOOTER)
}

/// `gh` on the README row: devicon, viewed eye, and status letter
/// sections, pinned.
fn file_row_sections(screen: &str) -> bool {
    screen.contains(FILE_TYPE_MEANING)
        && screen.contains("readme.md file")
        && screen.contains("Open in editor")
        && screen.contains(VIEWED_MEANING)
        && screen.contains(MODIFIED_MEANING)
        && screen.contains("worktree")
        && screen.contains("Revert")
        && screen.contains(PINNED_FOOTER)
}

fn file_popover_closed(screen: &str) -> bool {
    !screen.contains(VIEWED_MEANING) && !screen.contains(PINNED_FOOTER)
}

/// File-row icons peek and pin like branch-row icons.
///
/// Docs: every icon on a file-level tree row (section, folder, devicon,
/// status letter, viewed eye, comment) has a popover. `gh` on a file row
/// pins one section per icon.
///
/// Live PTY on the daily seed: the cursor starts on the dirty README.
/// Space marks it reviewed (`*`). Pointer rest on `*` peeks the viewed
/// section alone. `gh` pins the devicon (`readme.md file`, Open in
/// editor), viewed, and modified (`worktree`, Revert) sections with the
/// footer. Esc closes it and the row stays reviewed.
#[test]
fn pty_icon_popover_gh_on_a_file_row_lists_devicon_badge_and_viewed() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size(&workspace, 120, 40);
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "first paint: cursor on the dirty README",
        GIT_WAIT,
    );
    tui.key(' ');
    tui.wait_pred(
        documented_space_reviewed,
        "Space marks README reviewed",
        WAIT,
    );
    let (col, row) = viewed_eye_cell(&tui.screen())
        .unwrap_or_else(|| panic!("viewed eye cell:\n{}", tui.screen()));

    tui.sgr_mouse(SGR_POINTER_MOVE, col, row);
    tui.wait_pred(eye_peek, "pointer rest on * peeks the viewed eye", WAIT);
    tui.sgr_mouse(SGR_POINTER_MOVE, 0, 0);
    tui.wait_pred(file_popover_closed, "leaving the eye closes the peek", WAIT);

    tui.keys("gh");
    tui.wait_pred(
        file_row_sections,
        "gh lists the devicon, viewed, and status letter sections",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(file_popover_closed, "Esc closes the file-row popover", WAIT);
    tui.wait_pred(documented_space_reviewed, "the row stays reviewed", WAIT);
}
