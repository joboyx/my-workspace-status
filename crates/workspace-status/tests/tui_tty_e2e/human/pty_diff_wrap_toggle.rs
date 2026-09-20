use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{left_tree, PtySession};
use crate::seed::{daily_workspace, seed_long_diff_file};
use crate::support::{
    crumb_row, launch_breadcrumb_workspace_only, no_updates_group_folded, no_wrong_overlays,
    panes_tree_focused_diff_unfocused, right_pane, status_row, title_has_files, tree_cursor_on,
    tree_dir_expanded, tree_has, SETTLE_MS, WAIT,
};

const FILE: &str = "unique-diffline.rs";

fn status_has_diff_tail(screen: &str) -> bool {
    status_row(screen).contains(DIFF_HSCROLL_TAIL)
}

fn help_lists_wrap(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    screen.contains("VIEW")
        && compact.contains("i \\")
        && compact.contains("inline / split · wrap")
        && screen.lines().any(|line| {
            line.contains('\\')
                && line.contains("inline / split")
                && line.contains("wrap")
                && !line.contains("flat / tree")
        })
}

fn tree_stays_on_long_file(screen: &str) -> bool {
    tree_has(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
}

fn clipped_new_diff(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    left.contains(FILE)
        && !left.contains(DIFF_HSCROLL_TAIL)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains("nnnn")
        && right.contains("line 0")
        && right.contains("inline (too narrow)")
        && !right.contains("inline (too narrow) ·")
        && !right.contains(DIFF_HSCROLL_TAIL)
        && !right.contains('█')
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !screen.contains("WIP on graph")
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
}

fn wrapped_new_diff(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    left.contains(FILE)
        && !left.contains(DIFF_HSCROLL_TAIL)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains("nnnn")
        && right.contains(DIFF_HSCROLL_TAIL)
        && right.contains("inline (too narrow) · wrap")
        && !right.contains("· pan")
        && !right.contains('█')
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
}

fn idle_chrome_left(screen: &str) -> bool {
    launch_breadcrumb_workspace_only(screen)
        && status_row(screen).contains("focus right")
        && !status_row(screen).contains("drill")
        && no_wrong_overlays(screen)
}

fn long_diff_clipped(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && tree_stays_on_long_file(screen)
        && clipped_new_diff(screen)
        && idle_chrome_left(screen)
}

fn long_diff_wrapped(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && tree_stays_on_long_file(screen)
        && wrapped_new_diff(screen)
        && crumb_row(screen).contains("wrap on")
        && !crumb_row(screen).contains("wrap off")
        && no_wrong_overlays(screen)
}

fn long_diff_unwrapped(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && tree_stays_on_long_file(screen)
        && clipped_new_diff(screen)
        && crumb_row(screen).contains("wrap off")
        && !crumb_row(screen).contains("wrap on")
        && status_row(screen).contains("focus right")
        && !status_row(screen).contains("drill")
        && no_wrong_overlays(screen)
}

/// `\` toggles soft word-wrap on a file diff.
///
/// Docs + help VIEW: `\` is wrap / unwrap. Default clip hides
/// `UNIQUE_DIFF_TAIL` on a long NEW line. Wrap shows that tail without
/// `h`/`l` pan, paints `· wrap`, and toasts `wrap on`. A second `\`
/// restores clip. Pan is a no-op while wrap is on.
///
/// Live PTY (default 140×32 so the NEW line still clips). A no-op, a
/// header-only `· wrap`, or a pan that never needed wrap cannot pass.
#[test]
fn pty_diff_wrap_toggle() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace, FILE, DIFF_HSCROLL_TAIL);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(
        help_lists_wrap,
        "help VIEW lists i \\ inline/split · wrap",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("inline / split · wrap")
                && screen.contains("README.md")
                && screen.contains("? help")
        },
        "Esc closes help so \\ is wrap, not a help key",
        WAIT,
    );

    tui.search("unique-diffline");
    tui.wait_pred(
        long_diff_clipped,
        "search loads the clipped NEW file-diff; tail needs pan or wrap",
        WAIT,
    );

    tui.key('\\');
    tui.wait_pred(
        long_diff_wrapped,
        "\\ wraps the long NEW line; UNIQUE_DIFF_TAIL is visible without pan",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        long_diff_wrapped,
        "wrap holds (not a flicker, header-only, or a pan that revealed the tail)",
        WAIT,
    );

    tui.key('\\');
    tui.wait_pred(
        long_diff_unwrapped,
        "second \\ restores clip; UNIQUE_DIFF_TAIL is hidden again without pan",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        long_diff_unwrapped,
        "clip holds (not a no-op second \\ or a stale wrap header)",
        WAIT,
    );
}
