use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{left_tree, PtySession};
use crate::seed::{daily_workspace, seed_long_diff_file};
use crate::support::{
    crumb_row, launch_breadcrumb_workspace_app, no_updates_group_folded, no_wrong_overlays,
    panes_tree_focused_diff_unfocused, right_pane, right_vbar_at_top, status_row, title_has_files,
    tree_cursor_on, tree_dir_expanded, tree_has, SETTLE_MS, WAIT,
};

const FILE: &str = "unique-diffline.rs";

fn status_has_diff_tail(screen: &str) -> bool {
    status_row(screen).contains(DIFF_HSCROLL_TAIL)
}

fn help_lists_wrap(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    screen.contains("VIEW")
        && compact.contains("i \\")
        && compact.contains("inline / split · wrap · msg")
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
        // The vertical bar shows at the origin; the h-bar waits for a pan.
        && right_vbar_at_top(screen)
        && !right.contains('═')
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
        // Wrap hides the h-bar; the vertical bar still marks the overflow.
        && right_vbar_at_top(screen)
        && !right.contains('═')
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
}

fn idle_chrome_left(screen: &str) -> bool {
    launch_breadcrumb_workspace_app(screen)
        && status_row(screen).contains("focus right")
        && !status_row(screen).contains("drill")
        && no_wrong_overlays(screen)
}

fn long_diff_wrapped_at_launch(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && tree_stays_on_long_file(screen)
        && wrapped_new_diff(screen)
        && idle_chrome_left(screen)
        && !crumb_row(screen).contains("wrap on")
        && !crumb_row(screen).contains("wrap off")
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
/// Docs + help VIEW: `\` is wrap / unwrap. Wrap is on at launch, so a
/// long NEW line shows `UNIQUE_DIFF_TAIL` without `h`/`l` pan and the
/// header paints `· wrap`. `\` restores clip (tail hidden, `wrap off`
/// toast); a second `\` wraps again (`wrap on`). Pan is a no-op while
/// wrap is on.
///
/// Live PTY (default 140×32 so the NEW line clips once wrap is off). A
/// no-op, a header-only `· wrap`, or a pan that never needed wrap cannot
/// pass.
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
        long_diff_wrapped_at_launch,
        "search loads the NEW file-diff wrapped at launch; tail visible without pan",
        WAIT,
    );

    tui.key('\\');
    tui.wait_pred(
        long_diff_unwrapped,
        "\\ turns wrap off; UNIQUE_DIFF_TAIL is clipped again without pan",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        long_diff_unwrapped,
        "clip holds (not a flicker or a no-op \\)",
        WAIT,
    );

    tui.key('\\');
    tui.wait_pred(
        long_diff_wrapped,
        "second \\ wraps the long NEW line again; UNIQUE_DIFF_TAIL is visible",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        long_diff_wrapped,
        "wrap holds (not a stale clip header or a pan that revealed the tail)",
        WAIT,
    );
}
