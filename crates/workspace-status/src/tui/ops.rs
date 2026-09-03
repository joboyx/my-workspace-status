//! Fetch / pull / default targets. Hidden ignored stay out unless shown.

use crate::helpers::{is_detached_head_branch, DETACHED_HEAD_BRANCH};
use crate::snapshot::{
    CheckoutKind, FileChange, SyncStatus, WorkspaceRepoSnapshot, WorkspaceSnapshot,
};

use super::tree::{
    changes_side_change, dir_path_from_id, path_under_dir, staged_side_change, NodeKind, VisibleRow,
};

/// One dirty file in the focused write scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedFile {
    pub repo: String,
    pub change: FileChange,
}

fn is_family_container(snapshot: &WorkspaceSnapshot, repo: &str) -> bool {
    snapshot.repos.iter().any(|member| {
        member.checkout_kind == CheckoutKind::Linked && member.primary_repo.as_deref() == Some(repo)
    })
}

fn files_for_repo(snapshot: &WorkspaceSnapshot, repo: &str) -> Vec<ScopedFile> {
    snapshot
        .repos
        .iter()
        .find(|member| member.repo == repo)
        .map(|member| {
            member
                .changes
                .iter()
                .map(|change| ScopedFile {
                    repo: repo.to_string(),
                    change: change.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Dirty files a stage / unstage / revert should touch for the focused row.
///
/// File rows stay single-file. Dir rows walk dirty files under that dir
/// (the dir path itself and its children) filtered to that dir's section
/// side when Staged / Changes chrome is present. Section rows walk every
/// dirty file on that side of the checkout. Section and dir collections
/// split each `FileChange` the same way as tree rows (the other git side
/// is cleared). Checkout and flat repo rows walk every dirty file in that
/// checkout. Family containers, workspace, and group rows yield no files.
/// Hidden ignored stay out unless shown. Linked worktrees are included
/// only when that checkout is focused.
pub fn collect_write_files(
    snapshot: &WorkspaceSnapshot,
    focused: Option<&VisibleRow>,
    show_ignored: bool,
) -> Vec<ScopedFile> {
    let Some(row) = focused else {
        return Vec::new();
    };
    if row.ignored && !show_ignored {
        return Vec::new();
    }
    match row.kind {
        NodeKind::File => {
            let (Some(file), Some(repo)) = (row.file.as_ref(), row.repo.clone()) else {
                return Vec::new();
            };
            vec![ScopedFile {
                repo,
                change: file.clone(),
            }]
        }
        NodeKind::Dir => {
            let Some(repo) = row.repo.as_deref() else {
                return Vec::new();
            };
            let Some(dir) = dir_path_from_id(&row.id, repo) else {
                return Vec::new();
            };
            let under: Vec<ScopedFile> = files_for_repo(snapshot, repo)
                .into_iter()
                .filter(|file| path_under_dir(&file.change.path, &dir))
                .collect();
            let changes_side = row.id.ends_with("#unstaged")
                || !under.iter().any(|file| file.change.staged_status.is_some());
            under
                .into_iter()
                .filter(|file| file_on_section_side(&file.change, changes_side))
                .map(|file| split_scoped_file(file, changes_side))
                .collect()
        }
        NodeKind::Section => {
            let Some(repo) = row.repo.as_deref() else {
                return Vec::new();
            };
            let prefix = format!("section:{repo}:");
            let changes_side = match row.id.strip_prefix(&prefix) {
                Some("staged") => false,
                Some("changes") => true,
                _ => return Vec::new(),
            };
            files_for_repo(snapshot, repo)
                .into_iter()
                .filter(|file| file_on_section_side(&file.change, changes_side))
                .map(|file| split_scoped_file(file, changes_side))
                .collect()
        }
        NodeKind::Checkout => {
            let Some(repo) = row.repo.as_deref() else {
                return Vec::new();
            };
            files_for_repo(snapshot, repo)
        }
        NodeKind::Repo => {
            let Some(repo) = row.repo.as_deref() else {
                return Vec::new();
            };
            if is_family_container(snapshot, repo) {
                return Vec::new();
            }
            files_for_repo(snapshot, repo)
        }
        NodeKind::Workspace | NodeKind::Group => Vec::new(),
    }
}

fn file_on_section_side(change: &FileChange, changes_side: bool) -> bool {
    if changes_side {
        change.unstaged_status.is_some() || change.untracked
    } else {
        change.staged_status.is_some()
    }
}

fn split_scoped_file(file: ScopedFile, changes_side: bool) -> ScopedFile {
    ScopedFile {
        repo: file.repo,
        change: if changes_side {
            changes_side_change(&file.change)
        } else {
            staged_side_change(&file.change)
        },
    }
}

/// True when `P` may push this checkout (ahead / diverged / no-upstream).
pub fn snapshot_pushable(snap: &WorkspaceRepoSnapshot) -> bool {
    if is_detached_head_branch(&snap.branch) || snap.branch == DETACHED_HEAD_BRANCH {
        return false;
    }
    if snap.branch == "(unknown)" {
        return false;
    }
    matches!(
        snap.sync_status,
        SyncStatus::Ahead | SyncStatus::Diverged | SyncStatus::NoUpstream
    )
}

/// `Y` always deletes untracked. Plain `y` deletes only a sole untracked file.
pub fn should_delete_untracked(untracked_flags: &[bool], clean: bool) -> bool {
    if clean {
        return true;
    }
    untracked_flags.len() == 1 && untracked_flags[0]
}

/// Workspace ops that must skip hidden ignored repos and unfocused worktrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Fetch,
    Pull,
    DefaultBranch,
}

/// In-flight multi-repo git op painted on the breadcrumb trailing slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunningOp {
    Fetch,
    Pull,
    Push,
    DefaultBranch,
}

/// Progress line for a running workspace op: `Pulling 2/18…`.
///
/// `total == 0` drops the counter (`Pulling…`). The status line keeps
/// mode pills and hints; this string is the breadcrumb trailing slot.
pub fn format_running_op(kind: RunningOp, done: usize, total: usize) -> String {
    let verb = match kind {
        RunningOp::Fetch => "Fetching",
        RunningOp::Pull => "Pulling",
        RunningOp::Push => "Pushing",
        RunningOp::DefaultBranch => "Switching",
    };
    if total == 0 {
        format!("{verb}…")
    } else {
        format!("{verb} {done}/{total}…")
    }
}

/// Completion line for a finished workspace op: `Pulled 3 repos`.
///
/// `ok + failed` is how many repos the op ran against. `failed > 0`
/// appends ` (N failed)`. Repo names are never listed — a long
/// workspace must not swamp the breadcrumb trailing slot.
pub fn format_completed_op(kind: RunningOp, ok: usize, failed: usize) -> String {
    let verb = match kind {
        RunningOp::Fetch => "Fetched",
        RunningOp::Pull => "Pulled",
        RunningOp::Push => "Pushed",
        RunningOp::DefaultBranch => "Switched",
    };
    let total = ok.saturating_add(failed);
    let noun = if total == 1 { "repo" } else { "repos" };
    if failed > 0 {
        format!("{verb} {total} {noun} ({failed} failed)")
    } else {
        format!("{verb} {total} {noun}")
    }
}

/// True when `p` / `d` must stay a silent no-op on this row kind.
///
/// Pull / default-branch are workspace / repo / checkout only.
/// Fetch stays scoped on file and dir rows.
pub fn op_is_kind_noop(kind: NodeKind, op: Op) -> bool {
    matches!(op, Op::Pull | Op::DefaultBranch)
        && matches!(kind, NodeKind::File | NodeKind::Dir | NodeKind::Section)
}

/// Checkout paths that `op` may touch for the focused row.
///
/// Workspace rows resolve to visible primaries. Group rows yield no
/// targets. Hidden ignored repos are omitted
/// unless `show_ignored` is true. Linked worktrees are omitted unless the
/// focused row is that worktree (or a file inside it). Pull and
/// default-branch skip file and dir rows; fetch still includes
/// them.
pub fn op_targets(
    snapshot: &WorkspaceSnapshot,
    focused: Option<&VisibleRow>,
    show_ignored: bool,
    op: Op,
) -> Vec<String> {
    let visible: Vec<&WorkspaceRepoSnapshot> = snapshot
        .repos
        .iter()
        .filter(|repo| show_ignored || !repo.ignored)
        .collect();

    let Some(row) = focused else {
        return primaries_only(&visible);
    };

    if op_is_kind_noop(row.kind, op) {
        return Vec::new();
    }

    match row.kind {
        NodeKind::Workspace => primaries_only(&visible),
        NodeKind::Group => Vec::new(),
        NodeKind::Repo => {
            // Family container or flat primary: that primary only.
            let Some(repo) = row.repo.as_deref() else {
                return primaries_only(&visible);
            };
            include_if_visible(repo, &visible)
        }
        NodeKind::Checkout | NodeKind::File | NodeKind::Dir | NodeKind::Section => {
            let Some(repo) = row.repo.as_deref() else {
                return Vec::new();
            };
            include_if_visible(repo, &visible)
        }
    }
}

fn primaries_only(visible: &[&WorkspaceRepoSnapshot]) -> Vec<String> {
    visible
        .iter()
        .filter(|repo| repo.checkout_kind == CheckoutKind::Primary)
        .map(|repo| repo.repo.clone())
        .collect()
}

fn include_if_visible(repo: &str, visible: &[&WorkspaceRepoSnapshot]) -> Vec<String> {
    if visible.iter().any(|r| r.repo == repo) {
        vec![repo.to_string()]
    } else {
        Vec::new()
    }
}

/// Checkout path `r` should reload, or `None` for a whole-workspace reload.
///
/// Workspace and No-updates group (and no focus) reload everything. A
/// checkout, flat repo, file, or dir reloads that checkout only. Family
/// containers use the primary path.
pub fn refresh_target(focused: Option<&VisibleRow>) -> Option<String> {
    let row = focused?;
    match row.kind {
        NodeKind::Workspace | NodeKind::Group => None,
        NodeKind::Repo
        | NodeKind::Checkout
        | NodeKind::File
        | NodeKind::Dir
        | NodeKind::Section => row.repo.clone(),
    }
}

/// Push targets the focused visible checkout only.
///
/// Workspace and group rows do not fan out. Hidden ignored stay out.
/// A linked worktree is included only when that row is focused.
pub fn push_targets(
    snapshot: &WorkspaceSnapshot,
    focused: Option<&VisibleRow>,
    show_ignored: bool,
) -> Vec<String> {
    let Some(row) = focused else {
        return Vec::new();
    };
    match row.kind {
        NodeKind::Workspace
        | NodeKind::Group
        | NodeKind::File
        | NodeKind::Dir
        | NodeKind::Section => Vec::new(),
        NodeKind::Repo | NodeKind::Checkout => {
            let visible: Vec<&WorkspaceRepoSnapshot> = snapshot
                .repos
                .iter()
                .filter(|repo| show_ignored || !repo.ignored)
                .collect();
            let Some(repo) = row.repo.as_deref() else {
                return Vec::new();
            };
            include_if_visible(repo, &visible)
                .into_iter()
                .filter(|path| {
                    snapshot
                        .repos
                        .iter()
                        .any(|r| r.repo == *path && snapshot_pushable(r))
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::tui::tree::VisibleRow;

    fn snap(name: &str, _ignored: bool, linked: bool, behind: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: if behind {
                "feature/x".into()
            } else {
                "main".into()
            },
            sync_status: if behind {
                SyncStatus::Behind
            } else {
                SyncStatus::NoUpstream
            },
            sync_note: if behind {
                "behind by 1 commits".into()
            } else {
                String::new()
            },
            head: String::new(),
            has_unstaged: false,
            has_staged: false,
            has_untracked: false,
            changes: Vec::new(),
            checkout_kind: if linked {
                CheckoutKind::Linked
            } else {
                CheckoutKind::Primary
            },
            primary_repo: if linked { Some("app".into()) } else { None },
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
        }
    }

    fn file_row(repo: &str) -> VisibleRow {
        VisibleRow {
            id: format!("file:{repo}:README.md"),
            depth: 2,
            kind: NodeKind::File,
            label: "M README.md".into(),
            repo: Some(repo.into()),
            file: Some(FileChange {
                path: "README.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }),
            ..VisibleRow::default()
        }
    }

    fn group_row() -> VisibleRow {
        VisibleRow {
            id: "group:no-updates".into(),
            depth: 1,
            kind: NodeKind::Group,
            label: "No updates (1)".into(),
            foldable: true,
            folded: true,
            ..VisibleRow::default()
        }
    }

    fn workspace_row() -> VisibleRow {
        VisibleRow {
            id: "workspace".into(),
            kind: NodeKind::Workspace,
            label: "workspace".into(),
            foldable: true,
            ..VisibleRow::default()
        }
    }

    fn checkout_row(repo: &str) -> VisibleRow {
        VisibleRow {
            id: format!("checkout:{repo}"),
            depth: 2,
            kind: NodeKind::Checkout,
            label: repo.into(),
            repo: Some(repo.into()),
            primary_repo: Some("app".into()),
            ..VisibleRow::default()
        }
    }

    fn built() -> WorkspaceSnapshot {
        build_workspace_snapshot(
            &[
                snap("app", false, false, true),
                snap(".worktrees/app/feat", false, true, true),
                snap("notes", true, false, true),
            ],
            &["notes".into()],
            false,
            &[],
        )
    }

    #[test]
    fn workspace_fetch_skips_hidden_ignored_and_worktrees() {
        let snapshot = built();
        let targets = op_targets(&snapshot, Some(&workspace_row()), false, Op::Fetch);
        assert_eq!(targets, vec!["app"]);
    }

    #[test]
    fn show_ignored_includes_notes_on_workspace_fetch() {
        let snapshot = built();
        let targets = op_targets(&snapshot, Some(&workspace_row()), true, Op::Fetch);
        assert_eq!(targets, vec!["app", "notes"]);
    }

    #[test]
    fn group_fetch_pull_default_are_empty() {
        let snapshot = built();
        for op in [Op::Fetch, Op::Pull, Op::DefaultBranch] {
            assert!(
                op_targets(&snapshot, Some(&group_row()), false, op).is_empty(),
                "{op:?}"
            );
            assert!(
                op_targets(&snapshot, Some(&group_row()), true, op).is_empty(),
                "{op:?} shown ignored"
            );
        }
    }

    #[test]
    fn refresh_target_is_checkout_or_whole_workspace() {
        assert_eq!(refresh_target(Some(&workspace_row())), None);
        assert_eq!(refresh_target(Some(&group_row())), None);
        assert_eq!(refresh_target(None), None);
        assert_eq!(
            refresh_target(Some(&checkout_row(".worktrees/app/feat"))),
            Some(".worktrees/app/feat".into())
        );
        assert_eq!(refresh_target(Some(&file_row("app"))), Some("app".into()));
        assert_eq!(refresh_target(Some(&repo_row("lib"))), Some("lib".into()));
    }

    #[test]
    fn focused_worktree_is_the_only_target() {
        let snapshot = built();
        let targets = op_targets(
            &snapshot,
            Some(&checkout_row(".worktrees/app/feat")),
            false,
            Op::Pull,
        );
        assert_eq!(targets, vec![".worktrees/app/feat"]);
    }

    #[test]
    fn hidden_ignored_file_is_not_a_target() {
        let snapshot = built();
        let mut row = file_row("notes");
        row.ignored = true;
        let targets = op_targets(&snapshot, Some(&row), false, Op::DefaultBranch);
        assert!(targets.is_empty());
    }

    #[test]
    fn shown_ignored_file_is_a_target() {
        let snapshot = built();
        let mut row = file_row("notes");
        row.ignored = true;
        let targets = op_targets(&snapshot, Some(&row), true, Op::Fetch);
        assert_eq!(targets, vec!["notes"]);
    }

    fn repo_row(repo: &str) -> VisibleRow {
        VisibleRow {
            id: format!("repo:{repo}"),
            depth: 1,
            kind: NodeKind::Repo,
            label: repo.into(),
            repo: Some(repo.into()),
            ..VisibleRow::default()
        }
    }

    fn built_pushable() -> WorkspaceSnapshot {
        build_workspace_snapshot(
            &[
                snap("app", false, false, false),
                snap(".worktrees/app/feat", false, true, false),
                snap("notes", true, false, false),
            ],
            &["notes".into()],
            false,
            &[],
        )
    }

    #[test]
    fn push_skips_workspace_and_hidden_ignored() {
        let snapshot = built_pushable();
        assert!(push_targets(&snapshot, Some(&workspace_row()), false).is_empty());
        let mut notes = repo_row("notes");
        notes.ignored = true;
        assert!(push_targets(&snapshot, Some(&notes), false).is_empty());
        notes.ignored = true;
        assert_eq!(push_targets(&snapshot, Some(&notes), true), vec!["notes"]);
    }

    #[test]
    fn push_worktree_only_when_focused() {
        let snapshot = built_pushable();
        assert_eq!(
            push_targets(&snapshot, Some(&repo_row("app")), false),
            vec!["app"]
        );
        assert_eq!(
            push_targets(&snapshot, Some(&checkout_row(".worktrees/app/feat")), false),
            vec![".worktrees/app/feat"]
        );
        assert!(!push_targets(&snapshot, Some(&repo_row("app")), false)
            .contains(&".worktrees/app/feat".to_string()));
    }

    fn up_to_date(name: &str) -> RepoSnapshot {
        let mut row = snap(name, false, false, false);
        row.sync_status = SyncStatus::NoUpstream;
        row.sync_status = crate::snapshot::SyncStatus::UpToDate;
        row.sync_note = String::new();
        row.branch = "main".into();
        row
    }

    #[test]
    fn push_in_sync_is_empty_ahead_and_diverged_and_no_upstream_allowed() {
        let mut ahead = snap("app", false, false, false);
        ahead.sync_status = SyncStatus::Ahead;
        ahead.sync_note = "ahead 1".into();
        let mut diverged = snap("lib", false, false, false);
        diverged.repo = "lib".into();
        diverged.sync_status = SyncStatus::Diverged;
        diverged.sync_note = "diverged".into();
        let snapshot =
            build_workspace_snapshot(&[ahead, diverged, up_to_date("notes")], &[], false, &[]);
        assert_eq!(
            push_targets(&snapshot, Some(&repo_row("app")), false),
            vec!["app"]
        );
        assert_eq!(
            push_targets(&snapshot, Some(&repo_row("lib")), false),
            vec!["lib"]
        );
        assert!(push_targets(&snapshot, Some(&repo_row("notes")), false).is_empty());
        assert!(push_targets(&snapshot, Some(&workspace_row()), false).is_empty());
    }

    fn dirty(name: &str, ignored: bool, linked: bool, paths: &[&str]) -> RepoSnapshot {
        let mut row = snap(name, ignored, linked, true);
        row.has_unstaged = true;
        row.has_untracked = paths.iter().any(|p| *p != "README.md");
        row.changes = paths
            .iter()
            .map(|path| FileChange {
                path: (*path).into(),
                staged_status: None,
                unstaged_status: if *path == "README.md" {
                    Some("M".into())
                } else {
                    None
                },
                untracked: *path != "README.md",
                old_path: None,
            })
            .collect();
        row
    }

    #[test]
    fn collect_write_files_workspace_and_family_are_empty() {
        let snapshot = build_workspace_snapshot(
            &[
                dirty("app", false, false, &["README.md", "src/lib.rs"]),
                dirty(".worktrees/app/feat", false, true, &["wt.md"]),
                dirty("notes", true, false, &["secret.md"]),
                dirty("lib", false, false, &["a.rs"]),
            ],
            &["notes".into()],
            false,
            &[],
        );
        // Family container (app has a linked worktree): no mixed files.
        assert!(collect_write_files(&snapshot, Some(&repo_row("app")), false).is_empty());
        let files = collect_write_files(&snapshot, Some(&repo_row("lib")), false);
        assert_eq!(
            files
                .iter()
                .map(|f| f.change.path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.rs"]
        );
        assert!(files.iter().all(|f| f.repo == "lib"));
        assert!(collect_write_files(&snapshot, Some(&workspace_row()), false).is_empty());
        let file = collect_write_files(&snapshot, Some(&file_row("app")), false);
        assert_eq!(file.len(), 1);
        assert_eq!(file[0].change.path, "README.md");
    }

    #[test]
    fn y_deletes_only_sole_untracked() {
        assert!(!should_delete_untracked(&[false, true], false));
        assert!(should_delete_untracked(&[true], false));
        assert!(should_delete_untracked(&[false, true], true));
    }

    fn dir_row(repo: &str, dir: &str) -> VisibleRow {
        VisibleRow {
            id: format!("dir:{repo}:{dir}"),
            depth: 2,
            kind: NodeKind::Dir,
            label: dir.into(),
            repo: Some(repo.into()),
            foldable: true,
            ..VisibleRow::default()
        }
    }

    #[test]
    fn collect_write_files_dir_is_prefix_only() {
        let snapshot = build_workspace_snapshot(
            &[dirty(
                "app",
                false,
                false,
                &["README.md", "src/lib.rs", "src/main.rs"],
            )],
            &[],
            false,
            &[],
        );
        let files = collect_write_files(&snapshot, Some(&dir_row("app", "src")), false);
        let mut paths: Vec<_> = files.iter().map(|f| f.change.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, vec!["src/lib.rs", "src/main.rs"]);
        assert!(files.iter().all(|f| f.repo == "app"));
    }

    #[test]
    fn pull_and_default_skip_file_and_dir_fetch_stays() {
        let snapshot = built();
        let file = file_row("app");
        let dir = dir_row("app", "src");
        assert!(op_targets(&snapshot, Some(&file), false, Op::Pull).is_empty());
        assert!(op_targets(&snapshot, Some(&file), false, Op::DefaultBranch).is_empty());
        assert!(op_targets(&snapshot, Some(&dir), false, Op::Pull).is_empty());
        assert!(op_targets(&snapshot, Some(&dir), false, Op::DefaultBranch).is_empty());
        assert_eq!(
            op_targets(&snapshot, Some(&file), false, Op::Fetch),
            vec!["app"]
        );
        assert_eq!(
            op_targets(&snapshot, Some(&dir), false, Op::Fetch),
            vec!["app"]
        );
        assert!(op_is_kind_noop(NodeKind::File, Op::Pull));
        assert!(op_is_kind_noop(NodeKind::Dir, Op::DefaultBranch));
        assert!(!op_is_kind_noop(NodeKind::File, Op::Fetch));
        assert!(!op_is_kind_noop(NodeKind::Repo, Op::Pull));
    }

    #[test]
    fn collect_write_files_family_container_stays_noop() {
        let snapshot = build_workspace_snapshot(
            &[
                dirty("app", false, false, &["README.md", "src/lib.rs"]),
                dirty(".worktrees/app/feat", false, true, &["wt.md"]),
            ],
            &[],
            false,
            &[],
        );
        assert!(collect_write_files(&snapshot, Some(&repo_row("app")), false).is_empty());
    }

    fn is_stageable_change(change: &FileChange) -> bool {
        change.unstaged_status.is_some() || change.untracked
    }

    fn is_unstageable_change(change: &FileChange) -> bool {
        change.staged_status.is_some()
    }

    fn is_revertible_change(change: &FileChange) -> bool {
        change.unstaged_status.is_some() || change.untracked
    }

    fn colliding_dir_ms_snapshot() -> WorkspaceSnapshot {
        let mut row = dirty("app", false, false, &[]);
        row.has_staged = true;
        row.has_unstaged = true;
        row.changes = vec![
            FileChange {
                path: "pkg/a.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: None,
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "pkg/b.rs".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "pkg/both.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            },
        ];
        build_workspace_snapshot(&[row], &[], false, &[])
    }

    fn one_sided_nested_dirs_snapshot() -> WorkspaceSnapshot {
        let mut row = dirty("app", false, false, &[]);
        row.has_staged = true;
        row.has_unstaged = true;
        row.changes = vec![
            FileChange {
                path: "staged.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: None,
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "src/a.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: None,
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "docs/x.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            },
        ];
        build_workspace_snapshot(&[row], &[], false, &[])
    }

    fn mixed_section_snapshot() -> WorkspaceSnapshot {
        let mut row = dirty("app", false, false, &[]);
        row.has_staged = true;
        row.has_unstaged = true;
        row.has_untracked = true;
        row.changes = vec![
            FileChange {
                path: "src/a.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: None,
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "src/b.rs".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
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
                path: "new.ts".into(),
                staged_status: None,
                unstaged_status: None,
                untracked: true,
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
        build_workspace_snapshot(&[row], &[], false, &[])
    }

    fn mixed_section_rows(snapshot: &WorkspaceSnapshot) -> Vec<VisibleRow> {
        use crate::tui::tree::{build_tree, flatten, visible_for_tree};
        use std::collections::HashSet;
        let tree = build_tree(&visible_for_tree(snapshot), true, "ws");
        flatten(&tree, &HashSet::new())
    }

    fn write_paths(files: &[ScopedFile]) -> Vec<&str> {
        let mut paths: Vec<&str> = files.iter().map(|f| f.change.path.as_str()).collect();
        paths.sort();
        paths
    }

    #[test]
    fn collect_write_files_staged_section_unstage_filters_staged_paths() {
        let snapshot = mixed_section_snapshot();
        let rows = mixed_section_rows(&snapshot);
        let staged = rows
            .iter()
            .find(|r| r.id == "section:app:staged")
            .expect("staged section");
        let files = collect_write_files(&snapshot, Some(staged), false);
        assert_eq!(
            write_paths(&files),
            vec!["both.rs", "src/a.rs", "staged.rs"]
        );
        assert!(files.iter().all(|f| is_unstageable_change(&f.change)));
        assert!(files.iter().all(|f| !is_stageable_change(&f.change)));
        assert!(files.iter().all(|f| !is_revertible_change(&f.change)));
        let both = files
            .iter()
            .find(|f| f.change.path == "both.rs")
            .expect("both.rs");
        assert!(both.change.unstaged_status.is_none());
        assert!(!both.change.untracked);
    }

    #[test]
    fn collect_write_files_changes_section_stage_filters_unstaged_and_untracked() {
        let snapshot = mixed_section_snapshot();
        let rows = mixed_section_rows(&snapshot);
        let changes = rows
            .iter()
            .find(|r| r.id == "section:app:changes")
            .expect("changes section");
        let files = collect_write_files(&snapshot, Some(changes), false);
        assert_eq!(write_paths(&files), vec!["both.rs", "new.ts", "src/b.rs"]);
        assert!(files.iter().all(|f| is_stageable_change(&f.change)));
        assert!(files.iter().all(|f| !is_unstageable_change(&f.change)));
        let both = files
            .iter()
            .find(|f| f.change.path == "both.rs")
            .expect("both.rs");
        assert!(both.change.staged_status.is_none());
    }

    #[test]
    fn collect_write_files_dir_does_not_pull_the_other_section() {
        let snapshot = mixed_section_snapshot();
        let rows = mixed_section_rows(&snapshot);
        let staged_src = rows
            .iter()
            .find(|r| r.id == "dir:app:src")
            .expect("staged src dir");
        let changes_src = rows
            .iter()
            .find(|r| r.id == "dir:app:src#unstaged")
            .expect("changes src dir");
        assert_eq!(
            write_paths(&collect_write_files(&snapshot, Some(staged_src), false)),
            vec!["src/a.rs"]
        );
        assert_eq!(
            write_paths(&collect_write_files(&snapshot, Some(changes_src), false)),
            vec!["src/b.rs"]
        );
    }

    #[test]
    fn collect_write_files_ms_dual_file_rows_are_side_specific() {
        let snapshot = mixed_section_snapshot();
        let rows = mixed_section_rows(&snapshot);
        let staged_row = rows
            .iter()
            .find(|r| r.id == "file:app:both.rs")
            .expect("staged both.rs");
        let changes_row = rows
            .iter()
            .find(|r| r.id == "file:app:both.rs#unstaged")
            .expect("changes both.rs");
        let staged_files = collect_write_files(&snapshot, Some(staged_row), false);
        assert_eq!(staged_files.len(), 1);
        assert!(is_unstageable_change(&staged_files[0].change));
        assert!(!is_stageable_change(&staged_files[0].change));
        let changes_files = collect_write_files(&snapshot, Some(changes_row), false);
        assert_eq!(changes_files.len(), 1);
        assert!(is_stageable_change(&changes_files[0].change));
        assert!(!is_unstageable_change(&changes_files[0].change));
    }

    #[test]
    fn collect_write_files_changes_file_and_dir_are_stageable_not_unstageable() {
        let snapshot = mixed_section_snapshot();
        let rows = mixed_section_rows(&snapshot);
        let file = rows
            .iter()
            .find(|r| r.id == "file:app:src/b.rs")
            .expect("changes file");
        let files = collect_write_files(&snapshot, Some(file), false);
        assert_eq!(write_paths(&files), vec!["src/b.rs"]);
        assert!(files.iter().all(|f| is_stageable_change(&f.change)));
        assert!(files.iter().all(|f| !is_unstageable_change(&f.change)));
        let dir = rows
            .iter()
            .find(|r| r.id == "dir:app:src#unstaged")
            .expect("changes dir");
        let dir_files = collect_write_files(&snapshot, Some(dir), false);
        assert_eq!(write_paths(&dir_files), vec!["src/b.rs"]);
        assert!(dir_files.iter().all(|f| is_stageable_change(&f.change)));
    }

    #[test]
    fn collect_write_files_colliding_dir_ms_file_keeps_one_git_side() {
        let snapshot = colliding_dir_ms_snapshot();
        let rows = mixed_section_rows(&snapshot);
        let staged_dir = rows
            .iter()
            .find(|r| r.id == "dir:app:pkg")
            .expect("staged pkg dir");
        let changes_dir = rows
            .iter()
            .find(|r| r.id == "dir:app:pkg#unstaged")
            .expect("changes pkg dir");
        let staged_files = collect_write_files(&snapshot, Some(staged_dir), false);
        assert_eq!(write_paths(&staged_files), vec!["pkg/a.rs", "pkg/both.rs"]);
        assert!(staged_files
            .iter()
            .all(|f| is_unstageable_change(&f.change)));
        assert!(staged_files.iter().all(|f| !is_stageable_change(&f.change)));
        assert!(staged_files
            .iter()
            .all(|f| !is_revertible_change(&f.change)));
        let staged_both = staged_files
            .iter()
            .find(|f| f.change.path == "pkg/both.rs")
            .expect("staged pkg/both.rs");
        assert_eq!(staged_both.change.staged_status.as_deref(), Some("M"));
        assert!(staged_both.change.unstaged_status.is_none());
        assert!(!staged_both.change.untracked);
        let changes_files = collect_write_files(&snapshot, Some(changes_dir), false);
        assert_eq!(write_paths(&changes_files), vec!["pkg/b.rs", "pkg/both.rs"]);
        assert!(changes_files.iter().all(|f| is_stageable_change(&f.change)));
        assert!(changes_files
            .iter()
            .all(|f| !is_unstageable_change(&f.change)));
        let changes_both = changes_files
            .iter()
            .find(|f| f.change.path == "pkg/both.rs")
            .expect("changes pkg/both.rs");
        assert!(changes_both.change.staged_status.is_none());
        assert_eq!(changes_both.change.unstaged_status.as_deref(), Some("M"));
    }

    #[test]
    fn collect_write_files_one_sided_nested_dirs_keep_unsuffixed_ids() {
        let snapshot = one_sided_nested_dirs_snapshot();
        let rows = mixed_section_rows(&snapshot);
        assert!(rows.iter().any(|r| r.id == "dir:app:src"));
        assert!(rows.iter().any(|r| r.id == "dir:app:docs"));
        assert!(rows.iter().all(|r| r.id != "dir:app:src#unstaged"));
        assert!(rows.iter().all(|r| r.id != "dir:app:docs#unstaged"));
        let staged_src = rows
            .iter()
            .find(|r| r.id == "dir:app:src")
            .expect("staged-only src");
        let changes_docs = rows
            .iter()
            .find(|r| r.id == "dir:app:docs")
            .expect("changes-only docs");
        assert_eq!(
            write_paths(&collect_write_files(&snapshot, Some(staged_src), false)),
            vec!["src/a.rs"]
        );
        assert_eq!(
            write_paths(&collect_write_files(&snapshot, Some(changes_docs), false)),
            vec!["docs/x.md"]
        );
    }

    #[test]
    fn running_op_progress_uses_verb_done_over_total() {
        assert_eq!(format_running_op(RunningOp::Fetch, 0, 18), "Fetching 0/18…");
        assert_eq!(format_running_op(RunningOp::Pull, 2, 18), "Pulling 2/18…");
        assert_eq!(format_running_op(RunningOp::Push, 1, 3), "Pushing 1/3…");
        assert_eq!(
            format_running_op(RunningOp::DefaultBranch, 0, 5),
            "Switching 0/5…"
        );
        assert_eq!(format_running_op(RunningOp::Pull, 0, 0), "Pulling…");
    }

    #[test]
    fn completed_op_summary_counts_repos_and_failures() {
        assert_eq!(format_completed_op(RunningOp::Pull, 3, 0), "Pulled 3 repos");
        assert_eq!(
            format_completed_op(RunningOp::Fetch, 3, 1),
            "Fetched 4 repos (1 failed)"
        );
        assert_eq!(format_completed_op(RunningOp::Push, 1, 0), "Pushed 1 repo");
        assert_eq!(
            format_completed_op(RunningOp::Push, 0, 2),
            "Pushed 2 repos (2 failed)"
        );
        assert_eq!(
            format_completed_op(RunningOp::DefaultBranch, 2, 1),
            "Switched 3 repos (1 failed)"
        );
        let listed = format_completed_op(RunningOp::Fetch, 2, 0);
        assert!(!listed.contains("notes"), "{listed}");
        assert!(!listed.contains("dotfiles"), "{listed}");
    }
}
