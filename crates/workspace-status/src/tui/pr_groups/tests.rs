use super::*;
use crate::tui::pr_status::PrNotes;

const APP: &str = "https://github.com/acme/app/pull";
const MERGER: &str = "https://github.com/acme/merger/pull";
const API: &str = "https://gitlab.com/acme/services/api/-/merge_requests";

/// An entry for `number` of the repository behind `url_base`, in the
/// checkout `/ws/<repo>`.
fn entry(
    repo: &str,
    url_base: &str,
    number: u64,
    head: &str,
    title: &str,
    state: PrState,
) -> PrEntry {
    let mut pr = PullRequest::fixture(number, &format!("{url_base}/{number}"), state);
    pr.head = head.into();
    pr.title = title.into();
    PrEntry {
        checkout: PathBuf::from(format!("/ws/{repo}")),
        repo: repo.into(),
        pr,
        default_branch: None,
    }
}

fn app(number: u64, head: &str, title: &str, state: PrState) -> PrEntry {
    entry("acme/app", APP, number, head, title, state)
}

fn keys(groups: &[PrGroup]) -> Vec<&str> {
    groups.iter().map(|group| group.key.as_str()).collect()
}

fn numbers(group: &PrGroup) -> Vec<u64> {
    group.rows.iter().map(|row| row.pr.number).collect()
}

fn header(group: &PrGroup) -> String {
    match &group.headline {
        GroupHeadline::Next(step) => step.text(),
        other => panic!("not an open group: {other:?}"),
    }
}

// ---- key ladder -------------------------------------------------

#[test]
fn key_ladder_branch_rule() {
    for (branch, key) in [
        ("feature/ABC-123-retry", Some("ABC-123")),
        ("ABC-123", Some("ABC-123")),
        ("ABC-123_retry", Some("ABC-123")),
        ("ABC-123.retry", Some("ABC-123")),
        ("feature/ABC-123", Some("ABC-123")),
        ("bugfix/ABC-123/retry", Some("ABC-123")),
        ("user/ABC-123/ABC-125-x", Some("ABC-123")),
        ("feature/A1-2-x", Some("A1-2")),
        ("feature/ABCDEFGHIJ-1-x", Some("ABCDEFGHIJ-1")),
        // The key is not at the start of its segment.
        ("feature/fix-ABC-123", None),
        ("feature/xABC-123", None),
        // The key does not end at a boundary.
        ("feature/ABC-123x", None),
        ("feature/ABC-123+1", None),
        // Wrong shape.
        ("feature/abc-123-retry", None),
        ("feature/ABC-retry", None),
        ("feature/A-1", None),
        ("feature/ABCDEFGHIJK-1", None),
        ("main", None),
        ("", None),
    ] {
        assert_eq!(branch_key(branch), key, "branch {branch:?}");
    }
}

#[test]
fn standard_prefixes_never_count_as_keys() {
    for letters in NOT_KEYS {
        assert_eq!(
            ticket_key(&format!("fix/{letters}-2024-1234"), ""),
            None,
            "{letters} in a branch"
        );
        assert_eq!(
            ticket_key("topic", &format!("fix({letters}-1): x")),
            None,
            "{letters} in a scope"
        );
        assert_eq!(
            ticket_key("topic", &format!("{letters}-1: x")),
            None,
            "{letters} as a prefix"
        );
        assert_eq!(
            ticket_key("topic", &format!("[{letters}-1] x")),
            None,
            "{letters} in brackets"
        );
    }
    assert_eq!(ticket_key("fix/CVE-2024-1234", "fix: patch"), None);
    assert_eq!(ticket_key("topic", "UTF-8: handle it"), None);
    assert_eq!(ticket_key("topic", "chore(deps): bump SHA-256 dep"), None);
    // A longer name that only starts with the letters is a key.
    assert_eq!(
        ticket_key("feature/CVEX-1-x", ""),
        Some("CVEX-1".to_string())
    );
    assert_eq!(ticket_key("feature/MDX-7", ""), Some("MDX-7".to_string()));
}

#[test]
fn key_ladder_commit_scope_rule() {
    for (title, key) in [
        ("feat(ABC-123): add retry", Some("ABC-123")),
        ("fix(api,ABC-123): add retry", Some("ABC-123")),
        ("fix(api, ABC-123): add retry", Some("ABC-123")),
        ("fix(api/ABC-123): add retry", Some("ABC-123")),
        ("fix(ABC-125,ABC-123): add retry", Some("ABC-125")),
        ("FIX(ABC-123)!: add retry", Some("ABC-123")),
        ("feat(ABC-123) add retry", None),
        ("feat(api): add retry", None),
        ("feat(api-ABC-123): add retry", None),
        ("feat(ABC-123x): add retry", None),
        ("feat(): add retry", None),
        ("improve(ABC-123): add retry", None),
        ("(ABC-123): add retry", None),
    ] {
        assert_eq!(
            ticket_key("topic", title),
            key.map(str::to_string),
            "title {title:?}"
        );
    }
}

#[test]
fn key_ladder_title_prefix_rule() {
    for (title, key) in [
        ("ABC-123: add retry", Some("ABC-123")),
        ("[ABC-123] add retry", Some("ABC-123")),
        ("feat: ABC-123: add retry", Some("ABC-123")),
        ("feat: [ABC-123] add retry", Some("ABC-123")),
        ("fix(api): ABC-123: add retry", Some("ABC-123")),
        ("Draft: ABC-123: add retry", Some("ABC-123")),
        ("[Draft] [ABC-123] add retry", Some("ABC-123")),
        ("WIP: feat(ABC-123): add retry", Some("ABC-123")),
        ("ABC-123 add retry", None),
        ("ABC-123:add retry", None),
        ("[ABC-123]add retry", None),
        ("add retry for ABC-123", None),
        ("Reopen ABC-123: add retry", None),
    ] {
        assert_eq!(
            ticket_key("topic", title),
            key.map(str::to_string),
            "title {title:?}"
        );
    }
}

#[test]
fn key_ladder_first_rule_wins() {
    // Branch beats scope, scope beats prefix.
    assert_eq!(
        ticket_key("feature/ABC-123-x", "fix(ABC-125): y"),
        Some("ABC-123".to_string())
    );
    assert_eq!(
        ticket_key("topic", "fix(ABC-125): ABC-123: y"),
        Some("ABC-125".to_string())
    );
    // A branch that holds no key falls through to the title.
    assert_eq!(
        ticket_key("feature/fix-ABC-123", "ABC-125: y"),
        Some("ABC-125".to_string())
    );
    // The first segment with a key wins.
    assert_eq!(
        ticket_key("ABC-125/ABC-123-x", ""),
        Some("ABC-125".to_string())
    );
}

// ---- title cleanup ----------------------------------------------

#[test]
fn clean_title_strips_draft_marker_type_and_own_key() {
    let key = Some("ABC-123");
    for (title, cleaned) in [
        ("feat(ABC-123): add retry", "add retry"),
        ("feat: ABC-123: add retry", "add retry"),
        ("feat: [ABC-123] add retry", "add retry"),
        ("ABC-123: add retry", "add retry"),
        ("[ABC-123] add retry", "add retry"),
        ("[ABC-123] feat: add retry", "add retry"),
        ("Draft: feat(ABC-123): add retry", "add retry"),
        ("[Draft] [WIP] fix: add retry", "add retry"),
        ("wip: ABC-123: add retry", "add retry"),
        ("(DRAFT) ABC-123: add retry", "add retry"),
        ("  add retry  ", "add retry"),
        ("add retry", "add retry"),
    ] {
        assert_eq!(clean_title(title, key), cleaned, "title {title:?}");
    }
}

#[test]
fn clean_title_keeps_an_area_scope_as_scope_colon_description() {
    let key = Some("ABC-123");
    for (title, cleaned) in [
        ("fix(api): add retry", "api: add retry"),
        ("fix(api,ABC-123): add retry", "api: add retry"),
        ("fix(ABC-123,api): add retry", "api: add retry"),
        ("fix(api/ABC-123/ui): add retry", "api,ui: add retry"),
        ("fix(api/v2): add retry", "api/v2: add retry"),
        ("fix(api,ABC-125): add retry", "api,ABC-125: add retry"),
        ("fix(ABC-125): add retry", "ABC-125: add retry"),
        ("feat(api)!: ABC-123: add retry", "api: add retry"),
    ] {
        assert_eq!(clean_title(title, key), cleaned, "title {title:?}");
    }
}

#[test]
fn clean_title_without_a_ticket_key_keeps_other_keys_and_the_scope() {
    assert_eq!(
        clean_title("fix(ABC-123): add retry", None),
        "ABC-123: add retry"
    );
    assert_eq!(
        clean_title("ABC-123: add retry", None),
        "ABC-123: add retry"
    );
    assert_eq!(clean_title("Draft: feat: add retry", None), "add retry");
    // Another key is not the group's own key.
    assert_eq!(
        clean_title("ABC-125: add retry", Some("ABC-123")),
        "ABC-125: add retry"
    );
}

#[test]
fn clean_title_falls_back_to_the_full_title() {
    let key = Some("ABC-123");
    assert_eq!(clean_title("feat(ABC-123):", key), "feat(ABC-123):");
    assert_eq!(clean_title("ABC-123: ", key), "ABC-123:");
    assert_eq!(clean_title("Draft: ", key), "Draft:");
    assert_eq!(clean_title("", key), "");
    assert_eq!(clean_title("   ", key), "");
}

#[test]
fn clean_title_only_strips_a_known_commit_type() {
    assert_eq!(clean_title("Note: add retry", None), "Note: add retry");
    assert_eq!(clean_title("Fix: add retry", None), "add retry");
    assert_eq!(
        clean_title("docs(readme): add retry", None),
        "readme: add retry"
    );
}

// ---- grouping ---------------------------------------------------

#[test]
fn groups_are_made_by_key_shared_branch_and_other() {
    let groups = group_prs(vec![
        app(1, "feature/ABC-123-retry", "add retry", PrState::InReview),
        entry(
            "acme/merger",
            MERGER,
            2,
            "topic",
            "fix(ABC-123): x",
            PrState::InReview,
        ),
        app(3, "shared-topic", "one", PrState::InReview),
        entry(
            "acme/merger",
            MERGER,
            4,
            "shared-topic",
            "two",
            PrState::InReview,
        ),
        app(5, "lonely", "keyless", PrState::InReview),
        app(
            6,
            "develop",
            "keyless on a common branch",
            PrState::InReview,
        ),
    ]);
    let by_kind: Vec<(&str, GroupKind, Vec<u64>)> = groups
        .iter()
        .map(|group| (group.key.as_str(), group.kind, numbers(group)))
        .collect();
    assert_eq!(
        by_kind,
        vec![
            ("ABC-123", GroupKind::Ticket, vec![1, 2]),
            ("shared-topic", GroupKind::SharedBranch, vec![3, 4]),
            ("other", GroupKind::Other, vec![5, 6]),
        ]
    );
}

#[test]
fn a_shared_branch_needs_two_listed_prs_and_a_branch_that_is_not_common() {
    for branch in LONG_LIVED_BRANCHES {
        let groups = group_prs(vec![
            app(1, branch, "one", PrState::InReview),
            entry("acme/merger", MERGER, 2, branch, "two", PrState::InReview),
        ]);
        assert_eq!(keys(&groups), vec!["other"], "branch {branch}");
    }
    // One PR alone.
    let groups = group_prs(vec![app(1, "topic", "one", PrState::InReview)]);
    assert_eq!(keys(&groups), vec!["other"]);
    // No head branch from the forge.
    let groups = group_prs(vec![
        app(1, "", "one", PrState::InReview),
        entry("acme/merger", MERGER, 2, "", "two", PrState::InReview),
    ]);
    assert_eq!(keys(&groups), vec!["other"]);
}

#[test]
fn one_pr_listed_for_two_checkouts_is_not_a_shared_branch() {
    let mut worktree = app(1, "topic", "one", PrState::InReview);
    worktree.checkout = PathBuf::from("/ws/app-topic");
    let groups = group_prs(vec![app(1, "topic", "one", PrState::InReview), worktree]);
    assert_eq!(keys(&groups), vec!["other"]);
    assert_eq!(numbers(&groups[0]), vec![1, 1], "both checkouts are rows");
}

#[test]
fn a_shared_branch_spans_repositories_and_forges() {
    let groups = group_prs(vec![
        app(7, "topic", "one", PrState::InReview),
        entry(
            "acme/services/api",
            API,
            12,
            "topic",
            "two",
            PrState::InReview,
        ),
    ]);
    assert_eq!(keys(&groups), vec!["topic"]);
    assert_eq!(groups[0].kind, GroupKind::SharedBranch);
    assert_eq!(groups[0].rows.len(), 2);
}

#[test]
fn a_keyed_pr_does_not_count_toward_a_shared_branch() {
    // Same head branch, but the title of one names a key: the other has
    // no partner, so it is not a shared-branch group of one row.
    let groups = group_prs(vec![
        app(1, "topic", "ABC-123: one", PrState::InReview),
        entry("acme/merger", MERGER, 2, "topic", "two", PrState::InReview),
    ]);
    assert_eq!(keys(&groups), vec!["ABC-123", "other"]);
    // With a second keyless PR on the branch, they do share it.
    let groups = group_prs(vec![
        app(1, "topic", "ABC-123: one", PrState::InReview),
        entry("acme/merger", MERGER, 2, "topic", "two", PrState::InReview),
        entry(
            "acme/merger",
            MERGER,
            3,
            "topic",
            "three",
            PrState::InReview,
        ),
    ]);
    assert_eq!(keys(&groups), vec!["ABC-123", "topic"]);
    assert_eq!(numbers(&groups[1]), vec![2, 3]);
}

#[test]
fn a_group_named_other_by_branch_stays_apart_from_the_other_group() {
    let groups = group_prs(vec![
        app(1, "other", "one", PrState::InReview),
        entry("acme/merger", MERGER, 2, "other", "two", PrState::InReview),
        app(3, "lonely", "three", PrState::InReview),
    ]);
    let kinds: Vec<(&str, GroupKind)> = groups
        .iter()
        .map(|group| (group.key.as_str(), group.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("other", GroupKind::SharedBranch),
            ("other", GroupKind::Other)
        ]
    );
}

#[test]
fn rows_carry_the_cleaned_title_per_group_kind() {
    let groups = group_prs(vec![
        app(
            1,
            "feature/ABC-123-x",
            "feat(ABC-123): add retry",
            PrState::InReview,
        ),
        app(2, "topic", "Draft: fix(api): one", PrState::Draft),
        entry(
            "acme/merger",
            MERGER,
            3,
            "topic",
            "ABC-5 two",
            PrState::Draft,
        ),
        app(4, "solo", "chore: tidy", PrState::InReview),
    ]);
    let mut titles = Vec::new();
    for group in &groups {
        for row in &group.rows {
            titles.push((group.key.as_str(), row.title.as_str()));
        }
    }
    assert_eq!(
        titles,
        vec![
            ("ABC-123", "add retry"),
            ("topic", "api: one"),
            ("topic", "ABC-5 two"),
            ("other", "tidy"),
        ]
    );
}

// ---- next action of a group -------------------------------------

fn with_notes(mut entry: PrEntry, notes: PrNotes) -> PrEntry {
    entry.pr.notes = notes;
    entry
}

#[test]
fn header_text_is_the_action_and_the_pr_ref() {
    for (state, number, url, text) in [
        (PrState::CiFailed, 318, APP, "fix CI #318"),
        (PrState::Ready, 319, APP, "merge #319"),
        (PrState::Ready, 12, API, "merge !12"),
        (PrState::CiRunning, 320, APP, "wait for CI #320"),
        (PrState::InReview, 321, APP, "get review #321"),
        (PrState::Draft, 322, APP, "undraft #322"),
    ] {
        let groups = group_prs(vec![entry(
            "acme/app",
            url,
            number,
            "feature/ABC-123-x",
            "t",
            state,
        )]);
        assert_eq!(header(&groups[0]), text, "{state:?}");
    }
    let groups = group_prs(vec![with_notes(
        app(7, "feature/ABC-123-x", "t", PrState::InReview),
        PrNotes {
            conflicts: true,
            ..Default::default()
        },
    )]);
    assert_eq!(header(&groups[0]), "fix conflicts #7");
    let groups = group_prs(vec![with_notes(
        app(8, "feature/ABC-123-x", "t", PrState::InReview),
        PrNotes {
            changes_requested: true,
            ..Default::default()
        },
    )]);
    assert_eq!(header(&groups[0]), "address review #8");
    let groups = group_prs(vec![with_notes(
        app(9, "feature/ABC-123-x", "t", PrState::InReview),
        PrNotes {
            behind: true,
            ..Default::default()
        },
    )]);
    assert_eq!(header(&groups[0]), "update branch #9");
    assert_eq!(group_rank(&groups[0]), 1, "update branch needs you");
}

#[test]
fn header_merge_base_first_names_the_base_ref_once() {
    let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
    child.pr.base = "feature/ABC-123-a".into();
    let base = app(318, "feature/ABC-123-a", "t", PrState::InReview);
    let groups = group_prs(vec![child, base]);
    let GroupHeadline::Next(step) = &groups[0].headline else {
        panic!("open group");
    };
    assert_eq!(step.text(), "merge #318 first");
    assert_eq!(step.pr_ref, "#319", "it targets the stacked PR");
    assert!(step.needs_you());
}

#[test]
fn header_next_action_priority_is_fix_ci_then_conflicts_then_review_then_merge() {
    let conflicts = PrNotes {
        conflicts: true,
        ..Default::default()
    };
    let review = PrNotes {
        changes_requested: true,
        ..Default::default()
    };
    // Row order is 1, 2, 3, 4, 5; the lowest priority number leads.
    let rows = |states: [(PrState, PrNotes); 5]| {
        let entries = states
            .into_iter()
            .enumerate()
            .map(|(index, (state, notes))| {
                with_notes(
                    app(index as u64 + 1, "feature/ABC-123-x", "t", state),
                    notes,
                )
            })
            .collect();
        group_prs(entries)
    };
    let none = PrNotes::default();
    let groups = rows([
        (PrState::Ready, none),
        (PrState::InReview, review),
        (PrState::InReview, conflicts),
        (PrState::Draft, none),
        (PrState::CiFailed, none),
    ]);
    assert_eq!(
        header(&groups[0]),
        "fix CI #5",
        "CI failed from a later row"
    );
    let groups = rows([
        (PrState::Ready, none),
        (PrState::InReview, review),
        (PrState::InReview, conflicts),
        (PrState::Draft, none),
        (PrState::CiRunning, none),
    ]);
    assert_eq!(header(&groups[0]), "fix conflicts #3");
    let groups = rows([
        (PrState::Ready, none),
        (PrState::InReview, review),
        (PrState::Draft, none),
        (PrState::CiRunning, none),
        (PrState::InReview, none),
    ]);
    assert_eq!(header(&groups[0]), "address review #2");
    let groups = rows([
        (PrState::Draft, none),
        (PrState::Ready, none),
        (PrState::CiRunning, none),
        (PrState::Ready, none),
        (PrState::InReview, none),
    ]);
    assert_eq!(
        header(&groups[0]),
        "merge #2",
        "first ready row in row order"
    );
}

#[test]
fn header_merge_and_merge_base_first_share_one_slot_in_row_order() {
    // The base sorts first, so the header targets the base: `merge #318`,
    // not the child's `merge #318 first`.
    let base = app(318, "feature/ABC-123-a", "t", PrState::Ready);
    let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
    child.pr.base = "feature/ABC-123-a".into();
    let groups = group_prs(vec![child.clone(), base]);
    assert_eq!(numbers(&groups[0]), vec![318, 319]);
    assert_eq!(header(&groups[0]), "merge #318");
    let GroupHeadline::Next(step) = &groups[0].headline else {
        panic!("open group");
    };
    assert_eq!(step.pr_ref, "#318", "it targets the base");
    // With a base that is not ready, the child's action is the one.
    let base = app(318, "feature/ABC-123-a", "t", PrState::InReview);
    let groups = group_prs(vec![child, base]);
    assert_eq!(header(&groups[0]), "merge #318 first");
}

#[test]
fn header_without_a_needs_you_row_is_the_first_open_rows_action() {
    let groups = group_prs(vec![
        app(1, "feature/ABC-123-x", "t", PrState::Merged),
        app(2, "feature/ABC-123-x", "t", PrState::CiRunning),
        app(3, "feature/ABC-123-x", "t", PrState::Draft),
        app(4, "feature/ABC-123-x", "t", PrState::InReview),
    ]);
    assert_eq!(header(&groups[0]), "wait for CI #2");
    let GroupHeadline::Next(step) = &groups[0].headline else {
        panic!("open group");
    };
    assert_eq!(step.state, PrState::CiRunning, "the state of the target");
    assert!(!step.needs_you());
}

#[test]
fn a_group_of_merged_prs_is_finished_and_counts_them() {
    let groups = group_prs(vec![
        app(1, "feature/ABC-123-x", "t", PrState::Merged),
        app(2, "feature/ABC-123-y", "t", PrState::Merged),
    ]);
    assert_eq!(groups[0].headline, GroupHeadline::AllMerged(2));
    assert!(groups[0].is_finished());
    // One open row makes it an open group; merged rows are not targets.
    let groups = group_prs(vec![
        app(1, "feature/ABC-123-x", "t", PrState::Merged),
        app(2, "feature/ABC-123-y", "t", PrState::InReview),
    ]);
    assert!(!groups[0].is_finished());
    assert_eq!(header(&groups[0]), "get review #2");
}

#[test]
fn all_merged_counts_distinct_prs_not_checkouts() {
    // #1 is listed for two checkouts: one PR.
    let mut worktree = app(1, "feature/ABC-123-x", "t", PrState::Merged);
    worktree.checkout = PathBuf::from("/ws/app-wt");
    let groups = group_prs(vec![
        app(1, "feature/ABC-123-x", "t", PrState::Merged),
        worktree,
        app(2, "feature/ABC-123-y", "t", PrState::Merged),
    ]);
    assert_eq!(groups[0].rows.len(), 3, "every checkout is a row");
    assert_eq!(groups[0].headline, GroupHeadline::AllMerged(2));
    // The same number in another repository is another PR.
    let groups = group_prs(vec![
        app(1, "feature/ABC-123-x", "t", PrState::Merged),
        entry(
            "acme/merger",
            MERGER,
            1,
            "feature/ABC-123-x",
            "t",
            PrState::Merged,
        ),
    ]);
    assert_eq!(groups[0].headline, GroupHeadline::AllMerged(2));
}

#[test]
fn the_other_group_has_no_header_text_and_is_never_finished() {
    let groups = group_prs(vec![app(1, "solo", "t", PrState::Merged)]);
    assert_eq!(groups[0].kind, GroupKind::Other);
    assert_eq!(groups[0].headline, GroupHeadline::Bare);
    assert!(!groups[0].is_finished());
}

// ---- order ------------------------------------------------------

#[test]
fn groups_sort_ci_failed_then_needs_you_then_waiting_then_finished_then_other() {
    let groups = group_prs(vec![
        // other, merged
        app(1, "solo", "t", PrState::Merged),
        // finished
        app(2, "feature/ABC-100-a", "t", PrState::Merged),
        // waiting
        app(3, "feature/ABC-101-a", "t", PrState::InReview),
        // needs you: merge
        app(4, "feature/ABC-102-a", "t", PrState::Ready),
        // CI failed
        app(5, "feature/ABC-103-a", "t", PrState::CiFailed),
        // needs you: a later key than the first needs-you group
        app(6, "feature/ABC-099-a", "t", PrState::Ready),
        // CI failed in one row of a group with a ready row
        app(7, "feature/ABC-104-a", "t", PrState::Ready),
        app(8, "feature/ABC-104-b", "t", PrState::CiFailed),
        // waiting, earlier key
        app(9, "feature/ABC-098-a", "t", PrState::Draft),
    ]);
    assert_eq!(
        keys(&groups),
        vec!["ABC-103", "ABC-104", "ABC-099", "ABC-102", "ABC-098", "ABC-101", "ABC-100", "other"]
    );
}

#[test]
fn groups_with_the_same_rank_sort_by_key_ascending() {
    let groups = group_prs(vec![
        app(1, "feature/ABC-125-a", "t", PrState::Ready),
        app(2, "feature/ABC-123-a", "t", PrState::Ready),
        app(3, "shared", "t", PrState::Ready),
        entry("acme/merger", MERGER, 4, "shared", "t", PrState::Ready),
    ]);
    assert_eq!(keys(&groups), vec!["ABC-123", "ABC-125", "shared"]);
}

#[test]
fn group_keys_sort_in_natural_order_not_text_order() {
    // Not zero-padded, so a text sort would put ABC-1000 and ABC-100
    // before ABC-9.
    let groups = group_prs(vec![
        app(1, "feature/ABC-1000-a", "t", PrState::InReview),
        app(2, "feature/ABC-999-a", "t", PrState::InReview),
        app(3, "feature/ABC-10-a", "t", PrState::InReview),
        app(4, "feature/ABC-9-a", "t", PrState::InReview),
        app(5, "feature/ABC-100-a", "t", PrState::InReview),
        app(6, "feature/AB-5-a", "t", PrState::InReview),
        app(7, "feature/ABD-1-a", "t", PrState::InReview),
    ]);
    assert_eq!(
        keys(&groups),
        vec!["AB-5", "ABC-9", "ABC-10", "ABC-100", "ABC-999", "ABC-1000", "ABD-1"]
    );
    // The same inside one rank: a CI failed group still leads.
    let groups = group_prs(vec![
        app(1, "feature/ABC-1000-a", "t", PrState::CiFailed),
        app(2, "feature/ABC-999-a", "t", PrState::CiFailed),
        app(3, "feature/ABC-20-a", "t", PrState::Ready),
    ]);
    assert_eq!(keys(&groups), vec!["ABC-999", "ABC-1000", "ABC-20"]);
}

#[test]
fn natural_key_order_compares_letters_then_number_then_rest() {
    use std::cmp::Ordering::{Equal, Greater, Less};
    for (a, b, order) in [
        ("ABC-999", "ABC-1000", Less),
        ("ABC-9", "ABC-10", Less),
        ("ABC-10", "ABC-9", Greater),
        ("AB-100", "ABC-1", Less),
        ("ABC-007", "ABC-8", Less),
        ("ABC-7", "ABC-7", Equal),
        // The same number: the whole key decides, so the order is total.
        ("ABC-007", "ABC-7", Less),
        // A rest after the number is compared as text.
        ("topic-2-a", "topic-2-b", Less),
        ("topic-2-b", "topic-10-a", Less),
        // No number: the key is text, before the same text with one.
        ("shared", "shared-1", Less),
        ("my-topic", "my-topic-3", Less),
        ("my-topic-3", "my-topic-12", Less),
        // A digit run longer than any integer type still compares.
        (
            "x-99999999999999999999999",
            "x-100000000000000000000000",
            Less,
        ),
    ] {
        assert_eq!(natural_key_cmp(a, b), order, "{a} vs {b}");
        assert_eq!(natural_key_cmp(b, a), order.reverse(), "{b} vs {a}");
    }
}

#[test]
fn rows_sort_by_repo_then_number() {
    let groups = group_prs(vec![
        entry(
            "acme/merger",
            MERGER,
            1,
            "feature/ABC-123-a",
            "t",
            PrState::InReview,
        ),
        app(30, "feature/ABC-123-b", "t", PrState::InReview),
        app(4, "feature/ABC-123-c", "t", PrState::InReview),
        entry(
            "acme/services/api",
            API,
            2,
            "feature/ABC-123-d",
            "t",
            PrState::InReview,
        ),
    ]);
    let rows: Vec<(&str, u64)> = groups[0]
        .rows
        .iter()
        .map(|row| (row.repo.as_str(), row.pr.number))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("acme/app", 4),
            ("acme/app", 30),
            ("acme/merger", 1),
            ("acme/services/api", 2)
        ]
    );
}

#[test]
fn a_stacked_row_sorts_right_after_its_base() {
    // #10 is stacked on #12, #12 on #11.
    let mut ten = app(10, "feature/ABC-123-c", "t", PrState::InReview);
    ten.pr.base = "feature/ABC-123-b".into();
    let mut twelve = app(12, "feature/ABC-123-b", "t", PrState::InReview);
    twelve.pr.base = "feature/ABC-123-a".into();
    let eleven = app(11, "feature/ABC-123-a", "t", PrState::InReview);
    let plain = app(13, "feature/ABC-123-x", "t", PrState::InReview);
    let groups = group_prs(vec![ten, twelve, eleven, plain]);
    assert_eq!(numbers(&groups[0]), vec![11, 12, 10, 13]);
    let stacked: Vec<Option<&str>> = groups[0]
        .rows
        .iter()
        .map(|row| row.stacked_on.as_ref().map(|base| base.pr_ref.as_str()))
        .collect();
    assert_eq!(stacked, vec![None, Some("#11"), Some("#12"), None]);
}

#[test]
fn a_stack_across_groups_keeps_the_row_order_and_the_stack_mark() {
    // The base is in another group (its title names another key).
    let base = app(318, "topic-a", "ABC-125: base", PrState::CiFailed);
    let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
    child.pr.base = "topic-a".into();
    let groups = group_prs(vec![base, child]);
    let child_group = groups.iter().find(|group| group.key == "ABC-123").unwrap();
    let row = &child_group.rows[0];
    assert_eq!(
        row.stacked_on,
        Some(StackedOn {
            pr_ref: "#318".into(),
            number: 318,
            state: PrState::CiFailed
        })
    );
    assert_eq!(
        row.next_action,
        Some(NextAction::MergeBaseFirst("#318".into()))
    );
}

#[test]
fn a_stack_is_found_in_the_same_repository_only() {
    let base = entry(
        "acme/merger",
        MERGER,
        318,
        "feature/ABC-123-a",
        "t",
        PrState::InReview,
    );
    let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
    child.pr.base = "feature/ABC-123-a".into();
    let groups = group_prs(vec![base, child]);
    let child_row = groups[0]
        .rows
        .iter()
        .find(|row| row.pr.number == 319)
        .unwrap();
    assert_eq!(child_row.stacked_on, None);
    assert_eq!(child_row.next_action, Some(NextAction::Merge));
}

#[test]
fn a_merged_base_keeps_the_stack_mark_but_not_merge_first() {
    let base = app(318, "feature/ABC-123-a", "t", PrState::Merged);
    let mut child = app(319, "feature/ABC-123-b", "t", PrState::Ready);
    child.pr.base = "feature/ABC-123-a".into();
    let groups = group_prs(vec![base, child]);
    let row = groups[0]
        .rows
        .iter()
        .find(|row| row.pr.number == 319)
        .unwrap();
    assert_eq!(row.stacked_on.as_ref().unwrap().state, PrState::Merged);
    assert_eq!(row.next_action, Some(NextAction::Merge));
    assert_eq!(numbers(&groups[0]), vec![318, 319]);
}

#[test]
fn the_configured_default_branch_is_never_a_stack_base() {
    // `release` is open as a PR into `main`; `#11` targets `release`.
    let release = app(10, "release", "ABC-123: release", PrState::InReview);
    let mut child = app(11, "feature/ABC-123-a", "t", PrState::Ready);
    child.pr.base = "release".into();
    let child_row = |default_branch: Option<&str>| {
        let mut child = child.clone();
        child.default_branch = default_branch.map(str::to_string);
        let groups = group_prs(vec![release.clone(), child]);
        assert_eq!(numbers(&groups[0]), vec![10, 11]);
        let row = groups[0].rows[1].clone();
        (header(&groups[0]), row)
    };

    // Not configured and not a long-lived name: `release` is a base.
    let (header, row) = child_row(None);
    assert_eq!(header, "merge #10 first");
    assert_eq!(row.stacked_on.map(|base| base.pr_ref), Some("#10".into()));
    assert_eq!(
        row.next_action,
        Some(NextAction::MergeBaseFirst("#10".into()))
    );

    // Configured as the default branch: no stack; rows and header say so.
    let (header, row) = child_row(Some("release"));
    assert_eq!(header, "merge #11");
    assert_eq!(row.stacked_on, None);
    assert_eq!(row.next_action, Some(NextAction::Merge));

    // Another default branch changes nothing for this base.
    let (_, row) = child_row(Some("main"));
    assert!(row.stacked_on.is_some());
}

#[test]
fn a_pr_into_a_long_lived_branch_is_not_stacked_on_a_pr_from_it() {
    // `develop → main` is open; `#11` targets `develop`.
    let develop = app(10, "develop", "ABC-123: promote", PrState::InReview);
    let mut child = app(11, "feature/ABC-123-a", "t", PrState::Ready);
    child.pr.base = "develop".into();
    for default_branch in [None, Some("develop".to_string()), Some("main".to_string())] {
        let mut child = child.clone();
        child.default_branch = default_branch.clone();
        let groups = group_prs(vec![develop.clone(), child]);
        let row = groups[0]
            .rows
            .iter()
            .find(|row| row.pr.number == 11)
            .unwrap();
        assert_eq!(row.stacked_on, None, "default {default_branch:?}");
        assert_eq!(row.next_action, Some(NextAction::Merge));
        assert_eq!(header(&groups[0]), "merge #11");
    }
}

#[test]
fn rows_that_stack_on_each_other_in_a_loop_still_all_show() {
    let mut a = app(1, "branch-a", "ABC-123: a", PrState::InReview);
    a.pr.base = "branch-b".into();
    let mut b = app(2, "branch-b", "ABC-123: b", PrState::InReview);
    b.pr.base = "branch-a".into();
    let groups = group_prs(vec![a, b]);
    assert_eq!(numbers(&groups[0]), vec![1, 2]);
}

#[test]
fn listed_counts_every_row_and_the_fold_default_starts_above_twelve() {
    let make = |count: u64| {
        let mut entries: Vec<PrEntry> = (1..=count)
            .map(|number| app(number, "feature/ABC-123-x", "t", PrState::Merged))
            .collect();
        entries.push(app(1000, "solo", "t", PrState::InReview));
        group_prs(entries)
    };
    let twelve = make(11);
    assert_eq!(listed(&twelve), 12);
    assert!(!finished_start_folded(&twelve));
    let thirteen = make(12);
    assert_eq!(listed(&thirteen), 13);
    assert!(finished_start_folded(&thirteen));
    assert_eq!(listed(&[]), 0);
    assert!(!finished_start_folded(&[]));
}

// ---- title command ----------------------------------------------

#[cfg(unix)]
mod title_command {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// A temp dir with one executable `script` that holds `body`.
    struct Script {
        dir: PathBuf,
        path: PathBuf,
    }

    impl Script {
        fn new(body: &str) -> Self {
            let dir = crate::testutil::unique_dir("ws-pr-title");
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("script");
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            Self { dir, path }
        }
    }

    impl Drop for Script {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn run(body: &str, key: &str) -> Option<String> {
        run_title_command(&Script::new(body).path, key)
    }

    #[test]
    fn the_first_stdout_line_is_the_title() {
        assert_eq!(
            run("echo \"Retry the upload ($1)\"", "ABC-123"),
            Some("Retry the upload (ABC-123)".to_string())
        );
        assert_eq!(
            run("printf 'First\\nSecond\\n'", "ABC-123"),
            Some("First".to_string())
        );
        assert_eq!(
            run("printf '  padded \\r\\n'", "ABC-123"),
            Some("padded".to_string())
        );
        assert_eq!(
            run("printf 'No newline'", "ABC-123"),
            Some("No newline".to_string())
        );
    }

    #[test]
    fn the_key_is_the_only_argument_and_no_shell_reads_it() {
        let body = "printf '%s|%s' \"$#\" \"$1\"";
        assert_eq!(run(body, "ABC-123"), Some("1|ABC-123".to_string()));
        assert_eq!(
            run(body, "$(echo hi);ls"),
            Some("1|$(echo hi);ls".to_string())
        );
    }

    #[test]
    fn control_characters_become_spaces() {
        assert_eq!(
            run("printf 'A\\033[1mB\\tC\\n'", "ABC-123"),
            Some("A [1mB C".to_string())
        );
    }

    #[test]
    fn a_non_zero_exit_gives_no_title_even_with_output() {
        assert_eq!(run("echo Title; exit 1", "ABC-123"), None);
        assert_eq!(run("echo Title; exit 127", "ABC-123"), None);
    }

    #[test]
    fn empty_output_gives_no_title() {
        assert_eq!(run("exit 0", "ABC-123"), None);
        assert_eq!(run("echo", "ABC-123"), None);
        assert_eq!(run("echo '   '", "ABC-123"), None);
        // Only the first line counts, so an empty one is no title.
        assert_eq!(run("printf '\\nSecond\\n'", "ABC-123"), None);
    }

    #[test]
    fn stderr_is_ignored() {
        assert_eq!(
            run("echo warning >&2; echo Title", "ABC-123"),
            Some("Title".to_string())
        );
        assert_eq!(run("echo Title >&2", "ABC-123"), None);
    }

    #[test]
    fn a_missing_or_non_executable_file_gives_no_title() {
        let script = Script::new("echo Title");
        assert_eq!(
            run_title_command(&script.dir.join("no-such-file"), "ABC-123"),
            None
        );
        fs::set_permissions(&script.path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(run_title_command(&script.path, "ABC-123"), None);
        assert_eq!(run_title_command(&script.dir, "ABC-123"), None);
    }

    #[test]
    fn a_command_that_runs_too_long_is_killed_and_gives_no_title() {
        let script = Script::new("echo Title; exec sleep 5");
        let started = Instant::now();
        let title = run_title_command_within(&script.path, "ABC-123", Duration::from_millis(300));
        assert_eq!(title, None);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "stopped at the limit, took {:?}",
            started.elapsed()
        );
    }

    /// True while `pid` exists and is not a zombie: a killed orphan stays
    /// a zombie until init reaps it, and init may never reap in a
    /// container.
    fn process_runs(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        // Linux: the state letter follows the `(comm)` field.
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat
                .rsplit_once(')')
                .is_none_or(|(_, rest)| !rest.trim_start().starts_with('Z')),
            Err(_) => true,
        }
    }

    #[test]
    fn a_timeout_ends_the_whole_process_group() {
        let dir = crate::testutil::unique_dir("ws-pr-title-group");
        fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("grandchild.pid");
        // No `exec`: the shell stays the child, and `sleep` runs below
        // it. Killing only the shell would leave both sleeps behind.
        let script = Script::new(&format!(
            "sleep 30 &\necho $! > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\nsleep 5\necho late",
            pid = pid_file.display()
        ));
        // A busy host may not start the script within the short limit;
        // retry with a longer one so the group check always runs.
        for limit in [500, 3000].map(Duration::from_millis) {
            let started = Instant::now();
            let title = run_title_command_within(&script.path, "ABC-123", limit);
            assert_eq!(title, None, "the title came after the limit");
            assert!(started.elapsed() < limit + Duration::from_secs(3));
            let Some(pid) = fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
            else {
                continue;
            };
            let gone_by = Instant::now() + Duration::from_secs(3);
            while process_runs(pid) {
                assert!(Instant::now() < gone_by, "the grandchild still runs");
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = fs::remove_dir_all(&dir);
            return;
        }
        panic!("the script never started");
    }

    /// A command that exits 0 but leaves a background child holding its
    /// stdout still gives its title, and the child is killed with the
    /// rest of the group, so no reader thread waits on the pipe.
    #[test]
    fn a_finished_command_ends_what_it_left_in_its_group() {
        let dir = crate::testutil::unique_dir("ws-pr-title-leftover");
        fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("leftover.pid");
        // The background `sleep` inherits stdout and outlives the shell.
        let script = Script::new(&format!(
            "sleep 30 &\necho $! > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\necho Title",
            pid = pid_file.display()
        ));
        let title = run_title_command_within(&script.path, "ABC-123", Duration::from_secs(10));
        assert_eq!(title, Some("Title".to_string()));
        let pid: libc::pid_t = fs::read_to_string(&pid_file)
            .expect("the script wrote the pid before its title")
            .trim()
            .parse()
            .unwrap();
        // The kill was sent before the run returned; only the reap by
        // init can lag, so this waits on a generous deadline.
        let gone_by = Instant::now() + Duration::from_secs(10);
        while process_runs(pid) {
            assert!(Instant::now() < gone_by, "the leftover child still runs");
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_running_title_command_sits_in_the_exit_kill_registry() {
        let dir = crate::testutil::unique_dir("ws-pr-title-registry");
        fs::create_dir_all(&dir).unwrap();
        let pid_file = dir.join("command.pid");
        // `$$` is the shell: the child and its process group leader.
        let script = Script::new(&format!(
            "echo $$ > '{pid}.tmp'\nmv '{pid}.tmp' '{pid}'\nsleep 1\necho Title",
            pid = pid_file.display()
        ));
        let path = script.path.clone();
        let run = std::thread::spawn(move || {
            run_title_command_within(&path, "ABC-123", Duration::from_secs(10))
        });
        let started_by = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Some(pid) = fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
            {
                break pid;
            }
            assert!(Instant::now() < started_by, "the script never started");
            std::thread::sleep(Duration::from_millis(10));
        };
        // The parent registers the group right after spawn returns,
        // which may be after the script wrote its pid: wait for it
        // while the script still sleeps.
        let tracked_by = Instant::now() + Duration::from_millis(900);
        while !crate::git::is_tracked(pid) {
            assert!(
                Instant::now() < tracked_by && !run.is_finished(),
                "a quit stops the running command's group"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(run.join().unwrap(), Some("Title".to_string()));
        assert!(!crate::git::is_tracked(pid), "reaped: its slot is free");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_inside_the_limit_still_gives_its_title() {
        let script = Script::new("sleep 0.2; echo Title");
        assert_eq!(
            run_title_command_within(&script.path, "ABC-123", Duration::from_secs(5)),
            Some("Title".to_string())
        );
    }

    #[test]
    fn a_command_with_a_lot_of_output_is_not_stalled() {
        // About 1 MB after the first line: far more than a pipe holds.
        let body = "echo Title; i=0; while [ $i -lt 20000 ]; do \
                        echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; \
                        i=$((i+1)); done";
        assert_eq!(run(body, "ABC-123"), Some("Title".to_string()));
    }

    #[test]
    fn a_very_long_first_line_is_cut() {
        let title = run("head -c 100000 /dev/zero | tr '\\0' 'x'; echo", "ABC-123").unwrap();
        assert_eq!(title.len(), TITLE_LINE_MAX as usize);
    }

    fn one_pr(branch: &str, number: u64, title: &str) -> Vec<PrEntry> {
        vec![app(number, branch, title, PrState::InReview)]
    }

    #[test]
    fn the_cache_runs_the_command_once_per_key_and_only_for_ticket_groups() {
        let dir = crate::testutil::unique_dir("ws-pr-title-calls");
        fs::create_dir_all(&dir).unwrap();
        let calls = dir.join("calls");
        let script = Script::new(&format!(
            "echo \"$1\" >> '{}'; if [ \"$1\" = ABC-125 ]; then exit 1; fi; echo \"Title of $1\"",
            calls.display()
        ));
        let mut entries = one_pr("feature/ABC-123-x", 1, "t");
        entries.extend(one_pr("feature/ABC-125-x", 2, "t"));
        entries.extend(one_pr("shared-topic", 3, "t"));
        entries.push(entry(
            "acme/merger",
            MERGER,
            4,
            "shared-topic",
            "t",
            PrState::InReview,
        ));
        entries.extend(one_pr("solo", 5, "t"));
        let groups = group_prs(entries);
        assert_eq!(
            keys(&groups),
            vec!["ABC-123", "ABC-125", "shared-topic", "other"]
        );

        let mut cache = TitleCache::default();
        cache.fill(&script.path, &groups);
        cache.fill(&script.path, &groups);

        assert_eq!(cache.get("ABC-123"), Some("Title of ABC-123"));
        assert_eq!(cache.get("ABC-125"), None, "the command failed for it");
        assert_eq!(cache.get("shared-topic"), None);
        assert_eq!(cache.get("other"), None);
        assert_eq!(cache.get("ABC-999"), None, "never asked");
        let mut called: Vec<String> = fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        called.sort();
        assert_eq!(
            called,
            vec!["ABC-123", "ABC-125"],
            "once per key, no shared branch, no other"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_cache_runs_the_command_again() {
        let dir = crate::testutil::unique_dir("ws-pr-title-cycles");
        fs::create_dir_all(&dir).unwrap();
        let calls = dir.join("calls");
        let script = Script::new(&format!("echo \"$1\" >> '{}'; echo T", calls.display()));
        let groups = group_prs(one_pr("feature/ABC-123-x", 1, "t"));
        TitleCache::default().fill(&script.path, &groups);
        TitleCache::default().fill(&script.path, &groups);
        assert_eq!(fs::read_to_string(&calls).unwrap().lines().count(), 2);
        let _ = fs::remove_dir_all(&dir);
    }
}
