use crate::common::fake_forge::{set_origin, set_origin_head, FakeForge};
use crate::common::pr_scenarios::Badge;
use crate::common::seed::seed_repo;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, list_calls, named_workspace, seed_checkouts, total_is, tree_badge,
    tree_row_text, Forge, PrCheckout,
};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// Status line of a PR action on the default branch.
const REFUSAL: &str = "main is the default branch: no PR";

/// List calls of the fake forge for branch `main`.
fn main_lookups(forge: &FakeForge) -> Vec<String> {
    list_calls(forge)
        .into_iter()
        .filter(|call| call.contains("--head main"))
        .collect()
}

/// A checkout on its repository's default branch has no PR status
/// (`docs/configuration.md`, **Pull request badges**).
///
/// Seed: `trunk` (GitHub) on `main`, the branch `origin/HEAD` names, with a
/// change so its row shows; the fake forge would answer `#8` ready for
/// `main` if asked. `appbox` (GitHub) on `feature/auth-refresh` with `#7` in
/// review. Polling is off, so only the launch, `r`, and `gx` could ask.
///
/// Proof: `appbox` shows `#7 P` and the workspace total reads `1 PR`; the
/// `trunk` row paints no badge. `r` on `trunk` and then on `appbox` (whose
/// answer turned CI failed) shows `#7 x`, so both reloads ran, and the
/// total reads `1 PR · 1 needs you`. `gx` on `trunk` says `main is the
/// default branch: no PR` and opens nothing. The call log never holds a
/// `--head main` lookup. Without the rule the launch looks `main` up,
/// `trunk` shows `#8 A`, and the total reads `2 PRs`.
#[test]
fn pty_pr_default_branch_checkout_is_never_looked_up() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-default-branch", "ws");
    let forge = FakeForge::new(&root);
    let feature = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let main = PrCheckout::new("trunk", "main", Forge::GitHub, 8);
    seed_checkouts(&workspace, &[feature.clone()]);
    seed_repo(&workspace, "trunk", "main", true);
    set_origin(&workspace.join("trunk"), Forge::GitHub.origin());
    set_origin_head(&workspace.join("trunk"), "main");
    feature.set(&forge, Badge::InReview);
    main.set(&forge, Badge::Ready);

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", "0")]),
    );
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'P')
                && total_is(&tui, "# ws", "1 PR")
                && tree_row_text(&tui, "trunk").is_some()
        },
        "appbox shows #7, the total reads `1 PR`, and the trunk row shows",
        GIT_WAIT,
    );

    // `r` and `gx` on trunk, then `r` on appbox with a new answer:
    // appbox's badge turns only after the later reload ran. The cursor
    // starts on trunk's changed file, the first change in the tree.
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md"),
        "cursor on trunk's README.md",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "trunk"),
        "cursor on trunk",
        WAIT,
    );
    tui.key('r');
    feature.set(&forge, Badge::CiFailed);
    tui.send_bytes(b"gx");
    tui.wait_pred(
        |screen| screen.contains(REFUSAL),
        "gx on trunk names the default branch",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "appbox"),
        "cursor back on appbox",
        WAIT,
    );
    tui.key('r');
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#7"), 'x'),
        "r on appbox shows the failed PR",
        GIT_WAIT,
    );

    let trunk = tree_badge(&tui, "trunk").expect("trunk row");
    assert!(
        trunk.ref_text.is_none() && !Badge::ALL.iter().any(|b| b.ascii() == trunk.glyph),
        "the trunk row paints no badge:\n{}",
        tui.screen()
    );
    assert!(
        total_is(&tui, "# ws", "1 PR · 1 needs you"),
        "the total counts only #7:\n{}",
        tui.screen()
    );
    assert_eq!(
        main_lookups(&forge),
        Vec::<String>::new(),
        "no lookup for main: {:?}",
        forge.calls()
    );
    assert!(forge.opened().is_empty(), "gx opened nothing");
}
