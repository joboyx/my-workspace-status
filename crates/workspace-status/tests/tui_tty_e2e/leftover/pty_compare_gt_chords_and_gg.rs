use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{crumb_row, status_row, tree_cursor_on, GIT_WAIT, WAIT};

/// CSI-u press+release (`CSI code ; 1 : 1 u` / `: 3 u`).
fn csi_u_letter(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

/// CSI-u letter with no event type (`CSI code ; 1 u`).
///
/// Terminals that honor `REPORT_ALL_KEYS_AS_ESCAPE_CODES` but omit
/// `REPORT_EVENT_TYPES` send this for both press and release. The second
/// sequence is another Press, not Release. That is the live xfce/VTE hole:
/// `g` then typeless `g` completes `gg`, so `t` / `T` stay bare.
fn csi_u_typeless(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.send_bytes(format!("\x1b[{codepoint};1u").as_bytes());
}

fn csi_u_typeless_press_and_release(tui: &mut PtySession, letter: char) {
    csi_u_typeless(tui, letter);
    csi_u_typeless(tui, letter);
}

fn csi_u_gg(tui: &mut PtySession) {
    csi_u_letter(tui, 'g');
    tui.wait_ms(50);
    csi_u_letter(tui, 'g');
}

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

/// ToggleTreeMode toast / pill, or CycleTheme toast.
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
/// Hunt leftover: a strip label or `COMMITTED` leftover from another tab is
/// not enough. `ToggleTreeMode` (`Flat paths` / ` flat`) or `CycleTheme`
/// (`theme:`) must fail. Start on the last compare (user POV), not after a
/// raw-`g` jump that hid the VTE release-as-press hole.
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

    csi_u_typeless_press_and_release(&mut tui, 'g');
    csi_u_typeless_press_and_release(&mut tui, 't');
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "typeless CSI-u gt from the last compare is NextTab (wrap to Workspace), not ToggleTreeMode",
        WAIT,
    );

    csi_u_typeless_press_and_release(&mut tui, 'g');
    tui.shift_letter('T');
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "gT from Workspace is PreviousTab back to vs main, not CycleTheme",
        WAIT,
    );

    csi_u_letter(&mut tui, 'g');
    csi_u_letter(&mut tui, '1');
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "g1 activates Workspace",
        WAIT,
    );

    csi_u_letter(&mut tui, 'g');
    csi_u_letter(&mut tui, '2');
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "g2 activates vs origin/main",
        WAIT,
    );

    csi_u_letter(&mut tui, 'g');
    csi_u_letter(&mut tui, '3');
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "g3 activates vs main",
        WAIT,
    );

    csi_u_letter(&mut tui, 'g');
    csi_u_letter(&mut tui, '9');
    tui.wait_pred(
        |screen| on_compare_local_main(screen) && !tree_or_theme_fired(screen),
        "g9 missing index is a no-op (stay on vs main)",
        WAIT,
    );

    tui.key('g');
    tui.key('t');
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "raw-byte gt from vs main is NextTab",
        WAIT,
    );

    tui.search("app");
    csi_u_gg(&mut tui);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "workspace") && !tree_cursor_on(screen, "app"),
        "gg still jumps the workspace tree",
        WAIT,
    );
}
