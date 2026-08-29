use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, no_updates_group_folded, no_wrong_overlays, status_row,
    tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has, tree_line_containing,
    GIT_WAIT, SETTLE_MS, WAIT,
};

fn help_lists_j_k_down_up(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains("j   k")
            && line.contains("down / up")
            && !line.contains("page")
            && !line.contains("fold")
            && !line.contains("PgUp")
    })
}

fn left_tree_not_drilled(screen: &str) -> bool {
    screen.contains("focus right")
        && !screen.contains("[workspace]")
        && !screen.contains("drill")
        && status_row(screen).contains(" tree")
        && status_row(screen).contains("? help")
        && no_wrong_overlays(screen)
}

/// Open No-updates group: expanded chevron, `lib` on the tree.
fn no_updates_group_open(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, "No updates") else {
        return false;
    };
    let Some(lib) = tree_line_containing(screen, "lib") else {
        return false;
    };
    tree_dir_expanded(screen, "No updates")
        && !tree_dir_collapsed(screen, "No updates")
        && line.contains('v')
        && line.contains('1')
        && lib.contains("@ lib")
        && lib.contains("& main")
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

fn on_app_not_end(screen: &str) -> bool {
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && no_updates_group_folded(screen)
        && !tree_has(screen, "lib")
        && left_tree_not_drilled(screen)
}

fn on_readme_not_end(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && no_updates_group_folded(screen)
        && !tree_has(screen, "lib")
        && left_tree_not_drilled(screen)
}

fn on_folded_no_updates(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "lib")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && no_updates_group_folded(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && left_tree_not_drilled(screen)
}

fn on_open_no_updates_with_lib(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "lib")
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && no_updates_group_open(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && left_tree_not_drilled(screen)
}

/// CSI-u Repeat of `j` keeps moving the focused tree down to No updates.
///
/// Docs + MOVE: `j k` is down / up. Keymap: hold repeats (terminal
/// key-repeat). Repeat of `j` maps like Press (`Action::Move(1)`).
/// Repeat of `z` / `g` / writes / quit is ignored. A raw `j` byte is a
/// different path. `G` / End jump the last row in one shot.
///
/// Live PTY after first paint (cursor on dirty README): `gg` to the
/// workspace root. One CSI-u `j` press (`CSI 106 ; 1 : 1 u`) lands on
/// `app`, not No updates. One Repeat (`CSI 106 ; 1 : 2 u`) lands on
/// README, still not the last row. Further Repeats walk to folded No
/// updates and clamp there. Then `l` reveals `lib`. A no-op, a
/// single-step, a G/End jump on the first Repeat, or paint-only cannot
/// pass. This leftover does not claim `k` or arrows.
#[test]
fn pty_key_repeat_j_reaches_no_updates() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: cursor on dirty README, No updates folded, no lib",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && help_lists_j_k_down_up(screen)
                && screen.contains("gg   G")
                && screen.contains("top / bottom of focused")
                && screen.contains("PgUp   PgDn")
                && screen.contains("page focused pane")
        },
        "help MOVE lists j k as down/up, not page and not gg/G",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && documented_launch_first_paint(screen)
                && screen.contains("? help")
        },
        "Esc closes help so j Repeat is a tree move, not a help key",
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
        on_app_not_end,
        "one CSI-u j press moves one row to app (G/End would hit No updates; a no-op stays on workspace)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        on_app_not_end,
        "one press holds on app (not a delayed jump to No updates)",
        WAIT,
    );

    tui.letter_repeat('j');
    tui.wait_pred(
        on_readme_not_end,
        "one CSI-u j Repeat moves one more row to README (ignored Repeat stays on app; G would hit No updates)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        on_readme_not_end,
        "one Repeat holds on README (not a delayed G/End jump)",
        WAIT,
    );

    for _ in 0..9 {
        tui.letter_repeat('j');
        tui.wait_ms(50);
    }
    tui.wait_pred(
        on_folded_no_updates,
        "held CSI-u j Repeat walks to folded No updates (a no-op stays on README; one more j would hit merger)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        on_folded_no_updates,
        "Repeat j clamps on No updates (does not wrap, quit, or unfold)",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        on_open_no_updates_with_lib,
        "l on No updates opens the group (Repeat j actually selected that row)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        on_open_no_updates_with_lib,
        "opened No updates holds (not a flicker or Enter drill onto lib)",
        WAIT,
    );
}
