//! Live xfce/VTE encodings for `gg` / `gt` / `gT`.
//!
//! PTY writes that only send raw `g` bytes stay green while a terminal
//! that speaks typeless CSI-u (`CSI 103 ; 1 u`) still no-ops `gg` and
//! fires bare `t` / `T`. These tests send the bytes that path emits.
//!
//! Every chord goes out in one write ([`send_gesture`]): its bytes arrive
//! back to back, in practice in one read, so its keys share one arrival
//! instant. Two rules are timing windows and stay in unit tests with
//! explicit instants: a second one-report `g` 40 ms after the first is a
//! new tap (`typeless_second_g_at_40ms_is_a_new_tap`), and a CSI-u
//! press-only `gg` runs through the loop's decode on arrival instants
//! (`press_only_gg_runs_the_loop_decode_on_arrival_instants`).

use std::time::Duration;

use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{
    crumb_row, csi_u_typeless, csi_u_typeless_shift, send_gesture, status_row, tree_cursor_on,
    GIT_WAIT, WAIT,
};

/// Twice the TUI protocol echo window (`G_CHORD_PROTOCOL_ECHO_MS`, 8 ms).
///
/// Waited after the completing `g` of `gg` has painted: that `g` arrived
/// before the paint, so the next `g` arrives at least this long after it
/// and is a new tap, not that tap's typeless key-up.
const PAST_ECHO_WINDOW: Duration = Duration::from_millis(16);

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

/// Typeless `gg` in one write: the first tap with its typeless key-up
/// report (VTE sends it in the same burst), then a one-report tap.
///
/// The key-up report is dropped as an echo, the second tap completes
/// `gg`, and that completing `g` is left waiting for its own key-up: the
/// state the next `gt` must get past.
fn typeless_gg(tui: &mut PtySession) {
    let g = csi_u_typeless('g');
    send_gesture(tui, &[&g, &g, &g]);
}

/// Typeless one-report `g` then `key` in one write.
fn typeless_g_chord(tui: &mut PtySession, key: &str) {
    send_gesture(tui, &[&csi_u_typeless('g'), key]);
}

/// Typeless CSI-u `gg` jumps, and one-report `g2` / `gt` / `gT` switch tabs.
///
/// Compare uses Diff vs default in new tab on `app` (alpha.txt /
/// beta.txt), not an empty "No committed changes" list. Immediate typeless
/// press+echo `gt` stays in leftover `pty_compare_gt_chords_and_gg`.
#[test]
fn pty_typeless_one_report_gg_and_gt() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    leave_workspace_root(&mut tui);

    typeless_gg(&mut tui);
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "app")
                && !tree_or_theme_fired(screen)
        },
        "typeless gg on the workspace tree jumps to the workspace root",
        WAIT,
    );

    open_default_and_main(&mut tui);
    tui.wait_pred(
        on_compare_local_main,
        "setup: active compare is vs local main",
        GIT_WAIT,
    );

    typeless_g_chord(&mut tui, &csi_u_typeless('2'));
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "typeless g2 activates Diff vs default in new tab (origin/main)",
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

    typeless_gg(&mut tui);
    tui.wait_pred(
        |screen| {
            on_compare_origin_main(screen)
                && tree_cursor_on(screen, "alpha.txt")
                && !tree_cursor_on(screen, "beta.txt")
                && !tree_or_theme_fired(screen)
        },
        "typeless gg on a multi-file compare list jumps to the first file",
        WAIT,
    );

    std::thread::sleep(PAST_ECHO_WINDOW);
    typeless_g_chord(&mut tui, &csi_u_typeless('t'));
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "typeless gt after completing g is NextTab, not ToggleTreeMode",
        WAIT,
    );

    typeless_g_chord(&mut tui, &csi_u_typeless_shift('t'));
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "typeless gT is PreviousTab and sets no theme",
        WAIT,
    );
}
