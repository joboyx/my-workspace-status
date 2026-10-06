//! Graph model: commits, HEAD, sync, stash, worktrees, ignore visibility.

use crate::action::{Action, Effect};

/// Default `git log --max-count` for one graph window.
pub const DEFAULT_GRAPH_WINDOW: usize = 300;

/// Assembled graph payload for one checkout window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphModel {
    /// Newest-first commit window, then optional extra `stash^1` commits.
    pub commits: Vec<Commit>,
    /// Stash entries. Visible rows park each stash above its parent.
    pub stashes: Vec<Stash>,
    /// Linked or extra worktrees. Ignored rows stay hidden unless shown.
    pub worktrees: Vec<Worktree>,
    /// Full SHA of `HEAD`. `None` when the checkout is empty.
    pub head_id: Option<String>,
    /// Current branch vs upstream.
    pub sync: Option<SyncState>,
    /// When true, rows marked ignored stay in [`GraphModel::visible_rows`].
    pub show_ignored: bool,
    /// Working-tree row. `None` omits it (empty fixtures). A loaded graph
    /// always sets `Some(has_changes)` so the row stays on even when clean.
    pub uncommitted: Option<bool>,
    /// Offset of this log window's start in `git log --all`. Stays `0` after
    /// an autoload merge.
    pub skip: usize,
    /// Page size used for this window (`--max-count`). Default 300.
    pub limit: usize,
    /// True when the log page filled `limit`.
    pub has_more: bool,
    /// Length of the `git log` prefix in [`GraphModel::commits`] (excludes
    /// extra stash parents). Autoload skip uses this, not `commits.len()`.
    pub window: usize,
    /// Configured default branch (`None` → main/master/develop).
    pub default_branch_override: Option<String>,
}

/// Kind of annotated ref on a commit. Same set as `GraphRefKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// `refs/heads/*`
    Local,
    /// `refs/remotes/*` (short name may be `origin/…`)
    Remote,
    /// `refs/tags/*`
    Tag,
}

/// A branch or tag label pointing at a commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GraphRef {
    /// Local, remote, or tag.
    pub kind: RefKind,
    /// Short name (`main`, `origin/main`, `v1.0`).
    pub name: String,
}

impl GraphRef {
    pub fn local(name: impl Into<String>) -> Self {
        Self {
            kind: RefKind::Local,
            name: name.into(),
        }
    }

    pub fn remote(name: impl Into<String>) -> Self {
        Self {
            kind: RefKind::Remote,
            name: name.into(),
        }
    }

    pub fn tag(name: impl Into<String>) -> Self {
        Self {
            kind: RefKind::Tag,
            name: name.into(),
        }
    }
}

impl From<&str> for GraphRef {
    fn from(name: &str) -> Self {
        if name.starts_with("origin/") {
            Self::remote(name)
        } else {
            Self::local(name)
        }
    }
}

impl From<String> for GraphRef {
    fn from(name: String) -> Self {
        if name.starts_with("origin/") {
            Self::remote(name)
        } else {
            Self::local(name)
        }
    }
}

/// Bytes kept from a git `%b` body. Longer bodies truncate with `…`.
pub const COMMIT_BODY_MAX_BYTES: usize = 8 * 1024;

/// Cap a git `%b` body. Trailing whitespace is dropped first.
pub fn cap_commit_body(body: &str) -> String {
    let body = body.trim_end();
    if body.len() <= COMMIT_BODY_MAX_BYTES {
        return body.to_string();
    }
    let mut out = String::new();
    let ellipsis_len = '…'.len_utf8();
    for ch in body.chars() {
        let next = out.len() + ch.len_utf8();
        if next + ellipsis_len > COMMIT_BODY_MAX_BYTES {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// One commit in the loaded window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commit {
    /// Full commit id.
    pub id: String,
    /// First-line subject (`%s`). List rows stay on this line.
    pub subject: String,
    /// Rest of the message (`%b`). Empty when git reported none.
    ///
    /// Capped at [`COMMIT_BODY_MAX_BYTES`].
    pub body: String,
    /// Parent ids, first parent first.
    pub parents: Vec<String>,
    /// Branch or tag labels that point at this commit.
    pub refs: Vec<GraphRef>,
    /// `git log` `%an` author name. Empty when unknown.
    pub author_name: String,
    /// `git log` `%at` author date (unix seconds). `0` when unknown.
    pub author_date_unix: i64,
}

/// One `git stash` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stash {
    /// Full stash commit id (`%H`). Empty when unknown.
    pub id: String,
    /// `stash@{n}` name.
    pub stash_ref: String,
    /// Stash subject.
    pub subject: String,
    /// Rest of the stash message (`%b`). Empty for the usual WIP subject.
    ///
    /// Capped at [`COMMIT_BODY_MAX_BYTES`].
    pub body: String,
    /// `git stash list` `%an` author name. Empty when unknown.
    pub author_name: String,
    /// `git stash list` `%at` author date (unix seconds). `0` when unknown.
    pub author_date_unix: i64,
    /// First parent (`stash^1`). `None` when git did not report it.
    pub parent_id: Option<String>,
}

/// One git worktree checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    /// Workspace-relative path of the checkout.
    pub path: String,
    /// HEAD commit of this worktree, when known.
    pub head_id: Option<String>,
    /// Checked-out branch, when not detached.
    pub branch: Option<String>,
    /// True when config lists this path as ignored.
    pub ignored: bool,
    /// True when this is the current checkout.
    pub is_current: bool,
}

/// Branch sync vs its upstream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncState {
    /// Current branch name.
    pub branch: String,
    /// Coarse sync class. Matches the workspace snapshot `syncStatus` words.
    pub status: SyncStatus,
    /// Commits ahead of upstream.
    pub ahead: u32,
    /// Commits behind upstream.
    pub behind: u32,
}

/// Coarse sync class for the current branch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncStatus {
    /// Tracking branch, no ahead or behind.
    #[default]
    UpToDate,
    /// No upstream configured.
    NoUpstream,
    /// Ahead of upstream only.
    Ahead,
    /// Behind upstream only.
    Behind,
    /// Ahead and behind.
    Diverged,
}

/// One visible graph row after ignore filtering and stash placement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphRow {
    /// Working-tree row above the commit list. Always present on a loaded graph.
    Uncommitted {
        /// True when the worktree or index is dirty.
        has_changes: bool,
    },
    /// Stash side-leaf.
    Stash(Stash),
    /// Commit node, optional HEAD mark, and attached visible worktrees.
    Commit {
        /// The commit.
        commit: Commit,
        /// True when `commit.id` is [`GraphModel::head_id`].
        is_head: bool,
        /// Worktrees whose HEAD is this commit and that pass the ignore filter.
        worktrees: Vec<Worktree>,
    },
    /// Worktree with no matching commit in the loaded window.
    Worktree(Worktree),
}

impl GraphModel {
    /// Log-window prefix length. `window` when set; otherwise `commits.len()`.
    ///
    /// Autoload skip is `skip + window_count()`, so extra `stash^1` commits
    /// appended after the prefix do not drop history.
    pub fn window_count(&self) -> usize {
        if self.window == 0 {
            self.commits.len()
        } else {
            self.window
        }
    }

    /// Apply an [`Action`]. Dispatch is pure and returns an [`Effect`].
    pub fn dispatch(&mut self, action: Action) -> Effect {
        match action {
            Action::ToggleShowIgnored => {
                self.show_ignored = !self.show_ignored;
            }
            Action::SetShowIgnored(show) => {
                self.show_ignored = show;
            }
        }
        Effect::None
    }

    /// Rows the widget paints, newest first, after ignore filtering.
    ///
    /// Stashes sit immediately above their `parent_id` commit. Orphan stashes
    /// sit after the uncommitted row. Hidden ignored worktrees are omitted.
    pub fn visible_rows(&self) -> Vec<GraphRow> {
        self.visible_row_refs()
            .map(|row| self.owned_row(row))
            .collect()
    }

    /// Row `index` of [`Self::visible_rows`], cloning only that row.
    pub fn visible_row_at(&self, index: usize) -> Option<GraphRow> {
        self.visible_row_refs()
            .nth(index)
            .map(|row| self.owned_row(row))
    }

    /// The [`Self::visible_rows`] order, borrowed from the model.
    fn visible_row_refs(&self) -> impl Iterator<Item = RowRef<'_>> + '_ {
        let in_window = move |id: Option<&str>| {
            id.is_some_and(|id| self.commits.iter().any(|commit| commit.id == id))
        };
        let orphan_stashes = self
            .stashes
            .iter()
            .filter(move |stash| !in_window(stash.parent_id.as_deref()))
            .map(RowRef::Stash);
        let commits = self.commits.iter().flat_map(move |commit| {
            self.stashes
                .iter()
                .filter(move |stash| stash.parent_id.as_deref() == Some(commit.id.as_str()))
                .map(RowRef::Stash)
                .chain(std::iter::once(RowRef::Commit(commit)))
        });
        let orphan_worktrees = self
            .worktrees
            .iter()
            .filter(move |worktree| !in_window(worktree.head_id.as_deref()))
            .filter(move |worktree| self.show_ignored || !worktree.ignored)
            .map(RowRef::Worktree);
        self.uncommitted
            .map(RowRef::Uncommitted)
            .into_iter()
            .chain(orphan_stashes)
            .chain(commits)
            .chain(orphan_worktrees)
    }

    /// Owned [`GraphRow`] for a borrowed row.
    fn owned_row(&self, row: RowRef<'_>) -> GraphRow {
        match row {
            RowRef::Uncommitted(has_changes) => GraphRow::Uncommitted { has_changes },
            RowRef::Stash(stash) => GraphRow::Stash(stash.clone()),
            RowRef::Commit(commit) => GraphRow::Commit {
                is_head: self.head_id.as_deref() == Some(commit.id.as_str()),
                commit: commit.clone(),
                worktrees: self
                    .worktrees
                    .iter()
                    .filter(|wt| wt.head_id.as_deref() == Some(commit.id.as_str()))
                    .filter(|wt| self.show_ignored || !wt.ignored)
                    .cloned()
                    .collect(),
            },
            RowRef::Worktree(worktree) => GraphRow::Worktree(worktree.clone()),
        }
    }
}

/// A [`GraphRow`] borrowed from its [`GraphModel`].
#[derive(Clone, Copy)]
enum RowRef<'a> {
    Uncommitted(bool),
    Stash(&'a Stash),
    Commit(&'a Commit),
    Worktree(&'a Worktree),
}

#[cfg(test)]
mod tests {
    use super::{
        cap_commit_body, Commit, GraphModel, GraphRow, Stash, Worktree, COMMIT_BODY_MAX_BYTES,
    };

    fn worktree(path: &str, head: Option<&str>, ignored: bool) -> Worktree {
        Worktree {
            path: path.into(),
            head_id: head.map(Into::into),
            branch: None,
            ignored,
            is_current: false,
        }
    }

    #[test]
    fn visible_row_at_matches_visible_rows() {
        let commit = |id: &str| Commit {
            id: id.into(),
            ..Commit::default()
        };
        let stash = |id: &str, parent: &str| Stash {
            id: id.into(),
            parent_id: Some(parent.into()),
            ..Stash::default()
        };
        let mut model = GraphModel {
            commits: vec![commit("c1"), commit("c2")],
            stashes: vec![stash("s1", "c2"), stash("s2", "gone")],
            worktrees: vec![
                worktree("/w/on-c1", Some("c1"), false),
                worktree("/w/orphan", Some("gone"), false),
                worktree("/w/ignored", None, true),
            ],
            head_id: Some("c1".into()),
            uncommitted: Some(true),
            ..GraphModel::default()
        };
        for show_ignored in [false, true] {
            model.show_ignored = show_ignored;
            let rows = model.visible_rows();
            assert_eq!(rows.len(), if show_ignored { 7 } else { 6 });
            assert!(matches!(
                rows[0],
                GraphRow::Uncommitted { has_changes: true }
            ));
            assert!(matches!(&rows[1], GraphRow::Stash(s) if s.id == "s2"));
            assert!(
                matches!(&rows[2], GraphRow::Commit { commit, is_head: true, worktrees }
                    if commit.id == "c1" && worktrees.len() == 1)
            );
            assert!(matches!(&rows[3], GraphRow::Stash(s) if s.id == "s1"));
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(model.visible_row_at(index).as_ref(), Some(row));
            }
            assert_eq!(model.visible_row_at(rows.len()), None);
        }
    }

    #[test]
    fn cap_commit_body_keeps_short_and_truncates_huge() {
        assert_eq!(cap_commit_body("  hi \n"), "  hi");
        assert_eq!(cap_commit_body(""), "");
        let huge = "x".repeat(COMMIT_BODY_MAX_BYTES + 80);
        let capped = cap_commit_body(&huge);
        assert!(capped.ends_with('…'), "{capped}");
        assert!(capped.len() <= COMMIT_BODY_MAX_BYTES);
        assert!(capped.chars().count() < huge.chars().count());
    }
}
