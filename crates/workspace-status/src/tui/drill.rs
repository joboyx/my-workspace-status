//! Commit-files drill stack and graph-row source mapping.

use workspace_status_graph::GraphRow;

use crate::git::NameStatus;

use super::diff::DiffContent;

/// Git object that a commit-file list / diff is scoped to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitFileSource {
    /// First-parent files of a commit.
    Commit { commit_id: String },
    /// Files recorded in a stash entry.
    Stash { stash_ref: String },
    /// Dirty worktree versus HEAD.
    Worktree,
}

/// One file in a commit / stash / worktree list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitFile {
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
}

impl From<NameStatus> for CommitFile {
    fn from(row: NameStatus) -> Self {
        Self {
            status: row.status,
            path: row.path,
            old_path: row.old_path,
        }
    }
}

/// Right-pane drill depth: graph → file list → file diff.
///
/// Depth 1 puts the graph list on the left. Depth 2 puts the commit-file
/// list on the left (right is the numbered file diff).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrillView {
    /// Graph (or the tree-file worktree diff when the tree focuses a file).
    Graph,
    /// Depth 1: files in the selected commit / stash / worktree.
    Files {
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
        cursor: usize,
    },
    /// Depth 2: numbered file diff (commit-file list on the left).
    Diff {
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
        file_cursor: usize,
        path: String,
        content: DiffContent,
    },
}

impl DrillView {
    pub fn is_graph(&self) -> bool {
        matches!(self, Self::Graph)
    }

    pub fn is_files(&self) -> bool {
        matches!(self, Self::Files { .. })
    }

    pub fn is_diff(&self) -> bool {
        matches!(self, Self::Diff { .. })
    }

    pub fn files_cursor(files: &[CommitFile], cursor: usize) -> usize {
        if files.is_empty() {
            0
        } else {
            cursor.min(files.len() - 1)
        }
    }
}

/// Map a focused graph row to a commit-file source.
pub fn source_from_graph_row(row: &GraphRow) -> Option<CommitFileSource> {
    match row {
        GraphRow::Commit { commit, .. } => Some(CommitFileSource::Commit {
            commit_id: commit.id.clone(),
        }),
        GraphRow::Stash(stash) => Some(CommitFileSource::Stash {
            stash_ref: stash.stash_ref.clone(),
        }),
        GraphRow::Uncommitted { .. } => Some(CommitFileSource::Worktree),
        GraphRow::Worktree(_) => None,
    }
}

/// Stash ref when the focused graph row is a stash.
pub fn stash_ref_from_graph_row(row: &GraphRow) -> Option<String> {
    match row {
        GraphRow::Stash(stash) => Some(stash.stash_ref.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workspace_status_graph::{Commit, Stash};

    fn commit_row(id: &str) -> GraphRow {
        GraphRow::Commit {
            commit: Commit {
                id: id.into(),
                subject: "s".into(),
                parents: Vec::new(),
                refs: Vec::new(),
                author_name: String::new(),
                author_date_unix: 0,
            },
            is_head: true,
            worktrees: Vec::new(),
        }
    }

    #[test]
    fn graph_row_maps_to_commit_stash_worktree() {
        assert_eq!(
            source_from_graph_row(&commit_row("abc")),
            Some(CommitFileSource::Commit {
                commit_id: "abc".into()
            })
        );
        assert_eq!(
            source_from_graph_row(&GraphRow::Stash(Stash {
                stash_ref: "stash@{2}".into(),
                subject: "wip".into(),
                parent_id: None,
                ..Stash::default()
            })),
            Some(CommitFileSource::Stash {
                stash_ref: "stash@{2}".into()
            })
        );
        assert_eq!(
            source_from_graph_row(&GraphRow::Uncommitted { has_changes: true }),
            Some(CommitFileSource::Worktree)
        );
        assert_eq!(stash_ref_from_graph_row(&commit_row("abc")), None);
        assert_eq!(
            stash_ref_from_graph_row(&GraphRow::Stash(Stash {
                stash_ref: "stash@{1}".into(),
                subject: "wip".into(),
                parent_id: None,
                ..Stash::default()
            }))
            .as_deref(),
            Some("stash@{1}")
        );
    }

    #[test]
    fn files_cursor_clamps() {
        assert_eq!(DrillView::files_cursor(&[], 3), 0);
        let files = vec![
            CommitFile {
                status: "M".into(),
                path: "a".into(),
                old_path: None,
            },
            CommitFile {
                status: "A".into(),
                path: "b".into(),
                old_path: None,
            },
        ];
        assert_eq!(DrillView::files_cursor(&files, 9), 1);
    }
}
