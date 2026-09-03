//! Shared Effect interpreter for the live TTY loop and Headless e2e.
//!
//! `Action` still goes through [`AppState::dispatch`](super::state::AppState::dispatch).
//! This module turns [`Effect`] into Scheduler jobs, runs the blocking work,
//! and applies [`JobOutcome`] to [`AppState`].
//!
//! The live loop (`event_loop.rs`) spawns each job on a `JoinSet`. Headless
//! calls [`Interpreter::interpret_sync`], which runs the same schedule / spawn /
//! apply functions on the test thread and drains until idle.
//!
//! Headless does not run [`Effect::EditFile`] or [`Effect::ExternalDiff`].
//! Those arms unmount a TTY `$EDITOR` / diff tool. The live loop consumes
//! [`Interpreter::take_pending_edit`] and [`Interpreter::take_pending_diff`].

use std::collections::{HashMap, VecDeque};

use workspace_status_graph::LOADING_OLDER;

use crate::actions::switch_repo_to_default_branch;
use crate::discovery::{discover_checkouts, process_repo, RepoCheckoutMeta};
use crate::git::{
    create_branch_at, create_branch_checkout, exec_git_checked, latest_stash_ref,
    list_local_branches, pull_quiet_detailed, push_quiet, remove_untracked_file, remove_worktree,
    revert_tracked_file, stage_file, stash_apply, stash_drop, stash_pop, stash_push, unstage_file,
};
use crate::parallel::env_fetch_concurrency;
use crate::snapshot::RepoSnapshot;

use super::action::{Action, Effect, ExternalDiffKind};
use super::app::{
    apply_checkout_compute, apply_merge_compute, apply_one_repo_snapshot, apply_right_pane_load,
    commit_diff_list, compute_checkout, compute_commit_diff, compute_commit_files, compute_merge,
    compute_reload_repo, discover_config, drop_undiscovered_checkouts, filter_repo_set,
    focused_repo_needs_pane, RightPaneLoad, RightPaneRequest, RightPaneTarget, TuiOpts,
};
use super::comments;
use super::drill::{CommitFileSource, DrillView};
use super::event_pump::action_triggers_graph_autoload;
use super::graph_load::{
    autoload_limit, autoload_skip, load_graph_model_window, merge_autoload, should_autoload,
    GraphIdentity, ShouldAutoload,
};
use super::ops::{format_completed_op, format_running_op, RunningOp};
use super::scheduler::{ApplyDecision, Scheduler, SpawnKind, UserTag};
use super::state::AppState;

/// Blocking work that produces one [`JobOutcome`].
pub(crate) type JobWork = Box<dyn FnOnce() -> JobOutcome + Send>;

/// Worker result applied on the loop / Headless thread.
///
/// Autoload, commit-files, commit-diff, and picker outcomes carry a
/// generation plus an immutable target. Autoload identity is the
/// `GraphIdentity` queued at enqueue; the others capture at spawn.
/// Apply drops the result when that gen is stale or the live drill /
/// identity / focused checkout no longer matches.
pub(crate) enum JobOutcome {
    Discovered {
        gen: u64,
        entries: Vec<(String, RepoCheckoutMeta, Option<String>)>,
    },
    RepoStatus {
        gen: u64,
        path: String,
        snap: Option<RepoSnapshot>,
    },
    RightPane {
        req_id: u64,
        target: RightPaneTarget,
        load: super::app::RightPaneLoad,
    },
    Write {
        status: String,
    },
    BulkRemote {
        kind: RunningOp,
        ok: bool,
    },
    DefaultBranch {
        ok: bool,
    },
    PrepareStash {
        gen: u64,
        repo: String,
        latest: Option<String>,
    },
    PrepareBranches {
        gen: u64,
        repo: String,
        branches: Vec<crate::git::LocalBranch>,
        graph_focus: bool,
    },
    Checkout {
        repo: String,
        result: super::app::CheckoutCompute,
    },
    Merge {
        label: String,
        result: super::app::MergeCompute,
    },
    Autoload {
        gen: u64,
        page: workspace_status_graph::GraphModel,
        identity: GraphIdentity,
        prev_status: String,
    },
    CommitFiles {
        gen: u64,
        repo: String,
        source: CommitFileSource,
        files: Vec<crate::git::NameStatus>,
    },
    CommitDiff {
        gen: u64,
        repo: String,
        source: CommitFileSource,
        files: Vec<super::drill::CommitFile>,
        file_cursor: usize,
        path: String,
        content: super::diff::DiffContent,
    },
}

struct BulkState {
    kind: RunningOp,
    remaining: VecDeque<String>,
    inflight: usize,
    done: usize,
    ok: usize,
    failed: usize,
    repos: Vec<String>,
}

struct WriteJob {
    work: Box<dyn FnOnce() -> Result<String, String> + Send>,
}

/// Shared Effect scheduler, spawn, and apply.
///
/// Owns the queues the live `JoinSet` and Headless sync pump drain.
pub(crate) struct Interpreter {
    sched: Scheduler,
    metas: HashMap<String, (RepoCheckoutMeta, Option<String>)>,
    pane_req: Option<RightPaneRequest>,
    writes: VecDeque<WriteJob>,
    bulk: Option<BulkState>,
    default_queue: VecDeque<String>,
    prepare_stash: Option<(u64, String)>,
    prepare_branches: Option<(u64, String, bool)>,
    checkout: Option<(String, String, Option<String>)>,
    merge: Option<(String, String, String)>,
    commit_files: Option<(u64, String, CommitFileSource)>,
    commit_diff: Option<(u64, String, CommitFileSource, String)>,
    autoload: Option<(u64, GraphIdentity)>,
    default_ok: usize,
    default_failed: usize,
    default_total: usize,
    default_repos: Vec<String>,
    pending_edit: Option<(String, String)>,
    pending_diff: Option<(String, String, ExternalDiffKind)>,
    dirty: bool,
}

impl Interpreter {
    /// Empty interpreter with the live fetch/status cap.
    pub(crate) fn new() -> Self {
        Self {
            sched: Scheduler::new(env_fetch_concurrency()),
            metas: HashMap::new(),
            pane_req: None,
            writes: VecDeque::new(),
            bulk: None,
            default_queue: VecDeque::new(),
            prepare_stash: None,
            prepare_branches: None,
            checkout: None,
            merge: None,
            commit_files: None,
            commit_diff: None,
            autoload: None,
            default_ok: 0,
            default_failed: 0,
            default_total: 0,
            default_repos: Vec::new(),
            pending_edit: None,
            pending_diff: None,
            dirty: false,
        }
    }

    /// True when an exclusive write or remote batch is in flight or queued.
    pub(crate) fn busy_for_writes(&self) -> bool {
        self.sched.busy_for_writes()
    }

    /// True when schedule or apply changed state that needs a paint.
    pub(crate) fn take_dirty(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    /// TTY `$EDITOR` request, if [`Effect::EditFile`] ran since the last take.
    pub(crate) fn take_pending_edit(&mut self) -> Option<(String, String)> {
        self.pending_edit.take()
    }

    /// External diff request, if [`Effect::ExternalDiff`] ran since the last take.
    pub(crate) fn take_pending_diff(&mut self) -> Option<(String, String, ExternalDiffKind)> {
        self.pending_diff.take()
    }

    /// Schedule `effect`, then run and apply every job on this thread.
    ///
    /// Headless e2e uses this so tests see the same apply path as the live loop.
    /// [`Effect::EditFile`] and [`Effect::ExternalDiff`] are dropped (no TTY spawn).
    pub(crate) fn interpret_sync(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        effect: Effect,
        action: &Action,
    ) {
        self.schedule(state, opts, effect, action);
        if action_triggers_graph_autoload(action) {
            self.maybe_queue_autoload(state);
        }
        let _ = self.take_pending_edit();
        let _ = self.take_pending_diff();
        self.pump_sync(state, opts);
    }

    /// Enqueue work for `effect`. Does not spawn.
    pub(crate) fn schedule(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        effect: Effect,
        action: &Action,
    ) {
        match effect {
            Effect::None | Effect::Quit => {}
            Effect::Batch(effects) => {
                for child in effects {
                    self.schedule(state, opts, child, action);
                }
            }
            Effect::WatchRefresh => {
                self.sched.on_watch_tick(state.focused_checkout_path());
            }
            Effect::ReloadSnapshot => {
                self.sched.on_reload_snapshot(state.focused_checkout_path());
            }
            Effect::ReloadRepo { repo } => {
                self.sched.on_reload_repo(repo);
            }
            Effect::LoadRightPane => {
                self.pane_req = Some(RightPaneRequest::from_state(state));
                self.sched.request_pane();
            }
            Effect::Fetch { repos } => self.start_bulk(state, RunningOp::Fetch, repos),
            Effect::Pull { repos } => self.start_bulk(state, RunningOp::Pull, repos),
            Effect::Push { repos } => self.start_bulk(state, RunningOp::Push, repos),
            Effect::DefaultBranch { repos } => {
                self.default_total = repos.len();
                self.default_ok = 0;
                self.default_failed = 0;
                self.default_repos = repos.clone();
                state.status = format_running_op(RunningOp::DefaultBranch, 0, repos.len());
                self.mark();
                self.default_queue = repos.into();
                if !self.default_queue.is_empty() {
                    self.sched.enqueue_user(UserTag::DefaultBranch);
                }
            }
            Effect::Stage { repo, paths } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let last = paths.last().cloned().unwrap_or_default();
                Box::new(move || {
                    for path in &paths {
                        stage_file(&dir, path)?;
                    }
                    Ok(format!("staged {last}"))
                })
            }),
            Effect::Unstage { repo, paths } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let last = paths.last().cloned().unwrap_or_default();
                Box::new(move || {
                    for path in &paths {
                        unstage_file(&dir, path)?;
                    }
                    Ok(format!("unstaged {last}"))
                })
            }),
            Effect::Revert {
                repo,
                tracked,
                untracked,
            } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let ok_status = if tracked.len() + untracked.len() == 1 {
                    if untracked.len() == 1 {
                        format!("deleted {}", untracked[0])
                    } else {
                        format!("reverted {}", tracked[0])
                    }
                } else {
                    format!(
                        "reverted {} tracked, {} untracked",
                        tracked.len(),
                        untracked.len()
                    )
                };
                Box::new(move || {
                    for path in &tracked {
                        revert_tracked_file(&dir, path)?;
                    }
                    for path in &untracked {
                        remove_untracked_file(&dir, path)?;
                    }
                    Ok(ok_status)
                })
            }),
            Effect::EditFile { repo, path } => {
                self.pending_edit = Some((repo, path));
                self.mark();
            }
            Effect::ExternalDiff { repo, path, kind } => {
                self.pending_diff = Some((repo, path, kind));
                self.mark();
            }
            Effect::StashCreate { repo, paths } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let ok_status = if paths.len() == 1 {
                    "Stashed 1 file".to_string()
                } else if paths.is_empty() {
                    "Stashed".to_string()
                } else {
                    format!("Stashed {} files", paths.len())
                };
                Box::new(move || stash_push(&dir, &paths).map(|_| ok_status))
            }),
            Effect::StashApply { repo, stash_ref } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let label = stash_ref.clone();
                Box::new(move || stash_apply(&dir, &stash_ref).map(|_| format!("applied {label}")))
            }),
            Effect::StashPop { repo, stash_ref } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let label = stash_ref.clone();
                Box::new(move || stash_pop(&dir, &stash_ref).map(|_| format!("popped {label}")))
            }),
            Effect::StashDrop { repo, stash_ref } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let label = stash_ref.clone();
                Box::new(move || stash_drop(&dir, &stash_ref).map(|_| format!("dropped {label}")))
            }),
            Effect::PrepareStashMenu { repo } => {
                let gen = self.sched.request_prepare_stash();
                self.prepare_stash = Some((gen, repo));
                self.sched.enqueue_user(UserTag::Prepare);
            }
            Effect::PrepareBranchPicker { repo } => {
                let gen = self.sched.request_prepare_branches();
                self.prepare_branches = Some((gen, repo, false));
                self.sched.enqueue_user(UserTag::Prepare);
            }
            Effect::PrepareGraphFocusPicker { repo } => {
                let gen = self.sched.request_prepare_branches();
                self.prepare_branches = Some((gen, repo, true));
                self.sched.enqueue_user(UserTag::Prepare);
            }
            Effect::CheckoutBranch {
                repo,
                selected_name,
                fast_forward_ref,
            } => {
                self.checkout = Some((repo, selected_name, fast_forward_ref));
                self.sched.enqueue_user(UserTag::Write);
            }
            Effect::CreateBranch { repo, name } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let label = name.clone();
                Box::new(move || {
                    create_branch_checkout(&dir, &name).map(|_| format!("created {label}"))
                })
            }),
            Effect::CreateBranchAt {
                repo,
                name,
                commit_id,
            } => self.enqueue_write({
                let dir = opts.cwd.join(&repo);
                let label = name.clone();
                let short = commit_id.get(..7).unwrap_or(&commit_id).to_string();
                Box::new(move || {
                    create_branch_at(&dir, &name, &commit_id)
                        .map(|_| format!("created {label} at {short}"))
                })
            }),
            Effect::MergeIntoHead { repo, rev, label } => {
                self.merge = Some((repo, rev, label));
                self.sched.enqueue_user(UserTag::Write);
            }
            Effect::RemoveWorktree {
                primary,
                path,
                force,
            } => self.enqueue_write({
                let primary_dir = opts.cwd.join(&primary);
                let path_dir = opts.cwd.join(&path);
                let label = path.clone();
                Box::new(move || {
                    remove_worktree(&primary_dir, &path_dir, force)
                        .map(|_| format!("removed worktree {label}"))
                })
            }),
            Effect::LoadCommitFiles { repo, source } => {
                state.begin_commit_files(repo.clone(), source.clone());
                let gen = self.sched.request_commit_files();
                self.commit_files = Some((gen, repo, source));
                self.sched.enqueue_user_front(UserTag::Pane);
                self.mark();
            }
            Effect::LoadCommitDiff { repo, source, path } => {
                let gen = self.sched.request_commit_diff();
                self.commit_diff = Some((gen, repo, source, path));
                self.sched.enqueue_user_front(UserTag::Pane);
            }
            Effect::DropCommitDiff => {
                let _ = self.sched.request_commit_diff();
            }
            Effect::CopyClipboard { text, announce } => {
                let ok = comments::copy_to_clipboard(&text);
                if announce {
                    state.status = if ok {
                        "copied".into()
                    } else {
                        "copy failed".into()
                    };
                }
                self.mark();
            }
        }
    }

    /// Queue graph autoload when the cursor sits on the last loaded row.
    pub(crate) fn maybe_queue_autoload(&mut self, state: &mut AppState) {
        if state.graph_loading_older {
            return;
        }
        if state.right_is_diff() && !state.in_commit_drill() {
            return;
        }
        let Some(model) = state.graph.as_ref() else {
            return;
        };
        if !should_autoload(ShouldAutoload {
            cursor_index: state.graph_cursor,
            loaded_count: model.visible_rows().len(),
            has_more: model.has_more,
            loading: false,
        }) {
            return;
        }
        let Some((repo, head)) = state.graph_identity.as_ref() else {
            return;
        };
        state.graph_loading_older = true;
        let gen = self.sched.request_autoload();
        self.autoload = Some((
            gen,
            GraphIdentity {
                repo: repo.clone(),
                head: head.clone(),
            },
        ));
        state.status = LOADING_OLDER.to_string();
        self.sched.enqueue_user(UserTag::Autoload);
        self.mark();
    }

    /// Spawn every job the Scheduler will issue under the cap.
    pub(crate) fn spawn_ready(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        spawn: &mut dyn FnMut(u64, JobWork),
    ) {
        while let Some(req) = self.sched.spawn_next() {
            let id = req.id;
            match req.kind {
                SpawnKind::Discover { gen, .. } => {
                    let cwd = opts.cwd.clone();
                    let config = discover_config(&opts.config);
                    let filter = state.snapshot.filter_repos.clone();
                    spawn(
                        id,
                        Box::new(move || {
                            let only = filter_repo_set(&filter);
                            let entries = discover_checkouts(&cwd, &config, only.as_ref());
                            JobOutcome::Discovered { gen, entries }
                        }),
                    );
                }
                SpawnKind::ProcessRepo { gen, path, .. } => {
                    let cwd = opts.cwd.clone();
                    let snapshot = state.snapshot.clone();
                    let show_ignored = state.show_ignored;
                    let meta = self.metas.get(&path).cloned().or_else(|| {
                        snapshot.repos.iter().find(|r| r.repo == path).map(|row| {
                            (
                                RepoCheckoutMeta {
                                    checkout_kind: row.checkout_kind,
                                    primary_repo: row.primary_repo.clone(),
                                },
                                row.default_branch_override.clone(),
                            )
                        })
                    });
                    spawn(
                        id,
                        Box::new(move || {
                            let snap = if let Some((meta, override_name)) = meta {
                                process_repo(&path, &cwd, false, override_name.as_deref(), &meta)
                            } else {
                                let next =
                                    compute_reload_repo(&cwd, &snapshot, &path, show_ignored);
                                next.repos
                                    .into_iter()
                                    .find(|row| row.repo == path)
                                    .map(|row| RepoSnapshot {
                                        repo: row.repo,
                                        branch: row.branch,
                                        sync_status: row.sync_status,
                                        sync_note: row.sync_note,
                                        head: row.head,
                                        has_unstaged: row.has_unstaged,
                                        has_staged: row.has_staged,
                                        has_untracked: row.has_untracked,
                                        changes: row.changes,
                                        checkout_kind: row.checkout_kind,
                                        primary_repo: row.primary_repo,
                                        merged_into_default: row.merged_into_default,
                                        default_branch_override: row.default_branch_override,
                                        local_branches: row.local_branches,
                                    })
                            };
                            JobOutcome::RepoStatus { gen, path, snap }
                        }),
                    );
                }
                SpawnKind::LoadPane { req_id } => {
                    let request = self
                        .pane_req
                        .clone()
                        .unwrap_or_else(|| RightPaneRequest::from_state(state));
                    let target = request.target();
                    spawn(
                        id,
                        Box::new(move || {
                            let load = request.compute();
                            JobOutcome::RightPane {
                                req_id,
                                target,
                                load,
                            }
                        }),
                    );
                }
                SpawnKind::UserWork { tag } => self.spawn_user(state, opts, id, tag, spawn),
            }
        }
    }

    /// Apply one worker result. May enqueue follow-up jobs.
    pub(crate) fn apply(
        &mut self,
        state: &mut AppState,
        _opts: &TuiOpts,
        id: u64,
        outcome: JobOutcome,
    ) {
        self.sched.note_job_finished(id);
        match outcome {
            JobOutcome::Discovered { gen, entries } => {
                if self
                    .sched
                    .on_discovered(gen, entries.iter().map(|(p, _, _)| p.clone()).collect())
                    == ApplyDecision::Ignore
                {
                    return;
                }
                let keep: Vec<String> = entries.iter().map(|(p, _, _)| p.clone()).collect();
                drop_undiscovered_checkouts(state, &keep);
                self.metas = entries
                    .into_iter()
                    .map(|(path, meta, ov)| (path, (meta, ov)))
                    .collect();
                self.mark();
            }
            JobOutcome::RepoStatus { gen, path, snap } => {
                if !self.sched.accept_repo_result(gen, &path) {
                    return;
                }
                let before_sigs = state.signatures.clone();
                let before_snap = state.snapshot.clone();
                let focused = state.focused_checkout_path();
                apply_one_repo_snapshot(state, &path, snap);
                let decision = self.sched.note_repo_done(gen, &path);
                if focused.as_deref() == Some(path.as_str())
                    && focused_repo_needs_pane(&before_sigs, &before_snap, state, &path)
                {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
                if let ApplyDecision::StartDiscover { .. } = decision {
                    // latched collect already queued
                }
                self.mark();
            }
            JobOutcome::RightPane {
                req_id,
                target,
                load,
            } => {
                let accepted = self.sched.accept_pane_result(req_id);
                let current = RightPaneRequest::from_state(state).target();
                if accepted && target == current {
                    if matches!(&load, RightPaneLoad::Graph { .. }) {
                        let _ = self.sched.request_autoload();
                        state.graph_loading_older = false;
                    }
                    apply_right_pane_load(state, load);
                    self.mark();
                } else if current != target {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
            }
            JobOutcome::Write { status } => {
                self.sched.note_user_done(UserTag::Write);
                state.status = status;
                self.sched.on_reload_snapshot(state.focused_checkout_path());
                self.pane_req = Some(RightPaneRequest::from_state(state));
                self.sched.request_pane();
                self.mark();
            }
            JobOutcome::BulkRemote { kind, ok } => {
                let finished = if let Some(bulk) = self.bulk.as_mut() {
                    bulk.inflight = bulk.inflight.saturating_sub(1);
                    bulk.done += 1;
                    if ok {
                        bulk.ok += 1;
                    } else {
                        bulk.failed += 1;
                    }
                    state.status = format_running_op(kind, bulk.done, bulk.repos.len());
                    bulk.remaining.is_empty() && bulk.inflight == 0
                } else {
                    false
                };
                self.mark();
                if finished {
                    let Some(bulk) = self.bulk.take() else {
                        return;
                    };
                    state.stamp_checkout_flashes(&bulk.repos);
                    state.status = format_completed_op(kind, bulk.ok, bulk.failed);
                    self.sched.on_reload_snapshot(state.focused_checkout_path());
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
            }
            JobOutcome::DefaultBranch { ok } => {
                self.sched.note_user_done(UserTag::DefaultBranch);
                if ok {
                    self.default_ok += 1;
                } else {
                    self.default_failed += 1;
                }
                let done = self.default_ok + self.default_failed;
                state.status =
                    format_running_op(RunningOp::DefaultBranch, done, self.default_total);
                if !self.default_queue.is_empty() {
                    self.sched.enqueue_user(UserTag::DefaultBranch);
                } else {
                    state.stamp_checkout_flashes(&self.default_repos);
                    state.status = format_completed_op(
                        RunningOp::DefaultBranch,
                        self.default_ok,
                        self.default_failed,
                    );
                    self.default_repos.clear();
                    self.sched.on_reload_snapshot(state.focused_checkout_path());
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
                self.mark();
            }
            JobOutcome::PrepareStash { gen, repo, latest } => {
                self.sched.note_user_done(UserTag::Prepare);
                let accepted = self.sched.accept_prepare_stash_result(gen);
                let current = state.focused_checkout_path();
                if accepted && current.as_deref() == Some(repo.as_str()) {
                    state.open_stash_menu(repo, latest);
                    self.mark();
                }
            }
            JobOutcome::PrepareBranches {
                gen,
                repo,
                branches,
                graph_focus,
            } => {
                self.sched.note_user_done(UserTag::Prepare);
                let accepted = self.sched.accept_prepare_branches_result(gen);
                let current = if graph_focus {
                    state
                        .graph_identity
                        .as_ref()
                        .map(|(r, _)| r.clone())
                        .or_else(|| state.focused_graph_repo())
                } else {
                    state.focused_checkout_path()
                };
                if accepted && current.as_deref() == Some(repo.as_str()) {
                    if graph_focus {
                        state.open_graph_focus_picker(repo, branches);
                    } else {
                        state.open_branch_picker(repo, branches);
                    }
                    self.mark();
                }
            }
            JobOutcome::Checkout { repo, result } => {
                self.sched.note_user_done(UserTag::Write);
                if apply_checkout_compute(state, repo, result) {
                    self.sched.on_reload_snapshot(state.focused_checkout_path());
                }
                self.pane_req = Some(RightPaneRequest::from_state(state));
                self.sched.request_pane();
                self.mark();
            }
            JobOutcome::Merge { label, result } => {
                self.sched.note_user_done(UserTag::Write);
                if apply_merge_compute(state, &label, result) {
                    self.sched.on_reload_snapshot(state.focused_checkout_path());
                }
                self.pane_req = Some(RightPaneRequest::from_state(state));
                self.sched.request_pane();
                self.mark();
            }
            JobOutcome::Autoload {
                gen,
                page,
                identity,
                prev_status,
            } => {
                self.sched.note_user_done(UserTag::Autoload);
                let accepted = self.sched.accept_autoload_result(gen);
                let target_ok = state
                    .graph_identity
                    .as_ref()
                    .is_some_and(|(repo, head)| repo == &identity.repo && head == &identity.head);
                if accepted && target_ok {
                    if let Some(current) = state.graph.clone() {
                        let merged = merge_autoload(&current, page);
                        state.set_graph(merged, identity.repo, identity.head);
                    }
                }
                if accepted {
                    state.graph_loading_older = false;
                    if state.status == LOADING_OLDER {
                        state.status = prev_status;
                    }
                    self.mark();
                }
            }
            JobOutcome::CommitFiles {
                gen,
                repo,
                source,
                files,
            } => {
                let accepted = self.sched.accept_commit_files_result(gen);
                let current = match &state.drill {
                    DrillView::Files {
                        repo: live_repo,
                        source: live_source,
                        ..
                    } => live_repo == &repo && live_source == &source,
                    _ => false,
                };
                if accepted && current {
                    state.open_commit_files(
                        repo,
                        source,
                        files.into_iter().map(Into::into).collect(),
                    );
                    self.mark();
                }
            }
            JobOutcome::CommitDiff {
                gen,
                repo,
                source,
                files,
                file_cursor,
                path,
                content,
            } => {
                let accepted = self.sched.accept_commit_diff_result(gen);
                let current = match &state.drill {
                    DrillView::Diff {
                        repo: live_repo,
                        source: live_source,
                        path: live_path,
                        ..
                    } => live_repo == &repo && live_source == &source && live_path == &path,
                    DrillView::Files {
                        repo: live_repo,
                        source: live_source,
                        ..
                    } => live_repo == &repo && live_source == &source,
                    DrillView::Graph => false,
                };
                if accepted && current {
                    state.open_commit_diff(repo, source, files, file_cursor, path, content);
                    self.mark();
                }
            }
        }
    }

    /// Reload a checkout after a TTY editor returns.
    pub(crate) fn after_edit(&mut self, state: &mut AppState, repo: String) {
        self.sched.on_reload_repo(repo);
        self.pane_req = Some(RightPaneRequest::from_state(state));
        self.sched.request_pane();
        self.mark();
    }

    fn mark(&mut self) {
        self.dirty = true;
    }

    fn enqueue_write(&mut self, work: Box<dyn FnOnce() -> Result<String, String> + Send>) {
        let _ = self.sched.bump_write_gen();
        self.writes.push_back(WriteJob { work });
        self.sched.enqueue_user(UserTag::Write);
    }

    fn start_bulk(&mut self, state: &mut AppState, kind: RunningOp, repos: Vec<String>) {
        if repos.is_empty() {
            return;
        }
        state.status = format_running_op(kind, 0, repos.len());
        self.mark();
        let n = repos.len();
        self.bulk = Some(BulkState {
            kind,
            remaining: repos.iter().cloned().collect(),
            inflight: 0,
            done: 0,
            ok: 0,
            failed: 0,
            repos,
        });
        for _ in 0..n {
            self.sched.enqueue_user(UserTag::BulkRemote);
        }
    }

    fn spawn_user(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        id: u64,
        tag: UserTag,
        spawn: &mut dyn FnMut(u64, JobWork),
    ) {
        match tag {
            UserTag::Write => {
                if let Some((repo, name, ff)) = self.checkout.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        Box::new(move || {
                            let result = compute_checkout(&dir, &name, ff.as_deref());
                            JobOutcome::Checkout { repo, result }
                        }),
                    );
                    return;
                }
                if let Some((repo, rev, label)) = self.merge.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        Box::new(move || {
                            let result = compute_merge(&dir, &rev);
                            JobOutcome::Merge { label, result }
                        }),
                    );
                    return;
                }
                if let Some(job) = self.writes.pop_front() {
                    spawn(
                        id,
                        Box::new(move || {
                            let status = match (job.work)() {
                                Ok(s) => s,
                                Err(err) => err,
                            };
                            JobOutcome::Write { status }
                        }),
                    );
                    return;
                }
                self.sched.note_job_finished(id);
                self.sched.note_user_done(UserTag::Write);
            }
            UserTag::BulkRemote => {
                let Some(bulk) = self.bulk.as_mut() else {
                    self.sched.note_job_finished(id);
                    return;
                };
                let Some(repo) = bulk.remaining.pop_front() else {
                    self.sched.note_job_finished(id);
                    return;
                };
                bulk.inflight += 1;
                let kind = bulk.kind;
                let dir = opts.cwd.join(&repo);
                spawn(
                    id,
                    Box::new(move || {
                        let ok = match kind {
                            RunningOp::Fetch => {
                                exec_git_checked(&["fetch", "--quiet"], &dir).is_ok()
                            }
                            RunningOp::Pull => pull_quiet_detailed(&dir).ok,
                            RunningOp::Push => push_quiet(&dir).is_ok(),
                            RunningOp::DefaultBranch => false,
                        };
                        JobOutcome::BulkRemote { kind, ok }
                    }),
                );
            }
            UserTag::DefaultBranch => {
                let Some(repo) = self.default_queue.pop_front() else {
                    self.sched.note_job_finished(id);
                    self.sched.note_user_done(UserTag::DefaultBranch);
                    return;
                };
                let task = state
                    .snapshot
                    .repos
                    .iter()
                    .find(|r| r.repo == repo)
                    .map(|snap| (snap.branch.clone(), snap.default_branch_override.clone()));
                let cwd = opts.cwd.clone();
                spawn(
                    id,
                    Box::new(move || {
                        let ok = match task {
                            Some((branch, override_name)) => {
                                switch_repo_to_default_branch(
                                    &repo,
                                    &branch,
                                    &cwd,
                                    override_name.as_deref(),
                                )
                                .0
                            }
                            None => false,
                        };
                        JobOutcome::DefaultBranch { ok }
                    }),
                );
            }
            UserTag::Prepare => {
                if let Some((gen, repo)) = self.prepare_stash.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        Box::new(move || {
                            let latest = latest_stash_ref(&dir);
                            JobOutcome::PrepareStash { gen, repo, latest }
                        }),
                    );
                    return;
                }
                if let Some((gen, repo, graph_focus)) = self.prepare_branches.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        Box::new(move || {
                            let branches = list_local_branches(&dir);
                            JobOutcome::PrepareBranches {
                                gen,
                                repo,
                                branches,
                                graph_focus,
                            }
                        }),
                    );
                    return;
                }
                self.sched.note_job_finished(id);
                self.sched.note_user_done(UserTag::Prepare);
            }
            UserTag::Pane => {
                if let Some((gen, repo, source)) = self.commit_files.take() {
                    let dir = opts.cwd.join(&repo);
                    let source_work = source.clone();
                    spawn(
                        id,
                        Box::new(move || {
                            let files = compute_commit_files(&dir, &source_work);
                            JobOutcome::CommitFiles {
                                gen,
                                repo,
                                source,
                                files,
                            }
                        }),
                    );
                    return;
                }
                if let Some((gen, repo, source, path)) = self.commit_diff.take() {
                    let context = state.commit_diff_context(&repo, &path);
                    let focused = state.focused_file();
                    let (files, file_cursor) = commit_diff_list(state);
                    let cwd = opts.cwd.clone();
                    let repo_w = repo.clone();
                    let source_w = source.clone();
                    let path_w = path.clone();
                    spawn(
                        id,
                        Box::new(move || {
                            let content = compute_commit_diff(
                                &cwd,
                                &repo_w,
                                &source_w,
                                &path_w,
                                context,
                                focused.as_ref(),
                            );
                            JobOutcome::CommitDiff {
                                gen,
                                repo,
                                source,
                                files,
                                file_cursor,
                                path,
                                content,
                            }
                        }),
                    );
                    return;
                }
                self.sched.note_job_finished(id);
            }
            UserTag::Autoload => {
                let Some((gen, identity)) = self.autoload.take() else {
                    self.sched.note_job_finished(id);
                    state.graph_loading_older = false;
                    return;
                };
                let Some(model) = state.graph.as_ref() else {
                    self.sched.note_job_finished(id);
                    state.graph_loading_older = false;
                    return;
                };
                let skip = autoload_skip(model);
                let limit = autoload_limit(model);
                let cwd = opts.cwd.clone();
                let snapshot = state.snapshot.clone();
                let show_ignored = state.show_ignored;
                let focus = state.graph_focus_revs();
                let prev_status = state.status.clone();
                let repo = identity.repo.clone();
                spawn(
                    id,
                    Box::new(move || {
                        let (page, _loaded) = load_graph_model_window(
                            &cwd,
                            &snapshot,
                            &repo,
                            show_ignored,
                            skip,
                            limit,
                            &focus,
                        );
                        JobOutcome::Autoload {
                            gen,
                            page,
                            identity,
                            prev_status,
                        }
                    }),
                );
            }
        }
    }

    fn pump_sync(&mut self, state: &mut AppState, opts: &TuiOpts) {
        loop {
            let mut batch = Vec::new();
            self.spawn_ready(state, opts, &mut |id, work| {
                batch.push((id, work()));
            });
            if batch.is_empty() {
                break;
            }
            for (id, outcome) in batch {
                self.apply(state, opts, id, outcome);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use workspace_status_graph::{Commit, GraphModel};

    use crate::config::WorkspaceStatusConfig;
    use crate::git::{LocalBranch, NameStatus};
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::tui::app::TuiOpts;
    use crate::tui::diff::DiffContent;
    use crate::tui::drill::{CommitFile, CommitFileSource, DrillView};
    use crate::tui::graph_load::GraphIdentity;
    use crate::tui::state::{AppState, FocusPane};
    use crate::tui::tree::NodeKind;

    use super::*;

    fn repo(name: &str, dirty: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: dirty,
            has_staged: false,
            has_untracked: false,
            changes: if dirty {
                vec![FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                }]
            } else {
                vec![]
            },
            checkout_kind: crate::snapshot::CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
        }
    }

    fn fixture_state() -> AppState {
        let snapshot = build_workspace_snapshot(
            &[repo("app", true), repo("notes", true), repo("lib", true)],
            &["notes".into()],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn opts(state: &AppState) -> TuiOpts {
        TuiOpts {
            cwd: state.cwd.clone(),
            snapshot: state.snapshot.clone(),
            config: WorkspaceStatusConfig::with_defaults(),
            start_fetch: false,
        }
    }

    fn mini_graph(ids: &[&str]) -> GraphModel {
        GraphModel {
            uncommitted: Some(false),
            commits: ids
                .iter()
                .map(|id| Commit {
                    id: (*id).into(),
                    subject: format!("s-{id}"),
                    ..Commit::default()
                })
                .collect(),
            window: ids.len(),
            skip: 0,
            limit: 300,
            ..GraphModel::default()
        }
    }

    fn commit_source() -> CommitFileSource {
        CommitFileSource::Commit {
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        }
    }

    fn name_status(path: &str) -> NameStatus {
        NameStatus {
            status: "M".into(),
            path: path.into(),
            old_path: None,
        }
    }

    fn commit_file(path: &str) -> CommitFile {
        CommitFile {
            status: "M".into(),
            path: path.into(),
            old_path: None,
        }
    }

    fn local_branch(name: &str) -> LocalBranch {
        LocalBranch {
            name: name.into(),
            current: name == "main",
            authordate: 0,
        }
    }

    fn focus_repo(state: &mut AppState, name: &str) {
        let idx = state
            .rows
            .iter()
            .position(|row| {
                matches!(row.kind, NodeKind::Repo | NodeKind::Checkout)
                    && row.repo.as_deref() == Some(name)
            })
            .unwrap_or_else(|| panic!("missing repo row {name}"));
        state.cursor = idx;
    }

    fn apply(interp: &mut Interpreter, state: &mut AppState, outcome: JobOutcome) {
        let opts = opts(state);
        interp.apply(state, &opts, 1, outcome);
    }

    /// Headless must drain the same apply function the live loop uses.
    ///
    /// The remaining gap is TTY `$EDITOR` / external diff
    /// (`Effect::EditFile` → `pending_edit`, `Effect::ExternalDiff` → `pending_diff`).
    /// This fails if Headless grows a second apply match or live stops calling
    /// [`Interpreter::apply`].
    #[test]
    fn headless_and_live_share_apply_path() {
        let effect = include_str!("effect.rs");
        let headless = include_str!("headless.rs");
        let loop_src = include_str!("event_loop.rs");
        let app = include_str!("app.rs");

        assert!(
            effect.contains("pub(crate) fn interpret_sync"),
            "Headless entry must stay on Interpreter::interpret_sync"
        );
        assert!(
            effect.contains("pub(crate) fn apply("),
            "one apply function must exist"
        );
        assert!(
            effect.contains("pub(crate) fn schedule("),
            "one schedule function must exist"
        );
        assert!(
            headless.contains("interpret_sync("),
            "HeadlessTui must call interpret_sync"
        );
        assert!(
            !headless.contains("JoinSet"),
            "Headless stays a sync pump; live owns the JoinSet"
        );
        assert!(
            !headless.contains("apply_headless"),
            "Headless must not keep a second apply_headless path"
        );
        assert!(
            !app.contains("fn apply_headless"),
            "app.rs must not keep apply_headless_effect"
        );
        assert!(
            !app.contains("fn apply_headless_inner"),
            "app.rs must not keep apply_headless_inner"
        );
        assert!(
            loop_src.contains("interp.apply("),
            "live JoinSet completions must call Interpreter::apply"
        );
        assert!(loop_src.contains("JoinSet"), "live TTY must keep JoinSet");
        assert!(
            loop_src.contains("spawn_blocking"),
            "live TTY must spawn_blocking"
        );
        assert!(
            loop_src.contains("take_pending_edit"),
            "live TTY must consume EditFile via take_pending_edit"
        );
        assert!(
            loop_src.contains("take_pending_diff"),
            "live TTY must consume ExternalDiff via take_pending_diff"
        );
        assert!(
            effect.contains("let _ = self.take_pending_edit();"),
            "Headless interpret_sync must drop EditFile (TTY editor only)"
        );
        assert!(
            effect.contains("let _ = self.take_pending_diff();"),
            "Headless interpret_sync must drop ExternalDiff (no TTY spawn)"
        );
        assert!(
            loop_src.contains("fn schedule_diff"),
            "live TTY must spawn the external diff tool"
        );
        assert!(
            !effect.contains(concat!("collect_full_", "snapshot(")),
            "interpreter watch/refresh must stream process_repo"
        );
    }

    #[test]
    fn schedule_external_diff_sets_pending() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        let tui_opts = opts(&state);
        interp.schedule(
            &mut state,
            &tui_opts,
            Effect::ExternalDiff {
                repo: "app".into(),
                path: "README.md".into(),
                kind: ExternalDiffKind::Worktree,
            },
            &Action::ExternalDiff,
        );
        assert_eq!(
            interp.take_pending_diff(),
            Some(("app".into(), "README.md".into(), ExternalDiffKind::Worktree))
        );
    }

    #[test]
    fn interpret_sync_drops_pending_external_diff() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        let tui_opts = opts(&state);
        interp.interpret_sync(
            &mut state,
            &tui_opts,
            Effect::ExternalDiff {
                repo: "app".into(),
                path: "README.md".into(),
                kind: ExternalDiffKind::Rev {
                    left_rev: "HEAD^".into(),
                    right_rev: "HEAD".into(),
                },
            },
            &Action::ExternalDiff,
        );
        assert!(interp.take_pending_diff().is_none());
    }

    #[test]
    fn late_commit_files_after_graph_does_not_reopen_files() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_commit_files();
        state.drill = DrillView::Graph;
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitFiles {
                gen,
                repo: "app".into(),
                source: commit_source(),
                files: vec![name_status("README.md")],
            },
        );
        assert!(
            state.drill.is_graph(),
            "late CommitFiles must not reopen Files after drill=Graph, got {:?}",
            state.drill
        );
    }

    #[test]
    fn matching_commit_files_still_applies() {
        let mut state = fixture_state();
        let source = commit_source();
        state.begin_commit_files("app".into(), source.clone());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_commit_files();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitFiles {
                gen,
                repo: "app".into(),
                source,
                files: vec![name_status("README.md")],
            },
        );
        match &state.drill {
            DrillView::Files { repo, files, .. } => {
                assert_eq!(repo, "app");
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].path, "README.md");
            }
            other => panic!("matching CommitFiles must open Files, got {other:?}"),
        }
    }

    #[test]
    fn late_autoload_wrong_identity_does_not_replace_graph() {
        let mut state = fixture_state();
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("app".into(), "head-app".into()));
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_autoload();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::Autoload {
                gen,
                page: mini_graph(&["zzz"]),
                identity: GraphIdentity {
                    repo: "lib".into(),
                    head: "head-lib".into(),
                },
                prev_status: String::new(),
            },
        );
        assert_eq!(
            state.graph_identity,
            Some(("app".into(), "head-app".into())),
            "late Autoload must not replace live graph_identity"
        );
        let ids: Vec<_> = state
            .graph
            .as_ref()
            .unwrap()
            .commits
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["aaa"],
            "late Autoload must not merge a foreign page"
        );
    }

    #[test]
    fn matching_autoload_still_merges() {
        let mut state = fixture_state();
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("app".into(), "head-app".into()));
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_autoload();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::Autoload {
                gen,
                page: mini_graph(&["bbb"]),
                identity: GraphIdentity {
                    repo: "app".into(),
                    head: "head-app".into(),
                },
                prev_status: String::new(),
            },
        );
        assert_eq!(
            state.graph_identity,
            Some(("app".into(), "head-app".into()))
        );
        let ids: Vec<_> = state
            .graph
            .as_ref()
            .unwrap()
            .commits
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(ids, vec!["aaa", "bbb"]);
    }

    #[test]
    fn late_commit_diff_after_graph_does_not_reopen_diff() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_commit_diff();
        state.drill = DrillView::Graph;
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitDiff {
                gen,
                repo: "app".into(),
                source: commit_source(),
                files: vec![commit_file("README.md")],
                file_cursor: 0,
                path: "README.md".into(),
                content: DiffContent::from_unified("diff --git a/README.md"),
            },
        );
        assert!(
            state.drill.is_graph(),
            "late CommitDiff must not reopen Diff after drill=Graph, got {:?}",
            state.drill
        );
    }

    #[test]
    fn late_commit_diff_after_other_path_does_not_reopen_old_diff() {
        let mut state = fixture_state();
        let source = commit_source();
        state.open_commit_diff(
            "app".into(),
            source.clone(),
            vec![commit_file("README.md"), commit_file("src.rs")],
            1,
            "src.rs".into(),
            DiffContent::from_unified("diff --git a/src.rs"),
        );
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_commit_diff();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitDiff {
                gen,
                repo: "app".into(),
                source,
                files: vec![commit_file("README.md")],
                file_cursor: 0,
                path: "README.md".into(),
                content: DiffContent::from_unified("diff --git a/README.md"),
            },
        );
        match &state.drill {
            DrillView::Diff { path, .. } => {
                assert_eq!(
                    path, "src.rs",
                    "late CommitDiff must not reopen the old path"
                )
            }
            other => panic!("expected Diff for src.rs, got {other:?}"),
        }
    }

    #[test]
    fn matching_commit_diff_still_applies() {
        let mut state = fixture_state();
        let source = commit_source();
        state.open_commit_files("app".into(), source.clone(), vec![commit_file("README.md")]);
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_commit_diff();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitDiff {
                gen,
                repo: "app".into(),
                source,
                files: vec![commit_file("README.md")],
                file_cursor: 0,
                path: "README.md".into(),
                content: DiffContent::from_unified("diff --git a/README.md"),
            },
        );
        match &state.drill {
            DrillView::Diff { path, content, .. } => {
                assert_eq!(path, "README.md");
                assert!(content.unstaged.contains("README.md"));
            }
            other => panic!("matching CommitDiff must open Diff, got {other:?}"),
        }
    }

    #[test]
    fn late_commit_diff_after_esc_diff_to_files_does_not_reopen_diff() {
        let mut state = fixture_state();
        let source = commit_source();
        state.open_commit_files("app".into(), source.clone(), vec![commit_file("README.md")]);
        state.open_commit_diff(
            "app".into(),
            source.clone(),
            vec![commit_file("README.md")],
            0,
            "README.md".into(),
            DiffContent::from_unified("diff --git a/README.md"),
        );
        assert!(state.drill.is_diff());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_commit_diff();
        state.focus = FocusPane::Left;
        let effect = state.dispatch(Action::NavEsc);
        let opts = opts(&state);
        interp.schedule(&mut state, &opts, effect, &Action::NavEsc);
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitDiff {
                gen,
                repo: "app".into(),
                source,
                files: vec![commit_file("README.md")],
                file_cursor: 0,
                path: "README.md".into(),
                content: DiffContent::from_unified("diff --git a/README.md"),
            },
        );
        assert!(
            state.drill.is_files(),
            "late current-gen CommitDiff must not reopen Diff after Esc Diff→Files, got {:?}",
            state.drill
        );
    }

    #[test]
    fn late_prepare_stash_after_repo_change_does_not_open_menu() {
        let mut state = fixture_state();
        focus_repo(&mut state, "lib");
        assert_eq!(state.focused_checkout_path().as_deref(), Some("lib"));
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_stash();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareStash {
                gen,
                repo: "app".into(),
                latest: Some("stash@{0}".into()),
            },
        );
        assert!(
            state.stash_menu.is_none(),
            "late PrepareStash must not open stash menu after leaving app"
        );
        assert!(state.stash_repo.is_none());
    }

    #[test]
    fn matching_prepare_stash_still_opens_menu() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_stash();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareStash {
                gen,
                repo: "app".into(),
                latest: Some("stash@{0}".into()),
            },
        );
        assert!(
            state.stash_menu.is_some(),
            "matching PrepareStash must open stash menu"
        );
        assert_eq!(state.stash_repo.as_deref(), Some("app"));
    }

    #[test]
    fn late_prepare_branches_after_repo_change_does_not_open_picker() {
        let mut state = fixture_state();
        focus_repo(&mut state, "lib");
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareBranches {
                gen,
                repo: "app".into(),
                branches: vec![local_branch("main")],
                graph_focus: false,
            },
        );
        assert!(
            state.branch_picker.is_none(),
            "late PrepareBranches must not open branch picker after leaving app"
        );
    }

    #[test]
    fn late_prepare_graph_focus_after_identity_change_does_not_open_picker() {
        let mut state = fixture_state();
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("lib".into(), "head-lib".into()));
        focus_repo(&mut state, "lib");
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareBranches {
                gen,
                repo: "app".into(),
                branches: vec![local_branch("main")],
                graph_focus: true,
            },
        );
        assert!(
            state.graph_focus_picker.is_none(),
            "late graph-focus PrepareBranches must not open picker after leaving app"
        );
    }

    #[test]
    fn matching_prepare_branches_still_opens_picker() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareBranches {
                gen,
                repo: "app".into(),
                branches: vec![local_branch("main")],
                graph_focus: false,
            },
        );
        let picker = state
            .branch_picker
            .as_ref()
            .expect("matching PrepareBranches must open branch picker");
        assert_eq!(picker.repo, "app");
    }

    #[test]
    fn matching_prepare_graph_focus_still_opens_picker() {
        let mut state = fixture_state();
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("app".into(), "head-app".into()));
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareBranches {
                gen,
                repo: "app".into(),
                branches: vec![local_branch("main")],
                graph_focus: true,
            },
        );
        let picker = state
            .graph_focus_picker
            .as_ref()
            .expect("matching graph-focus PrepareBranches must open picker");
        assert_eq!(picker.repo, "app");
    }

    fn queue_autoload(state: &mut AppState, interp: &mut Interpreter) {
        focus_repo(state, "app");
        state.drill = DrillView::Graph;
        let mut graph = mini_graph(&["aaa"]);
        graph.has_more = true;
        state.graph = Some(graph);
        state.graph_cursor = 10;
        state.graph_identity = Some(("app".into(), "head-app".into()));
        interp.maybe_queue_autoload(state);
        assert!(
            state.graph_loading_older,
            "maybe_queue_autoload must enqueue"
        );
    }

    #[test]
    fn autoload_spawn_keeps_enqueued_identity_when_live_graph_moves() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        queue_autoload(&mut state, &mut interp);
        state.graph_identity = Some(("lib".into(), "head-lib".into()));
        let tui_opts = opts(&state);
        let mut batch = Vec::new();
        interp.spawn_ready(&mut state, &tui_opts, &mut |id, work| {
            batch.push((id, work()));
        });
        let identity = batch.into_iter().find_map(|(_, outcome)| match outcome {
            JobOutcome::Autoload { identity, .. } => Some(identity),
            _ => None,
        });
        let identity = identity.expect("spawned Autoload");
        assert_eq!(
            identity,
            GraphIdentity {
                repo: "app".into(),
                head: "head-app".into(),
            },
            "autoload JobOutcome identity must be the enqueue-time graph, not live recapture"
        );
    }

    #[test]
    fn stale_autoload_gen_does_not_merge_when_identity_still_matches() {
        let mut state = fixture_state();
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("app".into(), "head-app".into()));
        let mut interp = Interpreter::new();
        let old = interp.sched.request_autoload();
        let _latest = interp.sched.request_autoload();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::Autoload {
                gen: old,
                page: mini_graph(&["zzz"]),
                identity: GraphIdentity {
                    repo: "app".into(),
                    head: "head-app".into(),
                },
                prev_status: String::new(),
            },
        );
        assert_eq!(
            state.graph_identity,
            Some(("app".into(), "head-app".into()))
        );
        let ids: Vec<_> = state
            .graph
            .as_ref()
            .unwrap()
            .commits
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["aaa"],
            "stale autoload gen must not merge when live identity still matches"
        );
    }

    #[test]
    fn late_autoload_after_same_identity_pane_graph_does_not_merge() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        state.drill = DrillView::Graph;
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("app".into(), "head-app".into()));
        state.graph_loading_older = true;
        let mut interp = Interpreter::new();
        let autoload_gen = interp.sched.request_autoload();
        let pane_id = interp.sched.request_pane();
        let target = RightPaneRequest::from_state(&state).target();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::RightPane {
                req_id: pane_id,
                target,
                load: RightPaneLoad::Graph {
                    model: mini_graph(&["bbb"]),
                    identity: GraphIdentity {
                        repo: "app".into(),
                        head: "head-app".into(),
                    },
                    files: None,
                },
            },
        );
        apply(
            &mut interp,
            &mut state,
            JobOutcome::Autoload {
                gen: autoload_gen,
                page: mini_graph(&["zzz"]),
                identity: GraphIdentity {
                    repo: "app".into(),
                    head: "head-app".into(),
                },
                prev_status: String::new(),
            },
        );
        assert_eq!(
            state.graph_identity,
            Some(("app".into(), "head-app".into()))
        );
        let ids: Vec<_> = state
            .graph
            .as_ref()
            .unwrap()
            .commits
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["bbb"],
            "late Autoload must not merge the old window into a same-identity pane replace"
        );
        assert!(
            !state.graph_loading_older,
            "pane graph replace must clear graph_loading_older"
        );
    }

    #[test]
    fn stale_commit_files_gen_does_not_fill_when_drill_still_matches() {
        let mut state = fixture_state();
        let source = commit_source();
        state.begin_commit_files("app".into(), source.clone());
        let mut interp = Interpreter::new();
        let old = interp.sched.request_commit_files();
        let _latest = interp.sched.request_commit_files();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitFiles {
                gen: old,
                repo: "app".into(),
                source,
                files: vec![name_status("README.md")],
            },
        );
        match &state.drill {
            DrillView::Files { files, .. } => {
                assert!(
                    files.is_empty(),
                    "stale CommitFiles gen must not fill the list when drill still matches"
                )
            }
            other => panic!("expected Files drill, got {other:?}"),
        }
    }

    #[test]
    fn stale_commit_diff_gen_does_not_open_when_files_drill_still_matches() {
        let mut state = fixture_state();
        let source = commit_source();
        state.open_commit_files("app".into(), source.clone(), vec![commit_file("README.md")]);
        let mut interp = Interpreter::new();
        let old = interp.sched.request_commit_diff();
        let _latest = interp.sched.request_commit_diff();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitDiff {
                gen: old,
                repo: "app".into(),
                source,
                files: vec![commit_file("README.md")],
                file_cursor: 0,
                path: "README.md".into(),
                content: DiffContent::from_unified("diff --git a/README.md"),
            },
        );
        assert!(
            state.drill.is_files(),
            "stale CommitDiff gen must not open Diff when Files target still matches, got {:?}",
            state.drill
        );
    }

    #[test]
    fn stale_prepare_stash_gen_does_not_open_when_repo_still_matches() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::new();
        let old = interp.sched.request_prepare_stash();
        let _latest = interp.sched.request_prepare_stash();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareStash {
                gen: old,
                repo: "app".into(),
                latest: Some("stash@{0}".into()),
            },
        );
        assert!(
            state.stash_menu.is_none(),
            "stale PrepareStash gen must not open the menu when the repo still matches"
        );
    }

    #[test]
    fn stale_prepare_branches_gen_does_not_open_when_repo_still_matches() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::new();
        let old = interp.sched.request_prepare_branches();
        let _latest = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareBranches {
                gen: old,
                repo: "app".into(),
                branches: vec![local_branch("main")],
                graph_focus: false,
            },
        );
        assert!(
            state.branch_picker.is_none(),
            "stale PrepareBranches gen must not open the picker when the repo still matches"
        );
    }

    #[test]
    fn stale_prepare_graph_focus_gen_does_not_open_when_identity_still_matches() {
        let mut state = fixture_state();
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("app".into(), "head-app".into()));
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::new();
        let old = interp.sched.request_prepare_branches();
        let _latest = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareBranches {
                gen: old,
                repo: "app".into(),
                branches: vec![local_branch("main")],
                graph_focus: true,
            },
        );
        assert!(
            state.graph_focus_picker.is_none(),
            "stale graph-focus gen must not open the picker when identity still matches"
        );
    }
}
