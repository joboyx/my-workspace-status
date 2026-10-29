use crate::common::fake_forge::{set_origin, FakeForge, Reply, GITHUB_ORIGIN, GITLAB_ORIGIN};
use crate::common::seed::{new_workspace, seed_repo};
use crate::harness::PtySession;
use crate::support::{tree_cursor_on, tree_line_containing, GIT_WAIT, WAIT};

const GITHUB_REPO: &str = "acme/app";
const GITLAB_PROJECT: &str = "acme/services/api";
const PR_URL: &str = "https://github.com/acme/app/pull/7";

/// `gh pr list` answer: one open PR, with the given review decision.
fn github_open(review: &str) -> String {
    format!(
        r#"[{{"number":7,"state":"OPEN","url":"{PR_URL}","headRefName":"feature/pr-open","isCrossRepository":false,"reviewDecision":"{review}","updatedAt":"2026-01-02T03:04:05Z"}}]"#
    )
}

/// `glab api` merge request list answer: one merged MR.
const GITLAB_MERGED: &str = r#"[{"iid":3,"state":"merged","web_url":"https://gitlab.com/acme/services/api/-/merge_requests/3","source_branch":"feature/mr-merged","source_project_id":1,"target_project_id":1,"updated_at":"2026-01-02T03:04:05Z"}]"#;

/// True when the tree row of `repo` carries `badge` as its own cell.
fn row_has_badge(screen: &str, repo: &str, badge: &str) -> bool {
    tree_line_containing(screen, repo)
        .is_some_and(|line| line.split_whitespace().any(|cell| cell == badge))
}

/// The fake `gh` / `glab` stand in for the forge CLIs: a checkout on a
/// GitHub remote with an open PR and a checkout on a GitLab remote with a
/// merged MR paint their badges, and a scenario file swapped mid-run changes
/// the next answer.
///
/// Docs: `docs/tui-tty-e2e.md` (Fake forge). Harness proof: both CLIs appear
/// in the call log, `gx` opens the PR URL in the `$BROWSER` recorder, and
/// replacing the GitHub answer then `r` turns the open badge into approved.
///
/// Seed: `appbox` on `feature/pr-open` with a GitHub `origin`, `apibox` on
/// `feature/mr-merged` with a GitLab `origin`. Only `git remote` runs on the
/// remotes, so nothing touches the network.
#[test]
fn pty_fake_forge_shows_pr_badges() {
    let (root, workspace) = new_workspace("ws-tui-tty-forge");
    seed_repo(&workspace, "appbox", "feature/pr-open", false);
    seed_repo(&workspace, "apibox", "feature/mr-merged", false);
    set_origin(&workspace.join("appbox"), GITHUB_ORIGIN);
    set_origin(&workspace.join("apibox"), GITLAB_ORIGIN);

    let forge = FakeForge::new(&root);
    forge.github_pr_list(
        GITHUB_REPO,
        "feature/pr-open",
        Reply::Json(&github_open("")),
    );
    forge.gitlab_mr_list(
        GITLAB_PROJECT,
        "feature/mr-merged",
        Reply::Json(GITLAB_MERGED),
    );

    let mut tui = PtySession::open_with_env(&workspace, &forge.env());
    tui.wait_pred(
        |screen| row_has_badge(screen, "appbox", "P") && row_has_badge(screen, "apibox", "m"),
        "open PR badge `P` on appbox and merged MR badge `m` on apibox",
        GIT_WAIT,
    );
    let calls = forge.calls();
    assert!(
        calls.iter().any(|call| call.starts_with("gh pr list ")),
        "gh was called: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| call.starts_with("glab api ")),
        "glab was called: {calls:?}"
    );
    assert!(forge.opened().is_empty(), "nothing opened at launch");

    // `gx` on the GitHub checkout opens its PR in the $BROWSER recorder.
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "appbox"),
        "j moves the tree cursor to appbox",
        WAIT,
    );
    tui.keys("gx");
    tui.wait_pred(
        |_| forge.opened() == [PR_URL],
        "gx opens the PR URL in the browser recorder",
        GIT_WAIT,
    );

    // Swap the GitHub answer while the app runs; `r` looks the PR up again.
    forge.github_pr_list(
        GITHUB_REPO,
        "feature/pr-open",
        Reply::Json(&github_open("APPROVED")),
    );
    tui.key('r');
    tui.wait_pred(
        |screen| row_has_badge(screen, "appbox", "A") && !row_has_badge(screen, "appbox", "P"),
        "after the swap and `r`, appbox shows the approved badge `A`",
        GIT_WAIT,
    );
}
