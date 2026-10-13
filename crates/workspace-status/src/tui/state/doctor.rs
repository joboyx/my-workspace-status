//! Doctor setup checkup overlay: open, move, copy, re-run, close.
//!
//! [`Action::OpenDoctor`] opens the overlay in a checking state and asks
//! for [`Effect::RunDoctor`]; the interpreter runs
//! [`crate::tui::doctor::collect`] on a blocking worker and
//! [`AppState::apply_doctor`] fills the rows. Report only: nothing here
//! writes a file, a setting, or the UI-state store. The one write is `y`,
//! which copies the focused row's fix line through
//! [`Effect::CopyClipboard`], the same path as Copy entity reference.

use super::super::action::{Action, Effect};
use super::super::doctor::Check;
use super::super::status::StatusMessage;
use super::AppState;

/// Status text when `y` lands on a row with no fix line to copy.
pub(crate) const NO_FIX_TO_COPY: &str = "no fix to copy";

/// The open Doctor overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctorOverlay {
    /// Probe request whose answer may fill the rows.
    pub(crate) gen: u64,
    /// One row per check, in paint order. `None` while the probe runs.
    pub checks: Option<Vec<Check>>,
    /// Focused row index into [`Self::checks`].
    pub cursor: usize,
    /// First body line the last paint showed. Paint keeps the cursor row
    /// in view from here, so a move scrolls only as far as it must.
    pub scroll: usize,
}

impl AppState {
    /// Run a Doctor action: the overlay keys and the palette opener.
    pub(super) fn dispatch_doctor(&mut self, action: Action) -> Effect {
        match action {
            Action::OpenDoctor => {
                self.doctor = Some(DoctorOverlay {
                    gen: 0,
                    checks: None,
                    cursor: 0,
                    scroll: 0,
                });
                self.rerun_doctor()
            }
            Action::DoctorRerun => self.rerun_doctor(),
            Action::DoctorMove(delta) => {
                self.move_doctor_cursor(i64::from(delta));
                Effect::None
            }
            Action::DoctorTop => {
                self.move_doctor_cursor(i64::MIN / 2);
                Effect::None
            }
            Action::DoctorBottom => {
                self.move_doctor_cursor(i64::MAX / 2);
                Effect::None
            }
            Action::DoctorCopy => self.copy_doctor_fix(),
            Action::DoctorClose => {
                self.doctor = None;
                Effect::None
            }
            _ => Effect::None,
        }
    }

    /// Start a new probe request: the rows go back to checking, the cursor
    /// stays.
    fn rerun_doctor(&mut self) -> Effect {
        let Some(doctor) = self.doctor.as_mut() else {
            return Effect::None;
        };
        self.doctor_gen += 1;
        doctor.gen = self.doctor_gen;
        doctor.checks = None;
        Effect::RunDoctor {
            gen: self.doctor_gen,
        }
    }

    fn move_doctor_cursor(&mut self, delta: i64) {
        let Some(doctor) = self.doctor.as_mut() else {
            return;
        };
        let len = doctor.checks.as_ref().map_or(0, Vec::len);
        if len == 0 {
            return;
        }
        let last = i64::try_from(len - 1).unwrap_or(i64::MAX);
        let next = i64::try_from(doctor.cursor)
            .unwrap_or(i64::MAX)
            .saturating_add(delta)
            .clamp(0, last);
        doctor.cursor = usize::try_from(next).unwrap_or(0);
    }

    /// The fix line `y` copies on the focused row, if it has one.
    pub(crate) fn doctor_copy_text(&self) -> Option<String> {
        let doctor = self.doctor.as_ref()?;
        let check = doctor.checks.as_ref()?.get(doctor.cursor)?;
        check.fix.as_ref()?.copy.clone()
    }

    fn copy_doctor_fix(&mut self) -> Effect {
        match self.doctor_copy_text() {
            Some(text) => Effect::CopyClipboard {
                text,
                announce: true,
            },
            None => {
                self.status = StatusMessage::warn(NO_FIX_TO_COPY);
                Effect::None
            }
        }
    }

    /// True while the open Doctor waits for the answer to request `gen`.
    pub(crate) fn doctor_awaits(&self, gen: u64) -> bool {
        self.doctor
            .as_ref()
            .is_some_and(|doctor| doctor.gen == gen && doctor.checks.is_none())
    }

    /// Fill the Doctor rows with the answer to request `gen`.
    ///
    /// Returns false (and changes nothing) for a stale answer: the overlay
    /// closed, or a re-run asked again since.
    pub(crate) fn apply_doctor(&mut self, gen: u64, checks: Vec<Check>) -> bool {
        if !self.doctor_awaits(gen) {
            return false;
        }
        let Some(doctor) = self.doctor.as_mut() else {
            return false;
        };
        doctor.cursor = doctor.cursor.min(checks.len().saturating_sub(1));
        doctor.checks = Some(checks);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::snapshot::build_workspace_snapshot;
    use crate::tui::doctor::{CheckGroup, CheckStatus, Fix};
    use crate::tui::keys::{InputMode, DOCTOR_PAGE_ROWS};

    fn app() -> AppState {
        let snapshot = build_workspace_snapshot(&[], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn check(name: &'static str, copy: Option<&str>) -> Check {
        Check {
            group: CheckGroup::Tools,
            name,
            status: CheckStatus::Warn,
            why: format!("{name} why."),
            fix: Some(Fix {
                hint: Some("hint".into()),
                copy: copy.map(str::to_string),
            }),
        }
    }

    fn rows() -> Vec<Check> {
        vec![
            check("git", Some("sudo apt install git")),
            check("Editor (e)", None),
            check("curl", Some("sudo apt install curl")),
        ]
    }

    /// Open, then answer the open request.
    fn opened(app: &mut AppState) {
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        assert!(app.apply_doctor(gen, rows()));
    }

    #[test]
    fn open_shows_checking_and_asks_for_one_probe() {
        let mut app = app();
        let effect = app.dispatch(Action::OpenDoctor);
        assert_eq!(effect, Effect::RunDoctor { gen: 1 });
        assert_eq!(app.input_mode(), InputMode::Doctor);
        let doctor = app.doctor.as_ref().expect("open");
        assert_eq!(doctor.checks, None, "checking");
        assert!(app.doctor_awaits(1));
    }

    #[test]
    fn answer_fills_rows_and_a_stale_answer_is_dropped() {
        let mut app = app();
        app.dispatch(Action::OpenDoctor);
        // `r` before the first answer: only the second request lands.
        assert_eq!(
            app.dispatch(Action::DoctorRerun),
            Effect::RunDoctor { gen: 2 }
        );
        assert!(!app.apply_doctor(1, rows()), "superseded");
        assert_eq!(app.doctor.as_ref().unwrap().checks, None);
        assert!(app.apply_doctor(2, rows()));
        assert_eq!(
            app.doctor.as_ref().unwrap().checks.as_ref().unwrap().len(),
            3
        );
        assert!(!app.apply_doctor(2, rows()), "answered once");

        // Closed: a late answer changes nothing.
        app.dispatch(Action::DoctorRerun);
        app.dispatch(Action::DoctorClose);
        assert!(!app.apply_doctor(3, rows()));
        assert!(app.doctor.is_none());
    }

    #[test]
    fn keys_move_clamp_and_close() {
        let mut app = app();
        opened(&mut app);
        let cursor = |app: &AppState| app.doctor.as_ref().unwrap().cursor;
        assert_eq!(app.dispatch(Action::DoctorMove(1)), Effect::None);
        assert_eq!(cursor(&app), 1);
        app.dispatch(Action::DoctorMove(DOCTOR_PAGE_ROWS));
        assert_eq!(cursor(&app), 2, "clamped to the last row");
        app.dispatch(Action::DoctorMove(-1));
        assert_eq!(cursor(&app), 1);
        app.dispatch(Action::DoctorTop);
        assert_eq!(cursor(&app), 0);
        app.dispatch(Action::DoctorBottom);
        assert_eq!(cursor(&app), 2);
        app.dispatch(Action::DoctorMove(-DOCTOR_PAGE_ROWS));
        assert_eq!(cursor(&app), 0);
        // A re-run keeps the cursor; an answer with fewer rows clamps it.
        app.dispatch(Action::DoctorBottom);
        let Effect::RunDoctor { gen } = app.dispatch(Action::DoctorRerun) else {
            panic!("re-run asks again");
        };
        assert_eq!(cursor(&app), 2);
        assert!(app.apply_doctor(gen, rows()[..1].to_vec()));
        assert_eq!(cursor(&app), 0);
        assert_eq!(app.dispatch(Action::DoctorClose), Effect::None);
        assert!(app.doctor.is_none());
        assert_ne!(app.input_mode(), InputMode::Doctor);
    }

    #[test]
    fn y_copies_the_focused_fix_line_or_says_there_is_none() {
        let mut app = app();
        opened(&mut app);
        assert_eq!(
            app.dispatch(Action::DoctorCopy),
            Effect::CopyClipboard {
                text: "sudo apt install git".into(),
                announce: true,
            }
        );
        app.dispatch(Action::DoctorMove(1));
        assert_eq!(app.dispatch(Action::DoctorCopy), Effect::None);
        assert_eq!(app.status, NO_FIX_TO_COPY);
        // Still checking: nothing to copy either.
        app.dispatch(Action::DoctorRerun);
        assert_eq!(app.dispatch(Action::DoctorCopy), Effect::None);
    }

    /// Opening, moving, re-running, and closing write nothing: no
    /// clipboard, no UI-state store file.
    #[test]
    fn open_move_rerun_close_write_nothing() {
        let dir = crate::testutil::unique_dir("ws-doctor-report-only");
        std::fs::create_dir_all(&dir).unwrap();
        let snapshot = build_workspace_snapshot(&[], &[], false, &[]);
        let mut app =
            AppState::with_viewed_path(PathBuf::from("/tmp"), snapshot, true, dir.join("v.json"));
        let files = || std::fs::read_dir(&dir).unwrap().count();
        let before = files();
        let mut effects = vec![app.dispatch(Action::OpenDoctor)];
        let Some(Effect::RunDoctor { gen }) = effects.first().cloned() else {
            panic!("open asks for a probe");
        };
        app.apply_doctor(gen, rows());
        for action in [
            Action::DoctorMove(1),
            Action::DoctorTop,
            Action::DoctorBottom,
            Action::DoctorRerun,
            Action::DoctorClose,
        ] {
            effects.push(app.dispatch(action));
        }
        for effect in &effects {
            assert!(
                matches!(effect, Effect::None | Effect::RunDoctor { .. }),
                "{effect:?}"
            );
        }
        assert!(!app.ui_store_path.exists(), "no UI-state save");
        assert_eq!(files(), before, "no file written");
        let _ = std::fs::remove_dir_all(dir);
    }
}
