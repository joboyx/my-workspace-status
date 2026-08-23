//! Graph model: commits, HEAD, sync, stash, worktrees, ignore visibility.

use crate::action::{Action, Effect};

/// Assembled graph payload for one checkout window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphModel {
    /// Newest-first commit window.
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
    /// When true, paint a dirty / uncommitted row above the commit list.
    pub uncommitted: bool,
}

/// Kind of annotated ref on a commit. Same set as Ink `GraphRefKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    /// `refs/heads/*`
    Local,
    /// `refs/remotes/*` (short name may be `origin/…`)
    Remote,
    /// `refs/tags/*`
    Tag,
}

/// A branch or tag label pointing at a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// One commit in the loaded window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Commit {
    /// Full commit id.
    pub id: String,
    /// First-line subject.
    pub subject: String,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    /// `stash@{n}` name.
    pub stash_ref: String,
    /// Stash subject.
    pub subject: String,
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
    /// Dirty / uncommitted working tree above the commit list.
    Uncommitted,
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
        let commit_ids: Vec<&str> = self.commits.iter().map(|c| c.id.as_str()).collect();
        let mut rows = Vec::new();

        if self.uncommitted {
            rows.push(GraphRow::Uncommitted);
        }

        for stash in &self.stashes {
            let attached = stash
                .parent_id
                .as_deref()
                .is_some_and(|parent| commit_ids.contains(&parent));
            if !attached {
                rows.push(GraphRow::Stash(stash.clone()));
            }
        }

        for commit in &self.commits {
            for stash in &self.stashes {
                if stash.parent_id.as_deref() == Some(commit.id.as_str()) {
                    rows.push(GraphRow::Stash(stash.clone()));
                }
            }
            let worktrees = self
                .worktrees
                .iter()
                .filter(|wt| wt.head_id.as_deref() == Some(commit.id.as_str()))
                .filter(|wt| self.show_ignored || !wt.ignored)
                .cloned()
                .collect();
            rows.push(GraphRow::Commit {
                is_head: self.head_id.as_deref() == Some(commit.id.as_str()),
                commit: commit.clone(),
                worktrees,
            });
        }

        for worktree in &self.worktrees {
            let attached = worktree
                .head_id
                .as_deref()
                .is_some_and(|id| commit_ids.contains(&id));
            if attached {
                continue;
            }
            if !self.show_ignored && worktree.ignored {
                continue;
            }
            rows.push(GraphRow::Worktree(worktree.clone()));
        }

        rows
    }
}
