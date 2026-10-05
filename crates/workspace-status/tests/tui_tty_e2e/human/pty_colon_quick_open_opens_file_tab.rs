use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

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

/// `:`, type `readme`, Enter opens `app/README.md` in a new read-only tab.
///
/// Fail if `:` paints the commands list, if no `❯ README.md` hit row shows,
/// or if Enter leaves the overlay up, keeps the Workspace tab active, or
/// paints a tab without the file body.
#[test]
fn pty_colon_quick_open_opens_file_tab() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.key(':');
    tui.wait_pred(
        |screen| files_open(screen) && !commands_open(screen),
        ": opens Quick Open on files (Go to file + Enter open), not the commands list",
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
            // The Workspace diff header also reads `app/README.md`; the
            // file tab paints it as the pane title on the top border.
            tab_strip.contains("README.md")
                && screen
                    .lines()
                    .any(|line| line.trim_start().starts_with("┌app/README.md"))
                && screen.contains("# app")
                && screen.contains("dirty")
                && !screen.contains("Go to file")
        },
        "Enter opens a README.md tab titled app/README.md with the file body; Quick Open closes",
        GIT_WAIT,
    );
}

/// Esc closes files mode; `:` then `>` switches to commands; Esc closes
/// commands mode. The tree cursor stays on README.md throughout.
#[test]
fn pty_colon_quick_open_esc_closes_and_gt_switches_to_commands() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.key(':');
    tui.wait_pred(files_open, ": opens Quick Open on files (Go to file)", WAIT);
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("Go to file") && tree_cursor_on(screen, "README.md"),
        "Esc closes files mode and leaves the tree cursor on README.md",
        WAIT,
    );

    tui.key(':');
    tui.wait_pred(
        |screen| files_open(screen) && !commands_open(screen),
        ": opens Quick Open on files (Go to file), not the commands list",
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
