use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, named_workspace, seed_checkouts, total_is, tree_edge, tree_row_text,
    tree_text_paint, Forge, PrCheckout, MODIFIED, MUTED, POLL_MS,
};
use crate::support::GIT_WAIT;

/// Narrowest tree (inner columns) that paints refs and the full total.
const WIDE_TREE_COLS: u16 = 50;

/// Workspace row text, or an empty string until the row is on screen.
fn total_row(tui: &PtySession, name: &str) -> String {
    tree_row_text(tui, name).unwrap_or_default()
}

/// The workspace row counts PRs and those that need you
/// (`docs/tui-rust.md`, **Pull request badge**, workspace total).
///
/// Seed: `ws` holds `appbox` (GitHub `#7`) and `apibox` (GitLab `!5`), both
/// polled every 300 ms. A failed PR and a ready PR need you; a PR in
/// review does not.
///
/// Proof: the row ends in `2 PRs · 2 need you`, with `2 PRs` muted and
/// `2 need you` amber and bold. A narrow tree (120 columns, under the
/// 50-column rule) shows only `2 need you`. After the swaps the total reads
/// `2 PRs · 1 needs you` (singular verb), `2 PRs` with none needing you, and
/// `1 PR` when one checkout loses its PR. Reverting the total drops the
/// text from the row; reverting the singular, narrow, or none rules fails
/// the matching wait.
#[test]
fn pty_pr_workspace_total_counts_prs_and_those_that_need_you() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-total", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::Ready);

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL_MS)]),
    );
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "2 PRs · 2 need you"),
        "the workspace row ends in `2 PRs · 2 need you`",
        GIT_WAIT,
    );
    assert_eq!(
        tree_text_paint(&tui, "# ws", "2 PRs"),
        Some((MUTED, false)),
        "`2 PRs` paints muted"
    );
    assert_eq!(
        tree_text_paint(&tui, "# ws", "2 need you"),
        Some((MODIFIED, true)),
        "`2 need you` paints amber and bold"
    );

    tui.resize(120, 32);
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "2 need you") && !total_row(&tui, "# ws").contains("PRs"),
        "a tree under 50 columns shows only `2 need you`",
        GIT_WAIT,
    );
    tui.resize(140, 32);

    gitlab.set(&forge, Badge::InReview);
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "2 PRs · 1 needs you"),
        "one PR in review: `2 PRs · 1 needs you`",
        GIT_WAIT,
    );

    github.set(&forge, Badge::InReview);
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "2 PRs") && !total_row(&tui, "# ws").contains("need"),
        "no PR needs you: `2 PRs` alone",
        GIT_WAIT,
    );

    gitlab.set_none(&forge);
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "1 PR"),
        "one PR left: `1 PR`",
        GIT_WAIT,
    );
}

/// A workspace row that is too short for both parts keeps `2 need you` and
/// drops `2 PRs`, at a tree width that is not narrow.
///
/// Seed: the workspace directory is named `workspace`, a long name, so at 140
/// columns (inner tree 54, refs on) the room after the name holds
/// `0 changed · all current  2 need you` but not `...  2 PRs · 2 need you`.
/// `appbox` (GitHub `#7`) is CI failed and `apibox` (GitLab `!5`) is ready.
///
/// Proof: the tree is 50 columns or wider (the badge still carries its ref),
/// and the row ends in `2 need you`, amber and bold, with no `PRs` on it. At
/// 160 columns both parts fit again. With both PRs in review nothing needs
/// you and the row shows `2 PRs`. The old order (`2 PRs` kept, `need you`
/// dropped first) shows `2 PRs` at 140 columns and fails the first wait.
#[test]
fn pty_pr_workspace_total_keeps_need_you_when_the_row_is_short() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-total-short", "workspace");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::Ready);

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL_MS)]),
    );
    tui.wait_pred(
        |_| {
            tree_edge(&tui).is_some_and(|edge| edge.glyph_col >= WIDE_TREE_COLS)
                && badge_is(&tui, "appbox", Some("#7"), 'x')
                && badge_is(&tui, "apibox", Some("!5"), 'A')
                && total_is(&tui, "# workspace", "2 need you")
                && !total_row(&tui, "# workspace").contains("PRs")
        },
        "a tree 50+ columns wide, too short for both parts, ends in `2 need you` alone",
        GIT_WAIT,
    );
    assert_eq!(
        tree_text_paint(&tui, "# workspace", "2 need you"),
        Some((MODIFIED, true)),
        "`2 need you` paints amber and bold"
    );

    tui.resize(160, 32);
    tui.wait_pred(
        |_| total_is(&tui, "# workspace", "2 PRs · 2 need you"),
        "at 160 columns both parts fit: `2 PRs · 2 need you`",
        GIT_WAIT,
    );

    tui.resize(140, 32);
    github.set(&forge, Badge::InReview);
    gitlab.set(&forge, Badge::InReview);
    tui.wait_pred(
        |_| {
            total_is(&tui, "# workspace", "2 PRs")
                && !total_row(&tui, "# workspace").contains("need")
        },
        "nothing needs you: the row shows `2 PRs`",
        GIT_WAIT,
    );
}
