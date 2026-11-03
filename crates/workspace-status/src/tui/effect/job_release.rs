//! A worker job that panics still releases what it held.
//!
//! The live loop runs each job through [`run_job`], which catches a panic,
//! so the `JoinSet` result keeps the job id. [`Interpreter::spawn_ready`]
//! records a [`JobRelease`] for each job it spawns. On a panic,
//! [`Interpreter::apply_panic`] frees what that record names through the
//! same release function the job's outcome handler calls, and the status
//! line names the job and the panic message.

use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::time::Instant;

use tokio::task::JoinError;

use super::{Interpreter, JobOutcome, JobWork, PrDetailJob};
use crate::tui::app::TuiOpts;
use crate::tui::ops::{RepoOpResult, RunningOp};
use crate::tui::pr_groups::TitleCache;
use crate::tui::pull_request::PrDetailLookup;
use crate::tui::scheduler::UserTag;
use crate::tui::state::AppState;
use crate::tui::status::StatusMessage;

/// What a worker sends back: the outcome, or the panic message of a job
/// that panicked.
pub(crate) type JobResult = Result<JobOutcome, String>;

/// Run `work` on this worker and catch a panic, so the caller still knows
/// which job ended. `Err` holds the panic message.
pub(crate) fn run_job(work: JobWork) -> JobResult {
    // A job shares only atomics with the loop thread. A panicked job's
    // captures drop with it and no outcome of it is applied.
    std::panic::catch_unwind(AssertUnwindSafe(work)).map_err(|payload| panic_text(&*payload))
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
    /// plan, pickers, blame previous change).
    Prepare { what: &'static str },
    /// `E` blob and temp-file prepare.
    DiffPrepare,
    /// Older graph page of autoload `gen`.
    Autoload { gen: u64 },
    /// Focused-line blame (serial latest-only slot).
    LineBlame,
    /// Commit popover card (serial latest-only slot).
    CommitCard,
    /// Doctor probe (serial latest-only slot).
    Doctor,
    /// PRs tab title fill (one fill at a time).
    PrTitles,
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
            Self::DiffPrepare => "external diff prepare",
            Self::Autoload { .. } => "older graph page",
            Self::LineBlame => "line blame",
            Self::CommitCard => "commit card",
            Self::Doctor => "Doctor probe",
            Self::PrTitles => "PR title fill",
            Self::PrDetail { .. } => "PR detail",
            Self::PrBadge { .. } => "PR badge lookup",
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
    /// switch, PR detail) lands a failure that names the panic.
    fn release_panicked(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        id: u64,
        release: JobRelease,
        message: &str,
    ) {
        let failed = || format!("panicked: {message}");
        match release {
            JobRelease::Discover { gen } => {
                let _ = self.sched.abandon_collection(gen);
            }
            JobRelease::RepoStatus { gen, path } => {
                if self.sched.accept_repo_result(gen, &path) {
                    let _ = self.note_repo_status_done(state, gen, &path);
                }
            }
            JobRelease::RightPane { req_id } => {
                let _ = self.sched.accept_pane_result(req_id);
            }
            JobRelease::Write { .. } => self.release_write(state, id),
            JobRelease::Remote { kind, repo } => {
                self.land_remote(state, kind, &repo, RepoOpResult::Failed(failed()));
            }
            JobRelease::DefaultBranch { repo } => {
                self.land_default_branch(state, &repo, RepoOpResult::Failed(failed()));
            }
            JobRelease::Prepare { .. } => self.sched.note_user_done(UserTag::Prepare),
            JobRelease::DiffPrepare => self.sched.note_user_done(UserTag::DiffPrepare),
            JobRelease::Autoload { gen } => {
                let accepted = self.sched.accept_autoload_result(gen);
                self.finish_autoload(state, accepted);
            }
            JobRelease::LineBlame => self.release_line_blame(),
            JobRelease::CommitCard => self.release_commit_card(),
            JobRelease::Doctor => self.doctor_job.finish(&mut self.sched),
            JobRelease::PrTitles => {
                // A cut-short fill changes nothing on screen and asks for
                // no next fill; the next title cycle may start one.
                let _ = state.apply_pr_titles(TitleCache::default(), true, Instant::now());
            }
            JobRelease::PrDetail { job } => {
                self.land_pr_detail(state, opts, job, PrDetailLookup::Failed(Some(failed())));
            }
            JobRelease::PrBadge { repo, branch } => {
                self.release_pr_badge(&repo, &branch);
                state.abandon_pr_lookup(&repo, &branch);
                self.after_pr_badge(state, opts);
            }
            JobRelease::Read { .. } => {}
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
        capture_jobs, drain_jobs, fixture_state, gh_approved_pr, gh_merged_pr, gh_no_pr, opts,
        pr_app, schedule_effect,
    };
    use super::super::Interpreter;
    use crate::tui::action::{Action, Effect};
    use crate::tui::anim::RemoteActivity;
    use crate::tui::event_loop::{apply_joined, spawn_job, JobJoinSet};
    use crate::tui::pr_status::PrState;
    use crate::tui::pull_request::PrPage;
    use crate::tui::state::AppState;
    use crate::tui::status::StatusKind;

    const FORGE_REMOTE: &str = "git@github.com:octo/demo.git";

    const GH_PANIC: &str = "forge CLI runner exploded";

    fn gh_panics(_: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
        panic!("{GH_PANIC}")
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
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn panicked_jobs_free_their_scheduler_slots_so_a_new_job_still_spawns() {
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
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_worker_panic_names_the_job_and_its_message_in_the_status() {
        let (root, mut state) = pr_app("ws-job-release-status", Some(FORGE_REMOTE));
        let mut interp = Interpreter::with_cap(4);
        live_runtime().block_on(async {
            let mut join = JobJoinSet::new();
            interp.pr_cli = gh_panics;
            schedule_effect(&mut interp, &mut state, open_pr(), &Action::OpenPullRequest);
            assert_eq!(spawn_live(&mut interp, &mut state, &mut join), 1);
            join_live(&mut interp, &mut state, &mut join).await;
        });
        assert_eq!(state.status, format!("PR open job panicked: {GH_PANIC}"));
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
    /// a job whose worker panicked with [`GH_PANIC`].
    fn land_panic(interp: &mut Interpreter, state: &mut AppState, id: u64) {
        let tui_opts = opts(state);
        interp.apply_panic(state, &tui_opts, id, GH_PANIC);
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
            format!("workspace scan job panicked: {GH_PANIC}")
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
