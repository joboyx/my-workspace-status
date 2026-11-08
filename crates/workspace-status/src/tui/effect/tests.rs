use std::fs;
use std::path::PathBuf;

use workspace_status_graph::{Commit, GraphModel, GraphRow};

use crate::config::WorkspaceStatusConfig;
use crate::git::{LocalBranch, NameStatus};
use crate::snapshot::{build_workspace_snapshot, SyncStatus};
use crate::testutil::{app_state, app_state_of, repo, unstaged_change};
use crate::tui::app::{CompareRangeLoad, MergeCompute, TuiOpts};
use crate::tui::diff::DiffContent;
use crate::tui::drill::{CommitFile, CommitFileSource, DrillView};
use crate::tui::fetch_claim::{load_background_fetch_record, unix_now_ms, BackgroundFetchRecord};
use crate::tui::graph_load::GraphIdentity;
use crate::tui::line_blame::BlameSide;
use crate::tui::outage::OutageSource;
use crate::tui::state::{AppState, FocusPane};

use super::*;
use crate::tui::chrome::{diverged_pull_status, STATUS_NOTHING_TO_PULL};
use crate::tui::status::StatusKind;

pub(super) fn fixture_state() -> AppState {
    app_state_of(build_workspace_snapshot(
        &[repo("app", true), repo("notes", true), repo("lib", true)],
        &["notes".into()],
        false,
        &[],
    ))
}

pub(super) fn opts(state: &AppState) -> TuiOpts {
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
    let effect = include_str!("../effect.rs");
    let loop_src = include_str!("../event_loop.rs");
    let app = include_str!("../app.rs");

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

pub(super) fn compare_source() -> CommitFileSource {
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
        commit: None,
    }
}

pub(super) fn open_compare_tab(state: &mut AppState) -> (u64, u64) {
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
            latest: Some(crate::testutil::stash_target("stash@{0}")),
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
            latest: Some(crate::testutil::stash_target("stash@{0}")),
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
            latest: Some(crate::testutil::stash_target("stash@{0}")),
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
            latest: Some(crate::testutil::stash_target("stash@{0}")),
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

pub(super) fn capture_jobs(interp: &mut Interpreter, state: &mut AppState) -> Vec<(u64, JobWork)> {
    let tui_opts = opts(state);
    let mut jobs = Vec::new();
    interp.spawn_ready(state, &tui_opts, &mut |id, work| {
        jobs.push((id, work));
    });
    jobs
}

pub(super) fn schedule_effect(
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
    app_state(&[repo("app", false), wt])
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
    let mut state = app_state(&[app, lib]);
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
    let refused = write_status(
        "revert",
        StatusKind::Ok,
        Err("revert aborted: HEAD moved".into()),
    );
    assert_eq!(refused, "revert aborted: HEAD moved");
    assert_eq!(refused.kind(), StatusKind::Warn);
    let failed = write_status(
        "revert",
        StatusKind::Ok,
        Err("patch failed: a.txt:1".into()),
    );
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
    // The no-write upstream-rewrite warns are idle pull lines too.
    assert!(is_idle_pull_status(
        "app (+1 more) upstream rewritten — focus one repo and press p"
    ));
    assert!(is_idle_pull_status(
            "app: origin/main was force-pushed under your 1 commit — run git rebase --onto origin/main 0123456789ab main"
        ));
    assert!(!is_idle_pull_status("moving main to origin/main…"));
}

#[test]
fn workspace_pull_during_fetch_that_opens_the_reset_box_still_pulls_the_rest() {
    let mut app = repo("app", false);
    app.sync_status = SyncStatus::Diverged;
    app.sync_note = "diverged (ahead 2, behind 1)".into();
    app.upstream_rewrite = Some(crate::git::UpstreamRewrite {
        upstream: "origin/main".into(),
        fork_point: "a".repeat(40),
        tip: "a".repeat(40),
        yours: 0,
        dropped: 2,
    });
    let mut lib = repo("lib", false);
    lib.sync_status = SyncStatus::UpToDate;
    // The leftover status must not change the outcome: a live
    // `Fetching…` line and an idle line from an earlier `p` both queue.
    for before in ["Fetching 0/2…", STATUS_NOTHING_TO_PULL] {
        let mut state = app_state(&[app.clone(), lib.clone()]);
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
        state.status = before.into();
        let effect = state.dispatch(Action::Pull);
        assert_eq!(effect, Effect::None);
        assert!(
            matches!(state.confirm, Some(PendingConfirm::ResetToUpstream { .. })),
            "{before}: {:?}",
            state.confirm
        );
        schedule_effect(&mut interp, &mut state, effect, &Action::Pull);
        assert_eq!(
            interp.pending_remotes(),
            vec![(RunningOp::Pull, "lib".into())],
            "{before}"
        );
        assert!(state.confirm.is_some(), "the box stays up");
    }
}

#[test]
fn workspace_pull_during_fetch_with_your_commits_on_a_rewrite_still_pulls_the_rest() {
    let mut app = repo("app", false);
    app.sync_status = SyncStatus::Diverged;
    app.sync_note = "diverged (ahead 3, behind 1)".into();
    app.upstream_rewrite = Some(crate::git::UpstreamRewrite {
        upstream: "origin/main".into(),
        fork_point: "0123456789abcdef0123456789abcdef01234567".into(),
        tip: "a".repeat(40),
        yours: 1,
        dropped: 2,
    });
    let mut lib = repo("lib", false);
    lib.sync_status = SyncStatus::UpToDate;
    let mut state = app_state(&[app, lib]);
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
    let effect = state.dispatch(Action::Pull);
    assert_eq!(effect, Effect::None);
    assert!(
        state.status.contains("run git rebase --onto"),
        "{}",
        state.status
    );
    schedule_effect(&mut interp, &mut state, effect, &Action::Pull);
    assert_eq!(
        interp.pending_remotes(),
        vec![(RunningOp::Pull, "lib".into())]
    );
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
fn reset_confirm_on_occupied_gitdir_keeps_overlay() {
    use crate::tui::state::PendingConfirm;

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
    state.confirm = Some(PendingConfirm::ResetToUpstream {
        repo: "app".into(),
        branch: "main".into(),
        upstream: "origin/main".into(),
        tip: "a".repeat(40),
        dropped: 2,
    });
    state.status = "move main?".into();
    assert!(interp.keep_overlay_if_gitdir_busy(&mut state, &Action::ConfirmYes));
    assert!(state.confirm.is_some(), "the reset box stays up");
    assert_eq!(state.status, "busy");
    // `Y` only keeps the box open, so it is never busy.
    state.status = "move main?".into();
    assert!(!interp.keep_overlay_if_gitdir_busy(&mut state, &Action::ConfirmYesClean));
    assert_eq!(state.status, "move main?");

    // A reset that reaches `schedule` on the occupied gitdir is refused.
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::ResetToUpstream {
            repo: "app".into(),
            branch: "main".into(),
            upstream: "origin/main".into(),
            tip: "a".repeat(40),
        },
        &Action::ConfirmYes,
    );
    assert_eq!(state.status, "busy");
    assert_eq!(interp.write_jobs_queued(), 0);
}

/// Workspace at the fixture root with the checkout under test `checkout`,
/// its snapshot built from a real `process_repo`, and the row focused.
fn rewrite_state(fixture: &crate::testutil::RewriteFixture) -> AppState {
    let meta = RepoCheckoutMeta {
        checkout_kind: crate::snapshot::CheckoutKind::Primary,
        primary_repo: None,
    };
    let snap = crate::discovery::process_repo("checkout", &fixture.root, false, None, &meta)
        .expect("repo snapshot");
    assert!(snap.upstream_rewrite.is_some(), "{snap:?}");
    let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
    let mut state = AppState::new(fixture.root.clone(), snapshot, true);
    focus_repo(&mut state, "checkout");
    state
}

/// `p` then `y` on the focused rewritten checkout; schedules the reset.
fn press_p_then_y(interp: &mut Interpreter, state: &mut AppState) {
    let effect = state.dispatch(Action::Pull);
    schedule_effect(interp, state, effect, &Action::Pull);
    assert!(
        matches!(state.confirm, Some(PendingConfirm::ResetToUpstream { .. })),
        "{:?}",
        state.confirm
    );
    assert!(!interp.keep_overlay_if_gitdir_busy(state, &Action::ConfirmYes));
    let effect = state.dispatch(Action::ConfirmYes);
    assert!(
        matches!(effect, Effect::ResetToUpstream { .. }),
        "{effect:?}"
    );
    schedule_effect(interp, state, effect, &Action::ConfirmYes);
    assert_eq!(state.status, "moving main to origin/main…");
    assert_eq!(interp.write_jobs_queued(), 1, "one exclusive write");
    assert!(interp.busy_for_writes());
}

#[test]
fn reset_confirm_y_moves_the_branch_off_the_loop_thread_and_names_the_undo() {
    use crate::testutil::{upstream_rewrite_fixture, RewriteKind};

    let fixture = upstream_rewrite_fixture(RewriteKind::Dropped);
    let mut state = rewrite_state(&fixture);
    let mut interp = Interpreter::with_cap(4);
    press_p_then_y(&mut interp, &mut state);
    let head = || crate::git::exec_git(&["rev-parse", "HEAD"], &fixture.checkout);
    assert_eq!(head(), fixture.local_tip, "no git on the loop thread");

    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1);
    for (id, work) in jobs {
        let outcome = work();
        assert!(matches!(outcome, JobOutcome::Write { .. }));
        apply_id(&mut interp, &mut state, id, outcome);
    }
    assert_eq!(
        state.status,
        format!(
            "Moved main to origin/main · undo: git reset --keep {}",
            &fixture.local_tip[..12]
        )
    );
    // A note, not an ok ack: it stays on the status line instead of
    // moving to a toast that clips the undo sha.
    assert_eq!(state.status.kind(), StatusKind::Info);
    let undo = state.status.to_string();
    state.route_ack_to_toast(std::time::Instant::now());
    assert_eq!(state.status, undo, "the undo hint stays on the status line");
    assert_eq!(head(), fixture.upstream_tip);
    assert!(!interp.busy_for_writes(), "the write released the gitdir");

    // The follow-up reload shows the checkout in sync.
    let tui_opts = opts(&state);
    interp.pump_sync(&mut state, &tui_opts);
    let snap = state
        .snapshot
        .repos
        .iter()
        .find(|r| r.repo == "checkout")
        .expect("checkout row after reload");
    assert_eq!(snap.sync_status, SyncStatus::UpToDate);
    assert_eq!(snap.upstream_rewrite, None);
}

#[test]
fn reset_confirm_y_on_a_dirty_checkout_refuses_and_keeps_the_tip() {
    use crate::testutil::{upstream_rewrite_fixture, RewriteKind};

    let fixture = upstream_rewrite_fixture(RewriteKind::Dropped);
    let mut state = rewrite_state(&fixture);
    std::fs::write(fixture.checkout.join("base.txt"), "edited\n").unwrap();
    let mut interp = Interpreter::with_cap(4);
    press_p_then_y(&mut interp, &mut state);
    run_jobs(&mut interp, &mut state);
    assert_eq!(state.status, "reset failed: working tree has changes");
    assert_eq!(state.status.kind(), StatusKind::Error);
    assert_eq!(
        crate::git::exec_git(&["rev-parse", "HEAD"], &fixture.checkout),
        fixture.local_tip
    );
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
        stash: None,
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
        stash: Some(crate::testutil::stash_target("stash@{0}")),
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

/// Texts of the scheduled-fetch outage toasts shown now.
fn fetch_outage_toasts(state: &AppState) -> Vec<String> {
    state
        .toasts
        .visible(Instant::now())
        .filter(|(toast, _)| toast.outage == Some(OutageSource::ScheduledFetch))
        .map(|(toast, _)| toast.text.clone())
        .collect()
}

/// True while the scheduled fetch is an outage (its header mark shows).
fn fetch_outage(state: &AppState) -> bool {
    state.outages.get(&OutageSource::ScheduledFetch).is_some()
}

#[test]
fn background_fetch_failure_starts_the_outage_not_a_status_error() {
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
    assert_eq!(state.status, "", "no status-slot error");
    assert_eq!(
        fetch_outage_toasts(&state),
        ["git fetch (app): could not read from remote repository"]
    );
    assert!(fetch_outage(&state), "the header mark shows");
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
    assert!(state.apply_background_fetch_record(Instant::now()));
    assert_eq!(state.status, "", "no status-slot error");
    assert_eq!(
        fetch_outage_toasts(&state),
        ["git fetch: failed in another ws"]
    );
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
    assert_eq!(state.status, "", "no status-slot error");
    let detailed = vec!["git fetch (app): could not read from remote repository"];
    assert_eq!(fetch_outage_toasts(&state), detailed);
    assert!(
        !state.apply_background_fetch_record(Instant::now()),
        "the runner's own watch tick does not apply its record"
    );
    assert_eq!(
        fetch_outage_toasts(&state),
        detailed,
        "the runner keeps the detailed toast"
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
    assert!(!fetch_outage(&state), "the success ends the outage");
    assert!(fetch_outage_toasts(&state).is_empty());
    assert_eq!(state.status, "");
    assert!(!state.apply_background_fetch_record(Instant::now()));
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
    assert!(!fetch_outage(&state), "a key fetch starts no outage");
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
    let mut state = app_state(&[]);
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
    finish_fetches(&mut interp, &mut state, jobs, Some("app"));
    assert_eq!(state.status, "", "no status-slot error");
    assert!(
        fetch_outage(&state),
        "a failure with no claim and no record still starts the outage"
    );
    let _ = capture_jobs(&mut interp, &mut state);

    background_tick(&mut interp, &mut state, unix_now_ms());
    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 2, "no dedupe: the next tick fetches again");
    finish_fetches(&mut interp, &mut state, jobs, None);
    assert_eq!(state.status, "", "a quiet background success");
    assert!(!fetch_outage(&state), "and it ends the outage");
    let _ = fs::remove_dir_all(blocker);
}

#[test]
fn foreground_fetch_neither_starts_nor_ends_the_scheduled_fetch_outage() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(4);
    let fetch = |interp: &mut Interpreter, state: &mut AppState, failed: Option<&str>| {
        schedule_effect(
            interp,
            state,
            Effect::Fetch {
                repos: vec!["app".into()],
            },
            &Action::Fetch,
        );
        let jobs = capture_jobs(interp, state);
        finish_fetches(interp, state, jobs, failed);
        let _ = capture_jobs(interp, state);
    };
    fetch(&mut interp, &mut state, Some("app"));
    assert_eq!(
        state.status,
        "Fetched 1 repo (1 failed: app — could not read from remote repository)"
    );
    assert_eq!(state.status.kind(), StatusKind::Error, "the status slot");
    assert!(!fetch_outage(&state), "no outage for a key press");
    assert!(fetch_outage_toasts(&state).is_empty());

    background_tick(&mut interp, &mut state, unix_now_ms());
    let jobs = capture_jobs(&mut interp, &mut state);
    finish_fetches(&mut interp, &mut state, jobs, Some("app"));
    let _ = capture_jobs(&mut interp, &mut state);
    assert!(fetch_outage(&state));
    fetch(&mut interp, &mut state, None);
    assert_eq!(state.status, "Fetched 1 repo");
    assert!(
        fetch_outage(&state),
        "a good `f` does not end the scheduled-fetch outage"
    );
    assert_eq!(fetch_outage_toasts(&state).len(), 1);
}

#[test]
fn a_tick_that_queues_nothing_does_not_mark_the_next_key_fetch_as_scheduled() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(4);
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::Pull {
            repos: vec!["app".into(), "lib".into()],
        },
        &Action::Pull,
    );
    let pulls = capture_jobs(&mut interp, &mut state);
    assert_eq!(pulls.len(), 2, "app and lib pull");

    // A pull holds every tick target: the tick queues no fetch.
    background_tick(&mut interp, &mut state, unix_now_ms());
    assert!(!interp.kind_live(RunningOp::Fetch), "no fetch wave starts");
    assert!(interp.fetch_claim.is_none(), "the claim is released");
    for ((id, _), repo) in pulls.into_iter().zip(["app", "lib"]) {
        apply_id(
            &mut interp,
            &mut state,
            id,
            JobOutcome::BulkRemote {
                kind: RunningOp::Pull,
                result: RepoOpResult::Ok,
                repo: repo.into(),
            },
        );
    }
    let _ = capture_jobs(&mut interp, &mut state);

    // A failing `f` writes the status slot only: no outage starts.
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::Fetch {
            repos: vec!["app".into()],
        },
        &Action::Fetch,
    );
    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "the `f` fetch");
    finish_fetches(&mut interp, &mut state, jobs, Some("app"));
    assert_eq!(
        state.status,
        "Fetched 1 repo (1 failed: app — could not read from remote repository)"
    );
    assert!(!fetch_outage(&state), "a key fetch starts no outage");
    assert!(fetch_outage_toasts(&state).is_empty());
}

#[test]
fn a_key_fetch_that_joins_the_scheduled_wave_shows_its_line_and_feeds_the_outage() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(8);
    background_tick(&mut interp, &mut state, unix_now_ms());
    assert!(interp.fetch_claim.is_some(), "the tick claims the wave");
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::Fetch {
            repos: vec!["app".into()],
        },
        &Action::Fetch,
    );
    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 2, "app and lib, app once");
    finish_fetches(&mut interp, &mut state, jobs, Some("app"));
    assert!(
        state.status.starts_with("Fetched 2 repos (1 failed: app"),
        "the key gets its result line: {}",
        state.status
    );
    assert_eq!(state.status.kind(), StatusKind::Error);
    assert_eq!(
        fetch_outage_toasts(&state),
        ["git fetch (app): could not read from remote repository"],
        "the claim outcome still feeds the outage"
    );
    assert!(fetch_record(&state).expect("the claim writes").failed);
    assert!(!state.apply_background_fetch_record(Instant::now()));
}

/// One scheduled fetch tick at `now` on real checkouts: run its git
/// fetch jobs, apply them, and leave the wave-end follow-up loads unrun.
fn run_scheduled_fetch(interp: &mut Interpreter, state: &mut AppState, now: u64) {
    background_tick(interp, state, now);
    let jobs = capture_jobs(interp, state);
    assert!(!jobs.is_empty(), "the tick fetches");
    for (id, work) in jobs {
        let outcome = work();
        apply_id(interp, state, id, outcome);
    }
    assert!(interp.fetch_claim.is_none(), "the wave ended");
    let _ = capture_jobs(interp, state);
}

#[test]
fn scheduled_fetch_outage_toasts_once_per_outage_on_a_real_remote() {
    let (root, mut state) = git_app_state("ws-effect-fetch-outage", "main", false);
    let app = root.join("app");
    let missing = root.join("missing.git");
    let bare = root.join("remote.git");
    let set_origin = |url: &Path| {
        crate::testutil::git(
            &app,
            &["remote", "set-url", "origin", url.to_str().unwrap()],
        );
    };
    crate::testutil::git(
        &app,
        &["remote", "add", "origin", missing.to_str().unwrap()],
    );
    crate::testutil::git(
        &root,
        &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let mut interp = Interpreter::with_cap(4);
    let start = unix_now_ms();
    let tick = |n: u64| start + n * FETCH_MS;

    // An origin with no repo behind it: the wave fails.
    run_scheduled_fetch(&mut interp, &mut state, tick(0));
    assert!(fetch_record(&state).unwrap().failed);
    assert_eq!(state.status, "", "no status-slot error");
    let toasts = fetch_outage_toasts(&state);
    assert_eq!(toasts.len(), 1, "{toasts:?}");
    assert!(toasts[0].starts_with("git fetch (app): "), "{toasts:?}");
    assert!(fetch_outage(&state), "the header mark shows");

    // The next failed wave: no new toast.
    run_scheduled_fetch(&mut interp, &mut state, tick(1));
    assert_eq!(fetch_outage_toasts(&state), toasts, "no second toast");

    // Esc: the toast goes, the outage (header mark) stays, and a
    // failure while it lasts brings no toast back.
    assert!(state.dismiss_outage_toasts());
    assert!(fetch_outage_toasts(&state).is_empty());
    run_scheduled_fetch(&mut interp, &mut state, tick(2));
    assert!(fetch_outage_toasts(&state).is_empty());
    assert!(fetch_outage(&state));

    // A real remote: the good wave ends the outage.
    set_origin(&bare);
    run_scheduled_fetch(&mut interp, &mut state, tick(3));
    assert!(!fetch_record(&state).unwrap().failed);
    assert!(!fetch_outage(&state), "recovered");
    assert!(fetch_outage_toasts(&state).is_empty());
    assert_eq!(state.status, "", "a quiet background success");

    // It fails again: a new outage, a new toast.
    set_origin(&missing);
    run_scheduled_fetch(&mut interp, &mut state, tick(4));
    assert_eq!(fetch_outage_toasts(&state), toasts, "toasts again");
    assert!(fetch_outage(&state));
    let _ = std::fs::remove_dir_all(root);
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

    // Below the preview width nothing is read. Ctrl-f parks the dialog
    // with its query (Esc would clear the query first).
    state.dispatch(Action::ToggleSearchFiles);
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

/// Generation of a Quick Open or search-in-files outcome.
fn file_job_gen(outcome: &JobOutcome) -> u64 {
    match outcome {
        JobOutcome::FileIndex { gen, .. }
        | JobOutcome::FileScore { gen, .. }
        | JobOutcome::SearchIndex { gen, .. }
        | JobOutcome::SearchChunk { gen, .. }
        | JobOutcome::SearchPreview { gen, .. } => *gen,
        _ => panic!("not a Quick Open or search-in-files outcome"),
    }
}

/// Each latest-only Quick Open and search-in-files request kind keeps
/// only the newest request: two requests before a spawn queue one
/// slot, and the one job runs the newer request. The jobs run here over
/// no roots and an empty index, so they read no files.
#[test]
fn a_newer_file_job_request_replaces_the_waiting_one() {
    fn file_index(gen: u64) -> Effect {
        Effect::LoadFileIndex {
            gen,
            roots: Vec::new(),
        }
    }
    fn file_score(gen: u64) -> Effect {
        Effect::ScoreFiles {
            gen,
            index: Arc::new(FileIndex::default()),
            query: "main".into(),
        }
    }
    fn search_index(gen: u64) -> Effect {
        Effect::LoadSearchIndex {
            gen,
            roots: Vec::new(),
        }
    }
    fn search_chunk(gen: u64) -> Effect {
        Effect::SearchFilesChunk {
            gen,
            index: Arc::new(FileIndex::default()),
            query: "needle".into(),
            options: SearchOptions::default(),
            start: 0,
            room: 10,
        }
    }
    type Request = fn(u64) -> Effect;
    let kinds: [(&str, UserTag, Request); 4] = [
        ("file index", UserTag::QuickOpen, file_index),
        ("file score", UserTag::QuickOpen, file_score),
        ("search index", UserTag::SearchFiles, search_index),
        ("search chunk", UserTag::SearchFiles, search_chunk),
    ];
    for (kind, tag, request) in kinds {
        let mut state = fixture_state();
        let mut interp = Interpreter::with_cap(4);
        schedule_effect(&mut interp, &mut state, request(1), &Action::None);
        schedule_effect(&mut interp, &mut state, request(2), &Action::None);
        assert_eq!(interp.sched.queued_user_tag(tag), 1, "{kind}: one slot");
        // A chunk starts only while its search is live.
        interp.search_live.store(2, Ordering::Relaxed);
        let jobs = capture_jobs(&mut interp, &mut state);
        assert_eq!(jobs.len(), 1, "{kind}: one job");
        let (_, work) = jobs.into_iter().next().unwrap();
        assert_eq!(file_job_gen(&work()), 2, "{kind}: the newer request ran");

        // A request while that job runs gets its own slot.
        schedule_effect(&mut interp, &mut state, request(3), &Action::None);
        assert_eq!(interp.sched.queued_user_tag(tag), 1, "{kind}: next slot");
    }
}

/// Explorer loads and the latest-only index and score requests share
/// the Quick Open tag. Each waiting request gets a slot, in the order
/// Explorer load, index, score, although the Explorer load queued its
/// slot last.
#[test]
fn quick_open_jobs_sharing_a_tag_each_get_a_slot_in_order() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(4);
    let effects = [
        Effect::LoadFileIndex {
            gen: 1,
            roots: Vec::new(),
        },
        Effect::ScoreFiles {
            gen: 1,
            index: Arc::new(FileIndex::default()),
            query: "main".into(),
        },
        Effect::LoadExplorerDir {
            tab_id: 1,
            req: 1,
            repo: "app".into(),
            rel_dir: String::new(),
            parent_ignored: true,
        },
    ];
    for effect in effects {
        schedule_effect(&mut interp, &mut state, effect, &Action::None);
    }
    assert_eq!(interp.sched.queued_user_tag(UserTag::QuickOpen), 3);
    let kinds: Vec<&str> = capture_jobs(&mut interp, &mut state)
        .into_iter()
        .map(|(_, work)| match work() {
            JobOutcome::ExplorerDir { .. } => "explorer",
            JobOutcome::FileIndex { .. } => "index",
            JobOutcome::FileScore { .. } => "score",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, ["explorer", "index", "score"]);
}

/// Real repo `app` with `needle` in `a.txt` and `b.txt`, searched in
/// the search-in-files dialog. The highlight is on `a.txt` and its
/// preview is read.
pub(super) fn searched_needles(tag: &str) -> (PathBuf, AppState, Interpreter) {
    let (root, mut state) = git_app_state(tag, "main", false);
    std::fs::write(root.join("app/a.txt"), "needle a\n").unwrap();
    std::fs::write(root.join("app/b.txt"), "needle b\n").unwrap();
    let opts = opts(&state);
    let mut interp = Interpreter::with_cap(4);
    let effect = state.dispatch(Action::ToggleSearchFiles);
    interp.interpret_sync(&mut state, &opts, effect, &Action::None);
    for c in "needle".chars() {
        let typed = Action::SearchFilesChar(c);
        let effect = state.dispatch(typed.clone());
        interp.interpret_sync(&mut state, &opts, effect, &typed);
    }
    let due = state
        .search_files
        .as_ref()
        .and_then(|d| d.due)
        .expect("armed");
    let effect = state.fire_search_files_due(due).expect("due");
    interp.interpret_sync(&mut state, &opts, effect, &Action::None);
    let (_, first) = search_preview_of(&state).expect("first hit read");
    assert_eq!(first, "a.txt");
    (root, state, interp)
}

/// Generation and file of the search-in-files preview.
pub(super) fn search_preview_of(state: &AppState) -> Option<(u64, String)> {
    let preview = state.search_files.as_ref()?.preview.as_ref()?;
    Some((preview.gen, preview.rel.clone()))
}

/// Two highlight moves before the preview read spawns: one slot, and
/// the one read is the newer file.
#[test]
fn a_newer_search_preview_replaces_the_waiting_read() {
    let (root, mut state, mut interp) = searched_needles("ws-effect-search-preview");
    let preview_of = search_preview_of;

    // Down to b.txt, then back up to a.txt, with no spawn between.
    for delta in [1, -1] {
        let moved = Action::SearchFilesMove(delta);
        let effect = state.dispatch(moved.clone());
        schedule_effect(&mut interp, &mut state, effect, &moved);
    }
    let (newest, rel) = preview_of(&state).expect("preview waits");
    assert_eq!(rel, "a.txt");
    assert_eq!(interp.sched.queued_user_tag(UserTag::SearchFiles), 1);
    assert_eq!(
        interp.search_preview_job.waiting(),
        Some(&(newest, "app".to_string(), "a.txt".to_string()))
    );
    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "one read");
    let (id, work) = jobs.into_iter().next().unwrap();
    let outcome = work();
    assert_eq!(file_job_gen(&outcome), newest, "the newer read ran");
    apply_id(&mut interp, &mut state, id, outcome);
    let dialog = state.search_files.as_ref().expect("dialog open");
    match dialog
        .preview
        .as_ref()
        .and_then(|preview| preview.body.as_ref())
    {
        Some(FileRead::Text { lines, .. }) => assert_eq!(lines[0], "needle a"),
        _ => panic!("expected the text of a.txt"),
    }
    let _ = std::fs::remove_dir_all(root);
}

/// Open (or focus) the Explorer tab of `checkout` and make `rel` its
/// active inner tab: pinned, or the preview tab (replaced in place, as
/// a tree move does). Returns the tab id and the load generation.
pub(super) fn explorer_inner(
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

pub(super) fn explorer_load(tab_id: u64, gen: u64, path: &str) -> Effect {
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
    let change = unstaged_change("long.txt");
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
    // The stash entry exists, so the pop reaches git, which refuses it:
    // the local change would be overwritten.
    let dir = root.join("app");
    std::fs::write(dir.join("README.md"), "# stashed\n").unwrap();
    crate::testutil::git(&dir, &["stash", "-q"]);
    let stash = crate::git::latest_stash(&dir).expect("stash");
    std::fs::write(dir.join("README.md"), "# local\n").unwrap();
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::StashPop {
            repo: "app".into(),
            stash,
        },
        &Action::None,
    );
    run_jobs(&mut interp, &mut state);
    assert!(
        state
            .status
            .starts_with("stash pop failed: Your local changes to the following files"),
        "{}",
        state.status
    );
    assert_eq!(state.status.kind(), StatusKind::Error);
    let _ = std::fs::remove_dir_all(root);
}

/// Stash `content` into README.md of `dir` and return that entry.
fn push_stash(dir: &std::path::Path, content: &str) -> crate::git::StashTarget {
    std::fs::write(dir.join("README.md"), content).unwrap();
    crate::testutil::git(dir, &["stash", "-q"]);
    crate::git::latest_stash(dir).expect("stash")
}

fn stash_shas(dir: &std::path::Path) -> Vec<String> {
    crate::git::list_stashes(dir)
        .into_iter()
        .map(|stash| stash.sha)
        .collect()
}

#[test]
fn stash_pop_and_drop_act_on_the_picked_entry_after_a_newer_stash() {
    let (root, mut state) = git_app_state("ws-effect-stash-by-sha", "main", false);
    let dir = root.join("app");
    let mut interp = Interpreter::with_cap(4);

    // Picked as stash@{0}; a newer stash then moves it to stash@{1}.
    let picked = push_stash(&dir, "# picked\n");
    assert_eq!(picked.stash_ref, "stash@{0}");
    let newer = push_stash(&dir, "# newer\n");
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::StashPop {
            repo: "app".into(),
            stash: picked.clone(),
        },
        &Action::None,
    );
    run_jobs(&mut interp, &mut state);
    assert_eq!(state.status, "popped stash@{1}");
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "# picked\n"
    );
    assert_eq!(stash_shas(&dir), vec![newer.sha.clone()]);

    // Drop: the picked entry sits under a newer one again.
    crate::testutil::git(&dir, &["checkout", "-q", "--", "README.md"]);
    let picked = push_stash(&dir, "# drop me\n");
    let newest = push_stash(&dir, "# keep me\n");
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::StashDrop {
            repo: "app".into(),
            stash: picked,
        },
        &Action::None,
    );
    run_jobs(&mut interp, &mut state);
    assert_eq!(state.status, "dropped stash@{1}");
    assert_eq!(stash_shas(&dir), vec![newest.sha, newer.sha]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stash_pop_refuses_when_the_picked_entry_is_gone() {
    let (root, mut state) = git_app_state("ws-effect-stash-gone", "main", false);
    let dir = root.join("app");
    let mut interp = Interpreter::with_cap(4);
    let picked = push_stash(&dir, "# picked\n");
    // Another tool drops it, and a new stash takes stash@{0}.
    crate::testutil::git(&dir, &["stash", "drop", "-q"]);
    let other = push_stash(&dir, "# other\n");
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::StashPop {
            repo: "app".into(),
            stash: picked.clone(),
        },
        &Action::None,
    );
    run_jobs(&mut interp, &mut state);
    let short = &picked.sha[..7];
    assert_eq!(
        state.status,
        format!(
            "stash pop failed: stash@{{0}} ({short}) is no longer in the stash list; nothing changed"
        )
    );
    assert_eq!(state.status.kind(), StatusKind::Error);
    assert_eq!(stash_shas(&dir), vec![other.sha], "the other stash stays");
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
pub(super) fn blame_fixture(tag: &str) -> (PathBuf, AppState) {
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

/// Peek a commit popover section for commit `sha` of `app`.
pub(super) fn peek_commit(state: &mut AppState, sha: &str) {
    use crate::tui::icons::IconKind;
    use crate::tui::popover::{IconTarget, PopoverOrigin, PopoverState};
    state.popover = Some(PopoverState {
        origin: PopoverOrigin::Peek,
        targets: vec![(
            IconKind::CommitHash,
            IconTarget::Commit {
                repo: "app".into(),
                sha: sha.into(),
            },
        )],
        anchor: None,
        focus_line: 0,
        owner: state.popover_owner(),
    });
}

#[test]
fn a_commit_popover_off_the_graph_queues_one_card_lookup() {
    let root = crate::testutil::unique_dir("ws-effect-commit-card");
    let dir = root.join("app");
    crate::testutil::init_repo(&dir);
    let head = crate::git::exec_git(&["rev-parse", "HEAD"], &dir);
    assert_eq!(head.len(), 40, "{head:?}");
    let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
    let mut state = AppState::new(root.clone(), snapshot, true);
    let mut interp = Interpreter::with_cap(4);

    peek_commit(&mut state, &head);
    schedule_effect(&mut interp, &mut state, Effect::None, &Action::None);
    let mut jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "one card lookup");

    // Newer commits while git runs: no second job, the slot keeps the
    // newest.
    let replaced = "d".repeat(40);
    peek_commit(&mut state, &replaced);
    schedule_effect(&mut interp, &mut state, Effect::None, &Action::None);
    assert_eq!(
        interp.commit_card_job.waiting(),
        Some(&("app".to_string(), replaced))
    );
    let missing = "e".repeat(40);
    peek_commit(&mut state, &missing);
    schedule_effect(&mut interp, &mut state, Effect::None, &Action::None);
    assert_eq!(
        interp.commit_card_job.waiting(),
        Some(&("app".to_string(), missing.clone())),
        "the newer commit replaced the waiting one"
    );
    assert_eq!(interp.sched.queued_user_tag(UserTag::CommitCard), 0);
    assert!(capture_jobs(&mut interp, &mut state).is_empty());

    // The stale card fills the cache without a paint, then the newer
    // commit runs.
    let (id, work) = jobs.remove(0);
    let _ = interp.take_dirty();
    apply_id(&mut interp, &mut state, id, work());
    assert!(
        !interp.take_dirty(),
        "a card no popover shows needs no paint"
    );
    let card = state.commit_cards.get("app", &head).expect("cached");
    assert_eq!(
        card.as_ref().map(|commit| commit.subject.as_str()),
        Ok("seed")
    );
    let next = capture_jobs(&mut interp, &mut state);
    assert_eq!(next.len(), 1, "the newer commit runs after the first apply");
    for (id, work) in next {
        apply_id(&mut interp, &mut state, id, work());
    }
    assert!(interp.take_dirty(), "the shown card paints");
    assert_eq!(
        state.commit_cards.get("app", &missing),
        Some(&Err("commit not found".to_string()))
    );

    // Both answered: no more git.
    peek_commit(&mut state, &head);
    schedule_effect(&mut interp, &mut state, Effect::None, &Action::None);
    assert!(capture_jobs(&mut interp, &mut state).is_empty());
    let _ = std::fs::remove_dir_all(root);
}

pub(super) fn move_diff_cursor(interp: &mut Interpreter, state: &mut AppState, row: usize) {
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
    let replaced = state.line_blame_want().expect("removed line asks");
    assert_eq!(interp.line_blame_job.waiting(), Some(&replaced));
    move_diff_cursor(&mut interp, &mut state, 5);
    let latest = state.line_blame_want().expect("context line asks");
    assert_ne!(replaced, latest);
    assert_eq!(
        interp.line_blame_job.waiting(),
        Some(&latest),
        "the newer line replaced the waiting one"
    );
    assert_eq!(interp.sched.queued_user_tag(UserTag::LineBlame), 0);
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

/// A reload whose walk finds no checkout ends there: the tree empties, the
/// pending activity count is consumed, and the next watch tick collects.
#[test]
fn reload_that_discovers_nothing_ends_the_collect() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(4);
    schedule_effect(
        &mut interp,
        &mut state,
        Effect::ReloadSnapshot,
        &Action::Refresh,
    );
    assert!(interp.activity_after_reload);
    let gen = interp.sched.collection_gen().expect("the reload collects");
    apply(
        &mut interp,
        &mut state,
        JobOutcome::Discovered {
            gen,
            entries: vec![],
        },
    );
    assert!(state.snapshot.repos.is_empty(), "no checkout is left");
    assert!(!interp.sched.collection_active(), "the collect ended");
    assert!(!interp.activity_after_reload, "the reload landed");
    assert!(
        interp.sched.on_watch_tick(None),
        "the next watch tick starts a collect"
    );
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
pub(super) fn drain_jobs(interp: &mut Interpreter, state: &mut AppState, rounds: usize) {
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
    assert_eq!(interp.doctor_job.waiting(), Some(&2), "the newest request");
    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "one probe");
    assert_eq!(
        interp.doctor_job.waiting(),
        None,
        "the job took the newest request"
    );
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
    assert!(!interp.doctor_job.holds_slot());
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
    assert!(!interp.doctor_job.holds_slot());

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
    let mut state = app_state(&[app_repo]);
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

pub(super) fn gh_approved_pr(argv: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
    assert_eq!(argv.first().map(String::as_str), Some("gh"), "{argv:?}");
    Ok(format!(
        r#"[{{"number":7,"state":"OPEN","url":"{PR_URL}","headRefName":"feature","isCrossRepository":false,"reviewDecision":"APPROVED","updatedAt":"2026-01-02T00:00:00Z"}}]"#
    ))
}

pub(super) fn gh_no_pr(_: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
    Ok("[]".into())
}

fn gh_fails(_: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
    Err(Some(
        "HTTP 401: Bad credentials (https://api.github.com/graphql)".into(),
    ))
}

/// Status of `gx` / `gc` when [`gh_fails`] answers: it names the reason.
const GH_FAILS_STATUS: &str =
    "could not look up PR for feature: HTTP 401: Bad credentials (https://api.github.com/graphql)";

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
pub(super) fn pr_app(tag: &str, remote: Option<&str>) -> (PathBuf, AppState) {
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
            page: PrPage::Pr,
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
        (gh_fails, opener_ok, GH_FAILS_STATUS, StatusKind::Error, 0),
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

const FAILED_JOB_URL: &str = "https://github.com/octo/demo/actions/runs/1/job/2";
const GL_MR_URL: &str = "https://gitlab.com/acme/services/api/-/merge_requests/12";
const GL_PIPELINE_URL: &str = "https://gitlab.com/acme/services/api/-/pipelines/99";

/// `gh pr list`: PR #7 with a failed check that links its job.
fn gh_failed_pr(argv: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
    assert_eq!(argv.first().map(String::as_str), Some("gh"), "{argv:?}");
    Ok(format!(
        r#"[{{"number":7,"state":"OPEN","url":"{PR_URL}","headRefName":"feature","isCrossRepository":false,"statusCheckRollup":[{{"name":"test","status":"COMPLETED","conclusion":"FAILURE","detailsUrl":"{FAILED_JOB_URL}"}}]}}]"#
    ))
}

/// `glab api`: MR !12 on the list call, its failed head pipeline on the
/// MR call.
fn glab_failed_mr(argv: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
    assert_eq!(argv.first().map(String::as_str), Some("glab"), "{argv:?}");
    let endpoint = argv.last().expect("endpoint");
    if endpoint.contains("merge_requests?") {
        Ok(format!(
            r#"[{{"iid":12,"state":"opened","web_url":"{GL_MR_URL}","source_branch":"feature","source_project_id":1,"target_project_id":1}}]"#
        ))
    } else {
        Ok(format!(
            r#"{{"iid":12,"head_pipeline":{{"id":99,"status":"failed","web_url":"{GL_PIPELINE_URL}"}}}}"#
        ))
    }
}

/// `gc` on the focused row with fake CLI and opener; returns the effect
/// it dispatched and the status.
fn run_gc(state: &mut AppState, cli: PrCliRunner, opener: PrUrlOpener) -> (Effect, StatusMessage) {
    let mut interp = Interpreter::with_cap(4);
    interp.pr_cli = cli;
    interp.pr_opener = opener;
    state.status = StatusMessage::default();
    let effect = state.dispatch(Action::OpenPullRequestCi);
    let opts = opts(state);
    interp.interpret_sync(state, &opts, effect.clone(), &Action::OpenPullRequestCi);
    (effect, state.status.clone())
}

/// `gc` with no PR answer yet looks the PR up first, then opens the CI
/// page: the failed check (GitHub), the head pipeline (GitLab).
#[test]
fn gc_looks_the_pr_up_then_opens_its_ci_page_on_both_forges() {
    let cases: [(&str, &str, PrCliRunner, &str, &str); 2] = [
        (
            "ws-effect-gc-github",
            "git@github.com:octo/demo.git",
            gh_failed_pr,
            FAILED_JOB_URL,
            "#7",
        ),
        (
            "ws-effect-gc-gitlab",
            "git@gitlab.com:acme/services/api.git",
            glab_failed_mr,
            GL_PIPELINE_URL,
            "!12",
        ),
    ];
    for (tag, remote, cli, url, pr_ref) in cases {
        let (root, mut state) = pr_app(tag, Some(remote));
        let _ = opened_urls();
        let (effect, status) = run_gc(&mut state, cli, opener_ok);
        assert!(
            matches!(
                effect,
                Effect::OpenPullRequest {
                    page: PrPage::Ci,
                    ..
                }
            ),
            "{effect:?}"
        );
        assert_eq!(status, format!("opened CI for {pr_ref}"));
        assert_eq!(status.kind(), StatusKind::Ok);
        assert_eq!(opened_urls(), vec![url.to_string()]);
        assert_eq!(
            state.pr_badge(Path::new("app")),
            Some(crate::tui::pr_status::PrState::CiFailed),
            "the lookup feeds the badge"
        );

        // Now the answer is known: the next `gc` opens at once, with no
        // forge call.
        let (effect, status) = run_gc(&mut state, cli_must_not_run, opener_ok);
        assert_eq!(
            effect,
            Effect::OpenPullRequestCi {
                pr_ref: pr_ref.into(),
                url: url.into(),
            }
        );
        assert_eq!(status, format!("opened CI for {pr_ref}"));
        assert_eq!(opened_urls(), vec![url.to_string()]);

        let (_, status) = run_gc(&mut state, cli_must_not_run, opener_fails);
        assert_eq!(status, "could not open CI");
        assert_eq!(status.kind(), StatusKind::Error);
        assert_eq!(opened_urls(), vec![url.to_string()]);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn gc_reports_no_pr_a_failed_lookup_and_a_failed_open() {
    let cases: [(&str, PrCliRunner, PrUrlOpener, &str, StatusKind, usize); 3] = [
        (
            "ws-effect-gc-no-pr",
            gh_no_pr,
            opener_ok,
            "no PR for feature",
            StatusKind::Warn,
            0,
        ),
        (
            "ws-effect-gc-cli-fails",
            gh_fails,
            opener_ok,
            GH_FAILS_STATUS,
            StatusKind::Error,
            0,
        ),
        (
            "ws-effect-gc-open-fails",
            gh_failed_pr,
            opener_fails,
            "could not open CI",
            StatusKind::Error,
            1,
        ),
    ];
    for (tag, cli, opener, text, kind, opens) in cases {
        let (root, mut state) = pr_app(tag, Some("git@github.com:octo/demo.git"));
        let _ = opened_urls();
        let (_, status) = run_gc(&mut state, cli, opener);
        assert_eq!(status, text);
        assert_eq!(status.kind(), kind, "{text}");
        assert_eq!(opened_urls().len(), opens, "{text}");
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
fn gh_no_pr_until_cancelled(_: &[String], cancel: &AtomicBool) -> Result<String, Option<String>> {
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
            page: PrPage::Pr,
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
            page: PrPage::Pr,
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
fn gh_view_until_cancelled(argv: &[String], cancel: &AtomicBool) -> Result<String, Option<String>> {
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

    // A second change while that fetch is out: the detail is asked
    // again once it lands, so it never stays older than the badge.
    apply(
        &mut interp,
        &mut state,
        badge(crate::tui::pr_status::PrState::CiRunning),
    );
    assert_eq!(interp.pr_details.len(), 1, "one fetch at a time");
    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "the queued detail fetch runs");
    for (id, work) in jobs {
        apply_id(&mut interp, &mut state, id, work());
    }
    assert!(
        state.pr_detail_awaited(DETAIL_REMOTE, "main", 3),
        "asked again after it landed"
    );
    assert_eq!(interp.pr_details.len(), 1);
}

#[test]
fn a_default_branch_checkout_links_commits_with_no_forge_call() {
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    let root = crate::testutil::unique_dir("ws-effect-default-branch-link");
    let dir = root.join("app");
    crate::testutil::init_repo(&dir);
    crate::testutil::git(
        &dir,
        &["remote", "add", "origin", "git@github.com:octo/demo.git"],
    );
    let mut row = repo("app", false);
    row.branch = "main".into();
    row.default_branch = Some("main".into());
    let snapshot = build_workspace_snapshot(&[row], &[], false, &[]);
    let mut state = AppState::new(root.clone(), snapshot, true);
    let mut interp = Interpreter::with_cap(8);
    interp.pr_cli = cli_must_not_run;
    let tui_opts = opts(&state);
    interp.queue_due_pr_lookups(&mut state, &tui_opts);
    drain_jobs(&mut interp, &mut state, 10);
    let path = Path::new("app");
    assert_eq!(state.pr_badge(path), None, "no PR status");
    assert_eq!(
        state.commit_link(path, SHA),
        Some(format!("https://github.com/octo/demo/commit/{SHA}")),
        "the graph and the commit-files footer link the commit"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn r_keeps_the_badge_when_the_cli_fails_and_clears_it_on_no_pr() {
    let (root, mut state) = pr_app("ws-effect-pr-r-keeps", Some("git@github.com:octo/demo.git"));
    let path = Path::new("app");
    let mut interp = Interpreter::with_cap(8);
    let tui_opts = opts(&state);
    interp.pr_cli = gh_approved_pr;
    interp.queue_due_pr_lookups(&mut state, &tui_opts);
    drain_jobs(&mut interp, &mut state, 10);
    let ready = Some(crate::tui::pr_status::PrState::Ready);
    assert_eq!(state.pr_badge(path), ready);

    let refresh = |interp: &mut Interpreter, state: &mut AppState, cli: PrCliRunner| {
        interp.pr_cli = cli;
        let effect = state.dispatch(Action::Refresh);
        assert_eq!(effect, Effect::ReloadRepo { repo: "app".into() });
        schedule_effect(interp, state, effect, &Action::Refresh);
        drain_jobs(interp, state, 10);
    };
    refresh(&mut interp, &mut state, gh_fails);
    assert_eq!(state.pr_badge(path), ready, "a failed r keeps the badge");
    assert_eq!(
        state
            .pr_lookup_failure(path)
            .and_then(|failure| failure.reason.as_deref()),
        Some("HTTP 401: Bad credentials (https://api.github.com/graphql)")
    );
    assert!(state.status.is_empty(), "a background failure stays silent");
    refresh(&mut interp, &mut state, gh_no_pr);
    assert_eq!(state.pr_badge(path), None, "no PR clears it");
    assert_eq!(state.pr_lookup_failure(path), None);
    let _ = std::fs::remove_dir_all(root);
}

/// A badge lookup job whose CLI fails at launch starts the outage of
/// its forge host: one sticky toast naming tool, host, and reason, and
/// the failure mark. A failing `r` adds no toast; a good answer ends it.
#[test]
fn a_failing_pr_lookup_job_toasts_its_host_outage_once_until_it_works() {
    let (root, mut state) = pr_app("ws-effect-pr-outage", Some("git@github.com:octo/demo.git"));
    let path = Path::new("app");
    let mut interp = Interpreter::with_cap(8);
    let tui_opts = opts(&state);
    let outage_texts = |state: &AppState| -> Vec<String> {
        state
            .toasts
            .visible(Instant::now())
            .filter(|(toast, _)| toast.is_outage())
            .map(|(toast, _)| toast.text.clone())
            .collect()
    };
    interp.pr_cli = gh_fails;
    interp.queue_due_pr_lookups(&mut state, &tui_opts);
    drain_jobs(&mut interp, &mut state, 10);
    assert_eq!(
        outage_texts(&state),
        ["gh (github.com): HTTP 401: Bad credentials (https://api.github.com/graphql)"]
    );
    assert_eq!(
        state.pr_mark(path),
        Some(crate::tui::tree::PrMark::LookupFailed)
    );
    assert!(state.status.is_empty(), "the toast, not the status slot");

    let refresh = |interp: &mut Interpreter, state: &mut AppState, cli: PrCliRunner| {
        interp.pr_cli = cli;
        let effect = state.dispatch(Action::Refresh);
        schedule_effect(interp, state, effect, &Action::Refresh);
        drain_jobs(interp, state, 10);
    };
    refresh(&mut interp, &mut state, gh_fails);
    assert_eq!(outage_texts(&state).len(), 1, "no second toast");
    refresh(&mut interp, &mut state, gh_approved_pr);
    assert!(outage_texts(&state).is_empty(), "recovered");
    assert!(state.outages.is_empty());
    let _ = std::fs::remove_dir_all(root);
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

// ---- PR change alerts ----

pub(super) fn gh_merged_pr(argv: &[String], _: &AtomicBool) -> Result<String, Option<String>> {
    assert_eq!(argv.first().map(String::as_str), Some("gh"), "{argv:?}");
    Ok(format!(
        r#"[{{"number":7,"state":"MERGED","url":"{PR_URL}","headRefName":"feature","isCrossRepository":false,"updatedAt":"2026-01-03T00:00:00Z"}}]"#
    ))
}

/// Text and detail of every toast on the stack now, oldest first.
fn toast_texts(state: &AppState) -> Vec<(String, String)> {
    state
        .toasts
        .visible(Instant::now())
        .map(|(toast, _)| (toast.text.clone(), toast.detail.clone()))
        .collect()
}

#[test]
fn the_launch_lookup_never_alerts_and_a_poll_that_changes_the_pr_does() {
    let (root, mut state) = pr_app("ws-effect-pr-alert", Some("git@github.com:octo/demo.git"));
    let path = Path::new("app");
    let mut interp = Interpreter::with_cap(8);
    let tui_opts = opts(&state);
    interp.pr_cli = gh_approved_pr;
    interp.queue_due_pr_lookups(&mut state, &tui_opts);
    drain_jobs(&mut interp, &mut state, 10);
    let ready = Some(crate::tui::pr_status::PrState::Ready);
    assert_eq!(state.pr_badge(path), ready);
    assert!(state.toasts.is_empty(), "no alert on the first load");
    assert!(!state.flashes.contains_key("repo:app"));

    // A poll with the same answer stays quiet.
    let later = |secs| Instant::now() + std::time::Duration::from_secs(secs);
    interp.pr_poll_tick(&mut state, &tui_opts, later(60));
    drain_jobs(&mut interp, &mut state, 10);
    assert!(state.toasts.is_empty(), "same state: no alert");

    interp.pr_cli = gh_merged_pr;
    interp.pr_poll_tick(&mut state, &tui_opts, later(120));
    drain_jobs(&mut interp, &mut state, 10);
    assert_eq!(
        toast_texts(&state),
        [("#7 merged".to_string(), "feature".to_string())]
    );
    assert_eq!(
        state.flashes.get("repo:app").map(|stamp| stamp.kind),
        Some(crate::tui::watch::FlashKind::Add)
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_pr_changes_of_one_poll_cycle_alert_once_its_last_lookup_lands() {
    use crate::tui::pr_status::PrState;
    use crate::tui::pull_request::PullRequest;
    let names = ["a", "b", "c", "d"];
    let mut state = app_state(&names.map(|name| repo(name, false)));
    let remote = |name: &str| format!("git@github.com:acme/{name}.git");
    let found = |number: u64, pr_state| {
        let url = format!("https://github.com/acme/app/pull/{number}");
        PrLookup::Found(PullRequest::fixture(number, &url, pr_state))
    };
    let _ = state.due_pr_lookups();
    for (number, name) in (1..).zip(names) {
        let first = found(number, PrState::CiRunning);
        assert!(state.apply_pr_lookup(Path::new(name), "main", Some(remote(name)), first));
    }

    // The poll cycle's four lookups run under the cap and land one by
    // one.
    let mut interp = Interpreter::with_cap(4);
    let targets = state.due_pr_polls(Instant::now());
    assert_eq!(targets.len(), 4);
    interp.pr_badge_jobs.extend(targets);
    let into = [
        PrState::CiFailed,
        PrState::Ready,
        PrState::CiFailed,
        PrState::Ready,
    ];
    for ((number, name), pr_state) in (1..).zip(names).zip(into) {
        assert!(state.toasts.is_empty(), "{name}: waits for the last lookup");
        let outcome = JobOutcome::PullRequestBadge {
            repo: PathBuf::from(name),
            branch: "main".into(),
            remote: Some(remote(name)),
            lookup: found(number, pr_state),
        };
        apply(&mut interp, &mut state, outcome);
    }
    assert_eq!(
        toast_texts(&state),
        [
            ("#1 CI failed".into(), "feature".into()),
            ("#2 ready to merge".into(), "feature".into()),
            ("+2 more PR changes".into(), "gp".into()),
        ]
    );
    assert!(interp.take_dirty(), "the alert asks for a frame");

    // A `gx` lookup that lands with no badge lookup out alerts at once.
    state.toasts = Default::default();
    apply(
        &mut interp,
        &mut state,
        JobOutcome::PullRequestOpen {
            repo: PathBuf::from("b"),
            branch: "main".into(),
            remote: Some(remote("b")),
            lookup: found(2, PrState::Merged),
            page: PrPage::Pr,
            opened: true,
        },
    );
    assert_eq!(
        toast_texts(&state),
        [("#2 merged".to_string(), "feature".to_string())]
    );
}

/// Poll cycles that overlap (the next cycle's lookup of `a` already
/// waits) and a lookup of `c` that never lands keep the badge queue
/// busy. A PR change still alerts at the first landing after it has
/// waited [`PR_ALERT_WAIT_MS`].
#[test]
fn pr_changes_alert_within_the_wait_bound_while_poll_cycles_overlap() {
    use crate::tui::pr_status::PrState;
    use crate::tui::pull_request::PullRequest;
    use std::time::Duration;
    let names = ["a", "b", "c"];
    let remote = |name: &str| format!("git@github.com:acme/{name}.git");
    let found = |number: u64, pr_state| {
        let url = format!("https://github.com/acme/app/pull/{number}");
        PrLookup::Found(PullRequest::fixture(number, &url, pr_state))
    };
    let target = |name: &str| (PathBuf::from(name), "main".to_string());
    let busy = || {
        let mut state = app_state(&names.map(|name| repo(name, false)));
        let _ = state.due_pr_lookups();
        for (number, name) in (1..).zip(names) {
            let first = found(number, PrState::CiRunning);
            assert!(state.apply_pr_lookup(Path::new(name), "main", Some(remote(name)), first));
        }
        let mut interp = Interpreter::with_cap(4);
        interp.pr_badges.push_back(target("a"));
        interp.pr_badge_jobs.extend([target("b"), target("c")]);
        (state, interp)
    };
    let landed_ago = |state: &mut AppState, ms: u64| {
        let at = Instant::now() - Duration::from_millis(ms);
        let failed = found(1, PrState::CiFailed);
        assert!(state.apply_pr_lookup_at(Path::new("a"), "main", Some(remote("a")), failed, at));
        at
    };
    let land_b = |interp: &mut Interpreter, state: &mut AppState| {
        let outcome = JobOutcome::PullRequestBadge {
            repo: PathBuf::from("b"),
            branch: "main".into(),
            remote: Some(remote("b")),
            lookup: found(2, PrState::Ready),
        };
        apply(interp, state, outcome);
    };

    // The change of `a` landed 1 s ago: a busy landing keeps it waiting,
    // and the newer change of `b` waits with it.
    let (mut state, mut interp) = busy();
    let at = landed_ago(&mut state, 1000);
    land_b(&mut interp, &mut state);
    assert!(state.toasts.is_empty(), "busy and under the bound");
    assert_eq!(state.pr_transitions_waiting_since(), Some(at), "the oldest");

    // It landed 2.5 s ago: the next landing alerts both, while the
    // queue is still busy.
    let (mut state, mut interp) = busy();
    let _ = landed_ago(&mut state, PR_ALERT_WAIT_MS + 500);
    land_b(&mut interp, &mut state);
    assert!(!interp.pr_badges.is_empty() && !interp.pr_badge_jobs.is_empty());
    assert_eq!(
        toast_texts(&state),
        [
            ("#1 CI failed".to_string(), "feature".to_string()),
            ("#2 ready to merge".to_string(), "feature".to_string()),
        ]
    );
    assert_eq!(state.pr_transitions_waiting_since(), None, "drained");
    assert!(interp.take_dirty());
}

// ---- PR title command ----

#[cfg(unix)]
#[test]
fn a_title_fill_runs_on_a_worker_and_lands_in_the_group_headers() {
    use crate::tui::state::prs_fixtures::{set_title_command, title_script, workspace};
    let mut state = workspace(false);
    let cmd = title_script();
    set_title_command(&mut state, &cmd);
    let mut interp = Interpreter::with_cap(4);
    let effect = state.dispatch(Action::OpenPrsTab);
    schedule_effect(&mut interp, &mut state, effect, &Action::OpenPrsTab);
    assert_eq!(interp.sched.queued_user_tag(UserTag::PrTitles), 1);
    assert_eq!(state.pr_group_title("ABC-123"), None, "not yet");
    drain_jobs(&mut interp, &mut state, 10);
    assert_eq!(state.pr_group_title("ABC-123"), Some("Title ABC-123"));
    assert_eq!(state.pr_group_title("ABC-125"), Some("Title ABC-125"));
    assert_eq!(state.pr_group_title("other"), None, "never for other");
    assert!(interp.take_dirty(), "the headers repaint");

    // A badge lookup that lands with none left out ends a cycle: a new
    // fill runs, and the titles on screen stay until it lands.
    let outcome = JobOutcome::PullRequestBadge {
        repo: PathBuf::from("lib"),
        branch: "feature/ABC-120-cache".into(),
        remote: Some(crate::tui::state::prs_fixtures::GH_LIB.into()),
        lookup: PrLookup::NoPr,
    };
    apply(&mut interp, &mut state, outcome);
    assert_eq!(interp.sched.queued_user_tag(UserTag::PrTitles), 1);
    assert_eq!(state.pr_group_title("ABC-120"), Some("Title ABC-120"));
    drain_jobs(&mut interp, &mut state, 10);
    assert_eq!(
        state.pr_group_title("ABC-120"),
        None,
        "the merged ticket left the list with its PR"
    );
    assert_eq!(state.pr_group_title("ABC-123"), Some("Title ABC-123"));

    // Another landing with the same keys and a fresh fill runs nothing.
    let outcome = JobOutcome::PullRequestBadge {
        repo: PathBuf::from("lib"),
        branch: "feature/ABC-120-cache".into(),
        remote: Some(crate::tui::state::prs_fixtures::GH_LIB.into()),
        lookup: PrLookup::NoPr,
    };
    apply(&mut interp, &mut state, outcome);
    assert_eq!(interp.sched.queued_user_tag(UserTag::PrTitles), 0);

    // A quit stops a fill before its next key, and the titles on
    // screen stay.
    let later = Instant::now() + std::time::Duration::from_secs(3600);
    let effect = state.new_pr_title_cycle(later);
    let Effect::FillPrTitles { groups, .. } = &effect else {
        panic!("a poll period on: {effect:?}");
    };
    // What the fill would have shown had it not been cut short.
    let renamed = crate::tui::state::prs_fixtures::script_with("echo \"Renamed $1\"");
    let mut cut_short = crate::tui::pr_groups::TitleCache::default();
    cut_short.fill(&renamed, groups);
    assert_eq!(cut_short.get("ABC-123"), Some("Renamed ABC-123"));
    schedule_effect(&mut interp, &mut state, effect, &Action::None);
    interp.pr_cancel.store(true, Ordering::Relaxed);
    drain_jobs(&mut interp, &mut state, 10);
    assert_eq!(
        state.pr_group_title("ABC-123"),
        Some("Title ABC-123"),
        "a cancelled fill changes nothing on screen"
    );
    // A cancelled fill that did get titles still shows none of them.
    apply(
        &mut interp,
        &mut state,
        JobOutcome::PrTitles {
            titles: cut_short,
            cancelled: true,
        },
    );
    assert_eq!(state.pr_group_title("ABC-123"), Some("Title ABC-123"));
    assert_eq!(state.pr_group_title("ABC-125"), Some("Title ABC-125"));
    let _ = std::fs::remove_dir_all(cmd.parent().unwrap());
    let _ = std::fs::remove_dir_all(renamed.parent().unwrap());
}

/// A newer fill before the spawn replaces the waiting one: one slot,
/// and the one job fills the newer groups.
#[cfg(unix)]
#[test]
fn a_newer_title_fill_replaces_the_waiting_one() {
    use crate::tui::state::prs_fixtures::{set_title_command, title_script, workspace};
    let mut state = workspace(false);
    let cmd = title_script();
    set_title_command(&mut state, &cmd);
    let mut interp = Interpreter::with_cap(4);
    let effects = match state.dispatch(Action::OpenPrsTab) {
        Effect::Batch(effects) => effects,
        effect => vec![effect],
    };
    let groups = effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::FillPrTitles { groups, .. } => Some(groups),
            _ => None,
        })
        .expect("the PRs tab asks for titles");
    let older = Effect::FillPrTitles {
        cmd: cmd.clone(),
        groups: Vec::new(),
    };
    let newer = Effect::FillPrTitles {
        cmd: cmd.clone(),
        groups,
    };
    schedule_effect(&mut interp, &mut state, older, &Action::None);
    schedule_effect(&mut interp, &mut state, newer, &Action::None);
    assert_eq!(interp.sched.queued_user_tag(UserTag::PrTitles), 1);
    let jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "one fill");
    let (_, work) = jobs.into_iter().next().unwrap();
    let JobOutcome::PrTitles { titles, .. } = work() else {
        panic!("expected a title fill");
    };
    assert_eq!(titles.get("ABC-123"), Some("Title ABC-123"), "newer groups");
    let _ = std::fs::remove_dir_all(cmd.parent().unwrap());
}

fn copy_effect(text: &str) -> Effect {
    Effect::CopyClipboard {
        text: text.into(),
        announce: true,
    }
}

/// OSC 52 fails (no terminal): the host tool runs on a worker, never in
/// `schedule`, and its result lands as `copied` / `copy failed` plus the
/// `y` overlay header. One copy at a time; a copy asked for meanwhile runs
/// next.
#[test]
fn clipboard_host_tool_runs_on_a_worker_and_its_result_lands() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(4);
    interp.clipboard_tool = |text, _| text == "good";
    state.comment_export = Some(comments::CommentExport {
        markdown: "good".into(),
        copied: None,
    });
    schedule_effect(&mut interp, &mut state, copy_effect("good"), &Action::None);
    assert_eq!(state.status, "", "nothing shows before the tool answers");
    assert_eq!(state.comment_export.as_ref().unwrap().copied, None);
    assert_eq!(interp.sched.queued_user_tag(UserTag::Clipboard), 1);

    let mut jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "one host tool copy");
    let (id, work) = jobs.pop().unwrap();
    schedule_effect(&mut interp, &mut state, copy_effect("bad"), &Action::None);
    assert_eq!(
        interp.sched.queued_user_tag(UserTag::Clipboard),
        0,
        "the next copy waits for the running one"
    );
    let outcome = work();
    assert!(matches!(
        outcome,
        JobOutcome::ClipboardCopied {
            ok: Some(true),
            announce: true
        }
    ));
    apply_id(&mut interp, &mut state, id, outcome);
    assert_eq!(state.status, STATUS_COPIED);
    assert_eq!(state.status.kind(), StatusKind::Ok);
    assert_eq!(state.comment_export.as_ref().unwrap().copied, Some(true));

    let mut jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "the waiting copy spawns");
    let (id, work) = jobs.pop().unwrap();
    apply_id(&mut interp, &mut state, id, work());
    assert_eq!(state.status, STATUS_COPY_FAILED);
    assert_eq!(state.status.kind(), StatusKind::Error);
    assert!(!interp.clipboard_job.holds_slot());
    assert!(capture_jobs(&mut interp, &mut state).is_empty(), "idle");
}

/// OSC 52 works: the copy shows `copied` at once on the loop thread. The
/// host tool still runs on a worker, and its answer does not overwrite a
/// newer status.
#[test]
fn clipboard_osc52_shows_the_result_at_once_and_the_tool_answer_changes_nothing() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(4);
    interp.osc52_writer = |_| true;
    interp.clipboard_tool = |_, _| false;
    schedule_effect(&mut interp, &mut state, copy_effect("text"), &Action::None);
    assert_eq!(state.status, STATUS_COPIED, "OSC 52 is enough");
    let mut jobs = capture_jobs(&mut interp, &mut state);
    assert_eq!(jobs.len(), 1, "the host tool still runs");
    let (id, work) = jobs.pop().unwrap();
    state.status = StatusMessage::info("newer");
    apply_id(&mut interp, &mut state, id, work());
    assert_eq!(
        state.status, "newer",
        "a failed tool after OSC 52 is silent"
    );
    assert!(!interp.clipboard_job.holds_slot());
}

/// A host tool copy that panics frees its slot, and a copy that still
/// waits on its result gets `copy failed` in the `y` overlay header.
#[test]
fn clipboard_copy_that_panics_frees_its_slot_and_fails_the_copy() {
    let mut state = fixture_state();
    let mut interp = Interpreter::with_cap(4);
    interp.clipboard_tool = |_, _| panic!("clipboard tool exploded");
    state.comment_export = Some(comments::CommentExport {
        markdown: "text".into(),
        copied: None,
    });
    schedule_effect(&mut interp, &mut state, copy_effect("text"), &Action::None);
    let (id, work) = capture_jobs(&mut interp, &mut state).pop().unwrap();
    let message = run_job(work).err().expect("the job panicked");
    let tui_opts = opts(&state);
    interp.apply_panic(&mut state, &tui_opts, id, &message);
    assert_eq!(state.comment_export.as_ref().unwrap().copied, Some(false));
    assert_eq!(
        state.status,
        "clipboard copy job panicked: clipboard tool exploded"
    );
    assert!(!interp.clipboard_job.holds_slot());
}
