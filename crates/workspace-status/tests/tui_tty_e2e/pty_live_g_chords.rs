//! Live xfce/VTE encodings for `gg` / `gt` / `gT`.
//!
//! PTY writes that only send raw `g` bytes stay green while a terminal
//! that speaks typeless CSI-u (`CSI 103 ; 1 u`) still no-ops `gg` and
//! fires bare `t` / `T`. These tests send the bytes that path emits.
//! They do not inject a CSI-u Release or a second typeless report for
//! key-up.

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

fn leave_workspace_root(tui: &mut PtySession) {
    if tree_cursor_on(&tui.screen(), "workspace") {
        tui.key('j');
    }
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "workspace"),
        "cursor must leave the workspace root before gg",
        WAIT,
    );
}

/// One typeless CSI-u per tap: `gg` jumps, `gt` / `gT` switch tabs.
///
/// Compare uses Diff vs default on `app` (alpha.txt / beta.txt), not an
/// empty "No committed changes" list.
#[test]
fn pty_typeless_one_report_gg_and_gt() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    leave_workspace_root(&mut tui);

    tui.typeless_gg();
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "app")
                && !tree_or_theme_fired(screen)
        },
        "typeless one-report gg on the workspace tree jumps to the workspace root",
        WAIT,
    );

    open_default_and_main(&mut tui);
    tui.wait_pred(
        on_compare_local_main,
        "setup: active compare is vs local main",
        GIT_WAIT,
    );

    tui.csi_u_typeless('g');
    tui.csi_u_typeless('2');
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "typeless g2 activates Diff vs default (origin/main)",
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
        "j leaves the first compare file so a no-op gg cannot pass",
        WAIT,
    );

    tui.typeless_gg();
    tui.wait_pred(
        |screen| {
            on_compare_origin_main(screen)
                && tree_cursor_on(screen, "alpha.txt")
                && !tree_cursor_on(screen, "beta.txt")
                && !tree_or_theme_fired(screen)
        },
        "typeless one-report gg on a multi-file compare list jumps to the first file",
        WAIT,
    );

    tui.csi_u_typeless('g');
    tui.csi_u_typeless('t');
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "typeless one-report gt is NextTab, not ToggleTreeMode",
        WAIT,
    );

    tui.csi_u_typeless('g');
    tui.csi_u_typeless_shift('t');
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "typeless gT is PreviousTab, not CycleTheme",
        WAIT,
    );
}

/// Immediate typeless pair is still key-up. `t` must stay NextTab.
#[test]
fn pty_typeless_echo_gt_is_next_tab() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    open_default_and_main(&mut tui);
    tui.wait_pred(
        on_compare_local_main,
        "setup: active compare is vs local main",
        GIT_WAIT,
    );

    tui.csi_u_typeless('g');
    tui.csi_u_typeless('g');
    tui.csi_u_typeless('t');
    tui.csi_u_typeless('t');
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "typeless press+echo gt wraps to Workspace, not Flat paths",
        WAIT,
    );
}

/// Typed CSI-u press only (no Release) must still complete `gg`.
#[test]
fn pty_typed_press_only_gg_jumps_workspace() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    leave_workspace_root(&mut tui);

    tui.letter_press('g');
    tui.wait_ms(120);
    tui.letter_press('g');
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "app")
                && !tree_or_theme_fired(screen)
        },
        "typed CSI-u press-only gg jumps to the workspace root",
        WAIT,
    );
}
