use std::collections::HashSet;

use super::*;
use crate::helpers::visible_width;
use crate::snapshot::build_workspace_snapshot;
use crate::testutil::{focus_workspace_repos, repo_snapshot};
use crate::tui::tree::{
    build_focused_tree, build_tree, default_folds, flatten_with, painted_row_segments,
    workspace_trailing_fit, PrTotals, VisibleRow,
};

/// The focus demo workspace as a visible snapshot.
fn workspace() -> WorkspaceSnapshot {
    build_workspace_snapshot(&focus_workspace_repos(), &[], false, &[])
}

/// Root child ids of the tree built under `query`.
fn root_ids(query: &str) -> Vec<String> {
    let tree = build_focused_tree(&workspace(), true, "ws", Some(&TreeFocus::parse(query)));
    tree.children.into_iter().map(|child| child.id).collect()
}

/// Rows of the tree under `query`, every group open, ASCII glyphs.
fn open_rows(query: &str) -> Vec<VisibleRow> {
    let tree = build_focused_tree(&workspace(), true, "ws", Some(&TreeFocus::parse(query)));
    flatten_with(&tree, &HashSet::new(), true)
}

fn row<'a>(rows: &'a [VisibleRow], id: &str) -> &'a VisibleRow {
    rows.iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("no row {id}"))
}

/// S2: a feature focus keeps one worktree of `app` (`1/3 wt`) and
/// `services/api`; `lib` and `notes` fold into the last root child.
#[test]
fn feature_focus_puts_outside_families_in_the_last_group() {
    assert_eq!(
        root_ids("ABC-123"),
        ["repo:app", "repo:services/api", OUTSIDE_FOCUS_GROUP_ID]
    );
    let rows = open_rows("ABC-123");
    let outside = row(&rows, OUTSIDE_FOCUS_GROUP_ID);
    assert_eq!(outside.kind, NodeKind::Group);
    assert_eq!(outside.label, "2 outside focus · 1 dirty", "no glyph");
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "workspace",
            "repo:app",
            "checkout:app/.worktrees/abc-123",
            "repo:services/api",
            OUTSIDE_FOCUS_GROUP_ID,
            // Attention first (dirty `notes`), then the clean `lib`.
            "repo:notes",
            "file:notes:README.md",
            "repo:lib",
        ]
    );
    let app = row(&rows, "repo:app");
    assert!(app.label.contains("1/3 wt"), "{}", app.label);
}

/// A family the focus keeps whole shows `n wt`; with no focus the tree is
/// the plain tree.
#[test]
fn repo_focus_keeps_the_whole_family_and_no_focus_is_the_plain_tree() {
    let rows = open_rows("app");
    let app = row(&rows, "repo:app");
    assert!(app.label.contains("3 wt"), "{}", app.label);
    assert!(!app.label.contains("/3 wt"), "{}", app.label);
    let plain = build_tree(&workspace(), true, "ws");
    assert_eq!(build_focused_tree(&workspace(), true, "ws", None), plain);
    assert_eq!(
        build_focused_tree(&workspace(), true, "ws", Some(&TreeFocus::parse(" , "))),
        plain,
        "a query with no terms is no focus"
    );
    assert_eq!(plain.chrome.focus_chip, None);
}

/// The focused part keeps today's layout (attention, then `No updates`);
/// the outside group follows it, with no nested `No updates` group.
#[test]
fn focused_part_keeps_no_updates_and_the_outside_group_comes_last() {
    assert_eq!(
        root_ids("lib, app"),
        ["repo:app", "group:no-updates", OUTSIDE_FOCUS_GROUP_ID]
    );
    let tree = build_focused_tree(
        &workspace(),
        true,
        "ws",
        Some(&TreeFocus::parse("lib, app")),
    );
    let outside = tree.children.last().unwrap();
    let ids: Vec<&str> = outside.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["repo:notes", "repo:services/api"]);
    assert!(outside
        .children
        .iter()
        .all(|child| child.kind != NodeKind::Group));
    let chip = tree.chrome.focus_chip.expect("chip");
    assert_eq!(
        chip,
        FocusChip {
            query: "lib, app".into(),
            focused: 2,
            total: 4,
        }
    );
}

/// The outside group and every row under it are marked and paint muted;
/// they are not `No updates` rows, so a clean repo there gets no check.
#[test]
fn outside_rows_paint_muted_and_skip_the_no_updates_check() {
    let rows = open_rows("ABC-123");
    for id in [
        OUTSIDE_FOCUS_GROUP_ID,
        "repo:notes",
        "file:notes:README.md",
        "repo:lib",
    ] {
        let row = row(&rows, id);
        assert!(row.outside_focus, "{id}");
        assert!(!row.in_no_updates, "{id}");
        let segs = painted_row_segments(row, true, false, false, false, None);
        for seg in segs.segments.iter().chain(&segs.trailing) {
            assert_eq!(seg.role, SegRole::Muted, "{id}: {:?}", seg.text);
            assert_eq!(seg.hex, None, "{id}: {:?}", seg.text);
        }
    }
    for id in ["workspace", "repo:app", "repo:services/api"] {
        assert!(!row(&rows, id).outside_focus, "{id}");
    }
    let lib = row(&rows, "repo:lib");
    assert!(
        !lib.trailing.split(' ').any(|token| token == "."),
        "no clean check: {:?}",
        lib.trailing
    );
}

/// The outside group starts folded, like `No updates`.
#[test]
fn outside_group_starts_folded() {
    let tree = build_focused_tree(&workspace(), true, "ws", Some(&TreeFocus::parse("ABC-123")));
    let folds = default_folds(&tree);
    assert!(folds.contains(OUTSIDE_FOCUS_GROUP_ID));
    let rows = flatten_with(&tree, &folds, true);
    assert!(row(&rows, OUTSIDE_FOCUS_GROUP_ID).folded);
    assert!(!rows.iter().any(|row| row.id == "repo:lib"));
}

/// Workspace chip: `◎ <query> · f/t repos` leads the summary and outlasts
/// it; the ASCII set paints `(o)`.
#[test]
fn workspace_chip_leads_the_summary_and_gives_way_last() {
    let chrome = NodeChrome {
        change_count: 4,
        sync_summary: "2 behind".into(),
        focus_chip: Some(FocusChip {
            query: "ABC-123".into(),
            focused: 3,
            total: 31,
        }),
        ..NodeChrome::default()
    };
    let text = |room: usize, ascii: bool| -> (String, usize) {
        let fit = workspace_trailing_fit(&chrome, PrTotals::default(), false, room, ascii);
        let text: String = fit.segs.iter().map(|seg| seg.text.as_str()).collect();
        (text, fit.chip_cols)
    };
    let chip = "◎ ABC-123 · 3/31 repos";
    let full = format!("{chip}  4 changed · 2 behind");
    let steps = [
        (visible_width(&full) + 1, full.clone()),
        (visible_width(&full), format!("{chip}  4 changed")),
        (
            visible_width(&format!("{chip}  4 changed")),
            format!("{chip}  4"),
        ),
        (visible_width(&format!("{chip}  4")), chip.to_string()),
        (visible_width(chip), String::new()),
    ];
    for (room, want) in steps {
        let (got, cols) = text(room, false);
        assert_eq!(got, want, "room {room}");
        let want_cols = if want.is_empty() {
            0
        } else {
            visible_width(chip)
        };
        assert_eq!(cols, want_cols, "room {room}");
    }
    assert_eq!(
        text(80, true).0,
        "(o) ABC-123 · 3/31 repos  4 changed · 2 behind"
    );
    // The PR total still ends the run, after the chip and the summary.
    let fit = workspace_trailing_fit(
        &chrome,
        PrTotals {
            prs: 8,
            need_you: 0,
        },
        false,
        80,
        false,
    );
    let got: String = fit.segs.iter().map(|seg| seg.text.as_str()).collect();
    assert_eq!(got, format!("{full}  8 PRs"));
    assert_eq!(fit.pr_total_cols, visible_width("8 PRs"));
    assert_eq!(fit.chip_cols, visible_width(chip));
}

/// Focus matches split out of branch and repo names; icons and the other
/// rows stay as they are, and the text reads the same.
#[test]
fn focus_matches_split_out_of_repo_and_branch_names() {
    let focus = TreeFocus::parse("abc-123, api");
    let rows = open_rows("abc-123, api");
    let checkout = row(&rows, "checkout:app/.worktrees/abc-123");
    let mut segs = checkout.segments.clone();
    highlight_focus_matches(checkout.kind, &mut segs, &focus);
    let marked: Vec<(&str, SegRole)> = segs
        .iter()
        .map(|seg| (seg.text.as_str(), seg.role))
        .collect();
    assert_eq!(
        marked[1..],
        [
            ("feature/", SegRole::BranchFeature),
            ("ABC-123", SegRole::FocusMatch),
            ("-invoice-export", SegRole::BranchFeature),
        ]
    );
    assert_eq!(segs[0], checkout.segments[0], "the icon segment stays");

    let api = row(&rows, "repo:services/api");
    let mut segs = api.segments.clone();
    highlight_focus_matches(api.kind, &mut segs, &focus);
    let matched: Vec<&str> = segs
        .iter()
        .filter(|seg| seg.role == SegRole::FocusMatch)
        .map(|seg| seg.text.as_str())
        .collect();
    assert_eq!(matched, ["api", "abc-123", "api"], "name and branch");
    assert!(segs
        .iter()
        .filter(|seg| seg.role == SegRole::FocusMatch)
        .all(|seg| seg.bold && seg.icon.is_none()));
    let joined = |segs: &[TextSeg]| segs.iter().map(|s| s.text.clone()).collect::<String>();
    assert_eq!(joined(&segs), joined(&api.segments));

    let workspace_row = row(&rows, "workspace");
    let mut segs = workspace_row.segments.clone();
    highlight_focus_matches(workspace_row.kind, &mut segs, &TreeFocus::parse("ws"));
    assert_eq!(segs, workspace_row.segments, "only repo and checkout rows");
}

/// A linked checkout row lights its short name and branch, never the
/// separator or the muted ` · <primary>` text.
#[test]
fn focus_matches_skip_the_primary_suffix_and_the_separator() {
    let repos = [crate::testutil::linked_checkout(
        "app/.worktrees/x",
        "app",
        "feature/app-x",
    )];
    let snapshot = build_workspace_snapshot(&repos, &[], false, &[]);
    let tree = build_tree(&snapshot, true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let linked = row(&rows, "repo:app/.worktrees/x");
    assert!(
        linked.segments.iter().any(|seg| seg.text == " · app"),
        "{:?}",
        linked.segments
    );
    let mut segs = linked.segments.clone();
    highlight_focus_matches(linked.kind, &mut segs, &TreeFocus::parse("app, x"));
    let matched: Vec<&str> = segs
        .iter()
        .filter(|seg| seg.role == SegRole::FocusMatch)
        .map(|seg| seg.text.as_str())
        .collect();
    assert_eq!(matched, ["x", "app", "x"], "name and branch only");
    assert!(segs.iter().any(|seg| seg.text == " · app"), "suffix intact");
}

/// Hidden repos are already out of the visible snapshot: a feature focus
/// counts only what the tree shows.
#[test]
fn chip_counts_follow_the_visible_snapshot() {
    let mut repos = focus_workspace_repos();
    repos.push(repo_snapshot("vendor/big"));
    let all = build_workspace_snapshot(&repos, &["vendor".to_string()], false, &[]);
    let visible = crate::snapshot::visible_workspace_snapshot(&all);
    let tree = build_focused_tree(&visible, true, "ws", Some(&TreeFocus::parse("ABC-123")));
    let chip = tree.chrome.focus_chip.expect("chip");
    assert_eq!((chip.focused, chip.total), (2, 4));
}
