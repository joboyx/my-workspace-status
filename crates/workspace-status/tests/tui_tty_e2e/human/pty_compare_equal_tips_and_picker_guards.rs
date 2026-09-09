use crate::harness::PtySession;
use crate::seed::{compare_ahead_workspace, daily_workspace, git_stdout};
use crate::support::{GIT_WAIT, WAIT};

fn head_sha(workspace: &std::path::Path) -> String {
    git_stdout(&workspace.join("app"), &["rev-parse", "HEAD"])
}

fn head_branch(workspace: &std::path::Path) -> String {
    git_stdout(
        &workspace.join("app"),
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )
}

/// Equal tips hide dirty files. Diff vs default paints empty committed copy.
#[test]
fn pty_compare_vs_default_equal_tips_hides_dirty() {
    let (_root, workspace) = daily_workspace();
    let before = head_sha(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.wait_contains("Diff vs default", WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| {
            screen.contains("app · vs main")
                && screen.contains("No committed changes")
                && screen.contains("No committed changes vs main")
                && !screen.contains("UNSTAGED")
        },
        "equal tips paint empty committed compare, not dirty UNSTAGED",
        GIT_WAIT,
    );
    assert_eq!(head_sha(&workspace), before);
    tui.key('s');
    tui.wait_contains("Switch to Workspace tab", WAIT);
    assert_eq!(head_sha(&workspace), before);
}

/// Compare picker never checkouts. A missing query stays on the Workspace tab.
#[test]
fn pty_compare_picker_never_checkouts() {
    let (_root, workspace) = compare_ahead_workspace();
    let before = head_sha(&workspace);
    let branch = head_branch(&workspace);
    assert_eq!(branch, "feature/ahead");

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.wait_contains("feature/ahead", GIT_WAIT);
    tui.ctrl_letter('k');
    tui.keys("vs branch");
    tui.wait_contains("Diff vs branch", WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| screen.contains("Compare") && !screen.contains("Checkout"),
        "compare picker title is Compare, not Checkout",
        WAIT,
    );
    tui.keys("zzz-missing");
    tui.wait_contains("No branches to compare", WAIT);
    assert_eq!(head_sha(&workspace), before);
    assert_eq!(head_branch(&workspace), branch);
    tui.esc();
    tui.wait_pred(
        |screen| screen.contains("# workspace") && !screen.contains("app · vs"),
        "Esc closes the picker and stays on Workspace",
        WAIT,
    );
    assert_eq!(head_sha(&workspace), before);
    assert_eq!(head_branch(&workspace), branch);
}
