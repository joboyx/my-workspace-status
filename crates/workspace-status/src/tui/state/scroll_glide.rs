//! Ctrl-d / Ctrl-u viewport glide (`smoothScroll`).
//!
//! The chord still dispatches `Action::Move(±5)`: the cursor moves at once
//! and no extra pane load runs. [`AppState::arm_scroll_glide`] records the
//! focused pane's painted top before that dispatch; each painter then asks
//! [`AppState::glide_start`] for the top to paint, which eases from the old
//! top to the new one over [`GLIDE_MS`] and sets
//! [`super::LayoutHit::gliding`] so the loop ticks at the draw floor.

use std::time::Instant;

use super::super::anim::{glide_elapsed_ms, glide_step, GlidePane, ScrollGlide, GLIDE_MS};
use super::super::gates::ListFocusTarget;
use super::super::tabs::ExplorerPreview;
use super::{AppState, FocusPane};

impl AppState {
    /// Start a viewport glide on the pane a Ctrl-d / Ctrl-u `Move` is about
    /// to scroll. Call it before the dispatch, at `now`.
    ///
    /// The glide starts from the pane's painted top (from the last glide
    /// frame when a glide on that pane is still live). No-op, and any live
    /// glide dropped, when `smoothScroll` is off or the focused pane has no
    /// glide (Explorer tree, commit-file list).
    pub(crate) fn arm_scroll_glide(&mut self, now: Instant) {
        let pane_top = self.glide_pane_top().filter(|_| self.smooth_scroll);
        let Some((pane, top)) = pane_top else {
            self.scroll_glide = None;
            return;
        };
        let from = match self.scroll_glide {
            Some(live) if live.pane == pane && glide_elapsed_ms(&live, now) < GLIDE_MS => live.last,
            _ => top,
        };
        self.scroll_glide = Some(ScrollGlide {
            pane,
            from,
            started: now,
            last: from,
        });
    }

    /// Drop a live glide: any input other than Ctrl-d / Ctrl-u ends it, so
    /// the next frame paints the plain viewport.
    pub(crate) fn cancel_scroll_glide(&mut self) {
        self.scroll_glide = None;
    }

    /// A glide on `pane` is armed (live or not yet painted to its end).
    pub(crate) fn glide_pending(&self, pane: GlidePane) -> bool {
        self.scroll_glide.is_some_and(|glide| glide.pane == pane)
    }

    /// Viewport top to paint for `pane` at `now`, given the jump `target`
    /// ([`glide_step`]). A glide step marks the frame
    /// [`super::LayoutHit::gliding`].
    pub(crate) fn glide_start(
        &mut self,
        pane: GlidePane,
        target: usize,
        cursor: usize,
        height: usize,
        now: Instant,
    ) -> usize {
        let (top, gliding) = glide_step(&mut self.scroll_glide, pane, target, cursor, height, now);
        self.layout.gliding |= gliding;
        top
    }

    /// The pane a `Move` scrolls now and its painted top.
    fn glide_pane_top(&self) -> Option<(GlidePane, usize)> {
        if let Some(tab) = self.tabs.active_explorer() {
            if self.focus != FocusPane::Right {
                return None;
            }
            return match &tab.active_inner()?.view {
                ExplorerPreview::File(file) => Some((GlidePane::FileView, file.scroll)),
                ExplorerPreview::Diff { .. } => {
                    Some((GlidePane::Diff, usize::from(self.diff_scroll)))
                }
            };
        }
        match self.list_focus_target() {
            ListFocusTarget::Tree => Some((GlidePane::Tree, self.layout.list_offset)),
            ListFocusTarget::Graph => Some((GlidePane::Graph, usize::from(self.graph_scroll))),
            ListFocusTarget::None => Some((GlidePane::Diff, usize::from(self.diff_scroll))),
            ListFocusTarget::CommitFiles => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::settings::Settings;
    use crate::snapshot::build_workspace_snapshot;

    fn app() -> AppState {
        let snapshot = build_workspace_snapshot(&[], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, false)
    }

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// Tree focused, last frame painted from row `top`, glide armed at `now`.
    fn armed_tree(top: usize, now: Instant) -> AppState {
        let mut app = app();
        assert_eq!(app.focus, FocusPane::Left);
        app.layout.list_offset = top;
        app.arm_scroll_glide(now);
        app
    }

    #[test]
    fn glide_start_eases_from_old_top_to_new_top_within_48ms() {
        let t0 = Instant::now();
        let mut app = armed_tree(0, t0);
        assert!(app.glide_pending(GlidePane::Tree));
        // Ctrl-d moved the cursor to row 15, inside both the old view
        // [0, 20) and the jump view [10, 30), so no clamp applies.
        let (target, cursor, height) = (10, 15, 20);

        assert_eq!(
            app.glide_start(GlidePane::Tree, target, cursor, height, t0),
            0
        );
        assert!(app.layout.gliding, "the first frame is a glide frame");

        app.layout.gliding = false;
        let mid = app.glide_start(
            GlidePane::Tree,
            target,
            cursor,
            height,
            at(t0, GLIDE_MS / 2),
        );
        assert!(0 < mid && mid < target, "mid-glide top {mid} sits between");
        assert!(app.layout.gliding);

        app.layout.gliding = false;
        let end = app.glide_start(GlidePane::Tree, target, cursor, height, at(t0, GLIDE_MS));
        assert_eq!(end, target, "the glide lands on the jump top");
        assert!(
            !app.layout.gliding,
            "the last frame does not ask for a tick"
        );
        assert!(!app.glide_pending(GlidePane::Tree), "the glide is cleared");
        assert_eq!(
            app.glide_start(GlidePane::Tree, target, cursor, height, at(t0, GLIDE_MS)),
            target
        );
    }

    #[test]
    fn glide_keeps_cursor_on_screen() {
        let t0 = Instant::now();
        // From the top to row 50 with the cursor on the last row of the
        // new view: the eased top must not leave the cursor below the view.
        let mut app = armed_tree(0, t0);
        let top = app.glide_start(GlidePane::Tree, 50, 59, 10, at(t0, 1));
        assert!(
            (50..=59).contains(&top),
            "cursor 59 shows in [{top}, {top}+10)"
        );

        // Gliding up: the top never passes the cursor.
        let mut app = armed_tree(60, t0);
        let top = app.glide_start(GlidePane::Tree, 0, 2, 10, at(t0, 1));
        assert!(top <= 2, "cursor 2 shows in [{top}, {top}+10)");
    }

    #[test]
    fn glide_rearmed_mid_glide_starts_from_the_painted_top() {
        let t0 = Instant::now();
        let mut app = armed_tree(0, t0);
        let mid = app.glide_start(GlidePane::Tree, 10, 15, 20, at(t0, GLIDE_MS / 2));
        assert!(0 < mid && mid < 10);
        // A second Ctrl-d before the first glide ends restarts from the
        // step on screen, not from the pane's recorded top (the graph and
        // file view keep their jump top there).
        app.layout.list_offset = 10;
        let t1 = at(t0, GLIDE_MS / 2 + 1);
        app.arm_scroll_glide(t1);
        assert_eq!(app.glide_start(GlidePane::Tree, 15, 20, 20, t1), mid);
    }

    #[test]
    fn glide_only_moves_its_own_pane() {
        let t0 = Instant::now();
        let mut app = armed_tree(0, t0);
        assert_eq!(app.glide_start(GlidePane::Graph, 20, 30, 20, at(t0, 8)), 20);
        assert!(!app.layout.gliding);
        assert!(app.glide_pending(GlidePane::Tree), "the tree glide stays");
        app.cancel_scroll_glide();
        assert!(!app.glide_pending(GlidePane::Tree));
        assert_eq!(app.glide_start(GlidePane::Tree, 20, 30, 20, at(t0, 8)), 20);
    }

    #[test]
    fn glide_disabled_jumps_immediately() {
        let t0 = Instant::now();
        let mut app = app();
        app.smooth_scroll = false;
        app.layout.list_offset = 0;
        app.arm_scroll_glide(t0);
        assert!(!app.glide_pending(GlidePane::Tree), "off: nothing is armed");
        assert_eq!(app.glide_start(GlidePane::Tree, 20, 30, 20, t0), 20);
        assert!(!app.layout.gliding, "off: no glide tick");
        assert_eq!(app.next_anim_tick_ms(), None);
    }

    #[test]
    fn smooth_scroll_defaults_on() {
        let dir = std::env::temp_dir().join(format!("ws-smooth-scroll-{}", std::process::id()));
        let stores = [
            ("WS_STATUS_VIEWED_STORE", dir.join("viewed.json")),
            ("WS_STATUS_COMMENT_STORE", dir.join("comments.json")),
            ("WS_STATUS_THEME_STORE", dir.join("theme.json")),
        ];
        let settings = Settings::resolve(&Default::default(), |key| {
            stores
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, path)| path.display().to_string())
        });
        assert!(settings.smooth_scroll);
        let snapshot = build_workspace_snapshot(&[], &[], false, &[]);
        let launched = AppState::launch(PathBuf::from("/tmp"), snapshot, &settings);
        assert!(
            launched.smooth_scroll,
            "launch default: Ctrl-d / Ctrl-u glide"
        );
        assert!(app().smooth_scroll, "test state keeps the shipped default");
    }
}
