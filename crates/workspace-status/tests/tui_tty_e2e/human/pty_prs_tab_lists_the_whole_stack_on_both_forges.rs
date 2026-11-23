use crate::common::pr_scenarios::{github_pr_url, gitlab_mr_url};
use crate::harness::PtySession;
use crate::pr_support::{
    badge_text_is, click_badge, forge_env, has_run, line_with, prs_detail, prs_headers, prs_list,
    prs_move_to, screen_has_run, seed_stacks, tab_strip, PrsLine, GITHUB_STACK, GITLAB_STACK,
};
use crate::support::{ack_rows, GIT_WAIT, WAIT};

/// Every member of both chains, in list order: the chain stays together,
/// root first, each PR after its base.
const ROW_ORDER: [&str; 7] = ["#6", "#7", "#8", "#9", "!4", "!5", "!6"];

/// True when the PRs tab detail holds `want` on consecutive lines.
fn detail_has(tui: &PtySession, want: &[&str]) -> bool {
    prs_detail(tui).is_some_and(|lines| has_run(&lines, want))
}

/// The PR popover, the PRs tab list, and its detail show the whole stacked
/// chain of a checked-out PR, also the members no checkout has, on GitHub
/// and on GitLab; the actions of a stack-only row act on its PR
/// (`docs/tui-rust.md`, **PRs tab**, PR popover `stack` lines).
///
/// Seed: [`seed_stacks`]: GitHub `#6` → `#7` → `#8` → `#9` with `#7` and
/// `#8` checked out, GitLab `!4` → `!5` → `!6` with `!5` checked out.
///
/// Proof: the pinned popover of `!5` lists `stack 1/3 P !4 in review · not
/// checked out`, `> 2/3 r !5 CI running`, `3/3 P !6 in review · not checked
/// out` on consecutive lines. `gp` labels the tab `PRs 7` and lists the
/// seven members in chain order under the headers
/// `ABC-123 · > fix CI #7` and `ABC-125 · > get review !4`; each row ends
/// in its position and base, and the rows
/// no checkout has end in `· no checkout`. The detail of `#7` lists the four
/// chain lines with `#7` marked `>`. On the stack-only `#6` the detail says
/// `checkout none (not checked out)`, Enter opens `#6` and `o` says `#6 is
/// not checked out`; on `!6` Enter opens the merge request, `y` copies its
/// URL, and `gh` says `!6 is not checked out`. A walk that never looks up
/// a PR no checkout has lists only `#7`, `#8`, `!5`.
#[test]
fn pty_prs_tab_lists_the_whole_stack_on_both_forges() {
    let (_root, workspace, forge) = seed_stacks("ws-tui-tty-prs-stack");
    let mut tui = PtySession::open_size_with_env(&workspace, 160, 45, &forge_env(&forge, &[]));
    for (dir, badge) in [
        ("app-auth", "#7 2/4 x"),
        ("app-login", "#8 3/4 P"),
        ("api-limit", "!5 2/3 r"),
    ] {
        tui.wait_pred(
            |_| badge_text_is(&tui, dir, badge),
            &format!("{dir} ends in `{badge}`"),
            GIT_WAIT,
        );
    }

    click_badge(&mut tui, "api-limit");
    tui.wait_pred(
        |screen| {
            screen_has_run(
                screen,
                &[
                    "stack 1/3 P !4 in review · not checked out",
                    "> 2/3 r !5 CI running",
                    "3/3 P !6 in review · not checked out",
                ],
            )
        },
        "the pinned popover of !5 lists its chain",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| line_with(screen, "r PR · CI running").is_none(),
        "Esc closes the popover",
        WAIT,
    );

    tui.send_bytes(b"gp");
    tui.wait_pred(
        |_| {
            prs_list(&tui).is_some_and(|(lines, _)| {
                let refs: Vec<&str> = lines
                    .iter()
                    .filter_map(|line| match line {
                        PrsLine::Row { pr_ref, .. } => Some(pr_ref.as_str()),
                        _ => None,
                    })
                    .collect();
                refs == ROW_ORDER && !lines.iter().any(|line| matches!(line, PrsLine::Note(_)))
            })
        },
        "gp lists every member of both chains in chain order",
        WAIT,
    );
    let strip = tab_strip(&tui);
    assert!(
        strip.contains("PRs 7"),
        "the label counts the rows: {strip:?}"
    );
    assert_eq!(
        prs_headers(&tui),
        [
            (false, "ABC-123 · > fix CI #7".to_string()),
            (false, "ABC-125 · > get review !4".to_string()),
        ],
        "one group per chain:\n{}",
        tui.screen()
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
    let want = [
        ('P', "#6", "1/4 · no checkout"),
        ('x', "#7", "2/4 after #6"),
        ('P', "#8", "3/4 after #7"),
        ('d', "#9", "4/4 after #8 · no checkout"),
        ('P', "!4", "1/3 · no checkout"),
        ('r', "!5", "2/3 after !4"),
        ('P', "!6", "3/3 after !5 · no checkout"),
    ];
    for ((glyph, pr_ref, rest), (want_glyph, want_ref, stack)) in rows.iter().zip(want) {
        assert_eq!(
            (*glyph, pr_ref.as_str()),
            (want_glyph, want_ref),
            "row glyph and ref"
        );
        assert!(
            rest.ends_with(stack),
            "{pr_ref} ends in `{stack}`: {rest:?}"
        );
    }
    for (spec, (_, _, rest)) in GITHUB_STACK.iter().chain(GITLAB_STACK.iter()).zip(&rows) {
        // The title without its key and commit type; the column keeps at
        // least 16 cells before it cuts the rest with `…`.
        let title = spec.text.title.split_once(' ').map_or("", |(_, t)| t);
        let head: String = title.chars().take(16).collect();
        assert!(
            rest.contains(&head),
            "{} shows its title `{title}`: {rest:?}",
            spec.number
        );
    }

    // The cursor starts on the first header, which names #7.
    tui.wait_pred(
        |_| {
            detail_has(
                &tui,
                &[
                    "stack 1/4 P #6 in review · not checked out",
                    "> 2/4 x #7 CI failed",
                    "3/4 P #8 in review",
                    "4/4 d #9 draft · not checked out",
                ],
            )
        },
        "the detail of #7 lists the chain, #7 marked",
        WAIT,
    );

    prs_move_to(&mut tui, "#6");
    tui.wait_pred(
        |_| {
            detail_has(&tui, &["checkout none (not checked out)"])
                && detail_has(
                    &tui,
                    &[
                        "stack > 1/4 P #6 in review · not checked out",
                        "2/4 x #7 CI failed",
                    ],
                )
        },
        "the detail of the stack-only #6 has no checkout and marks #6",
        WAIT,
    );
    let before = forge.opened().len();
    tui.enter();
    tui.wait_pred(
        |screen| ack_rows(screen).contains("opened PR #6") && forge.opened().len() == before + 1,
        "Enter on #6 opens its PR",
        WAIT,
    );
    assert_eq!(
        forge.opened().last().map(String::as_str),
        Some(github_pr_url(6).as_str())
    );
    tui.key('o');
    tui.wait_pred(
        |screen| screen.contains("#6 is not checked out") && prs_list(&tui).is_some(),
        "o on #6 refuses and the PRs tab stays",
        WAIT,
    );

    prs_move_to(&mut tui, "!6");
    let before = forge.opened().len();
    tui.enter();
    tui.wait_pred(
        |screen| ack_rows(screen).contains("opened PR !6") && forge.opened().len() == before + 1,
        "Enter on !6 opens its merge request",
        WAIT,
    );
    assert_eq!(
        forge.opened().last().map(String::as_str),
        Some(gitlab_mr_url(6).as_str())
    );
    tui.key('y');
    let url = gitlab_mr_url(6);
    tui.wait_clipboard_pred(
        |payloads| payloads.last().is_some_and(|last| *last == url),
        "y on !6 copies its URL",
        WAIT,
    );
    tui.send_bytes(b"gh");
    tui.wait_pred(
        |screen| screen.contains("!6 is not checked out"),
        "gh on !6 refuses: no popover to pin",
        WAIT,
    );
}
