use crate::seed::compare_regions_workspace;
use crate::support::{open_compare_regions_diff, status_row, WAIT};

/// Header row of the `?` overlay with `middle` as the second column title.
fn help_header_has(screen: &str, middle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains("MOVE") && line.contains(middle) && line.contains("VIEW"))
}

/// The compare help: COMPARE replaces GIT, with the `x` condition and the
/// Workspace-only keys, and no Workspace git rows.
fn compare_help_open(screen: &str) -> bool {
    help_header_has(screen, "COMPARE")
        && !help_header_has(screen, "GIT")
        && screen.contains("revert to merge base")
        && screen.contains("only if head checked out,")
        && screen.contains("file clean")
        && screen.contains("Workspace tab only")
        && screen.contains("close tab (or palette)")
        && screen.contains("/ search help")
        && !screen.contains("stage scope")
        && !screen.contains("fetch remotes")
}

/// The Workspace help as before: GIT column, no compare column.
fn workspace_help_open(screen: &str) -> bool {
    help_header_has(screen, "GIT")
        && !help_header_has(screen, "COMPARE")
        && screen.contains("stage scope")
        && screen.contains("fetch remotes")
        && !screen.contains("Workspace tab only")
        && !screen.contains("only if head checked out")
}

fn help_closed(screen: &str) -> bool {
    !screen.contains("MOVE") && !screen.contains("/ search help") && screen.contains("? help")
}

/// On a compare tab, `?` shows the COMPARE column in place of GIT and the
/// idle hint row offers only compare keys. The Workspace tab keeps GIT.
///
/// Fail if the compare help still lists stage / fetch, if the hint row
/// offers stage / unstage / fetch from the parked Workspace row, or if the
/// compare column leaks onto the Workspace tab.
#[test]
fn pty_compare_help() {
    let (_root, workspace) = compare_regions_workspace();
    let mut tui = open_compare_regions_diff(&workspace);

    // Clean file at the compare head: `x` reverts to the merge base.
    tui.wait_pred(
        |screen| {
            let hints = status_row(screen);
            hints.contains("revert to merge base")
                && hints.contains("full file")
                && !hints.contains("stage")
                && !hints.contains("fetch")
        },
        "compare hint row: x revert to merge base, no stage / unstage / fetch",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        compare_help_open,
        "? on a compare tab shows the COMPARE column in place of GIT",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(help_closed, "Esc closes the compare help", WAIT);

    tui.key('g');
    tui.key('1');
    tui.wait_pred(
        |screen| screen.contains("# workspace") && !screen.contains("COMMITTED"),
        "g1 activates the Workspace tab",
        WAIT,
    );
    tui.key('?');
    tui.wait_pred(
        workspace_help_open,
        "? on the Workspace tab shows GIT, not the COMPARE column",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(help_closed, "Esc closes the Workspace help", WAIT);
}
