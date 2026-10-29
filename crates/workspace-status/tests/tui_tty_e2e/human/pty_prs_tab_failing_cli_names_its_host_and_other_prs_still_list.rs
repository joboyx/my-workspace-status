use crate::common::fake_forge::Reply;
use crate::common::pr_scenarios::{GITHUB_REPO, GITLAB_PROJECT};
use crate::harness::PtySession;
use crate::pr_support::{
    forge_env, list_calls, prs_list, prs_list_rows, prs_refs, seed_prs, tab_strip, PrSpec, PrsLine,
    PRS_BOARD,
};
use crate::support::{GIT_WAIT, WAIT};

/// The board PRs this test keeps: GitHub `#40` (`ABC-130`, in review) and
/// `#43` (`ABC-120`, ready), and GitLab `!31` (`ABC-130`, CI failed).
fn specs() -> Vec<PrSpec> {
    PRS_BOARD
        .iter()
        .filter(|spec| matches!(spec.number, 31 | 40 | 43))
        .copied()
        .collect()
}

/// The CLI error lines above the groups.
fn notes(tui: &PtySession) -> Vec<String> {
    prs_list(tui)
        .map(|(lines, _)| {
            lines
                .into_iter()
                .filter_map(|line| match line {
                    PrsLine::Note(text) => Some(text),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The list pane row of the first group header, poll age included.
fn first_header_row(tui: &PtySession) -> String {
    prs_list_rows(tui)
        .and_then(|rows| rows.into_iter().find(|row| row.contains("ABC-1")))
        .unwrap_or_default()
}

/// A failing `glab` shows one error line with its host above the groups
/// while the GitHub PRs still list, and a failing `gh` does the same for
/// GitLab (`docs/tui-rust.md`, **PRs tab**, states).
///
/// Seed: GitHub `#40` and `#43` and GitLab `!31`. The GitLab list call
/// exits 1 with a two-line stderr from the start.
///
/// Proof: the tab reads `PRs 2`; the list shows `! glab (gitlab.com): glab:
/// 401 Unauthorized` (first stderr line only), then `ABC-120` with `#43` and
/// `ABC-130` with `#40`, and no `!31`; the first header ends in `· lookups
/// failing`. Then `glab` answers and `gh` fails; `r` on the tab looks every
/// checkout up again: the `glab` line goes, `! gh (github.com): gh: HTTP
/// 502` shows, `!31` lists (CI failed, so `ABC-130` moves first), and the
/// GitHub PRs stay listed from their last answer. Dropping the error line
/// leaves no `!` line; dropping the PRs of a failing host empties the list
/// in the second step.
#[test]
fn pty_prs_tab_failing_cli_names_its_host_and_other_prs_still_list() {
    let specs = specs();
    let (_root, workspace, forge) = seed_prs("ws-tui-tty-prs-cli-fails", &specs);
    let gitlab = specs
        .iter()
        .find(|spec| spec.number == 31)
        .copied()
        .expect("!31 in the board");
    forge.gitlab_mr_list(
        GITLAB_PROJECT,
        gitlab.branch,
        Reply::Fail {
            code: 1,
            stderr: "glab: 401 Unauthorized\nsecond line of stderr\n",
        },
    );

    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| list_calls(&forge).len() >= 3,
        "every checkout is looked up once",
        GIT_WAIT,
    );
    tui.keys("gp");
    tui.wait_pred(
        |_| {
            notes(&tui) == ["! glab (gitlab.com): glab: 401 Unauthorized"]
                && prs_refs(&tui) == ["#43", "#40"]
        },
        "the glab error line and the GitHub PRs",
        GIT_WAIT,
    );
    assert!(
        !tui.screen().contains("second line of stderr"),
        "only the first stderr line shows:\n{}",
        tui.screen()
    );
    assert!(
        tab_strip(&tui).contains("PRs 2"),
        "tab strip: {:?}",
        tab_strip(&tui)
    );
    assert!(
        first_header_row(&tui).ends_with("· lookups failing"),
        "the first header says the lookups fail: {:?}",
        first_header_row(&tui)
    );

    gitlab
        .checkout()
        .set_with(&forge, gitlab.badge, gitlab.text);
    for spec in specs.iter().filter(|spec| spec.number != 31) {
        forge.github_pr_list(
            GITHUB_REPO,
            spec.branch,
            Reply::Fail {
                code: 1,
                stderr: "gh: HTTP 502\n",
            },
        );
    }
    let before = list_calls(&forge).len();
    tui.key('r');
    tui.wait_pred(
        |_| list_calls(&forge).len() >= before + 3,
        "r looks every checkout up again",
        WAIT,
    );
    tui.wait_pred(
        |_| {
            notes(&tui) == ["! gh (github.com): gh: HTTP 502"]
                && prs_refs(&tui) == ["#40", "!31", "#43"]
        },
        "the gh error line, the GitLab PR, and the kept GitHub PRs",
        GIT_WAIT,
    );
}
