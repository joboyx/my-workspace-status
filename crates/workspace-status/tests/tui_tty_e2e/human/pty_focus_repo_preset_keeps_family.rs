use crate::harness::PtySession;
use crate::seed::{tree_focus_family_workspace, FOCUS_LINKED_BRANCH};
use crate::support::{
    after_ack, open_command_palette, tree_cursor_on, tree_focus_first_paint, tree_has,
    tree_has_branch, tree_line_containing, type_palette_filter, workspace_row_has, GIT_WAIT, WAIT,
};

/// Primary branch of `app` in [`tree_focus_family_workspace`].
const PRIMARY_BRANCH: &str = "feature/ABC-123-invoice-export";

/// The app family is focused as a whole: `app` chip, `2 wt` with no
/// `k/n wt` mark, both checkouts listed, the other three repos outside.
fn whole_family_focused(screen: &str) -> bool {
    workspace_row_has(screen, "(o) app · 1/4 repos")
        // The only row with a ` wt` count is the family row (the chip on the
        // workspace row also holds `app`).
        && tree_line_containing(screen, " wt").is_some_and(|line| {
            line.contains("app") && line.contains("2 wt") && !line.contains("/2 wt")
        })
        && tree_has_branch(screen, PRIMARY_BRANCH)
        && tree_has(screen, &format!("L {FOCUS_LINKED_BRANCH}"))
        && tree_has(screen, "3 outside focus · 1 dirty")
        && !tree_has(screen, "services/api")
        && !tree_has(screen, "notes")
}

/// `Focus: this repo` on a linked checkout focuses its family's repo path
/// and keeps the whole family.
///
/// `/login-form` puts the cursor on the linked worktree of `app` (branch
/// `feature/login-form`). The palette row `Focus: this repo` focuses `app`:
/// ack `focus: app`, chip `(o) app · 1/4 repos`, the family row keeps
/// `2 wt` (no `1/2 wt`), both checkouts stay, and `services/api`, `lib` and
/// `notes` fold into `3 outside focus · 1 dirty`.
#[test]
fn pty_focus_repo_preset_keeps_family() {
    let (_root, workspace) = tree_focus_family_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            tree_focus_first_paint(screen) && tree_has(screen, &format!("L {FOCUS_LINKED_BRANCH}"))
        },
        "first paint: no focus, the app family lists its linked worktree",
        GIT_WAIT,
    );
    tui.search("login-form");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, FOCUS_LINKED_BRANCH),
        "/login-form moves the cursor to the linked worktree row of app",
        WAIT,
    );

    open_command_palette(&mut tui);
    type_palette_filter(&mut tui, "this repo");
    tui.wait_contains("Focus: this repo", WAIT);
    tui.enter();
    tui.wait_pred(
        after_ack("focus: app", |screen| {
            !screen.contains("Enter run") && whole_family_focused(screen)
        }),
        "`Focus: this repo` on the linked checkout focuses app and keeps the family",
        WAIT,
    );
}
