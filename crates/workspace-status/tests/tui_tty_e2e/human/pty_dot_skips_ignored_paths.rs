use crate::harness::PtySession;
use crate::seed::hidden_and_ignored_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, prove_keys_handled, tree_has, tree_line_containing,
    WAIT,
};

/// Launch: `notes` (hiddenPaths) and `tmp/scratch` (ignoredPaths) both off.
fn launch_without_scratch(screen: &str) -> bool {
    documented_launch_first_paint(screen) && !screen.contains("scratch")
}

/// After `.`: hidden `notes` is back, ignored `tmp/scratch` is not.
fn dot_shows_notes_not_scratch(screen: &str) -> bool {
    let heading = tree_line_containing(screen, "# workspace").unwrap_or_default();
    tree_has(screen, "notes")
        && heading.contains("2 changed · all current")
        && !heading.contains("3 changed")
        && crumb_row(screen).contains("showing hidden repos")
        && !screen.contains("scratch")
        && !tree_has(screen, "tmp")
}

/// `.` reveals a `hiddenPaths` repo but never an `ignoredPaths` repo.
///
/// Seed: daily workspace plus dirty `tmp/scratch`; config
/// `hiddenPaths: ["notes"]`, `ignoredPaths: ["tmp"]`. First paint shows
/// one changed repo. `.` adds `notes` (two changed). A leaked
/// `tmp/scratch` would paint its row and make the heading three changed.
#[test]
fn pty_dot_skips_ignored_paths() {
    let (_root, workspace) = hidden_and_ignored_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        launch_without_scratch,
        "first paint: notes hidden, tmp/scratch never discovered",
        WAIT,
    );

    tui.key('.');
    tui.wait_pred(
        dot_shows_notes_not_scratch,
        "`.` shows hidden notes; ignored tmp/scratch stays out",
        WAIT,
    );
    prove_keys_handled(&mut tui);
    tui.wait_pred(
        dot_shows_notes_not_scratch,
        "shown notes hold and tmp/scratch stays out after settle",
        WAIT,
    );
}
