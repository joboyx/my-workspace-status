use crate::harness::PtySession;
use crate::seed::{compare_ahead_workspace, git_stdout};
use crate::support::{GIT_WAIT, WAIT};

fn head_sha(workspace: &std::path::Path) -> String {
    git_stdout(&workspace.join("app"), &["rev-parse", "HEAD"])
}

fn head_branch(workspace: &std::path::Path) -> String {
    git_stdout(&workspace.join("app"), &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// Palette Diff vs branch opens a compare tab and leaves HEAD on feature.
#[test]
fn pty_compare_palette_opens_tab() {
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
    tui.keys("main");
    tui.enter();
    tui.wait_pred(
        |screen| {
            screen.contains("app · vs main")
                && screen.contains("COMMITTED")
                && screen.contains("alpha.txt")
                && !screen.contains("UNSTAGED")
        },
        "palette picker opens a committed compare tab",
        GIT_WAIT,
    );
    assert_eq!(head_sha(&workspace), before);
    assert_eq!(head_branch(&workspace), branch);
}
