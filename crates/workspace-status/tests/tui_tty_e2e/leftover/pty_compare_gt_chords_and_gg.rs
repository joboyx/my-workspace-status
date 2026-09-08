use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

fn csi_u_letter(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

fn csi_u_gg(tui: &mut PtySession) {
    csi_u_letter(tui, 'g');
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

/// CSI-u `gt` / `gT` / `g1` switch tabs. `gg` still jumps the workspace tree.
#[test]
fn pty_compare_gt_chords_and_gg() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    open_default_and_main(&mut tui);

    tui.key('g');
    csi_u_letter(&mut tui, '1');
    tui.wait_pred(
        |screen| {
            screen.contains("Workspace")
                && screen.contains("# workspace")
                && !screen.contains("COMMITTED")
        },
        "g1 activates Workspace",
        WAIT,
    );

    tui.key('g');
    csi_u_letter(&mut tui, 't');
    tui.wait_pred(
        |screen| screen.contains("app · vs origin/main") && screen.contains("COMMITTED"),
        "gt activates the next compare tab",
        WAIT,
    );

    tui.key('g');
    tui.shift_letter('T');
    tui.wait_pred(
        |screen| screen.contains("# workspace") && !screen.contains("COMMITTED"),
        "gT activates the previous tab",
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
