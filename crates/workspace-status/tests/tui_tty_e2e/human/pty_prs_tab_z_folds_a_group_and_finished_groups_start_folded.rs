use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    forge_env, prs_cursor_line, prs_headers, prs_move_to, prs_move_until, prs_refs, seed_prs, spec,
    tab_strip, total_is, Forge, PrSpec, PrsLine, PRS_BOARD,
};
use crate::support::{GIT_WAIT, WAIT};

/// Four more merged PRs in a second finished group, `ABC-105`, so the tab
/// lists 13 PRs: one more than the fold limit.
const MORE_MERGED: [PrSpec; 4] = [
    spec(
        "app-search",
        "feature/ABC-105-search",
        Forge::GitHub,
        21,
        Badge::Merged,
        "Search the totals",
        "main",
    ),
    spec(
        "app-search-ui",
        "feature/ABC-105-search-ui",
        Forge::GitHub,
        22,
        Badge::Merged,
        "Show the search box",
        "main",
    ),
    spec(
        "api-search",
        "feature/ABC-105-search",
        Forge::GitLab,
        12,
        Badge::Merged,
        "Search the rate limits",
        "main",
    ),
    spec(
        "api-search-docs",
        "feature/ABC-105-docs",
        Forge::GitLab,
        13,
        Badge::Merged,
        "Document the search",
        "main",
    ),
];

/// Rows of the open groups and `other`, with every finished group folded.
const OPEN_ROWS: [&str; 7] = ["#40", "!31", "#43", "#45", "#44", "!32", "#50"];

/// Every row, with every group unfolded.
const ALL_ROWS: [&str; 13] = [
    "#40", "!31", "#43", "#45", "#44", "!32", "#21", "#22", "!12", "!13", "#30", "!20", "#50",
];

const ABC_130: &str = "ABC-130 · > fix CI !31";
const ABC_120: &str = "ABC-120 · > merge #43";
const ABC_125: &str = "ABC-125 · > wait for CI !32";
const ABC_105: &str = "ABC-105 · m all 4 merged";
const ABC_110: &str = "ABC-110 · m all 2 merged";

/// The headers in list order, each with its fold state.
fn headers(folded: [bool; 6]) -> Vec<(bool, String)> {
    [ABC_130, ABC_120, ABC_125, ABC_105, ABC_110, "other"]
        .iter()
        .zip(folded)
        .map(|(text, folded)| (folded, text.to_string()))
        .collect()
}

/// True when the PRs tab shows `want` headers and `rows`.
fn shows(tui: &PtySession, want: &[(bool, String)], rows: &[&str]) -> bool {
    prs_headers(tui) == want && prs_refs(tui) == rows
}

/// Finished groups start folded when the tab lists more than 12 PRs, `Z`
/// unfolds and folds every finished group, and `z` folds or unfolds the
/// group under the cursor (`docs/tui-rust.md`, **PRs tab**).
///
/// Seed: [`PRS_BOARD`] plus [`MORE_MERGED`]: 13 PRs on both forges, with
/// two finished groups (`ABC-105`, four merged PRs; `ABC-110`, two).
///
/// Proof: on open the tab reads `PRs 13` and both finished headers paint
/// the folded chevron `>` with their rows hidden, while the open groups
/// and `other` stay unfolded. `Z` unfolds both finished groups (their rows
/// list in repository then number order), and `Z` again folds both. `z` on
/// the `#43` row folds `ABC-120` only and moves the cursor to its header;
/// `z` again unfolds it. `z` on the folded `ABC-105` header unfolds that
/// group and leaves `ABC-110` folded. The 12-or-fewer side is in
/// `pty_prs_tab_gp_lists_groups_in_order` (nine PRs, finished unfolded).
/// Dropping the start-folded rule lists all 13 rows on open; a `Z` that
/// toggles every group also folds the open ones.
#[test]
fn pty_prs_tab_z_folds_a_group_and_finished_groups_start_folded() {
    let specs: Vec<PrSpec> = PRS_BOARD.iter().chain(&MORE_MERGED).copied().collect();
    let (_root, workspace, forge) = seed_prs("ws-tui-tty-prs-fold", &specs);
    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "13 PRs · 2 need you"),
        "every lookup lands: the workspace row counts 13 PRs",
        GIT_WAIT,
    );

    tui.keys("gp");
    let start = headers([false, false, false, true, true, false]);
    tui.wait_pred(
        |_| shows(&tui, &start, &OPEN_ROWS),
        "the PRs tab opens with both finished groups folded",
        WAIT,
    );
    assert!(
        tab_strip(&tui).contains("PRs 13"),
        "tab strip: {:?}",
        tab_strip(&tui)
    );

    let unfolded = headers([false; 6]);
    tui.shift_letter('z');
    tui.wait_pred(
        |_| shows(&tui, &unfolded, &ALL_ROWS),
        "Z unfolds every finished group",
        WAIT,
    );
    tui.shift_letter('z');
    tui.wait_pred(
        |_| shows(&tui, &start, &OPEN_ROWS),
        "Z again folds every finished group",
        WAIT,
    );

    prs_move_to(&mut tui, "#43");
    tui.key('z');
    let abc_120_folded = headers([false, true, false, true, true, false]);
    tui.wait_pred(
        |_| shows(&tui, &abc_120_folded, &["#40", "!31", "!32", "#50"]),
        "z folds the group of the row under the cursor",
        WAIT,
    );
    assert_eq!(
        prs_cursor_line(&tui),
        Some(PrsLine::Header {
            folded: true,
            text: ABC_120.to_string(),
        }),
        "the cursor moves to the folded group's header"
    );
    tui.key('z');
    tui.wait_pred(
        |_| shows(&tui, &start, &OPEN_ROWS),
        "z again unfolds the group",
        WAIT,
    );

    prs_move_until(
        &mut tui,
        "the ABC-105 header",
        |line| matches!(line, PrsLine::Header { text, .. } if text == ABC_105),
    );
    tui.key('z');
    let abc_105_open = headers([false, false, false, false, true, false]);
    tui.wait_pred(
        |_| {
            shows(
                &tui,
                &abc_105_open,
                &[
                    "#40", "!31", "#43", "#45", "#44", "!32", "#21", "#22", "!12", "!13", "#50",
                ],
            )
        },
        "z on a folded finished header unfolds that group only",
        WAIT,
    );
}
