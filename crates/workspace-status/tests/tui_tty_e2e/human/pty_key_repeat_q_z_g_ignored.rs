use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, no_updates_group_folded, no_wrong_overlays,
    status_row, tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has, GIT_WAIT,
    SETTLE_MS, WAIT,
};

fn left_tree_not_drilled(screen: &str) -> bool {
    screen.contains("focus right")
        && !screen.contains("[workspace]")
        && !screen.contains("drill")
        && status_row(screen).contains(" tree")
        && status_row(screen).contains("? help")
        && no_wrong_overlays(screen)
}

fn on_workspace_root(screen: &str) -> bool {
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && no_updates_group_folded(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && left_tree_not_drilled(screen)
}

/// Expanded `app` after one CSI-u `j` press. Graph loaded. Not folded.
fn on_app_expanded(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && !tree_dir_collapsed(screen, "app")
        && no_updates_group_folded(screen)
        && screen.contains("uncommitted changes")
        && screen.contains("seed app")
        && !screen.contains("UNSTAGED")
        && !screen.contains("z…")
        && !screen.contains("Press Ctrl+C again to exit")
        && crumb.contains("workspace › app")
        && !crumb.contains("[app]")
        && left_tree_not_drilled(screen)
}

/// CSI-u Repeat of `q` / `z` / `g` is ignored after a normal CSI-u press move.
///
/// Docs + keymap: hold repeats for nav (`j`/`k`/…). Repeat of `z` / `g` /
/// writes / quit is ignored so chords stay one-shot. This is the live PTY
/// `event::read` path (`CSI code ; 1 : 2 u`). A raw byte is a different path.
///
/// After first paint (cursor on dirty README): `gg` to the workspace root.
/// One CSI-u `j` press (`CSI 106 ; 1 : 1 u`) lands on expanded `app` with
/// README still visible. Repeat `q` must not quit. Repeat `z` must not fold
/// or paint `z…`. Repeat `g` must not arm the `g` chord; a following CSI-u
/// `g` press stays on `app` (it would complete `gg` if Repeat had armed).
/// Then a real `q` byte still quits immediately. A no-op first move, a
/// G/End jump, a fold, a chord arm, or a broken quit cannot pass.
#[test]
fn pty_key_repeat_q_z_g_ignored() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: cursor on dirty README, No updates folded",
        WAIT,
    );

    tui.gg();
    tui.wait_pred(
        on_workspace_root,
        "gg jumps to the workspace root (a no-op stays on README; G would hit No updates)",
        GIT_WAIT,
    );

    tui.letter_press('j');
    tui.wait_pred(
        on_app_expanded,
        "one CSI-u j press moves one row to expanded app with graph loaded (G/End would hit No updates; a no-op stays on workspace)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        on_app_expanded,
        "one press holds on expanded app (not a delayed jump to No updates)",
        WAIT,
    );

    tui.letter_repeat('q');
    tui.assert_running("CSI-u Repeat q must not quit");
    tui.wait_pred(
        on_app_expanded,
        "CSI-u Repeat q stays on expanded app (no Ctrl+C prompt; a press q would quit)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(on_app_expanded, "Repeat q hold (not a delayed quit)", WAIT);
    tui.assert_running("CSI-u Repeat q must not quit after settle");

    tui.letter_repeat('z');
    tui.wait_pred(
        on_app_expanded,
        "CSI-u Repeat z does not fold app or arm z… (a press z on app would hide README and paint z…)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        on_app_expanded,
        "Repeat z hold (not a delayed fold or z… chord)",
        WAIT,
    );

    tui.letter_repeat('g');
    tui.wait_pred(
        on_app_expanded,
        "CSI-u Repeat g does not jump (gg would hit workspace; G would hit No updates)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        on_app_expanded,
        "Repeat g hold (not a delayed gg or G)",
        WAIT,
    );

    tui.letter_press('g');
    tui.wait_pred(
        on_app_expanded,
        "CSI-u g press after Repeat g stays on app (Repeat g did not arm the g chord as a press)",
        WAIT,
    );
    tui.wait_ms(500);
    tui.wait_pred(
        on_app_expanded,
        "lone g press expires; still on app (not delayed gg)",
        WAIT,
    );

    tui.key('q');
    tui.wait_exit_without("Press Ctrl+C again to exit", WAIT);
}
