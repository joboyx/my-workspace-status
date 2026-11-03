//! Tab view parking: one shared form for the live view fields a tab owns.
//!
//! The live pane focus, `/` search, diff viewport, pans, and commit-file
//! folds on [`AppState`] belong to the active tab. A tab switch captures
//! them as one [`TabView`] ([`AppState::tab_view`]) and hands it to the tab
//! it leaves ([`SessionTab::park_view`], or the Workspace park). The tab it
//! enters returns the view to restore ([`SessionTab::parked_view`]) and
//! [`AppState::set_tab_view`] writes it back. A tab that parks every field
//! stores the [`TabView`] whole; a tab that owns fewer fields maps only
//! those and keeps the live value for the rest.

use std::collections::HashSet;

use super::super::search::SearchPane;
use super::super::tabs::{CompareTab, ExplorerTab, ParkedDiff, ParkedSearch, PrsTab, SessionTab};
use super::{AppState, FocusPane};

/// The live view fields of the active tab, parked while another tab is
/// active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabView {
    /// Pane with keyboard focus.
    pub(crate) focus: FocusPane,
    /// `/` prompt, armed query, bound pane, and current hit.
    pub(crate) search: ParkedSearch,
    /// File-diff cursor, scroll, and pan.
    pub(crate) diff: ParkedDiff,
    /// Columns the left pane is panned.
    pub(crate) left_col_offset: u16,
    /// Columns the right pane is panned.
    pub(crate) right_col_offset: u16,
    /// Folded commit-file directories.
    pub(crate) commit_file_folds: HashSet<String>,
    /// Commit-file directory tree vs flat paths.
    pub(crate) commit_tree_mode: bool,
}

impl AppState {
    /// The live file-diff viewport.
    pub(super) fn live_diff(&self) -> ParkedDiff {
        ParkedDiff {
            cursor: self.diff_cursor,
            scroll: self.diff_scroll,
            col_offset: self.diff_col_offset,
        }
    }

    /// Replace the live file-diff viewport with `diff`.
    pub(super) fn set_live_diff(&mut self, diff: ParkedDiff) {
        self.diff_cursor = diff.cursor;
        self.diff_scroll = diff.scroll;
        self.diff_col_offset = diff.col_offset;
    }

    /// Capture the live view fields of the active tab.
    pub(super) fn tab_view(&self) -> TabView {
        TabView {
            focus: self.focus,
            search: ParkedSearch {
                mode: self.search_mode,
                active: self.search_active,
                query: self.search_query.clone(),
                target: self.search_target,
                hit: self.search_hit,
            },
            diff: self.live_diff(),
            left_col_offset: self.left_col_offset,
            right_col_offset: self.right_col_offset,
            commit_file_folds: self.commit_file_folds.clone(),
            commit_tree_mode: self.commit_tree_mode,
        }
    }

    /// Write `view` into the live view fields.
    pub(super) fn set_tab_view(&mut self, view: TabView) {
        self.focus = view.focus;
        self.search_mode = view.search.mode;
        self.search_active = view.search.active;
        self.search_query = view.search.query;
        self.search_target = view.search.target;
        self.search_hit = view.search.hit;
        self.set_live_diff(view.diff);
        self.left_col_offset = view.left_col_offset;
        self.right_col_offset = view.right_col_offset;
        self.commit_file_folds = view.commit_file_folds;
        self.commit_tree_mode = view.commit_tree_mode;
    }

    /// Before the active tab changes: park the live view in the active tab
    /// (Workspace keeps it whole in its park).
    pub(super) fn park_active_session(&mut self) {
        let view = self.tab_view();
        match self.tabs.active_tab_mut() {
            Some(tab) => tab.park_view(&view),
            None => self.workspace_park = Some(view),
        }
    }

    /// After the active tab changed: restore the view it parked. Workspace
    /// takes its park; with none, the live view stays.
    pub(super) fn apply_active_session(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            if let Some(view) = self.workspace_park.take() {
                self.set_tab_view(view);
            }
            return;
        };
        let view = tab.parked_view(self.tab_view());
        // An Explorer or PRs tab restores a `/` search of its own panes
        // (or none), so the origin saved for another tab's panes must not
        // let Esc restore those. Workspace and compare tabs never dropped
        // it; that stays as it was.
        let drop_search_origin = matches!(tab, SessionTab::Explorer(_) | SessionTab::Prs(_));
        self.set_tab_view(view);
        if drop_search_origin {
            self.search_origin = None;
        }
    }
}

impl SessionTab {
    /// Park `view`, the live view this tab leaves, in the tab.
    pub(crate) fn park_view(&mut self, view: &TabView) {
        match self {
            Self::Compare(tab) => tab.park_view(view),
            Self::Explorer(tab) => tab.park_view(view),
            Self::Prs(tab) => tab.park_view(view),
        }
    }

    /// The view to restore when this tab becomes active, given the `live`
    /// one: fields the tab does not own keep the live value.
    pub(crate) fn parked_view(&self, live: TabView) -> TabView {
        match self {
            Self::Compare(tab) => tab.parked_view(live),
            Self::Explorer(tab) => tab.parked_view(live),
            Self::Prs(tab) => tab.parked_view(live),
        }
    }
}

impl CompareTab {
    /// Park `view` in the flat compare fields. The right pane pan and the
    /// search target are not parked.
    pub(crate) fn park_view(&mut self, view: &TabView) {
        self.focus_right = view.focus == FocusPane::Right;
        self.search_mode = view.search.mode;
        self.search_active = view.search.active;
        self.search_query = view.search.query.clone();
        self.search_hit = view.search.hit;
        self.left_col_offset = view.left_col_offset;
        self.diff_col_offset = view.diff.col_offset;
        self.diff_cursor = view.diff.cursor;
        self.diff_scroll = view.diff.scroll;
        self.folds = view.commit_file_folds.clone();
        self.tree_mode = view.commit_tree_mode;
    }

    /// The parked compare view. A running or typed search binds the diff
    /// when the diff had focus, else the commit-file list; the right pane
    /// pan keeps the `live` value.
    pub(crate) fn parked_view(&self, live: TabView) -> TabView {
        let searching = self.search_active || self.search_mode;
        TabView {
            focus: if self.focus_right {
                FocusPane::Right
            } else {
                FocusPane::Left
            },
            search: ParkedSearch {
                mode: self.search_mode,
                active: self.search_active,
                query: self.search_query.clone(),
                target: if searching && self.focus_right {
                    SearchPane::Diff
                } else {
                    SearchPane::CommitFiles
                },
                hit: self.search_hit,
            },
            diff: ParkedDiff {
                cursor: self.diff_cursor,
                scroll: self.diff_scroll,
                col_offset: self.diff_col_offset,
            },
            left_col_offset: self.left_col_offset,
            right_col_offset: live.right_col_offset,
            commit_file_folds: self.folds.clone(),
            commit_tree_mode: self.tree_mode,
        }
    }
}

impl ExplorerTab {
    /// Park the pane focus and `/` search of `view` in the tab, and its
    /// diff viewport in the active inner file tab.
    pub(crate) fn park_view(&mut self, view: &TabView) {
        if let Some(inner) = self.active_inner_mut() {
            inner.diff = view.diff;
        }
        self.focus_preview = view.focus == FocusPane::Right;
        self.search = view.search.clone();
    }

    /// The parked Explorer view: the file pane keeps focus only while an
    /// inner file tab is open, the diff viewport is the active inner tab's
    /// (the top with none), and the `/` search is the tab's. The other
    /// fields keep the `live` value.
    pub(crate) fn parked_view(&self, live: TabView) -> TabView {
        let inner = self.active_inner();
        TabView {
            focus: if self.focus_preview && inner.is_some() {
                FocusPane::Right
            } else {
                FocusPane::Left
            },
            search: self.search.clone(),
            diff: inner.map_or_else(ParkedDiff::default, |inner| inner.diff),
            ..live
        }
    }
}

impl PrsTab {
    /// Park nothing: the PRs tab keeps its view state in the tab itself,
    /// and the Workspace park stays as it was.
    pub(crate) fn park_view(&mut self, _view: &TabView) {}

    /// The PRs view: the list has the focus and no pane search runs (the
    /// Workspace search stays parked). The search target, the diff
    /// viewport, the pans, and the folds keep the `live` value.
    pub(crate) fn parked_view(&self, live: TabView) -> TabView {
        TabView {
            focus: FocusPane::Left,
            search: ParkedSearch {
                mode: false,
                active: false,
                query: String::new(),
                target: live.search.target,
                hit: None,
            },
            ..live
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::action::Action;
    use super::super::super::diff::DiffContent;
    use super::super::super::tabs::COMPARE_HEAD_REF;
    use super::super::explorer_tab::ExplorerTarget;
    use super::*;
    use crate::snapshot::RepoSnapshot;
    use crate::testutil::{app_state, repo_snapshot, unstaged_change};

    /// Opens a session tab from the Workspace.
    type Open = fn(&mut AppState);

    /// Whole-file diff of one changed line.
    const BODY: &str = "@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n";

    /// `app` on a born HEAD with a default tip (a compare tab opens), with
    /// unstaged edits of `README.md` and `src/lib.rs` (Explorer diffs).
    fn state() -> AppState {
        app_state(&[RepoSnapshot {
            head: "abc".into(),
            default_tip_ref: Some("main".into()),
            has_unstaged: true,
            changes: vec![unstaged_change("README.md"), unstaged_change("src/lib.rs")],
            ..repo_snapshot("app")
        }])
    }

    /// The view a tab is left in: every field off its default.
    fn parked() -> TabView {
        TabView {
            focus: FocusPane::Right,
            search: ParkedSearch {
                mode: false,
                active: true,
                query: "needle".into(),
                target: SearchPane::Diff,
                hit: Some(4),
            },
            diff: ParkedDiff {
                cursor: 7,
                scroll: 3,
                col_offset: 5,
            },
            left_col_offset: 2,
            right_col_offset: 6,
            commit_file_folds: HashSet::from(["src".to_string()]),
            commit_tree_mode: false,
        }
    }

    /// What another tab leaves in the live fields: every field differs
    /// from [`parked`].
    fn scrambled() -> TabView {
        TabView {
            focus: FocusPane::Left,
            search: ParkedSearch {
                mode: true,
                active: false,
                query: "other".into(),
                target: SearchPane::Tree,
                hit: Some(1),
            },
            diff: ParkedDiff {
                cursor: 11,
                scroll: 9,
                col_offset: 8,
            },
            left_col_offset: 12,
            right_col_offset: 13,
            commit_file_folds: HashSet::from(["docs".to_string()]),
            commit_tree_mode: true,
        }
    }

    fn open_compare(app: &mut AppState) {
        app.open_compare_tab("app".into(), "main".into(), COMPARE_HEAD_REF.into());
        assert!(app.is_compare_tab());
    }

    fn open_explorer_diff(app: &mut AppState) {
        app.open_loaded_inner_diff("app", "README.md", DiffContent::from_unified(BODY));
        assert!(app.is_explorer_tab());
    }

    #[test]
    fn tab_view_reads_and_writes_the_live_fields() {
        let mut app = state();
        app.focus = FocusPane::Left;
        app.search_mode = true;
        app.search_active = false;
        app.search_query = "other".into();
        app.search_target = SearchPane::Tree;
        app.search_hit = Some(1);
        app.diff_cursor = 11;
        app.diff_scroll = 9;
        app.diff_col_offset = 8;
        app.left_col_offset = 12;
        app.right_col_offset = 13;
        app.commit_file_folds = HashSet::from(["docs".to_string()]);
        app.commit_tree_mode = true;
        assert_eq!(app.tab_view(), scrambled());

        app.set_tab_view(parked());
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(
            (
                app.search_mode,
                app.search_active,
                app.search_query.as_str()
            ),
            (false, true, "needle")
        );
        assert_eq!(
            (app.search_target, app.search_hit),
            (SearchPane::Diff, Some(4))
        );
        assert_eq!(
            (app.diff_cursor, app.diff_scroll, app.diff_col_offset),
            (7, 3, 5)
        );
        assert_eq!((app.left_col_offset, app.right_col_offset), (2, 6));
        assert_eq!(app.commit_file_folds, HashSet::from(["src".to_string()]));
        assert!(!app.commit_tree_mode);

        let view = app.tab_view();
        app.set_tab_view(view);
        assert_eq!(app.tab_view(), parked(), "a round trip changes nothing");
    }

    #[test]
    fn workspace_view_survives_every_session_tab() {
        let cases: [(&str, Open); 3] = [
            ("compare", open_compare),
            ("explorer", open_explorer_diff),
            ("prs", |app| {
                app.dispatch(Action::OpenPrsTab);
                assert!(app.is_prs_tab());
            }),
        ];
        for (what, open) in cases {
            let mut app = state();
            app.set_tab_view(parked());
            open(&mut app);
            app.set_tab_view(scrambled());
            app.dispatch(Action::JumpToTab(1));
            assert!(app.tabs.is_workspace(), "{what}");
            assert_eq!(app.tab_view(), parked(), "{what}");
            assert!(app.workspace_park.is_none(), "{what}: the park is taken");
        }
    }

    #[test]
    fn compare_tab_parks_its_view_and_derives_the_search_target() {
        // (focus, search mode, search active, restored target)
        let cases = [
            (FocusPane::Right, false, true, SearchPane::Diff),
            (FocusPane::Right, true, false, SearchPane::Diff),
            (FocusPane::Left, false, true, SearchPane::CommitFiles),
            (FocusPane::Right, false, false, SearchPane::CommitFiles),
        ];
        for (focus, mode, active, target) in cases {
            let mut app = state();
            open_compare(&mut app);
            app.set_tab_view(TabView {
                focus,
                search: ParkedSearch {
                    mode,
                    active,
                    ..parked().search
                },
                ..parked()
            });
            app.dispatch(Action::JumpToTab(1));
            app.dispatch(Action::SearchStart);
            app.set_tab_view(scrambled());
            app.dispatch(Action::JumpToTab(2));
            let expected = TabView {
                focus,
                search: ParkedSearch {
                    mode,
                    active,
                    query: "needle".into(),
                    target,
                    hit: Some(4),
                },
                diff: ParkedDiff {
                    cursor: 7,
                    scroll: 3,
                    col_offset: 5,
                },
                left_col_offset: 2,
                right_col_offset: 13,
                commit_file_folds: HashSet::from(["src".to_string()]),
                commit_tree_mode: false,
            };
            let what = format!("{focus:?} mode={mode} active={active}");
            assert_eq!(app.tab_view(), expected, "{what}");
            assert!(app.search_origin.is_some(), "{what}: the origin stays");
        }
    }

    #[test]
    fn explorer_tab_parks_focus_search_and_the_inner_diff() {
        // (opener, restored focus, restored diff viewport)
        let cases: [(&str, Open, FocusPane, ParkedDiff); 2] = [
            (
                "inner diff open",
                open_explorer_diff,
                FocusPane::Right,
                ParkedDiff {
                    cursor: 7,
                    scroll: 3,
                    col_offset: 5,
                },
            ),
            (
                "no inner tab",
                |app| {
                    app.open_explorer_tab("app".into(), ExplorerTarget::Root);
                    assert!(app.is_explorer_tab());
                },
                FocusPane::Left,
                ParkedDiff::default(),
            ),
        ];
        for (what, open, focus, diff) in cases {
            let mut app = state();
            open(&mut app);
            app.set_tab_view(parked());
            app.dispatch(Action::JumpToTab(1));
            app.dispatch(Action::SearchStart);
            app.set_tab_view(scrambled());
            app.dispatch(Action::JumpToTab(2));
            let expected = TabView {
                focus,
                search: ParkedSearch {
                    mode: false,
                    active: true,
                    query: "needle".into(),
                    target: SearchPane::Diff,
                    hit: Some(4),
                },
                diff,
                left_col_offset: 12,
                right_col_offset: 13,
                commit_file_folds: HashSet::from(["docs".to_string()]),
                commit_tree_mode: true,
            };
            assert_eq!(app.tab_view(), expected, "{what}");
            assert!(app.search_origin.is_none(), "{what}: the origin drops");
        }
    }

    #[test]
    fn each_explorer_file_tab_keeps_its_diff_view() {
        let mut app = state();
        open_explorer_diff(&mut app);
        let readme = ParkedDiff {
            cursor: 1,
            scroll: 0,
            col_offset: 2,
        };
        app.set_live_diff(readme);
        app.open_loaded_inner_diff("app", "src/lib.rs", DiffContent::from_unified(BODY));
        assert_eq!(
            app.live_diff(),
            ParkedDiff::default(),
            "a new tab at the top"
        );
        let lib = ParkedDiff {
            cursor: 3,
            scroll: 1,
            col_offset: 4,
        };
        app.set_live_diff(lib);

        app.dispatch(Action::PreviousFileTab);
        assert_eq!(app.live_diff(), readme);
        app.dispatch(Action::NextFileTab);
        assert_eq!(app.live_diff(), lib);

        // The active inner tab's view also rides out a tab switch.
        app.dispatch(Action::JumpToTab(1));
        app.set_live_diff(ParkedDiff {
            cursor: 11,
            scroll: 9,
            col_offset: 8,
        });
        app.dispatch(Action::JumpToTab(2));
        assert_eq!(app.live_diff(), lib);
        app.dispatch(Action::PreviousFileTab);
        assert_eq!(app.live_diff(), readme);
    }

    #[test]
    fn prs_tab_drops_the_search_and_keeps_the_workspace_park() {
        let mut app = state();
        app.dispatch(Action::SearchStart);
        app.set_tab_view(parked());
        app.dispatch(Action::OpenPrsTab);
        assert!(app.is_prs_tab());
        let expected = TabView {
            focus: FocusPane::Left,
            search: ParkedSearch {
                mode: false,
                active: false,
                query: String::new(),
                target: SearchPane::Diff,
                hit: None,
            },
            diff: ParkedDiff {
                cursor: 7,
                scroll: 3,
                col_offset: 5,
            },
            left_col_offset: 2,
            right_col_offset: 6,
            commit_file_folds: HashSet::from(["src".to_string()]),
            commit_tree_mode: false,
        };
        assert_eq!(app.tab_view(), expected);
        assert!(app.search_origin.is_none(), "the origin drops");

        app.set_tab_view(scrambled());
        app.dispatch(Action::JumpToTab(1));
        assert_eq!(app.tab_view(), parked(), "the Workspace park is intact");
    }
}
