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
    create_branch_checkout, exec_git_checked, latest_stash_ref, list_compare_picker_branches,
    list_compare_picker_commits, list_local_branches, previous_line_change, pull_quiet_detailed,
    push_plan, push_quiet, remote_url_for_branch, remove_untracked_file, remove_worktree,
    revert_compare_file, revert_compare_patch, revert_tracked_file, stage_file, stash_apply,
    stash_drop, stash_pop, stash_push, unstage_file, PushPlan, COMPARE_REVERT_ABORTED,
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
use super::fetch_claim::{try_claim_background_fetch, BackgroundFetchClaim};
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
use super::remote_view::{self, RemoteTarget};
use super::scheduler::{ApplyDecision, Scheduler, SpawnKind, UserTag};
use super::search_files::SEARCH_CHUNK_BUDGET;
use super::stash::{resolve_stash_menu_key, StashMenuKeyResult, StashOpId};
use super::state::{revert_scope, AppState, PendingConfirm};
use super::status::StatusMessage;
use super::tabs::{ComparePickerKind, ComparePickerRows, WorktreeFile};
use super::viewed::workspace_store_id;

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
    /// "View remotely" check, then the browser open. `page` is the page URL,
    /// or the status line that says why there is none; `opened` is true
    /// when the browser started for it.
    RemoteView {
        page: Result<String, String>,
        opened: bool,
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
    file_index_job: Option<(u64, Vec<IndexRoot>)>,
    /// Latest Quick Open score request; a newer one replaces it before spawn.
    file_score_job: Option<(u64, Arc<FileIndex>, String)>,
    /// Queued Explorer folder listings, one per enqueue.
    explorer_jobs: VecDeque<ExplorerJob>,
    /// Queued Explorer inner-tab loads, at most one per `(tab, path)`. A
    /// new load first drops each queued one whose generation no inner tab
    /// holds any more, so a held `j` (each move replacing the preview tab)
    /// queues one read.
    explorer_previews: Vec<ExplorerJob>,
    /// Latest search-in-files index request; a newer one replaces it
    /// before spawn.
    search_index_job: Option<(u64, Vec<IndexRoot>)>,
    /// Latest search-in-files chunk request; a newer one replaces it
    /// before spawn.
    search_chunk_job: Option<SearchChunkJob>,
    /// Latest search-in-files preview read: generation, checkout, path. A
    /// newer one replaces it before spawn.
    search_preview_job: Option<(u64, String, String)>,
    /// Generation of the search that may still run (0: none). Synced from
    /// the state after every change; a running chunk stops between files
    /// once it no longer names the chunk's own generation.
    search_live: Arc<AtomicU64>,
    /// Latest focused-line blame question; a newer one replaces it
    /// before spawn, so cursor moves never queue a blame per line.
    line_blame_slot: Option<BlameKey>,
    /// A [`UserTag::LineBlame`] job is queued or running.
    line_blame_inflight: bool,
    /// Latest Doctor probe request; a newer one replaces it before spawn.
    doctor_slot: Option<u64>,
    /// A [`UserTag::Doctor`] job is queued or running.
    doctor_inflight: bool,
    /// Latest blame previous-change request; a newer one replaces it
    /// before spawn.
    blame_previous: Option<(u64, String, Box<crate::git::LineBlame>)>,
    /// Queued `gx` opens: checkout, branch. Each has its own slot.
    pr_opens: VecDeque<(PathBuf, String)>,
    /// Queued "View remotely" opens: checkout, target. Each has its own
    /// [`UserTag::PullRequest`] slot and runs after the queued `gx` opens.
    remote_views: VecDeque<(PathBuf, RemoteTarget)>,
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
    /// once, so the runtime shutdown does not wait for them.
    pr_cancel: Arc<AtomicBool>,
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
            file_index_job: None,
            file_score_job: None,
            explorer_jobs: VecDeque::new(),
            explorer_previews: Vec::new(),
            search_index_job: None,
            search_chunk_job: None,
            search_preview_job: None,
            search_live: Arc::new(AtomicU64::new(0)),
            line_blame_slot: None,
            line_blame_inflight: false,
            doctor_slot: None,
            doctor_inflight: false,
            blame_previous: None,
            pr_opens: VecDeque::new(),
            remote_views: VecDeque::new(),
            pr_badges: VecDeque::new(),
            pr_badge_jobs: HashSet::new(),
            pr_details: VecDeque::new(),
            pr_badges_running: 0,
            pr_cli: DEFAULT_PR_CLI,
            pr_opener: DEFAULT_PR_OPENER,
            pr_cancel: Arc::new(AtomicBool::new(false)),
            activity_pending: VecDeque::new(),
            activity_cancel: Arc::new(AtomicBool::new(false)),
            activity_after_reload: false,
            exclusive_inflight: HashMap::new(),
            running_write: None,
            fetch_claim: None,
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
            Effect::OpenPullRequest { repo, branch } => {
                self.pr_opens.push_back((repo, branch));
                self.sched.enqueue_user(UserTag::PullRequest);
            }
            Effect::ViewRemotely { checkout, target } => {
                self.remote_views.push_back((checkout, target));
                self.sched.enqueue_user(UserTag::PullRequest);
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
            Effect::RunDoctor { gen } => {
                self.doctor_slot = Some(gen);
                self.queue_doctor();
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

    /// Jobs the state asks for after any schedule or apply: the
    /// search-in-files preview read, the focused line's blame, and an older
    /// graph page for a pending blame reveal (dropped once the graph pane
    /// loses focus).
    fn after_change(&mut self, state: &mut AppState) {
        self.search_live
            .store(state.search_files_live_gen(), Ordering::Relaxed);
        if let Some(job) = state.search_preview_request() {
            self.search_preview_job = Some(job);
            self.sched.enqueue_user(UserTag::SearchFiles);
        }
        self.maybe_queue_line_blame(state);
        state.drop_graph_reveal_off_graph();
        if Self::autoload_allowed(state) && state.graph_reveal_wants_older() {
            self.queue_autoload(state);
        }
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

    /// Queue one [`UserTag::Doctor`] slot for the request in
    /// `doctor_slot`.
    ///
    /// Latest-only, like the line blame: at most one job is queued or
    /// running, and the slot holds the newest request. Its apply calls this
    /// again, so a re-run asked while a probe ran gets its own probe next.
    fn queue_doctor(&mut self) {
        if self.doctor_slot.is_some() && !self.doctor_inflight {
            self.doctor_inflight = true;
            self.sched.enqueue_user(UserTag::Doctor);
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
                    spawn(
                        id,
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
                if let Some(reload) = state.explorer_live_reload(&path) {
                    self.schedule(state, opts, reload, &Action::None);
                }
                if let Some(relist) = state.explorer_status_relist(&path) {
                    self.schedule(state, opts, relist, &Action::None);
                }
                self.queue_due_pr_lookups(state, opts);
                let decision = self.sched.note_repo_done(gen, &path);
                // The `r` reload has landed: count every repo it found.
                if decision == ApplyDecision::CollectionIdle
                    && std::mem::take(&mut self.activity_after_reload)
                {
                    self.queue_activity(state, activity_targets(&state.snapshot, None));
                }
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
            JobOutcome::Doctor { gen, checks } => {
                self.doctor_inflight = false;
                // A closed or re-run Doctor drops this answer.
                if state.apply_doctor(gen, checks) {
                    self.mark();
                }
                self.queue_doctor();
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
                self.pr_badge_jobs.remove(&(repo.clone(), branch.clone()));
                if state.apply_pr_lookup(&repo, &branch, remote, lookup) {
                    // A pinned or peeked popover asks again for a detail
                    // whose answer changed.
                    let details = state.request_popover_pr_details();
                    if details != Effect::None {
                        self.schedule(state, opts, details, &Action::None);
                    }
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
            JobOutcome::RemoteView { page, opened } => {
                state.apply_remote_view(page, opened);
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
    /// detail fetches, up to [`PR_BADGE_LOOKUPS_MAX`] on workers. `gx` and
    /// "View remotely" opens queue their own slot.
    fn fill_pr_badge_slots(&mut self) {
        let free = PR_BADGE_LOOKUPS_MAX.saturating_sub(self.pr_badges_running);
        let waiting = self.pr_badges.len() + self.pr_details.len();
        let wanted = self.pr_opens.len() + self.remote_views.len() + waiting.min(free);
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
            return false;
        }
        let repos = std::mem::take(&mut wave.repos);
        let tally = std::mem::take(&mut wave.tally);
        let quiet = !wave.foreground && !tally.has_failure();
        *wave = KindWave::empty();
        state.stamp_checkout_flashes(&repos);
        // A pull can bring commits onto local branches: recount the repos
        // this wave touched.
        if kind == RunningOp::Pull {
            self.queue_activity(state, activity_targets(&state.snapshot, Some(&repos)));
        }
        if !quiet {
            state.status = completed_op_status(kind, &tally);
        }
        if let Some(claim) = claim {
            let failed = tally.has_failure();
            // A failed record write has no recovery; the claim is released.
            if let Ok(generation) = claim.finish(failed) {
                state.note_background_fetch_outcome(generation, failed);
            }
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
                if let Some((gen, repo)) = self.prepare_push.take() {
                    let dir = opts.cwd.join(&repo);
                    spawn(
                        id,
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
                // Explorer loads are queued one per enqueue, so draining them
                // first never strands one behind the latest-only index /
                // score slots.
                let job = self.explorer_jobs.pop_front().or_else(|| {
                    (!self.explorer_previews.is_empty()).then(|| self.explorer_previews.remove(0))
                });
                if let Some(job) = job {
                    let cwd = opts.cwd.clone();
                    spawn(id, Box::new(move || run_explorer_job(&cwd, job)));
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
                // The preview is one file read: it goes before a chunk so
                // a moving highlight does not wait on a whole search.
                if let Some((gen, checkout, rel)) = self.search_preview_job.take() {
                    let file = opts.cwd.join(checkout).join(rel);
                    spawn(
                        id,
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
            UserTag::Doctor => {
                // Nothing to probe when the Doctor closed or a newer
                // request already took the slot.
                let Some(gen) = self
                    .doctor_slot
                    .take()
                    .filter(|gen| state.doctor_awaits(*gen))
                else {
                    self.doctor_inflight = false;
                    self.sched.note_job_finished(id);
                    return;
                };
                // Owned copies: `collect` borrows, and the remote
                // lookups and probe run on the worker.
                let inputs = DoctorJob::new(state, opts);
                spawn(
                    id,
                    Box::new(move || JobOutcome::Doctor {
                        gen,
                        checks: inputs.run(),
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
                                PrLookup::NoPr | PrLookup::Failed(_) => false,
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
                if let Some((checkout, target)) = self.remote_views.pop_front() {
                    let open = self.pr_opener;
                    spawn(
                        id,
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
                        self.pr_badge_jobs.insert((repo.clone(), branch.clone()));
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
                let show_hidden = state.show_hidden;
                let focus = state.graph_focus_revs();
                let repo = identity.repo.clone();
                spawn(
                    id,
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
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use workspace_status_graph::{Commit, GraphModel, GraphRow};

    use crate::config::WorkspaceStatusConfig;
    use crate::git::{LocalBranch, NameStatus};
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::tui::app::{CompareRangeLoad, MergeCompute, TuiOpts};
    use crate::tui::diff::DiffContent;
    use crate::tui::drill::{CommitFile, CommitFileSource, DrillView};
    use crate::tui::fetch_claim::{
        load_background_fetch_record, unix_now_ms, BackgroundFetchRecord,
    };
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
            patch_keys: Default::default(),
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

    fn push_plan_ok() -> Result<PushPlan, String> {
        Ok(PushPlan {
            branch: "main".into(),
            remote: "origin".into(),
            publish: true,
        })
    }

    #[test]
    fn prepare_push_reads_the_plan_on_the_pool_then_opens_the_confirm() {
        let (root, mut state) = git_app_state("ws-effect-prepare-push", "feature/x", false);
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::PreparePush { repo: "app".into() },
            &Action::Push,
        );
        assert!(state.confirm.is_none(), "no git on the loop thread");
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1, "one Prepare job");
        for (id, work) in jobs {
            let outcome = work();
            assert!(matches!(outcome, JobOutcome::PreparePush { .. }));
            apply_id(&mut interp, &mut state, id, outcome);
        }
        assert_eq!(
            state.confirm,
            Some(PendingConfirm::Push {
                repo: "app".into(),
                branch: "feature/x".into(),
                remote: "origin".into(),
                publish: true,
                ahead_behind: None,
            })
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn late_or_stale_prepare_push_does_not_open_the_confirm() {
        let mut state = fixture_state();
        focus_repo(&mut state, "lib");
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_push();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PreparePush {
                gen,
                repo: "app".into(),
                plan: push_plan_ok(),
            },
        );
        assert!(state.confirm.is_none(), "focus moved off app");

        focus_repo(&mut state, "app");
        let old = interp.sched.request_prepare_push();
        let _latest = interp.sched.request_prepare_push();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PreparePush {
                gen: old,
                repo: "app".into(),
                plan: push_plan_ok(),
            },
        );
        assert!(state.confirm.is_none(), "stale gen");

        let _ = open_compare_tab(&mut state);
        let gen = interp.sched.request_prepare_push();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PreparePush {
                gen,
                repo: "app".into(),
                plan: push_plan_ok(),
            },
        );
        assert!(state.confirm.is_none(), "compare tab");
    }

    #[test]
    fn matching_prepare_push_opens_the_confirm() {
        let mut state = fixture_state();
        focus_repo(&mut state, "app");
        let mut interp = Interpreter::new();
        let gen = interp.sched.request_prepare_push();
        apply(
            &mut interp,
            &mut state,
            JobOutcome::PreparePush {
                gen,
                repo: "app".into(),
                plan: push_plan_ok(),
            },
        );
        assert!(
            matches!(state.confirm, Some(PendingConfirm::Push { .. })),
            "{:?}",
            state.confirm
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
    fn publish_remote_activity_tracks_running_fetch_checkouts() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into(), "notes".into()],
            },
            &Action::Fetch,
        );
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 2);
        let running: HashSet<String> = ["app".to_string(), "notes".to_string()].into();
        assert_eq!(state.remote_activity.running, running);
        assert_eq!(
            state.remote_activity.batch,
            Some(BatchProgress {
                done: 0,
                total: 2,
                failed: false,
            })
        );
        assert!(interp.take_dirty(), "a new spinner asks for a redraw");

        apply_id(
            &mut interp,
            &mut state,
            jobs[0].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Failed("no remote".into()),
                repo: "app".into(),
            },
        );
        assert_eq!(
            state.remote_activity.running,
            HashSet::from(["notes".to_string()])
        );
        assert_eq!(
            state.remote_activity.batch,
            Some(BatchProgress {
                done: 1,
                total: 2,
                failed: true,
            })
        );

        apply_id(
            &mut interp,
            &mut state,
            jobs[1].0,
            JobOutcome::BulkRemote {
                kind: RunningOp::Fetch,
                result: RepoOpResult::Ok,
                repo: "notes".into(),
            },
        );
        assert_eq!(state.remote_activity, RemoteActivity::default());
    }

    #[test]
    fn background_fetch_tick_sets_no_batch_progress() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into(), "notes".into()],
            },
            &Action::FetchTick,
        );
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 2);
        assert_eq!(
            state.remote_activity.running.len(),
            2,
            "background fetches still spin their rows"
        );
        assert_eq!(
            state.remote_activity.batch, None,
            "the fetch tick paints no progress"
        );
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

    const FETCH_MS: u64 = 300_000;

    /// The claim as another `ws` instance in `state`'s workspace takes it:
    /// a second open of the same lock file.
    fn other_instance_claim(state: &AppState, now: u64) -> Option<BackgroundFetchClaim> {
        try_claim_background_fetch(
            &state.fetch_store_path,
            &workspace_store_id(&state.cwd),
            1,
            now,
        )
        .unwrap()
    }

    fn fetch_record(state: &AppState) -> Option<BackgroundFetchRecord> {
        load_background_fetch_record(&state.fetch_store_path, &workspace_store_id(&state.cwd))
    }

    fn background_tick(interp: &mut Interpreter, state: &mut AppState, now: u64) {
        let tui_opts = opts(state);
        interp.background_fetch_tick(state, &tui_opts, FETCH_MS, now);
    }

    /// Apply every captured fetch job; `failed` names the repo that fails.
    fn finish_fetches(
        interp: &mut Interpreter,
        state: &mut AppState,
        jobs: Vec<(u64, JobWork)>,
        failed: Option<&str>,
    ) {
        for (id, _) in jobs {
            let repo = interp
                .remote
                .occupy
                .values()
                .find_map(|occ| match occ {
                    OccupyReason::Remote(inf) => Some(inf.checkout.clone()),
                    _ => None,
                })
                .expect("a running fetch");
            let result = if failed == Some(repo.as_str()) {
                RepoOpResult::Failed("could not read from remote repository".into())
            } else {
                RepoOpResult::Ok
            };
            apply_id(
                interp,
                state,
                id,
                JobOutcome::BulkRemote {
                    kind: RunningOp::Fetch,
                    result,
                    repo,
                },
            );
        }
    }

    #[test]
    fn background_fetch_tick_skips_while_another_instance_holds_the_claim() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let now = unix_now_ms();
        let other = other_instance_claim(&state, now).expect("the other instance claims");
        background_tick(&mut interp, &mut state, now);
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "no git fetch"
        );
        assert!(interp.pending_remotes().is_empty());
        assert!(interp.fetch_claim.is_none());
        assert_eq!(state.status, "", "no status change");
        assert!(!interp.take_dirty());

        assert_eq!(other.finish(true).unwrap(), 1);
        // This instance's own tick lands shortly after the other start.
        background_tick(&mut interp, &mut state, now + 2_000);
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "a background fetch inside this instance's interval is not run again"
        );
        assert!(state.apply_background_fetch_record());
        assert_eq!(state.status, "fetch failed");
        assert_eq!(state.status.kind(), StatusKind::Error);
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
    }

    #[test]
    fn claimed_background_fetch_records_failure_then_success() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(8);
        let start = unix_now_ms();
        background_tick(&mut interp, &mut state, start);
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 2, "app and lib fetch");
        assert!(
            interp.fetch_claim.is_some(),
            "the live wave holds the claim"
        );
        assert!(
            other_instance_claim(&state, start).is_none(),
            "another instance cannot claim while the wave runs"
        );
        background_tick(&mut interp, &mut state, start + FETCH_MS);
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "a held claim skips the next tick"
        );

        finish_fetches(&mut interp, &mut state, jobs, Some("app"));
        let record = fetch_record(&state).expect("the wave end writes the record");
        assert_eq!(record.generation, 1);
        assert!(record.failed);
        assert_eq!(
            record.started_unix_ms, start,
            "the claim time, not the wave end"
        );
        assert!(interp.fetch_claim.is_none());
        let detailed = state.status.clone();
        assert!(
            detailed.starts_with("Fetched 2 repos (1 failed: app"),
            "{detailed}"
        );
        assert_eq!(detailed.kind(), StatusKind::Error);
        assert!(
            !state.apply_background_fetch_record(),
            "the runner's own watch tick does not repaint"
        );
        assert_eq!(
            state.status, detailed,
            "the runner keeps the detailed status"
        );

        // The wave end queued follow-up loads (snapshot reload); leave
        // them unrun so only the next fetch jobs are captured below.
        let _ = capture_jobs(&mut interp, &mut state);
        // The timer fires one period after the previous tick started, so
        // the wave length must not push this tick into a skip.
        let later = start + FETCH_MS;
        background_tick(&mut interp, &mut state, later);
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 2, "the next timer tick fetches again");
        assert!(interp.fetch_claim.is_some(), "and claims again");
        finish_fetches(&mut interp, &mut state, jobs, None);
        let record = fetch_record(&state).unwrap();
        assert_eq!(record.generation, 2);
        assert!(!record.failed);
        assert_eq!(
            state.status, "",
            "the success clears the runner's own stamp"
        );
        assert!(!state.apply_background_fetch_record());
        assert!(other_instance_claim(&state, later + FETCH_MS).is_some());
    }

    #[test]
    fn manual_fetch_wave_writes_no_record() {
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
        let jobs = capture_jobs(&mut interp, &mut state);
        finish_fetches(&mut interp, &mut state, jobs, Some("app"));
        assert!(state.status.starts_with("Fetched 1 repo (1 failed"));
        assert_eq!(fetch_record(&state), None);
        assert!(!state.fetch_store_path.exists());
    }

    #[test]
    fn background_fetch_tick_off_never_claims() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let tui_opts = opts(&state);
        interp.background_fetch_tick(&mut state, &tui_opts, 0, unix_now_ms());
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
        let dir = state.fetch_store_path.parent().unwrap();
        let claim_files = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_name().to_string_lossy().contains("claim"))
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(claim_files, 0, "no claim lock file in {dir:?}");
    }

    #[test]
    fn background_fetch_tick_with_nothing_to_fetch_releases_the_claim() {
        let snapshot = build_workspace_snapshot(&[], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let mut interp = Interpreter::with_cap(4);
        let now = unix_now_ms();
        background_tick(&mut interp, &mut state, now);
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
        assert!(interp.fetch_claim.is_none());
        assert!(other_instance_claim(&state, now).is_some(), "released");
        assert_eq!(fetch_record(&state), None, "no record");
    }

    #[test]
    fn background_fetch_tick_fetches_without_claim_when_the_store_is_unusable() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let blocker = crate::testutil::unique_dir("ws-fetch-tick-blocked");
        fs::create_dir_all(&blocker).unwrap();
        let not_a_dir = blocker.join("not-a-dir");
        fs::write(&not_a_dir, "x").unwrap();
        state.fetch_store_path = not_a_dir.join("background-fetch.json");
        background_tick(&mut interp, &mut state, unix_now_ms());
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 2, "fetches as before, without dedupe");
        assert!(interp.fetch_claim.is_none());
        finish_fetches(&mut interp, &mut state, jobs, None);
        assert_eq!(state.status, "", "a quiet background success");
        let _ = fs::remove_dir_all(blocker);
    }

    #[test]
    fn timed_out_fetch_wave_finishes_with_the_timeout_reason() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Fetch {
                repos: vec!["app".into(), "notes".into()],
            },
            &Action::Fetch,
        );
        assert!(state.status.starts_with("Fetching"), "{}", state.status);
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 2);
        for ((id, _), repo) in jobs.iter().zip(["app", "notes"]) {
            apply_id(
                &mut interp,
                &mut state,
                *id,
                JobOutcome::BulkRemote {
                    kind: RunningOp::Fetch,
                    result: RepoOpResult::Failed(
                        "git fetch timed out after 120s: the remote did not answer".into(),
                    ),
                    repo: repo.into(),
                },
            );
        }
        assert!(!state.status.starts_with("Fetching"), "{}", state.status);
        assert!(interp.occupied_gitdirs().is_empty());
        assert_eq!(state.status.kind(), StatusKind::Error);
        let text = state.status.to_string();
        assert!(text.contains("timed out"), "{text}");
        assert!(text.contains("the remote did not answer"), "{text}");
        assert!(
            text.contains("2 failed: app — git fetch timed out after 120s"),
            "{text}"
        );
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
        state.status = "showing hidden repos".into();
        state.open_commit_files("app".into(), source, vec![commit_file("README.md")]);
        assert_eq!(state.status, "showing hidden repos");
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
        // The highlighted hit's file was read for the preview pane on the
        // pool (the default terminal is wide enough to paint it).
        let preview = dialog.preview.as_ref().expect("preview requested");
        assert_eq!(
            (preview.checkout.as_str(), preview.rel.as_str()),
            ("app", "src/main.rs")
        );
        match &preview.body {
            Some(FileRead::Text { lines, .. }) => assert_eq!(lines[3], "fn needle() {}"),
            other => panic!("expected the file text, got {other:?}"),
        }

        // Below the preview width nothing is read.
        state.dispatch(Action::SearchFilesCancel);
        state.search_files_parked.as_mut().expect("parked").preview = None;
        state.layout.term_cols = crate::tui::search_files::SEARCH_PREVIEW_MIN_COLS - 1;
        let effect = state.dispatch(Action::ToggleSearchFiles);
        interp.interpret_sync(&mut state, &opts, effect, &Action::None);
        let dialog = state.search_files.as_ref().expect("dialog open");
        assert_eq!(dialog.hits.len(), 2, "restored with its hits");
        assert_eq!(dialog.preview, None, "no preview below 100 columns");
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

    /// Open (or focus) the Explorer tab of `checkout` and make `rel` its
    /// active inner tab: pinned, or the preview tab (replaced in place, as
    /// a tree move does). Returns the tab id and the load generation.
    fn explorer_inner(
        state: &mut AppState,
        checkout: &str,
        rel: &str,
        changed: bool,
        pinned: bool,
    ) -> (u64, u64) {
        state.tabs.open_or_focus_explorer(checkout.into());
        let tab = state.tabs.active_explorer_mut().expect("explorer tab");
        let inner = tab.new_inner(rel, changed, pinned);
        let gen = inner.gen;
        let index = match tab.preview_index().filter(|_| !pinned) {
            Some(index) => {
                tab.files[index] = inner;
                index
            }
            None => {
                tab.files.push(inner);
                tab.files.len() - 1
            }
        };
        tab.active_file = Some(index);
        (tab.id, gen)
    }

    fn explorer_load(tab_id: u64, gen: u64, path: &str) -> Effect {
        Effect::LoadExplorerPreview {
            tab_id,
            gen,
            repo: "app".into(),
            path: path.into(),
            change: None,
        }
    }

    /// Generations of the queued Explorer file loads, by tab, after
    /// running every ready job.
    fn run_explorer_loads(interp: &mut Interpreter, state: &mut AppState) -> Vec<(u64, u64)> {
        let mut previews = Vec::new();
        for (_, work) in capture_jobs(interp, state) {
            match work() {
                JobOutcome::ExplorerPreview { tab_id, gen, .. } => previews.push((tab_id, gen)),
                JobOutcome::ExplorerDir { .. } => {}
                _ => panic!("unexpected job"),
            }
        }
        previews.sort_unstable();
        previews
    }

    /// A held `j` that replaces the preview tab twice before a worker is
    /// free reads only the last file: the first load's generation is no
    /// inner tab's any more. Each tab keeps its own load; folder listings
    /// stay one job each.
    #[test]
    fn explorer_preview_queue_keeps_only_live_loads() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let dir = |tab_id: u64, req: u64| Effect::LoadExplorerDir {
            tab_id,
            req,
            repo: "app".into(),
            rel_dir: String::new(),
            parent_ignored: false,
        };
        let (app, a) = explorer_inner(&mut state, "app", "a.txt", false, false);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(app, a, "a.txt"),
            &Action::None,
        );
        let (_, b) = explorer_inner(&mut state, "app", "b.txt", false, false);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(app, b, "b.txt"),
            &Action::None,
        );
        schedule_effect(&mut interp, &mut state, dir(app, 1), &Action::None);
        schedule_effect(&mut interp, &mut state, dir(app, 2), &Action::None);
        let (lib, c) = explorer_inner(&mut state, "lib", "c.txt", false, false);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(lib, c, "c.txt"),
            &Action::None,
        );
        assert_eq!(interp.explorer_previews.len(), 2, "one live load per tab");
        assert_eq!(interp.explorer_jobs.len(), 2, "listings stay FIFO");
        assert_eq!(
            run_explorer_loads(&mut interp, &mut state),
            [(app, b), (lib, c)],
            "only the newest pick per tab"
        );
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
    }

    /// A pinned file whose load is still queued keeps it when a later tree
    /// move queues the preview tab's load on the same tab.
    #[test]
    fn pinned_inner_load_survives_a_later_preview_load() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let (tab, kept) = explorer_inner(&mut state, "app", "kept.txt", false, true);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(tab, kept, "kept.txt"),
            &Action::None,
        );
        let (_, first) = explorer_inner(&mut state, "app", "a.txt", false, false);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(tab, first, "a.txt"),
            &Action::None,
        );
        let (_, last) = explorer_inner(&mut state, "app", "b.txt", false, false);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(tab, last, "b.txt"),
            &Action::None,
        );
        assert_eq!(
            run_explorer_loads(&mut interp, &mut state),
            [(tab, kept), (tab, last)],
            "the pinned load and the newest preview load run"
        );

        // A closed Explorer tab's queued load is dropped by the next load.
        let (lib, gen) = explorer_inner(&mut state, "lib", "lib.rs", false, true);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(lib, gen, "lib.rs"),
            &Action::None,
        );
        let index = state.tabs.find_explorer("lib").unwrap();
        state.tabs.close_at(index);
        let (_, next) = explorer_inner(&mut state, "app", "c.txt", false, false);
        schedule_effect(
            &mut interp,
            &mut state,
            explorer_load(tab, next, "c.txt"),
            &Action::None,
        );
        assert_eq!(run_explorer_loads(&mut interp, &mut state), [(tab, next)]);
    }

    /// A changed file's Explorer diff loads with the whole file as context,
    /// so a line far from the change still shows.
    #[test]
    fn changed_file_preview_loads_full_file_context() {
        let (root, mut state) = git_app_state("ws-effect-explorer-full", "main", false);
        let app = root.join("app");
        let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        std::fs::write(app.join("long.txt"), lines.join("\n") + "\n").unwrap();
        crate::testutil::git(&app, &["add", "long.txt"]);
        crate::testutil::git(&app, &["commit", "-q", "-m", "long"]);
        let mut changed = lines.clone();
        changed[19] = "line twenty".into();
        std::fs::write(app.join("long.txt"), changed.join("\n") + "\n").unwrap();
        let change = FileChange {
            path: "long.txt".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        };
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);
        let (tab_id, gen) = explorer_inner(&mut state, "app", "long.txt", true, false);
        let load = Effect::LoadExplorerPreview {
            tab_id,
            gen,
            repo: "app".into(),
            path: "long.txt".into(),
            change: Some(change),
        };
        interp.interpret_sync(&mut state, &opts, load, &Action::None);
        let (_, path, diff) = state.explorer_diff().expect("diff view");
        assert_eq!(path, "long.txt");
        assert!(diff.unstaged.contains("+line twenty"), "{diff:?}");
        assert!(
            diff.unstaged.contains(" line 1\n"),
            "line 1 is 19 lines from the change: {diff:?}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    /// The Explorer lists folders and loads previews on the pool: ignored
    /// entries list flagged, a changed file previews its worktree diff, a
    /// clean one its body.
    #[test]
    fn explorer_lists_and_previews_a_real_checkout() {
        let (root, mut state) = git_app_state("ws-effect-explorer", "main", true);
        let app = root.join("app");
        std::fs::write(app.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(app.join("target/debug")).unwrap();
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {}\n").unwrap();
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);
        state.cursor = state
            .rows
            .iter()
            .position(|row| row.kind == crate::tui::tree::NodeKind::Repo)
            .expect("repo row");
        let open = state.dispatch(Action::ExplorerReveal);
        interp.interpret_sync(&mut state, &opts, open, &Action::None);
        let (rows, _) = state.explorer_rows().expect("explorer tab");
        let listed: Vec<(&str, bool)> = rows
            .iter()
            .map(|row| (row.rel.as_str(), row.ignored))
            .collect();
        assert_eq!(
            listed,
            [
                ("src", false),
                ("target", true),
                (".gitignore", false),
                ("README.md", false)
            ]
        );

        let target = state.dispatch(Action::Move(1));
        assert_eq!(target, Effect::None, "a folder row loads nothing");
        let expand = state.dispatch(Action::NavEnter);
        interp.interpret_sync(&mut state, &opts, expand, &Action::None);
        let (rows, _) = state.explorer_rows().unwrap();
        let debug = rows.iter().find(|row| row.rel == "target/debug").unwrap();
        assert!(debug.ignored, "children of an ignored folder are ignored");
        state.dispatch(Action::NavEnter);

        let readme = loop {
            match state.dispatch(Action::Move(1)) {
                effect @ Effect::LoadExplorerPreview { .. } => {
                    if state.explorer_diff().is_some() {
                        break effect;
                    }
                    interp.interpret_sync(&mut state, &opts, effect, &Action::None);
                    let body = state
                        .tabs
                        .active_file_view()
                        .and_then(|tab| tab.body.clone());
                    assert!(
                        matches!(body.as_deref(), Some(FileRead::Text { lines, .. }) if lines == &["target/"]),
                        "{body:?}"
                    );
                }
                other => panic!("{other:?}"),
            }
        };
        interp.interpret_sync(&mut state, &opts, readme, &Action::None);
        let (_, path, diff) = state.explorer_diff().expect("diff preview");
        assert_eq!(path, "README.md");
        assert!(diff.unstaged.contains("+# dirty"), "{diff:?}");
        assert!(!state.explorer_diff_loading());
        let _ = std::fs::remove_dir_all(root);
    }

    /// The inner file load of `effect` (a batch with the Explorer folder
    /// listings): `(tab_id, gen)`.
    fn inner_load_of(effect: &Effect) -> (u64, u64) {
        let effects = match effect {
            Effect::Batch(effects) => effects.as_slice(),
            other => std::slice::from_ref(other),
        };
        effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadExplorerPreview { tab_id, gen, .. } => Some((*tab_id, *gen)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("an inner file load: {effect:?}"))
    }

    #[test]
    fn explorer_file_load_drops_stale_generation() {
        let (root, mut state) = git_app_state("ws-effect-file-view-stale", "main", false);
        std::fs::write(root.join("app/notes.txt"), "old\n").unwrap();
        let mut interp = Interpreter::with_cap(4);
        let first = state.open_file_in_explorer("app".into(), "notes.txt".into(), None, None);
        let (tab_id, first_gen) = inner_load_of(&first);
        schedule_effect(&mut interp, &mut state, first, &Action::None);
        let stale = capture_jobs(&mut interp, &mut state);
        assert!(!stale.is_empty());
        let reload = state.dispatch(Action::Refresh);
        assert!(
            inner_load_of(&reload).1 > first_gen,
            "r loads at a newer generation: {reload:?}"
        );
        std::fs::write(root.join("app/notes.txt"), "fresh\nlines\n").unwrap();
        for (id, work) in stale {
            apply_id(&mut interp, &mut state, id, work());
        }
        assert!(
            state.tabs.active_file_view().unwrap().body.is_none(),
            "the first read is dropped after r"
        );
        let opts = opts(&state);
        interp.interpret_sync(&mut state, &opts, reload, &Action::Refresh);
        assert_eq!(
            state.tabs.active_file_view().unwrap().lines(),
            ["fresh", "lines"]
        );

        // A late result for the replaced generation changes nothing.
        let stale = |gen| JobOutcome::ExplorerPreview {
            tab_id,
            gen,
            body: ExplorerPreviewBody::File(FileRead::Binary),
        };
        apply(&mut interp, &mut state, stale(first_gen));
        assert_eq!(
            state.tabs.active_file_view().unwrap().lines(),
            ["fresh", "lines"],
            "the stale result is dropped"
        );

        // Ctrl-w closes the inner tab, then the Explorer tab; a result for
        // the closed tab is dropped.
        let live_gen = state.tabs.active_file_view().unwrap().generation;
        state.dispatch(Action::CloseTab);
        state.dispatch(Action::CloseTab);
        assert!(state.tabs.is_workspace());
        assert!(!state.apply_explorer_preview(
            tab_id,
            live_gen,
            ExplorerPreviewBody::File(FileRead::Binary)
        ));
        apply(&mut interp, &mut state, stale(live_gen));
        assert!(state.tabs.get_explorer_id(tab_id).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn explorer_file_loads_text_binary_and_too_large() {
        let (root, mut state) = git_app_state("ws-effect-file-view-kinds", "main", false);
        let app = root.join("app");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::write(app.join("src/main.rs"), "fn main() {\n\tprintln!();\n}\n").unwrap();
        std::fs::write(app.join("logo.bin"), b"PNG\0\x01\x02").unwrap();
        std::fs::write(app.join("big.log"), vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
        let opts = opts(&state);
        let mut interp = Interpreter::with_cap(4);
        let mut load = |state: &mut AppState, rel: &str| -> FileRead {
            let effect = state.open_file_in_explorer("app".into(), rel.into(), None, None);
            interp.interpret_sync(state, &opts, effect, &Action::None);
            let tab = state.tabs.active_file_view().expect("a file view");
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
        assert_eq!(state.tabs.len(), 2, "one Explorer tab");
        assert_eq!(
            state.tabs.active_explorer().map(|tab| tab.files.len()),
            Some(4),
            "one pinned inner tab per file"
        );
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

    /// Real repos `app` and `lib` under one root; the snapshot lists both.
    fn two_repo_state(tag: &str) -> (PathBuf, AppState) {
        let root = crate::testutil::unique_dir(tag);
        crate::testutil::init_repo(&root.join("app"));
        crate::testutil::init_repo(&root.join("lib"));
        let snapshot =
            build_workspace_snapshot(&[repo("app", false), repo("lib", false)], &[], false, &[]);
        (root.clone(), AppState::new(root, snapshot, true))
    }

    #[test]
    fn launch_schedules_activity_on_a_worker_not_inline() {
        let (root, mut state) = git_app_state("ws-effect-activity", "main", false);
        state.activity_sparkline = true;
        let mut interp = Interpreter::with_cap(1);
        let checkouts = activity_targets(&state.snapshot, None);
        assert_eq!(checkouts, vec!["app".to_string()]);
        let load = Effect::LoadActivity { checkouts };
        schedule_effect(&mut interp, &mut state, load.clone(), &Action::None);
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 1);
        assert!(state.activity.is_empty(), "schedule runs no git");
        // A second ask while the repo waits does not count it twice.
        schedule_effect(&mut interp, &mut state, load, &Action::None);
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 1);

        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1);
        for (id, work) in jobs {
            let outcome = work();
            assert!(matches!(outcome, JobOutcome::Activity { .. }));
            assert!(
                state.activity.is_empty(),
                "counts land only when the worker outcome is applied"
            );
            apply_id(&mut interp, &mut state, id, outcome);
        }
        let days = state.activity.get("app").expect("app counted");
        assert!(
            days[ACTIVITY_DAYS - 1] >= 1,
            "seed commit is today: {days:?}"
        );
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    /// One job per repo: with one pool slot, a user job queued after the
    /// activity jobs runs before the second repo's count.
    #[test]
    fn activity_jobs_yield_the_slot_to_user_work_between_repos() {
        let (root, mut state) = two_repo_state("ws-effect-activity-yield");
        state.activity_sparkline = true;
        let mut interp = Interpreter::with_cap(1);
        let checkouts = activity_targets(&state.snapshot, None);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LoadActivity { checkouts },
            &Action::None,
        );
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 2);
        let first = capture_jobs(&mut interp, &mut state);
        assert_eq!(first.len(), 1, "cap 1");
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LoadRightPane,
            &Action::None,
        );
        let mut order = Vec::new();
        let mut jobs = first;
        while !jobs.is_empty() {
            for (id, work) in jobs {
                let outcome = work();
                order.push(match &outcome {
                    JobOutcome::Activity { key, .. } => format!("activity:{key}"),
                    _ => "user".to_string(),
                });
                apply_id(&mut interp, &mut state, id, outcome);
            }
            jobs = capture_jobs(&mut interp, &mut state);
        }
        assert_eq!(order.first().map(String::as_str), Some("activity:app"));
        let user = order
            .iter()
            .position(|job| job == "user")
            .expect("pane ran");
        let lib = order
            .iter()
            .position(|job| job == "activity:lib")
            .expect("lib counted");
        assert!(user < lib, "user work runs between repos: {order:?}");
        assert!(state.activity.contains_key("app") && state.activity.contains_key("lib"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// Quit sets the cancel flag: the started job runs no git and the next
    /// repo never starts.
    #[test]
    fn quit_flag_stops_the_activity_walk_before_the_next_repo() {
        let (root, mut state) = two_repo_state("ws-effect-activity-quit");
        state.activity_sparkline = true;
        let mut interp = Interpreter::with_cap(1);
        let checkouts = activity_targets(&state.snapshot, None);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LoadActivity { checkouts },
            &Action::None,
        );
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1);
        interp.cancel_activity_jobs();
        for (id, work) in jobs {
            let outcome = work();
            assert!(
                matches!(outcome, JobOutcome::Activity { days: None, .. }),
                "a cancelled job reports no counts"
            );
            apply_id(&mut interp, &mut state, id, outcome);
        }
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "lib never starts"
        );
        assert!(state.activity.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    /// `r` counts once its reload lands, from the new snapshot: a repo the
    /// reload finds gets bars too.
    #[test]
    fn reload_counts_activity_when_the_snapshot_lands() {
        let (root, mut state) = git_app_state("ws-effect-activity-reload", "main", false);
        state.activity_sparkline = true;
        crate::testutil::init_repo(&root.join("lib"));
        let mut interp = Interpreter::with_cap(4);
        let effect = state.dispatch(Action::Refresh);
        assert_eq!(effect, Effect::ReloadSnapshot);
        schedule_effect(&mut interp, &mut state, effect, &Action::Refresh);
        assert_eq!(
            interp.sched.queued_user_tag(UserTag::Activity),
            0,
            "nothing counted from the old snapshot"
        );
        let mut rounds = 0;
        while !state.activity.contains_key("lib") && rounds < 20 {
            run_jobs(&mut interp, &mut state);
            rounds += 1;
        }
        assert!(
            state.snapshot.repos.iter().any(|row| row.repo == "lib"),
            "the reload found lib"
        );
        assert!(state.activity.contains_key("app"));
        assert!(state.activity.contains_key("lib"), "new repo counted");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn finished_pull_wave_recounts_activity_of_its_repos() {
        let (root, mut state) = git_app_state("ws-effect-activity-pull", "main", false);
        state.activity_sparkline = true;
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Pull {
                repos: vec!["app".into()],
            },
            &Action::Pull,
        );
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 0);
        let mut rounds = 0;
        while state.activity.is_empty() && rounds < 20 {
            run_jobs(&mut interp, &mut state);
            rounds += 1;
        }
        let days = state
            .activity
            .get("app")
            .expect("the pulled repo is counted");
        assert!(days[ACTIVITY_DAYS - 1] >= 1, "{days:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Run spawned jobs until none is left (at most `rounds` passes).
    fn drain_jobs(interp: &mut Interpreter, state: &mut AppState, rounds: usize) {
        for _ in 0..rounds {
            let jobs = capture_jobs(interp, state);
            if jobs.is_empty() {
                return;
            }
            for (id, work) in jobs {
                let outcome = work();
                apply_id(interp, state, id, outcome);
            }
        }
    }

    /// Off (the launch default): the launch ask, a landed `r` reload, and a
    /// finished Pull wave queue no activity count and run no git for it.
    #[test]
    fn sparkline_off_queues_no_activity_on_launch_reload_or_pull() {
        let (root, mut state) = git_app_state("ws-effect-activity-off", "main", false);
        assert!(!state.activity_sparkline, "off by default");
        let mut interp = Interpreter::with_cap(4);
        let checkouts = activity_targets(&state.snapshot, None);
        schedule_effect(
            &mut interp,
            &mut state,
            Effect::LoadActivity { checkouts },
            &Action::None,
        );
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 0, "launch");

        let effect = state.dispatch(Action::Refresh);
        schedule_effect(&mut interp, &mut state, effect, &Action::Refresh);
        drain_jobs(&mut interp, &mut state, 20);
        assert!(!interp.activity_after_reload, "the reload landed");
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 0, "reload");

        schedule_effect(
            &mut interp,
            &mut state,
            Effect::Pull {
                repos: vec!["app".into()],
            },
            &Action::Pull,
        );
        drain_jobs(&mut interp, &mut state, 20);
        assert!(!interp.kind_live(RunningOp::Pull), "the pull wave finished");
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 0, "pull");
        assert!(interp.activity_pending.is_empty());
        assert!(state.activity.is_empty(), "no count ran");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Toggling the sparkline on mid-session queues one count per repo, so
    /// the bars appear without a reload; toggling it off queues nothing.
    #[test]
    fn toggle_sparkline_on_queues_one_count_per_repo() {
        let (root, mut state) = two_repo_state("ws-effect-activity-toggle");
        state.ui_store_path = root.join("state").join("ui-state.json");
        let mut interp = Interpreter::with_cap(1);
        let effect = state.dispatch(Action::ToggleActivitySparkline);
        assert!(state.activity_sparkline);
        assert_eq!(
            effect,
            Effect::LoadActivity {
                checkouts: vec!["app".to_string(), "lib".to_string()],
            }
        );
        schedule_effect(
            &mut interp,
            &mut state,
            effect,
            &Action::ToggleActivitySparkline,
        );
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 2);
        drain_jobs(&mut interp, &mut state, 10);
        assert!(state.activity.contains_key("app") && state.activity.contains_key("lib"));

        let effect = state.dispatch(Action::ToggleActivitySparkline);
        assert!(!state.activity_sparkline);
        assert_eq!(effect, Effect::None);
        schedule_effect(
            &mut interp,
            &mut state,
            effect,
            &Action::ToggleActivitySparkline,
        );
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Toggling off before the queued counts start: their slots finish
    /// and run no git, so no bars fill while the sparkline is off.
    #[test]
    fn toggle_sparkline_off_drops_counts_still_queued() {
        let (root, mut state) = two_repo_state("ws-effect-activity-toggle-off");
        state.ui_store_path = root.join("state").join("ui-state.json");
        let mut interp = Interpreter::with_cap(1);
        let effect = state.dispatch(Action::ToggleActivitySparkline);
        schedule_effect(
            &mut interp,
            &mut state,
            effect,
            &Action::ToggleActivitySparkline,
        );
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 2);

        let effect = state.dispatch(Action::ToggleActivitySparkline);
        assert!(!state.activity_sparkline);
        schedule_effect(
            &mut interp,
            &mut state,
            effect,
            &Action::ToggleActivitySparkline,
        );
        drain_jobs(&mut interp, &mut state, 10);
        assert!(state.activity.is_empty(), "no count ran");
        assert!(interp.activity_pending.is_empty());
        assert_eq!(interp.sched.queued_user_tag(UserTag::Activity), 0);
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    fn doctor_row(name: &'static str) -> Check {
        Check {
            group: crate::tui::doctor::CheckGroup::Tools,
            name,
            status: crate::tui::doctor::CheckStatus::Ok,
            why: "found.".into(),
            fix: None,
        }
    }

    /// Open queues one probe job; a re-run before spawn keeps it at one
    /// (latest-only). The worker answer fills the rows. The probe closure
    /// is never run here: it reads the real host setup.
    #[test]
    fn doctor_opens_with_one_latest_only_probe_and_its_answer_lands() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let effect = state.dispatch(Action::OpenDoctor);
        assert_eq!(effect, Effect::RunDoctor { gen: 1 });
        schedule_effect(&mut interp, &mut state, effect, &Action::OpenDoctor);
        assert_eq!(interp.sched.queued_user_tag(UserTag::Doctor), 1);
        let effect = state.dispatch(Action::DoctorRerun);
        schedule_effect(&mut interp, &mut state, effect, &Action::DoctorRerun);
        assert_eq!(
            interp.sched.queued_user_tag(UserTag::Doctor),
            1,
            "a re-run replaces the queued request"
        );
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1, "one probe");
        assert_eq!(interp.doctor_slot, None, "the job took the newest request");
        let (id, _probe) = jobs.into_iter().next().unwrap();
        apply_id(
            &mut interp,
            &mut state,
            id,
            JobOutcome::Doctor {
                gen: 2,
                checks: vec![doctor_row("git"), doctor_row("curl")],
            },
        );
        let doctor = state.doctor.as_ref().expect("open");
        assert_eq!(doctor.checks.as_ref().map(Vec::len), Some(2));
        assert!(!interp.doctor_inflight);
        assert!(capture_jobs(&mut interp, &mut state).is_empty(), "idle");
    }

    /// A re-run while a probe runs queues the next probe when the first
    /// lands, and the first answer is dropped. A Doctor closed before the
    /// spawn runs nothing, and a late answer after close changes nothing.
    #[test]
    fn doctor_drops_stale_answers_and_spawns_nothing_once_closed() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let effect = state.dispatch(Action::OpenDoctor);
        schedule_effect(&mut interp, &mut state, effect, &Action::OpenDoctor);
        let (first, _probe) = capture_jobs(&mut interp, &mut state).pop().unwrap();
        let effect = state.dispatch(Action::DoctorRerun);
        schedule_effect(&mut interp, &mut state, effect, &Action::DoctorRerun);
        assert_eq!(
            interp.sched.queued_user_tag(UserTag::Doctor),
            0,
            "one probe at a time"
        );
        let stale = JobOutcome::Doctor {
            gen: 1,
            checks: vec![doctor_row("old")],
        };
        apply_id(&mut interp, &mut state, first, stale);
        assert_eq!(state.doctor.as_ref().unwrap().checks, None, "stale");
        assert_eq!(interp.sched.queued_user_tag(UserTag::Doctor), 1, "next");

        // Closed before the queued probe spawns: no work starts.
        state.dispatch(Action::DoctorClose);
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
        assert!(!interp.doctor_inflight);

        // Closed while a probe runs: its answer changes nothing.
        let effect = state.dispatch(Action::OpenDoctor);
        schedule_effect(&mut interp, &mut state, effect, &Action::OpenDoctor);
        let (id, _probe) = capture_jobs(&mut interp, &mut state).pop().unwrap();
        state.dispatch(Action::DoctorClose);
        apply_id(
            &mut interp,
            &mut state,
            id,
            JobOutcome::Doctor {
                gen: 3,
                checks: vec![doctor_row("late")],
            },
        );
        assert!(state.doctor.is_none());
        assert!(capture_jobs(&mut interp, &mut state).is_empty());
    }

    /// The job copies every loaded checkout (a hidden one and a detached
    /// one too) on the event thread, and only its worker side runs git:
    /// the remote URLs come from each checkout's own config, deduplicated,
    /// whether or not any PR badge lookup ran.
    #[test]
    fn doctor_job_carries_every_checkout_and_reads_remotes_with_git() {
        let root = crate::testutil::unique_dir("ws-effect-doctor-remotes");
        for name in ["app", "lib", "wt", "plain"] {
            crate::testutil::init_repo(&root.join(name));
        }
        let remote = |name: &str, remote: &str, url: &str| {
            crate::testutil::git(&root.join(name), &["remote", "add", remote, url]);
        };
        remote("app", "origin", "https://example.invalid/octo/app.git");
        remote("lib", "upstream", "https://example.invalid/octo/lib.git");
        // Same remote as `app`: listed once.
        remote("wt", "origin", "https://example.invalid/octo/app.git");
        let mut detached = repo("wt", false);
        detached.branch = "HEAD".into();
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", false),
                repo("lib", false),
                detached,
                repo("plain", false),
            ],
            &["lib".into()],
            false,
            &[],
        );
        let state = AppState::new(root.clone(), snapshot, true);
        let job = DoctorJob::new(&state, &opts(&state));
        let mut checkouts = job.checkouts.clone();
        checkouts.sort();
        assert_eq!(
            checkouts,
            vec![
                (root.join("app"), "main".to_string()),
                (root.join("lib"), "main".to_string()),
                (root.join("plain"), "main".to_string()),
                (root.join("wt"), "HEAD".to_string()),
            ]
        );
        let mut urls = job.remote_urls();
        urls.sort();
        assert_eq!(
            urls,
            vec![
                "https://example.invalid/octo/app.git".to_string(),
                "https://example.invalid/octo/lib.git".to_string(),
            ]
        );
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

    #[test]
    fn explorer_file_view_blames_the_focused_line_with_git() {
        let root = crate::testutil::unique_dir("ws-effect-explorer-blame");
        let dir = root.join("app");
        crate::testutil::init_repo(&dir);
        std::fs::write(dir.join("README.md"), "a\nb\nc\n").unwrap();
        crate::testutil::git(&dir, &["commit", "-q", "-am", "three lines"]);
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(root.clone(), snapshot, true);
        state.open_loaded_inner_file("app", "README.md", &["a", "b", "c"]);
        // The open focuses the file; start from the tree.
        state.dispatch(Action::FocusLeft);
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(&mut interp, &mut state, Effect::None, &Action::Move(1));
        assert!(
            capture_jobs(&mut interp, &mut state).is_empty(),
            "the tree has focus: no git"
        );
        let effect = state.dispatch(Action::FocusRight);
        schedule_effect(&mut interp, &mut state, effect, &Action::FocusRight);
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1, "the cursor line asks once");
        for (id, work) in jobs {
            apply_id(&mut interp, &mut state, id, work());
        }
        let (text, side) = state.painted_line_annotation().expect("annotation");
        assert_eq!(side, BlameSide::New);
        assert!(text.ends_with(" · three lines"), "{text}");
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
        // A compare tab whose focused diff line blames `ccc`.
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab = state.tabs.active_compare_mut().unwrap();
        tab.source = Some(CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: "bbb".into(),
            merge_base: "bbb".into(),
            head: "head-app".into(),
        });
        tab.files = vec![CommitFile {
            status: "M".into(),
            path: "README.md".into(),
            old_path: None,
            stat: None,
        }];
        tab.path = Some("README.md".into());
        tab.content = DiffContent::from_compare_lines(vec!["@@ -1 +1 @@".into(), "+a".into()]);
        state.focus = FocusPane::Right;
        state.diff_cursor = 2;
        let key = state.line_blame_want().expect("cursor line asks");
        state.apply_line_blame(
            key,
            Ok(Some(crate::git::LineBlame {
                sha: "ccc".into(),
                author: "Ada".into(),
                author_time: 0,
                summary: "s-ccc".into(),
                orig_line: 1,
                filename: "README.md".into(),
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

    /// Drill commit `sha` (`a`, `b` → `a`, `B` in README.md) with the diff
    /// focused. Rows: section, hunk, `a`, `-b`, `+B`.
    fn drill_commit_readme(state: &mut AppState, sha: &str) {
        state.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: sha.into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
            0,
            "README.md".into(),
            DiffContent::from_unified("@@ -1,2 +1,2 @@\n a\n-b\n+B\n"),
        );
        state.focus = FocusPane::Right;
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

        // Commit `two` drilled on README.md, cursor on its line 2 (`B`).
        drill_commit_readme(&mut state, &rev("HEAD"));
        state.diff_cursor = 4;
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

        // Commit `two` drilled on README.md, cursor on line 1 (`a`, from
        // `one`).
        drill_commit_readme(&mut state, &rev("HEAD"));
        state.diff_cursor = 2;
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

    fn gh_approved_pr(argv: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
        assert_eq!(argv.first().map(String::as_str), Some("gh"), "{argv:?}");
        Ok(format!(
            r#"[{{"number":7,"state":"OPEN","url":"{PR_URL}","headRefName":"feature","isCrossRepository":false,"reviewDecision":"APPROVED","updatedAt":"2026-01-02T00:00:00Z"}}]"#
        ))
    }

    fn gh_no_pr(_: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
        Ok("[]".into())
    }

    fn gh_fails(_: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
        Err(Some(
            "HTTP 401: Bad credentials (https://api.github.com/graphql)".into(),
        ))
    }

    fn cli_must_not_run(argv: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
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
            Some(crate::tui::pr_status::PrState::Ready),
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
        assert_eq!((interp.pr_cli)(&["gh".to_string()], &live), Err(None));
        assert_eq!((interp.pr_opener)(PR_URL, &live), Err(()));
        let effect = state.dispatch(Action::OpenPullRequest);
        let opts = opts(&state);
        interp.interpret_sync(&mut state, &opts, effect, &Action::OpenPullRequest);
        assert_eq!(state.status, "could not look up PR for feature");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Answers "no PR" until the quit flag is set, then fails as a killed
    /// CLI would.
    fn gh_no_pr_until_cancelled(
        _: &[String],
        cancel: &AtomicBool,
    ) -> Result<String, Option<String>> {
        if cancel.load(Ordering::Relaxed) {
            Err(None)
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

    const VIEW_REMOTE: &str = "git@github.example.invalid:octo/demo.git";
    const VIEW_BASE: &str = "https://github.example.invalid/octo/demo";

    /// Real checkout `app` on `feature` with a GitHub-looking `origin` and
    /// `refs/remotes/origin/feature` at HEAD (a stand-in for a push), focused
    /// in the tree.
    fn view_app(tag: &str) -> (PathBuf, AppState) {
        let (root, mut state) = git_app_state(tag, "feature", false);
        let dir = root.join("app");
        crate::testutil::git(&dir, &["remote", "add", "origin", VIEW_REMOTE]);
        crate::testutil::git(&dir, &["update-ref", "refs/remotes/origin/feature", "HEAD"]);
        focus_repo(&mut state, "app");
        (root, state)
    }

    /// Show `rows` as the graph of the focused repo, focus it, and put the
    /// cursor on the first row `pick` accepts.
    fn focus_graph_row(state: &mut AppState, model: GraphModel, pick: fn(&GraphRow) -> bool) {
        let at = model
            .visible_rows()
            .iter()
            .position(pick)
            .expect("graph row");
        state.graph = Some(model);
        state.focus = FocusPane::Right;
        assert!(state.graph_pane_focused());
        state.graph_cursor = at;
    }

    /// "View remotely" on the focused row with the fake opener.
    fn run_view_remotely(state: &mut AppState) -> Effect {
        let mut interp = Interpreter::with_cap(4);
        interp.pr_cli = cli_must_not_run;
        interp.pr_opener = opener_ok;
        let effect = state.dispatch(Action::ViewRemotely);
        let opts = opts(state);
        interp.interpret_sync(state, &opts, effect.clone(), &Action::ViewRemotely);
        effect
    }

    #[test]
    fn view_remotely_opens_the_repository_home() {
        let (root, mut state) = view_app("ws-effect-view-repo");
        let _ = opened_urls();
        run_view_remotely(&mut state);
        assert_eq!(opened_urls(), vec![VIEW_BASE.to_string()]);
        assert_eq!(state.status, format!("opened {VIEW_BASE}"));
        assert_eq!(state.status.kind(), StatusKind::Ok);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn view_remotely_opens_a_pushed_graph_commit_and_refuses_an_unpushed_one() {
        let (root, mut state) = view_app("ws-effect-view-commit");
        let dir = root.join("app");
        let pushed = crate::git::exec_git(&["rev-parse", "HEAD"], &dir);
        let _ = opened_urls();
        focus_graph_row(&mut state, mini_graph(&[&pushed]), |row| {
            matches!(row, GraphRow::Commit { .. })
        });
        run_view_remotely(&mut state);
        let want = format!("{VIEW_BASE}/commit/{pushed}");
        assert_eq!(opened_urls(), vec![want.clone()]);
        assert_eq!(state.status, format!("opened {want}"));

        std::fs::write(dir.join("local.txt"), "local\n").unwrap();
        crate::testutil::git(&dir, &["add", "local.txt"]);
        crate::testutil::git(&dir, &["commit", "-q", "-m", "local"]);
        let local = crate::git::exec_git(&["rev-parse", "HEAD"], &dir);
        focus_graph_row(&mut state, mini_graph(&[&local, &pushed]), |row| {
            matches!(row, GraphRow::Commit { .. })
        });
        run_view_remotely(&mut state);
        assert_eq!(
            state.status,
            format!("commit {} is not on origin", &local[..7])
        );
        assert_eq!(state.status.kind(), StatusKind::Warn);
        assert!(opened_urls().is_empty(), "nothing opens");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn view_remotely_on_a_stash_row_says_local_only_and_queues_nothing() {
        let (root, mut state) = view_app("ws-effect-view-stash");
        let mut model = mini_graph(&["a".repeat(40).as_str()]);
        model.stashes = vec![workspace_status_graph::Stash {
            id: "5".repeat(40),
            stash_ref: "stash@{0}".into(),
            parent_id: Some("a".repeat(40)),
            ..Default::default()
        }];
        focus_graph_row(&mut state, model, |row| matches!(row, GraphRow::Stash(_)));
        let mut interp = Interpreter::with_cap(4);
        interp.pr_opener = opener_ok;
        let effect = state.dispatch(Action::ViewRemotely);
        assert_eq!(effect, Effect::None);
        assert_eq!(state.status, "stash@{0} is local only");
        assert_eq!(state.status.kind(), StatusKind::Warn);
        let opts = opts(&state);
        interp.schedule(&mut state, &opts, effect, &Action::ViewRemotely);
        assert!(interp.remote_views.is_empty());
        assert_eq!(interp.sched.queued_user_tag(UserTag::PullRequest), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn view_remotely_shares_the_pr_slot_after_queued_gx_opens() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(8);
        interp.pr_cli = cli_must_not_run;
        interp.pr_opener = opener_ok;
        for effect in [
            Effect::ViewRemotely {
                checkout: PathBuf::from("missing-view"),
                target: RemoteTarget::Repo,
            },
            Effect::OpenPullRequest {
                repo: PathBuf::from("missing-open"),
                branch: "feature".into(),
            },
        ] {
            schedule_effect(&mut interp, &mut state, effect, &Action::None);
        }
        assert_eq!(interp.sched.queued_user_tag(UserTag::PullRequest), 2);
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 2, "each open has its own slot");
        let outcomes: Vec<JobOutcome> = jobs.into_iter().map(|(_, work)| work()).collect();
        assert!(matches!(outcomes[0], JobOutcome::PullRequestOpen { .. }));
        assert!(
            matches!(
                &outcomes[1],
                JobOutcome::RemoteView { page: Err(text), opened: false }
                    if text == remote_view::NO_REMOTE
            ),
            "a missing checkout has no remote"
        );
        assert!(opened_urls().is_empty());
    }

    #[test]
    fn view_remotely_job_after_quit_runs_no_check() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        interp.pr_opener = opener_ok;
        let effect = Effect::ViewRemotely {
            checkout: PathBuf::from("missing-view"),
            target: RemoteTarget::Repo,
        };
        schedule_effect(&mut interp, &mut state, effect, &Action::None);
        interp.cancel_pull_request_jobs();
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1);
        let outcomes: Vec<JobOutcome> = jobs.into_iter().map(|(_, work)| work()).collect();
        assert!(
            matches!(
                &outcomes[0],
                JobOutcome::RemoteView { page: Err(text), opened: false }
                    if text == remote_view::OPEN_FAILED
            ),
            "the remote check (NO_REMOTE here) never runs"
        );
        assert!(opened_urls().is_empty());
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
    fn gh_view_until_cancelled(
        argv: &[String],
        cancel: &AtomicBool,
    ) -> Result<String, Option<String>> {
        assert_eq!(
            argv.get(..3),
            Some(&["gh".to_string(), "pr".into(), "view".into()][..])
        );
        if cancel.load(Ordering::Relaxed) {
            return Err(None);
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
            PrLookup::Found(crate::tui::pull_request::PullRequest::fixture(
                7,
                PR_URL,
                crate::tui::pr_status::PrState::InReview,
            )),
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
    fn applied_status_looks_up_new_branches_only_and_polls_and_refresh_look_up_again() {
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

        // The poll cycle looks a known PR up again and keeps its badge.
        assert!(state.apply_pr_lookup(
            Path::new("app"),
            "main",
            Some(DETAIL_REMOTE.into()),
            PrLookup::Found(crate::tui::pull_request::PullRequest::fixture(
                7,
                PR_URL,
                crate::tui::pr_status::PrState::InReview,
            )),
        ));
        let tui_opts = opts(&state);
        interp.pr_poll_tick(&mut state, &tui_opts, Instant::now());
        assert_eq!(
            interp.pr_badges,
            VecDeque::from([(PathBuf::from("app"), "main".to_string())]),
            "lib has no answer yet, so it is not polled"
        );
        assert_eq!(
            state.pr_badge(Path::new("app")),
            Some(crate::tui::pr_status::PrState::InReview),
            "a poll keeps the badge"
        );
        apply_app_status(&mut interp, &mut state, 0, "main");
        assert_eq!(interp.pr_badges.len(), 1, "a status apply adds nothing");
        interp.pr_badges.clear();

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

    #[test]
    fn a_checkout_queued_or_running_is_not_queued_again() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(8);
        interp.pr_cli = cli_must_not_run;
        let target = |i: usize| (PathBuf::from(format!("missing-{i}")), "feature".to_string());
        let lookups = |targets: Vec<(PathBuf, String)>| Effect::LookupPullRequests { targets };
        schedule_effect(
            &mut interp,
            &mut state,
            lookups(vec![target(0), target(1), target(2), target(2)]),
            &Action::None,
        );
        assert_eq!(interp.pr_badges.len(), 3, "a repeat in one batch");
        let running = capture_jobs(&mut interp, &mut state);
        assert_eq!(running.len(), 2, "0 and 1 run");
        schedule_effect(
            &mut interp,
            &mut state,
            lookups(vec![target(0), target(1), target(2), target(3)]),
            &Action::None,
        );
        assert_eq!(
            interp.pr_badges,
            VecDeque::from([target(2), target(3)]),
            "running 0 and 1 and queued 2 are not queued again"
        );
        for (id, work) in running {
            apply_id(&mut interp, &mut state, id, work());
        }
        schedule_effect(
            &mut interp,
            &mut state,
            lookups(vec![target(0)]),
            &Action::None,
        );
        assert_eq!(
            interp.pr_badges,
            VecDeque::from([target(2), target(3), target(0)]),
            "a finished lookup can be queued again"
        );
    }

    #[test]
    fn a_landed_badge_lookup_asks_a_pinned_popover_detail_again_after_a_change() {
        let (mut state, detail) = detail_state();
        let mut interp = Interpreter::with_cap(4);
        interp.pr_cli = gh_view_until_cancelled;
        let tui_opts = opts(&state);
        interp.interpret_sync(&mut state, &tui_opts, detail, &Action::None);
        assert!(matches!(
            state.pull_request_detail_for(Path::new("app")),
            Some(crate::tui::state::PrDetailState::Ready(_))
        ));
        let badge = |pr_state| JobOutcome::PullRequestBadge {
            repo: PathBuf::from("app"),
            branch: "main".into(),
            remote: Some(DETAIL_REMOTE.into()),
            lookup: PrLookup::Found(crate::tui::pull_request::PullRequest::fixture(
                7, PR_URL, pr_state,
            )),
        };
        apply(
            &mut interp,
            &mut state,
            badge(crate::tui::pr_status::PrState::InReview),
        );
        assert!(interp.pr_details.is_empty(), "the same answer asks nothing");

        apply(
            &mut interp,
            &mut state,
            badge(crate::tui::pr_status::PrState::Ready),
        );
        assert_eq!(interp.pr_details.len(), 1, "a changed answer asks again");
        assert!(state.pr_detail_awaited(DETAIL_REMOTE, "main", 2));
        assert!(
            matches!(
                state.pull_request_detail_for(Path::new("app")),
                Some(crate::tui::state::PrDetailState::Ready(_))
            ),
            "the old detail stays on screen meanwhile"
        );
    }

    #[test]
    fn a_poll_tick_queues_pollable_checkouts_once() {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        let _ = state.due_pr_lookups();
        assert!(state.apply_pr_lookup(
            Path::new("app"),
            "main",
            Some(DETAIL_REMOTE.into()),
            PrLookup::Found(crate::tui::pull_request::PullRequest::fixture(
                7,
                PR_URL,
                crate::tui::pr_status::PrState::CiRunning,
            )),
        ));
        let tui_opts = opts(&state);
        let t0 = Instant::now();
        interp.pr_poll_tick(&mut state, &tui_opts, t0);
        let app = (PathBuf::from("app"), "main".to_string());
        assert_eq!(interp.pr_badges, VecDeque::from([app.clone()]));
        interp.pr_poll_tick(
            &mut state,
            &tui_opts,
            t0 + std::time::Duration::from_secs(60),
        );
        assert_eq!(
            interp.pr_badges,
            VecDeque::from([app]),
            "still in flight: not queued again"
        );
    }
}
