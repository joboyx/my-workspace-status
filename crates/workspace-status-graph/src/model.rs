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
    /// Linked or extra worktrees. Hidden rows stay hidden unless shown.
    pub worktrees: Vec<Worktree>,
    /// Full SHA of `HEAD`. `None` when the checkout is empty.
    pub head_id: Option<String>,
    /// Current branch vs upstream.
    pub sync: Option<SyncState>,
    /// When true, rows marked hidden stay in [`GraphModel::visible_rows`].
    pub show_hidden: bool,
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

/// A name and email pair, for example a `Co-authored-by` trailer value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Person {
    /// Display name. Empty when the source gave none.
    pub name: String,
    /// Email address without the angle brackets. Empty when the source gave none.
    pub email: String,
}

/// Who did something to a commit, and when, as git recorded it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Signature {
    /// Display name. Empty when unknown.
    pub name: String,
    /// Email address without the angle brackets. Empty when unknown.
    pub email: String,
    /// Unix seconds. `0` when unknown.
    pub unix: i64,
    /// Seconds east of UTC, as the commit recorded it. `None` when unknown.
    pub offset_secs: Option<i32>,
}

/// Identity data of a commit that the list rows do not paint.
///
/// Rows paint [`Commit::author_name`] and [`Commit::author_date_unix`]. The
/// commit popovers read the rest from here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitPeople {
    /// Author email (`%ae`). Empty when unknown.
    pub author_email: String,
    /// Author date offset (`%aI`) in seconds east of UTC. `None` when unknown.
    pub author_offset_secs: Option<i32>,
    /// Committer name, email, date, and offset (`%cn %ce %ct %cI`).
    pub committer: Signature,
    /// `Co-authored-by` trailers of the full message, in message order.
    pub co_authors: Vec<Person>,
}

/// Read the `Co-authored-by` trailers of a commit message body.
///
/// Only the last paragraph counts (after the last blank line, trailing
/// whitespace trimmed), as in git trailer rules. The key match ignores case.
/// A value `Name <email>` gives both parts; a value without `<…>` gives the
/// name only. Trailers elsewhere in the body are ignored.
pub fn co_author_trailers(body: &str) -> Vec<Person> {
    let body = body.trim_end();
    let mut paragraph_start = 0;
    let mut line_start = 0;
    for line in body.split_inclusive('\n') {
        if line.trim().is_empty() {
            paragraph_start = line_start + line.len();
        }
        line_start += line.len();
    }
    body[paragraph_start..]
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            if !key.trim().eq_ignore_ascii_case("co-authored-by") {
                return None;
            }
            Some(parse_person(value.trim()))
        })
        .collect()
}

/// Split `Name <email>` into a [`Person`]. A value without `<…>` is a name.
fn parse_person(value: &str) -> Person {
    if let (Some(open), Some(close)) = (value.rfind('<'), value.rfind('>')) {
        if open < close {
            return Person {
                name: value[..open].trim().to_string(),
                email: value[open + 1..close].trim().to_string(),
            };
        }
    }
    Person {
        name: value.to_string(),
        email: String::new(),
    }
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
    /// Emails, committer, date offsets, and co-authors. Not painted by rows.
    pub people: CommitPeople,
}

impl Commit {
    /// The author as a [`Signature`]: row name and date plus the emails and
    /// offset in [`Commit::people`].
    pub fn author(&self) -> Signature {
        Signature {
            name: self.author_name.clone(),
            email: self.people.author_email.clone(),
            unix: self.author_date_unix,
            offset_secs: self.people.author_offset_secs,
        }
    }
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
    /// Emails, committer, date offsets, and co-authors. Not painted by rows.
    pub people: CommitPeople,
}

impl Stash {
    /// The author as a [`Signature`]: row name and date plus the emails and
    /// offset in [`Stash::people`].
    pub fn author(&self) -> Signature {
        Signature {
            name: self.author_name.clone(),
            email: self.people.author_email.clone(),
            unix: self.author_date_unix,
            offset_secs: self.people.author_offset_secs,
        }
    }
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
    /// True when config lists this path as hidden.
    pub hidden: bool,
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
            Action::ToggleShowHidden => {
                self.show_hidden = !self.show_hidden;
            }
            Action::SetShowHidden(show) => {
                self.show_hidden = show;
            }
        }
        Effect::None
    }

    /// Rows the widget paints, newest first, after ignore filtering.
    ///
    /// Stashes sit immediately above their `parent_id` commit. Orphan stashes
    /// sit after the uncommitted row. Hidden worktrees are omitted.
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

    /// The [`Self::visible_rows`] row of commit `id`, cloning only that row.
    /// `None` when the commit is not loaded.
    pub fn commit_row(&self, id: &str) -> Option<GraphRow> {
        let commit = self.commits.iter().find(|commit| commit.id == id)?;
        Some(self.owned_row(RowRef::Commit(commit)))
    }

    /// The [`Self::visible_rows`] row of stash `stash_ref` (`stash@{n}`),
    /// cloning only that row. `None` when the stash is not loaded.
    pub fn stash_row(&self, stash_ref: &str) -> Option<GraphRow> {
        let stash = self
            .stashes
            .iter()
            .find(|stash| stash.stash_ref == stash_ref)?;
        Some(self.owned_row(RowRef::Stash(stash)))
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
            .filter(move |worktree| self.show_hidden || !worktree.hidden)
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
                    .filter(|wt| self.show_hidden || !wt.hidden)
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
        cap_commit_body, co_author_trailers, Commit, GraphModel, GraphRow, Person, Stash, Worktree,
        COMMIT_BODY_MAX_BYTES,
    };

    fn person(name: &str, email: &str) -> Person {
        Person {
            name: name.into(),
            email: email.into(),
        }
    }

    #[test]
    fn co_author_trailers_read_the_last_paragraph_only() {
        let body = "Co-authored-by: Early Bird <early@example.com>\n\nWhy this change.\n\n\
                    Co-authored-by: Bob Builder <bob@example.com>\n\
                    co-AUTHORED-by: Cy Coder <cy@example.com>\n\
                    Co-authored-by: Plain Name\n\
                    Signed-off-by: Ada Lovelace <ada@example.com>\n\n";
        assert_eq!(
            co_author_trailers(body),
            vec![
                person("Bob Builder", "bob@example.com"),
                person("Cy Coder", "cy@example.com"),
                person("Plain Name", ""),
            ]
        );
        assert!(co_author_trailers("").is_empty());
        assert!(
            co_author_trailers("Co-authored-by: Early <e@example.com>\n\nTail text").is_empty()
        );
    }

    fn worktree(path: &str, head: Option<&str>, hidden: bool) -> Worktree {
        Worktree {
            path: path.into(),
            head_id: head.map(Into::into),
            branch: None,
            hidden,
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
            stash_ref: format!("ref-{id}"),
            parent_id: Some(parent.into()),
            ..Stash::default()
        };
        let mut model = GraphModel {
            commits: vec![commit("c1"), commit("c2")],
            stashes: vec![stash("s1", "c2"), stash("s2", "gone")],
            worktrees: vec![
                worktree("/w/on-c1", Some("c1"), false),
                worktree("/w/orphan", Some("gone"), false),
                worktree("/w/hidden", None, true),
            ],
            head_id: Some("c1".into()),
            uncommitted: Some(true),
            ..GraphModel::default()
        };
        for show_hidden in [false, true] {
            model.show_hidden = show_hidden;
            let rows = model.visible_rows();
            assert_eq!(rows.len(), if show_hidden { 7 } else { 6 });
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
            // The finders return the same row as the visible list.
            assert_eq!(model.commit_row("c1").as_ref(), Some(&rows[2]));
            assert_eq!(model.commit_row("gone"), None);
            assert_eq!(model.stash_row("ref-s1").as_ref(), Some(&rows[3]));
            assert_eq!(model.stash_row("ref-none"), None);
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
