use super::super::store::{put_comment, CommentEntry, CommentKey, CommentStore};
use super::*;
use crate::helpers::DETACHED_HEAD_BRANCH;
use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot};
use crate::testutil::{repo_snapshot, unique_dir, unstaged_change};
use crate::tui::diff::{DiffCell, DiffCellKind, DiffContent, DiffRow};
use crate::tui::tree::NodeChrome;
use std::path::PathBuf;
use workspace_status_graph::Commit;

/// A workspace root that does not exist, so every checkout directory under
/// it is gone on disk.
fn no_cwd() -> PathBuf {
    unique_dir("ws-comment-gc-none")
}

fn snap(repo: &str, branch: &str, kind: CheckoutKind, primary: Option<&str>) -> RepoSnapshot {
    RepoSnapshot {
        branch: branch.into(),
        checkout_kind: kind,
        primary_repo: primary.map(str::to_string),
        ..repo_snapshot(repo)
    }
}

fn snap_with_branches(
    repo: &str,
    branch: &str,
    kind: CheckoutKind,
    primary: Option<&str>,
    branches: &[&str],
) -> RepoSnapshot {
    let mut row = snap(repo, branch, kind, primary);
    row.local_branches = branches.iter().map(|s| (*s).to_string()).collect();
    row
}

#[test]
fn gc_drops_gone_branch_keeps_commit() {
    let snapshot = build_workspace_snapshot(
        &[snap_with_branches(
            "app",
            "main",
            CheckoutKind::Primary,
            None,
            &["main"],
        )],
        &[],
        false,
        &[],
    );
    let branch = CommentKey::Branch {
        repo: "app".into(),
        branch: "feature/gone".into(),
    };
    let line = CommentKey::WorktreeLine {
        repo: "app".into(),
        branch: "feature/gone".into(),
        path: "a.rs".into(),
        line: 1,
        end_line: 1,
    };
    let commit = CommentKey::Commit {
        repo: "app".into(),
        sha: "abc".into(),
    };
    let mut store = CommentStore::new();
    store = put_comment(&store, branch, "b");
    store = put_comment(&store, line, "l");
    store = put_comment(&store, commit.clone(), "c");
    let live = collect_live_set(&snapshot, &store, &HashMap::new());
    let next = gc_comments(&store, &live, &no_cwd());
    assert!(next.contains_key(&commit));
    assert_eq!(next.len(), 1);
}

/// `app` primary on `main` with branch `feature/x`, plus its linked
/// worktree `app/.worktrees/feat` on `feature/x`.
fn app_with_linked_worktree() -> WorkspaceSnapshot {
    build_workspace_snapshot(
        &[
            snap_with_branches(
                "app",
                "main",
                CheckoutKind::Primary,
                None,
                &["main", "feature/x"],
            ),
            snap_with_branches(
                "app/.worktrees/feat",
                "feature/x",
                CheckoutKind::Linked,
                Some("app"),
                &["main", "feature/x"],
            ),
        ],
        &[],
        false,
        &[],
    )
}

fn app_keys() -> Vec<CommentKey> {
    vec![
        CommentKey::Branch {
            repo: "app".into(),
            branch: "feature/x".into(),
        },
        CommentKey::Branch {
            repo: "app".into(),
            branch: DETACHED_HEAD_BRANCH.into(),
        },
        CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "feature/x".into(),
            path: "a.rs".into(),
            line: 1,
            end_line: 1,
        },
        CommentKey::Commit {
            repo: "app".into(),
            sha: "abc".into(),
        },
        CommentKey::CommitLine {
            repo: "app".into(),
            sha: "abc".into(),
            path: "a.rs".into(),
            line: 2,
            end_line: 2,
        },
        CommentKey::Worktree { path: "app".into() },
        CommentKey::Worktree {
            path: "app/.worktrees/feat".into(),
        },
    ]
}

#[test]
fn gc_keeps_comments_of_repo_missing_from_snapshot() {
    let mut store = CommentStore::new();
    for key in app_keys() {
        store = put_comment(&store, key, "keep");
    }
    let earlier = collect_live_set(&app_with_linked_worktree(), &store, &HashMap::new());
    assert_eq!(
        gc_comments(&store, &earlier, &no_cwd()),
        store,
        "all app keys are live"
    );
    // `app` and its worktree drop out of one snapshot (discovery or status
    // failure); only `lib` is left.
    let snapshot = build_workspace_snapshot(
        &[snap_with_branches(
            "lib",
            "main",
            CheckoutKind::Primary,
            None,
            &["main"],
        )],
        &[],
        false,
        &[],
    );
    for owners in [HashMap::new(), earlier.checkout_owners.clone()] {
        let live = collect_live_set(&snapshot, &store, &owners);
        let next = gc_comments(&store, &live, &no_cwd());
        assert_eq!(
            next, store,
            "a repo missing from the snapshot is unknown, not gone: keep its comments"
        );
    }
}

#[test]
fn gc_drops_gone_branch_and_worktree_of_present_authoritative_repo() {
    let mut store = CommentStore::new();
    for key in app_keys() {
        store = put_comment(&store, key, "note");
    }
    let earlier = collect_live_set(&app_with_linked_worktree(), &store, &HashMap::new());
    // The linked worktree and branch `feature/x` are gone; `app` itself is
    // present with a readable branch list.
    let snapshot = build_workspace_snapshot(
        &[snap_with_branches(
            "app",
            "main",
            CheckoutKind::Primary,
            None,
            &["main"],
        )],
        &[],
        false,
        &[],
    );
    let live = collect_live_set(&snapshot, &store, &earlier.checkout_owners);
    let next = gc_comments(&store, &live, &no_cwd());
    let gone_branch = CommentKey::Branch {
        repo: "app".into(),
        branch: "feature/x".into(),
    };
    let gone_line = CommentKey::WorktreeLine {
        repo: "app".into(),
        branch: "feature/x".into(),
        path: "a.rs".into(),
        line: 1,
        end_line: 1,
    };
    let gone_worktree = CommentKey::Worktree {
        path: "app/.worktrees/feat".into(),
    };
    assert!(!next.contains_key(&gone_branch), "{next:?}");
    assert!(!next.contains_key(&gone_line), "{next:?}");
    assert!(
        !next.contains_key(&gone_worktree),
        "a worktree of a present, authoritative repo that is gone drops: {next:?}"
    );
    assert_eq!(next.len(), store.len() - 3, "{next:?}");
    // With no memory of which repo owned the missing worktree, keep it.
    let unknown_owner = collect_live_set(&snapshot, &store, &HashMap::new());
    assert!(gc_comments(&store, &unknown_owner, &no_cwd()).contains_key(&gone_worktree));
    // A status-failed owner is not authoritative: keep the worktree key.
    let mut failed = snap(
        "app",
        crate::helpers::UNKNOWN_HEAD_BRANCH,
        CheckoutKind::Primary,
        None,
    );
    failed.sync_note = "status failed".into();
    let failed_snapshot = build_workspace_snapshot(&[failed], &[], false, &[]);
    let live = collect_live_set(&failed_snapshot, &store, &earlier.checkout_owners);
    assert!(gc_comments(&store, &live, &no_cwd()).contains_key(&gone_worktree));
}

#[test]
fn gc_keeps_missing_worktree_key_while_its_directory_exists() {
    let worktree = CommentKey::Worktree {
        path: "app/.worktrees/feat".into(),
    };
    let store = put_comment(&CommentStore::new(), worktree.clone(), "note");
    let earlier = collect_live_set(&app_with_linked_worktree(), &store, &HashMap::new());
    // `app` is present and authoritative; its linked worktree row is missing
    // (for example `git worktree list` failed).
    let snapshot = build_workspace_snapshot(
        &[snap_with_branches(
            "app",
            "main",
            CheckoutKind::Primary,
            None,
            &["main", "feature/x"],
        )],
        &[],
        false,
        &[],
    );
    let live = collect_live_set(&snapshot, &store, &earlier.checkout_owners);
    assert!(live.authoritative.contains("app"));
    let cwd = unique_dir("ws-comment-gc-wt");
    let dir = cwd.join("app/.worktrees/feat");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(
        gc_comments(&store, &live, &cwd).contains_key(&worktree),
        "the worktree directory still exists: keep the key"
    );
    std::fs::remove_dir(&dir).unwrap();
    assert!(
        !gc_comments(&store, &live, &cwd).contains_key(&worktree),
        "the worktree directory is gone: drop the key"
    );
    std::fs::remove_dir_all(&cwd).unwrap();
}

#[test]
fn gc_keeps_branch_comments_when_status_failed() {
    let mut row = snap(
        "app",
        crate::helpers::UNKNOWN_HEAD_BRANCH,
        CheckoutKind::Primary,
        None,
    );
    row.sync_note = "status failed".into();
    let snapshot = build_workspace_snapshot(&[row], &[], false, &[]);
    let branch = CommentKey::Branch {
        repo: "app".into(),
        branch: "topic/keep".into(),
    };
    let line = CommentKey::WorktreeLine {
        repo: "app".into(),
        branch: "topic/keep".into(),
        path: "a.rs".into(),
        line: 1,
        end_line: 1,
    };
    let mut store = CommentStore::new();
    store = put_comment(&store, branch.clone(), "b");
    store = put_comment(&store, line.clone(), "l");
    let live = collect_live_set(&snapshot, &store, &HashMap::new());
    let next = gc_comments(&store, &live, &no_cwd());
    assert_eq!(next.get(&branch).map(CommentEntry::as_str), Some("b"));
    assert_eq!(next.get(&line).map(CommentEntry::as_str), Some("l"));
    assert!(!live.branches.contains(&("app".into(), "(unknown)".into())));
}

#[test]
fn gc_drops_deleted_branch_when_sibling_status_failed() {
    let primary = snap_with_branches(
        "app",
        "main",
        CheckoutKind::Primary,
        None,
        &["main", "feature/linked-open"],
    );
    let mut linked = snap(
        "app/.worktrees/feat",
        crate::helpers::UNKNOWN_HEAD_BRANCH,
        CheckoutKind::Linked,
        Some("app"),
    );
    linked.sync_note = "status failed".into();
    linked.local_branches = vec!["main".into(), "doomed".into(), "feature/linked-open".into()];
    let snapshot = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
    let branch = CommentKey::Branch {
        repo: "app".into(),
        branch: "doomed".into(),
    };
    let line = CommentKey::WorktreeLine {
        repo: "app".into(),
        branch: "doomed".into(),
        path: "a.rs".into(),
        line: 1,
        end_line: 1,
    };
    let mut store = CommentStore::new();
    store = put_comment(&store, branch.clone(), "b");
    store = put_comment(&store, line.clone(), "l");
    let live = collect_live_set(&snapshot, &store, &HashMap::new());
    let next = gc_comments(&store, &live, &no_cwd());
    assert!(!next.contains_key(&branch));
    assert!(!next.contains_key(&line));
    assert!(!live.branches.contains(&("app".into(), "doomed".into())));
}

#[test]
fn detached_primary_row_is_worktree_path_key() {
    let snapshot = build_workspace_snapshot(
        &[snap(
            "app",
            DETACHED_HEAD_BRANCH,
            CheckoutKind::Primary,
            None,
        )],
        &[],
        false,
        &[],
    );
    let row = VisibleRow {
        kind: NodeKind::Repo,
        repo: Some("app".into()),
        chrome: crate::tui::tree::NodeChrome {
            branch: DETACHED_HEAD_BRANCH.into(),
            is_family: false,
            checkout_kind: Some(CheckoutKind::Primary),
            ..Default::default()
        },
        ..VisibleRow::default()
    };
    assert_eq!(
        resolve_tree_target(&snapshot, &row),
        Some(CommentKey::Worktree { path: "app".into() })
    );
    assert!(!matches!(
        resolve_tree_target(&snapshot, &row),
        Some(CommentKey::Branch { .. })
    ));
}

#[test]
fn watch_gc_keeps_detached_worktree_comment() {
    let snapshot = build_workspace_snapshot(
        &[snap(
            "app",
            DETACHED_HEAD_BRANCH,
            CheckoutKind::Primary,
            None,
        )],
        &[],
        false,
        &[],
    );
    let key = CommentKey::Worktree { path: "app".into() };
    let store = put_comment(&CommentStore::new(), key.clone(), "detached note");
    let live = collect_live_set(&snapshot, &store, &HashMap::new());
    let next = gc_comments(&store, &live, &no_cwd());
    assert_eq!(
        next.get(&key).map(CommentEntry::as_str),
        Some("detached note")
    );
}

#[test]
fn viewport_line_prefers_right_number() {
    let rows = vec![
        DiffRow::Section(crate::tui::diff::DiffSection::Unstaged),
        DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Ctx,
                text: "a".into(),
                line_no: Some(1),
            },
            right: Some(DiffCell {
                kind: DiffCellKind::Add,
                text: "b".into(),
                line_no: Some(2),
            }),
        },
    ];
    assert_eq!(viewport_line_number(&rows, 0), Some(2));
    assert_eq!(viewport_line_number(&rows, 1), Some(2));
    assert_eq!(viewport_line_number(&[], 0), None);
    assert_eq!(viewport_line_range(&rows, 0, 1), Some((2, 2)));
    assert_eq!(viewport_line_range(&rows, 0, 0), None);
    assert_eq!(viewport_line_range(&[], 0, 0), None);
    let _ = DiffContent::default();
}

fn worktree_line(line: u32, end_line: u32) -> CommentKey {
    CommentKey::WorktreeLine {
        repo: "app".into(),
        branch: "main".into(),
        path: "README.md".into(),
        line,
        end_line,
    }
}

#[test]
fn covering_line_picks_tightest_then_lowest_start() {
    let probe = worktree_line(2, 2);
    let wide = worktree_line(1, 4);
    let mid = worktree_line(2, 3);
    let one = worktree_line(2, 2);
    let other_file = CommentKey::WorktreeLine {
        repo: "app".into(),
        branch: "main".into(),
        path: "other.md".into(),
        line: 1,
        end_line: 4,
    };
    let mut store = CommentStore::new();
    store = put_comment(&store, wide.clone(), "wide");
    store = put_comment(&store, other_file, "other");
    assert_eq!(covering_line_comment(&store, &probe), Some(wide.clone()));
    store = put_comment(&store, mid.clone(), "mid");
    assert_eq!(covering_line_comment(&store, &probe), Some(mid.clone()));
    store = put_comment(&store, one.clone(), "one");
    assert_eq!(covering_line_comment(&store, &probe), Some(one));

    let left = worktree_line(1, 3);
    let right = worktree_line(2, 4);
    let mut tied = CommentStore::new();
    tied = put_comment(&tied, right, "right");
    tied = put_comment(&tied, left.clone(), "left");
    assert_eq!(covering_line_comment(&tied, &probe), Some(left));
    assert!(covering_line_comment(&CommentStore::new(), &probe).is_none());
    assert!(covering_line_comment(&store, &worktree_line(9, 9)).is_none());
}

#[test]
fn default_branch_row_is_repo_root_without_attach() {
    let snapshot = build_workspace_snapshot(
        &[snap("app", "main", CheckoutKind::Primary, None)],
        &[],
        false,
        &[],
    );
    let mut row = VisibleRow {
        kind: NodeKind::Repo,
        repo: Some("app".into()),
        chrome: crate::tui::tree::NodeChrome {
            branch: "main".into(),
            is_family: false,
            ..Default::default()
        },
        ..VisibleRow::default()
    };
    assert!(resolve_tree_target(&snapshot, &row).is_none());
    row.kind = NodeKind::Workspace;
    row.repo = None;
    assert!(resolve_tree_target(&snapshot, &row).is_none());
}

#[test]
fn linked_worktree_row_is_path_key() {
    let snapshot = build_workspace_snapshot(
        &[snap(
            "app/.worktrees/feat",
            "feature/linked-open",
            CheckoutKind::Linked,
            Some("app"),
        )],
        &[],
        false,
        &[],
    );
    let row = VisibleRow {
        kind: NodeKind::Checkout,
        repo: Some("app/.worktrees/feat".into()),
        primary_repo: Some("app".into()),
        chrome: crate::tui::tree::NodeChrome {
            branch: "feature/linked-open".into(),
            checkout_kind: Some(CheckoutKind::Linked),
            ..Default::default()
        },
        ..VisibleRow::default()
    };
    assert_eq!(
        resolve_tree_target(&snapshot, &row),
        Some(CommentKey::Worktree {
            path: "app/.worktrees/feat".into()
        })
    );
}

fn workspace_tree() -> TreeNode {
    TreeNode {
        id: "workspace".into(),
        kind: NodeKind::Workspace,
        label: "workspace".into(),
        repo: None,
        primary_repo: None,
        hidden: false,
        file: None,
        children: Vec::new(),
        chrome: NodeChrome::default(),
    }
}

fn scoped_bodies(
    store: &CommentStore,
    snapshot: &WorkspaceSnapshot,
    list: CommentExportList<'_>,
) -> Vec<String> {
    let scoped = comments_in_focus_scope(store, snapshot, &workspace_tree(), list);
    scoped.values().map(|entry| entry.body.clone()).collect()
}

fn folder_store() -> (WorkspaceSnapshot, CommentStore) {
    let snapshot = build_workspace_snapshot(
        &[snap("app", "main", CheckoutKind::Primary, None)],
        &[],
        false,
        &[],
    );
    let mut store = CommentStore::new();
    store = put_comment(
        &store,
        CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "folder1/file1".into(),
            line: 1,
            end_line: 1,
        },
        "note-file1",
    );
    store = put_comment(
        &store,
        CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "folder1/file2".into(),
            line: 2,
            end_line: 2,
        },
        "note-file2",
    );
    store = put_comment(
        &store,
        CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 1,
            end_line: 1,
        },
        "note-readme",
    );
    (snapshot, store)
}

fn file_row(path: &str) -> VisibleRow {
    VisibleRow {
        kind: NodeKind::File,
        repo: Some("app".into()),
        file: Some(unstaged_change(path)),
        chrome: NodeChrome {
            path: path.into(),
            ..Default::default()
        },
        ..VisibleRow::default()
    }
}

fn dir_row(path: &str) -> VisibleRow {
    VisibleRow {
        kind: NodeKind::Dir,
        repo: Some("app".into()),
        id: format!("dir:app:{path}"),
        chrome: NodeChrome {
            path: path.into(),
            ..Default::default()
        },
        ..VisibleRow::default()
    }
}

#[test]
fn export_file_scope_omits_folder_sibling() {
    let (snapshot, store) = folder_store();
    let row = file_row("folder1/file2");
    let bodies = scoped_bodies(
        &store,
        &snapshot,
        CommentExportList::Tree { row: Some(&row) },
    );
    assert_eq!(bodies, vec!["note-file2".to_string()]);
}

#[test]
fn export_dir_scope_includes_descendants_omits_sibling_file() {
    let (snapshot, store) = folder_store();
    let row = dir_row("folder1");
    let mut bodies = scoped_bodies(
        &store,
        &snapshot,
        CommentExportList::Tree { row: Some(&row) },
    );
    bodies.sort();
    assert_eq!(
        bodies,
        vec!["note-file1".to_string(), "note-file2".to_string()]
    );
}

#[test]
fn export_workspace_scope_includes_every_live_comment() {
    let (snapshot, store) = folder_store();
    let row = VisibleRow {
        kind: NodeKind::Workspace,
        ..VisibleRow::default()
    };
    let mut bodies = scoped_bodies(
        &store,
        &snapshot,
        CommentExportList::Tree { row: Some(&row) },
    );
    bodies.sort();
    assert_eq!(
        bodies,
        vec![
            "note-file1".to_string(),
            "note-file2".to_string(),
            "note-readme".to_string()
        ]
    );
}

#[test]
fn export_graph_commit_omits_branch_and_sibling_commit() {
    let snapshot = build_workspace_snapshot(
        &[snap("merger", "feature/graph", CheckoutKind::Primary, None)],
        &[],
        false,
        &[],
    );
    let mut store = CommentStore::new();
    store = put_comment(
        &store,
        CommentKey::Branch {
            repo: "merger".into(),
            branch: "feature/graph".into(),
        },
        "branch-note",
    );
    store = put_comment(
        &store,
        CommentKey::Commit {
            repo: "merger".into(),
            sha: "aaa".into(),
        },
        "commit-a",
    );
    store = put_comment(
        &store,
        CommentKey::Commit {
            repo: "merger".into(),
            sha: "bbb".into(),
        },
        "commit-b",
    );
    let row = GraphRow::Commit {
        commit: Commit {
            id: "aaa".into(),
            ..Default::default()
        },
        is_head: false,
        worktrees: Vec::new(),
    };
    let bodies = scoped_bodies(
        &store,
        &snapshot,
        CommentExportList::Graph {
            repo: Some("merger"),
            row: Some(&row),
        },
    );
    assert_eq!(bodies, vec!["commit-a".to_string()]);
}

#[test]
fn export_repo_row_includes_attached_branch_and_commits() {
    let snapshot = build_workspace_snapshot(
        &[snap_with_branches(
            "app",
            "main",
            CheckoutKind::Primary,
            None,
            &["main", "doomed"],
        )],
        &[],
        false,
        &[],
    );
    let mut store = CommentStore::new();
    store = put_comment(
        &store,
        CommentKey::Branch {
            repo: "app".into(),
            branch: "doomed".into(),
        },
        "attached",
    );
    store = put_comment(
        &store,
        CommentKey::Commit {
            repo: "app".into(),
            sha: "dead".into(),
        },
        "commit-note",
    );
    store = put_comment(
        &store,
        CommentKey::WorktreeLine {
            repo: "other".into(),
            branch: "main".into(),
            path: "x.rs".into(),
            line: 1,
            end_line: 1,
        },
        "sibling-repo",
    );
    let row = VisibleRow {
        kind: NodeKind::Repo,
        repo: Some("app".into()),
        chrome: NodeChrome {
            branch: "main".into(),
            ..Default::default()
        },
        ..VisibleRow::default()
    };
    let mut bodies = scoped_bodies(
        &store,
        &snapshot,
        CommentExportList::Tree { row: Some(&row) },
    );
    bodies.sort();
    assert_eq!(
        bodies,
        vec!["attached".to_string(), "commit-note".to_string()]
    );
}

fn commit_row(sha: &str) -> GraphRow {
    GraphRow::Commit {
        commit: workspace_status_graph::Commit {
            id: sha.into(),
            subject: "subject".into(),
            ..Default::default()
        },
        is_head: false,
        worktrees: Vec::new(),
    }
}

#[test]
fn graph_commit_row_marks_object_and_file_comments() {
    let row = commit_row("abc");
    let mut store = CommentStore::new();
    assert!(!graph_row_has_comment(&store, "app", None, &row, None));
    store = put_comment(
        &store,
        CommentKey::Commit {
            repo: "app".into(),
            sha: "abc".into(),
        },
        "c",
    );
    assert!(graph_row_has_comment(&store, "app", None, &row, None));
    assert!(!graph_row_has_comment(
        &store,
        "app",
        None,
        &GraphRow::Uncommitted { has_changes: true },
        Some("main")
    ));
}

#[test]
fn graph_commit_row_marks_file_line_comments() {
    let row = commit_row("abc");
    let store = put_comment(
        &CommentStore::new(),
        CommentKey::CommitLine {
            repo: "app".into(),
            sha: "abc".into(),
            path: "a.rs".into(),
            line: 3,
            end_line: 3,
        },
        "l",
    );
    assert!(graph_row_has_comment(&store, "app", None, &row, None));
    assert!(!graph_row_has_comment(
        &store,
        "app",
        None,
        &commit_row("other"),
        None
    ));
    assert!(!graph_row_has_comment(
        &store,
        "app",
        None,
        &GraphRow::Stash(workspace_status_graph::Stash::default()),
        None
    ));
}

#[test]
fn graph_uncommitted_row_marks_worktree_line_comments() {
    let row = GraphRow::Uncommitted { has_changes: true };
    let store = put_comment(
        &CommentStore::new(),
        CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 2,
            end_line: 2,
        },
        "l",
    );
    assert!(graph_row_has_comment(
        &store,
        "app",
        None,
        &row,
        Some("main")
    ));
    assert!(!graph_row_has_comment(
        &store,
        "app",
        None,
        &row,
        Some("other")
    ));
    assert!(!graph_row_has_comment(&store, "app", None, &row, None));
}

#[test]
fn commit_file_row_marks_line_comments_only() {
    let source = CommitFileSource::Commit {
        commit_id: "abc".into(),
    };
    let store = put_comment(
        &CommentStore::new(),
        CommentKey::CommitLine {
            repo: "app".into(),
            sha: "abc".into(),
            path: "src/lib.rs".into(),
            line: 1,
            end_line: 1,
        },
        "l",
    );
    assert!(commit_file_row_has_comment(
        &store,
        "app",
        None,
        &source,
        "src/lib.rs",
        None
    ));
    assert!(!commit_file_row_has_comment(
        &store,
        "app",
        None,
        &source,
        "README.md",
        None
    ));
    assert!(!commit_file_row_has_comment(
        &store,
        "app",
        None,
        &CommitFileSource::Stash {
            stash_ref: "stash@{0}".into()
        },
        "src/lib.rs",
        None
    ));
}

fn identity_vs_linked_family_snapshot() -> WorkspaceSnapshot {
    build_workspace_snapshot(
        &[
            snap("app", "main", CheckoutKind::Primary, None),
            snap(
                ".worktrees/app/feat",
                "feature/linked-open",
                CheckoutKind::Linked,
                Some("app"),
            ),
            snap(
                ".worktrees/app/other",
                "feature/linked-other",
                CheckoutKind::Linked,
                Some("app"),
            ),
            snap("other", "main", CheckoutKind::Primary, None),
        ],
        &[],
        false,
        &[],
    )
}

fn identity_vs_linked_family_store() -> CommentStore {
    let mut store = CommentStore::new();
    store = put_comment(
        &store,
        CommentKey::Worktree { path: "app".into() },
        "on-identity",
    );
    store = put_comment(
        &store,
        CommentKey::Worktree {
            path: ".worktrees/app/feat".into(),
        },
        "on-worktree",
    );
    store = put_comment(
        &store,
        CommentKey::Worktree {
            path: ".worktrees/app/other".into(),
        },
        "on-sibling",
    );
    store = put_comment(
        &store,
        CommentKey::Branch {
            repo: "app".into(),
            branch: "feature/linked-open".into(),
        },
        "on-branch",
    );
    store = put_comment(
        &store,
        CommentKey::Worktree {
            path: "other".into(),
        },
        "on-other",
    );
    store
}

fn family_app_row() -> VisibleRow {
    VisibleRow {
        kind: NodeKind::Repo,
        repo: Some("app".into()),
        chrome: NodeChrome {
            is_family: true,
            checkout_kind: Some(CheckoutKind::Primary),
            ..Default::default()
        },
        ..VisibleRow::default()
    }
}

fn linked_worktree_app_feat_row() -> VisibleRow {
    VisibleRow {
        kind: NodeKind::Checkout,
        repo: Some(".worktrees/app/feat".into()),
        primary_repo: Some("app".into()),
        chrome: NodeChrome {
            branch: "feature/linked-open".into(),
            checkout_kind: Some(CheckoutKind::Linked),
            ..Default::default()
        },
        ..VisibleRow::default()
    }
}

fn sorted_tree_bodies(
    store: &CommentStore,
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
) -> Vec<String> {
    let mut bodies = scoped_bodies(store, snapshot, CommentExportList::Tree { row: Some(row) });
    bodies.sort();
    bodies
}

#[test]
fn export_family_row_includes_identity_and_linked_worktree_object_comments() {
    let snapshot = identity_vs_linked_family_snapshot();
    let store = identity_vs_linked_family_store();
    let row = family_app_row();
    assert_eq!(
        sorted_tree_bodies(&store, &snapshot, &row),
        vec![
            "on-branch".to_string(),
            "on-identity".to_string(),
            "on-sibling".to_string(),
            "on-worktree".to_string(),
        ]
    );
}

#[test]
fn export_linked_worktree_row_includes_identity_and_worktree_object_comments() {
    let snapshot = identity_vs_linked_family_snapshot();
    let store = identity_vs_linked_family_store();
    let row = linked_worktree_app_feat_row();
    assert_eq!(
        sorted_tree_bodies(&store, &snapshot, &row),
        vec![
            "on-branch".to_string(),
            "on-identity".to_string(),
            "on-worktree".to_string(),
        ]
    );
}

#[test]
fn family_row_marks_linked_worktree_object_comment() {
    let store = put_comment(
        &CommentStore::new(),
        CommentKey::Worktree {
            path: ".worktrees/app/feat".into(),
        },
        "on-worktree",
    );
    assert!(tree_row_has_comment(
        &store,
        &identity_vs_linked_family_snapshot(),
        &family_app_row()
    ));
}

#[test]
fn linked_worktree_row_marks_identity_object_comment() {
    let store = put_comment(
        &CommentStore::new(),
        CommentKey::Worktree { path: "app".into() },
        "on-identity",
    );
    assert!(tree_row_has_comment(
        &store,
        &identity_vs_linked_family_snapshot(),
        &linked_worktree_app_feat_row()
    ));
}

#[test]
fn linked_worktree_row_omits_sibling_worktree_object_comment() {
    let snapshot = identity_vs_linked_family_snapshot();
    let store = put_comment(
        &CommentStore::new(),
        CommentKey::Worktree {
            path: ".worktrees/app/other".into(),
        },
        "on-sibling",
    );
    assert!(tree_row_has_comment(&store, &snapshot, &family_app_row()));
    assert!(!tree_row_has_comment(
        &store,
        &snapshot,
        &linked_worktree_app_feat_row()
    ));
}

#[test]
fn worktree_path_app_other_is_not_family_of_app() {
    let snapshot = identity_vs_linked_family_snapshot();
    let store = put_comment(
        &CommentStore::new(),
        CommentKey::Worktree {
            path: "app-other/x".into(),
        },
        "on-app-other",
    );
    let row = family_app_row();
    let bodies = sorted_tree_bodies(&store, &snapshot, &row);
    assert!(!bodies.iter().any(|b| b == "on-app-other"));
    assert!(!tree_row_has_comment(&store, &snapshot, &row));
}

fn entity_from_tree(snapshot: &WorkspaceSnapshot, row: &VisibleRow, ws: Option<&str>) -> EntityRef {
    resolve_entity_reference(
        snapshot,
        Some(row),
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        None,
        None,
        ws,
    )
    .expect("tree entity")
}

#[test]
fn entity_file_dir_workspace_repo_family_worktree() {
    let snapshot = build_workspace_snapshot(
        &[
            snap("app", "main", CheckoutKind::Primary, None),
            snap(
                "app/.worktrees/feat",
                "feature/linked-open",
                CheckoutKind::Linked,
                Some("app"),
            ),
        ],
        &[],
        false,
        &[],
    );
    let file = file_row("src/lib.rs");
    match entity_from_tree(&snapshot, &file, None) {
        EntityRef::File { repo, path, .. } => {
            assert_eq!(repo, "app");
            assert_eq!(path, "src/lib.rs");
        }
        other => panic!("{other:?}"),
    }
    let dir = dir_row("src");
    match entity_from_tree(&snapshot, &dir, None) {
        EntityRef::Dir { path, .. } => assert_eq!(path, "src"),
        other => panic!("{other:?}"),
    }
    let workspace = VisibleRow {
        kind: NodeKind::Workspace,
        ..VisibleRow::default()
    };
    match entity_from_tree(&snapshot, &workspace, Some("/tmp/ws")) {
        EntityRef::Workspace { path } => assert_eq!(path, "/tmp/ws"),
        other => panic!("{other:?}"),
    }
    let family = family_app_row();
    match entity_from_tree(&snapshot, &family, None) {
        EntityRef::Family { repo, path } => {
            assert_eq!(repo, "app");
            assert_eq!(path, "app");
        }
        other => panic!("{other:?}"),
    }
    let linked = linked_worktree_app_feat_row();
    match entity_from_tree(&snapshot, &linked, None) {
        EntityRef::Worktree { repo, path } => {
            assert_eq!(repo, "app");
            assert_eq!(path, ".worktrees/app/feat");
        }
        other => panic!("{other:?}"),
    }
    let repo_row = VisibleRow {
        kind: NodeKind::Repo,
        repo: Some("app".into()),
        chrome: NodeChrome {
            path: "app".into(),
            checkout_kind: Some(CheckoutKind::Primary),
            ..Default::default()
        },
        ..VisibleRow::default()
    };
    match entity_from_tree(&snapshot, &repo_row, None) {
        EntityRef::Repo { repo, path } => {
            assert_eq!(repo, "app");
            assert_eq!(path, "app");
        }
        other => panic!("{other:?}"),
    }
    let group = VisibleRow {
        kind: NodeKind::Group,
        ..VisibleRow::default()
    };
    assert!(resolve_entity_reference(
        &snapshot,
        Some(&group),
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        None,
        None,
        None,
    )
    .is_none());
    assert!(resolve_comment_target(
        &snapshot,
        Some(&file),
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        None,
    )
    .is_none());
}

#[test]
fn entity_graph_commit_stash_worktree() {
    let snapshot = build_workspace_snapshot(
        &[snap("app", "main", CheckoutKind::Primary, None)],
        &[],
        false,
        &[],
    );
    let commit = GraphRow::Commit {
        commit: Commit {
            id: "abcdef1234567890abcdef1234567890abcdef12".into(),
            ..Commit::default()
        },
        is_head: true,
        worktrees: Vec::new(),
    };
    match resolve_entity_reference(
        &snapshot,
        None,
        Some(&commit),
        Some("app"),
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        None,
        None,
        None,
    ) {
        Some(EntityRef::Commit { repo, sha }) => {
            assert_eq!(repo, "app");
            assert_eq!(sha, "abcdef1234567890abcdef1234567890abcdef12");
        }
        other => panic!("{other:?}"),
    }
    let stash = GraphRow::Stash(workspace_status_graph::Stash {
        id: "deadbeef1234567890abdeadbeef1234567890ab".into(),
        stash_ref: "stash@{0}".into(),
        subject: "WIP".into(),
        ..workspace_status_graph::Stash::default()
    });
    match resolve_entity_reference(
        &snapshot,
        None,
        Some(&stash),
        Some("app"),
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        None,
        None,
        None,
    ) {
        Some(EntityRef::Stash { stash_ref, sha, .. }) => {
            assert_eq!(stash_ref, "stash@{0}");
            assert_eq!(sha, "deadbeef1234567890abdeadbeef1234567890ab");
        }
        other => panic!("{other:?}"),
    }
    assert!(resolve_comment_target(
        &snapshot,
        None,
        Some(&stash),
        Some("app"),
        false,
        None,
        None,
        None,
        None,
        None,
    )
    .is_none());
    let uncommitted = GraphRow::Uncommitted { has_changes: true };
    match resolve_entity_reference(
        &snapshot,
        None,
        Some(&uncommitted),
        Some("app"),
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        None,
        None,
        None,
    ) {
        Some(EntityRef::Worktree { path, .. }) => assert_eq!(path, "app"),
        other => panic!("{other:?}"),
    }
    let worktree = GraphRow::Worktree(workspace_status_graph::Worktree {
        path: "app".into(),
        head_id: None,
        branch: Some("main".into()),
        hidden: false,
        is_current: false,
    });
    match resolve_entity_reference(
        &snapshot,
        None,
        Some(&worktree),
        Some("app"),
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        None,
        None,
        None,
    ) {
        Some(entity) => {
            assert!(
                matches!(&entity, EntityRef::Worktree { path, .. } if path == "app"),
                "{entity:?}"
            );
            let text = super::super::format_entity_reference(&entity);
            assert!(text.starts_with("kind: worktree\n"), "{text}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn entity_commit_file_file_and_dir() {
    let snapshot = build_workspace_snapshot(
        &[snap("app", "main", CheckoutKind::Primary, None)],
        &[],
        false,
        &[],
    );
    let file = crate::tui::commit_files::CommitFileRow {
        id: "file:README.md".into(),
        depth: 0,
        kind: crate::tui::commit_files::CommitFileRowKind::File,
        label: "README.md".into(),
        trailing: String::new(),
        segments: Vec::new(),
        trailing_segs: Vec::new(),
        path: "README.md".into(),
        foldable: false,
        folded: false,
        file: None,
    };
    match resolve_entity_reference(
        &snapshot,
        None,
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        Some(&file),
        Some("app"),
        None,
    ) {
        Some(EntityRef::File { path, .. }) => assert_eq!(path, "README.md"),
        other => panic!("{other:?}"),
    }
    let dir = crate::tui::commit_files::CommitFileRow {
        kind: crate::tui::commit_files::CommitFileRowKind::Dir,
        path: "src".into(),
        ..file.clone()
    };
    match resolve_entity_reference(
        &snapshot,
        None,
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        None,
        DiffSide::Unified,
        Some(&dir),
        Some("app"),
        None,
    ) {
        Some(EntityRef::Dir { path, .. }) => assert_eq!(path, "src"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn entity_diff_side_from_painted_rows() {
    let add = DiffRow::Line {
        left: DiffCell {
            kind: DiffCellKind::Add,
            text: "+x".into(),
            line_no: Some(2),
        },
        right: None,
    };
    let del = DiffRow::Line {
        left: DiffCell {
            kind: DiffCellKind::Del,
            text: "-y".into(),
            line_no: Some(1),
        },
        right: None,
    };
    let ctx = DiffRow::Line {
        left: DiffCell {
            kind: DiffCellKind::Ctx,
            text: " z".into(),
            line_no: Some(1),
        },
        right: None,
    };
    assert_eq!(diff_focus_side(&[add.clone()], 0, 0), DiffSide::New);
    assert_eq!(diff_focus_side(&[del.clone()], 0, 0), DiffSide::Old);
    assert_eq!(diff_focus_side(&[ctx], 0, 0), DiffSide::Unified);
    assert_eq!(diff_focus_side(&[add, del], 0, 1), DiffSide::Unified);
    let snapshot = build_workspace_snapshot(
        &[snap("app", "main", CheckoutKind::Primary, None)],
        &[],
        false,
        &[],
    );
    match resolve_entity_reference(
        &snapshot,
        None,
        None,
        None,
        true,
        Some("app"),
        Some("README.md"),
        None,
        Some(2),
        Some(2),
        DiffSide::New,
        None,
        None,
        None,
    ) {
        Some(EntityRef::Diff {
            path,
            start_line,
            end_line,
            source,
            side,
            ..
        }) => {
            assert_eq!(path, "README.md");
            assert_eq!(start_line, 2);
            assert_eq!(end_line, 2);
            assert_eq!(source, DiffSource::Worktree);
            assert_eq!(side, DiffSide::New);
        }
        other => panic!("{other:?}"),
    }
}
