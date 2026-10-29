use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{
    crumb_row, csi_u_shift_tap, csi_u_tap, csi_u_typeless, send_gesture, status_row,
    tree_cursor_on, GIT_WAIT, WAIT,
};

/// Typeless CSI-u tap of `key`: the press report and the key-up report
/// that VTE sends as a second identical report in the same burst.
///
/// Terminals that honor `REPORT_ALL_KEYS_AS_ESCAPE_CODES` but omit
/// `REPORT_EVENT_TYPES` send `CSI code ; 1 u` for both press and release.
/// The second report is another Press, not Release. That is the live
/// xfce/VTE hole: `g` then typeless `g` completes `gg`, so `t` / `T` stay
/// bare.
fn typeless_tap(key: char) -> String {
    csi_u_typeless(key).repeat(2)
}

fn open_default_and_main(tui: &mut PtySession) {
    tui.search("app");
    tui.key(':');
    tui.keys("vs default");
    tui.wait_contains("Diff vs default in new tab", WAIT);
    tui.enter();
    tui.wait_contains("app ↔ origin/main", GIT_WAIT);
    tui.key(':');
    tui.keys("vs branch");
    tui.enter();
    tui.wait_contains("Compare", WAIT);
    tui.keys("main");
    tui.enter();
    tui.wait_contains("app ↔ main", GIT_WAIT);
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

/// ToggleTreeMode toast / pill, or a `theme:` toast (no key sets a theme).
fn tree_or_theme_fired(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    crumb.contains("Flat paths")
        || status.contains(" flat")
        || crumb.contains("theme:")
        || screen.contains("theme: ")
}

/// On a compare tab, typeless CSI-u `gt` / `gT` must change the active tab.
///
/// A strip label or `COMMITTED` text from another tab is not enough.
/// `ToggleTreeMode` (`Flat paths` / ` flat`) or a theme toast
/// (`theme:`) must fail. Start on the last compare (user POV), not after a
/// raw-`g` jump that hid the VTE release-as-press hole.
///
/// Each chord goes out in one write ([`send_gesture`]), so its keys share
/// one arrival instant: the chord window and the typeless echo window
/// cannot split it on a loaded machine. The window lengths themselves are
/// pinned by the `tui::keys` and `tui::state` unit tests.
#[test]
fn pty_compare_gt_chords_and_gg() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    open_default_and_main(&mut tui);
    tui.wait_pred(
        on_compare_local_main,
        "setup: active compare is vs local main (not only a strip label)",
        GIT_WAIT,
    );

    send_gesture(&mut tui, &[&typeless_tap('g'), &typeless_tap('t')]);
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "typeless CSI-u gt from the last compare is NextTab (wrap to Workspace), not ToggleTreeMode",
        WAIT,
    );

    send_gesture(&mut tui, &[&typeless_tap('g'), &csi_u_shift_tap('T')]);
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "gT from Workspace is PreviousTab back to vs main and sets no theme",
        WAIT,
    );

    send_gesture(&mut tui, &[&csi_u_tap('g'), &csi_u_tap('1')]);
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "g1 activates Workspace",
        WAIT,
    );

    send_gesture(&mut tui, &[&csi_u_tap('g'), &csi_u_tap('2')]);
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "g2 activates vs origin/main",
        WAIT,
    );

    send_gesture(&mut tui, &[&csi_u_tap('g'), &csi_u_tap('3')]);
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "g3 activates vs main",
        WAIT,
    );

    send_gesture(&mut tui, &[&csi_u_tap('g'), &csi_u_tap('9')]);
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "g9 missing index is a no-op (stay on vs main)",
        WAIT,
    );

    send_gesture(&mut tui, &["g", "t"]);
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "raw-byte gt from vs main is NextTab",
        WAIT,
    );

    tui.search("app");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "workspace"),
        "search lands on app, off the workspace root, so a no-op gg cannot pass",
        WAIT,
    );
    send_gesture(&mut tui, &[&csi_u_tap('g'), &csi_u_tap('g')]);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "workspace") && !tree_cursor_on(screen, "app"),
        "gg still jumps the workspace tree",
        WAIT,
    );
}
