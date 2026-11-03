//! Current-line blame: which line has focus, what it asks git, and what
//! the pane paints at its end.
//!
//! Only a focused line asks: the Explorer file view's cursor line, or the
//! focused file-diff row, while the right pane has focus. Moving
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
use super::super::tabs::{WorktreeFile, ROOT_COMMIT_HAS_NO_PARENT, TAB_TO_THE_FILE_PANE};
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

    /// Checkout and committed blame behind the note
    /// [`Self::painted_line_annotation`] paints now: the commit the note's
    /// author, age, and short hash name, for their popovers.
    ///
    /// `None` when that note names no commit: loading, no blame, an
    /// uncommitted or staged line (`You · …`), line blame off, or a drag
    /// text selection active.
    pub(crate) fn painted_line_blame(&self) -> Option<(String, LineBlame)> {
        if self.text_selection.is_some() {
            return None;
        }
        let FocusedBlame::Ask(key) = self.focused_blame()?.0 else {
            return None;
        };
        match self.line_blame.cached(&key) {
            Some(Some(blame)) if !blame.uncommitted => Some((key.repo, blame.clone())),
            _ => None,
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
    /// Workspace, compare, and Explorer tabs; on an Explorer tab the file
    /// pane must have focus ([`TAB_TO_THE_FILE_PANE`]).
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
        if self.is_explorer_tab() && self.list_focus_target() != ListFocusTarget::None {
            return Err(TAB_TO_THE_FILE_PANE);
        }
        if let Some(tab) = self.tabs.active_file_view() {
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

    /// The focused line's file on disk: the file view's file, else the open
    /// diff's path.
    fn blamed_disk_path(&self) -> Option<String> {
        if let Some(tab) = self.tabs.active_file_view() {
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
    pub(super) fn focus_checkout_row(&mut self, repo: &str) -> Option<bool> {
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
        // The PRs tab shows no line: the parked Workspace diff never blames.
        if !self.line_blame.enabled || self.is_prs_tab() {
            return None;
        }
        // On an Explorer tab only the focused right pane blames: a tree
        // move never asks git.
        if self.is_explorer_tab() && self.list_focus_target() != ListFocusTarget::None {
            return None;
        }
        if let Some(tab) = self.tabs.active_file_view() {
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
mod tests;
