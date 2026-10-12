use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git, git_stdout};
use crate::support::{
    right_of_split, right_pane, tree_cursor_on, tree_has, tree_inactive_selection_on,
    type_palette_filter, GIT_WAIT, WAIT,
};

/// Strip label of the `app` checkout's Explorer tab.
const EXPLORER_APP: &str = "Explorer · app";
/// Status copy when a git write is pressed on an Explorer tab.
const READ_ONLY: &str = "Explorer tab is read-only · switch to Workspace tab";
/// Empty files-mode Quick Open prompt: the caret, then the placeholder.
const FILES_PROMPT: &str = "▏type a file name…";
/// Committed, then modified: the changed file Quick Open opens.
const LONG: &str = "long.rs";
/// Clean file right under [`LONG`] in the tree.
const MID: &str = "mid.rs";
/// Clean file right under [`MID`] in the tree.
const NEXT: &str = "next.rs";
/// Lines of [`LONG`]; only line 1 changes.
const LONG_LINES: usize = 16;
/// Worktree side of the changed line 1 of [`LONG`].
const LONG_CHANGED: &str = "+pub fn line_01_changed() {}";
/// Unchanged line 14 of [`LONG`]: 13 lines from the hunk, so only a
/// whole-file diff paints it (the default context is 3 lines).
const LONG_FAR_LINE: &str = "pub fn line_14() {}";
/// Body of the clean [`MID`].
const MID_BODY: &str = "pub fn mid_body() {}";
/// Body of the clean [`NEXT`].
const NEXT_BODY: &str = "pub fn next_body() {}";
/// Screen row of the Explorer's inner file tab row (under the strip and
/// the pane title row).
// Assumes the harness's fixed baseline chrome: boxed panes (`BASELINE_USER_CONFIG`).
const INNER_TAB_ROW: u16 = 2;

/// Add to `app` of the daily seed: `src/long.rs` (16 lines, committed,
/// then line 1 changed on disk) and the clean, committed `src/mid.rs` and
/// `src/next.rs`.
fn seed_inner_tab_files(workspace: &Path) {
    let app = workspace.join("app");
    fs::create_dir_all(app.join("src")).unwrap();
    let body = |first: &str| {
        let mut text = format!("{first}\n");
        for n in 2..=LONG_LINES {
            text.push_str(&format!("pub fn line_{n:02}() {{}}\n"));
        }
        text
    };
    fs::write(app.join("src").join(LONG), body("pub fn line_01() {}")).unwrap();
    fs::write(app.join("src").join(MID), format!("{MID_BODY}\n")).unwrap();
    fs::write(app.join("src").join(NEXT), format!("{NEXT_BODY}\n")).unwrap();
    git(&app, &["add", "src"]);
    git(&app, &["commit", "-q", "-m", "add src"]);
    fs::write(
        app.join("src").join(LONG),
        body("pub fn line_01_changed() {}"),
    )
    .unwrap();
}

fn tab_strip(screen: &str) -> &str {
    screen.lines().next().unwrap_or_default()
}

/// Pane title row: `┌<left title>──┐┌<right title>──┐`.
fn title_row(screen: &str) -> &str {
    screen.lines().nth(1).unwrap_or_default()
}

/// The Explorer's inner file tab row: the right pane's first row.
fn inner_tab_row(screen: &str) -> String {
    right_of_split(
        screen
            .lines()
            .nth(usize::from(INNER_TAB_ROW))
            .unwrap_or_default(),
    )
}

/// `Explorer · app` is active with the tree focused on `name` (bold `▌`
/// bar, not the `▏` of an unfocused pane), and no file names a top-level
/// tab.
fn explorer_tree_on(screen: &str, name: &str) -> bool {
    let strip = tab_strip(screen);
    strip.contains(EXPLORER_APP)
        && [LONG, MID, NEXT].iter().all(|file| !strip.contains(file))
        && title_row(screen).starts_with("┌app─")
        && tree_cursor_on(screen, name)
        && !tree_inactive_selection_on(screen, name)
}

/// Right pane is the whole-file diff of `src/long.rs`: the changed line
/// and a line far from the hunk.
fn long_full_diff(screen: &str) -> bool {
    let right = right_pane(screen);
    title_row(screen).contains("┐┌diff")
        && right.contains("app/src/long.rs")
        && right.contains(LONG_CHANGED)
        && right.contains(LONG_FAR_LINE)
}

/// Right pane is the read-only body of the clean `src/<name>`.
fn clean_body(screen: &str, name: &str, body: &str) -> bool {
    let right = right_pane(screen);
    title_row(screen).contains(name)
        && !title_row(screen).contains("┐┌diff")
        && right.contains(body)
        && !right.contains("@@")
}

/// Whether the inner tab label `name` paints italic (the preview tab).
/// `None` while the label is not on the inner tab row.
fn label_italic(tui: &PtySession, name: &str) -> Option<bool> {
    let (cols, _) = tui.grid_size();
    let row = tui.grid_row_text(INNER_TAB_ROW, 0..cols);
    let split = row.find("││")?;
    let at = row[split..].find(name)? + split;
    let col = u16::try_from(row[..at].chars().count()).ok()?;
    let cells: Option<Vec<bool>> = (col..col + name.chars().count() as u16)
        .map(|c| tui.cell_paint(INNER_TAB_ROW, c).map(|cell| cell.italic))
        .collect();
    let cells = cells?;
    // A label paints as one span: every cell italic, or none.
    if cells.iter().all(|&italic| italic) {
        Some(true)
    } else if cells.iter().all(|&italic| !italic) {
        Some(false)
    } else {
        None
    }
}

/// Open `app/src/long.rs` from the Workspace tab with Quick Open (`F`,
/// `long`, Enter).
fn quick_open_long(tui: &mut PtySession) {
    tui.shift_letter('F');
    tui.wait_pred(
        |screen| screen.contains("Go to file") && screen.contains(FILES_PROMPT),
        "F opens Quick Open on files with an empty prompt",
        WAIT,
    );
    type_palette_filter(tui, "long");
    tui.wait_pred(
        |screen| {
            screen
                .lines()
                .any(|line| line.contains('❯') && line.contains(LONG))
        },
        "`long` lists src/long.rs under the ❯ cursor",
        GIT_WAIT,
    );
    tui.enter();
}

/// Quick Open pins a changed file in its checkout's Explorer tab (no
/// top-level file tab, tree focused on the file, whole-file diff); `j`
/// onto clean files shows them in one preview tab that the next `j`
/// replaces; Enter pins it; `[` goes back to the first file tab and
/// selects it in the tree; `s` still refuses with the read-only copy.
///
/// Fail if a file opens a top-level tab, the diff shows only the hunk,
/// a tree move adds a second preview tab or keeps the replaced one, a pin
/// leaves the label italic, `[` leaves the tree on another row, or `s`
/// stages anything.
#[test]
fn pty_explorer_inner_file_tabs_preview_pin_and_switch() {
    let (_root, workspace) = daily_workspace();
    seed_inner_tab_files(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            tree_has(screen, LONG) && screen.contains("UNSTAGED") && !screen.contains("Go to file")
        },
        "first paint: Workspace tree lists src/long.rs with a diff on the right",
        GIT_WAIT,
    );

    quick_open_long(&mut tui);
    tui.wait_pred(
        |screen| {
            explorer_tree_on(screen, LONG)
                && long_full_diff(screen)
                && inner_tab_row(screen).contains(LONG)
                && !screen.contains("Go to file")
        },
        "Enter opens long.rs in Explorer · app: tree focused on it, whole-file diff, one file tab",
        GIT_WAIT,
    );
    tui.wait_pred(
        |_| label_italic(&tui, LONG) == Some(false),
        "the Quick Open file tab is pinned (not italic)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| {
            let tabs = inner_tab_row(screen);
            explorer_tree_on(screen, MID)
                && clean_body(screen, MID, MID_BODY)
                && tabs.contains(LONG)
                && tabs.contains(MID)
                && label_italic(&tui, MID) == Some(true)
                && label_italic(&tui, LONG) == Some(false)
        },
        "`j` onto the clean mid.rs shows it in an italic preview tab beside the pinned long.rs",
        GIT_WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| {
            let tabs = inner_tab_row(screen);
            explorer_tree_on(screen, NEXT)
                && clean_body(screen, NEXT, NEXT_BODY)
                && tabs.contains(LONG)
                && tabs.contains(NEXT)
                && !tabs.contains(MID)
                && label_italic(&tui, NEXT) == Some(true)
                && label_italic(&tui, LONG) == Some(false)
        },
        "`j` onto next.rs replaces the mid.rs preview tab (one preview, one pinned)",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| {
            explorer_tree_on(screen, NEXT)
                && clean_body(screen, NEXT, NEXT_BODY)
                && label_italic(&tui, NEXT) == Some(false)
                && label_italic(&tui, LONG) == Some(false)
        },
        "Enter pins next.rs (no italic label) and keeps the tree focused",
        WAIT,
    );

    tui.key('[');
    tui.wait_pred(
        |screen| {
            let tabs = inner_tab_row(screen);
            explorer_tree_on(screen, LONG)
                && long_full_diff(screen)
                && tabs.contains(LONG)
                && tabs.contains(NEXT)
        },
        "`[` shows the first file tab, long.rs, and selects it in the tree",
        GIT_WAIT,
    );

    tui.key('s');
    tui.wait_pred(
        |screen| explorer_tree_on(screen, LONG) && screen.contains(READ_ONLY),
        "`s` on the Explorer tab shows the read-only refusal",
        WAIT,
    );
    let staged = git_stdout(&workspace.join("app"), &["diff", "--cached", "--name-only"]);
    assert!(
        staged.is_empty(),
        "`s` on the Explorer tab must stage nothing; staged: {staged:?}"
    );
}
