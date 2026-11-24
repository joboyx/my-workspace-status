use std::fs;
use std::time::Duration;

use crate::common::fake_forge::{set_origin, FakeForge, Reply};
use crate::common::pr_scenarios::{github_list_json, github_view_json, Badge, GITHUB_TEXT};
use crate::common::seed::seed_repo;
use crate::harness::PtySession;
use crate::pr_support::{
    assert_holds_for, badge_is, forge_env, named_workspace, tab_strip, toast_text,
};
use crate::support::GIT_WAIT;

/// Repository only the second `gh` account can see.
const REPO: &str = "acme-corp/secret";

/// Branch of the seeded checkout, with PR `#7`.
const BRANCH: &str = "feature/auth-refresh";

/// Token the fake `gh auth token` prints for the second account.
const WORK_TOKEN: &str = "fake-token-octo-work";

/// What the active account gets for any call on [`REPO`].
const NOT_VISIBLE: &str =
    "GraphQL: Could not resolve to a Repository with the name 'acme-corp/secret'. (repository)\n";

/// Two logged-in accounts on github.com; `octo-main` is active.
const AUTH_STATUS: &str = r#"{"hosts":{"github.com":[{"state":"success","active":true,"host":"github.com","login":"octo-main"},{"state":"success","active":false,"host":"github.com","login":"octo-work"}]}}"#;

/// The header mark of a failing forge host.
const HEADER: &str = "PR lookup failed";

/// With several `gh` accounts on one host, a repo that only a non-active
/// account can see still gets its badge, and no outage shows
/// (`docs/configuration.md`, **Several `gh` accounts**).
///
/// Seed: `secretbox` on `feature/auth-refresh` with origin
/// `github.com/acme-corp/secret`. The fake `gh` lists `octo-main` (active)
/// and `octo-work`. With no `GH_TOKEN`, every `gh pr` call on the repo fails
/// with `Could not resolve to a Repository`; with `octo-work`'s token it
/// answers PR `#7` in review.
///
/// Proof: the badge reads `#7 P`, and over a second after it no toast and
/// no `PR lookup failed` header mark show. The call log has the `gh auth
/// status` and `gh auth token --user octo-work` calls and no token text.
/// The UI-state file keeps `octo-work` for `github.com/acme-corp`, with no
/// token. Reverting the fallback leaves the failure mark `!` and the outage
/// toast.
#[test]
fn pty_pr_second_gh_account_shows_the_badge_with_no_outage() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-second-account", "ws");
    seed_repo(&workspace, "secretbox", BRANCH, false);
    set_origin(
        &workspace.join("secretbox"),
        "https://github.com/acme-corp/secret.git",
    );
    let forge = FakeForge::new(&root);
    forge.github_auth_status(AUTH_STATUS);
    forge.github_auth_token("octo-main", "fake-token-octo-main");
    forge.github_auth_token("octo-work", WORK_TOKEN);
    let hidden = || Reply::Fail {
        code: 1,
        stderr: NOT_VISIBLE,
    };
    forge.github_pr_list(REPO, BRANCH, hidden());
    forge.github_pr_list_base(REPO, BRANCH, hidden());
    forge.github_pr_view(REPO, 7, hidden());
    forge.github_pr_list_as(
        WORK_TOKEN,
        REPO,
        BRANCH,
        Reply::Json(&github_list_json(7, BRANCH, Badge::InReview, GITHUB_TEXT)),
    );
    forge.github_pr_list_base_as(WORK_TOKEN, REPO, BRANCH, Reply::Json("[]"));
    forge.github_pr_view_as(
        WORK_TOKEN,
        REPO,
        7,
        Reply::Json(&github_view_json(7, BRANCH, Badge::InReview, GITHUB_TEXT)),
    );
    let store = root.join("ui-state.json");
    let store_env = store.display().to_string();

    let tui = PtySession::open_size_with_env(
        &workspace,
        140,
        40,
        &forge_env(&forge, &[("WS_STATUS_UI_STORE", &store_env)]),
    );
    tui.wait_pred(
        |_| badge_is(&tui, "secretbox", Some("#7"), 'P'),
        "the in-review badge `#7 P` through the second account",
        GIT_WAIT,
    );
    assert_holds_for(
        &tui,
        |tui| {
            let screen = tui.screen();
            !toast_text(&screen).contains("Esc dismiss")
                && !tab_strip(tui).contains(HEADER)
                && badge_is(tui, "secretbox", Some("#7"), 'P')
        },
        Duration::from_secs(1),
        "no outage toast, no header mark, the badge stays",
    );

    let calls = forge.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("gh auth status --hostname github.com --json hosts")),
        "the accounts were listed: {calls:?}"
    );
    assert!(
        calls
            .iter()
            .any(|call| call == "gh auth token --hostname github.com --user octo-work"),
        "the second account's token was read: {calls:?}"
    );
    assert!(
        calls.iter().all(|call| !call.contains("fake-token")),
        "no token on argv: {calls:?}"
    );
    let saved = fs::read_to_string(&store).expect("the UI-state file was written");
    assert!(
        saved.contains(r#""github.com/acme-corp": "octo-work""#),
        "the account is kept for the owner: {saved}"
    );
    assert!(
        !saved.contains("fake-token"),
        "no token in the file: {saved}"
    );
}
