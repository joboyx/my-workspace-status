//! Background job outages on [`AppState`]: start, recover, and the Esc
//! dismiss of their sticky toasts ([`super::super::outage`]).

use std::time::Instant;

use super::super::action::Action;
use super::super::outage::{OutageNote, OutageSource};
use super::super::toast::Toast;
use super::AppState;

impl AppState {
    /// Note that `source` failed, with `toast_text` naming the tool, the
    /// host, and the reason (`glab (gitlab.com): invalid_token`).
    ///
    /// A source that was not failing starts an outage and pushes one
    /// sticky outage toast at `now`. A source that already fails only
    /// updates its stored reason (and the text of its toast while that
    /// still shows): no new toast, and a dismissed toast stays dismissed.
    /// Pure state, no I/O.
    pub(crate) fn note_outage(&mut self, source: OutageSource, toast_text: String, now: Instant) {
        match self.outages.fail(source.clone(), toast_text.clone()) {
            OutageNote::Started => self
                .toasts
                .push_with(Toast::outage(source, toast_text, self.ascii, now)),
            OutageNote::ReasonChanged => self.toasts.set_outage_text(&source, &toast_text),
            OutageNote::Unchanged => {}
        }
    }

    /// Note that `source` works again: end its outage, drop its toast if
    /// it still shows, and clear its part of the header mark. A source
    /// that was not failing changes nothing.
    pub(crate) fn note_recovered(&mut self, source: &OutageSource) {
        if self.outages.recover(source) {
            self.toasts.drop_outage(source);
        }
    }

    /// Hide every shown outage toast. The outages stay, so the header
    /// mark stays, and their toasts do not come back while they last.
    /// True when a toast was shown.
    pub(crate) fn dismiss_outage_toasts(&mut self) -> bool {
        self.outages.dismiss_all();
        self.toasts.dismiss_outages()
    }

    /// True when `action` is an Esc (or a right click, which is Esc) that
    /// only dismisses the shown outage toasts: an outage toast shows and
    /// nothing open takes the press first.
    ///
    /// Overlays, pickers, a pinned popover, chords, and visual highlight
    /// are input modes and never send [`Action::NavEsc`]. Here an armed
    /// search, a compare picker still loading, and (for a right click) a
    /// `V` highlight keep the press.
    pub(crate) fn esc_dismisses_outage_toasts(&self, action: &Action) -> bool {
        let esc = match action {
            Action::NavEsc => true,
            Action::BackClick => self.mouse_enabled,
            _ => false,
        };
        esc && !self.search_active
            && self.compare_picker_pending.is_none()
            && self.diff_visual_anchor.is_none()
            && self.toasts.has_outage()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Instant;

    use super::super::super::action::{Action, Effect};
    use super::super::super::drill::{CommitFile, CommitFileSource, DrillView};
    use super::super::super::toast::OUTAGE_DISMISS_HINT;
    use super::super::{AppState, FocusPane};
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};

    fn checkout(name: &str) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::UpToDate,
            sync_note: String::new(),
            upstream_rewrite: None,
            head: "abc".into(),
            has_unstaged: false,
            has_staged: false,
            has_untracked: false,
            changes: Vec::new(),
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            default_branch: None,
            local_branches: Vec::new(),
        }
    }

    fn app() -> AppState {
        let snapshot = build_workspace_snapshot(&[checkout("app")], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, false)
    }

    fn gitlab() -> OutageSource {
        OutageSource::PrLookup {
            tool: "glab",
            host: "gitlab.com".into(),
        }
    }

    /// Texts of the toasts held now, oldest first.
    fn toast_texts(app: &AppState) -> Vec<String> {
        app.toasts
            .visible(Instant::now())
            .map(|(toast, _)| toast.text.clone())
            .collect()
    }

    #[test]
    fn an_outage_toasts_once_per_outage_and_recovery_clears_it() {
        let mut app = app();
        let now = Instant::now();
        app.note_outage(gitlab(), "glab (gitlab.com): HTTP 401".into(), now);
        assert_eq!(toast_texts(&app), ["glab (gitlab.com): HTTP 401"]);
        let toast = app.toasts.visible(now).next().unwrap().0.clone();
        assert_eq!(toast.outage, Some(gitlab()));
        assert_eq!(toast.detail, OUTAGE_DISMISS_HINT);

        // The same source fails again: no new toast, the reason updates.
        app.note_outage(gitlab(), "glab (gitlab.com): invalid_token".into(), now);
        assert_eq!(toast_texts(&app), ["glab (gitlab.com): invalid_token"]);
        assert_eq!(
            app.outages.get(&gitlab()).unwrap().reason,
            "glab (gitlab.com): invalid_token"
        );

        // Dismiss: the toast goes, the outage (header) stays.
        assert!(app.dismiss_outage_toasts());
        assert!(toast_texts(&app).is_empty());
        assert!(!app.outages.is_empty(), "the header mark stays");
        assert!(!app.dismiss_outage_toasts(), "nothing left to dismiss");

        // A fail while dismissed shows no toast.
        app.note_outage(gitlab(), "glab (gitlab.com): HTTP 401".into(), now);
        assert!(toast_texts(&app).is_empty());
        assert!(app.outages.get(&gitlab()).unwrap().dismissed);

        // Recover: the outage and its header part go.
        app.note_recovered(&gitlab());
        assert!(app.outages.is_empty());

        // A fail after the recover toasts again.
        app.note_outage(gitlab(), "glab (gitlab.com): HTTP 401".into(), now);
        assert_eq!(toast_texts(&app), ["glab (gitlab.com): HTTP 401"]);
        // Recover while the toast shows drops it too.
        app.note_recovered(&gitlab());
        assert!(toast_texts(&app).is_empty());
        assert!(app.outages.is_empty());
    }

    #[test]
    fn two_sources_toast_and_recover_on_their_own() {
        let mut app = app();
        let now = Instant::now();
        app.note_outage(gitlab(), "glab (gitlab.com): invalid_token".into(), now);
        app.note_outage(
            OutageSource::ScheduledFetch,
            "git fetch (app): could not read from remote".into(),
            now,
        );
        assert_eq!(
            toast_texts(&app),
            [
                "glab (gitlab.com): invalid_token",
                "git fetch (app): could not read from remote"
            ]
        );
        app.note_recovered(&gitlab());
        assert_eq!(
            toast_texts(&app),
            ["git fetch (app): could not read from remote"]
        );
        assert!(app.outages.get(&gitlab()).is_none());
        assert!(app.outages.get(&OutageSource::ScheduledFetch).is_some());
        app.note_recovered(&OutageSource::LiveRefresh);
        assert_eq!(
            toast_texts(&app).len(),
            1,
            "a working source changes nothing"
        );
    }

    #[test]
    fn esc_with_an_outage_toast_dismisses_it_and_keeps_the_depth() {
        let mut app = app();
        app.drill = DrillView::Files {
            repo: "app".into(),
            source: CommitFileSource::Commit {
                commit_id: "aaa1111".into(),
            },
            files: vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
            cursor: 0,
        };
        app.focus = FocusPane::Left;
        app.note_outage(
            gitlab(),
            "glab (gitlab.com): invalid_token".into(),
            Instant::now(),
        );
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(toast_texts(&app).is_empty(), "dismissed");
        assert!(app.drill.is_files(), "no back step in that press");
        assert!(!app.outages.is_empty(), "the header mark stays");
        assert_eq!(
            app.dispatch(Action::NavEsc),
            Effect::LoadRightPane,
            "the next Esc steps back as before"
        );
        assert!(app.drill.is_graph());

        // From the right pane the focus stays right in the dismiss press.
        app.focus = FocusPane::Right;
        app.note_recovered(&gitlab());
        app.note_outage(
            gitlab(),
            "glab (gitlab.com): invalid_token".into(),
            Instant::now(),
        );
        app.dispatch(Action::NavEsc);
        assert_eq!(app.focus, FocusPane::Right);
        assert!(toast_texts(&app).is_empty());
        app.dispatch(Action::NavEsc);
        assert_eq!(app.focus, FocusPane::Left, "then Esc moves left as before");
    }

    #[test]
    fn esc_without_an_outage_toast_behaves_as_before() {
        let mut app = app();
        app.focus = FocusPane::Right;
        app.toasts.push("copied", Instant::now());
        app.dispatch(Action::NavEsc);
        assert_eq!(app.focus, FocusPane::Left, "an ack toast does not take Esc");
        assert_eq!(toast_texts(&app), ["copied"]);

        // A dismissed outage keeps its header but no longer takes Esc.
        app.note_outage(
            gitlab(),
            "glab (gitlab.com): invalid_token".into(),
            Instant::now(),
        );
        app.dismiss_outage_toasts();
        app.focus = FocusPane::Right;
        app.dispatch(Action::NavEsc);
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn an_armed_search_takes_esc_before_the_outage_toast() {
        let mut app = app();
        app.note_outage(
            gitlab(),
            "glab (gitlab.com): invalid_token".into(),
            Instant::now(),
        );
        app.search_active = true;
        app.search_query = "app".into();
        app.dispatch(Action::NavEsc);
        assert!(!app.search_active, "the first Esc clears the search");
        assert_eq!(toast_texts(&app).len(), 1, "the toast still shows");
        app.dispatch(Action::NavEsc);
        assert!(toast_texts(&app).is_empty(), "the next Esc dismisses it");
    }

    #[test]
    fn a_pinned_popover_or_an_overlay_never_sends_nav_esc() {
        use super::super::super::keys::{event_to_action, InputMode};
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        for mode in [
            InputMode::Popover,
            InputMode::Help,
            InputMode::QuickOpen,
            InputMode::BranchPicker,
            InputMode::DiffVisual,
        ] {
            assert_ne!(
                event_to_action(&esc, mode, false, false),
                Action::NavEsc,
                "{mode:?} takes Esc itself"
            );
        }
    }

    #[test]
    fn esc_and_right_click_dismiss_on_the_prs_tab_before_the_detail_steps_back() {
        let mut app = app();
        app.dispatch(Action::OpenPrsTab);
        assert!(app.is_prs_tab());
        app.layout.term_cols = 90;
        app.dispatch(Action::FocusRight);
        let shown = |app: &AppState| app.tabs.active_prs().unwrap().show_detail;
        assert!(shown(&app), "narrow: the detail shows");
        app.note_outage(
            gitlab(),
            "glab (gitlab.com): invalid_token".into(),
            Instant::now(),
        );
        app.dispatch(Action::NavEsc);
        assert!(toast_texts(&app).is_empty());
        assert!(shown(&app), "the dismiss press keeps the detail");
        app.dispatch(Action::NavEsc);
        assert!(!shown(&app), "the next Esc goes back to the list");

        app.dispatch(Action::FocusRight);
        app.note_recovered(&gitlab());
        app.note_outage(
            gitlab(),
            "glab (gitlab.com): invalid_token".into(),
            Instant::now(),
        );
        app.mouse_enabled = true;
        app.dispatch(Action::BackClick);
        assert!(toast_texts(&app).is_empty(), "a right click is Esc");
        assert!(shown(&app));
    }
}
