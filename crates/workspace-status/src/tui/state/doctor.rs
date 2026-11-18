//! Doctor setup checkup overlay: open, move, copy, re-run, close.
//!
//! [`Action::OpenDoctor`] and [`Action::DoctorRerun`] start one request
//! with two parts:
//!
//! - The probe: [`Effect::RunDoctor`] runs [`crate::tui::doctor::collect`]
//!   on a blocking worker, and [`AppState::apply_doctor`] stores its rows.
//! - The terminal capability query ([`CapsQuery`]): the next frame writes
//!   [`QUERY`](crate::tui::term_caps::QUERY)
//!   ([`AppState::doctor_caps_query_to_send`],
//!   [`AppState::doctor_caps_query_sent`]), the input thread forwards the
//!   replies ([`AppState::apply_doctor_term_replies`]), and the DA1 reply or
//!   the deadline ([`AppState::expire_doctor_caps_query`]) ends the wait.
//!
//! The head stays `checking…` until both parts ended. Then
//! [`term_caps::doctor_rows`] builds the one row list that paint, the
//! cursor clamp, and the `y` copy read, so no probe or capability row
//! changes after it paints.
//!
//! The live mouse wheel test ([`WheelTest`]) is not a probe: Up / Down
//! arrows ([`Action::DoctorArrow`]) and wheel events
//! ([`Action::DoctorWheel`]) record it on the open overlay, also while it
//! shows `checking…`, and it survives `r`. When it changes on painted rows,
//! the list is built again from the stored probe rows and replies (no new
//! probe or query), so only its row changes and the cursor stays.
//!
//! Report only: nothing here writes a file, a setting, or the UI-state
//! store, and the query changes no terminal mode. The one write is `y`,
//! which copies the focused row's fix line through
//! [`Effect::CopyClipboard`], the same path as Copy entity reference.

use std::time::Instant;

use super::super::action::{Action, Effect};
use super::super::doctor::{Check, Os, WheelRow, WheelTest};
use super::super::status::StatusMessage;
use super::super::term_caps::{self, TermCaps, TermReply, CAPS_QUERY_TIMEOUT};
use super::AppState;

/// Status text when `y` lands on a row with no fix line to copy.
pub(crate) const NO_FIX_TO_COPY: &str = "no fix to copy";

/// Where the Doctor's terminal capability query stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CapsQuery {
    /// Asked for: the next frame writes the query bytes.
    Requested,
    /// Written. The DA1 reply or `deadline` (send instant plus
    /// [`CAPS_QUERY_TIMEOUT`]) ends the wait.
    Sent {
        /// When the wait ends with no DA1 reply.
        deadline: Instant,
    },
    /// The DA1 reply came, the deadline passed, or the query was not sent.
    Ended,
}

/// The open Doctor overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctorOverlay {
    /// The request whose probe answer and terminal replies may fill the
    /// rows. The input thread tags replies with it too.
    pub(crate) gen: u64,
    /// OS family the rows target ([`AppState::doctor_os`] at open).
    pub(crate) os: Os,
    /// Probe rows of request `gen`. `None` while the probe runs.
    pub(crate) probe: Option<Vec<Check>>,
    /// `TMUX` was set in the environment the probe read.
    pub(crate) in_tmux: bool,
    /// The capability query of request `gen`.
    pub(crate) caps_query: CapsQuery,
    /// The terminal replies to that query.
    pub(crate) caps: TermCaps,
    /// The live mouse wheel test since open. Kept across `r`.
    pub(crate) wheel: WheelTest,
    /// The full row list in paint order ([`term_caps::doctor_rows`]).
    /// `None` until the probe answered and the query ended.
    pub checks: Option<Vec<Check>>,
    /// Focused row index into [`Self::checks`].
    pub cursor: usize,
    /// First body line the last paint showed. Paint keeps the cursor row
    /// in view from here, so a move scrolls only as far as it must.
    pub scroll: usize,
}

/// A new capability query for `os`: requested, except on Windows, where it
/// ends at once with every capability row "not asked" (crossterm reads the
/// input there, and it may read the replies as keys).
fn new_caps_query(os: Os) -> (CapsQuery, TermCaps) {
    if os == Os::Windows {
        (CapsQuery::Ended, TermCaps::not_asked())
    } else {
        (CapsQuery::Requested, TermCaps::default())
    }
}

impl DoctorOverlay {
    /// Build the row list once the probe answered and the query ended
    /// (`capture`: mouse capture is on). True when it did so now.
    fn finish_rows(&mut self, capture: bool) -> bool {
        if self.checks.is_some() || self.caps_query != CapsQuery::Ended {
            return false;
        }
        self.build_rows(capture)
    }

    /// Build the row list from the stored probe rows, replies, and wheel
    /// test, clamping the cursor. False while the probe runs.
    fn build_rows(&mut self, capture: bool) -> bool {
        let Some(probe) = self.probe.as_deref() else {
            return false;
        };
        let wheel = WheelRow::Live {
            test: self.wheel,
            capture,
        };
        let rows = term_caps::doctor_rows(probe, wheel, &self.caps, self.os, self.in_tmux);
        self.cursor = self.cursor.min(rows.len().saturating_sub(1));
        self.checks = Some(rows);
        true
    }

    /// Store the wheel test state `next`; rebuild painted rows when it
    /// changed (rows still checking pick it up when they are built).
    fn record_wheel_test(&mut self, next: WheelTest, capture: bool) {
        if next == self.wheel {
            return;
        }
        self.wheel = next;
        if self.checks.is_some() {
            self.build_rows(capture);
        }
    }
}

impl AppState {
    /// Run a Doctor action: the overlay keys and the palette opener.
    pub(super) fn dispatch_doctor(&mut self, action: Action) -> Effect {
        match action {
            Action::OpenDoctor => {
                let os = self.doctor_os;
                let (caps_query, caps) = new_caps_query(os);
                self.doctor = Some(DoctorOverlay {
                    gen: 0,
                    os,
                    probe: None,
                    in_tmux: false,
                    caps_query,
                    caps,
                    wheel: WheelTest::Waiting,
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
            Action::DoctorArrow(delta) => {
                self.record_doctor_wheel_test(WheelTest::arrow);
                self.move_doctor_cursor(i64::from(delta));
                Effect::None
            }
            Action::DoctorWheel(delta) => {
                self.record_doctor_wheel_test(|_| WheelTest::Wheel);
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

    /// Start a new request: the rows go back to checking, a new probe and
    /// a new capability query are asked for, the cursor stays.
    fn rerun_doctor(&mut self) -> Effect {
        let Some(doctor) = self.doctor.as_mut() else {
            return Effect::None;
        };
        self.doctor_gen += 1;
        doctor.gen = self.doctor_gen;
        doctor.probe = None;
        doctor.checks = None;
        (doctor.caps_query, doctor.caps) = new_caps_query(doctor.os);
        Effect::RunDoctor {
            gen: self.doctor_gen,
        }
    }

    /// Record one wheel test input on the open Doctor: `step` maps the
    /// state before it to the state after it.
    fn record_doctor_wheel_test(&mut self, step: impl FnOnce(WheelTest) -> WheelTest) {
        let capture = self.mouse_enabled;
        if let Some(doctor) = self.doctor.as_mut() {
            doctor.record_wheel_test(step(doctor.wheel), capture);
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

    /// True while the open Doctor waits for the probe answer to request
    /// `gen`.
    pub(crate) fn doctor_awaits(&self, gen: u64) -> bool {
        self.doctor
            .as_ref()
            .is_some_and(|doctor| doctor.gen == gen && doctor.probe.is_none())
    }

    /// Store the probe rows of request `gen` (`in_tmux`: `TMUX` was set in
    /// the environment the probe read). The rows paint once the capability
    /// query ended too.
    ///
    /// Returns false (and changes nothing) for a stale answer: the overlay
    /// closed, or a re-run asked again since.
    pub(crate) fn apply_doctor(&mut self, gen: u64, checks: Vec<Check>, in_tmux: bool) -> bool {
        if !self.doctor_awaits(gen) {
            return false;
        }
        let capture = self.mouse_enabled;
        let Some(doctor) = self.doctor.as_mut() else {
            return false;
        };
        doctor.probe = Some(checks);
        doctor.in_tmux = in_tmux;
        doctor.finish_rows(capture);
        true
    }

    /// The request whose capability query the next frame must write, if
    /// one is asked for and not written yet.
    pub(crate) fn doctor_caps_query_to_send(&self) -> Option<u64> {
        self.doctor
            .as_ref()
            .filter(|doctor| doctor.caps_query == CapsQuery::Requested)
            .map(|doctor| doctor.gen)
    }

    /// The query of request `gen` was written at `at`: the wait ends at
    /// `at` plus [`CAPS_QUERY_TIMEOUT`] unless the DA1 reply comes first.
    pub(crate) fn doctor_caps_query_sent(&mut self, gen: u64, at: Instant) {
        if let Some(doctor) = self.doctor.as_mut() {
            if doctor.gen == gen && doctor.caps_query == CapsQuery::Requested {
                doctor.caps_query = CapsQuery::Sent {
                    deadline: at + CAPS_QUERY_TIMEOUT,
                };
            }
        }
    }

    /// Apply terminal replies the input thread tagged with request `epoch`
    /// (the request whose query was the last one written when it took
    /// them). Replies for another request, with the Doctor closed, or once
    /// the wait ended are dropped. True when the rows are ready now.
    pub(crate) fn apply_doctor_term_replies(
        &mut self,
        epoch: u64,
        replies: Vec<TermReply>,
    ) -> bool {
        let capture = self.mouse_enabled;
        let Some(doctor) = self.doctor.as_mut() else {
            return false;
        };
        if doctor.gen != epoch || !matches!(doctor.caps_query, CapsQuery::Sent { .. }) {
            return false;
        }
        for reply in replies {
            doctor.caps.apply(reply);
        }
        if !doctor.caps.is_done() {
            return false;
        }
        doctor.caps_query = CapsQuery::Ended;
        doctor.finish_rows(capture)
    }

    /// Milliseconds until the capability query's deadline (rounded up),
    /// while a written query waits.
    pub(crate) fn doctor_caps_remain_ms(&self, now: Instant) -> Option<u64> {
        match self.doctor.as_ref()?.caps_query {
            CapsQuery::Sent { deadline } => {
                let micros = deadline.saturating_duration_since(now).as_micros();
                Some(u64::try_from(micros.div_ceil(1000)).unwrap_or(u64::MAX))
            }
            CapsQuery::Requested | CapsQuery::Ended => None,
        }
    }

    /// End a written query whose deadline passed by `now`: the rows with no
    /// reply show unknown. True when the wait ended now (redraw).
    pub(crate) fn expire_doctor_caps_query(&mut self, now: Instant) -> bool {
        let capture = self.mouse_enabled;
        let Some(doctor) = self.doctor.as_mut() else {
            return false;
        };
        let CapsQuery::Sent { deadline } = doctor.caps_query else {
            return false;
        };
        if now < deadline {
            return false;
        }
        doctor.caps.time_out();
        doctor.caps_query = CapsQuery::Ended;
        doctor.finish_rows(capture);
        true
    }

    /// Test only: put `rows` in the open Doctor as its finished row list,
    /// with the probe answered and the query ended, for paint tests that
    /// do not need the probe or the terminal query.
    #[cfg(test)]
    pub(crate) fn show_doctor_rows(&mut self, rows: Vec<Check>) {
        let doctor = self.doctor.as_mut().expect("open Doctor");
        doctor.probe = Some(rows.clone());
        doctor.caps_query = CapsQuery::Ended;
        doctor.caps = TermCaps::not_asked();
        doctor.cursor = doctor.cursor.min(rows.len().saturating_sub(1));
        doctor.checks = Some(rows);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::snapshot::build_workspace_snapshot;
    use crate::testutil::{app_state, instant_ago};
    use crate::tui::doctor::{CheckGroup, CheckStatus, Fix};
    use crate::tui::keys::{InputMode, DOCTOR_PAGE_ROWS};

    /// An app whose Doctor targets `os`.
    fn app_on(os: Os) -> AppState {
        let mut app = app_state(&[]);
        app.doctor_os = os;
        app
    }

    /// An app whose Doctor targets Linux, so the query is sent on every
    /// host the tests run on.
    fn app() -> AppState {
        app_on(Os::Linux)
    }

    fn check(group: CheckGroup, name: &'static str, copy: Option<&str>) -> Check {
        Check {
            group,
            name,
            status: CheckStatus::Warn,
            why: format!("{name} why."),
            fix: Some(Fix {
                hint: Some("hint".into()),
                copy: copy.map(str::to_string),
            }),
        }
    }

    /// Probe rows: `Mouse` (TERMINAL), then three TOOLS rows.
    fn rows() -> Vec<Check> {
        vec![
            check(CheckGroup::Terminal, "Mouse", None),
            check(CheckGroup::Tools, "git", Some("sudo apt install git")),
            check(CheckGroup::Tools, "Editor (e)", None),
            check(CheckGroup::Tools, "curl", Some("sudo apt install curl")),
        ]
    }

    /// Row names of the merged list for [`rows`]: the live wheel test row
    /// and the six capability rows right after `Mouse`.
    const MERGED: [&str; 11] = [
        "Mouse",
        "Mouse wheel (live test)",
        "Terminal (XTVERSION)",
        "SGR mouse (1006)",
        "Bracketed paste (2004)",
        "Synchronized output (2026)",
        "Kitty keyboard (CSI u)",
        "Background (OSC 11)",
        "git",
        "Editor (e)",
        "curl",
    ];

    /// Index of `git` in [`MERGED`].
    const GIT: usize = 8;

    /// Index of the wheel test row in [`MERGED`].
    const WHEEL: usize = 1;

    /// Name of the wheel test row.
    const WHEEL_ROW: &str = "Mouse wheel (live test)";

    /// A full answer to the query, DA1 last.
    fn full_answer() -> Vec<TermReply> {
        vec![
            TermReply::XtVersion("fake-term(1.2)".to_string()),
            TermReply::ModeReport {
                mode: 1006,
                value: 2,
            },
            TermReply::ModeReport {
                mode: 2004,
                value: 2,
            },
            TermReply::ModeReport {
                mode: 2026,
                value: 0,
            },
            TermReply::KittyFlags(1),
            TermReply::Background((0, 0, 0)),
            TermReply::DeviceAttributes,
        ]
    }

    fn names(app: &AppState) -> Vec<&'static str> {
        app.doctor
            .as_ref()
            .and_then(|doctor| doctor.checks.as_ref())
            .map(|checks| checks.iter().map(|check| check.name).collect())
            .unwrap_or_default()
    }

    fn row<'a>(app: &'a AppState, name: &str) -> &'a Check {
        app.doctor.as_ref().unwrap().checks.as_ref().unwrap()[..]
            .iter()
            .find(|check| check.name == name)
            .unwrap_or_else(|| panic!("row {name}"))
    }

    fn caps_query(app: &AppState) -> CapsQuery {
        app.doctor.as_ref().unwrap().caps_query.clone()
    }

    /// The draw path writes the query of the open request now.
    fn send_query(app: &mut AppState) -> u64 {
        let gen = app.doctor_caps_query_to_send().expect("query asked for");
        app.doctor_caps_query_sent(gen, Instant::now());
        gen
    }

    /// Open, answer the probe, send the query, and end it with the full
    /// answer.
    fn opened(app: &mut AppState) {
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        assert!(app.apply_doctor(gen, rows(), false));
        let epoch = send_query(app);
        assert!(app.apply_doctor_term_replies(epoch, full_answer()));
    }

    #[test]
    fn open_shows_checking_and_asks_for_one_probe_and_one_query() {
        let mut app = app();
        let effect = app.dispatch(Action::OpenDoctor);
        assert_eq!(effect, Effect::RunDoctor { gen: 1 });
        assert_eq!(app.input_mode(), InputMode::Doctor);
        let doctor = app.doctor.as_ref().expect("open");
        assert_eq!(doctor.checks, None, "checking");
        assert_eq!(doctor.caps_query, CapsQuery::Requested);
        assert_eq!(doctor.caps, TermCaps::default());
        assert!(app.doctor_awaits(1));
        assert_eq!(app.doctor_caps_query_to_send(), Some(1));
        assert_eq!(app.doctor_caps_remain_ms(Instant::now()), None, "not sent");
    }

    /// Request → sent (once) → replies applied → ended by DA1 → rows, with
    /// the capability rows after `Mouse`.
    #[test]
    fn query_is_sent_once_and_the_da1_reply_ends_it_with_the_answers() {
        let mut app = app();
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        assert!(app.apply_doctor(gen, rows(), false));
        assert_eq!(app.doctor.as_ref().unwrap().checks, None, "query waits");
        let sent_at = Instant::now();
        app.doctor_caps_query_sent(gen, sent_at);
        assert_eq!(
            caps_query(&app),
            CapsQuery::Sent {
                deadline: sent_at + CAPS_QUERY_TIMEOUT
            }
        );
        assert_eq!(app.doctor_caps_query_to_send(), None, "written once");
        assert_eq!(app.doctor_caps_remain_ms(sent_at), Some(200));

        let mut replies = full_answer();
        let da1 = replies.pop().unwrap();
        assert!(!app.apply_doctor_term_replies(gen, replies), "no DA1 yet");
        assert_eq!(app.doctor.as_ref().unwrap().checks, None);
        assert!(app.apply_doctor_term_replies(gen, vec![da1]));
        assert_eq!(caps_query(&app), CapsQuery::Ended);
        assert_eq!(app.doctor_caps_remain_ms(Instant::now()), None);
        assert_eq!(names(&app), MERGED);
        assert_eq!(row(&app, "Terminal (XTVERSION)").why, "fake-term(1.2)");
        assert_eq!(
            row(&app, "Synchronized output (2026)").why,
            "Not supported (optional): a large redraw can flicker."
        );
        assert_eq!(
            row(&app, "Kitty keyboard (CSI u)").why,
            "Supported (flags 1)."
        );
        assert_eq!(row(&app, "Background (OSC 11)").why, "#000000, dark.");
        assert!(
            !app.expire_doctor_caps_query(Instant::now() + CAPS_QUERY_TIMEOUT),
            "an ended query has no deadline"
        );
    }

    /// No DA1 reply: the deadline ends the wait and the rows with no reply
    /// show unknown with the wait in their text. A reply after that is
    /// dropped.
    #[test]
    fn deadline_ends_the_query_with_unknown_rows() {
        let mut app = app();
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        assert!(app.apply_doctor(gen, rows(), false));
        let sent_at = Instant::now();
        app.doctor_caps_query_sent(gen, sent_at);
        let later = sent_at + Duration::from_millis(150);
        assert!(!app.expire_doctor_caps_query(later), "50 ms still to go");
        assert_eq!(app.doctor_caps_remain_ms(later), Some(50));
        assert_eq!(
            app.doctor_caps_remain_ms(later + Duration::from_micros(10)),
            Some(50),
            "rounded up, so the timer does not wake before the deadline"
        );
        assert!(!app.apply_doctor_term_replies(
            gen,
            vec![TermReply::XtVersion("fake-term(1.2)".to_string())]
        ));
        assert!(app.expire_doctor_caps_query(sent_at + CAPS_QUERY_TIMEOUT));
        assert_eq!(caps_query(&app), CapsQuery::Ended);
        assert_eq!(names(&app), MERGED);
        assert_eq!(row(&app, "Terminal (XTVERSION)").status, CheckStatus::Ok);
        for name in &MERGED[3..8] {
            let check = row(&app, name);
            assert_eq!(check.status, CheckStatus::Unknown, "{name}");
            assert!(
                check.why.ends_with("(no answer within 200 ms)"),
                "{name}: {}",
                check.why
            );
        }
        assert!(
            !app.apply_doctor_term_replies(gen, vec![TermReply::DeviceAttributes]),
            "late reply"
        );
        assert_eq!(
            row(&app, "Kitty keyboard (CSI u)").status,
            CheckStatus::Unknown
        );
    }

    /// The head stays checking until the probe answered AND the query
    /// ended, in either order.
    #[test]
    fn rows_wait_for_both_the_probe_and_the_query() {
        // Query first, probe second.
        let mut app = app();
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        let epoch = send_query(&mut app);
        assert!(
            !app.apply_doctor_term_replies(epoch, full_answer()),
            "probe runs"
        );
        assert_eq!(caps_query(&app), CapsQuery::Ended);
        assert_eq!(app.doctor.as_ref().unwrap().checks, None);
        assert!(app.apply_doctor(gen, rows(), false));
        assert_eq!(names(&app), MERGED);

        // Probe first, deadline second.
        let mut app = app_on(Os::Linux);
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        assert!(app.apply_doctor(gen, rows(), true));
        assert_eq!(app.doctor.as_ref().unwrap().checks, None, "not sent yet");
        app.doctor_caps_query_sent(gen, instant_ago(CAPS_QUERY_TIMEOUT));
        assert_eq!(app.doctor.as_ref().unwrap().checks, None, "sent, waiting");
        assert!(app.expire_doctor_caps_query(Instant::now()));
        assert_eq!(names(&app), MERGED);
        assert_eq!(
            row(&app, "Background (OSC 11)")
                .fix
                .as_ref()
                .and_then(|fix| fix.hint.as_deref()),
            Some("Inside tmux, tmux answers for the outer terminal."),
            "in_tmux from the probe answer"
        );
    }

    /// Replies for an older request, before the query is written, or
    /// after the Doctor closed are dropped.
    #[test]
    fn stale_replies_are_dropped() {
        let mut app = app();
        let Effect::RunDoctor { gen: first } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        send_query(&mut app);
        // `r` before the first query ended: a new request asks again.
        let Effect::RunDoctor { gen } = app.dispatch(Action::DoctorRerun) else {
            panic!("re-run asks for a probe");
        };
        assert_eq!(caps_query(&app), CapsQuery::Requested);
        assert_eq!(app.doctor.as_ref().unwrap().caps, TermCaps::default());
        assert!(
            !app.apply_doctor_term_replies(first, full_answer()),
            "older request"
        );
        assert!(
            !app.apply_doctor_term_replies(gen, full_answer()),
            "not written yet"
        );
        assert_eq!(app.doctor.as_ref().unwrap().caps, TermCaps::default());
        assert!(app.apply_doctor(gen, rows(), false));
        send_query(&mut app);
        assert!(app.apply_doctor_term_replies(gen, full_answer()));

        // Closed: a late reply or deadline changes nothing.
        app.dispatch(Action::DoctorRerun);
        let epoch = send_query(&mut app);
        app.dispatch(Action::DoctorClose);
        assert!(!app.apply_doctor_term_replies(epoch, full_answer()));
        assert!(!app.expire_doctor_caps_query(Instant::now() + CAPS_QUERY_TIMEOUT));
        assert_eq!(app.doctor_caps_query_to_send(), None);
        assert!(app.doctor.is_none());
    }

    /// Windows: open and `r` send no query; it ends at once, so the rows
    /// wait for the probe only, and every capability row says it was not
    /// asked.
    #[test]
    fn windows_sends_no_query_and_its_rows_say_not_asked() {
        let mut app = app_on(Os::Windows);
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        assert_eq!(caps_query(&app), CapsQuery::Ended);
        assert_eq!(app.doctor_caps_query_to_send(), None, "nothing to write");
        assert_eq!(app.doctor_caps_remain_ms(Instant::now()), None);
        assert_eq!(app.doctor.as_ref().unwrap().checks, None, "probe runs");
        assert!(app.apply_doctor(gen, rows(), true));
        assert_eq!(names(&app), MERGED);
        for name in &MERGED[2..8] {
            let check = row(&app, name);
            assert_eq!(check.status, CheckStatus::Unknown, "{name}");
            assert_eq!(check.why, "Not asked on Windows.", "{name}");
            assert_eq!(check.fix, None, "{name}: no tmux hint either");
        }

        let Effect::RunDoctor { gen } = app.dispatch(Action::DoctorRerun) else {
            panic!("re-run asks for a probe");
        };
        assert_eq!(caps_query(&app), CapsQuery::Ended, "r asks nothing either");
        assert_eq!(app.doctor_caps_query_to_send(), None);
        assert!(app.apply_doctor(gen, rows(), false));
        assert_eq!(
            row(&app, "Background (OSC 11)").why,
            "Not asked on Windows."
        );
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
        assert!(!app.apply_doctor(1, rows(), false), "superseded");
        assert_eq!(app.doctor.as_ref().unwrap().probe, None);
        assert!(app.apply_doctor(2, rows(), false));
        assert_eq!(
            app.doctor.as_ref().unwrap().probe.as_ref().map(Vec::len),
            Some(4)
        );
        assert!(!app.apply_doctor(2, rows(), false), "answered once");

        // Closed: a late answer changes nothing.
        app.dispatch(Action::DoctorRerun);
        app.dispatch(Action::DoctorClose);
        assert!(!app.apply_doctor(3, rows(), false));
        assert!(app.doctor.is_none());
    }

    /// The cursor moves and clamps on the merged list (probe rows plus
    /// capability rows).
    #[test]
    fn keys_move_clamp_and_close() {
        let mut app = app();
        opened(&mut app);
        let last = MERGED.len() - 1;
        let cursor = |app: &AppState| app.doctor.as_ref().unwrap().cursor;
        assert_eq!(app.dispatch(Action::DoctorMove(1)), Effect::None);
        assert_eq!(cursor(&app), 1);
        app.dispatch(Action::DoctorMove(DOCTOR_PAGE_ROWS));
        assert_eq!(cursor(&app), 6);
        app.dispatch(Action::DoctorMove(DOCTOR_PAGE_ROWS));
        assert_eq!(cursor(&app), last, "clamped to the last merged row");
        app.dispatch(Action::DoctorMove(-1));
        assert_eq!(cursor(&app), last - 1);
        app.dispatch(Action::DoctorTop);
        assert_eq!(cursor(&app), 0);
        app.dispatch(Action::DoctorBottom);
        assert_eq!(cursor(&app), last);
        app.dispatch(Action::DoctorMove(-DOCTOR_PAGE_ROWS));
        assert_eq!(cursor(&app), last - 5);
        // A re-run keeps the cursor; an answer with fewer rows clamps it
        // on the merged list.
        app.dispatch(Action::DoctorBottom);
        let Effect::RunDoctor { gen } = app.dispatch(Action::DoctorRerun) else {
            panic!("re-run asks again");
        };
        assert_eq!(cursor(&app), last);
        assert!(app.apply_doctor(gen, rows()[..1].to_vec(), false));
        assert_eq!(cursor(&app), last, "still checking");
        let epoch = send_query(&mut app);
        assert!(app.apply_doctor_term_replies(epoch, full_answer()));
        assert_eq!(
            cursor(&app),
            7,
            "Mouse, the wheel row, and six capability rows"
        );
        assert_eq!(app.dispatch(Action::DoctorClose), Effect::None);
        assert!(app.doctor.is_none());
        assert_ne!(app.input_mode(), InputMode::Doctor);
    }

    #[test]
    fn y_copies_the_focused_fix_line_or_says_there_is_none() {
        let mut app = app();
        opened(&mut app);
        app.dispatch(Action::DoctorMove(GIT as i32));
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
        // A capability row has no copy line.
        app.dispatch(Action::DoctorTop);
        app.dispatch(Action::DoctorMove(2));
        assert_eq!(app.dispatch(Action::DoctorCopy), Effect::None);
        // Still checking: nothing to copy either.
        app.dispatch(Action::DoctorMove(GIT as i32 - 2));
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
        app.apply_doctor(gen, rows(), false);
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

    fn cursor(app: &AppState) -> usize {
        app.doctor.as_ref().unwrap().cursor
    }

    fn wheel_status(app: &AppState) -> CheckStatus {
        row(app, WHEEL_ROW).status
    }

    /// Open: the wheel row waits (unknown). A wheel event turns it ok and
    /// moves the cursor one row (down = next, up = previous); later arrow
    /// keys keep it ok. Each change rebuilds the rows in place: no new
    /// probe or query, same rows apart from the wheel row, cursor kept.
    #[test]
    fn a_wheel_event_turns_the_wheel_row_ok_and_arrows_after_it_keep_it() {
        let mut app = app();
        opened(&mut app);
        assert_eq!(names(&app)[WHEEL], WHEEL_ROW, "right after Mouse");
        assert_eq!(wheel_status(&app), CheckStatus::Unknown);
        assert_eq!(
            row(&app, WHEEL_ROW).why,
            "Scroll the mouse wheel once over this dialog."
        );
        let before = app.doctor.as_ref().unwrap().clone();

        app.dispatch(Action::DoctorMove(3));
        assert_eq!(app.dispatch(Action::DoctorWheel(1)), Effect::None);
        assert_eq!(wheel_status(&app), CheckStatus::Ok);
        assert_eq!(cursor(&app), 4, "wheel down: next row");
        let doctor = app.doctor.as_ref().unwrap();
        assert_eq!(doctor.gen, before.gen, "no new request");
        assert_eq!(doctor.caps_query, CapsQuery::Ended, "no new query");
        assert_eq!(app.doctor_caps_query_to_send(), None);
        let rows = doctor.checks.as_ref().unwrap();
        let old = before.checks.as_ref().unwrap();
        assert_eq!(names(&app), MERGED);
        for (i, (new, old)) in rows.iter().zip(old).enumerate() {
            if i != WHEEL {
                assert_eq!(new, old, "only the wheel row changes");
            }
        }

        app.dispatch(Action::DoctorWheel(-1));
        assert_eq!(cursor(&app), 3, "wheel up: previous row");
        app.dispatch(Action::DoctorWheel(0));
        assert_eq!(cursor(&app), 3, "a horizontal wheel only records");
        assert_eq!(app.dispatch(Action::DoctorArrow(1)), Effect::None);
        assert_eq!(cursor(&app), 4, "arrows still move");
        assert_eq!(wheel_status(&app), CheckStatus::Ok, "sticky");
    }

    /// Up / Down arrows with no wheel event warn with the Terminal.app and
    /// tmux fix; the `set -g mouse on` copy line only inside tmux. A wheel
    /// event after the arrows turns the row ok. `j` / `k` do not count.
    #[test]
    fn arrows_without_a_wheel_event_warn_and_a_wheel_after_them_is_ok() {
        for (in_tmux, line) in [(false, None), (true, Some("set -g mouse on"))] {
            let mut app = app();
            let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
                panic!("open asks for a probe");
            };
            assert!(app.apply_doctor(gen, rows(), in_tmux));
            let epoch = send_query(&mut app);
            assert!(app.apply_doctor_term_replies(epoch, full_answer()));
            app.dispatch(Action::DoctorMove(1));
            app.dispatch(Action::DoctorMove(-1));
            assert_eq!(wheel_status(&app), CheckStatus::Unknown, "j / k");

            app.dispatch(Action::DoctorArrow(1));
            assert_eq!(cursor(&app), 1, "Down moves");
            app.dispatch(Action::DoctorArrow(-1));
            assert_eq!(cursor(&app), 0, "Up moves");
            let check = row(&app, WHEEL_ROW);
            assert_eq!(check.status, CheckStatus::Warn, "tmux {in_tmux}");
            let fix = check.fix.as_ref().expect("fix");
            assert!(
                fix.hint
                    .as_deref()
                    .is_some_and(|hint| hint.contains("Terminal.app: View → Allow Mouse")),
                "{fix:?}"
            );
            assert_eq!(fix.copy.as_deref(), line, "tmux {in_tmux}");
            app.dispatch(Action::DoctorMove(WHEEL as i32));
            assert_eq!(
                app.doctor_copy_text().as_deref(),
                line,
                "y copies the tmux line on the wheel row"
            );

            app.dispatch(Action::DoctorWheel(1));
            assert_eq!(wheel_status(&app), CheckStatus::Ok, "tmux {in_tmux}");
        }
    }

    /// Mouse capture off: the wheel row says it cannot test, whatever came.
    #[test]
    fn capture_off_makes_the_wheel_row_unknown() {
        let mut app = app();
        app.mouse_enabled = false;
        opened(&mut app);
        app.dispatch(Action::DoctorArrow(1));
        let check = row(&app, WHEEL_ROW);
        assert_eq!(check.status, CheckStatus::Unknown);
        assert_eq!(
            check.why,
            "Mouse capture is off, so the wheel cannot be tested. Close the Doctor and press m \
             to turn it on."
        );
    }

    /// An event while the head shows `checking…` is kept and shows when the
    /// rows paint; the result survives `r`; a new open waits again.
    #[test]
    fn the_wheel_test_is_kept_while_checking_and_across_r_but_not_a_new_open() {
        let mut app = app();
        let Effect::RunDoctor { gen } = app.dispatch(Action::OpenDoctor) else {
            panic!("open asks for a probe");
        };
        app.dispatch(Action::DoctorWheel(1));
        assert_eq!(app.doctor.as_ref().unwrap().checks, None, "checking");
        assert_eq!(cursor(&app), 0, "no rows to move on");
        assert!(app.apply_doctor(gen, rows(), false));
        let epoch = send_query(&mut app);
        assert!(app.apply_doctor_term_replies(epoch, full_answer()));
        assert_eq!(
            wheel_status(&app),
            CheckStatus::Ok,
            "recorded while checking"
        );

        let Effect::RunDoctor { gen } = app.dispatch(Action::DoctorRerun) else {
            panic!("re-run asks for a probe");
        };
        app.dispatch(Action::DoctorArrow(1));
        assert!(app.apply_doctor(gen, rows(), false));
        let epoch = send_query(&mut app);
        assert!(app.apply_doctor_term_replies(epoch, full_answer()));
        assert_eq!(wheel_status(&app), CheckStatus::Ok, "survives r");

        app.dispatch(Action::DoctorClose);
        opened(&mut app);
        assert_eq!(wheel_status(&app), CheckStatus::Unknown, "a new open waits");

        // Arrows while checking show as a warning once the rows paint.
        let Effect::RunDoctor { gen } = app.dispatch(Action::DoctorRerun) else {
            panic!("re-run asks for a probe");
        };
        app.dispatch(Action::DoctorArrow(-1));
        assert!(app.apply_doctor(gen, rows(), false));
        let epoch = send_query(&mut app);
        assert!(app.apply_doctor_term_replies(epoch, full_answer()));
        assert_eq!(wheel_status(&app), CheckStatus::Warn);
    }
}
