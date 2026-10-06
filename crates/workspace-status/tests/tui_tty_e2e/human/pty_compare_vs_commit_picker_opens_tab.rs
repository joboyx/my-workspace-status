use crate::harness::{left_tree, PtySession};
use crate::seed::{compare_history_workspace, git_stdout, HISTORY_SUBJECTS};
use crate::support::{GIT_WAIT, WAIT};

fn rev(workspace: &std::path::Path, rev: &str) -> String {
    git_stdout(&workspace.join("app"), &["rev-parse", rev])
}

/// Palette Diff vs commit lists HEAD's ancestors. A typed subject filter
/// plus Enter opens `app ↔ <short>` with the files changed since that commit.
#[test]
fn pty_compare_vs_commit_picker_opens_tab() {
    let (_root, workspace) = compare_history_workspace();
    let [oldest, middle, _] = HISTORY_SUBJECTS;
    let before = rev(&workspace, "HEAD");
    let oldest_sha = rev(&workspace, "HEAD~2");
    let middle_short = rev(&workspace, "HEAD~1")[..7].to_string();
    let label = format!("app ↔ {}", &oldest_sha[..7]);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.wait_contains("feature/history", GIT_WAIT);
    tui.key(':');
    tui.keys("vs commit");
    tui.wait_contains("Diff vs commit in new tab", WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| {
            screen.contains("Compare vs commit")
                && screen.contains(&format!("{middle_short}  {middle}"))
                && screen.contains(oldest)
                && screen.contains("seed app")
        },
        "compare picker lists HEAD's ancestors as `<short>  <subject>`",
        GIT_WAIT,
    );

    tui.keys("first");
    tui.wait_pred(
        |screen| screen.contains(oldest) && !screen.contains(&format!("{middle_short}  {middle}")),
        "typed subject filter keeps only the oldest history commit",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            let left = left_tree(screen);
            screen
                .lines()
                .any(|line| line.contains("Workspace") && line.contains(&label))
                && screen.contains("COMMITTED")
                && left.contains("second.txt")
                && left.contains("third.txt")
                && !left.contains("first.txt")
                && !left.contains("README.md")
        },
        "picked commit opens a tab with the files changed since it",
        GIT_WAIT,
    );
    assert_eq!(rev(&workspace, "HEAD"), before);
}
