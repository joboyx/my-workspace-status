use super::*;
use crate::git::UpstreamRewrite;
use crate::snapshot::{
    build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
};
use crate::testutil::unstaged_change;

/// A middle-biased list: the cursor goes to the one row whose viewport
/// starts at the top, or stays when its own viewport already does (the
/// first and the last page).
#[test]
fn cursor_for_viewport_top_keeps_or_centres_the_cursor() {
    let (n, h) = (81, 24);
    let start = |c: usize| list_viewport_start(n, c, h);
    assert_eq!(cursor_for_viewport_top(n, 40, 30, start), 42);
    assert_eq!(cursor_for_viewport_top(n, 40, 0, start), 12, "first page");
    assert_eq!(cursor_for_viewport_top(n, 5, 0, start), 5, "stays");
    assert_eq!(cursor_for_viewport_top(n, 40, 57, start), 69, "last page");
    assert_eq!(cursor_for_viewport_top(n, 75, 57, start), 75, "stays");
    assert_eq!(cursor_for_viewport_top(0, 3, 4, start), 0, "no rows");
}

/// A wrapped diff: every top a cursor reaches lands exactly; a top
/// inside a tall row lands on the first cursor past it.
#[test]
fn cursor_for_viewport_top_on_wrapped_rows() {
    use crate::tui::diff::wrap_viewport_start;
    let heights = [1, 1, 8, 1, 1, 1, 1, 1, 1, 1, 1, 1];
    let start = |c: usize| wrap_viewport_start(&heights, c, 4);
    let reached: Vec<usize> = (0..heights.len()).map(start).collect();
    let last = *reached.last().unwrap();
    for top in 0..=last {
        let cursor = cursor_for_viewport_top(heights.len(), 0, top, start);
        if reached.contains(&top) {
            assert_eq!(start(cursor), top, "top {top}");
        } else {
            assert!(start(cursor) > top, "top {top} lands past it");
            assert!(
                cursor == 0 || start(cursor - 1) < top,
                "top {top}: first past"
            );
        }
    }
}

#[test]
fn sync_marks_and_pr_badges_carry_their_icon_kind() {
    for (status, note, kind, text) in [
        (
            SyncStatus::Ahead,
            "ahead by 3 commits",
            IconKind::Ahead,
            "^3",
        ),
        (
            SyncStatus::Behind,
            "behind by 2 commits",
            IconKind::Behind,
            "v2",
        ),
        (
            SyncStatus::Diverged,
            "diverged (ahead 1, behind 2)",
            IconKind::Diverged,
            "Y",
        ),
        (SyncStatus::NoUpstream, "", IconKind::NoUpstream, "?"),
    ] {
        let chrome = NodeChrome {
            sync_status: Some(status),
            sync_note: note.into(),
            ..NodeChrome::default()
        };
        let segs = sync_trailing(&chrome, false, true);
        assert_eq!(segs.len(), 1, "{status:?}");
        assert_eq!(segs[0].text, text, "{status:?}: paint text unchanged");
        assert_eq!(segs[0].icon, Some(kind), "{status:?}");
    }
    let clean = NodeChrome {
        sync_status: Some(SyncStatus::UpToDate),
        ..NodeChrome::default()
    };
    assert!(sync_trailing(&clean, false, true).is_empty());
    assert_eq!(
        sync_trailing(&clean, true, true)[0].icon,
        Some(IconKind::Clean),
        "the clean check under No updates"
    );
    let failed = NodeChrome {
        sync_status: Some(SyncStatus::NoUpstream),
        sync_note: STATUS_FAILED_NOTE.into(),
        ..NodeChrome::default()
    };
    let failed = sync_trailing(&failed, false, true);
    assert_eq!(
        failed.iter().map(|seg| seg.icon).collect::<Vec<_>>(),
        vec![Some(IconKind::StatusFailed), None],
        "status failed is not a sync mark; only its glyph is an icon"
    );

    let mut segs = NodeSegments {
        segments: vec![
            text_seg("app", SegRole::Repo),
            text_seg("feature", SegRole::BranchFeature),
        ],
        trailing: sync_trailing(
            &NodeChrome {
                sync_status: Some(SyncStatus::Behind),
                sync_note: "behind by 1 commits".into(),
                ..NodeChrome::default()
            },
            false,
            true,
        ),
    };
    let at =
        with_pr_badge(&mut segs, true, Some(&badge(PrState::Ready, Some("#7")))).expect("badge");
    assert_eq!(
        at,
        segs.trailing.len() - 1,
        "the glyph ends the trailing run"
    );
    assert_eq!(segs.trailing[at].icon, Some(IconKind::PrReady));
    let trailing: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        trailing, "v1  #7 A",
        "two spaces between sync and the badge"
    );
    assert_eq!(
        segment_icons(&segs),
        vec![IconKind::Behind, IconKind::PrReady],
        "label first, then the trailing run; ref and glyph are one icon"
    );
    let mut no_branch = NodeSegments {
        segments: vec![text_seg("app", SegRole::Repo)],
        trailing: Vec::new(),
    };
    assert_eq!(
        with_pr_badge(&mut no_branch, true, Some(&badge(PrState::InReview, None))),
        None,
        "a row that paints no branch text gets no badge"
    );
    assert!(no_branch.trailing.is_empty());
}

/// A fresh badge in `state` with `ref_text`.
fn badge(state: PrState, ref_text: Option<&str>) -> PrBadge {
    PrBadge {
        mark: PrMark::State(state),
        ref_text: ref_text.map(str::to_string),
        stale: false,
    }
}

/// `<ref> <glyph>` ends the run: ref muted, glyph bold in the state
/// colour, both tagged; stale paints both muted; no ref paints the glyph
/// alone. GitHub and GitLab refs, both glyph modes.
#[test]
fn pr_badge_paints_its_ref_muted_and_its_glyph_bold_in_the_state_colour() {
    let roles = [
        (PrState::CiFailed, SegRole::Deleted, "✕", "x"),
        (PrState::Ready, SegRole::Added, "◉", "A"),
        (PrState::CiRunning, SegRole::Modified, "◐", "r"),
        (PrState::Draft, SegRole::Muted, "◌", "d"),
        (PrState::InReview, SegRole::BranchDefault, "○", "P"),
        (PrState::Merged, SegRole::BranchFeature, "✓", "m"),
    ];
    for (state, role, nerd, letter) in roles {
        for ascii in [false, true] {
            let glyph = if ascii { letter } else { nerd };
            for ref_text in [Some("#318"), Some("!12"), None] {
                for stale in [false, true] {
                    let what = format!("{state:?} ascii={ascii} {ref_text:?} stale={stale}");
                    let mut segs = NodeSegments {
                        segments: vec![text_seg("feature", SegRole::BranchFeature)],
                        trailing: vec![change_count_seg(3)],
                    };
                    let pr = PrBadge {
                        stale,
                        ..badge(state, ref_text)
                    };
                    let at = with_pr_badge(&mut segs, ascii, Some(&pr)).expect("badge");
                    let texts: Vec<&str> =
                        segs.trailing.iter().map(|seg| seg.text.as_str()).collect();
                    let kind = pr_badge_kind(state);
                    match ref_text {
                        Some(r) => assert_eq!(texts, ["3", "  ", r, " ", glyph], "{what}"),
                        None => assert_eq!(texts, ["3", "  ", glyph], "{what}"),
                    }
                    let mark = &segs.trailing[at];
                    assert_eq!(at, segs.trailing.len() - 1, "{what}");
                    assert_eq!(mark.icon, Some(kind), "{what}");
                    assert!(mark.bold, "{what}");
                    let want = if stale { SegRole::Muted } else { role };
                    assert_eq!(mark.role, want, "{what}");
                    if ref_text.is_some() {
                        let r = &segs.trailing[at - 2];
                        assert_eq!(
                            (r.role, r.icon, r.bold),
                            (SegRole::Muted, Some(kind), false)
                        );
                        assert_eq!(segs.trailing[at - 1].icon, None, "{what}: the gap");
                    }
                    assert_eq!(pr_badge_mark(ascii, state), (glyph, role), "{what}");
                    assert_eq!(pr_badge_role(state, stale), want, "{what}");
                    assert_eq!(PrMark::State(state).kind(), kind, "{what}");
                    assert_eq!(PrMark::State(state).role(stale), want, "{what}");
                }
            }
        }
    }
}

/// A failed lookup with no earlier answer ends the run with `!` alone,
/// bold in the deleted colour, tagged as the failure mark, in both
/// glyph modes; it is never muted.
#[test]
fn pr_lookup_failed_badge_paints_a_bold_red_bang_alone() {
    for ascii in [false, true] {
        for stale in [false, true] {
            let what = format!("ascii={ascii} stale={stale}");
            let mut segs = NodeSegments {
                segments: vec![text_seg("feature", SegRole::BranchFeature)],
                trailing: vec![change_count_seg(3)],
            };
            let pr = PrBadge {
                mark: PrMark::LookupFailed,
                ref_text: None,
                stale,
            };
            let at = with_pr_badge(&mut segs, ascii, Some(&pr)).expect("badge");
            let texts: Vec<&str> = segs.trailing.iter().map(|seg| seg.text.as_str()).collect();
            assert_eq!(texts, ["3", "  ", "!"], "{what}");
            let mark = &segs.trailing[at];
            assert_eq!(at, segs.trailing.len() - 1, "{what}");
            assert_eq!(mark.icon, Some(IconKind::PrLookupFailed), "{what}");
            assert_eq!((mark.role, mark.bold), (SegRole::Deleted, true), "{what}");
        }
    }
}

/// The workspace total sits after the change summary. While a PR needs
/// the author, `need you` outlasts the PR count and the change summary:
/// the PR count goes, then the summary shortens beside `need you`, then
/// `need you` stands alone, then the summary's own steps. With none to
/// act on, the PR count goes before the summary does.
#[test]
fn workspace_need_you_outlasts_the_pr_count_and_the_change_summary() {
    let chrome = NodeChrome {
        change_count: 4,
        sync_summary: "2 behind".into(),
        ..NodeChrome::default()
    };
    let text = |totals: PrTotals, narrow: bool, room: usize| -> String {
        let (segs, total_cols) = workspace_trailing_fit(&chrome, totals, narrow, room);
        let text: String = segs.iter().map(|seg| seg.text.as_str()).collect();
        // The total's columns are the text after the gap, or the whole
        // text when the total stands alone.
        let total = [
            "8 PRs · 2 need you",
            "1 PR · 1 needs you",
            "8 PRs",
            "2 need you",
            "1 needs you",
        ]
        .into_iter()
        .find(|total| text.ends_with(total))
        .map_or(0, visible_width);
        assert_eq!(total_cols, total, "{text:?}");
        text
    };
    let both = PrTotals {
        prs: 8,
        need_you: 2,
    };
    // `room` counts the one-column pad: a text fits when it is narrower.
    let full = "4 changed · 2 behind  8 PRs · 2 need you";
    let steps = [
        (visible_width(full) + 1, full),
        (visible_width(full), "4 changed · 2 behind  2 need you"),
        (
            visible_width("4 changed · 2 behind  2 need you"),
            "4 changed  2 need you",
        ),
        (visible_width("4 changed  2 need you"), "4  2 need you"),
        (visible_width("4  2 need you"), "2 need you"),
        (visible_width("2 need you"), "4 changed"),
        (visible_width("4 changed"), "4"),
        (1, ""),
    ];
    for (room, want) in steps {
        assert_eq!(text(both, false, room), want, "room {room}");
    }
    let none_need = PrTotals {
        prs: 8,
        need_you: 0,
    };
    assert_eq!(text(none_need, false, 80), "4 changed · 2 behind  8 PRs");
    assert_eq!(text(none_need, true, 80), "4 changed · 2 behind  8 PRs");
    // With nobody to act, the PR count is the only part and it goes
    // before the summary does.
    let tight = visible_width("4 changed · 2 behind  8 PRs");
    assert_eq!(text(none_need, false, tight), "4 changed · 2 behind");
    assert_eq!(
        text(none_need, false, visible_width("4 changed · 2 behind")),
        "4 changed",
        "with none to act on, no short summary keeps the PR count"
    );
    assert_eq!(text(both, true, 80), "4 changed · 2 behind  2 need you");
    assert_eq!(
        text(
            both,
            true,
            visible_width("4 changed · 2 behind  2 need you")
        ),
        "4 changed  2 need you",
        "a narrow tree shortens the summary beside need you too"
    );
    let (short, _) =
        workspace_trailing_fit(&chrome, both, false, visible_width("4 changed  2 need you"));
    let short: Vec<(&str, SegRole, bool, Option<IconKind>)> = short
        .iter()
        .map(|seg| (seg.text.as_str(), seg.role, seg.bold, seg.icon))
        .collect();
    assert_eq!(
        short,
        [
            ("4", SegRole::Muted, false, Some(IconKind::Workspace)),
            ("  ", SegRole::Muted, false, None),
            ("2 need you", SegRole::Modified, true, None),
        ]
    );
    let one = PrTotals {
        prs: 1,
        need_you: 1,
    };
    assert_eq!(
        text(one, false, 80),
        "4 changed · 2 behind  1 PR · 1 needs you"
    );
    assert_eq!(text(one, true, 80), "4 changed · 2 behind  1 needs you");
    assert_eq!(text(PrTotals::default(), false, 80), "4 changed · 2 behind");

    let (segs, _) = workspace_trailing_fit(&chrome, both, false, 80);
    let styles: Vec<(&str, SegRole, bool, bool, Option<IconKind>)> = segs
        .iter()
        .map(|seg| (seg.text.as_str(), seg.role, seg.bold, seg.dim, seg.icon))
        .collect();
    assert_eq!(
        styles,
        [
            (
                "4 changed · 2 behind",
                SegRole::Muted,
                false,
                false,
                Some(IconKind::Workspace)
            ),
            ("  ", SegRole::Muted, false, false, None),
            ("8 PRs", SegRole::Muted, false, false, None),
            (" · ", SegRole::Muted, false, true, None),
            ("2 need you", SegRole::Modified, true, false, None),
        ]
    );
}

fn force_pushed() -> UpstreamRewrite {
    UpstreamRewrite {
        upstream: "origin/main".into(),
        fork_point: "f".repeat(40),
        tip: "t".repeat(40),
        yours: 0,
        dropped: 2,
    }
}

#[test]
fn diverged_mark_is_followed_by_the_force_push_text() {
    let chrome = NodeChrome {
        sync_status: Some(SyncStatus::Diverged),
        sync_note: "diverged (ahead 2, behind 1)".into(),
        upstream_rewrite: Some(force_pushed()),
        ..NodeChrome::default()
    };
    let segs = sync_trailing(&chrome, false, true);
    assert_eq!(segs.len(), 2, "{segs:?}");
    assert_eq!(segs[0].text, "Y");
    assert_eq!(segs[0].icon, Some(IconKind::Diverged));
    assert_eq!(
        segs[1].text, " force-pushed \u{b7} 2 dropped, 0 yours",
        "short form: the popover names the upstream"
    );
    assert_eq!(segs[1].icon, None, "the text is not a second icon");
    assert_eq!(segs[1].role, segs[0].role);

    let plain = NodeChrome {
        upstream_rewrite: None,
        ..chrome
    };
    assert_eq!(sync_trailing(&plain, false, true).len(), 1);
}

#[test]
fn repo_row_carries_the_force_push_text_from_the_snapshot() {
    let mut diverged = repo("ops", false, false);
    diverged.sync_status = SyncStatus::Diverged;
    diverged.sync_note = "diverged (ahead 2, behind 1)".into();
    diverged.upstream_rewrite = Some(force_pushed());
    let built = build_workspace_snapshot(&[diverged], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let row = rows.iter().find(|r| r.id == "repo:ops").expect("ops");
    assert_eq!(row.chrome.upstream_rewrite, Some(force_pushed()));
    assert!(
        row.trailing
            .contains("force-pushed \u{b7} 2 dropped, 0 yours")
            && !row.trailing.contains("origin/main"),
        "{}",
        row.trailing
    );
    assert_eq!(
        row.chrome.sync_note, "diverged (ahead 2, behind 1)",
        "the sync note stays for the parsers"
    );
}

/// The force-push text gives way before the row's name: whole while the
/// name and the run fit (the branch after the name clips first), then
/// ` force-pushed`, then gone. The PR glyph still ends the run, the
/// label is not touched, and a row with no rewrite is left alone.
#[test]
fn force_push_text_gives_way_before_the_name_and_the_pr_badge() {
    let chrome = NodeChrome {
        sync_status: Some(SyncStatus::Diverged),
        sync_note: "diverged (ahead 2, behind 1)".into(),
        upstream_rewrite: Some(force_pushed()),
        ..NodeChrome::default()
    };
    let row = |chrome: &NodeChrome| {
        let mut segs = NodeSegments {
            segments: vec![
                text_seg("app", SegRole::Repo),
                text_seg("  feature", SegRole::BranchFeature),
            ],
            trailing: sync_trailing(chrome, false, true),
        };
        with_pr_badge(&mut segs, true, Some(&badge(PrState::CiFailed, Some("#7")))).expect("badge");
        segs
    };
    let text = |segs: &NodeSegments| -> String {
        segs.trailing.iter().map(|seg| seg.text.as_str()).collect()
    };
    let whole = text(&row(&chrome));
    assert_eq!(whole, "Y force-pushed \u{b7} 2 dropped, 0 yours  #7 x");
    // The name `app`, the run, and the one-column pad before the run.
    // `  feature` is not counted: it clips before the text gives way.
    let full = visible_width("app") + visible_width(&whole) + 1;
    let tiny = full - visible_width(" \u{b7} 2 dropped, 0 yours");
    for (room, want) in [
        (full + 10, whole.as_str()),
        (full, whole.as_str()),
        (full - 1, "Y force-pushed  #7 x"),
        (tiny, "Y force-pushed  #7 x"),
        (tiny - 1, "Y  #7 x"),
        (0, "Y  #7 x"),
    ] {
        let mut segs = row(&chrome);
        fit_upstream_rewrite_text(&mut segs, &chrome, room);
        assert_eq!(text(&segs), want, "room {room}");
        assert_eq!(
            segs.trailing.last().and_then(|seg| seg.icon),
            Some(IconKind::PrCiFailed),
            "room {room}: the glyph ends the run"
        );
        assert_eq!(segs.segments, row(&chrome).segments, "room {room}");
    }

    let plain = NodeChrome {
        upstream_rewrite: None,
        ..chrome.clone()
    };
    let mut segs = row(&plain);
    let before = segs.clone();
    fit_upstream_rewrite_text(&mut segs, &plain, 0);
    assert_eq!(segs, before, "no rewrite: nothing to fit");
}

fn repo(name: &str, with_change: bool, linked: bool) -> RepoSnapshot {
    let snap = crate::testutil::repo(name, with_change);
    if !linked {
        return snap;
    }
    RepoSnapshot {
        checkout_kind: CheckoutKind::Linked,
        primary_repo: Some("app".into()),
        ..snap
    }
}

#[test]
fn hidden_omitted_from_visible_tree() {
    let built = build_workspace_snapshot(
        &[repo("app", true, false), repo("notes", true, false)],
        &["notes".into()],
        false,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "workspace");
    let rows = flatten(&tree, &HashSet::new());
    assert!(rows.iter().any(|r| r.label.contains("app")));
    assert!(rows.iter().all(|r| !r.label.contains("notes")));
}

#[test]
fn show_hidden_includes_notes() {
    let built = build_workspace_snapshot(
        &[repo("app", true, false), repo("notes", true, false)],
        &["notes".into()],
        true,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "workspace");
    let rows = flatten(&tree, &HashSet::new());
    assert!(rows.iter().any(|r| r.label.contains("notes")));
}

fn hidden_primary_and_linked_child() -> (RepoSnapshot, RepoSnapshot) {
    let primary = repo("app", true, false);
    let mut linked = repo("app/.worktrees/feat", true, true);
    linked.branch = "feature/linked-open".into();
    (primary, linked)
}

fn tree_rows(snapshot: &crate::snapshot::WorkspaceSnapshot) -> Vec<VisibleRow> {
    let tree = build_tree(&visible_for_tree(snapshot), true, "workspace");
    flatten_with(&tree, &HashSet::new(), true)
}

#[test]
fn hidden_primary_omits_linked_child_from_visible_tree() {
    let (primary, linked) = hidden_primary_and_linked_child();
    let built = build_workspace_snapshot(&[primary, linked], &["app".into()], false, &[]);
    assert!(
        built.repos.iter().any(|r| r.repo == "app" && r.hidden),
        "primary stays tagged hidden"
    );
    assert!(
        built
            .repos
            .iter()
            .any(|r| r.repo == "app/.worktrees/feat" && !r.hidden),
        "linked child must not be retagged hidden"
    );
    let rows = tree_rows(&built);
    assert!(rows.iter().all(|r| r.id != "repo:app"));
    assert!(rows.iter().all(|r| r.id != "checkout:app"));
    assert!(
        rows.iter().all(|r| r.id != "repo:app/.worktrees/feat"),
        "linked child must not paint as a workspace-root orphan, got {:?}",
        rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
    );
    assert!(rows.iter().all(|r| r.id != "checkout:app/.worktrees/feat"));
    assert!(rows
        .iter()
        .all(|r| !r.label.contains("feature/linked-open")));
    assert!(rows.iter().all(|r| !r.label.contains("app/.worktrees")));
}

#[test]
fn show_hidden_nests_linked_child_of_hidden_primary() {
    let (primary, linked) = hidden_primary_and_linked_child();
    let built = build_workspace_snapshot(&[primary, linked], &["app".into()], true, &[]);
    let rows = tree_rows(&built);
    let family = rows.iter().find(|r| r.id == "repo:app").expect("family");
    assert!(family.chrome.is_family);
    assert!(family.trailing.contains("2 wt"), "{}", family.trailing);
    let checkout = rows
        .iter()
        .find(|r| r.id == "checkout:app/.worktrees/feat")
        .expect("linked checkout");
    assert_eq!(checkout.kind, NodeKind::Checkout);
    assert!(checkout.label.contains("feature/linked-open"));
    assert!(
        checkout.label.contains('L'),
        "linked row uses ASCII L, got {}",
        checkout.label
    );
    assert!(
        rows.iter().all(|r| r.id != "repo:app/.worktrees/feat"),
        "shown family must not paint a workspace-root orphan"
    );
}

#[test]
fn standalone_hidden_linked_path_omitted_until_shown() {
    let primary = repo("app", true, false);
    let mut linked = repo("app/.worktrees/feat", true, true);
    linked.branch = "feature/linked-open".into();
    let hidden = build_workspace_snapshot(
        &[primary.clone(), linked.clone()],
        &["app/.worktrees/feat".into()],
        false,
        &[],
    );
    let hidden_rows = tree_rows(&hidden);
    assert!(hidden_rows.iter().any(|r| r.id == "repo:app"));
    assert!(hidden_rows
        .iter()
        .all(|r| r.id != "checkout:app/.worktrees/feat"));
    assert!(hidden_rows
        .iter()
        .all(|r| r.id != "repo:app/.worktrees/feat"));
    assert!(hidden_rows
        .iter()
        .all(|r| !r.label.contains("feature/linked-open")));

    let shown = build_workspace_snapshot(
        &[primary, linked],
        &["app/.worktrees/feat".into()],
        true,
        &[],
    );
    let shown_rows = tree_rows(&shown);
    assert!(shown_rows.iter().any(|r| r.id == "repo:app"));
    assert!(shown_rows
        .iter()
        .any(|r| r.id == "checkout:app/.worktrees/feat"));
    assert!(shown_rows
        .iter()
        .all(|r| r.id != "repo:app/.worktrees/feat"));
}

#[test]
fn no_updates_group_starts_folded() {
    let built = build_workspace_snapshot(
        &[repo("app", true, false), repo("lib", false, false)],
        &[],
        false,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "workspace");
    let folds = default_folds(&tree);
    assert!(folds.contains("group:no-updates"));
    let rows = flatten(&tree, &folds);
    assert!(rows.iter().any(|r| r.id == "group:no-updates" && r.folded));
    assert!(rows
        .iter()
        .all(|r| !r.label.contains("lib  main") || r.id == "group:no-updates"));
}

fn dirty_repo(name: &str, paths: &[&str]) -> RepoSnapshot {
    let mut snap = repo(name, true, false);
    snap.has_untracked = paths
        .iter()
        .any(|p| *p != "README.md" && *p != "src/lib.rs");
    snap.changes = paths.iter().map(|path| unstaged_change(path)).collect();
    snap
}

#[test]
fn tree_mode_inserts_dir_and_basename_file() {
    let built = build_workspace_snapshot(
        &[dirty_repo("app", &["src/lib.rs", "README.md"])],
        &[],
        false,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "workspace");
    let rows = flatten(&tree, &HashSet::new());
    let dir = rows
        .iter()
        .find(|r| r.id == "dir:app:src")
        .expect("dir:app:src");
    assert_eq!(dir.kind, NodeKind::Dir);
    assert!(dir.foldable);
    assert!(dir.label.contains("src"));
    assert!(!dir.label.contains("lib.rs"));
    let lib = rows
        .iter()
        .find(|r| r.id == "file:app:src/lib.rs")
        .expect("lib.rs");
    assert_eq!(lib.kind, NodeKind::File);
    assert!(lib.label.contains("lib.rs"));
    assert!(!lib.label.contains("src/lib.rs"));
    let readme = rows
        .iter()
        .find(|r| r.id == "file:app:README.md")
        .expect("README.md");
    assert!(readme.label.contains("README.md"));
    let dir_idx = rows.iter().position(|r| r.id == "dir:app:src").unwrap();
    let readme_idx = rows
        .iter()
        .position(|r| r.id == "file:app:README.md")
        .unwrap();
    let repo_idx = rows.iter().position(|r| r.id == "repo:app").unwrap();
    assert!(dir_idx < readme_idx);
    assert_eq!(rows[dir_idx + 1].id, "file:app:src/lib.rs");
    assert!(repo_idx < dir_idx);
}

#[test]
fn flat_mode_is_full_paths_without_dir_rows() {
    let built = build_workspace_snapshot(
        &[dirty_repo("app", &["src/lib.rs", "README.md"])],
        &[],
        false,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), false, "workspace");
    let rows = flatten(&tree, &HashSet::new());
    assert!(rows.iter().all(|r| r.kind != NodeKind::Dir));
    let lib = rows
        .iter()
        .find(|r| r.id == "file:app:src/lib.rs")
        .expect("lib.rs");
    assert!(lib.label.contains("lib.rs"));
    assert!(lib.label.contains("src"));
    assert!(lib.trailing.contains("M"));
    assert!(!lib.label.contains("src/lib.rs"));
    assert!(rows.iter().any(|r| r.id == "file:app:README.md"));
}

#[test]
fn collapse_single_child_dirs() {
    let built =
        build_workspace_snapshot(&[dirty_repo("app", &["src/foo/bar.rs"])], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "workspace");
    let rows = flatten(&tree, &HashSet::new());
    assert!(rows
        .iter()
        .any(|r| r.id == "dir:app:src/foo" && r.label.contains("src/foo")));
    assert!(rows.iter().all(|r| r.id != "dir:app:src"));
    let file = rows
        .iter()
        .find(|r| r.id == "file:app:src/foo/bar.rs")
        .expect("bar.rs");
    assert!(file.label.contains("bar.rs"));
    assert!(!file.label.contains("src/foo/bar.rs"));
}

#[test]
fn workspace_header_is_changed_files_not_dirty_repos() {
    let built = build_workspace_snapshot(
        &[
            dirty_repo("app", &["a.rs", "b.rs"]),
            repo("lib", false, false),
        ],
        &[],
        false,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "demo");
    assert_eq!(tree.label, "demo");
    assert_eq!(tree.chrome.change_count, 2);
    assert_eq!(tree.chrome.sync_summary, "all current");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let ws = rows
        .iter()
        .find(|r| r.id == "workspace")
        .expect("workspace");
    assert!(ws.label.contains("demo"));
    assert!(ws.trailing.contains("2 changed · all current"));
    assert!(!ws.label.contains("dirty"));
    assert!(!ws.trailing.contains("dirty"));
}

#[test]
fn status_letters_and_badge_are_trailing() {
    let mut snap = dirty_repo("app", &[]);
    snap.has_untracked = true;
    snap.has_staged = true;
    snap.has_unstaged = true;
    snap.changes = vec![
        FileChange {
            path: "new.ts".into(),
            staged_status: None,
            unstaged_status: None,
            untracked: true,
            old_path: None,
        },
        FileChange {
            path: "staged.rs".into(),
            staged_status: Some("M".into()),
            unstaged_status: None,
            untracked: false,
            old_path: None,
        },
        FileChange {
            path: "both.rs".into(),
            staged_status: Some("M".into()),
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        },
    ];
    let built = build_workspace_snapshot(&[snap], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let new = rows.iter().find(|r| r.id == "file:app:new.ts").expect("A");
    let staged = rows
        .iter()
        .find(|r| r.id == "file:app:staged.rs")
        .expect("S");
    let both_staged = rows
        .iter()
        .find(|r| r.id == "file:app:both.rs")
        .expect("staged both.rs");
    let both_unstaged = rows
        .iter()
        .find(|r| r.id == "file:app:both.rs#unstaged")
        .expect("unstaged both.rs");
    assert_eq!(new.trailing, "A ");
    assert_eq!(staged.trailing, "S ");
    assert_eq!(both_staged.trailing, "S ");
    assert_eq!(both_unstaged.trailing, "M ");
    assert_ne!(both_staged.id, both_unstaged.id);
    assert!(!both_staged.trailing.contains("MS"));
    assert!(!both_unstaged.trailing.contains("MS"));
    assert!(!new.label.contains('?'));
    assert!(!both_staged.label.contains("M+"));
    assert!(new.label.starts_with('·') || new.label.contains("new.ts"));
}

#[test]
fn linked_checkout_is_labeled_by_branch_not_wt_path() {
    let mut primary = dirty_repo("app", &["src/a.ts"]);
    primary.branch = "main".into();
    let mut linked = repo("app/.worktrees/feat", true, true);
    linked.branch = "feature/login-page".into();
    linked.merged_into_default = Some(false);
    linked.changes = vec![FileChange {
        path: "src/b.ts".into(),
        staged_status: None,
        unstaged_status: Some("M".into()),
        untracked: false,
        old_path: None,
    }];
    let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let family = rows.iter().find(|r| r.id == "repo:app").expect("family");
    assert!(family.chrome.is_family);
    assert!(family.trailing.contains("2 wt"));
    let checkout = rows
        .iter()
        .find(|r| r.id == "checkout:app/.worktrees/feat")
        .expect("linked checkout");
    assert_eq!(checkout.kind, NodeKind::Checkout);
    assert!(checkout.label.contains("feature/login-page"));
    assert!(checkout.label.contains('L') || checkout.label.contains("feature/"));
    assert!(checkout.label.contains('o'), "open-vs-default ascii mark");
    assert!(!checkout.label.contains("wt "));
    assert!(!checkout.label.contains("app/.worktrees/feat") || checkout.label.contains("feature/"));
    let primary_row = rows
        .iter()
        .find(|r| r.id == "checkout:app")
        .expect("primary checkout");
    assert!(primary_row.label.contains("main"));
    assert!(primary_row.label.contains('&') || primary_row.label.contains("main"));
    assert_ne!(
        primary_row.segments.first().map(|s| s.text.trim()),
        Some(icon_linked_worktree(true)),
        "primary checkout must not use the linked-worktree glyph"
    );
    assert_eq!(
        checkout.segments.first().map(|s| s.text.trim()),
        Some(icon_linked_worktree(true)),
        "linked extra keeps the worktree glyph on the left"
    );
    assert!(
        !trailing_starts_with_glyph(checkout, icon_linked_worktree(true)),
        "linked extra must not treat the worktree glyph as trailing, got {}",
        checkout.trailing
    );
    assert!(
        !left_has_glyph(family, icon_repo(true)),
        "family must not keep the repo glyph on the left, got {:?}",
        family.segments.first().map(|s| s.text.as_str())
    );
    assert!(
        trailing_starts_with_glyph(family, icon_repo(true)),
        "family repo glyph sits in trailing, got {}",
        family.trailing
    );
    assert_eq!(
        primary_row.segments.first().map(|s| s.text.trim()),
        Some(icon_branch(true)),
    );
    assert!(
        !trailing_starts_with_glyph(primary_row, icon_linked_worktree(true)),
        "primary trailing must not gain the linked-worktree glyph, got {}",
        primary_row.trailing
    );
    assert!(
        !primary_row.label.contains('o'),
        "primary checkout omits the open-vs-default mark, got {}",
        primary_row.label
    );
}

#[test]
fn primary_checkout_omits_open_vs_default_mark() {
    let mut primary = dirty_repo("app", &["src/a.ts"]);
    primary.branch = "feature/auth-refresh".into();
    primary.merged_into_default = Some(false);
    let mut linked = repo("app/.worktrees/feat", true, true);
    linked.branch = "feature/side-leaf".into();
    linked.merged_into_default = Some(false);
    let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let primary_row = rows
        .iter()
        .find(|r| r.id == "checkout:app")
        .expect("primary checkout");
    let linked_row = rows
        .iter()
        .find(|r| r.id == "checkout:app/.worktrees/feat")
        .expect("linked checkout");
    assert!(
        !primary_row.label.contains('o'),
        "primary must not paint open-vs-default, got {}",
        primary_row.label
    );
    assert!(
        linked_row.label.contains('o'),
        "linked extra keeps open-vs-default, got {}",
        linked_row.label
    );
}

#[test]
fn nested_primary_merged_into_default_paints_check() {
    let mut primary = dirty_repo("app", &["src/a.ts"]);
    primary.branch = "feature/auth-landed".into();
    primary.merged_into_default = Some(true);
    let mut linked = repo("app/.worktrees/feat", true, true);
    linked.branch = "feature/side-landed".into();
    linked.merged_into_default = Some(true);
    let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let primary_row = rows
        .iter()
        .find(|r| r.id == "checkout:app")
        .expect("primary checkout");
    let linked_row = rows
        .iter()
        .find(|r| r.id == "checkout:app/.worktrees/feat")
        .expect("linked checkout");
    assert!(
        primary_row.label.contains('M'),
        "primary merged into default must paint the check, got {}",
        primary_row.label
    );
    assert!(
        !primary_row.label.contains('o'),
        "primary must not paint open-vs-default, got {}",
        primary_row.label
    );
    assert!(
        linked_row.label.contains('M'),
        "linked extra keeps the merged check, got {}",
        linked_row.label
    );
}

#[test]
fn flat_primary_repo_merged_into_default_paints_check() {
    let mut app = repo("app", true, false);
    app.branch = "feature/auth-landed".into();
    app.merged_into_default = Some(true);
    let built = build_workspace_snapshot(&[app], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let app_row = rows.iter().find(|r| r.id == "repo:app").expect("repo:app");
    assert_eq!(app_row.kind, NodeKind::Repo);
    assert!(
        app_row.label.contains('M'),
        "flat primary merged into default must paint the check, got {}",
        app_row.label
    );
    assert!(
        !app_row.label.contains('o'),
        "flat primary must not paint open-vs-default, got {}",
        app_row.label
    );
}

#[test]
fn flat_primary_repo_uses_repo_glyph_not_worktree() {
    let built = build_workspace_snapshot(&[repo("app", true, false)], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let app = rows.iter().find(|r| r.id == "repo:app").expect("repo:app");
    assert!(
        trailing_has_glyph(app, icon_repo(true)),
        "flat primary repo glyph sits in trailing, got {}",
        app.trailing
    );
    assert!(
        !left_has_glyph(app, icon_repo(true)),
        "flat primary must not keep the repo glyph on the left"
    );
    assert!(
        !left_has_glyph(app, icon_linked_worktree(true))
            && !trailing_has_glyph(app, icon_linked_worktree(true)),
        "flat primary must not use the linked-worktree glyph, got left {:?} trailing {}",
        app.segments.first().map(|s| s.text.as_str()),
        app.trailing
    );
}

fn left_has_glyph(row: &VisibleRow, glyph: &str) -> bool {
    row.segments.iter().any(|s| s.text.trim() == glyph)
}

fn trailing_has_glyph(row: &VisibleRow, glyph: &str) -> bool {
    row.trailing_segs.iter().any(|s| s.text.trim() == glyph)
}

fn trailing_starts_with_glyph(row: &VisibleRow, glyph: &str) -> bool {
    row.trailing_segs
        .first()
        .is_some_and(|s| s.text.trim() == glyph)
}

#[test]
fn linked_worktree_kind_icon_stays_leading_while_trailing_indicators_align_right() {
    let mut primary = dirty_repo("app", &["src/a.ts"]);
    primary.branch = "main".into();
    let mut linked = repo("app/.worktrees/feat", true, true);
    linked.branch = "feature/login-page".into();
    linked.sync_status = SyncStatus::Ahead;
    linked.sync_note = "ahead by 1".into();
    linked.merged_into_default = Some(false);
    let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let checkout = rows
        .iter()
        .find(|r| r.id == "checkout:app/.worktrees/feat")
        .expect("linked checkout");
    assert_eq!(
        checkout.segments.first().map(|s| s.text.trim()),
        Some(icon_linked_worktree(true)),
        "worktree kind icon leads the name, got {:?}",
        checkout.segments.first().map(|s| s.text.as_str())
    );
    assert!(
        !left_has_glyph(checkout, icon_repo(true)),
        "linked extra must not use the repo glyph"
    );
    assert!(
        !trailing_starts_with_glyph(checkout, icon_linked_worktree(true))
            && !checkout.trailing.contains(icon_linked_worktree(true)),
        "worktree kind icon stays out of trailing, got {}",
        checkout.trailing
    );
    assert!(
        checkout.trailing.contains('^'),
        "sync mark stays trailing, got {}",
        checkout.trailing
    );
    let segs = row_segments(checkout, true, false, true, false);
    let left: String = segs.segments.iter().map(|s| s.text.as_str()).collect();
    let trail: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    let kind = left.find('L').expect("leading worktree glyph");
    let name = left.find("feature/login-page").expect("worktree name");
    let comment = trail.find('"').expect("comment mark");
    let ahead = trail.find('^').expect("ahead mark");
    assert!(
        kind < name,
        "kind icon sits immediately before the name, got {left}"
    );
    assert!(
        !trail.contains('L'),
        "comment and sync stay in trailing without the kind icon, got {trail}"
    );
    assert!(
        comment < ahead,
        "comment mark stays ahead of status in trailing, got {trail}"
    );
    // The open mark follows the name in paint order, so it leads the
    // trailing run as its own muted segment; the label ends at the name.
    assert!(
        checkout
            .segments
            .iter()
            .all(|seg| seg.icon != Some(IconKind::OpenVsDefault)),
        "merge mark leaves the label"
    );
    assert_eq!(
        checkout.segments.last().map(|seg| seg.text.as_str()),
        Some("feature/login-page")
    );
    assert_eq!(
        checkout.trailing_segs[0].icon,
        Some(IconKind::OpenVsDefault)
    );
    assert_eq!(checkout.trailing_segs[0].text, "o");
    assert_eq!(checkout.trailing_segs[0].role, SegRole::Muted);
    assert_eq!(checkout.segments[0].icon, Some(IconKind::LinkedWorktree));
    assert_eq!(checkout.trailing, "o ^1 1");
}

#[test]
fn repo_kind_icon_stays_ahead_of_sync_in_trailing() {
    let mut app = repo("app", true, false);
    app.sync_status = SyncStatus::Ahead;
    app.sync_note = "ahead by 2".into();
    let built = build_workspace_snapshot(&[app], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let app = rows.iter().find(|r| r.id == "repo:app").expect("repo:app");
    let trail: String = app.trailing_segs.iter().map(|s| s.text.as_str()).collect();
    let kind = trail.find('@').expect("repo glyph");
    let ahead = trail.find('^').expect("ahead mark");
    assert!(
        kind < ahead,
        "kind glyph sits before status in trailing, got {trail}"
    );
    let segs = row_segments(app, true, false, true, false);
    let marked: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    let comment = marked.find('"').expect("comment mark");
    let kind = marked.find('@').expect("repo glyph after comment");
    assert!(
        comment < kind,
        "comment mark stays ahead of the kind glyph, got {marked}"
    );
    let kind_at = app
        .trailing_segs
        .iter()
        .position(|seg| seg.icon == Some(IconKind::Repo))
        .expect("repo glyph");
    assert_eq!(
        app.trailing_segs[kind_at].text, "@",
        "the glyph is a bare token"
    );
    assert_eq!(trail, "& @ ^2 1", "branch glyph, then the repo glyph");
    assert_eq!(
        segment_icons(&row_segments(app, true, false, false, false)),
        vec![
            IconKind::Branch,
            IconKind::Repo,
            IconKind::Ahead,
            IconKind::ChangeCount,
        ],
    );
}

/// Kinds tagged on `row`, label then trailing, with each segment text.
fn tagged_segs(row: &VisibleRow) -> Vec<(IconKind, String)> {
    row.segments
        .iter()
        .chain(&row.trailing_segs)
        .filter_map(|seg| seg.icon.map(|kind| (kind, seg.text.clone())))
        .collect()
}

#[test]
fn branch_level_rows_tag_every_icon_and_indicator() {
    let mut primary = dirty_repo("app", &["src/a.ts", "src/b.ts"]);
    primary.branch = "feature/landed".into();
    primary.merged_into_default = Some(true);
    let mut linked = repo("app/.worktrees/feat", false, true);
    linked.branch = "feature/side".into();
    linked.merged_into_default = Some(false);
    linked.sync_status = SyncStatus::Behind;
    linked.sync_note = "behind by 3 commits".into();
    let mut broken = repo("broken", false, false);
    broken.sync_note = STATUS_FAILED_NOTE.into();
    let notes = repo("notes", true, false);
    let mut idle = repo("idle", false, false);
    idle.sync_status = SyncStatus::UpToDate;
    let built = build_workspace_snapshot(
        &[primary, linked, broken, notes, idle],
        &["notes".into()],
        true,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let row = |id: &str| rows.iter().find(|r| r.id == id).expect(id).clone();
    let s = |text: &str| text.to_string();
    use IconKind as K;
    assert_eq!(
        tagged_segs(&row("workspace")),
        vec![
            (K::Workspace, s("# ")),
            (K::Workspace, s("3 changed · 1 behind, 1 attention")),
        ]
    );
    assert_eq!(
        tagged_segs(&row("repo:app")),
        vec![
            (K::Repo, s("@")),
            (K::Behind, s("v3")),
            (K::WorktreeCount, s("2 wt")),
            (K::ChangeCount, s("2")),
        ],
        "family: repo glyph, sync, worktree count, change count"
    );
    assert_eq!(
        row("repo:app").trailing,
        "@ v3 2 wt 2",
        "bare tokens, one space apart"
    );
    assert_eq!(
        tagged_segs(&row("checkout:app")),
        vec![
            (K::Branch, s("& ")),
            (K::MergedIntoDefault, s("M")),
            (K::NoUpstream, s("?")),
            (K::ChangeCount, s("2")),
        ]
    );
    let merged = row("checkout:app");
    let mark = merged
        .trailing_segs
        .iter()
        .find(|seg| seg.icon == Some(K::MergedIntoDefault))
        .expect("merge mark");
    assert_eq!(mark.role, SegRole::Added, "merged paints in the added role");
    assert_eq!(merged.label, "& feature/landed  M ? 2");
    assert_eq!(
        tagged_segs(&row("repo:notes"))
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<String>(),
        "~&@?1",
        "hidden and branch glyphs lead the trailing run"
    );
    assert_eq!(row("repo:notes").trailing, "~ & @ ? 1");
    assert_eq!(
        tagged_segs(&row("checkout:app/.worktrees/feat")),
        vec![
            (K::LinkedWorktree, s("L ")),
            (K::OpenVsDefault, s("o")),
            (K::Behind, s("v3")),
        ]
    );
    assert_eq!(
        tagged_segs(&row("repo:broken")),
        vec![
            (K::Branch, s("&")),
            (K::Repo, s("@")),
            (K::StatusFailed, s("!")),
        ]
    );
    assert_eq!(
        row("repo:broken").trailing,
        format!("& @ ! {STATUS_FAILED_NOTE}"),
        "the words stay one space after their glyph"
    );
    assert_eq!(
        tagged_segs(&row("repo:notes")),
        vec![
            (K::Hidden, s("~")),
            (K::Branch, s("&")),
            (K::Repo, s("@")),
            (K::NoUpstream, s("?")),
            (K::ChangeCount, s("1")),
        ]
    );
    assert_eq!(
        tagged_segs(&row("group:no-updates")),
        vec![(K::Clean, s(". ")), (K::Clean, s("1"))]
    );
    assert_eq!(
        tagged_segs(&row("repo:idle")),
        vec![(K::Branch, s("&")), (K::Repo, s("@")), (K::Clean, s("."))]
    );
    let group = row("group:no-updates");
    assert_eq!(
        segment_icons(&row_segments(&group, true, false, false, false)),
        vec![K::Clean],
        "a kind on two segments lists once"
    );
}

#[test]
fn pr_badge_ends_the_trailing_run_after_the_repo_and_sync_marks() {
    let mut primary = repo("app", false, false);
    primary.branch = "feature/landed".into();
    primary.merged_into_default = Some(true);
    let built = build_workspace_snapshot(&[primary], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let row = rows.iter().find(|r| r.id == "repo:app").expect("repo:app");
    let pr = badge(PrState::InReview, Some("#7"));
    let segs = painted_row_segments(row, true, false, false, false, Some(&pr));
    let left: String = segs.segments.iter().map(|seg| seg.text.as_str()).collect();
    assert!(left.ends_with("  feature/landed"), "{left}");
    let trail: String = segs.trailing.iter().map(|seg| seg.text.as_str()).collect();
    assert_eq!(
        trail, "& M @ ?  #7 P",
        "branch glyph, merge mark, repo, sync, then the PR badge"
    );
    assert_eq!(
        segment_icons(&segs),
        vec![
            IconKind::Branch,
            IconKind::MergedIntoDefault,
            IconKind::Repo,
            IconKind::NoUpstream,
            IconKind::PrInReview,
        ]
    );
}

#[test]
fn checkout_pr_badge_ends_the_run_and_the_comment_mark_stays_first() {
    let mut primary = dirty_repo("app", &["src/a.ts"]);
    primary.branch = "main".into();
    let mut linked = repo("app/.worktrees/feat", false, true);
    linked.branch = "feature/side".into();
    linked.sync_status = SyncStatus::Ahead;
    linked.merged_into_default = Some(false);
    linked.sync_note = "ahead by 1".into();
    let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let row = rows
        .iter()
        .find(|r| r.id == "checkout:app/.worktrees/feat")
        .expect("linked checkout");
    let pr = badge(PrState::Merged, Some("!12"));
    let segs = painted_row_segments(row, true, false, true, false, Some(&pr));
    let trail: String = segs.trailing.iter().map(|seg| seg.text.as_str()).collect();
    // The comment mark stays first; the badge ends the run.
    assert_eq!(trail, "\" o ^1  !12 m");
    assert_eq!(
        segment_icons(&segs),
        vec![
            IconKind::LinkedWorktree,
            IconKind::Comment,
            IconKind::OpenVsDefault,
            IconKind::Ahead,
            IconKind::PrMerged,
        ]
    );
}

#[test]
fn file_rows_keep_type_glyph_on_the_left() {
    let built = build_workspace_snapshot(&[dirty_repo("app", &["README.md"])], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let file = rows
        .iter()
        .find(|r| r.id == "file:app:README.md")
        .expect("file");
    assert!(
        !file.trailing.contains('@') && !file.trailing.contains('L'),
        "file trailing stays a status badge, got {}",
        file.trailing
    );
    assert_eq!(file.trailing, "M ");
    assert!(
        !file.segments.is_empty(),
        "file type glyph stays on the left"
    );
}

#[test]
fn sync_marks_on_repo_rows() {
    let mut ahead = repo("lib", false, false);
    ahead.sync_status = SyncStatus::Ahead;
    ahead.sync_note = "ahead by 2".into();
    ahead.branch = "main".into();
    let mut diverged = repo("ops", false, false);
    diverged.sync_status = SyncStatus::Diverged;
    diverged.sync_note = "ahead 1, behind 1".into();
    let built = build_workspace_snapshot(&[ahead, diverged], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    assert!(tree.chrome.sync_summary.contains("1 ahead"));
    assert!(tree.chrome.sync_summary.contains("1 diverged"));
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let lib = rows.iter().find(|r| r.id == "repo:lib").expect("lib");
    assert!(
        lib.trailing.contains('^') && lib.trailing.contains('2'),
        "{}",
        lib.trailing
    );
    let ops = rows.iter().find(|r| r.id == "repo:ops").expect("ops");
    assert!(ops.trailing.contains('Y'), "{}", ops.trailing);
    assert!(!ops.label.contains("wt "));
}

#[test]
fn status_failed_repo_paints_an_error_mark_not_no_upstream() {
    let mut broken = repo("broken", false, false);
    broken.branch = crate::helpers::UNKNOWN_HEAD_BRANCH.into();
    broken.sync_note = STATUS_FAILED_NOTE.into();
    let built = build_workspace_snapshot(&[broken], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let row = rows.iter().find(|r| r.id == "repo:broken").expect("row");
    assert!(row.trailing.contains("! status failed"), "{}", row.trailing);
    assert!(
        !row.trailing.contains('?'),
        "no no-upstream mark: {}",
        row.trailing
    );
    let segs = row_segments(row, true, false, false, false);
    let failed: Vec<_> = segs
        .trailing
        .iter()
        .filter(|seg| seg.text.contains('!') || seg.text.contains("status failed"))
        .collect();
    assert_eq!(failed.len(), 2, "{:?}", segs.trailing);
    assert!(failed.iter().all(|seg| seg.role == SegRole::Deleted));
}

#[test]
fn linked_only_does_not_invent_family_container() {
    let linked = repo("app/.worktrees/feat", true, true);
    let built = build_workspace_snapshot(&[linked], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    assert!(rows.iter().any(|r| r.id == "repo:app/.worktrees/feat"));
    assert!(rows.iter().all(|r| r.id != "repo:app"));
    assert!(rows.iter().all(|r| r.kind != NodeKind::Checkout));
    let row = rows
        .iter()
        .find(|r| r.id == "repo:app/.worktrees/feat")
        .expect("linked-only repo");
    assert!(!row.chrome.is_family);
    assert!(row.label.contains("main"));
    assert!(!row.label.contains("wt "));
    assert_eq!(
        row.segments.first().map(|s| s.text.trim()),
        Some(icon_linked_worktree(true)),
        "linked-only repo keeps the worktree glyph on the left"
    );
    assert!(
        !trailing_starts_with_glyph(row, icon_linked_worktree(true)),
        "linked-only repo must not treat the worktree glyph as trailing, got {}",
        row.trailing
    );
}

#[test]
fn hidden_uses_icon_not_bracket_text() {
    let built = build_workspace_snapshot(
        &[repo("app", true, false), repo("notes", true, false)],
        &["notes".into()],
        true,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let notes = rows.iter().find(|r| r.id == "repo:notes").expect("notes");
    assert!(notes.label.contains(icon_hidden(true)), "{}", notes.label);
    assert!(!notes.label.contains("[hidden]"));
}

#[test]
fn no_updates_count_is_trailing_not_parens() {
    let built = build_workspace_snapshot(
        &[repo("app", true, false), repo("lib", false, false)],
        &[],
        false,
        &[],
    );
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let folds = default_folds(&tree);
    let rows = flatten_with(&tree, &folds, true);
    let group = rows
        .iter()
        .find(|r| r.id == "group:no-updates")
        .expect("group");
    assert_eq!(group.trailing.trim(), "1");
    assert!(group.label.contains("No updates"));
    assert!(!group.label.contains("(1)"));
    assert!(!group.trailing.contains('('));
}

#[test]
fn viewed_glyph_is_eye_not_circle() {
    let built = build_workspace_snapshot(&[dirty_repo("app", &["README.md"])], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let file = rows
        .iter()
        .find(|r| r.id == "file:app:README.md")
        .expect("file");
    let segs = row_segments(file, true, true, false, false);
    let trail: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(icon_viewed(true), "*");
    assert_eq!(icon_viewed(false), "\u{f06e}");
    assert!(trail.contains(icon_viewed(true)), "{trail}");
    assert!(!trail.contains('\u{25c9}'), "{trail}");
    let nerd = row_segments(file, false, true, false, false);
    let nerd_trail: String = nerd.trailing.iter().map(|s| s.text.as_str()).collect();
    assert!(nerd_trail.contains('\u{f06e}'), "{nerd_trail}");
    assert!(!nerd_trail.contains('\u{25c9}'), "{nerd_trail}");
    assert!(!nerd_trail.contains('\u{f07a}'), "{nerd_trail}");
    assert_eq!(
        nerd.trailing
            .iter()
            .filter(|s| s.text == icon_viewed(false))
            .count(),
        1,
        "{nerd_trail}"
    );
    let eye = nerd
        .trailing
        .iter()
        .find(|s| s.text == icon_viewed(false))
        .expect("eye");
    assert_eq!(eye.role, SegRole::Viewed);
    assert!(eye.bold);
    assert!(!eye.dim);
}

#[test]
fn comment_glyph_is_quote_not_eye() {
    let built = build_workspace_snapshot(&[dirty_repo("app", &["README.md"])], &[], false, &[]);
    let tree = build_tree(&visible_for_tree(&built), true, "ws");
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let file = rows
        .iter()
        .find(|r| r.id == "file:app:README.md")
        .expect("file");
    let segs = row_segments(file, true, false, true, false);
    let trail: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(icon_comment(true), "\"");
    assert!(trail.contains('"'), "{trail}");
    assert!(!trail.contains('*'), "{trail}");
    let unmarked = row_segments(file, true, false, false, false);
    let unmarked_trail: String = unmarked.trailing.iter().map(|s| s.text.as_str()).collect();
    assert!(!unmarked_trail.contains('"'), "{unmarked_trail}");
    let nerd = row_segments(file, false, false, true, false);
    let nerd_trail: String = nerd.trailing.iter().map(|s| s.text.as_str()).collect();
    assert!(nerd_trail.contains('\u{f075}'), "{nerd_trail}");
    let resolved = row_segments(file, true, false, true, true);
    let resolved_trail: String = resolved.trailing.iter().map(|s| s.text.as_str()).collect();
    assert!(resolved_trail.contains('\''), "{resolved_trail}");
    assert!(!resolved_trail.contains('"'), "{resolved_trail}");
}

/// Section, folder, devicon, status letter, viewed eye, and comment
/// marks each carry their catalog kind. The badge seg keeps its pad
/// (paint hits trim it); the text is unchanged.
#[test]
fn file_level_rows_tag_every_icon_and_indicator() {
    let tree = built_tree(
        vec![
            fc("src/a.rs", Some("M"), None, false),
            fc("src/b.rs", None, Some("M"), false),
            fc("new.ts", None, None, true),
        ],
        true,
    );
    let rows = flatten_with(&tree, &HashSet::new(), true);
    let row = |id: &str| rows.iter().find(|r| r.id == id).expect(id);
    assert_eq!(
        tagged_segs(row("section:app:staged")),
        vec![(IconKind::Staged, "# ".to_string())]
    );
    assert_eq!(
        tagged_segs(row("section:app:changes")),
        vec![(IconKind::Changes, "~ ".to_string())]
    );
    assert_eq!(
        tagged_segs(row("dir:app:src")),
        vec![(IconKind::Folder, "/ ".to_string())]
    );
    assert_eq!(
        tagged_segs(row("file:app:src/a.rs")),
        vec![
            (IconKind::FileType, "· ".to_string()),
            (IconKind::StatusStaged, "S ".to_string()),
        ]
    );
    assert_eq!(
        tagged_segs(row("file:app:new.ts")),
        vec![
            (IconKind::FileType, "· ".to_string()),
            (IconKind::StatusAdded, "A ".to_string()),
        ]
    );
    let file = row("file:app:src/b.rs");
    let marked = |viewed, commented, resolved| {
        let segs = row_segments(file, true, viewed, commented, resolved);
        segs.trailing
            .iter()
            .filter_map(|seg| seg.icon.map(|kind| (kind, seg.text.clone())))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        marked(true, true, false),
        vec![
            (IconKind::Viewed, "*".to_string()),
            (IconKind::Comment, "\"".to_string()),
            (IconKind::StatusModified, "M ".to_string()),
        ]
    );
    assert_eq!(
        marked(false, true, true)[0],
        (IconKind::CommentResolved, "'".to_string())
    );
    let segs = row_segments(file, true, true, true, false);
    assert_eq!(
        segment_icons(&segs),
        vec![
            IconKind::FileType,
            IconKind::Viewed,
            IconKind::Comment,
            IconKind::StatusModified
        ]
    );
    let (label, _) = segments_search_label(&segs);
    assert_eq!(
        label,
        format!("{}  * \" M", file.label.split("  ").next().unwrap())
    );
}

#[test]
fn viewport_centres_on_cursor() {
    assert_eq!(visible_window(100, 50, 10), (45, 10));
    assert_eq!(visible_window(8, 7, 20), (0, 8));
    assert_eq!(visible_window(40, 39, 2), (38, 2));
}

fn fc(path: &str, staged: Option<&str>, unstaged: Option<&str>, untracked: bool) -> FileChange {
    FileChange {
        path: path.into(),
        staged_status: staged.map(str::to_string),
        unstaged_status: unstaged.map(str::to_string),
        untracked,
        old_path: None,
    }
}

fn repo_with_changes(name: &str, changes: Vec<FileChange>) -> RepoSnapshot {
    let mut snap = repo(name, true, false);
    snap.has_staged = changes.iter().any(|c| c.staged_status.is_some());
    snap.has_unstaged = changes.iter().any(|c| c.unstaged_status.is_some());
    snap.has_untracked = changes.iter().any(|c| c.untracked);
    snap.changes = changes;
    snap
}

fn built_tree(changes: Vec<FileChange>, tree_mode: bool) -> TreeNode {
    let built = build_workspace_snapshot(&[repo_with_changes("app", changes)], &[], false, &[]);
    build_tree(&visible_for_tree(&built), tree_mode, "ws")
}

#[test]
fn unstaged_only_keeps_flat_file_tree_without_sections() {
    let tree = built_tree(
        vec![
            fc("README.md", None, Some("M"), false),
            fc("src/lib.rs", None, Some("M"), false),
            fc("new.ts", None, None, true),
        ],
        true,
    );
    let repo_node = find_node(&tree, "repo:app").expect("repo:app");
    assert!(
        repo_node
            .children
            .iter()
            .all(|c| !c.id.starts_with("section:")),
        "no staged paths must not insert section chrome, got {:?}",
        repo_node
            .children
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>()
    );
    assert!(find_node(&tree, "section:app:staged").is_none());
    assert!(find_node(&tree, "section:app:changes").is_none());
    assert!(find_node(&tree, "dir:app:src").is_some());
    assert!(find_node(&tree, "file:app:src/lib.rs").is_some());
    assert!(find_node(&tree, "file:app:README.md").is_some());
    assert!(find_node(&tree, "file:app:new.ts").is_some());
    let rows = flatten_with(&tree, &HashSet::new(), true);
    assert!(rows.iter().all(|r| !r.id.starts_with("section:")));
    assert!(rows.iter().all(|r| !r.id.contains("#unstaged")));
}

#[test]
fn staged_paths_split_checkout_into_staged_then_changes_trees() {
    let tree = built_tree(
        vec![
            fc("src/a.rs", Some("M"), None, false),
            fc("src/b.rs", None, Some("M"), false),
            fc("staged.rs", Some("A"), None, false),
            fc("new.ts", None, None, true),
        ],
        true,
    );
    let repo_node = find_node(&tree, "repo:app").expect("repo:app");
    assert_eq!(
        repo_node
            .children
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        vec!["section:app:staged", "section:app:changes"]
    );
    assert_eq!(repo_node.children[0].label, "Staged");
    assert_eq!(repo_node.children[1].label, "Changes");
    assert_eq!(repo_node.children[0].kind, NodeKind::Section);
    assert_eq!(repo_node.children[1].kind, NodeKind::Section);
    assert_eq!(repo_node.children[0].repo.as_deref(), Some("app"));
    assert!(repo_node.children[0].file.is_none());

    let staged = find_node(&tree, "section:app:staged").expect("staged");
    assert!(find_node(staged, "dir:app:src").is_some());
    assert!(find_node(staged, "file:app:src/a.rs").is_some());
    assert!(find_node(staged, "file:app:staged.rs").is_some());
    assert!(find_node(staged, "file:app:src/b.rs").is_none());
    assert!(find_node(staged, "file:app:new.ts").is_none());

    let changes = find_node(&tree, "section:app:changes").expect("changes");
    assert!(find_node(changes, "dir:app:src#unstaged").is_some());
    assert!(find_node(changes, "file:app:src/b.rs").is_some());
    assert!(find_node(changes, "file:app:new.ts").is_some());
    assert!(find_node(changes, "file:app:src/a.rs").is_none());
    assert!(find_node(changes, "file:app:staged.rs").is_none());
    assert!(find_node(changes, "dir:app:src").is_none());

    let rows = flatten_with(&tree, &HashSet::new(), true);
    let staged_file = rows
        .iter()
        .find(|r| r.id == "file:app:src/a.rs")
        .expect("staged a.rs");
    assert!(!staged_file.in_no_updates);
    assert_eq!(staged_file.trailing, "S ");
    let added = rows
        .iter()
        .find(|r| r.id == "file:app:staged.rs")
        .expect("staged add");
    assert_eq!(added.trailing, "A ");
    let untracked = rows
        .iter()
        .find(|r| r.id == "file:app:new.ts")
        .expect("untracked");
    assert_eq!(untracked.trailing, "A ");
}

#[test]
fn hide_empty_section_when_all_staged() {
    let tree = built_tree(vec![fc("only.rs", Some("M"), None, false)], true);
    let repo_node = find_node(&tree, "repo:app").expect("repo:app");
    assert_eq!(
        repo_node
            .children
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        vec!["section:app:staged"]
    );
    assert!(find_node(&tree, "section:app:changes").is_none());
    assert!(find_node(&tree, "file:app:only.rs").is_some());
}

#[test]
fn ms_dual_rows_split_letters_and_ids() {
    let tree = built_tree(vec![fc("both.rs", Some("M"), Some("M"), false)], true);
    let staged = find_node(&tree, "section:app:staged").expect("staged");
    let changes = find_node(&tree, "section:app:changes").expect("changes");
    let staged_file = find_node(staged, "file:app:both.rs").expect("staged file");
    let changes_file = find_node(changes, "file:app:both.rs#unstaged").expect("changes file");
    assert_eq!(staged_file.id, "file:app:both.rs");
    assert_eq!(changes_file.id, "file:app:both.rs#unstaged");
    assert_ne!(staged_file.id, changes_file.id);
    let staged_change = staged_file.file.as_ref().expect("staged FileChange");
    let changes_change = changes_file.file.as_ref().expect("changes FileChange");
    assert_eq!(staged_change.staged_status.as_deref(), Some("M"));
    assert!(staged_change.unstaged_status.is_none());
    assert!(!staged_change.untracked);
    assert!(changes_change.staged_status.is_none());
    assert_eq!(changes_change.unstaged_status.as_deref(), Some("M"));
    let rows = flatten_with(&tree, &HashSet::new(), true);
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "file:app:both.rs")
            .expect("staged row")
            .trailing,
        "S "
    );
    assert_eq!(
        rows.iter()
            .find(|r| r.id == "file:app:both.rs#unstaged")
            .expect("changes row")
            .trailing,
        "M "
    );
}

#[test]
fn section_glyphs_are_ascii_hash_tilde_and_nerd_package_pencil() {
    let tree = built_tree(
        vec![
            fc("staged.rs", Some("M"), None, false),
            fc("dirty.rs", None, Some("M"), false),
        ],
        true,
    );
    let ascii = flatten_with(&tree, &HashSet::new(), true);
    let nerd = flatten_with(&tree, &HashSet::new(), false);
    let ascii_staged = ascii
        .iter()
        .find(|r| r.id == "section:app:staged")
        .expect("ascii staged");
    let ascii_changes = ascii
        .iter()
        .find(|r| r.id == "section:app:changes")
        .expect("ascii changes");
    assert!(
        ascii_staged.label.contains("Staged"),
        "{}",
        ascii_staged.label
    );
    assert!(
        ascii_changes.label.contains("Changes"),
        "{}",
        ascii_changes.label
    );
    assert!(
        ascii_staged.label.contains('#'),
        "ASCII staged glyph, got {}",
        ascii_staged.label
    );
    assert!(
        ascii_changes.label.contains('~'),
        "ASCII changes glyph, got {}",
        ascii_changes.label
    );
    let nerd_staged = nerd
        .iter()
        .find(|r| r.id == "section:app:staged")
        .expect("nerd staged");
    let nerd_changes = nerd
        .iter()
        .find(|r| r.id == "section:app:changes")
        .expect("nerd changes");
    assert!(
        nerd_staged.label.contains('\u{f487}'),
        "nerd staged nf-oct-package, got {}",
        nerd_staged.label
    );
    assert!(
        nerd_changes.label.contains('\u{f040}'),
        "nerd changes nf-fa-pencil, got {}",
        nerd_changes.label
    );
    assert_eq!(crate::helpers::visible_width("\u{f487}"), 1);
    assert_eq!(crate::helpers::visible_width("\u{f040}"), 1);
    assert_eq!(ascii_staged.segments[0].role, SegRole::Heading);
    assert_eq!(ascii_changes.segments[0].role, SegRole::Heading);
}

#[test]
fn dir_path_from_id_strips_unstaged_suffix() {
    assert_eq!(
        dir_path_from_id("dir:app:src", "app").as_deref(),
        Some("src")
    );
    assert_eq!(
        dir_path_from_id("dir:app:src#unstaged", "app").as_deref(),
        Some("src")
    );
    assert_eq!(
        dir_path_from_id("dir:app:src/foo#unstaged", "app").as_deref(),
        Some("src/foo")
    );
}

#[test]
fn changes_dirs_keep_unstaged_suffix_when_collapse_names_differ() {
    let tree = built_tree(
        vec![
            fc("src/lib.rs", None, Some("M"), false),
            fc("src/deep/mod.rs", Some("M"), None, false),
        ],
        true,
    );
    let staged = find_node(&tree, "section:app:staged").expect("staged");
    let changes = find_node(&tree, "section:app:changes").expect("changes");
    assert!(find_node(staged, "dir:app:src/deep").is_some());
    assert!(find_node(staged, "file:app:src/deep/mod.rs").is_some());
    assert!(find_node(staged, "dir:app:src").is_none());
    assert!(find_node(changes, "dir:app:src#unstaged").is_some());
    assert!(find_node(changes, "file:app:src/lib.rs").is_some());
    assert!(find_node(changes, "dir:app:src").is_none());
    assert!(find_node(changes, "dir:app:src/deep").is_none());
}

#[test]
fn split_flat_mode_keeps_file_lists_under_sections() {
    let tree = built_tree(
        vec![
            fc("src/a.rs", Some("M"), None, false),
            fc("src/b.rs", None, Some("M"), false),
        ],
        false,
    );
    let repo_node = find_node(&tree, "repo:app").expect("repo:app");
    assert_eq!(
        repo_node
            .children
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        vec!["section:app:staged", "section:app:changes"]
    );
    let staged = find_node(&tree, "section:app:staged").expect("staged");
    let changes = find_node(&tree, "section:app:changes").expect("changes");
    assert!(staged.children.iter().all(|c| c.kind != NodeKind::Dir));
    assert!(changes.children.iter().all(|c| c.kind != NodeKind::Dir));
    assert!(find_node(staged, "file:app:src/a.rs").is_some());
    assert!(find_node(changes, "file:app:src/b.rs").is_some());
    let rows = flatten(&tree, &HashSet::new());
    assert!(rows.iter().all(|r| r.kind != NodeKind::Dir));
}
