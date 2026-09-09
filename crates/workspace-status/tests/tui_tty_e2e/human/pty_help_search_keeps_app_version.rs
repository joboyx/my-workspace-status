use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// Package version sits on the help overlay lower-right, same idea as
/// `pty_help_overlay` / headless `assert_help_version`.
fn help_version_lower_right(screen: &str) -> bool {
    let version = workspace_status::APP_VERSION;
    let Some(line) = screen.lines().rev().find(|line| line.contains(version)) else {
        return false;
    };
    let Some(idx) = line.rfind(version) else {
        return false;
    };
    line[idx + version.len()..]
        .chars()
        .all(|c| c.is_whitespace() || matches!(c, '│' | '╯' | '╮' | '┘' | '┐' | '║' | '┤'))
}

fn help_overlay_idle(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("GIT")
        && screen.contains("VIEW")
        && screen.contains("/ search help")
        && !screen.contains("HELP  /")
        && !screen.contains("Esc clears search")
        && help_version_lower_right(screen)
}

fn help_searching_quit(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("GIT")
        && screen.contains("VIEW")
        && screen.contains("HELP  /quit")
        && screen.contains("Esc clears search")
        && screen.contains("stage scope")
        && !screen.contains("/ search help")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && help_version_lower_right(screen)
}

/// Help `/` search still paints the package version on the overlay footer.
///
/// Idle overlay version and the MOVE / GIT / VIEW keymap table stay on
/// `pty_help_overlay`. Help `/` vs pane search stays on
/// `pty_help_enter_does_not_arm_pane_search`. This claim is only that a
/// typed help query does not drop the lower-right version.
///
/// Fail if `?` or `/` is a no-op (`? help` chrome, idle `/ search help`),
/// or if the version vanishes while `HELP  /quit` is open.
#[test]
fn pty_help_search_keeps_app_version() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("? help")
                && screen.contains("README.md")
                && !screen.contains("MOVE")
                && !screen.contains("/ search help")
        },
        "idle chrome shows ? help and the overlay is closed",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        help_overlay_idle,
        "help overlay is open with idle / search help and the package version",
        WAIT,
    );
    assert!(
        !help_searching_quit(&tui.screen()),
        "help-search claim must be false before / quit:\n{}",
        tui.screen()
    );

    tui.key('/');
    tui.wait_pred(
        |screen| {
            screen.contains("HELP  /")
                && screen.contains("Esc clears search")
                && !screen.contains("/ search help")
                && help_version_lower_right(screen)
        },
        "help / opens overlay search; version stays (a no-op keeps / search help)",
        WAIT,
    );

    tui.keys("quit");
    tui.wait_pred(
        help_searching_quit,
        "typing quit keeps help search and the lower-right package version",
        WAIT,
    );
}
