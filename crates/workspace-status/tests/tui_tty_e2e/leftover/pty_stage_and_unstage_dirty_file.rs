use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    after_readme_name, crumb_row, has_stage_hint, has_unstage_hint, idle_dirty_readme_unstaged,
    no_wrong_overlays, pane_unstaged_readme, readme_unstaged_badge, status_row, tree_cursor_on,
    tree_has, GIT_WAIT, SETTLE_MS, WAIT,
};

/// Trailing staged `S `, not `M ` / `MS`, not reviewed `*`.
fn readme_staged_badge(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| after.contains("S ") && !after.contains('M') && !after.contains('*'))
}

fn pane_staged_readme(screen: &str) -> bool {
    screen.contains("STAGED")
        && !screen.contains("UNSTAGED")
        && screen.contains("app/README.md")
        && screen.contains("+dirty")
}

/// `s` staged the focused dirty file. File stays. Not Space. Not stash.
fn documented_s_staged(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_staged_badge(screen)
        && pane_staged_readme(screen)
        && has_unstage_hint(screen)
        && !has_stage_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb.contains("staged README.md")
        && !crumb.contains("unstaged")
        && no_wrong_overlays(screen)
}

/// `u` restored unstaged. Same file. Not a no-op after stage.
fn documented_u_unstaged(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && has_stage_hint(screen)
        && !has_unstage_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).contains("unstaged README.md")
        && no_wrong_overlays(screen)
}

/// `s` stages the focused dirty file; `u` unstages.
///
/// Docs: Help GIT `s` = stage scope, `u` = unstage scope. Configuration:
/// file / dir / repo. Live PTY after first paint staged `app/README.md`
/// (`git add`): tree badge `M ` → `S `, pane UNSTAGED → STAGED, status
/// `s stage` → `u unstage`, breadcrumb `staged README.md`. `u` reversed
/// (`git restore --staged`). Not Space reviewed (`*`). Not Shift+S stash
/// overlay. Not a toast-only tick.
///
/// After first paint the cursor is already on the dirty README. Do not
/// `/` search. A no-op, wrong file, stage-only with no unstage, paint
/// flicker, toast-only, Space `*`, or stash overlay is red.
#[test]
fn pty_stage_and_unstage_dirty_file() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unstaged,
        "first paint: cursor on dirty README, unstaged, stage hint, no unstage",
        WAIT,
    );

    tui.key('s');
    tui.wait_pred(
        documented_s_staged,
        "s stages focused README: tree S, pane STAGED, unstage hint, staged toast",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_s_staged,
        "staged paint holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key('u');
    tui.wait_pred(
        documented_u_unstaged,
        "u unstages the same README: tree M, pane UNSTAGED, stage hint",
        GIT_WAIT,
    );
}
