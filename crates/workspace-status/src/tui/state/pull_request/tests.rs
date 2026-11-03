use std::path::{Path, PathBuf};

use workspace_status_graph::{Commit, GraphModel, Worktree};

use super::super::super::app::apply_one_repo_snapshot;
use super::super::super::icons::IconKind;
use super::super::super::popover::{popover_sections, IconTarget};
use super::super::super::status::StatusKind;
use super::super::super::tree::pr_badge_kind;
use super::super::{FocusPane, IconHit};
use super::*;
use crate::helpers::DETACHED_HEAD_BRANCH;
use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};
use crate::testutil::{app_state_of, repo_snapshot, unstaged_change};
use crate::tui::action::Action;

fn repo(name: &str, branch: &str) -> RepoSnapshot {
    RepoSnapshot {
        branch: branch.into(),
        head: "abc".into(),
        has_unstaged: true,
        changes: vec![unstaged_change("src/lib.rs")],
        ..repo_snapshot(name)
    }
}

fn linked(name: &str, primary: &str, branch: &str) -> RepoSnapshot {
    let mut row = repo(name, branch);
    row.checkout_kind = CheckoutKind::Linked;
    row.primary_repo = Some(primary.into());
    row
}

fn app_with(repos: &[RepoSnapshot], hidden: &[&str]) -> AppState {
    let hidden: Vec<String> = hidden.iter().map(|name| name.to_string()).collect();
    app_state_of(build_workspace_snapshot(repos, &hidden, false, &[]))
}

fn focus(app: &mut AppState, id: &str) {
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.id == id)
        .unwrap_or_else(|| panic!("missing row {id}"));
}

fn gx(app: &mut AppState) -> Effect {
    app.status = StatusMessage::default();
    app.dispatch(Action::OpenPullRequest)
}

fn open(repo: &str, branch: &str) -> Effect {
    Effect::OpenPullRequest {
        repo: PathBuf::from(repo),
        branch: branch.into(),
        page: PrPage::Pr,
    }
}

fn assert_no_pr_for_row(app: &mut AppState, what: &str) {
    assert_eq!(gx(app), Effect::None, "{what}");
    assert_eq!(app.status, NO_PR_FOR_ROW, "{what}");
    assert_eq!(app.status.kind(), StatusKind::Warn, "{what}");
}

fn found(state: PrState) -> PrLookup {
    PrLookup::Found(PullRequest::fixture(
        7,
        "https://github.com/octo/demo/pull/7",
        state,
    ))
}

const REMOTE: &str = "git@github.com:octo/demo.git";

#[test]
fn gx_on_a_checkout_row_opens_that_branch() {
    let mut app = app_with(&[repo("solo", "feature")], &[]);
    focus(&mut app, "repo:solo");
    assert_eq!(gx(&mut app), open("solo", "feature"));

    let mut app = app_with(
        &[
            repo("app", "main"),
            linked("app/.worktrees/feat", "app", "feat"),
        ],
        &[],
    );
    focus(&mut app, "checkout:app");
    assert_eq!(gx(&mut app), open("app", "main"));
    focus(&mut app, "checkout:app/.worktrees/feat");
    assert_eq!(gx(&mut app), open("app/.worktrees/feat", "feat"));
    assert!(app.status.is_empty());
}

#[test]
fn gx_on_rows_without_a_branch_says_no_pr_for_this_row() {
    let mut app = app_with(
        &[
            repo("app", "main"),
            linked("app/.worktrees/feat", "app", "feat"),
            repo("det", DETACHED_HEAD_BRANCH),
        ],
        &[],
    );
    focus(&mut app, "repo:app");
    assert_no_pr_for_row(&mut app, "repo that groups checkouts");
    focus(&mut app, "repo:det");
    assert_no_pr_for_row(&mut app, "detached HEAD");
    focus(&mut app, "dir:det:src");
    assert_no_pr_for_row(&mut app, "dir");
    let file = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("file row");
    app.cursor = file;
    assert_no_pr_for_row(&mut app, "file");
    app.cursor = 0;
    assert_no_pr_for_row(&mut app, "workspace");
}

#[test]
fn gx_on_an_explorer_file_says_no_pr_for_this_row() {
    let mut app = app_with(&[repo("app", "feature")], &[]);
    focus(&mut app, "repo:app");
    app.open_loaded_inner_file("app", "README.md", &["# app"]);
    assert!(app.is_explorer_tab());
    assert_no_pr_for_row(&mut app, "Explorer file");
}

#[test]
fn gx_in_the_graph_opens_worktree_rows_with_a_branch_only() {
    let mut app = app_with(&[repo("app", "main")], &[]);
    focus(&mut app, "repo:app");
    let worktree = |path: &str, branch: Option<&str>| Worktree {
        path: path.into(),
        head_id: None,
        branch: branch.map(str::to_string),
        hidden: false,
        is_current: false,
    };
    app.graph = Some(GraphModel {
        commits: vec![Commit {
            id: "aaa".into(),
            subject: "seed".into(),
            ..Commit::default()
        }],
        worktrees: vec![
            worktree("app/.worktrees/feat", Some("feat")),
            worktree("app/.worktrees/loose", None),
        ],
        window: 1,
        ..GraphModel::default()
    });
    app.focus = FocusPane::Right;
    assert!(app.graph_pane_focused());
    let rows = app.graph.as_ref().expect("graph").visible_rows();
    let at = |pick: &dyn Fn(&GraphRow) -> bool| rows.iter().position(pick).expect("row");

    app.graph_cursor =
        at(&|row| matches!(row, GraphRow::Worktree(wt) if wt.branch.as_deref() == Some("feat")));
    assert_eq!(gx(&mut app), open("app/.worktrees/feat", "feat"));

    app.graph_cursor = at(&|row| matches!(row, GraphRow::Commit { .. }));
    assert_no_pr_for_row(&mut app, "graph commit");

    app.graph_cursor = at(&|row| matches!(row, GraphRow::Worktree(wt) if wt.branch.is_none()));
    assert_no_pr_for_row(&mut app, "detached graph worktree");
}

#[test]
fn badge_shows_only_a_ready_pr_for_the_current_branch() {
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let path = Path::new("app");
    assert_eq!(
        app.due_pr_lookups(),
        vec![(PathBuf::from("app"), "feature".to_string())]
    );
    assert_eq!(app.pr_badge(path), None, "in flight");

    for state in [PrState::InReview, PrState::Ready, PrState::Merged] {
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(state)));
        assert_eq!(app.pr_badge(path), Some(state));
    }
    assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::NoPr));
    assert_eq!(app.pr_badge(path), None, "no PR");
    assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::Failed(None)));
    assert_eq!(app.pr_badge(path), None, "failed lookup after no PR");
    assert!(app.status.is_empty(), "a background failure stays silent");
    assert!(app.apply_pr_lookup(path, "feature", None, PrLookup::NoPr));
    assert_eq!(app.pr_badge(path), None, "no remote");
}

#[test]
fn commit_link_needs_a_resolved_remote_and_a_full_id() {
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    assert_eq!(app.commit_link(path, SHA), None, "lookup in flight");
    assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::NoPr));
    assert_eq!(
        app.commit_link(path, SHA).as_deref(),
        Some("https://github.com/octo/demo/commit/0123456789abcdef0123456789abcdef01234567")
    );
    let sha256 = "ab".repeat(32);
    assert!(app.commit_link(path, &sha256).is_some(), "SHA-256 id");
    for short in ["0123456", &SHA[..39], "HEAD", &"g".repeat(40)] {
        assert_eq!(app.commit_link(path, short), None, "{short}");
    }
}

#[test]
fn a_lookup_for_an_old_branch_is_dropped_and_one_after_r_lands() {
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
    assert!(!app.apply_pr_lookup(
        path,
        "feature",
        Some(REMOTE.into()),
        found(PrState::InReview)
    ));
    assert_eq!(app.pr_badge(path), None);
    assert_eq!(
        app.due_pr_lookups(),
        vec![(PathBuf::from("app"), "other".to_string())],
        "the changed branch is looked up"
    );

    // `r` keeps the record of the current branch: an answer for it
    // lands, one for the old branch is still dropped.
    app.forget_pr_lookups(Some(path));
    assert!(app.apply_pr_lookup(path, "other", Some(REMOTE.into()), found(PrState::InReview)));
    assert_eq!(app.pr_badge(path), Some(PrState::InReview));
    assert!(!app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(PrState::Ready)));
    assert_eq!(app.pr_badge(path), Some(PrState::InReview));
}

#[test]
fn a_cached_answer_is_hidden_once_the_checkout_leaves_its_branch() {
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    assert!(app.apply_pr_lookup(
        path,
        "feature",
        Some(REMOTE.into()),
        found(PrState::InReview)
    ));
    apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
    assert_eq!(app.pr_badge(path), None, "answer is for another branch");
}

#[test]
fn due_lookups_skip_detached_and_hidden_and_drop_vanished_checkouts() {
    let mut app = app_with(
        &[
            repo("app", "feature"),
            repo("det", DETACHED_HEAD_BRANCH),
            repo("vendor", "main"),
        ],
        &["vendor"],
    );
    assert_eq!(
        app.due_pr_lookups(),
        vec![(PathBuf::from("app"), "feature".to_string())]
    );
    assert!(app.due_pr_lookups().is_empty(), "same branch again: none");

    let path = Path::new("app");
    assert!(app.apply_pr_lookup(
        path,
        "feature",
        Some(REMOTE.into()),
        found(PrState::InReview)
    ));
    apply_one_repo_snapshot(&mut app, "app", None);
    assert!(app.due_pr_lookups().is_empty());
    assert!(
        app.pr_cache.checkouts.is_empty(),
        "vanished checkout dropped"
    );
    assert!(app.pr_cache.answers.is_empty(), "its answer too");
}

/// `row` with `default` as its repository's known default branch.
fn with_default(mut row: RepoSnapshot, default: &str) -> RepoSnapshot {
    row.default_branch = Some(default.into());
    row
}

/// Sorted `(checkout, branch)` pairs of `targets`, for comparison.
fn names(targets: Vec<(PathBuf, String)>) -> Vec<(String, String)> {
    let mut names: Vec<(String, String)> = targets
        .into_iter()
        .map(|(path, branch)| (path.display().to_string(), branch))
        .collect();
    names.sort();
    names
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(name, branch)| (name.to_string(), branch.to_string()))
        .collect()
}

#[test]
fn a_checkout_on_its_default_branch_is_never_looked_up_or_polled() {
    let mut app = app_with(
        &[
            with_default(pushed_on("app", "main"), "main"),
            with_default(pushed_on("dev", "develop"), "main"),
            pushed_on("guess", "main"),
            with_default(pushed("feat"), "main"),
        ],
        &[],
    );
    let lookups = pairs(&[("dev", "develop"), ("feat", "feature"), ("guess", "main")]);
    assert_eq!(
        names(app.due_pr_lookups()),
        lookups,
        "not the default branch; `develop` is not the default here, and an \
             unknown default branch is looked up as before"
    );
    let t0 = Instant::now();
    for (name, branch) in [
        ("app", "main"),
        ("dev", "develop"),
        ("guess", "main"),
        ("feat", "feature"),
    ] {
        // The default branch has no record, so its answer never lands.
        let landed = app.apply_pr_lookup_at(
            Path::new(name),
            branch,
            Some(gh(name)),
            open_on(&gh(name), 1),
            t0,
        );
        assert_eq!(landed, name != "app", "{name} lands");
    }
    let Effect::LookupPullRequests { targets } = app.refresh_pull_requests(None) else {
        panic!("Refresh PRs looks up the other checkouts");
    };
    assert_eq!(names(targets), lookups, "Refresh PRs");
    for (name, branch) in [("dev", "develop"), ("guess", "main"), ("feat", "feature")] {
        let path = Path::new(name);
        let lookup = open_on(&gh(name), 1);
        assert!(app.apply_pr_lookup_at(path, branch, Some(gh(name)), lookup, t0));
    }
    assert_eq!(
        names(app.due_pr_polls(t0 + secs(3600))),
        lookups,
        "a poll cycle"
    );

    app.forget_pr_lookups(None);
    app.forget_pr_lookups(Some(Path::new("app")));
    assert!(
        !names(app.due_pr_lookups()).contains(&("app".to_string(), "main".to_string())),
        "`r` and a full reload"
    );
}

#[test]
fn a_default_branch_checkout_shows_no_badge_total_section_or_prs_tab_row() {
    let mut app = app_with(
        &[
            with_default(pushed_on("app", "main"), "main"),
            with_default(pushed("feat"), "main"),
        ],
        &[],
    );
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    let main = Path::new("app");
    let _ = app.apply_pr_lookup_at(main, "main", Some(gh("app")), open_on(&gh("app"), 1), t0);
    land_pr(&mut app, "feat", &gh("feat"), open_on(&gh("feat"), 2), t0);

    assert_eq!(app.pr_badge(main), None, "badge");
    assert_eq!(app.tree_pr_badge(main, 120, t0), None, "tree badge");
    assert_eq!(app.pull_request_for(main), None, "graph header chip");
    assert_eq!(app.pr_totals().prs, 1, "the workspace total");
    assert_eq!(
        app.prs_listed_checkouts(),
        vec![PathBuf::from("feat")],
        "the PRs tab"
    );
    assert_eq!(app.pr_lookup_progress(), (1, 1), "the PRs tab loading line");
    let target = (
        pr_badge_kind(PrState::InReview),
        IconTarget::PullRequest(main.to_path_buf()),
    );
    assert!(
        popover_sections(&app, &[target], 0).is_empty(),
        "the popover PR section"
    );

    let worktree = |path: &str, branch: &str| Worktree {
        path: path.into(),
        head_id: None,
        branch: Some(branch.into()),
        hidden: false,
        is_current: false,
    };
    app.graph = Some(GraphModel {
        commits: vec![Commit {
            id: "aaa".into(),
            subject: "seed".into(),
            ..Commit::default()
        }],
        worktrees: vec![worktree("app", "main"), worktree("feat", "feature")],
        window: 1,
        ..GraphModel::default()
    });
    let graph: Vec<PathBuf> = app
        .graph_pr_badges()
        .into_iter()
        .map(|(_, path, _)| path)
        .collect();
    assert_eq!(graph, vec![PathBuf::from("feat")], "graph worktree badges");
}

#[test]
fn gx_and_gc_on_the_default_branch_say_it_has_no_pr() {
    let mut app = app_with(
        &[
            with_default(repo("app", "main"), "main"),
            with_default(linked("app/.worktrees/feat", "app", "feat"), "main"),
        ],
        &[],
    );
    let refused = |app: &mut AppState, action: Action, what: &str| {
        app.status = StatusMessage::default();
        assert_eq!(app.dispatch(action), Effect::None, "{what}");
        assert_eq!(app.status, "main is the default branch: no PR", "{what}");
        assert_eq!(app.status.kind(), StatusKind::Info, "{what}");
    };
    focus(&mut app, "checkout:app");
    refused(&mut app, Action::OpenPullRequest, "gx");
    refused(&mut app, Action::OpenPullRequestCi, "gc");
    refused(&mut app, Action::RefreshPullRequest, "Refresh this PR");
    refused(&mut app, Action::CopyPullRequestUrl, "Copy PR URL");
    focus(&mut app, "checkout:app/.worktrees/feat");
    assert_eq!(gx(&mut app), open("app/.worktrees/feat", "feat"));

    // A graph worktree row on the default branch too.
    focus(&mut app, "checkout:app/.worktrees/feat");
    app.graph = Some(GraphModel {
        commits: vec![Commit {
            id: "aaa".into(),
            subject: "seed".into(),
            ..Commit::default()
        }],
        worktrees: vec![Worktree {
            path: "app".into(),
            head_id: None,
            branch: Some("main".into()),
            hidden: false,
            is_current: false,
        }],
        window: 1,
        ..GraphModel::default()
    });
    app.focus = FocusPane::Right;
    let rows = app.graph.as_ref().expect("graph").visible_rows();
    app.graph_cursor = rows
        .iter()
        .position(|row| matches!(row, GraphRow::Worktree(_)))
        .expect("worktree row");
    refused(
        &mut app,
        Action::OpenPullRequest,
        "gx on a graph worktree row",
    );
}

#[test]
fn a_switch_onto_the_default_branch_drops_the_pr_and_a_switch_away_looks_up() {
    let mut app = app_with(&[with_default(pushed("app"), "main")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    land_pr(
        &mut app,
        "app",
        &gh("app"),
        open_on(&gh("app"), 7),
        Instant::now(),
    );
    assert_eq!(app.pr_badge(path), Some(PrState::InReview));

    let on_main = with_default(pushed_on("app", "main"), "main");
    apply_one_repo_snapshot(&mut app, "app", Some(on_main));
    assert!(app.due_pr_lookups().is_empty(), "no lookup for main");
    assert_eq!(app.pr_badge(path), None, "the badge goes");
    assert!(app.pr_cache.checkouts.is_empty(), "the record goes");
    assert!(app.pr_cache.answers.is_empty(), "its answer too");
    assert!(app.take_pr_transitions().is_empty(), "no alert");

    apply_one_repo_snapshot(&mut app, "app", Some(with_default(pushed("app"), "main")));
    assert_eq!(
        app.due_pr_lookups(),
        vec![(PathBuf::from("app"), "feature".to_string())],
        "a switch away looks up"
    );
    land_pr(
        &mut app,
        "app",
        &gh("app"),
        open_on(&gh("app"), 7),
        Instant::now(),
    );
    assert_eq!(app.pr_badge(path), Some(PrState::InReview));
    assert!(app.take_pr_transitions().is_empty(), "no alert");
}

#[test]
fn a_default_branch_checkout_resolves_only_its_remote_for_commit_links() {
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    let mut app = app_with(&[with_default(repo("app", "main"), "main")], &[]);
    let path = Path::new("app");
    let main = vec![(PathBuf::from("app"), "main".to_string())];
    assert!(app.due_pr_lookups().is_empty(), "no PR lookup");
    assert_eq!(app.due_remote_lookups(), main, "a remote resolve");
    assert!(app.due_remote_lookups().is_empty(), "once per branch");
    assert_eq!(app.commit_link(path, SHA), None, "resolve in flight");
    assert!(app.apply_checkout_remote(path, "main", Some(REMOTE.into())));
    let link = Some(format!("https://github.com/octo/demo/commit/{SHA}"));
    assert_eq!(app.commit_link(path, SHA), link);
    assert_eq!(app.pr_badge(path), None, "still no PR status");
    assert_eq!(app.pr_totals().prs, 0);

    app.forget_pr_lookups(Some(path));
    assert_eq!(app.due_remote_lookups(), main, "`r` resolves again");
    assert_eq!(app.commit_link(path, SHA), link, "the old remote stays");

    apply_one_repo_snapshot(
        &mut app,
        "app",
        Some(with_default(repo("app", "x"), "main")),
    );
    assert!(app.due_remote_lookups().is_empty());
    assert!(
        !app.apply_checkout_remote(path, "main", Some(REMOTE.into())),
        "a late resolve for the old branch is dropped"
    );
    assert_eq!(
        app.due_pr_lookups(),
        vec![(PathBuf::from("app"), "x".to_string())],
        "off the default branch the PR lookup runs again"
    );
    assert_eq!(app.commit_link(path, SHA), None, "until it lands");
}

#[test]
fn open_result_sets_status_and_refreshes_the_cache() {
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let path = Path::new("app");
    app.apply_pr_open(
        path,
        "feature",
        Some(REMOTE.into()),
        found(PrState::Merged),
        PrPage::Pr,
        true,
    );
    assert_eq!(app.status, "opened PR #7");
    assert_eq!(app.status.kind(), StatusKind::Ok);
    assert_eq!(app.pr_badge(path), Some(PrState::Merged));
    assert!(
        app.due_pr_lookups().is_empty(),
        "the open answered this branch"
    );

    app.apply_pr_open(
        path,
        "feature",
        Some(REMOTE.into()),
        PrLookup::NoPr,
        PrPage::Pr,
        false,
    );
    assert_eq!(app.status, "no PR for feature");
    assert_eq!(app.pr_badge(path), None, "fresh answer replaces the badge");
}

/// Screen row of tree row `id` with the default layout (no scroll).
fn tree_y(app: &AppState, id: &str) -> u16 {
    let index = app
        .rows
        .iter()
        .position(|row| row.id == id)
        .unwrap_or_else(|| panic!("missing row {id}"));
    app.layout.tree_y + u16::try_from(index).expect("small tree")
}

fn badge_at(app: &mut AppState, repo: &str, x: u16, y: u16) {
    app.layout.icon_hits = vec![IconHit {
        y,
        x,
        width: 2,
        kind: IconKind::PrInReview,
        target: IconTarget::PullRequest(PathBuf::from(repo)),
    }];
}

fn focused_id(app: &AppState) -> String {
    app.focused_row().expect("focused row").id.clone()
}

#[test]
fn ctrl_click_on_a_badge_selects_the_row_and_opens_its_pr() {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    focus(&mut app, "repo:lib");

    for col in [20, 21] {
        focus(&mut app, "repo:lib");
        app.last_click = None;
        assert_eq!(
            app.dispatch(Action::CtrlClick { col, row: y }),
            Effect::Batch(vec![Effect::LoadRightPane, open("app", "feature")]),
            "col {col}"
        );
        assert_eq!(focused_id(&app), "repo:app", "col {col}");
        assert_eq!(app.focus, FocusPane::Left);
    }
}

#[test]
fn ctrl_click_on_a_stale_badge_of_a_default_branch_checkout_opens_nothing() {
    // The badge was painted before the default branch became known.
    let mut app = app_with(
        &[
            with_default(repo("app", "main"), "main"),
            repo("lib", "feature"),
        ],
        &[],
    );
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    focus(&mut app, "repo:lib");
    app.last_click = None;
    assert_eq!(
        app.dispatch(Action::CtrlClick { col: 20, row: y }),
        Effect::LoadRightPane,
        "the row is selected, no PR opens"
    );
    assert_eq!(focused_id(&app), "repo:app");
    assert_eq!(app.status, "main is the default branch: no PR");
}

#[test]
fn a_second_ctrl_click_on_a_badge_opens_again_without_drilling() {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    focus(&mut app, "repo:lib");
    app.last_click = None;
    // Control: two plain clicks in the window are a double-click, which
    // moves focus to the right pane. They land off the badge: a plain
    // click on an icon pins its popover and never drills.
    app.dispatch(Action::Click { col: 10, row: y });
    app.dispatch(Action::Click { col: 10, row: y });
    assert_eq!(app.focus, FocusPane::Right);

    focus(&mut app, "repo:lib");
    app.focus = FocusPane::Left;
    app.last_click = None;
    let first = app.dispatch(Action::CtrlClick { col: 20, row: y });
    assert!(
        app.last_click.is_none(),
        "an opening Ctrl+click arms no double-click"
    );
    let second = app.dispatch(Action::CtrlClick { col: 20, row: y });
    let single = Effect::Batch(vec![Effect::LoadRightPane, open("app", "feature")]);
    assert_eq!(first, single);
    assert_eq!(second, single, "a single click again, not a double-click");
    assert_eq!(app.focus, FocusPane::Left, "no drill");
    assert_eq!(focused_id(&app), "repo:app");
}

#[test]
fn ctrl_click_off_the_badge_is_a_plain_click() {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    for col in [10, 19, 22] {
        focus(&mut app, "repo:lib");
        app.last_click = None;
        let ctrl = app.dispatch(Action::CtrlClick { col, row: y });
        let ctrl_focus = focused_id(&app);
        focus(&mut app, "repo:lib");
        app.last_click = None;
        let plain = app.dispatch(Action::Click { col, row: y });
        assert_eq!(ctrl, Effect::LoadRightPane, "col {col}");
        assert_eq!(ctrl, plain, "col {col}");
        assert_eq!(ctrl_focus, focused_id(&app), "col {col}");
        assert_eq!(ctrl_focus, "repo:app", "col {col}");
    }
    // The badge on another row does not open from this row.
    let lib_y = tree_y(&app, "repo:lib");
    app.last_click = None;
    assert_eq!(
        app.dispatch(Action::CtrlClick {
            col: 20,
            row: lib_y
        }),
        Effect::LoadRightPane
    );
    assert!(app.status.is_empty());
}

#[test]
fn plain_click_on_a_badge_only_selects() {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    focus(&mut app, "repo:lib");
    assert_eq!(
        app.dispatch(Action::Click { col: 20, row: y }),
        Effect::LoadRightPane
    );
    assert_eq!(focused_id(&app), "repo:app");
}

#[test]
fn ctrl_click_with_the_mouse_off_does_nothing() {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    focus(&mut app, "repo:lib");
    app.mouse_enabled = false;
    assert_eq!(
        app.dispatch(Action::CtrlClick { col: 20, row: y }),
        Effect::None
    );
    assert_eq!(focused_id(&app), "repo:lib");
}

#[test]
fn ctrl_click_on_a_badge_without_a_branch_says_no_pr_for_this_row() {
    let mut app = app_with(
        &[repo("app", DETACHED_HEAD_BRANCH), repo("lib", "main")],
        &[],
    );
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    focus(&mut app, "repo:lib");
    assert_eq!(
        app.dispatch(Action::CtrlClick { col: 20, row: y }),
        Effect::LoadRightPane
    );
    assert_eq!(focused_id(&app), "repo:app");
    assert_eq!(app.status, NO_PR_FOR_ROW);
    assert_eq!(app.status.kind(), StatusKind::Warn);
}

// ---- PR detail for the popover ----

use super::super::super::popover::PEEK_DWELL_MS;
use super::super::super::pr_status::{checks_ci_link, ChecksSummary, PrNotes};
use super::super::super::pull_request::PullRequestDetail;

/// `app` on `feature` with a ready open PR #`number` in the badge cache.
fn app_with_pr(number: u64) -> AppState {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let _ = app.due_pr_lookups();
    set_answer(&mut app, number, PrState::InReview);
    app
}

fn set_answer(app: &mut AppState, number: u64, state: PrState) {
    let pr = PullRequest::fixture(
        number,
        &format!("https://github.com/octo/demo/pull/{number}"),
        state,
    );
    assert!(app.apply_pr_lookup(
        Path::new("app"),
        "feature",
        Some(REMOTE.into()),
        PrLookup::Found(pr)
    ));
}

fn fetch(number: u64, request: u64) -> Effect {
    Effect::LookupPullRequestDetail {
        repo: PathBuf::from("app"),
        branch: "feature".into(),
        remote: REMOTE.into(),
        number,
        request,
    }
}

fn pr_detail(number: u64) -> PrDetailLookup {
    PrDetailLookup::Found(PullRequestDetail {
        number,
        title: "Add login".into(),
        state: "OPEN".into(),
        draft: false,
        author: Some("octocat".into()),
        head: Some("feature".into()),
        base: Some("main".into()),
        review: None,
        reviews: None,
        checks: ChecksSummary::default(),
        pipeline: None,
        check_names: Vec::new(),
        more_checks: 0,
        notes: PrNotes::default(),
        ci_link: checks_ci_link(&[], &format!("https://github.com/octo/demo/pull/{number}")),
        updated_at: None,
    })
}

/// Land the answer to the request the cache waits for, a first fetch or
/// a new fetch (0 when it waits for none).
fn land(app: &mut AppState, branch: &str, number: u64, lookup: PrDetailLookup) -> bool {
    let first = app
        .pr_cache
        .details
        .values()
        .find_map(|detail| match detail {
            PrDetailState::Loading(request) => Some(*request),
            _ => None,
        });
    let again = app
        .pr_cache
        .detail_refresh
        .values()
        .find_map(|refresh| match refresh {
            DetailRefresh::Loading { request, .. } => Some(*request),
            DetailRefresh::Due => None,
        });
    land_request(app, branch, number, first.or(again).unwrap_or(0), lookup)
}

fn land_request(
    app: &mut AppState,
    branch: &str,
    number: u64,
    request: u64,
    lookup: PrDetailLookup,
) -> bool {
    app.apply_pr_detail(Path::new("app"), branch, REMOTE, number, request, lookup)
}

fn detail_of(app: &AppState) -> Option<PrDetailState> {
    app.pull_request_detail_for(Path::new("app")).cloned()
}

/// Pin the PR badge popover of `app` with a click on its badge.
fn click_badge(app: &mut AppState) -> Effect {
    let y = tree_y(app, "repo:app");
    badge_at(app, "app", 20, y);
    app.close_popover();
    app.dispatch(Action::Click { col: 20, row: y })
}

#[test]
fn a_peek_or_pin_requests_the_detail_once_and_later_ones_read_the_cache() {
    let mut app = app_with_pr(7);
    assert_eq!(detail_of(&app), None, "nothing asked before a popover");

    // A peek that opens on the badge asks once.
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    let at = std::time::Instant::now();
    app.set_pointer_at(Some((20, y)), at);
    assert_eq!(
        app.expire_peek_with_details(at + std::time::Duration::from_millis(PEEK_DWELL_MS)),
        (true, fetch(7, 1))
    );
    assert!(app.popover.is_some(), "peek open");
    assert_eq!(detail_of(&app), Some(PrDetailState::Loading(1)));
    assert_eq!(
        app.request_pr_details(),
        Effect::None,
        "a later peek reads the cache"
    );

    // Pinning the same PR while it loads asks nothing.
    assert_eq!(click_badge(&mut app), Effect::LoadRightPane);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
    assert_eq!(
        click_badge(&mut app),
        Effect::LoadRightPane,
        "ready: cached"
    );

    // A failure is cached too; it is not retried on the next pin.
    app.forget_pr_lookups(Some(Path::new("app")));
    let _ = app.due_pr_lookups();
    set_answer(&mut app, 7, PrState::InReview);
    assert_eq!(
        click_badge(&mut app),
        Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)]),
        "a pin on a fresh answer asks"
    );
    assert!(land(&mut app, "feature", 7, PrDetailLookup::Failed(None)));
    assert_eq!(detail_of(&app), Some(PrDetailState::Failed));
    assert_eq!(click_badge(&mut app), Effect::LoadRightPane);
}

#[test]
fn a_second_peek_dwell_on_the_badge_asks_nothing() {
    use super::super::super::popover::PEEK_GRACE_MS;
    use std::time::{Duration, Instant};
    let mut app = app_with_pr(7);
    let y = tree_y(&app, "repo:app");
    badge_at(&mut app, "app", 20, y);
    let dwell = Duration::from_millis(PEEK_DWELL_MS);
    let t0 = Instant::now();
    app.set_pointer_at(Some((20, y)), t0);
    assert_eq!(
        app.expire_peek_with_details(t0 + dwell),
        (true, fetch(7, 1)),
        "the first dwell opens the peek and asks once"
    );
    // Leave: the grace closes the peek and asks nothing.
    let t1 = t0 + dwell;
    app.set_pointer_at(None, t1);
    assert_eq!(
        app.expire_peek_with_details(t1 + Duration::from_millis(PEEK_GRACE_MS)),
        (true, Effect::None)
    );
    assert!(app.popover.is_none());
    // Back on the badge: a second peek reads the cache.
    let t2 = t1 + Duration::from_secs(1);
    app.set_pointer_at(Some((20, y)), t2);
    assert_eq!(
        app.expire_peek_with_details(t2 + dwell),
        (true, Effect::None)
    );
    assert!(app.popover.is_some(), "second peek open");
    assert_eq!(
        app.expire_peek_with_details(t2 + dwell),
        (false, Effect::None),
        "nothing due"
    );
}

#[test]
fn a_late_answer_to_an_older_request_loses_to_the_newer_one() {
    let mut app = app_with_pr(7);
    assert_eq!(
        click_badge(&mut app),
        Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 1)])
    );
    // `r` while request 1 hangs, then a pin asks again.
    app.forget_pr_lookups(Some(Path::new("app")));
    let _ = app.due_pr_lookups();
    set_answer(&mut app, 7, PrState::InReview);
    assert_eq!(
        click_badge(&mut app),
        Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)])
    );
    assert!(
        !land_request(&mut app, "feature", 7, 1, PrDetailLookup::Failed(None)),
        "the old request times out late: dropped"
    );
    assert_eq!(detail_of(&app), Some(PrDetailState::Loading(2)));
    assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
    assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
    assert!(
        !land_request(&mut app, "feature", 7, 1, PrDetailLookup::Failed(None)),
        "and never replaces the fresh one"
    );
}

#[test]
fn no_detail_is_requested_without_a_pr_answer() {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let _ = app.due_pr_lookups();
    assert_eq!(click_badge(&mut app), Effect::LoadRightPane, "in flight");
    assert!(app.apply_pr_lookup(
        Path::new("app"),
        "feature",
        Some(REMOTE.into()),
        PrLookup::NoPr
    ));
    assert_eq!(click_badge(&mut app), Effect::LoadRightPane, "no PR");
    assert!(app.pr_cache.details.is_empty());
}

#[test]
fn a_branch_switch_r_and_another_pr_drop_the_detail() {
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));

    // `gx` with the same PR keeps the detail, even when its answer
    // changed; another PR drops it.
    app.apply_pr_open(
        Path::new("app"),
        "feature",
        Some(REMOTE.into()),
        found(PrState::InReview),
        PrPage::Pr,
        true,
    );
    assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
    assert!(app.pr_cache.detail_refresh.is_empty(), "same answer");
    app.apply_pr_open(
        Path::new("app"),
        "feature",
        Some(REMOTE.into()),
        found(PrState::Ready),
        PrPage::Pr,
        true,
    );
    assert!(
        matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
        "the same PR changed under gx: the detail stays"
    );
    set_answer(&mut app, 8, PrState::Ready);
    assert_eq!(detail_of(&app), None, "another PR under gx");
    assert!(app.pr_cache.details.is_empty());
    set_answer(&mut app, 7, PrState::Ready);
    assert_eq!(
        click_badge(&mut app),
        Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)])
    );

    // `r` forgets the checkout and its detail.
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    app.forget_pr_lookups(Some(Path::new("app")));
    assert!(app.pr_cache.details.is_empty(), "r drops the detail");
    let _ = app.due_pr_lookups();
    set_answer(&mut app, 7, PrState::InReview);
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    app.forget_pr_lookups(None);
    assert!(app.pr_cache.details.is_empty(), "a full reload drops all");

    // A branch switch drops the old branch's detail.
    let _ = app.due_pr_lookups();
    set_answer(&mut app, 7, PrState::InReview);
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
    assert_eq!(detail_of(&app), None, "hidden at once");
    let _ = app.due_pr_lookups();
    assert!(app.pr_cache.details.is_empty(), "pruned with the answer");
}

#[test]
fn a_late_detail_for_an_old_branch_or_a_forgotten_checkout_is_dropped() {
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
    assert!(!land(&mut app, "feature", 7, pr_detail(7)), "old branch");

    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    app.forget_pr_lookups(Some(Path::new("app")));
    assert!(!land(&mut app, "feature", 7, pr_detail(7)), "forgotten");
    assert_eq!(detail_of(&app), None);

    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    set_answer(&mut app, 8, PrState::InReview);
    assert!(
        !land(&mut app, "feature", 7, pr_detail(7)),
        "another PR now"
    );
    assert_eq!(detail_of(&app), None);

    let mut app = app_with_pr(7);
    assert!(!land(&mut app, "feature", 7, pr_detail(7)), "never asked");
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    assert!(
        !land(&mut app, "feature", 7, PrDetailLookup::Failed(None)),
        "a second answer does not replace the first"
    );
    assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
}

#[test]
fn the_pinned_focus_stays_on_its_line_when_the_detail_lands() {
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    assert_eq!(
        app.popover_run_action(),
        Some(Action::OpenPullRequest),
        "lands on Open PR below the loading line"
    );
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    assert_eq!(app.popover_run_action(), Some(Action::OpenPullRequest));

    // A field keeps the focus too: the URL moves down past the new
    // fields and `y` still copies it.
    let copy_url = Effect::CopyClipboard {
        text: "https://github.com/octo/demo/pull/7".into(),
        announce: true,
    };
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    app.dispatch(Action::PopoverMove(-1));
    assert_eq!(app.dispatch(Action::PopoverCopyLine), copy_url);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    assert_eq!(app.dispatch(Action::PopoverCopyLine), copy_url);
}

#[test]
fn a_release_after_the_detail_lands_runs_the_pressed_line() {
    use crate::tui::popover::{flat_lines, PopoverLine};
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
    terminal
        .draw(|frame| crate::tui::render::draw(frame, &mut app))
        .unwrap();
    let painted = app.layout.popover.clone().expect("painted");
    let sections = app.open_popover_sections();
    let lines = flat_lines(&sections);
    let (y, index) = *painted
        .lines
        .iter()
        .find(|(_, index)| {
            matches!(lines[*index], PopoverLine::Action { command, .. }
                    if command.action == Action::OpenPullRequest)
        })
        .expect("Open PR line");
    app.dispatch(Action::Click {
        col: painted.inner.x + 2,
        row: y,
    });
    assert_eq!(app.popover_release_action(), Some(Action::OpenPullRequest));

    // The detail lands between press and release: the lines move.
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    let sections = app.open_popover_sections();
    assert!(
        !matches!(flat_lines(&sections)[index], PopoverLine::Action { command, .. }
                if command.action == Action::OpenPullRequest),
        "the pressed index now holds another line"
    );
    assert_eq!(
        app.popover_release_action(),
        Some(Action::OpenPullRequest),
        "the press follows its line"
    );
    let mut twin = app.clone();
    twin.close_popover();
    let want = twin.dispatch(Action::OpenPullRequest);
    assert_eq!(app.dispatch(Action::Release), want);
    assert!(app.popover.is_none());
}

#[test]
fn watch_ticks_never_request_a_detail() {
    let mut app = app_with_pr(7);
    focus(&mut app, "repo:app");
    let _ = click_badge(&mut app);
    assert!(app.popover_pinned());
    // Another PR drops the detail while the popover stays pinned.
    set_answer(&mut app, 8, PrState::Ready);
    assert_eq!(detail_of(&app), None);
    for tick in [Action::WatchTick, Action::FetchTick] {
        let effect = app.dispatch(tick.clone());
        assert!(
            !format!("{effect:?}").contains("LookupPullRequestDetail"),
            "{tick:?}: {effect:?}"
        );
    }
    assert!(app.pr_cache.details.is_empty());
    let _ = app.due_pr_lookups();
    assert!(
        app.pr_cache.details.is_empty(),
        "a status apply asks nothing"
    );
}

#[test]
fn palette_open_pr_closes_the_palette_and_opens_the_focused_branch() {
    use super::super::super::action::QuickOpenEntry;
    use super::super::super::command_palette::PALETTE_COMMANDS;
    let mut app = app_with(&[repo("app", "feature")], &[]);
    focus(&mut app, "repo:app");
    let command = PALETTE_COMMANDS
        .iter()
        .find(|command| command.title == "Open PR")
        .expect("Open PR row");
    assert_eq!(app.palette_disabled_reason(command), None);

    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
    for c in "open pr".chars() {
        app.dispatch(Action::QuickOpenChar(c));
    }
    let selected = app
        .command_palette()
        .and_then(|palette| palette.selected())
        .map(|command| command.title);
    assert_eq!(selected, Some("Open PR"));
    assert_eq!(
        app.dispatch(Action::QuickOpenSubmit),
        open("app", "feature")
    );
    assert!(app.quick_open.is_none(), "submit closes the palette");
}

/// `pull` also finds Open PR by its alias. On a file row Pull behind is
/// disabled and Open PR is enabled, but the cursor stays on Pull behind,
/// so Enter keeps the palette open instead of running `gx`.
#[test]
fn palette_pull_on_a_file_row_lands_on_disabled_pull_not_open_pr() {
    use super::super::super::action::QuickOpenEntry;
    let mut app = app_with(&[repo("app", "feature")], &[]);
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("file row");
    let selected = |app: &mut AppState, query: &str| {
        app.quick_open = None;
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in query.chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        app.command_palette()
            .and_then(|palette| palette.selected())
            .map(|command| command.title)
    };
    assert_eq!(selected(&mut app, "pull"), Some("Pull behind"));
    assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
    assert!(
        app.command_palette().is_some(),
        "disabled row keeps it open"
    );
    assert_eq!(selected(&mut app, "pull request"), Some("Open PR"));
    assert_eq!(selected(&mut app, "merge request"), Some("Open PR"));
}

// ---- polling, backoff, stale state, transitions ----

use std::time::{Duration, Instant};

use super::super::super::pr_poll::PR_POLL_CALM_MS;

const GH_REMOTE: &str = "git@github.com:acme/app.git";
const GL_REMOTE: &str = "git@gitlab.com:acme/services/api.git";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Checkout `name` on `branch`, pushed (it has an upstream).
fn pushed_on(name: &str, branch: &str) -> RepoSnapshot {
    let mut row = repo(name, branch);
    row.sync_status = SyncStatus::UpToDate;
    row
}

fn pushed(name: &str) -> RepoSnapshot {
    pushed_on(name, "feature")
}

/// GitHub remote of repository `acme/<name>`. Answers are cached per
/// (remote URL, branch), so each checkout of a test needs its own.
fn gh(name: &str) -> String {
    format!("git@github.com:acme/{name}.git")
}

/// GitLab remote of project `acme/services/<name>`.
fn gl(name: &str) -> String {
    format!("git@gitlab.com:acme/services/{name}.git")
}

/// PR `number` in `state` on the forge of `remote`.
fn pr_on(remote: &str, number: u64, state: PrState) -> PullRequest {
    let url = if remote.contains("gitlab") {
        format!("https://gitlab.com/acme/services/api/-/merge_requests/{number}")
    } else {
        format!("https://github.com/acme/app/pull/{number}")
    };
    PullRequest::fixture(number, &url, state)
}

/// [`pr_on`] while the forge still computes its merge facts.
fn computing(remote: &str, number: u64, state: PrState) -> PullRequest {
    PullRequest {
        merge_state_settled: false,
        ..pr_on(remote, number, state)
    }
}

fn open_on(remote: &str, number: u64) -> PrLookup {
    PrLookup::Found(pr_on(remote, number, PrState::InReview))
}

/// Land `lookup` of `feature` for checkout `name` at `remote` at `now`.
fn land_pr(app: &mut AppState, name: &str, remote: &str, lookup: PrLookup, now: Instant) {
    assert!(
        app.apply_pr_lookup_at(Path::new(name), "feature", Some(remote.into()), lookup, now),
        "{name} lands"
    );
}

/// Checkouts the poll cycle at `now` looks up, by name.
fn polled(app: &mut AppState, now: Instant) -> Vec<String> {
    let mut names: Vec<String> = app
        .due_pr_polls(now)
        .into_iter()
        .map(|(path, branch)| {
            assert_eq!(branch, "feature");
            path.display().to_string()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn pollable_checkouts_show_an_open_pr_or_a_pushed_branch_with_no_pr_yet() {
    let mut app = app_with(
        &[
            repo("open", "feature"),
            pushed("lab"),
            pushed("merged"),
            pushed("nopr"),
            repo("local", "feature"),
            pushed("failed"),
            pushed("other"),
            pushed("pending"),
            repo("det", DETACHED_HEAD_BRANCH),
            pushed("vendor"),
        ],
        &["vendor"],
    );
    app.show_hidden = true;
    assert_eq!(app.due_pr_lookups().len(), 9, "every checkout on a branch");
    let t0 = Instant::now();
    assert!(polled(&mut app, t0).is_empty(), "no remote resolved yet");
    let remote = |name: &str| match name {
        "lab" | "failed" => gl(name),
        "other" => "https://git.example.org/acme/other.git".to_string(),
        _ => gh(name),
    };
    let answer = |name: &str| match name {
        "open" | "vendor" => open_on(&remote(name), 1),
        "lab" => PrLookup::Found(pr_on(&remote(name), 2, PrState::Draft)),
        "merged" => PrLookup::Found(pr_on(&remote(name), 3, PrState::Merged)),
        "failed" => PrLookup::Failed(None),
        _ => PrLookup::NoPr,
    };
    for name in [
        "open", "lab", "merged", "nopr", "local", "failed", "other", "vendor",
    ] {
        land_pr(&mut app, name, &remote(name), answer(name), t0);
    }

    let t1 = t0 + secs(180);
    let due = polled(&mut app, t1);
    assert_eq!(
        due,
        ["failed", "lab", "nopr", "open", "vendor"],
        "open PRs (with or without an upstream), and pushed branches with no PR \
             known; not a merged PR, an unpushed branch with no PR, another forge, \
             a lookup in flight, or a detached HEAD"
    );
    for name in &due {
        let lookup = match answer(name) {
            PrLookup::Failed(_) => PrLookup::NoPr,
            other => other,
        };
        land_pr(&mut app, name, &remote(name), lookup, t1);
    }
    app.show_hidden = false;
    assert_eq!(
        polled(&mut app, t1 + secs(180)),
        ["failed", "lab", "nopr", "open"],
        "a hidden repo polls only while hidden repos show"
    );
}

#[test]
fn a_poll_keeps_the_badge_and_skips_a_checkout_still_in_flight() {
    let mut app = app_with(&[pushed("app")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    land_pr(&mut app, "app", GH_REMOTE, open_on(GH_REMOTE, 7), t0);
    let t1 = t0 + secs(180);
    assert_eq!(polled(&mut app, t1), ["app"]);
    assert_eq!(
        app.pr_badge(path),
        Some(PrState::InReview),
        "the badge stays"
    );
    assert!(
        polled(&mut app, t1 + secs(180)).is_empty(),
        "a lookup in flight is not polled again"
    );
    assert!(
        app.due_pr_lookups().is_empty(),
        "a status apply on the same branch asks nothing"
    );
    assert_eq!(app.pr_badge(path), Some(PrState::InReview));
    let running = PrLookup::Found(pr_on(GH_REMOTE, 7, PrState::CiRunning));
    land_pr(&mut app, "app", GH_REMOTE, running, t1 + secs(2));
    assert_eq!(app.pr_badge(path), Some(PrState::CiRunning));
    assert_eq!(polled(&mut app, t1 + secs(240)), ["app"]);
}

#[test]
fn the_poll_period_is_hot_while_a_pollable_pr_runs_ci_or_is_ready() {
    let mut app = app_with(&[pushed("app"), pushed("vendor")], &["vendor"]);
    app.show_hidden = true;
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    land_pr(
        &mut app,
        "vendor",
        &gh("vendor"),
        open_on(&gh("vendor"), 2),
        t0,
    );
    for (state, want) in [
        (PrState::InReview, 180_000),
        (PrState::Draft, 180_000),
        (PrState::CiFailed, 180_000),
        (PrState::CiRunning, 60_000),
        (PrState::Ready, 60_000),
        (PrState::Merged, 180_000),
    ] {
        let lookup = PrLookup::Found(pr_on(GH_REMOTE, 1, state));
        land_pr(&mut app, "app", GH_REMOTE, lookup, t0);
        assert_eq!(app.pr_poll_interval_ms(), want, "{state:?}");
    }
    let running = PrLookup::Found(pr_on(&gh("vendor"), 2, PrState::CiRunning));
    land_pr(&mut app, "vendor", &gh("vendor"), running, t0);
    assert_eq!(app.pr_poll_interval_ms(), 60_000, "a shown hidden repo");
    app.show_hidden = false;
    // The next status apply syncs the records with the snapshot.
    assert!(app.due_pr_lookups().is_empty());
    assert_eq!(
        app.pr_poll_interval_ms(),
        180_000,
        "a hidden repo is not pollable"
    );

    app.set_pr_poll_mode(PrPollMode::Every(300));
    assert_eq!(app.pr_poll_interval_ms(), 300);
    app.set_pr_poll_mode(PrPollMode::Off);
    assert_eq!(app.pr_poll_interval_ms(), 0);
    assert!(polled(&mut app, t0 + secs(3600)).is_empty(), "polling off");
}

/// Run the poll cycle at `at` over `app` (GitHub PR 1), `lib` (GitHub
/// PR 2), and `lab` (GitLab MR 3): land each polled checkout, the GitHub
/// ones failed when `github_fails`. Returns the polled names.
fn run_cycle(app: &mut AppState, at: Instant, github_fails: bool) -> Vec<String> {
    let names = polled(app, at);
    for name in &names {
        let (remote, number) = match name.as_str() {
            "app" => (gh("app"), 1),
            "lib" => (gh("lib"), 2),
            _ => (gl("lab"), 3),
        };
        let lookup = if github_fails && remote.contains("github") {
            PrLookup::Failed(Some("HTTP 502: Bad Gateway".into()))
        } else {
            open_on(&remote, number)
        };
        land_pr(app, name, &remote, lookup, at + secs(1));
    }
    names
}

#[test]
fn a_failed_poll_doubles_its_host_period_up_to_fifteen_minutes() {
    let mut app = app_with(&[pushed("app"), pushed("lab"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    land_pr(&mut app, "app", &gh("app"), open_on(&gh("app"), 1), t0);
    land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
    land_pr(&mut app, "lab", &gl("lab"), open_on(&gl("lab"), 3), t0);
    assert_eq!(app.pr_poll_interval_ms(), PR_POLL_CALM_MS);
    // Cycle k starts k × 180 s after t0. Both GitHub checkouts fail in
    // one cycle, which counts once. github.com then waits 360 s, 720 s,
    // 900 s (the cap), 900 s; GitLab keeps 180 s. A success resets it.
    let github_polled = [1, 3, 7, 12, 17, 18];
    let github_fails = [1, 3, 7, 12];
    for k in 1..=18u64 {
        let want: &[&str] = if github_polled.contains(&k) {
            &["app", "lab", "lib"]
        } else {
            &["lab"]
        };
        let at = t0 + secs(180 * k);
        assert_eq!(
            run_cycle(&mut app, at, github_fails.contains(&k)),
            want,
            "cycle {k}"
        );
        assert_eq!(
            app.pr_badge(Path::new("app")),
            Some(PrState::InReview),
            "cycle {k}: a failed poll keeps the badge"
        );
    }
}

#[test]
fn a_failed_lookup_keeps_a_known_badge_and_records_why() {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let (path, lib) = (Path::new("app"), Path::new("lib"));
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    let reason = |app: &AppState, path| {
        app.pr_lookup_failure(path)
            .map(|failure| failure.reason.clone())
    };

    land_pr(
        &mut app,
        "lib",
        &gh("lib"),
        PrLookup::Failed(Some("gh not found".into())),
        t0,
    );
    assert_eq!(app.pr_badge(lib), None, "a failed first lookup shows none");
    assert_eq!(reason(&app, lib), Some(Some("gh not found".into())));
    assert_eq!(app.pr_checked_at(lib), None);

    let ready = PrLookup::Found(pr_on(GH_REMOTE, 1, PrState::Ready));
    land_pr(&mut app, "app", GH_REMOTE, ready, t0);
    assert_eq!(app.pr_checked_at(path), Some(t0));
    assert_eq!(reason(&app, path), None);
    land_pr(
        &mut app,
        "app",
        GH_REMOTE,
        PrLookup::Failed(None),
        t0 + secs(5),
    );
    assert_eq!(app.pr_badge(path), Some(PrState::Ready), "the badge stays");
    assert_eq!(reason(&app, path), Some(None), "failed, with no reason");
    assert_eq!(app.pr_checked_at(path), Some(t0), "the last success");

    app.apply_pr_open(
        path,
        "feature",
        Some(GH_REMOTE.into()),
        PrLookup::Failed(Some("HTTP 401: Bad credentials".into())),
        PrPage::Pr,
        false,
    );
    assert_eq!(
        app.status, "could not look up PR for feature: HTTP 401: Bad credentials",
        "gx names the reason"
    );
    assert_eq!(app.pr_badge(path), Some(PrState::Ready), "gx keeps it too");
    assert_eq!(
        reason(&app, path),
        Some(Some("HTTP 401: Bad credentials".into()))
    );
    assert!(
        app.take_pr_transitions().is_empty(),
        "a failure never alerts"
    );

    let t1 = t0 + secs(60);
    land_pr(&mut app, "app", GH_REMOTE, PrLookup::NoPr, t1);
    assert_eq!(reason(&app, path), None, "a success clears it");
    assert_eq!(app.pr_checked_at(path), Some(t1));
    assert_eq!(app.pr_badge(path), None);
}

#[test]
fn a_polled_badge_goes_stale_after_twice_the_planned_period() {
    let mut app = app_with(&[pushed("app")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    assert!(!app.pr_is_stale(path, t0 + secs(3600)), "no answer yet");
    land_pr(&mut app, "app", GH_REMOTE, open_on(GH_REMOTE, 1), t0);
    assert!(!app.pr_is_stale(path, t0 + secs(360)), "calm: 2 × 180 s");
    assert!(app.pr_is_stale(path, t0 + secs(361)));

    assert_eq!(polled(&mut app, t0 + secs(180)), ["app"]);
    let failed = PrLookup::Failed(Some("HTTP 502: Bad Gateway".into()));
    land_pr(&mut app, "app", GH_REMOTE, failed, t0 + secs(181));
    assert!(
        app.pr_is_stale(path, t0 + secs(400)),
        "a failed poll keeps the old check time"
    );
    assert_eq!(app.pr_badge(path), Some(PrState::InReview));

    let t1 = t0 + secs(500);
    let running = PrLookup::Found(pr_on(GH_REMOTE, 1, PrState::CiRunning));
    land_pr(&mut app, "app", GH_REMOTE, running, t1);
    assert!(!app.pr_is_stale(path, t1 + secs(120)), "hot: 2 × 60 s");
    assert!(app.pr_is_stale(path, t1 + secs(121)));

    app.set_pr_poll_mode(PrPollMode::Every(300));
    assert!(app.pr_is_stale(path, t1 + Duration::from_millis(601)));
    app.set_pr_poll_mode(PrPollMode::Off);
    assert!(!app.pr_is_stale(path, t1 + secs(9999)), "polling off");
    app.set_pr_poll_mode(PrPollMode::HotCalm);
    let merged = PrLookup::Found(pr_on(GH_REMOTE, 1, PrState::Merged));
    land_pr(&mut app, "app", GH_REMOTE, merged, t1);
    assert!(
        !app.pr_is_stale(path, t1 + secs(9999)),
        "a merged PR is not polled"
    );
}

/// `app` on `feature` after a first lookup of `before` at `remote`.
fn seen_app(remote: &str, before: PrLookup) -> AppState {
    let mut app = app_with(&[pushed("app")], &[]);
    let _ = app.due_pr_lookups();
    land_pr(&mut app, "app", remote, before, Instant::now());
    assert!(
        app.take_pr_transitions().is_empty(),
        "never on the first lookup"
    );
    app
}

/// Land `pr` (or no PR) for `app` at `remote` and take the transitions.
fn changes(app: &mut AppState, remote: &str, pr: Option<PullRequest>) -> Vec<PrTransition> {
    let lookup = pr.map_or(PrLookup::NoPr, PrLookup::Found);
    land_pr(app, "app", remote, lookup, Instant::now());
    app.take_pr_transitions()
}

fn transition(pr: PullRequest, to: PrTransitionTo) -> PrTransition {
    PrTransition {
        checkout: PathBuf::from("app"),
        pr,
        to,
    }
}

#[test]
fn a_change_into_ci_failed_ready_or_merged_or_a_closed_pr_queues_a_transition() {
    use PrState::*;
    for remote in [GH_REMOTE, GL_REMOTE] {
        for (from, to) in [
            (InReview, CiFailed),
            (CiRunning, CiFailed),
            (CiRunning, Ready),
            (InReview, Ready),
            (Draft, Ready),
            (Ready, Merged),
            (CiFailed, Merged),
        ] {
            let mut app = seen_app(remote, PrLookup::Found(pr_on(remote, 12, from)));
            let pr = pr_on(remote, 12, to);
            assert_eq!(
                changes(&mut app, remote, Some(pr.clone())),
                vec![transition(pr, PrTransitionTo::State(to))],
                "{remote}: {from:?} -> {to:?}"
            );
            assert!(app.take_pr_transitions().is_empty(), "drained");
        }

        let before = pr_on(remote, 12, CiRunning);
        let mut app = seen_app(remote, PrLookup::Found(before.clone()));
        assert_eq!(
            changes(&mut app, remote, None),
            vec![transition(before, PrTransitionTo::Closed)],
            "{remote}: closed"
        );

        // A failed lookup in between keeps the answer before it, so the
        // change still alerts.
        let mut app = seen_app(remote, open_on(remote, 12));
        land_pr(
            &mut app,
            "app",
            remote,
            PrLookup::Failed(None),
            Instant::now(),
        );
        let failed = pr_on(remote, 12, CiFailed);
        assert_eq!(
            changes(&mut app, remote, Some(failed.clone())),
            vec![transition(failed, PrTransitionTo::State(CiFailed))],
            "{remote}: across a failed lookup"
        );
    }
}

#[test]
fn other_pr_changes_update_silently() {
    use PrState::*;
    for remote in [GH_REMOTE, GL_REMOTE] {
        for (from, to) in [
            (InReview, CiRunning),
            (CiFailed, CiRunning),
            (Ready, InReview),
            (InReview, Draft),
            (CiFailed, CiFailed),
            (Ready, Ready),
        ] {
            let mut app = seen_app(remote, PrLookup::Found(pr_on(remote, 12, from)));
            assert!(
                changes(&mut app, remote, Some(pr_on(remote, 12, to))).is_empty(),
                "{remote}: {from:?} -> {to:?}"
            );
        }
        let mut app = seen_app(remote, open_on(remote, 12));
        assert!(
            changes(&mut app, remote, Some(pr_on(remote, 13, CiFailed))).is_empty(),
            "{remote}: another PR number"
        );
        let mut app = seen_app(remote, PrLookup::Found(pr_on(remote, 12, Merged)));
        assert!(
            changes(&mut app, remote, None).is_empty(),
            "{remote}: a merged PR that goes away"
        );
        let mut app = seen_app(remote, PrLookup::NoPr);
        assert!(
            changes(&mut app, remote, Some(pr_on(remote, 12, CiFailed))).is_empty(),
            "{remote}: a new PR"
        );
    }
}

#[test]
fn no_transition_after_a_branch_change_and_r_still_compares() {
    let mut app = seen_app(GH_REMOTE, open_on(GH_REMOTE, 7));
    let path = Path::new("app");
    let failed = || Some(pr_on(GH_REMOTE, 7, PrState::CiFailed));

    // feature → other → feature, each answer landed.
    apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
    let _ = app.due_pr_lookups();
    assert!(app.apply_pr_lookup(path, "other", Some(GH_REMOTE.into()), PrLookup::NoPr));
    apply_one_repo_snapshot(&mut app, "app", Some(pushed("app")));
    let _ = app.due_pr_lookups();
    assert!(
        changes(&mut app, GH_REMOTE, failed()).is_empty(),
        "the first lookup after a branch change"
    );

    // Away and back before any answer lands.
    apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
    let _ = app.due_pr_lookups();
    apply_one_repo_snapshot(&mut app, "app", Some(pushed("app")));
    let _ = app.due_pr_lookups();
    let ready = pr_on(GH_REMOTE, 7, PrState::Ready);
    assert!(
        changes(&mut app, GH_REMOTE, Some(ready.clone())).is_empty(),
        "a switch away and back"
    );

    // `r` keeps the badge while it looks again, and the last answer
    // seen: the same state is quiet, a change alerts.
    app.forget_pr_lookups(Some(path));
    assert!(app.take_pr_transitions().is_empty(), "a forget queues none");
    assert_eq!(app.pr_badge(path), Some(PrState::Ready));
    let _ = app.due_pr_lookups();
    assert!(changes(&mut app, GH_REMOTE, Some(ready)).is_empty());
    app.forget_pr_lookups(None);
    let _ = app.due_pr_lookups();
    let merged = pr_on(GH_REMOTE, 7, PrState::Merged);
    assert_eq!(
        changes(&mut app, GH_REMOTE, Some(merged.clone())),
        vec![transition(merged, PrTransitionTo::State(PrState::Merged))],
        "a full reload compares with the last answer too"
    );
}

#[test]
fn ready_versus_in_review_alerts_only_on_settled_merge_facts() {
    use PrState::*;
    for remote in [GH_REMOTE, GL_REMOTE] {
        let mut app = seen_app(remote, open_on(remote, 5));
        assert!(
            changes(&mut app, remote, Some(computing(remote, 5, Ready))).is_empty(),
            "{remote}: ready while the forge computes"
        );
        assert_eq!(
            app.pr_badge(Path::new("app")),
            Some(Ready),
            "the badge shows it"
        );
        assert!(
            changes(&mut app, remote, Some(computing(remote, 5, InReview))).is_empty(),
            "{remote}: back to in review while it computes"
        );
        let ready = pr_on(remote, 5, Ready);
        assert_eq!(
            changes(&mut app, remote, Some(ready.clone())),
            vec![transition(ready, PrTransitionTo::State(Ready))],
            "{remote}: the settled answer alerts once"
        );
        assert!(
            changes(&mut app, remote, Some(computing(remote, 5, InReview))).is_empty(),
            "{remote}: in review while it computes"
        );
        assert!(
            changes(&mut app, remote, Some(pr_on(remote, 5, Ready))).is_empty(),
            "{remote}: still ready as last settled: no flap"
        );
        let failed = computing(remote, 5, CiFailed);
        assert_eq!(
            changes(&mut app, remote, Some(failed.clone())),
            vec![transition(failed, PrTransitionTo::State(CiFailed))],
            "{remote}: CI failed still alerts"
        );

        let mut app = seen_app(remote, PrLookup::Found(computing(remote, 6, InReview)));
        assert!(
            changes(&mut app, remote, Some(pr_on(remote, 6, Ready))).is_empty(),
            "{remote}: an answer that still computes is not the before"
        );
    }
}

// ---- change alerts: toasts and row flashes ----

use super::super::super::toast::Toast;

/// Mark, text, detail, text role, and TTL of `toast`.
fn toast_parts(toast: &Toast) -> (ToastMark, String, String, SegRole, u64) {
    (
        toast.mark,
        toast.text.clone(),
        toast.detail.clone(),
        toast.role,
        toast.ttl_ms,
    )
}

/// Text and detail of every toast on the stack at `now`, oldest first.
fn toast_texts(app: &AppState, now: Instant) -> Vec<(String, String)> {
    app.toasts
        .visible(now)
        .map(|(toast, _)| (toast.text.clone(), toast.detail.clone()))
        .collect()
}

/// Flash ramp of each painted tree row that flashes, by row id.
fn flashed_rows(app: &AppState) -> Vec<(String, FlashKind)> {
    app.rows
        .iter()
        .filter_map(|row| Some((row.id.clone(), app.flashes.get(&row.id)?.kind)))
        .collect()
}

#[test]
fn pr_change_toasts_read_glyph_ref_state_and_check_or_branch_on_both_forges() {
    use PrState::*;
    use PrTransitionTo::{Closed, State};
    let at = Instant::now();
    let branch = "feature/ABC-123-retry-ui";
    for (remote, sign) in [(GH_REMOTE, "#"), (GL_REMOTE, "!")] {
        for (to, unicode, ascii, role, label) in [
            (State(CiFailed), "✕", "x", SegRole::Deleted, "CI failed"),
            (State(Ready), "◉", "A", SegRole::Added, "ready to merge"),
            (State(Merged), "✓", "m", SegRole::BranchFeature, "merged"),
            (Closed, "⊘", "c", SegRole::Deleted, "closed"),
        ] {
            let state = match to {
                State(state) => state,
                Closed => InReview,
            };
            let pr = PullRequest {
                head: branch.into(),
                ..pr_on(remote, 318, state)
            };
            let change = transition(pr, to);
            for (look, glyph) in [(false, unicode), (true, ascii)] {
                assert_eq!(
                    toast_parts(&change.toast(look, at)),
                    (
                        ToastMark::Glyph(glyph, role),
                        format!("{sign}318 {label}"),
                        branch.to_string(),
                        SegRole::Repo,
                        PR_TOAST_TTL_MS,
                    ),
                    "{remote}: {to:?}, ascii {look}"
                );
            }
            assert_eq!(change.toast(false, at).at, at);
        }
    }

    // GitHub names the first failed check on CI failed; any other
    // change keeps the branch.
    let named = |state| PullRequest {
        head: branch.into(),
        failed_check: Some("test (ubuntu-latest)".into()),
        ..pr_on(GH_REMOTE, 318, state)
    };
    let failed = transition(named(CiFailed), State(CiFailed));
    assert_eq!(
        (failed.toast(false, at).text, failed.toast(false, at).detail),
        ("#318 CI failed".into(), "test (ubuntu-latest)".into())
    );
    let merged = transition(named(Merged), State(Merged));
    assert_eq!(merged.toast(false, at).detail, branch);

    // A GitLab badge answer names no job (`failed_check` is `None`), so
    // its CI failed toast names the branch.
    let gitlab = PullRequest {
        head: branch.into(),
        ..pr_on(GL_REMOTE, 12, CiFailed)
    };
    assert_eq!(gitlab.failed_check, None);
    let failed = transition(gitlab, State(CiFailed));
    assert_eq!(
        (failed.toast(false, at).text, failed.toast(false, at).detail),
        ("!12 CI failed".into(), branch.into())
    );
    assert_eq!(PR_TOAST_TTL_MS, 8000);
}

#[test]
fn a_batch_of_more_than_three_pr_changes_toasts_the_first_two_and_a_summary() {
    use PrState::*;
    let names = ["a", "b", "c", "d", "e"];
    let mut app = app_with(&names.map(pushed), &[]);
    app.ascii = false;
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    for (number, name) in (1..).zip(names) {
        land_pr(&mut app, name, &gh(name), open_on(&gh(name), number), t0);
    }
    assert!(!app.alert_pr_transitions(t0), "the first lookups: none");
    assert!(app.toasts.is_empty());

    // Five changes in one batch: two toasts and a summary of three.
    let into = [CiFailed, Ready, Merged, CiFailed];
    for ((number, name), state) in (1..).zip(names).zip(into) {
        let found = PrLookup::Found(pr_on(&gh(name), number, state));
        land_pr(&mut app, name, &gh(name), found, t0);
    }
    land_pr(&mut app, "e", &gh("e"), PrLookup::NoPr, t0);
    assert!(app.alert_pr_transitions(t0));
    assert_eq!(
        toast_texts(&app, t0),
        [
            ("#1 CI failed".into(), "feature".into()),
            ("#2 ready to merge".into(), "feature".into()),
            ("+3 more PR changes".into(), "gp".into()),
        ]
    );
    let summary = app.toasts.visible(t0).last().unwrap().0.clone();
    assert_eq!(
        toast_parts(&summary),
        (
            ToastMark::None,
            "+3 more PR changes".into(),
            "gp".into(),
            SegRole::Muted,
            PR_TOAST_TTL_MS,
        ),
        "muted, no mark, 8 s"
    );
    assert_eq!(
        flashed_rows(&app),
        [
            ("repo:a".into(), FlashKind::Remove),
            ("repo:b".into(), FlashKind::Add),
            ("repo:c".into(), FlashKind::Add),
            ("repo:d".into(), FlashKind::Remove),
            ("repo:e".into(), FlashKind::Remove),
        ],
        "every change flashes, toasted or summed up"
    );
    assert!(!app.alert_pr_transitions(t0), "drained");

    // Exactly three: three toasts, no summary.
    let mut app = app_with(&names.map(pushed), &[]);
    let _ = app.due_pr_lookups();
    for (number, name) in (1..).zip(&names[..3]) {
        land_pr(&mut app, name, &gh(name), open_on(&gh(name), number), t0);
    }
    for (number, name) in (1..).zip(&names[..3]) {
        let ready = PrLookup::Found(pr_on(&gh(name), number, Ready));
        land_pr(&mut app, name, &gh(name), ready, t0);
    }
    assert!(app.alert_pr_transitions(t0));
    let texts: Vec<String> = toast_texts(&app, t0).into_iter().map(|t| t.0).collect();
    assert_eq!(
        texts,
        [
            "#1 ready to merge",
            "#2 ready to merge",
            "#3 ready to merge"
        ]
    );
}

#[test]
fn a_pr_change_flashes_the_checkout_tree_row_on_the_remove_or_add_ramp() {
    use PrState::*;
    // `app` is a repo row of its own. `api` (primary) and `api-wt`
    // (linked) are checkout rows under the `repo:api` family row, which
    // paints no badge and never flashes.
    let linked_wt = RepoSnapshot {
        sync_status: SyncStatus::UpToDate,
        ..linked("api-wt", "api", "feature")
    };
    let repos = [pushed("app"), pushed("api"), linked_wt];
    for (to, kind) in [
        (Some(CiFailed), FlashKind::Remove),
        (Some(Ready), FlashKind::Add),
        (Some(Merged), FlashKind::Add),
        (None, FlashKind::Remove),
    ] {
        for (checkout, row) in [
            ("app", "repo:app"),
            ("api", "checkout:api"),
            ("api-wt", "checkout:api-wt"),
        ] {
            let mut app = app_with(&repos, &[]);
            let _ = app.due_pr_lookups();
            let remote = gh(checkout);
            land_pr(
                &mut app,
                checkout,
                &remote,
                open_on(&remote, 9),
                Instant::now(),
            );
            let lookup = to.map_or(PrLookup::NoPr, |state| {
                PrLookup::Found(pr_on(&remote, 9, state))
            });
            land_pr(&mut app, checkout, &remote, lookup, Instant::now());
            assert!(app.flashes.is_empty());
            let at = Instant::now() + secs(1);
            assert!(app.alert_pr_transitions(at));
            assert_eq!(
                flashed_rows(&app),
                [(row.to_string(), kind)],
                "{checkout}: {to:?}"
            );
            let stamps: Vec<(String, Instant)> = app
                .flashes
                .iter()
                .map(|(id, stamp)| (id.clone(), stamp.at))
                .collect();
            assert_eq!(
                stamps,
                [(row.to_string(), at)],
                "{checkout}: only the badge row, stamped at the alert time"
            );
        }
    }
}

#[test]
fn no_pr_alert_on_first_load_or_a_branch_change_and_one_after_a_poll_changes_the_state() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let mut app = app_with(&[pushed("app")], &[]);
        let path = Path::new("app");
        let t0 = Instant::now();
        let _ = app.due_pr_lookups();
        land_pr(&mut app, "app", remote, open_on(remote, 7), t0);
        assert!(!app.alert_pr_transitions(t0), "{remote}: first load");
        assert!(app.toasts.is_empty() && app.flashes.is_empty());

        // A branch change, then back: each first lookup is quiet.
        apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
        let _ = app.due_pr_lookups();
        let failed = PrLookup::Found(pr_on(remote, 8, PrState::CiFailed));
        assert!(app.apply_pr_lookup(path, "other", Some(remote.into()), failed));
        assert!(!app.alert_pr_transitions(t0), "{remote}: branch change");
        apply_one_repo_snapshot(&mut app, "app", Some(pushed("app")));
        let _ = app.due_pr_lookups();
        land_pr(&mut app, "app", remote, open_on(remote, 7), t0);
        assert!(!app.alert_pr_transitions(t0), "{remote}: back again");

        // A poll that lands the same state is quiet; a new state alerts.
        let t1 = t0 + secs(180);
        assert_eq!(polled(&mut app, t1), ["app"]);
        land_pr(&mut app, "app", remote, open_on(remote, 7), t1);
        assert!(!app.alert_pr_transitions(t1), "{remote}: same state");
        let t2 = t1 + secs(180);
        assert_eq!(polled(&mut app, t2), ["app"]);
        let merged = PrLookup::Found(pr_on(remote, 7, PrState::Merged));
        land_pr(&mut app, "app", remote, merged, t2);
        assert!(app.alert_pr_transitions(t2), "{remote}: merged");
        let sign = if remote == GH_REMOTE { "#" } else { "!" };
        assert_eq!(
            toast_texts(&app, t2),
            [(format!("{sign}7 merged"), "feature".to_string())]
        );
        assert_eq!(
            flashed_rows(&app),
            [("repo:app".to_string(), FlashKind::Add)]
        );
    }
}

#[test]
fn a_poll_with_the_same_pr_keeps_the_detail_and_another_pr_drops_it() {
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    let t0 = Instant::now();
    assert_eq!(
        app.due_pr_polls(t0),
        vec![(PathBuf::from("app"), "feature".to_string())]
    );
    // Same PR, new update time: a changed answer.
    let mut changed =
        PullRequest::fixture(7, "https://github.com/octo/demo/pull/7", PrState::InReview);
    changed.updated_at = Some(1_767_312_000);
    assert!(app.apply_pr_lookup_at(
        Path::new("app"),
        "feature",
        Some(REMOTE.into()),
        PrLookup::Found(changed),
        t0 + secs(1)
    ));
    assert!(
        matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
        "the same PR keeps its detail"
    );
    assert_eq!(
        app.pr_cache.detail_refresh.values().collect::<Vec<_>>(),
        vec![&DetailRefresh::Due],
        "marked for a new fetch"
    );
    set_answer(&mut app, 8, PrState::InReview);
    assert_eq!(detail_of(&app), None, "another PR drops it");
    assert!(app.pr_cache.details.is_empty());
    assert!(app.pr_cache.detail_refresh.is_empty());
}

#[test]
fn a_pinned_popover_asks_again_after_its_pr_answer_changed() {
    let title = |app: &AppState| match detail_of(app) {
        Some(PrDetailState::Ready(detail)) => detail.title,
        other => panic!("a ready detail expected: {other:?}"),
    };
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    assert!(app.popover_pinned());
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    assert_eq!(app.request_pr_details(), Effect::None, "no change");

    set_answer(&mut app, 7, PrState::Ready);
    assert_eq!(
        app.request_pr_details(),
        fetch(7, 2),
        "a changed answer asks again"
    );
    assert_eq!(title(&app), "Add login", "the old detail stays meanwhile");
    assert_eq!(app.request_pr_details(), Effect::None, "asked once");
    let PrDetailLookup::Found(mut newer) = pr_detail(7) else {
        unreachable!()
    };
    newer.title = "Add login and logout".into();
    assert!(land_request(
        &mut app,
        "feature",
        7,
        2,
        PrDetailLookup::Found(newer)
    ));
    assert_eq!(title(&app), "Add login and logout");
    assert!(app.pr_cache.detail_refresh.is_empty());

    // A failed new fetch keeps the detail that landed.
    set_answer(&mut app, 7, PrState::CiRunning);
    assert_eq!(app.request_pr_details(), fetch(7, 3));
    assert!(land_request(
        &mut app,
        "feature",
        7,
        3,
        PrDetailLookup::Failed(None)
    ));
    assert_eq!(title(&app), "Add login and logout");

    // "Refresh this PR" marks it too, with no change in the answer.
    assert_eq!(
        app.refresh_pull_requests(Some(Path::new("app"))),
        Effect::LookupPullRequests {
            targets: vec![(PathBuf::from("app"), "feature".to_string())]
        }
    );
    assert_eq!(app.request_pr_details(), fetch(7, 4));
}

#[test]
fn poll_cycles_never_request_a_detail_on_their_own() {
    let mut app = app_with_pr(7);
    focus(&mut app, "repo:app");
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    assert_eq!(
        app.due_pr_polls(Instant::now()),
        vec![(PathBuf::from("app"), "feature".to_string())]
    );
    assert!(
        matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
        "a poll keeps the detail"
    );
    for tick in [Action::WatchTick, Action::FetchTick] {
        let effect = app.dispatch(tick.clone());
        assert!(
            !format!("{effect:?}").contains("LookupPullRequestDetail"),
            "{tick:?}: {effect:?}"
        );
    }
    set_answer(&mut app, 7, PrState::InReview);
    assert_eq!(
        app.request_pr_details(),
        Effect::None,
        "the same answer asks nothing"
    );
    assert!(app.pr_cache.detail_refresh.is_empty());
}

#[test]
fn refresh_looks_up_now_keeps_the_badge_and_skips_the_backoff() {
    let mut app = app_with(
        &[pushed("app"), pushed("lib"), repo("local", "feature")],
        &[],
    );
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    land_pr(&mut app, "app", GH_REMOTE, open_on(GH_REMOTE, 1), t0);
    land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
    land_pr(&mut app, "local", &gh("local"), PrLookup::NoPr, t0);
    assert_eq!(run_cycle(&mut app, t0 + secs(180), true), ["app", "lib"]);
    assert!(
        polled(&mut app, t0 + secs(360)).is_empty(),
        "github.com backs off"
    );

    let targets = |effect: Effect| match effect {
        Effect::LookupPullRequests { targets } => {
            let mut names: Vec<String> = targets
                .into_iter()
                .map(|(path, _)| path.display().to_string())
                .collect();
            names.sort();
            names
        }
        other => panic!("lookups expected: {other:?}"),
    };
    assert_eq!(targets(app.refresh_pull_requests(Some(path))), ["app"]);
    assert_eq!(
        app.pr_badge(path),
        Some(PrState::InReview),
        "the badge stays"
    );
    assert_eq!(
        app.refresh_pull_requests(Some(path)),
        Effect::None,
        "already in flight"
    );
    assert_eq!(
        targets(app.refresh_pull_requests(None)),
        ["lib", "local"],
        "every checkout with a forge remote, polled or not"
    );
}

/// `app` on `feature` with an open PR 7 at `remote` checked at `t0`,
/// then `r` (`Some`) or a full reload (`None`) and the status apply
/// that queues the new lookup.
fn reloaded(remote: &str, full: bool, t0: Instant) -> AppState {
    let mut app = app_with(&[pushed("app")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    land_pr(&mut app, "app", remote, open_on(remote, 7), t0);
    app.forget_pr_lookups((!full).then_some(path));
    assert_eq!(
        app.pr_badge(path),
        Some(PrState::InReview),
        "the badge stays while it looks again"
    );
    assert_eq!(
        app.due_pr_lookups(),
        vec![(PathBuf::from("app"), "feature".to_string())],
        "the next status apply looks it up again"
    );
    assert!(app.due_pr_lookups().is_empty(), "once");
    app
}

#[test]
fn r_or_a_reload_with_a_failing_cli_keeps_the_badge_and_records_the_error() {
    let path = Path::new("app");
    for remote in [GH_REMOTE, GL_REMOTE] {
        for full in [false, true] {
            let what = format!("{remote}, full reload {full}");
            let t0 = Instant::now();
            let mut app = reloaded(remote, full, t0);
            let failed = PrLookup::Failed(Some("HTTP 401: Bad credentials".into()));
            land_pr(&mut app, "app", remote, failed, t0 + secs(30));
            assert_eq!(app.pr_badge(path), Some(PrState::InReview), "{what}");
            assert_eq!(
                app.pr_lookup_failure(path)
                    .and_then(|failure| failure.reason.as_deref()),
                Some("HTTP 401: Bad credentials"),
                "{what}"
            );
            assert_eq!(app.pr_checked_at(path), Some(t0), "{what}: last success");
            assert!(!app.pr_is_stale(path, t0 + secs(360)), "{what}");
            assert!(
                app.pr_is_stale(path, t0 + secs(361)),
                "{what}: stale dims it like a failed poll"
            );
        }
    }
}

#[test]
fn r_or_a_reload_with_no_pr_clears_the_badge() {
    let path = Path::new("app");
    for remote in [GH_REMOTE, GL_REMOTE] {
        for full in [false, true] {
            let what = format!("{remote}, full reload {full}");
            let t0 = Instant::now();
            let mut app = reloaded(remote, full, t0);
            land_pr(&mut app, "app", remote, PrLookup::NoPr, t0 + secs(30));
            assert_eq!(app.pr_badge(path), None, "{what}");
            assert_eq!(app.pr_lookup_failure(path), None, "{what}");
            assert_eq!(app.pr_checked_at(path), Some(t0 + secs(30)), "{what}");
        }
    }
}

/// `app` (GitHub PR 1) and `lib` (GitHub PR 2), two repos on one host,
/// both open and looked up at `t0`.
fn one_host_app(t0: Instant) -> AppState {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    land_pr(&mut app, "app", &gh("app"), open_on(&gh("app"), 1), t0);
    land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
    app
}

#[test]
fn one_failing_checkout_never_backs_off_a_host_that_answers_in_the_same_cycle() {
    let not_found = || PrLookup::Failed(Some("HTTP 404: Not Found".into()));
    for fail_first in [true, false] {
        let t0 = Instant::now();
        let mut app = one_host_app(t0);
        for k in 1..=4u64 {
            let at = t0 + secs(180 * k);
            assert_eq!(
                polled(&mut app, at),
                ["app", "lib"],
                "fail first {fail_first}: cycle {k} polls both"
            );
            let (first, second) = (at + secs(1), at + secs(2));
            if fail_first {
                land_pr(&mut app, "app", &gh("app"), not_found(), first);
                land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), second);
            } else {
                land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), first);
                land_pr(&mut app, "app", &gh("app"), not_found(), second);
            }
        }
        let end = t0 + secs(180 * 4 + 3);
        assert!(
            !app.pr_is_stale(Path::new("lib"), end),
            "fail first {fail_first}: the healthy badge stays fresh"
        );
        assert!(
            app.pr_lookup_failure(Path::new("app")).is_some(),
            "fail first {fail_first}: the failing one records it"
        );
    }
}

#[test]
fn a_cycle_where_every_lookup_on_a_host_fails_backs_it_off() {
    let t0 = Instant::now();
    let mut app = one_host_app(t0);
    let at = t0 + secs(180);
    assert_eq!(polled(&mut app, at), ["app", "lib"]);
    for (name, remote) in [("app", gh("app")), ("lib", gh("lib"))] {
        let failed = PrLookup::Failed(Some("HTTP 502: Bad Gateway".into()));
        land_pr(&mut app, name, &remote, failed, at + secs(1));
    }
    assert!(
        polled(&mut app, t0 + secs(360)).is_empty(),
        "github.com waits 2 × 180 s"
    );
    assert_eq!(polled(&mut app, t0 + secs(540)), ["app", "lib"]);
}

#[test]
fn the_poll_period_and_the_stale_test_parse_no_remote() {
    let parses = || FORGE_HOST_PARSES.with(std::cell::Cell::get);
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    let running = |remote: &str| PrLookup::Found(pr_on(remote, 1, PrState::CiRunning));
    land_pr(&mut app, "app", &gh("app"), running(&gh("app")), t0);
    land_pr(&mut app, "lib", &gl("lib"), open_on(&gl("lib"), 2), t0);
    let before = parses();
    for _ in 0..100 {
        assert_eq!(app.pr_poll_interval_ms(), 60_000);
        assert!(!app.pr_is_stale(Path::new("app"), t0));
        assert!(app.pr_is_stale(Path::new("lib"), t0 + secs(121)));
    }
    assert_eq!(polled(&mut app, t0 + secs(60)), ["app", "lib"]);
    assert_eq!(parses(), before, "no remote parse per frame or per cycle");
    land_pr(
        &mut app,
        "app",
        &gh("app"),
        running(&gh("app")),
        t0 + secs(61),
    );
    assert_eq!(parses(), before, "a poll that lands the same remote");
    land_pr(
        &mut app,
        "lib",
        &gh("lib"),
        open_on(&gh("lib"), 2),
        t0 + secs(61),
    );
    assert_eq!(parses(), before + 1, "a new remote is parsed once");
}

#[test]
fn a_second_change_during_a_detail_reload_asks_again_after_it_lands() {
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, pr_detail(7)));
    set_answer(&mut app, 7, PrState::Ready);
    assert_eq!(app.request_pr_details(), fetch(7, 2));
    set_answer(&mut app, 7, PrState::CiRunning);
    assert_eq!(
        app.request_pr_details(),
        Effect::None,
        "one fetch at a time"
    );
    assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
    assert_eq!(
        app.request_pr_details(),
        fetch(7, 3),
        "that detail is older than the badge: ask again"
    );
    assert!(land_request(&mut app, "feature", 7, 3, pr_detail(7)));
    assert_eq!(app.request_pr_details(), Effect::None, "up to date");
}

#[test]
fn a_change_during_the_first_detail_fetch_asks_again_after_it_lands() {
    let mut app = app_with_pr(7);
    assert_eq!(
        click_badge(&mut app),
        Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 1)])
    );
    set_answer(&mut app, 7, PrState::Ready);
    assert_eq!(
        app.request_pr_details(),
        Effect::None,
        "waits for the first fetch"
    );
    assert!(land_request(&mut app, "feature", 7, 1, pr_detail(7)));
    assert_eq!(app.request_pr_details(), fetch(7, 2));
    assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
    assert_eq!(app.request_pr_details(), Effect::None);
}

// ---- gc (Open CI), Copy URL, Refresh this PR ----

use super::super::super::pr_status::{pipeline_ci_link, CiLink, CiLinkKind};

const FAILED_JOB: &str = "https://github.com/acme/app/actions/runs/1/job/2";
const PIPELINE: &str = "https://gitlab.com/acme/services/api/-/pipelines/99";

/// A PR on the forge of `remote` whose CI link is a failed check
/// (GitHub) or the head pipeline (GitLab), with that ref and URL.
fn pr_with_ci(remote: &str, state: PrState) -> (PullRequest, &'static str, &'static str) {
    if remote.contains("gitlab") {
        let mut pr = pr_on(remote, 12, state);
        pr.ci_link = pipeline_ci_link(Some(PIPELINE), &pr.url);
        (pr, "!12", PIPELINE)
    } else {
        let mut pr = pr_on(remote, 318, state);
        pr.ci_link = CiLink {
            url: FAILED_JOB.into(),
            kind: CiLinkKind::FailedCheck,
        };
        (pr, "#318", FAILED_JOB)
    }
}

/// `app` on `feature` with `lookup` landed at `remote`, focused.
fn app_answered(remote: &str, lookup: PrLookup) -> AppState {
    let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
    let _ = app.due_pr_lookups();
    land_pr(&mut app, "app", remote, lookup, Instant::now());
    focus(&mut app, "repo:app");
    app
}

fn gc(app: &mut AppState) -> Effect {
    app.status = StatusMessage::default();
    app.dispatch(Action::OpenPullRequestCi)
}

fn ci_open(pr_ref: &str, url: &str) -> Effect {
    Effect::OpenPullRequestCi {
        pr_ref: pr_ref.into(),
        url: url.into(),
    }
}

/// Pin the PR popover of `app` (its badge on the focused row).
fn pin_pr(app: &mut AppState) -> Effect {
    let kind =
        super::super::super::tree::pr_badge_kind(app.pr_badge(Path::new("app")).expect("badge"));
    app.pin_icon(&IconHit {
        y: 1,
        x: 20,
        width: 2,
        kind,
        target: IconTarget::PullRequest(PathBuf::from("app")),
    })
}

/// Move the pinned popover focus to the action line titled `title`.
fn focus_line(app: &mut AppState, title: &str) {
    use crate::tui::popover::{flat_lines, PopoverLine};
    let sections = app.open_popover_sections();
    let index = flat_lines(&sections)
        .iter()
        .position(|line| matches!(line, PopoverLine::Action { title: t, .. } if *t == title))
        .unwrap_or_else(|| panic!("no {title} line"));
    app.popover.as_mut().expect("pinned").focus_line = index;
}

#[test]
fn gc_on_a_tree_row_opens_the_known_ci_link_on_both_forges() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        for state in [PrState::CiFailed, PrState::Ready, PrState::Merged] {
            let (pr, pr_ref, url) = pr_with_ci(remote, state);
            let mut app = app_answered(remote, PrLookup::Found(pr));
            assert_eq!(gc(&mut app), ci_open(pr_ref, url), "{remote} {state:?}");
            assert!(app.status.is_empty(), "the worker says how it went");
        }
    }
}

#[test]
fn gc_says_no_pr_no_ci_or_looks_the_pr_up_first() {
    // No branch to look up.
    let mut app = app_answered(GH_REMOTE, PrLookup::NoPr);
    focus(&mut app, "repo:lib");
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("file row");
    assert_eq!(gc(&mut app), Effect::None);
    assert_eq!(app.status, NO_PR_FOR_ROW);
    assert_eq!(app.status.kind(), StatusKind::Warn);

    for remote in [GH_REMOTE, GL_REMOTE] {
        // The forge answered: no PR.
        let mut app = app_answered(remote, PrLookup::NoPr);
        assert_eq!(gc(&mut app), Effect::None);
        assert_eq!(app.status, "no PR for feature");
        assert_eq!(app.status.kind(), StatusKind::Warn);

        // A PR with no CI web page.
        let (mut pr, pr_ref, _) = pr_with_ci(remote, PrState::InReview);
        pr.ci_link.url = String::new();
        let mut app = app_answered(remote, PrLookup::Found(pr));
        assert_eq!(gc(&mut app), Effect::None);
        assert_eq!(app.status, format!("no CI for {pr_ref}"));
        assert_eq!(app.status.kind(), StatusKind::Warn);

        // A failed first lookup knows nothing: look the PR up first.
        let mut app = app_answered(remote, PrLookup::Failed(None));
        assert_eq!(
            gc(&mut app),
            Effect::OpenPullRequest {
                repo: PathBuf::from("app"),
                branch: "feature".into(),
                page: PrPage::Ci,
            }
        );
    }

    // The first lookup is still out.
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let _ = app.due_pr_lookups();
    focus(&mut app, "repo:app");
    assert!(matches!(
        gc(&mut app),
        Effect::OpenPullRequest {
            page: PrPage::Ci,
            ..
        }
    ));
}

#[test]
fn gc_in_the_graph_opens_a_worktree_rows_ci_on_both_forges() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let wt = "app/.worktrees/feat";
        let mut app = app_with(&[repo("app", "main"), linked(wt, "app", "feature")], &[]);
        let _ = app.due_pr_lookups();
        let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiRunning);
        land_pr(&mut app, wt, remote, PrLookup::Found(pr), Instant::now());
        focus(&mut app, "repo:app");
        app.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa".into(),
                subject: "seed".into(),
                ..Commit::default()
            }],
            worktrees: vec![Worktree {
                path: wt.into(),
                head_id: None,
                branch: Some("feature".into()),
                hidden: false,
                is_current: false,
            }],
            window: 1,
            ..GraphModel::default()
        });
        app.focus = FocusPane::Right;
        assert!(app.graph_pane_focused());
        let rows = app.graph.as_ref().expect("graph").visible_rows();
        app.graph_cursor = rows
            .iter()
            .position(|row| matches!(row, GraphRow::Worktree(_)))
            .expect("worktree row");
        assert_eq!(gc(&mut app), ci_open(pr_ref, url), "{remote}");

        app.graph_cursor = rows
            .iter()
            .position(|row| matches!(row, GraphRow::Commit { .. }))
            .expect("commit row");
        assert_eq!(gc(&mut app), Effect::None, "gc is no longer graph c");
        assert_eq!(app.status, NO_PR_FOR_ROW);
    }
}

#[test]
fn the_popover_open_ci_line_runs_gc_on_both_forges() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiFailed);
        let title = pr.ci_link.kind.label();
        let mut app = app_answered(remote, PrLookup::Found(pr));
        let _ = pin_pr(&mut app);
        focus_line(&mut app, title);
        assert_eq!(app.popover_run_action(), Some(Action::OpenPullRequestCi));
        assert_eq!(app.dispatch(Action::PopoverRun), ci_open(pr_ref, url));
        assert!(app.popover.is_none(), "Enter closes, then opens CI");
    }
}

/// The graph header chip pins the PR popover of the graph's checkout
/// while a commit row has the focus: its PR lines act on that PR, not
/// on the focused row (which has none).
#[test]
fn pr_popover_lines_act_on_their_own_pr_not_the_focused_graph_row() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiFailed);
        let (ci_title, pr_url) = (pr.ci_link.kind.label(), pr.url.clone());
        let mut app = app_answered(remote, PrLookup::Found(pr));
        app.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa".into(),
                subject: "seed".into(),
                ..Commit::default()
            }],
            window: 1,
            ..GraphModel::default()
        });
        app.focus = FocusPane::Right;
        let rows = app.graph.as_ref().expect("graph").visible_rows();
        app.graph_cursor = rows
            .iter()
            .position(|row| matches!(row, GraphRow::Commit { .. }))
            .expect("commit row");
        assert_eq!(gc(&mut app), Effect::None, "the commit row has no PR");
        for (title, want) in [
            (ci_title, ci_open(pr_ref, url)),
            (
                "Open PR",
                Effect::OpenPullRequest {
                    repo: PathBuf::from("app"),
                    branch: "feature".into(),
                    page: PrPage::Pr,
                },
            ),
            (
                "Copy URL",
                Effect::CopyClipboard {
                    text: pr_url.clone(),
                    announce: true,
                },
            ),
            (
                "Refresh this PR",
                Effect::LookupPullRequests {
                    targets: vec![(PathBuf::from("app"), "feature".into())],
                },
            ),
        ] {
            let _ = pin_pr(&mut app);
            focus_line(&mut app, title);
            assert_eq!(app.dispatch(Action::PopoverRun), want, "{remote} {title}");
        }
    }
}

#[test]
fn a_ci_open_reports_each_outcome_on_the_status_line() {
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let _ = app.due_pr_lookups();
    for (pr_ref, opened, text, kind) in [
        ("#318", true, "opened CI for #318", StatusKind::Ok),
        ("!12", true, "opened CI for !12", StatusKind::Ok),
        ("#318", false, "could not open CI", StatusKind::Error),
    ] {
        app.apply_pr_ci_open(pr_ref, opened);
        assert_eq!(app.status, text);
        assert_eq!(app.status.kind(), kind, "{text}");
    }

    // `gc` that looked the PR up first.
    let path = Path::new("app");
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, pr_ref, _) = pr_with_ci(remote, PrState::CiFailed);
        let mut no_ci = pr.clone();
        no_ci.ci_link.url = String::new();
        for (lookup, opened, text, kind) in [
            (
                PrLookup::Found(pr.clone()),
                true,
                format!("opened CI for {pr_ref}"),
                StatusKind::Ok,
            ),
            (
                PrLookup::Found(pr.clone()),
                false,
                "could not open CI".into(),
                StatusKind::Error,
            ),
            (
                PrLookup::Found(no_ci),
                false,
                format!("no CI for {pr_ref}"),
                StatusKind::Warn,
            ),
            (
                PrLookup::NoPr,
                false,
                "no PR for feature".into(),
                StatusKind::Warn,
            ),
            (
                PrLookup::Failed(None),
                false,
                "could not look up PR for feature".into(),
                StatusKind::Error,
            ),
            (
                PrLookup::Failed(Some("gh not found".into())),
                false,
                "could not look up PR for feature: gh not found".into(),
                StatusKind::Error,
            ),
        ] {
            app.apply_pr_open(
                path,
                "feature",
                Some(remote.into()),
                lookup,
                PrPage::Ci,
                opened,
            );
            assert_eq!(app.status, text.as_str(), "{remote}");
            assert_eq!(app.status.kind(), kind, "{remote}: {text}");
        }
    }
}

#[test]
fn copy_url_copies_the_pr_url_from_the_palette_and_the_popover_line() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, _, _) = pr_with_ci(remote, PrState::InReview);
        let copy = Effect::CopyClipboard {
            text: pr.url.clone(),
            announce: true,
        };
        let mut app = app_answered(remote, PrLookup::Found(pr));
        assert_eq!(app.dispatch(Action::CopyPullRequestUrl), copy, "{remote}");

        // `y` on the Copy URL line copies the URL, not the title, and
        // the popover stays open.
        let _ = pin_pr(&mut app);
        focus_line(&mut app, "Copy URL");
        assert_eq!(app.dispatch(Action::PopoverCopyLine), copy, "{remote}");
        assert!(app.popover_pinned());
        // Enter runs it like the palette row.
        assert_eq!(app.dispatch(Action::PopoverRun), copy, "{remote}");
        assert!(app.popover.is_none());
    }
    let mut app = app_answered(GH_REMOTE, PrLookup::NoPr);
    assert_eq!(app.dispatch(Action::CopyPullRequestUrl), Effect::None);
    assert_eq!(app.status, "no PR for feature");
    focus(&mut app, "repo:lib");
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("file row");
    assert_eq!(app.dispatch(Action::CopyPullRequestUrl), Effect::None);
    assert_eq!(app.status, NO_PR_FOR_ROW);
}

#[test]
fn r_in_a_pinned_pr_popover_refreshes_that_pr_and_keeps_it_open() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, _, _) = pr_with_ci(remote, PrState::CiRunning);
        let mut app = app_answered(remote, PrLookup::Found(pr.clone()));
        let Effect::LookupPullRequestDetail {
            repo,
            branch,
            remote: resolved,
            number,
            request,
        } = pin_pr(&mut app)
        else {
            panic!("the pin asks for the detail");
        };
        assert!(app.apply_pr_detail(
            &repo,
            &branch,
            &resolved,
            number,
            request,
            pr_detail(number)
        ));
        let refresh = Effect::LookupPullRequests {
            targets: vec![(PathBuf::from("app"), "feature".into())],
        };
        assert_eq!(
            app.dispatch(Action::RefreshPullRequest),
            refresh,
            "{remote}"
        );
        assert!(app.popover_pinned(), "r keeps the popover");
        assert_eq!(
            app.pr_badge(Path::new("app")),
            Some(PrState::CiRunning),
            "the badge stays while it looks again"
        );
        assert!(
            matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
            "the detail stays on screen while it looks again"
        );
        assert_eq!(
            app.dispatch(Action::RefreshPullRequest),
            Effect::None,
            "already looking"
        );
        assert_eq!(app.status, PR_LOOKUP_RUNNING);
        assert_eq!(app.status.kind(), StatusKind::Warn);

        // The answer lands: the pinned popover asks for the detail
        // again.
        land_pr(&mut app, "app", remote, PrLookup::Found(pr), Instant::now());
        assert_eq!(
            app.request_pr_details(),
            Effect::LookupPullRequestDetail {
                repo: PathBuf::from("app"),
                branch: "feature".into(),
                remote: remote.into(),
                number,
                request: request + 1,
            },
            "{remote}: the detail is fetched again"
        );

        // The palette row (no popover) refreshes the focused row.
        let (pr, _, _) = pr_with_ci(remote, PrState::Ready);
        let mut app = app_answered(remote, PrLookup::Found(pr));
        assert_eq!(
            app.dispatch(Action::RefreshPullRequest),
            refresh,
            "{remote}"
        );
    }

    // A pinned popover with no PR section ignores `r`.
    let mut app = app_answered(GH_REMOTE, PrLookup::NoPr);
    let _ = app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned());
    assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
    assert!(app.status.is_empty());
    assert!(app.popover_pinned());
}

/// Refresh this PR says why it looks nothing up: a lookup of that
/// checkout is already out, or the checkout has no forge remote or no
/// branch.
#[test]
fn refresh_this_pr_says_why_it_cannot_run() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, _, _) = pr_with_ci(remote, PrState::InReview);
        let mut app = app_answered(remote, PrLookup::Found(pr));
        assert!(matches!(
            app.dispatch(Action::RefreshPullRequest),
            Effect::LookupPullRequests { .. }
        ));
        assert!(app.status.is_empty());
        assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
        assert_eq!(app.status, PR_LOOKUP_RUNNING, "{remote}");
        assert_eq!(app.status.kind(), StatusKind::Warn);
    }

    // The first lookup is still out.
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let _ = app.due_pr_lookups();
    focus(&mut app, "repo:app");
    assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
    assert_eq!(app.status, PR_LOOKUP_RUNNING);

    // No GitHub or GitLab remote.
    let mut app = app_with(&[repo("app", "feature")], &[]);
    let _ = app.due_pr_lookups();
    assert!(app.apply_pr_lookup(Path::new("app"), "feature", None, PrLookup::NoPr));
    focus(&mut app, "repo:app");
    assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
    assert_eq!(app.status, NO_PR_FOR_ROW);
    assert_eq!(app.status.kind(), StatusKind::Warn);

    // No branch: a file row.
    app.status = StatusMessage::default();
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("file row");
    assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
    assert_eq!(app.status, NO_PR_FOR_ROW);
}

/// Dispatch the action the key `code` maps to in the current mode.
fn press(app: &mut AppState, code: crossterm::event::KeyCode) -> Effect {
    use crossterm::event::{Event, KeyEvent, KeyModifiers};
    let event = Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    let action = crate::tui::keys::event_to_action(&event, app.input_mode(), false, false);
    app.dispatch(action)
}

/// `gx` and `gc` inside a pinned PR popover act on its PR section, even
/// while the focused graph row (a commit) has no PR; the popover closes
/// as Enter on the line closes it.
#[test]
fn gx_and_gc_in_a_pinned_pr_popover_act_on_its_pr_on_both_forges() {
    use crate::tui::keys::InputMode;
    use crossterm::event::KeyCode;
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiFailed);
        let mut app = app_answered(remote, PrLookup::Found(pr));
        app.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa".into(),
                subject: "seed".into(),
                ..Commit::default()
            }],
            window: 1,
            ..GraphModel::default()
        });
        app.focus = FocusPane::Right;
        let rows = app.graph.as_ref().expect("graph").visible_rows();
        app.graph_cursor = rows
            .iter()
            .position(|row| matches!(row, GraphRow::Commit { .. }))
            .expect("commit row");
        for (second, want) in [
            (KeyCode::Char('c'), ci_open(pr_ref, url)),
            (
                KeyCode::Char('x'),
                Effect::OpenPullRequest {
                    repo: PathBuf::from("app"),
                    branch: "feature".into(),
                    page: PrPage::Pr,
                },
            ),
        ] {
            let _ = pin_pr(&mut app);
            assert_eq!(app.input_mode(), InputMode::Popover);
            assert_eq!(press(&mut app, KeyCode::Char('g')), Effect::None);
            assert_eq!(app.input_mode(), InputMode::PopoverGPending);
            assert_eq!(press(&mut app, second), want, "{remote} g{second:?}");
            assert!(app.popover.is_none(), "the line's key closes it");
        }
    }
}

/// `gc` in a pinned popover with no PR section says so and keeps the
/// popover; `g` then Esc only ends the chord.
#[test]
fn g_chords_in_a_popover_without_a_pr_or_ended_by_esc() {
    use crate::tui::keys::InputMode;
    use crossterm::event::KeyCode;
    let mut app = app_answered(GH_REMOTE, open_on(GH_REMOTE, 7));
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("file row");
    let _ = app.dispatch(Action::PopoverOpenFocused);
    assert!(app.popover_pinned(), "a file row popover");
    for second in [KeyCode::Char('c'), KeyCode::Char('x')] {
        app.status = StatusMessage::default();
        let _ = press(&mut app, KeyCode::Char('g'));
        assert_eq!(press(&mut app, second), Effect::None);
        assert_eq!(app.status, NO_PR_FOR_ROW);
        assert!(app.popover_pinned(), "the popover stays");
        assert_eq!(app.input_mode(), InputMode::Popover);
    }

    // `g` then Esc ends the chord and keeps the popover; a later `c`
    // is a plain popover key again (it does nothing).
    let mut app = app_answered(GH_REMOTE, open_on(GH_REMOTE, 7));
    let _ = pin_pr(&mut app);
    let _ = press(&mut app, KeyCode::Char('g'));
    assert_eq!(app.input_mode(), InputMode::PopoverGPending);
    assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
    assert_eq!(app.input_mode(), InputMode::Popover);
    assert!(app.popover_pinned(), "Esc in the chord does not close");
    assert_eq!(press(&mut app, KeyCode::Char('c')), Effect::None);
    assert!(app.popover_pinned());
    // `g` then `j` ends the chord without moving the focus.
    let focus = app.popover.as_ref().expect("pinned").focus_line;
    let _ = press(&mut app, KeyCode::Char('g'));
    let _ = press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.input_mode(), InputMode::Popover);
    assert_eq!(app.popover.as_ref().expect("pinned").focus_line, focus);
}

/// A poll that drops a line above the focused one (the notes) keeps
/// the pinned focus on the same action line.
#[test]
fn a_landed_badge_lookup_keeps_the_pinned_focus_on_its_line() {
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (mut pr, _, _) = pr_with_ci(remote, PrState::InReview);
        pr.notes.conflicts = true;
        let mut app = app_answered(remote, PrLookup::Found(pr.clone()));
        let _ = pin_pr(&mut app);
        focus_line(&mut app, "Copy URL");
        let notes = |app: &AppState| {
            app.open_popover_sections()
                .iter()
                .flat_map(|section| section.lines.iter())
                .any(|line| line.copy_text() == "has conflicts")
        };
        assert!(notes(&app));
        pr.notes.conflicts = false;
        land_pr(&mut app, "app", remote, PrLookup::Found(pr), Instant::now());
        assert!(!notes(&app), "the poll dropped the notes line");
        assert_eq!(
            app.popover_run_action(),
            Some(Action::CopyPullRequestUrl),
            "{remote}: the focus stays on Copy URL"
        );
    }
}

/// A pinned PR popover does not pause polling: the event loop runs the
/// poll tick in `Popover` and `PopoverGPending` (both pass
/// `overlay_blocks_background_ticks`), the cycle takes the popover's
/// checkout, and the landed answer shows in the PR section while the
/// popover stays pinned.
#[test]
fn a_pinned_pr_popover_keeps_polling_and_shows_the_new_state() {
    use super::super::super::event_pump::overlay_blocks_background_ticks;
    use super::super::super::keys::InputMode;
    assert!(!overlay_blocks_background_ticks(InputMode::PopoverGPending));
    for remote in [GH_REMOTE, GL_REMOTE] {
        let (pr, _, _) = pr_with_ci(remote, PrState::CiRunning);
        let mut app = app_answered(remote, PrLookup::Found(pr.clone()));
        let _ = pin_pr(&mut app);
        assert!(app.popover_pinned(), "{remote}");
        assert_eq!(app.input_mode(), InputMode::Popover, "{remote}");
        assert!(!overlay_blocks_background_ticks(app.input_mode()));
        let heading = |app: &AppState| {
            app.open_popover_sections()
                .iter()
                .find(|section| section.target == IconTarget::PullRequest(PathBuf::from("app")))
                .map(|section| section.heading().to_string())
        };
        assert_eq!(heading(&app).as_deref(), Some("PR · CI running"));

        let due = app.due_pr_polls(Instant::now());
        assert!(
            due.contains(&(PathBuf::from("app"), "feature".to_string())),
            "{remote}: the poll takes the pinned checkout: {due:?}"
        );
        let ready = PullRequest {
            state: PrState::Ready,
            ..pr
        };
        land_pr(
            &mut app,
            "app",
            remote,
            PrLookup::Found(ready),
            Instant::now(),
        );
        assert!(app.popover_pinned(), "{remote}: still pinned");
        assert_eq!(
            heading(&app).as_deref(),
            Some("PR · ready to merge"),
            "{remote}"
        );
    }
}

/// Two check names share the blank label: the focus on the second one
/// stays there when a new detail adds a line above it.
#[test]
fn the_focus_on_a_check_name_survives_a_detail_landing() {
    use super::super::super::pr_status::{Check, CheckBucket};
    use crate::tui::popover::flat_lines;
    let with_checks = |author: Option<&str>| {
        let PrDetailLookup::Found(mut detail) = pr_detail(7) else {
            unreachable!("a found detail");
        };
        detail.author = author.map(str::to_string);
        detail.check_names = ["test", "build"]
            .iter()
            .zip([CheckBucket::Fail, CheckBucket::Pending])
            .map(|(name, bucket)| Check {
                bucket,
                name: name.to_string(),
                url: None,
            })
            .collect();
        PrDetailLookup::Found(detail)
    };
    let focused_text = |app: &AppState| {
        let sections = app.open_popover_sections();
        let index = app.popover.as_ref().expect("pinned").focus_line;
        flat_lines(&sections)[index].copy_text()
    };
    let mut app = app_with_pr(7);
    let _ = click_badge(&mut app);
    assert!(land(&mut app, "feature", 7, with_checks(None)));
    let sections = app.open_popover_sections();
    let build = flat_lines(&sections)
        .iter()
        .position(|line| line.copy_text() == "build")
        .expect("build line");
    app.popover.as_mut().expect("pinned").focus_line = build;
    assert_eq!(focused_text(&app), "build");

    // The answer changes, so the detail is fetched again; the new one
    // adds `author` above the checks.
    set_answer(&mut app, 7, PrState::CiRunning);
    assert_eq!(app.request_pr_details(), fetch(7, 2));
    assert!(land(&mut app, "feature", 7, with_checks(Some("octo-user"))));
    assert_eq!(focused_text(&app), "build", "not the first check line");
}

#[test]
fn palette_refresh_pull_requests_looks_every_pr_up_again() {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let now = Instant::now();
    land_pr(&mut app, "app", &gh("app"), open_on(GH_REMOTE, 1), now);
    land_pr(&mut app, "lib", &gl("lib"), open_on(GL_REMOTE, 2), now);
    let Effect::LookupPullRequests { mut targets } = app.dispatch(Action::RefreshPullRequests)
    else {
        panic!("a lookup of every PR");
    };
    targets.sort();
    assert_eq!(
        targets,
        vec![
            (PathBuf::from("app"), "feature".to_string()),
            (PathBuf::from("lib"), "feature".to_string()),
        ]
    );
}

fn gitlab_outage() -> OutageSource {
    OutageSource::PrLookup {
        tool: "glab",
        host: "gitlab.com".into(),
    }
}

fn github_outage() -> OutageSource {
    OutageSource::PrLookup {
        tool: "gh",
        host: "github.com".into(),
    }
}

/// Texts of the outage toasts shown now, oldest first.
fn outage_toasts(app: &AppState) -> Vec<String> {
    app.toasts
        .visible(Instant::now())
        .filter(|(toast, _)| toast.is_outage())
        .map(|(toast, _)| toast.text.clone())
        .collect()
}

fn invalid_token() -> PrLookup {
    PrLookup::Failed(Some("invalid_token".into()))
}

#[test]
fn a_pr_outage_toast_cuts_one_leading_tool_name_from_the_reason() {
    assert_eq!(
        pr_outage_toast_text("glab", "gitlab.com", Some("glab: 401 Unauthorized")),
        "glab (gitlab.com): 401 Unauthorized"
    );
    assert_eq!(
        pr_outage_toast_text("glab", "gitlab.com", Some("invalid_token")),
        "glab (gitlab.com): invalid_token",
        "a reason without the prefix is unchanged"
    );
    assert_eq!(
        pr_outage_toast_text("gh", "github.com", Some("gh: gh: HTTP 401")),
        "gh (github.com): gh: HTTP 401",
        "only one prefix goes"
    );
    assert_eq!(
        pr_outage_toast_text("gh", "github.com", Some("ghost: HTTP 401")),
        "gh (github.com): ghost: HTTP 401",
        "only the exact tool name"
    );
    assert_eq!(
        pr_outage_toast_text("gh", "github.com", Some("glab: HTTP 401")),
        "gh (github.com): glab: HTTP 401",
        "another tool's name stays"
    );
    assert_eq!(
        pr_outage_toast_text("gh", "github.com", None),
        "gh (github.com): lookup failed"
    );
}

#[test]
fn a_failed_lookup_whose_reason_repeats_the_tool_toasts_it_once() {
    let mut app = app_with(&[pushed("app")], &[]);
    let _ = app.due_pr_lookups();
    let reason = PrLookup::Failed(Some("glab: 401 Unauthorized".into()));
    land_pr(&mut app, "app", &gl("app"), reason, Instant::now());
    assert_eq!(outage_toasts(&app), ["glab (gitlab.com): 401 Unauthorized"]);
    assert_eq!(
        app.outages.get(&gitlab_outage()).unwrap().reason,
        "glab (gitlab.com): 401 Unauthorized"
    );
}

#[test]
fn a_pr_host_outage_toasts_once_and_ends_when_no_checkout_fails_on_it() {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    let gitlab = gitlab_outage();

    // The first failed lookup on gitlab.com starts the outage.
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t0);
    assert_eq!(outage_toasts(&app), ["glab (gitlab.com): invalid_token"]);
    assert_eq!(app.pr_mark(Path::new("app")), Some(PrMark::LookupFailed));
    // A second checkout on the same host failing: no second toast.
    land_pr(&mut app, "lib", &gl("lib"), invalid_token(), t0);
    assert_eq!(outage_toasts(&app).len(), 1);

    // Esc: the toast goes, the outage (header) stays.
    assert!(app.dismiss_outage_toasts());
    assert!(outage_toasts(&app).is_empty());
    assert!(app.outages.get(&gitlab).is_some());

    // Polls that keep failing show no toast, also with a new reason.
    let t1 = t0 + secs(180);
    assert_eq!(polled(&mut app, t1), ["app", "lib"]);
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t1);
    let http_401 = PrLookup::Failed(Some("HTTP 401".into()));
    land_pr(&mut app, "lib", &gl("lib"), http_401, t1);
    assert!(outage_toasts(&app).is_empty());
    assert!(app.outages.get(&gitlab).is_some());

    // A success on the host while another checkout still fails there:
    // the outage goes on.
    land_pr(&mut app, "app", &gl("app"), open_on(&gl("app"), 1), t1);
    assert_eq!(
        app.pr_mark(Path::new("app")),
        Some(PrMark::State(PrState::InReview))
    );
    assert!(app.outages.get(&gitlab).is_some());
    // No checkout fails on it any more: it recovers.
    land_pr(&mut app, "lib", &gl("lib"), PrLookup::NoPr, t1);
    assert!(app.outages.is_empty());
    assert!(outage_toasts(&app).is_empty());

    // A failure after the recovery toasts again; a failed lookup after
    // a good answer keeps the badge.
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t1);
    assert_eq!(outage_toasts(&app), ["glab (gitlab.com): invalid_token"]);
    assert_eq!(
        app.pr_mark(Path::new("app")),
        Some(PrMark::State(PrState::InReview))
    );
    // Recovery while the toast shows drops it.
    land_pr(&mut app, "app", &gl("app"), open_on(&gl("app"), 1), t1);
    assert!(outage_toasts(&app).is_empty());
    assert!(app.outages.is_empty());
}

#[test]
fn a_failing_gitlab_and_a_working_github_are_separate_outages() {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t0);
    land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
    assert!(
        app.outages.get(&gitlab_outage()).is_some(),
        "github answers"
    );
    assert!(app.outages.get(&github_outage()).is_none());

    land_pr(&mut app, "lib", &gh("lib"), PrLookup::Failed(None), t0);
    assert_eq!(
        outage_toasts(&app),
        [
            "glab (gitlab.com): invalid_token",
            "gh (github.com): lookup failed"
        ],
        "one toast per host; no reason reads `lookup failed`"
    );
    land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
    assert!(app.outages.get(&github_outage()).is_none());
    assert_eq!(outage_toasts(&app), ["glab (gitlab.com): invalid_token"]);
    assert!(app.outages.get(&gitlab_outage()).is_some());
}

#[test]
fn r_a_full_reload_and_a_lookup_in_flight_never_end_a_pr_outage() {
    let mut app = app_with(&[pushed("app")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    land_pr(&mut app, "app", &gl("app"), invalid_token(), Instant::now());
    for full in [false, true] {
        app.forget_pr_lookups((!full).then_some(path));
        assert_eq!(app.due_pr_lookups().len(), 1, "full={full}: looks again");
        assert!(app.outages.get(&gitlab_outage()).is_some(), "full={full}");
        assert_eq!(app.pr_mark(path), Some(PrMark::LookupFailed), "full={full}");
        assert_eq!(outage_toasts(&app).len(), 1, "full={full}");
        land_pr(&mut app, "app", &gl("app"), invalid_token(), Instant::now());
    }
    assert_ne!(app.refresh_pull_requests(None), Effect::None);
    assert!(app.outages.get(&gitlab_outage()).is_some(), "in flight");
    assert_eq!(outage_toasts(&app).len(), 1, "never a second toast");
}

#[test]
fn a_branch_switch_of_the_only_failing_checkout_ends_the_outage() {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    // `app` is the only checkout that fails on gitlab.com; `lib` works.
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t0);
    land_pr(&mut app, "lib", &gl("lib"), open_on(&gl("lib"), 2), t0);
    assert!(app.outages.get(&gitlab_outage()).is_some());

    // `app` switches branch. No checkout vanished, so only the replaced
    // record ends the outage.
    apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
    assert!(
        app.outages.get(&gitlab_outage()).is_some(),
        "the outage waits for the lookup sync"
    );
    let _ = app.due_pr_lookups();
    assert_eq!(app.pr_mark(Path::new("app")), None, "a new branch");
    assert!(app.outages.is_empty(), "the header part goes");
    assert!(outage_toasts(&app).is_empty(), "the toast goes");
}

#[test]
fn a_vanished_checkout_that_was_the_only_failing_one_ends_the_outage() {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    land_pr(&mut app, "app", &gl("app"), open_on(&gl("app"), 1), t0);
    land_pr(&mut app, "lib", &gl("lib"), invalid_token(), t0);
    assert!(app.outages.get(&gitlab_outage()).is_some());

    // `lib` vanishes; `app` keeps its branch, so no record is replaced.
    apply_one_repo_snapshot(&mut app, "lib", None);
    let _ = app.due_pr_lookups();
    assert!(app.outages.is_empty(), "the header part goes");
    assert!(outage_toasts(&app).is_empty(), "the toast goes");
}

#[test]
fn a_failed_lookup_is_polled_until_it_works_also_with_no_upstream() {
    // `app` was pushed; `wip` has no upstream, so with no PR known it
    // is not polled for a PR.
    let mut app = app_with(&[pushed("app"), repo("wip", "feature")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    // The token expired at launch: both fail on gitlab.com.
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t0);
    land_pr(&mut app, "wip", &gl("wip"), invalid_token(), t0);
    assert_eq!(outage_toasts(&app).len(), 1);

    // The poll cycle retries both, under the host backoff.
    let t1 = t0 + secs(180);
    assert_eq!(polled(&mut app, t1), ["app", "wip"]);
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t1);
    land_pr(&mut app, "wip", &gl("wip"), invalid_token(), t1);
    assert!(
        polled(&mut app, t1 + secs(180)).is_empty(),
        "the failing host backs off"
    );
    // The token works again: the next cycle retries both.
    let t2 = t1 + secs(360);
    assert_eq!(polled(&mut app, t2), ["app", "wip"]);
    land_pr(&mut app, "app", &gl("app"), PrLookup::NoPr, t2);
    assert!(
        app.outages.get(&gitlab_outage()).is_some(),
        "`wip` still fails"
    );
    land_pr(&mut app, "wip", &gl("wip"), PrLookup::NoPr, t2);
    assert!(app.outages.is_empty(), "the header part goes");
    assert!(outage_toasts(&app).is_empty(), "the toast goes");
    assert_eq!(app.pr_mark(Path::new("wip")), None);

    // Working again, the branch with no upstream is not polled.
    assert_eq!(polled(&mut app, t2 + secs(180)), ["app"]);
}

#[test]
fn a_merged_pr_whose_later_lookup_failed_is_polled_until_it_works() {
    let mut app = app_with(&[pushed("app")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    let merged = || PrLookup::Found(pr_on(&gl("app"), 1, PrState::Merged));
    land_pr(&mut app, "app", &gl("app"), merged(), t0);
    assert!(
        polled(&mut app, t0 + secs(180)).is_empty(),
        "a merged PR is not polled"
    );

    // `r` looks it up again, and that lookup fails.
    app.forget_pr_lookups(Some(path));
    assert_eq!(app.due_pr_lookups().len(), 1);
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t0 + secs(200));
    assert!(app.outages.get(&gitlab_outage()).is_some());
    assert_eq!(
        app.pr_mark(path),
        Some(PrMark::State(PrState::Merged)),
        "the badge stays"
    );

    let t1 = t0 + secs(400);
    assert_eq!(
        polled(&mut app, t1),
        ["app"],
        "the failed lookup is retried"
    );
    land_pr(&mut app, "app", &gl("app"), merged(), t1);
    assert!(app.outages.is_empty(), "the header part goes");
    assert!(outage_toasts(&app).is_empty(), "the toast goes");
    assert!(
        polled(&mut app, t1 + secs(400)).is_empty(),
        "merged and working: not polled"
    );
}

#[test]
fn gx_on_a_new_branch_ends_the_outage_of_the_old_branchs_failure() {
    let mut app = app_with(&[pushed("app")], &[]);
    let path = Path::new("app");
    let _ = app.due_pr_lookups();
    land_pr(&mut app, "app", &gl("app"), invalid_token(), Instant::now());
    assert!(app.outages.get(&gitlab_outage()).is_some());

    // `app` switches branch, and `gx` lands before the lookup sync: its
    // record of `feature` is replaced. The `gx` lookup fails on another
    // host, so only the replaced record can end the GitLab outage.
    apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
    assert!(app.outages.get(&gitlab_outage()).is_some());
    app.apply_pr_open(
        path,
        "other",
        Some(gh("app")),
        PrLookup::Failed(Some("HTTP 401".into())),
        PrPage::Pr,
        false,
    );
    assert!(app.outages.get(&gitlab_outage()).is_none());
    assert!(app.outages.get(&github_outage()).is_some());
    assert_eq!(outage_toasts(&app), ["gh (github.com): HTTP 401"]);
}

#[test]
fn a_failure_on_a_moved_remote_ends_the_outage_of_the_old_host() {
    let mut app = app_with(&[pushed("app")], &[]);
    let _ = app.due_pr_lookups();
    let t0 = Instant::now();
    land_pr(&mut app, "app", &gl("app"), invalid_token(), t0);
    assert!(app.outages.get(&gitlab_outage()).is_some());

    // The remote moved to GitHub, which fails too: the record now fails
    // on github.com only, so the GitLab outage ends.
    land_pr(&mut app, "app", &gh("app"), PrLookup::Failed(None), t0);
    assert!(app.outages.get(&gitlab_outage()).is_none());
    assert!(app.outages.get(&github_outage()).is_some());
    assert_eq!(outage_toasts(&app), ["gh (github.com): lookup failed"]);
    assert_eq!(app.pr_mark(Path::new("app")), Some(PrMark::LookupFailed));
}

#[test]
fn a_failed_gx_lookup_starts_the_outage_and_a_good_one_ends_it() {
    let mut app = app_with(&[pushed("app")], &[]);
    let path = Path::new("app");
    let open = |app: &mut AppState, lookup| {
        app.apply_pr_open(
            path,
            "feature",
            Some(GL_REMOTE.into()),
            lookup,
            PrPage::Pr,
            false,
        );
    };
    open(&mut app, invalid_token());
    assert_eq!(outage_toasts(&app), ["glab (gitlab.com): invalid_token"]);
    assert_eq!(
        app.status, "could not look up PR for feature: invalid_token",
        "gx still names the error on the status line"
    );
    assert_eq!(app.pr_mark(path), Some(PrMark::LookupFailed));
    open(&mut app, PrLookup::NoPr);
    assert!(app.outages.is_empty());
    assert_eq!(app.pr_mark(path), None);
}

#[test]
fn a_failed_lookup_with_no_forge_host_shares_one_unknown_outage() {
    let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
    let _ = app.due_pr_lookups();
    let now = Instant::now();
    for name in ["app", "lib"] {
        assert!(app.apply_pr_lookup_at(
            Path::new(name),
            "feature",
            None,
            PrLookup::Failed(None),
            now
        ));
    }
    let unknown = OutageSource::PrLookup {
        tool: "PR",
        host: "unknown".into(),
    };
    assert!(app.outages.get(&unknown).is_some());
    assert_eq!(outage_toasts(&app), ["PR (unknown): lookup failed"]);
}
