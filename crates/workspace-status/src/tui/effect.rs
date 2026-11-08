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
use std::time::Instant;

use workspace_status_graph::LOADING_OLDER;

use crate::actions::{switch_repo_to_default_branch, SwitchOutcome};
use crate::config::user_config_path;
use crate::discovery::{discover_checkouts, process_repo, RepoCheckoutMeta};
use crate::file_index::{
    build_file_index, read_text_file, score_files, FileHit, FileIndex, FileRead, IndexRoot,
    MAX_FILE_BYTES, MAX_INDEX_ENTRIES, MAX_RESULTS,
};
use crate::file_search::{build_matcher, search_chunk, SearchChunk, SearchOptions};
use crate::git::{
    apply_cached_patch, apply_worktree_patch_reverse, blame_line, create_branch_at,
    create_branch_checkout, exec_git_checked, latest_stash, list_compare_picker_branches,
    list_compare_picker_commits, list_local_branches, previous_line_change, pull_quiet_detailed,
    push_plan, push_quiet, remote_url_for_branch, remove_untracked_file, remove_worktree,
    reset_keep_to_upstream, resolve_stash_target, revert_compare_file, revert_compare_patch,
    revert_tracked_file, stage_file, stash_apply, stash_drop, stash_pop, stash_push, unstage_file,
    PushPlan, StashTarget, COMPARE_REVERT_ABORTED,
};
use crate::settings::Settings;
use crate::snapshot::RepoSnapshot;

use super::action::{Action, Effect, ExternalDiffKind};
use super::activity::{activity_targets, load_activity, ACTIVITY_DAYS};
use super::anim::{BatchProgress, RemoteActivity};
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
use super::doctor::{self, Check, CollectInputs};
use super::drill::{CommitFileSource, DrillView};
#[cfg(test)]
use super::event_pump::action_triggers_graph_autoload;
use super::explorer::{list_dir, ExplorerEntry};
use super::fetch::scheduled_fetch_failure_text;
use super::fetch_claim::{try_claim_background_fetch, BackgroundFetchClaim};
use super::file_facts::{collect_repo_file_facts, RepoFileFacts};
use super::graph_load::{
    autoload_limit, autoload_skip, load_commit_card, load_graph_model_window, merge_autoload,
    should_autoload, GraphIdentity, ShouldAutoload,
};
use super::latest_only::LatestOnly;
use super::line_blame::BlameKey;
use super::ops::{
    format_completed_op, format_mixed_running_op, format_running_op, op_targets, Op, OpTally,
    RepoOpResult, RunningOp,
};
use super::pr_groups::{PrGroup, TitleCache};
use super::pull_request::{is_web_url, lookup, lookup_detail, PrDetailLookup, PrLookup, PrPage};
#[cfg(not(test))]
use super::pull_request::{open_in_browser, run_cli};
use super::remote_view::{self, RemoteTarget};
use super::scheduler::{ApplyDecision, Scheduler, SpawnKind, UserTag};
use super::search_files::SEARCH_CHUNK_BUDGET;
use super::stash::{resolve_stash_menu_key, StashMenuKeyResult, StashOpId};
use super::state::{revert_scope, AppState, PendingConfirm};
use super::status::{StatusKind, StatusMessage};
use super::store_save::{StoreKind, StoreSaved};
use super::tabs::{ComparePickerKind, ComparePickerRows, WorktreeFile};
use super::viewed::workspace_store_id;

mod job_release;

use job_release::JobRelease;
pub(crate) use job_release::{in_worker_job, note_worker_panic_at, run_job, JobResult};

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
        /// Disk facts of the dirty files of `snap`, read on the worker.
        /// `None` with no `snap`.
        facts: Option<RepoFileFacts>,
    },
    /// One viewed or comment store save: the merged bucket or the error.
    StoreSaved {
        saved: StoreSaved,
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
        latest: Option<StashTarget>,
    },
    PreparePush {
        gen: u64,
        repo: String,
        plan: Result<PushPlan, String>,
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
    /// Search-in-files preview body for preview generation `gen`.
    SearchPreview {
        gen: u64,
        body: FileRead,
    },
    /// One Explorer folder listing for request `req`. `Err` is the read
    /// error; the folder then lists empty.
    ExplorerDir {
        tab_id: u64,
        req: u64,
        rel_dir: String,
        entries: Result<Vec<ExplorerEntry>, String>,
    },
    /// Explorer preview for generation `gen`: a worktree diff or a file
    /// body.
    ExplorerPreview {
        tab_id: u64,
        gen: u64,
        body: ExplorerPreviewBody,
    },
    /// Doctor rows for probe request `gen`.
    Doctor {
        gen: u64,
        checks: Vec<Check>,
    },
    /// `git blame -L n,n` answer for one focused line.
    LineBlame {
        key: BlameKey,
        result: Result<Option<crate::git::LineBlame>, String>,
    },
    /// Card lookup for commit `sha` of checkout `repo` (a commit popover
    /// off the loaded graph).
    CommitCard {
        repo: String,
        sha: String,
        result: Result<workspace_status_graph::Commit, String>,
    },
    /// Previous change to a blamed line, for request `gen` on blame
    /// commit `sha`.
    BlamePrevious {
        gen: u64,
        repo: String,
        sha: String,
        result: Result<crate::git::PreviousLineChange, String>,
    },
    /// Git-only remote resolve of checkout `repo` on its default branch
    /// `branch`, for commit links.
    CheckoutRemote {
        repo: PathBuf,
        branch: String,
        remote: Option<String>,
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
    /// `gx` (or `gc`) lookup of `branch` in checkout `repo`, then the
    /// browser open of `page`. `opened` is true when the browser started
    /// for a found PR.
    PullRequestOpen {
        repo: PathBuf,
        branch: String,
        remote: Option<String>,
        lookup: PrLookup,
        page: PrPage,
        opened: bool,
    },
    /// Titles of one PR title command fill, by group key.
    PrTitles {
        titles: TitleCache,
        /// The quit flag stopped the fill before its last key.
        cancelled: bool,
    },
    /// `gc` browser open of the CI page of the known PR `pr_ref`. `opened`
    /// is true when the browser started.
    PullRequestCiOpen {
        pr_ref: String,
        opened: bool,
    },
    /// "View remotely" check, then the browser open. `page` is the page URL,
    /// or the status line that says why there is none; `opened` is true
    /// when the browser started for it.
    RemoteView {
        page: Result<String, String>,
        opened: bool,
    },
    /// Host clipboard tool copy of one [`Effect::CopyClipboard`]. `ok` is
    /// the copy result to show; `None` when OSC 52 already showed a
    /// success on the loop thread.
    ClipboardCopied {
        ok: Option<bool>,
        announce: bool,
    },
    /// Day commit counts of activity key `key` for its tree sparkline.
    /// `None` when the job saw the quit flag and ran no git.
    Activity {
        key: String,
        days: Option<[u32; ACTIVITY_DAYS]>,
    },
}

/// Loaded Explorer preview of one file.
#[derive(Debug)]
pub(crate) enum ExplorerPreviewBody {
    /// Worktree diff of a changed file.
    Diff(super::diff::DiffContent),
    /// Body of a clean file.
    File(FileRead),
}

/// Queued Explorer pool job: the fields of [`Effect::LoadExplorerDir`] or
/// [`Effect::LoadExplorerPreview`].
#[derive(Clone, Debug)]
enum ExplorerJob {
    Dir {
        tab_id: u64,
        req: u64,
        repo: String,
        rel_dir: String,
        parent_ignored: bool,
    },
    Preview {
        tab_id: u64,
        gen: u64,
        repo: String,
        path: String,
        change: Option<crate::snapshot::FileChange>,
    },
}

/// Run one Explorer job on a worker. Blocking: folder read plus one
/// `git check-ignore`, a file read, or `git diff`.
fn run_explorer_job(cwd: &Path, job: ExplorerJob) -> JobOutcome {
    match job {
        ExplorerJob::Dir {
            tab_id,
            req,
            repo,
            rel_dir,
            parent_ignored,
        } => {
            let entries = list_dir(&cwd.join(&repo), &rel_dir, parent_ignored).map_err(|err| {
                let folder = if rel_dir.is_empty() { &repo } else { &rel_dir };
                format!("{folder}: {err}")
            });
            JobOutcome::ExplorerDir {
                tab_id,
                req,
                rel_dir,
                entries,
            }
        }
        ExplorerJob::Preview {
            tab_id,
            gen,
            repo,
            path,
            change,
        } => {
            // The Explorer always shows the whole file of a changed file.
            let body = match change {
                Some(change) => ExplorerPreviewBody::Diff(super::diff::load_file_diff(
                    cwd,
                    &repo,
                    &change,
                    Some(crate::git::FULL_DIFF_CONTEXT_LINES),
                )),
                None => ExplorerPreviewBody::File(read_text_file(
                    &cwd.join(&repo).join(&path),
                    MAX_FILE_BYTES,
                )),
            };
            JobOutcome::ExplorerPreview { tab_id, gen, body }
        }
    }
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

/// Forge CLI runner for PR lookups: argv and the quit flag in, stdout or
/// the failure reason out.
type PrCliRunner = fn(&[String], &AtomicBool) -> Result<String, Option<String>>;

/// Opens a PR URL in the operator's browser; stops waiting on the quit flag.
type PrUrlOpener = fn(&str, &AtomicBool) -> Result<(), ()>;

/// Forge CLI runner a new [`Interpreter`] starts with.
#[cfg(not(test))]
const DEFAULT_PR_CLI: PrCliRunner = run_cli;

/// Test builds never spawn `gh` / `glab`: the default fails without a
/// spawn, and a test that needs an answer sets a fake.
#[cfg(test)]
const DEFAULT_PR_CLI: PrCliRunner = |_, _| Err(None);

/// Browser opener a new [`Interpreter`] starts with.
#[cfg(not(test))]
const DEFAULT_PR_OPENER: PrUrlOpener = open_in_browser;

/// Test builds never start a browser: the default fails without a spawn,
/// and a test that needs an open sets a fake.
#[cfg(test)]
const DEFAULT_PR_OPENER: PrUrlOpener = |_, _| Err(());

/// OSC 52 clipboard write on the loop thread: the text in, success out.
type Osc52Writer = fn(&str) -> bool;

/// Host clipboard copy: the text and the deadline in, success out.
type ClipboardTool = fn(&str, Instant) -> bool;

/// OSC 52 writer a new [`Interpreter`] starts with.
#[cfg(not(test))]
const DEFAULT_OSC52_WRITER: Osc52Writer = comments::write_osc52;

/// Test builds never write OSC 52 to the terminal that runs them: the
/// default fails without a write, and a test that needs one sets a fake.
#[cfg(test)]
const DEFAULT_OSC52_WRITER: Osc52Writer = |_| false;

/// Host clipboard copy a new [`Interpreter`] starts with.
#[cfg(not(test))]
const DEFAULT_CLIPBOARD_TOOL: ClipboardTool = comments::copy_with_host_tool;

/// Test builds never spawn `wl-copy` / `xclip` / `pbcopy`: the default fails
/// without a spawn, and a test that needs a copy sets a fake.
#[cfg(test)]
const DEFAULT_CLIPBOARD_TOOL: ClipboardTool = |_, _| false;

/// One queued host clipboard copy: the fields of [`Effect::CopyClipboard`]
/// plus the OSC 52 result the loop thread already showed.
#[derive(Clone, Debug)]
struct ClipboardJob {
    text: String,
    announce: bool,
    /// OSC 52 succeeded, so the copy result is already on screen and the
    /// host tool's result changes nothing.
    osc52: bool,
}

/// Show the result `ok` of one clipboard copy: the `y` export overlay
/// header (once per overlay), and the status line when `announce`.
fn show_copy_result(state: &mut AppState, ok: bool, announce: bool) {
    // `y` opens the export overlay right before this copy; its header
    // shows the result.
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
}

/// Most badge lookups and popover detail fetches on workers at once, so
/// pane loads are not starved. A `gx` open does not wait for this cap.
const PR_BADGE_LOOKUPS_MAX: usize = 2;

/// Longest a PR change waits for the badge lookups to go idle (ms): a
/// landing after that alerts it even while lookups keep coming.
const PR_ALERT_WAIT_MS: u64 = 2000;

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
    /// A key (not only the background fetch tick) started this wave. Only
    /// such a wave writes its result to the status slot.
    foreground: bool,
    /// The background fetch tick started or joined this wave. Its end
    /// starts or ends the scheduled-fetch outage.
    scheduled: bool,
}

impl KindWave {
    fn empty() -> Self {
        Self {
            tally: OpTally::default(),
            repos: Vec::new(),
            foreground: false,
            scheduled: false,
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

/// Final status of an exclusive write named `op`. Success has `ok_kind`:
/// [`StatusKind::Ok`] for a one-shot ack (a toast), [`StatusKind::Info`] for
/// a note that must stay readable on the status line.
///
/// A compare-tab refusal ([`COMPARE_REVERT_ABORTED`]) keeps its own copy as
/// a warn: git did not fail, the write never ran.
fn write_status(op: &str, ok_kind: StatusKind, result: Result<String, String>) -> StatusMessage {
    match result {
        Ok(text) => StatusMessage::new(ok_kind, text),
        Err(err) if err.starts_with(COMPARE_REVERT_ABORTED) => StatusMessage::warn(err),
        Err(err) => StatusMessage::error(format!("{op} failed: {err}")),
    }
}

/// Ok status after the reset confirm moved `branch` to `upstream`. The undo
/// puts the branch back on `old_tip` (first 12 hex chars).
fn reset_done_status(branch: &str, upstream: &str, old_tip: &str) -> String {
    let short = old_tip.get(..12).unwrap_or(old_tip);
    format!("Moved {branch} to {upstream} · undo: git reset --keep {short}")
}

/// Bulk pull result for one repo.
fn pull_result(dir: &std::path::Path) -> RepoOpResult {
    let pulled = pull_quiet_detailed(dir);
    if let Some(note) = pulled.stash_note {
        RepoOpResult::Failed(note)
    } else if pulled.stash_pop_failed {
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
    /// Status kind of the success text (see [`write_status`]).
    ok_kind: StatusKind,
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

/// Owned Doctor probe inputs, copied out of the state and the launch
/// options on the event thread ([`Effect::RunDoctor`]).
#[derive(Debug)]
struct DoctorJob {
    term_cols: u16,
    term_rows: u16,
    mouse_enabled: bool,
    /// Every loaded checkout (hidden ones too), as an absolute path and
    /// the branch the snapshot names (detached or empty is fine: the
    /// remote lookup then takes the first remote).
    checkouts: Vec<(PathBuf, String)>,
    settings: Settings,
    editor_config: Option<String>,
    diff_tool_config: Option<String>,
    workspace_root: PathBuf,
}

impl DoctorJob {
    /// Copy the probe inputs out of `state` and `opts`. Runs on the event
    /// thread, so it reads memory only: no git, no file.
    fn new(state: &AppState, opts: &TuiOpts) -> Self {
        Self {
            term_cols: state.layout.term_cols,
            term_rows: state.layout.term_rows,
            mouse_enabled: state.mouse_enabled,
            checkouts: state
                .snapshot
                .repos
                .iter()
                .map(|row| (opts.cwd.join(&row.repo), row.branch.clone()))
                .collect(),
            settings: opts.settings.clone(),
            editor_config: opts.config.editor.clone(),
            diff_tool_config: opts.config.diff_tool.clone(),
            workspace_root: opts.cwd.clone(),
        }
    }

    /// Remote URLs of the checkouts, read with read-only git
    /// ([`remote_url_for_branch`]), without duplicates. Blocks.
    fn remote_urls(&self) -> Vec<String> {
        let mut urls: Vec<String> = Vec::new();
        for (dir, branch) in &self.checkouts {
            if let Some(url) = remote_url_for_branch(dir, branch) {
                if !urls.contains(&url) {
                    urls.push(url);
                }
            }
        }
        urls
    }

    /// Probe the setup and build the rows. Blocks (PATH scan, read-only
    /// commands, file reads), so it runs only on a worker. Writes nothing.
    fn run(self) -> Vec<Check> {
        let user_config = user_config_path();
        let remote_urls = self.remote_urls();
        let probe = doctor::collect(&CollectInputs {
            term_cols: self.term_cols,
            term_rows: self.term_rows,
            settings: &self.settings,
            mouse_enabled: self.mouse_enabled,
            editor_config: self.editor_config.as_deref(),
            diff_tool_config: self.diff_tool_config.as_deref(),
            remote_urls: &remote_urls,
            workspace_root: &self.workspace_root,
            user_config: user_config.as_deref(),
        });
        doctor::checks(&probe)
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
            // Push goes on the per-gitdir remote queue; it is not an exclusive write.
            Some(PendingConfirm::Push { .. }) => Vec::new(),
            // The reset is an exclusive write; `Y` only keeps its box open.
            Some(PendingConfirm::ResetToUpstream { repo, .. }) => {
                if matches!(action, Action::ConfirmYesClean) {
                    Vec::new()
                } else {
                    vec![repo.clone()]
                }
            }
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
    prepare_push: Option<(u64, String)>,
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
    file_index_job: LatestOnly<(u64, Vec<IndexRoot>)>,
    /// Latest Quick Open score request; a newer one replaces it before spawn.
    file_score_job: LatestOnly<(u64, Arc<FileIndex>, String)>,
    /// Queued Explorer folder listings, one per enqueue.
    explorer_jobs: VecDeque<ExplorerJob>,
    /// Queued Explorer inner-tab loads, at most one per `(tab, path)`. A
    /// new load first drops each queued one whose generation no inner tab
    /// holds any more, so a held `j` (each move replacing the preview tab)
    /// queues one read.
    explorer_previews: Vec<ExplorerJob>,
    /// Latest search-in-files index request; a newer one replaces it
    /// before spawn.
    search_index_job: LatestOnly<(u64, Vec<IndexRoot>)>,
    /// Latest search-in-files chunk request; a newer one replaces it
    /// before spawn.
    search_chunk_job: LatestOnly<SearchChunkJob>,
    /// Latest search-in-files preview read: generation, checkout, path. A
    /// newer one replaces it before spawn.
    search_preview_job: LatestOnly<(u64, String, String)>,
    /// Generation of the search that may still run (0: none). Synced from
    /// the state after every change; a running chunk stops between files
    /// once it no longer names the chunk's own generation.
    search_live: Arc<AtomicU64>,
    /// Latest focused-line blame question; a newer one replaces it
    /// before spawn, so cursor moves never queue a blame per line. At most
    /// one job is queued or running.
    line_blame_job: LatestOnly<BlameKey>,
    /// Latest commit popover card to look up (checkout, full id); a newer
    /// one replaces it before spawn. At most one job is queued or running.
    commit_card_job: LatestOnly<(String, String)>,
    /// Latest Doctor probe request; a newer one replaces it before spawn.
    /// At most one job is queued or running.
    doctor_job: LatestOnly<u64>,
    /// Viewed-store save slot (serial: one save of the file at a time).
    /// The job takes the store at spawn ([`AppState::begin_store_save`]),
    /// so it always sends every change made so far.
    viewed_save_job: LatestOnly<()>,
    /// Comment-store save slot. Same rules as [`Self::viewed_save_job`].
    comment_save_job: LatestOnly<()>,
    /// Latest host clipboard copy; a newer one replaces a waiting one, so
    /// the clipboard ends on the newest text. At most one job is queued or
    /// running.
    clipboard_job: LatestOnly<ClipboardJob>,
    /// OSC 52 writer ([`DEFAULT_OSC52_WRITER`]; tests swap in a fake).
    osc52_writer: Osc52Writer,
    /// Host clipboard copy ([`DEFAULT_CLIPBOARD_TOOL`]; tests swap in a
    /// fake).
    clipboard_tool: ClipboardTool,
    /// Latest blame previous-change request; a newer one replaces it
    /// before spawn.
    blame_previous: Option<(u64, String, Box<crate::git::LineBlame>)>,
    /// Queued `gx` opens, and `gc` opens that look the PR up first:
    /// checkout, branch, page. Each has its own slot.
    pr_opens: VecDeque<(PathBuf, String, PrPage)>,
    /// Queued `gc` opens of a known PR: its ref and CI URL. Each has its
    /// own [`UserTag::PullRequest`] slot and runs after the queued `gx`
    /// opens.
    pr_ci_opens: VecDeque<(String, String)>,
    /// Queued "View remotely" opens: checkout, target. Each has its own
    /// [`UserTag::PullRequest`] slot and runs after the queued `gx` and
    /// `gc` opens.
    remote_views: VecDeque<(PathBuf, RemoteTarget)>,
    /// Queued git-only remote resolves of checkouts on their default
    /// branch: checkout, branch. Each has its own
    /// [`UserTag::PullRequest`] slot and runs after the "View remotely"
    /// opens.
    checkout_remotes: VecDeque<(PathBuf, String)>,
    /// Queued badge lookups: checkout, branch. A checkout and branch that
    /// is queued or running is not queued again.
    pr_badges: VecDeque<(PathBuf, String)>,
    /// Badge lookups on a worker: checkout, branch.
    pr_badge_jobs: HashSet<(PathBuf, String)>,
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
    /// once, so the runtime shutdown does not wait for them; a title fill
    /// runs no further key.
    pr_cancel: Arc<AtomicBool>,
    /// Latest PR title fill: the title command and the PRs tab groups. A
    /// newer one replaces it before spawn.
    pr_title_job: LatestOnly<(PathBuf, Vec<PrGroup>)>,
    /// Activity keys waiting for a sparkline count, one queued
    /// [`UserTag::Activity`] slot each; a slot takes the front key.
    activity_pending: VecDeque<String>,
    /// Set on quit: activity jobs not yet started run no git.
    activity_cancel: Arc<AtomicBool>,
    /// A full reload (`r`) waits to land; its collect queues the counts.
    activity_after_reload: bool,
    exclusive_inflight: HashMap<u64, Vec<String>>,
    /// Name of the exclusive write or default-branch job on a worker.
    running_write: Option<&'static str>,
    /// Workspace claim of the scheduled background fetch this instance
    /// runs. Held while that fetch wave is live; the wave end records its
    /// outcome and releases it.
    fetch_claim: Option<BackgroundFetchClaim>,
    /// What each job on a worker holds, by job id, so a job that panics
    /// still releases it ([`Self::apply_panic`]).
    job_releases: HashMap<u64, JobRelease>,
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
            prepare_push: None,
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
            file_index_job: LatestOnly::overlap(UserTag::QuickOpen),
            file_score_job: LatestOnly::overlap(UserTag::QuickOpen),
            explorer_jobs: VecDeque::new(),
            explorer_previews: Vec::new(),
            search_index_job: LatestOnly::overlap(UserTag::SearchFiles),
            search_chunk_job: LatestOnly::overlap(UserTag::SearchFiles),
            search_preview_job: LatestOnly::overlap(UserTag::SearchFiles),
            search_live: Arc::new(AtomicU64::new(0)),
            line_blame_job: LatestOnly::serial(UserTag::LineBlame),
            commit_card_job: LatestOnly::serial(UserTag::CommitCard),
            doctor_job: LatestOnly::serial(UserTag::Doctor),
            viewed_save_job: LatestOnly::serial(UserTag::StoreSave(StoreKind::Viewed)),
            comment_save_job: LatestOnly::serial(UserTag::StoreSave(StoreKind::Comments)),
            clipboard_job: LatestOnly::serial(UserTag::Clipboard),
            osc52_writer: DEFAULT_OSC52_WRITER,
            clipboard_tool: DEFAULT_CLIPBOARD_TOOL,
            blame_previous: None,
            pr_opens: VecDeque::new(),
            pr_ci_opens: VecDeque::new(),
            remote_views: VecDeque::new(),
            checkout_remotes: VecDeque::new(),
            pr_badges: VecDeque::new(),
            pr_badge_jobs: HashSet::new(),
            pr_details: VecDeque::new(),
            pr_badges_running: 0,
            pr_cli: DEFAULT_PR_CLI,
            pr_opener: DEFAULT_PR_OPENER,
            pr_cancel: Arc::new(AtomicBool::new(false)),
            pr_title_job: LatestOnly::overlap(UserTag::PrTitles),
            activity_pending: VecDeque::new(),
            activity_cancel: Arc::new(AtomicBool::new(false)),
            activity_after_reload: false,
            exclusive_inflight: HashMap::new(),
            running_write: None,
            fetch_claim: None,
            job_releases: HashMap::new(),
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

    /// Run the scheduled background fetch tick unless another `ws` instance
    /// in this workspace runs it.
    ///
    /// `fetch_ms == 0` and a claim this instance already holds skip it. Else
    /// it tries the workspace claim ([`try_claim_background_fetch`]) with
    /// `fetch_ms` and wall-clock `now_unix_ms`. No claim (another instance
    /// holds it, or the last background fetch is recent): no fetch, no
    /// status change. Claimed: dispatch [`Action::FetchTick`]; keep the claim
    /// while its fetch wave is live, else drop it (released, no record). The
    /// claim cannot be tried (I/O error): fetch as before, with no claim and
    /// no record.
    pub(crate) fn background_fetch_tick(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        fetch_ms: u64,
        now_unix_ms: u64,
    ) {
        if fetch_ms == 0 || self.fetch_claim.is_some() {
            return;
        }
        let workspace_id = workspace_store_id(&state.cwd);
        let claim = match try_claim_background_fetch(
            &state.fetch_store_path,
            &workspace_id,
            fetch_ms,
            now_unix_ms,
        ) {
            Ok(None) => return,
            Ok(Some(claim)) => Some(claim),
            // A permanent stop of the background fetch is worse than a
            // duplicate one: fetch without dedupe.
            Err(_) => None,
        };
        let effect = state.dispatch(Action::FetchTick);
        let fetches = matches!(effect, Effect::Fetch { .. });
        self.schedule(state, opts, effect, &Action::FetchTick);
        if fetches && self.kind_live(RunningOp::Fetch) {
            self.fetch_claim = claim;
        }
    }

    /// Run the PR poll cycle that starts at `now`: queue badge lookups for
    /// the pollable checkouts that are due ([`AppState::due_pr_polls`]).
    /// They share the badge lookup queue, its dedupe, and its cap.
    pub(crate) fn pr_poll_tick(&mut self, state: &mut AppState, opts: &TuiOpts, now: Instant) {
        let targets = state.due_pr_polls(now);
        if !targets.is_empty() {
            self.schedule(
                state,
                opts,
                Effect::LookupPullRequests { targets },
                &Action::None,
            );
        }
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
                // The reset box `p` just opened sets no status, so it counts
                // as idle too (a modal box cannot be open before `p` runs).
                let reset_box =
                    matches!(state.confirm, Some(PendingConfirm::ResetToUpstream { .. }));
                if matches!(action, Action::Pull)
                    && (reset_box || is_idle_pull_status(&state.status))
                {
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
                // Count once the reload lands, so repos it finds get bars.
                self.activity_after_reload = true;
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
            // Each stash write first finds the entry's current `stash@{n}`
            // by its commit id: a stash pushed or dropped since the pick
            // moves `n`, and `git stash pop` / `drop` take only a ref.
            Effect::StashApply { repo, stash } => {
                let dir = opts.cwd.join(&repo);
                self.enqueue_write(
                    state,
                    "stash apply",
                    &[&repo],
                    Box::new(move || {
                        let stash_ref = resolve_stash_target(&dir, &stash)?;
                        stash_apply(&dir, &stash_ref).map(|_| format!("applied {stash_ref}"))
                    }),
                );
            }
            Effect::StashPop { repo, stash } => {
                let dir = opts.cwd.join(&repo);
                self.enqueue_write(
                    state,
                    "stash pop",
                    &[&repo],
                    Box::new(move || {
                        let stash_ref = resolve_stash_target(&dir, &stash)?;
                        stash_pop(&dir, &stash_ref).map(|_| format!("popped {stash_ref}"))
                    }),
                );
            }
            Effect::StashDrop { repo, stash } => {
                let dir = opts.cwd.join(&repo);
                self.enqueue_write(
                    state,
                    "stash drop",
                    &[&repo],
                    Box::new(move || {
                        let stash_ref = resolve_stash_target(&dir, &stash)?;
                        stash_drop(&dir, &stash_ref).map(|_| format!("dropped {stash_ref}"))
                    }),
                );
            }
            Effect::PrepareStashMenu { repo } => {
                let gen = self.sched.request_prepare_stash();
                self.prepare_stash = Some((gen, repo));
                self.sched.enqueue_user(UserTag::Prepare);
            }
            Effect::PreparePush { repo } => {
                let gen = self.sched.request_prepare_push();
                self.prepare_push = Some((gen, repo));
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
            Effect::ResetToUpstream {
                repo,
                branch,
                upstream,
                tip,
            } => {
                // Exclusive write on the gitdir: serial with other writes and
                // with the gitdir's remote queue. The guards run on the worker.
                // Success is an info note, not an ok toast: a toast clips the
                // undo sha at the pane edge.
                let dir = opts.cwd.join(&repo);
                self.enqueue_write_as(
                    state,
                    "reset",
                    StatusKind::Info,
                    &[&repo],
                    Box::new(move || {
                        reset_keep_to_upstream(&dir, &branch, &upstream, &tip)
                            .map(|old_tip| reset_done_status(&branch, &upstream, &old_tip))
                    }),
                );
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
                self.file_index_job.offer((gen, roots), &mut self.sched);
            }
            Effect::ScoreFiles { gen, index, query } => {
                self.file_score_job
                    .offer((gen, index, query), &mut self.sched);
            }
            Effect::LoadSearchIndex { gen, roots } => {
                self.search_index_job.offer((gen, roots), &mut self.sched);
            }
            Effect::SearchFilesChunk {
                gen,
                index,
                query,
                options,
                start,
                room,
            } => {
                let job = SearchChunkJob {
                    gen,
                    index,
                    query,
                    options,
                    start,
                    room,
                };
                self.search_chunk_job.offer(job, &mut self.sched);
            }
            Effect::LoadExplorerDir {
                tab_id,
                req,
                repo,
                rel_dir,
                parent_ignored,
            } => {
                self.explorer_jobs.push_back(ExplorerJob::Dir {
                    tab_id,
                    req,
                    repo,
                    rel_dir,
                    parent_ignored,
                });
                self.sched.enqueue_user(UserTag::QuickOpen);
            }
            Effect::LoadExplorerPreview {
                tab_id,
                gen,
                repo,
                path,
                change,
            } => {
                // Drop queued loads no inner tab waits for any more (a
                // replaced preview, a closed inner tab or Explorer tab), so
                // a held `j` reads only the last file while a pinned tab's
                // queued load still runs.
                self.explorer_previews.retain(|queued| match queued {
                    ExplorerJob::Preview { tab_id, gen, .. } => state
                        .tabs
                        .get_explorer_id(*tab_id)
                        .is_some_and(|tab| tab.has_gen(*gen)),
                    ExplorerJob::Dir { .. } => true,
                });
                let same_file = |queued: &ExplorerJob| {
                    matches!(queued, ExplorerJob::Preview { tab_id: id, path: queued_path, .. }
                        if *id == tab_id && *queued_path == path)
                };
                let job = ExplorerJob::Preview {
                    tab_id,
                    gen,
                    repo,
                    path: path.clone(),
                    change,
                };
                if let Some(slot) = self
                    .explorer_previews
                    .iter_mut()
                    .find(|queued| same_file(queued))
                {
                    *slot = job;
                } else {
                    self.explorer_previews.push(job);
                    self.sched.enqueue_user(UserTag::QuickOpen);
                }
            }
            Effect::OpenPullRequest { repo, branch, page } => {
                self.pr_opens.push_back((repo, branch, page));
                self.sched.enqueue_user(UserTag::PullRequest);
            }
            Effect::OpenPullRequestCi { pr_ref, url } => {
                self.pr_ci_opens.push_back((pr_ref, url));
                self.sched.enqueue_user(UserTag::PullRequest);
            }
            Effect::ViewRemotely { checkout, target } => {
                self.remote_views.push_back((checkout, target));
                self.sched.enqueue_user(UserTag::PullRequest);
            }
            Effect::ResolveCheckoutRemotes { targets } => {
                for target in targets {
                    if !self.checkout_remotes.contains(&target) {
                        self.checkout_remotes.push_back(target);
                        self.sched.enqueue_user(UserTag::PullRequest);
                    }
                }
            }
            Effect::LookupPullRequests { targets } => {
                for target in targets {
                    if !self.pr_badges.contains(&target) && !self.pr_badge_jobs.contains(&target) {
                        self.pr_badges.push_back(target);
                    }
                }
                self.fill_pr_badge_slots();
            }
            Effect::LoadActivity { checkouts } => self.queue_activity(state, checkouts),
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
            Effect::RunDoctor { gen } => self.doctor_job.offer(gen, &mut self.sched),
            Effect::FillPrTitles { cmd, groups } => {
                // The state runs one fill at a time; a queued slot only
                // takes the newer groups.
                self.pr_title_job.offer((cmd, groups), &mut self.sched);
            }
            Effect::CopyClipboard { text, announce } => {
                // OSC 52 is a terminal write between frames. The host tool
                // is a process that can hang, so it runs on a worker.
                let osc52 = (self.osc52_writer)(&text);
                if osc52 {
                    // One working path makes the copy a success.
                    show_copy_result(state, true, announce);
                    self.mark();
                }
                self.clipboard_job.offer(
                    ClipboardJob {
                        text,
                        announce,
                        osc52,
                    },
                    &mut self.sched,
                );
            }
        }
    }

    /// Jobs the state asks for after any schedule or apply: the
    /// search-in-files preview read, the focused line's blame, the card of
    /// an open commit popover off the graph, an older graph page for a
    /// pending blame reveal (dropped once the graph pane loses focus), and
    /// the viewed / comment store saves the state asks for.
    fn after_change(&mut self, state: &mut AppState) {
        self.queue_store_saves(state);
        self.search_live
            .store(state.search_files_live_gen(), Ordering::Relaxed);
        if let Some(job) = state.search_preview_request() {
            self.search_preview_job.offer(job, &mut self.sched);
        }
        self.maybe_queue_line_blame(state);
        self.maybe_queue_commit_card(state);
        state.drop_graph_reveal_off_graph();
        if Self::autoload_allowed(state) && state.graph_reveal_wants_older() {
            self.queue_autoload(state);
        }
    }

    /// Queue a save of each store whose save the state asks for and that
    /// has none running ([`AppState::store_save_ready`]). The serial slot
    /// holds at most one job per store file; the landing of a save calls
    /// this again for changes made while it ran.
    fn queue_store_saves(&mut self, state: &AppState) {
        for kind in StoreKind::ALL {
            if state.store_save_ready(kind) {
                let (slot, sched) = self.store_save_slot(kind);
                slot.offer((), sched);
            }
        }
    }

    /// Save slot of store `kind`, with the scheduler it queues on.
    fn store_save_slot(&mut self, kind: StoreKind) -> (&mut LatestOnly<()>, &mut Scheduler) {
        let slot = match kind {
            StoreKind::Viewed => &mut self.viewed_save_job,
            StoreKind::Comments => &mut self.comment_save_job,
        };
        (slot, &mut self.sched)
    }

    /// Land store save `saved`: free its slot, then apply the merged
    /// bucket or the error ([`AppState::land_store_save`]). `after_change`
    /// queues the next save when a change came in while this one ran.
    fn land_store_save(&mut self, state: &mut AppState, saved: StoreSaved) {
        let (slot, sched) = self.store_save_slot(saved.kind());
        slot.finish(sched);
        state.land_store_save(saved);
        self.mark();
    }

    /// Queue a sparkline count for each of `checkouts` (activity keys).
    ///
    /// One low-priority [`UserTag::Activity`] slot per repo, so user jobs
    /// take the pool between repos. A key that already waits is not
    /// queued again, so a reload and a Pull wave count a repo once. While
    /// the sparkline is off ([`AppState::activity_sparkline`]) nothing is
    /// queued: the launch, a reload, and a Pull wave run no count.
    fn queue_activity(&mut self, state: &AppState, checkouts: Vec<String>) {
        if !state.activity_sparkline {
            return;
        }
        for key in checkouts {
            if !self.activity_pending.contains(&key) {
                self.activity_pending.push_back(key);
                self.sched.enqueue_background(UserTag::Activity);
            }
        }
    }

    /// Stop sparkline counts: a job not yet started runs no git, and no
    /// waiting repo starts. The live loop calls this on quit, so the
    /// runtime shutdown does not wait for a walk over every repo.
    pub(crate) fn cancel_activity_jobs(&self) {
        self.activity_cancel.store(true, Ordering::Relaxed);
    }

    /// Queue a blame for the focused line when the cache has no answer.
    ///
    /// Latest-only ([`LatestOnly`], serial): the slot holds the newest
    /// question, and at most one job is queued or running. Its apply calls
    /// this again, so a cursor that moved on while git ran gets its own
    /// answer next.
    fn maybe_queue_line_blame(&mut self, state: &AppState) {
        let Some(key) = state.line_blame_want() else {
            self.line_blame_job.clear();
            return;
        };
        if state.line_blame.cached(&key).is_some() {
            return;
        }
        self.line_blame_job.offer(key, &mut self.sched);
    }

    /// Queue a card lookup for the open commit popover when neither the
    /// graph nor the cache has its commit ([`AppState::commit_card_want`]).
    ///
    /// Latest-only ([`LatestOnly`], serial): the slot holds the newest
    /// commit, and at most one job is queued or running. Its apply calls
    /// this again, so a popover that moved on while git ran gets its own
    /// card next.
    fn maybe_queue_commit_card(&mut self, state: &AppState) {
        let Some(want) = state.commit_card_want() else {
            self.commit_card_job.clear();
            return;
        };
        self.commit_card_job.offer(want, &mut self.sched);
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
    ///
    /// Each spawn records what its job holds ([`JobRelease`]), so a job
    /// that panics still releases it.
    pub(crate) fn spawn_ready(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        spawn: &mut dyn FnMut(u64, JobWork),
    ) {
        let mut started = Vec::new();
        let mut start = |id: u64, release: JobRelease, work: JobWork| {
            started.push((id, release));
            spawn(id, work);
        };
        while let Some(req) = self.sched.spawn_next() {
            let id = req.id;
            match req.kind {
                SpawnKind::Discover { gen, .. } => {
                    let cwd = opts.cwd.clone();
                    let config = discover_config(&opts.config);
                    let filter = state.snapshot.filter_repos.clone();
                    start(
                        id,
                        JobRelease::Discover { gen },
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
                    let show_hidden = state.show_hidden;
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
                    start(
                        id,
                        JobRelease::RepoStatus {
                            gen,
                            path: path.clone(),
                        },
                        Box::new(move || {
                            let snap = if let Some((meta, override_name)) = meta {
                                process_repo(&path, &cwd, false, override_name.as_deref(), &meta)
                            } else {
                                let next = compute_reload_repo(&cwd, &snapshot, &path, show_hidden);
                                next.repos
                                    .into_iter()
                                    .find(|row| row.repo == path)
                                    .map(|row| RepoSnapshot {
                                        repo: row.repo,
                                        branch: row.branch,
                                        sync_status: row.sync_status,
                                        sync_note: row.sync_note,
                                        upstream_rewrite: row.upstream_rewrite,
                                        fetch_error: row.fetch_error,
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
                                        default_branch: row.default_branch,
                                        local_branches: row.local_branches,
                                    })
                            };
                            // Disk facts of the dirty files, read here so the
                            // reducer never stats or reads them.
                            let facts = snap
                                .as_ref()
                                .map(|snap| collect_repo_file_facts(&cwd, &path, &snap.changes));
                            JobOutcome::RepoStatus {
                                gen,
                                path,
                                snap,
                                facts,
                            }
                        }),
                    );
                }
                SpawnKind::LoadPane { req_id } => {
                    let request = self
                        .pane_req
                        .clone()
                        .unwrap_or_else(|| RightPaneRequest::from_state(state));
                    let target = request.target();
                    start(
                        id,
                        JobRelease::RightPane { req_id },
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
                SpawnKind::UserWork { tag } => self.spawn_user(state, opts, id, tag, &mut start),
            }
        }
        self.job_releases.extend(started);
        self.publish_remote_activity(state);
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
        self.publish_remote_activity(state);
    }

    /// Copy the remote queue into [`AppState::remote_activity`] for paint.
    ///
    /// `running` holds each checkout with a fetch / pull / push in flight.
    /// `batch` sums the key-started waves that still have work and span two
    /// or more repos; the background fetch tick stays silent, the same rule
    /// as [`Self::paint_remote_running`]. Marks a redraw when it changes.
    fn publish_remote_activity(&mut self, state: &mut AppState) {
        let running = self
            .remote
            .occupy
            .values()
            .filter_map(|occ| match occ {
                OccupyReason::Remote(job) => Some(job.checkout.clone()),
                _ => None,
            })
            .collect();
        let mut batch: Option<BatchProgress> = None;
        for kind in [RunningOp::Fetch, RunningOp::Pull, RunningOp::Push] {
            let Some(wave) = self.remote.wave(kind) else {
                continue;
            };
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
            let done = wave.done();
            if !wave.foreground || inflight + pending == 0 || done + inflight + pending < 2 {
                continue;
            }
            let sum = batch.get_or_insert_with(BatchProgress::default);
            sum.done += done;
            sum.total += done + inflight + pending;
            sum.failed |= wave.tally.has_failure();
        }
        let next = RemoteActivity { running, batch };
        if state.remote_activity != next {
            state.remote_activity = next;
            self.mark();
        }
    }

    fn apply_outcome(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        id: u64,
        outcome: JobOutcome,
    ) {
        let _ = self.finish_job(id);
        match outcome {
            JobOutcome::Discovered { gen, entries } => {
                let keep: Vec<String> = entries.iter().map(|(p, _, _)| p.clone()).collect();
                let decision = self.sched.on_discovered(gen, keep.clone());
                if decision == ApplyDecision::Ignore {
                    return;
                }
                drop_undiscovered_checkouts(state, &keep);
                self.metas = entries
                    .into_iter()
                    .map(|(path, meta, ov)| (path, (meta, ov)))
                    .collect();
                self.queue_due_pr_lookups(state, opts);
                // A walk with nothing to load ends the collect here, not on a repo result.
                self.after_collection_step(state, &decision);
                self.mark();
            }
            JobOutcome::RepoStatus {
                gen,
                path,
                snap,
                facts,
            } => {
                if !self.sched.accept_repo_result(gen, &path) {
                    return;
                }
                let before_sigs = state.signatures.clone();
                let before_snap = state.snapshot.clone();
                let focused = state.focused_checkout_path();
                if facts.is_some() {
                    state.set_file_facts(&path, facts);
                }
                apply_one_repo_snapshot(state, &path, snap);
                if let Some(reload) = state.worktree_compare_reload(&path) {
                    self.schedule(state, opts, reload, &Action::None);
                }
                if let Some(reload) = state.explorer_live_reload(&path) {
                    self.schedule(state, opts, reload, &Action::None);
                }
                if let Some(relist) = state.explorer_status_relist(&path) {
                    self.schedule(state, opts, relist, &Action::None);
                }
                self.queue_due_pr_lookups(state, opts);
                self.note_repo_status_done(state, gen, &path);
                if focused.as_deref() == Some(path.as_str())
                    && focused_repo_needs_pane(&before_sigs, &before_snap, state, &path)
                    && state.tabs.is_workspace()
                {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
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
                self.release_write(state, id);
                state.status = status;
                self.sched.on_reload_snapshot(state.focused_checkout_path());
                if state.tabs.is_workspace() {
                    self.pane_req = Some(RightPaneRequest::from_state(state));
                    self.sched.request_pane();
                }
                self.mark();
            }
            JobOutcome::BulkRemote { kind, result, repo } => {
                self.land_remote(state, kind, &repo, result);
            }
            JobOutcome::DefaultBranch { repo, result } => {
                self.land_default_branch(state, &repo, result);
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
            JobOutcome::PreparePush { gen, repo, plan } => {
                self.sched.note_user_done(UserTag::Prepare);
                let accepted = self.sched.accept_prepare_push_result(gen);
                let current = state.focused_checkout_path();
                // Push runs only from the Workspace tab (compare and Explorer tabs refuse it).
                if accepted
                    && current.as_deref() == Some(repo.as_str())
                    && state.tabs.is_workspace()
                {
                    state.open_push_confirm(repo, plan);
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
                self.release_write(state, id);
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
                self.release_write(state, id);
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
                self.finish_autoload(state, accepted);
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
            JobOutcome::SearchPreview { gen, body } => {
                if state.apply_search_preview(gen, body) {
                    self.mark();
                }
            }
            JobOutcome::ExplorerDir {
                tab_id,
                req,
                rel_dir,
                entries,
            } => {
                if let Some(follow) = state.apply_explorer_dir(tab_id, req, &rel_dir, entries) {
                    self.schedule(state, opts, follow, &Action::None);
                    self.mark();
                }
            }
            JobOutcome::ExplorerPreview { tab_id, gen, body } => {
                if state.apply_explorer_preview(tab_id, gen, body) {
                    self.mark();
                }
            }
            JobOutcome::Activity { key, days } => {
                // `None`: the job saw the quit flag and ran no git.
                if let Some(days) = days {
                    if state.activity.insert(key, days) != Some(days) {
                        self.mark();
                    }
                }
            }
            JobOutcome::StoreSaved { saved } => self.land_store_save(state, saved),
            JobOutcome::Doctor { gen, checks } => {
                // A re-run asked while this probe ran gets its own probe
                // next.
                self.doctor_job.finish(&mut self.sched);
                // A closed or re-run Doctor drops this answer.
                if state.apply_doctor(gen, checks) {
                    self.mark();
                }
            }
            JobOutcome::LineBlame { key, result } => {
                self.release_line_blame();
                if state.apply_line_blame(key, result) {
                    self.mark();
                }
            }
            JobOutcome::CommitCard { repo, sha, result } => {
                self.release_commit_card();
                if state.apply_commit_card(repo, sha, result) {
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
                self.release_pr_badge(&repo, &branch);
                if state.apply_pr_lookup(&repo, &branch, remote, lookup) {
                    // A pinned or peeked popover, and the active PRs tab,
                    // ask again for a detail whose answer changed.
                    let details = state.request_pr_details();
                    if details != Effect::None {
                        self.schedule(state, opts, details, &Action::None);
                    }
                    self.mark();
                }
                self.after_pr_badge(state, opts);
            }
            JobOutcome::PrTitles { titles, cancelled } => {
                let next = state.apply_pr_titles(titles, cancelled, Instant::now());
                if next != Effect::None {
                    self.schedule(state, opts, next, &Action::None);
                }
                self.mark();
            }
            JobOutcome::PullRequestDetail { job, lookup } => {
                self.land_pr_detail(state, opts, job, lookup);
            }
            JobOutcome::PullRequestOpen {
                repo,
                branch,
                remote,
                lookup,
                page,
                opened,
            } => {
                state.apply_pr_open(&repo, &branch, remote, lookup, page, opened);
                self.mark();
                self.alert_pr_changes(state);
            }
            JobOutcome::PullRequestCiOpen { pr_ref, opened } => {
                state.apply_pr_ci_open(&pr_ref, opened);
                self.mark();
            }
            JobOutcome::RemoteView { page, opened } => {
                state.apply_remote_view(page, opened);
                self.mark();
            }
            JobOutcome::ClipboardCopied { ok, announce } => {
                self.clipboard_job.finish(&mut self.sched);
                if let Some(ok) = ok {
                    show_copy_result(state, ok, announce);
                    self.mark();
                }
            }
            JobOutcome::CheckoutRemote {
                repo,
                branch,
                remote,
            } => {
                if state.apply_checkout_remote(&repo, &branch, remote) {
                    self.mark();
                }
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

    // Releases shared by the outcome handlers above and the panic path
    // (`job_release.rs`): each job kind frees what it holds in one place.

    /// Checkout `path` of collect `gen` is done: the collect may finish.
    /// The `r` reload that lands here counts every repo it found.
    fn note_repo_status_done(&mut self, state: &AppState, gen: u64, path: &str) -> ApplyDecision {
        let decision = self.sched.note_repo_done(gen, path);
        self.after_collection_step(state, &decision);
        decision
    }

    /// Follow-up once a collect step lands: an `r` reload that ended the
    /// collect counts activity from the new snapshot.
    fn after_collection_step(&mut self, state: &AppState, decision: &ApplyDecision) {
        if *decision == ApplyDecision::CollectionIdle
            && std::mem::take(&mut self.activity_after_reload)
        {
            self.queue_activity(state, activity_targets(&state.snapshot, None));
        }
    }

    /// Free the exclusive write slot of job `id` and the gitdirs it holds.
    fn release_write(&mut self, state: &mut AppState, id: u64) {
        self.sched.note_user_done(UserTag::Write);
        self.release_exclusive(state, id);
    }

    /// Land the fetch / pull / push `result` of checkout `repo`: free its
    /// gitdir, count it in its wave, and end the wave when it was the last.
    fn land_remote(
        &mut self,
        state: &mut AppState,
        kind: RunningOp,
        repo: &str,
        result: RepoOpResult,
    ) {
        let gitdir = self
            .remote
            .occupy
            .iter()
            .find(|(_, occ)| match occ {
                OccupyReason::Remote(inf) => inf.checkout == repo,
                _ => false,
            })
            .map(|(key, _)| key.clone())
            .unwrap_or_else(|| gitdir_key(state, repo));
        self.remote.occupy.remove(&gitdir);
        if let Some(wave) = self.remote.wave_mut(kind) {
            wave.tally.note(repo, result);
            wave.note_repo(repo);
        }
        self.mark();
        if self.kind_live(kind) {
            self.paint_remote_running(state);
        } else {
            self.finish_kind_if_idle(state, kind);
        }
        self.pump_remote_slots(state);
    }

    /// Land the default-branch switch `result` of checkout `repo`: start
    /// the next queued switch, or end the batch and free its gitdirs.
    fn land_default_branch(&mut self, state: &mut AppState, repo: &str, result: RepoOpResult) {
        self.sched.note_user_done(UserTag::DefaultBranch);
        self.default_tally.note(repo, result);
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

    /// An older graph page job ended. `accepted`: it was the latest
    /// autoload, so `loading older…` ends and older pages may load again.
    fn finish_autoload(&mut self, state: &mut AppState, accepted: bool) {
        self.sched.note_user_done(UserTag::Autoload);
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

    /// Free the focused-line blame slot. The waiting question may be
    /// cached or off target by now: `after_change` offers the current one
    /// again.
    fn release_line_blame(&mut self) {
        self.line_blame_job.clear();
        self.line_blame_job.finish(&mut self.sched);
    }

    /// Free the commit card slot. `after_change` offers the current
    /// popover commit again.
    fn release_commit_card(&mut self) {
        self.commit_card_job.clear();
        self.commit_card_job.finish(&mut self.sched);
    }

    /// Free the badge slot of the lookup of `branch` in checkout `repo`.
    fn release_pr_badge(&mut self, repo: &Path, branch: &str) {
        self.pr_badges_running = self.pr_badges_running.saturating_sub(1);
        self.pr_badge_jobs
            .remove(&(repo.to_path_buf(), branch.to_string()));
    }

    /// After a badge lookup ended: alert the PR changes, start a title
    /// cycle once the lookups are idle, and give the freed slot to a
    /// waiting lookup.
    fn after_pr_badge(&mut self, state: &mut AppState, opts: &TuiOpts) {
        self.alert_pr_changes(state);
        self.start_pr_title_cycle(state, opts);
        self.fill_pr_badge_slots();
    }

    /// Land the detail `lookup` of `job` and free its badge slot.
    fn land_pr_detail(
        &mut self,
        state: &mut AppState,
        opts: &TuiOpts,
        job: PrDetailJob,
        lookup: PrDetailLookup,
    ) {
        self.pr_badges_running = self.pr_badges_running.saturating_sub(1);
        let PrDetailJob {
            repo,
            branch,
            remote,
            number,
            request,
        } = job;
        if state.apply_pr_detail(&repo, &branch, &remote, number, request, lookup) {
            // The answer changed while this fetch was out: ask
            // again for the open popover and the active PRs tab.
            let details = state.request_pr_details();
            if details != Effect::None {
                self.schedule(state, opts, details, &Action::None);
            }
            self.mark();
        }
        self.fill_pr_badge_slots();
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

    /// Queue badge lookups for checkouts whose branch is new or changed,
    /// and git-only remote resolves for checkouts on their default branch.
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
        let targets = state.due_remote_lookups();
        if !targets.is_empty() {
            self.schedule(
                state,
                opts,
                Effect::ResolveCheckoutRemotes { targets },
                &Action::None,
            );
        }
    }

    /// Alert the PR changes that landed lookups queued
    /// ([`AppState::alert_pr_transitions`]) when no badge lookup waits or
    /// runs, or when the oldest change has waited [`PR_ALERT_WAIT_MS`].
    ///
    /// The lookups of one poll cycle, one `r`, or the launch land one by
    /// one under the cap, so they usually alert as one batch when the last
    /// one lands. Cycles that overlap (a short poll period, many
    /// checkouts, a slow forge) or a lookup that never lands keep the queue
    /// busy; the wait bound still alerts at the next landing. A `gx`
    /// lookup that lands with none out alerts at once.
    fn alert_pr_changes(&mut self, state: &mut AppState) {
        let now = Instant::now();
        let idle = self.pr_badges.is_empty() && self.pr_badge_jobs.is_empty();
        let waited = state.pr_transitions_waiting_since().is_some_and(|since| {
            now.saturating_duration_since(since).as_millis() >= u128::from(PR_ALERT_WAIT_MS)
        });
        if (idle || waited) && state.alert_pr_transitions(now) {
            self.mark();
        }
    }

    /// Start a PR title cycle once no badge lookup waits or runs: a poll
    /// cycle, an `r`, or the launch lookups ended
    /// ([`AppState::new_pr_title_cycle`], which runs the command only when
    /// the listed keys changed, the last fill is a poll period old, or `r`
    /// asked).
    fn start_pr_title_cycle(&mut self, state: &mut AppState, opts: &TuiOpts) {
        if !self.pr_badges.is_empty() || !self.pr_badge_jobs.is_empty() {
            return;
        }
        let fill = state.new_pr_title_cycle(Instant::now());
        if fill != Effect::None {
            self.schedule(state, opts, fill, &Action::None);
        }
    }

    /// Queue `UserTag::PullRequest` slots for waiting badge lookups and
    /// detail fetches, up to [`PR_BADGE_LOOKUPS_MAX`] on workers. `gx` and
    /// "View remotely" opens queue their own slot.
    fn fill_pr_badge_slots(&mut self) {
        let free = PR_BADGE_LOOKUPS_MAX.saturating_sub(self.pr_badges_running);
        let waiting = self.pr_badges.len() + self.pr_details.len();
        let wanted = self.pr_opens.len()
            + self.pr_ci_opens.len()
            + self.remote_views.len()
            + self.checkout_remotes.len()
            + waiting.min(free);
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
        self.enqueue_write_as(state, op, StatusKind::Ok, checkouts, work);
    }

    /// [`Self::enqueue_write`] with the success status kind `ok_kind`.
    fn enqueue_write_as(
        &mut self,
        state: &AppState,
        op: &'static str,
        ok_kind: StatusKind,
        checkouts: &[&str],
        work: Box<dyn FnOnce() -> Result<String, String> + Send>,
    ) {
        let gitdirs: Vec<String> = checkouts
            .iter()
            .map(|checkout| gitdir_key(state, checkout))
            .collect();
        self.occupy_exclusive(&gitdirs);
        self.writes.push_back(WriteJob {
            op,
            gitdirs,
            ok_kind,
            work,
        });
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
            | Effect::MergeIntoHead { repo, .. }
            | Effect::ResetToUpstream { repo, .. } => self.remote_gitdir_busy(state, repo),
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
    /// `nothing behind to pull` (or a diverged or upstream-rewrite line, or
    /// it opened the reset box). An unfetched tracking
    /// checkout still looks in-sync, so that path would drop `p` while fetch
    /// occupies the gitdir. Other None Pull paths (right pane, compare tab,
    /// drill) must not follow. Diverged targets stay out: `p` never pulls
    /// them.
    fn enqueue_pull_after_inflight_fetch(&mut self, state: &mut AppState) {
        let targets = op_targets(
            &state.snapshot,
            state.focused_row(),
            state.show_hidden,
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
    /// background fetch tick, which never writes the status slot: its wave
    /// end feeds the scheduled-fetch outage.
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
        let mut flushed = false;
        for checkout in repos {
            flushed |= self.enqueue_remote_job(state, kind, checkout);
        }
        // Mark the wave only when a job of this kind is live: when every
        // target was dropped (a pull already holds it), no wave runs, and a
        // flag set here would leak into the next wave a key starts.
        if self.kind_live(kind) {
            if let Some(wave) = self.remote.wave_mut(kind) {
                if foreground {
                    wave.foreground = true;
                } else {
                    wave.scheduled = true;
                }
            }
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
        // No fetch is live: the claimed wave ends here. An early return
        // drops (releases) the claim with no record.
        let claim = match kind {
            RunningOp::Fetch => self.fetch_claim.take(),
            _ => None,
        };
        let Some(wave) = self.remote.wave_mut(kind) else {
            return false;
        };
        if wave.done() == 0 && wave.repos.is_empty() {
            // No wave ran: clear its origin flags so they cannot reach the
            // next wave.
            wave.foreground = false;
            wave.scheduled = false;
            return false;
        }
        let repos = std::mem::take(&mut wave.repos);
        let tally = std::mem::take(&mut wave.tally);
        let foreground = wave.foreground;
        // A claim is only held for a wave the tick joined.
        let scheduled = wave.scheduled || claim.is_some();
        *wave = KindWave::empty();
        state.stamp_checkout_flashes(&repos);
        // A pull can bring commits onto local branches: recount the repos
        // this wave touched.
        if kind == RunningOp::Pull {
            self.queue_activity(state, activity_targets(&state.snapshot, Some(&repos)));
        }
        // The background tick never writes the status slot: its failure is
        // the scheduled-fetch outage below. A key that joined its wave
        // still gets the result line.
        if foreground {
            state.status = completed_op_status(kind, &tally);
        }
        // A failed record write has no recovery; the claim is released.
        let generation = claim.and_then(|claim| claim.finish(tally.has_failure()).ok());
        if scheduled {
            state.note_background_fetch_outcome(
                generation,
                scheduled_fetch_failure_text(&tally),
                Instant::now(),
            );
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
        spawn: &mut dyn FnMut(u64, JobRelease, JobWork),
    ) {
        match tag {
            UserTag::Write => {
                if let Some((repo, name, ff, gitdir)) = self.checkout.take() {
                    self.exclusive_inflight.insert(id, vec![gitdir]);
                    self.running_write = Some("checkout");
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        JobRelease::Write { op: "checkout" },
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
                        JobRelease::Write { op: "merge" },
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
                    let ok_kind = job.ok_kind;
                    spawn(
                        id,
                        JobRelease::Write { op },
                        Box::new(move || JobOutcome::Write {
                            status: write_status(op, ok_kind, (job.work)()),
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
                    JobRelease::Remote {
                        kind,
                        repo: repo.clone(),
                    },
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
                    JobRelease::DefaultBranch { repo: repo.clone() },
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
                        JobRelease::Prepare { what: "stash menu" },
                        Box::new(move || {
                            let latest = latest_stash(&dir);
                            JobOutcome::PrepareStash { gen, repo, latest }
                        }),
                    );
                    return;
                }
                if let Some((gen, repo)) = self.prepare_push.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        JobRelease::Prepare { what: "push plan" },
                        Box::new(move || {
                            let plan = push_plan(&dir);
                            JobOutcome::PreparePush { gen, repo, plan }
                        }),
                    );
                    return;
                }
                if let Some((gen, repo, kind)) = self.prepare_compare.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
                        JobRelease::ComparePicker {
                            gen,
                            repo: repo.clone(),
                        },
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
                        JobRelease::Prepare {
                            what: "blame previous change",
                        },
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
                        JobRelease::Prepare {
                            what: "branch picker",
                        },
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
                        JobRelease::DiffPrepare,
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
                        JobRelease::CompareRange {
                            tab_id: job.tab_id,
                            gen: job.gen,
                        },
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
                        JobRelease::CompareDiff {
                            tab_id: job.tab_id,
                            req_id: job.req_id,
                            gen: job.gen,
                            source: job.source.clone(),
                            path: job.path.clone(),
                        },
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
                        JobRelease::CompareProbe { tab_id: job.tab_id },
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
                        JobRelease::Read {
                            what: "commit files",
                        },
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
                        JobRelease::Read {
                            what: "commit diff",
                        },
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
                // Explorer loads are queued one per enqueue, so draining them
                // first never strands one behind the latest-only index /
                // score slots (`LatestOnly`, overlap): each of those still
                // holds its own queued slot.
                let job = self.explorer_jobs.pop_front().or_else(|| {
                    (!self.explorer_previews.is_empty()).then(|| self.explorer_previews.remove(0))
                });
                if let Some(job) = job {
                    let cwd = opts.cwd.clone();
                    spawn(
                        id,
                        job.release(),
                        Box::new(move || run_explorer_job(&cwd, job)),
                    );
                    return;
                }
                if let Some((gen, roots)) = self.file_index_job.take() {
                    let cwd = opts.cwd.clone();
                    spawn(
                        id,
                        JobRelease::Read {
                            what: "Quick Open index",
                        },
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
                        JobRelease::Read {
                            what: "Quick Open score",
                        },
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
                // The preview is one file read: it goes before a chunk so
                // a moving highlight does not wait on a whole search.
                if let Some((gen, checkout, rel)) = self.search_preview_job.take() {
                    let file = opts.cwd.join(checkout).join(rel);
                    spawn(
                        id,
                        JobRelease::SearchPreview { gen },
                        Box::new(move || JobOutcome::SearchPreview {
                            gen,
                            body: read_text_file(&file, MAX_FILE_BYTES),
                        }),
                    );
                    return;
                }
                if let Some((gen, roots)) = self.search_index_job.take() {
                    let cwd = opts.cwd.clone();
                    spawn(
                        id,
                        JobRelease::Read {
                            what: "search index",
                        },
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
                    .take_if(|job| job.gen == live.load(Ordering::Relaxed))
                {
                    let cwd = opts.cwd.clone();
                    spawn(
                        id,
                        JobRelease::Read {
                            what: "search chunk",
                        },
                        Box::new(move || job.run(&cwd, &live)),
                    );
                    return;
                }
                self.sched.note_job_finished(id);
            }
            UserTag::LineBlame => {
                let Some(key) = self.line_blame_job.take() else {
                    // The focused line moved off every blame target.
                    self.sched.note_job_finished(id);
                    return;
                };
                let dir = opts.cwd.join(&key.repo);
                spawn(
                    id,
                    JobRelease::LineBlame { key: key.clone() },
                    Box::new(move || {
                        let result = blame_line(&dir, &key.rev, &key.path, key.line);
                        JobOutcome::LineBlame { key, result }
                    }),
                );
            }
            UserTag::CommitCard => {
                let Some((repo, sha)) = self.commit_card_job.take() else {
                    // The popover closed or its commit got its data.
                    self.sched.note_job_finished(id);
                    return;
                };
                let dir = opts.cwd.join(&repo);
                spawn(
                    id,
                    JobRelease::CommitCard {
                        repo: repo.clone(),
                        sha: sha.clone(),
                    },
                    Box::new(move || JobOutcome::CommitCard {
                        result: load_commit_card(&dir, &sha),
                        repo,
                        sha,
                    }),
                );
            }
            UserTag::StoreSave(kind) => {
                // The job takes the store now, so it sends every change so
                // far. Nothing to save when an earlier slot already sent it.
                let (slot, _) = self.store_save_slot(kind);
                let job = slot
                    .take_if(|_| state.store_save_ready(kind))
                    .and_then(|()| state.begin_store_save(kind));
                let Some(job) = job else {
                    self.sched.note_job_finished(id);
                    return;
                };
                spawn(
                    id,
                    JobRelease::StoreSave { kind },
                    Box::new(move || JobOutcome::StoreSaved { saved: job.run() }),
                );
            }
            UserTag::Doctor => {
                // Nothing to probe when the Doctor closed or a newer
                // request already took the slot.
                let Some(gen) = self.doctor_job.take_if(|gen| state.doctor_awaits(*gen)) else {
                    self.sched.note_job_finished(id);
                    return;
                };
                // Owned copies: `collect` borrows, and the remote
                // lookups and probe run on the worker.
                let inputs = DoctorJob::new(state, opts);
                spawn(
                    id,
                    JobRelease::Doctor,
                    Box::new(move || JobOutcome::Doctor {
                        gen,
                        checks: inputs.run(),
                    }),
                );
            }
            UserTag::Clipboard => {
                let Some(job) = self.clipboard_job.take() else {
                    self.sched.note_job_finished(id);
                    return;
                };
                let copy = self.clipboard_tool;
                // Only a copy whose OSC 52 failed still waits on a result.
                let pending = !job.osc52;
                let announce = job.announce;
                spawn(
                    id,
                    JobRelease::Clipboard { pending, announce },
                    Box::new(move || {
                        let ok = copy(
                            &job.text,
                            Instant::now() + comments::HOST_CLIPBOARD_TIME_LIMIT,
                        );
                        JobOutcome::ClipboardCopied {
                            ok: pending.then_some(ok),
                            announce,
                        }
                    }),
                );
            }
            UserTag::PrTitles => {
                let Some((cmd, groups)) = self.pr_title_job.take() else {
                    self.sched.note_job_finished(id);
                    return;
                };
                let cancel = Arc::clone(&self.pr_cancel);
                spawn(
                    id,
                    JobRelease::PrTitles,
                    Box::new(move || {
                        // One group at a time, so a quit stops between keys;
                        // the registry ends a command that runs then.
                        let mut titles = TitleCache::default();
                        let mut cancelled = false;
                        for group in &groups {
                            if cancel.load(Ordering::Relaxed) {
                                cancelled = true;
                                break;
                            }
                            titles.fill(&cmd, std::slice::from_ref(group));
                        }
                        JobOutcome::PrTitles { titles, cancelled }
                    }),
                );
            }
            UserTag::Activity => {
                // One repo per job, so user work takes the slot between
                // repos. After quit, or once the sparkline is off, nothing
                // more starts.
                let key = self.activity_pending.pop_front();
                let cancel = Arc::clone(&self.activity_cancel);
                let Some(key) =
                    key.filter(|_| state.activity_sparkline && !cancel.load(Ordering::Relaxed))
                else {
                    self.sched.note_job_finished(id);
                    return;
                };
                let cwd = opts.cwd.clone();
                spawn(
                    id,
                    JobRelease::Read {
                        what: "activity count",
                    },
                    Box::new(move || {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
                        let days = load_activity(&cwd, &key, now, &cancel);
                        JobOutcome::Activity { key, days }
                    }),
                );
            }
            UserTag::PullRequest => {
                let cwd = opts.cwd.clone();
                let cli = self.pr_cli;
                let cancel = Arc::clone(&self.pr_cancel);
                if let Some((repo, branch, page)) = self.pr_opens.pop_front() {
                    let open = self.pr_opener;
                    spawn(
                        id,
                        JobRelease::Read { what: "PR open" },
                        Box::new(move || {
                            let run = |argv: &[String]| cli(argv, &cancel);
                            let remote = remote_url_for_branch(&cwd.join(&repo), &branch);
                            let lookup = lookup(remote.as_deref(), &branch, &run);
                            let opened = match (&lookup, page) {
                                (PrLookup::Found(pr), PrPage::Pr) => open(&pr.url, &cancel).is_ok(),
                                (PrLookup::Found(pr), PrPage::Ci) => {
                                    is_web_url(&pr.ci_link.url)
                                        && open(&pr.ci_link.url, &cancel).is_ok()
                                }
                                (PrLookup::NoPr | PrLookup::Failed(_), _) => false,
                            };
                            JobOutcome::PullRequestOpen {
                                repo,
                                branch,
                                remote,
                                lookup,
                                page,
                                opened,
                            }
                        }),
                    );
                    return;
                }
                if let Some((pr_ref, url)) = self.pr_ci_opens.pop_front() {
                    let open = self.pr_opener;
                    spawn(
                        id,
                        JobRelease::Read { what: "PR CI open" },
                        Box::new(move || JobOutcome::PullRequestCiOpen {
                            pr_ref,
                            opened: open(&url, &cancel).is_ok(),
                        }),
                    );
                    return;
                }
                if let Some((checkout, target)) = self.remote_views.pop_front() {
                    let open = self.pr_opener;
                    spawn(
                        id,
                        JobRelease::Read {
                            what: "remote view",
                        },
                        Box::new(move || {
                            // A job that starts after quit runs no git.
                            if cancel.load(Ordering::Relaxed) {
                                return JobOutcome::RemoteView {
                                    page: Err(remote_view::OPEN_FAILED.to_string()),
                                    opened: false,
                                };
                            }
                            let page = remote_view::resolve(&cwd.join(&checkout), &target)
                                .map(|(repo, page)| remote_view::page_url(&repo, &page));
                            let opened = match &page {
                                Ok(url) if !cancel.load(Ordering::Relaxed) => {
                                    open(url, &cancel).is_ok()
                                }
                                Ok(_) | Err(_) => false,
                            };
                            JobOutcome::RemoteView { page, opened }
                        }),
                    );
                    return;
                }
                if let Some((repo, branch)) = self.checkout_remotes.pop_front() {
                    spawn(
                        id,
                        JobRelease::Read {
                            what: "checkout remote",
                        },
                        Box::new(move || {
                            // A job that starts after quit runs no git.
                            let remote = if cancel.load(Ordering::Relaxed) {
                                None
                            } else {
                                remote_url_for_branch(&cwd.join(&repo), &branch)
                            };
                            JobOutcome::CheckoutRemote {
                                repo,
                                branch,
                                remote,
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
                            JobRelease::PrDetail { job: job.clone() },
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
                        self.pr_badge_jobs.insert((repo.clone(), branch.clone()));
                        spawn(
                            id,
                            JobRelease::PrBadge {
                                repo: repo.clone(),
                                branch: branch.clone(),
                            },
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
                let show_hidden = state.show_hidden;
                let focus = state.graph_focus_revs();
                let repo = identity.repo.clone();
                spawn(
                    id,
                    JobRelease::Autoload { gen },
                    Box::new(move || {
                        let (page, _loaded) = load_graph_model_window(
                            &cwd,
                            &snapshot,
                            &repo,
                            show_hidden,
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
mod tests;
