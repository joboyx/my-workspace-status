//! Current-line blame: which line has focus, what it asks git, and what
//! the pane paints at its end.
//!
//! Only a focused line asks: the active file tab's cursor line, or the
//! focused file-diff row while the right pane drives the diff. Moving
//! through the tree, graph, or file lists never asks. The question is a
//! [`BlameKey`]; the interpreter runs it on the blocking pool and lands
//! the answer through [`AppState::apply_line_blame`].

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use workspace_status_graph::GraphRow;

use super::super::action::{Action, Effect};
use super::super::diff::{row_line_ref, DiffCellKind, DiffSection, RowLineRef};
use super::super::drill::{CommitFileSource, DrillView};
use super::super::gates::ListFocusTarget;
use super::super::line_blame::{
    annotation_text, short_sha, BlameKey, BlameSide, GraphReveal, LineAnnotation, BLAME_IS_OFF,
    BLAME_MENU_ROWS, BLAME_STILL_LOADING, FOCUS_A_DIFF_OR_FILE_LINE, GRAPH_REVEAL_MAX_PAGES,
    LINE_NOT_COMMITTED, NO_BLAME_FOR_LINE, STAGED_TEXT, UNCOMMITTED_TEXT,
};
use super::super::search::unfold_ancestors;
use super::super::split::DiffMode;
use super::super::status::StatusMessage;
use super::super::tabs::{WorktreeFile, ROOT_COMMIT_HAS_NO_PARENT};
use super::super::tree::NodeKind;
use super::super::watch::graph_row_id;
use super::{single_or_batch, AppState, FocusPane};
use crate::git::{BlameRev, LineBlame, PreviousLineChange};

/// Where the focused line's annotation comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FocusedBlame {
    /// Ask git (through the cache) for this key.
    Ask(BlameKey),
    /// Text the diff already proves, with no git call.
    Fixed(&'static str),
}

/// Memo key for [`AppState::focused_row_line`]: content fingerprint,
/// painted layout, and diff cursor.
pub(super) type RowLineMemo = Option<((u64, DiffMode, usize), Option<RowLineRef>)>;

impl AppState {
    /// The blame question for the focused line, or `None` when line blame
    /// is off, no line has focus, or the line needs no git call.
    ///
    /// The interpreter reads this after every schedule and apply, and asks
    /// git only when the cache has no answer for it.
    pub(crate) fn line_blame_want(&self) -> Option<BlameKey> {
        match self.focused_blame()?.0 {
            FocusedBlame::Ask(key) => Some(key),
            FocusedBlame::Fixed(_) => None,
        }
    }

    /// What the focused line shows at its end, and on which side of a
    /// split row, at `now_unix`.
    ///
    /// [`LineAnnotation::Loading`] until git answers. `None` paints
    /// nothing: line blame off, no focused line, git had no blame for it,
    /// or a drag text selection is active (release copies the painted
    /// screen cells, so the annotation must not be there).
    pub(crate) fn focused_line_annotation(
        &self,
        now_unix: i64,
    ) -> Option<(LineAnnotation, BlameSide)> {
        if self.text_selection.is_some() {
            return None;
        }
        let (blame, side) = self.focused_blame()?;
        let note = match blame {
            FocusedBlame::Fixed(text) => LineAnnotation::Text(text.into()),
            FocusedBlame::Ask(key) => match self.line_blame.cached(&key) {
                None => LineAnnotation::Loading,
                Some(None) => return None,
                Some(Some(blame)) => LineAnnotation::Text(annotation_text(blame, now_unix)),
            },
        };
        Some((note, side))
    }

    /// Text to paint at the end of the focused line now, and its side of
    /// a split row. `None` while loading or when nothing shows.
    pub(crate) fn painted_line_annotation(&self) -> Option<(String, BlameSide)> {
        match self.focused_line_annotation(super::unix_now())? {
            (LineAnnotation::Text(text), side) => Some((text, side)),
            (LineAnnotation::Loading, _) => None,
        }
    }

    /// Land a blame answer for `key`. Every answer fills the cache, so a
    /// line the cursor already left shows at once when it comes back. A
    /// failed run caches as no blame.
    ///
    /// Returns true when `key` is the focused line (the pane needs a
    /// paint). Dropped while line blame is off.
    pub(crate) fn apply_line_blame(
        &mut self,
        key: BlameKey,
        result: Result<Option<LineBlame>, String>,
    ) -> bool {
        if !self.line_blame.enabled {
            return false;
        }
        let wanted = self.line_blame_want().as_ref() == Some(&key);
        self.line_blame.insert(key, result.unwrap_or(None));
        wanted
    }

    /// `B`: line blame on / off for this session. Never written to config.
    pub(super) fn toggle_line_blame(&mut self) -> Effect {
        let on = !self.line_blame.enabled;
        self.line_blame.set_enabled(on);
        self.status = if on {
            "line blame on".into()
        } else {
            "line blame off".into()
        };
        Effect::None
    }

    /// Why a blame action cannot run, or `None` when it may (also for
    /// every other action).
    ///
    /// The one blame gate. [`Self::dispatch`] puts the reason on the
    /// status line and `palette_disabled_reason` paints it on the row, so
    /// a key and a palette row give the same copy. The actions run on the
    /// Workspace, compare, and file tabs.
    pub(crate) fn blame_refusal(&self, action: &Action) -> Option<String> {
        if !matches!(
            action,
            Action::BlameCommitVsParent
                | Action::BlamePreviousChange
                | Action::BlameCommitVsWorktree
                | Action::BlameRevealGraph
                | Action::BlameMenu
        ) {
            return None;
        }
        match self.blamed_line() {
            Err(reason) => Some(reason.into()),
            Ok((_, blame)) if blame.boundary && *action == Action::BlameCommitVsParent => {
                Some(ROOT_COMMIT_HAS_NO_PARENT.into())
            }
            Ok(_) => None,
        }
    }

    /// `A`: open the blame-actions menu. [`Self::blame_refusal`] has
    /// already refused a line with no committed blame, with the copy the
    /// actions use. A root commit still opens it: `c` refuses on pick.
    /// A drag text selection hides the annotation, so it ends here and
    /// the header shows the line's blame.
    pub(super) fn open_blame_menu(&mut self) -> Effect {
        // The box lists each action with its key; a status would repeat it.
        self.status.clear();
        self.cancel_mouse_drag();
        self.blame_menu = true;
        Effect::None
    }

    /// A key in the blame-actions menu: `Some(key)` picks the row with
    /// that key, `None` (Enter) the first row. A pick closes the menu and
    /// dispatches the row's blame action, which runs its own gate. Other
    /// keys leave the menu open.
    pub(super) fn blame_menu_pick(&mut self, key: Option<char>) -> Effect {
        let row = match key {
            Some(key) => BLAME_MENU_ROWS.iter().find(|row| row.key == key),
            None => BLAME_MENU_ROWS.first(),
        };
        let Some(row) = row else {
            return Effect::None;
        };
        self.blame_menu = false;
        self.dispatch(row.action.clone())
    }

    /// Checkout and committed blame of the focused line, or why the blame
    /// actions refuse.
    fn blamed_line(&self) -> Result<(String, LineBlame), &'static str> {
        if !self.line_blame.enabled {
            return Err(BLAME_IS_OFF);
        }
        if let Some(tab) = self.tabs.active_file() {
            if tab.body.is_none() {
                return Err(BLAME_STILL_LOADING);
            }
        } else if self.list_focus_target() != ListFocusTarget::None
            || self.folder_summary().is_some()
            || self.open_diff_target().is_none()
        {
            return Err(FOCUS_A_DIFF_OR_FILE_LINE);
        }
        match self.focused_blame().map(|(blame, _)| blame) {
            None => Err(NO_BLAME_FOR_LINE),
            Some(FocusedBlame::Fixed(_)) => Err(LINE_NOT_COMMITTED),
            Some(FocusedBlame::Ask(key)) => match self.line_blame.cached(&key) {
                None => Err(BLAME_STILL_LOADING),
                Some(None) => Err(NO_BLAME_FOR_LINE),
                Some(Some(blame)) if blame.uncommitted => Err(LINE_NOT_COMMITTED),
                Some(Some(blame)) => Ok((key.repo, blame.clone())),
            },
        }
    }

    /// Blame: open commit changes. Open or focus `<sha>^...<sha>` with the
    /// head pinned to the focused line's blame commit, in the surface's
    /// checkout. Never checks anything out. [`Self::blame_refusal`] has
    /// already refused everything else.
    pub(super) fn blame_commit_vs_parent(&mut self) -> Effect {
        let Ok((repo, blame)) = self.blamed_line() else {
            return Effect::None;
        };
        self.open_compare_tab(repo, format!("{}^", blame.sha), blame.sha)
    }

    /// Blame: diff commit to working tree. Open or focus the tab of the
    /// focused line's file at its blame commit (under the file's path
    /// there) against the file on disk at the surface's path now.
    pub(super) fn blame_commit_vs_worktree(&mut self) -> Effect {
        let Ok((repo, blame)) = self.blamed_line() else {
            return Effect::None;
        };
        let Some(path) = self.blamed_disk_path() else {
            return Effect::None;
        };
        let old_path =
            (!blame.filename.is_empty() && blame.filename != path).then_some(blame.filename);
        self.open_worktree_compare_tab(repo, blame.sha, WorktreeFile { path, old_path })
    }

    /// The focused line's file on disk: the file tab's file, else the open
    /// diff's path.
    fn blamed_disk_path(&self) -> Option<String> {
        if let Some(tab) = self.tabs.active_file() {
            return Some(tab.rel.clone());
        }
        self.open_diff_target().map(|(_, path, _)| path.to_string())
    }

    /// Blame: open previous line change. Starts the
    /// [`Effect::LoadBlamePrevious`] search; [`Self::apply_blame_previous`]
    /// lands it.
    pub(super) fn blame_previous_change(&mut self) -> Effect {
        let Ok((repo, blame)) = self.blamed_line() else {
            return Effect::None;
        };
        self.blame_previous_gen += 1;
        self.status = StatusMessage::progress("finding previous change…");
        Effect::LoadBlamePrevious {
            gen: self.blame_previous_gen,
            repo,
            blame: Box::new(blame),
        }
    }

    /// Land a previous-line-change search for request `gen` on blame
    /// commit `sha` in `repo`.
    ///
    /// A stale `gen` is dropped (`None`). An earlier commit opens its own
    /// `<prev>^...<prev>` tab; the returned effect loads it. A line the
    /// commit added, or an earlier change in a root commit, says so on the
    /// status line.
    pub(crate) fn apply_blame_previous(
        &mut self,
        gen: u64,
        repo: String,
        sha: &str,
        result: Result<PreviousLineChange, String>,
    ) -> Option<Effect> {
        if gen != self.blame_previous_gen {
            return None;
        }
        match result {
            Ok(PreviousLineChange::AddedIn) => {
                self.status = StatusMessage::warn(format!("line was added in {}", short_sha(sha)));
                None
            }
            Ok(PreviousLineChange::Found(prev)) if prev.boundary => {
                self.status = StatusMessage::warn(format!(
                    "earlier change {} is the root commit",
                    short_sha(&prev.sha)
                ));
                None
            }
            Ok(PreviousLineChange::Found(prev)) => {
                self.status.clear();
                Some(self.open_compare_tab(repo, format!("{}^", prev.sha), prev.sha))
            }
            Err(err) => {
                self.status = StatusMessage::error(err);
                None
            }
        }
    }

    /// Blame: show commit in graph. Goes to the Workspace tab, leaves a
    /// commit drill, puts the tree cursor on the checkout, and focuses its
    /// graph. The commit row is selected now when the graph holds it, else
    /// after the graph loads ([`Self::retry_graph_reveal`]) or after older
    /// pages ([`Self::graph_reveal_wants_older`]).
    pub(super) fn blame_reveal_graph(&mut self) -> Effect {
        let Ok((repo, blame)) = self.blamed_line() else {
            return Effect::None;
        };
        // The Workspace graph does not reload while another tab is
        // active, so a parked graph may miss newer commits.
        let from_other_tab = !self.tabs.is_workspace();
        if from_other_tab {
            // A tab switch only swaps session state; it loads nothing.
            let switched = self.activate_tab(0);
            debug_assert_eq!(switched, Effect::None);
        }
        let mut effects = Vec::new();
        if !self.drill.is_graph() {
            self.drill = DrillView::Graph;
            effects.push(Effect::DropCommitDiff);
        }
        let moved = match self.focus_checkout_row(&repo) {
            Some(moved) => moved,
            None => {
                self.status = StatusMessage::warn(format!("{repo} is not in the tree"));
                return single_or_batch(effects);
            }
        };
        self.focus = FocusPane::Right;
        self.graph_reveal = Some(GraphReveal {
            repo: repo.clone(),
            sha: blame.sha,
            pages: 0,
            seen_load: false,
        });
        let loaded = self.graph.is_some()
            && self
                .graph_identity
                .as_ref()
                .is_some_and(|(graph_repo, _)| *graph_repo == repo);
        // The graph on screen may already hold the commit. Only a graph
        // with no load pending counts as seen: after a cursor move or a
        // switch from another tab the pane loads again, and widening or
        // giving up waits for that load.
        let reload = !loaded || moved || from_other_tab;
        if loaded {
            self.retry_graph_reveal(&repo, !reload);
        }
        if reload {
            effects.push(Effect::LoadRightPane);
        }
        single_or_batch(effects)
    }

    /// Put the tree cursor on `repo`'s checkout row (or its repo row),
    /// unfolding its ancestors. `Some(true)` when the cursor moved,
    /// `Some(false)` when it was already there, `None` when the tree has
    /// no such row.
    fn focus_checkout_row(&mut self, repo: &str) -> Option<bool> {
        if self.focused_row().is_some_and(|row| {
            matches!(row.kind, NodeKind::Repo | NodeKind::Checkout)
                && row.repo.as_deref() == Some(repo)
        }) && self.focused_graph_repo().as_deref() == Some(repo)
        {
            return Some(false);
        }
        for id in [format!("checkout:{repo}"), format!("repo:{repo}")] {
            let folds = unfold_ancestors(&self.tree, &self.folds, &id);
            if folds != self.folds {
                self.folds = folds;
                self.rebuild_rows();
            }
            if let Some(idx) = self.rows.iter().position(|row| row.id == id) {
                self.cursor = idx;
                return Some(true);
            }
        }
        None
    }

    /// Select the pending reveal's commit when the graph of `repo` holds
    /// it. Called after every [`Self::set_graph`] (`seen` true: the graph
    /// just loaded) and once when the reveal starts; a graph of another
    /// repo drops the reveal.
    pub(super) fn retry_graph_reveal(&mut self, repo: &str, seen: bool) {
        let Some(reveal) = self.graph_reveal.as_mut() else {
            return;
        };
        if reveal.repo != repo {
            self.graph_reveal = None;
            return;
        }
        reveal.seen_load |= seen;
        let reveal = &*reveal;
        let want = format!("commit:{}", reveal.sha);
        let Some(idx) = self.graph.as_ref().and_then(|model| {
            model.visible_rows().iter().position(|row| match row {
                GraphRow::Stash(stash) => stash.id == reveal.sha,
                row => graph_row_id(row) == want,
            })
        }) else {
            return;
        };
        self.graph_cursor = idx;
        self.sync_graph_scroll();
        self.graph_reveal = None;
    }

    /// Drop a pending reveal once the Workspace graph pane loses focus,
    /// so a later load of that graph does not move the cursor.
    pub(crate) fn drop_graph_reveal_off_graph(&mut self) {
        if self.graph_reveal.is_some() && self.tabs.is_workspace() && !self.graph_pane_focused() {
            self.graph_reveal = None;
        }
    }

    /// True when the pending reveal needs one more page of older graph
    /// rows; counts the page. Past [`GRAPH_REVEAL_MAX_PAGES`], or when the
    /// history ends, the reveal gives up with a status.
    ///
    /// Waits (false) until the reveal has seen a graph load
    /// ([`GraphReveal::seen_load`]), and while an older page is already on
    /// its way.
    pub(crate) fn graph_reveal_wants_older(&mut self) -> bool {
        let Some(reveal) = self.graph_reveal.as_mut() else {
            return false;
        };
        let Some(model) = self.graph.as_ref() else {
            return false;
        };
        let same_repo = self
            .graph_identity
            .as_ref()
            .is_some_and(|(repo, _)| *repo == reveal.repo);
        if !same_repo || !reveal.seen_load || self.graph_loading_older {
            return false;
        }
        if model.has_more && reveal.pages < GRAPH_REVEAL_MAX_PAGES {
            reveal.pages += 1;
            return true;
        }
        let hint = if self.graph_branch_focus.is_some() {
            " (graph focus on, O clears)"
        } else {
            ""
        };
        self.status = StatusMessage::warn(format!(
            "{} is not in the loaded graph{hint}",
            short_sha(&reveal.sha)
        ));
        self.graph_reveal = None;
        false
    }

    /// True when the focused line asks git and the cache has no answer yet.
    #[cfg(test)]
    pub(crate) fn line_blame_loading(&self) -> bool {
        self.line_blame_want()
            .is_some_and(|key| self.line_blame.cached(&key).is_none())
    }

    fn focused_blame(&self) -> Option<(FocusedBlame, BlameSide)> {
        if !self.line_blame.enabled {
            return None;
        }
        if self.is_file_tab() {
            let tab = self.tabs.active_file()?;
            if tab.cursor >= tab.lines().len() {
                return None;
            }
            let mut hasher = DefaultHasher::new();
            (tab.id, tab.body_generation).hash(&mut hasher);
            let key = BlameKey {
                repo: tab.checkout.clone(),
                rev: BlameRev::Worktree,
                path: tab.rel.clone(),
                line: u32::try_from(tab.cursor + 1).ok()?,
                epoch: hasher.finish(),
            };
            return Some((FocusedBlame::Ask(key), BlameSide::New));
        }
        if self.list_focus_target() != ListFocusTarget::None || self.folder_summary().is_some() {
            return None;
        }
        let (repo, path, source) = self.open_diff_target()?;
        let content = self.current_diff_content();
        let fingerprint = content.syntax_fingerprint();
        let line = self.focused_row_line(fingerprint)?;
        let side = if line.kind == DiffCellKind::Del {
            BlameSide::Old
        } else {
            BlameSide::New
        };
        let ask = |rev: BlameRev, path: &str, line: Option<u32>, epoch: u64| {
            let line = line.filter(|n| *n > 0)?;
            Some(FocusedBlame::Ask(BlameKey {
                repo: repo.to_string(),
                rev,
                path: path.to_string(),
                line,
                epoch,
            }))
        };
        let blame = match source {
            None | Some(CommitFileSource::Worktree) => {
                let head = || BlameRev::Commit("HEAD".into());
                match (line.section, line.kind) {
                    (DiffSection::Unstaged, DiffCellKind::Add) => {
                        Some(FocusedBlame::Fixed(UNCOMMITTED_TEXT))
                    }
                    (DiffSection::Staged, DiffCellKind::Add) => {
                        Some(FocusedBlame::Fixed(STAGED_TEXT))
                    }
                    (DiffSection::Unstaged, DiffCellKind::Ctx) => {
                        ask(BlameRev::Worktree, path, line.new_no, fingerprint)
                    }
                    // The old side of UNSTAGED is the index; with nothing
                    // staged that is HEAD, which needs no `--contents -`.
                    (DiffSection::Unstaged, DiffCellKind::Del) => {
                        let rev = if content.staged.is_empty() {
                            head()
                        } else {
                            BlameRev::Index
                        };
                        ask(rev, path, line.old_no, fingerprint)
                    }
                    (DiffSection::Staged, _) => ask(head(), path, line.old_no, fingerprint),
                    _ => None,
                }
            }
            Some(CommitFileSource::Commit { commit_id }) => {
                self.commit_side_blame(&line, path, commit_id, &format!("{commit_id}^"), 0, ask)
            }
            Some(CommitFileSource::Stash { stash_ref }) => self.commit_side_blame(
                &line,
                path,
                stash_ref,
                &format!("{stash_ref}^1"),
                fingerprint,
                ask,
            ),
            Some(CommitFileSource::Compare {
                merge_base, head, ..
            }) => self.commit_side_blame(&line, path, head, merge_base, 0, ask),
            // The new side is the file on disk now; the old side is the
            // base commit under the file's path there.
            Some(CommitFileSource::CommitVsWorktree { base, .. }) => {
                if line.kind == DiffCellKind::Del {
                    let old_path = self.listed_old_path(path).unwrap_or(path);
                    ask(BlameRev::Commit(base.clone()), old_path, line.old_no, 0)
                } else {
                    ask(BlameRev::Worktree, path, line.new_no, fingerprint)
                }
            }
        }?;
        Some((blame, side))
    }

    /// Commit-range diff: a new-side line blames `new_rev` at its new line
    /// number; a deleted line blames `old_rev` at its old line number under
    /// the file's old path.
    fn commit_side_blame(
        &self,
        line: &RowLineRef,
        path: &str,
        new_rev: &str,
        old_rev: &str,
        epoch: u64,
        ask: impl Fn(BlameRev, &str, Option<u32>, u64) -> Option<FocusedBlame>,
    ) -> Option<FocusedBlame> {
        if line.kind == DiffCellKind::Del {
            let old_path = self.listed_old_path(path).unwrap_or(path);
            ask(
                BlameRev::Commit(old_rev.into()),
                old_path,
                line.old_no,
                epoch,
            )
        } else {
            ask(BlameRev::Commit(new_rev.into()), path, line.new_no, epoch)
        }
    }

    /// Old path of `path`'s row in the shown commit-file list, when the
    /// row is a rename.
    fn listed_old_path(&self, path: &str) -> Option<&str> {
        self.commit_drill_files()?
            .iter()
            .find(|file| file.path == path)?
            .old_path
            .as_deref()
    }

    /// [`row_line_ref`] of the focused diff row, memoised by content
    /// fingerprint, layout, and cursor: [`Self::line_blame_want`] runs
    /// after every schedule and apply and must not rebuild a large diff's
    /// rows each time.
    fn focused_row_line(&self, fingerprint: u64) -> Option<RowLineRef> {
        let key = (fingerprint, self.diff_layout(), self.diff_cursor);
        if let Some((hit_key, hit)) = self.line_blame_row_memo.borrow().as_ref() {
            if *hit_key == key {
                return *hit;
            }
        }
        let hit = row_line_ref(self.current_diff_content(), key.1, key.2);
        *self.line_blame_row_memo.borrow_mut() = Some((key, hit));
        hit
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use workspace_status_graph::{Commit, GraphModel};

    use super::super::super::action::Action;
    use super::super::super::command_palette::PALETTE_COMMANDS;
    use super::super::super::diff::DiffContent;
    use super::super::super::drill::CommitFile;
    use super::super::super::keys::InputMode;
    use super::super::super::selection::TextSelection;
    use super::*;
    use crate::config::ViewDefaults;
    use crate::file_index::FileRead;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };

    const SHA: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    /// Inline rows: 0 label, 1 hunk, 2 ` a` (1,1), 3 `-b` (2,-), 4 `+c`
    /// (-,2), 5 ` d` (3,3).
    const BODY: &str = "@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n";

    fn repo(name: &str) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: "abc".into(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: false,
            changes: vec![FileChange {
                path: "README.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: Some("main".into()),
            local_branches: Vec::new(),
        }
    }

    fn state() -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app")], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.diff_mode = DiffMode::Inline;
        app
    }

    fn blame(summary: &str) -> LineBlame {
        LineBlame {
            sha: SHA.into(),
            author: "Ada".into(),
            author_time: 1_000,
            summary: summary.into(),
            orig_line: 1,
            filename: "README.md".into(),
            previous: None,
            boundary: false,
            uncommitted: false,
        }
    }

    /// Workspace file diff of README.md with focus on the diff pane.
    fn worktree_diff(staged: &str, unstaged: &str, is_new: bool) -> AppState {
        let mut app = state();
        let row = app
            .rows
            .iter()
            .position(|r| r.label.contains("README.md"))
            .expect("file row");
        app.cursor = row;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent {
                staged: staged.into(),
                unstaged: unstaged.into(),
                is_new,
                is_committed: false,
                vs_worktree: false,
                error: None,
            },
        );
        app.focus = FocusPane::Right;
        assert!(app.right_is_diff());
        app
    }

    fn at(app: &mut AppState, row: usize) -> Option<BlameKey> {
        app.diff_cursor = row;
        app.line_blame_want()
    }

    fn rev_line_path(key: Option<BlameKey>) -> Option<(BlameRev, u32, String)> {
        key.map(|k| (k.rev, k.line, k.path))
    }

    fn commit(rev: &str) -> BlameRev {
        BlameRev::Commit(rev.into())
    }

    #[test]
    fn unstaged_rows_blame_the_worktree_or_the_old_side() {
        let mut app = worktree_diff("", BODY, false);
        let fingerprint = app.current_diff_content().syntax_fingerprint();
        assert_eq!(at(&mut app, 0), None, "section label");
        assert_eq!(at(&mut app, 1), None, "hunk header");
        let ctx = at(&mut app, 2).expect("context line asks");
        assert_eq!(ctx.repo, "app");
        assert_eq!(ctx.epoch, fingerprint);
        assert_eq!(
            rev_line_path(Some(ctx)),
            Some((BlameRev::Worktree, 1, "README.md".into()))
        );
        assert_eq!(
            rev_line_path(at(&mut app, 3)),
            Some((commit("HEAD"), 2, "README.md".into())),
            "nothing staged: the old side is HEAD"
        );
        assert_eq!(at(&mut app, 4), None, "added line needs no git");
        assert_eq!(
            app.focused_line_annotation(0),
            Some((
                LineAnnotation::Text(UNCOMMITTED_TEXT.into()),
                BlameSide::New
            ))
        );

        let mut staged_too = worktree_diff("@@ -9 +9 @@\n-x\n+y\n", BODY, false);
        // STAGED rows 0..=3, then the UNSTAGED label (4) and hunk (5).
        assert_eq!(
            rev_line_path(at(&mut staged_too, 7)),
            Some((BlameRev::Index, 2, "README.md".into())),
            "something staged: the old side is the index"
        );
    }

    #[test]
    fn staged_rows_blame_head_and_added_lines_say_staged() {
        let mut app = worktree_diff(BODY, "", false);
        assert_eq!(
            rev_line_path(at(&mut app, 2)),
            Some((commit("HEAD"), 1, "README.md".into()))
        );
        assert_eq!(
            rev_line_path(at(&mut app, 3)),
            Some((commit("HEAD"), 2, "README.md".into()))
        );
        assert_eq!(at(&mut app, 4), None);
        assert_eq!(
            app.focused_line_annotation(0),
            Some((LineAnnotation::Text(STAGED_TEXT.into()), BlameSide::New))
        );
    }

    #[test]
    fn untracked_file_and_unfocused_panes_ask_nothing() {
        let mut app = worktree_diff("", "@@ -0,0 +1,2 @@\n+a\n+b\n", true);
        assert_eq!(at(&mut app, 2), None);
        assert_eq!(app.focused_line_annotation(0), None, "NEW paints nothing");

        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        assert!(app.line_blame_want().is_some());
        app.focus = FocusPane::Left;
        assert_eq!(app.line_blame_want(), None, "tree moves never blame");
        app.focus = FocusPane::Right;
        app.line_blame.set_enabled(false);
        assert_eq!(app.line_blame_want(), None, "off");
        assert_eq!(app.focused_line_annotation(0), None);
    }

    #[test]
    fn commit_drill_blames_the_commit_and_its_parent_under_the_old_path() {
        let mut app = state();
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: SHA.into(),
            },
            vec![CommitFile {
                status: "R".into(),
                path: "README.md".into(),
                old_path: Some("OLD.md".into()),
                stat: None,
            }],
            0,
            "README.md".into(),
            DiffContent::from_unified(BODY),
        );
        app.focus = FocusPane::Right;
        let add = at(&mut app, 4).expect("added line asks");
        assert_eq!(add.epoch, 0, "a commit never changes");
        assert_eq!(
            rev_line_path(Some(add)),
            Some((commit(SHA), 2, "README.md".into()))
        );
        assert_eq!(
            rev_line_path(at(&mut app, 3)),
            Some((commit(&format!("{SHA}^")), 2, "OLD.md".into()))
        );
        app.focus = FocusPane::Left;
        assert_eq!(app.line_blame_want(), None, "file list moves never blame");
    }

    #[test]
    fn stash_and_compare_diffs_blame_their_endpoints() {
        let mut app = state();
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Stash {
                stash_ref: "stash@{0}".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
            0,
            "README.md".into(),
            DiffContent::from_unified(BODY),
        );
        app.focus = FocusPane::Right;
        let ctx = at(&mut app, 2).expect("context line asks");
        assert_ne!(ctx.epoch, 0, "a stash ref can move");
        assert_eq!(
            rev_line_path(Some(ctx)),
            Some((commit("stash@{0}"), 1, "README.md".into()))
        );
        assert_eq!(
            rev_line_path(at(&mut app, 3)),
            Some((commit("stash@{0}^1"), 2, "README.md".into()))
        );

        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab = app.tabs.active_compare_mut().unwrap();
        tab.source = Some(CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: "bbb".into(),
            merge_base: "aaa".into(),
            head: "ccc".into(),
        });
        tab.files = vec![CommitFile {
            status: "M".into(),
            path: "README.md".into(),
            old_path: None,
            stat: None,
        }];
        tab.path = Some("README.md".into());
        tab.content = DiffContent::from_compare_lines(BODY.lines().map(String::from).collect());
        app.focus = FocusPane::Right;
        assert_eq!(
            rev_line_path(at(&mut app, 4)),
            Some((commit("ccc"), 2, "README.md".into()))
        );
        assert_eq!(
            rev_line_path(at(&mut app, 3)),
            Some((commit("aaa"), 2, "README.md".into()))
        );
    }

    #[test]
    fn file_tab_blames_the_cursor_line_of_the_working_tree() {
        let mut app = state();
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "src/lib.rs".into())
        else {
            panic!("expected a load");
        };
        assert_eq!(app.line_blame_want(), None, "still loading");
        let lines = ["one", "two", "three"];
        assert!(app.apply_file_tab(
            tab_id,
            gen,
            FileRead::Text {
                lines: lines.iter().map(|l| l.to_string()).collect(),
                max_cols: 5,
            }
        ));
        app.tabs.active_file_mut().unwrap().cursor = 2;
        let key = app.line_blame_want().expect("cursor line asks");
        assert_eq!(
            (key.repo.as_str(), &key.rev, key.path.as_str(), key.line),
            ("app", &BlameRev::Worktree, "src/lib.rs", 3)
        );
        let epoch = key.epoch;
        let reload = app.tabs.active_file_mut().unwrap().bump_generation();
        assert!(app.apply_file_tab(
            tab_id,
            reload,
            FileRead::Text {
                lines: lines.iter().map(|l| l.to_string()).collect(),
                max_cols: 5,
            }
        ));
        app.tabs.active_file_mut().unwrap().cursor = 2;
        assert_ne!(
            app.line_blame_want().unwrap().epoch,
            epoch,
            "a reload asks again"
        );
    }

    #[test]
    fn annotation_loads_then_shows_the_cached_answer() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        let key = app.line_blame_want().unwrap();
        assert!(app.line_blame_loading());
        assert_eq!(
            app.focused_line_annotation(1_000),
            Some((LineAnnotation::Loading, BlameSide::New))
        );
        assert_eq!(
            app.painted_line_annotation(),
            None,
            "loading paints nothing"
        );

        // A late answer for a line the cursor left fills the cache only.
        app.diff_cursor = 5;
        assert!(!app.apply_line_blame(key.clone(), Ok(Some(blame("first")))));
        app.diff_cursor = 2;
        assert!(!app.line_blame_loading(), "cached: no second git call");
        assert_eq!(
            app.focused_line_annotation(1_000 + 3 * 86_400),
            Some((
                LineAnnotation::Text("Ada, 3d ago · aaa1111 · first".into()),
                BlameSide::New
            ))
        );

        app.text_selection = Some(TextSelection {
            pane: ratatui::layout::Rect::default(),
            anchor: (0, 0),
            head: (0, 0),
        });
        assert_eq!(app.focused_line_annotation(0), None, "drag select");
        app.text_selection = None;

        app.diff_cursor = 3;
        let del = app.line_blame_want().unwrap();
        assert!(app.apply_line_blame(del, Err("fatal: no such path".into())));
        assert_eq!(app.focused_line_annotation(0), None, "no blame");
        assert!(!app.line_blame_loading());
    }

    #[test]
    fn b_toggles_line_blame_and_off_drops_the_cache() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        let key = app.line_blame_want().unwrap();
        assert!(app.apply_line_blame(key.clone(), Ok(Some(blame("x")))));
        assert_eq!(app.dispatch(Action::ToggleLineBlame), Effect::None);
        assert_eq!(app.status, "line blame off");
        assert_eq!(app.line_blame_want(), None);
        assert!(
            !app.apply_line_blame(key.clone(), Ok(None)),
            "late answer while off"
        );
        app.dispatch(Action::ToggleLineBlame);
        assert_eq!(app.status, "line blame on");
        assert_eq!(app.line_blame.cached(&key), None, "off dropped the cache");

        // The toggle runs on a file tab too.
        let _ = app.open_file_tab("app".into(), "src/lib.rs".into());
        assert!(app.is_file_tab());
        app.dispatch(Action::ToggleLineBlame);
        assert_eq!(app.status, "line blame off");
        assert!(!app.line_blame.enabled);
    }

    #[test]
    fn view_defaults_line_blame_sets_the_launch_value() {
        let mut app = state();
        assert!(app.line_blame.enabled, "on by default");
        app.apply_view_defaults(&ViewDefaults::default());
        assert!(app.line_blame.enabled, "omitted key keeps the default");
        app.apply_view_defaults(&ViewDefaults {
            line_blame: Some(false),
            ..ViewDefaults::default()
        });
        assert!(!app.line_blame.enabled);
        assert_eq!(app.status, "", "launch defaults post no status");
    }

    #[test]
    fn want_reuses_the_row_mapping_for_the_same_content_and_cursor() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        let first = app.line_blame_want();
        let memo = *app.line_blame_row_memo.borrow();
        assert_eq!(
            memo.map(|(key, _)| key.2),
            Some(2),
            "memo holds the cursor row"
        );
        assert_eq!(app.line_blame_want(), first);
        app.diff_cursor = 3;
        assert_ne!(app.line_blame_want(), first, "cursor move maps again");
    }

    const OTHER: &str = "ddd4444eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const BLAME_ACTIONS: [(&str, Action); 4] = [
        ("Blame: open commit changes", Action::BlameCommitVsParent),
        (
            "Blame: open previous line change",
            Action::BlamePreviousChange,
        ),
        (
            "Blame: diff commit to working tree",
            Action::BlameCommitVsWorktree,
        ),
        ("Blame: show commit in graph", Action::BlameRevealGraph),
    ];

    fn palette_reason(app: &AppState, title: &str) -> Option<String> {
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == title)
            .unwrap_or_else(|| panic!("no palette row {title}"));
        app.palette_disabled_reason(command)
    }

    /// Every blame action gives `reason` on its palette row and as the
    /// status of a dispatch, which changes nothing else.
    fn assert_blame_refused(app: &mut AppState, reason: &str) {
        for (title, action) in BLAME_ACTIONS {
            assert_eq!(
                palette_reason(app, title).as_deref(),
                Some(reason),
                "{title}"
            );
            let tabs = app.tabs.len();
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(app.status, reason, "{action:?}");
            assert_eq!(app.tabs.len(), tabs, "{action:?} opens no tab");
        }
    }

    fn blamed(sha: &str) -> LineBlame {
        LineBlame {
            sha: sha.into(),
            ..blame("subject")
        }
    }

    /// Answer the focused line's blame question with `answer`.
    fn answer(app: &mut AppState, answer: Option<LineBlame>) {
        let key = app.line_blame_want().expect("focused line asks");
        assert!(app.apply_line_blame(key, Ok(answer)));
    }

    #[test]
    fn blame_actions_refuse_with_the_same_copy_on_key_and_palette() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        assert_blame_refused(&mut app, BLAME_STILL_LOADING);
        answer(&mut app, None);
        assert_blame_refused(&mut app, NO_BLAME_FOR_LINE);
        app.diff_cursor = 1;
        assert_blame_refused(&mut app, NO_BLAME_FOR_LINE);
        app.diff_cursor = 4;
        assert_blame_refused(&mut app, LINE_NOT_COMMITTED);
        app.diff_cursor = 5;
        answer(
            &mut app,
            Some(LineBlame {
                uncommitted: true,
                ..blamed(&"0".repeat(40))
            }),
        );
        assert_blame_refused(&mut app, LINE_NOT_COMMITTED);
        app.focus = FocusPane::Left;
        assert_blame_refused(&mut app, FOCUS_A_DIFF_OR_FILE_LINE);
        app.focus = FocusPane::Right;
        app.line_blame.set_enabled(false);
        assert_blame_refused(&mut app, BLAME_IS_OFF);

        let mut untracked = worktree_diff("", "@@ -0,0 +1,2 @@\n+a\n+b\n", true);
        untracked.diff_cursor = 2;
        assert_blame_refused(&mut untracked, NO_BLAME_FOR_LINE);
    }

    #[test]
    fn blame_vs_parent_refuses_a_root_commit_but_reveal_runs() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        answer(
            &mut app,
            Some(LineBlame {
                boundary: true,
                ..blamed(SHA)
            }),
        );
        assert_eq!(
            palette_reason(&app, "Blame: open commit changes").as_deref(),
            Some(ROOT_COMMIT_HAS_NO_PARENT)
        );
        app.dispatch(Action::BlameCommitVsParent);
        assert_eq!(app.status, ROOT_COMMIT_HAS_NO_PARENT);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(
            palette_reason(&app, "Blame: open previous line change"),
            None
        );
        assert_eq!(palette_reason(&app, "Blame: show commit in graph"), None);
    }

    fn expect_parent_tab(effect: Effect, sha: &str) -> u64 {
        match effect {
            Effect::LoadCompareRange {
                tab_id,
                repo,
                base_ref,
                head_ref,
                force,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(base_ref, format!("{sha}^"));
                assert_eq!(head_ref, sha);
                assert!(force);
                tab_id
            }
            other => panic!("expected a compare load, got {other:?}"),
        }
    }

    #[test]
    fn blame_vs_parent_opens_a_pinned_tab_from_drill_compare_and_file_tab() {
        // Commit drill: the added line blames the drilled commit.
        let mut app = state();
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: SHA.into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
            0,
            "README.md".into(),
            DiffContent::from_unified(BODY),
        );
        app.focus = FocusPane::Right;
        app.diff_cursor = 4;
        answer(&mut app, Some(blamed(SHA)));
        assert_eq!(palette_reason(&app, "Blame: open commit changes"), None);
        let tab_id = expect_parent_tab(app.dispatch(Action::BlameCommitVsParent), SHA);
        let tab = app.tabs.active_compare().expect("compare tab");
        assert_eq!(tab.id, tab_id);
        assert!(tab.is_pinned());
        assert_eq!(tab.label(), "app ↔ aaa1111^");

        // That compare tab: a line blamed to another commit opens its own tab.
        let tab = app.tabs.active_compare_mut().unwrap();
        tab.source = Some(CommitFileSource::Compare {
            base_ref: format!("{SHA}^"),
            head_ref: SHA.into(),
            base_tip: "bbb".into(),
            merge_base: "bbb".into(),
            head: SHA.into(),
        });
        tab.files = vec![CommitFile {
            status: "M".into(),
            path: "README.md".into(),
            old_path: None,
            stat: None,
        }];
        tab.path = Some("README.md".into());
        tab.content = DiffContent::from_compare_lines(BODY.lines().map(String::from).collect());
        app.focus = FocusPane::Right;
        app.diff_cursor = 2;
        answer(&mut app, Some(blamed(OTHER)));
        expect_parent_tab(app.dispatch(Action::BlameCommitVsParent), OTHER);
        assert_eq!(app.tabs.compare_count(), 2);
        assert_eq!(app.tabs.active_compare().unwrap().label(), "app ↔ ddd4444^");

        // A file tab: the cursor line's blame, same checkout.
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "README.md".into())
        else {
            panic!("expected a load");
        };
        assert_blame_refused(&mut app, BLAME_STILL_LOADING);
        assert!(app.apply_file_tab(
            tab_id,
            gen,
            FileRead::Text {
                lines: vec!["a".into(), "c".into()],
                max_cols: 1,
            }
        ));
        answer(&mut app, Some(blamed(SHA)));
        assert_eq!(app.dispatch(Action::BlameCommitVsParent), Effect::None);
        assert_eq!(app.tabs.compare_count(), 2, "focuses the drill's tab");
        assert_eq!(app.tabs.active_compare().unwrap().label(), "app ↔ aaa1111^");
    }

    #[test]
    fn blame_previous_change_lands_only_the_latest_request() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        answer(&mut app, Some(blamed(SHA)));
        let Effect::LoadBlamePrevious { gen, repo, blame } =
            app.dispatch(Action::BlamePreviousChange)
        else {
            panic!("expected a previous-change load");
        };
        assert_eq!((repo.as_str(), blame.sha.as_str()), ("app", SHA));
        assert_eq!(app.status, "finding previous change…");
        let Effect::LoadBlamePrevious { gen: latest, .. } =
            app.dispatch(Action::BlamePreviousChange)
        else {
            panic!("expected a previous-change load");
        };
        assert_eq!(
            app.apply_blame_previous(
                gen,
                "app".into(),
                SHA,
                Ok(PreviousLineChange::Found(blamed(OTHER)))
            ),
            None,
            "stale request dropped"
        );
        assert_eq!(app.tabs.len(), 1);

        assert_eq!(
            app.apply_blame_previous(latest, "app".into(), SHA, Ok(PreviousLineChange::AddedIn)),
            None
        );
        assert_eq!(app.status, "line was added in aaa1111");
        assert_eq!(
            app.status.kind(),
            super::super::super::status::StatusKind::Warn
        );

        let root = LineBlame {
            boundary: true,
            ..blamed(OTHER)
        };
        assert_eq!(
            app.apply_blame_previous(
                latest,
                "app".into(),
                SHA,
                Ok(PreviousLineChange::Found(root))
            ),
            None
        );
        assert_eq!(app.status, "earlier change ddd4444 is the root commit");
        assert_eq!(app.tabs.len(), 1);

        let follow = app.apply_blame_previous(
            latest,
            "app".into(),
            SHA,
            Ok(PreviousLineChange::Found(blamed(OTHER))),
        );
        expect_parent_tab(follow.expect("opens a tab"), OTHER);
        assert_eq!(app.tabs.active_compare().unwrap().label(), "app ↔ ddd4444^");
        assert_eq!(app.status, "");
    }

    fn graph_with(ids: &[&str], has_more: bool) -> GraphModel {
        GraphModel {
            commits: ids
                .iter()
                .map(|id| Commit {
                    id: (*id).into(),
                    subject: format!("s-{id}"),
                    parents: vec!["parent".into()],
                    ..Commit::default()
                })
                .collect(),
            has_more,
            ..GraphModel::default()
        }
    }

    fn focused_commit(app: &AppState) -> Option<String> {
        match app.focused_graph_row() {
            Some(GraphRow::Commit { commit, .. }) => Some(commit.id),
            _ => None,
        }
    }

    /// The graph selection footer (the details view) is on commit `sha`:
    /// its row identity and its subject line.
    fn assert_footer_on(app: &AppState, sha: &str) {
        assert_eq!(
            app.graph_selected_row_identity(),
            Some(format!("app#commit:{sha}"))
        );
        let model = app.graph.as_ref().expect("graph");
        let rows = model.visible_rows();
        let [subject, _] = workspace_status_graph::selection_detail_lines(
            model,
            workspace_status_graph::GraphFooterSelection::from(rows.get(app.graph_cursor)),
            &workspace_status_graph::UNICODE,
            200,
            0,
        );
        assert!(subject.contains(&format!("s-{sha}")), "{subject}");
    }

    /// Workspace with a change under `src/`, the tree cursor on the
    /// `dir:app:src` row, and the app graph loaded from `ids`.
    fn dir_row_state(ids: &[&str], has_more: bool) -> AppState {
        let mut app_repo = repo("app");
        app_repo.changes[0].path = "src/lib.rs".into();
        let snapshot = build_workspace_snapshot(&[app_repo], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == "dir:app:src")
            .expect("dir row");
        app.set_graph(graph_with(ids, has_more), "app".into(), ids[0].into());
        app
    }

    /// Open a file tab of `src/lib.rs` whose cursor line blames `sha`.
    fn file_tab_blamed(app: &mut AppState, sha: &str) {
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "src/lib.rs".into())
        else {
            panic!("expected a load");
        };
        assert!(app.apply_file_tab(
            tab_id,
            gen,
            FileRead::Text {
                lines: vec!["a".into()],
                max_cols: 1,
            }
        ));
        answer(app, Some(blamed(sha)));
    }

    #[test]
    fn blame_reveal_from_a_dir_row_selects_a_loaded_commit_at_once() {
        for has_more in [false, true] {
            let mut app = dir_row_state(&[OTHER, SHA], has_more);
            file_tab_blamed(&mut app, SHA);
            assert_eq!(
                app.dispatch(Action::BlameRevealGraph),
                Effect::LoadRightPane
            );
            assert_eq!(
                app.focused_row().map(|row| row.id.as_str()),
                Some("repo:app")
            );
            assert_eq!(app.graph_reveal, None, "selected from the graph on screen");
            assert_eq!(focused_commit(&app).as_deref(), Some(SHA));
            assert_footer_on(&app, SHA);
            assert!(!app.graph_reveal_wants_older(), "no older page");
            assert!(
                !app.status.contains("not in the loaded graph"),
                "{}",
                app.status
            );
        }
    }

    #[test]
    fn blame_reveal_after_a_cursor_move_waits_for_the_graph_load() {
        let mut app = dir_row_state(&[OTHER], false);
        file_tab_blamed(&mut app, SHA);
        assert_eq!(
            app.dispatch(Action::BlameRevealGraph),
            Effect::LoadRightPane
        );
        assert!(
            !app.graph_reveal_wants_older(),
            "the old graph may be stale"
        );
        assert!(app.graph_reveal.is_some(), "still waiting for the load");
        assert!(!app.status.contains("not in the loaded graph"));

        // The pane load lands: now the reveal may give up.
        app.set_graph(graph_with(&[OTHER], false), "app".into(), OTHER.into());
        assert!(!app.graph_reveal_wants_older());
        assert_eq!(app.status, "aaa1111 is not in the loaded graph");
        assert_eq!(app.graph_reveal, None);
    }

    #[test]
    fn blame_reveal_from_a_worktree_diff_loads_the_graph_then_selects() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        answer(&mut app, Some(blamed(SHA)));
        assert_eq!(
            app.dispatch(Action::BlameRevealGraph),
            Effect::LoadRightPane
        );
        assert_eq!(
            app.focused_row().map(|row| row.id.as_str()),
            Some("repo:app")
        );
        assert!(app.graph_pane_focused(), "graph pane has focus");
        assert!(app.graph_reveal.is_some());

        app.set_graph(graph_with(&[OTHER, SHA], false), "app".into(), OTHER.into());
        assert_eq!(focused_commit(&app).as_deref(), Some(SHA));
        assert_footer_on(&app, SHA);
        assert_eq!(app.graph_reveal, None, "done");
    }

    #[test]
    fn blame_reveal_from_a_commit_drill_selects_the_loaded_row() {
        let mut app = state();
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == "repo:app")
            .unwrap();
        app.set_graph(graph_with(&[OTHER, SHA], false), "app".into(), OTHER.into());
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: OTHER.into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
            0,
            "README.md".into(),
            DiffContent::from_unified(BODY),
        );
        app.focus = FocusPane::Right;
        app.diff_cursor = 2;
        answer(&mut app, Some(blamed(SHA)));
        assert_eq!(
            app.dispatch(Action::BlameRevealGraph),
            Effect::DropCommitDiff
        );
        assert!(app.drill.is_graph());
        assert!(app.graph_pane_focused());
        assert_eq!(focused_commit(&app).as_deref(), Some(SHA));
        assert_footer_on(&app, SHA);
        assert_eq!(app.graph_reveal, None);
    }

    #[test]
    fn blame_reveal_from_a_file_tab_goes_to_workspace_and_widens_to_the_cap() {
        let mut app = state();
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == "repo:app")
            .unwrap();
        app.set_graph(graph_with(&[OTHER], true), "app".into(), OTHER.into());
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "README.md".into())
        else {
            panic!("expected a load");
        };
        assert!(app.apply_file_tab(
            tab_id,
            gen,
            FileRead::Text {
                lines: vec!["a".into()],
                max_cols: 1,
            }
        ));
        answer(&mut app, Some(blamed(SHA)));
        app.graph_branch_focus = Some(("app".into(), vec!["main".into()]));
        assert_eq!(
            app.dispatch(Action::BlameRevealGraph),
            Effect::LoadRightPane,
            "the parked graph may be stale: reload it"
        );
        assert!(app.tabs.is_workspace());
        assert!(app.graph_pane_focused());
        assert_eq!(
            focused_commit(&app).as_deref(),
            Some(OTHER),
            "not loaded yet"
        );
        assert!(
            !app.graph_reveal_wants_older(),
            "no widening from the parked graph"
        );
        assert!(app.graph_reveal.is_some());

        // The reload lands without the commit; now the reveal widens.
        app.set_graph(graph_with(&[OTHER], true), "app".into(), OTHER.into());
        app.graph_loading_older = true;
        assert!(
            !app.graph_reveal_wants_older(),
            "an older page is on its way"
        );
        app.graph_loading_older = false;
        for page in 1..=GRAPH_REVEAL_MAX_PAGES {
            assert!(app.graph_reveal_wants_older(), "page {page}");
            assert_eq!(app.graph_reveal.as_ref().map(|r| r.pages), Some(page));
        }
        assert!(!app.graph_reveal_wants_older(), "cap reached");
        assert_eq!(app.graph_reveal, None);
        assert_eq!(
            app.status,
            "aaa1111 is not in the loaded graph (graph focus on, O clears)"
        );
    }

    #[test]
    fn blame_reveal_from_a_file_tab_finds_a_commit_only_the_reload_has() {
        for has_more in [false, true] {
            let mut app = state();
            app.cursor = app
                .rows
                .iter()
                .position(|row| row.id == "repo:app")
                .unwrap();
            app.set_graph(graph_with(&[OTHER], has_more), "app".into(), OTHER.into());
            file_tab_on_readme_blamed(&mut app, SHA);
            assert_eq!(
                app.dispatch(Action::BlameRevealGraph),
                Effect::LoadRightPane
            );
            assert!(
                !app.graph_reveal_wants_older(),
                "no older page from the parked graph"
            );
            assert!(app.graph_reveal.is_some(), "waits for the reload");
            assert!(
                !app.status.contains("not in the loaded graph"),
                "{}",
                app.status
            );

            // A commit made while the file tab was active is in the reload.
            app.set_graph(
                graph_with(&[SHA, OTHER], has_more),
                "app".into(),
                SHA.into(),
            );
            assert_eq!(app.graph_reveal, None);
            assert_eq!(focused_commit(&app).as_deref(), Some(SHA));
            assert_footer_on(&app, SHA);
            assert!(!app.graph_reveal_wants_older());
            assert!(
                !app.status.contains("not in the loaded graph"),
                "{}",
                app.status
            );
        }
    }

    /// Open a file tab of `README.md` whose cursor line blames `sha`.
    fn file_tab_on_readme_blamed(app: &mut AppState, sha: &str) {
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "README.md".into())
        else {
            panic!("expected a load");
        };
        assert!(app.apply_file_tab(
            tab_id,
            gen,
            FileRead::Text {
                lines: vec!["a".into()],
                max_cols: 1,
            }
        ));
        answer(app, Some(blamed(sha)));
    }

    #[test]
    fn blame_reveal_drops_on_another_repo_graph_and_gives_up_at_the_end() {
        let mut app = state();
        app.graph_reveal = Some(GraphReveal {
            repo: "app".into(),
            sha: SHA.into(),
            pages: 0,
            seen_load: false,
        });
        app.set_graph(graph_with(&[OTHER], true), "lib".into(), OTHER.into());
        assert_eq!(app.graph_reveal, None, "another repo's graph drops it");

        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == "repo:app")
            .unwrap();
        app.focus = FocusPane::Right;
        app.graph_reveal = Some(GraphReveal {
            repo: "app".into(),
            sha: SHA.into(),
            pages: 0,
            seen_load: false,
        });
        app.drop_graph_reveal_off_graph();
        assert!(app.graph_reveal.is_some(), "graph pane still focused");
        app.focus = FocusPane::Left;
        app.drop_graph_reveal_off_graph();
        assert_eq!(app.graph_reveal, None, "leaving the graph drops it");

        app.graph_reveal = Some(GraphReveal {
            repo: "app".into(),
            sha: SHA.into(),
            pages: 0,
            seen_load: false,
        });
        app.set_graph(graph_with(&[OTHER], false), "app".into(), OTHER.into());
        assert!(app.graph_reveal.is_some(), "not found yet");
        assert!(!app.graph_reveal_wants_older(), "history ends");
        assert_eq!(app.status, "aaa1111 is not in the loaded graph");
        assert_eq!(app.graph_reveal, None);
    }

    fn readme_row(status: &str, old_path: Option<&str>) -> CommitFile {
        CommitFile {
            status: status.into(),
            path: "README.md".into(),
            old_path: old_path.map(String::from),
            stat: None,
        }
    }

    /// Expect the load of a new commit-vs-working-tree tab of `sha`.
    fn expect_worktree_tab(app: &AppState, effect: Effect, sha: &str, file: WorktreeFile) -> u64 {
        let Effect::LoadCompareRange {
            tab_id,
            repo,
            base_ref,
            head_ref,
            force,
        } = effect
        else {
            panic!("expected a compare load, got {effect:?}");
        };
        assert_eq!(
            (repo.as_str(), base_ref.as_str(), head_ref.as_str(), force),
            ("app", sha, "HEAD", true)
        );
        let tab = app.tabs.active_compare().expect("compare tab");
        assert_eq!(tab.id, tab_id);
        assert_eq!(tab.worktree_file.as_ref(), Some(&file));
        assert!(!tab.is_pinned());
        let leaf = file.path.rsplit('/').next().unwrap();
        assert_eq!(tab.label(), format!("{leaf} ↔ {}", &sha[..7]));
        assert_eq!(tab.range_header(), format!("{} ↔ working tree", &sha[..7]));
        tab_id
    }

    fn disk_file(path: &str, old_path: Option<&str>) -> WorktreeFile {
        WorktreeFile {
            path: path.into(),
            old_path: old_path.map(String::from),
        }
    }

    /// Make the active compare tab a loaded commit-vs-working-tree diff of
    /// README.md (at `OLD.md` in the base) with [`BODY`] open.
    fn load_worktree_tab(app: &mut AppState) {
        let tab = app.tabs.active_compare_mut().unwrap();
        tab.loading = false;
        tab.source = Some(CommitFileSource::CommitVsWorktree {
            base: SHA.into(),
            path: "README.md".into(),
            old_path: Some("OLD.md".into()),
        });
        tab.files = vec![readme_row("R", Some("OLD.md"))];
        tab.path = Some("README.md".into());
        tab.content =
            DiffContent::from_worktree_compare_lines(BODY.lines().map(String::from).collect());
        tab.content_for = Some((tab.source.clone().unwrap(), "README.md".into()));
        app.focus = FocusPane::Right;
    }

    #[test]
    fn blame_vs_worktree_opens_one_tab_per_commit_and_file() {
        // Workspace diff: the disk path is the diff's path.
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        answer(&mut app, Some(blamed(SHA)));
        assert_eq!(
            palette_reason(&app, "Blame: diff commit to working tree"),
            None
        );
        let effect = app.dispatch(Action::BlameCommitVsWorktree);
        let tab_id = expect_worktree_tab(&app, effect, SHA, disk_file("README.md", None));

        // `<sha>...HEAD` (Diff vs commit) is another identity.
        assert!(matches!(
            app.open_compare_tab("app".into(), SHA.into(), "HEAD".into()),
            Effect::LoadCompareRange { .. }
        ));
        assert_eq!(app.tabs.compare_count(), 2);
        assert_eq!(app.tabs.active_compare().unwrap().worktree_file, None);

        // The same line again focuses the first tab.
        assert_eq!(app.activate_tab(0), Effect::None);
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::BlameCommitVsWorktree), Effect::None);
        assert_eq!(app.tabs.compare_count(), 2);
        assert_eq!(app.tabs.active_compare().unwrap().id, tab_id);

        // A root commit still has a file to diff.
        let mut root = worktree_diff("", BODY, false);
        root.diff_cursor = 2;
        answer(
            &mut root,
            Some(LineBlame {
                boundary: true,
                ..blamed(SHA)
            }),
        );
        assert_eq!(
            palette_reason(&root, "Blame: diff commit to working tree"),
            None
        );
    }

    #[test]
    fn blame_vs_worktree_from_drill_compare_and_file_tab_keeps_the_commit_path() {
        // Commit drill of a rename: the line blames OLD.md at the commit.
        let mut app = state();
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: OTHER.into(),
            },
            vec![readme_row("R", Some("OLD.md"))],
            0,
            "README.md".into(),
            DiffContent::from_unified(BODY),
        );
        app.focus = FocusPane::Right;
        app.diff_cursor = 4;
        answer(
            &mut app,
            Some(LineBlame {
                filename: "OLD.md".into(),
                ..blamed(SHA)
            }),
        );
        let effect = app.dispatch(Action::BlameCommitVsWorktree);
        expect_worktree_tab(&app, effect, SHA, disk_file("README.md", Some("OLD.md")));

        // A commit-range compare tab: the open path is the disk path.
        let mut app = state();
        app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
        let tab = app.tabs.active_compare_mut().unwrap();
        tab.source = Some(CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: "bbb".into(),
            merge_base: "bbb".into(),
            head: "ccc".into(),
        });
        tab.files = vec![readme_row("M", None)];
        tab.path = Some("README.md".into());
        tab.content = DiffContent::from_compare_lines(BODY.lines().map(String::from).collect());
        app.focus = FocusPane::Right;
        app.diff_cursor = 2;
        answer(&mut app, Some(blamed(OTHER)));
        let effect = app.dispatch(Action::BlameCommitVsWorktree);
        expect_worktree_tab(&app, effect, OTHER, disk_file("README.md", None));
        assert_eq!(app.tabs.compare_count(), 2);

        // A file tab: its own file.
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "src/lib.rs".into())
        else {
            panic!("expected a load");
        };
        assert!(app.apply_file_tab(
            tab_id,
            gen,
            FileRead::Text {
                lines: vec!["a".into()],
                max_cols: 1,
            }
        ));
        answer(
            &mut app,
            Some(LineBlame {
                filename: "src/lib.rs".into(),
                ..blamed(SHA)
            }),
        );
        let effect = app.dispatch(Action::BlameCommitVsWorktree);
        expect_worktree_tab(&app, effect, SHA, disk_file("src/lib.rs", None));
    }

    #[test]
    fn worktree_compare_tab_blames_disk_lines_and_the_base_commit() {
        let mut app = state();
        app.open_worktree_compare_tab("app".into(), SHA.into(), disk_file("README.md", None));
        load_worktree_tab(&mut app);
        let fingerprint = app.current_diff_content().syntax_fingerprint();
        let add = at(&mut app, 4).expect("added line asks");
        assert_eq!(add.epoch, fingerprint, "the working tree can change");
        assert_eq!(
            rev_line_path(Some(add)),
            Some((BlameRev::Worktree, 2, "README.md".into()))
        );
        assert_eq!(
            rev_line_path(at(&mut app, 2)),
            Some((BlameRev::Worktree, 1, "README.md".into()))
        );
        let del = at(&mut app, 3).expect("deleted line asks");
        assert_eq!(del.epoch, 0);
        assert_eq!(
            rev_line_path(Some(del)),
            Some((commit(SHA), 2, "OLD.md".into())),
            "the base side keeps the file's path at the commit"
        );
        app.focus = FocusPane::Left;
        assert_eq!(app.line_blame_want(), None, "file list moves never blame");
    }

    #[test]
    fn worktree_compare_tab_refuses_revert_marks_comments_and_e() {
        use super::super::super::tabs::{
            CANNOT_REVERT_WORKTREE_COMPARE, NOT_ON_WORKTREE_COMPARE,
            REVIEWED_MARKS_NEED_A_COMMIT_RANGE,
        };
        let mut app = state();
        app.open_worktree_compare_tab("app".into(), SHA.into(), disk_file("README.md", None));
        load_worktree_tab(&mut app);
        for (title, action, reason) in [
            ("Revert", Action::Revert, CANNOT_REVERT_WORKTREE_COMPARE),
            (
                "Mark reviewed",
                Action::ToggleReviewed,
                REVIEWED_MARKS_NEED_A_COMMIT_RANGE,
            ),
            ("Comment", Action::CommentStart, NOT_ON_WORKTREE_COMPARE),
            (
                "Copy entity reference",
                Action::CopyEntityReference,
                NOT_ON_WORKTREE_COMPARE,
            ),
            (
                "Copy comments",
                Action::ExportComments,
                NOT_ON_WORKTREE_COMPARE,
            ),
            (
                "Open in diff tool",
                Action::ExternalDiff,
                NOT_ON_WORKTREE_COMPARE,
            ),
        ] {
            assert_eq!(
                palette_reason(&app, title).as_deref(),
                Some(reason),
                "{title}"
            );
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(app.status, reason, "{action:?}");
        }
        // The file list refuses the same way.
        app.focus = FocusPane::Left;
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert_eq!(app.status, CANNOT_REVERT_WORKTREE_COMPARE);
        // A `V` highlight on the WORKING TREE section never builds a patch.
        let content = app.current_diff_content().clone();
        assert!(super::super::super::diff::build_partial_patch(
            &content,
            DiffMode::Inline,
            3,
            4,
            super::super::super::diff::PartialPatchKind::RevertCommitted,
            "README.md",
        )
        .is_err());
    }

    fn expect_range_reload(effect: Option<Effect>, tab_id: u64) {
        match effect {
            Some(Effect::LoadCompareRange {
                tab_id: id,
                repo,
                base_ref,
                head_ref,
                force,
            }) => {
                assert_eq!(id, tab_id);
                assert_eq!(
                    (repo.as_str(), base_ref.as_str(), head_ref.as_str(), force),
                    ("app", SHA, "HEAD", true)
                );
            }
            other => panic!("expected a range reload, got {other:?}"),
        }
    }

    #[test]
    fn worktree_compare_tab_reloads_when_the_file_changes_on_watch() {
        let mut app = state();
        app.open_worktree_compare_tab("app".into(), SHA.into(), disk_file("README.md", None));
        load_worktree_tab(&mut app);
        let tab_id = app.tabs.active_compare().unwrap().id;
        assert!(app.compare_probe_effects().is_empty(), "never HEAD-probed");
        assert_eq!(app.worktree_compare_reload("app"), None, "nothing moved");

        // README.md leaves the dirty set: its entry changed. The list
        // reloads too, so the row's status follows the disk.
        let clean = RepoSnapshot {
            changes: Vec::new(),
            has_unstaged: false,
            ..repo("app")
        };
        app.apply_watch_snapshot(build_workspace_snapshot(&[clean], &[], false, &[]));
        assert_eq!(app.worktree_compare_reload("lib"), None, "another checkout");
        expect_range_reload(app.worktree_compare_reload("app"), tab_id);
        assert_eq!(app.worktree_compare_reload("app"), None, "reloaded once");

        // Inactive: the watch skips it; activating it again catches up.
        assert_eq!(app.activate_tab(0), Effect::None);
        let moved = RepoSnapshot {
            head: "def".into(),
            ..repo("app")
        };
        app.apply_watch_snapshot(build_workspace_snapshot(&[moved], &[], false, &[]));
        assert_eq!(app.worktree_compare_reload("app"), None, "not active");
        let back = app.activate_tab(1);
        expect_range_reload(Some(back), tab_id);
        app.tabs.active_compare_mut().unwrap().loading = false;
        assert_eq!(app.activate_tab(0), Effect::None);
        assert_eq!(app.activate_tab(1), Effect::None, "stamp unchanged");
    }

    #[test]
    fn worktree_compare_tab_has_no_external_diff_pair() {
        let mut app = state();
        app.open_worktree_compare_tab("app".into(), SHA.into(), disk_file("README.md", None));
        load_worktree_tab(&mut app);
        assert_eq!(app.external_diff_kind(), None);
    }

    /// `A` refuses with `reason`, the copy every blame palette row shows,
    /// and leaves the menu closed.
    fn assert_menu_refused(app: &mut AppState, reason: &str) {
        app.status.clear();
        assert_eq!(app.dispatch(Action::BlameMenu), Effect::None);
        assert_eq!(app.status, reason);
        assert!(!app.blame_menu);
        assert_eq!(
            palette_reason(app, "Blame: show commit in graph").as_deref(),
            Some(reason)
        );
    }

    #[test]
    fn blame_menu_opens_only_on_a_committed_line_with_the_actions_copy() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        assert_menu_refused(&mut app, BLAME_STILL_LOADING);
        answer(&mut app, None);
        assert_menu_refused(&mut app, NO_BLAME_FOR_LINE);
        app.diff_cursor = 4;
        assert_menu_refused(&mut app, LINE_NOT_COMMITTED);
        app.diff_cursor = 5;
        answer(
            &mut app,
            Some(LineBlame {
                uncommitted: true,
                ..blamed(&"0".repeat(40))
            }),
        );
        assert_menu_refused(&mut app, LINE_NOT_COMMITTED);
        app.diff_cursor = 3;
        answer(&mut app, Some(blamed(SHA)));
        app.focus = FocusPane::Left;
        assert_menu_refused(&mut app, FOCUS_A_DIFF_OR_FILE_LINE);
        app.focus = FocusPane::Right;
        app.line_blame.set_enabled(false);
        assert_menu_refused(&mut app, BLAME_IS_OFF);
        app.line_blame.set_enabled(true);
        answer(&mut app, Some(blamed(SHA)));

        app.status = "stale".into();
        assert_eq!(app.dispatch(Action::BlameMenu), Effect::None);
        assert!(app.blame_menu);
        assert_eq!(app.input_mode(), InputMode::BlameMenu);
        assert_eq!(app.status, "", "the box lists the actions");

        // A key the menu does not bind keeps it open; Esc closes it.
        assert_eq!(app.dispatch(Action::BlameMenuChar('x')), Effect::None);
        assert!(app.blame_menu);
        assert_eq!(app.dispatch(Action::BlameMenuCancel), Effect::None);
        assert!(!app.blame_menu);
        assert!(matches!(app.input_mode(), InputMode::Normal { .. }));

        // A drag selection hides the annotation; opening the menu ends it
        // so the header shows the blame.
        app.text_selection = Some(TextSelection {
            pane: Default::default(),
            anchor: (0, 0),
            head: (0, 0),
        });
        assert_eq!(app.painted_line_annotation(), None);
        app.dispatch(Action::BlameMenu);
        assert!(app.blame_menu);
        assert!(app.text_selection.is_none());
        assert!(app.painted_line_annotation().is_some());
    }

    #[test]
    fn blame_menu_opens_on_a_root_commit_and_c_refuses_like_the_palette() {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        answer(
            &mut app,
            Some(LineBlame {
                boundary: true,
                ..blamed(SHA)
            }),
        );
        app.dispatch(Action::BlameMenu);
        assert!(app.blame_menu);
        assert_eq!(app.dispatch(Action::BlameMenuChar('c')), Effect::None);
        assert!(!app.blame_menu, "a pick closes the menu");
        assert_eq!(app.status, ROOT_COMMIT_HAS_NO_PARENT);
        assert_eq!(app.tabs.len(), 1);
    }

    /// Worktree diff with a committed focused line and the menu open.
    fn menu_open() -> AppState {
        let mut app = worktree_diff("", BODY, false);
        app.diff_cursor = 2;
        answer(&mut app, Some(blamed(SHA)));
        app.dispatch(Action::BlameMenu);
        assert!(app.blame_menu);
        app
    }

    #[test]
    fn blame_menu_keys_dispatch_the_blame_actions() {
        let picks = BLAME_MENU_ROWS
            .iter()
            .map(|row| (Action::BlameMenuChar(row.key), row.action.clone()))
            .chain([(Action::BlameMenuEnter, Action::BlameCommitVsParent)]);
        for (key, action) in picks {
            let mut direct = menu_open();
            direct.dispatch(Action::BlameMenuCancel);
            let want = direct.dispatch(action.clone());
            assert_ne!(want, Effect::None, "{action:?} runs");

            let mut menu = menu_open();
            assert_eq!(menu.dispatch(key.clone()), want, "{key:?}");
            assert!(!menu.blame_menu, "{key:?} closes the menu");
            assert_eq!(menu.status, direct.status, "{key:?}");
            assert_eq!(menu.tabs.len(), direct.tabs.len(), "{key:?}");
            assert_eq!(menu.focus, direct.focus, "{key:?}");
        }
        let titles: Vec<String> = BLAME_MENU_ROWS
            .iter()
            .map(|row| format!("Blame: {}", row.label))
            .collect();
        let palette: Vec<String> = BLAME_ACTIONS
            .iter()
            .map(|(title, _)| (*title).to_string())
            .collect();
        assert_eq!(titles, palette, "rows read like the palette rows");
        for (row, (_, action)) in BLAME_MENU_ROWS.iter().zip(BLAME_ACTIONS) {
            assert_eq!(row.action, action);
        }
    }

    #[test]
    fn blame_menu_runs_on_a_file_tab() {
        let mut app = dir_row_state(&[OTHER, SHA], false);
        file_tab_blamed(&mut app, SHA);
        assert_eq!(app.dispatch(Action::BlameMenu), Effect::None);
        assert!(app.blame_menu);
        expect_parent_tab(app.dispatch(Action::BlameMenuChar('c')), SHA);
        assert!(!app.blame_menu);
        assert_eq!(app.tabs.active_compare().unwrap().label(), "app ↔ aaa1111^");
    }

    #[test]
    fn blame_menu_stays_open_and_on_its_line_when_an_answer_lands() {
        let mut app = menu_open();
        let focused = app.line_blame_want().expect("focused line");
        let header = app.painted_line_annotation();
        let other = BlameKey {
            line: 99,
            ..focused.clone()
        };
        assert!(!app.apply_line_blame(other, Ok(Some(blamed(OTHER)))));
        assert!(app.apply_line_blame(focused.clone(), Ok(Some(blamed(SHA)))));
        assert!(app.blame_menu);
        assert_eq!(app.input_mode(), InputMode::BlameMenu);
        assert_eq!(app.line_blame_want(), Some(focused));
        assert_eq!(app.painted_line_annotation(), header);
    }
}
