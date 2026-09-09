use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{crumb_row, status_row, tree_cursor_on, GIT_WAIT, WAIT};

fn open_default_and_main(tui: &mut PtySession) {
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.wait_contains("Diff vs default", WAIT);
    tui.enter();
    tui.wait_contains("app · vs origin/main", GIT_WAIT);
    tui.ctrl_letter('k');
    tui.keys("vs branch");
    tui.enter();
    tui.wait_contains("Compare", WAIT);
    tui.keys("main");
    tui.enter();
    tui.wait_contains("app · vs main", GIT_WAIT);
}

fn on_workspace(screen: &str) -> bool {
    screen.contains("# workspace") && !screen.contains("COMMITTED") && !screen.contains("...HEAD")
}

fn on_compare_origin_main(screen: &str) -> bool {
    screen.contains("COMMITTED")
        && screen.contains("origin/main...HEAD")
        && !screen.contains("# workspace")
}

fn on_compare_local_main(screen: &str) -> bool {
    screen.contains("COMMITTED")
        && screen.contains("main...HEAD")
        && !screen.contains("origin/main...HEAD")
        && !screen.contains("# workspace")
}

fn tree_or_theme_fired(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    crumb.contains("Flat paths")
        || status.contains(" flat")
        || crumb.contains("theme:")
        || screen.contains("theme: ")
}

fn raw_gg(tui: &mut PtySession) {
    tui.key('g');
    tui.key('g');
}

/// Two raw `g` bytes must `MoveToStart`. Raw `gt` / `gT` must switch tabs.
///
/// A no-op `gg` stays on the mid-list row. A wrong-key `gt` / `gT` fires
/// Flat paths or a theme cycle. This path is two Presses, not CSI-u.
#[test]
fn pty_raw_gg_jumps_and_gt_stays_tab_switch() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);

    if tree_cursor_on(&tui.screen(), "workspace") {
        tui.key('j');
        tui.wait_pred(
            |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "workspace"),
            "leave the workspace root before raw gg",
            WAIT,
        );
    } else {
        tui.wait_pred(
            |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "workspace"),
            "launch cursor is not the workspace root",
            WAIT,
        );
    }

    raw_gg(&mut tui);
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "app")
                && !tree_or_theme_fired(screen)
        },
        "raw gg on the workspace tree jumps to the workspace root (a no-op stays on app)",
        WAIT,
    );

    open_default_and_main(&mut tui);
    tui.wait_pred(
        on_compare_local_main,
        "setup: active compare is vs local main",
        GIT_WAIT,
    );

    tui.key('g');
    tui.key('2');
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "g2 activates vs origin/main",
        WAIT,
    );

    tui.wait_pred(
        |screen| tree_cursor_on(screen, "alpha.txt"),
        "compare vs origin/main starts on the first file",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            on_compare_origin_main(screen)
                && tree_cursor_on(screen, "beta.txt")
                && !tree_cursor_on(screen, "alpha.txt")
        },
        "j leaves the first compare file so a later no-op gg cannot pass",
        WAIT,
    );

    raw_gg(&mut tui);
    tui.wait_pred(
        |screen| {
            on_compare_origin_main(screen)
                && tree_cursor_on(screen, "alpha.txt")
                && !tree_cursor_on(screen, "beta.txt")
                && !tree_or_theme_fired(screen)
        },
        "raw gg on a compare list jumps to the first file (a no-op stays on beta.txt)",
        WAIT,
    );

    tui.key('g');
    tui.key('t');
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "raw gt from vs origin/main is NextTab, not ToggleTreeMode",
        WAIT,
    );

    tui.key('g');
    tui.key('T');
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "raw gT from vs main is PreviousTab, not CycleTheme",
        WAIT,
    );
}
