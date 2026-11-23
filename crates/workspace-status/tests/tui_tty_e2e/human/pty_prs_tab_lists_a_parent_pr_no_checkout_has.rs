use crate::common::fake_forge::{set_origin, FakeForge, Reply};
use crate::common::pr_scenarios::{Badge, GITHUB_REPO};
use crate::common::seed::seed_primary_on_main_with_worktree;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_text_is, forge_env, list_calls, named_workspace, prs_detail, prs_list, put_chain, spec,
    tab_strip, Forge, PrSpec, PrsLine,
};
use crate::support::{GIT_WAIT, WAIT};

/// The chain of the repro: `#7` → `#8`. Only `#8` is checked out, in a
/// linked worktree; the primary checkout sits on `main`.
const CHAIN: [PrSpec; 2] = [
    spec(
        "app",
        "feature/auth-refresh",
        Forge::GitHub,
        7,
        Badge::CiFailed,
        "fix(ABC-123): Refresh auth tokens before expiry",
        "main",
    ),
    spec(
        "feat-login",
        "feature/login-page",
        Forge::GitHub,
        8,
        Badge::InReview,
        "feat(ABC-123): Add the login page",
        "feature/auth-refresh",
    ),
];

/// A PR whose base PR no checkout has still lists that parent: the stack
/// walk looks the parent up by its branch (`docs/tui-rust.md`, **PRs tab**).
///
/// Seed: `app` on `main` (the forge has no PR for it) with the
/// linked worktree `app/.worktrees/feat-login` on `feature/login-page`
/// (`#8`, base `feature/auth-refresh`). The fake forge knows `#7` on
/// `feature/auth-refresh`, which no checkout has.
///
/// Proof: the worktree row ends in `#8 2/2 P`; `gp` labels the tab `PRs 2`
/// and lists `#7` (`1/2 · no checkout`) then `#8` (`2/2 after #7`); the
/// detail of `#7` says `checkout none (not checked out)`; the forge calls
/// hold a `--head feature/auth-refresh` lookup. Before the fix the tab
/// listed `#8` alone with no stack mark, and the app never asked for
/// `feature/auth-refresh`.
#[test]
fn pty_prs_tab_lists_a_parent_pr_no_checkout_has() {
    let (root, workspace) = named_workspace("ws-tui-tty-prs-parent", "ws");
    seed_primary_on_main_with_worktree(&workspace, "app", "feat-login", "feature/login-page");
    set_origin(&workspace.join("app"), Forge::GitHub.origin());
    let forge = FakeForge::new(&root);
    put_chain(&forge, &CHAIN);
    // No `origin/HEAD`, so the app does not know `main` is the default
    // branch and looks it up: the forge has no PR for it.
    forge.github_pr_list(GITHUB_REPO, "main", Reply::Json("[]"));

    let mut tui = PtySession::open_size_with_env(&workspace, 160, 45, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| badge_text_is(&tui, "feature/login-page", "#8 2/2 P"),
        "the worktree row ends in `#8 2/2 P`",
        GIT_WAIT,
    );

    tui.send_bytes(b"gp");
    tui.wait_pred(
        |_| {
            prs_list(&tui).is_some_and(|(lines, _)| {
                let rows: Vec<(String, String)> = lines
                    .iter()
                    .filter_map(|line| match line {
                        PrsLine::Row { pr_ref, rest, .. } => Some((pr_ref.clone(), rest.clone())),
                        _ => None,
                    })
                    .collect();
                rows.len() == 2
                    && rows[0].0 == "#7"
                    && rows[0].1.ends_with("1/2 · no checkout")
                    && rows[1].0 == "#8"
                    && rows[1].1.ends_with("2/2 after #7")
            })
        },
        "gp lists #7 (no checkout) and #8 after it",
        WAIT,
    );
    let strip = tab_strip(&tui);
    assert!(strip.contains("PRs 2"), "tab strip: {strip:?}");
    assert!(
        !tui.screen().contains("PR lookup failed"),
        "every call has an answer: no outage\n{}",
        tui.screen()
    );
    tui.wait_pred(
        |_| {
            prs_detail(&tui).is_some_and(|lines| {
                lines.iter().any(|line| {
                    line.split_whitespace().collect::<Vec<_>>().join(" ")
                        == "checkout none (not checked out)"
                })
            })
        },
        "the detail of #7, the PR the header names, has no checkout",
        WAIT,
    );
    let calls = list_calls(&forge);
    assert!(
        calls
            .iter()
            .any(|call| call.contains("--head feature/auth-refresh")),
        "the walk looked the parent up by its branch: {calls:?}"
    );
}
