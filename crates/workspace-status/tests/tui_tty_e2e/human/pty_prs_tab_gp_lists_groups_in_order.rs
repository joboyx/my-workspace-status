use crate::harness::PtySession;
use crate::pr_support::{
    forge_env, prs_cursor_line, prs_detail, prs_headers, prs_list, prs_list_rows, prs_refs,
    seed_prs, tab_strip, total_is, PrsLine, PRS_BOARD,
};
use crate::support::{GIT_WAIT, WAIT};

/// PR rows of the board in list order: groups by rank, rows by repository
/// then number, `#45` right after its base `#43`.
const ROW_ORDER: [&str; 9] = [
    "#40", "!31", "#43", "#45", "#44", "!32", "#30", "!20", "#50",
];

/// `gp` opens the PRs tab with the groups and rows of mixed GitHub and
/// GitLab PRs in their order (`docs/tui-rust.md`, **PRs tab**).
///
/// Seed: [`PRS_BOARD`], nine PRs on both forges. `gp` is pressed once the
/// workspace row counts all nine, so the cursor starts on the final first
/// line.
///
/// Proof: the tab strip reads `PRs 9`; the headers read, in order,
/// `ABC-130 · > fix CI !31` (a CI failed row, so first), `ABC-120 · > merge
/// #43` (needs you), `ABC-125 · > wait for CI !32` (waiting), `ABC-110 · m
/// all 2 merged` (finished, unfolded with 12 PRs or fewer), then `other`;
/// the first header carries the poll age. Rows: `acme/app` before
/// `acme/services/api` in a group, then by number, except the stacked
/// `#45`, which follows its base `#43` and says `after #43`. Each row
/// paints the glyph of its state, and `#40` shows its title without the
/// commit type and key. The cursor starts on the first header and the
/// detail pane shows the PR its next action names (`!31`). Sorting groups by
/// key or rows by number alone fails the order; a group key from the branch
/// only (no commit scope rule) moves `#40` to `other`.
#[test]
fn pty_prs_tab_gp_lists_groups_in_order() {
    let (_root, workspace, forge) = seed_prs("ws-tui-tty-prs-order", &PRS_BOARD);
    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "9 PRs · 2 need you"),
        "every lookup lands: the workspace row counts nine PRs",
        GIT_WAIT,
    );

    tui.send_bytes(b"gp");
    tui.wait_pred(
        |_| {
            prs_refs(&tui) == ROW_ORDER
                && prs_list(&tui).is_some_and(|(lines, _)| {
                    !lines.iter().any(|line| matches!(line, PrsLine::Note(_)))
                })
        },
        "gp opens the PRs tab with every PR listed",
        WAIT,
    );
    let strip = tab_strip(&tui);
    assert!(
        strip.contains("Workspace") && strip.contains("PRs 9"),
        "tab strip: {strip:?}"
    );

    let headers = prs_headers(&tui);
    let want = [
        "ABC-130 · > fix CI !31",
        "ABC-120 · > merge #43",
        "ABC-125 · > wait for CI !32",
        "ABC-110 · m all 2 merged",
        "other",
    ];
    assert_eq!(
        headers,
        want.map(|text| (false, text.to_string())),
        "group order and headers:\n{}",
        tui.screen()
    );
    let list_rows = prs_list_rows(&tui).expect("PRs list on screen");
    let with_age: Vec<&String> = list_rows
        .iter()
        .filter(|row| row.contains("checked "))
        .collect();
    assert!(
        with_age.len() == 1 && with_age[0].contains("ABC-130 ·"),
        "only the first header carries the poll age: {with_age:?}"
    );

    let (lines, _) = prs_list(&tui).expect("PRs list on screen");
    let rows: Vec<(char, String, String)> = lines
        .into_iter()
        .filter_map(|line| match line {
            PrsLine::Row {
                glyph,
                pr_ref,
                rest,
            } => Some((glyph, pr_ref, rest)),
            _ => None,
        })
        .collect();
    let glyphs: Vec<char> = rows.iter().map(|(glyph, _, _)| *glyph).collect();
    assert_eq!(
        glyphs,
        ['P', 'x', 'A', 'P', 'd', 'r', 'm', 'm', 'P'],
        "state glyphs in row order"
    );
    let rest = |pr_ref: &str| -> String {
        rows.iter()
            .find(|(_, at, _)| at == pr_ref)
            .map(|(_, _, rest)| rest.clone())
            .unwrap_or_default()
    };
    assert!(
        rest("#40").starts_with("acme/app Retry failed uploads CI ok"),
        "#40 shows its repository and the title without `feat(ABC-130): `: {:?}",
        rest("#40")
    );
    assert!(
        rest("!31").starts_with("acme/servic… Limit the upload rate CI x")
            && rest("!31").ends_with("CI failed"),
        "!31 shows the cut GitLab path, its title, and its state: {:?}",
        rest("!31")
    );
    assert!(
        rest("#45").ends_with("in review after #43"),
        "#45 is stacked on #43: {:?}",
        rest("#45")
    );
    for pr_ref in ROW_ORDER.iter().filter(|r| **r != "#45") {
        assert!(
            !rest(pr_ref).contains("after #"),
            "{pr_ref} is not stacked: {:?}",
            rest(pr_ref)
        );
    }

    assert_eq!(
        prs_cursor_line(&tui),
        Some(PrsLine::Header {
            folded: false,
            text: "ABC-130 · > fix CI !31".to_string(),
        }),
        "the cursor starts on the first header"
    );
    // Field labels pad to the widest label shown, which changes once the
    // detail lands, so compare single-spaced text.
    let detail: Vec<String> = prs_detail(&tui)
        .expect("detail pane on screen")
        .iter()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        detail.first().map(String::as_str),
        Some("x !31 Limit the upload rate"),
        "the detail shows the PR the header names:\n{}",
        tui.screen()
    );
    assert!(
        detail
            .iter()
            .any(|line| line == "checkout api-limit · primary")
            && detail.iter().any(|line| line == "repo acme/services/api"),
        "the detail names the checkout and the repository:\n{}",
        tui.screen()
    );
}
