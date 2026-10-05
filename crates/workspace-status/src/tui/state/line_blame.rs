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

use super::super::action::Effect;
use super::super::diff::{row_line_ref, DiffCellKind, DiffSection, RowLineRef};
use super::super::drill::CommitFileSource;
use super::super::gates::ListFocusTarget;
use super::super::line_blame::{
    annotation_text, BlameKey, BlameSide, LineAnnotation, STAGED_TEXT, UNCOMMITTED_TEXT,
};
use super::super::split::DiffMode;
use super::AppState;
use crate::git::{BlameRev, LineBlame};

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
            (tab.id, tab.generation).hash(&mut hasher);
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
            let old_path = self
                .commit_drill_files()
                .and_then(|files| files.iter().find(|file| file.path == path))
                .and_then(|file| file.old_path.as_deref())
                .unwrap_or(path);
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

    use super::super::super::action::Action;
    use super::super::super::diff::DiffContent;
    use super::super::super::drill::CommitFile;
    use super::super::super::selection::TextSelection;
    use super::super::FocusPane;
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
}
