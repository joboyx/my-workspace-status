use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{right_pane, tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Strip label of the `app` checkout's Explorer tab.
const EXPLORER_APP: &str = "Explorer · app";

/// Same gap as `pty_workspace_command_palette`. A same-letter `h`/`j`/`k`/`l`
/// burst is dropped by `discard_held_nav_backlog` after the first press.
const QUERY_NAV_LETTER_GAP_MS: u64 = 50;

fn files_open(screen: &str) -> bool {
    screen.contains("Go to file") && screen.contains("Enter open")
}

fn commands_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

fn first_paint(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && !screen.contains("Go to file")
        && !commands_open(screen)
}

fn wait_first_paint(tui: &PtySession) {
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        first_paint,
        "first paint: tree cursor on README.md; Quick Open is closed",
        WAIT,
    );
}

/// Type a Quick Open query at human key gaps for nav letters.
fn type_query(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(QUERY_NAV_LETTER_GAP_MS);
        }
    }
}

/// Quick Open hit row: the `❯` cursor chip in front of `needle`. The tree
/// cursor is `▌`, so a tree row cannot pass.
fn hit_row_on(screen: &str, needle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains('❯') && line.contains(needle))
}

/// `F` (CSI-u Shift+F), type `readme`, Enter opens `app/README.md` in the
/// `Explorer · app` tab as a pinned inner file tab: the tree selects the
/// file and keeps focus, and the right pane shows its whole-file diff (the
/// seed leaves it modified).
///
/// Fail if `F` paints the commands list, if no `❯ README.md` hit row shows,
/// or if Enter leaves the overlay up, keeps the Workspace tab active, adds
/// a top-level README.md tab, or paints no diff of the file.
#[test]
fn pty_shift_f_quick_open_opens_file_in_explorer() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.shift_letter('F');
    tui.wait_pred(
        |screen| files_open(screen) && !commands_open(screen),
        "F opens Quick Open on files (Go to file + Enter open), not the commands list",
        WAIT,
    );
    type_query(&mut tui, "readme");
    tui.wait_pred(
        |screen| files_open(screen) && screen.contains("readme") && hit_row_on(screen, "README.md"),
        "`readme` lists a README.md hit under the ❯ cursor",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            let tab_strip = screen.lines().next().unwrap_or_default();
            let title_row = screen.lines().nth(1).unwrap_or_default();
            let right = right_pane(screen);
            tab_strip.contains(EXPLORER_APP)
                && !tab_strip.contains("README.md")
                && title_row.starts_with("┌app─")
                && title_row.contains("┐┌diff")
                && tree_cursor_on(screen, "README.md")
                && right.contains("README.md")
                && right.contains("@@")
                && right.contains("dirty")
                && !screen.contains("Go to file")
        },
        "Enter opens README.md in Explorer · app: tree cursor on it, its diff on the right; Quick Open closes",
        GIT_WAIT,
    );
}

/// Ctrl-p opens files mode and Esc closes it; Ctrl-p then `>` as the first
/// char switches to commands; Esc closes commands mode. The tree cursor
/// stays on README.md throughout.
#[test]
fn pty_ctrl_p_quick_open_esc_closes_and_gt_switches_to_commands() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.ctrl_letter('p');
    tui.wait_pred(
        files_open,
        "Ctrl-p opens Quick Open on files (Go to file)",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("Go to file") && tree_cursor_on(screen, "README.md"),
        "Esc closes files mode and leaves the tree cursor on README.md",
        WAIT,
    );

    tui.ctrl_letter('p');
    tui.wait_pred(
        |screen| files_open(screen) && !commands_open(screen),
        "Ctrl-p opens Quick Open on files (Go to file), not the commands list",
        WAIT,
    );
    tui.key('>');
    tui.wait_pred(
        |screen| commands_open(screen) && !screen.contains("Go to file"),
        "> as the first char switches Quick Open to commands (Enter run)",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("Go to file")
                && !commands_open(screen)
                && tree_cursor_on(screen, "README.md")
        },
        "Esc closes commands mode and leaves the tree cursor on README.md",
        WAIT,
    );
}
