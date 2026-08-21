//! Fetch / pull / default targets. Hidden ignored stay out unless shown.

use crate::snapshot::{CheckoutKind, WorkspaceRepoSnapshot, WorkspaceSnapshot};

use super::tree::{NodeKind, VisibleRow};

/// Workspace ops that must skip hidden ignored repos and unfocused worktrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Fetch,
    Pull,
    DefaultBranch,
}

/// Checkout paths that `op` may touch for the focused row.
///
/// Hidden ignored repos are omitted unless `show_ignored` is true.
/// Linked worktrees are omitted unless the focused row is that worktree
/// (or a file inside it).
pub fn op_targets(
    snapshot: &WorkspaceSnapshot,
    focused: Option<&VisibleRow>,
    show_ignored: bool,
    _op: Op,
) -> Vec<String> {
    let visible: Vec<&WorkspaceRepoSnapshot> = snapshot
        .repos
        .iter()
        .filter(|repo| show_ignored || !repo.ignored)
        .collect();

    let Some(row) = focused else {
        return primaries_only(&visible);
    };

    match row.kind {
        NodeKind::Workspace | NodeKind::Group => primaries_only(&visible),
        NodeKind::Repo => {
            // Family container or flat primary: that primary only.
            let Some(repo) = row.repo.as_deref() else {
                return primaries_only(&visible);
            };
            include_if_visible(repo, &visible)
        }
        NodeKind::Checkout | NodeKind::File => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::tree::VisibleRow;

    fn snap(
        name: &str,
        _ignored: bool,
        linked: bool,
        behind: bool,
    ) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: if behind { "feature/x".into() } else { "main".into() },
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
        }
    }

    fn file_row(repo: &str) -> VisibleRow {
        VisibleRow {
            id: format!("file:{repo}:README.md"),
            depth: 2,
            kind: NodeKind::File,
            label: "M README.md".into(),
            repo: Some(repo.into()),
            primary_repo: None,
            ignored: false,
            file: Some(FileChange {
                path: "README.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }),
            foldable: false,
            folded: false,
        }
    }

    fn workspace_row() -> VisibleRow {
        VisibleRow {
            id: "workspace".into(),
            depth: 0,
            kind: NodeKind::Workspace,
            label: "workspace".into(),
            repo: None,
            primary_repo: None,
            ignored: false,
            file: None,
            foldable: true,
            folded: false,
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
            ignored: false,
            file: None,
            foldable: false,
            folded: false,
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
}
