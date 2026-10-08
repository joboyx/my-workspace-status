use crate::seed::compare_regions_workspace;
use crate::support::{open_compare_regions_diff, status_row, WAIT};

/// Header row of the `?` overlay with `middle` as the second column title.
fn help_header_has(screen: &str, middle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains("MOVE") && line.contains(middle) && line.contains("VIEW"))
}

/// Painted COMPARE column, whitespace-compacted so wrapped rows rejoin.
///
/// The column runs from its title icon (`{icon}  COMPARE`) to the VIEW
/// icon on the header row; help columns are not even.
fn compare_column(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(start) = lines
        .iter()
        .position(|line| line.contains("COMPARE") && line.contains("VIEW"))
    else {
        return String::new();
    };
    let header: Vec<char> = lines[start].chars().collect();
    let title_at = |title: &str| -> Option<usize> {
        let title: Vec<char> = title.chars().collect();
        header
            .windows(title.len())
            .position(|window| window == title.as_slice())
            .map(|at| at.saturating_sub(3))
    };
    let (Some(from), Some(to)) = (title_at("COMPARE"), title_at("VIEW")) else {
        return String::new();
    };
    let mut column = String::new();
    for line in &lines[start..] {
        if line.contains("/ search help") {
            break;
        }
        let chars: Vec<char> = line.chars().collect();
        column.extend(chars[from.min(chars.len())..to.min(chars.len())].iter());
        column.push(' ');
    }
    column.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The compare help: COMPARE replaces GIT, with the `x` condition and the
/// Workspace-only keys, and no Workspace git rows.
fn compare_help_open(screen: &str) -> bool {
    let column = compare_column(screen);
    help_header_has(screen, "COMPARE")
        && !help_header_has(screen, "GIT")
        && column.contains("x revert to merge base (only if head checked out, file clean)")
        && column.contains("Workspace tab only")
        && column.contains("[✗] close tab (Ctrl-w or :)")
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
