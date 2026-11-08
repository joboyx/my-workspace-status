//! A worker job that panics still releases what it held.
//!
//! The live loop runs each job through [`run_job`], which catches a panic,
//! so the `JoinSet` result keeps the job id. [`Interpreter::spawn_ready`]
//! records a [`JobRelease`] for each job it spawns. On a panic,
//! [`Interpreter::apply_panic`] frees what that record names through the
//! same release function the job's outcome handler calls, and the status
//! line names the job, the panic message, and (while the TUI owns the
//! terminal) where it panicked. The TUI panic hook does not print a worker
//! panic then: the text would land in the alternate screen.
//!
//! A lookup whose answer a view waits on (a cache entry, a loading pane, a
//! request id) gets a failed answer through its outcome handler. Without
//! one, the follow-ups after the panic would see no answer and queue the
//! same lookup again at once.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::time::Instant;

use tokio::task::JoinError;

use super::{ExplorerJob, ExplorerPreviewBody, Interpreter, JobOutcome, JobWork, PrDetailJob};
use crate::file_index::FileRead;
use crate::tui::app::TuiOpts;
use crate::tui::diff::DiffContent;
use crate::tui::drill::CommitFileSource;
use crate::tui::line_blame::BlameKey;
use crate::tui::ops::{RepoOpResult, RunningOp};
use crate::tui::pr_groups::TitleCache;
use crate::tui::pull_request::PrDetailLookup;
use crate::tui::scheduler::UserTag;
use crate::tui::state::AppState;
use crate::tui::status::StatusMessage;

/// What a worker sends back: the outcome, or the panic message of a job
/// that panicked.
pub(crate) type JobResult = Result<JobOutcome, String>;

thread_local! {
    /// True on a worker thread while [`run_job`] runs a job there.
    static IN_WORKER_JOB: Cell<bool> = const { Cell::new(false) };
    /// Where the job on this worker panicked (`file:line:col`), noted by
    /// the TUI panic hook ([`note_worker_panic_at`]) when it does not print
    /// the panic.
    static WORKER_PANIC_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Run `work` on this worker and catch a panic, so the caller still knows
/// which job ended. `Err` holds the panic message, then ` at <location>`
/// when the panic hook noted one.
pub(crate) fn run_job(work: JobWork) -> JobResult {
    WORKER_PANIC_AT.with(|at| at.borrow_mut().take());
    IN_WORKER_JOB.with(|flag| flag.set(true));
    // A job shares only atomics with the loop thread. A panicked job's
    // captures drop with it and no outcome of it is applied.
    let result = std::panic::catch_unwind(AssertUnwindSafe(work));
    IN_WORKER_JOB.with(|flag| flag.set(false));
    result.map_err(|payload| {
        let text = panic_text(&*payload);
        match WORKER_PANIC_AT.with(|at| at.borrow_mut().take()) {
            Some(at) => format!("{text} at {at}"),
            None => text,
        }
    })
}

/// True while this thread runs a job under [`run_job`]: a panic here is
/// caught and reaches the status line.
pub(crate) fn in_worker_job() -> bool {
    IN_WORKER_JOB.with(Cell::get)
}

/// Note where the job on this worker panicked. [`run_job`] adds it to the
/// panic message, so the status line shows what the panic hook did not
/// print.
pub(crate) fn note_worker_panic_at(location: String) {
    WORKER_PANIC_AT.with(|at| *at.borrow_mut() = Some(location));
}

/// The message of a panic payload (`panic!` gives `&str` or `String`).
fn panic_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no panic message".to_string())
}

/// What one job on a worker holds, recorded when it spawns.
///
/// Each variant names a job kind and the keys its release needs; the
/// match in [`Interpreter::release_panicked`] has one arm per variant, so
/// a new kind must say what it releases.
#[derive(Clone, Debug)]
pub(super) enum JobRelease {
    /// Workspace walk of collect `gen`.
    Discover { gen: u64 },
    /// `git status` of checkout `path` for collect `gen`.
    RepoStatus { gen: u64, path: String },
    /// Right-pane load of request `req_id`.
    RightPane { req_id: u64 },
    /// Exclusive write `op` (stage, checkout, merge, …) and its gitdirs.
    Write { op: &'static str },
    /// Fetch, pull, or push of checkout `repo`: its gitdir and its wave.
    Remote { kind: RunningOp, repo: String },
    /// Default-branch switch of checkout `repo`, one of a batch.
    DefaultBranch { repo: String },
    /// Prepare `what` on the [`UserTag::Prepare`] slot (stash menu, push
    /// plan, branch pickers, blame previous change).
    Prepare { what: &'static str },
    /// Compare picker rows of request `gen` for checkout `repo`, on the
    /// [`UserTag::Prepare`] slot.
    ComparePicker { gen: u64, repo: String },
    /// `E` blob and temp-file prepare.
    DiffPrepare,
    /// Older graph page of autoload `gen`.
    Autoload { gen: u64 },
    /// Compare endpoints and file list of compare tab `tab_id`, load `gen`.
    CompareRange { tab_id: u64, gen: u64 },
    /// Compare diff of `path` from `source` for request `req_id` of tab
    /// `tab_id`, load `gen`.
    CompareDiff {
        tab_id: u64,
        req_id: u64,
        gen: u64,
        source: CommitFileSource,
        path: String,
    },
    /// Watch probe of compare tab `tab_id`.
    CompareProbe { tab_id: u64 },
    /// Folder listing `rel_dir` of request `req` in Explorer tab `tab_id`.
    ExplorerDir {
        tab_id: u64,
        req: u64,
        rel_dir: String,
    },
    /// Explorer preview `gen` of tab `tab_id`: a worktree diff when `diff`,
    /// else a file body.
    ExplorerPreview { tab_id: u64, gen: u64, diff: bool },
    /// Search-in-files preview body of preview `gen`.
    SearchPreview { gen: u64 },
    /// Blame of the focused line `key` (serial latest-only slot).
    LineBlame { key: BlameKey },
    /// Commit popover card of `sha` in checkout `repo` (serial latest-only
    /// slot).
    CommitCard { repo: String, sha: String },
    /// Doctor probe (serial latest-only slot).
    Doctor,
    /// PRs tab title fill (one fill at a time).
    PrTitles,
    /// Host clipboard tool copy (serial latest-only slot). `pending` when
    /// OSC 52 failed, so the copy result still waits on this job;
    /// `announce` as in [`super::Effect::CopyClipboard`].
    Clipboard { pending: bool, announce: bool },
    /// PR popover detail fetch `job`, on a badge slot.
    PrDetail { job: PrDetailJob },
    /// Badge lookup of `branch` in checkout `repo`, on a badge slot.
    PrBadge { repo: PathBuf, branch: String },
    /// Job `what` that holds nothing past its Scheduler entry: a read whose
    /// latest-only slot frees at spawn, a queued one-off, or a browser open.
    Read { what: &'static str },
}

impl JobRelease {
    /// Name of the job kind for the status line.
    fn label(&self) -> &'static str {
        match self {
            Self::Discover { .. } => "workspace scan",
            Self::RepoStatus { .. } => "git status",
            Self::RightPane { .. } => "pane load",
            Self::Write { op } => op,
            Self::Remote { kind, .. } => match kind {
                RunningOp::Fetch => "fetch",
                RunningOp::Pull => "pull",
                RunningOp::Push => "push",
                RunningOp::DefaultBranch => "default-branch switch",
            },
            Self::DefaultBranch { .. } => "default-branch switch",
            Self::Prepare { what } | Self::Read { what } => what,
            Self::ComparePicker { .. } => "compare picker",
            Self::DiffPrepare => "external diff prepare",
            Self::Autoload { .. } => "older graph page",
            Self::CompareRange { .. } => "compare range",
            Self::CompareDiff { .. } => "compare diff",
            Self::CompareProbe { .. } => "compare probe",
            Self::ExplorerDir { .. } => "Explorer folder load",
            Self::ExplorerPreview { .. } => "Explorer preview",
            Self::SearchPreview { .. } => "search preview",
            Self::LineBlame { .. } => "line blame",
            Self::CommitCard { .. } => "commit card",
            Self::Doctor => "Doctor probe",
            Self::PrTitles => "PR title fill",
            Self::Clipboard { .. } => "clipboard copy",
            Self::PrDetail { .. } => "PR detail",
            Self::PrBadge { .. } => "PR badge lookup",
        }
    }
}

impl ExplorerJob {
    /// The record of this job: the request its outcome answers.
    pub(super) fn release(&self) -> JobRelease {
        match self {
            Self::Dir {
                tab_id,
                req,
                rel_dir,
                ..
            } => JobRelease::ExplorerDir {
                tab_id: *tab_id,
                req: *req,
                rel_dir: rel_dir.clone(),
            },
            Self::Preview {
                tab_id,
                gen,
                change,
                ..
            } => JobRelease::ExplorerPreview {
                tab_id: *tab_id,
                gen: *gen,
                diff: change.is_some(),
            },
        }
    }
}

impl Interpreter {
    /// End job `id` on the loop thread: drop its Scheduler entry and
    /// return what it held. The outcome path and the panic path both start
    /// here.
    pub(super) fn finish_job(&mut self, id: u64) -> Option<JobRelease> {
        self.sched.note_job_finished(id);
        self.job_releases.remove(&id)
    }

    /// Apply a job that panicked on its worker with `message`: release
    /// what it held, say so on the status line, then run the follow-ups
    /// [`Self::apply`] runs. The next poll, tick, or key starts that kind
    /// of job again.
    pub(crate) fn apply_panic(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        id: u64,
        message: &str,
    ) {
        let label = match self.finish_job(id) {
            Some(release) => {
                let label = release.label();
                self.release_panicked(state, opts, id, release, message);
                label
            }
            None => "worker",
        };
        state.status = StatusMessage::error(format!("{label} job panicked: {message}"));
        self.mark();
        self.after_change(state);
        self.publish_remote_activity(state);
        self.fill_pr_badge_slots();
    }

    /// Report a worker that ended with no job id (cancelled, or a panic
    /// [`run_job`] did not catch). Nothing can be released for it.
    pub(crate) fn apply_lost_job(&mut self, state: &mut AppState, err: &JoinError) {
        state.status = StatusMessage::error(format!("worker job failed: {err}"));
        self.mark();
    }

    /// Free what panicked job `id` held, by the release its outcome
    /// handler uses. A job with a result to count (remote, default-branch
    /// switch, PR detail) or a view that waits on its answer lands a
    /// failure that names the panic, through that outcome handler.
    ///
    /// Jobs whose failure is only a status line (push plan, blame previous
    /// change, `E` prepare, writes) keep the panic status instead.
    fn release_panicked(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        id: u64,
        release: JobRelease,
        message: &str,
    ) {
        let failed = || format!("panicked: {message}");
        let answer = match release {
            JobRelease::Discover { gen } => {
                let _ = self.sched.abandon_collection(gen);
                None
            }
            JobRelease::RepoStatus { gen, path } => {
                if self.sched.accept_repo_result(gen, &path) {
                    let _ = self.note_repo_status_done(state, gen, &path);
                }
                None
            }
            JobRelease::RightPane { req_id } => {
                let _ = self.sched.accept_pane_result(req_id);
                None
            }
            JobRelease::Write { .. } => {
                self.release_write(state, id);
                None
            }
            JobRelease::Remote { kind, repo } => {
                self.land_remote(state, kind, &repo, RepoOpResult::Failed(failed()));
                None
            }
            JobRelease::DefaultBranch { repo } => {
                self.land_default_branch(state, &repo, RepoOpResult::Failed(failed()));
                None
            }
            JobRelease::Prepare { .. } => {
                self.sched.note_user_done(UserTag::Prepare);
                None
            }
            JobRelease::ComparePicker { gen, repo } => Some(JobOutcome::ComparePicker {
                gen,
                repo,
                result: Err(failed()),
            }),
            JobRelease::DiffPrepare => {
                self.sched.note_user_done(UserTag::DiffPrepare);
                None
            }
            JobRelease::Autoload { gen } => {
                let accepted = self.sched.accept_autoload_result(gen);
                self.finish_autoload(state, accepted);
                None
            }
            JobRelease::CompareRange { tab_id, gen } => Some(JobOutcome::CompareRange {
                tab_id,
                gen,
                result: Err(failed()),
            }),
            JobRelease::CompareDiff {
                tab_id,
                req_id,
                gen,
                source,
                path,
            } => Some(JobOutcome::CompareDiff {
                tab_id,
                req_id,
                gen,
                source,
                path,
                content: Err(failed()),
            }),
            JobRelease::CompareProbe { tab_id } => Some(JobOutcome::CompareProbe {
                tab_id,
                result: Err(failed()),
            }),
            JobRelease::ExplorerDir {
                tab_id,
                req,
                rel_dir,
            } => Some(JobOutcome::ExplorerDir {
                tab_id,
                req,
                rel_dir,
                entries: Err(failed()),
            }),
            JobRelease::ExplorerPreview { tab_id, gen, diff } => {
                let body = if diff {
                    ExplorerPreviewBody::Diff(DiffContent {
                        error: Some(failed()),
                        ..DiffContent::default()
                    })
                } else {
                    ExplorerPreviewBody::File(FileRead::Failed(failed()))
                };
                Some(JobOutcome::ExplorerPreview { tab_id, gen, body })
            }
            JobRelease::SearchPreview { gen } => Some(JobOutcome::SearchPreview {
                gen,
                body: FileRead::Failed(failed()),
            }),
            JobRelease::LineBlame { key } => Some(JobOutcome::LineBlame {
                key,
                result: Err(failed()),
            }),
            JobRelease::CommitCard { repo, sha } => Some(JobOutcome::CommitCard {
                repo,
                sha,
                result: Err(failed()),
            }),
            JobRelease::Doctor => {
                self.doctor_job.finish(&mut self.sched);
                None
            }
            JobRelease::PrTitles => {
                // A cut-short fill changes nothing on screen and asks for
                // no next fill; the next title cycle may start one.
                let _ = state.apply_pr_titles(TitleCache::default(), true, Instant::now());
                None
            }
            // The outcome frees the slot; a copy still waiting on its
            // result shows `copy failed`.
            JobRelease::Clipboard { pending, announce } => Some(JobOutcome::ClipboardCopied {
                ok: pending.then_some(false),
                announce,
            }),
            JobRelease::PrDetail { job } => {
                self.land_pr_detail(state, opts, job, PrDetailLookup::Failed(Some(failed())));
                None
            }
            JobRelease::PrBadge { repo, branch } => {
                self.release_pr_badge(&repo, &branch);
                state.abandon_pr_lookup(&repo, &branch);
                self.after_pr_badge(state, opts);
                None
            }
            JobRelease::Read { .. } => None,
        };
        // The outcome handler frees the slot and lands the failed answer.
        // It ends the job again; that is a no-op by now.
        if let Some(answer) = answer {
            self.apply_outcome(state, opts, id, answer);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    use super::super::tests::{
        blame_fixture, capture_jobs, compare_source, drain_jobs, explorer_inner, explorer_load,
        fixture_state, gh_approved_pr, gh_merged_pr, gh_no_pr, move_diff_cursor, open_compare_tab,
        opts, peek_commit, pr_app, schedule_effect, search_preview_of, searched_needles,
    };
    use super::super::Interpreter;
    use crate::file_index::FileRead;
    use crate::testutil::panic_hook_lock;
    use crate::tui::action::{Action, Effect};
    use crate::tui::anim::RemoteActivity;
    use crate::tui::event_loop::{apply_joined, spawn_job, JobJoinSet};
    use crate::tui::pr_status::PrState;
    use crate::tui::pull_request::PrPage;
    use crate::tui::state::AppState;
    use crate::tui::status::StatusKind;
    use crate::tui::tabs::{ComparePickerKind, ExplorerPreview};

    const FORGE_REMOTE: &str = "git@github.com:octo/demo.git";

    /// Panic message of every panicked job in these tests.
    const JOB_PANIC: &str = "worker job exploded";

    /// The `gh` stand-in that panics on the worker. A test that uses it
    /// holds [`panic_hook_lock`], so the TUI panic hook tests cannot add a
    /// location to [`JOB_PANIC`] in the status while it runs.
    fn gh_panics(_: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
        panic!("{JOB_PANIC}")
    }

    /// The runtime the live loop runs on (`run_tui`).
    fn live_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("current-thread runtime")
    }

    /// Spawn every ready job onto `join` with the live spawn function.
    /// Returns how many started.
    fn spawn_live(interp: &mut Interpreter, state: &mut AppState, join: &mut JobJoinSet) -> usize {
        let tui_opts = opts(state);
        let mut started = 0;
        interp.spawn_ready(state, &tui_opts, &mut |id, work| {
            started += 1;
            spawn_job(join, id, work);
        });
        started
    }

    /// Wait for every job on `join` and hand each result to the live join
    /// handler.
    async fn join_live(interp: &mut Interpreter, state: &mut AppState, join: &mut JobJoinSet) {
        let tui_opts = opts(state);
        while let Some(joined) = join.join_next().await {
            apply_joined(interp, state, &tui_opts, joined);
        }
    }

    /// `gx` on checkout `app`, branch `feature`.
    fn open_pr() -> Effect {
        Effect::OpenPullRequest {
            repo: PathBuf::from("app"),
            branch: "feature".into(),
            page: PrPage::Pr,
        }
    }

    #[test]
    fn a_panicked_badge_lookup_frees_its_slot_and_the_next_poll_looks_it_up_again() {
        let _serial = panic_hook_lock();
        let (root, mut state) = pr_app("ws-job-release-badge", Some(FORGE_REMOTE));
        let path = Path::new("app");
        let target = (PathBuf::from("app"), "feature".to_string());
        let mut interp = Interpreter::with_cap(8);
        let tui_opts = opts(&state);
        interp.pr_cli = gh_approved_pr;
        interp.queue_due_pr_lookups(&mut state, &tui_opts);
        drain_jobs(&mut interp, &mut state, 10);
        assert_eq!(
            state.pr_badge(path),
            Some(PrState::Ready),
            "the launch lookup landed, so the checkout is polled"
        );

        let later = |secs| Instant::now() + Duration::from_secs(secs);
        live_runtime().block_on(async {
            let mut join = JobJoinSet::new();
            interp.pr_cli = gh_panics;
            interp.pr_poll_tick(&mut state, &tui_opts, later(60));
            assert_eq!(interp.pr_badges, VecDeque::from([target.clone()]));
            assert_eq!(spawn_live(&mut interp, &mut state, &mut join), 1);
            join_live(&mut interp, &mut state, &mut join).await;

            assert_eq!(interp.pr_badges_running, 0, "the badge slot is free");
            assert!(
                !interp.pr_badge_jobs.contains(&target),
                "the checkout is no longer running"
            );
            assert_eq!(
                interp.sched.inflight_count(),
                0,
                "the Scheduler holds no entry for the panicked job"
            );
            assert!(interp.job_releases.is_empty(), "the job record is gone");
            assert_eq!(
                spawn_live(&mut interp, &mut state, &mut join),
                0,
                "no lookup again before the next poll"
            );

            interp.pr_cli = gh_merged_pr;
            interp.pr_poll_tick(&mut state, &tui_opts, later(120));
            assert_eq!(
                spawn_live(&mut interp, &mut state, &mut join),
                1,
                "the next poll looks the checkout up again"
            );
            join_live(&mut interp, &mut state, &mut join).await;
        });
        assert_eq!(
            state.pr_badge(path),
            Some(PrState::Merged),
            "its answer lands"
        );
        assert_eq!(interp.pr_badges_running, 0);
        assert!(interp.job_releases.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn panicked_jobs_free_their_scheduler_slots_so_a_new_job_still_spawns() {
        let _serial = panic_hook_lock();
        let (root, mut state) = pr_app("ws-job-release-cap", Some(FORGE_REMOTE));
        let cap = 2;
        let mut interp = Interpreter::with_cap(cap);
        live_runtime().block_on(async {
            let mut join = JobJoinSet::new();
            interp.pr_cli = gh_panics;
            for _ in 0..cap {
                schedule_effect(&mut interp, &mut state, open_pr(), &Action::OpenPullRequest);
            }
            assert_eq!(
                spawn_live(&mut interp, &mut state, &mut join),
                cap,
                "the lane is full"
            );
            join_live(&mut interp, &mut state, &mut join).await;
            assert!(interp.job_releases.is_empty(), "no job record is left");

            interp.pr_cli = gh_no_pr;
            schedule_effect(&mut interp, &mut state, open_pr(), &Action::OpenPullRequest);
            assert_eq!(
                spawn_live(&mut interp, &mut state, &mut join),
                1,
                "a new job still spawns"
            );
            join_live(&mut interp, &mut state, &mut join).await;
        });
        assert_eq!(state.status, "no PR for feature", "its outcome applies");
        assert!(interp.job_releases.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_worker_panic_names_the_job_and_its_message_in_the_status() {
        let _serial = panic_hook_lock();
        let (root, mut state) = pr_app("ws-job-release-status", Some(FORGE_REMOTE));
        let mut interp = Interpreter::with_cap(4);
        live_runtime().block_on(async {
            let mut join = JobJoinSet::new();
            interp.pr_cli = gh_panics;
            schedule_effect(&mut interp, &mut state, open_pr(), &Action::OpenPullRequest);
            assert_eq!(spawn_live(&mut interp, &mut state, &mut join), 1);
            join_live(&mut interp, &mut state, &mut join).await;
        });
        assert_eq!(state.status, format!("PR open job panicked: {JOB_PANIC}"));
        assert_eq!(state.status.kind(), StatusKind::Error);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Spawn the ready jobs without running them; returns their ids.
    fn spawned_ids(interp: &mut Interpreter, state: &mut AppState) -> Vec<u64> {
        capture_jobs(interp, state)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    /// Hand job `id` to the live panic path, as [`apply_joined`] does for
    /// a job whose worker panicked with [`JOB_PANIC`].
    fn land_panic(interp: &mut Interpreter, state: &mut AppState, id: u64) {
        let tui_opts = opts(state);
        interp.apply_panic(state, &tui_opts, id, JOB_PANIC);
    }

    /// Spawn the one ready job, `label`, and land it as a panic. The
    /// status names it, its record is gone, and the follow-ups start no
    /// new job: the live loop spawns the ready jobs again at once.
    fn panic_the_one_job(interp: &mut Interpreter, state: &mut AppState, label: &str) {
        let jobs = spawned_ids(interp, state);
        assert_eq!(jobs.len(), 1, "one {label} job");
        land_panic(interp, state, jobs[0]);
        assert_eq!(state.status, format!("{label} job panicked: {JOB_PANIC}"));
        assert!(interp.job_releases.is_empty());
        assert!(
            spawned_ids(interp, state).is_empty(),
            "the {label} does not start again"
        );
    }

    /// The failed answer a panicked job lands.
    fn panicked() -> String {
        format!("panicked: {JOB_PANIC}")
    }

    #[test]
    fn a_panicked_line_blame_caches_no_blame_so_it_does_not_run_again() {
        let (root, mut state) = blame_fixture("ws-job-release-blame");
        let mut interp = Interpreter::with_cap(4);
        move_diff_cursor(&mut interp, &mut state, 2);
        let key = state.line_blame_want().expect("context line asks");
        panic_the_one_job(&mut interp, &mut state, "line blame");
        assert_eq!(
            state.line_blame.cached(&key),
            Some(None),
            "the line is answered with no blame"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_panicked_commit_card_caches_the_failure_so_it_does_not_run_again() {
        let (root, mut state) = blame_fixture("ws-job-release-card");
        let mut interp = Interpreter::with_cap(4);
        let sha = "d".repeat(40);
        peek_commit(&mut state, &sha);
        schedule_effect(&mut interp, &mut state, Effect::None, &Action::None);
        panic_the_one_job(&mut interp, &mut state, "commit card");
        assert_eq!(state.commit_cards.get("app", &sha), Some(&Err(panicked())));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_panicked_compare_load_shows_the_failure_on_its_tab() {
        let tab_error = |state: &AppState| {
            let tab = state.tabs.active_compare().expect("compare tab");
            (tab.error.clone(), tab.loading)
        };
        let range = |tab_id| Effect::LoadCompareRange {
            tab_id,
            repo: "app".into(),
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            force: true,
        };
        let diff = |tab_id| Effect::LoadCompareDiff {
            tab_id,
            repo: "app".into(),
            source: compare_source(),
            path: "a.txt".into(),
            old_path: None,
        };
        let probe = |tab_id| Effect::ProbeCompareTab {
            tab_id,
            repo: "app".into(),
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            last_head: None,
            last_base_tip: None,
        };
        /// The load of compare tab `tab_id` that one case panics.
        type TabLoad = fn(u64) -> Effect;
        let cases: [(&str, TabLoad); 3] = [
            ("compare range", range),
            ("compare diff", diff),
            ("compare probe", probe),
        ];
        for (label, load) in cases {
            let mut state = fixture_state();
            let (tab_id, _) = open_compare_tab(&mut state);
            state.tabs.active_compare_mut().unwrap().source = Some(compare_source());
            let mut interp = Interpreter::with_cap(4);
            schedule_effect(&mut interp, &mut state, load(tab_id), &Action::None);
            panic_the_one_job(&mut interp, &mut state, label);
            let (error, loading) = tab_error(&state);
            assert_eq!(error, Some(panicked()), "{label}");
            if label == "compare range" {
                assert!(!loading, "the range load no longer shows loading");
            }
        }
    }

    #[test]
    fn a_panicked_compare_picker_ends_the_wait_for_its_rows() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        state.compare_picker_pending = Some("app".into());
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::PrepareComparePicker {
                repo: "app".into(),
                kind: ComparePickerKind::Branch,
            },
            &Action::None,
        );
        panic_the_one_job(&mut interp, &mut state, "compare picker");
        assert_eq!(
            state.compare_picker_pending, None,
            "the picker no longer waits"
        );
    }

    #[test]
    fn a_panicked_explorer_load_answers_the_folder_and_the_preview() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let (tab_id, gen) = explorer_inner(&mut state, "app", "a.txt", false, false);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(tab_id, gen, "a.txt"),
            &Action::None,
        );
        panic_the_one_job(&mut interp, &mut state, "Explorer preview");
        let tab = state.tabs.active_explorer().expect("explorer tab");
        match &tab.active_inner().expect("inner tab").view {
            ExplorerPreview::File(file) => {
                assert_eq!(file.body.as_deref(), Some(&FileRead::Failed(panicked())));
            }
            view => panic!("expected a file view, got {view:?}"),
        }

        let req = 7;
        let tab = state.tabs.active_explorer_mut().expect("explorer tab");
        tab.dir_reqs.insert(String::new(), req);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LoadExplorerDir {
                tab_id,
                req,
                repo: "app".into(),
                rel_dir: String::new(),
                parent_ignored: false,
            },
            &Action::None,
        );
        panic_the_one_job(&mut interp, &mut state, "Explorer folder load");
        let tab = state.tabs.active_explorer().expect("explorer tab");
        assert!(tab.dir_reqs.is_empty(), "the folder no longer waits");
    }

    #[test]
    fn a_panicked_search_preview_shows_the_failure_and_does_not_read_again() {
        let (root, mut state, mut interp) = searched_needles("ws-job-release-search-preview");
        let moved = Action::SearchFilesMove(1);
        let effect = state.dispatch(moved.clone());
        schedule_effect(&mut interp, &mut state, effect, &moved);
        let (_, rel) = search_preview_of(&state).expect("preview waits");
        assert_eq!(rel, "b.txt");
        panic_the_one_job(&mut interp, &mut state, "search preview");
        let preview = state
            .search_files
            .as_ref()
            .and_then(|dialog| dialog.preview.as_ref())
            .expect("preview");
        assert_eq!(preview.body, Some(FileRead::Failed(panicked())));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_panicked_workspace_scan_ends_its_collect_so_the_next_watch_tick_scans_again() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::WatchRefresh,
            &Action::WatchTick,
        );
        let scan = spawned_ids(&mut interp, &mut state);
        assert_eq!(scan.len(), 1, "the workspace scan");
        land_panic(&mut interp, &mut state, scan[0]);
        assert!(!interp.sched.collection_active(), "the collect ended");
        assert_eq!(
            state.status,
            format!("workspace scan job panicked: {JOB_PANIC}")
        );

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::WatchRefresh,
            &Action::WatchTick,
        );
        assert_eq!(
            spawned_ids(&mut interp, &mut state).len(),
            1,
            "the next watch tick scans again"
        );
    }

    #[test]
    fn a_panicked_fetch_frees_its_gitdir_and_ends_its_wave() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let fetch = || Effect::Fetch {
            repos: vec!["app".into()],
        };
        schedule_effect(&mut interp, &mut state, fetch(), &Action::Fetch);
        let jobs = spawned_ids(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1);
        assert_eq!(interp.occupied_gitdirs(), ["app".to_string()]);
        land_panic(&mut interp, &mut state, jobs[0]);
        assert!(interp.occupied_gitdirs().is_empty(), "the gitdir is free");
        assert_eq!(
            state.remote_activity,
            RemoteActivity::default(),
            "no fetch runs"
        );

        // The wave end reloads the workspace, as a failed fetch does.
        let _ = spawned_ids(&mut interp, &mut state);
        schedule_effect(&mut interp, &mut state, fetch(), &Action::Fetch);
        assert_eq!(spawned_ids(&mut interp, &mut state).len(), 1);
        assert_eq!(
            interp.occupied_gitdirs(),
            ["app".to_string()],
            "a new fetch of the checkout starts"
        );
    }

    #[test]
    fn a_join_error_with_no_job_id_still_reaches_the_status() {
        let (root, mut state) = pr_app("ws-job-release-join-error", None);
        let mut interp = Interpreter::with_cap(4);
        live_runtime().block_on(async {
            let mut join = JobJoinSet::new();
            join.spawn(std::future::pending());
            join.abort_all();
            join_live(&mut interp, &mut state, &mut join).await;
        });
        assert!(
            state.status.starts_with("worker job failed: "),
            "{:?}",
            state.status
        );
        assert_eq!(state.status.kind(), StatusKind::Error);
        let _ = std::fs::remove_dir_all(root);
    }
}
