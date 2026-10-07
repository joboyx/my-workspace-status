//! Shared Effect interpreter for the live TTY loop.
//!
//! `Action` still goes through [`AppState::dispatch`](super::state::AppState::dispatch).
//! This module turns [`Effect`] into Scheduler jobs, runs the blocking work,
//! and applies [`JobOutcome`] to [`AppState`].
//!
//! The live loop (`event_loop.rs`) spawns each job on a `JoinSet`.
//! [`Interpreter::interpret_sync`] runs the same schedule / spawn / apply
//! functions on the calling thread and drains until idle. Unit tests use that
//! sync path. It does not run [`Effect::EditFile`] or [`Effect::ExternalDiff`].
//! Those arms unmount a TTY `$EDITOR` / diff tool. The live loop consumes
//! [`Interpreter::take_pending_edit`] and [`Interpreter::take_pending_diff`],
//! then enqueues blob/temp prepare on `spawn_blocking` before spawning the tool.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use workspace_status_graph::LOADING_OLDER;

use crate::actions::{switch_repo_to_default_branch, SwitchOutcome};
use crate::discovery::{discover_checkouts, process_repo, RepoCheckoutMeta};
use crate::file_index::{
    build_file_index, read_text_file, score_files, FileHit, FileIndex, FileRead, IndexRoot,
    MAX_FILE_BYTES, MAX_INDEX_ENTRIES, MAX_RESULTS,
};
use crate::file_search::{build_matcher, search_chunk, SearchChunk, SearchOptions};
use crate::git::{
    apply_cached_patch, apply_worktree_patch_reverse, blame_line, create_branch_at,
    create_branch_checkout, exec_git_checked, latest_stash_ref, list_compare_picker_branches,
    list_compare_picker_commits, list_local_branches, previous_line_change, pull_quiet_detailed,
    push_quiet, remote_url_for_branch, remove_untracked_file, remove_worktree, revert_compare_file,
    revert_compare_patch, revert_tracked_file, stage_file, stash_apply, stash_drop, stash_pop,
    stash_push, unstage_file, COMPARE_REVERT_ABORTED,
};
use crate::snapshot::RepoSnapshot;

use super::action::{Action, Effect, ExternalDiffKind};
use super::app::{
    apply_checkout_compute, apply_merge_compute, apply_one_repo_snapshot, apply_right_pane_load,
    commit_diff_list, compute_checkout, compute_commit_diff, compute_commit_files,
    compute_compare_diff, compute_compare_range, compute_merge, compute_reload_repo,
    compute_worktree_compare_range, discover_config, drop_undiscovered_checkouts, filter_repo_set,
    focused_repo_needs_pane, probe_compare_range, RightPaneLoad, RightPaneRequest, RightPaneTarget,
    TuiOpts,
};
use super::chrome::{is_idle_pull_status, STATUS_COPIED, STATUS_COPY_FAILED};
use super::comments;
use super::diff_tool::{
    prepare_rev_diff_paths, prepare_worktree_diff, resolve_diff_tool, PreparedDiff,
};
use super::drill::{CommitFileSource, DrillView};
#[cfg(test)]
use super::event_pump::action_triggers_graph_autoload;
use super::graph_load::{
    autoload_limit, autoload_skip, load_graph_model_window, merge_autoload, should_autoload,
    GraphIdentity, ShouldAutoload,
};
use super::line_blame::BlameKey;
use super::ops::{
    format_completed_op, format_mixed_running_op, format_running_op, op_targets, Op, OpTally,
    RepoOpResult, RunningOp,
};
use super::pull_request::{lookup, lookup_detail, PrDetailLookup, PrLookup};
#[cfg(not(test))]
use super::pull_request::{open_in_browser, run_cli};
use super::scheduler::{ApplyDecision, Scheduler, SpawnKind, UserTag};
use super::search_files::SEARCH_CHUNK_BUDGET;
use super::stash::{resolve_stash_menu_key, StashMenuKeyResult, StashOpId};
use super::state::{revert_scope, AppState, PendingConfirm};
use super::status::StatusMessage;
use super::tabs::{ComparePickerKind, ComparePickerRows, WorktreeFile};

/// Blocking work that produces one [`JobOutcome`].
pub(crate) type JobWork = Box<dyn FnOnce() -> JobOutcome + Send>;

/// Worker result applied on the loop thread.
///
/// Autoload, commit-files, commit-diff, compare-file-diff, and picker
/// outcomes carry a generation plus an immutable target. Autoload identity
/// is the `GraphIdentity` queued at enqueue; the others capture at spawn.
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
        status: StatusMessage,
    },
    BulkRemote {
        kind: RunningOp,
        result: RepoOpResult,
        /// Checkout path the worker ran in. Occupancy releases this gitdir.
        repo: String,
    },
    DefaultBranch {
        repo: String,
        result: RepoOpResult,
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
    /// Blob bytes + temp files for `E`. Live loop then spawns the tool.
    DiffPrepared {
        repo: String,
        path: String,
        kind: ExternalDiffKind,
        tool: String,
        repo_abs: std::path::PathBuf,
        prepared: Result<PreparedDiff, String>,
    },
    /// Resolved compare endpoints plus the committed file list.
    CompareRange {
        tab_id: u64,
        gen: u64,
        result: Result<super::app::CompareRangeLoad, String>,
    },
    /// One compare-file unified diff.
    CompareDiff {
        tab_id: u64,
        req_id: u64,
        gen: u64,
        source: CommitFileSource,
        path: String,
        content: Result<super::diff::DiffContent, String>,
    },
    /// Compare picker rows: local + `origin/*` names, or HEAD's ancestors.
    ComparePicker {
        gen: u64,
        repo: String,
        result: Result<ComparePickerRows, String>,
    },
    /// Watch probe: whether HEAD or the base tip moved.
    CompareProbe {
        tab_id: u64,
        result: Result<bool, String>,
    },
    /// Quick Open file listing for index generation `gen`.
    FileIndex {
        gen: u64,
        index: FileIndex,
    },
    /// Quick Open ranked hits for score generation `gen`.
    FileScore {
        gen: u64,
        hits: Vec<FileHit>,
    },
    /// Search-in-files file listing for index generation `gen`.
    SearchIndex {
        gen: u64,
        index: FileIndex,
    },
    /// One search-in-files chunk for search generation `gen`.
    SearchChunk {
        gen: u64,
        chunk: SearchChunk,
    },
    /// File tab body for load generation `gen`.
    FileTab {
        tab_id: u64,
        gen: u64,
        body: FileRead,
    },
    /// `git blame -L n,n` answer for one focused line.
    LineBlame {
        key: BlameKey,
        result: Result<Option<crate::git::LineBlame>, String>,
    },
    /// Previous change to a blamed line, for request `gen` on blame
    /// commit `sha`.
    BlamePrevious {
        gen: u64,
        repo: String,
        sha: String,
        result: Result<crate::git::PreviousLineChange, String>,
    },
    /// Badge lookup of `branch` in checkout `repo`. `remote` is the URL the
    /// worker resolved.
    PullRequestBadge {
        repo: PathBuf,
        branch: String,
        remote: Option<String>,
        lookup: PrLookup,
    },
    /// Popover detail fetch of PR `number` for `branch` of checkout `repo`
    /// at `remote`.
    PullRequestDetail {
        job: PrDetailJob,
        lookup: PrDetailLookup,
    },
    /// `gx` lookup of `branch` in checkout `repo`, then the browser open.
    /// `opened` is true when the browser started for a found PR.
    PullRequestOpen {
        repo: PathBuf,
        branch: String,
        remote: Option<String>,
        lookup: PrLookup,
        opened: bool,
    },
}

/// One popover detail fetch: the fields of
/// [`Effect::LookupPullRequestDetail`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrDetailJob {
    repo: PathBuf,
    branch: String,
    remote: String,
    number: u64,
    request: u64,
}

/// Forge CLI runner for PR lookups: argv and the quit flag in, stdout out.
type PrCliRunner = fn(&[String], &AtomicBool) -> Result<String, ()>;

/// Opens a PR URL in the operator's browser; stops waiting on the quit flag.
type PrUrlOpener = fn(&str, &AtomicBool) -> Result<(), ()>;

/// Forge CLI runner a new [`Interpreter`] starts with.
#[cfg(not(test))]
const DEFAULT_PR_CLI: PrCliRunner = run_cli;

/// Test builds never spawn `gh` / `glab`: the default fails without a
/// spawn, and a test that needs an answer sets a fake.
#[cfg(test)]
const DEFAULT_PR_CLI: PrCliRunner = |_, _| Err(());

/// Browser opener a new [`Interpreter`] starts with.
#[cfg(not(test))]
const DEFAULT_PR_OPENER: PrUrlOpener = open_in_browser;

/// Test builds never start a browser: the default fails without a spawn,
/// and a test that needs an open sets a fake.
#[cfg(test)]
const DEFAULT_PR_OPENER: PrUrlOpener = |_, _| Err(());

/// Most badge lookups and popover detail fetches on workers at once, so
/// pane loads are not starved. A `gx` open does not wait for this cap.
const PR_BADGE_LOOKUPS_MAX: usize = 2;

/// Live TTY launch after [`JobOutcome::DiffPrepared`].
pub(crate) struct DiffLaunch {
    pub repo: String,
    pub path: String,
    pub kind: ExternalDiffKind,
    pub tool: String,
    pub repo_abs: std::path::PathBuf,
    pub prepared: Result<PreparedDiff, String>,
}

struct DiffPrepareJob {
    repo: String,
    path: String,
    kind: ExternalDiffKind,
    tool: String,
    repo_abs: std::path::PathBuf,
}

struct RemoteJob {
    kind: RunningOp,
    checkout: String,
}

struct InflightRemote {
    kind: RunningOp,
    checkout: String,
}

enum OccupyReason {
    Remote(InflightRemote),
    Exclusive,
    DefaultBranch,
}

struct KindWave {
    tally: OpTally,
    repos: Vec<String>,
    /// A key (not only the background fetch tick) started this wave.
    /// Background-only waves leave the status slot alone unless a repo fails.
    foreground: bool,
}

impl KindWave {
    fn empty() -> Self {
        Self {
            tally: OpTally::default(),
            repos: Vec::new(),
            foreground: false,
        }
    }

    fn done(&self) -> usize {
        self.tally.done()
    }

    fn note_repo(&mut self, checkout: &str) {
        if !self.repos.iter().any(|row| row == checkout) {
            self.repos.push(checkout.to_string());
        }
    }

    fn remove_repo(&mut self, checkout: &str) {
        self.repos.retain(|row| row != checkout);
    }
}

struct RemoteQueue {
    pending: VecDeque<RemoteJob>,
    occupy: HashMap<String, OccupyReason>,
    fetch: KindWave,
    pull: KindWave,
    push: KindWave,
}

impl RemoteQueue {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            occupy: HashMap::new(),
            fetch: KindWave::empty(),
            pull: KindWave::empty(),
            push: KindWave::empty(),
        }
    }

    fn wave_mut(&mut self, kind: RunningOp) -> Option<&mut KindWave> {
        match kind {
            RunningOp::Fetch => Some(&mut self.fetch),
            RunningOp::Pull => Some(&mut self.pull),
            RunningOp::Push => Some(&mut self.push),
            RunningOp::DefaultBranch => None,
        }
    }

    fn wave(&self, kind: RunningOp) -> Option<&KindWave> {
        match kind {
            RunningOp::Fetch => Some(&self.fetch),
            RunningOp::Pull => Some(&self.pull),
            RunningOp::Push => Some(&self.push),
            RunningOp::DefaultBranch => None,
        }
    }
}

/// Final status of a finished bulk op: error when a repo failed, warn when
/// one was skipped, else ok.
fn completed_op_status(kind: RunningOp, tally: &OpTally) -> StatusMessage {
    let text = format_completed_op(kind, tally);
    if tally.has_failure() {
        StatusMessage::error(text)
    } else if tally.has_skip() {
        StatusMessage::warn(text)
    } else {
        StatusMessage::ok(text)
    }
}

/// Final status of an exclusive write named `op`.
///
/// A compare-tab refusal ([`COMPARE_REVERT_ABORTED`]) keeps its own copy as
/// a warn: git did not fail, the write never ran.
fn write_status(op: &str, result: Result<String, String>) -> StatusMessage {
    match result {
        Ok(text) => StatusMessage::ok(text),
        Err(err) if err.starts_with(COMPARE_REVERT_ABORTED) => StatusMessage::warn(err),
        Err(err) => StatusMessage::error(format!("{op} failed: {err}")),
    }
}

/// Bulk pull result for one repo.
fn pull_result(dir: &std::path::Path) -> RepoOpResult {
    let pulled = pull_quiet_detailed(dir);
    if pulled.stash_pop_failed {
        RepoOpResult::StashConflict
    } else if pulled.ok {
        RepoOpResult::Ok
    } else {
        RepoOpResult::Failed(pulled.error.unwrap_or_else(|| "pull failed".into()))
    }
}

/// Map a `git` `Result` to a bulk op result.
fn git_result(result: Result<(), String>) -> RepoOpResult {
    match result {
        Ok(()) => RepoOpResult::Ok,
        Err(reason) => RepoOpResult::Failed(reason),
    }
}

/// Default-branch switch result for one repo.
fn switch_result(outcome: SwitchOutcome) -> RepoOpResult {
    match outcome {
        SwitchOutcome::Switched => RepoOpResult::Ok,
        SwitchOutcome::Skipped(reason) => RepoOpResult::Skipped(reason),
        SwitchOutcome::Failed(reason) => RepoOpResult::Failed(reason),
    }
}

struct WriteJob {
    /// Short name for `busy: <op> running`.
    op: &'static str,
    gitdirs: Vec<String>,
    work: Box<dyn FnOnce() -> Result<String, String> + Send>,
}

struct CompareRangeJob {
    gen: u64,
    tab_id: u64,
    repo: String,
    base_ref: String,
    head_ref: String,
    /// The tab's file on disk when it is a commit-vs-working-tree tab.
    worktree_file: Option<WorktreeFile>,
}

struct CompareDiffJob {
    req_id: u64,
    gen: u64,
    tab_id: u64,
    repo: String,
    source: CommitFileSource,
    path: String,
    old_path: Option<String>,
}

struct CompareProbeJob {
    tab_id: u64,
    repo: String,
    base_ref: String,
    head_ref: String,
    last_head: Option<String>,
    last_base_tip: Option<String>,
}

/// One search-in-files chunk waiting for a worker
/// ([`Effect::SearchFilesChunk`]).
struct SearchChunkJob {
    gen: u64,
    index: Arc<FileIndex>,
    query: String,
    options: SearchOptions,
    start: usize,
    room: usize,
}

impl SearchChunkJob {
    /// Compile the query and search one chunk under `cwd`. Stops between
    /// files once `live` no longer holds this job's generation.
    ///
    /// The event thread validated the query before it asked, so a compile
    /// error here only ends the search with no hits.
    fn run(self, cwd: &Path, live: &AtomicU64) -> JobOutcome {
        let chunk = match build_matcher(&self.query, self.options) {
            Ok(matcher) => search_chunk(
                cwd,
                &self.index,
                &matcher,
                self.start,
                self.room,
                SEARCH_CHUNK_BUDGET,
                || live.load(Ordering::Relaxed) != self.gen,
            ),
            Err(_) => SearchChunk::default(),
        };
        JobOutcome::SearchChunk {
            gen: self.gen,
            chunk,
        }
    }
}

/// Occupancy key: linked checkouts use `primary_repo`, else the checkout path.
fn gitdir_key(state: &AppState, checkout: &str) -> String {
    state
        .snapshot
        .repos
        .iter()
        .find(|row| row.repo == checkout)
        .map(|row| row.primary_repo.clone().unwrap_or_else(|| row.repo.clone()))
        .unwrap_or_else(|| checkout.to_string())
}

fn overlay_write_checkouts(state: &AppState, action: &Action) -> Vec<String> {
    match action {
        Action::ConfirmYes | Action::ConfirmYesClean => match state.confirm.as_ref() {
            Some(PendingConfirm::Revert { targets, .. }) => {
                // A key the box does not offer writes nothing, so it is never busy.
                let clean = matches!(action, Action::ConfirmYesClean);
                let offered = revert_scope(targets).key_deletes_untracked(clean).is_some();
                if offered {
                    targets.iter().map(|t| t.repo.clone()).collect()
                } else {
                    Vec::new()
                }
            }
            Some(PendingConfirm::RevertRange { repo, .. })
            | Some(PendingConfirm::StashDrop { repo, .. })
            | Some(PendingConfirm::CheckoutOutOfSync { repo, .. })
            | Some(PendingConfirm::MergeIntoHead { repo, .. }) => vec![repo.clone()],
            Some(PendingConfirm::RemoveWorktree { primary, path, .. }) => {
                vec![primary.clone(), path.clone()]
            }
            Some(PendingConfirm::SwitchToDefault { repos }) => repos.clone(),
            // `Y` keeps a compare confirm open, so it writes nothing.
            Some(
                PendingConfirm::CompareRevertRange { target, .. }
                | PendingConfirm::CompareRevertFile { target },
            ) => {
                if matches!(action, Action::ConfirmYesClean) {
                    Vec::new()
                } else {
                    vec![target.repo.clone()]
                }
            }
            None => Vec::new(),
        },
        Action::CreateBranchSubmit => state
            .create_branch
            .as_ref()
            .map(|create| vec![create.repo.clone()])
            .unwrap_or_default(),
        Action::StashMenuEnter => stash_menu_write_checkouts(state, None, true),
        Action::StashMenuChar(key) => stash_menu_write_checkouts(state, Some(*key), false),
        Action::BranchSubmit => branch_submit_write_checkouts(state),
        _ => Vec::new(),
    }
}

fn stash_menu_write_checkouts(state: &AppState, input: Option<char>, enter: bool) -> Vec<String> {
    let Some(ops) = state.stash_menu.as_ref() else {
        return Vec::new();
    };
    match resolve_stash_menu_key(input, enter, false, ops) {
        StashMenuKeyResult::Run(op)
            if matches!(op.id, StashOpId::Create | StashOpId::Apply | StashOpId::Pop) =>
        {
            state.stash_repo.clone().into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn branch_submit_write_checkouts(state: &AppState) -> Vec<String> {
    let Some(picker) = state.branch_picker.as_ref() else {
        return Vec::new();
    };
    // Only the create row writes a new ref from the picker.
    if picker.on_create_row() {
        vec![picker.repo.clone()]
    } else {
        Vec::new()
    }
}

/// Shared Effect scheduler, spawn, and apply.
///
/// Owns the queues the live `JoinSet` and the sync pump drain.
pub(crate) struct Interpreter {
    sched: Scheduler,
    metas: HashMap<String, (RepoCheckoutMeta, Option<String>)>,
    pane_req: Option<RightPaneRequest>,
    writes: VecDeque<WriteJob>,
    remote: RemoteQueue,
    default_queue: VecDeque<String>,
    prepare_stash: Option<(u64, String)>,
    prepare_branches: Option<(u64, String, bool)>,
    prepare_compare: Option<(u64, String, ComparePickerKind)>,
    checkout: Option<(String, String, Option<String>, String)>,
    merge: Option<(String, String, String, String)>,
    commit_files: Option<(u64, String, CommitFileSource)>,
    commit_diff: Option<(u64, String, CommitFileSource, String)>,
    compare_range: VecDeque<CompareRangeJob>,
    compare_diff: VecDeque<CompareDiffJob>,
    compare_probe: VecDeque<CompareProbeJob>,
    /// Queued autoload: generation and graph target.
    autoload: Option<(u64, GraphIdentity)>,
    /// The status `loading older…` replaced. Kept here, not on the job, so
    /// a cancelled or stale autoload still puts it back.
    older_prev: Option<StatusMessage>,
    default_tally: OpTally,
    default_total: usize,
    default_repos: Vec<String>,
    pending_edit: Option<(String, String, Option<u32>)>,
    pending_diff: Option<(String, String, ExternalDiffKind)>,
    diff_prepare: Option<DiffPrepareJob>,
    pending_diff_launch: Option<DiffLaunch>,
    /// Latest Quick Open index request; a newer one replaces it before spawn.
    file_index_job: Option<(u64, Vec<IndexRoot>)>,
    /// Latest Quick Open score request; a newer one replaces it before spawn.
    file_score_job: Option<(u64, Arc<FileIndex>, String)>,
    /// Queued file tab reads: tab id, load generation, checkout, path.
    file_tab_jobs: VecDeque<(u64, u64, String, String)>,
    /// Latest search-in-files index request; a newer one replaces it
    /// before spawn.
    search_index_job: Option<(u64, Vec<IndexRoot>)>,
    /// Latest search-in-files chunk request; a newer one replaces it
    /// before spawn.
    search_chunk_job: Option<SearchChunkJob>,
    /// Generation of the search that may still run (0: none). Synced from
    /// the state after every change; a running chunk stops between files
    /// once it no longer names the chunk's own generation.
    search_live: Arc<AtomicU64>,
    /// Latest focused-line blame question; a newer one replaces it
    /// before spawn, so cursor moves never queue a blame per line.
    line_blame_slot: Option<BlameKey>,
    /// A [`UserTag::LineBlame`] job is queued or running.
    line_blame_inflight: bool,
    /// Latest blame previous-change request; a newer one replaces it
    /// before spawn.
    blame_previous: Option<(u64, String, Box<crate::git::LineBlame>)>,
    /// Queued `gx` opens: checkout, branch. Each has its own slot.
    pr_opens: VecDeque<(PathBuf, String)>,
    /// Queued badge lookups: checkout, branch.
    pr_badges: VecDeque<(PathBuf, String)>,
    /// Queued popover detail fetches. They run before waiting badge
    /// lookups.
    pr_details: VecDeque<PrDetailJob>,
    /// Badge lookups and detail fetches on a worker (at most
    /// [`PR_BADGE_LOOKUPS_MAX`]).
    pr_badges_running: usize,
    /// Forge CLI runner ([`DEFAULT_PR_CLI`]; tests swap in a fake).
    pr_cli: PrCliRunner,
    /// Browser opener ([`DEFAULT_PR_OPENER`]; tests swap in a fake).
    pr_opener: PrUrlOpener,
    /// Set on quit. PR jobs on workers stop their CLI or opener wait at
    /// once, so the runtime shutdown does not wait for them.
    pr_cancel: Arc<AtomicBool>,
    exclusive_inflight: HashMap<u64, Vec<String>>,
    /// Name of the exclusive write or default-branch job on a worker.
    running_write: Option<&'static str>,
    dirty: bool,
}

impl Interpreter {
    /// Empty interpreter with the default fetch/status cap.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_fetch_cap(crate::parallel::FETCH_CONCURRENCY)
    }

    /// Empty interpreter whose Scheduler runs at most `cap` jobs at once
    /// (the resolved `fetchConcurrency`).
    pub(crate) fn with_fetch_cap(cap: usize) -> Self {
        Self {
            sched: Scheduler::new(cap),
            metas: HashMap::new(),
            pane_req: None,
            writes: VecDeque::new(),
            remote: RemoteQueue::new(),
            default_queue: VecDeque::new(),
            prepare_stash: None,
            prepare_branches: None,
            prepare_compare: None,
            checkout: None,
            merge: None,
            commit_files: None,
            commit_diff: None,
            compare_range: VecDeque::new(),
            compare_diff: VecDeque::new(),
            compare_probe: VecDeque::new(),
            autoload: None,
            older_prev: None,
            default_tally: OpTally::default(),
            default_total: 0,
            default_repos: Vec::new(),
            pending_edit: None,
            pending_diff: None,
            diff_prepare: None,
            pending_diff_launch: None,
            file_index_job: None,
            file_score_job: None,
            file_tab_jobs: VecDeque::new(),
            search_index_job: None,
            search_chunk_job: None,
            search_live: Arc::new(AtomicU64::new(0)),
            line_blame_slot: None,
            line_blame_inflight: false,
            blame_previous: None,
            pr_opens: VecDeque::new(),
            pr_badges: VecDeque::new(),
            pr_details: VecDeque::new(),
            pr_badges_running: 0,
            pr_cli: DEFAULT_PR_CLI,
            pr_opener: DEFAULT_PR_OPENER,
            pr_cancel: Arc::new(AtomicBool::new(false)),
            exclusive_inflight: HashMap::new(),
            running_write: None,
            dirty: false,
        }
    }

    /// Stop PR lookups and opens on workers: a running forge CLI is killed,
    /// an opener wait ends, and a job that has not started fails at once.
    ///
    /// The live loop calls this when it exits, before the runtime that owns
    /// the worker threads shuts down and waits for them. Their outcomes are
    /// never applied. Other jobs are not affected.
    pub(crate) fn cancel_pull_request_jobs(&self) {
        self.pr_cancel.store(true, Ordering::Relaxed);
    }

    /// True when an exclusive write or default-branch switch is in flight or queued.
    ///
    /// Remote fetch / pull / push use the per-gitdir queue and do not set this.
    pub(crate) fn busy_for_writes(&self) -> bool {
        self.sched.busy_for_writes()
    }

    /// Name of the running exclusive write (`stage`, `checkout`, …), if one
    /// is on a worker. `None` while the job is still queued.
    pub(crate) fn running_write_op(&self) -> Option<&'static str> {
        self.running_write
    }

    #[cfg(test)]
    fn with_cap(cap: usize) -> Self {
        let mut this = Self::new();
        this.sched = Scheduler::new(cap);
        this
    }

    #[cfg(test)]
    fn pending_remotes(&self) -> Vec<(RunningOp, String)> {
        self.remote
            .pending
            .iter()
            .map(|job| (job.kind, job.checkout.clone()))
            .collect()
    }

    #[cfg(test)]
    fn occupied_gitdirs(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.remote.occupy.keys().cloned().collect();
        keys.sort();
        keys
    }

    #[cfg(test)]
    fn write_jobs_queued(&self) -> usize {
        self.writes.len() + usize::from(self.checkout.is_some()) + usize::from(self.merge.is_some())
    }

    #[cfg(test)]
    fn default_branch_queued(&self) -> bool {
        !self.default_queue.is_empty() || self.sched.queued_user_tag(UserTag::DefaultBranch) > 0
    }

    /// True when schedule or apply changed state that needs a paint.
    pub(crate) fn take_dirty(&mut self) -> bool {
        let dirty = self.dirty;
        self.dirty = false;
        dirty
    }

    /// TTY `$EDITOR` request, if [`Effect::EditFile`] ran since the last take.
    ///
    /// `(checkout, path, 1-based line)`.
    pub(crate) fn take_pending_edit(&mut self) -> Option<(String, String, Option<u32>)> {
        self.pending_edit.take()
    }

    /// External diff request, if [`Effect::ExternalDiff`] ran since the last take.
    pub(crate) fn take_pending_diff(&mut self) -> Option<(String, String, ExternalDiffKind)> {
        self.pending_diff.take()
    }

    /// Queue blob/temp prepare on the blocking pool. The live loop launches the tool after apply.
    pub(crate) fn enqueue_diff_prepare(
        &mut self,
        repo: String,
        path: String,
        kind: ExternalDiffKind,
        opts: &TuiOpts,
    ) {
        let tool = resolve_diff_tool(opts.config.diff_tool.as_deref());
        let repo_abs = opts.cwd.join(&repo);
        self.diff_prepare = Some(DiffPrepareJob {
            repo,
            path,
            kind,
            tool,
            repo_abs,
        });
        self.sched.enqueue_user(UserTag::DiffPrepare);
    }

    /// Prepared LEFT/RIGHT paths, if a DiffPrepare worker finished since the last take.
    pub(crate) fn take_pending_diff_launch(&mut self) -> Option<DiffLaunch> {
        self.pending_diff_launch.take()
    }

    /// Schedule `effect`, then run and apply every job on this thread.
    ///
    /// Unit tests use this so they see the same apply path as the live loop.
    /// [`Effect::EditFile`] and [`Effect::ExternalDiff`] are dropped (no TTY spawn).
    #[cfg(test)]
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

    /// Enqueue work for `effect`, then the state-driven follow-ups
    /// ([`Self::after_change`]). Does not spawn.
    pub(crate) fn schedule(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        effect: Effect,
        action: &Action,
    ) {
        self.schedule_effect(state, opts, effect, action);
        self.after_change(state);
    }

    fn schedule_effect(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        effect: Effect,
        action: &Action,
    ) {
        if self.refuse_exclusive_if_busy(state, &effect) {
            return;
        }
        match effect {
            Effect::Quit => {}
            Effect::None => {
                if matches!(action, Action::Pull) && is_idle_pull_status(&state.status) {
                    self.enqueue_pull_after_inflight_fetch(state);
                }
            }
            Effect::Batch(effects) => {
                for child in effects {
                    self.schedule_effect(state, opts, child, action);
                }
            }
            Effect::WatchRefresh => {
                self.sched.on_watch_tick(state.focused_checkout_path());
            }
            Effect::ReloadSnapshot => {
                state.forget_pr_lookups(None);
                self.sched.on_reload_snapshot(state.focused_checkout_path());
            }
            Effect::ReloadRepo { repo } => {
                state.forget_pr_lookups(Some(Path::new(&repo)));
                self.sched.on_reload_repo(repo);
            }
            Effect::LoadRightPane => {
                if state.tabs.is_workspace() {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
            }
            Effect::Fetch { repos } => {
                let foreground = !matches!(action, Action::FetchTick);
                self.start_bulk(state, RunningOp::Fetch, repos, foreground)
            }
            Effect::Pull { repos } => self.start_bulk(state, RunningOp::Pull, repos, true),
            Effect::Push { repos } => self.start_bulk(state, RunningOp::Push, repos, true),
            Effect::DefaultBranch { repos } => {
                self.default_total = repos.len();
                self.default_tally = OpTally::default();
                self.default_repos = repos.clone();
                state.status = StatusMessage::progress(format_running_op(
                    RunningOp::DefaultBranch,
                    0,
                    repos.len(),
                ));
                self.mark();
                self.occupy_default_branch(state, &repos);
                self.default_queue = repos.into();
                if !self.default_queue.is_empty() {
                    self.sched.enqueue_user(UserTag::DefaultBranch);
                }
            }
            Effect::Stage { repo, paths } => {
                let dir = opts.cwd.join(&repo);
                let last = paths.last().cloned().unwrap_or_default();
                self.enqueue_write(
                    state,
                    "stage",
                    &[&repo],
                    Box::new(move || {
                        for path in &paths {
                            stage_file(&dir, path)?;
                        }
                        Ok(format!("staged {last}"))
                    }),
                );
            }
            Effect::Unstage { repo, paths } => {
                let dir = opts.cwd.join(&repo);
                let last = paths.last().cloned().unwrap_or_default();
                self.enqueue_write(
                    state,
                    "unstage",
                    &[&repo],
                    Box::new(move || {
                        for path in &paths {
                            unstage_file(&dir, path)?;
                        }
                        Ok(format!("unstaged {last}"))
                    }),
                );
            }
            Effect::ApplyCachedPatch {
                repo,
                path,
                patch,
                reverse,
            } => {
                let dir = opts.cwd.join(&repo);
                let verb = if reverse { "unstaged" } else { "staged" };
                self.enqueue_write(
                    state,
                    if reverse { "unstage" } else { "stage" },
                    &[&repo],
                    Box::new(move || {
                        apply_cached_patch(&dir, &patch, reverse)?;
                        Ok(format!("{verb} range {path}"))
                    }),
                );
            }
            Effect::RevertPatch { repo, path, patch } => {
                let dir = opts.cwd.join(&repo);
                self.enqueue_write(
                    state,
                    "revert",
                    &[&repo],
                    Box::new(move || {
                        apply_worktree_patch_reverse(&dir, &patch)?;
                        Ok(format!("reverted range {path}"))
                    }),
                );
            }
            Effect::Revert {
                repo,
                tracked,
                untracked,
            } => {
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
                self.enqueue_write(
                    state,
                    "revert",
                    &[&repo],
                    Box::new(move || {
                        for path in &tracked {
                            revert_tracked_file(&dir, path)?;
                        }
                        for path in &untracked {
                            remove_untracked_file(&dir, path)?;
                        }
                        Ok(ok_status)
                    }),
                );
            }
            Effect::CompareRevertPatch {
                repo,
                path,
                head,
                patch,
            } => {
                let dir = opts.cwd.join(&repo);
                self.enqueue_write(
                    state,
                    "revert",
                    &[&repo],
                    Box::new(move || {
                        revert_compare_patch(&dir, &head, &path, &patch)?;
                        Ok(format!(
                            "reverted range {path} to merge base (see Workspace tab)"
                        ))
                    }),
                );
            }
            Effect::CompareRevertFile {
                repo,
                path,
                old_path,
                deletes,
                merge_base,
                head,
            } => {
                let dir = opts.cwd.join(&repo);
                let ok_status = if deletes {
                    format!("deleted {path} (not in merge base; see Workspace tab)")
                } else {
                    format!("reverted {path} to merge base (see Workspace tab)")
                };
                self.enqueue_write(
                    state,
                    "revert",
                    &[&repo],
                    Box::new(move || {
                        revert_compare_file(&dir, &merge_base, &head, &path, old_path.as_deref())?;
                        Ok(ok_status)
                    }),
                );
            }
            Effect::EditFile { repo, path, line } => {
                self.pending_edit = Some((repo, path, line));
                self.mark();
            }
            Effect::ExternalDiff { repo, path, kind } => {
                self.pending_diff = Some((repo, path, kind));
                self.mark();
            }
            Effect::StashCreate { repo, paths } => {
                let dir = opts.cwd.join(&repo);
                let ok_status = if paths.len() == 1 {
                    "Stashed 1 file".to_string()
                } else if paths.is_empty() {
                    "Stashed".to_string()
                } else {
                    format!("Stashed {} files", paths.len())
                };
                self.enqueue_write(
                    state,
                    "stash",
                    &[&repo],
                    Box::new(move || stash_push(&dir, &paths).map(|_| ok_status)),
                );
            }
            Effect::StashApply { repo, stash_ref } => {
                let dir = opts.cwd.join(&repo);
                let label = stash_ref.clone();
                self.enqueue_write(
                    state,
                    "stash apply",
                    &[&repo],
                    Box::new(move || {
                        stash_apply(&dir, &stash_ref).map(|_| format!("applied {label}"))
                    }),
                );
            }
            Effect::StashPop { repo, stash_ref } => {
                let dir = opts.cwd.join(&repo);
                let label = stash_ref.clone();
                self.enqueue_write(
                    state,
                    "stash pop",
                    &[&repo],
                    Box::new(move || {
                        stash_pop(&dir, &stash_ref).map(|_| format!("popped {label}"))
                    }),
                );
            }
            Effect::StashDrop { repo, stash_ref } => {
                let dir = opts.cwd.join(&repo);
                let label = stash_ref.clone();
                self.enqueue_write(
                    state,
                    "stash drop",
                    &[&repo],
                    Box::new(move || {
                        stash_drop(&dir, &stash_ref).map(|_| format!("dropped {label}"))
                    }),
                );
            }
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
                let gitdir = gitdir_key(state, &repo);
                self.occupy_exclusive(std::slice::from_ref(&gitdir));
                self.checkout = Some((repo, selected_name, fast_forward_ref, gitdir));
                self.sched.enqueue_user(UserTag::Write);
            }
            Effect::CreateBranch { repo, name } => {
                let dir = opts.cwd.join(&repo);
                let label = name.clone();
                self.enqueue_write(
                    state,
                    "create branch",
                    &[&repo],
                    Box::new(move || {
                        create_branch_checkout(&dir, &name).map(|_| format!("created {label}"))
                    }),
                );
            }
            Effect::CreateBranchAt {
                repo,
                name,
                commit_id,
            } => {
                let dir = opts.cwd.join(&repo);
                let label = name.clone();
                let short = commit_id.get(..7).unwrap_or(&commit_id).to_string();
                self.enqueue_write(
                    state,
                    "create branch",
                    &[&repo],
                    Box::new(move || {
                        create_branch_at(&dir, &name, &commit_id)
                            .map(|_| format!("created {label} at {short}"))
                    }),
                );
            }
            Effect::MergeIntoHead { repo, rev, label } => {
                let gitdir = gitdir_key(state, &repo);
                self.occupy_exclusive(std::slice::from_ref(&gitdir));
                self.merge = Some((repo, rev, label, gitdir));
                self.sched.enqueue_user(UserTag::Write);
            }
            Effect::RemoveWorktree {
                primary,
                path,
                force,
            } => {
                let primary_dir = opts.cwd.join(&primary);
                let path_dir = opts.cwd.join(&path);
                let label = path.clone();
                self.enqueue_write(
                    state,
                    "worktree remove",
                    &[&primary, &path],
                    Box::new(move || {
                        remove_worktree(&primary_dir, &path_dir, force)
                            .map(|_| format!("removed worktree {label}"))
                    }),
                );
            }
            Effect::LoadCommitFiles { repo, source } => {
                if state.tabs.is_workspace() {
                    state.begin_commit_files(repo.clone(), source.clone());
                    let gen = self.sched.request_commit_files();
                    self.commit_files = Some((gen, repo, source));
                    self.sched.enqueue_user_front(UserTag::Pane);
                    self.mark();
                }
            }
            Effect::LoadCommitDiff { repo, source, path } => {
                let gen = self.sched.request_commit_diff();
                self.commit_diff = Some((gen, repo, source, path));
                self.sched.enqueue_user_front(UserTag::Pane);
            }
            Effect::DropCommitDiff => {
                let _ = self.sched.request_commit_diff();
            }
            Effect::LoadCompareRange {
                tab_id,
                repo,
                base_ref,
                head_ref,
                force,
            } => {
                if let Some(gen) = state.begin_compare_range(tab_id, force) {
                    let worktree_file = state
                        .tabs
                        .get_id(tab_id)
                        .and_then(|tab| tab.worktree_file.clone());
                    self.compare_range.push_back(CompareRangeJob {
                        gen,
                        tab_id,
                        repo,
                        base_ref,
                        head_ref,
                        worktree_file,
                    });
                    self.sched.enqueue_user_front(UserTag::Pane);
                    self.mark();
                }
            }
            Effect::LoadCompareDiff {
                tab_id,
                repo,
                source,
                path,
                old_path,
            } => {
                if let Some(tab) = state.tabs.get_id_mut(tab_id) {
                    let gen = tab.generation;
                    tab.diff_req = tab.diff_req.saturating_add(1);
                    let req_id = tab.diff_req;
                    tab.path = Some(path.clone());
                    self.compare_diff.push_back(CompareDiffJob {
                        req_id,
                        gen,
                        tab_id,
                        repo,
                        source,
                        path,
                        old_path,
                    });
                    self.sched.enqueue_user_front(UserTag::Pane);
                }
            }
            Effect::LoadBlamePrevious { gen, repo, blame } => {
                self.blame_previous = Some((gen, repo, blame));
                self.sched.enqueue_user(UserTag::Prepare);
            }
            Effect::PrepareComparePicker { repo, kind } => {
                let gen = self.sched.request_prepare_compare();
                self.prepare_compare = Some((gen, repo, kind));
                self.sched.enqueue_user(UserTag::Prepare);
            }
            Effect::ProbeCompareTab {
                tab_id,
                repo,
                base_ref,
                head_ref,
                last_head,
                last_base_tip,
            } => {
                self.compare_probe.push_back(CompareProbeJob {
                    tab_id,
                    repo,
                    base_ref,
                    head_ref,
                    last_head,
                    last_base_tip,
                });
                self.sched.enqueue_user(UserTag::Pane);
            }
            Effect::LoadFileIndex { gen, roots } => {
                self.file_index_job = Some((gen, roots));
                self.sched.enqueue_user(UserTag::QuickOpen);
            }
            Effect::ScoreFiles { gen, index, query } => {
                self.file_score_job = Some((gen, index, query));
                self.sched.enqueue_user(UserTag::QuickOpen);
            }
            Effect::LoadSearchIndex { gen, roots } => {
                self.search_index_job = Some((gen, roots));
                self.sched.enqueue_user(UserTag::SearchFiles);
            }
            Effect::SearchFilesChunk {
                gen,
                index,
                query,
                options,
                start,
                room,
            } => {
                self.search_chunk_job = Some(SearchChunkJob {
                    gen,
                    index,
                    query,
                    options,
                    start,
                    room,
                });
                self.sched.enqueue_user(UserTag::SearchFiles);
            }
            Effect::LoadFileTab {
                tab_id,
                gen,
                repo,
                path,
            } => {
                self.file_tab_jobs.push_back((tab_id, gen, repo, path));
                self.sched.enqueue_user(UserTag::QuickOpen);
            }
            Effect::OpenPullRequest { repo, branch } => {
                self.pr_opens.push_back((repo, branch));
                self.sched.enqueue_user(UserTag::PullRequest);
            }
            Effect::LookupPullRequests { targets } => {
                self.pr_badges.extend(targets);
                self.fill_pr_badge_slots();
            }
            Effect::LookupPullRequestDetail {
                repo,
                branch,
                remote,
                number,
                request,
            } => {
                self.pr_details.push_back(PrDetailJob {
                    repo,
                    branch,
                    remote,
                    number,
                    request,
                });
                self.fill_pr_badge_slots();
            }
            Effect::CopyClipboard { text, announce } => {
                let ok = comments::copy_to_clipboard(&text);
                // `y` opens the export overlay right before this copy; its
                // header shows the result.
                if let Some(export) = state
                    .comment_export
                    .as_mut()
                    .filter(|export| export.copied.is_none())
                {
                    export.copied = Some(ok);
                }
                if announce {
                    state.status = if ok {
                        StatusMessage::ok(STATUS_COPIED)
                    } else {
                        StatusMessage::error(STATUS_COPY_FAILED)
                    };
                }
                self.mark();
            }
        }
    }

    /// Jobs the state asks for after any schedule or apply: the focused
    /// line's blame, and an older graph page for a pending blame reveal
    /// (dropped once the graph pane loses focus).
    fn after_change(&mut self, state: &mut AppState) {
        self.search_live
            .store(state.search_files_live_gen(), Ordering::Relaxed);
        self.maybe_queue_line_blame(state);
        state.drop_graph_reveal_off_graph();
        if Self::autoload_allowed(state) && state.graph_reveal_wants_older() {
            self.queue_autoload(state);
        }
    }

    /// Queue a blame for the focused line when the cache has no answer.
    ///
    /// Latest-only: the slot holds the newest question, and at most one
    /// job is queued or running. Its apply calls this again, so a cursor
    /// that moved on while git ran gets its own answer next.
    fn maybe_queue_line_blame(&mut self, state: &AppState) {
        let Some(key) = state.line_blame_want() else {
            self.line_blame_slot = None;
            return;
        };
        if state.line_blame.cached(&key).is_some() {
            return;
        }
        self.line_blame_slot = Some(key);
        if !self.line_blame_inflight {
            self.line_blame_inflight = true;
            self.sched.enqueue_user(UserTag::LineBlame);
        }
    }

    /// Queue graph autoload when the cursor sits on the last loaded row.
    pub(crate) fn maybe_queue_autoload(&mut self, state: &mut AppState) {
        if !Self::autoload_allowed(state) {
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
        self.queue_autoload(state);
    }

    /// True when an older graph page may load now: Workspace tab, a graph
    /// loaded and shown, and no older page already on its way.
    fn autoload_allowed(state: &AppState) -> bool {
        state.tabs.is_workspace()
            && !state.graph_loading_older
            && (!state.right_is_diff() || state.in_commit_drill())
            && state.graph.is_some()
    }

    /// Queue the next older page of the loaded graph and say `loading
    /// older…`.
    fn queue_autoload(&mut self, state: &mut AppState) {
        let Some((repo, head)) = state.graph_identity.as_ref() else {
            return;
        };
        state.graph_loading_older = true;
        let gen = self.sched.request_autoload();
        // Keep the slot's previous message so completion can put it back.
        // A slot that still says `loading older…` (a cancelled load not yet
        // back) keeps the message saved before it.
        if state.status != LOADING_OLDER {
            self.older_prev = Some(state.status.clone());
        }
        self.autoload = Some((
            gen,
            GraphIdentity {
                repo: repo.clone(),
                head: head.clone(),
            },
        ));
        state.status = StatusMessage::progress(LOADING_OLDER);
        self.sched.enqueue_user(UserTag::Autoload);
        self.mark();
    }

    /// Put back the status `loading older…` replaced, if the slot still
    /// says `loading older…`. Called whenever no autoload is left to finish.
    fn restore_older_prev(&mut self, state: &mut AppState) {
        let prev = self.older_prev.take().unwrap_or_default();
        if state.status == LOADING_OLDER {
            state.status = prev;
        }
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
                                        default_tip_ref: row.default_tip_ref,
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

    /// Apply one worker result. May enqueue follow-up jobs, then the
    /// state-driven follow-ups ([`Self::after_change`]).
    pub(crate) fn apply(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        id: u64,
        outcome: JobOutcome,
    ) {
        self.apply_outcome(state, opts, id, outcome);
        self.after_change(state);
    }

    fn apply_outcome(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
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
                self.queue_due_pr_lookups(state, opts);
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
                if let Some(reload) = state.worktree_compare_reload(&path) {
                    self.schedule(state, opts, reload, &Action::None);
                }
                if let Some(reload) = state.file_tab_live_reload(&path) {
                    self.schedule(state, opts, reload, &Action::None);
                }
                self.queue_due_pr_lookups(state, opts);
                let decision = self.sched.note_repo_done(gen, &path);
                if focused.as_deref() == Some(path.as_str())
                    && focused_repo_needs_pane(&before_sigs, &before_snap, state, &path)
                    && state.tabs.is_workspace()
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
                if !state.tabs.is_workspace() {
                    // Consume the result. Do not recapture parked Workspace
                    // drill / tree cursor into a new pane request.
                } else {
                    let current = RightPaneRequest::from_state(state).target();
                    if accepted && target == current {
                        if matches!(&load, RightPaneLoad::Graph { .. }) {
                            // A new graph cancels any older-page load.
                            let _ = self.sched.request_autoload();
                            self.autoload = None;
                            state.graph_loading_older = false;
                            self.restore_older_prev(state);
                        }
                        apply_right_pane_load(state, load);
                        self.mark();
                    } else if current != target {
                        self.pane_req = Some(RightPaneRequest::from_state(state));
                        self.sched.request_pane();
                    }
                }
            }
            JobOutcome::Write { status } => {
                self.sched.note_user_done(UserTag::Write);
                self.release_exclusive(state, id);
                state.status = status;
                self.sched.on_reload_snapshot(state.focused_checkout_path());
                if state.tabs.is_workspace() {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
                self.mark();
            }
            JobOutcome::BulkRemote { kind, result, repo } => {
                let gitdir = self
                    .remote
                    .occupy
                    .iter()
                    .find(|(_, occ)| match occ {
                        OccupyReason::Remote(inf) => inf.checkout == repo,
                        _ => false,
                    })
                    .map(|(key, _)| key.clone())
                    .unwrap_or_else(|| gitdir_key(state, &repo));
                self.remote.occupy.remove(&gitdir);
                if let Some(wave) = self.remote.wave_mut(kind) {
                    wave.tally.note(&repo, result);
                    wave.note_repo(&repo);
                }
                self.mark();
                if self.kind_live(kind) {
                    self.paint_remote_running(state);
                } else {
                    self.finish_kind_if_idle(state, kind);
                }
                self.pump_remote_slots(state);
            }
            JobOutcome::DefaultBranch { repo, result } => {
                self.sched.note_user_done(UserTag::DefaultBranch);
                self.default_tally.note(&repo, result);
                let done = self.default_tally.done();
                state.status = StatusMessage::progress(format_running_op(
                    RunningOp::DefaultBranch,
                    done,
                    self.default_total,
                ));
                if !self.default_queue.is_empty() {
                    self.sched.enqueue_user(UserTag::DefaultBranch);
                } else {
                    state.stamp_checkout_flashes(&self.default_repos);
                    state.status = completed_op_status(
                        RunningOp::DefaultBranch,
                        &std::mem::take(&mut self.default_tally),
                    );
                    self.default_repos.clear();
                    self.release_default_branch(state);
                    self.sched.on_reload_snapshot(state.focused_checkout_path());
                    if state.tabs.is_workspace() {
                        self.pane_req = Some(RightPaneRequest::from_state(state));
                        self.sched.request_pane();
                    }
                }
                self.mark();
            }
            JobOutcome::PrepareStash { gen, repo, latest } => {
                self.sched.note_user_done(UserTag::Prepare);
                let accepted = self.sched.accept_prepare_stash_result(gen);
                let current = state.focused_checkout_path();
                if accepted
                    && current.as_deref() == Some(repo.as_str())
                    && state.tabs.is_workspace()
                {
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
                    state.graph_focus_picker_repo()
                } else {
                    state.focused_checkout_path()
                };
                if accepted
                    && current.as_deref() == Some(repo.as_str())
                    && state.tabs.is_workspace()
                {
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
                self.release_exclusive(state, id);
                if apply_checkout_compute(state, repo, result) {
                    self.sched.on_reload_snapshot(state.focused_checkout_path());
                }
                if state.tabs.is_workspace() {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
                self.mark();
            }
            JobOutcome::Merge { label, result } => {
                self.sched.note_user_done(UserTag::Write);
                self.release_exclusive(state, id);
                if apply_merge_compute(state, &label, result) {
                    self.sched.on_reload_snapshot(state.focused_checkout_path());
                }
                if state.tabs.is_workspace() {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
                self.mark();
            }
            JobOutcome::Autoload {
                gen,
                page,
                identity,
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
                    self.restore_older_prev(state);
                    self.mark();
                } else if !state.graph_loading_older {
                    // Stale, and no newer autoload is running.
                    self.restore_older_prev(state);
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
                if accepted && current && state.tabs.is_workspace() {
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
                if accepted && current && state.tabs.is_workspace() {
                    state.open_commit_diff(repo, source, files, file_cursor, path, content);
                    self.mark();
                }
            }
            JobOutcome::DiffPrepared {
                repo,
                path,
                kind,
                tool,
                repo_abs,
                prepared,
            } => {
                self.sched.note_user_done(UserTag::DiffPrepare);
                self.pending_diff_launch = Some(DiffLaunch {
                    repo,
                    path,
                    kind,
                    tool,
                    repo_abs,
                    prepared,
                });
                self.mark();
            }
            JobOutcome::CompareRange {
                tab_id,
                gen,
                result,
            } => {
                let accepted = state
                    .tabs
                    .get_id(tab_id)
                    .is_some_and(|tab| tab.generation == gen);
                if let Some(follow) = state.apply_compare_range(tab_id, gen, result) {
                    self.schedule(state, opts, follow, &Action::None);
                }
                if accepted {
                    self.mark();
                }
            }
            JobOutcome::CompareDiff {
                tab_id,
                req_id,
                gen,
                source,
                path,
                content,
            } => {
                let accepted = state.tabs.get_id(tab_id).is_some_and(|tab| {
                    tab.diff_req == req_id
                        && tab.generation == gen
                        && tab.source.as_ref() == Some(&source)
                });
                if accepted {
                    state.apply_compare_diff(tab_id, gen, &source, &path, content);
                    self.mark();
                }
            }
            JobOutcome::FileIndex { gen, index } => {
                if let Some(follow) = state.apply_file_index(gen, index) {
                    self.schedule(state, opts, follow, &Action::None);
                    self.mark();
                }
            }
            JobOutcome::FileScore { gen, hits } => {
                if let Some(follow) = state.apply_file_score(gen, hits) {
                    self.schedule(state, opts, follow, &Action::None);
                    self.mark();
                }
            }
            JobOutcome::SearchIndex { gen, index } => {
                if let Some(follow) = state.apply_search_index(gen, index) {
                    self.schedule(state, opts, follow, &Action::None);
                    self.mark();
                }
            }
            JobOutcome::SearchChunk { gen, chunk } => {
                if let Some(follow) = state.apply_search_chunk(gen, chunk) {
                    self.schedule(state, opts, follow, &Action::None);
                    self.mark();
                }
            }
            JobOutcome::FileTab { tab_id, gen, body } => {
                if state.apply_file_tab(tab_id, gen, body) {
                    self.mark();
                }
            }
            JobOutcome::LineBlame { key, result } => {
                self.line_blame_inflight = false;
                if state.apply_line_blame(key, result) {
                    self.mark();
                }
            }
            JobOutcome::BlamePrevious {
                gen,
                repo,
                sha,
                result,
            } => {
                self.sched.note_user_done(UserTag::Prepare);
                if let Some(follow) = state.apply_blame_previous(gen, repo, &sha, result) {
                    self.schedule(state, opts, follow, &Action::None);
                }
                self.mark();
            }
            JobOutcome::PullRequestBadge {
                repo,
                branch,
                remote,
                lookup,
            } => {
                self.pr_badges_running = self.pr_badges_running.saturating_sub(1);
                if state.apply_pr_lookup(&repo, &branch, remote, lookup) {
                    self.mark();
                }
                self.fill_pr_badge_slots();
            }
            JobOutcome::PullRequestDetail { job, lookup } => {
                self.pr_badges_running = self.pr_badges_running.saturating_sub(1);
                let PrDetailJob {
                    repo,
                    branch,
                    remote,
                    number,
                    request,
                } = job;
                if state.apply_pr_detail(&repo, &branch, &remote, number, request, lookup) {
                    self.mark();
                }
                self.fill_pr_badge_slots();
            }
            JobOutcome::PullRequestOpen {
                repo,
                branch,
                remote,
                lookup,
                opened,
            } => {
                state.apply_pr_open(&repo, &branch, remote, lookup, opened);
                self.mark();
            }
            JobOutcome::ComparePicker { gen, repo, result } => {
                self.sched.note_user_done(UserTag::Prepare);
                let accepted = self.sched.accept_prepare_compare_result(gen);
                let waiting = state.compare_picker_pending.as_deref() == Some(repo.as_str());
                if accepted && waiting {
                    match result {
                        Ok(ComparePickerRows::Branches(branches)) => {
                            state.open_compare_picker(repo, branches)
                        }
                        Ok(ComparePickerRows::Commits(commits)) => {
                            state.open_compare_commit_picker(repo, commits)
                        }
                        Err(err) => {
                            state.abandon_compare_picker();
                            state.status = StatusMessage::error(err);
                        }
                    }
                    self.mark();
                }
            }
            JobOutcome::CompareProbe { tab_id, result } => {
                let follow = match result {
                    Ok(changed) => state.apply_compare_probe(tab_id, changed, None),
                    Err(err) => {
                        let _ = state.apply_compare_probe(tab_id, false, Some(err));
                        None
                    }
                };
                if let Some(follow) = follow {
                    self.schedule(state, opts, follow, &Action::None);
                }
                if state.tabs.get_id(tab_id).is_some() {
                    self.mark();
                }
            }
        }
    }

    /// Reload a checkout after a TTY editor returns.
    pub(crate) fn after_edit(&mut self, state: &mut AppState, repo: String) {
        self.sched.on_reload_repo(repo);
        if state.tabs.is_workspace() {
            self.pane_req = Some(RightPaneRequest::from_state(state));
            self.sched.request_pane();
        }
        self.mark();
    }

    fn mark(&mut self) {
        self.dirty = true;
    }

    /// Queue badge lookups for checkouts whose branch is new or changed.
    fn queue_due_pr_lookups(&mut self, state: &mut AppState, opts: &TuiOpts) {
        let targets = state.due_pr_lookups();
        if !targets.is_empty() {
            self.schedule(
                state,
                opts,
                Effect::LookupPullRequests { targets },
                &Action::None,
            );
        }
    }

    /// Queue `UserTag::PullRequest` slots for waiting badge lookups and
    /// detail fetches, up to [`PR_BADGE_LOOKUPS_MAX`] on workers. `gx` opens
    /// queue their own slot.
    fn fill_pr_badge_slots(&mut self) {
        let free = PR_BADGE_LOOKUPS_MAX.saturating_sub(self.pr_badges_running);
        let waiting = self.pr_badges.len() + self.pr_details.len();
        let wanted = self.pr_opens.len() + waiting.min(free);
        for _ in self.sched.queued_user_tag(UserTag::PullRequest)..wanted {
            self.sched.enqueue_user(UserTag::PullRequest);
        }
    }

    fn enqueue_write(
        &mut self,
        state: &AppState,
        op: &'static str,
        checkouts: &[&str],
        work: Box<dyn FnOnce() -> Result<String, String> + Send>,
    ) {
        let gitdirs: Vec<String> = checkouts
            .iter()
            .map(|checkout| gitdir_key(state, checkout))
            .collect();
        self.occupy_exclusive(&gitdirs);
        self.writes.push_back(WriteJob { op, gitdirs, work });
        self.sched.enqueue_user(UserTag::Write);
    }

    fn occupy_exclusive(&mut self, gitdirs: &[String]) {
        for gitdir in gitdirs {
            self.remote
                .occupy
                .entry(gitdir.clone())
                .or_insert(OccupyReason::Exclusive);
        }
    }

    fn occupy_default_branch(&mut self, state: &AppState, repos: &[String]) {
        for repo in repos {
            self.remote
                .occupy
                .entry(gitdir_key(state, repo))
                .or_insert(OccupyReason::DefaultBranch);
        }
    }

    fn release_exclusive(&mut self, state: &mut AppState, id: u64) {
        self.running_write = None;
        if let Some(gitdirs) = self.exclusive_inflight.remove(&id) {
            for gitdir in gitdirs {
                if matches!(
                    self.remote.occupy.get(&gitdir),
                    Some(OccupyReason::Exclusive)
                ) {
                    self.remote.occupy.remove(&gitdir);
                }
            }
        }
        self.pump_remote_slots(state);
    }

    fn release_default_branch(&mut self, state: &mut AppState) {
        self.running_write = None;
        self.remote
            .occupy
            .retain(|_, occ| !matches!(occ, OccupyReason::DefaultBranch));
        self.pump_remote_slots(state);
    }

    fn refuse_busy(&mut self, state: &mut AppState) {
        state.status = StatusMessage::warn("busy");
        self.mark();
    }

    fn remote_gitdir_busy(&self, state: &AppState, checkout: &str) -> bool {
        self.remote
            .occupy
            .contains_key(&gitdir_key(state, checkout))
    }

    /// True when an overlay submit would close itself and then hit occupy
    /// refuse. Keep the overlay and set status `busy`.
    pub(crate) fn keep_overlay_if_gitdir_busy(
        &self,
        state: &mut AppState,
        action: &Action,
    ) -> bool {
        let checkouts = overlay_write_checkouts(state, action);
        if checkouts
            .iter()
            .any(|checkout| self.remote_gitdir_busy(state, checkout))
        {
            state.status = StatusMessage::warn("busy");
            true
        } else {
            false
        }
    }

    fn refuse_exclusive_if_busy(&mut self, state: &mut AppState, effect: &Effect) -> bool {
        let busy = match effect {
            Effect::Stage { repo, .. }
            | Effect::Unstage { repo, .. }
            | Effect::ApplyCachedPatch { repo, .. }
            | Effect::RevertPatch { repo, .. }
            | Effect::Revert { repo, .. }
            | Effect::CompareRevertPatch { repo, .. }
            | Effect::CompareRevertFile { repo, .. }
            | Effect::StashCreate { repo, .. }
            | Effect::StashApply { repo, .. }
            | Effect::StashPop { repo, .. }
            | Effect::StashDrop { repo, .. }
            | Effect::CheckoutBranch { repo, .. }
            | Effect::CreateBranch { repo, .. }
            | Effect::CreateBranchAt { repo, .. }
            | Effect::MergeIntoHead { repo, .. } => self.remote_gitdir_busy(state, repo),
            Effect::RemoveWorktree { primary, path, .. } => {
                self.remote_gitdir_busy(state, primary) || self.remote_gitdir_busy(state, path)
            }
            Effect::DefaultBranch { repos } => repos
                .iter()
                .any(|repo| self.remote_gitdir_busy(state, repo)),
            _ => false,
        };
        if busy {
            self.refuse_busy(state);
        }
        busy
    }

    fn fetch_live_for(&self, state: &AppState, checkout: &str) -> bool {
        let gitdir = gitdir_key(state, checkout);
        self.remote
            .pending
            .iter()
            .any(|job| job.kind == RunningOp::Fetch && gitdir_key(state, &job.checkout) == gitdir)
            || matches!(
                self.remote.occupy.get(&gitdir),
                Some(OccupyReason::Remote(inf)) if inf.kind == RunningOp::Fetch
            )
    }

    /// Queue pull when `p` lands during an inflight/pending fetch on that gitdir.
    ///
    /// Dispatch keeps idle in-sync `p` as [`Effect::None`] and status
    /// `nothing behind to pull` (or a diverged line). An unfetched tracking
    /// checkout still looks in-sync, so that path would drop `p` while fetch
    /// occupies the gitdir. Other None Pull paths (right pane, compare tab,
    /// drill) must not follow. Diverged targets stay out: `p` never pulls
    /// them.
    fn enqueue_pull_after_inflight_fetch(&mut self, state: &mut AppState) {
        let targets = op_targets(
            &state.snapshot,
            state.focused_row(),
            state.show_ignored,
            Op::Pull,
        );
        let follow: Vec<String> = targets
            .into_iter()
            .filter(|checkout| {
                !state.snapshot.repos.iter().any(|r| {
                    r.repo == *checkout && r.sync_status == crate::snapshot::SyncStatus::Diverged
                })
            })
            .filter(|checkout| self.fetch_live_for(state, checkout))
            .collect();
        if follow.is_empty() {
            return;
        }
        self.start_bulk(state, RunningOp::Pull, follow, true);
    }

    /// Queue `repos` for `kind`. `foreground` is false only for the
    /// background fetch tick, which paints nothing unless a repo fails.
    fn start_bulk(
        &mut self,
        state: &mut AppState,
        kind: RunningOp,
        repos: Vec<String>,
        foreground: bool,
    ) {
        if repos.is_empty() {
            return;
        }
        if foreground {
            if let Some(wave) = self.remote.wave_mut(kind) {
                wave.foreground = true;
            }
        }
        let mut flushed = false;
        for checkout in repos {
            flushed |= self.enqueue_remote_job(state, kind, checkout);
        }
        if !flushed {
            self.paint_remote_running(state);
        }
        self.pump_remote_slots(state);
        self.mark();
    }

    fn enqueue_remote_job(
        &mut self,
        state: &mut AppState,
        kind: RunningOp,
        checkout: String,
    ) -> bool {
        if matches!(kind, RunningOp::DefaultBranch) {
            return false;
        }
        if self.drop_new_remote(&checkout, kind) {
            return false;
        }

        let mut fetch_idx = None;
        let mut saw_fetch = false;
        let mut saw_pull = false;
        let mut saw_push = false;
        for (i, job) in self.remote.pending.iter().enumerate() {
            if job.checkout != checkout {
                continue;
            }
            match job.kind {
                RunningOp::Fetch => {
                    saw_fetch = true;
                    fetch_idx = Some(i);
                }
                RunningOp::Pull => saw_pull = true,
                RunningOp::Push => saw_push = true,
                RunningOp::DefaultBranch => {}
            }
        }

        match kind {
            RunningOp::Fetch => {
                if saw_fetch || saw_pull {
                    return false;
                }
                self.note_wave_repo(kind, &checkout);
                self.remote.pending.push_back(RemoteJob { kind, checkout });
                false
            }
            RunningOp::Pull => {
                if saw_pull {
                    return false;
                }
                if let Some(i) = fetch_idx {
                    let prev = self.remote.pending[i].checkout.clone();
                    self.remove_wave_repo(RunningOp::Fetch, &prev);
                    self.remote.pending[i].kind = RunningOp::Pull;
                    self.remote.pending[i].checkout = checkout.clone();
                    self.note_wave_repo(RunningOp::Pull, &checkout);
                    return self.finish_kind_if_idle(state, RunningOp::Fetch);
                }
                self.note_wave_repo(kind, &checkout);
                self.remote.pending.push_back(RemoteJob { kind, checkout });
                false
            }
            RunningOp::Push => {
                if saw_push {
                    return false;
                }
                self.note_wave_repo(kind, &checkout);
                self.remote.pending.push_back(RemoteJob { kind, checkout });
                false
            }
            RunningOp::DefaultBranch => false,
        }
    }

    fn drop_new_remote(&self, checkout: &str, kind: RunningOp) -> bool {
        self.remote.occupy.values().any(|occ| match occ {
            OccupyReason::Remote(inf) => {
                inf.checkout == checkout
                    && (inf.kind == kind
                        || (kind == RunningOp::Fetch && inf.kind == RunningOp::Pull))
            }
            _ => false,
        })
    }

    fn note_wave_repo(&mut self, kind: RunningOp, checkout: &str) {
        if let Some(wave) = self.remote.wave_mut(kind) {
            wave.note_repo(checkout);
        }
    }

    fn remove_wave_repo(&mut self, kind: RunningOp, checkout: &str) {
        if let Some(wave) = self.remote.wave_mut(kind) {
            wave.remove_repo(checkout);
        }
    }

    fn kind_live(&self, kind: RunningOp) -> bool {
        self.remote.pending.iter().any(|job| job.kind == kind)
            || self.remote.occupy.values().any(|occ| match occ {
                OccupyReason::Remote(inf) => inf.kind == kind,
                _ => false,
            })
    }

    fn finish_kind_if_idle(&mut self, state: &mut AppState, kind: RunningOp) -> bool {
        if self.kind_live(kind) {
            return false;
        }
        let Some(wave) = self.remote.wave_mut(kind) else {
            return false;
        };
        if wave.done() == 0 && wave.repos.is_empty() {
            return false;
        }
        let repos = std::mem::take(&mut wave.repos);
        let tally = std::mem::take(&mut wave.tally);
        let quiet = !wave.foreground && !tally.has_failure();
        *wave = KindWave::empty();
        state.stamp_checkout_flashes(&repos);
        if !quiet {
            state.status = completed_op_status(kind, &tally);
        }
        self.sched.on_reload_snapshot(state.focused_checkout_path());
        if state.tabs.is_workspace() {
            self.pane_req = Some(RightPaneRequest::from_state(state));
            self.sched.request_pane();
        }
        self.mark();
        true
    }

    fn eligible_remote_count(&self, state: &AppState) -> usize {
        let mut reserved: HashSet<String> = self.remote.occupy.keys().cloned().collect();
        let mut n = 0;
        for job in &self.remote.pending {
            let gitdir = gitdir_key(state, &job.checkout);
            if reserved.contains(&gitdir) {
                continue;
            }
            reserved.insert(gitdir);
            n += 1;
        }
        n
    }

    fn pump_remote_slots(&mut self, state: &AppState) {
        let need = self.eligible_remote_count(state);
        let have = self.sched.queued_user_tag(UserTag::BulkRemote);
        for _ in have..need {
            self.sched.enqueue_user(UserTag::BulkRemote);
        }
    }

    fn paint_remote_running(&self, state: &mut AppState) {
        let mut progress = Vec::new();
        for kind in [RunningOp::Fetch, RunningOp::Pull, RunningOp::Push] {
            let inflight = self
                .remote
                .occupy
                .values()
                .filter(|occ| matches!(occ, OccupyReason::Remote(job) if job.kind == kind))
                .count();
            let pending = self
                .remote
                .pending
                .iter()
                .filter(|job| job.kind == kind)
                .count();
            let Some(wave) = self.remote.wave(kind) else {
                continue;
            };
            // The background fetch tick runs without a progress line.
            if inflight + pending == 0 || !wave.foreground {
                continue;
            }
            let done = wave.done();
            progress.push((kind, done, inflight, pending));
        }
        if progress.is_empty() {
            return;
        }
        let inflight_parts: Vec<(RunningOp, usize)> = progress
            .iter()
            .filter(|(_, _, inflight, _)| *inflight > 0)
            .map(|(kind, _, inflight, _)| (*kind, *inflight))
            .collect();
        if inflight_parts.len() >= 2 {
            state.status =
                StatusMessage::progress(format_mixed_running_op(&inflight_parts, 0, None));
            return;
        }
        if inflight_parts.len() == 1 {
            let run = inflight_parts[0].0;
            let (done, inflight, pending) = progress
                .iter()
                .find(|(kind, ..)| *kind == run)
                .map(|(_, done, inflight, pending)| (*done, *inflight, *pending))
                .unwrap_or((0, 0, 0));
            let total = done + inflight + pending;
            let other_queued: usize = progress
                .iter()
                .filter(|(kind, ..)| *kind != run)
                .map(|(_, _, _, pending)| *pending)
                .sum();
            state.status = StatusMessage::progress(if other_queued > 0 {
                format_mixed_running_op(&[], other_queued, Some((run, done, total)))
            } else {
                format_running_op(run, done, total)
            });
            return;
        }
        if progress.len() == 1 {
            let (kind, done, inflight, pending) = progress[0];
            state.status =
                StatusMessage::progress(format_running_op(kind, done, done + inflight + pending));
            return;
        }
        let (kind, done, inflight, pending) = progress[0];
        let other: usize = progress[1..]
            .iter()
            .map(|(_, _, _, pending)| *pending)
            .sum();
        state.status = StatusMessage::progress(format_mixed_running_op(
            &[],
            other,
            Some((kind, done, done + inflight + pending)),
        ));
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
                if let Some((repo, name, ff, gitdir)) = self.checkout.take() {
                    self.exclusive_inflight.insert(id, vec![gitdir]);
                    self.running_write = Some("checkout");
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
                if let Some((repo, rev, label, gitdir)) = self.merge.take() {
                    self.exclusive_inflight.insert(id, vec![gitdir]);
                    self.running_write = Some("merge");
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
                    self.exclusive_inflight.insert(id, job.gitdirs);
                    self.running_write = Some(job.op);
                    let op = job.op;
                    spawn(
                        id,
                        Box::new(move || JobOutcome::Write {
                            status: write_status(op, (job.work)()),
                        }),
                    );
                    return;
                }
                self.sched.note_job_finished(id);
                self.sched.note_user_done(UserTag::Write);
            }
            UserTag::BulkRemote => {
                let Some(idx) = self.remote.pending.iter().position(|job| {
                    !self
                        .remote
                        .occupy
                        .contains_key(&gitdir_key(state, &job.checkout))
                }) else {
                    self.sched.note_job_finished(id);
                    return;
                };
                let job = self.remote.pending.remove(idx).expect("eligible remote");
                let gitdir = gitdir_key(state, &job.checkout);
                self.remote.occupy.insert(
                    gitdir,
                    OccupyReason::Remote(InflightRemote {
                        kind: job.kind,
                        checkout: job.checkout.clone(),
                    }),
                );
                let kind = job.kind;
                let repo = job.checkout;
                let dir = opts.cwd.join(&repo);
                self.paint_remote_running(state);
                self.mark();
                spawn(
                    id,
                    Box::new(move || {
                        let result = match kind {
                            RunningOp::Fetch => {
                                git_result(exec_git_checked(&["fetch", "--quiet"], &dir))
                            }
                            RunningOp::Pull => pull_result(&dir),
                            RunningOp::Push => git_result(push_quiet(&dir)),
                            RunningOp::DefaultBranch => {
                                RepoOpResult::Failed("not a remote op".into())
                            }
                        };
                        JobOutcome::BulkRemote { kind, result, repo }
                    }),
                );
            }
            UserTag::DefaultBranch => {
                let Some(repo) = self.default_queue.pop_front() else {
                    self.sched.note_job_finished(id);
                    self.sched.note_user_done(UserTag::DefaultBranch);
                    return;
                };
                self.running_write = Some("default-branch switch");
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
                        let result = match task {
                            Some((branch, override_name)) => switch_result(
                                switch_repo_to_default_branch(
                                    &repo,
                                    &branch,
                                    &cwd,
                                    override_name.as_deref(),
                                )
                                .0,
                            ),
                            None => RepoOpResult::Failed("not in the snapshot".into()),
                        };
                        JobOutcome::DefaultBranch { repo, result }
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
                if let Some((gen, repo, kind)) = self.prepare_compare.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        Box::new(move || {
                            let result = match kind {
                                ComparePickerKind::Branch => list_compare_picker_branches(&dir)
                                    .map(ComparePickerRows::Branches),
                                ComparePickerKind::Commit => list_compare_picker_commits(&dir)
                                    .map(ComparePickerRows::Commits),
                            };
                            JobOutcome::ComparePicker { gen, repo, result }
                        }),
                    );
                    return;
                }
                if let Some((gen, repo, blame)) = self.blame_previous.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        Box::new(move || {
                            let result = previous_line_change(&dir, &blame);
                            JobOutcome::BlamePrevious {
                                gen,
                                repo,
                                sha: blame.sha,
                                result,
                            }
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
            UserTag::DiffPrepare => {
                if let Some(job) = self.diff_prepare.take() {
                    spawn(
                        id,
                        Box::new(move || {
                            let prepared = match &job.kind {
                                ExternalDiffKind::Worktree => {
                                    prepare_worktree_diff(&job.repo_abs, &job.path)
                                }
                                ExternalDiffKind::Rev {
                                    left_rev,
                                    right_rev,
                                    left_path,
                                } => prepare_rev_diff_paths(
                                    &job.repo_abs,
                                    left_rev,
                                    right_rev,
                                    &job.path,
                                    left_path.as_deref(),
                                ),
                            };
                            JobOutcome::DiffPrepared {
                                repo: job.repo,
                                path: job.path,
                                kind: job.kind,
                                tool: job.tool,
                                repo_abs: job.repo_abs,
                                prepared,
                            }
                        }),
                    );
                    return;
                }
                self.sched.note_job_finished(id);
                self.sched.note_user_done(UserTag::DiffPrepare);
            }
            UserTag::Pane => {
                if let Some(job) = self.compare_range.pop_front() {
                    let dir = opts.cwd.join(&job.repo);
                    spawn(
                        id,
                        Box::new(move || JobOutcome::CompareRange {
                            tab_id: job.tab_id,
                            gen: job.gen,
                            result: match &job.worktree_file {
                                Some(file) => {
                                    compute_worktree_compare_range(&dir, &job.base_ref, file)
                                }
                                None => compute_compare_range(&dir, &job.base_ref, &job.head_ref),
                            },
                        }),
                    );
                    return;
                }
                if let Some(job) = self.compare_diff.pop_front() {
                    let dir = opts.cwd.join(&job.repo);
                    let context = state.commit_diff_context(&job.repo, &job.path);
                    spawn(
                        id,
                        Box::new(move || {
                            let content = compute_compare_diff(
                                &dir,
                                &job.source,
                                &job.path,
                                job.old_path.as_deref(),
                                context,
                            );
                            JobOutcome::CompareDiff {
                                tab_id: job.tab_id,
                                req_id: job.req_id,
                                gen: job.gen,
                                source: job.source,
                                path: job.path,
                                content,
                            }
                        }),
                    );
                    return;
                }
                if let Some(job) = self.compare_probe.pop_front() {
                    let dir = opts.cwd.join(&job.repo);
                    spawn(
                        id,
                        Box::new(move || {
                            let result = probe_compare_range(
                                &dir,
                                &job.base_ref,
                                &job.head_ref,
                                job.last_head.as_deref(),
                                job.last_base_tip.as_deref(),
                            )
                            .map(|(changed, _, _)| changed);
                            JobOutcome::CompareProbe {
                                tab_id: job.tab_id,
                                result,
                            }
                        }),
                    );
                    return;
                }
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
            UserTag::QuickOpen => {
                // Reads are queued one per enqueue, so draining them first
                // never strands one behind the latest-only index / score slots.
                if let Some((tab_id, gen, repo, path)) = self.file_tab_jobs.pop_front() {
                    let file = opts.cwd.join(repo).join(path);
                    spawn(
                        id,
                        Box::new(move || JobOutcome::FileTab {
                            tab_id,
                            gen,
                            body: read_text_file(&file, MAX_FILE_BYTES),
                        }),
                    );
                    return;
                }
                if let Some((gen, roots)) = self.file_index_job.take() {
                    let cwd = opts.cwd.clone();
                    spawn(
                        id,
                        Box::new(move || JobOutcome::FileIndex {
                            gen,
                            index: build_file_index(&cwd, roots, MAX_INDEX_ENTRIES),
                        }),
                    );
                    return;
                }
                if let Some((gen, index, query)) = self.file_score_job.take() {
                    spawn(
                        id,
                        Box::new(move || JobOutcome::FileScore {
                            gen,
                            hits: score_files(&index, &query, MAX_RESULTS),
                        }),
                    );
                    return;
                }
                self.sched.note_job_finished(id);
            }
            UserTag::SearchFiles => {
                if let Some((gen, roots)) = self.search_index_job.take() {
                    let cwd = opts.cwd.clone();
                    spawn(
                        id,
                        Box::new(move || JobOutcome::SearchIndex {
                            gen,
                            index: build_file_index(&cwd, roots, MAX_INDEX_ENTRIES),
                        }),
                    );
                    return;
                }
                // A chunk whose search is no longer live never starts.
                let live = Arc::clone(&self.search_live);
                if let Some(job) = self
                    .search_chunk_job
                    .take()
                    .filter(|job| job.gen == live.load(Ordering::Relaxed))
                {
                    let cwd = opts.cwd.clone();
                    spawn(id, Box::new(move || job.run(&cwd, &live)));
                    return;
                }
                self.sched.note_job_finished(id);
            }
            UserTag::LineBlame => {
                let Some(key) = self.line_blame_slot.take() else {
                    // The focused line moved off every blame target.
                    self.line_blame_inflight = false;
                    self.sched.note_job_finished(id);
                    return;
                };
                let dir = opts.cwd.join(&key.repo);
                spawn(
                    id,
                    Box::new(move || {
                        let result = blame_line(&dir, &key.rev, &key.path, key.line);
                        JobOutcome::LineBlame { key, result }
                    }),
                );
            }
            UserTag::PullRequest => {
                let cwd = opts.cwd.clone();
                let cli = self.pr_cli;
                let cancel = Arc::clone(&self.pr_cancel);
                if let Some((repo, branch)) = self.pr_opens.pop_front() {
                    let open = self.pr_opener;
                    spawn(
                        id,
                        Box::new(move || {
                            let run = |argv: &[String]| cli(argv, &cancel);
                            let remote = remote_url_for_branch(&cwd.join(&repo), &branch);
                            let lookup = lookup(remote.as_deref(), &branch, &run);
                            let opened = match &lookup {
                                PrLookup::Found(pr) => open(&pr.url, &cancel).is_ok(),
                                PrLookup::NoPr | PrLookup::Failed => false,
                            };
                            JobOutcome::PullRequestOpen {
                                repo,
                                branch,
                                remote,
                                lookup,
                                opened,
                            }
                        }),
                    );
                    return;
                }
                if self.pr_badges_running < PR_BADGE_LOOKUPS_MAX {
                    // A fetch the cache no longer waits on would land
                    // nothing: skip it.
                    while let Some(job) = self.pr_details.pop_front() {
                        if !state.pr_detail_awaited(&job.remote, &job.branch, job.request) {
                            continue;
                        }
                        self.pr_badges_running += 1;
                        spawn(
                            id,
                            Box::new(move || {
                                let run = |argv: &[String]| cli(argv, &cancel);
                                let lookup = lookup_detail(&job.remote, job.number, &run);
                                JobOutcome::PullRequestDetail { job, lookup }
                            }),
                        );
                        return;
                    }
                    if let Some((repo, branch)) = self.pr_badges.pop_front() {
                        self.pr_badges_running += 1;
                        spawn(
                            id,
                            Box::new(move || {
                                let run = |argv: &[String]| cli(argv, &cancel);
                                let remote = remote_url_for_branch(&cwd.join(&repo), &branch);
                                let lookup = lookup(remote.as_deref(), &branch, &run);
                                JobOutcome::PullRequestBadge {
                                    repo,
                                    branch,
                                    remote,
                                    lookup,
                                }
                            }),
                        );
                        return;
                    }
                }
                self.sched.note_job_finished(id);
            }
            UserTag::Autoload => {
                let Some((gen, identity)) = self.autoload.take() else {
                    // A graph pane load cancelled it and already reset the
                    // slot. A newer autoload may be running: leave it alone.
                    self.sched.note_job_finished(id);
                    return;
                };
                let Some(model) = state.graph.as_ref() else {
                    self.sched.note_job_finished(id);
                    state.graph_loading_older = false;
                    self.restore_older_prev(state);
                    return;
                };
                let skip = autoload_skip(model);
                let limit = autoload_limit(model);
                let cwd = opts.cwd.clone();
                let snapshot = state.snapshot.clone();
                let show_ignored = state.show_ignored;
                let focus = state.graph_focus_revs();
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
                        }
                    }),
                );
            }
        }
    }

    #[cfg(test)]
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

    use workspace_status_graph::{Commit, GraphModel, GraphRow};

    use crate::config::WorkspaceStatusConfig;
    use crate::git::{LocalBranch, NameStatus};
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::tui::app::{CompareRangeLoad, MergeCompute, TuiOpts};
    use crate::tui::diff::DiffContent;
    use crate::tui::drill::{CommitFile, CommitFileSource, DrillView};
    use crate::tui::graph_load::GraphIdentity;
    use crate::tui::line_blame::BlameSide;
    use crate::tui::state::{AppState, FocusPane};

    use super::*;
    use crate::tui::chrome::{diverged_pull_status, STATUS_NOTHING_TO_PULL};
    use crate::tui::status::StatusKind;

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
            default_tip_ref: None,
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
            settings: crate::settings::Settings::resolve(&Default::default(), |_| None),
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
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        }
    }

    fn name_status(path: &str) -> NameStatus {
        NameStatus {
            status: "M".into(),
            path: path.into(),
            old_path: None,
            stat: None,
        }
    }

    fn commit_file(path: &str) -> CommitFile {
        CommitFile {
            status: "M".into(),
            path: path.into(),
            old_path: None,
            stat: None,
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
            .position(|row| row.repo.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing repo row {name}"));
        state.cursor = idx;
    }

    fn apply(interp: &mut Interpreter, state: &mut AppState, outcome: JobOutcome) {
        let opts = opts(state);
        interp.apply(state, &opts, 1, outcome);
    }

    /// The live loop must drain the same apply function unit tests use.
    ///
    /// The remaining gap is TTY `$EDITOR` / external diff
    /// (`Effect::EditFile` → `pending_edit`, `Effect::ExternalDiff` → `pending_diff`).
    /// This fails if a second apply match returns or live stops calling
    /// [`Interpreter::apply`].
    #[test]
    fn live_loop_shares_interpreter_apply() {
        let effect = include_str!("effect.rs");
        let loop_src = include_str!("event_loop.rs");
        let app = include_str!("app.rs");

        assert!(
            effect.contains("pub(crate) fn interpret_sync"),
            "sync tests must stay on Interpreter::interpret_sync"
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
            loop_src.contains("enqueue_diff_prepare"),
            "live TTY must enqueue blob/temp prepare off the loop thread"
        );
        assert!(
            loop_src.contains("take_pending_diff_launch"),
            "live TTY must launch the diff tool after temps exist"
        );
        assert!(
            effect.contains("let _ = self.take_pending_edit();"),
            "interpret_sync must drop EditFile (TTY editor only)"
        );
        assert!(
            effect.contains("let _ = self.take_pending_diff();"),
            "interpret_sync must drop ExternalDiff (no TTY spawn)"
        );
        assert!(
            loop_src.contains("fn launch_diff"),
            "live TTY must spawn the external diff tool after prepare"
        );
        assert!(
            effect.contains("prepare_worktree_diff"),
            "blob/temp prepare must run in Interpreter spawn_blocking work"
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
    fn enqueue_diff_prepare_yields_launch_after_worker() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        let tui_opts = opts(&state);
        interp.enqueue_diff_prepare(
            "app".into(),
            "README.md".into(),
            ExternalDiffKind::Worktree,
            &tui_opts,
        );
        let mut outcomes = Vec::new();
        interp.spawn_ready(&mut state, &tui_opts, &mut |id, work| {
            outcomes.push((id, work()));
        });
        assert_eq!(outcomes.len(), 1, "DiffPrepare must spawn one worker");
        let (id, outcome) = outcomes.pop().unwrap();
        assert!(matches!(outcome, JobOutcome::DiffPrepared { .. }));
        interp.apply(&mut state, &tui_opts, id, outcome);
        let launch = interp
            .take_pending_diff_launch()
            .expect("apply must stage a diff launch");
        assert_eq!(launch.repo, "app");
        assert_eq!(launch.path, "README.md");
        if let Ok(prepared) = launch.prepared {
            crate::tui::diff_tool::cleanup_prepared(&prepared);
        }
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
                    left_path: None,
                },
            },
            &Action::ExternalDiff,
        );
        assert!(interp.take_pending_diff().is_none());
    }

    fn compare_source() -> CommitFileSource {
        CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: "bbb".into(),
            merge_base: "aaa".into(),
            head: "ccc".into(),
        }
    }

    fn compare_load(files: &[&str]) -> CompareRangeLoad {
        CompareRangeLoad {
            source: compare_source(),
            files: files.iter().map(|path| commit_file(path)).collect(),
            head: Some("ccc".into()),
            base_tip: "bbb".into(),
        }
    }

    fn open_compare_tab(state: &mut AppState) -> (u64, u64) {
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab = state.tabs.active_compare_mut().unwrap();
        tab.generation = 2;
        (tab.id, tab.generation)
    }

    #[test]
    fn late_compare_range_after_close_does_not_reopen_tab() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        assert!(state.tabs.close_active());
        let mut interp = Interpreter::new();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareRange {
                tab_id,
                gen,
                result: Ok(compare_load(&["stale.txt"])),
            },
        );
        assert!(state.tabs.is_workspace());
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn late_compare_range_wrong_gen_does_not_replace_files() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        state.tabs.active_compare_mut().unwrap().files = vec![commit_file("keep.txt")];
        let mut interp = Interpreter::new();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareRange {
                tab_id,
                gen: gen.saturating_sub(1),
                result: Ok(compare_load(&["stale.txt"])),
            },
        );
        let files: Vec<_> = state
            .tabs
            .active_compare()
            .unwrap()
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(files, vec!["keep.txt"]);
    }

    #[test]
    fn matching_compare_range_still_applies() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        let mut interp = Interpreter::new();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareRange {
                tab_id,
                gen,
                result: Ok(compare_load(&["alpha.txt"])),
            },
        );
        let files: Vec<_> = state
            .tabs
            .active_compare()
            .unwrap()
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(files, vec!["alpha.txt"]);
        assert!(state.tabs.active_compare().unwrap().error.is_none());
    }

    #[test]
    fn compare_range_git_failure_is_error_not_empty() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        let mut interp = Interpreter::new();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareRange {
                tab_id,
                gen,
                result: Err("git compare failed".into()),
            },
        );
        let tab = state.tabs.active_compare().unwrap();
        assert_eq!(tab.error.as_deref(), Some("git compare failed"));
        assert!(tab.files.is_empty());
    }

    #[test]
    fn late_compare_diff_wrong_source_is_discarded() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.source = Some(compare_source());
            tab.path = Some("keep.txt".into());
        }
        let mut interp = Interpreter::new();
        state.tabs.active_compare_mut().unwrap().diff_req = 1;
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareDiff {
                tab_id,
                req_id: 1,
                gen,
                source: CommitFileSource::Compare {
                    base_ref: "main".into(),
                    head_ref: "HEAD".into(),
                    base_tip: "other".into(),
                    merge_base: "aaa".into(),
                    head: "ccc".into(),
                },
                path: "stale.txt".into(),
                content: Ok(DiffContent::default()),
            },
        );
        assert_eq!(
            state.tabs.active_compare().unwrap().path.as_deref(),
            Some("keep.txt")
        );
    }

    #[test]
    fn late_compare_diff_wrong_path_is_discarded() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        let keep = DiffContent::from_unified(" keep\n");
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.source = Some(compare_source());
            tab.path = Some("keep.txt".into());
            tab.content = keep.clone();
            tab.diff_req = 2;
        }
        let mut interp = Interpreter::new();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareDiff {
                tab_id,
                req_id: 1,
                gen,
                source: compare_source(),
                path: "stale.txt".into(),
                content: Ok(DiffContent::from_unified("+stale\n")),
            },
        );
        let tab = state.tabs.active_compare().unwrap();
        assert_eq!(tab.path.as_deref(), Some("keep.txt"));
        assert_eq!(tab.content, keep);
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareDiff {
                tab_id,
                req_id: 2,
                gen,
                source: compare_source(),
                path: "keep.txt".into(),
                content: Ok(DiffContent::from_unified("+keep\n")),
            },
        );
        let tab = state.tabs.active_compare().unwrap();
        assert_eq!(tab.path.as_deref(), Some("keep.txt"));
        assert_eq!(tab.content, DiffContent::from_unified("+keep\n"));
    }

    #[test]
    fn compare_diff_request_is_per_tab() {
        let mut state = fixture_state();
        let (tab_a, gen_a) = open_compare_tab(&mut state);
        {
            let tab = state.tabs.get_id_mut(tab_a).unwrap();
            tab.source = Some(compare_source());
            tab.diff_req = 1;
        }
        state
            .tabs
            .open_or_focus("app".into(), "develop".into(), "HEAD".into());
        let tab_b = state.tabs.active_compare().unwrap().id;
        {
            let tab = state.tabs.get_id_mut(tab_b).unwrap();
            tab.generation = 2;
            tab.source = Some(CommitFileSource::Compare {
                base_ref: "develop".into(),
                head_ref: "HEAD".into(),
                base_tip: "bbb".into(),
                merge_base: "aaa".into(),
                head: "ccc".into(),
            });
            tab.diff_req = 1;
        }
        let mut interp = Interpreter::new();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareDiff {
                tab_id: tab_a,
                req_id: 1,
                gen: gen_a,
                source: compare_source(),
                path: "a.txt".into(),
                content: Ok(DiffContent::from_unified("+a\n")),
            },
        );
        assert_eq!(
            state.tabs.get_id(tab_a).unwrap().path.as_deref(),
            Some("a.txt"),
            "tab A file diff must apply while tab B is active"
        );
        assert!(state.tabs.get_id(tab_b).unwrap().path.is_none());
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareDiff {
                tab_id: tab_b,
                req_id: 1,
                gen: 2,
                source: CommitFileSource::Compare {
                    base_ref: "develop".into(),
                    head_ref: "HEAD".into(),
                    base_tip: "bbb".into(),
                    merge_base: "aaa".into(),
                    head: "ccc".into(),
                },
                path: "b.txt".into(),
                content: Ok(DiffContent::from_unified("+b\n")),
            },
        );
        assert_eq!(
            state.tabs.get_id(tab_a).unwrap().path.as_deref(),
            Some("a.txt")
        );
        assert_eq!(
            state.tabs.get_id(tab_b).unwrap().path.as_deref(),
            Some("b.txt")
        );
    }

    #[test]
    fn compare_diff_return_trip_drops_middle_file() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        let keep = DiffContent::from_unified("+a\n");
        {
            let tab = state.tabs.get_id_mut(tab_id).unwrap();
            tab.source = Some(compare_source());
            tab.files = vec![commit_file("a.txt"), commit_file("b.txt")];
            tab.file_cursor = 0;
            tab.path = Some("a.txt".into());
            tab.content = keep.clone();
        }
        let mut interp = Interpreter::new();
        let opts = opts(&state);
        interp.schedule(
            &mut state,
            &opts,
            Effect::LoadCompareDiff {
                tab_id,
                repo: "app".into(),
                source: compare_source(),
                path: "b.txt".into(),
                old_path: None,
            },
            &Action::None,
        );
        assert_eq!(
            state.tabs.get_id(tab_id).unwrap().path.as_deref(),
            Some("b.txt")
        );
        interp.schedule(
            &mut state,
            &opts,
            Effect::LoadCompareDiff {
                tab_id,
                repo: "app".into(),
                source: compare_source(),
                path: "a.txt".into(),
                old_path: None,
            },
            &Action::None,
        );
        assert_eq!(state.tabs.get_id(tab_id).unwrap().diff_req, 2);
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareDiff {
                tab_id,
                req_id: 1,
                gen,
                source: compare_source(),
                path: "b.txt".into(),
                content: Ok(DiffContent::from_unified("+b\n")),
            },
        );
        let tab = state.tabs.get_id(tab_id).unwrap();
        assert_eq!(tab.path.as_deref(), Some("a.txt"));
        assert_eq!(tab.content, keep);
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CompareDiff {
                tab_id,
                req_id: 2,
                gen,
                source: compare_source(),
                path: "a.txt".into(),
                content: Ok(DiffContent::from_unified("+a2\n")),
            },
        );
        let tab = state.tabs.get_id(tab_id).unwrap();
        assert_eq!(tab.path.as_deref(), Some("a.txt"));
        assert_eq!(tab.content, DiffContent::from_unified("+a2\n"));
    }

    #[test]
    fn late_compare_picker_after_abandon_does_not_open() {
        let mut state = fixture_state();
        state.compare_picker_pending = Some("app".into());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_compare();
        state.abandon_compare_picker();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::ComparePicker {
                gen,
                repo: "app".into(),
                result: Ok(ComparePickerRows::Branches(vec![LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                }])),
            },
        );
        assert!(
            state.compare_picker.is_none(),
            "late ComparePicker must not open after abandon"
        );
        assert!(state.compare_picker_pending.is_none());
    }

    #[test]
    fn matching_compare_picker_still_opens() {
        let mut state = fixture_state();
        state.compare_picker_pending = Some("app".into());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_compare();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::ComparePicker {
                gen,
                repo: "app".into(),
                result: Ok(ComparePickerRows::Branches(vec![LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                }])),
            },
        );
        assert!(
            state.compare_picker.is_some(),
            "matching ComparePicker must open"
        );
        assert!(state.compare_picker_pending.is_none());
    }

    #[test]
    fn late_compare_picker_does_not_clear_other_pending() {
        let mut state = fixture_state();
        state.compare_picker_pending = Some("lib".into());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_compare();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::ComparePicker {
                gen,
                repo: "app".into(),
                result: Ok(ComparePickerRows::Branches(vec![LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                }])),
            },
        );
        assert!(
            state.compare_picker.is_none(),
            "wrong-repo ComparePicker must not open"
        );
        assert_eq!(
            state.compare_picker_pending.as_deref(),
            Some("lib"),
            "late app picker must not drop a later lib pending"
        );
    }

    #[test]
    fn compare_picker_survives_branch_picker_gen_bump() {
        let mut state = fixture_state();
        state.compare_picker_pending = Some("app".into());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_compare();
        let _ = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::ComparePicker {
                gen,
                repo: "app".into(),
                result: Ok(ComparePickerRows::Branches(vec![LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                }])),
            },
        );
        assert!(
            state.compare_picker.is_some(),
            "branch-picker gen must not drop a matching compare picker"
        );
    }

    fn ancestor_rows() -> ComparePickerRows {
        ComparePickerRows::Commits(vec![crate::git::AncestorCommit {
            id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            subject: "seed".into(),
        }])
    }

    #[test]
    fn matching_compare_commit_picker_opens_the_commit_rows() {
        let mut state = fixture_state();
        state.compare_picker_pending = Some("app".into());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_compare();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::ComparePicker {
                gen,
                repo: "app".into(),
                result: Ok(ancestor_rows()),
            },
        );
        assert!(
            matches!(
                state.compare_picker,
                Some(crate::tui::tabs::ComparePickerState::Commit(_))
            ),
            "{:?}",
            state.compare_picker
        );
        assert!(state.compare_picker_pending.is_none());
    }

    #[test]
    fn superseded_compare_picker_kind_does_not_open() {
        let mut state = fixture_state();
        state.compare_picker_pending = Some("app".into());
        let mut interp = Interpreter::new();
        let branch_gen = interp.sched.request_prepare_compare();
        let _commit_gen = interp.sched.request_prepare_compare();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::ComparePicker {
                gen: branch_gen,
                repo: "app".into(),
                result: Ok(ComparePickerRows::Branches(Vec::new())),
            },
        );
        assert!(
            state.compare_picker.is_none(),
            "an older Diff vs branch job must not open over a newer Diff vs commit"
        );
        assert_eq!(state.compare_picker_pending.as_deref(), Some("app"));
    }

    #[test]
    fn matching_right_pane_does_not_apply_on_compare_tab() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        state.drill = DrillView::Graph;
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("app".into(), "head-app".into()));
        state.diff_cursor = 4;
        let _ = open_compare_tab(&mut state);
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.file_cursor = 3;
            tab.path = Some("compare.txt".into());
        }
        state.diff_cursor = 4;
        let mut interp = Interpreter::new();
        let pane_id = interp.sched.request_pane();
        let target = RightPaneRequest::from_state(&state).target();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::RightPane {
                req_id: pane_id,
                target,
                load: RightPaneLoad::Diff {
                    repo: "app".into(),
                    path: "README.md".into(),
                    content: DiffContent::from_unified("-old\n+new\n"),
                },
            },
        );
        let tab = state.tabs.active_compare().unwrap();
        assert_eq!(tab.file_cursor, 3);
        assert_eq!(tab.path.as_deref(), Some("compare.txt"));
        assert_eq!(state.diff_cursor, 4);
        assert!(
            state.graph.is_some(),
            "workspace graph must stay parked on a compare tab"
        );
        assert!(state.drill.is_graph());
    }

    #[test]
    fn merge_completion_does_not_request_pane_on_compare_tab() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        state.open_commit_files(
            "app".into(),
            commit_source(),
            vec![commit_file("parked-drill.md")],
        );
        let _ = open_compare_tab(&mut state);
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.file_cursor = 3;
            tab.path = Some("compare.txt".into());
        }
        let mut interp = Interpreter::new();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::Merge {
                label: "origin/main".into(),
                result: MergeCompute::FastForward,
            },
        );
        assert_eq!(
            interp.sched.latest_pane_id(),
            0,
            "merge on a compare tab must not enqueue LoadPane"
        );
        assert!(interp.pane_req.is_none());
        assert!(state.is_compare_tab());
        let tab = state.tabs.active_compare().unwrap();
        assert_eq!(tab.file_cursor, 3);
        assert_eq!(tab.path.as_deref(), Some("compare.txt"));
        match &state.drill {
            DrillView::Files { files, .. } => {
                assert_eq!(files[0].path, "parked-drill.md");
            }
            other => panic!("parked drill must stay Files, got {other:?}"),
        }
        assert_eq!(state.status, "Fast-forwarded to origin/main");
    }

    #[test]
    fn matching_commit_files_do_not_apply_on_compare_tab() {
        let mut state = fixture_state();
        let source = commit_source();
        state.open_commit_files(
            "app".into(),
            source.clone(),
            vec![commit_file("parked-drill.md")],
        );
        assert!(state.drill.is_files());
        let _ = open_compare_tab(&mut state);
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.file_cursor = 2;
            tab.files = vec![commit_file("compare-only.md")];
        }
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_commit_files();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::CommitFiles {
                gen,
                repo: "app".into(),
                source,
                files: vec![name_status("late.txt")],
            },
        );
        let tab = state.tabs.active_compare().unwrap();
        assert_eq!(tab.file_cursor, 2);
        assert_eq!(tab.files[0].path, "compare-only.md");
        match &state.drill {
            DrillView::Files { files, .. } => {
                assert_eq!(files[0].path, "parked-drill.md");
            }
            other => panic!("parked drill must stay Files, got {other:?}"),
        }
    }

    #[test]
    fn maybe_queue_autoload_skips_compare_tab() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        focus_repo(&mut state, "app");
        state.drill = DrillView::Graph;
        let mut graph = mini_graph(&["aaa"]);
        graph.has_more = true;
        state.graph = Some(graph);
        state.graph_cursor = 10;
        state.graph_identity = Some(("app".into(), "head-app".into()));
        let _ = open_compare_tab(&mut state);
        interp.maybe_queue_autoload(&mut state);
        assert!(
            !state.graph_loading_older,
            "compare tabs must not enqueue graph autoload"
        );
        assert_ne!(state.status, LOADING_OLDER);
    }

    #[test]
    fn autoload_completion_restores_the_status_it_replaced() {
        for before in [StatusMessage::warn("push failed"), StatusMessage::default()] {
            let mut state = fixture_state();
            let mut interp = Interpreter::new();
            focus_repo(&mut state, "app");
            state.drill = DrillView::Graph;
            let mut graph = mini_graph(&["aaa"]);
            graph.has_more = true;
            state.graph = Some(graph);
            state.graph_cursor = 10;
            state.graph_identity = Some(("app".into(), "head-app".into()));
            state.status = before.clone();
            interp.maybe_queue_autoload(&mut state);
            assert_eq!(state.status, LOADING_OLDER);
            let opts = opts(&state);
            interp.pump_sync(&mut state, &opts);
            assert!(!state.graph_loading_older);
            assert_ne!(state.status, LOADING_OLDER);
            assert_eq!(state.status, before);
        }
    }

    #[test]
    fn compare_probe_without_recorded_sha_does_not_force_reload() {
        let mut state = fixture_state();
        let (tab_id, gen) = open_compare_tab(&mut state);
        let follow = state.apply_compare_probe(tab_id, false, None);
        assert!(follow.is_none());
        assert_eq!(state.tabs.get_id(tab_id).unwrap().generation, gen);
        let follow = state.apply_compare_probe(tab_id, true, None);
        assert!(matches!(
            follow,
            Some(Effect::LoadCompareRange { force: true, .. })
        ));
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
    fn prepare_branches_does_not_open_picker_on_compare_tab() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let _ = open_compare_tab(&mut state);
        assert!(state.is_compare_tab());
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_branches();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PrepareBranches {
                gen,
                repo: "app".into(),
                branches: vec![local_branch("main"), local_branch("feature")],
                graph_focus: false,
            },
        );
        assert!(
            state.branch_picker.is_none(),
            "PrepareBranches must not open the checkout picker on a compare tab"
        );
    }

    #[test]
    fn prepare_stash_does_not_open_menu_on_compare_tab() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let _ = open_compare_tab(&mut state);
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
            "PrepareStash must not open the stash menu on a compare tab"
        );
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

    #[test]
    fn tree_prepare_graph_focus_opens_when_identity_is_stale() {
        let mut state = fixture_state();
        state.graph = Some(mini_graph(&["aaa"]));
        state.graph_identity = Some(("lib".into(), "head-lib".into()));
        state.focus = FocusPane::Left;
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
            .expect("tree graph-focus PrepareBranches must open for the highlighted repo");
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

    fn apply_pane_graph(interp: &mut Interpreter, state: &mut AppState) {
        let pane_id = interp.sched.request_pane();
        let target = RightPaneRequest::from_state(state).target();
        apply(
            interp,
            state,
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
    }

    #[test]
    fn pane_graph_over_a_running_autoload_puts_the_status_back() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        state.status = StatusMessage::warn("push failed");
        queue_autoload(&mut state, &mut interp);
        assert_eq!(state.status, LOADING_OLDER);
        let tui_opts = opts(&state);
        let mut batch = Vec::new();
        interp.spawn_ready(&mut state, &tui_opts, &mut |id, work| {
            batch.push((id, work()));
        });
        let (id, stale) = batch
            .into_iter()
            .find(|(_, outcome)| matches!(outcome, JobOutcome::Autoload { .. }))
            .expect("spawned Autoload");

        apply_pane_graph(&mut interp, &mut state);
        assert_eq!(state.status, "push failed");
        apply_id(&mut interp, &mut state, id, stale);
        assert!(!state.graph_loading_older);
        assert_eq!(state.status, "push failed");
        assert_eq!(state.status.kind(), StatusKind::Warn);
    }

    #[test]
    fn pane_graph_over_a_queued_autoload_puts_the_status_back() {
        let mut state = fixture_state();
        let mut interp = Interpreter::new();
        state.status = StatusMessage::warn("push failed");
        queue_autoload(&mut state, &mut interp);
        apply_pane_graph(&mut interp, &mut state);
        assert_eq!(state.status, "push failed");
        // The cancelled autoload tag still drains without a job.
        let tui_opts = opts(&state);
        let mut autoloads = 0;
        interp.spawn_ready(&mut state, &tui_opts, &mut |_, work| {
            if matches!(work(), JobOutcome::Autoload { .. }) {
                autoloads += 1;
            }
        });
        assert_eq!(autoloads, 0);
        assert!(!state.graph_loading_older);
        assert_eq!(state.status, "push failed");
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

    fn capture_jobs(interp: &mut Interpreter, state: &mut AppState) -> Vec<(u64, JobWork)> {
        let tui_opts = opts(state);
        let mut jobs = Vec::new();
        interp.spawn_ready(state, &tui_opts, &mut |id, work| {
            jobs.push((id, work));
        });
        jobs
    }

    fn schedule_effect(
        interp: &mut Interpreter,
        state: &mut AppState,
        effect: Effect,
        action: &Action,
    ) {
        let tui_opts = opts(state);
        interp.schedule(state, &tui_opts, effect, action);
    }

    fn apply_id(interp: &mut Interpreter, state: &mut AppState, id: u64, outcome: JobOutcome) {
        let tui_opts = opts(state);
        interp.apply(state, &tui_opts, id, outcome);
    }

    fn linked_fixture() -> AppState {
        let mut wt = repo("wt", false);
        wt.checkout_kind = crate::snapshot::CheckoutKind::Linked;
        wt.primary_repo = Some("app".into());
        let snapshot = build_workspace_snapshot(&[repo("app", false), wt], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    #[test]
    fn fetch_a_inflight_and_fetch_b_both_spawn_under_cap() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(2);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        let first = capture_jobs(&mut interp, &mut state);
        assert_eq!(first.len(), 1);
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["notes".into()],
            },
            &Action::Fetch,
        );
        let second = capture_jobs(&mut interp, &mut state);
        assert_eq!(second.len(), 1);
        assert_eq!(
            interp.occupied_gitdirs(),
            vec!["app".to_string(), "notes".to_string()]
        );

        let mut capped = Interpreter::with_cap(1);
        let mut capped_state = fixture_state();
        schedule_effect(
            &mut capped,
            &mut capped_state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut capped, &mut capped_state).len(), 1);
        schedule_effect(
            &mut capped,
            &mut capped_state,
            Effect::Fetch {
                repos: vec!["notes".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut capped, &mut capped_state).len(), 0);
        assert_eq!(capped.occupied_gitdirs(), vec!["app".to_string()]);
        assert_eq!(
            capped.pending_remotes(),
            vec![(RunningOp::Fetch, "notes".into())]
        );
    }

    #[test]
    fn pull_same_gitdir_waits_for_inflight_fetch() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        let first = capture_jobs(&mut interp, &mut state);
        assert_eq!(first.len(), 1);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Pull {
                repos: vec!["app".into()],
            },
            &Action::Pull,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 0);
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Pull, "app".into())]
        );

        apply_id(
            &mut interp,
            &mut state,
            first[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Ok,
                repo: "app".into(),
            },
        );
        let _spawned = capture_jobs(&mut interp, &mut state);
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);
        assert!(interp.pending_remotes().is_empty());
    }

    #[test]
    fn pull_none_during_inflight_fetch_still_queues() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);
        state.status = STATUS_NOTHING_TO_PULL.into();
        schedule_effect(&mut interp, &mut state, Effect::None, &Action::Pull);
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Pull, "app".into())]
        );
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);
    }

    #[test]
    fn pull_none_without_behind_status_does_not_queue() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);
        state.status = "Fetching 0/1…".into();
        schedule_effect(&mut interp, &mut state, Effect::None, &Action::Pull);
        assert!(interp.pending_remotes().is_empty());
    }

    #[test]
    fn pull_none_on_linked_checkout_follows_primary_fetch() {
        let mut state = linked_fixture();
        state.folds.remove("group:no-updates");
        state.rebuild_rows();
        focus_repo(&mut state, "wt");
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);
        state.status = STATUS_NOTHING_TO_PULL.into();
        schedule_effect(&mut interp, &mut state, Effect::None, &Action::Pull);
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Pull, "wt".into())]
        );
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 0);
    }

    #[test]
    fn workspace_pull_during_fetch_with_a_diverged_repo_still_pulls_the_rest() {
        let mut app = repo("app", false);
        app.sync_status = SyncStatus::Diverged;
        app.sync_note = "ahead 1, behind 1".into();
        let mut lib = repo("lib", false);
        lib.sync_status = SyncStatus::UpToDate;
        let snapshot = build_workspace_snapshot(&[app, lib], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.cursor = 0;
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into(), "lib".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 2);

        // `lib` is not fetched yet, so it still looks in sync. The diverged
        // `app` stays out of the follow-up pull.
        let effect = state.dispatch(Action::Pull);
        assert_eq!(effect, Effect::None);
        assert_eq!(state.status, diverged_pull_status(&["app".into()]));
        schedule_effect(&mut interp, &mut state, effect, &Action::Pull);
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Pull, "lib".into())]
        );
    }

    #[test]
    fn compare_refusal_is_a_warn_without_the_failed_prefix() {
        let refused = write_status("revert", Err("revert aborted: HEAD moved".into()));
        assert_eq!(refused, "revert aborted: HEAD moved");
        assert_eq!(refused.kind(), StatusKind::Warn);
        let failed = write_status("revert", Err("patch failed: a.txt:1".into()));
        assert_eq!(failed, "revert failed: patch failed: a.txt:1");
        assert_eq!(failed.kind(), StatusKind::Error);
    }

    #[test]
    fn idle_pull_status_matches_the_dispatch_copy() {
        assert!(is_idle_pull_status(STATUS_NOTHING_TO_PULL));
        assert!(is_idle_pull_status(&diverged_pull_status(&["app".into()])));
        assert!(is_idle_pull_status(&diverged_pull_status(&[
            "app".into(),
            "lib".into()
        ])));
        assert!(!is_idle_pull_status("Fetching 0/1…"));
    }

    #[test]
    fn pending_fetch_coalesces_duplicate_fetch() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Fetch, "app".into())]
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);
    }

    #[test]
    fn pending_fetch_replaced_by_pull() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Pull {
                repos: vec!["app".into()],
            },
            &Action::Pull,
        );
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Pull, "app".into())]
        );
    }

    #[test]
    fn fetchtick_coalesces_same_checkouts_and_enqueues_extra() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        let first = capture_jobs(&mut interp, &mut state);
        assert_eq!(first.len(), 1);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into(), "notes".into()],
            },
            &Action::FetchTick,
        );
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Fetch, "notes".into())]
        );
        let extra = capture_jobs(&mut interp, &mut state);
        assert_eq!(extra.len(), 1);
        assert_eq!(
            interp.occupied_gitdirs(),
            vec!["app".to_string(), "notes".to_string()]
        );
        assert!(interp.pending_remotes().is_empty());
    }

    #[test]
    fn start_bulk_keeps_live_other_kind_batch() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Pull {
                repos: vec!["notes".into()],
            },
            &Action::Pull,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);
        assert_eq!(
            interp.occupied_gitdirs(),
            vec!["app".to_string(), "notes".to_string()]
        );
    }

    #[test]
    fn linked_checkout_occupies_primary_gitdir() {
        let mut state = linked_fixture();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        let first = capture_jobs(&mut interp, &mut state);
        assert_eq!(first.len(), 1);
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["wt".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 0);
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Fetch, "wt".into())]
        );
        apply_id(
            &mut interp,
            &mut state,
            first[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Ok,
                repo: "app".into(),
            },
        );
        let second = capture_jobs(&mut interp, &mut state);
        assert_eq!(second.len(), 1);
        assert_eq!(interp.occupied_gitdirs(), vec!["app".to_string()]);
        assert!(interp.pending_remotes().is_empty());
    }

    #[test]
    fn exclusive_write_refuses_occupied_gitdir_and_allows_free() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Stage {
                repo: "app".into(),
                paths: vec!["README.md".into()],
            },
            &Action::Stage,
        );
        assert_eq!(state.status, "busy");
        assert_eq!(interp.write_jobs_queued(), 0);
        assert!(!interp.busy_for_writes());

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Stage {
                repo: "notes".into(),
                paths: vec!["README.md".into()],
            },
            &Action::Stage,
        );
        assert_eq!(interp.write_jobs_queued(), 1);
        assert!(interp.busy_for_writes());
    }

    #[test]
    fn confirm_yes_on_occupied_gitdir_keeps_overlay() {
        use crate::tui::state::{PendingConfirm, RevertTarget};

        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);

        state.confirm = Some(PendingConfirm::Revert {
            targets: vec![RevertTarget {
                repo: "app".into(),
                path: "README.md".into(),
                untracked: false,
                old_path: None,
            }],
            label: "README.md".into(),
        });
        state.status = "confirm revert app?".into();

        assert!(
            interp.keep_overlay_if_gitdir_busy(&mut state, &Action::ConfirmYes),
            "must not dispatch ConfirmYes while that gitdir is occupied"
        );
        assert!(state.confirm.is_some(), "revert overlay must stay pending");
        assert_eq!(state.status, "busy");
        assert_eq!(interp.write_jobs_queued(), 0);

        // Tracked-only scope does not offer `Y`, so it never reports busy.
        state.status = "confirm revert app?".into();
        assert!(!interp.keep_overlay_if_gitdir_busy(&mut state, &Action::ConfirmYesClean));
        assert_eq!(state.status, "confirm revert app?");

        if let Some(PendingConfirm::Revert { targets, .. }) = state.confirm.as_mut() {
            targets.push(RevertTarget {
                repo: "app".into(),
                path: "scratch.txt".into(),
                untracked: true,
                old_path: None,
            });
        }
        assert!(interp.keep_overlay_if_gitdir_busy(&mut state, &Action::ConfirmYesClean));
        assert!(state.confirm.is_some());
        assert_eq!(state.status, "busy");

        state.confirm = Some(PendingConfirm::Revert {
            targets: vec![RevertTarget {
                repo: "notes".into(),
                path: "README.md".into(),
                untracked: false,
                old_path: None,
            }],
            label: "README.md".into(),
        });
        state.status = "confirm revert notes?".into();
        assert!(
            !interp.keep_overlay_if_gitdir_busy(&mut state, &Action::ConfirmYes),
            "free gitdir ConfirmYes must dispatch"
        );
        assert!(state.confirm.is_some());
        assert_eq!(state.status, "confirm revert notes?");
        assert_eq!(interp.write_jobs_queued(), 0);
    }

    #[test]
    fn create_branch_submit_on_occupied_gitdir_keeps_prompt() {
        use crate::tui::branches::CreateBranchState;

        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);

        state.create_branch = Some(CreateBranchState {
            repo: "app".into(),
            name: "topic".into(),
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        });
        state.status = "new branch".into();

        assert!(interp.keep_overlay_if_gitdir_busy(&mut state, &Action::CreateBranchSubmit));
        assert!(state.create_branch.is_some());
        assert_eq!(state.status, "busy");
        assert_eq!(interp.write_jobs_queued(), 0);
    }

    #[test]
    fn stash_menu_write_on_occupied_gitdir_keeps_menu() {
        use crate::tui::stash::{StashOp, StashOpId};

        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);

        let create = StashOp {
            id: StashOpId::Create,
            key: 's',
            label: "stash",
            stash_ref: None,
            paths: Some(vec!["README.md".into()]),
        };
        state.stash_repo = Some("app".into());
        state.stash_menu = Some(vec![create.clone()]);
        state.status = "stash".into();

        assert!(interp.keep_overlay_if_gitdir_busy(&mut state, &Action::StashMenuEnter));
        assert!(state.stash_menu.is_some());
        assert_eq!(state.status, "busy");
        assert_eq!(interp.write_jobs_queued(), 0);

        state.status = "stash".into();
        assert!(interp.keep_overlay_if_gitdir_busy(&mut state, &Action::StashMenuChar('s')));
        assert!(state.stash_menu.is_some());
        assert_eq!(state.status, "busy");

        let drop = StashOp {
            id: StashOpId::Drop,
            key: 'D',
            label: "drop stash",
            stash_ref: Some("stash@{0}".into()),
            paths: None,
        };
        state.stash_menu = Some(vec![drop]);
        state.status = "drop stash".into();
        assert!(
            !interp.keep_overlay_if_gitdir_busy(&mut state, &Action::StashMenuEnter),
            "stash drop opens confirm and must dispatch"
        );
        assert!(state.stash_menu.is_some());
        assert_eq!(state.status, "drop stash");
    }

    #[test]
    fn branch_submit_create_on_occupied_gitdir_keeps_picker() {
        use crate::tui::branches::BranchPickerState;

        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);

        let mut picker = BranchPickerState::checkout("app".into(), vec![local_branch("main")]);
        picker.set_filter("topic".into());
        state.branch_picker = Some(picker);
        state.status = "branch /topic".into();

        assert!(interp.keep_overlay_if_gitdir_busy(&mut state, &Action::BranchSubmit));
        assert!(state.branch_picker.is_some());
        assert_eq!(state.status, "busy");
        assert_eq!(interp.write_jobs_queued(), 0);

        let mut checkout = BranchPickerState::checkout("app".into(), vec![local_branch("feature")]);
        checkout.cursor = 0;
        state.branch_picker = Some(checkout);
        state.status = "branch".into();
        assert!(
            !interp.keep_overlay_if_gitdir_busy(&mut state, &Action::BranchSubmit),
            "checkout submit keeps the picker until compute applies"
        );
        assert_eq!(state.status, "branch");
    }

    #[test]
    fn busy_for_writes_false_for_remotes_true_for_write_and_default_branch() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert!(!interp.busy_for_writes());
        let _ = capture_jobs(&mut interp, &mut state);
        assert!(!interp.busy_for_writes());

        let mut write_state = fixture_state();
        let mut write_interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut write_interp,
            &mut write_state,
            Effect::Stage {
                repo: "app".into(),
                paths: vec!["README.md".into()],
            },
            &Action::Stage,
        );
        assert!(write_interp.busy_for_writes());

        let mut switch_state = fixture_state();
        let mut switch_interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut switch_interp,
            &mut switch_state,
            Effect::DefaultBranch {
                repos: vec!["app".into()],
            },
            &Action::DefaultBranch,
        );
        assert!(switch_interp.busy_for_writes());
    }

    #[test]
    fn inflight_fetch_plus_same_checkout_fetch_does_not_queue() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert!(interp.pending_remotes().is_empty());
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 0);
    }

    #[test]
    fn exclusive_write_blocks_same_gitdir_remote_and_allows_other() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Stage {
                repo: "app".into(),
                paths: vec!["README.md".into()],
            },
            &Action::Stage,
        );
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into(), "notes".into()],
            },
            &Action::Fetch,
        );
        let spawned = capture_jobs(&mut interp, &mut state);
        assert_eq!(spawned.len(), 2);
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Fetch, "app".into())]
        );
        assert_eq!(
            interp.occupied_gitdirs(),
            vec!["app".to_string(), "notes".to_string()]
        );

        apply_id(
            &mut interp,
            &mut state,
            spawned[0].0,
            JobOutcome::Write {
                status: StatusMessage::ok("staged README.md"),
            },
        );
        let _after = capture_jobs(&mut interp, &mut state);
        assert!(interp.pending_remotes().is_empty());
        assert!(interp.occupied_gitdirs().contains(&"app".to_string()));
        assert!(interp.occupied_gitdirs().contains(&"notes".to_string()));
    }

    #[test]
    fn background_fetch_keeps_an_unread_error_on_success() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        state.status = StatusMessage::error("push failed");
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::FetchTick,
        );
        assert_eq!(state.status, "push failed", "no progress line");
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1);
        apply_id(
            &mut interp,
            &mut state,
            jobs[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Ok,
                repo: "app".into(),
            },
        );
        assert_eq!(state.status, "push failed", "no completion line");
        assert_eq!(state.status.kind(), StatusKind::Error);
    }

    #[test]
    fn background_fetch_failure_writes_an_error() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::FetchTick,
        );
        let jobs = capture_jobs(&mut interp, &mut state);
        apply_id(
            &mut interp,
            &mut state,
            jobs[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Failed("could not read from remote repository".into()),
                repo: "app".into(),
            },
        );
        assert_eq!(
            state.status,
            "Fetched 1 repo (1 failed: app — could not read from remote repository)"
        );
        assert_eq!(state.status.kind(), StatusKind::Error);
    }

    #[test]
    fn pane_reload_after_quiet_background_fetch_keeps_an_unread_error() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        focus_repo(&mut state, "app");
        let source = commit_source();
        state.begin_commit_files("app".into(), source.clone());
        state.open_commit_files("app".into(), source.clone(), vec![commit_file("README.md")]);
        assert_eq!(state.status, "1 file in aaa1111");
        state.status = StatusMessage::error("push failed");
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::FetchTick,
        );
        let jobs = capture_jobs(&mut interp, &mut state);
        apply_id(
            &mut interp,
            &mut state,
            jobs[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Ok,
                repo: "app".into(),
            },
        );
        let pane_id = interp.sched.request_pane();
        let target = RightPaneRequest::from_state(&state).target();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::RightPane {
                req_id: pane_id,
                target,
                load: RightPaneLoad::CommitFiles {
                    repo: "app".into(),
                    source,
                    files: vec![name_status("README.md"), name_status("src.rs")],
                },
            },
        );
        assert!(
            matches!(&state.drill, DrillView::Files { files, .. } if files.len() == 2),
            "the pane load must apply, got {:?}",
            state.drill
        );
        assert_eq!(state.status, "push failed");
        assert_eq!(state.status.kind(), StatusKind::Error);
    }

    #[test]
    fn same_source_reload_does_not_rewrite_the_files_note() {
        let mut state = fixture_state();
        let source = commit_source();
        state.begin_commit_files("app".into(), source.clone());
        state.open_commit_files(
            "app".into(),
            source.clone(),
            vec![commit_file("README.md"), commit_file("src.rs")],
        );
        assert_eq!(state.status, "2 files in aaa1111");
        assert_eq!(state.status.kind(), StatusKind::Info);
        state.status = "showing ignored repos".into();
        state.open_commit_files("app".into(), source, vec![commit_file("README.md")]);
        assert_eq!(state.status, "showing ignored repos");
    }

    #[test]
    fn manual_fetch_paints_progress_then_ok() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(state.status.kind(), StatusKind::Progress);
        assert!(state.status.starts_with("Fetching"), "{}", state.status);
        let jobs = capture_jobs(&mut interp, &mut state);
        apply_id(
            &mut interp,
            &mut state,
            jobs[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Ok,
                repo: "app".into(),
            },
        );
        assert_eq!(state.status, "Fetched 1 repo");
        assert_eq!(state.status.kind(), StatusKind::Ok);
    }

    #[test]
    fn running_write_names_the_op_until_it_finishes() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Stage {
                repo: "app".into(),
                paths: vec!["README.md".into()],
            },
            &Action::Stage,
        );
        assert_eq!(interp.running_write_op(), None, "queued, not running");
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(interp.running_write_op(), Some("stage"));
        apply_id(
            &mut interp,
            &mut state,
            jobs[0].0,
            JobOutcome::Write {
                status: StatusMessage::error("git add exited with code 1"),
            },
        );
        assert_eq!(interp.running_write_op(), None);
        assert_eq!(state.status.kind(), StatusKind::Error);
    }

    #[test]
    fn default_branch_refuses_occupied_gitdir() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 1);

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::DefaultBranch {
                repos: vec!["app".into(), "notes".into()],
            },
            &Action::DefaultBranch,
        );
        assert_eq!(state.status, "busy");
        assert!(!interp.default_branch_queued());
        assert!(!interp.busy_for_writes());

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::DefaultBranch {
                repos: vec!["notes".into()],
            },
            &Action::DefaultBranch,
        );
        assert!(interp.default_branch_queued());
        assert!(interp.busy_for_writes());
    }

    #[test]
    fn pull_replacing_last_pending_fetch_completes_fetch_wave() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(1);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into(), "notes".into()],
            },
            &Action::Fetch,
        );
        let first = capture_jobs(&mut interp, &mut state);
        assert_eq!(first.len(), 1);
        apply_id(
            &mut interp,
            &mut state,
            first[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Ok,
                repo: "app".into(),
            },
        );
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Fetch, "notes".into())]
        );
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Pull {
                repos: vec!["notes".into()],
            },
            &Action::Pull,
        );
        assert_eq!(state.status, "Fetched 1 repo");
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Pull, "notes".into())]
        );
    }

    /// Workspace at a temp root with one real git repo `app` on `branch`.
    fn git_app_state(tag: &str, branch: &str, dirty: bool) -> (PathBuf, AppState) {
        let root = crate::testutil::unique_dir(tag);
        let dir = root.join("app");
        crate::testutil::init_repo(&dir);
        if branch != "main" {
            crate::testutil::git(&dir, &["checkout", "-q", "-b", branch]);
        }
        if dirty {
            std::fs::write(dir.join("README.md"), "# dirty\n").unwrap();
        }
        let mut snap = repo("app", dirty);
        snap.branch = branch.into();
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        (root.clone(), AppState::new(root, snapshot, true))
    }

    /// Run every ready job on this thread and apply its outcome.
    fn run_jobs(interp: &mut Interpreter, state: &mut AppState) {
        for (id, work) in capture_jobs(interp, state) {
            let outcome = work();
            apply_id(interp, state, id, outcome);
        }
    }

    #[test]
    fn quick_open_pipeline_indexes_and_scores_real_checkout() {
        use crate::tui::action::QuickOpenEntry;
        use crate::tui::quick_open::FileIndexState;
        let (root, mut state) = git_app_state("ws-effect-quick-open", "main", true);
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(root.join("app/src/main.rs"), "fn main() {}\n").unwrap();
        state.cursor = state
            .rows
            .iter()
            .position(|row| row.repo.as_deref() == Some("app"))
            .expect("app row");
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);
        let open = Action::ToggleQuickOpen(QuickOpenEntry::Files);
        let effect = state.dispatch(open.clone());
        assert!(matches!(effect, Effect::LoadFileIndex { .. }), "{effect:?}");
        interp.interpret_sync(&mut state, &opts, effect, &open);
        for c in "main".chars() {
            let typed = Action::QuickOpenChar(c);
            let effect = state.dispatch(typed.clone());
            interp.interpret_sync(&mut state, &opts, effect, &typed);
        }
        let quick = state.quick_open.as_ref().expect("Quick Open open");
        let FileIndexState::Ready(index) = &quick.index else {
            panic!("index not ready: {:?}", quick.index);
        };
        assert!(!quick.score_pending);
        let top = quick.hits.first().expect("a hit for main");
        assert_eq!(index.entries[top.entry].display, "src/main.rs");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn search_files_pipeline_streams_hits_from_a_real_checkout() {
        let (root, mut state) = git_app_state("ws-effect-search-files", "main", true);
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(
            root.join("app/src/main.rs"),
            "fn main() {\n    needle();\n}\nfn needle() {}\n",
        )
        .unwrap();
        std::fs::write(root.join("app/blob.bin"), b"needle\0binary").unwrap();
        state.cursor = state
            .rows
            .iter()
            .position(|row| row.repo.as_deref() == Some("app"))
            .expect("app row");
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);
        let open = Action::ToggleSearchFiles;
        let effect = state.dispatch(open.clone());
        assert!(
            matches!(effect, Effect::LoadSearchIndex { .. }),
            "{effect:?}"
        );
        interp.interpret_sync(&mut state, &opts, effect, &open);
        for c in "needle".chars() {
            let typed = Action::SearchFilesChar(c);
            let effect = state.dispatch(typed.clone());
            assert_eq!(effect, Effect::None, "typing waits for the debounce");
            interp.interpret_sync(&mut state, &opts, effect, &typed);
        }
        let due = state
            .search_files
            .as_ref()
            .and_then(|d| d.due)
            .expect("armed");
        let effect = state.fire_search_files_due(due).expect("due");
        interp.interpret_sync(&mut state, &opts, effect, &Action::None);

        let dialog = state.search_files.as_ref().expect("dialog open");
        let crate::tui::quick_open::FileIndexState::Ready(index) = &dialog.index else {
            panic!("index not ready: {:?}", dialog.index);
        };
        assert!(!dialog.searching, "search finished");
        let lines: Vec<(&str, u32, &str)> = dialog
            .hits
            .iter()
            .map(|hit| {
                (
                    index.entries[hit.entry].display.as_str(),
                    hit.line,
                    hit.text.as_str(),
                )
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                ("src/main.rs", 2, "    needle();"),
                ("src/main.rs", 4, "fn needle() {}"),
            ]
        );
        assert_eq!(dialog.skipped, 1, "the binary file");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_newer_search_stops_the_running_chunk() {
        let (root, mut state) = git_app_state("ws-effect-search-files-cancel", "main", false);
        std::fs::write(root.join("app/notes.txt"), "alpha\nbeta\n").unwrap();
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);
        let effect = state.dispatch(Action::ToggleSearchFiles);
        interp.interpret_sync(&mut state, &opts, effect, &Action::None);
        state.dispatch(Action::SearchFilesChar('a'));
        let due = state
            .search_files
            .as_ref()
            .and_then(|d| d.due)
            .expect("armed");
        let first = state.fire_search_files_due(due).expect("due");
        schedule_effect(&mut interp, &mut state, first, &Action::None);
        let running = capture_jobs(&mut interp, &mut state);
        assert_eq!(running.len(), 1);

        // Regex on: a new generation while the first chunk is on a worker.
        let toggle = Action::SearchFilesToggleOption(crate::tui::action::SearchFilesOption::Regex);
        let second = state.dispatch(toggle.clone());
        assert!(
            matches!(second, Effect::SearchFilesChunk { .. }),
            "{second:?}"
        );
        schedule_effect(&mut interp, &mut state, second, &toggle);
        for (id, work) in running {
            let outcome = work();
            match &outcome {
                JobOutcome::SearchChunk { chunk, .. } => {
                    assert!(chunk.hits.is_empty(), "stopped before the first file");
                    assert_eq!(chunk.next_entry, Some(0));
                }
                _ => panic!("expected a search chunk"),
            }
            apply_id(&mut interp, &mut state, id, outcome);
        }
        assert!(
            state.search_files.as_ref().unwrap().hits.is_empty(),
            "dropped"
        );
        run_jobs(&mut interp, &mut state);
        let dialog = state.search_files.as_ref().unwrap();
        assert!(!dialog.searching);
        let lines: Vec<u32> = dialog.hits.iter().map(|hit| hit.line).collect();
        assert_eq!(lines, vec![1, 2], "the newer search ran");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn file_tab_load_drops_stale_generation() {
        let (root, mut state) = git_app_state("ws-effect-file-tab-stale", "main", false);
        std::fs::write(root.join("app/notes.txt"), "old\n").unwrap();
        let mut interp = Interpreter::with_cap(4);
        let first = state.open_file_tab("app".into(), "notes.txt".into());
        assert!(
            matches!(first, Effect::LoadFileTab { gen: 0, .. }),
            "{first:?}"
        );
        schedule_effect(&mut interp, &mut state, first, &Action::None);
        let stale = capture_jobs(&mut interp, &mut state);
        assert_eq!(stale.len(), 1);
        let reload = state.dispatch(Action::Refresh);
        assert!(
            matches!(reload, Effect::LoadFileTab { gen: 1, .. }),
            "{reload:?}"
        );
        std::fs::write(root.join("app/notes.txt"), "fresh\nlines\n").unwrap();
        for (id, work) in stale {
            apply_id(&mut interp, &mut state, id, work());
        }
        assert!(
            state.tabs.active_file().unwrap().body.is_none(),
            "a generation-0 read is dropped after r"
        );
        let opts = opts(&state);
        interp.interpret_sync(&mut state, &opts, reload, &Action::Refresh);
        assert_eq!(
            state.tabs.active_file().unwrap().lines(),
            ["fresh", "lines"]
        );

        state.dispatch(Action::CloseTab);
        let gone = JobOutcome::FileTab {
            tab_id: 1,
            gen: 1,
            body: FileRead::Binary,
        };
        apply(&mut interp, &mut state, gone);
        assert!(state.tabs.is_workspace(), "a closed tab takes no result");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn file_tab_loads_text_binary_and_too_large() {
        let (root, mut state) = git_app_state("ws-effect-file-tab-kinds", "main", false);
        let app = root.join("app");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {\n\tprintln!();\n}\n").unwrap();
        std::fs::write(app.join("logo.bin"), b"PNG\0\x01\x02").unwrap();
        std::fs::write(app.join("big.log"), vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);
        let mut load = |state: &mut AppState, rel: &str| -> FileRead {
            let effect = state.open_file_tab("app".into(), rel.into());
            interp.interpret_sync(state, &opts, effect, &Action::None);
            let tab = state.tabs.active_file().expect("file tab");
            assert_eq!(tab.rel, rel);
            tab.body.as_deref().cloned().expect("loaded")
        };
        match load(&mut state, "src/main.rs") {
            FileRead::Text { lines, .. } => {
                assert_eq!(lines.len(), 3);
                assert_eq!(lines[1], "    println!();", "tabs expand");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(load(&mut state, "logo.bin"), FileRead::Binary);
        assert_eq!(
            load(&mut state, "big.log"),
            FileRead::TooLarge {
                bytes: MAX_FILE_BYTES + 1
            }
        );
        assert!(matches!(
            load(&mut state, "missing.txt"),
            FileRead::Failed(_)
        ));
        assert_eq!(state.tabs.len(), 5, "one tab per file");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_write_names_the_op_and_git_reason() {
        let (root, mut state) = git_app_state("ws-effect-write-fail", "main", false);
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::StashPop {
                repo: "app".into(),
                stash_ref: "stash@{0}".into(),
            },
            &Action::None,
        );
        run_jobs(&mut interp, &mut state);
        assert!(
            state
                .status
                .starts_with("stash pop failed: stash@{0} is not a valid reference"),
            "{}",
            state.status
        );
        assert_eq!(state.status.kind(), StatusKind::Error);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn push_failure_names_the_repo_and_reason() {
        let (root, mut state) = git_app_state("ws-effect-push-detached", "main", false);
        crate::testutil::git(&root.join("app"), &["checkout", "-q", "--detach"]);
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Push {
                repos: vec!["app".into()],
            },
            &Action::None,
        );
        run_jobs(&mut interp, &mut state);
        assert_eq!(
            state.status,
            "Pushed 1 repo (1 failed: app — detached HEAD cannot push)"
        );
        assert_eq!(state.status.kind(), StatusKind::Error);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pull_stash_conflict_outcome_says_the_stash_is_kept() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Pull {
                repos: vec!["app".into()],
            },
            &Action::Pull,
        );
        let jobs = capture_jobs(&mut interp, &mut state);
        apply_id(
            &mut interp,
            &mut state,
            jobs[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Pull,
                result: RepoOpResult::StashConflict,
                repo: "app".into(),
            },
        );
        assert_eq!(
            state.status,
            "app: pulled, but restoring local changes conflicted — resolve, then check `git stash list`"
        );
        assert_eq!(state.status.kind(), StatusKind::Error);
    }

    #[test]
    fn default_branch_counts_a_dirty_skip_apart() {
        let (root, mut state) = git_app_state("ws-effect-default-dirty", "feature/x", true);
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::DefaultBranch {
                repos: vec!["app".into()],
            },
            &Action::DefaultBranch,
        );
        run_jobs(&mut interp, &mut state);
        assert_eq!(state.status, "Switched 1 repo (1 skipped: dirty)");
        assert_eq!(state.status.kind(), StatusKind::Warn);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Real repo `app` whose README.md has committed lines `a b c` and a
    /// worktree edit of line 2, with that diff focused (inline).
    fn blame_fixture(tag: &str) -> (PathBuf, AppState) {
        let root = crate::testutil::unique_dir(tag);
        let dir = root.join("app");
        crate::testutil::init_repo(&dir);
        std::fs::write(dir.join("README.md"), "a\nb\nc\n").unwrap();
        crate::testutil::git(&dir, &["commit", "-q", "-am", "three lines"]);
        std::fs::write(dir.join("README.md"), "a\nB\nc\n").unwrap();
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(root.clone(), snapshot, true);
        state.diff_mode = crate::tui::split::DiffMode::Inline;
        state.cursor = state
            .rows
            .iter()
            .position(|row| row.label.contains("README.md"))
            .expect("README row");
        // Inline rows: 2 ` a` (1), 3 `-b`, 4 `+B`, 5 ` c` (3).
        state.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified("@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n"),
        );
        state.focus = FocusPane::Right;
        (root, state)
    }

    fn move_diff_cursor(interp: &mut Interpreter, state: &mut AppState, row: usize) {
        state.diff_cursor = row;
        schedule_effect(interp, state, Effect::None, &Action::Move(1));
    }

    #[test]
    fn line_blame_runs_one_job_at_a_time_for_the_latest_line() {
        let (root, mut state) = blame_fixture("ws-effect-line-blame");
        let mut interp = Interpreter::with_cap(4);
        move_diff_cursor(&mut interp, &mut state, 2);
        let first = state.line_blame_want().expect("context line asks");
        let mut jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1, "one blame job");

        // Two more moves while git runs: no new job, the slot keeps the latest.
        move_diff_cursor(&mut interp, &mut state, 3);
        move_diff_cursor(&mut interp, &mut state, 5);
        let latest = state.line_blame_want().expect("context line asks");
        assert!(capture_jobs(&mut interp, &mut state).is_empty());

        // The stale answer fills the cache, then the latest line runs.
        let (id, work) = jobs.remove(0);
        let _ = interp.take_dirty();
        apply_id(&mut interp, &mut state, id, work());
        assert!(!interp.take_dirty(), "stale answer needs no paint");
        let cached = state.line_blame.cached(&first).flatten().expect("cached");
        assert_eq!(cached.summary, "three lines");
        let next = capture_jobs(&mut interp, &mut state);
        assert_eq!(next.len(), 1, "the latest line runs after the first apply");
        for (id, work) in next {
            apply_id(&mut interp, &mut state, id, work());
        }
        assert!(interp.take_dirty(), "the focused line's answer paints");
        assert!(state.line_blame.cached(&latest).is_some());
        let (text, side) = state.painted_line_annotation().expect("annotation");
        assert_eq!(side, BlameSide::New);
        assert!(text.ends_with(" · three lines"), "{text}");

        // Back on a cached line: no git.
        move_diff_cursor(&mut interp, &mut state, 2);
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn line_blame_off_or_unfocused_spawns_nothing() {
        let (root, mut state) = blame_fixture("ws-effect-line-blame-off");
        let mut interp = Interpreter::with_cap(4);
        let effect = state.dispatch(Action::ToggleLineBlame);
        assert_eq!(state.status, "line blame off");
        schedule_effect(&mut interp, &mut state, effect, &Action::ToggleLineBlame);
        move_diff_cursor(&mut interp, &mut state, 2);
        assert!(capture_jobs(&mut interp, &mut state).is_empty(), "off");

        let effect = state.dispatch(Action::ToggleLineBlame);
        state.focus = FocusPane::Left;
        schedule_effect(&mut interp, &mut state, effect, &Action::ToggleLineBlame);
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "tree focus never blames"
        );

        // An added line shows its text with no git call.
        state.focus = FocusPane::Right;
        move_diff_cursor(&mut interp, &mut state, 4);
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
        assert_eq!(
            state.painted_line_annotation(),
            Some(("You · uncommitted".into(), BlameSide::New))
        );
        let _ = std::fs::remove_dir_all(root);
    }

    fn older_page(ids: &[&str]) -> GraphModel {
        GraphModel {
            has_more: true,
            ..mini_graph(ids)
        }
    }

    /// Workspace graph of `app` focused on the right with a pending reveal
    /// of commit `sha`.
    fn reveal_state(sha: &str) -> AppState {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        state.drill = DrillView::Graph;
        state.focus = FocusPane::Right;
        state.set_graph(older_page(&["aaa"]), "app".into(), "head-app".into());
        state.graph_reveal = Some(crate::tui::line_blame::GraphReveal {
            repo: "app".into(),
            sha: sha.into(),
            pages: 0,
            // The graph is on screen with no load pending.
            seen_load: true,
        });
        assert!(state.graph_pane_focused());
        state
    }

    /// Land the queued autoload with `page`.
    fn land_older_page(interp: &mut Interpreter, state: &mut AppState, page: GraphModel) {
        let (gen, _) = interp.autoload.clone().expect("autoload queued");
        apply(
            interp,
            state,
            JobOutcome::Autoload {
                gen,
                page,
                identity: GraphIdentity {
                    repo: "app".into(),
                    head: "head-app".into(),
                },
            },
        );
    }

    #[test]
    fn blame_reveal_loads_older_pages_until_the_commit_shows() {
        let mut state = reveal_state("ccc");
        let mut interp = Interpreter::new();
        schedule_effect(&mut interp, &mut state, Effect::None, &Action::None);
        assert!(state.graph_loading_older, "the hook asks for an older page");
        assert_eq!(state.status, LOADING_OLDER);
        land_older_page(&mut interp, &mut state, older_page(&["bbb"]));
        assert!(state.graph_loading_older, "still missing: one more page");
        land_older_page(&mut interp, &mut state, older_page(&["ccc", "ddd"]));
        assert!(!state.graph_loading_older, "found: no more pages");
        assert_eq!(state.graph_reveal, None);
        match state.focused_graph_row() {
            Some(GraphRow::Commit { commit, .. }) => assert_eq!(commit.id, "ccc"),
            other => panic!("expected the revealed commit, got {other:?}"),
        }
        assert_ne!(state.status, LOADING_OLDER);
    }

    #[test]
    fn blame_reveal_from_a_dir_row_queues_no_older_page_for_a_loaded_commit() {
        let mut app_repo = repo("app", true);
        app_repo.changes[0].path = "src/lib.rs".into();
        let snapshot = build_workspace_snapshot(&[app_repo], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.cursor = state
            .rows
            .iter()
            .position(|row| row.id == "dir:app:src")
            .expect("dir row");
        state.set_graph(older_page(&["aaa", "ccc"]), "app".into(), "head-app".into());
        let Effect::LoadFileTab { tab_id, gen, .. } =
            state.open_file_tab("app".into(), "src/lib.rs".into())
        else {
            panic!("expected a load");
        };
        assert!(state.apply_file_tab(
            tab_id,
            gen,
            FileRead::Text {
                lines: vec!["a".into()],
                max_cols: 1,
            }
        ));
        let key = state.line_blame_want().expect("cursor line asks");
        state.apply_line_blame(
            key,
            Ok(Some(crate::git::LineBlame {
                sha: "ccc".into(),
                author: "Ada".into(),
                author_time: 0,
                summary: "s-ccc".into(),
                orig_line: 1,
                filename: "src/lib.rs".into(),
                previous: None,
                boundary: false,
                uncommitted: false,
            })),
        );
        let mut interp = Interpreter::new();
        let effect = state.dispatch(Action::BlameRevealGraph);
        schedule_effect(&mut interp, &mut state, effect, &Action::BlameRevealGraph);
        assert!(!state.graph_loading_older, "no older page");
        assert!(interp.autoload.is_none());
        assert_eq!(state.graph_reveal, None);
        match state.focused_graph_row() {
            Some(GraphRow::Commit { commit, .. }) => assert_eq!(commit.id, "ccc"),
            other => panic!("expected the revealed commit, got {other:?}"),
        }
    }

    #[test]
    fn blame_reveal_stops_widening_at_the_page_cap() {
        let mut state = reveal_state("zzz");
        let mut interp = Interpreter::new();
        schedule_effect(&mut interp, &mut state, Effect::None, &Action::None);
        let mut pages = 0;
        while state.graph_loading_older {
            pages += 1;
            assert!(pages <= 20, "the chain must stop");
            let id = format!("old{pages}");
            land_older_page(&mut interp, &mut state, older_page(&[id.as_str()]));
        }
        assert_eq!(
            pages,
            usize::from(crate::tui::line_blame::GRAPH_REVEAL_MAX_PAGES)
        );
        assert_eq!(state.graph_reveal, None);
        assert_eq!(state.status, "zzz is not in the loaded graph");
        assert_eq!(state.status.kind(), StatusKind::Warn);
    }

    #[test]
    fn blame_previous_change_opens_the_earlier_commit_tab() {
        let root = crate::testutil::unique_dir("ws-effect-blame-previous");
        let dir = root.join("app");
        crate::testutil::init_repo(&dir);
        let rev = |rev: &str| crate::git::exec_git(&["rev-parse", rev], &dir);
        std::fs::write(dir.join("README.md"), "a\nb\n").unwrap();
        crate::testutil::git(&dir, &["commit", "-q", "-am", "one"]);
        let one = rev("HEAD");
        std::fs::write(dir.join("README.md"), "a\nB\n").unwrap();
        crate::testutil::git(&dir, &["commit", "-q", "-am", "two"]);
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(root.clone(), snapshot, true);
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);

        // File tab on README.md, cursor on line 2 (`B`, from `two`).
        let open = state.open_file_tab("app".into(), "README.md".into());
        interp.interpret_sync(&mut state, &opts, open, &Action::None);
        state.tabs.active_file_mut().unwrap().cursor = 1;
        interp.interpret_sync(&mut state, &opts, Effect::None, &Action::Move(1));
        let (text, _) = state.painted_line_annotation().expect("line 2 blamed");
        assert!(text.ends_with(" · two"), "{text}");

        let effect = state.dispatch(Action::BlamePreviousChange);
        assert!(
            matches!(effect, Effect::LoadBlamePrevious { .. }),
            "{effect:?}"
        );
        interp.interpret_sync(&mut state, &opts, effect, &Action::BlamePreviousChange);
        let tab = state.tabs.active_compare().expect("earlier commit tab");
        assert_eq!(tab.label(), format!("app ↔ {}^", &one[..7]));
        assert!(tab.is_pinned());
        assert!(!tab.loading, "the follow-up range load ran");
        assert_eq!(
            tab.files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["README.md"]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn blame_vs_worktree_loads_one_file_and_reloads_after_a_disk_edit() {
        let root = crate::testutil::unique_dir("ws-effect-blame-worktree");
        let dir = root.join("app");
        crate::testutil::init_repo(&dir);
        let rev = |rev: &str| crate::git::exec_git(&["rev-parse", rev], &dir);
        std::fs::write(dir.join("README.md"), "a\nb\n").unwrap();
        crate::testutil::git(&dir, &["commit", "-q", "-am", "one"]);
        let one = rev("HEAD");
        std::fs::write(dir.join("README.md"), "a\nB\n").unwrap();
        crate::testutil::git(&dir, &["commit", "-q", "-am", "two"]);
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(root.clone(), snapshot, true);
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);

        // File tab on README.md, cursor on line 1 (`a`, from `one`).
        let open = state.open_file_tab("app".into(), "README.md".into());
        interp.interpret_sync(&mut state, &opts, open, &Action::None);
        interp.interpret_sync(&mut state, &opts, Effect::None, &Action::None);
        let (text, _) = state.painted_line_annotation().expect("line 1 blamed");
        assert!(text.ends_with(" · one"), "{text}");

        let effect = state.dispatch(Action::BlameCommitVsWorktree);
        interp.interpret_sync(&mut state, &opts, effect, &Action::BlameCommitVsWorktree);
        let tab = state.tabs.active_compare().expect("worktree tab");
        assert_eq!(tab.base_ref, one);
        assert!(!tab.loading, "the range load ran");
        assert_eq!(tab.error, None);
        assert_eq!(
            tab.files
                .iter()
                .map(|f| (f.status.as_str(), f.path.as_str()))
                .collect::<Vec<_>>(),
            vec![("M", "README.md")]
        );
        assert_eq!(tab.loaded_diff_path(), Some("README.md"), "the diff loaded");
        assert!(tab.content.vs_worktree);
        assert!(tab.content.unstaged.contains("+B"), "{:?}", tab.content);

        // A disk edit shows after the checkout's next status load.
        std::fs::write(dir.join("README.md"), "a\nB\nC\n").unwrap();
        interp.interpret_sync(
            &mut state,
            &opts,
            Effect::ReloadRepo { repo: "app".into() },
            &Action::None,
        );
        let tab = state.tabs.active_compare().unwrap();
        assert!(tab.content.unstaged.contains("+C"), "{:?}", tab.content);
        let _ = std::fs::remove_dir_all(root);
    }

    const PR_URL: &str = "https://github.com/octo/demo/pull/7";

    thread_local! {
        /// URLs the fake opener was asked to open (sync pump: same thread).
        static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn opened_urls() -> Vec<String> {
        OPENED.with(|urls| urls.borrow_mut().drain(..).collect())
    }

    fn gh_approved_pr(argv: &[String], _: &AtomicBool) -> Result<String, ()> {
        assert_eq!(argv.first().map(String::as_str), Some("gh"), "{argv:?}");
        Ok(format!(
            r#"[{{"number":7,"state":"OPEN","url":"{PR_URL}","headRefName":"feature","isCrossRepository":false,"reviewDecision":"APPROVED","updatedAt":"2026-01-02T00:00:00Z"}}]"#
        ))
    }

    fn gh_no_pr(_: &[String], _: &AtomicBool) -> Result<String, ()> {
        Ok("[]".into())
    }

    fn gh_fails(_: &[String], _: &AtomicBool) -> Result<String, ()> {
        Err(())
    }

    fn cli_must_not_run(argv: &[String], _: &AtomicBool) -> Result<String, ()> {
        panic!("no forge CLI expected: {argv:?}")
    }

    fn opener_ok(url: &str, _: &AtomicBool) -> Result<(), ()> {
        OPENED.with(|urls| urls.borrow_mut().push(url.to_string()));
        Ok(())
    }

    fn opener_fails(url: &str, _: &AtomicBool) -> Result<(), ()> {
        OPENED.with(|urls| urls.borrow_mut().push(url.to_string()));
        Err(())
    }

    /// Real checkout `app` on `feature`, with `origin` at `remote` when set,
    /// focused in the tree.
    fn pr_app(tag: &str, remote: Option<&str>) -> (PathBuf, AppState) {
        let (root, mut state) = git_app_state(tag, "feature", false);
        if let Some(remote) = remote {
            crate::testutil::git(&root.join("app"), &["remote", "add", "origin", remote]);
        }
        focus_repo(&mut state, "app");
        (root, state)
    }

    /// `gx` on the focused row with fake CLI and opener; returns the status.
    fn run_gx(state: &mut AppState, cli: PrCliRunner, opener: PrUrlOpener) -> StatusMessage {
        let mut interp = Interpreter::with_cap(4);
        interp.pr_cli = cli;
        interp.pr_opener = opener;
        let effect = state.dispatch(Action::OpenPullRequest);
        assert_eq!(
            effect,
            Effect::OpenPullRequest {
                repo: PathBuf::from("app"),
                branch: "feature".into(),
            }
        );
        let opts = opts(state);
        interp.interpret_sync(state, &opts, effect, &Action::OpenPullRequest);
        state.status.clone()
    }

    #[test]
    fn open_pull_request_opens_the_found_pr_and_caches_it() {
        let (root, mut state) = pr_app("ws-effect-pr-open", Some("git@github.com:octo/demo.git"));
        let _ = opened_urls();
        let status = run_gx(&mut state, gh_approved_pr, opener_ok);
        assert_eq!(status, "opened PR #7");
        assert_eq!(status.kind(), StatusKind::Ok);
        assert_eq!(opened_urls(), vec![PR_URL.to_string()]);
        assert_eq!(
            state.pr_badge(Path::new("app")),
            Some(crate::tui::pull_request::PrState::Approved),
            "the open's fresh answer feeds the badge"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_pull_request_reports_no_pr_lookup_failure_and_open_failure() {
        let (root, mut state) = pr_app("ws-effect-pr-status", Some("git@github.com:octo/demo.git"));
        let _ = opened_urls();
        let cases: [(PrCliRunner, PrUrlOpener, &str, StatusKind, usize); 3] = [
            (
                gh_no_pr,
                opener_ok,
                "no PR for feature",
                StatusKind::Warn,
                0,
            ),
            (
                gh_fails,
                opener_ok,
                "could not look up PR for feature",
                StatusKind::Error,
                0,
            ),
            (
                gh_approved_pr,
                opener_fails,
                "could not open PR",
                StatusKind::Error,
                1,
            ),
        ];
        for (cli, opener, text, kind, opens) in cases {
            let status = run_gx(&mut state, cli, opener);
            assert_eq!(status, text);
            assert_eq!(status.kind(), kind, "{text}");
            assert_eq!(opened_urls().len(), opens, "{text}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn open_pull_request_without_a_forge_remote_says_no_pr_without_a_cli() {
        for (tag, remote) in [
            ("ws-effect-pr-no-remote", None),
            (
                "ws-effect-pr-other-forge",
                Some("https://git.example.org/o/r.git"),
            ),
        ] {
            let (root, mut state) = pr_app(tag, remote);
            let status = run_gx(&mut state, cli_must_not_run, opener_ok);
            assert_eq!(status, "no PR for feature", "{remote:?}");
            assert!(opened_urls().is_empty());
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn test_build_defaults_fail_without_a_forge_cli_or_browser() {
        let (root, mut state) = pr_app(
            "ws-effect-pr-defaults",
            Some("git@github.com:octo/demo.git"),
        );
        let mut interp = Interpreter::with_cap(4);
        let live = AtomicBool::new(false);
        assert_eq!((interp.pr_cli)(&["gh".to_string()], &live), Err(()));
        assert_eq!((interp.pr_opener)(PR_URL, &live), Err(()));
        let effect = state.dispatch(Action::OpenPullRequest);
        let opts = opts(&state);
        interp.interpret_sync(&mut state, &opts, effect, &Action::OpenPullRequest);
        assert_eq!(state.status, "could not look up PR for feature");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Answers "no PR" until the quit flag is set, then fails as a killed
    /// CLI would.
    fn gh_no_pr_until_cancelled(_: &[String], cancel: &AtomicBool) -> Result<String, ()> {
        if cancel.load(Ordering::Relaxed) {
            Err(())
        } else {
            Ok("[]".into())
        }
    }

    #[test]
    fn cancel_pull_request_jobs_sets_the_flag_pr_jobs_see() {
        let (root, mut state) = pr_app("ws-effect-pr-cancel", Some("git@github.com:octo/demo.git"));
        let mut interp = Interpreter::with_cap(4);
        interp.pr_cli = gh_no_pr_until_cancelled;
        assert!(!interp.pr_cancel.load(Ordering::Relaxed));
        let opts = opts(&state);
        let effect = state.dispatch(Action::OpenPullRequest);
        interp.interpret_sync(&mut state, &opts, effect, &Action::OpenPullRequest);
        assert_eq!(state.status, "no PR for feature");

        interp.cancel_pull_request_jobs();
        assert!(interp.pr_cancel.load(Ordering::Relaxed));
        let effect = state.dispatch(Action::OpenPullRequest);
        interp.interpret_sync(&mut state, &opts, effect, &Action::OpenPullRequest);
        assert_eq!(
            state.status, "could not look up PR for feature",
            "a job after the cancel gets the set flag"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn badge_lookups_run_two_at_a_time_and_an_open_does_not_wait() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(8);
        interp.pr_cli = cli_must_not_run;
        let targets: Vec<(PathBuf, String)> = (0..5)
            .map(|i| (PathBuf::from(format!("missing-{i}")), "feature".to_string()))
            .collect();
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LookupPullRequests { targets },
            &Action::None,
        );
        let first = capture_jobs(&mut interp, &mut state);
        assert_eq!(first.len(), 2, "badge lookups are capped");
        assert_eq!(interp.pr_badges.len(), 3);

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::OpenPullRequest {
                repo: PathBuf::from("missing-open"),
                branch: "feature".into(),
            },
            &Action::OpenPullRequest,
        );
        let open = capture_jobs(&mut interp, &mut state);
        assert_eq!(open.len(), 1, "gx runs beside two badge lookups");
        assert_eq!(interp.pr_badges.len(), 3);

        for (id, work) in first {
            let outcome = work();
            assert!(matches!(outcome, JobOutcome::PullRequestBadge { .. }));
            apply_id(&mut interp, &mut state, id, outcome);
        }
        assert_eq!(capture_jobs(&mut interp, &mut state).len(), 2);
        assert_eq!(interp.pr_badges.len(), 1);
    }

    const DETAIL_REMOTE: &str = "git@github.com:octo/demo.git";

    /// Answers `gh pr view` for PR 7 until the quit flag is set, then fails
    /// as a killed CLI would.
    fn gh_view_until_cancelled(argv: &[String], cancel: &AtomicBool) -> Result<String, ()> {
        assert_eq!(
            argv.get(..3),
            Some(&["gh".to_string(), "pr".into(), "view".into()][..])
        );
        if cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        Ok(format!(
            r#"{{"number":7,"title":"Add login","state":"OPEN","url":"{PR_URL}","statusCheckRollup":[{{"status":"COMPLETED","conclusion":"SUCCESS"}}]}}"#
        ))
    }

    /// Fixture `app` (on `main`) with a ready PR #7 and a pinned popover
    /// that asked for its detail.
    fn detail_state() -> (AppState, Effect) {
        let mut state = fixture_state();
        let _ = state.due_pr_lookups();
        assert!(state.apply_pr_lookup(
            Path::new("app"),
            "main",
            Some(DETAIL_REMOTE.into()),
            PrLookup::Found(crate::tui::pull_request::PullRequest {
                number: 7,
                url: PR_URL.into(),
                state: crate::tui::pull_request::PrState::Open,
            }),
        ));
        focus_repo(&mut state, "app");
        let effect = state.dispatch(Action::PopoverOpenFocused);
        assert_eq!(
            effect,
            Effect::LookupPullRequestDetail {
                repo: PathBuf::from("app"),
                branch: "main".into(),
                remote: DETAIL_REMOTE.into(),
                number: 7,
                request: 1,
            }
        );
        (state, effect)
    }

    #[test]
    fn detail_fetches_share_the_badge_slots_and_run_before_waiting_badges() {
        let (mut state, detail) = detail_state();
        let mut interp = Interpreter::with_cap(8);
        interp.pr_cli = gh_view_until_cancelled;
        let targets: Vec<(PathBuf, String)> = (0..3)
            .map(|i| (PathBuf::from(format!("missing-{i}")), "feature".to_string()))
            .collect();
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LookupPullRequests { targets },
            &Action::None,
        );
        let badges = capture_jobs(&mut interp, &mut state);
        assert_eq!(badges.len(), 2, "both slots busy");
        schedule_effect(&mut interp, &mut state, detail, &Action::None);
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "the detail waits for a slot"
        );
        assert_eq!(interp.pr_details.len(), 1);

        let (id, work) = badges.into_iter().next().expect("badge job");
        interp.pr_cli = cli_must_not_run;
        apply_id(&mut interp, &mut state, id, work());
        interp.pr_cli = gh_view_until_cancelled;
        let next = capture_jobs(&mut interp, &mut state);
        assert_eq!(next.len(), 1, "one slot freed");
        assert!(interp.pr_details.is_empty(), "the detail goes first");
        assert_eq!(interp.pr_badges.len(), 1, "the badge still waits");
        let (id, work) = next.into_iter().next().expect("detail job");
        let outcome = work();
        assert!(matches!(outcome, JobOutcome::PullRequestDetail { .. }));
        apply_id(&mut interp, &mut state, id, outcome);
        match state.pull_request_detail_for(Path::new("app")) {
            Some(crate::tui::state::PrDetailState::Ready(detail)) => {
                assert_eq!(detail.title, "Add login");
                assert_eq!(detail.checks.pass, 1);
            }
            other => panic!("ready detail expected: {other:?}"),
        }
        assert_eq!(
            capture_jobs(&mut interp, &mut state).len(),
            1,
            "the freed slot takes the waiting badge"
        );
    }

    #[test]
    fn a_queued_detail_fetch_the_cache_dropped_never_runs() {
        let (mut state, detail) = detail_state();
        let mut interp = Interpreter::with_cap(8);
        interp.pr_cli = gh_view_until_cancelled;
        let targets: Vec<(PathBuf, String)> = (0..2)
            .map(|i| (PathBuf::from(format!("missing-{i}")), "feature".to_string()))
            .collect();
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LookupPullRequests { targets },
            &Action::None,
        );
        let badges = capture_jobs(&mut interp, &mut state);
        assert_eq!(badges.len(), 2, "both slots busy");
        schedule_effect(&mut interp, &mut state, detail, &Action::None);
        assert_eq!(interp.pr_details.len(), 1, "the detail waits for a slot");

        // `r` drops the Loading entry while the fetch still waits.
        state.forget_pr_lookups(None);
        let (id, work) = badges.into_iter().next().expect("badge job");
        interp.pr_cli = cli_must_not_run;
        apply_id(&mut interp, &mut state, id, work());
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "the dropped detail does not run"
        );
        assert!(interp.pr_details.is_empty());
        assert_eq!(
            interp.pr_badges_running, 1,
            "only the other badge holds a slot"
        );
    }

    #[test]
    fn a_detail_fetch_after_quit_fails_without_an_answer() {
        let (mut state, detail) = detail_state();
        let mut interp = Interpreter::with_cap(4);
        interp.pr_cli = gh_view_until_cancelled;
        interp.cancel_pull_request_jobs();
        let opts = opts(&state);
        interp.interpret_sync(&mut state, &opts, detail, &Action::None);
        assert_eq!(
            state.pull_request_detail_for(Path::new("app")),
            Some(&crate::tui::state::PrDetailState::Failed),
            "the job saw the set cancel flag"
        );
        assert_eq!(interp.pr_badges_running, 0, "the slot is released");
    }

    /// Land a fresh status for checkout `app` on `branch` (collect `gen`).
    fn apply_app_status(interp: &mut Interpreter, state: &mut AppState, gen: u64, branch: &str) {
        let mut snap = repo("app", true);
        snap.branch = branch.into();
        apply(
            interp,
            state,
            JobOutcome::RepoStatus {
                gen,
                path: "app".into(),
                snap: Some(snap),
            },
        );
    }

    #[test]
    fn applied_status_looks_up_new_branches_only_and_refresh_looks_up_again() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let first = state.due_pr_lookups();
        assert_eq!(
            first,
            vec![
                (PathBuf::from("app"), "main".to_string()),
                (PathBuf::from("lib"), "main".to_string()),
            ],
            "first snapshot: every visible checkout with a branch"
        );

        apply_app_status(&mut interp, &mut state, 0, "main");
        assert!(interp.pr_badges.is_empty(), "same branch: no new lookup");

        apply_app_status(&mut interp, &mut state, 0, "feature");
        assert_eq!(
            interp.pr_badges,
            VecDeque::from([(PathBuf::from("app"), "feature".to_string())])
        );
        assert_eq!(state.pr_badge(Path::new("app")), None, "in flight");
        interp.pr_badges.clear();

        focus_repo(&mut state, "app");
        let effect = state.dispatch(Action::Refresh);
        assert_eq!(effect, Effect::ReloadRepo { repo: "app".into() });
        schedule_effect(&mut interp, &mut state, effect, &Action::Refresh);
        apply_app_status(&mut interp, &mut state, 1, "feature");
        assert_eq!(
            interp.pr_badges,
            VecDeque::from([(PathBuf::from("app"), "feature".to_string())]),
            "r forgets the checkout, so the next status looks it up again"
        );
        interp.pr_badges.clear();

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::ReloadSnapshot,
            &Action::Refresh,
        );
        assert_eq!(
            state.due_pr_lookups().len(),
            2,
            "a full reload forgets every checkout"
        );
    }
}
