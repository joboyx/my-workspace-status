use crate::harness::PtySession;
use crate::pr_support::{
    forge_env, list_calls, prs_list, prs_list_rows, seed_prs, tab_strip, PrsLine, PRS_BOARD,
};
use crate::support::{GIT_WAIT, WAIT};

/// The three lines of the empty PRs tab.
const EMPTY: [&str; 3] = [
    "No PR for any checked-out branch.",
    "Open and merged PRs of each checkout's branch show here.",
    "Push a branch and open a PR, then press r.",
];

/// With no PR for any checked-out branch, the PRs tab says so in its three
/// empty lines (`docs/tui-rust.md`, **PRs tab**, states).
///
/// Seed: a GitHub checkout (`app-retry`) and a GitLab checkout
/// (`api-limit`) whose forges answer an empty PR list.
///
/// Proof: once both lookups ran (one `gh pr list` and one GitLab merge
/// request list call), `gp` shows a tab labelled `PRs 0` whose list pane
/// holds exactly the three empty lines, with no group header, PR row, or
/// loading line. A header with no rows, or a loading or error line left
/// over, fails the exact list.
#[test]
fn pty_prs_tab_shows_the_empty_state() {
    let specs: Vec<_> = PRS_BOARD
        .iter()
        .filter(|spec| matches!(spec.number, 31 | 40))
        .copied()
        .collect();
    let (_root, workspace, forge) = seed_prs("ws-tui-tty-prs-empty", &specs);
    for spec in &specs {
        spec.checkout().set_none(&forge);
    }

    let mut tui = PtySession::open_size_with_env(&workspace, 140, 32, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| {
            let calls = list_calls(&forge);
            calls.iter().any(|call| call.starts_with("gh pr list "))
                && calls.iter().any(|call| call.contains("/merge_requests?"))
        },
        "both checkouts are looked up",
        GIT_WAIT,
    );
    tui.send_bytes(b"gp");
    tui.wait_pred(
        |_| {
            prs_list(&tui).is_some_and(|(lines, _)| {
                lines == EMPTY.map(|text| PrsLine::Note(text.to_string()))
            })
        },
        "the empty PRs tab shows its three lines",
        WAIT,
    );
    let rows = prs_list_rows(&tui).expect("PRs list on screen");
    let painted: Vec<&str> = rows
        .iter()
        .map(|row| row.trim())
        .filter(|row| !row.is_empty())
        .collect();
    assert_eq!(painted, EMPTY, "nothing else in the list pane");
    assert!(
        tab_strip(&tui).contains("PRs 0"),
        "tab strip: {:?}",
        tab_strip(&tui)
    );
}
